# Chapter 13: testing.md - Testing the Application

> "Tests are not just checks—they're living documentation of what the code should do." — Testing Philosophy

## 13.1 What Does This Chapter Cover? (In Simple Terms)

This chapter explains how we test the FTS5 File Indexer to ensure it works correctly, stays working correctly, and doesn't introduce bugs when we make changes. Testing is like having a quality control department in a factory—every product (code change) goes through inspection before shipping.

### The Quality Control Analogy

| Factory Quality Control | Software Testing |
|-------------------------|------------------|
| Checking products on assembly line | Running automated tests |
| Rejecting defective products | Failing tests catch bugs |
| Documenting test procedures | Tests document expected behavior |
| Safety certifications | Regression prevention |

---

## 13.2 The Test Philosophy

### Tests as Documentation

Tests are the **most accurate documentation** of how code should behave. When you wonder "What does this function actually do?", look at its tests:

```rust
/// This doc comment says: "Upsert a file"
/// But the tests show: ONLY if content hash is different!
#[test]
fn test_lazy_invalidation_skips_unchanged() {
    // ... test shows indexed_at doesn't change when content is same
}
```

### The Testing Pyramid

```
                    ┌─────────────┐
                   /   Manual     \          Few: Exploratory, edge cases
                  /   Testing      \
                 /                  \
                ├────────────────────┤
               /   Integration       \      Medium: API, multi-component tests
              /     Tests             \
             /                        \
            ├──────────────────────────┤
           /                            \
          /     Unit Tests              \  Most: Fast, isolated, focused
         /                                \
        └──────────────────────────────────┘
```

---

## 13.3 Test Types in This Project

### Unit Tests

Located **inline** in source files with `#[cfg(test)]`:

```rust
// In db.rs, indexer.rs, etc.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_something() {
        // Test isolated functionality
    }
}
```

| Module | Unit Test Focus |
|--------|-----------------|
| `db.rs` | Schema, queries, triggers |
| `indexer.rs` | File processing, UTF-8 validation |
| `search.rs` | Query sanitization |
| `init.rs` | Gitignore parsing |

### Integration Tests

Located in `tests/integration.rs` (see `tests/integration.rs:1-50`):

```rust
/// Helper to create a test database and indexer.
fn create_test_indexer(dir: &tempfile::TempDir) -> Indexer {
    let db_path = dir.path().join(DB_NAME);
    let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
    db.init_schema().unwrap();
    let config = IndexerConfig::default();
    Indexer::new(dir.path(), db, config)
}
```

| Test Category | Examples |
|---------------|----------|
| Core functionality | `test_index_and_search_roundtrip`, `test_unicode_content` |
| Edge cases | `test_indexer_skips_binary_files`, `test_indexer_skips_large_files` |
| Security | `test_symlink_path_traversal_attack_rejected` |
| Performance | `test_transaction_batching_performance` |
| Error handling | `test_fts5_query_error_handling`, `test_permission_denied_handling` |
| CLI refresh behavior | `tests/refresh_behavior.rs` (refresh validation and auto-init coverage) |

---

## 13.4 Key Integration Tests Explained

### Core Functionality: Index and Search Roundtrip

See `tests/integration.rs:23-43`:

```rust
#[test]
fn test_index_and_search_roundtrip() {
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    // Create test files
    fs::write(dir.path().join("main.rs"), "fn main() { println!(\"Hello\"); }").unwrap();
    fs::write(dir.path().join("lib.rs"), "pub fn add(a: i32, b: i32) -> i32 { a + b }").unwrap();
    fs::write(dir.path().join("README.md"), "# Example\n\nThis is a test.").unwrap();

    // Index files
    let stats = indexer.index_directory().unwrap();
    assert_eq!(stats.files_indexed, 3);
    assert!(stats.bytes_indexed > 0);

    // Search for "main"
    let db = indexer.db_mut();
    let results = db.search("main", false, 10).unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].path, "main.rs");
}
```

This test verifies the **complete data flow**:
1. Create files
2. Index them
3. Search and verify results

### Edge Case: Binary Files

See `tests/integration.rs:198-213`:

```rust
#[test]
fn test_indexer_skips_binary_files() {
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    // Create a binary file
    let binary_content = [0x80, 0x81, 0x82, 0xff, 0x00];
    fs::write(dir.path().join("binary.bin"), binary_content).unwrap();

    // Create a text file
    fs::write(dir.path().join("text.rs"), "text content").unwrap();

    let stats = indexer.index_directory().unwrap();
    assert_eq!(stats.files_indexed, 1); // Only text.rs
    assert_eq!(stats.files_skipped, 1); // binary.bin was skipped
}
```

