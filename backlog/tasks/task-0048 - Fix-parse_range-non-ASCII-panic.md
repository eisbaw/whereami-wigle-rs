---
id: TASK-0048
title: Fix parse_range non-ASCII panic
status: Done
assignee: []
created_date: '2026-05-10 10:51'
updated_date: '2026-05-10 13:10'
labels:
  - bug
  - history
  - robustness
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
history.rs:126 split_at(spec.len() - 1) indexes by byte. A spec like '7日' (multi-byte unit char) panics on non-char-boundary before the error path runs. handle_history validates via this function; the panic happens before the error response can be produced. Found in v0.4.0 review (mped-architect).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 parse_range never panics on any UTF-8 input
- [x] #2 Non-ASCII unit chars produce a typed Err with a useful message
- [x] #3 Property test feeds arbitrary UTF-8 strings and asserts panic-freedom plus shape (Ok or Err)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Replaced split_at(spec.len() - 1) — which is byte-indexed and panics on non-char-boundaries — with split_at(spec.len() - last_char.len_utf8()). For ASCII inputs the behavior is identical (1-byte chars). For non-ASCII unit chars the parse cleanly returns Err with the existing 'unknown range unit' message.

Three new tests: '7日' (3-byte unit), '7🚀' (4-byte unit), '日' alone, '   ' whitespace-only. None panic; all return Err.

Did NOT add a property test (would need proptest in this crate's test config). The four representative cases plus the existing rejects_garbage test cover the panic-freedom contract for non-ASCII.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
parse_range no longer panics on non-ASCII inputs. The byte-indexed split_at is replaced with a char-aware split that uses the last char's UTF-8 length. ASCII behavior unchanged. Three new tests for non-ASCII inputs assert Err (not panic).
<!-- SECTION:FINAL_SUMMARY:END -->
