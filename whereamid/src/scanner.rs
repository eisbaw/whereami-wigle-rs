//! Wi-Fi scanner with result provenance tied to one blocking refresh command.

use std::future::Future;
use std::pin::Pin;

use anyhow::{anyhow, bail, Result};
use tokio::process::Command;
use tokio::time::{sleep, timeout, timeout_at, Duration, Instant as TokioInstant};
use tracing::{debug, warn};

use whereami_protocol::Channel;

use crate::boot_clock::BootTime;

const SCAN_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const NM_FORCED_COMMAND_TIMEOUT: Duration = Duration::from_secs(18);
const NM_REFRESH_DEADLINE: Duration = Duration::from_secs(20);
const IW_BUSY_RETRIES: usize = 3;
const NM_LAST_SCAN_POLL_INTERVAL: Duration = Duration::from_millis(100);
const NM_SERVICE: &str = "org.freedesktop.NetworkManager";
const NM_ROOT_PATH: &str = "/org/freedesktop/NetworkManager";
const NM_WIRELESS_INTERFACE: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const DBUS_SERVICE: &str = "org.freedesktop.DBus";
const DBUS_PATH: &str = "/org/freedesktop/DBus";

#[derive(Clone, Copy)]
struct NmRefreshTiming {
    deadline: Duration,
    poll_interval: Duration,
}

const NM_REFRESH_TIMING: NmRefreshTiming = NmRefreshTiming {
    deadline: NM_REFRESH_DEADLINE,
    poll_interval: NM_LAST_SCAN_POLL_INTERVAL,
};

/// Default comma-separated value accepted by `--mobile-ssid-denylist`.
pub const DEFAULT_MOBILE_SSID_DENYLIST: &str =
    "DSB_Wi-Fi,.DSB_Wi-Fi,dsb-mng-wifi,MOVIA*,Bus_WiFi,Bus WiFi,*_iPhone,*_Hotspot,iPhone,AndroidAP*";

pub fn default_mobile_ssid_denylist() -> Vec<String> {
    DEFAULT_MOBILE_SSID_DENYLIST
        .split(',')
        .map(str::to_owned)
        .collect()
}

/// Return whether a scanner-observed SSID is classified as mobile.
///
/// Missing, empty, and the scanner sentinel `<hidden>` are always treated as
/// fixed, even when a broad pattern such as `*` would otherwise match. Patterns
/// are anchored globs: `*` matches zero or more Unicode scalar values and `?`
/// exactly one. Literal scalars compare by their locale-independent Unicode
/// lowercase expansions; this is not locale-sensitive collation or full
/// Unicode case folding.
pub fn is_mobile_ssid(ssid: Option<&str>, denylist: &[String]) -> bool {
    let Some(ssid) = ssid else {
        return false;
    };
    if ssid.is_empty() || ssid.eq_ignore_ascii_case("<hidden>") {
        return false;
    }

    denylist.iter().any(|pattern| glob_matches(pattern, ssid))
}

/// Match one anchored `*`/`?` glob case-insensitively. A dynamic-programming
/// row keeps matching total for arbitrary UTF-8 and avoids backtracking.
fn glob_matches(pattern: &str, value: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let value: Vec<char> = value.chars().collect();

    let mut previous = vec![false; value.len() + 1];
    previous[0] = true;

    for token in pattern {
        let mut current = vec![false; value.len() + 1];
        if token == '*' {
            current[0] = previous[0];
        }
        for index in 1..=value.len() {
            current[index] = match token {
                '*' => previous[index] || current[index - 1],
                '?' => previous[index - 1],
                literal => {
                    previous[index - 1]
                        && literal.to_lowercase().eq(value[index - 1].to_lowercase())
                }
            };
        }
        previous = current;
    }

    previous[value.len()]
}

/// A single observed Wi-Fi network from a scan.
#[derive(Debug, Clone)]
pub struct ScannedNetwork {
    pub bssid: String,
    pub ssid: Option<String>,
    pub signal_dbm: i32,
    /// Which backend produced `signal_dbm`, and thus how trustworthy it is:
    /// iw reports authoritative raw dBm; nmcli's dBm is a biased linearization
    /// of its 0–100 percentage (see [`crate::rssi`]). `signal_dbm` stays `i32`
    /// (wire/DB unchanged); the provenance rides alongside it. task-0126.
    pub rssi_source: crate::rssi::RssiSource,
    pub channel: Option<Channel>,
    #[allow(dead_code)]
    pub frequency: Option<i32>,
}

/// One readable scanner batch plus optional proof that its backend completed a
/// radio refresh before reading the batch. Networks remain useful to
/// interactive commands when the proof is absent; autonomous history applies
/// the stricter policy.
#[derive(Debug, Clone)]
pub struct ScanBatch {
    pub networks: Vec<ScannedNetwork>,
    pub confirmed_refresh_started_at: Option<BootTime>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScanCommand {
    program: String,
    args: Vec<String>,
    locale_c: bool,
    timeout: Duration,
    kill_on_drop: bool,
}

impl ScanCommand {
    fn new(program: &str, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            program: program.to_owned(),
            args: args.into_iter().map(Into::into).collect(),
            locale_c: false,
            timeout: SCAN_COMMAND_TIMEOUT,
            kill_on_drop: true,
        }
    }

    fn with_c_locale(mut self) -> Self {
        self.locale_c = true;
        self
    }

    fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CommandOutcome {
    Succeeded { stdout: String },
    NonZero { stderr: String },
    TimedOut,
    FailedToStart { error: String },
}

type CommandFuture<'a> = Pin<Box<dyn Future<Output = CommandOutcome> + Send + 'a>>;
type DelayFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// Narrow execution seam for the scanner's fixed command state machine. Tests
/// inject exact command outcomes without replacing any other daemon behavior.
trait ScanCommandRunner {
    fn boot_time(&mut self) -> std::io::Result<BootTime>;
    fn run<'a>(&'a mut self, command: ScanCommand) -> CommandFuture<'a>;
    fn delay<'a>(&'a mut self, duration: Duration) -> DelayFuture<'a>;
}

struct TokioScanCommandRunner;

impl ScanCommandRunner for TokioScanCommandRunner {
    fn boot_time(&mut self) -> std::io::Result<BootTime> {
        BootTime::now()
    }

    fn run<'a>(&'a mut self, command: ScanCommand) -> CommandFuture<'a> {
        Box::pin(async move {
            let mut process = Command::new(&command.program);
            process
                .args(&command.args)
                .kill_on_drop(command.kill_on_drop);
            if command.locale_c {
                process.env("LC_ALL", "C");
            }

            match timeout(command.timeout, process.output()).await {
                Ok(Ok(output)) if output.status.success() => CommandOutcome::Succeeded {
                    stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                },
                Ok(Ok(output)) => CommandOutcome::NonZero {
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                },
                Ok(Err(error)) => CommandOutcome::FailedToStart {
                    error: error.to_string(),
                },
                Err(_) => CommandOutcome::TimedOut,
            }
        })
    }

    fn delay<'a>(&'a mut self, duration: Duration) -> DelayFuture<'a> {
        Box::pin(sleep(duration))
    }
}

fn finish_refresh_evidence(
    backend: &str,
    started_at: std::io::Result<BootTime>,
) -> Option<BootTime> {
    match started_at {
        Ok(evidence) => Some(evidence),
        Err(error) => {
            warn!(
                "{backend} refresh succeeded but CLOCK_BOOTTIME could not timestamp its start: \
                 {error}; autonomous scan freshness is unconfirmed"
            );
            None
        }
    }
}

fn nmcli_list_command(interface: &str, force_refresh: bool) -> ScanCommand {
    ScanCommand::new(
        "nmcli",
        [
            "-t",
            "-f",
            "BSSID,SSID,SIGNAL,CHAN,FREQ",
            "device",
            "wifi",
            "list",
            "ifname",
            interface,
            "--rescan",
            if force_refresh { "yes" } else { "no" },
        ],
    )
    .with_c_locale()
    .with_timeout(if force_refresh {
        NM_FORCED_COMMAND_TIMEOUT
    } else {
        SCAN_COMMAND_TIMEOUT
    })
}

fn nm_device_path_command(owner: &str, interface: &str) -> ScanCommand {
    ScanCommand::new(
        "busctl",
        [
            "--json=short",
            "--system",
            "call",
            owner,
            NM_ROOT_PATH,
            NM_SERVICE,
            "GetDeviceByIpIface",
            "s",
            interface,
        ],
    )
}

fn nm_owner_command() -> ScanCommand {
    ScanCommand::new(
        "busctl",
        [
            "--json=short",
            "--system",
            "call",
            DBUS_SERVICE,
            DBUS_PATH,
            DBUS_SERVICE,
            "GetNameOwner",
            "s",
            NM_SERVICE,
        ],
    )
}

fn nm_last_scan_command(owner: &str, device_path: &str) -> ScanCommand {
    ScanCommand::new(
        "busctl",
        [
            "--json=short",
            "--system",
            "get-property",
            owner,
            device_path,
            NM_WIRELESS_INTERFACE,
            "LastScan",
        ],
    )
}

fn iw_direct_scan_command(interface: &str) -> ScanCommand {
    ScanCommand::new("iw", ["dev", interface, "scan"])
}

fn iw_cached_dump_command(interface: &str) -> ScanCommand {
    ScanCommand::new("iw", ["dev", interface, "scan", "dump"])
}

/// Scan Wi-Fi networks. NetworkManager's forced list is preferred because it
/// needs no daemon capability. Only an unavailable or empty *unconfirmed*
/// NetworkManager fallback proceeds to the CAP_NET_ADMIN `iw` path.
pub async fn wifi_scan(interface: &str) -> Result<ScanBatch> {
    let mut runner = TokioScanCommandRunner;
    wifi_scan_with_runner(interface, &mut runner).await
}

async fn wifi_scan_with_runner<R: ScanCommandRunner>(
    interface: &str,
    runner: &mut R,
) -> Result<ScanBatch> {
    match wifi_scan_nmcli(interface, runner).await {
        Ok(Some(batch)) => return Ok(batch),
        Ok(None) => debug!("unconfirmed nmcli cache is empty, falling back to iw"),
        Err(e) => debug!("nmcli scan failed ({e}), falling back to iw"),
    }

    wifi_scan_iw(interface, runner).await
}

/// NetworkManager provenance requires more than nmcli's exit status: nmcli can
/// force-finish `--rescan yes` with cached rows after a D-Bus request failure.
/// Resolve the device and baseline its suspend-inclusive `LastScan` generation,
/// run the forced command, wait for generation proof, then re-list with
/// `--rescan no` so the retained rows cannot predate that completion.
async fn wifi_scan_nmcli<R: ScanCommandRunner>(
    interface: &str,
    runner: &mut R,
) -> Result<Option<ScanBatch>> {
    wifi_scan_nmcli_with_timing(interface, runner, NM_REFRESH_TIMING).await
}

