//! Strongly-typed IEEE-802 48-bit MAC address (a BSSID).
//!
//! A BSSID is not free-form text: it is exactly six octets. Modelling it as a
//! `[u8; 6]` newtype (rather than a `String`) makes two whole classes of bug
//! unrepresentable:
//!
//!   * **Injection (task-0122 AC#10).** A `Bssid` can only ever hold six
//!     hex-derived octets, so metacharacters (`& # ? /`, whitespace, `../`)
//!     can never survive construction and therefore can never reach the
//!     outbound WiGLE/Apple request. The type *is* the sanitiser.
//!   * **Randomized-client confusion (task-0122 AC#1/#2).** The U/L bit
//!     (`0x02` in the first octet) marks a locally-administered MAC — an OS/
//!     phone-hotspot randomized address. The same randomized address seen in
//!     Copenhagen, Paris and Salvador is three different devices wearing one
//!     costume, NOT one moving infrastructure AP. `is_locally_administered`
//!     exposes that domain fact as a total function of the six bytes, so the
//!     trilateration boundary can exclude such addresses (subsumes the
//!     task-0093 SSID denylist).
//!
//! `FromStr` is deliberately strict: it rejects wrong lengths, non-hex,
//! all-zeros, the broadcast address and any multicast address. A
//! locally-administered *unicast* MAC is a valid `Bssid` (a phone hotspot is a
//! real, if ephemeral, AP) — it is only excluded later, at trilateration.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A parsed 48-bit IEEE-802 MAC address, stored as its six canonical octets.
///
/// `#[repr(transparent)]` keeps the in-memory layout identical to `[u8; 6]`,
/// so the newtype is a pure compile-time distinction with zero runtime cost.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Bssid([u8; 6]);

/// Why a string failed to parse as a `Bssid`. Typed (not a bare string) so
/// callers can branch on the reason and so the boundary log names the exact
/// defect. Fail-fast: every rejection is one of these, never a silent default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BssidParseError {
    /// Not six octets: wrong number of groups, wrong group width, or a bare
    /// hex string whose length is not exactly 12 hex digits.
    WrongLength,
    /// A character outside `[0-9a-fA-F]` appeared where a hex digit was
    /// required (this is what makes `& # ? /` and whitespace unrepresentable).
    NonHex,
    /// All six octets are zero — not a usable address.
    AllZeros,
    /// The broadcast address `FF:FF:FF:FF:FF:FF` — not a single AP.
    Broadcast,
    /// A multicast address (`0x01` bit of the first octet set). A BSSID is a
    /// unicast station address by definition; multicast here is malformed.
    Multicast,
}

impl fmt::Display for BssidParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BssidParseError::WrongLength => {
                write!(f, "BSSID must be six hex octets (AA:BB:CC:DD:EE:FF)")
            }
            BssidParseError::NonHex => write!(f, "BSSID contains a non-hex character"),
            BssidParseError::AllZeros => write!(f, "BSSID is all zeros"),
            BssidParseError::Broadcast => write!(f, "BSSID is the broadcast address"),
            BssidParseError::Multicast => write!(f, "BSSID is a multicast address"),
        }
    }
}

impl std::error::Error for BssidParseError {}

impl Bssid {
    /// The broadcast address `FF:FF:FF:FF:FF:FF`.
    const BROADCAST: [u8; 6] = [0xFF; 6];

    /// True iff the U/L bit (`0x02` of the first octet) is set — i.e. the
    /// address is locally administered. OS MAC randomization and phone
    /// hotspots produce locally-administered addresses; universally-
    /// administered (OUI-assigned) infrastructure APs do not. This is a total,
    /// deterministic function of the six bytes, which is exactly why the
    /// derived flag is computed on read rather than persisted (task-0122 AC#5).
    pub fn is_locally_administered(&self) -> bool {
        self.0[0] & 0x02 != 0
    }

    /// True iff the multicast/group bit (`0x01` of the first octet) is set.
    /// `FromStr` already rejects such inputs; retained as a domain predicate.
    pub fn is_multicast(&self) -> bool {
        self.0[0] & 0x01 != 0
    }

    /// True iff this is the broadcast address. `FromStr` already rejects it;
    /// retained as a domain predicate.
    pub fn is_broadcast(&self) -> bool {
        self.0 == Self::BROADCAST
    }

