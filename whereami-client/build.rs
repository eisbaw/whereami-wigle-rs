fn main() {
    let rev = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| {
            std::fs::read_to_string("GIT_REV")
                .unwrap_or_else(|_| "unknown".to_string())
                .trim()
                .to_string()
        });
    println!("cargo:rustc-env=GIT_REV={rev}");
}