async fn wifi_scan_nmcli_with_timing<R: ScanCommandRunner>(
    interface: &str,
    runner: &mut R,
    timing: NmRefreshTiming,
) -> Result<Option<ScanBatch>> {
    let generation_baseline = match nm_owner(runner).await {
        Ok(owner) => match nm_device_path(&owner, interface, runner).await {
            Ok(device_path) => match nm_last_scan(&owner, &device_path, runner).await {
                Ok(last_scan) => Some((owner, device_path, last_scan)),
                Err(error) => {
                    debug!(
                        "cannot baseline NetworkManager LastScan; scan will be unconfirmed: {error}"
                    );
                    None
                }
            },
            Err(error) => {
                debug!(
                    "cannot resolve NetworkManager Wi-Fi device; scan will be unconfirmed: {error}"
                );
                None
            }
        },
        Err(error) => {
            debug!("cannot resolve NetworkManager owner; scan will be unconfirmed: {error}");
            None
        }
    };

    let refresh_started_at = runner.boot_time();
    let refresh_deadline = TokioInstant::now() + timing.deadline;
    let forced_stdout = match timeout_at(
        refresh_deadline,
        run_checked(
            runner,
            nmcli_list_command(interface, true),
            "forced nmcli wifi list",
        ),
    )
    .await
    {
        Ok(Ok(stdout)) => Some(stdout),
        Ok(Err(error)) => {
            debug!("{error}; trying unconfirmed nmcli cache");
            None
        }
        Err(_) => {
            debug!("forced nmcli refresh deadline expired; trying unconfirmed nmcli cache");
            None
        }
    };

    let refresh_started_at = if forced_stdout.is_some() {
        finish_refresh_evidence("nmcli", refresh_started_at)
    } else {
        None
    };
    let provisional_generation = match (generation_baseline.as_ref(), refresh_started_at) {
        (Some((owner, device_path, baseline)), Some(started_at)) if *baseline >= 0 => {
            match wait_for_nm_generation(
                owner,
                device_path,
                *baseline,
                started_at,
                refresh_deadline,
                timing.poll_interval,
                runner,
            )
            .await
            {
                Ok(Some(generation)) => {
                    Some((started_at, generation, owner.clone(), device_path.clone()))
                }
                Ok(None) => {
                    debug!(
                        "forced nmcli exited successfully but NetworkManager LastScan did not \
                             advance; scan will be unconfirmed"
                    );
                    None
                }
                Err(error) => {
                    debug!(
                        "cannot verify NetworkManager LastScan after forced scan; scan will be \
                             unconfirmed: {error}"
                    );
                    None
                }
            }
        }
        (Some((_owner, _device_path, baseline)), Some(_started_at)) if *baseline < 0 => {
            debug!(
                "NetworkManager LastScan baseline is unset; first scan cycle cannot \
                     distinguish scan-start from scan-completion and will be unconfirmed"
            );
            None
        }
        _ => None,
    };

    match run_checked(
        runner,
        nmcli_list_command(interface, false),
        "post-refresh nmcli cached list",
    )
    .await
    {
        Ok(stdout) => {
            let confirmed_refresh_started_at = match provisional_generation {
                Some((started_at, proven_generation, expected_owner, expected_device_path)) => {
                    match verify_nm_post_list(
                        interface,
                        &expected_owner,
                        &expected_device_path,
                        proven_generation,
                        runner,
                    )
                    .await
                    {
                        Ok(true) => Some(started_at),
                        Ok(false) => {
                            debug!(
                                "NetworkManager device path changed or LastScan regressed after \
                                 post-refresh list; scan will be unconfirmed"
                            );
                            None
                        }
                        Err(error) => {
                            debug!(
                                "cannot verify NetworkManager generation after post-refresh list; \
                                 scan will be unconfirmed: {error}"
                            );
                            None
                        }
                    }
                }
                None => None,
            };
            let batch = nmcli_batch(&stdout, confirmed_refresh_started_at);
            if batch.confirmed_refresh_started_at.is_none() && batch.networks.is_empty() {
                Ok(None)
            } else {
                Ok(Some(batch))
            }
        }
        Err(post_error) => match forced_stdout {
            Some(stdout) => {
                let batch = nmcli_batch(&stdout, None);
                if batch.networks.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(batch))
                }
            }
            None => Err(post_error),
        },
    }
}

async fn run_checked<R: ScanCommandRunner>(
    runner: &mut R,
    command: ScanCommand,
    label: &str,
) -> Result<String> {
    match runner.run(command).await {
        CommandOutcome::Succeeded { stdout } => Ok(stdout),
        CommandOutcome::NonZero { stderr } => bail!("{label} failed: {stderr}"),
        CommandOutcome::TimedOut => bail!("{label} timed out"),
        CommandOutcome::FailedToStart { error } => bail!("failed to start {label}: {error}"),
    }
}

async fn nm_device_path<R: ScanCommandRunner>(
    owner: &str,
    interface: &str,
    runner: &mut R,
) -> Result<String> {
    let stdout = run_checked(
        runner,
        nm_device_path_command(owner, interface),
        "NetworkManager device-path lookup",
    )
    .await?;
    parse_nm_device_path(&stdout)
}

async fn nm_owner<R: ScanCommandRunner>(runner: &mut R) -> Result<String> {
    let stdout = run_checked(runner, nm_owner_command(), "NetworkManager owner lookup").await?;
    parse_nm_owner(&stdout)
}

async fn nm_last_scan<R: ScanCommandRunner>(
    owner: &str,
    device_path: &str,
    runner: &mut R,
) -> Result<i64> {
    let stdout = run_checked(
        runner,
        nm_last_scan_command(owner, device_path),
        "NetworkManager LastScan read",
    )
    .await?;
    parse_nm_last_scan(&stdout)
}

async fn wait_for_nm_generation<R: ScanCommandRunner>(
    owner: &str,
    device_path: &str,
    baseline: i64,
    refresh_started_at: BootTime,
    deadline: TokioInstant,
    poll_interval: Duration,
    runner: &mut R,
) -> Result<Option<i64>> {
    loop {
        let current = timeout_at(deadline, nm_last_scan(owner, device_path, runner))
            .await
            .map_err(|_| anyhow!("NetworkManager refresh deadline expired"))??;
        let observed_at = runner.boot_time().map_err(|error| {
            warn!("cannot timestamp NetworkManager LastScan observation: {error}");
            anyhow!("cannot timestamp NetworkManager LastScan observation: {error}")
        })?;
        if last_scan_is_future(current, observed_at) {
            warn!(
                "NetworkManager LastScan {current}ms is in the future for the current boot clock; \
                 scan freshness will be unconfirmed"
            );
            return Ok(None);
        }
        if nm_generation_confirms_refresh(baseline, current, refresh_started_at, observed_at) {
            return Ok(Some(current));
        }
        timeout_at(deadline, runner.delay(poll_interval))
            .await
            .map_err(|_| anyhow!("NetworkManager refresh deadline expired"))?;
    }
}

async fn verify_nm_post_list<R: ScanCommandRunner>(
    interface: &str,
    expected_owner: &str,
    expected_device_path: &str,
    proven_generation: i64,
    runner: &mut R,
) -> Result<bool> {
    let final_owner = nm_owner(runner).await?;
    if final_owner != expected_owner {
        return Ok(false);
    }
    let final_device_path = nm_device_path(expected_owner, interface, runner).await?;
    if final_device_path != expected_device_path {
        return Ok(false);
    }
    let final_generation = nm_last_scan(expected_owner, &final_device_path, runner).await?;
    let observed_at = runner.boot_time().map_err(|error| {
        warn!("cannot timestamp final NetworkManager LastScan observation: {error}");
        anyhow!("cannot timestamp final NetworkManager LastScan observation: {error}")
    })?;
    if last_scan_is_future(final_generation, observed_at) {
        warn!(
            "final NetworkManager LastScan {final_generation}ms is in the future for the current \
             boot clock; scan freshness will be unconfirmed"
        );
        return Ok(false);
    }
    if final_generation < proven_generation {
        return Ok(false);
    }

    // This check must trail the property read. Otherwise NetworkManager can
    // restart after the first owner check and a successor's compatible path
    // and generation can be mistaken for evidence from the captured owner.
    let trailing_owner = nm_owner(runner).await?;
    Ok(trailing_owner == expected_owner)
}

fn nm_generation_confirms_refresh(
    baseline: i64,
    current: i64,
    refresh_started_at: BootTime,
    observed_at: BootTime,
) -> bool {
    let Some(start_ceiling_ms) = refresh_started_at.ceil_millis() else {
        return false;
    };
    let Some(observed_floor_ms) = observed_at.floor_millis() else {
        return false;
    };
    baseline >= 0
        && current > baseline
        && current >= start_ceiling_ms
        && current <= observed_floor_ms
}

fn last_scan_is_future(last_scan: i64, observed_at: BootTime) -> bool {
    match observed_at.floor_millis() {
        Some(observed_floor_ms) => last_scan > observed_floor_ms,
        None => true,
    }
}

fn parse_nm_owner(stdout: &str) -> Result<String> {
    let value: serde_json::Value = serde_json::from_str(stdout)
        .map_err(|error| anyhow!("invalid busctl owner JSON: {error}"))?;
    let owner = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .and_then(|data| (data.len() == 1).then_some(data))
        .and_then(|data| data[0].as_str())
        .filter(|owner| owner.starts_with(':') && !owner.chars().any(char::is_whitespace))
        .ok_or_else(|| anyhow!("busctl returned no valid NetworkManager unique owner"))?;
    if value.get("type").and_then(serde_json::Value::as_str) != Some("s") {
        bail!("busctl owner reply had the wrong D-Bus type");
    }
    Ok(owner.to_owned())
}

fn parse_nm_device_path(stdout: &str) -> Result<String> {
    let value: serde_json::Value = serde_json::from_str(stdout)
        .map_err(|error| anyhow!("invalid busctl device-path JSON: {error}"))?;
    let path = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .and_then(|data| (data.len() == 1).then_some(data))
        .and_then(|data| data[0].as_str())
        .filter(|path| path.starts_with("/org/freedesktop/NetworkManager/Devices/"))
        .ok_or_else(|| anyhow!("busctl returned no valid NetworkManager device path"))?;
    if value.get("type").and_then(serde_json::Value::as_str) != Some("o") {
        bail!("busctl device-path reply had the wrong D-Bus type");
    }
    Ok(path.to_owned())
}

fn parse_nm_last_scan(stdout: &str) -> Result<i64> {
    let value: serde_json::Value = serde_json::from_str(stdout)
        .map_err(|error| anyhow!("invalid busctl LastScan JSON: {error}"))?;
    if value.get("type").and_then(serde_json::Value::as_str) != Some("x") {
        bail!("busctl LastScan reply had the wrong D-Bus type");
    }
    value
        .get("data")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| anyhow!("busctl LastScan reply had no signed 64-bit value"))
}

fn nmcli_batch(stdout: &str, confirmed_refresh_started_at: Option<BootTime>) -> ScanBatch {
    let mut networks = parse_nmcli_output(stdout);
    networks.sort_by_key(|network| std::cmp::Reverse(network.signal_dbm));
    ScanBatch {
        networks,
        confirmed_refresh_started_at,
    }
}

/// Process-lifetime set of scanner parse warnings already emitted at WARN.
///
/// `parse_iw_output` / `parse_nmcli_output` run on EVERY scan cycle in the
/// long-running daemon, so a chronically-malformed line (a driver that prints
/// `signal: unknown dBm` for the same AP on every scan) would emit its WARN
/// forever — steady WARN-level log spam. This set demotes such a line to DEBUG
/// after its first occurrence: [`malformed_signal_warn_is_first`] returns `true`
/// exactly once per key, `false` thereafter. Keyed by a source-tagged BSSID
/// (`"iw:<bssid>"` / `"nmcli:<bssid>"`) so the two parsers do not suppress each
/// other and so distinct broken APs each get one WARN (task-0256).
///
/// The state is process-global and never cleared (a bounded set: at most one
/// entry per malformed BSSID ever seen). Tests must key on BSSIDs unique to the
/// test to avoid cross-test contamination of this global set.
fn malformed_signal_warn_is_first(key: &str) -> bool {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    // `HashSet::insert` returns true iff the key was newly inserted, i.e. the
    // first time this process has seen it.
    SEEN.get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .expect("scanner malformed-warn set mutex poisoned")
        .insert(key.to_owned())
}

/// Parse nmcli terse output. In -t mode, nmcli escapes ':' in field values as '\:'.
/// The field separator is an unescaped ':'.
/// Fields: BSSID:SSID:SIGNAL:CHAN:FREQ
/// Under the C locale requested by `wifi_scan_nmcli`, NetworkManager renders
/// FREQ as an integer followed by the literal ` MHz` suffix.
/// Signal is 0-100 percentage, convert to approximate dBm.
pub fn parse_nmcli_output(output: &str) -> Vec<ScannedNetwork> {
    let mut networks = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Split on unescaped ':' (i.e. ':' not preceded by '\')
        let fields = split_nmcli_fields(line);
        if fields.len() < 5 {
            // A truncated row (e.g. clipped nmcli output) is dropped, same as
            // before — but log it so the AP does not vanish silently, matching
            // the malformed-signal observability (task-0256). DEBUG, not WARN:
            // a chronically-truncated source would otherwise re-spam every scan,
            // and this is off by default.
            debug!(
                "nmcli scan: skipping truncated row with {} of 5 expected fields: {line:?}",
                fields.len()
            );
            continue;
        }

        let bssid = fields[0].replace("\\:", ":");
        let ssid_raw = fields[1].replace("\\:", ":");
        let ssid = if ssid_raw.is_empty() {
            None
        } else {
            Some(ssid_raw)
        };

        // Skip rows where signal is missing or unparseable. The previous
        // behaviour fell back to 0%, which the linear conversion below would
        // then map to -90 dBm — indistinguishable from a real weak AP and
        // thus poisons trilateration weights with fake data.
        let signal_pct: i32 = match fields[2].trim().parse() {
            Ok(v) => v,
            Err(_) => {
                // First occurrence for this BSSID warns; chronic repeats (the
                // same driver printing a bad signal for the same AP every scan)
                // are demoted to DEBUG so they cannot spam WARN for the daemon's
                // lifetime (task-0256).
                if malformed_signal_warn_is_first(&format!("nmcli:{bssid}")) {
                    warn!(
                        "nmcli scan: unparseable signal {:?} for BSSID {bssid}; skipping AP \
                         row (further repeats for this BSSID log at DEBUG)",
                        fields[2].trim()
                    );
                } else {
                    debug!(
                        "nmcli scan: unparseable signal {:?} for BSSID {bssid}; skipping AP \
                         row (repeat; first occurrence logged at WARN)",
                        fields[2].trim()
                    );
                }
                continue;
            }
        };
        let explicit_channel: Option<i32> = fields[3].parse().ok();
        let freq = parse_nmcli_frequency(&fields[4]);
        let channel = resolve_channel(freq, explicit_channel);

        // Convert signal percentage (0-100) to approximate dBm. The biased
        // linear map (100 ≈ -30 dBm, 0 ≈ -90 dBm) and its rationale now live in
        // one place, `Rssi::from_nmcli_percent` (task-0126); the recovered dBm
        // is systematically biased because nmcli's percentage is itself a
        // non-linear, driver-dependent transform of RSSI. `signal_dbm` stays an
        // `i32` on the wire/DB; the provenance is stamped as
        // `RssiSource::NmcliPercentLinearized` so trilateration can tell it
        // apart from authoritative iw dBm.
        let signal_dbm = crate::rssi::Rssi::from_nmcli_percent(signal_pct).dbm();

        push_validated(
            &mut networks,
            &bssid,
            ssid,
            signal_dbm,
            crate::rssi::RssiSource::NmcliPercentLinearized,
            channel,
            freq,
        );
    }
    networks
}

