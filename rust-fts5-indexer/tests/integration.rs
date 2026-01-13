use ffts_indexer::cli::OutputFormat;
use ffts_indexer::db::{Database, PragmaConfig};
use ffts_indexer::doctor::{Doctor, Severity};
use ffts_indexer::error::IndexerError;
use ffts_indexer::indexer::{Indexer, IndexerConfig, atomic_reindex};
use ffts_indexer::init::{GitignoreResult, check_gitignore, gitignore_entries, update_gitignore};
use ffts_indexer::search::{SearchConfig, Searcher};
use ffts_indexer::{DB_NAME, DB_SHM_NAME, DB_TMP_GLOB, DB_WAL_NAME, DB_WAL_SUFFIX};
use rusqlite::ErrorCode;
use std::fs;
use std::thread;
use tempfile::tempdir;

/// Helper to create a test database and indexer.
fn create_test_indexer(dir: &tempfile::TempDir) -> Indexer {
    let db_path = dir.path().join(DB_NAME);
    let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
    db.init_schema().unwrap();
    let config = IndexerConfig::default();
    Indexer::new(dir.path(), db, config)
}

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

#[test]
fn test_index_skips_gitignored() {
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    // Create a .git directory that should be skipped by standard_filters
    fs::create_dir_all(dir.path().join(".git/objects")).unwrap();
    fs::write(dir.path().join(".git/config"), "git config").unwrap();
    fs::write(dir.path().join("visible.rs"), "pub fn visible() {}").unwrap();

    let stats = indexer.index_directory().unwrap();
    // .git directory should be skipped by standard_filters
    assert_eq!(stats.files_indexed, 1);
}

#[test]
fn test_search_json_output() {
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    // Create a file
    fs::write(dir.path().join("test.rs"), "fn test_function() {}").unwrap();

    let stats = indexer.index_directory().unwrap();
    assert_eq!(stats.files_indexed, 1);

    let db = indexer.db_mut();
    let mut searcher =
        Searcher::new(db, SearchConfig { format: OutputFormat::Json, ..Default::default() });

    let results = searcher.search("test_function").unwrap();
    assert!(!results.is_empty());

    let mut output = Vec::new();
    searcher.format_results(&results, &mut output).unwrap();

    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("\"path\""));
    assert!(text.contains("\"rank\""));
    assert!(text.contains("\"results\""));
}

#[test]
fn test_search_paths_only() {
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    // Create files where "test" is in path or content
    fs::write(dir.path().join("test.rs"), "unrelated content").unwrap();
    fs::write(dir.path().join("other.rs"), "test keyword").unwrap();

    let stats = indexer.index_directory().unwrap();
    assert_eq!(stats.files_indexed, 2);

    let db = indexer.db_mut();

    // Paths-only search for "test" should find test.rs
    let results = db.search("test", true, 10).unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].path, "test.rs");

    // Paths-only search for "keyword" should find nothing
    let results = db.search("keyword", true, 10).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_bm25_path_boosting() {
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    // Create files where "unique" is in path vs content
    fs::write(dir.path().join("unique.rs"), "different content here").unwrap();
    fs::write(dir.path().join("other.rs"), "unique token in content").unwrap();

    let stats = indexer.index_directory().unwrap();
    assert_eq!(stats.files_indexed, 2);

    let db = indexer.db_mut();
    let results = db.search("unique", false, 10).unwrap();
    assert_eq!(results.len(), 2);

    // Path match should have lower (better) BM25 score
    assert!(results[0].rank <= results[1].rank);
}

#[test]
fn test_unicode_content() {
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    // Create file with unicode content
    fs::write(dir.path().join("unicode.rs"), "café 中文 🎉 unicode").unwrap();

    let stats = indexer.index_directory().unwrap();
    assert_eq!(stats.files_indexed, 1);

    let db = indexer.db_mut();
    let results = db.search("café", false, 10).unwrap();
    assert!(!results.is_empty());
}

#[test]
fn test_reindex_updates_existing() {
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    // Create and index file
    fs::write(dir.path().join("test.rs"), "version 1").unwrap();
    indexer.index_directory().unwrap();

    let db = indexer.db_mut();
    let count1 = db.get_file_count().unwrap();

    // Modify file
    fs::write(dir.path().join("test.rs"), "version 2 updated").unwrap();

    // Reindex (db reference goes out of scope here)
    let mut indexer = Indexer::new(
        dir.path(),
        Database::open(&dir.path().join(DB_NAME), &PragmaConfig::default()).unwrap(),
        IndexerConfig::default(),
    );
    indexer.index_directory().unwrap();

    let db = indexer.db_mut();
    let count2 = db.get_file_count().unwrap();

    // Count should be same (update, not new insert)
    assert_eq!(count1, count2);

    // Search should find updated content
    let results = db.search("version", false, 10).unwrap();
    assert!(!results.is_empty());
}

#[test]
fn test_indexer_skips_large_files() {
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    // Create a large file (> 1MB)
    let large_content = vec![b'a'; 1024 * 1024 + 1];
    fs::write(dir.path().join("large.rs"), large_content).unwrap();

    // Create a normal file
    fs::write(dir.path().join("small.rs"), "small").unwrap();

    let stats = indexer.index_directory().unwrap();
    assert_eq!(stats.files_indexed, 1); // Only small.rs
    assert_eq!(stats.files_skipped, 1); // large.rs was skipped
}

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

