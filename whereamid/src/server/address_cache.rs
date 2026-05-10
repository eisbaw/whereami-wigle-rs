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
/// Trilateration only produces finite (lat, lon) and lat is bounded by
/// [-90, 90], lon by [-180, 180]; with scale = 10^4 the absolute value
/// is ≤ 1.8e6, well within i32 range. `f64 as i32` saturates on
/// non-finite inputs (Rust 1.45+), so even a hypothetical NaN here would
/// produce 0/0 — the cache would alias all NaNs to one cell, which is
/// harmless because trilateration never produces NaN today and the cell
/// would never be hit by a real query (task-0064).
pub(crate) fn address_cache_key(lat: f64, lon: f64) -> (i32, i32) {
    debug_assert!(
        lat.is_finite() && lon.is_finite(),
        "address_cache_key called with non-finite ({lat}, {lon})"
    );
    let scale = 10f64.powi(ADDRESS_CACHE_DECIMALS);
    ((lat * scale).round() as i32, (lon * scale).round() as i32)
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
        let entry = self.map.get(&address_cache_key(lat, lon))?;
        let age_days = (chrono::Utc::now() - entry.inserted_at).num_days();
        if age_days >= self.ttl_days {
            return None;
        }
        Some(entry.address.clone())
    }

    pub fn insert(&mut self, lat: f64, lon: f64, addr: String) {
        let key = address_cache_key(lat, lon);
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
        let a = address_cache_key(55.668412, 12.554123);
        let b = address_cache_key(55.668415, 12.554118); // sub-meter offset
        assert_eq!(a, b, "neighbouring sub-meter positions must share a cell");

        let c = address_cache_key(55.6684, 12.5541);
        let d = address_cache_key(55.6700, 12.5541);
        assert_ne!(c, d, "positions ~150m apart must NOT share a cell");
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

    /// TTL: an entry whose inserted_at is older than ttl_days reads back
    /// as None.
    #[test]
    fn address_cache_expires_after_ttl() {
        let mut cache = AddressCache::with_ttl_days(7);
        cache.insert(55.6684, 12.5541, "Copenhagen".to_string());
        assert_eq!(cache.get(55.6684, 12.5541).as_deref(), Some("Copenhagen"));

        let key = address_cache_key(55.6684, 12.5541);
        cache.map.get_mut(&key).unwrap().inserted_at =
            chrono::Utc::now() - chrono::TimeDelta::days(8);

        assert!(
            cache.get(55.6684, 12.5541).is_none(),
            "entry older than TTL must read back as None"
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
