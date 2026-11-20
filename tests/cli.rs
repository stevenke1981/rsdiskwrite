//! End-to-end tests of the binary that never touch a disk for writing.

use std::process::{Command, Output, Stdio};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rsdiskwrite"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run rsdiskwrite")
}

fn json(output: &Output) -> serde_json::Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();
    let value = serde_json::from_str(lines.next().expect("one JSON line")).expect("valid JSON");
    assert!(lines.next().is_none(), "exactly one JSON object: {stdout}");
    value
}

#[test]
fn help_and_version_exit_zero() {
    let help = run(&["--help"]);
    assert_eq!(help.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&help.stdout).contains("Exit codes"));
    let version = run(&["--version"]);
    assert_eq!(version.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&version.stdout).contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn usage_errors_exit_two_with_json_envelope() {
    let output = run(&["--json"]);
    assert_eq!(output.status.code(), Some(2));
    let value = json(&output);
    assert_eq!(value["schema"], 1);
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], "usage");

    let output = run(&["write", "--json"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(json(&output)["error"]["code"], "usage");

    let output = run(&["--bogus"]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn list_json_is_one_envelope() {
    let output = run(&["list", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let value = json(&output);
    assert_eq!(value["ok"], true);
    assert!(value["data"]["disks"].is_array());
}

#[test]
fn write_without_disk_on_a_pipe_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("x.img");
    std::fs::write(&image, [0u8; 512]).unwrap();
    let output = run(&["--json", image.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(json(&output)["error"]["code"], "usage");
}

#[test]
fn dry_run_against_a_missing_disk_exits_seven() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("x.img");
    std::fs::write(&image, [0u8; 512]).unwrap();
    let output = run(&[
        "write",
        image.to_str().unwrap(),
        "-d",
        "4000",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    assert_eq!(json(&output)["error"]["code"], "disk_not_found");
}

#[test]
fn hash_of_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("abc.img");
    std::fs::write(&image, b"abc").unwrap();
    let output = run(&["hash", image.to_str().unwrap(), "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        json(&output)["data"]["sha256"],
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