This test ensures **binary files are rejected** (UTF-8 validation from `indexer.rs:279-280`).

### Security: Symlink Attack Prevention

See `tests/integration.rs:349-387`:

```rust
/// Verify that symlinks pointing outside the project root are rejected.
/// This prevents path traversal attacks where malicious symlinks could
/// cause the indexer to index files outside the intended directory.
#[test]
fn test_symlink_path_traversal_attack_rejected() {
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    // Create a file outside the project root
    let outside_dir = tempdir().unwrap();
    let outside_file = outside_dir.path().join("secret.txt");
    std::fs::write(&outside_file, "sensitive data").unwrap();

    // Create a symlink inside the project pointing to the outside file
    let symlink = dir.path().join("escape_link");
    if cfg!(unix) {
        std::os::unix::fs::symlink(&outside_file, &symlink).unwrap();
    } else {
        // Skip test on non-Unix systems
        return;
    }

    // Create a normal file inside the project
    std::fs::write(dir.path().join("normal.rs"), "normal content").unwrap();

    let stats = indexer.index_directory().unwrap();

    // Only the normal.rs file should be indexed
    assert_eq!(stats.files_indexed, 1);
    assert!(stats.files_skipped >= 1);

    // Verify the outside file was NOT indexed
    let db = indexer.db_mut();
    let results = db.search("sensitive", false, 10).unwrap();
    assert!(results.is_empty());
}
```

This test proves **symlink security works**—the `is_within_root()` check from `indexer.rs:308-324` prevents escaping the project root.

---

## 13.5 Performance Tests

### Transaction Batching

See `tests/integration.rs:389-414`:

```rust
/// Verify transaction batching provides significant performance improvement.
/// With transaction batching, indexing 1000 files should complete in <5 seconds.
/// Without batching (auto-commit per file), it would take 10-20+ seconds.
#[test]
fn test_transaction_batching_performance() {
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    // Create 1000 small files to test batching performance
    for i in 0..1000 {
        let content = format!("// Test file {i}\npub fn test_{i}() {{}}\n");
        fs::write(dir.path().join(format!("test_{i}.rs")), content).unwrap();
    }

    let start = std::time::Instant::now();
    let stats = indexer.index_directory().unwrap();
    let duration = start.elapsed();

    assert_eq!(stats.files_indexed, 1000);
    assert!(
        duration.as_secs() < 5,
        "Indexing 1000 files took {duration:?}, expected <5s"
    );
}
```

| Batching | Expected Time | Why |
|----------|---------------|-----|
| Without batching | 10-20 seconds | Each INSERT triggers fsync |
| With batching | <5 seconds | Batched fsync every 500 files |

---

## 13.6 Concurrent Access Tests

### WAL Mode + Busy Timeout

See `tests/integration.rs:537-554`:

```rust
/// Verify WAL `busy_timeout` handles concurrent access gracefully.
#[test]
fn test_database_lock_timeout() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join(DB_NAME);

    // Create and initialize first connection
    let db1 = Database::open(&db_path, &PragmaConfig::default()).unwrap();
    db1.init_schema().unwrap();

    // Create second connection - should succeed thanks to WAL mode + busy_timeout
    let db2 = Database::open(&db_path, &PragmaConfig::default()).unwrap();

    // Both connections should be able to read
    let count1 = db1.get_file_count().unwrap();
    let count2 = db2.get_file_count().unwrap();
    assert_eq!(count1, count2);
}
```

WAL mode allows **concurrent readers**—the `busy_timeout` pragma from `db.rs:47-49` prevents failures during contention.

---

## 13.7 State Machine Verification Tests

### Health State Machine

See `tests/integration.rs:954-1015`:

```rust
/// Verify health check state machine correctly identifies all DatabaseHealth variants
/// (docs/state-machines/04-search-flow.md).
#[test]
fn test_health_state_machine_transitions() {
    use ffts_indexer::health::{check_health_fast, DatabaseHealth};

    // State 1: Missing - no database file
    let dir_missing = tempdir().unwrap();
    assert_eq!(check_health_fast(dir_missing.path()), DatabaseHealth::Missing);
    assert!(DatabaseHealth::Missing.needs_init());

    // State 2: Empty - schema exists but no files
    let dir_empty = tempdir().unwrap();
    let db_empty = Database::open(&dir_empty.path().join(DB_NAME), &PragmaConfig::default()).unwrap();
    db_empty.init_schema().unwrap();
    assert_eq!(check_health_fast(dir_empty.path()), DatabaseHealth::Empty);

    // State 3: Healthy - schema + content exists
    let dir_healthy = tempdir().unwrap();
    let db_healthy = Database::open(&dir_healthy.path().join(DB_NAME), &PragmaConfig::default()).unwrap();
    db_healthy.init_schema().unwrap();
    db_healthy.upsert_file("test.rs", "fn main() {}", 0, 12).unwrap();
    let health = check_health_fast(dir_healthy.path());
    assert_eq!(health, DatabaseHealth::Healthy);

    // State 4: WrongApplicationId - different app's database
    // State 5: SchemaInvalid - correct app ID but incomplete schema
    // State 6: Corrupted/Unreadable - garbage file
}
```

