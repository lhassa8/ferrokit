//! Builds and runs the fixture apps in `tests/fixtures` with this crate in
//! their `build.rs`: plain Swift in two folders, and SwiftUI with Swift
//! Charts and Swift Concurrency (Swift 6 mode); also for Intel when that
//! target is installed.

use std::path::{Path, PathBuf};
use std::process::Command;

fn target_dir() -> PathBuf {
    // Their own target folder: the workspace's is locked by this test's build.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/ferrokit-build-fixtures")
}

fn run(fixture: &str, target: Option<&str>) -> String {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(fixture);
    let mut cmd = Command::new(env!("CARGO"));
    cmd.args(["run", "-q"]).current_dir(&dir).env("CARGO_TARGET_DIR", target_dir()).env_remove("RUSTFLAGS");
    if let Some(t) = target {
        cmd.args(["--target", t]);
    }
    let out = cmd.output().expect("cargo runs");
    assert!(out.status.success(), "{fixture} failed:\n{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn plain_swift_across_folders() {
    // "Hello, Ferrokit!" is 16 characters; the module is PlainSwift (10).
    assert_eq!(run("plain", None), "greeting 16 module 10");
}

#[test]
fn swiftui_charts_and_concurrency() {
    assert_eq!(run("swiftui", None), "chart 200 task 42");
}

#[test]
fn intel_build() {
    let installed = Command::new("rustup").args(["target", "list", "--installed"]).output().map(|o| String::from_utf8_lossy(&o.stdout).contains("x86_64-apple-darwin")).unwrap_or(false);
    let rosetta = Path::new("/Library/Apple/usr/libexec/oah/libRosettaRuntime").exists() || cfg!(target_arch = "x86_64");
    if !installed || !rosetta {
        eprintln!("SKIP intel_build: x86_64-apple-darwin installed {installed}, Rosetta {rosetta}");
        return;
    }
    assert_eq!(run("swiftui", Some("x86_64-apple-darwin")), "chart 200 task 42");
}
