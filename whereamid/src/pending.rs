//! Pending queue background drain task.

use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::{debug, info, warn};

use crate::provider::{AppleProvider, Provider, WigleProvider};
use crate::resolver::{resolve_chain_with_observations, ChainMode, ObservedAp};
use crate::server::DaemonState;

/// Maximum number of expired not_found entries to revive per drain pass.
/// Bounded so a long-tail not_found revival cannot dominate the drain
/// budget; subsequent passes pick up the remaining ones (task-0063).
const NOT_FOUND_REVIVAL_BATCH: usize = 5;
/// Maximum number of pending BSSIDs to attempt to resolve per drain pass.
/// Bounded by --wigle-daily-limit and provider rate limits in practice; this
/// is the structural upper bound on per-cycle WiGLE spend (task-0063).
const PENDING_DRAIN_BATCH: usize = 10;

/// Spawn the pending queue drain loop. Runs every `pending_interval` seconds.
pub async fn run_pending_drain(state: Arc<DaemonState>) {
    let interval = Duration::from_secs(state.args.pending_interval);
    let max_attempts = state.args.pending_max_attempts;

    info!(
        "pending drain task started (interval: {}s, max_attempts: {})",
        state.args.pending_interval, max_attempts
    );

    loop {
        // Level-triggered shutdown (task-0190): wake immediately on cancel
        // instead of waiting out `interval`. `biased` checks cancel first.
        tokio::select! {
            biased;
            _ = state.shutdown.cancelled() => break,
            _ = sleep(interval) => {}
        }
        // Run one pass, abandoning it promptly if shutdown fires mid-pass.
        // Dropping drain_once mid-flight is safe: resolve_chain never holds the
        // SQLite resource across an await, actor-side batches are atomic, and
        // a WiGLE quota lease is conditionally refunded by its Drop protocol
        // even if cancellation races reservation commit (task-0190/0192). This does not change resolve_chain
        // semantics — it only cancels the in-flight pass.
        tokio::select! {
            biased;
            _ = state.shutdown.cancelled() => break,
            _ = drain_once(
                &state,
                max_attempts,
                &[&AppleProvider, &WigleProvider],
                state.privacy.ticket(),
            ) => {}
        }
    }
}

