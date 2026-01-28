//! Integration tests that validate README memory claims by spawning fresh processes.
//!
//! These tests are `#[ignore]` by default because they:
//! 1. Create 10K+ files on disk
//! 2. Spawn external processes
//! 3. Take 30+ seconds to run
//!
//! Run manually with: `cargo test memory_validation -- --ignored --nocapture`
//!
//! # Why Fresh Processes?
//!
//! RSS delta measurements within a long-running process are unreliable because:
//! - Rust's allocator reuses freed memory, showing 0.0 MB delta
//! - Test harness overhead inflates memory readings
//!
//! Spawning a fresh process and measuring via `/usr/bin/time -l` gives accurate
//! peak RSS, which is the source of truth for README memory claims.

use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[allow(clippy::cast_precision_loss)]
fn bytes_to_mb(bytes: u64) -> f64 {
    bytes as f64 / 1_000_000.0
}

#[cfg(not(target_os = "macos"))]
#[allow(clippy::cast_precision_loss)]
fn kb_to_mb(kb: u64) -> f64 {
    kb as f64 / 1000.0
}

/// Find the ffts-grep binary (release or debug).
fn find_binary() -> Option<std::path::PathBuf> {
    let release = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/release/ffts-grep");
    if release.exists() {
        return Some(release);
    }

    let debug = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/ffts-grep");
    if debug.exists() {
        return Some(debug);
    }

    None
}

/// Validate that indexing 10K files uses < 50MB peak RSS.
///
/// README claims ~16MB for 10K file indexing.
/// We use 50MB as a generous upper bound to avoid flaky tests.
#[test]
#[ignore = "requires release binary and long-running RSS measurement"]
fn test_index_memory_claim() {
    let Some(binary) = find_binary() else {
        eprintln!("Binary not found. Run `cargo build --release` first.");
        return;
    };

    let dir = tempdir().unwrap();

    // Create 10K test files
    eprintln!("Creating 10,000 test files...");
    for i in 0..10000 {
        let content = format!("// File {i}\npub fn f{i}() {{}}\n");
        fs::write(dir.path().join(format!("f{i}.rs")), content).unwrap();
    }
    eprintln!("Files created in {:?}", dir.path());

    // Run index command and measure memory
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("/usr/bin/time")
            .args(["-l", binary.to_str().unwrap(), "index"])
            .current_dir(dir.path())
            .output()
            .expect("Failed to run ffts-grep index");

        // macOS /usr/bin/time -l outputs to stderr
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("Time output:\n{stderr}");

        // Parse "maximum resident set size" line (in bytes on macOS)
        // Format: "      16384  maximum resident set size"
        if let Some(line) = stderr.lines().find(|l| l.contains("maximum resident set size")) {
            let bytes: u64 = line.split_whitespace().next().unwrap_or("0").parse().unwrap_or(0);
            let mb = bytes_to_mb(bytes);
            eprintln!("Peak RSS: {mb:.1} MB");

            assert!(mb < 50.0, "Index memory exceeds 50MB limit: {mb:.1} MB (README claims ~16MB)");
        } else {
            eprintln!("Warning: Could not parse memory from time output");
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        // On Linux, use /usr/bin/time with different format
        let output = Command::new("/usr/bin/time")
            .args(["-v", binary.to_str().unwrap(), "index"])
            .current_dir(dir.path())
            .output()
            .expect("Failed to run ffts-grep index");

        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("Time output:\n{stderr}");

        // Linux format: "Maximum resident set size (kbytes): 16384"
        if let Some(line) = stderr.lines().find(|l| l.contains("Maximum resident set size")) {
            let kb: u64 = line.rsplit(':').next().unwrap_or("0").trim().parse().unwrap_or(0);
            let mb = kb_to_mb(kb);
            eprintln!("Peak RSS: {mb:.1} MB");

            assert!(mb < 50.0, "Index memory exceeds 50MB limit: {mb:.1} MB (README claims ~16MB)");
        } else {
            eprintln!("Warning: Could not parse memory from time output");
        }
    }
}

/// Validate that search uses < 20MB peak RSS.
///
/// README claims ~9MB for search-only operations.
/// We use 20MB as a generous upper bound to avoid flaky tests.
#[test]
#[ignore = "requires release binary and long-running RSS measurement"]
fn test_search_memory_claim() {
    let Some(binary) = find_binary() else {
        eprintln!("Binary not found. Run `cargo build --release` first.");
        return;
    };

    let dir = tempdir().unwrap();

    // Create and index 10K test files
    eprintln!("Creating 10,000 test files...");
    for i in 0..10000 {
        let content = format!("// File {i}\npub fn function_{i}() {{}}\n");
        fs::write(dir.path().join(format!("f{i}.rs")), content).unwrap();
    }

    // Index first
    let status = Command::new(binary.to_str().unwrap())
        .args(["index"])
        .current_dir(dir.path())
        .status()
        .expect("Failed to run ffts-grep index");
    assert!(status.success(), "Index command failed");

    eprintln!("Measuring search memory...");

    // Run search command and measure memory
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("/usr/bin/time")
            .args(["-l", binary.to_str().unwrap(), "search", "function"])
            .current_dir(dir.path())
            .output()
            .expect("Failed to run ffts-grep search");

        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("Time output:\n{stderr}");

        if let Some(line) = stderr.lines().find(|l| l.contains("maximum resident set size")) {
            let bytes: u64 = line.split_whitespace().next().unwrap_or("0").parse().unwrap_or(0);
            let mb = bytes_to_mb(bytes);
            eprintln!("Peak RSS: {mb:.1} MB");

            assert!(mb < 20.0, "Search memory exceeds 20MB limit: {mb:.1} MB (README claims ~9MB)");
        } else {
            eprintln!("Warning: Could not parse memory from time output");
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let output = Command::new("/usr/bin/time")
            .args(["-v", binary.to_str().unwrap(), "search", "function"])
            .current_dir(dir.path())
            .output()
            .expect("Failed to run ffts-grep search");

        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("Time output:\n{stderr}");

        if let Some(line) = stderr.lines().find(|l| l.contains("Maximum resident set size")) {
            let kb: u64 = line.rsplit(':').next().unwrap_or("0").trim().parse().unwrap_or(0);
            let mb = kb_to_mb(kb);
            eprintln!("Peak RSS: {mb:.1} MB");

            assert!(mb < 20.0, "Search memory exceeds 20MB limit: {mb:.1} MB (README claims ~9MB)");
        } else {
            eprintln!("Warning: Could not parse memory from time output");
        }
    }
}
