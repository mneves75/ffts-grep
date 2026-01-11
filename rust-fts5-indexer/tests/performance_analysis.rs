//! Performance analysis test to understand transaction batching overhead.
//!
//! This test measures indexing performance with and without explicit transactions
//! to quantify the overhead and determine optimal batch size.
//!
//! All numeric casts are for performance metrics display - precision loss is acceptable.

#![allow(clippy::cast_precision_loss)]

use ffts_indexer::DB_NAME;
use ffts_indexer::db::{Database, PragmaConfig};
use ffts_indexer::indexer::{Indexer, IndexerConfig};
use std::fs;
use std::time::Instant;
use tempfile::tempdir;

#[test]
fn analyze_transaction_overhead() {
    println!("\n=== Transaction Batching Performance Analysis ===\n");

    // Test scenarios: small, medium, large repos
    let scenarios = vec![
        ("Small (10 files)", 10),
        ("Medium (100 files)", 100),
        ("Large (1000 files)", 1000),
        ("Very Large (5000 files)", 5000),
    ];

    for (name, num_files) in scenarios {
        println!("Scenario: {name}");

        // Create test files
        let dir = tempdir().unwrap();
        for i in 0..num_files {
            let content = format!("// File {i}\npub fn func_{i}() {{}}\n");
            fs::write(dir.path().join(format!("file_{i}.rs")), content).unwrap();
        }

        let db_path = dir.path().join(DB_NAME);
        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();

        // Measure indexing time
        let mut indexer = Indexer::new(dir.path(), db, IndexerConfig::default());

        let start = Instant::now();
        let stats = indexer.index_directory().unwrap();
        let duration = start.elapsed();

        println!(
            "  Files indexed: {}, Time: {:?} ({:.2} files/sec)",
            stats.files_indexed,
            duration,
            stats.files_indexed as f64 / duration.as_secs_f64()
        );
        println!(
            "  Avg per file: {:.2}ms\n",
            duration.as_millis() as f64 / stats.files_indexed as f64
        );
    }
}

#[test]
fn analyze_batch_size_impact() {
    println!("\n=== Batch Size Impact Analysis ===\n");

    let num_files = 1000;
    let batch_sizes = vec![100, 250, 500, 1000, 2000];

    for batch_size in batch_sizes {
        println!("Batch size: {batch_size}");

        // Create test files
        let dir = tempdir().unwrap();
        for i in 0..num_files {
            let content = format!("// File {i}\npub fn func_{i}() {{}}\n");
            fs::write(dir.path().join(format!("file_{i}.rs")), content).unwrap();
        }

        let db_path = dir.path().join(DB_NAME);
        let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();

        let config = IndexerConfig { batch_size, ..Default::default() };

        let mut indexer = Indexer::new(dir.path(), db, config);

        let start = Instant::now();
        let stats = indexer.index_directory().unwrap();
        let duration = start.elapsed();

        println!(
            "  Time: {:?}, Throughput: {:.2} files/sec\n",
            duration,
            stats.files_indexed as f64 / duration.as_secs_f64()
        );
    }
}
