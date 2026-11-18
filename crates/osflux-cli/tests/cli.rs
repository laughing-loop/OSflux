use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

#[test]
fn rejects_unknown_arguments() {
    let output = osflux(&["--plan-imag", "image.iso"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("unknown argument: --plan-imag"));
}

#[test]
fn rejects_invalid_demo_pipeline_numbers() {
    let output = osflux(&["--blocks=many"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("block count must be a non-negative number"));
}

#[test]
fn exits_zero_when_image_plan_is_ready() {
    let mut bytes = vec![0u8; 1024];
    bytes[510] = 0x55;
    bytes[511] = 0xAA;
    let path = temp_file("img", &bytes);

    let output = osflux(&["--plan-image", path.to_str().unwrap(), "--json"]);
    let _ = fs::remove_file(&path);

    assert_eq!(output.status.code(), Some(0));
    assert!(stdout(&output).contains("\"value\": \"ready\""));
}

#[test]
fn exits_not_ready_when_image_plan_is_blocked_and_still_prints_report() {
    let path = temp_file("dat", b"not a disk image");

    let output = osflux(&["--plan-image", path.to_str().unwrap(), "--json"]);
    let _ = fs::remove_file(&path);

    assert_eq!(output.status.code(), Some(3));
    assert!(stdout(&output).contains("\"value\": \"blocked\""));
}

#[test]
fn exits_one_when_image_cannot_be_read() {
    let path = std::env::temp_dir().join(unique_name("missing.iso"));

    let output = osflux(&["--plan-image", path.to_str().unwrap()]);

    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("image path does not exist"));
}

#[test]
fn execute_only_works_with_write() {
    let output = osflux(&["--execute", "--image", "disk.img", "--target", "250"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("--execute can only be used with --write"));
}

#[test]
fn execute_needs_a_confirmation_phrase() {
    let output = osflux(&[
        "--write",
        "--image",
        "disk.img",
        "--target",
        "250",
        "--execute",
    ]);

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("needs --confirm"));
}

#[test]
fn execute_rejects_chunk_sizes_a_disk_cannot_take() {
    let output = osflux(&[
        "--write",
        "--image",
        "disk.img",
        "--target",
        "250",
        "--confirm",
        "OSFLUX ERASE TARGET 250 0000",
        "--chunk-size",
        "1000",
        "--execute",
        "--yes",
    ]);

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("must be a multiple of 4096 bytes"));
}

#[test]
fn execute_writes_nothing_when_the_target_does_not_exist() {
    let mut bytes = vec![0u8; 1024];
    bytes[510] = 0x55;
    bytes[511] = 0xAA;
    let path = temp_file("img", &bytes);

    // Index 250 never exists, so the safety chain stops before any drive is opened.
    let output = osflux(&[
        "--write",
        "--image",
        path.to_str().unwrap(),
        "--target",
        "250",
        "--confirm",
        "OSFLUX ERASE TARGET 250 0000",
        "--execute",
        "--yes",
    ]);
    let _ = fs::remove_file(&path);

    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("OSflux did not write the image."));
    assert!(stderr(&output).contains("target device index was not found: 250"));
}

#[test]
fn execute_refuses_without_a_terminal_unless_yes_is_given() {
    // Tests run with stdin closed, which is not a terminal.
    let output = osflux(&[
        "--write",
        "--image",
        "disk.img",
        "--target",
        "250",
        "--confirm",
        "OSFLUX ERASE TARGET 250 0000",
        "--execute",
    ]);

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("Pass --yes"));
}

#[test]
fn yes_only_works_with_execute() {
    let output = osflux(&["--write", "--image", "disk.img", "--target", "250", "--yes"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("--yes can only be used with --write --execute"));
}

#[test]
fn help_lists_exit_codes() {
    let output = osflux(&["--help"]);

    assert_eq!(output.status.code(), Some(0));
    assert!(stdout(&output).contains("Exit codes:"));
}

fn osflux(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_osflux-cli"))
        .args(args)
        .output()
        .expect("osflux-cli should start")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn temp_file(extension: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(unique_name(extension));
    fs::write(&path, bytes).unwrap();
    path
}

fn unique_name(suffix: &str) -> String {
    format!(
        "osflux-cli-test-{}-{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        suffix
    )
}