### FTS5 Trigger Auto-Sync

See `tests/integration.rs:1017-1045`:

```rust
/// Verify FTS5 trigger auto-sync behavior
/// (docs/state-machines/03-database-states.md).
#[test]
fn test_fts5_trigger_auto_sync() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join(DB_NAME);
    let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
    db.init_schema().unwrap();

    // Test INSERT trigger (files_ai)
    db.upsert_file("test.rs", "unique_insert_content", 0, 21).unwrap();
    let results = db.search("unique_insert_content", false, 10).unwrap();
    assert_eq!(results.len(), 1, "FTS5 should index after INSERT");

    // Test UPDATE trigger (files_au)
    db.upsert_file("test.rs", "updated_content_here", 1, 20).unwrap();
    let old_results = db.search("unique_insert_content", false, 10).unwrap();
    let new_results = db.search("updated_content_here", false, 10).unwrap();
    assert!(old_results.is_empty(), "Old content should be removed");
    assert_eq!(new_results.len(), 1, "New content should be in FTS5");

    // Test DELETE trigger (files_ad)
    db.delete_file("test.rs").unwrap();
    let deleted_results = db.search("updated_content_here", false, 10).unwrap();
    assert!(deleted_results.is_empty(), "FTS5 should remove deleted content");
}
```

This test proves **triggers work correctly**—the three triggers from `db.rs:172-202` (INSERT, UPDATE, DELETE) automatically sync the FTS5 index.

---

## 13.8 Doctor and Init Tests

### 10-Check Pipeline Verification

See `tests/integration.rs:1102-1147`:

```rust
/// Verify doctor check pipeline runs all 10 checks in order
/// (docs/state-machines/05-doctor-diagnostics.md).
#[test]
fn test_doctor_10_check_pipeline() {
    let dir = tempdir().unwrap();

    // Setup healthy database
    let db_path = dir.path().join(DB_NAME);
    let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
    db.init_schema().unwrap();
    db.upsert_file("test.rs", "fn main() {}", 0, 12).unwrap();
    update_gitignore(dir.path()).unwrap();

    let mut doctor = Doctor::new(dir.path(), true);
    let summary = doctor.run();

    // Verify all 10 checks ran
    let checks = doctor.checks();
    assert_eq!(checks.len(), 10, "Doctor should run exactly 10 checks");

    // Verify check names match documented order
    let expected_names = [
        "Database exists",
        "Database readable",
        "Application ID",
        "Schema complete",
        "FTS5 integrity",
        "Journal mode",
        "File count",
        "Gitignore",
        "Binary availability",
        "Orphan WAL files",
    ];
```

### Gitignore Idempotency

See `tests/integration.rs:733-752`:

```rust
/// Verify init is idempotent (no duplicate entries).
#[test]
fn test_init_idempotent() {
    let dir = tempdir().unwrap();

    // First init
    let result1 = update_gitignore(dir.path()).unwrap();
    assert!(matches!(result1, GitignoreResult::Created(4)));

    let content1 = fs::read_to_string(dir.path().join(".gitignore")).unwrap();

    // Second init - should do nothing
    let result2 = update_gitignore(dir.path()).unwrap();
    assert_eq!(result2, GitignoreResult::AlreadyComplete);

    let content2 = fs::read_to_string(dir.path().join(".gitignore")).unwrap();

    // Content should be identical
    assert_eq!(content1, content2);
}
```

---

## 13.9 Running Tests

### All Tests

```bash
cd rust-fts5-indexer
cargo test
```

### Single Test

```bash
cargo test test_index_and_search_roundtrip
```

### With Output Visible

```bash
cargo test -- --nocapture
```

### Benchmarks

```bash
cargo bench
```

---

## 13.10 Test Patterns Explained

### Pattern 1: TempDir for Isolation

