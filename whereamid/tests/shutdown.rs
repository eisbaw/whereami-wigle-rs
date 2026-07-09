//! End-to-end shutdown tests driving the real `whereamid` binary
//! (task-0190/0195).
//!
//! SAFETY / HERMETICITY: every test uses an ephemeral 127.0.0.1 port and NEVER
//! binds or contacts 127.0.0.1:4747, where a production daemon serves the owner
//! on this machine. Each test also passes an explicit `--socket` under its own
//! throwaway tempdir, so the daemon never touches the DEFAULT unix-socket path
//! ($XDG_RUNTIME_DIR/whereamid.sock or /run/whereami/...) and parallel tests
//! cannot collide. `--config` points at either a nonexistent file or a file
//! with empty WiGLE values, so WiGLE stays unconfigured (zero quota spend / no
//! network), `--db` is a throwaway tempdir, and `--interface` is bogus so no
//! real Wi-Fi scan runs. No test ever sends the daemon a `resolve`.
#![cfg(unix)]

use std::io::Read;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const SIGTERM: i32 = 15;

// `kill(2)` from libc, which std already links; avoids depending on a `kill`
// executable being on PATH or pulling in the `nix`/`libc` crates.
extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

/// Unique throwaway directory for one test (no `tempfile` dependency).
fn make_tempdir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir =
        std::env::temp_dir().join(format!("whereamid-it-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Spawn the daemon with an explicit unix `--socket`, an optional TCP `--bind`,
/// and hermetic flags (throwaway DB, explicit config, bogus interface). Both
/// stdout and stderr are captured to files: the tracing subscriber writes
/// lifecycle logs to STDERR (task-0240), and panics plus a `main`-returned
/// error also land on STDERR; stdout stays empty for this daemon. Tests read
/// the combined streams (`read_output`) so they observe a lifecycle line
/// regardless of which stream it uses. Passing an explicit `--socket` keeps the
/// daemon off the default socket path (task-0196 hermeticity).
fn spawn_daemon(bind: Option<&str>, socket: &Path, config: &Path, dir: &Path) -> Child {
    let out = std::fs::File::create(dir.join("stdout.log")).expect("create stdout.log");
    let err = std::fs::File::create(dir.join("stderr.log")).expect("create stderr.log");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_whereamid"));
    cmd.arg("--socket").arg(socket);
    if let Some(addr) = bind {
        cmd.arg("--bind").arg(addr);
    }
    cmd.arg("--db")
        .arg(dir.join("aps.sqlite"))
        .arg("--config")
        .arg(config)
        // Bogus interface: wifi_scan fails fast, never touches real hardware.
        .arg("--interface")
        .arg("whereami-nonexistent0")
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn()
        .expect("spawn whereamid")
}

fn read_stderr(dir: &Path) -> String {
    let mut s = String::new();
    if let Ok(mut f) = std::fs::File::open(dir.join("stderr.log")) {
        let _ = f.read_to_string(&mut s);
    }
    s
}

/// Combined daemon output (stdout ++ stderr). The tracing subscriber logs to
/// stderr (task-0240) and panics / a `main`-returned error also land on stderr;
/// reading both streams keeps a test robust to which stream any given line uses
/// (this daemon leaves stdout empty, but combining is future-proof).
fn read_output(dir: &Path) -> String {
    let mut s = String::new();
    if let Ok(mut f) = std::fs::File::open(dir.join("stdout.log")) {
        let _ = f.read_to_string(&mut s);
    }
    if let Ok(mut f) = std::fs::File::open(dir.join("stderr.log")) {
        let _ = f.read_to_string(&mut s);
    }
    s
}

/// Poll for child exit up to `budget`. Returns the elapsed time + status, or
/// None on timeout.
fn wait_for_exit(child: &mut Child, budget: Duration) -> Option<(Duration, ExitStatus)> {
    let start = Instant::now();
    loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => return Some((start.elapsed(), status)),
            None if start.elapsed() >= budget => return None,
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

/// Block until the daemon has bound its control socket, then return. This
/// replaces a blind fixed startup sleep (task-0237) with a poll on an actual
/// readiness condition — the socket file appearing. The daemon installs its
/// SIGTERM/SIGINT handlers BEFORE binding the socket, so a bound socket implies
/// the handler is already installed: exactly the precondition the sigterm test
/// needs before it signals. The 10ms cadence is a poll backoff, not a fixed
/// readiness delay — the loop returns the instant the socket exists — and it
/// fails fast if the daemon exits during startup.
fn wait_until_socket_ready(child: &mut Child, socket: &Path, dir: &Path, budget: Duration) {
    let start = Instant::now();
    loop {
        if socket.exists() {
            return;
        }
        if let Some(status) = child.try_wait().expect("try_wait") {
            panic!(
                "daemon exited during startup (status {status:?}) before binding its \
                 socket; stderr:\n{}",
                read_stderr(dir)
            );
        }
        assert!(
            start.elapsed() < budget,
            "daemon did not bind its control socket within {budget:?}; stderr:\n{}",
            read_stderr(dir)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Poll the daemon's combined output until it contains `needle`, or fail. Waits
/// for a specific lifecycle line (control channel ready, a graceful-degrade
/// warning) without a fixed sleep; fails fast if the daemon exits first. The
/// 10ms cadence is a poll backoff, not a fixed delay.
fn wait_until_output_contains(child: &mut Child, dir: &Path, needle: &str, budget: Duration) {
    let start = Instant::now();
    loop {
        if read_output(dir).contains(needle) {
            return;
        }
        if let Some(status) = child.try_wait().expect("try_wait") {
            panic!(
                "daemon exited (status {status:?}) before output contained {needle:?}; \
                 output:\n{}",
                read_output(dir)
            );
        }
        assert!(
            start.elapsed() < budget,
            "daemon output never contained {needle:?} within {budget:?}; output:\n{}",
            read_output(dir)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// RAII cleanup for assertions/setup failures in tests that start a daemon.
/// The explicit happy-path cleanup still runs before assertions; this guard is
/// the panic/unwind backstop that prevents a failed readiness check from
/// leaking a daemon into later tests.
struct ChildCleanup(Child);

impl Drop for ChildCleanup {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

struct DirCleanup(PathBuf);

impl Drop for DirCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct ConfigPermissionRun {
    config_path: String,
    output: String,
    signal_rc: i32,
    outcome: Option<(Duration, ExitStatus)>,
}

/// Start the real daemon with a config at exactly `mode`, wait for its unix
/// socket, then shut it down. All process and filesystem cleanup happens here,
/// before the caller performs assertions on the captured result.
fn run_config_permission_case(mode: u32) -> ConfigPermissionRun {
    let dir = make_tempdir(&format!("config-mode-{mode:04o}"));
    let dir_cleanup = DirCleanup(dir.clone());
    let config = dir.join("config.toml");
    std::fs::write(&config, "[wigle]\napi_user = \"\"\napi_key = \"\"\n")
        .expect("write empty WiGLE config");
    let mut permissions = std::fs::metadata(&config)
        .expect("stat empty WiGLE config")
        .permissions();
    permissions.set_mode(mode);
    std::fs::set_permissions(&config, permissions).expect("set WiGLE config mode");

    let socket = dir.join("whereamid.sock");
    let mut child = ChildCleanup(spawn_daemon(None, &socket, &config, &dir));
    wait_until_socket_ready(&mut child.0, &socket, &dir, Duration::from_secs(10));

    let signal_rc = unsafe { kill(child.0.id() as i32, SIGTERM) };
    let outcome = if signal_rc == 0 {
        wait_for_exit(&mut child.0, Duration::from_secs(6))
    } else {
        None
    };
    let output = read_output(&dir);
    let config_path = config.display().to_string();

    // The guards are unwind backstops; explicitly complete cleanup before any
    // assertion in the caller so even a failed assertion cannot leak state.
    drop(child);
    drop(dir_cleanup);

    ConfigPermissionRun {
        config_path,
        output,
        signal_rc,
        outcome,
    }
}

/// task-0200: the real startup path warns for a group/world-accessible WiGLE
/// credentials file and remains silent for owner-only permissions. Unit tests
/// cover additional modes and the descriptor/symlink semantics; this test
/// guards tracing initialization, CLI path plumbing, and startup ordering.
#[test]
fn config_permission_warning_is_visible_at_real_daemon_startup() {
    for (mode, should_warn) in [(0o644, true), (0o600, false)] {
        let run = run_config_permission_case(mode);

        assert_eq!(
            run.signal_rc, 0,
            "failed to send SIGTERM for mode {mode:04o}; output:\n{}",
            run.output
        );
        let (_elapsed, status) = run.outcome.expect(
            "daemon must shut down after permission-warning startup case, not hang or exit early",
        );
        assert_eq!(
            status.code(),
            Some(0),
            "clean shutdown expected for mode {mode:04o}, got {status:?}; output:\n{}",
            run.output
        );
        assert!(
            !run.output.contains("panicked"),
            "daemon panicked for mode {mode:04o}; output:\n{}",
            run.output
        );

        if should_warn {
            assert!(
                run.output.contains("WARN")
                    && run.output.contains("WiGLE credentials file")
                    && run.output.contains(&run.config_path)
                    && run.output.contains("0644")
                    && run.output.contains("another local user")
                    && run.output.contains("read or modify")
                    && run.output.contains("0600"),
                "0644 startup must emit a clear credential-permission warning; output:\n{}",
                run.output
            );
        } else {
            assert!(
                !run.output.contains("WiGLE credentials file")
                    && !run.output.contains("cannot verify permissions"),
                "0600 startup must not emit a credential-permission warning; output:\n{}",
                run.output
            );
        }
    }
}

/// task-0233 (amending task-0195/0196): an explicitly-requested TCP `--bind`
/// whose port is already taken must NOT take the daemon down. `run_server`'s
/// `TcpListener::bind` returns EADDRINUSE, and the daemon must degrade to
/// serving the mandatory unix socket ONLY — warning loudly — instead of
/// self-exiting (which under systemd `Restart=on-failure` would report a dead
/// daemon as a success). It must still shut down cleanly on SIGTERM. The unix
/// socket (the mandatory transport) staying fatal on ITS failure is covered by
/// `live_socket_refused_nonzero_exit`.
#[test]
fn tcp_bind_failure_degrades_to_unix_only() {
    let dir = make_tempdir("bindfail");

    // Hold the port for the daemon's whole life so its TCP bind fails. Two live
    // listeners on the same 127.0.0.1:port need SO_REUSEPORT (which neither
    // std nor tokio set), so the daemon's bind reliably fails.
    let listener = TcpListener::bind("127.0.0.1:0").expect("probe bind");
    let addr = listener.local_addr().unwrap();
    assert_ne!(
        addr.port(),
        4747,
        "must never collide with the production daemon's port"
    );

    let sock = dir.join("whereamid.sock");
    let mut child = spawn_daemon(
        Some(&addr.to_string()),
        &sock,
        &dir.join("no-such-config.toml"),
        &dir,
    );

    // The daemon reaches its accept loop despite the TCP failure. "unix socket
    // ready" is logged AFTER the TCP bind attempt (run_server), so observing it
    // proves the daemon degraded rather than exited — and, being emitted after
    // the degrade warn!, guarantees that warning is already in the log below.
    wait_until_output_contains(
        &mut child,
        &dir,
        "control channel: unix socket ready",
        Duration::from_secs(10),
    );

    let output = read_output(&dir);
    // The degrade is loud and names the failure (the operator must notice the
    // requested TCP endpoint is unavailable).
    assert!(
        output.contains("failed to bind") && output.contains("unix control socket ONLY"),
        "daemon must warn loudly that it degraded to unix-only; output:\n{output}"
    );
    // And it is STILL RUNNING — the opt-in TCP failure did not exit it.
    assert!(
        child.try_wait().expect("try_wait").is_none(),
        "daemon must keep serving the unix socket after a TCP bind failure, not exit; \
         output:\n{output}"
    );

    // A degraded (unix-only) daemon still shuts down cleanly on SIGTERM.
    let rc = unsafe { kill(child.id() as i32, SIGTERM) };
    assert_eq!(rc, 0, "kill(SIGTERM) syscall failed");
    let outcome = wait_for_exit(&mut child, Duration::from_secs(6));
    let output = read_output(&dir);

    // Clean up before asserting so a failure does not leak a process/dir.
    if outcome.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    drop(listener);
    let _ = std::fs::remove_dir_all(&dir);

    let (_elapsed, status) = outcome.expect("degraded daemon must exit on SIGTERM, not hang");
    assert!(
        !output.contains("panicked"),
        "daemon panicked during degraded-mode shutdown; output:\n{output}"
    );
    assert_eq!(
        status.code(),
        Some(0),
        "clean shutdown expected on SIGTERM for a degraded daemon, got {status:?}; output:\n{output}"
    );
}

/// task-0190 AC#1/#3: on SIGTERM the daemon drains and exits well within the
/// 5s budget. The old lost-wakeup + never-abort drain always ate the full 5s;
/// a <4s exit is a clean discriminator. Generous margins (real exit is <1s).
#[test]
fn sigterm_exits_within_drain_budget() {
    let dir = make_tempdir("sigterm");

    // Grab a free port by binding then immediately dropping the probe.
    let port = TcpListener::bind("127.0.0.1:0")
        .expect("probe bind")
        .local_addr()
        .unwrap()
        .port();
    assert_ne!(port, 4747, "must never collide with the production port");
    let addr = format!("127.0.0.1:{port}");

    let sock = dir.join("whereamid.sock");
    let mut child = spawn_daemon(Some(&addr), &sock, &dir.join("no-such-config.toml"), &dir);

    // Readiness handshake: wait for the daemon to bind its control socket rather
    // than sleeping a fixed interval and hoping startup finished. Since the
    // daemon installs its signal handlers before the socket bind (task-0237), a
    // bound socket guarantees the SIGTERM handler is live, so the signal below
    // can never race an uninstalled handler.
    wait_until_socket_ready(&mut child, &sock, &dir, Duration::from_secs(10));

    let rc = unsafe { kill(child.id() as i32, SIGTERM) };
    assert_eq!(rc, 0, "kill(SIGTERM) syscall failed");

    // Wait past the daemon's 5s drain budget (6s) so a slow-but-correct drain is
    // reported as "too slow" by the < 4s health assertion below rather than
    // masquerading as a hang/timeout here; a healthy graceful shutdown finishes
    // in well under 1s.
    let outcome = wait_for_exit(&mut child, Duration::from_secs(6));
    let stderr = read_stderr(&dir);

    if outcome.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = std::fs::remove_dir_all(&dir);

    let (elapsed, status) = outcome.expect(
        "daemon did not exit within 6s of SIGTERM (drain budget is 5s) — a graceful \
         shutdown should finish near-instantly, so this indicates a hang",
    );
    assert!(
        !stderr.contains("panicked"),
        "daemon panicked on SIGTERM; stderr:\n{stderr}"
    );
    assert!(
        elapsed < Duration::from_secs(4),
        "SIGTERM shutdown took {elapsed:?}, expected < 4s (drain should be near-instant)"
    );
    // task-0143: the accept loop must exit COOPERATIVELY via the cancel token,
    // not be force-aborted. run_server logs this clean-exit line only on the
    // cooperative `break` path; its presence proves the shutdown.cancelled()
    // branch fired and run_server returned Ok before the drain reaped it.
    assert!(
        stderr.contains("accept loop stopped (cooperative shutdown)"),
        "accept loop must exit cooperatively via the cancel token (clean-exit log \
         missing); stderr:\n{stderr}"
    );
    // The bounded drain's force-abort backstop must NOT have fired for a healthy
    // near-instant shutdown — that would mean the cooperative exit did not work.
    assert!(
        !stderr.contains("did not drain within"),
        "cooperative shutdown must not trip the drain's force-abort backstop; \
         stderr:\n{stderr}"
    );
    // Graceful path returns Ok(()) -> exit 0 (the signal is handled, not fatal).
    assert_eq!(
        status.code(),
        Some(0),
        "clean shutdown expected on SIGTERM, got {status:?}; stderr:\n{stderr}"
    );
}

/// task-0196: refusing to steal a LIVE control socket. Holding a bound unix
/// listener at the daemon's `--socket` path makes the daemon's startup connect
/// probe succeed, so it must REFUSE to bind (never unlink a live socket) and
/// exit non-zero with a clear error — the fail-fast counterpart to the TCP
/// bind-collision test above.
#[test]
fn live_socket_refused_nonzero_exit() {
    let dir = make_tempdir("livesock");
    let sock = dir.join("whereamid.sock");

    // Hold a LIVE (bound, listening) unix socket for the daemon's whole
    // startup. The kernel completes connect handshakes up to the backlog even
    // without accept(), so the daemon's connect probe sees a live owner.
    let live = UnixListener::bind(&sock).expect("hold a live socket at the daemon's path");

    // No --bind: the unix socket is the mandatory transport, and its failure
    // must abort startup by itself.
    let mut child = spawn_daemon(None, &sock, &dir.join("no-such-config.toml"), &dir);
    let outcome = wait_for_exit(&mut child, Duration::from_secs(15));
    let stderr = read_stderr(&dir);

    if outcome.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    drop(live);
    let _ = std::fs::remove_dir_all(&dir);

    let (elapsed, status) =
        outcome.expect("daemon must exit on its own when the socket is already live, not hang");
    assert!(
        !stderr.contains("panicked"),
        "daemon panicked on live-socket refusal; stderr:\n{stderr}"
    );
    assert!(
        !status.success(),
        "live-socket refusal must be a non-zero exit (fail-fast), got {status:?}; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("already listening") || stderr.contains("live control socket"),
        "stderr must explain the live-socket refusal; stderr:\n{stderr}"
    );
    assert!(
        elapsed < Duration::from_secs(15),
        "daemon took {elapsed:?} to refuse a live socket"
    );
}
