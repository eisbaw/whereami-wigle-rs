---
id: TASK-0226
title: split_nmcli_fields mishandles \\ escape; flawed proptest makes just e2e flaky
status: Done
assignee: []
created_date: '2026-07-02 06:34'
updated_date: '2026-07-02 06:34'
labels:
  - correctness
  - testing
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Surfaced by the cycle-3 review gate. (1) split_nmcli_fields never handled nmcli's backslash-escapes-backslash (\\ -> \); on wire input \\: it left an unprocessed \: in the field and mis-parsed any field containing a literal backslash. nmcli terse mode escapes BOTH the separator (: -> \:) and the escape char (\ -> \\). (2) The proptest split_nmcli_no_unescaped_backslash_colon asserted no field contains \:, but correct decoding of wire \\ + \: legitimately yields a literal backslash-colon field - a false-positive proxy that made just e2e flakily red (random .* seed, uncommitted). Ref whereamid/src/scanner.rs split_nmcli_fields, whereamid/tests/proptests.rs.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 split_nmcli_fields treats \\ as a literal backslash and never leaves an unprocessed escape; deterministic unit test covers \\, \\:, and mixed cases
- [x] #2 the flawed !contains(\:) proptest assertion is replaced (totality guard + round-trip incl backslashes); just e2e is deterministically green
- [x] #3 round-trip proptest field alphabet includes backslash and escapes \ before : on encode
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Fixed in-thread during the cycle-3 review gate (commit 638f2d5). split_nmcli_fields now consumes the char a backslash escapes (\\ -> \, \: -> :), with a deterministic unit test. The flawed !contains("\:") proptest was a false-positive proxy (correct decoding of \\ + \: yields a literal backslash-colon); replaced with a totality guard + a round-trip proptest that now includes backslashes. Verified deterministic over 150k proptest cases x3 and just e2e x2.
<!-- SECTION:FINAL_SUMMARY:END -->