/// Parse nmcli's C-locale `<integer> MHz` frequency representation.
///
/// Bare integers remain accepted for compatibility with older callers of the
/// public parser, but no other suffix or trailing content is accepted.
fn parse_nmcli_frequency(field: &str) -> Option<i32> {
    field.strip_suffix(" MHz").unwrap_or(field).parse().ok()
}

/// Split an nmcli -t line on unescaped ':' characters.
/// In nmcli terse mode, literal colons in values are escaped as '\:'.
pub fn split_nmcli_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            // nmcli terse mode escapes both the separator (`:` -> `\:`) and
            // the escape char itself (`\` -> `\\`). Consume the escaped char
            // so a literal backslash (`\\`) does not leave a dangling `\` that
            // re-triggers escape handling on the following byte (which would
            // mis-parse `\\:` into a field containing `\:`). task-0226.
            match chars.peek() {
                Some(&':') => {
                    current.push(':');
                    chars.next();
                }
                Some(&'\\') => {
                    current.push('\\');
                    chars.next();
                }
                // Lone/other backslash: emit it verbatim, best-effort.
                _ => current.push('\\'),
            }
        } else if ch == ':' {
            fields.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
        }
    }
    fields.push(current);
    fields
}

/// Scan via iw (needs CAP_NET_ADMIN). `iw dev <iface> scan` blocks until its
/// own refreshed results are available, so one successful command can confirm
/// both provenance and output. Generic failures may preserve the old cached
/// dump for interactive continuity, but spawn/timeout/interface-down failures
/// retain the pre-existing total-error behavior and publish nothing.
async fn wifi_scan_iw<R: ScanCommandRunner>(interface: &str, runner: &mut R) -> Result<ScanBatch> {
    let mut retries_remaining = IW_BUSY_RETRIES;
    loop {
        let refresh_started_at = runner.boot_time();
        match runner.run(iw_direct_scan_command(interface)).await {
            CommandOutcome::Succeeded { stdout } if iw_scan_was_aborted(&stdout) => {
                warn!("iw direct scan was aborted; trying unconfirmed cached dump");
                break;
            }
            CommandOutcome::Succeeded { stdout } => {
                return Ok(iw_batch(
                    &stdout,
                    finish_refresh_evidence("iw", refresh_started_at),
                ));
            }
            CommandOutcome::FailedToStart { error } => {
                bail!("failed to run iw scan: {error}");
            }
            CommandOutcome::TimedOut => bail!("iw scan timed out"),
            CommandOutcome::NonZero { stderr } if iw_errno(&stderr) == Some(-libc::ENETDOWN) => {
                bail!("wifi interface {interface} is down");
            }
            CommandOutcome::NonZero { stderr }
                if iw_errno(&stderr) == Some(-libc::EBUSY) && retries_remaining > 0 =>
            {
                retries_remaining -= 1;
                debug!("iw direct scan busy, retrying in 1s ({retries_remaining} retries left)");
                runner.delay(Duration::from_secs(1)).await;
            }
            CommandOutcome::NonZero { stderr } => {
                warn!("iw direct scan failed; trying unconfirmed cached dump: {stderr}");
                break;
            }
        }
    }

    match runner.run(iw_cached_dump_command(interface)).await {
        CommandOutcome::Succeeded { stdout } => Ok(iw_batch(&stdout, None)),
        CommandOutcome::NonZero { stderr } => bail!("iw scan dump failed: {stderr}"),
        CommandOutcome::TimedOut => bail!("iw scan dump timed out"),
        CommandOutcome::FailedToStart { error } => {
            bail!("failed to run iw scan dump: {error}")
        }
    }
}

fn iw_scan_was_aborted(stdout: &str) -> bool {
    // iw exits zero for NL80211_CMD_SCAN_ABORTED and prints this sentinel
    // instead of scan results. It is not a confirmed empty scan.
    stdout.lines().any(|line| line.trim() == "scan aborted!")
}

fn iw_errno(stderr: &str) -> Option<i32> {
    // iw formats nl80211 failures as `<strerror> (<negative errno>)`. Match the
    // numeric errno because strerror wording differs between libc
    // implementations (for example, EBUSY is "Resource busy" on musl).
    stderr.lines().rev().find_map(|line| {
        let open = line.rfind('(')?;
        let close = line[open + 1..].find(')')? + open + 1;
        line[open + 1..close].trim().parse().ok()
    })
}

fn iw_batch(stdout: &str, confirmed_refresh_started_at: Option<BootTime>) -> ScanBatch {
    let mut networks = parse_iw_output(stdout);
    networks.sort_by_key(|network| std::cmp::Reverse(network.signal_dbm));
    ScanBatch {
        networks,
        confirmed_refresh_started_at,
    }
}

