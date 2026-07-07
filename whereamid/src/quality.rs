//! Validated quality metadata attached to one persisted location observation.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Closed confidence vocabulary shared by locate responses and persisted
/// history. Adding a variant is a wire-protocol and schema compatibility
/// change, not a display-only edit.
///
/// MIRROR: `whereami_client::Confidence` is a hand-kept copy across the crate
/// boundary (no shared protocol crate yet — task-0114). Both derive
/// `#[serde(rename_all = "lowercase")]` over the SAME variants in the SAME
/// order, so they MUST serialize to identical tokens. If you reorder or rename
/// a variant here, change the client mirror in lockstep. The only intended
/// asymmetry is the client's `#[default] High` (inert for history, which is
/// `Option<Confidence>`).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    High,
    Medium,
    Low,
    Suspect,
}

impl Confidence {
    pub const fn as_db_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Suspect => "suspect",
        }
    }

    pub fn from_db_str(value: &str) -> Result<Self> {
        match value {
            "high" => Ok(Self::High),
            "medium" => Ok(Self::Medium),
            "low" => Ok(Self::Low),
            "suspect" => Ok(Self::Suspect),
            _ => anyhow::bail!("unknown confidence value {value:?}"),
        }
    }
}

impl std::fmt::Display for Confidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::High => "High",
            Self::Medium => "Medium",
            Self::Low => "Low",
            Self::Suspect => "Suspect",
        })
    }
}

/// Quality evidence for exactly one fix observation.
///
/// The constructor is the sole production creation boundary. In particular,
/// speed and age are a pair: both are undefined on a cold start and both are
/// present once a previous fix exists. A suspect reason exists exactly for a
/// suspect decision.
#[derive(Debug, Clone, PartialEq)]
pub struct FixQuality {
    implied_speed_kmh: Option<f64>,
    last_fix_age_secs: Option<i64>,
    inlier_ratio: Option<f64>,
    confidence: Confidence,
    suspect_reason: Option<String>,
}

impl FixQuality {
    pub fn new(
        implied_speed_kmh: Option<f64>,
        last_fix_age_secs: Option<i64>,
        inlier_ratio: Option<f64>,
        confidence: Confidence,
        suspect_reason: Option<String>,
    ) -> Result<Self> {
        if implied_speed_kmh.is_some() != last_fix_age_secs.is_some() {
            anyhow::bail!("implied speed and last-fix age must be present together");
        }
        if implied_speed_kmh.is_some_and(|value| !value.is_finite() || value < 0.0) {
            anyhow::bail!("implied speed must be finite and nonnegative");
        }
        if last_fix_age_secs.is_some_and(|value| value < 0) {
            anyhow::bail!("last-fix age must be nonnegative");
        }
        if inlier_ratio.is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value)) {
            anyhow::bail!("inlier ratio must be finite and within 0..=1");
        }
        match (confidence, suspect_reason.as_deref()) {
            (Confidence::Suspect, Some(reason)) if !reason.is_empty() => {}
            (Confidence::Suspect, _) => {
                anyhow::bail!("suspect confidence requires a nonempty reason")
            }
            (_, None) => {}
            (_, Some(_)) => anyhow::bail!("a suspect reason is valid only for suspect confidence"),
        }

        Ok(Self {
            implied_speed_kmh,
            last_fix_age_secs,
            inlier_ratio,
            confidence,
            suspect_reason,
        })
    }

    pub const fn implied_speed_kmh(&self) -> Option<f64> {
        self.implied_speed_kmh
    }

    pub const fn last_fix_age_secs(&self) -> Option<i64> {
        self.last_fix_age_secs
    }

    pub const fn inlier_ratio(&self) -> Option<f64> {
        self.inlier_ratio
    }

    pub const fn confidence(&self) -> Confidence {
        self.confidence
    }

    pub fn suspect_reason(&self) -> Option<&str> {
        self.suspect_reason.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task0245_quality_validation_rejects_invalid_bundles() {
        let invalid = [
            FixQuality::new(Some(1.0), None, None, Confidence::High, None),
            FixQuality::new(Some(f64::NAN), Some(1), None, Confidence::High, None),
            FixQuality::new(Some(-1.0), Some(1), None, Confidence::High, None),
            FixQuality::new(Some(1.0), Some(-1), None, Confidence::High, None),
            FixQuality::new(None, None, Some(f64::INFINITY), Confidence::High, None),
            FixQuality::new(None, None, Some(-0.01), Confidence::High, None),
            FixQuality::new(None, None, Some(1.01), Confidence::High, None),
            FixQuality::new(None, None, None, Confidence::Suspect, None),
            FixQuality::new(
                None,
                None,
                None,
                Confidence::High,
                Some("unexpected".to_owned()),
            ),
        ];
        assert!(invalid.into_iter().all(|result| result.is_err()));
    }

    #[test]
    fn task0245_quality_validation_accepts_boundaries_and_cold_start() {
        assert!(FixQuality::new(None, None, Some(0.0), Confidence::Low, None).is_ok());
        assert!(FixQuality::new(
            Some(0.0),
            Some(0),
            Some(1.0),
            Confidence::Suspect,
            Some("clock moved".to_owned()),
        )
        .is_ok());
    }
}