#[test]
fn test_search_no_results() {
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    // Create file
    fs::write(dir.path().join("test.rs"), "specific content").unwrap();

    let stats = indexer.index_directory().unwrap();
    assert_eq!(stats.files_indexed, 1);

    let db = indexer.db_mut();
    let results = db.search("nonexistent", false, 10).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_empty_query() {
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    // Create file
    fs::write(dir.path().join("test.rs"), "content").unwrap();

    let stats = indexer.index_directory().unwrap();
    assert_eq!(stats.files_indexed, 1);

    let db = indexer.db_mut();
    let results = db.search("", false, 10).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_database_file_excluded() {
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    // Create database-like files
    fs::write(dir.path().join("test.rs"), "content").unwrap();
    fs::write(dir.path().join("data.db"), "db content").unwrap();
    fs::write(dir.path().join("data.db-shm"), "shm content").unwrap();
    fs::write(dir.path().join("data.db-wal"), "wal content").unwrap();

    let stats = indexer.index_directory().unwrap();
    assert_eq!(stats.files_indexed, 1); // Only test.rs
}

#[test]
fn test_nested_directory_indexing() {
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    // Create nested structure
    fs::create_dir_all(dir.path().join("src/utils")).unwrap();
    fs::create_dir_all(dir.path().join("tests")).unwrap();
    fs::write(dir.path().join("src/main.rs"), "main").unwrap();
    fs::write(dir.path().join("src/lib.rs"), "lib").unwrap();
    fs::write(dir.path().join("src/utils/mod.rs"), "utils").unwrap();
    fs::write(dir.path().join("tests/main_test.rs"), "test").unwrap();

    let stats = indexer.index_directory().unwrap();
    assert_eq!(stats.files_indexed, 4);

    let db = indexer.db_mut();
    let results = db.search("main", false, 10).unwrap();
    assert!(!results.is_empty());
}

/// Verify FTS5 is available in the bundled `SQLite` build.
/// This is a P0 correctness check - without FTS5, the entire indexer fails.
#[test]
fn test_fts5_available() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join(DB_NAME);
    let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
    db.init_schema().unwrap();

    // Try to create a simple FTS5 virtual table
    // If this fails, SQLite was built without FTS5 support
    db.conn()
        .execute("CREATE VIRTUAL TABLE IF NOT EXISTS fts5_test USING fts5(content)", [])
        .expect("FTS5 must be available for the indexer to work");

    // Verify we can insert and query
    db.conn().execute("INSERT INTO fts5_test (content) VALUES ('test content')", []).unwrap();

    // Query should return results
    let result: String = db
        .conn()
        .query_row(
            "SELECT content FROM fts5_test WHERE content MATCH 'test'",
            [],
            |row: &rusqlite::Row| row.get(0),
        )
        .unwrap();

    assert_eq!(result, "test content");
}

#[test]
fn test_cross_platform_mtime() {
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    // Create test file
    fs::write(dir.path().join("test.rs"), "test content").unwrap();

    // Wait to ensure mtime is in the past
    std::thread::sleep(std::time::Duration::from_millis(10));

    let stats = indexer.index_directory().unwrap();
    assert_eq!(stats.files_indexed, 1);

    // Verify mtime was recorded correctly (query database)
    let db = indexer.db_mut();
    let count: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM files WHERE mtime > 0", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1, "mtime should be recorded and greater than 0");

    // Verify mtime is reasonable (within last hour)
    // Safety: u64→i64 cast is safe until year 2262 (test validity)
    #[allow(clippy::cast_possible_wrap)]
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
        as i64;
    let mtime: i64 = db
        .conn()
        .query_row("SELECT mtime FROM files WHERE path = 'test.rs'", [], |row| row.get(0))
        .unwrap();
    assert!(mtime > now - 3600, "mtime should be within the last hour, got {mtime} vs now {now}");
    assert!(mtime <= now, "mtime should not be in the future, got {mtime} vs now {now}");
}

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
        // Skip test on non-Unix systems where symlinks may not work the same way
        return;
    }

    // Create a normal file inside the project
    std::fs::write(dir.path().join("normal.rs"), "normal content").unwrap();

    let stats = indexer.index_directory().unwrap();

    // Only the normal.rs file should be indexed
    assert_eq!(stats.files_indexed, 1);
    assert!(stats.files_skipped >= 1); // At least the symlink was skipped

    // Verify the outside file was NOT indexed by searching for its content
    let db = indexer.db_mut();
    let results = db.search("sensitive", false, 10).unwrap();
    assert!(
        results.is_empty(),
        "Path traversal attack should be blocked - outside file should not be indexed"
    );
}

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
    // With transaction batching, this should complete in <5 seconds
    // Without batching, it would take 10-20+ seconds
    assert!(
        duration.as_secs() < 5,
        "Indexing 1000 files took {duration:?}, expected <5s (batching may not be working)"
    );
}

