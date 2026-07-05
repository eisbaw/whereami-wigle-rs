---
id: TASK-0094
title: Plausibility-check locate via implied travel speed + AP-consensus tie-break
status: Done
assignee:
  - task0094-implementer
created_date: '2026-05-19 06:49'
updated_date: '2026-07-05 13:42'
labels:
  - correctness
  - observability
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cached-AP trilateration silently assumes every AP is geographically fixed. Mobile APs (trains, buses, hotspots) violate this and cause systematic mis-localization without any error signal. Real incident: user at Copenhagen Central Station, daemon reports Maglevænget (7 km off, train Wi-Fi cached positions). This task adds a principled plausibility-check that does not depend on hand-curated mobile-AP taxonomy.

DESIGN (pinned, no more 'e.g.' wiggle):

(1) prev_fix is the last_fix table row (not history segments, not anything else). If last_fix is NULL (cold boot), the speed check is skipped and reported as not-applicable.

(2) implied_speed_kmh = haversine_kmh(prev_fix.lat,prev_fix.lon -> current_trilat.lat,current_trilat.lon, now - prev_fix.at_rfc3339). If now == prev_fix.at, treat as 0 km/h (do not divide by zero).

(3) max_travel_speed_kmh: configurable, default 250 (covers high-speed rail and motorway, rules out flight). When implied_speed > max, confidence is downgraded to Suspect regardless of inlier_ratio.

(4) AP-consensus score (single definition, no in-between dead zone):
    score(candidate_latlon) = |{ visible APs with cached lat/lon within INLIER_RADIUS_M of candidate }| / max(1, |visible cached APs|)
    INLIER_RADIUS_M = 200. Score is in [0, 1].
    'visible cached APs' = APs from the current debouncer snapshot that have a cached lat/lon (excluding mobile-flagged from task-0093 if that landed first). If fewer than MIN_CONSENSUS_N=3 cached APs are visible, score is undefined and confidence is forced to Low.

