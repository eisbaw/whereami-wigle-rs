//! Shared test-only helpers reachable from BOTH build targets.
//!
//! `whereamid` compiles as both a library (`lib.rs`) and a binary (`main.rs`),
//! and several modules (`db`, `trilaterate`, `quality`, …) are declared under
//! both roots, so their `#[cfg(test)]` code runs in TWO separate test binaries.
//! Helpers that those tests share must therefore live in a module declared under
//! both roots. This file is declared `#[cfg(test)] mod test_support;` in BOTH
//! `lib.rs` and `main.rs`; `crate::test_support::…` then resolves in either
//! target (task-0251).

#![cfg(test)]

/// Install, once per process, a permissive global default subscriber so that
/// thread-local (`with_default` / `with_subscriber`) capture tests reliably
/// see their own events under `cargo test`'s parallel harness (task-0248).
///
/// ROOT CAUSE: tracing caches a process-global per-callsite `Interest`. That
/// cache is (re)built by folding every registered dispatcher's
/// `register_callsite` and defaulting to `Interest::never()` when the fold is
/// empty (tracing-core 0.1.36 `rebuild_callsite_interest`). A callsite that is
/// only ever exercised by ONE scoped-subscriber test — e.g. the non-finite
/// skip `warn!` in `handle_locate` — can be first-registered in a window where
/// that test's dispatcher is not yet visible to the fold, caching it as
/// `never`. Thereafter `warn!` short-circuits BEFORE consulting the active
/// dispatcher, the event is silently dropped, and the scoped capture sees `[]`
/// (measured ~12% under full-bin parallelism; two thread-local-only fixes
/// failed because they never touched the dispatcher registry the fold reads).
///
/// FIX (algebraic, not probabilistic): a global default built via
/// `Dispatch::new` is registered into that same registry, and
/// `set_global_default` leaks it so its weak ref never dies — it is therefore
/// a permanent member of the dispatcher set the interest fold reads. Its
/// `register_callsite` returns `Interest::sometimes()`, and `Interest::and`
/// collapses any mismatched pair to `sometimes()`, so no fold can ever cache a
/// callsite as `never`. A `sometimes` callsite always evaluates `enabled()`
/// against the CURRENT thread-local dispatcher, so the scoped capture
/// deterministically sees its own events regardless of cross-thread timing.
///
/// This subscriber is a no-op sink: `event()` does nothing and `enabled()` is
/// false, so it never records and can never FABRICATE an event in any scoped
/// capture. What it deliberately DOES change is that genuinely-emitted events
/// at previously-`never`-cached callsites are no longer silently dropped — so
/// sibling capture tests become deterministic instead of occasionally lossy.
/// It cannot hide a real absence (it records nothing), so absence assertions
/// stay sound; the only behavioural change is "lost real event" -> "seen real
/// event".
///
/// task-0251 hoisted this out of `server::dispatch_tests` into this shared,
/// lib+bin-reachable module and wired it into EVERY tracing-capture test
/// (server, db, trilaterate, config) so the guarantee holds regardless of run
/// shape, test order, or which test binary (lib vs bin) is running — not only
/// the full-`just e2e` bin run where a server capture test happened to install
/// it first.
pub(crate) fn install_permissive_global_test_subscriber() {
    use std::sync::Once;
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        struct Permissive;
        impl tracing::Subscriber for Permissive {
            fn register_callsite(
                &self,
                _metadata: &'static tracing::Metadata<'static>,
            ) -> tracing::subscriber::Interest {
                // The whole point: force every callsite to `sometimes` so the
                // global interest cache can never short-circuit `warn!`.
                tracing::subscriber::Interest::sometimes()
            }
            fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
                false
            }
            fn new_span(&self, _attrs: &tracing::span::Attributes<'_>) -> tracing::span::Id {
                tracing::span::Id::from_u64(1)
            }
            fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
            fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {
            }
            fn event(&self, _event: &tracing::Event<'_>) {}
            fn enter(&self, _span: &tracing::span::Id) {}
            fn exit(&self, _span: &tracing::span::Id) {}
        }
        // No other code sets a global default in these test binaries (main()'s
        // fmt init does not run under the harness), so this succeeds and the
        // `sometimes()` guarantee holds. HONEST FAILURE MODE: if some future
        // code (a dep, or a test that calls `fmt().init()`) installs a global
        // default first, this returns `Err`, the `Once` is still consumed, and
        // the de-flake silently no-ops — the flake would quietly return with no
        // signal. If that ever happens, this must become a fail-loud install
        // (assert the resulting global yields `sometimes()`), not a silent
        // fallback. We keep the swallow only because that precondition is
        // currently guaranteed for these binaries.
        let _ = tracing::subscriber::set_global_default(Permissive);
    });
}
