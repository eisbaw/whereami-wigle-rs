//! In-memory cache mapping rounded (lat, lon) → reverse-geocoded address.
//!
//! Populated lazily by the locate hot path so the next call at the same
//! grid cell gets the address for free without blocking on Nominatim's
//! 1 req/s rate limit. Entries expire after a configurable TTL because
//! Nominatim data drifts (renamed streets, closed businesses, expanded
//! coverage). task-0056 extracted this into its own submodule.

/// Key precision: 4 decimal degrees ~ 11 m at the equator. Locate fixes
/// drift by more than that scan-to-scan, so we don't gain anything from
/// finer keys. Coarser keys would alias adjacent buildings.
const ADDRESS_CACHE_DECIMALS: i32 = 4;
/// Capacity is a soft bound; eviction is naive (drop oldest entry by
/// insertion order) because the access pattern is "small set of locations
/// you actually visit". A real LRU would be overkill for one user.
const ADDRESS_CACHE_CAP: usize = 256;
/// Default TTL for an address-cache entry. CLI-configurable via
/// --address-cache-ttl-days.
pub const ADDRESS_CACHE_TTL_DAYS_DEFAULT: i64 = 7;

/// Round (lat, lon) to a fixed-precision integer key. We use
/// `(i32, i32)` rather than floats so equality is bit-exact and we
/// don't have to worry about NaN comparisons.
///
/// Returns `None` on any non-finite input (NaN / ±Inf) — this is an
/// explicit fail-closed guard, not a coincidence of the cast (task-0106).
/// `f64 as i32` *saturates* non-finite values to 0, which would silently
/// alias every NaN/Inf position to the (0, 0) grid cell and cache a bogus
/// address there; the previous `debug_assert!` that was meant to catch
/// this is compiled out in release. Callers (`get`/`insert`) treat `None`
/// as a miss / no-op so a bad float can never mutate or read cache state.
///
/// For finite inputs the key is always in range: lat ∈ [-90, 90],
/// lon ∈ [-180, 180]; with scale = 10^4 the absolute value is ≤ 1.8e6,
/// well within i32.
pub(crate) fn address_cache_key(lat: f64, lon: f64) -> Option<(i32, i32)> {
    if !lat.is_finite() || !lon.is_finite() {
        return None;
    }
    let scale = 10f64.powi(ADDRESS_CACHE_DECIMALS);
    Some(((lat * scale).round() as i32, (lon * scale).round() as i32))
}

struct AddressCacheEntry {
    address: String,
    inserted_at: chrono::DateTime<chrono::Utc>,
}

/// Tiny bounded cache mapping rounded (lat, lon) -> resolved address.
/// `order` tracks insertion order so we can evict the oldest entry when
/// capacity is exceeded. Not LRU; access doesn't promote.
///
/// Entries also expire after `ttl_days`; the TTL is checked on read so
/// expired entries are reported as misses.
pub struct AddressCache {
    map: std::collections::HashMap<(i32, i32), AddressCacheEntry>,
    order: std::collections::VecDeque<(i32, i32)>,
    ttl_days: i64,
}