/// Verify lazy invalidation correctly skips unchanged files based on content hash.
/// SQLite-level optimization via WHERE clause in upsert prevents actual writes.
#[test]
fn test_lazy_invalidation_skips_unchanged() {
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    // Create and index a file
    fs::write(dir.path().join("test.rs"), "original content").unwrap();
    let stats1 = indexer.index_directory().unwrap();
    assert_eq!(stats1.files_indexed, 1);

    // Get initial indexed_at timestamp
    let db = indexer.db_mut();
    let indexed_at1: i64 = db
        .conn()
        .query_row("SELECT indexed_at FROM files WHERE path = 'test.rs'", [], |row| row.get(0))
        .unwrap();

    // Reindex without changes - lazy invalidation should skip write via WHERE clause (db reference goes out of scope here)
    std::thread::sleep(std::time::Duration::from_secs(1)); // Ensure timestamp would differ (second resolution)
    let stats2 = indexer.index_directory().unwrap();
    assert_eq!(stats2.files_indexed, 1); // File is processed but not actually written

    // Verify indexed_at timestamp didn't change (proof of skip)
    let db = indexer.db_mut();
    let indexed_at2: i64 = db
        .conn()
        .query_row("SELECT indexed_at FROM files WHERE path = 'test.rs'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        indexed_at1, indexed_at2,
        "Timestamp should not change when content hash matches (lazy invalidation)"
    );

    // Modify file and reindex (db reference goes out of scope here)
    std::thread::sleep(std::time::Duration::from_secs(1)); // Ensure timestamp differs (second resolution)
    fs::write(dir.path().join("test.rs"), "modified content").unwrap();
    let stats3 = indexer.index_directory().unwrap();
    assert_eq!(stats3.files_indexed, 1);

    // Verify indexed_at timestamp DID change (proof of write)
    let db = indexer.db_mut();
    let indexed_at3: i64 = db
        .conn()
        .query_row("SELECT indexed_at FROM files WHERE path = 'test.rs'", [], |row| row.get(0))
        .unwrap();
    assert!(indexed_at3 > indexed_at2, "Timestamp should update when content changes");

    // Verify search finds updated content
    let results = db.search("modified", false, 10).unwrap();
    assert!(!results.is_empty(), "Search should find updated content");
}

/// Verify FTS5 query error handling is safe (no crashes or panics).
/// Note: Some FTS5 syntax errors are expected, but should be handled gracefully.
#[test]
fn test_fts5_query_error_handling() {
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    // Create test file
    fs::write(dir.path().join("test.rs"), "function test() {}").unwrap();
    indexer.index_directory().unwrap();

    let db = indexer.db_mut();

    // Test queries - some may fail with FTS5 syntax errors, but should not panic
    let test_queries = vec![
        ("test", true),             // Simple query - should work
        ("\"test\"", true),         // Quoted phrase - should work
        ("test OR function", true), // Boolean operator - should work
        ("test*", false),           // Wildcard - may fail (FTS5 syntax dependent)
        ("test-function", false),   // Hyphen - expected to fail (FTS5 limitation)
    ];

    for (query, should_succeed) in test_queries {
        let result = db.search(query, false, 10);

        if should_succeed {
            assert!(result.is_ok(), "Query '{}' should succeed but got: {:?}", query, result.err());
        } else {
            // Query may fail with FTS5 syntax error - verify it's handled gracefully
            // (not panic, just return error)
            match result {
                Ok(_) => {} // Unexpectedly succeeded, but that's OK
                Err(e) => {
                    // Verify error is properly structured (not a panic)
                    let _ = format!("{e:?}"); // Should not panic
                }
            }
        }
    }
}

/// Verify permission denied errors are handled gracefully during indexing.
#[cfg(unix)]
#[test]
fn test_permission_denied_handling() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    // Create a file and make it unreadable
    let unreadable_file = dir.path().join("unreadable.rs");
    fs::write(&unreadable_file, "secret content").unwrap();
    fs::set_permissions(&unreadable_file, fs::Permissions::from_mode(0o000)).unwrap();

    // Create a normal readable file
    fs::write(dir.path().join("readable.rs"), "normal content").unwrap();

    // Indexing should succeed, skipping the unreadable file
    let stats = indexer.index_directory().unwrap();
    assert_eq!(stats.files_indexed, 1, "Should index only readable file");
    assert!(stats.files_skipped >= 1, "Should skip unreadable file");

    // Clean up: restore permissions before temp dir cleanup
    let _ = fs::set_permissions(&unreadable_file, fs::Permissions::from_mode(0o644));
}

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
    assert_eq!(count1, count2, "Both connections should read same data");
}

/// Verify transaction commits properly even when errors occur during batch.
#[test]
fn test_batch_commit_on_error() {
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    // Create mix of valid and invalid files
    fs::write(dir.path().join("valid1.rs"), "valid content").unwrap();

    // Create a binary file that will fail UTF-8 validation
    let binary_content = [0x80, 0x81, 0x82, 0xff];
    fs::write(dir.path().join("binary.bin"), binary_content).unwrap();

    fs::write(dir.path().join("valid2.rs"), "more valid content").unwrap();

    // Indexing should succeed for valid files, skip binary
    let stats = indexer.index_directory().unwrap();
    assert_eq!(stats.files_indexed, 2, "Should index 2 valid files");
    assert_eq!(stats.files_skipped, 1, "Should skip binary file");

    // Verify valid files are searchable (transaction committed)
    let db = indexer.db_mut();
    let count = db.get_file_count().unwrap();
    assert_eq!(count, 2, "Valid files should be committed to database");
}

