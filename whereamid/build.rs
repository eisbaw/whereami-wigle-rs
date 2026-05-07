fn main() {
    // Embed git revision at compile time
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output();
    let git_rev = match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => "unknown".to_string(),
    };
    println!("cargo:rustc-env=GIT_REV={git_rev}");
}
