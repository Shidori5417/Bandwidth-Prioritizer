//! `bwp start` must fail fast on hosts where the engine cannot run, instead of spawning
//! the stub engine and reporting a daemon that is not there.

#![cfg(not(windows))]

use std::process::Command;

#[test]
fn start_fails_fast_on_non_windows_hosts() {
    let output = Command::new(env!("CARGO_BIN_EXE_bwp"))
        .arg("start")
        .output()
        .expect("run bwp");
    assert!(!output.status.success(), "start must not report success");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("only runs on Windows"), "stderr: {stderr}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("spawned"), "stdout: {stdout}");
}
