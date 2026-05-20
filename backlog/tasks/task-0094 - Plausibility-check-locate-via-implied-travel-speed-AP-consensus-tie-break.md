---
id: TASK-0094
title: Plausibility-check locate via implied travel speed + AP-consensus tie-break
status: To Do
assignee: []
created_date: '2026-05-19 06:49'
updated_date: '2026-05-19 07:06'
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
- [ ] #1 Helper haversine_kmh(lat1, lon1, lat2, lon2, dt_secs: f64) -> f64 in whereamid::geo. Returns 0.0 when dt_secs <= 0.0 (avoids div-by-zero). Unit tests: Copenhagen->Malmö in 1h ≈ 28 km/h; same point ≈ 0; zero dt -> 0; negative dt -> 0
- [ ] #2 Add --max-travel-speed-kmh <N> CLI flag, default 250, also exposed in nix/module.nix as services.whereami.maxTravelSpeedKmh
- [ ] #3 ap_consensus_score(visible_cached: &[(f64,f64)], candidate: (f64,f64)) -> Option<f64> in trilaterate.rs (or new consensus.rs). Returns None when visible_cached.len() < 3, else fraction of points within 200m haversine of candidate
- [ ] #4 Confidence enum (High|Medium|Low|Suspect) in server.rs, serde-tagged as lowercase strings on the wire; Display impl for CLI rendering
- [ ] #5 Locate response struct gains five additive fields (implied_speed_kmh, last_fix_age_secs, inlier_ratio, confidence, suspect_reason). Serde Option/default ensures old clients keep decoding new daemons; new client deserializes old daemons by treating absent fields as None / High default for confidence
- [ ] #6 handle_locate computes: implied_speed from prev_fix=last_fix table; score at current_trilat; if implied_speed > threshold then also score at prev_fix and tie-break; populates the five response fields per the design rules
- [ ] #7 When Suspect after tie-break and prev_fix wins: response lat/lon = prev_fix.lat/lon; accuracy_m = prev_fix.accuracy_m + haversine_m(prev_fix, current_trilat); suspect_reason = format!("implied {speed} km/h exceeds max {max}; tie-break favored last_fix (score {a} vs {b})")
- [ ] #8 CLI human-renderer appends confidence to locate output line; uses is-terminal crate (or std::io::IsTerminal in Rust 1.70+) to detect tty; uses nu-ansi-term for colors (already in dep tree via tracing-subscriber). Examples: '55.7065,12.5856 ±11m (8 sources, High)'; Suspect line in red+bold
- [ ] #9 Cold-boot test: empty last_fix table; locate returns confidence reflecting inlier_ratio only; implied_speed_kmh and last_fix_age_secs are None (omitted from JSON)
- [ ] #10 Speed-alarm test: prev_fix=Strandboulevarden 5 min ago, current AP cluster at Brazil; implied_speed >> 250; tie-break must pick Strandboulevarden (the Brazilian outlier dominates current_trilat's score because no other APs cluster there); response confidence=Suspect with suspect_reason populated
- [ ] #11 Legitimate-drive test: prev_fix 30 km away, 30 min ago, current_trilat consistent with current APs; implied_speed=60 km/h; no tie-break; confidence=High (assuming inlier_ratio >= 0.7)
- [ ] #12 Min-N test: only 2 visible cached APs; inlier_ratio is None, confidence is forced to Low
- [ ] #13 DSB-train documented-limitation test: prev_fix Strandboulevarden 30 min ago, visible APs cluster at Maglevænget 4 km north; implied_speed=8 km/h (under threshold); confidence=High (inlier_ratio computed for the 350m AP cluster is 1.0); test asserts this WRONG-but-not-flagged behavior and includes a doc-comment explaining that the speed check is necessary-but-not-sufficient; consensus-only mode is a follow-up
<!-- AC:END -->
