fn main() {
    // Add git version.
    let git = std::process::Command::new("git")
        .args(["describe", "--tags", "--dirty", "--always"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=GIT_VERSION={}", git.trim());

    let git_commit_timestamp = std::process::Command::new("git")
        .env("TZ", "UTC")
        .args([
            "show",
            "-s",
            "--format=%cd",
            "--date=format-local:%Y-%m-%dT%H:%M:%SZ",
            "HEAD",
        ])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "unknown".into());
    println!(
        "cargo:rustc-env=GIT_COMMIT_TIMESTAMP={}",
        git_commit_timestamp.trim()
    );

    let git_author_timestamp = std::process::Command::new("git")
        .env("TZ", "UTC")
        .args([
            "show",
            "-s",
            "--format=%ad",
            "--date=format-local:%Y-%m-%dT%H:%M:%SZ",
            "HEAD",
        ])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "unknown".into());
    println!(
        "cargo:rustc-env=GIT_AUTHOR_TIMESTAMP={}",
        git_author_timestamp.trim()
    );

    {
        let rustc = std::env::var("RUSTC").unwrap();
        let out = std::process::Command::new(rustc)
            .arg("--version")
            .output()
            .unwrap();
        let version = String::from_utf8(out.stdout).unwrap();
        println!("cargo:rustc-env=RUSTC_VERSION={version}");
    }
}