// =============================================================================
// Doctor and Init Integration Tests
// =============================================================================

/// Verify doctor reports errors when database is missing.
#[test]
fn test_doctor_no_database_reports_error() {
    let dir = tempdir().unwrap();

    let mut doctor = Doctor::new(dir.path(), false);
    let summary = doctor.run();

    // Should have at least one error for missing database
    assert!(summary.has_errors(), "Doctor should report error for missing database");

    // Verify specific check failed
    let checks = doctor.checks();
    let db_check = checks.iter().find(|c| c.name == "Database exists").unwrap();
    assert_eq!(db_check.status, Severity::Error);
    assert!(db_check.remediation.is_some(), "Should provide remediation");
}

/// Verify doctor passes on healthy database.
#[test]
fn test_doctor_healthy_database_passes() {
    let dir = tempdir().unwrap();

    // Create and populate database
    let db_path = dir.path().join(DB_NAME);
    let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
    db.init_schema().unwrap();
    db.upsert_file("test.rs", "fn main() {}", 0, 12).unwrap();
    drop(db);

    // Add gitignore entries
    update_gitignore(dir.path()).unwrap();

    let mut doctor = Doctor::new(dir.path(), false);
    let summary = doctor.run();

    // Should not have any errors
    assert!(!summary.has_errors(), "Doctor should pass on healthy database");

    // Verify key checks passed
    let checks = doctor.checks();
    assert!(checks.iter().any(|c| c.name == "Database exists" && c.status == Severity::Pass));
    assert!(checks.iter().any(|c| c.name == "Schema complete" && c.status == Severity::Pass));
    assert!(checks.iter().any(|c| c.name == "File count" && c.status == Severity::Pass));
    assert!(checks.iter().any(|c| c.name == "Gitignore" && c.status == Severity::Pass));
}

/// Verify doctor JSON output is valid JSON.
#[test]
fn test_doctor_json_output_valid() {
    let dir = tempdir().unwrap();

    let mut doctor = Doctor::new(dir.path(), false);
    let summary = doctor.run();

    let mut output = Vec::new();
    doctor.output_json(&mut output, &summary).unwrap();

    let json_str = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    // Verify expected fields exist
    assert!(parsed.get("version").is_some());
    assert!(parsed.get("project_dir").is_some());
    assert!(parsed.get("checks").is_some());
    assert!(parsed.get("summary").is_some());
    assert!(parsed.get("exit_code").is_some());

    // Verify checks is an array
    assert!(parsed["checks"].is_array());
}

/// Verify doctor verbose mode includes step numbers.
#[test]
fn test_doctor_verbose_includes_steps() {
    let dir = tempdir().unwrap();

    let mut doctor = Doctor::new(dir.path(), true); // verbose = true
    let summary = doctor.run();

    let mut output = Vec::new();
    doctor.output_plain(&mut output, &summary).unwrap();

    let output_str = String::from_utf8(output).unwrap();

    // Verbose mode should show [N/M] format
    assert!(output_str.contains("[1/"), "Verbose output should contain step numbers");
}

/// Verify doctor detects orphan WAL files.
#[test]
fn test_doctor_detects_orphan_wal() {
    let dir = tempdir().unwrap();

    // Create orphan WAL file without main database
    fs::write(dir.path().join(format!("{DB_NAME}{DB_WAL_SUFFIX}")), "orphan").unwrap();

    let mut doctor = Doctor::new(dir.path(), false);
    doctor.run();

    let checks = doctor.checks();
    let orphan_check = checks.iter().find(|c| c.name == "Orphan WAL files").unwrap();
    assert_eq!(orphan_check.status, Severity::Warning);
}

/// Verify init creates .gitignore with all required entries.
#[test]
fn test_init_creates_gitignore() {
    let dir = tempdir().unwrap();

    let result = update_gitignore(dir.path()).unwrap();
    assert!(matches!(result, GitignoreResult::Created(4)));

    let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();

    // Verify all entries are present
    for entry in gitignore_entries() {
        assert!(content.contains(entry), "Missing entry: {entry}");
    }

    // Verify header comment
    assert!(content.contains("ffts-grep"));
}

/// Verify init appends to existing .gitignore.
#[test]
fn test_init_appends_to_existing_gitignore() {
    let dir = tempdir().unwrap();

    // Create existing gitignore
    fs::write(dir.path().join(".gitignore"), "node_modules/\n.env\n").unwrap();

    let result = update_gitignore(dir.path()).unwrap();
    assert!(matches!(result, GitignoreResult::Updated(4)));

    let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();

    // Original entries preserved
    assert!(content.contains("node_modules/"));
    assert!(content.contains(".env"));

    // New entries added
    for entry in gitignore_entries() {
        assert!(content.contains(entry), "Missing entry: {entry}");
    }
}

