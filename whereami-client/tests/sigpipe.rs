//! task-0199: `whereami | head` must not panic.
//!
//! The Rust runtime installs SIG_IGN for SIGPIPE before `main`, so once a
//! downstream reader closes the pipe the next `println!` returns EPIPE,
//! which the stdout writer escalates into a panic ("failed printing to
//! stdout: Broken pipe") and exit code 101. `main` now resets SIGPIPE to
//! SIG_DFL, so an early-closed pipe kills the process with SIGPIPE (exit
//! status 141) like any standard Unix filter.
//!
//! This test spawns the real binary with its stdout pointed at a pipe whose
//! read end is already closed, so the very first write to stdout triggers
//! EPIPE. It depends on task-0183: with the daemon down, `--json version`
//! still writes exactly one synthesized JSON line to stdout, which is what
//! provokes the EPIPE. If a daemon happens to be live, the real response
//! line provokes it just the same, so the test holds either way.

#![cfg(unix)]

use std::os::unix::io::FromRawFd;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};

#[test]
fn json_write_to_closed_pipe_dies_by_sigpipe_not_panic() {
    // Create a pipe and immediately close its READ end so any write to the
    // write end raises EPIPE (and, with SIG_DFL restored, SIGPIPE).
    let mut fds = [0_i32; 2];
    // Safety: `fds` is a valid 2-element buffer for pipe(2) to fill.
    let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
    assert_eq!(rc, 0, "pipe() failed: {}", std::io::Error::last_os_error());
    let (read_fd, write_fd) = (fds[0], fds[1]);

    // Safety: read_fd is a fresh, open fd we own and reference nowhere else.
    let close_rc = unsafe { libc::close(read_fd) };
    assert_eq!(close_rc, 0, "close(read_fd) failed");

    // Hand the write end to the child as stdout; Stdio owns and closes it.
    // Safety: write_fd is a fresh, open fd we own and reference nowhere else.
    let child_stdout = unsafe { Stdio::from_raw_fd(write_fd) };

    let child = Command::new(env!("CARGO_BIN_EXE_whereami"))
        .args(["--json", "version"])
        .stdout(child_stdout)
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn whereami");

    let output = child
        .wait_with_output()
        .expect("failed to wait on whereami");
    let stderr = String::from_utf8_lossy(&output.stderr);

    // The write to the broken pipe must terminate the process via SIGPIPE,
    // never via a Rust panic. `signal()` is Some(_) only on signal death.
    assert_eq!(
        output.status.signal(),
        Some(libc::SIGPIPE),
        "expected death by SIGPIPE (status 141), got {:?}; stderr: {stderr}",
        output.status
    );
    assert!(
        !stderr.contains("panicked"),
        "stderr must not contain a Rust panic message, got: {stderr}"
    );
}