impl AddressCache {
    /// Create an address cache with the default TTL. Production code uses
    /// `with_ttl_days` to honour --address-cache-ttl-days; this exists for
    /// tests and external consumers.
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::with_ttl_days(ADDRESS_CACHE_TTL_DAYS_DEFAULT)
    }

    pub fn with_ttl_days(ttl_days: i64) -> Self {
        Self {
            map: std::collections::HashMap::new(),
            order: std::collections::VecDeque::new(),
            ttl_days,
        }
    }

    pub fn get(&self, lat: f64, lon: f64) -> Option<String> {
        // Non-finite (lat, lon) has no key — treat as a miss (task-0106).
        let entry = self.map.get(&address_cache_key(lat, lon)?)?;
        let age_days = (chrono::Utc::now() - entry.inserted_at).num_days();
        if age_days >= self.ttl_days {
            return None;
        }
        Some(entry.address.clone())
    }

    pub fn insert(&mut self, lat: f64, lon: f64, addr: String) {
        // Non-finite (lat, lon) has no key — no-op rather than aliasing the
        // bogus position onto some grid cell (task-0106).
        let Some(key) = address_cache_key(lat, lon) else {
            return;
        };
        let entry = AddressCacheEntry {
            address: addr,
            inserted_at: chrono::Utc::now(),
        };
        if self.map.insert(key, entry).is_none() {
            self.order.push_back(key);
            while self.order.len() > ADDRESS_CACHE_CAP {
                if let Some(oldest) = self.order.pop_front() {
                    self.map.remove(&oldest);
                }
            }
        }
    }

    /// Remove all rounded positions and addresses without changing the
    /// configured TTL. The privacy wipe calls this under its exclusive gate.
    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two positions within the same ~10m grid cell must collide on the
    /// rounded key. The 4-decimal scale corresponds to ~11m at the equator;
    /// this catches the symmetric round-half-to-even behaviour that
    /// `round() as i32` performs.
    #[test]
    fn address_cache_key_rounds_to_grid() {
        let a = address_cache_key(55.668412, 12.554123).unwrap();
        let b = address_cache_key(55.668415, 12.554118).unwrap(); // sub-meter offset
        assert_eq!(a, b, "neighbouring sub-meter positions must share a cell");

        let c = address_cache_key(55.6684, 12.5541).unwrap();
        let d = address_cache_key(55.6700, 12.5541).unwrap();
        assert_ne!(c, d, "positions ~150m apart must NOT share a cell");
    }

    /// task-0106: the key helper fails closed (returns `None`) on any
    /// non-finite coordinate instead of saturating the `f64 as i32` cast to
    /// the (0, 0) cell. This is the release-mode guard that replaced the
    /// compiled-out `debug_assert!`.
    #[test]
    fn address_cache_key_none_on_non_finite() {
        assert_eq!(address_cache_key(f64::NAN, 12.5), None);
        assert_eq!(address_cache_key(55.6, f64::NAN), None);
        assert_eq!(address_cache_key(f64::INFINITY, 12.5), None);
        assert_eq!(address_cache_key(55.6, f64::NEG_INFINITY), None);
        // Sanity: finite input still yields a key.
        assert!(address_cache_key(55.6, 12.5).is_some());
    }

    /// task-0106 AC#3: a non-finite `get`/`insert` must be inert — `get`
    /// returns `None` and neither call may mutate `map` or `order`. Guards
    /// against a future NaN/Inf coordinate silently reading or writing the
    /// saturated (0, 0) cell.
    #[test]
    fn address_cache_non_finite_is_inert() {
        let mut cache = AddressCache::new();
        // Seed one real entry so we can prove the length is untouched.
        cache.insert(55.6684, 12.5541, "Copenhagen".to_string());
        assert_eq!(cache.map.len(), 1);
        assert_eq!(cache.order.len(), 1);

        // get with non-finite coords: miss, no state change.
        assert!(cache.get(f64::NAN, f64::NAN).is_none());
        assert!(cache.get(55.6684, f64::INFINITY).is_none());

        // insert with non-finite coords: no-op, no state change.
        cache.insert(f64::NAN, f64::NAN, "garbage".to_string());
        cache.insert(55.6684, f64::NEG_INFINITY, "garbage".to_string());
        cache.insert(f64::INFINITY, 12.5541, "garbage".to_string());

        assert_eq!(
            cache.map.len(),
            1,
            "non-finite insert must not grow the map"
        );
        assert_eq!(
            cache.order.len(),
            1,
            "non-finite insert must not grow the order queue"
        );
        // The real entry is still readable and unclobbered.
        assert_eq!(
            cache.get(55.6684, 12.5541).as_deref(),
            Some("Copenhagen"),
            "finite entry must survive intervening non-finite calls"
        );
    }

    #[test]
    fn address_cache_get_insert_and_eviction() {
        let mut cache = AddressCache::new();
        assert!(cache.get(0.0, 0.0).is_none());

        cache.insert(55.6684, 12.5541, "Copenhagen".to_string());
        assert_eq!(cache.get(55.6684, 12.5541).as_deref(), Some("Copenhagen"));

        // Fill past capacity and verify size stays bounded.
        for i in 0..(ADDRESS_CACHE_CAP as i32 + 50) {
            // shift each insertion into a distinct grid cell
            let lat = (i as f64) * 0.001 + 60.0;
            cache.insert(lat, 12.0, format!("addr-{i}"));
        }
        assert!(
            cache.map.len() <= ADDRESS_CACHE_CAP,
            "cache must respect capacity, got {}",
            cache.map.len()
        );
        assert!(
            cache.order.len() <= ADDRESS_CACHE_CAP,
            "order must respect capacity"
        );
    }

    #[test]
    fn clear_removes_map_and_eviction_order() {
        let mut cache = AddressCache::new();
        cache.insert(55.6684, 12.5541, "Copenhagen".to_string());
        cache.clear();
        assert!(cache.map.is_empty());
        assert!(cache.order.is_empty());
        assert!(cache.get(55.6684, 12.5541).is_none());
    }

    /// TTL: an entry whose inserted_at is older than ttl_days reads back
    /// as None.
    #[test]
    fn address_cache_expires_after_ttl() {
        let mut cache = AddressCache::with_ttl_days(7);
        cache.insert(55.6684, 12.5541, "Copenhagen".to_string());
        assert_eq!(cache.get(55.6684, 12.5541).as_deref(), Some("Copenhagen"));

        let key = address_cache_key(55.6684, 12.5541).unwrap();
        cache.map.get_mut(&key).unwrap().inserted_at =
            chrono::Utc::now() - chrono::TimeDelta::days(8);

        assert!(
            cache.get(55.6684, 12.5541).is_none(),
            "entry older than TTL must read back as None"
        );
    }

    /// task-0162: the TTL comparison is `age_days >= ttl_days`, so an entry
    /// aged EXACTLY `ttl_days` is already expired (reads back None) — not one
    /// tick later. Setting `inserted_at = now - ttl_days` makes the read-side
    /// `now2 - inserted_at` equal ttl_days plus a sub-day epsilon, which
    /// `num_days()` truncates back to exactly ttl_days. This pins the `>=`
    /// boundary: a regression to `>` would return Some here. Deterministic
    /// (no sleep) via the `inserted_at` seam the sibling TTL tests use.
    #[test]
    fn address_cache_expires_exactly_at_ttl_boundary() {
        const TTL: i64 = 7;
        let mut cache = AddressCache::with_ttl_days(TTL);
        cache.insert(55.6684, 12.5541, "Copenhagen".to_string());

        let key = address_cache_key(55.6684, 12.5541).unwrap();
        cache.map.get_mut(&key).unwrap().inserted_at =
            chrono::Utc::now() - chrono::TimeDelta::days(TTL);

        assert!(
            cache.get(55.6684, 12.5541).is_none(),
            "entry aged exactly ttl_days must be expired (>= boundary)"
        );
    }

    /// TTL of 0 means every read is a miss (useful for disabling).
    #[test]
    fn address_cache_zero_ttl_always_misses() {
        let mut cache = AddressCache::with_ttl_days(0);
        cache.insert(0.0, 0.0, "anywhere".to_string());
        assert!(
            cache.get(0.0, 0.0).is_none(),
            "ttl_days=0 must produce always-miss behaviour"
        );
    }
}
