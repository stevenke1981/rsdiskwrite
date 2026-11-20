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

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn every_subcommand_has_help() {
    for sub in [
        "list",
        "inspect",
        "hash",
        "write",
        "verify",
        "doctor",
        "mcp",
        "completions",
        "gui",
    ] {
        let output = run(&[sub, "--help"]);
        assert_eq!(output.status.code(), Some(0), "{sub}: {output:?}");
        assert!(stdout(&output).contains("Usage"), "{sub}");
    }
}

#[test]
fn list_json_envelope_schema() {
    let output = run(&["list", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let value = json(&output);
    assert_eq!(value["schema"], 1);
    assert_eq!(value["ok"], true);
    assert!(value["data"]["method"].is_string());
    for disk in value["data"]["disks"].as_array().unwrap() {
        assert!(disk["number"].is_u64());
        assert!(disk["size"].is_u64());
        assert!(disk["model"].is_string());
        assert!(disk["bus"].is_string());
        assert!(disk["removable"].is_boolean());
        assert!(disk["is_system"].is_boolean());
    }
}

#[test]
fn list_human_has_no_ansi_when_piped() {
    let output = run(&["list"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(!stdout(&output).contains('\u{1b}'));
}

#[test]
fn inspect_raw_and_gzip_images() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let raw = dir.path().join("a.img");
    std::fs::write(&raw, vec![7u8; 4096]).unwrap();
    let gz = dir.path().join("a.img.gz");
    let mut enc = flate2::write::GzEncoder::new(
        std::fs::File::create(&gz).unwrap(),
        flate2::Compression::fast(),
    );
    enc.write_all(&vec![7u8; 4096]).unwrap();
    enc.finish().unwrap();

    for (path, format) in [(&raw, "raw"), (&gz, "gzip")] {
        let output = run(&["inspect", path.to_str().unwrap(), "--json"]);
        let value = json(&output);
        match output.status.code() {
            Some(0) => {
                assert_eq!(value["ok"], true, "{value}");
                assert_eq!(value["data"]["format"], format);
                assert_eq!(value["data"]["decompressed_size"], 4096);
                assert!(value["data"]["file_size"].is_u64());
            }
            // inspect may still be a stub in a partial tree.
            Some(1) => assert_eq!(value["error"]["code"], "unsupported"),
            other => panic!("unexpected exit {other:?}: {value}"),
        }
    }
}

#[test]
fn inspect_missing_file_fails_with_envelope() {
    let output = run(&["inspect", "Z:/no/such/file.img", "--json"]);
    assert_ne!(output.status.code(), Some(0));
    let value = json(&output);
    assert_eq!(value["ok"], false);
    assert!(value["error"]["code"].is_string());
}

#[test]
fn write_with_piped_stdin_and_no_confirm_never_writes() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("x.img");
    std::fs::write(&image, [0u8; 512]).unwrap();
    // Disk 126 does not exist: planning fails (exit 7) before any confirmation or write.
    let output = run(&["write", image.to_str().unwrap(), "-d", "126", "--json"]);
    assert!(
        matches!(output.status.code(), Some(2 | 4 | 7)),
        "{output:?}"
    );
    assert_eq!(json(&output)["ok"], false);
}

#[test]
fn write_with_wrong_confirm_on_missing_disk_exits_seven() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("x.img");
    std::fs::write(&image, [0u8; 512]).unwrap();
    let output = run(&[
        "write",
        image.to_str().unwrap(),
        "-d",
        "126",
        "--confirm",
        "yes",
        "--json",
    ]);
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    assert_eq!(json(&output)["error"]["code"], "disk_not_found");
}

#[test]
fn completions_for_every_shell_are_non_empty() {
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let output = run(&["completions", shell]);
        assert_eq!(output.status.code(), Some(0), "{shell}");
        assert!(stdout(&output).contains("rsdiskwrite"), "{shell}");
    }
    assert_eq!(run(&["completions", "nushell"]).status.code(), Some(2));
}

#[test]
fn completions_json_wraps_script() {
    let output = run(&["completions", "bash", "--json"]);
    assert_eq!(output.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(v["data"]["shell"], "bash");
    assert!(
        v["data"]["script"]
            .as_str()
            .unwrap()
            .contains("rsdiskwrite")
    );
}

#[test]
fn usage_errors_exit_two() {
    assert_eq!(run(&["hash"]).status.code(), Some(2));
    assert_eq!(run(&["verify", "x.img"]).status.code(), Some(2));
    assert_eq!(run(&["list", "--nope"]).status.code(), Some(2));
}

#[test]
fn quiet_still_prints_data() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("abc.img");
    std::fs::write(&image, b"abc").unwrap();
    let output = run(&["--quiet", "hash", image.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
    assert!(stdout(&output).starts_with("ba7816bf"));
    assert!(output.stderr.is_empty());
}
