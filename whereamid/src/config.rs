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
    #[arg(long, default_value = "~/.whereami.toml")]
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

/// TOML config file structure (secrets only).
#[derive(Deserialize, Debug, Clone, Default)]
pub struct ConfigFile {
    #[serde(default)]
    pub wigle: WigleConfig,
    #[serde(default)]
    pub beacondb: BeaconDbConfig,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct WigleConfig {
    #[serde(default)]
    pub api_user: String,
    #[serde(default)]
    pub api_key: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct BeaconDbConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for BeaconDbConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
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
