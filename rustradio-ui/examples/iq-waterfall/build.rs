fn main() {
    // Packaging supplies one version for both HTML and WASM. Tracking the
    // environment variable refreshes cached builds when that version changes.
    println!("cargo:rerun-if-env-changed=IQ_WATERFALL_GIT_VERSION");
    let version = std::env::var("IQ_WATERFALL_GIT_VERSION").unwrap_or_else(|_| {
        let output = std::process::Command::new("git")
            .args(["describe", "--tags", "--dirty", "--always"])
            .output()
            .expect("read application Git version");
        assert!(output.status.success(), "read application Git version");
        String::from_utf8(output.stdout)
            .expect("Git version is UTF-8")
            .trim()
            .to_owned()
    });
    assert!(!version.is_empty(), "application Git version must be set");
    println!("cargo:rustc-env=GIT_VERSION={version}");
}