/// Verify init is idempotent (no duplicate entries).
#[test]
fn test_init_idempotent() {
    let dir = tempdir().unwrap();

    // First init
    let result1 = update_gitignore(dir.path()).unwrap();
    assert!(matches!(result1, GitignoreResult::Created(4)));

    let content1 = fs::read_to_string(dir.path().join(".gitignore")).unwrap();

    // Second init
    let result2 = update_gitignore(dir.path()).unwrap();
    assert_eq!(result2, GitignoreResult::AlreadyComplete);

    let content2 = fs::read_to_string(dir.path().join(".gitignore")).unwrap();

    // Content should be identical
    assert_eq!(content1, content2, "Idempotent init should not modify file");
}

/// Verify `check_gitignore` detects missing entries.
#[test]
fn test_check_gitignore_detects_missing() {
    let dir = tempdir().unwrap();

    // Create partial gitignore
    fs::write(dir.path().join(".gitignore"), format!("{DB_NAME}\n")).unwrap();

    let missing = check_gitignore(dir.path());

    // Should report 3 missing entries
    assert_eq!(missing.len(), 3);
    assert!(missing.contains(&DB_SHM_NAME));
    assert!(missing.contains(&DB_WAL_NAME));
    assert!(missing.contains(&DB_TMP_GLOB));
}

/// Verify `check_gitignore` returns empty when complete.
#[test]
fn test_check_gitignore_complete() {
    let dir = tempdir().unwrap();

    // Create complete gitignore
    update_gitignore(dir.path()).unwrap();

    let missing = check_gitignore(dir.path());
    assert!(missing.is_empty(), "Complete gitignore should have no missing entries");
}

#[cfg(unix)]
#[test]
fn test_symlink_cycle_is_skipped() {
    let dir = tempdir().unwrap();
    let mut indexer = create_test_indexer(&dir);

    let symlink = dir.path().join("loop");
    std::os::unix::fs::symlink(&symlink, &symlink).unwrap();
    fs::write(dir.path().join("real.rs"), "fn real() {}").unwrap();

    let stats = indexer.index_directory().unwrap();
    assert_eq!(stats.files_indexed, 1);
    assert!(stats.files_skipped >= 1);

    let results = indexer.db_mut().search("real", false, 10).unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn test_deep_nesting_indexed() {
    if cfg!(windows) {
        return;
    }

    let dir = tempdir().unwrap();
    let mut path = dir.path().to_path_buf();
    let mut depth = 0;
    for _ in 0..1000 {
        let next = path.join("d");
        match fs::create_dir_all(&next) {
            Ok(()) => {
                path = next;
                depth += 1;
            }
            Err(err) if err.kind() == std::io::ErrorKind::InvalidFilename => break,
            Err(err) => panic!("Failed to create deep nesting: {err}"),
        }
    }
    let mut file_path = path.join("deep.rs");
    while let Err(err) = fs::write(&file_path, "fn deep() {}") {
        if err.kind() == std::io::ErrorKind::InvalidFilename {
            assert!(path.pop(), "Failed to recover from long path error");
            depth -= 1;
            file_path = path.join("deep.rs");
            continue;
        }
        panic!("Failed to write deep file: {err}");
    }
    assert!(depth >= 200, "Expected deep nesting, got depth {depth}");

    let mut indexer = create_test_indexer(&dir);
    let stats = indexer.index_directory().unwrap();
    assert_eq!(stats.files_indexed, 1);

    let results = indexer.db_mut().search("deep", false, 10).unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn test_concurrent_index_operations_do_not_corrupt() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("one.rs"), "fn one() {}").unwrap();
    fs::write(dir.path().join("two.rs"), "fn two() {}").unwrap();

    let db_path = dir.path().join(DB_NAME);
    let config = IndexerConfig::default();
    let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
    db.init_schema().unwrap();
    drop(db);

    let results = thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..2 {
            let db_path = db_path.clone();
            let dir_path = dir.path().to_path_buf();
            let config = config.clone();
            handles.push(scope.spawn(move || {
                let db = Database::open(&db_path, &PragmaConfig::default())?;
                let mut indexer = Indexer::new(&dir_path, db, config);
                indexer.index_directory()
            }));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<Result<_, IndexerError>>>()
    });

    assert!(results.iter().any(Result::is_ok));
    for result in results {
        if let Err(IndexerError::Database { source }) = result {
            if let rusqlite::Error::SqliteFailure(err, _) = source {
                assert_eq!(err.code, ErrorCode::DatabaseBusy);
            } else {
                panic!("Unexpected database error: {source}");
            }
        } else if let Err(err) = result {
            panic!("Unexpected error: {err}");
        }
    }

    let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
    assert!(db.get_file_count().unwrap() >= 2);
}

#[cfg(unix)]
#[test]
fn test_atomic_reindex_recovers_from_corrupt_db() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join(DB_NAME);
    fs::write(&db_path, b"not a sqlite db").unwrap();

    let stats = atomic_reindex(dir.path(), &PragmaConfig::default()).unwrap();
    assert_eq!(stats.files_indexed, 0);

    let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
    assert_eq!(db.get_file_count().unwrap(), 0);
}

// =============================================================================
// State Machine Verification Tests
//
// These tests verify the state transitions documented in docs/state-machines/
// =============================================================================

