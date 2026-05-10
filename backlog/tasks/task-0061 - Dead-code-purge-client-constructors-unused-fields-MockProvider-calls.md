---
id: TASK-0061
title: 'Dead-code purge: client constructors, unused fields, MockProvider::calls'
status: To Do
assignee: []
created_date: '2026-05-10 10:56'
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
- [ ] #1 Each #[allow(dead_code)] either has a real consumer added, or the dead item is deleted
- [ ] #2 Tests still pass after the deletions
- [ ] #3 No bare 'TODO' comments in src/; tracked items become backlog tasks instead
<!-- AC:END -->