    /// Parse a single ASCII byte as a hex nibble, or `NonHex`.
    fn hex_nibble(b: u8) -> Result<u8, BssidParseError> {
        match b {
            b'0'..=b'9' => Ok(b - b'0'),
            b'a'..=b'f' => Ok(b - b'a' + 10),
            b'A'..=b'F' => Ok(b - b'A' + 10),
            _ => Err(BssidParseError::NonHex),
        }
    }
}

/// Canonical rendering: uppercase, colon-separated (`AA:BB:CC:DD:EE:FF`). This
/// is the single serialized form; `FromStr` accepts more shapes but `Display`
/// emits exactly one, so a round-trip normalizes.
impl fmt::Display for Bssid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c, d, e, g] = self.0;
        write!(f, "{a:02X}:{b:02X}:{c:02X}:{d:02X}:{e:02X}:{g:02X}")
    }
}

/// `Debug` shows the canonical string too (not six raw byte literals), so test
/// failures and logs read as addresses.
impl fmt::Debug for Bssid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bssid({self})")
    }
}

impl FromStr for Bssid {
    type Err = BssidParseError;

    /// Accepts `AA:BB:CC:DD:EE:FF`, `AA-BB-CC-DD-EE-FF`, or a bare 12-hex-digit
    /// string (case-insensitive; surrounding whitespace trimmed). Rejects
    /// wrong lengths, non-hex, all-zeros, broadcast and multicast.
    ///
    /// Operates on raw bytes throughout (never str slicing), so a multibyte
    /// UTF-8 input can never panic on a char boundary — it simply fails as
    /// `NonHex`/`WrongLength`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let bytes = s.as_bytes();

        let mut octets = [0u8; 6];

        if bytes.contains(&b':') || bytes.contains(&b'-') {
            // Separated form: exactly six two-digit groups.
            let mut groups = s.split([':', '-']);
            for octet in octets.iter_mut() {
                let group = groups.next().ok_or(BssidParseError::WrongLength)?;
                let gb = group.as_bytes();
                if gb.len() != 2 {
                    return Err(BssidParseError::WrongLength);
                }
                *octet = (Bssid::hex_nibble(gb[0])? << 4) | Bssid::hex_nibble(gb[1])?;
            }
            if groups.next().is_some() {
                return Err(BssidParseError::WrongLength);
            }
        } else {
            // Bare form: exactly 12 hex digits.
            if bytes.len() != 12 {
                return Err(BssidParseError::WrongLength);
            }
            for (i, octet) in octets.iter_mut().enumerate() {
                *octet =
                    (Bssid::hex_nibble(bytes[2 * i])? << 4) | Bssid::hex_nibble(bytes[2 * i + 1])?;
            }
        }

        // Semantic rejections, expressed through the domain predicates so the
        // meaning of "broadcast"/"multicast" lives in exactly one place. Order:
        // broadcast is also multicast, so check it first for the more specific
        // reason.
        let candidate = Bssid(octets);
        if octets == [0u8; 6] {
            return Err(BssidParseError::AllZeros);
        }
        if candidate.is_broadcast() {
            return Err(BssidParseError::Broadcast);
        }
        if candidate.is_multicast() {
            return Err(BssidParseError::Multicast);
        }

        Ok(candidate)
    }
}