/// Run one drain pass over the pending queue using `providers` as the chain.
///
/// `providers` is injected (rather than hard-coded) so tests can drive the
/// pass with `MockProvider`s; production passes `[AppleProvider, WigleProvider]`.
/// The real Apple provider makes network calls and never returns `Skipped`, so the
/// all-Skipped fairness/age-out cases below are reachable only under mocks.
async fn drain_once(
    state: &Arc<DaemonState>,
    max_attempts: i32,
    providers: &[&dyn Provider],
    ticket: crate::privacy::PrivacyTicket,
) {
    // Clean up entries that exceeded max attempts
    {
        let Some(_privacy) = state.privacy.read(ticket).await else {
            return;
        };
        match state.db.delete_expired_pending(max_attempts).await {
            Ok(deleted) if deleted > 0 => {
                info!("deleted {deleted} pending entries exceeding {max_attempts} attempts");
            }
            Ok(_) => {}
            Err(e) => {
                warn!("failed to delete expired pending: {e}");
                state.record_db_failure();
            }
        }
    }

    // Re-check expired not_found entries (30 day TTL)
    {
        let Some(_privacy) = state.privacy.read(ticket).await else {
            return;
        };
        match state
            .db
            .revive_expired_not_found(state.args.not_found_ttl_days, NOT_FOUND_REVIVAL_BATCH)
            .await
        {
            Ok(expired) => {
                for bssid in expired {
                    debug!("re-checking expired not_found entry: {bssid}");
                }
            }
            Err(e) => {
                warn!("failed to get expired not_found: {e}");
                state.record_db_failure();
            }
        }
    }

    // Prune definitively-dead not_found rows (older than 2x TTL) so the
    // negative cache cannot grow monotonically on a long-running daemon at a
    // busy location. Past 1x TTL a row is never re-checked anyway (task-0110).
    {
        let Some(_privacy) = state.privacy.read(ticket).await else {
            return;
        };
        match state
            .db
            .prune_not_found(state.args.not_found_ttl_days)
            .await
        {
            Ok(n) if n > 0 => {
                info!("pruned {n} not_found entries older than 2x TTL");
            }
            Ok(_) => {}
            Err(e) => {
                warn!("failed to prune not_found: {e}");
                state.record_db_failure();
            }
        }
    }

    // Optional hard row-count cap on the not_found negative cache. Opt-in via
    // --not-found-max-rows: absent (None) means byte-identical behavior to the
    // age-only prune above (no eviction). When set, evict the oldest rows (by
    // checked_at) down to the cap for extremely busy locations where within-TTL
    // churn alone could blow the size budget (task-0258).
    if let Some(max_rows) = state.args.not_found_max_rows {
        let Some(_privacy) = state.privacy.read(ticket).await else {
            return;
        };
        match state.db.cap_not_found_rows(max_rows).await {
            Ok(n) if n > 0 => {
                info!("evicted {n} oldest not_found entries to cap at {max_rows} rows");
            }
            Ok(_) => {}
            Err(e) => {
                warn!("failed to cap not_found rows: {e}");
                state.record_db_failure();
            }
        }
    }

    // Pick up to PENDING_DRAIN_BATCH pending MACs
    let pending = {
        let Some(_privacy) = state.privacy.read(ticket).await else {
            return;
        };
        match state.db.get_pending(PENDING_DRAIN_BATCH).await {
            Ok(p) => p,
            Err(e) => {
                // A read error silently aborts this drain pass (invisible to
                // ops except in the journal); count it so /stats reflects the
                // DB read fault (task-0252).
                warn!("failed to get pending entries: {e}");
                state.record_db_read_failure();
                return;
            }
        }
    };

    if pending.is_empty() {
        return;
    }

    debug!("draining {} pending entries", pending.len());

    // The pending list IS the input — they're already known to need
    // resolution, so don't pre-filter on cache or not_found.
    //
    // The chain marks not_found only when a provider authoritatively returned
    // NotFound and nothing was genuinely transient (task-0189). When it marks,
    // manage_pending (task-0052) removes the pending row in the
    // same critical section, so this drain replaces the removed
    // drain_cleanup_after_chain.
    let bssids: Vec<String> = pending
        .iter()
        .map(|pending| pending.bssid.clone())
        .collect();
    let observations: Vec<ObservedAp> = pending
        .iter()
        .map(|pending| ObservedAp {
            bssid: pending.bssid.clone(),
            ssid: pending.ssid.clone(),
        })
        .collect();
    // ChainMode::Drain owns the pending table: it writes through, marks
    // not_found at chain end, and deletes the pending row on a definitive
    // outcome. task-0189: retry accounting is done once per pass below (a
    // per-error increment would double-count), so Drain does not queue pending
    // on network errors.
    let _ = resolve_chain_with_observations(
        &bssids,
        state,
        providers,
        ChainMode::Drain,
        ticket,
        &observations,
    )
    .await;

    // task-0189: advance the attempts counter once per drain pass for every
    // BSSID we just tried. resolve_chain deletes rows it resolved or
    // definitively marked not_found, so those UPDATEs affect 0 rows (no-op).
    // Rows that survived this pass — a genuinely-transient failure, or (only
    // reachable under mocks, since the real Apple provider never Skips) an
    // all-providers-skipped BSSID — advance one step. This both (a) ages
    // hopeless rows out via delete_expired_pending(pending_max_attempts) and
    // (b) makes get_pending's `ORDER BY attempts ASC` rotate, so a batch of
    // stuck rows cannot monopolise every drain and starve newer entries
    // (task-0189 AC#2/AC#3).
    //
    // Caveat: a BSSID deferred to a concurrent in-flight task is still pending
    // here and is bumped too. Harmless — that task's success/not_found deletes
    // the row well before attempts reaches the cap.
    {
        if let Some(_privacy) = state.privacy.read(ticket).await {
            if let Err(e) = state.db.increment_pending_attempts(bssids.clone()).await {
                warn!("failed to increment pending attempts: {e}");
                state.record_db_failure();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{MockProvider, ProviderOutcome};
    use crate::resolver::{resolve_chain, ChainMode};
    use crate::server::DaemonState;
    // task-0230: delegate to the single shared DaemonState constructor.
    fn test_state() -> Arc<DaemonState> {
        crate::server::test_state()
    }

    /// task-0045 + task-0052 end-to-end: definitive NotFound from every
    /// provider results in (1) the BSSID being marked not_found in the DB,
    /// AND (2) the BSSID being deleted from pending so it won't retry on
    /// every drain. Both happen inside resolve_chain via the
    /// manage_pending policy bit (drain_cleanup_after_chain
    /// was collapsed in task-0052).
    #[tokio::test]
    async fn drain_chain_marks_not_found_and_deletes_pending_on_definitive_miss() {
        let state = test_state();
        let bssid = "aa:bb:cc:dd:ee:f1".to_string();

        // Seed: BSSID is in pending.
        crate::server::lock_db(&state)
            .insert_pending(&bssid, None, None, None)
            .unwrap();
        assert!(crate::server::lock_db(&state).is_pending(&bssid).unwrap());

        let mock_a = MockProvider::new("a", |_| ProviderOutcome::NotFound);
        let mock_b = MockProvider::new("b", |_| ProviderOutcome::NotFound);
        let providers: [&dyn Provider; 2] = [&mock_a, &mock_b];
        let _ = resolve_chain(
            std::slice::from_ref(&bssid),
            &state,
            &providers,
            ChainMode::Drain,
            state.privacy.ticket(),
        )
        .await;

        let nf = crate::server::lock_db(&state)
            .is_not_found(&bssid, state.args.not_found_ttl_days)
            .unwrap();
        assert!(nf, "definitive NotFound must mark not_found");
        let pending = crate::server::lock_db(&state).is_pending(&bssid).unwrap();
        assert!(
            !pending,
            "pending row must be deleted by the chain when not_found is marked"
        );
    }

    /// Counterexample: a transient NetworkError must NOT delete the pending
    /// row (the row is still pending, not_found wasn't marked).
    #[tokio::test]
    async fn drain_chain_keeps_pending_when_transient_error() {
        let state = test_state();
        let bssid = "aa:bb:cc:dd:ee:f2".to_string();

        crate::server::lock_db(&state)
            .insert_pending(&bssid, None, None, None)
            .unwrap();

        let mock_a = MockProvider::new("a", |_| ProviderOutcome::NotFound);
        let mock_b = MockProvider::new("b", |_| {
            ProviderOutcome::NetworkError(anyhow::anyhow!("simulated"))
        });
        let providers: [&dyn Provider; 2] = [&mock_a, &mock_b];
        let _ = resolve_chain(
            std::slice::from_ref(&bssid),
            &state,
            &providers,
            ChainMode::Drain,
            state.privacy.ticket(),
        )
        .await;

        let nf = crate::server::lock_db(&state)
            .is_not_found(&bssid, state.args.not_found_ttl_days)
            .unwrap();
        assert!(!nf, "transient error must not mark not_found");
        let pending = crate::server::lock_db(&state).is_pending(&bssid).unwrap();
        assert!(pending, "pending row must survive a transient error");
    }

    fn mock(name: &'static str, outcome: fn(&str) -> ProviderOutcome) -> MockProvider {
        MockProvider::new(name, outcome)
    }

    #[tokio::test]
    async fn pending_retry_preserves_scanner_ssid_for_classification() {
        let state = test_state();
        let mobile = "aa:bb:cc:dd:ee:b1".to_owned();
        let hidden = "b8:50:01:aa:bb:cc".to_owned();
        {
            let db = crate::server::lock_db(&state);
            db.insert_pending(&mobile, Some("MOVIA_BUS_42"), None, None)
                .unwrap();
            db.insert_pending(&hidden, None, None, None).unwrap();
        }
        let provider = mock("apple-like", |bssid| {
            ProviderOutcome::Found(crate::db::ApInfo {
                bssid: bssid.to_owned(),
                ssid: Some("DSB_Wi-Fi".to_owned()),
                provider_reported_lat: 55.0,
                provider_reported_lon: 12.0,
                encryption: None,
                frequency: None,
                city: None,
                country: None,
                is_mobile: false,
                source: crate::db::Source::Apple,
            })
        });

        drain_once(
            &state,
            20,
            &[&provider as &dyn Provider],
            state.privacy.ticket(),
        )
        .await;

        let db = crate::server::lock_db(&state);
        let mobile_ap = db.get_ap(&mobile).unwrap().unwrap();
        assert_eq!(mobile_ap.ssid.as_deref(), Some("MOVIA_BUS_42"));
        assert!(mobile_ap.is_mobile);
        let hidden_ap = db.get_ap(&hidden).unwrap().unwrap();
        assert_eq!(hidden_ap.ssid, None);
        assert!(
            !hidden_ap.is_mobile,
            "hidden scanner observation must survive provider metadata"
        );
        assert!(!db.is_pending(&mobile).unwrap());
        assert!(!db.is_pending(&hidden).unwrap());
    }

    /// task-0189 AC#1: with WiGLE unconfigured (Skipped) and Apple
    /// authoritatively NotFound, the BSSID is marked not_found and, under the
    /// drain policy, its pending row deleted. Before task-0189, Skipped fed the
    /// transient guard and suppressed the mark forever, so every scan
    /// re-queried an Apple WPS call for the same unknown AP.
    #[tokio::test]
    async fn drain_chain_marks_not_found_when_apple_nf_and_wigle_skipped() {
        let state = test_state();
        let bssid = "aa:bb:cc:dd:ee:a1".to_string();
        crate::server::lock_db(&state)
            .insert_pending(&bssid, None, None, None)
            .unwrap();

        let apple = MockProvider::new("apple", |_| ProviderOutcome::NotFound);
        let wigle = mock("wigle", |_| ProviderOutcome::Skipped);
        let providers: [&dyn Provider; 2] = [&apple, &wigle];
        let _ = resolve_chain(
            std::slice::from_ref(&bssid),
            &state,
            &providers,
            ChainMode::Drain,
            state.privacy.ticket(),
        )
        .await;

        assert_eq!(apple.call_count(), 1, "apple must be consulted");
        let nf = crate::server::lock_db(&state)
            .is_not_found(&bssid, state.args.not_found_ttl_days)
            .unwrap();
        assert!(
            nf,
            "Apple-definitive NotFound with WiGLE Skipped must mark not_found"
        );
        assert!(
            !crate::server::lock_db(&state).is_pending(&bssid).unwrap(),
            "marking not_found must delete the pending row under the drain policy"
        );
    }

    /// task-0189 AC#1 negative twin: when EVERY provider merely Skipped, no
    /// provider was actually consulted, so there is no authoritative absence —
    /// the BSSID stays pending and is NOT marked not_found.
    #[tokio::test]
    async fn drain_chain_all_skipped_leaves_pending_unmarked() {
        let state = test_state();
        let bssid = "aa:bb:cc:dd:ee:a2".to_string();
        crate::server::lock_db(&state)
            .insert_pending(&bssid, None, None, None)
            .unwrap();

        let p1 = mock("p1", |_| ProviderOutcome::Skipped);
        let p2 = mock("p2", |_| ProviderOutcome::Skipped);
        let providers: [&dyn Provider; 2] = [&p1, &p2];
        let _ = resolve_chain(
            std::slice::from_ref(&bssid),
            &state,
            &providers,
            ChainMode::Drain,
            state.privacy.ticket(),
        )
        .await;

        assert!(
            !crate::server::lock_db(&state)
                .is_not_found(&bssid, state.args.not_found_ttl_days)
                .unwrap(),
            "all-Skipped BSSID must NOT be marked not_found"
        );
        assert!(
            crate::server::lock_db(&state).is_pending(&bssid).unwrap(),
            "all-Skipped BSSID must remain pending"
        );
    }

    /// Stopped-provider shadow (review follow-up to task-0189): once WiGLE
    /// hard-stops (429) on one BSSID, later BSSIDs in the same batch see it
    /// only as `stopped` — that absence is just as transient as the direct
    /// HardStop, so an Apple-definitive NotFound alone must NOT mark them
    /// not_found. Without the `stopped[idx]` -> hard_transient insert in
    /// resolve_chain, B2 would be blacklisted for the full TTL off a
    /// momentary rate-limit shadow.
    #[tokio::test]
    async fn drain_chain_hard_stop_shadow_does_not_mark_later_bssids() {
        let state = test_state();
        let b1 = "aa:bb:cc:dd:ee:a7".to_string();
        let b2 = "aa:bb:cc:dd:ee:a8".to_string();
        for b in [&b1, &b2] {
            crate::server::lock_db(&state)
                .insert_pending(b, None, None, None)
                .unwrap();
        }

        let apple = mock("apple", |_| ProviderOutcome::NotFound);
        let wigle = MockProvider::new("wigle-429", |_| ProviderOutcome::HardStop);
        let providers: [&dyn Provider; 2] = [&apple, &wigle];
        let bssids = [b1.clone(), b2.clone()];
        let _ = resolve_chain(
            &bssids,
            &state,
            &providers,
            ChainMode::Drain,
            state.privacy.ticket(),
        )
        .await;

        assert_eq!(
            wigle.call_count(),
            1,
            "wigle must not be consulted again after hard-stopping"
        );
        for b in [&b1, &b2] {
            assert!(
                !crate::server::lock_db(&state)
                    .is_not_found(b, state.args.not_found_ttl_days)
                    .unwrap(),
                "{b}: transient hard-stop (direct or shadowed) must not mark not_found"
            );
            assert!(
                crate::server::lock_db(&state).is_pending(b).unwrap(),
                "{b}: pending row must survive the transient outage"
            );
        }
    }

    /// task-0189 AC#4: a quota-exhausted WiGLE returns Skipped exactly like an
    /// unconfigured one (provider.rs `wigle_lookup`: both `!is_configured` and
    /// `!reserved` yield `Skipped`), so from the chain's viewpoint this is
    /// identical to AC#1. A quota-exhausted day must not disable not_found
    /// marking for an Apple-definitive miss.
    #[tokio::test]
    async fn drain_chain_marks_not_found_when_apple_nf_and_wigle_quota_exhausted() {
        let state = test_state();
        let bssid = "aa:bb:cc:dd:ee:a4".to_string();
        crate::server::lock_db(&state)
            .insert_pending(&bssid, None, None, None)
            .unwrap();

        let apple = mock("apple", |_| ProviderOutcome::NotFound);
        // Stands in for wigle_lookup's quota-exhausted (`!reserved`) branch.
        let wigle = mock("wigle-quota-exhausted", |_| ProviderOutcome::Skipped);
        let providers: [&dyn Provider; 2] = [&apple, &wigle];
        let _ = resolve_chain(
            std::slice::from_ref(&bssid),
            &state,
            &providers,
            ChainMode::Drain,
            state.privacy.ticket(),
        )
        .await;

        assert!(
            crate::server::lock_db(&state)
                .is_not_found(&bssid, state.args.not_found_ttl_days)
                .unwrap(),
            "quota-exhausted WiGLE must not suppress an Apple-definitive not_found mark"
        );
    }

    /// task-0189 AC#2: a pending row whose every drain pass ends without
    /// success must eventually age out via `pending_max_attempts`. The
    /// post-pass attempts bump feeds `delete_expired_pending`. (All-Skipped is
    /// a mock-only shape — the real Apple never Skips; the production
    /// equivalent is a persistently NetworkError-ing Apple.)
    #[tokio::test]
    async fn drain_once_ages_out_perpetually_unresolved_row() {
        let state = test_state();
        let max_attempts = 2;
        let bssid = "aa:bb:cc:dd:ee:a3".to_string();
        crate::server::lock_db(&state)
            .insert_pending(&bssid, None, None, None)
            .unwrap();

        let skip = mock("skip", |_| ProviderOutcome::Skipped);
        let providers: [&dyn Provider; 1] = [&skip];
        // attempts: 0 -> 1 -> 2, then delete_expired removes it at attempts>=2.
        for _ in 0..(max_attempts + 1) {
            drain_once(&state, max_attempts, &providers, state.privacy.ticket()).await;
        }

        assert!(
            !crate::server::lock_db(&state).is_pending(&bssid).unwrap(),
            "a perpetually-unresolved pending row must age out via max_attempts"
        );
    }

    /// task-0189 AC#3: with more stuck (all-Skipped) rows than one drain batch,
    /// no row is starved. The attempts bump rotates get_pending's
    /// `ORDER BY attempts ASC`, so a second pass drains the rows the first pass
    /// could not reach. Assert every row advanced (none stuck at 0), which is
    /// impossible under the old head-of-line monopoly.
    #[tokio::test]
    async fn drain_once_rotates_across_batches_no_head_of_line_monopoly() {
        let state = test_state();
        let total = PENDING_DRAIN_BATCH + 5; // 15 rows > batch of 10
        let bssids: Vec<String> = (0..total)
            .map(|i| format!("aa:bb:cc:dd:ee:{i:02x}"))
            .collect();
        {
            let db = crate::server::lock_db(&state);
            for b in &bssids {
                db.insert_pending(b, None, None, None).unwrap();
            }
        }

        let skip = mock("skip", |_| ProviderOutcome::Skipped);
        let providers: [&dyn Provider; 1] = [&skip];
        // High cap so nothing expires; we are testing rotation, not age-out.
        let max_attempts = 1000;
        drain_once(&state, max_attempts, &providers, state.privacy.ticket()).await;
        drain_once(&state, max_attempts, &providers, state.privacy.ticket()).await;

        let all = crate::server::lock_db(&state)
            .get_pending(total * 2)
            .unwrap();
        assert_eq!(all.len(), total, "no rows resolved/removed (all Skipped)");
        assert!(
            all.iter().all(|pending| pending.attempts >= 1),
            "every pending row must advance across two passes (no starvation): {all:?}"
        );
    }

    /// task-0190 AC#1: cancelling during the sleep phase wakes the drain loop
    /// immediately (level-triggered CancellationToken) instead of waiting out
    /// the 300s interval. Paused clock makes this exact: no wall-clock wait.
    #[tokio::test(start_paused = true)]
    async fn run_pending_drain_wakes_from_sleep_on_cancel() {
        let state = test_state(); // pending_interval defaults to 300s
        let loop_state = Arc::clone(&state);
        let handle = tokio::spawn(async move { run_pending_drain(loop_state).await });

        // Let the spawned loop reach its sleep select (virtual 10ms).
        sleep(Duration::from_millis(10)).await;
        let start = tokio::time::Instant::now();
        state.shutdown.cancel();

        // The loop must exit promptly, well before the 300s interval elapses.
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("drain loop must exit promptly after cancel")
            .expect("drain loop task must not panic");
        assert!(
            start.elapsed() < Duration::from_secs(300),
            "must not wait out the pending interval (elapsed {:?})",
            start.elapsed()
        );
    }

    /// task-0252: `drain_once` reads the pending queue via `get_pending`; a real
    /// read error there silently aborts the drain pass (invisible to ops except
    /// in the journal). Drop `pending` to force the error deterministically and
    /// assert it is counted via `db_read_failures`.
    #[tokio::test]
    async fn drain_once_counts_db_read_failure_when_pending_read_errors() {
        use std::sync::atomic::Ordering::Relaxed;

        let state = test_state();
        crate::server::lock_db(&state)
            .execute_batch_for_test("DROP TABLE pending;")
            .unwrap();
        let m = mock("m", |_| ProviderOutcome::NotFound);
        let providers: [&dyn Provider; 1] = [&m];

        assert_eq!(state.db_read_failures.load(Relaxed), 0);
        drain_once(
            &state,
            state.args.pending_max_attempts,
            &providers,
            state.privacy.ticket(),
        )
        .await;
        assert_eq!(
            state.db_read_failures.load(Relaxed),
            1,
            "get_pending read error must be counted"
        );
    }
}
