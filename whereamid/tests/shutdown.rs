//! End-to-end shutdown tests driving the real `whereamid` binary
//! (task-0190/0195).
//!
//! SAFETY: every test uses an ephemeral 127.0.0.1 port and NEVER binds or
//! contacts 127.0.0.1:4747, where a production daemon serves the owner on this
//! machine. `--config` points at a nonexistent file so WiGLE stays
//! unconfigured (zero quota spend / no network), `--db` is a throwaway
//! tempdir, and `--interface` is bogus so no real Wi-Fi scan runs. No test
//! ever sends the daemon a `resolve`.
#![cfg(unix)]

use std::io::Read;
use std::net::TcpListener;
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

/// Spawn the daemon bound to `addr`, with hermetic flags, stderr -> file.
fn spawn_daemon(addr: &str, dir: &Path) -> Child {
    let err = std::fs::File::create(dir.join("stderr.log")).expect("create stderr.log");
    Command::new(env!("CARGO_BIN_EXE_whereamid"))
        .arg("--bind")
        .arg(addr)
        .arg("--db")
        .arg(dir.join("aps.sqlite"))
        // Nonexistent config -> defaults -> WiGLE unconfigured (no network).
        .arg("--config")
        .arg(dir.join("no-such-config.toml"))
        // Bogus interface: wifi_scan fails fast, never touches real hardware.
        .arg("--interface")
        .arg("whereami-nonexistent0")
        .stdout(Stdio::null())
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

/// task-0195 AC#1: when the server task exits on its own (here: the port is
/// already bound, so `TcpListener::bind` returns EADDRINUSE), the daemon must
/// shut down cleanly — NOT panic on a double-polled JoinHandle.
#[test]
fn bind_failure_exits_cleanly_without_panic() {
    let dir = make_tempdir("bindfail");

    // Hold the port for the daemon's whole life so its bind fails. Two live
    // listeners on the same 127.0.0.1:port need SO_REUSEPORT (which neither
    // std nor tokio set), so the daemon's bind reliably fails.
    let listener = TcpListener::bind("127.0.0.1:0").expect("probe bind");
    let addr = listener.local_addr().unwrap();
    assert_ne!(
        addr.port(),
        4747,
        "must never collide with the production daemon's port"
    );

    let mut child = spawn_daemon(&addr.to_string(), &dir);
    let outcome = wait_for_exit(&mut child, Duration::from_secs(15));
    let stderr = read_stderr(&dir);

    // Clean up before asserting so a failure does not leak a process/dir.
    if outcome.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    drop(listener);
    let _ = std::fs::remove_dir_all(&dir);

    let (elapsed, status) =
        outcome.expect("daemon must exit on its own after a bind failure, not hang");
    assert!(
        !stderr.contains("panicked"),
        "daemon panicked on bind failure (task-0195 double-poll); stderr:\n{stderr}"
    );
    assert_ne!(
        status.code(),
        Some(101),
        "exit 101 indicates a panic; stderr:\n{stderr}"
    );
    assert!(
        status.success(),
        "bind-failure shutdown must be a clean exit, got {status:?}; stderr:\n{stderr}"
    );
    assert!(
        elapsed < Duration::from_secs(15),
        "daemon took {elapsed:?} to exit after bind failure"
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

    let mut child = spawn_daemon(&addr, &dir);

    // Let it finish startup and install the SIGTERM handler.
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        child.try_wait().expect("try_wait").is_none(),
        "daemon exited before SIGTERM was sent; stderr:\n{}",
        read_stderr(&dir)
    );

    let rc = unsafe { kill(child.id() as i32, SIGTERM) };
    assert_eq!(rc, 0, "kill(SIGTERM) syscall failed");

    let outcome = wait_for_exit(&mut child, Duration::from_secs(4));
    let stderr = read_stderr(&dir);

    if outcome.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = std::fs::remove_dir_all(&dir);

    let (elapsed, status) = outcome.expect(
        "daemon did not exit within 4s of SIGTERM — the drain budget is 5s but a \
         graceful shutdown should finish near-instantly",
    );
    assert!(
        !stderr.contains("panicked"),
        "daemon panicked on SIGTERM; stderr:\n{stderr}"
    );
    assert!(
        elapsed < Duration::from_secs(4),
        "SIGTERM shutdown took {elapsed:?}, expected < 4s"
    );
    // Graceful path returns Ok(()) -> exit 0 (the signal is handled, not fatal).
    assert_eq!(
        status.code(),
        Some(0),
        "clean shutdown expected on SIGTERM, got {status:?}; stderr:\n{stderr}"
    );
}
