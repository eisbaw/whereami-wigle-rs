//! Regression coverage for the local `-V` / `--version` path.
//!
//! These tests spawn the real executable against a guaranteed-absent socket.
//! A successful result therefore proves the flags return before daemon target
//! resolution or I/O. The similarly named `version` subcommand remains an
//! explicit daemon query and must fail against the same socket.

#![cfg(unix)]

use std::process::{Command, Output};

fn absent_socket() -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock must be after the Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "whereami-version-flags-{}-{nanos}.sock",
        std::process::id()
    ));
    assert!(!path.exists(), "test socket unexpectedly exists: {path:?}");
    path
}

fn run(args: &[&str], socket: &std::path::Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_whereami"))
        .args(args)
        .env("WHEREAMI_SOCKET", socket)
        .env_remove("WHEREAMI_ADDR")
        .output()
        .expect("failed to run whereami")
}

fn assert_local_human_flag(flag: &str, socket: &std::path::Path) {
    let output = run(&[flag], socket);
    assert!(
        output.status.success(),
        "{flag} failed: status={:?}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout must be UTF-8"),
        format!(
            "cli:    {} ({})\n",
            env!("CARGO_PKG_VERSION"),
            env!("GIT_REV")
        )
    );
    assert!(output.stderr.is_empty(), "{flag} wrote to stderr");
}

#[test]
fn version_flags_are_local_with_daemon_absent() {
    let socket = absent_socket();
    assert_local_human_flag("-V", &socket);
    assert_local_human_flag("--version", &socket);

    let output = run(&["--json", "--version"], &socket);
    assert!(
        output.status.success(),
        "--json --version failed: status={:?}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty(), "--json --version wrote to stderr");

    let stdout = String::from_utf8(output.stdout).expect("stdout must be UTF-8");
    assert_eq!(stdout.lines().count(), 1, "expected exactly one JSON line");
    assert!(
        stdout.ends_with('\n'),
        "JSON output must end with a newline"
    );
    let value: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout must be one valid JSON object");
    let object = value.as_object().expect("stdout must be a JSON object");
    assert_eq!(
        object.len(),
        3,
        "local version JSON gained unexpected fields"
    );
    assert_eq!(value["ok"], true);
    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(value["git_rev"], env!("GIT_REV"));
}

#[test]
fn version_subcommand_still_queries_daemon() {
    let socket = absent_socket();
    let output = run(&["version"], &socket);

    assert!(
        !output.status.success(),
        "version subcommand unexpectedly succeeded without a daemon"
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout must be UTF-8"),
        format!(
            "cli:    {} ({})\n",
            env!("CARGO_PKG_VERSION"),
            env!("GIT_REV")
        )
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).starts_with("daemon: error:"),
        "version subcommand did not report a daemon error: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
