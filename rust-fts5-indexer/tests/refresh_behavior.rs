use assert_cmd::Command;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_search_refresh_indexes_new_files() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("old.txt"), "old_content").unwrap();

    Command::new(assert_cmd::cargo::cargo_bin!("ffts-grep"))
        .args(["--project-dir", dir.path().to_str().unwrap(), "index"])
        .assert()
        .success();

    fs::write(dir.path().join("new.txt"), "refresh_token").unwrap();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("ffts-grep"))
        .args(["--project-dir", dir.path().to_str().unwrap(), "search", "refresh_token"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.trim().is_empty());

    let output = Command::new(assert_cmd::cargo::cargo_bin!("ffts-grep"))
        .args([
            "--project-dir",
            dir.path().to_str().unwrap(),
            "search",
            "--refresh",
            "refresh_token",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.lines().any(|line| line.ends_with("new.txt")));
}

#[test]
fn test_refresh_flag_rejected_for_index() {
    let dir = tempdir().unwrap();

    Command::new(assert_cmd::cargo::cargo_bin!("ffts-grep"))
        .args(["--project-dir", dir.path().to_str().unwrap(), "index", "--refresh"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn test_refresh_via_stdin_json() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("seed.txt"), "seed").unwrap();

    Command::new(assert_cmd::cargo::cargo_bin!("ffts-grep"))
        .args(["--project-dir", dir.path().to_str().unwrap(), "index"])
        .assert()
        .success();

    fs::write(dir.path().join("stdin.txt"), "stdin_token").unwrap();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("ffts-grep"))
        .args(["--project-dir", dir.path().to_str().unwrap()])
        .write_stdin("{\"query\":\"stdin_token\"}\n")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.trim().is_empty());

    let output = Command::new(assert_cmd::cargo::cargo_bin!("ffts-grep"))
        .args(["--project-dir", dir.path().to_str().unwrap()])
        .write_stdin("{\"query\":\"stdin_token\",\"refresh\":true}\n")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.lines().any(|line| line.ends_with("stdin.txt")));
}

#[test]
fn test_refresh_requires_auto_init_when_missing() {
    let dir = tempdir().unwrap();

    Command::new(assert_cmd::cargo::cargo_bin!("ffts-grep"))
        .args([
            "--project-dir",
            dir.path().to_str().unwrap(),
            "search",
            "--refresh",
            "--no-auto-init",
            "missing_token",
        ])
        .assert()
        .failure()
        .code(2);
}
