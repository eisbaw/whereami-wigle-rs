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

fn main() {
    emit_git_rerun_paths();

    let rev = git_output(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| {
        std::fs::read_to_string("GIT_REV")
            .unwrap_or_else(|_| "unknown".to_string())
            .trim()
            .to_string()
    });
    println!("cargo:rustc-env=GIT_REV={rev}");
}
