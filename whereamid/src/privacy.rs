//! Linearization boundary for privacy-bearing daemon state.
//!
//! Long-running scans and provider requests carry an epoch ticket without
//! holding the gate. Immediately before publishing a result they acquire a
//! read lease and revalidate that ticket. A wipe owns the write lease, so it
//! can atomically clear managed in-memory state after SQLite commits and then
//! invalidate every ticket captured during that complete commit/clear window.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PrivacyTicket(u64);

#[derive(Clone)]
pub(crate) struct PrivacyCoordinator {
    epoch: Arc<AtomicU64>,
    gate: Arc<RwLock<()>>,
}

pub(crate) struct PrivacyReadLease {
    _guard: OwnedRwLockReadGuard<()>,
}

pub(crate) struct PrivacyWipeLease {
    coordinator: PrivacyCoordinator,
    _guard: OwnedRwLockWriteGuard<()>,
    advanced: bool,
}

impl PrivacyCoordinator {
    pub(crate) fn new() -> Self {
        Self {
            epoch: Arc::new(AtomicU64::new(0)),
            gate: Arc::new(RwLock::new(())),
        }
    }

    /// Capture the current publication epoch before starting long-running work.
    pub(crate) fn ticket(&self) -> PrivacyTicket {
        PrivacyTicket(self.epoch.load(Ordering::Acquire))
    }

    /// Acquire publication admission and reject a ticket invalidated by a wipe.
    ///
    /// The epoch is checked only after the fair read lock is acquired. A writer
    /// already waiting therefore wins first, advances the epoch, and makes this
    /// publication stale instead of letting readers starve the wipe.
    pub(crate) async fn read(&self, ticket: PrivacyTicket) -> Option<PrivacyReadLease> {
        let guard = Arc::clone(&self.gate).read_owned().await;
        if self.epoch.load(Ordering::Acquire) != ticket.0 {
            return None;
        }
        Some(PrivacyReadLease { _guard: guard })
    }

    /// Test-only read that reports the exact first `Poll::Pending` from the
    /// underlying gate future. The callback therefore proves the reader is
    /// registered behind a live writer rather than merely spawned or about to
    /// await it.
    #[cfg(test)]
    pub(crate) async fn read_observing_wait<H>(
        &self,
        ticket: PrivacyTicket,
        waiting: H,
    ) -> Option<PrivacyReadLease>
    where
        H: FnOnce(),
    {
        let mut read = Box::pin(Arc::clone(&self.gate).read_owned());
        let mut waiting = Some(waiting);
        let guard = std::future::poll_fn(|cx| match std::future::Future::poll(read.as_mut(), cx) {
            std::task::Poll::Pending => {
                if let Some(waiting) = waiting.take() {
                    waiting();
                }
                std::task::Poll::Pending
            }
            std::task::Poll::Ready(guard) => std::task::Poll::Ready(guard),
        })
        .await;
        if self.epoch.load(Ordering::Acquire) != ticket.0 {
            return None;
        }
        Some(PrivacyReadLease { _guard: guard })
    }

    /// Begin an exclusive wipe boundary. The epoch is deliberately unchanged
    /// until the database transaction commits and managed memory is cleared;
    /// a failed wipe must leave both persisted and managed state unchanged.
    pub(crate) async fn begin_wipe(&self) -> PrivacyWipeLease {
        let guard = Arc::clone(&self.gate).write_owned().await;
        PrivacyWipeLease {
            coordinator: self.clone(),
            _guard: guard,
            advanced: false,
        }
    }
}

impl PrivacyWipeLease {
    /// Invalidate every ticket captured before the committed wipe.
    ///
    /// Must be called exactly once, after SQLite COMMIT and after clearing all
    /// managed memory. Keeping the old epoch throughout both phases ensures a
    /// lock-free ticket captured mid-wipe cannot validate against pre-clear
    /// state. Exhausting 2^64 successful wipes is not a recoverable runtime
    /// condition; wrapping would make ancient tickets current again. The
    /// production wipe invokes this inside the database actor's fail-stop
    /// continuation, so either assertion terminates the process before it can
    /// resume serving committed disk state under an invalid epoch.
    pub(crate) fn advance_epoch(&mut self) -> PrivacyTicket {
        assert!(!self.advanced, "privacy wipe epoch advanced twice");
        let next = self
            .coordinator
            .epoch
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(1)
            })
            .expect("privacy wipe epoch exhausted");
        self.advanced = true;
        PrivacyTicket(next + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn committed_wipe_invalidates_old_tickets_only_after_advance() {
        let coordinator = PrivacyCoordinator::new();
        let old = coordinator.ticket();
        let mut wipe = coordinator.begin_wipe().await;
        assert_eq!(coordinator.ticket(), old);
        let new = wipe.advance_epoch();
        drop(wipe);

        assert!(coordinator.read(old).await.is_none());
        assert!(coordinator.read(new).await.is_some());
    }

    #[tokio::test]
    async fn failed_wipe_drop_leaves_ticket_current() {
        let coordinator = PrivacyCoordinator::new();
        let ticket = coordinator.ticket();
        drop(coordinator.begin_wipe().await);
        assert!(coordinator.read(ticket).await.is_some());
    }
}
