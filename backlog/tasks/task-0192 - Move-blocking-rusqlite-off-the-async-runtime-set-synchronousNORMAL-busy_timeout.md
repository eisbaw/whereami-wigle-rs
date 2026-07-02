---
id: TASK-0192
title: >-
  Move blocking rusqlite off the async runtime; set synchronous=NORMAL +
  busy_timeout
status: To Do
assignee: []
created_date: '2026-07-02 04:51'
updated_date: '2026-07-02 23:06'
labels:
  - correctness
  - performance
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Database sits behind std::sync::Mutex and every call runs directly on tokio workers (no spawn_blocking anywhere). One locate does up to 10 touch_ap UPDATEs + set_last_fix + insert_fix ~= 12 autocommit write txns. With SQLite default synchronous=FULL each commit fsyncs the WAL (5-30ms on SBC/SD-card hardware) -> 100-350ms per locate holding the global lock, blocking every other handler's OS worker; a concurrent daily prune_fixes DELETE or inline WAL checkpoint can freeze the whole runtime incl timers and the accept loop. No busy_timeout is set (external sqlite writer -> instant SQLITE_BUSY, data silently dropped). try_reserve_api_call doc claims IMMEDIATE but uses a DEFERRED transaction. Ref whereamid/src/server.rs:74-78, resolver.rs:141-147, db.rs:104-123,656-667,888-894.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 no rusqlite call executes on a tokio worker thread (spawn_blocking or a dedicated DB thread)
- [ ] #2 a locate touching 10 cached APs performs at most 1-2 write transactions
- [ ] #3 PRAGMA synchronous=NORMAL and a busy_timeout are set; concurrent stats/version stay responsive under a slowed DB or large prune
- [ ] #4 try_reserve_api_call uses (or documents) the correct transaction mode
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from task-0197: db.rs Database::open() now calls tighten_db_file_permissions(path) (chmod DB file to 0600) IMMEDIATELY after Connection::open and BEFORE the journal_mode=WAL pragma. This ordering is load-bearing: SQLite copies the main DB files perms onto the -wal/-shm sidecars it creates on WAL activation, so the chmod must precede WAL. When you add synchronous=NORMAL + busy_timeout pragmas (and/or move rusqlite onto spawn_blocking), keep the tighten call before the journal_mode pragma. There is an in-code NOTE(task-0192) marking this constraint at the call site.
<!-- SECTION:NOTES:END -->