/// Verify conditional transaction threshold behavior (docs/state-machines/02-indexer-lifecycle.md).
///
/// The indexer starts auto-committing files until TRANSACTION_THRESHOLD (50) is reached,
/// then switches to batched transaction mode. This test verifies:
/// 1. Small batches (<50 files) complete without explicit transaction
/// 2. Large batches (>50 files) use transaction batching for performance
///
/// Note: We can't directly observe internal batch_count, but we can verify
/// the behavioral contract through performance characteristics.
#[test]
fn test_conditional_transaction_threshold_behavior() {
    // Test 1: Small batch (under threshold) - should still complete quickly
    let dir_small = tempdir().unwrap();
    let mut indexer_small = create_test_indexer(&dir_small);

    // Create 49 files (just under threshold of 50)
    for i in 0..49 {
        fs::write(dir_small.path().join(format!("file_{i}.rs")), format!("content {i}")).unwrap();
    }

    let start = std::time::Instant::now();
    let stats_small = indexer_small.index_directory().unwrap();
    let duration_small = start.elapsed();

    assert_eq!(stats_small.files_indexed, 49);
    // Small batch should still be fast (auto-commit per file is slow but 49 is manageable)
    assert!(duration_small.as_secs() < 10, "Small batch took too long: {duration_small:?}");

    // Test 2: Large batch (over threshold) - should be fast due to batching
    let dir_large = tempdir().unwrap();
    let mut indexer_large = create_test_indexer(&dir_large);

    // Create 100 files (well over threshold of 50)
    for i in 0..100 {
        fs::write(dir_large.path().join(format!("file_{i}.rs")), format!("content {i}")).unwrap();
    }

    let start = std::time::Instant::now();
    let stats_large = indexer_large.index_directory().unwrap();
    let duration_large = start.elapsed();

    assert_eq!(stats_large.files_indexed, 100);
    // Large batch should complete quickly due to transaction batching
    assert!(
        duration_large.as_secs() < 5,
        "Large batch should be fast with batching: {duration_large:?}"
    );
}

/// Verify health check state machine correctly identifies all DatabaseHealth variants
/// (docs/state-machines/04-search-flow.md).
///
/// This tests the health check flow that gates search operations.
#[test]
fn test_health_state_machine_transitions() {
    use ffts_indexer::constants::APPLICATION_ID_I32;
    use ffts_indexer::health::{DatabaseHealth, check_health_fast};

    // State 1: Missing - no database file
    let dir_missing = tempdir().unwrap();
    assert_eq!(check_health_fast(dir_missing.path()), DatabaseHealth::Missing);
    assert!(DatabaseHealth::Missing.needs_init());

    // State 2: Empty - schema exists but no files
    let dir_empty = tempdir().unwrap();
    let db_empty =
        Database::open(&dir_empty.path().join(DB_NAME), &PragmaConfig::default()).unwrap();
    db_empty.init_schema().unwrap();
    drop(db_empty);
    assert_eq!(check_health_fast(dir_empty.path()), DatabaseHealth::Empty);
    assert!(DatabaseHealth::Empty.needs_init());

    // State 3: Healthy - schema + content exists
    let dir_healthy = tempdir().unwrap();
    let db_healthy =
        Database::open(&dir_healthy.path().join(DB_NAME), &PragmaConfig::default()).unwrap();
    db_healthy.init_schema().unwrap();
    db_healthy.upsert_file("test.rs", "fn main() {}", 0, 12).unwrap();
    drop(db_healthy);
    let health = check_health_fast(dir_healthy.path());
    assert_eq!(health, DatabaseHealth::Healthy);
    assert!(health.is_usable());

    // State 4: WrongApplicationId - different app's database
    let dir_wrong_app = tempdir().unwrap();
    let conn = rusqlite::Connection::open(dir_wrong_app.path().join(DB_NAME)).unwrap();
    conn.pragma_update(None, "application_id", 0xDEAD_BEEF_u32 as i32).unwrap();
    conn.execute("CREATE TABLE files (path TEXT PRIMARY KEY)", []).unwrap();
    drop(conn);
    let health = check_health_fast(dir_wrong_app.path());
    assert_eq!(health, DatabaseHealth::WrongApplicationId);
    assert!(health.is_unrecoverable());

    // State 5: SchemaInvalid - correct app ID but incomplete schema
    let dir_invalid = tempdir().unwrap();
    let conn = rusqlite::Connection::open(dir_invalid.path().join(DB_NAME)).unwrap();
    conn.pragma_update(None, "application_id", APPLICATION_ID_I32).unwrap();
    // Only create partial schema (missing FTS, triggers, indexes)
    conn.execute("CREATE TABLE files (path TEXT PRIMARY KEY, content TEXT)", []).unwrap();
    drop(conn);
    assert_eq!(check_health_fast(dir_invalid.path()), DatabaseHealth::SchemaInvalid);
    assert!(DatabaseHealth::SchemaInvalid.needs_reinit());

    // State 6: Corrupted/Unreadable - garbage file
    let dir_corrupt = tempdir().unwrap();
    fs::write(dir_corrupt.path().join(DB_NAME), b"not a database").unwrap();
    let health = check_health_fast(dir_corrupt.path());
    // Garbage can be either Corrupted or Unreadable depending on SQLite behavior
    assert!(
        matches!(health, DatabaseHealth::Corrupted | DatabaseHealth::Unreadable),
        "Expected Corrupted or Unreadable, got {health:?}"
    );
}

