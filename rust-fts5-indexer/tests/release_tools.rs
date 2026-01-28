use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}

#[test]
fn test_check_version_consistency() {
    let mut cmd = cargo_bin_cmd!("release-tools");
    cmd.arg("check-version").assert().success().stdout(contains("Version check OK"));
}

#[test]
fn test_release_notes_for_version() {
    let changelog = repo_root().join("CHANGELOG.md");

    let mut cmd = cargo_bin_cmd!("release-tools");
    cmd.args([
        "release-notes",
        "--changelog",
        changelog.to_string_lossy().as_ref(),
        "--version",
        "0.11",
    ])
    .assert()
    .success()
    .stdout(contains("## [0.11]"));
}

#[test]
fn test_checklist_runs() {
    let mut cmd = cargo_bin_cmd!("release-tools");
    cmd.arg("checklist").assert().success().stdout(contains("Release checklist"));
}
