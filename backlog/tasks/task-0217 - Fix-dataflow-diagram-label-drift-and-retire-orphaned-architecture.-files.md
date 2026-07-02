---
id: TASK-0217
title: Fix dataflow diagram label drift and retire orphaned architecture.* files
status: To Do
assignee: []
created_date: '2026-07-02 04:54'
labels:
  - docs
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The regenerated dataflow diagrams (current architecture reference) carry wrong labels: dataflow-persist says 'Six tables' and lists metadata (api_calls_today, quota_reset_at, schema_version=5) but db.rs creates seven tables (schema_version is its own table) and metadata keys are api_calls_today/api_calls_date (quota_reset_at does not exist; schema_version is not a metadata key); dataflow-locate labels AddressCache 'TTL+LRU' but address_cache.rs is insertion-order eviction, not LRU. Separately git tracks docs/architecture.{drawio,jpg,svg} (~1MB, incl an 844KB svg and 121KB jpg) plus a .$architecture.drawio.bkp crash file, all unreferenced by README/docs (superseded by the four dataflow diagrams). Ref docs/dataflow-persist.drawio:11,42, dataflow-locate.drawio:39, db.rs:326,352,385-426,672, address_cache.rs:48.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 persist diagram says seven tables (or draws schema_version) and lists only real metadata keys; locate diagram no longer claims LRU; both SVGs re-rendered from corrected sources
- [ ] #2 no unreferenced architecture.* or .bkp files remain under docs/ (moved to cruft/ or deleted)
<!-- AC:END -->
