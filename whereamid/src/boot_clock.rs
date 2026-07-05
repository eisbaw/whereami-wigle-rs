//! Linux suspend-inclusive monotonic clock support.
//!
//! `std::time::Instant` is suitable for short process-local latency display,
//! but Linux does not guarantee that it advances while the machine is
//! suspended. Autonomous scan freshness needs suspend time to count, so it is
//! based on `CLOCK_BOOTTIME` instead.

use std::fmt;
use std::io;
use std::time::Duration;

/// A timestamp from Linux `CLOCK_BOOTTIME`.
///
/// The value is process-local evidence. It is never serialized or compared
/// across boots.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BootTime(Duration);

impl BootTime {
    /// Read Linux `CLOCK_BOOTTIME`, which advances across system suspend.
    pub fn now() -> io::Result<Self> {
        let mut value = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `value` points to writable storage for one `timespec`, and
        // CLOCK_BOOTTIME is a non-CPU Linux clock accepted by clock_gettime.
        let result = unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut value) };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }

        let seconds = u64::try_from(value.tv_sec).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("CLOCK_BOOTTIME returned negative seconds: {}", value.tv_sec),
            )
        })?;
        let nanoseconds = u32::try_from(value.tv_nsec)
            .ok()
            .filter(|nanoseconds| *nanoseconds < 1_000_000_000)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "CLOCK_BOOTTIME returned invalid nanoseconds: {}",
                        value.tv_nsec
                    ),
                )
            })?;
        Ok(Self(Duration::new(seconds, nanoseconds)))
    }

    /// Exact elapsed time, or `None` when `earlier` is impossibly in the
    /// future for this monotonic clock domain.
    pub fn checked_duration_since(self, earlier: Self) -> Option<Duration> {
        self.0.checked_sub(earlier.0)
    }

    /// Smallest signed millisecond generation that is not earlier than this
    /// exact instant. NetworkManager exposes `LastScan` only at millisecond
    /// precision, so flooring here could accept a generation before the
    /// refresh attempt by up to one millisecond.
    pub(crate) fn ceil_millis(self) -> Option<i64> {
        let floor = self.0.as_millis();
        let has_submillisecond = !self.0.subsec_nanos().is_multiple_of(1_000_000);
        let ceiling = floor.checked_add(u128::from(has_submillisecond))?;
        i64::try_from(ceiling).ok()
    }

    pub(crate) fn floor_millis(self) -> Option<i64> {
        i64::try_from(self.0.as_millis()).ok()
    }

    #[cfg(test)]
    pub(crate) const fn from_duration(value: Duration) -> Self {
        Self(value)
    }
}

impl fmt::Debug for BootTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("BootTime").field(&self.0).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_boottime_reads_and_orders_exact_durations() {
        let first = BootTime::now().expect("Linux CLOCK_BOOTTIME must be readable");
        let second = BootTime::now().expect("Linux CLOCK_BOOTTIME must remain readable");
        assert!(second.checked_duration_since(first).is_some());
    }

    #[test]
    fn checked_duration_rejects_future_stamp_without_saturating() {
        let now = BootTime::from_duration(Duration::from_secs(10));
        let future = BootTime::from_duration(Duration::from_secs(10) + Duration::from_nanos(1));
        assert_eq!(now.checked_duration_since(future), None);
        assert_eq!(
            future.checked_duration_since(now),
            Some(Duration::from_nanos(1))
        );
    }

    #[test]
    fn millisecond_comparison_is_conservative_at_submillisecond_boundary() {
        let exact = BootTime::from_duration(Duration::from_millis(10));
        let after_floor =
            BootTime::from_duration(Duration::from_millis(10) + Duration::from_nanos(1));

        assert_eq!(exact.ceil_millis(), Some(10));
        assert_eq!(after_floor.ceil_millis(), Some(11));
        assert_eq!(exact.floor_millis(), Some(10));
        assert_eq!(after_floor.floor_millis(), Some(10));
    }
}