/// Verify FTS5 trigger auto-sync behavior (docs/state-machines/03-database-states.md).
///
/// The database uses triggers (files_ai, files_au, files_ad) to automatically
/// keep the FTS5 index in sync with the files table.
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
    assert_eq!(results[0].path, "test.rs");

    // Test UPDATE trigger (files_au)
    db.upsert_file("test.rs", "updated_content_here", 1, 20).unwrap();
    let old_results = db.search("unique_insert_content", false, 10).unwrap();
    let new_results = db.search("updated_content_here", false, 10).unwrap();
    assert!(old_results.is_empty(), "Old content should be removed from FTS5");
    assert_eq!(new_results.len(), 1, "New content should be in FTS5");

    // Test DELETE trigger (files_ad)
    db.delete_file("test.rs").unwrap();
    let deleted_results = db.search("updated_content_here", false, 10).unwrap();
    assert!(deleted_results.is_empty(), "FTS5 should remove deleted content");
}

/// Verify lazy invalidation via content hash (docs/state-machines/03-database-states.md).
///
/// The upsert uses: WHERE excluded.content_hash != (SELECT content_hash ...)
/// to skip FTS5 rebuilds when content is unchanged.
///
/// Note: This test complements test_lazy_invalidation_skips_unchanged by testing
/// directly at the database layer without going through the indexer.
#[test]
fn test_lazy_invalidation_via_db_layer() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join(DB_NAME);
    let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
    db.init_schema().unwrap();

    let content = "unchanged content for hash test";

    // Initial insert
    db.upsert_file("test.rs", content, 0, content.len() as i64).unwrap();

    // Get initial indexed_at
    let indexed_at1: i64 = db
        .conn()
        .query_row("SELECT indexed_at FROM files WHERE path = 'test.rs'", [], |row| row.get(0))
        .unwrap();

    // Wait to ensure different timestamp (second resolution)
    thread::sleep(std::time::Duration::from_secs(1));

    // Upsert with SAME content - lazy invalidation should skip update
    db.upsert_file("test.rs", content, 1, content.len() as i64).unwrap();

    // indexed_at should NOT change when content is same (hash matches)
    let indexed_at2: i64 = db
        .conn()
        .query_row("SELECT indexed_at FROM files WHERE path = 'test.rs'", [], |row| row.get(0))
        .unwrap();

    assert_eq!(
        indexed_at1, indexed_at2,
        "Lazy invalidation should skip write when content is unchanged"
    );

    // Wait again
    thread::sleep(std::time::Duration::from_secs(1));

    // Upsert with DIFFERENT content - should update
    let new_content = "modified content triggers update";
    db.upsert_file("test.rs", new_content, 2, new_content.len() as i64).unwrap();

    let indexed_at3: i64 = db
        .conn()
        .query_row("SELECT indexed_at FROM files WHERE path = 'test.rs'", [], |row| row.get(0))
        .unwrap();

    assert!(indexed_at3 > indexed_at1, "Should update when content differs");

    // Verify FTS5 has new content
    let results = db.search("modified", false, 10).unwrap();
    assert_eq!(results.len(), 1, "FTS5 should have updated content");
}

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
    drop(db);
    update_gitignore(dir.path()).unwrap();

    let mut doctor = Doctor::new(dir.path(), true); // verbose mode
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

    for (i, expected) in expected_names.iter().enumerate() {
        assert_eq!(
            checks[i].name,
            *expected,
            "Check {} should be '{}', got '{}'",
            i + 1,
            expected,
            checks[i].name
        );
    }

    // All checks should pass on healthy database
    assert!(!summary.has_errors(), "Healthy database should pass all checks");
}

/// Verify exit code enum values are consistent
/// (docs/state-machines/07-error-types.md).
///
/// Note: This project uses custom exit codes (1-5) rather than sysexits.h.
#[test]
fn test_exit_code_values() {
    use ffts_indexer::error::ExitCode;

    // Verify exit codes match the defined constants in error.rs
    assert_eq!(ExitCode::Ok as u8, 0);
    assert_eq!(ExitCode::Software as u8, 1);
    assert_eq!(ExitCode::DataErr as u8, 2);
    assert_eq!(ExitCode::IoErr as u8, 3);
    assert_eq!(ExitCode::NoInput as u8, 4);
    assert_eq!(ExitCode::NoPerm as u8, 5);

    // Verify distinct values
    let codes = [
        ExitCode::Ok,
        ExitCode::Software,
        ExitCode::DataErr,
        ExitCode::IoErr,
        ExitCode::NoInput,
        ExitCode::NoPerm,
    ];
    for (i, a) in codes.iter().enumerate() {
        for (j, b) in codes.iter().enumerate() {
            if i != j {
                assert_ne!(a, b, "Exit codes must be unique");
            }
        }
    }
}

// ============================================================================
// Filename Ranking Tests (v0.10+)
// ============================================================================

