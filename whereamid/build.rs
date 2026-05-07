fn main() {
    // Try git first (dev builds), fall back to GIT_REV file (nix builds)
    let rev = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| {
            // Nix sandbox: read from GIT_REV file written by package.nix
            std::fs::read_to_string("GIT_REV")
                .unwrap_or_else(|_| "unknown".to_string())
                .trim()
                .to_string()
        });
    println!("cargo:rustc-env=GIT_REV={rev}");
}