/// Parse the result output of `iw dev <iface> scan` or its cached `scan dump`.
pub fn parse_iw_output(output: &str) -> Vec<ScannedNetwork> {
    let mut networks = Vec::new();
    let mut current_bssid: Option<String> = None;
    let mut current_ssid: Option<String> = None;
    let mut current_signal: Option<i32> = None;
    // Raw "DS Parameter set: channel N" number (2.4 GHz in practice), kept only
    // as a fallback for when the frequency line is absent; frequency is
    // canonical for band-qualified derivation (task-0123).
    let mut current_ds_channel: Option<i32> = None;
    let mut current_freq: Option<i32> = None;

    for line in output.lines() {
        let line = line.trim();

        if let Some(rest) = line.strip_prefix("BSS ") {
            // Save previous entry
            if let (Some(bssid), Some(signal)) = (current_bssid.take(), current_signal.take()) {
                let freq = current_freq.take();
                push_validated(
                    &mut networks,
                    &bssid,
                    current_ssid.take(),
                    signal,
                    // iw reports authoritative raw dBm (task-0126).
                    crate::rssi::RssiSource::IwDbm,
                    resolve_channel(freq, current_ds_channel.take()),
                    freq,
                );
            } else {
                current_ssid = None;
                current_ds_channel = None;
                current_freq = None;
            }

            // Extract BSSID from "BSS aa:bb:cc:dd:ee:ff(on wlan0)" or similar
            let bssid_part = rest.split('(').next().unwrap_or(rest).trim();
            current_bssid = Some(bssid_part.to_string());
            current_signal = None;
        } else if let Some(rest) = line.strip_prefix("signal:") {
            // "signal: -65.00 dBm"
            let signal_str = rest.split_whitespace().next().unwrap_or("");
            if let Ok(signal) = signal_str.parse::<f64>() {
                current_signal = Some(signal as i32);
            } else {
                // Malformed signal (e.g. driver prints "signal: unknown dBm").
                // `current_signal` stays None, so this BSS block is dropped at
                // the next `BSS ` boundary. Without this warn the AP vanishes
                // silently and an operator cannot see why. There is one
                // `signal:` line per BSS block, so this is once-per-block within
                // ONE parse — but parse runs every scan cycle in the daemon, so
                // a chronically-broken driver line would spam WARN for the whole
                // process lifetime. First occurrence per BSS warns; repeats are
                // demoted to DEBUG (task-0256).
                let bssid = current_bssid.as_deref().unwrap_or("<unknown>");
                if malformed_signal_warn_is_first(&format!("iw:{bssid}")) {
                    warn!(
                        "iw scan: unparseable signal value in {line:?} for BSS {bssid}; the AP \
                         is dropped unless another signal line in this block parses (further \
                         repeats for this BSS log at DEBUG)"
                    );
                } else {
                    debug!(
                        "iw scan: unparseable signal value in {line:?} for BSS {bssid}; the AP \
                         is dropped (repeat; first occurrence logged at WARN)"
                    );
                }
            }
        } else if let Some(rest) = line.strip_prefix("SSID:") {
            let ssid = rest.trim();
            if !ssid.is_empty() {
                current_ssid = Some(ssid.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("DS Parameter set: channel") {
            if let Ok(ch) = rest.trim().parse::<i32>() {
                current_ds_channel = Some(ch);
            }
        } else if let Some(rest) = line.strip_prefix("freq:") {
            if let Ok(freq) = rest.trim().parse::<i32>() {
                current_freq = Some(freq);
            }
        }
    }

    // Don't forget the last entry
    if let (Some(bssid), Some(signal)) = (current_bssid, current_signal) {
        push_validated(
            &mut networks,
            &bssid,
            current_ssid,
            signal,
            // iw reports authoritative raw dBm (task-0126).
            crate::rssi::RssiSource::IwDbm,
            resolve_channel(current_freq, current_ds_channel),
            current_freq,
        );
    }

    networks
}

/// Normalize BSSID to uppercase, colon-separated.
pub fn normalize_bssid(bssid: &str) -> String {
    bssid.trim().to_uppercase()
}

/// Push a scanned network only if its BSSID is a well-formed IEEE-802 MAC
/// (task-0122 AC#3). A malformed / multicast / broadcast / all-zero BSSID is
/// not a real infrastructure AP; storing it would let garbage propagate into
/// the cache and the trilateration solver, so it is dropped with a warning
/// (fail loud, never silently). A locally-administered (randomized) MAC is a
/// valid unicast address and is KEPT here — a phone hotspot still belongs in a
/// raw scan; the randomized-MAC exclusion happens later, at the trilateration
/// boundary — the inline `Bssid::is_locally_administered()` skip in
/// `server::solve_locate_from_cache` — not at capture.
fn push_validated(
    networks: &mut Vec<ScannedNetwork>,
    bssid: &str,
    ssid: Option<String>,
    signal_dbm: i32,
    rssi_source: crate::rssi::RssiSource,
    channel: Option<Channel>,
    frequency: Option<i32>,
) {
    let normalized = normalize_bssid(bssid);
    match normalized.parse::<crate::bssid::Bssid>() {
        Ok(_) => networks.push(ScannedNetwork {
            bssid: normalized,
            ssid,
            signal_dbm,
            rssi_source,
            channel,
            frequency,
        }),
        Err(e) => warn!("scan: dropping AP with malformed BSSID {bssid:?}: {e}"),
    }
}

/// Resolve a band-qualified [`Channel`] for one scanned AP.
///
/// FREQUENCY IS CANONICAL (task-0123): when present it fully determines
/// `(band, channel)` via [`Channel::from_frequency`], so any scanner-reported
/// bare channel number is ignored in its favour. The explicit number is used
/// ONLY as a fallback when the frequency is absent — and only in the 2.4 GHz
/// range `1..=14`, the one band where a bare number is unambiguous (6 GHz reuses
/// 1.. so a band-less number there cannot be trusted). Outside that, no channel.
fn resolve_channel(frequency: Option<i32>, explicit_channel: Option<i32>) -> Option<Channel> {
    if let Some(freq) = frequency {
        return Channel::from_frequency(freq);
    }
    match explicit_channel {
        Some(n @ 1..=14) => u8::try_from(n).ok().map(Channel::TwoFourGhz),
        _ => None,
    }
}

/// Convert scan results to a ScanSample for the debounce ring buffer.
/// Preserves SSID, channel, and signal for each AP.
pub fn scan_to_sample(networks: &[ScannedNetwork]) -> crate::debounce::ScanSample {
    networks
        .iter()
        .map(|n| {
            (
                n.bssid.clone(),
                crate::debounce::ScanEntry {
                    signal_dbm: n.signal_dbm,
                    rssi_source: n.rssi_source,
                    ssid: n.ssid.clone(),
                    channel: n.channel,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    const NMCLI_NETWORK: &str = "02\\:00\\:00\\:00\\:24\\:10:fixture-nmcli:80:6:2437 MHz\n";
    const NMCLI_POST_NETWORK: &str =
        "02\\:00\\:00\\:00\\:24\\:11:fixture-post-proof:75:11:2462 MHz\n";
    const NM_DEVICE_PATH: &str = "/org/freedesktop/NetworkManager/Devices/42";
    const NM_OWNER: &str = ":1.4242";
    const IW_NETWORK: &str = "BSS 02:00:00:00:24:20(on wlan-test)\n\
        freq: 5180\n\
        signal: -61.00 dBm\n\
        SSID: fixture-iw\n\
        DS Parameter set: channel 36\n";

    enum FakeClockOutcome {
        Time(BootTime),
        Failed,
    }

    struct FakeRunner {
        clocks: VecDeque<FakeClockOutcome>,
        outcomes: VecDeque<CommandOutcome>,
        commands: Vec<ScanCommand>,
        delays: Vec<Duration>,
        timed_delays: bool,
    }

    impl FakeRunner {
        fn new(clocks: Vec<FakeClockOutcome>, outcomes: Vec<CommandOutcome>) -> Self {
            Self {
                clocks: clocks.into(),
                outcomes: outcomes.into(),
                commands: Vec::new(),
                delays: Vec::new(),
                timed_delays: false,
            }
        }

        fn with_timed_delays(mut self) -> Self {
            self.timed_delays = true;
            self
        }

        fn assert_exhausted(&self) {
            assert!(self.clocks.is_empty(), "unused injected clock readings");
            assert!(self.outcomes.is_empty(), "unused injected command outcomes");
        }
    }

    impl ScanCommandRunner for FakeRunner {
        fn boot_time(&mut self) -> std::io::Result<BootTime> {
            match self
                .clocks
                .pop_front()
                .expect("scanner requested an unexpected clock reading")
            {
                FakeClockOutcome::Time(value) => Ok(value),
                FakeClockOutcome::Failed => {
                    Err(std::io::Error::other("injected CLOCK_BOOTTIME failure"))
                }
            }
        }

        fn run<'a>(&'a mut self, command: ScanCommand) -> CommandFuture<'a> {
            self.commands.push(command);
            let outcome = self
                .outcomes
                .pop_front()
                .expect("scanner ran an unexpected command");
            Box::pin(async move { outcome })
        }

        fn delay<'a>(&'a mut self, duration: Duration) -> DelayFuture<'a> {
            self.delays.push(duration);
            if self.timed_delays {
                Box::pin(sleep(duration))
            } else {
                Box::pin(async {})
            }
        }
    }

    struct HangingForcedRunner {
        commands: Vec<ScanCommand>,
    }

    impl ScanCommandRunner for HangingForcedRunner {
        fn boot_time(&mut self) -> std::io::Result<BootTime> {
            Ok(BootTime::from_duration(Duration::from_secs(100)))
        }

        fn run<'a>(&'a mut self, command: ScanCommand) -> CommandFuture<'a> {
            let is_forced = command.program == "nmcli"
                && command.args.last().map(String::as_str) == Some("yes");
            let is_post =
                command.program == "nmcli" && command.args.last().map(String::as_str) == Some("no");
            let is_owner =
                command.program == "busctl" && command.args.iter().any(|arg| arg == "GetNameOwner");
            let is_path = command.program == "busctl"
                && command.args.iter().any(|arg| arg == "GetDeviceByIpIface");
            self.commands.push(command);

            if is_forced {
                Box::pin(std::future::pending())
            } else if is_post {
                Box::pin(async { succeeded(NMCLI_POST_NETWORK) })
            } else if is_owner {
                Box::pin(async { owner_reply() })
            } else if is_path {
                Box::pin(async { device_path_reply() })
            } else {
                Box::pin(async { last_scan_reply(99_000) })
            }
        }

        fn delay<'a>(&'a mut self, _duration: Duration) -> DelayFuture<'a> {
            Box::pin(async {})
        }
    }

    fn at(seconds: u64) -> FakeClockOutcome {
        FakeClockOutcome::Time(BootTime::from_duration(Duration::from_secs(seconds)))
    }

    fn at_millis(milliseconds: u64) -> FakeClockOutcome {
        FakeClockOutcome::Time(BootTime::from_duration(Duration::from_millis(milliseconds)))
    }

    fn succeeded(stdout: &str) -> CommandOutcome {
        CommandOutcome::Succeeded {
            stdout: stdout.to_owned(),
        }
    }

    fn failed(stderr: &str) -> CommandOutcome {
        CommandOutcome::NonZero {
            stderr: stderr.to_owned(),
        }
    }

    fn device_path_reply() -> CommandOutcome {
        succeeded(r#"{"type":"o","data":["/org/freedesktop/NetworkManager/Devices/42"]}"#)
    }

    fn owner_reply() -> CommandOutcome {
        succeeded(r#"{"type":"s","data":[":1.4242"]}"#)
    }

    fn last_scan_reply(milliseconds: i64) -> CommandOutcome {
        succeeded(&format!(r#"{{"type":"x","data":{milliseconds}}}"#))
    }

    fn assert_production_command_contract(command: &ScanCommand) {
        assert!(
            command.kill_on_drop,
            "every scanner subprocess must die when cancellation drops its future"
        );
    }

    fn assert_literal_nmcli_command(command: &ScanCommand, rescan: &str) {
        assert_eq!(command.program, "nmcli");
        assert_eq!(
            command.args,
            [
                "-t",
                "-f",
                "BSSID,SSID,SIGNAL,CHAN,FREQ",
                "device",
                "wifi",
                "list",
                "ifname",
                "wlan-test",
                "--rescan",
                rescan,
            ]
        );
        assert!(command.locale_c);
        assert_eq!(
            command.timeout,
            if rescan == "yes" {
                NM_FORCED_COMMAND_TIMEOUT
            } else {
                SCAN_COMMAND_TIMEOUT
            }
        );
        assert_production_command_contract(command);
    }

    fn assert_literal_iw_direct_command(command: &ScanCommand) {
        assert_eq!(command.program, "iw");
        assert_eq!(command.args, ["dev", "wlan-test", "scan"]);
        assert!(!command.locale_c);
        assert_eq!(command.timeout, SCAN_COMMAND_TIMEOUT);
        assert_production_command_contract(command);
    }

    fn assert_literal_iw_dump_command(command: &ScanCommand) {
        assert_eq!(command.program, "iw");
        assert_eq!(command.args, ["dev", "wlan-test", "scan", "dump"]);
        assert!(!command.locale_c);
        assert_eq!(command.timeout, SCAN_COMMAND_TIMEOUT);
        assert_production_command_contract(command);
    }

    fn assert_literal_nm_device_path_command(command: &ScanCommand) {
        assert_eq!(command.program, "busctl");
        assert_eq!(
            command.args,
            [
                "--json=short",
                "--system",
                "call",
                ":1.4242",
                "/org/freedesktop/NetworkManager",
                "org.freedesktop.NetworkManager",
                "GetDeviceByIpIface",
                "s",
                "wlan-test",
            ]
        );
        assert!(!command.locale_c);
        assert_eq!(command.timeout, SCAN_COMMAND_TIMEOUT);
        assert_production_command_contract(command);
    }

    fn assert_literal_nm_owner_command(command: &ScanCommand) {
        assert_eq!(command.program, "busctl");
        assert_eq!(
            command.args,
            [
                "--json=short",
                "--system",
                "call",
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "GetNameOwner",
                "s",
                "org.freedesktop.NetworkManager",
            ]
        );
        assert!(!command.locale_c);
        assert_eq!(command.timeout, SCAN_COMMAND_TIMEOUT);
        assert_production_command_contract(command);
    }

    fn assert_literal_nm_last_scan_command(command: &ScanCommand) {
        assert_eq!(command.program, "busctl");
        assert_eq!(
            command.args,
            [
                "--json=short",
                "--system",
                "get-property",
                ":1.4242",
                "/org/freedesktop/NetworkManager/Devices/42",
                "org.freedesktop.NetworkManager.Device.Wireless",
                "LastScan",
            ]
        );
        assert!(!command.locale_c);
        assert_eq!(command.timeout, SCAN_COMMAND_TIMEOUT);
        assert_production_command_contract(command);
    }

    #[test]
    fn networkmanager_busctl_json_and_generation_rules_are_strict() {
        assert_eq!(
            parse_nm_owner(r#"{"type":"s","data":[":1.4242"]}"#).unwrap(),
            NM_OWNER
        );
        assert!(
            parse_nm_owner(r#"{"type":"s","data":["org.freedesktop.NetworkManager"]}"#).is_err()
        );
        assert_eq!(
            parse_nm_device_path(
                r#"{"type":"o","data":["/org/freedesktop/NetworkManager/Devices/42"]}"#
            )
            .unwrap(),
            NM_DEVICE_PATH
        );
        assert_eq!(
            parse_nm_last_scan(r#"{"type":"x","data":12345}"#).unwrap(),
            12_345
        );
        assert!(parse_nm_device_path(r#"{"type":"s","data":["wrong"]}"#).is_err());
        assert!(parse_nm_last_scan(r#"{"type":"u","data":12345}"#).is_err());
        assert!(parse_nm_last_scan(r#"{"type":"x","data":18446744073709551615}"#).is_err());
        assert!(parse_nm_last_scan("not json").is_err());

        let start = BootTime::from_duration(Duration::from_millis(10_000));
        let observed = BootTime::from_duration(Duration::from_millis(10_001));
        assert!(!nm_generation_confirms_refresh(
            9_000, 9_000, start, observed
        ));
        assert!(nm_generation_confirms_refresh(
            9_000, 10_000, start, observed
        ));
        let submillisecond_start =
            BootTime::from_duration(Duration::from_millis(10_000) + Duration::from_nanos(1));
        assert!(!nm_generation_confirms_refresh(
            9_999,
            10_000,
            submillisecond_start,
            observed,
        ));
        assert!(nm_generation_confirms_refresh(
            9_999,
            10_001,
            submillisecond_start,
            observed,
        ));
        assert!(!nm_generation_confirms_refresh(-1, -1, start, observed));
        assert!(!nm_generation_confirms_refresh(-1, 10_001, start, observed));
        assert!(!nm_generation_confirms_refresh(
            9_000, 10_002, start, observed
        ));
    }

    #[tokio::test]
    async fn nmcli_generation_proof_relists_and_returns_post_completion_rows() {
        let stamp = BootTime::from_duration(Duration::from_secs(10));
        let mut runner = FakeRunner::new(
            vec![at(10), at_millis(10_001), at_millis(10_002)],
            vec![
                owner_reply(),
                device_path_reply(),
                last_scan_reply(9_000),
                succeeded(NMCLI_NETWORK),
                last_scan_reply(10_001),
                succeeded(NMCLI_POST_NETWORK),
                owner_reply(),
                device_path_reply(),
                last_scan_reply(10_001),
                owner_reply(),
            ],
        );

        let batch = wifi_scan_with_runner("wlan-test", &mut runner)
            .await
            .expect("proved nmcli generation should succeed");

        assert_eq!(batch.networks.len(), 1);
        assert_eq!(
            batch.networks[0].bssid, "02:00:00:00:24:11",
            "forced-command rows must be discarded in favor of the post-proof list"
        );
        assert_eq!(batch.confirmed_refresh_started_at, Some(stamp));
        assert_eq!(
            runner.commands,
            vec![
                nm_owner_command(),
                nm_device_path_command(NM_OWNER, "wlan-test"),
                nm_last_scan_command(NM_OWNER, NM_DEVICE_PATH),
                nmcli_list_command("wlan-test", true),
                nm_last_scan_command(NM_OWNER, NM_DEVICE_PATH),
                nmcli_list_command("wlan-test", false),
                nm_owner_command(),
                nm_device_path_command(NM_OWNER, "wlan-test"),
                nm_last_scan_command(NM_OWNER, NM_DEVICE_PATH),
                nm_owner_command(),
            ]
        );
        assert_literal_nm_owner_command(&runner.commands[0]);
        assert_literal_nm_device_path_command(&runner.commands[1]);
        assert_literal_nm_last_scan_command(&runner.commands[2]);
        assert_literal_nmcli_command(&runner.commands[3], "yes");
        assert_literal_nm_last_scan_command(&runner.commands[4]);
        assert_literal_nmcli_command(&runner.commands[5], "no");
        assert_literal_nm_owner_command(&runner.commands[6]);
        assert_literal_nm_device_path_command(&runner.commands[7]);
        assert_literal_nm_last_scan_command(&runner.commands[8]);
        assert_literal_nm_owner_command(&runner.commands[9]);
        runner.assert_exhausted();
    }

    #[tokio::test]
    async fn nmcli_unset_baseline_cannot_mistake_scan_start_update_for_completion() {
        let mut runner = FakeRunner::new(
            vec![at(25)],
            vec![
                owner_reply(),
                device_path_reply(),
                last_scan_reply(-1),
                succeeded(NMCLI_NETWORK),
                // NetworkManager can update -1 at scan start; this cycle must
                // not poll or accept that transition as completion proof.
                succeeded(NMCLI_POST_NETWORK),
            ],
        );

        let batch = wifi_scan_nmcli("wlan-test", &mut runner)
            .await
            .expect("post-list should remain readable")
            .expect("non-empty post-list should be retained");

        assert_eq!(batch.confirmed_refresh_started_at, None);
        assert_eq!(runner.commands.len(), 5);
        assert!(runner.delays.is_empty());
        runner.assert_exhausted();
    }

    #[tokio::test]
    async fn nmcli_proved_generation_preserves_confirmed_empty_post_list_without_iw() {
        let stamp = BootTime::from_duration(Duration::from_secs(11));
        let mut runner = FakeRunner::new(
            vec![at(11), at_millis(11_001), at_millis(11_002)],
            vec![
                owner_reply(),
                device_path_reply(),
                last_scan_reply(10_000),
                succeeded(NMCLI_NETWORK),
                last_scan_reply(11_001),
                succeeded(""),
                owner_reply(),
                device_path_reply(),
                last_scan_reply(11_001),
                owner_reply(),
            ],
        );

        let batch = wifi_scan_with_runner("wlan-test", &mut runner)
            .await
            .expect("proved empty nmcli generation should be publishable");

        assert!(batch.networks.is_empty());
        assert_eq!(batch.confirmed_refresh_started_at, Some(stamp));
        assert_eq!(
            runner.commands.len(),
            10,
            "confirmed empty must not reach iw"
        );
        runner.assert_exhausted();
    }

    #[tokio::test(start_paused = true)]
    async fn nmcli_zero_exit_with_stale_last_scan_polls_to_deadline_unconfirmed() {
        let timing = NmRefreshTiming {
            deadline: Duration::from_millis(250),
            poll_interval: Duration::from_millis(100),
        };
        let mut runner = FakeRunner::new(
            vec![at(10), at(10), at(10), at(10)],
            vec![
                owner_reply(),
                device_path_reply(),
                last_scan_reply(9_000),
                succeeded(NMCLI_NETWORK),
                last_scan_reply(9_000),
                last_scan_reply(9_000),
                last_scan_reply(9_000),
                succeeded(NMCLI_POST_NETWORK),
            ],
        )
        .with_timed_delays();
        let started = TokioInstant::now();

        let batch = wifi_scan_nmcli_with_timing("wlan-test", &mut runner, timing)
            .await
            .expect("post-list remains useful interactively")
            .expect("non-empty post-list should be retained");

        assert_eq!(batch.networks[0].bssid, "02:00:00:00:24:11");
        assert_eq!(batch.confirmed_refresh_started_at, None);
        assert_eq!(
            runner.delays,
            vec![Duration::from_millis(100); 3],
            "the third delay is cancelled by the shared deadline"
        );
        assert_eq!(TokioInstant::now() - started, timing.deadline);
        assert_eq!(runner.commands.len(), 8);
        assert_literal_nmcli_command(runner.commands.last().unwrap(), "no");
        runner.assert_exhausted();
    }

    #[tokio::test]
    async fn nmcli_future_last_scan_is_unconfirmed_and_skips_final_fence() {
        let mut runner = FakeRunner::new(
            vec![at(10), at_millis(10_001)],
            vec![
                owner_reply(),
                device_path_reply(),
                last_scan_reply(9_000),
                succeeded(NMCLI_NETWORK),
                last_scan_reply(10_002),
                succeeded(NMCLI_POST_NETWORK),
            ],
        );

        let batch = wifi_scan_nmcli("wlan-test", &mut runner)
            .await
            .expect("post-list remains useful after impossible property value")
            .expect("non-empty post-list should be retained");

        assert_eq!(batch.confirmed_refresh_started_at, None);
        assert_eq!(
            runner.commands.len(),
            6,
            "future value must not reach final fence"
        );
        runner.assert_exhausted();
    }

    #[tokio::test]
    async fn nmcli_generation_poll_continues_beyond_five_reads_until_later_advance() {
        let stamp = BootTime::from_duration(Duration::from_secs(20));
        let mut clocks = vec![at(20)];
        clocks.extend((0..6).map(|_| at(20)));
        clocks.extend([at_millis(20_001), at_millis(20_002)]);
        let mut outcomes = vec![
            owner_reply(),
            device_path_reply(),
            last_scan_reply(19_000),
            succeeded(NMCLI_NETWORK),
        ];
        outcomes.extend((0..6).map(|_| last_scan_reply(19_000)));
        outcomes.extend([
            last_scan_reply(20_001),
            succeeded(NMCLI_POST_NETWORK),
            owner_reply(),
            device_path_reply(),
            last_scan_reply(20_001),
            owner_reply(),
        ]);
        let mut runner = FakeRunner::new(clocks, outcomes);

        let batch = wifi_scan_nmcli("wlan-test", &mut runner)
            .await
            .expect("later generation should verify")
            .expect("post-list should be returned");

        assert_eq!(batch.confirmed_refresh_started_at, Some(stamp));
        assert_eq!(
            runner.delays,
            vec![NM_LAST_SCAN_POLL_INTERVAL; 6],
            "polling must not stop at the former fixed five-read cap"
        );
        runner.assert_exhausted();
    }

    #[tokio::test]
    async fn nmcli_final_owner_path_or_generation_fence_downgrades_post_list() {
        let changed_owner_reply = succeeded(r#"{"type":"s","data":[":1.9999"]}"#);
        let changed_path_reply =
            succeeded(r#"{"type":"o","data":["/org/freedesktop/NetworkManager/Devices/43"]}"#);
        let scenarios = vec![
            (vec![changed_owner_reply], vec![at(90), at_millis(90_001)]),
            (
                vec![owner_reply(), changed_path_reply],
                vec![at(90), at_millis(90_001)],
            ),
            (
                vec![owner_reply(), device_path_reply(), last_scan_reply(89_999)],
                vec![at(90), at_millis(90_001), at_millis(90_002)],
            ),
            (
                vec![
                    owner_reply(),
                    device_path_reply(),
                    failed("final LastScan unavailable"),
                ],
                vec![at(90), at_millis(90_001)],
            ),
            (
                vec![
                    owner_reply(),
                    device_path_reply(),
                    last_scan_reply(90_001),
                    succeeded(r#"{"type":"s","data":[":1.9999"]}"#),
                ],
                vec![at(90), at_millis(90_001), at_millis(90_002)],
            ),
        ];

        for (final_checks, clocks) in scenarios {
            let mut outcomes = vec![
                owner_reply(),
                device_path_reply(),
                last_scan_reply(89_000),
                succeeded(NMCLI_NETWORK),
                last_scan_reply(90_001),
                succeeded(NMCLI_POST_NETWORK),
            ];
            outcomes.extend(final_checks);
            let mut runner = FakeRunner::new(clocks, outcomes);

            let batch = wifi_scan_nmcli("wlan-test", &mut runner)
                .await
                .expect("post-list should survive final-fence failure")
                .expect("non-empty post-list should be retained");

            assert_eq!(batch.networks[0].bssid, "02:00:00:00:24:11");
            assert_eq!(batch.confirmed_refresh_started_at, None);
            runner.assert_exhausted();
        }
    }

    #[tokio::test]
    async fn nmcli_forced_failures_use_only_separate_unconfirmed_post_list() {
        let failures = [
            failed("forced scan rejected"),
            CommandOutcome::TimedOut,
            CommandOutcome::FailedToStart {
                error: "missing nmcli".to_owned(),
            },
        ];

        for forced_failure in failures {
            let mut runner = FakeRunner::new(
                vec![at(30)],
                vec![
                    owner_reply(),
                    device_path_reply(),
                    last_scan_reply(29_000),
                    forced_failure,
                    succeeded(NMCLI_POST_NETWORK),
                ],
            );
            let batch = wifi_scan_nmcli("wlan-test", &mut runner)
                .await
                .expect("cached nmcli list should remain readable")
                .expect("non-empty cache should be returned");

            assert_eq!(batch.networks.len(), 1);
            assert_eq!(batch.confirmed_refresh_started_at, None);
            assert_eq!(
                runner.commands.len(),
                5,
                "failed force must not poll LastScan"
            );
            assert_literal_nmcli_command(&runner.commands[3], "yes");
            assert_literal_nmcli_command(&runner.commands[4], "no");
            runner.assert_exhausted();
        }
    }

    #[tokio::test]
    async fn nmcli_owner_path_and_property_failures_never_stamp() {
        let scenarios = vec![
            vec![
                failed("owner unavailable"),
                succeeded(NMCLI_NETWORK),
                succeeded(NMCLI_POST_NETWORK),
            ],
            vec![
                owner_reply(),
                failed("device absent"),
                succeeded(NMCLI_NETWORK),
                succeeded(NMCLI_POST_NETWORK),
            ],
            vec![
                owner_reply(),
                device_path_reply(),
                failed("LastScan unavailable"),
                succeeded(NMCLI_NETWORK),
                succeeded(NMCLI_POST_NETWORK),
            ],
            vec![
                owner_reply(),
                device_path_reply(),
                last_scan_reply(39_000),
                succeeded(NMCLI_NETWORK),
                failed("LastScan disappeared"),
                succeeded(NMCLI_POST_NETWORK),
            ],
        ];

        for outcomes in scenarios {
            let mut runner = FakeRunner::new(vec![at(40)], outcomes);
            let batch = wifi_scan_nmcli("wlan-test", &mut runner)
                .await
                .expect("readable post-list should survive verification failure")
                .expect("non-empty post-list should be returned");

            assert_eq!(batch.networks[0].bssid, "02:00:00:00:24:11");
            assert_eq!(batch.confirmed_refresh_started_at, None);
            assert_literal_nmcli_command(runner.commands.last().unwrap(), "no");
            runner.assert_exhausted();
        }
    }

    #[tokio::test]
    async fn nmcli_post_list_failure_downgrades_readable_forced_rows() {
        let mut runner = FakeRunner::new(
            vec![at(50), at_millis(50_001)],
            vec![
                owner_reply(),
                device_path_reply(),
                last_scan_reply(49_000),
                succeeded(NMCLI_NETWORK),
                last_scan_reply(50_001),
                CommandOutcome::TimedOut,
            ],
        );

        let batch = wifi_scan_nmcli("wlan-test", &mut runner)
            .await
            .expect("forced rows remain useful after post-list failure")
            .expect("non-empty forced rows should be retained");

        assert_eq!(batch.networks[0].bssid, "02:00:00:00:24:10");
        assert_eq!(
            batch.confirmed_refresh_started_at, None,
            "rows not re-listed after completion must never inherit proof"
        );
        runner.assert_exhausted();
    }

    #[tokio::test]
    async fn nmcli_forced_and_post_list_failure_is_total_for_nmcli_backend() {
        let mut runner = FakeRunner::new(
            vec![at(60)],
            vec![
                owner_reply(),
                device_path_reply(),
                last_scan_reply(59_000),
                failed("forced failed"),
                CommandOutcome::TimedOut,
            ],
        );

        let error = wifi_scan_nmcli("wlan-test", &mut runner)
            .await
            .expect_err("unavailable forced and cached commands must be an error");

        assert!(error
            .to_string()
            .contains("post-refresh nmcli cached list timed out"));
        runner.assert_exhausted();
    }

    #[tokio::test(start_paused = true)]
    async fn nmcli_forced_and_poll_share_one_bounded_refresh_deadline() {
        let mut runner = HangingForcedRunner {
            commands: Vec::new(),
        };
        let started = TokioInstant::now();

        let batch = wifi_scan_nmcli("wlan-test", &mut runner)
            .await
            .expect("post-deadline cached list should remain readable")
            .expect("non-empty post-list should be returned");

        assert_eq!(TokioInstant::now() - started, NM_REFRESH_DEADLINE);
        assert_eq!(batch.confirmed_refresh_started_at, None);
        assert_eq!(runner.commands.len(), 5);
        assert_literal_nmcli_command(&runner.commands[3], "yes");
        assert_literal_nmcli_command(&runner.commands[4], "no");
    }

    #[tokio::test]
    async fn empty_or_unavailable_unconfirmed_nmcli_post_list_falls_through_to_iw() {
        for post_outcome in [succeeded(""), CommandOutcome::TimedOut] {
            let iw_stamp = BootTime::from_duration(Duration::from_secs(71));
            let mut runner = FakeRunner::new(
                vec![at(70), at(71)],
                vec![
                    owner_reply(),
                    device_path_reply(),
                    last_scan_reply(69_000),
                    failed("forced failed"),
                    post_outcome,
                    succeeded(IW_NETWORK),
                ],
            );

            let batch = wifi_scan_with_runner("wlan-test", &mut runner)
                .await
                .expect("iw should recover from unusable nmcli output");

            assert_eq!(batch.networks[0].bssid, "02:00:00:00:24:20");
            assert_eq!(batch.confirmed_refresh_started_at, Some(iw_stamp));
            assert_literal_iw_direct_command(runner.commands.last().unwrap());
            runner.assert_exhausted();
        }
    }

    #[tokio::test]
    async fn unavailable_nmcli_and_total_iw_failure_return_total_scanner_error() {
        let mut runner = FakeRunner::new(
            vec![at(80), at(81)],
            vec![
                owner_reply(),
                device_path_reply(),
                last_scan_reply(79_000),
                failed("forced failed"),
                CommandOutcome::FailedToStart {
                    error: "missing nmcli".to_owned(),
                },
                CommandOutcome::TimedOut,
            ],
        );

        let error = wifi_scan_with_runner("wlan-test", &mut runner)
            .await
            .expect_err("no readable backend result must publish nothing");

        assert!(error.to_string().contains("iw scan timed out"));
        assert_literal_iw_direct_command(runner.commands.last().unwrap());
        runner.assert_exhausted();
    }

    #[tokio::test]
    async fn successful_iw_direct_scan_returns_its_nonempty_or_empty_output() {
        for (stdout, expected_networks, seconds) in [(IW_NETWORK, 1, 50), ("", 0, 51)] {
            let stamp = BootTime::from_duration(Duration::from_secs(seconds));
            let mut runner = FakeRunner::new(vec![at(seconds)], vec![succeeded(stdout)]);

            let batch = wifi_scan_iw("wlan-test", &mut runner)
                .await
                .expect("blocking iw direct scan should succeed");

            assert_eq!(batch.networks.len(), expected_networks);
            assert_eq!(batch.confirmed_refresh_started_at, Some(stamp));
            assert_eq!(
                runner.commands,
                vec![iw_direct_scan_command("wlan-test")],
                "successful direct output, including empty, must not use scan dump"
            );
            assert_literal_iw_direct_command(&runner.commands[0]);
            runner.assert_exhausted();
        }
    }

    #[tokio::test]
    async fn iw_numeric_busy_errno_handles_musl_wording_and_uses_later_attempt_stamp() {
        let later = BootTime::from_duration(Duration::from_secs(61));
        let mut runner = FakeRunner::new(
            vec![at(60), at(61)],
            vec![
                failed("command failed: Resource busy (-16)"),
                succeeded(IW_NETWORK),
            ],
        );

        let batch = wifi_scan_iw("wlan-test", &mut runner)
            .await
            .expect("second direct scan should succeed");

        assert_eq!(batch.confirmed_refresh_started_at, Some(later));
        assert_eq!(
            runner.commands,
            vec![
                iw_direct_scan_command("wlan-test"),
                iw_direct_scan_command("wlan-test"),
            ]
        );
        assert_eq!(runner.delays, vec![Duration::from_secs(1)]);
        runner.assert_exhausted();
    }

    #[tokio::test]
    async fn exhausted_iw_busy_retries_use_unconfirmed_cached_dump() {
        let mut runner = FakeRunner::new(
            vec![at(70), at(71), at(72), at(73)],
            vec![
                failed("Device or resource busy (-16)"),
                failed("Resource busy (-16)"),
                failed("another libc translation (-16)"),
                failed("EBUSY (-16)"),
                succeeded(IW_NETWORK),
            ],
        );

        let batch = wifi_scan_iw("wlan-test", &mut runner)
            .await
            .expect("cached iw dump should remain readable");

        assert_eq!(batch.networks.len(), 1);
        assert_eq!(batch.confirmed_refresh_started_at, None);
        assert_eq!(runner.commands.len(), IW_BUSY_RETRIES + 2);
        assert!(runner.commands[..=IW_BUSY_RETRIES]
            .iter()
            .all(|command| command == &iw_direct_scan_command("wlan-test")));
        assert_eq!(
            runner.commands.last(),
            Some(&iw_cached_dump_command("wlan-test"))
        );
        assert_literal_iw_direct_command(&runner.commands[0]);
        assert_literal_iw_dump_command(runner.commands.last().unwrap());
        assert_eq!(runner.delays, vec![Duration::from_secs(1); IW_BUSY_RETRIES]);
        runner.assert_exhausted();
    }

    #[tokio::test]
    async fn generic_iw_failure_may_use_unconfirmed_cached_dump() {
        let mut runner = FakeRunner::new(
            vec![at(80)],
            vec![failed("Operation not permitted"), succeeded(IW_NETWORK)],
        );

        let batch = wifi_scan_iw("wlan-test", &mut runner)
            .await
            .expect("generic failure should preserve cached interactive continuity");

        assert_eq!(batch.networks.len(), 1);
        assert_eq!(batch.confirmed_refresh_started_at, None);
        assert_eq!(
            runner.commands,
            vec![
                iw_direct_scan_command("wlan-test"),
                iw_cached_dump_command("wlan-test"),
            ]
        );
        assert_literal_iw_direct_command(&runner.commands[0]);
        assert_literal_iw_dump_command(&runner.commands[1]);
        runner.assert_exhausted();
    }

    #[tokio::test]
    async fn iw_zero_exit_abort_sentinel_is_not_a_confirmed_empty_scan() {
        let mut runner = FakeRunner::new(
            vec![at(85)],
            vec![succeeded("scan aborted!\n"), succeeded(IW_NETWORK)],
        );

        let batch = wifi_scan_iw("wlan-test", &mut runner)
            .await
            .expect("aborted iw scan may retain readable cached output");

        assert_eq!(batch.networks.len(), 1);
        assert_eq!(batch.confirmed_refresh_started_at, None);
        assert_eq!(
            runner.commands,
            vec![
                iw_direct_scan_command("wlan-test"),
                iw_cached_dump_command("wlan-test"),
            ]
        );
        runner.assert_exhausted();
    }

    #[tokio::test]
    async fn iw_spawn_timeout_and_network_down_are_total_without_cached_dump() {
        let failures = [
            CommandOutcome::FailedToStart {
                error: "missing iw".to_owned(),
            },
            CommandOutcome::TimedOut,
            failed("command failed: interface unavailable (-100)"),
        ];

        for direct_failure in failures {
            let mut runner = FakeRunner::new(vec![at(90)], vec![direct_failure]);
            let error = wifi_scan_iw("wlan-test", &mut runner)
                .await
                .expect_err("failure must publish nothing");

            assert!(!error.to_string().is_empty());
            assert_eq!(
                runner.commands,
                vec![iw_direct_scan_command("wlan-test")],
                "total direct-scan failures must not read or publish stale cache"
            );
            runner.assert_exhausted();
        }
    }

    #[tokio::test]
    async fn clock_failure_leaves_post_list_unconfirmed_without_iw_fallback() {
        let mut runner = FakeRunner::new(
            vec![FakeClockOutcome::Failed],
            vec![
                owner_reply(),
                device_path_reply(),
                last_scan_reply(99_000),
                succeeded(NMCLI_NETWORK),
                succeeded(NMCLI_POST_NETWORK),
            ],
        );

        let batch = wifi_scan_with_runner("wlan-test", &mut runner)
            .await
            .expect("readable post-list remains useful interactively");

        assert_eq!(batch.networks.len(), 1);
        assert_eq!(batch.networks[0].bssid, "02:00:00:00:24:11");
        assert_eq!(batch.confirmed_refresh_started_at, None);
        assert_eq!(runner.commands.len(), 5);
        assert_literal_nmcli_command(&runner.commands[3], "yes");
        assert_literal_nmcli_command(&runner.commands[4], "no");
        runner.assert_exhausted();
    }

    // FLAVOR DEPENDENCY: this test relies on `#[tokio::test]` defaulting to the
    // CURRENT-THREAD runtime. Two sleeps below are intentionally different and
    // must NOT be unified: phase 1 (wait for the child's pid) uses async
    // `sleep().await` because it shares the single runtime thread with the
    // child-spawning task and must yield for it to start; phase 2 (verify the
    // child died) uses BLOCKING `std::thread::sleep` so its verdict is
    // independent of the (load-starved) runtime AND so the runtime cannot reap
    // the child mid-check (see phase-2 INVARIANT). Switching to
    // `flavor = "multi_thread"` invalidates BOTH rationales — don't, without
    // re-deriving them (task-0249).
    #[tokio::test]
    async fn tokio_runner_kills_child_when_scan_future_is_cancelled() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("wall clock should be after Unix epoch")
            .as_nanos();
        let pid_file = std::env::temp_dir().join(format!(
            "whereamid-scanner-cancel-{}-{unique}",
            std::process::id()
        ));
        let script = format!("printf '%s' $$ > {}; exec sleep 60", pid_file.display());
        let command = ScanCommand::new("sh", ["-c", script.as_str()]);
        assert_production_command_contract(&command);

        let task = tokio::spawn(async move {
            let mut runner = TokioScanCommandRunner;
            runner.run(command).await
        });
        // The child writes its PID via `printf '%s' $$ > pid_file`. The shell's
        // `>` redirection TRUNCATES/creates the file BEFORE `printf` writes, so a
        // read that wins the race sees an empty (or, defensively, partial) file.
        // Treat "exists but not yet a valid PID" as not-ready and keep polling —
        // do NOT `expect`-panic on it (that was a second, rarer flake exposed
        // once the reap flake below was fixed, task-0249). Budget widened to 10s
        // for headroom under parallel-load starvation. Async sleep is required:
        // this poll loop shares the single current-thread runtime with the
        // child-spawning task, so it must yield for the child to start at all.
        let child_pid = timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(contents) = std::fs::read_to_string(&pid_file) {
                    if let Ok(pid) = contents.trim().parse::<u32>() {
                        break pid;
                    }
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("scanner child did not start and write its PID in time");

        task.abort();
        let aborted = task.await.expect_err("scanner future should be cancelled");
        assert!(aborted.is_cancelled());

        // Once the aborted task's future (and its tokio Child) is dropped,
        // kill_on_drop delivers SIGKILL SYNCHRONOUSLY (tokio 1.52
        // ChildDropGuard::drop -> Kill::kill; verified against source). The
        // kernel turns the child into a zombie near-instantly. `/proc/<pid>`
        // then lingers only until tokio's BEST-EFFORT async reaper runs waitpid
        // (documented as best-effort). Under parallel `cargo test` load the
        // current-thread runtime — and thus that reaper — can be starved for
        // many seconds, so the OLD check (poll `/proc/<pid>` existence via a
        // tokio `timeout`) misread a correctly-killed-but-unreaped zombie as
        // "still alive" and also depended on the very runtime that was starved
        // (task-0249). We instead assert the child is DEAD — gone OR a zombie —
        // rather than fully reaped, and poll with BLOCKING std + a wall-clock
        // deadline so the verdict does not depend on the async runtime at all.
        // Kernel SIGKILL teardown is not load-sensitive, so the generous budget
        // is only ever consumed on a genuine "still running" regression.
        //
        // INVARIANT — this loop MUST NOT `.await`/yield. Blocking the (already
        // idle, post-abort) current-thread runtime here is deliberate: it keeps
        // tokio's async reaper from running, so the killed child stays an
        // un-reaped zombie, its pid is never freed, and `child_is_dead` can never
        // read a REUSED pid's live process as "our child still alive". If this
        // loop ever yields, a concurrent forking test could reuse the reaped pid
        // and the safety-net `libc::kill` below would then target a stranger.
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let killed = loop {
            if child_is_dead(child_pid) {
                break true;
            }
            if std::time::Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let _ = std::fs::remove_file(&pid_file);
        if !killed {
            // Genuine regression: the child is still a live process. Reap the
            // leaked fixture so the failing test does not orphan a `sleep 60`.
            // SAFETY: `child_pid` came from the exact child this test spawned.
            unsafe {
                libc::kill(child_pid as libc::pid_t, libc::SIGKILL);
            }
            panic!("cancelled scanner child {child_pid} remained a live process");
        }
    }

    /// Whether `pid` is no longer a live process — i.e. fully reaped (gone), a
    /// zombie awaiting reap ('Z'), or in the transient dead states ('X'/'x'). A
    /// zombie/dead process counts as killed: SIGKILL has already torn it down,
    /// only the `waitpid` bookkeeping is pending. Reads `/proc/<pid>/stat` with
    /// blocking std so the verdict does not depend on any async runtime (which is
    /// what the de-flake requires under load). Linux-only, which is fine — the
    /// whole path is `/proc`-bound.
    ///
    /// NOTE(pid-reuse): a reaped pid reused by another concurrent process would
    /// read as "alive" here. In this test that cannot happen: the phase-2 loop
    /// blocks the only runtime that could reap this child (see its INVARIANT), so
    /// the pid is never freed while we poll. The safety only holds as long as
    /// that loop stays yield-free.
    fn child_is_dead(pid: u32) -> bool {
        let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(contents) => contents,
            // No stat file => already reaped (or never existed) => dead.
            Err(_) => return true,
        };
        // `/proc/<pid>/stat` is "<pid> (<comm>) <state> ...". `comm` may itself
        // contain spaces and ')', so the state char is the first token AFTER the
        // final ')'. 'Z' = zombie; 'X'/'x' = dead (both mean killed). Any other
        // (or unparseable) state is treated as still-alive — conservative: never
        // certify a kill we cannot confirm.
        let state = stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().next())
            .and_then(|token| token.chars().next());
        matches!(state, Some('Z' | 'X' | 'x'))
    }

    fn patterns(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn mobile_movia_glob_is_anchored_and_case_insensitive() {
        let denylist = patterns(&["MOVIA*"]);
        assert!(is_mobile_ssid(Some("MOVIA_BUS_42"), &denylist));
        assert!(is_mobile_ssid(Some("movia-night"), &denylist));
        assert!(!is_mobile_ssid(Some("mymoviabus"), &denylist));
    }

    #[test]
    fn mobile_question_mark_matches_one_unicode_scalar() {
        let denylist = patterns(&["Bus_?"]);
        assert!(is_mobile_ssid(Some("bus_4"), &denylist));
        assert!(is_mobile_ssid(Some("BUS_å"), &denylist));
        assert!(!is_mobile_ssid(Some("Bus_42"), &denylist));
        assert!(!is_mobile_ssid(Some("Bus_"), &denylist));
    }

    #[test]
    fn mobile_missing_empty_and_hidden_are_never_flagged() {
        let denylist = patterns(&["*"]);
        assert!(!is_mobile_ssid(None, &denylist));
        assert!(!is_mobile_ssid(Some(""), &denylist));
        assert!(!is_mobile_ssid(Some("<hidden>"), &denylist));
        assert!(!is_mobile_ssid(Some("<HIDDEN>"), &denylist));
    }

    #[test]
    fn mobile_glob_specials_are_limited_to_star_and_question_mark() {
        let denylist = patterns(&["bus[0-9]"]);
        assert!(is_mobile_ssid(Some("BUS[0-9]"), &denylist));
        assert!(!is_mobile_ssid(Some("bus4"), &denylist));
    }

    #[test]
    fn test_parse_iw_output() {
        let output = r#"BSS aa:bb:cc:dd:ee:ff(on wlan0)
	freq: 2437
	signal: -65.00 dBm
	SSID: TestWiFi
	DS Parameter set: channel 6
BSS 10:22:33:44:55:66(on wlan0)
	freq: 5180
	signal: -78.00 dBm
	SSID: eduroam
"#;
        let networks = parse_iw_output(output);
        assert_eq!(networks.len(), 2);
        assert_eq!(networks[0].bssid, "AA:BB:CC:DD:EE:FF");
        assert_eq!(networks[0].ssid, Some("TestWiFi".to_string()));
        assert_eq!(networks[0].signal_dbm, -65);
        assert_eq!(networks[0].channel, Some(Channel::TwoFourGhz(6)));
        assert_eq!(networks[0].frequency, Some(2437));

        // 10:… is a universally-administered unicast MAC (task-0122). The
        // former 11:… fixture was multicast (0x01 bit set) and is now rejected
        // at the scanner boundary, so it would silently vanish from the scan.
        assert_eq!(networks[1].bssid, "10:22:33:44:55:66");
        assert_eq!(networks[1].signal_dbm, -78);
    }

    /// task-0122 AC#3: a malformed / multicast / broadcast / all-zero BSSID is
    /// dropped at the scanner boundary rather than stored, while the
    /// well-formed AP in the same output survives.
    #[test]
    fn parse_iw_drops_malformed_bssid_keeps_valid() {
        let output = r#"BSS ff:ff:ff:ff:ff:ff(on wlan0)
	freq: 2437
	signal: -50.00 dBm
	SSID: Broadcast
BSS 00:00:00:00:00:00(on wlan0)
	freq: 2437
	signal: -55.00 dBm
	SSID: Zeros
BSS b8:50:01:aa:bb:cc(on wlan0)
	freq: 2437
	signal: -60.00 dBm
	SSID: RealAP
"#;
        let networks = parse_iw_output(output);
        assert_eq!(
            networks.len(),
            1,
            "broadcast and all-zero BSSIDs must be dropped, got {networks:?}"
        );
        assert_eq!(networks[0].bssid, "B8:50:01:AA:BB:CC");
    }

    #[test]
    fn test_parse_nmcli_output() {
        // nmcli -t escapes colons in BSSIDs as \:
        let output = "AA\\:BB\\:CC\\:DD\\:EE\\:01:TestNet_2G:65:11:2462 MHz\nAA\\:BB\\:CC\\:DD\\:EE\\:02:TestNet_5G:49:100:5500 MHz\n";
        let networks = parse_nmcli_output(output);
        assert_eq!(networks.len(), 2);
        assert_eq!(networks[0].bssid, "AA:BB:CC:DD:EE:01");
        assert_eq!(networks[0].ssid, Some("TestNet_2G".to_string()));
        assert_eq!(networks[0].channel, Some(Channel::TwoFourGhz(11)));
        assert_eq!(networks[0].frequency, Some(2462));
        // signal: 65% -> -90 + (65*60/100) = -90 + 39 = -51
        assert_eq!(networks[0].signal_dbm, -51);
        assert_eq!(networks[1].bssid, "AA:BB:CC:DD:EE:02");
        assert_eq!(networks[1].ssid, Some("TestNet_5G".to_string()));
        assert_eq!(networks[1].channel, Some(Channel::FiveGhz(100)));
        assert_eq!(networks[1].frequency, Some(5500));
    }

    #[test]
    fn test_parse_nmcli_skips_empty_signal() {
        // First row has no signal field; second row has it. Only the second
        // row should be emitted — empty signal must NOT be coerced to 0%/-90 dBm.
        let output = concat!(
            "AA\\:BB\\:CC\\:DD\\:EE\\:01:NoSig::6:2437 MHz\n",
            "AA\\:BB\\:CC\\:DD\\:EE\\:02:HasSig:50:6:2437 MHz\n",
        );
        let networks = parse_nmcli_output(output);
        assert_eq!(
            networks.len(),
            1,
            "row with empty signal must be skipped, got {networks:?}"
        );
        assert_eq!(networks[0].bssid, "AA:BB:CC:DD:EE:02");
    }

    #[test]
    fn test_parse_nmcli_skips_garbage_signal() {
        // Non-numeric signal field also skipped, not coerced to 0.
        let output = "AA\\:BB\\:CC\\:DD\\:EE\\:03:Bad:not_a_number:6:2437 MHz\n";
        let networks = parse_nmcli_output(output);
        assert!(
            networks.is_empty(),
            "row with garbage signal must be skipped, got {networks:?}"
        );
    }

    #[test]
    fn test_split_nmcli_fields() {
        let line = "AA\\:BB\\:CC\\:DD\\:EE\\:FF:MySSID:75:6:2437 MHz";
        let fields = split_nmcli_fields(line);
        assert_eq!(
            fields,
            vec!["AA:BB:CC:DD:EE:FF", "MySSID", "75", "6", "2437 MHz"]
        );
    }

    #[test]
    fn test_parse_nmcli_keeps_rows_with_malformed_frequency() {
        let output = concat!(
            "AA\\:BB\\:CC\\:DD\\:EE\\:01:WrongUnit:50:100:5500 GHz\n",
            "AA\\:BB\\:CC\\:DD\\:EE\\:02:MissingSpace:50:100:5500MHz\n",
            "AA\\:BB\\:CC\\:DD\\:EE\\:03:TrailingGarbage:50:100:5500 MHz extra\n",
        );
        let networks = parse_nmcli_output(output);
        assert_eq!(networks.len(), 3);
        assert!(networks.iter().all(|network| network.frequency.is_none()));

        // Preserve the parser's previous bare-integer contract for callers
        // that supply synthetic or older nmcli output.
        assert_eq!(parse_nmcli_frequency("5500"), Some(5500));
    }

    /// task-0226: nmcli escapes `\` as `\\`. A field ending in a literal
    /// backslash (`SSID\` -> wire `SSID\\`) followed by the separator must
    /// yield the backslash and a clean split, NOT leave a dangling `\:`
    /// escape. The `.*` proptest `split_nmcli_no_unescaped_backslash_colon`
    /// catches this flakily; this pins it deterministically.
    #[test]
    fn split_nmcli_handles_escaped_backslash() {
        // Wire: field "a\" (a + literal backslash) then separator then "b".
        // "a" + "\\" (escaped backslash) + ":" (separator) + "b"
        assert_eq!(split_nmcli_fields("a\\\\:b"), vec!["a\\", "b"]);
        // Escaped backslash immediately followed by an escaped colon:
        // "\\" -> "\", "\:" -> ":", all one field.
        assert_eq!(split_nmcli_fields("\\\\\\:"), vec!["\\:"]);
        // No field may retain an unprocessed "\:" escape.
        for f in split_nmcli_fields("a\\\\:b\\:c") {
            assert!(!f.contains("\\:"), "unprocessed escape in {f:?}");
        }
    }

    #[test]
    fn test_normalize_bssid() {
        assert_eq!(normalize_bssid("aa:bb:cc:dd:ee:ff"), "AA:BB:CC:DD:EE:FF");
    }

    #[test]
    fn from_frequency_band_boundaries() {
        // Ported from the pre-enum `freq_to_channel_band_boundaries` (task-0123):
        // the boundary logic is now `Channel::from_frequency`, and the expected
        // values carry the BAND, so a misclassification (e.g. a 6 GHz frequency
        // decoded as 5 GHz) fails here rather than silently erasing the band.
        use Channel::{FiveGhz, SixGhz, TwoFourGhz};
        let cases = [
            // 2.4 GHz: frequencies are discrete rather than a continuous range.
            (2407, None, "below 2.4 GHz"),
            (2411, None, "immediately below 2.4 GHz"),
            (2412, Some(TwoFourGhz(1)), "first 2.4 GHz channel"),
            (2437, Some(TwoFourGhz(6)), "interior 2.4 GHz channel"),
            (2484, Some(TwoFourGhz(14)), "last 2.4 GHz channel"),
            (2485, None, "immediately above 2.4 GHz"),
            (2489, None, "above 2.4 GHz"),
            // Pin the first two 5 GHz channels so moving the lower guard from
            // 5180 to 5185 cannot slip through as an off-by-one regression.
            (5175, None, "below 5 GHz"),
            (5179, None, "immediately below 5 GHz"),
            (5180, Some(FiveGhz(36)), "first 5 GHz channel"),
            (5185, Some(FiveGhz(37)), "second 5 GHz channel"),
            (5825, Some(FiveGhz(165)), "interior 5 GHz channel"),
            (5885, Some(FiveGhz(177)), "last 5 GHz channel"),
            (5886, None, "immediately above 5 GHz"),
            (5890, None, "above 5 GHz"),
            (5905, None, "UNII-4 gap"),
            // 6 GHz / Wi-Fi 6E. Channel 2 at 5935 MHz is isolated from the
            // formula-based range that starts with channel 1 at 5955 MHz.
            (5934, None, "immediately below special-case channel 2"),
            (5935, Some(SixGhz(2)), "special-case 6 GHz channel 2"),
            (5936, None, "immediately above special-case channel 2"),
            (5950, None, "below 6 GHz"),
            (5954, None, "immediately below 6 GHz"),
            (5955, Some(SixGhz(1)), "first 6 GHz channel"),
            (6105, Some(SixGhz(31)), "interior 6 GHz channel"),
            (7115, Some(SixGhz(233)), "last 6 GHz channel"),
            (7116, None, "immediately above 6 GHz"),
            (7120, None, "above 6 GHz"),
        ];

        for (frequency, expected, description) in cases {
            assert_eq!(
                Channel::from_frequency(frequency),
                expected,
                "unexpected mapping at {description} ({frequency} MHz)"
            );
        }
    }

    #[test]
    fn from_frequency_outputs_are_valid_channels() {
        // Frequencies are parsed as MHz. Exhausting u16 covers the complete
        // practical non-negative domain and every branch that can return Some,
        // while keeping this regression property deterministic and fast.
        for frequency in 0..=u16::MAX {
            if let Some(channel) = Channel::from_frequency(i32::from(frequency)) {
                let number = match channel {
                    Channel::TwoFourGhz(n) => u16::from(n),
                    Channel::FiveGhz(n) | Channel::SixGhz(n) => n,
                };
                assert!(
                    (1..=233).contains(&number),
                    "{frequency} MHz mapped to invalid channel {channel}"
                );
            }
        }
    }

    // --- tracing-capture support for the malformed-signal warn tests ---------
    //
    // `parse_iw_output` / `parse_nmcli_output` are pure fns, so verifying their
    // `warn!` requires a scoped subscriber. This delegates to the shared
    // `test_support::capture` sink (task-0255), which installs the permissive
    // global (task-0251) so the process-global tracing interest cache cannot
    // silently drop the event under the parallel harness (task-0248/0251) —
    // this module compiles into BOTH the lib and bin test binaries, so the
    // de-flake must be active in both. This site asserts on (level, message)
    // only, so it drops the unified sink's `target` column.

    fn capture_scanner_events<T>(
        operation: impl FnOnce() -> T,
    ) -> (T, Vec<(tracing::Level, String)>) {
        let (result, events) = crate::test_support::capture(operation);
        (
            result,
            events
                .into_iter()
                .map(|(level, _target, message)| (level, message))
                .collect(),
        )
    }

    fn warning_messages(events: &[(tracing::Level, String)]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|(level, message)| {
                (*level == tracing::Level::WARN).then_some(message.as_str())
            })
            .collect()
    }

    #[test]
    fn parse_iw_output_warns_and_drops_bss_with_malformed_signal() {
        // Two BSS blocks: the first fully valid, the second malformed only in
        // its `signal:` line. The malformed AP must be dropped (correct
        // behavior, AC#2) but the drop must be observable via a WARN (AC#1).
        let text = "BSS 02:00:00:00:24:20(on wlan-test)\n\
            freq: 5180\n\
            signal: -60.00 dBm\n\
            SSID: fixture-valid\n\
            DS Parameter set: channel 36\n\
            BSS 02:00:00:00:24:21(on wlan-test)\n\
            freq: 2437\n\
            signal: unknown dBm\n\
            SSID: fixture-malformed\n\
            DS Parameter set: channel 6\n";

        let (networks, events) = capture_scanner_events(|| parse_iw_output(text));

        // AC#2: valid BSS present, malformed BSS absent.
        let bssids: Vec<&str> = networks.iter().map(|n| n.bssid.as_str()).collect();
        assert_eq!(
            bssids,
            vec!["02:00:00:00:24:20"],
            "malformed-signal BSS must be dropped, valid one kept: {networks:?}"
        );

        // AC#1: the drop is observable via a WARN naming the offending line.
        let warnings = warning_messages(&events);
        assert!(
            warnings
                .iter()
                .any(|m| m.contains("unparseable signal value") && m.contains("unknown dBm")),
            "expected a warn naming the malformed signal line; got: {events:?}"
        );
    }

    /// task-0158: a BSS block that is DROPPED (has a BSSID + SSID + freq +
    /// channel but NO `signal:` line) must not bleed its accumulator fields into
    /// the NEXT block. At the `BSS ` boundary the parser only commits when
    /// (bssid, signal) are both Some; the dropped-previous `else` branch resets
    /// `current_ssid`/`current_ds_channel`/`current_freq` to None. If that reset
    /// were removed, the following AP — here carrying ONLY its own bssid+signal —
    /// would silently inherit the dropped block's SSID/channel/frequency:
    /// cross-AP data corruption under a different, valid BSSID.
    ///
    /// This is the complement of `..._drops_bss_with_malformed_signal`, which
    /// drops the SECOND block; here the FIRST (leaky) block is dropped and the
    /// surviving second block must be clean. The second block deliberately sets
    /// none of its own SSID/channel/freq so any leak is unambiguous.
    #[test]
    fn parse_iw_output_dropped_bss_does_not_leak_fields_into_next() {
        let text = "BSS 02:00:00:00:24:40(on wlan-test)\n\
            freq: 5180\n\
            SSID: leaky-ssid\n\
            DS Parameter set: channel 36\n\
            BSS 02:00:00:00:24:41(on wlan-test)\n\
            signal: -55.00 dBm\n";

        let networks = parse_iw_output(text);

        // The first block (no signal) is dropped; only the second survives.
        assert_eq!(
            networks
                .iter()
                .map(|n| n.bssid.as_str())
                .collect::<Vec<_>>(),
            vec!["02:00:00:00:24:41"],
            "no-signal BSS must be dropped, the signal-bearing BSS kept: {networks:?}"
        );

        // The surviving AP must carry ONLY its own (empty) fields — no bleed
        // from the dropped predecessor. If the reset regressed, these would be
        // Some("leaky-ssid") / Some(channel 36) / Some(5180).
        let survivor = &networks[0];
        assert_eq!(
            survivor.ssid, None,
            "second BSS must not inherit the dropped block's SSID: {survivor:?}"
        );
        assert_eq!(
            survivor.channel, None,
            "second BSS must not inherit the dropped block's channel: {survivor:?}"
        );
        assert_eq!(
            survivor.frequency, None,
            "second BSS must not inherit the dropped block's frequency: {survivor:?}"
        );
    }

    #[test]
    fn parse_nmcli_output_warns_and_skips_row_with_malformed_signal() {
        // Parity with the iw parser: an unparseable nmcli signal field must skip
        // the row (correct) but emit a WARN so the skip is observable.
        let text = "02\\:00\\:00\\:00\\:24\\:30:fixture-valid:80:6:2437 MHz\n\
            02\\:00\\:00\\:00\\:24\\:31:fixture-malformed:notanumber:11:2462 MHz\n";

        let (networks, events) = capture_scanner_events(|| parse_nmcli_output(text));

        let bssids: Vec<&str> = networks.iter().map(|n| n.bssid.as_str()).collect();
        assert_eq!(
            bssids,
            vec!["02:00:00:00:24:30"],
            "malformed-signal nmcli row must be skipped, valid one kept: {networks:?}"
        );

        let warnings = warning_messages(&events);
        assert!(
            warnings
                .iter()
                .any(|m| m.contains("unparseable signal") && m.contains("notanumber")),
            "expected a warn naming the malformed nmcli signal; got: {events:?}"
        );
    }

    /// task-0256 AC#1: a chronically-malformed iw signal line must WARN on the
    /// FIRST parse and DEBUG (never WARN) on subsequent parses, so a
    /// permanently-broken driver line cannot spam WARN on every scan for the
    /// daemon's lifetime.
    ///
    /// NON-FLAKY: the already-warned set is process-global and persists across
    /// tests, so this test uses a BSSID (`02:00:00:00:56:01`) unique to it — no
    /// sibling test warms that key first, so the first-occurrence WARN is
    /// deterministic regardless of test order. The two parses run sequentially
    /// within this one test, so the WARN-then-DEBUG ordering is deterministic.
    #[test]
    fn parse_iw_output_malformed_signal_warns_once_then_debug() {
        let text = "BSS 02:00:00:00:56:01(on wlan-test)\n\
            freq: 2437\n\
            signal: unknown dBm\n\
            SSID: fixture-chronic\n\
            DS Parameter set: channel 6\n";

        // First scan: exactly one WARN naming the offending line + BSS.
        let (_first_networks, first) = capture_scanner_events(|| parse_iw_output(text));
        let first_warns: Vec<&str> = warning_messages(&first)
            .into_iter()
            .filter(|m| m.contains("unparseable signal value") && m.contains("56:01"))
            .collect();
        assert_eq!(
            first_warns.len(),
            1,
            "first scan of a malformed BSS must WARN exactly once; got: {first:?}"
        );

        // Second scan of the SAME malformed input: NO WARN (spam guard) —
        // demoted to a single DEBUG so the drop stays observable at DEBUG.
        let (_second_networks, second) = capture_scanner_events(|| parse_iw_output(text));
        let second_warns: Vec<&str> = warning_messages(&second)
            .into_iter()
            .filter(|m| m.contains("unparseable signal value") && m.contains("56:01"))
            .collect();
        assert!(
            second_warns.is_empty(),
            "a repeated malformed BSS must NOT WARN again (spam guard); got: {second:?}"
        );
        let second_debugs: Vec<&str> = second
            .iter()
            .filter(|(level, message)| {
                *level == tracing::Level::DEBUG
                    && message.contains("unparseable signal value")
                    && message.contains("56:01")
            })
            .map(|(_, message)| message.as_str())
            .collect();
        assert_eq!(
            second_debugs.len(),
            1,
            "the repeat must be logged once at DEBUG, not dropped silently; got: {second:?}"
        );
    }

    /// task-0256 AC#2: a truncated nmcli row (fewer than the 5 expected fields)
    /// is still dropped, but the drop is now observable via a DEBUG line naming
    /// the offending row — parity with the malformed-signal observability, so an
    /// AP no longer vanishes silently on clipped nmcli output. DEBUG (not WARN)
    /// because a chronically-truncated source would otherwise re-spam every scan.
    #[test]
    fn parse_nmcli_output_debug_logs_truncated_short_row() {
        // Three fields (BSSID:SSID:SIGNAL) — below the 5-field minimum.
        let text = "02\\:00\\:00\\:00\\:56\\:02:truncated-row:80\n";

        let (networks, events) = capture_scanner_events(|| parse_nmcli_output(text));

        assert!(
            networks.is_empty(),
            "a short (truncated) nmcli row must be dropped: {networks:?}"
        );
        let short_row_debugs: Vec<&str> = events
            .iter()
            .filter(|(level, message)| {
                *level == tracing::Level::DEBUG
                    && message.contains("expected fields")
                    && message.contains("truncated-row")
            })
            .map(|(_, message)| message.as_str())
            .collect();
        assert_eq!(
            short_row_debugs.len(),
            1,
            "a truncated nmcli row must emit one DEBUG naming the row; got: {events:?}"
        );
    }
}

// task-0082: Kani harnesses. See apple.rs for setup notes.
#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// `Channel::from_frequency` is a total function — every i32 must produce
    /// either Some or None, never panic. Kani exhaustively explores the
    /// branch / range guards (no loops, no unwind needed).
    #[kani::proof]
    fn from_frequency_total() {
        let freq: i32 = kani::any();
        let _ = Channel::from_frequency(freq);
    }

    /// `split_nmcli_fields` walks bytes once, handling backslash escapes.
    /// Bounded UTF-8 input; Kani proves panic-freedom (no out-of-bounds
    /// peeks, no integer panics) over every valid 32-byte UTF-8 string.
    #[kani::proof]
    #[kani::unwind(33)]
    fn split_nmcli_fields_no_panic() {
        const N: usize = 32;
        let bytes: [u8; N] = kani::any();
        if let Ok(s) = std::str::from_utf8(&bytes) {
            let _ = split_nmcli_fields(s);
        }
    }

    /// `normalize_bssid` is .trim().to_uppercase() — a thin wrapper
    /// around stdlib but still worth proving panic-free under symbolic
    /// UTF-8 input.
    #[kani::proof]
    #[kani::unwind(33)]
    fn normalize_bssid_no_panic() {
        const N: usize = 32;
        let bytes: [u8; N] = kani::any();
        if let Ok(s) = std::str::from_utf8(&bytes) {
            let _ = normalize_bssid(s);
        }
    }
}
