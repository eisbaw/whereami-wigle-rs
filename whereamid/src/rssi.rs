//! Typed Wi-Fi signal strength with measurement provenance.
//!
//! Two scanner backends deliver a signal that both ultimately reach
//! trilateration as a dBm value, but they are **not the same physical
//! quantity measured the same way**:
//!
//! - **iw** reports a raw, driver-supplied dBm — the authoritative source.
//!   `Rssi::from_iw_dbm` is the identity: the integer already *is* dBm.
//! - **nmcli** reports a 0–100 percentage that NetworkManager derived from
//!   RSSI via a non-linear, driver-dependent internal formula. We recover an
//!   approximate dBm with the coarse linear map in `Rssi::from_nmcli_percent`.
//!   This mapping is a **systematically biased approximation**: positions
//!   trilaterated purely from nmcli scans have skewed vote weights relative to
//!   iw-sourced ones.
//!
//! The bare `i32 signal_dbm` that used to flow to trilateration erased this
//! distinction — a future reader could not tell an authoritative reading from
//! a linearized guess. [`Rssi`] carries the dBm value; [`RssiSource`] records
//! which backend produced it so provenance survives to the point where votes
//! are weighted.
//!
//! **Future capability (not implemented here):** because the source travels
//! alongside the value, trilateration *may* later choose to down-weight
//! nmcli-linearized APs, or refuse to mix the two sources in one fix. This
//! module deliberately does NOT change any weighting behavior today — the
//! vote-weight math is identical for both sources. It only makes the
//! information available. See task-0126.

/// Which scanner backend produced a signal reading, and therefore how much to
/// trust the dBm value carried by an [`Rssi`].
///
/// - [`RssiSource::IwDbm`]: raw dBm straight from `iw` — authoritative.
/// - [`RssiSource::NmcliPercentLinearized`]: an approximate dBm recovered from
///   nmcli's 0–100 percentage by [`Rssi::from_nmcli_percent`]. Carries a known
///   systematic bias; never treat it as a calibrated measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RssiSource {
    /// Raw driver dBm from `iw`. Authoritative.
    IwDbm,
    /// Approximate dBm linearized from nmcli's 0–100 percentage. Biased.
    NmcliPercentLinearized,
}

/// Map the daemon-internal [`RssiSource`] to its wire mirror
/// [`whereami_protocol::RssiSource`] at the `debug`/`scan` wire boundary
/// (task-0271). The internal enum stays here (it is used widely across the
/// scanner/debounce/trilateration paths); only the wire surface needs the
/// protocol type, so we map at the boundary rather than moving the enum.
impl From<RssiSource> for whereami_protocol::RssiSource {
    fn from(source: RssiSource) -> Self {
        match source {
            RssiSource::IwDbm => whereami_protocol::RssiSource::Iw,
            RssiSource::NmcliPercentLinearized => whereami_protocol::RssiSource::Nmcli,
        }
    }
}

/// A Wi-Fi signal strength in dBm.
///
/// `#[repr(transparent)]` over a single `i32`, so it has the exact ABI and
/// size of the dBm integer it replaces — this is a zero-cost type-safety
/// wrapper, not a new data layout. The measurement's provenance is tracked
/// **alongside** it as an [`RssiSource`] (a transparent newtype cannot also
/// hold the source without ceasing to be transparent), typically on the
/// carrying struct such as `PositionedAp`.
///
/// Construct via the source-specific constructors — [`Rssi::from_iw_dbm`] for
/// authoritative iw readings, [`Rssi::from_nmcli_percent`] for nmcli's
/// linearized-percentage approximation — so the biased mapping lives in one
/// place. Read the raw dBm back with [`Rssi::dbm`] at wire/DB boundaries where
/// an `i32` is still required.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rssi(i32);

impl Rssi {
    /// Wrap a raw dBm value reported by `iw`. Identity: `iw` already emits
    /// dBm, so there is no conversion — this constructor only records intent.
    ///
    /// Also used at the trilateration boundary to re-wrap an already-resolved
    /// dBm read back from the cache/debounce path (whose original provenance
    /// is carried separately as an [`RssiSource`]); the vote-weight math is
    /// source-agnostic, so this identity wrap changes no numeric result.
    pub fn from_iw_dbm(dbm: i32) -> Self {
        Rssi(dbm)
    }

    /// Convert nmcli's signal percentage (0–100) to an approximate dBm.
    ///
    /// `100 -> -30 dBm`, `0 -> -90 dBm` via the linear map `-90 + pct*60/100`.
    /// This is the single home of that mapping (moved here from the inline
    /// computation in the nmcli scanner parser). It is a coarse approximation:
    /// nmcli's percentage is itself derived from RSSI by a non-linear,
    /// driver-dependent formula inside NetworkManager, so the recovered dBm is
    /// systematically biased. Pair with [`RssiSource::NmcliPercentLinearized`].
    pub fn from_nmcli_percent(percent: i32) -> Self {
        Rssi(-90 + ((percent as f64 * 60.0 / 100.0) as i32))
    }

    /// The raw dBm value. Use at wire/DB boundaries that still require an
    /// `i32`, and inside the vote-weight exponent.
    pub fn dbm(&self) -> i32 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the nmcli linearization that was MOVED here from the scanner
    /// (scanner.rs). If this mapping ever changes, that is a behavior change to
    /// nmcli-sourced weights and must be deliberate.
    #[test]
    fn nmcli_percent_endpoints_pin_the_linearization() {
        assert_eq!(Rssi::from_nmcli_percent(100).dbm(), -30);
        assert_eq!(Rssi::from_nmcli_percent(0).dbm(), -90);
    }

    /// Midpoint sanity: 50% -> -60 dBm under `-90 + 50*60/100`.
    #[test]
    fn nmcli_percent_midpoint() {
        assert_eq!(Rssi::from_nmcli_percent(50).dbm(), -60);
    }

    #[test]
    fn from_iw_dbm_is_identity() {
        assert_eq!(Rssi::from_iw_dbm(-67).dbm(), -67);
        assert_eq!(Rssi::from_iw_dbm(0).dbm(), 0);
    }
}