impl Serialize for Bssid {
    /// Serialized as the canonical string, so on-wire/JSON representations stay
    /// human-readable and interoperable with the `String`-typed fields this
    /// newtype has not yet replaced (see the deferred follow-up in task-0122).
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Bssid {
    /// Deserialized from a string through the same strict `FromStr`, so a
    /// malformed BSSID cannot enter the system via a deserialization path
    /// either.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC#6: the Brazil-in-Copenhagen poisoner F6:B1:9C:0A:3A:60 is a
    /// well-formed unicast MAC (so it parses) but is locally-administered
    /// (F6 = 1111_0110, U/L bit set), so the trilateration boundary excludes it.
    #[test]
    fn poisoner_f6_is_locally_administered() {
        let b: Bssid = "F6:B1:9C:0A:3A:60"
            .parse()
            .expect("F6… is a valid unicast MAC");
        assert!(
            b.is_locally_administered(),
            "F6 first octet has the U/L bit set — must be locally administered"
        );
        assert!(!b.is_multicast(), "F6 is unicast (0x01 bit clear)");
    }

    /// AC#7: a real universally-administered infrastructure BSSID (TP-Link OUI
    /// B8:50:01, U/L bit clear) parses and is NOT locally administered, so it
    /// passes the trilateration boundary unchanged.
    #[test]
    fn universal_infra_b8_is_not_locally_administered() {
        let b: Bssid = "B8:50:01:11:22:33"
            .parse()
            .expect("B8… is a valid infra MAC");
        assert!(
            !b.is_locally_administered(),
            "B8 = 1011_1000, U/L bit clear — universally administered"
        );
    }

    /// AC#10: strings carrying URL metacharacters or path traversal cannot be
    /// constructed as a `Bssid`, so they can never reach the outbound WiGLE
    /// query. The type is the sanitiser.
    #[test]
    fn injection_shaped_strings_never_parse() {
        for evil in [
            "aa&x=1",
            "../",
            "aa:bb:cc:dd:ee:ff&netid=evil",
            "aa bb cc dd ee ff",
            "aa\tbb:cc:dd:ee:ff",
            "aa:bb:cc:dd:ee:f#",
            "%2e%2e/",
            "AA:BB:CC:DD:EE:FF/../secret",
        ] {
            assert!(
                evil.parse::<Bssid>().is_err(),
                "injection-shaped input {evil:?} must not parse as a Bssid"
            );
        }
    }

    #[test]
    fn accepts_colon_hyphen_and_bare_forms_case_insensitively() {
        let canonical = "AA:BB:CC:DD:EE:0F";
        for form in [
            "aa:bb:cc:dd:ee:0f",
            "AA:BB:CC:DD:EE:0F",
            "aa-bb-cc-dd-ee-0f",
            "aabbccddee0f",
            "  aa:bb:cc:dd:ee:0f  ",
        ] {
            let b: Bssid = form
                .parse()
                .unwrap_or_else(|e| panic!("{form:?} must parse: {e}"));
            assert_eq!(b.to_string(), canonical, "{form:?} must canonicalize");
        }
    }

    #[test]
    fn rejects_zeros_broadcast_multicast_and_malformed() {
        assert_eq!(
            "00:00:00:00:00:00".parse::<Bssid>().unwrap_err(),
            BssidParseError::AllZeros
        );
        assert_eq!(
            "ff:ff:ff:ff:ff:ff".parse::<Bssid>().unwrap_err(),
            BssidParseError::Broadcast
        );
        // 0x01 first octet: multicast/group bit set.
        assert_eq!(
            "01:00:5e:00:00:01".parse::<Bssid>().unwrap_err(),
            BssidParseError::Multicast
        );
        // 0x11 = 0001_0001 also has the group bit set.
        assert_eq!(
            "11:22:33:44:55:66".parse::<Bssid>().unwrap_err(),
            BssidParseError::Multicast
        );
        assert_eq!(
            "aa:bb:cc:dd:ee".parse::<Bssid>().unwrap_err(),
            BssidParseError::WrongLength
        );
        assert_eq!(
            "aa:bb:cc:dd:ee:ff:00".parse::<Bssid>().unwrap_err(),
            BssidParseError::WrongLength
        );
        assert_eq!(
            "zz:bb:cc:dd:ee:ff".parse::<Bssid>().unwrap_err(),
            BssidParseError::NonHex
        );
    }

    #[test]
    fn serde_round_trips_through_canonical_string() {
        let b: Bssid = "aa:bb:cc:dd:ee:0f".parse().unwrap();
        let json = serde_json::to_string(&b).unwrap();
        assert_eq!(json, "\"AA:BB:CC:DD:EE:0F\"");
        let back: Bssid = serde_json::from_str(&json).unwrap();
        assert_eq!(b, back);
        // A malformed BSSID cannot enter via deserialization.
        assert!(serde_json::from_str::<Bssid>("\"01:00:5e:00:00:01\"").is_err());
    }

    /// A tight cluster made ONLY of locally-administered poisoners is entirely
    /// locally-administered, so the trilateration boundary excludes every one
    /// of them before the outlier pipeline (drop_isolated) — they cannot vouch
    /// for one another as a fake stay (AC#6, ordering).
    #[test]
    fn poisoner_cluster_is_wholly_locally_administered() {
        let cluster = [
            "F6:B1:9C:0A:3A:60",
            "F6:B1:9C:0A:3A:61",
            "F6:B1:9C:0A:3A:62",
        ];
        assert!(
            cluster
                .iter()
                .all(|b| b.parse::<Bssid>().unwrap().is_locally_administered()),
            "every randomized-MAC poisoner must be locally administered"
        );
    }
}
