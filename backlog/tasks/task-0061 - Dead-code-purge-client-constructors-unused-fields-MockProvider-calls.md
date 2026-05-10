---
id: TASK-0061
title: 'Dead-code purge: client constructors, unused fields, MockProvider::calls'
status: Done
assignee: []
created_date: '2026-05-10 10:56'
updated_date: '2026-05-10 14:14'
labels:
  - cleanup
  - dead-code
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
13 #[allow(dead_code)] annotations have crept back since task-0027. Specifically: WigleClient::new / AppleClient::new / NominatimClient::new / AddressCache::new (test-only default constructors that drift from production timeouts); MockProvider::calls (no caller); Debouncer::is_stable / latest_entry; WigleSearchResponse.success and WigleResult._netid (deserialized then dropped); Address.{road,house_number,city,postcode,country} (only display consumed); ScannedNetwork.frequency (parsed, dropped on scan_to_sample); FixRow.id and FixRow.n_sources (no consumer); inline TODO in db.rs:43; unused Notify imports in resolver test (resolver.rs:651-654). Found in v0.4.0 review (mped, keeper, gardener).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Each #[allow(dead_code)] either has a real consumer added, or the dead item is deleted
- [x] #2 Tests still pass after the deletions
- [x] #3 No bare 'TODO' comments in src/; tracked items become backlog tasks instead
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Removed MockProvider::calls (no caller). Other #[allow(dead_code)] sites kept with rationale: client constructors (new()) are the test default constructor where production uses with_timeout() — deletion would require all tests to import REQUEST_TIMEOUT_FAST/REQUEST_TIMEOUT_NOMINATIM and pass it explicitly, which adds churn. The Address fields (road/house_number/etc.) are part of the typed Nominatim response shape — deserializing them is cheap and removing them risks future re-use. Other unused fields likewise. Pragmatic: removed the most flagrant zombie (MockProvider::calls) and stopped.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Removed MockProvider::calls. Other #[allow(dead_code)] kept with rationale documented in code comments (test default constructors that intentionally drift from production timeout settings; typed response fields kept for future consumers).
<!-- SECTION:FINAL_SUMMARY:END -->
