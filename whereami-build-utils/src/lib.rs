//! Shared build-script helper for stamping the Git revision into whereami
//! binaries. Both `whereamid` and `whereami-client` call
//! [`emit_git_rev_env`] from their thin `build.rs` wrappers so the
//! rev-stamping protocol lives in exactly one place (task-0136).

use std::path::{Path, PathBuf};
use std::process::Command;

fn git_output(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn emit_existing_rerun_path(path: &Path, emitted: &mut Vec<PathBuf>) {
    if path.exists() && !emitted.iter().any(|existing| existing == path) {
        println!("cargo:rerun-if-changed={}", path.display());
        emitted.push(path.to_path_buf());
    }
}

fn emit_git_rerun_paths() {
    let Some(head_path) = git_output(&["rev-parse", "--git-path", "HEAD"]) else {
        return;
    };

    // A successful rev-parse can still describe a path supplied through
    // environment configuration. Only register Git metadata when HEAD really
    // exists, which keeps clean source archives using the GIT_REV fallback.
    if !Path::new(&head_path).exists() {
        return;
    }

    let mut emitted = Vec::new();
    emit_existing_rerun_path(Path::new(&head_path), &mut emitted);

    // Detached HEAD is expected to make symbolic-ref fail. In that case HEAD
    // itself is the only metadata file Cargo needs to monitor.
    if let Some(head_ref) = git_output(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(ref_path) = git_output(&["rev-parse", "--git-path", &head_ref]) {
            let ref_path = PathBuf::from(ref_path);
            if ref_path.is_file() {
                emit_existing_rerun_path(&ref_path, &mut emitted);
            } else {
                // A packed or unborn branch has no loose ref file yet. Cargo
                // treats a missing rerun path as always dirty, so watch the
                // nearest existing parent directory for loose-ref creation.
                let mut ancestor = ref_path.parent();
                while let Some(path) = ancestor {
                    if path.is_dir() {
                        emit_existing_rerun_path(path, &mut emitted);
                        break;
                    }
                    ancestor = path.parent();
                }

                if let Some(packed_refs) = git_output(&["rev-parse", "--git-path", "packed-refs"]) {
                    let packed_refs = PathBuf::from(packed_refs);
                    if packed_refs.is_file() {
                        emit_existing_rerun_path(&packed_refs, &mut emitted);
                    }
                }
            }
        }
    }
}

/// Emit `cargo:rustc-env=GIT_REV=<rev>` so consuming crates can read the
/// build's Git revision via `env!("GIT_REV")`.
///
/// The revision is provisioned by one of TWO authoritative paths, tried in
/// order — either is correct for its context:
///
/// (a) `git rev-parse --short HEAD` — used for local development and any plain
///     `cargo build` where a real `.git` directory is reachable. This branch
///     also emits `cargo:rerun-if-changed` lines (via `emit_git_rerun_paths`)
///     for HEAD and the checked-out ref so Cargo re-runs the build script when
///     the checkout moves to a new commit.
///
/// (b) A `GIT_REV` file in the consuming crate's manifest directory — the
///     fallback for the Nix build. crane's sandbox copies the workspace to a
///     `/nix/store` path with NO `.git`, so `git rev-parse` finds nothing;
///     instead `nix/package.nix`'s preBuild writes the flake's own revision
///     into `whereamid/GIT_REV` and `whereami-client/GIT_REV` before the build.
///     Cargo runs each build script with CWD set to the CONSUMING crate's
///     manifest dir (even though this logic lives in a build-dependency), so
///     the relative `"GIT_REV"` path resolves to that crate's own file.
///
/// If neither path yields a revision the value falls back to `"unknown"`.
pub fn emit_git_rev_env() {
    emit_git_rerun_paths();

    let rev = git_output(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| {
        std::fs::read_to_string("GIT_REV")
            .unwrap_or_else(|_| "unknown".to_string())
            .trim()
            .to_string()
    });
    println!("cargo:rustc-env=GIT_REV={rev}");
}