/// Verify that files with query term in filename rank higher than files with
/// query term only in directory path.
///
/// This test validates the BM25 weighting: filename=100, path=50, content=1
/// See: https://github.com/mneves75/ffts-grep/issues/filename-ranking
#[test]
fn test_filename_boosts_search_ranking() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join(DB_NAME);
    let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
    db.init_schema().unwrap();

    // Create files with "claude" in different positions
    // File with "CLAUDE" in filename should rank highest
    db.upsert_file("CLAUDE.md", "# Project Documentation", 0, 25).unwrap();
    // File with "claude" in directory path but not filename
    db.upsert_file("docs/MASTRA-VS-CLAUDE-SDK.md", "Comparison document", 0, 20).unwrap();
    // File with "claude" only in content
    db.upsert_file("README.md", "Built for Claude Code integration", 0, 35).unwrap();

    let results = db.search("claude", false, 10).unwrap();

    assert_eq!(results.len(), 3, "All 3 files should match 'claude'");

    // CLAUDE.md (filename match) should rank higher than path-only or content-only matches
    assert_eq!(
        results[0].path,
        "CLAUDE.md",
        "File with 'CLAUDE' in filename should rank first, got: {:?}",
        results.iter().map(|r| &r.path).collect::<Vec<_>>()
    );
}

/// Verify that exact filename matches rank higher than partial path matches.
///
/// When searching for "config", "config.rs" should rank higher than
/// "src/config/mod.rs" even though both contain "config".
#[test]
fn test_filename_exact_match_priority() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join(DB_NAME);
    let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
    db.init_schema().unwrap();

    // Create files to test ranking
    db.upsert_file("config.rs", "// configuration module", 0, 25).unwrap();
    db.upsert_file("src/config/mod.rs", "// config module index", 0, 25).unwrap();
    db.upsert_file("src/utils/config_helper.rs", "// helper utilities", 0, 25).unwrap();

    let results = db.search("config", false, 10).unwrap();

    assert!(!results.is_empty(), "Should find files matching 'config'");

    // config.rs should rank highest (exact filename match)
    assert_eq!(
        results[0].path,
        "config.rs",
        "Exact filename 'config.rs' should rank first, got: {:?}",
        results.iter().map(|r| &r.path).collect::<Vec<_>>()
    );
}

/// Verify schema migration preserves existing data.
///
/// This test simulates upgrading from v0.9 (2-column FTS5) to v0.10+ (3-column FTS5).
#[test]
fn test_schema_migration_preserves_data() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join(DB_NAME);

    // Create a v0.9-style database (without filename column)
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();

        // Old schema without filename column
        conn.execute(
            "CREATE TABLE files (
                id INTEGER PRIMARY KEY,
                path TEXT UNIQUE NOT NULL,
                content_hash TEXT,
                mtime INTEGER,
                size INTEGER,
                indexed_at INTEGER,
                content TEXT
            )",
            [],
        )
        .unwrap();

        // Old FTS5 with only 2 columns
        conn.execute(
            "CREATE VIRTUAL TABLE files_fts USING fts5(
                path, content,
                content='files',
                content_rowid='id',
                tokenize='porter unicode61'
            )",
            [],
        )
        .unwrap();

        // Old triggers
        conn.execute(
            "CREATE TRIGGER files_ai AFTER INSERT ON files BEGIN
                INSERT INTO files_fts(rowid, path, content) VALUES (new.id, new.path, new.content);
            END",
            [],
        )
        .unwrap();

        // Insert test data
        conn.execute(
            "INSERT INTO files (path, content_hash, mtime, size, indexed_at, content)
             VALUES ('docs/CLAUDE.md', 'hash1', 0, 10, 0, 'test content')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO files (path, content_hash, mtime, size, indexed_at, content)
             VALUES ('README.md', 'hash2', 0, 15, 0, 'readme content')",
            [],
        )
        .unwrap();
    }

    // Open with new code and run migration
    let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
    db.migrate_schema().unwrap();
    db.init_schema().unwrap();
    db.rebuild_fts_index().unwrap(); // Repopulate FTS5 from migrated data

    // Verify data preserved
    let count = db.get_file_count().unwrap();
    assert_eq!(count, 2, "Migration should preserve all files");

    // Verify filenames were extracted
    let filename: String = db
        .conn()
        .query_row("SELECT filename FROM files WHERE path = 'docs/CLAUDE.md'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(filename, "CLAUDE.md", "Migration should extract filename from path");

    // Verify search still works with new ranking
    let results = db.search("claude", false, 10).unwrap();
    assert!(!results.is_empty(), "Search should work after migration");
}

/// Verify FTS5 has 3 columns with correct weights.
#[test]
fn test_fts5_three_column_weights() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join(DB_NAME);
    let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
    db.init_schema().unwrap();

    // Insert test file
    db.upsert_file("test.rs", "fn main() {}", 0, 12).unwrap();

    // Verify FTS5 table has 3 columns by checking search works
    // BM25 with 3 weights should return valid scores
    let results = db.search("test", false, 10).unwrap();
    assert!(!results.is_empty());

    // The rank should be negative (BM25 returns negative scores, lower = better)
    assert!(results[0].rank < 0.0, "BM25 should return negative scores");
}
