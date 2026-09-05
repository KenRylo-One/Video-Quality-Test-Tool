//! Puts the short git hash into the build, for the run record.
//!
//! A build outside a checkout gets nothing, and the record then reports no hash rather
//! than a wrong one.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=../../.git/HEAD");

    let hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok());

    if let Some(hash) = hash {
        println!("cargo:rustc-env=VQA_GIT_SHA={}", hash.trim());
    }
}
