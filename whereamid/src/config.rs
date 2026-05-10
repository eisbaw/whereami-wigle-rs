//! CLI arg parsing (clap) and TOML config file loading for secrets.

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;
use std::path::PathBuf;

/// whereamid: Wi-Fi geolocation daemon
#[derive(Parser, Debug, Clone)]
#[command(name = "whereamid", version, about)]
pub struct Args {
    /// TCP bind address
    #[arg(long, default_value = "127.0.0.1:4747")]
    pub bind: String,

    /// Path to SQLite database
    #[arg(long, default_value = "/var/lib/whereami/aps.sqlite")]
    pub db: PathBuf,

    /// Wi-Fi interface name
    #[arg(long, default_value = "wlan0")]
    pub interface: String,

    /// Path to TOML config file (API secrets)
    #[arg(long, default_value = "~/.config/whereami/config.toml")]
    pub config: String,

    /// Scan interval during fast phase (seconds)
    #[arg(long, default_value_t = 10)]
    pub scan_interval_fast: u64,

    /// How long the fast scan phase lasts (seconds)
    #[arg(long, default_value_t = 60)]
    pub scan_fast_duration: u64,

    /// Scan interval during steady phase (seconds)
    #[arg(long, default_value_t = 60)]
    pub scan_interval_slow: u64,

    /// Number of scan samples in debounce ring buffer
    #[arg(long, default_value_t = 10)]
    pub debounce_window: usize,

    /// Minimum appearances in ring buffer to be considered stable
    #[arg(long, default_value_t = 5)]
    pub debounce_threshold: usize,

    /// Only consider top-N strongest APs for trilateration
    #[arg(long, default_value_t = 10)]
    pub top_n: usize,

    /// Seconds between pending queue drain runs
    #[arg(long, default_value_t = 300)]
    pub pending_interval: u64,

    /// Drop from pending after this many failed attempts
    #[arg(long, default_value_t = 20)]
    pub pending_max_attempts: i32,

    /// Maximum WiGLE API calls per day
    #[arg(long, default_value_t = 100)]
    pub daily_limit: u32,

    /// Days before re-checking a not-found BSSID
    #[arg(long, default_value_t = 30)]
    pub not_found_ttl_days: i64,

    /// Include approximate street address in locate responses (via OSM Nominatim)
    #[arg(long)]
    pub address_approx: bool,
}

impl Args {
    /// Cross-field validation that clap cannot express in a single
    /// `value_parser`. Call immediately after `parse()` so we fail fast
    /// before any side effects (DB open, signals, threads, etc.).
    ///
    /// Single-field bounds (e.g. > 0) live on the field's `value_parser`
    /// where clap can render the help text; this method is for invariants
    /// across multiple fields.
    pub fn validate(&self) -> Result<()> {
        if self.debounce_window == 0 {
            anyhow::bail!("--debounce-window must be > 0");
        }
        if self.debounce_threshold == 0 {
            anyhow::bail!("--debounce-threshold must be > 0");
        }
        if self.debounce_threshold > self.debounce_window {
            anyhow::bail!(
                "--debounce-threshold ({}) must be <= --debounce-window ({})",
                self.debounce_threshold,
                self.debounce_window
            );
        }
        if self.scan_interval_fast == 0 {
            anyhow::bail!("--scan-interval-fast must be > 0");
        }
        if self.scan_interval_slow == 0 {
            anyhow::bail!("--scan-interval-slow must be > 0");
        }
        if self.top_n == 0 {
            anyhow::bail!("--top-n must be > 0");
        }
        if self.pending_interval == 0 {
            anyhow::bail!("--pending-interval must be > 0");
        }
        if self.pending_max_attempts <= 0 {
            anyhow::bail!("--pending-max-attempts must be > 0");
        }
        if self.not_found_ttl_days <= 0 {
            anyhow::bail!("--not-found-ttl-days must be > 0");
        }
        Ok(())
    }
}

/// TOML config file structure (secrets only).
#[derive(Deserialize, Debug, Clone, Default)]
pub struct ConfigFile {
    #[serde(default)]
    pub wigle: WigleConfig,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct WigleConfig {
    #[serde(default)]
    pub api_user: String,
    #[serde(default)]
    pub api_key: String,
}

/// Expand ~ to home directory.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

/// Load the TOML config file. Returns default if file does not exist.
pub fn load_config_file(path: &str) -> Result<ConfigFile> {
    let expanded = expand_tilde(path);
    if !expanded.exists() {
        tracing::warn!(
            "config file not found at {}, using defaults",
            expanded.display()
        );
        return Ok(ConfigFile::default());
    }
    let contents = std::fs::read_to_string(&expanded)
        .with_context(|| format!("reading config file {}", expanded.display()))?;
    let config: ConfigFile = toml::from_str(&contents)
        .with_context(|| format!("parsing config file {}", expanded.display()))?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(extra: &[&str]) -> Args {
        let mut argv = vec!["whereamid"];
        argv.extend_from_slice(extra);
        Args::parse_from(argv)
    }

    #[test]
    fn defaults_validate_clean() {
        parse(&[]).validate().expect("default Args must validate");
    }

    #[test]
    fn debounce_threshold_greater_than_window_rejected() {
        let args = parse(&["--debounce-window", "3", "--debounce-threshold", "5"]);
        let err = args.validate().expect_err("expected validation error");
        let msg = format!("{err}");
        assert!(
            msg.contains("debounce-threshold"),
            "error message should name the offending flag, got: {msg}"
        );
    }

    #[test]
    fn debounce_threshold_equal_to_window_accepted() {
        parse(&["--debounce-window", "5", "--debounce-threshold", "5"])
            .validate()
            .expect("threshold == window must be allowed");
    }

    #[test]
    fn zero_window_rejected() {
        assert!(parse(&["--debounce-window", "0"]).validate().is_err());
    }

    #[test]
    fn zero_top_n_rejected() {
        assert!(parse(&["--top-n", "0"]).validate().is_err());
    }

    #[test]
    fn zero_scan_interval_fast_rejected() {
        assert!(parse(&["--scan-interval-fast", "0"]).validate().is_err());
    }
}
