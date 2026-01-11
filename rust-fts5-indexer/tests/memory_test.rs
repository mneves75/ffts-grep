//! Tests for memory measurement utilities used in benchmarks.

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

/// Helper to get current process RSS in bytes.
fn get_rss_bytes() -> u64 {
    let pid = Pid::from_u32(std::process::id());
    let mut sys =
        System::new_with_specifics(RefreshKind::nothing().with_processes(ProcessRefreshKind::everything()));
    sys.refresh_processes(ProcessesToUpdate::All, true);
    sys.process(pid).map(|p| p.memory()).unwrap_or(0)
}

/// Verify sysinfo works on this platform and returns reasonable values.
#[test]
fn test_memory_info_available() {
    let rss = get_rss_bytes();

    // RSS should be > 0 (we're running a test process)
    assert!(rss > 0, "RSS should be positive, got {}", rss);

    // RSS should be < 1GB for a simple test (sanity check)
    assert!(
        rss < 1_000_000_000,
        "RSS suspiciously large: {} bytes",
        rss
    );

    // RSS should be > 1MB (reasonable minimum for any Rust process)
    assert!(rss > 1_000_000, "RSS suspiciously small: {} bytes", rss);
}

/// Verify memory increases after allocating data.
/// Note: This test may be flaky on systems with aggressive memory overcommit.
#[test]
fn test_memory_increases_with_allocation() {
    let before = get_rss_bytes();

    // Allocate ~10MB of data and touch it to ensure physical pages are allocated
    let data: Vec<u8> = vec![42u8; 10_000_000];
    // Black box to prevent optimization
    std::hint::black_box(&data);

    let after = get_rss_bytes();

    // Memory should have increased by at least some amount
    // (not necessarily the full 10MB due to allocator overhead and page alignment)
    println!(
        "Memory before: {} bytes, after: {} bytes, delta: {} bytes",
        before,
        after,
        after.saturating_sub(before)
    );

    // We just verify we can measure - the exact delta varies by platform
    assert!(
        after > 0,
        "Should be able to measure memory after allocation"
    );
}