```rust
#[test]
fn test_something() {
    let dir = tempdir().unwrap();  // Auto-cleanup after test
    // ... test with dir.path()
}
```

The `tempdir` crate ensures **test isolation**—each test gets a fresh temporary directory that's automatically deleted.

### Pattern 2: Arrange-Act-Assert

```rust
#[test]
fn test_search_finds_indexed_files() {
    // Arrange: Create and index files
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);
    fs::write(dir.path().join("test.rs"), "content").unwrap();
    indexer.index_directory().unwrap();

    // Act: Perform search
    let db = indexer.db_mut();
    let results = db.search("content", false, 10).unwrap();

    // Assert: Verify results
    assert_eq!(results.len(), 1);
}
```

### Pattern 3: Platform-Specific Tests

```rust
#[cfg(unix)]
#[test]
fn test_symlink_handling() {
    // Only runs on Unix
}

#[cfg(windows)]
#[test]
fn test_windows_paths() {
    // Only runs on Windows
}
```

### Pattern 4: Error Handling Tests

```rust
#[test]
fn test_fts5_query_error_handling() {
    let test_queries = vec![
        ("test", true),           // Should succeed
        ("test-function", false), // Expected to fail (FTS5 limitation)
    ];

    for (query, should_succeed) in test_queries {
        let result = db.search(query, false, 10);
        if should_succeed {
            assert!(result.is_ok());
        } else {
            // Query may fail but shouldn't panic
            let _ = result; // Just verify no panic
        }
    }
}
```

---

## 13.11 What Tests Are Missing?

### Self-Critique: What Should We Test But Don't?

| Missing Test | Why It Matters | Priority |
|--------------|----------------|----------|
| Concurrent init race condition | Auto-init might fail with multiple processes | Medium |
| Large file (100MB+) edge case | What happens with very large files? | Low |
| Unicode edge cases (emoji, RTL) | Full Unicode support verification | Low |
| Disk full scenario | What happens when writes fail? | Medium |
| Corruption recovery | Can we recover from partial writes? | Low |

### Exercise 13.7 (At End) asks you to identify more gaps.

---

## 13.12 Chapter Summary

| Concept | What We Learned |
|---------|-----------------|
| Testing pyramid | Unit tests (fast) → Integration (comprehensive) |
| TempDir isolation | Each test gets clean environment |
| Test patterns | Arrange-Act-Assert, platform-specific, error handling |
| State machine tests | Verify all health states transition correctly |
| Performance tests | Ensure batching actually improves speed |
| Security tests | Verify symlink attacks are blocked |
| Idempotency tests | Verify init can be run multiple times safely |

---

## 13.13 Exercises

### Exercise 13.1: Run the Test Suite

Run all tests and observe the output:

```bash
cd rust-fts5-indexer
cargo test
```

**Deliverable:** Show the test output summary.

### Exercise 13.2: Run a Single Test

Run just the FTS5 trigger test:

```bash
cargo test test_fts5_trigger_auto_sync
```

**Deliverable:** Show the test result.

### Exercise 13.3: Add a New Test

Add a test for searching with special characters:

```rust
#[test]
fn test_search_with_special_characters() {
    // Create a file
    // Search for queries with special chars
    // Verify sanitization works
}
```

**Deliverable:** Show your test code and result.

### Exercise 13.4: Verify Performance Test

Run the transaction batching performance test:

```bash
cargo test test_transaction_batching_performance
```

**Deliverable:** Show how long it took to index 1000 files.

### Exercise 13.5: Test Platform-Specific Code

Check if platform-specific tests pass:

```bash
# On Unix
cargo test test_symlink_path_traversal_attack_rejected

# On Windows
cargo test test_windows_paths
```

**Deliverable:** Show the test results.

### Exercise 13.6: Write a Failure Test

Write a test that verifies error handling:

```rust
#[test]
fn test_invalid_utf8_is_rejected() {
    // Create file with invalid UTF-8
    // Verify it was skipped during indexing
}
```

**Deliverable:** Show your test code.

### Exercise 13.7: Gap Analysis

Review the test suite and identify missing test cases:

**Deliverable:** List 3-5 tests that should be added and explain why.

---

## 13.14 Self-Correction Exercise

After reviewing the test suite, answer:

1. **What works well?**
   - Clear test names that describe behavior
   - Integration tests cover complete workflows
   - Security tests verify attack prevention

2. **What needs improvement?**
   - [Your answer here]
   - [Your answer here]

3. **How would you improve the test suite?**
   - [Your answer here]

---

**Next Chapter**: [Chapter 14: exercises-solutions.md - Complete Solutions](14-exercises-solutions.md)
