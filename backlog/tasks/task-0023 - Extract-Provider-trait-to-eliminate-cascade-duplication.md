---
id: TASK-0023
title: Extract Provider trait to eliminate cascade duplication
status: To Do
assignee: []
created_date: '2026-05-09 21:07'
labels: []
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
resolve_for_locate, resolve_readonly, resolve_background, and pending::drain_once all reimplement the same Apple->WiGLE cascade. Extract a Provider trait with async fn lookup(&self, bssid) -> Result<Option<ApInfo>, ProviderError> and a provider chain. Resolve functions differ only in config (which providers, write-through vs read-only, pending behavior), not control flow.
<!-- SECTION:DESCRIPTION:END -->