(5) Tie-break (only runs when implied_speed > max): compare score(current_trilat) vs score(prev_fix). Whichever is higher wins; if tied, prefer current_trilat (the daemon's best guess). Loser is logged via tracing::warn with both scores. Reported position is the winner; reported accuracy is the trilateration accuracy if winner==current_trilat, else prev_fix.accuracy_m + (implied distance) as a coarse drift bound.

(6) prev_fix is used literally — no drift extrapolation. Future task may model assumed velocity.

(7) Confidence policy (hardcoded constants for now, may become configurable):
      score >= 0.7 -> High
      0.4 <= score < 0.7 -> Medium
      score < 0.4 -> Low
      score undefined (cached count < 3) -> Low
      implied_speed > max (after tie-break) -> Suspect

(8) Always populate uncertainty fields in the locate response, on every call:
      implied_speed_kmh: Option<f64>      // None on cold boot
      last_fix_age_secs: Option<i64>      // None on cold boot
      inlier_ratio: Option<f64>           // None when cached count < 3
      confidence: Confidence              // High|Medium|Low|Suspect (always present)
      suspect_reason: Option<String>      // present iff confidence==Suspect
    Old daemons emit nothing here; client deserializes with #[serde(default)] / Option so older daemons remain compatible.

(9) CLI human-renderer: append confidence to the existing line, color-coded via the existing tracing-subscriber color dep (nu-ansi-term) when stdout is a tty. Suspect -> red+bold; Low -> yellow; Medium -> default; High -> dim green. is_terminal=false (pipe/redirect) -> no color codes.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Helper haversine_kmh(lat1, lon1, lat2, lon2, dt_secs: f64) -> f64 in whereamid::geo. Returns 0.0 when dt_secs <= 0.0 (avoids div-by-zero). Unit tests: Copenhagen->Malmö in 1h ≈ 28 km/h; same point ≈ 0; zero dt -> 0; negative dt -> 0
- [x] #2 Add --max-travel-speed-kmh <N> CLI flag, default 250, also exposed in nix/module.nix as services.whereami.maxTravelSpeedKmh
- [x] #3 ap_consensus_score(visible_cached: &[(f64,f64)], candidate: (f64,f64)) -> Option<f64> in trilaterate.rs (or new consensus.rs). Returns None when visible_cached.len() < 3, else fraction of points within 200m haversine of candidate
- [x] #4 Confidence enum (High|Medium|Low|Suspect) in server.rs, serde-tagged as lowercase strings on the wire; Display impl for CLI rendering
- [x] #5 Locate response struct gains five additive fields (implied_speed_kmh, last_fix_age_secs, inlier_ratio, confidence, suspect_reason). Serde Option/default ensures old clients keep decoding new daemons; new client deserializes old daemons by treating absent fields as None / High default for confidence
- [x] #6 handle_locate computes: implied_speed from prev_fix=last_fix table; score at current_trilat; if implied_speed > threshold then also score at prev_fix and tie-break; populates the five response fields per the design rules
- [x] #7 When Suspect after tie-break and prev_fix wins: response lat/lon = prev_fix.lat/lon; accuracy_m = prev_fix.accuracy_m + haversine_m(prev_fix, current_trilat); suspect_reason = format!("implied {speed} km/h exceeds max {max}; tie-break favored last_fix (score {a} vs {b})")
- [x] #8 CLI human-renderer appends confidence to locate output line; uses is-terminal crate (or std::io::IsTerminal in Rust 1.70+) to detect tty; uses nu-ansi-term for colors (already in dep tree via tracing-subscriber). Examples: '55.7065,12.5856 ±11m (8 sources, High)'; Suspect line in red+bold
- [x] #9 Cold-boot test: empty last_fix table; locate returns confidence reflecting inlier_ratio only; implied_speed_kmh and last_fix_age_secs are None (omitted from JSON)
- [x] #10 Speed-alarm test: prev_fix=Strandboulevarden 5 min ago, current AP cluster at Brazil; implied_speed >> 250; tie-break must pick Strandboulevarden (the Brazilian outlier dominates current_trilat's score because no other APs cluster there); response confidence=Suspect with suspect_reason populated
- [x] #11 Legitimate-drive test: prev_fix 30 km away, 30 min ago, current_trilat consistent with current APs; implied_speed=60 km/h; no tie-break; confidence=High (assuming inlier_ratio >= 0.7)
- [x] #12 Min-N test: only 2 visible cached APs; inlier_ratio is None, confidence is forced to Low
- [x] #13 DSB-train documented-limitation test: prev_fix Strandboulevarden 30 min ago, visible APs cluster at Maglevænget 4 km north; implied_speed=8 km/h (under threshold); confidence=High (inlier_ratio computed for the 350m AP cluster is 1.0); test asserts this WRONG-but-not-flagged behavior and includes a doc-comment explaining that the speed check is necessary-but-not-sufficient; consensus-only mode is a follow-up
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add the geographic/consensus/config/wire primitives with boundary tests: haversine_kmh, inclusive 200m consensus, finite-positive max speed, mirrored lowercase Confidence enums, and backward-compatible optional fields.
2. Refactor handle_locate around one latest-snapshot cache lookup that derives two explicit sets: all visible fixed/finite cached coordinates for consensus, and the existing stable-or-fallback fixed/finite top-N solver inputs. Serialize previous-row read, clock capture, decision, and any current-winner persistence under the existing privacy and last_fix lock order.
3. Implement the pinned speed/score policy and real dispatch regressions for cold boot, previous-winner alarm, legitimate drive, min-N, the documented train limitation, threshold/tie/current-winner edges, stale fallback, and selected metadata.
4. Extend the typed client, TTY-aware human rendering, NixOS/Home Manager shared option/emission, packaged flag/default checks, and protocol/user docs.
5. Run focused tests, formatting/lint, Nix flag-parity and option-bounds checks, full just e2e, mutation checks, freeze the diff, pass parallel QA/architecture review, then close and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Phase 3 pre-implementation audit pinned underspecified interactions with already-landed TASK-0093 and the robust outlier filter. Decisions: maxTravelSpeedKmh is a finite positive f64/Nix positive number, default 250; equality is not suspect. Consensus includes each unique latest_scan BSSID only when current SSID is not denylisted, a cache row exists, persisted is_mobile is false, and coordinates are finite; stability, RSSI, top_n, and trilateration outlier survival never affect this denominator. The cache is queried once for the full scanner-eligible latest snapshot, then solver membership is selected separately to preserve TASK-0093 semantics and the two-commit locate boundary. The 200m edge is inclusive. Capture a high-resolution signed duration and one Utc::now only after acquiring the last_fix serialization guard; clamp reported future-clock age to zero and warn, while haversine_kmh returns zero for non-positive duration. Below/equal threshold, confidence follows winner/current score (>=.7 High, >=.4 Medium, else Low; undefined Low). Above threshold, confidence is Suspect regardless of defined score; undefined scores tie and current wins. inlier_ratio describes the reported winner. Reasons use one decimal speed, winner-first two-decimal scores (or n/a), and name last_fix/current_trilat. The loser is warned with both raw scores. A current winner is persisted normally. A last_fix winner preserves its address and returns prev accuracy plus candidate distance, but is not rewritten to last_fix/history: the row is already the literal previous winner, rewriting would invent a fresh observation and repeatedly compound the drift bound. Such a policy-selected response remains stale=false/age_s omitted; cached/sources retain current solve semantics. Existing stale fallback emits implied_speed=None, last_fix_age_secs=age, inlier_ratio=None, Low, no reason. Invalid persisted accuracy joins invalid coordinates at rehydration rejection. Server/client mirror Confidence because no shared protocol crate exists; client missing confidence defaults High as explicitly required. Color only the coordinate line, only on TTY; raw JSON stays byte-preserving. AC10 uses top_n=1 with one strong Brazilian solver row plus four weaker visible previous-location rows, proving the full-visible denominator. AC13 uses hidden/unrecognized train SSIDs that evade the coarse TASK-0093 taxonomy and documents that necessary-but-insufficient boundary.

First frozen review gate (code f94d5729460f91b72d71da7a6fe4e75bdceccf472f848254d7be9c10cec64d1e, combined 701d0c9f4859cde59774ee54da88e4296236785691f520e00b7b6732c3808ccf): mandatory QA and architecture NO-GO. Blocker 1: AC13 claimed a 350m train AP cluster but the fixture span was only 2.552m, trivially yielding consensus 1.0; correction must place endpoints about +/-175m around Maglevænget, assert measured span near 350m, and retain the wrong-but-High behavior. Blocker 2: signed_elapsed_seconds converted through integer microseconds, so +/-500ns truncated to zero, suppressing both a real speed calculation and future-clock warning; use Chrono fractional seconds and pin positive/negative nanosecond signs. QA otherwise passed exact e2e (592), fmt, both Nix checks, 10x scenario loops, hashes/index. Architecture otherwise accepted snapshot separation, filtering/top_n, lock order, non-rewrite policy, wire/TTY/Nix paths. Correction pass will also tighten the legitimate-drive speed tolerance and document that new confidence vocabulary requires a protocol-version bump; warning capture and policy extraction remain nonblocking follow-up debt.

Completion after correction re-review: Added finite-positive configurable max travel speed across CLI and shared NixOS/Home Manager modules; nanosecond-preserving implied-speed math; independent full-visible cached-AP consensus; serialized persisted-last-fix plausibility selection; additive confidence/uncertainty wire fields; backward-compatible typed client; TTY-only confidence styling; and protocol/user documentation. Real dispatch regressions cover cold boot, full-visible Brazil alarm and previous winner, current Suspect winner persistence/address/history, legitimate drive, min-N, a measured 350m wrong-but-High train limitation, stale fallback, threshold/tie/clock boundaries, and prior mobile/non-finite/top_n behavior. Final correction hash code 9789d7ae0a750b884921c73395aca7c44f89494e9abd56bb37763a689797eba6 / combined 4c48971558cf218243319ca83a946618d592eae04f6a1099b35d0cefb11f83c3. Mandatory QA re-review GO: all 13 ACs, corrected scenarios 10/10, exact e2e 593 plus strict Clippy, fmt, flag-parity, option-bounds, hashes/index. Mandatory architecture re-review GO: both prior blockers resolved; snapshot/filter/top_n, privacy-last_fix-DB ordering, non-rewrite winner policy, protocol/TTY/Nix all coherent. Mutations proved top_n-limited consensus, exclusive threshold, microsecond truncation, and tight train geometry each fail their intended regressions. Nonblocking follow-up debt: trace-capture the tie-break WARN and eventually extract pure plausibility policy/tests from oversized server.rs.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Implemented travel-speed plausibility and full-visible AP consensus tie-breaking end to end. Locate now reports additive confidence/uncertainty fields, rejects implausible jumps through a documented winner policy, preserves prior mobile/top-N/privacy semantics, renders TTY confidence safely, and exposes a validated shared Nix speed option. All 13 ACs passed mandatory QA and architecture re-review, 593 E2E tests, strict Clippy, packaged flag/default parity, option bounds, repeated boundary tests, and mutation checks.
<!-- SECTION:FINAL_SUMMARY:END -->
