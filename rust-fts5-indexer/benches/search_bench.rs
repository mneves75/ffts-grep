use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::fs;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};
use tempfile::tempdir;

use ffts_indexer::DB_NAME;
use ffts_indexer::db::{Database, PragmaConfig};
use ffts_indexer::indexer::{Indexer, IndexerConfig};

/// Lightweight RSS sampler for the current process.
struct RssSampler {
    pid: Pid,
    sys: System,
}

impl RssSampler {
    fn new() -> Self {
        let pid = Pid::from_u32(std::process::id());
        let sys = System::new_with_specifics(
            RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing().with_memory()),
        );
        Self { pid, sys }
    }

    /// Sample current RSS in MB (post-operation snapshot, not peak).
    fn sample_mb(&mut self) -> f64 {
        self.sys.refresh_processes(ProcessesToUpdate::Some(&[self.pid]), true);
        self.sys.process(self.pid).map_or(0.0, |p| rss_mb(p.memory()))
    }
}

#[allow(clippy::cast_precision_loss)]
fn rss_mb(bytes: u64) -> f64 {
    bytes as f64 / 1_000_000.0
}

/// Create a test database with N files for benchmarking.
fn create_benchmark_db(num_files: usize) -> (tempfile::TempDir, Database) {
    let dir = tempdir().unwrap();

    // Create test files
    for i in 0..num_files {
        let content = format!(
            "// File {i}
            pub fn function_{i}() {{
                let data = vec![1, 2, 3, 4, 5];
                for item in data.iter() {{
                    println!(\"Item: {{}}\", item);
                }}
            }}

            pub struct Struct_{i} {{
                field: i32,
            }}

            impl Struct_{i} {{
                pub fn new() -> Self {{
                    Self {{ field: 0 }}
                }}
            }}
            "
        );
        let path = dir.path().join(format!("file_{i}.rs"));
        fs::write(&path, content).unwrap();
    }

    // Create subdirectory with more files
    let subdir = dir.path().join("src");
    fs::create_dir_all(&subdir).unwrap();
    for i in 0..num_files / 2 {
        let content = format!("// Subdir file {i}\npub fn helper_{i}() {{}}\n");
        let path = subdir.join(format!("helper_{i}.rs"));
        fs::write(&path, content).unwrap();
    }

    let db_path = dir.path().join(DB_NAME);
    let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
    db.init_schema().unwrap();

    (dir, db)
}

fn index_files(dir: &tempfile::TempDir, db: Database) -> Indexer {
    let config = IndexerConfig::default();
    let mut indexer = Indexer::new(dir.path(), db, config);
    indexer.index_directory().expect("Failed to index benchmark files");
    indexer
}

fn benchmark_search(c: &mut Criterion) {
    let (dir, db) = create_benchmark_db(500);
    let mut indexer = index_files(&dir, db);
    let db = indexer.db_mut();

    // Verify search works before benchmarking (correctness check)
    let test_results = db.search("function", false, 200).unwrap();
    assert!(
        !test_results.is_empty(),
        "Search returned no results for 'function' - search may be broken!"
    );
    assert!(
        test_results.len() >= 100,
        "Expected >=100 results for 'function' query (found {}), search may not be indexing properly",
        test_results.len()
    );

    // Verify path-only search also works
    let path_results = db.search("file", true, 50).unwrap();
    assert!(
        !path_results.is_empty(),
        "Path-only search returned no results - path search may be broken!"
    );

    let mut group = c.benchmark_group("search");

    for query in ["main", "function", "println", "Struct", "new", "helper"] {
        group.bench_with_input(BenchmarkId::new("search", query), query, |b, q| {
            b.iter(|| {
                let _ = db.search(q, false, 50);
            });
        });
    }

    group.finish();
}

fn benchmark_index(c: &mut Criterion) {
    let mut group = c.benchmark_group("index");

    for num_files in [100, 500, 1000] {
        group.bench_with_input(BenchmarkId::new("index_files", num_files), &num_files, |b, n| {
            b.iter(|| {
                let (dir, db) = create_benchmark_db(*n);
                let config = IndexerConfig::default();
                let mut indexer = Indexer::new(dir.path(), db, config);
                let _ = indexer.index_directory();
            });
        });
    }

    group.finish();
}

fn benchmark_hash(c: &mut Criterion) {
    let content = b"fn main() { println!(\"Hello, world!\"); }";

    c.bench_function("wyhash_100bytes", |b| {
        b.iter(|| {
            let _ = ffts_indexer::db::wyhash(content);
        });
    });
}

/// Cold start benchmark: validates README claims for 10K files.
///
/// Measures cold query time (opening fresh connection + first query).
/// Target: < 500ms cold, < 10ms warm (typical 100-300ms cold, 1-5ms warm).
fn benchmark_cold_start_10k(c: &mut Criterion) {
    // Create and index 10K files (setup phase, not benchmarked)
    let (dir, db) = create_benchmark_db(10000);
    let _indexer = index_files(&dir, db);

    let db_path = dir.path().join(DB_NAME);

    let mut group = c.benchmark_group("cold_start_10k");
    group.sample_size(50); // Reduce samples for expensive benchmark

    // Cold start: fresh connection + first query
    group.bench_function("cold_query", |b| {
        b.iter(|| {
            // Simulate cold start: open new connection
            let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
            db.init_schema().unwrap();
            // First query on fresh connection
            let _ = db.search("function", false, 50);
        });
    });

    // Warm query: reuse connection
    let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
    db.init_schema().unwrap();
    // Warm up the cache
    let _ = db.search("function", false, 50);

    group.bench_function("warm_query", |b| {
        b.iter(|| {
            let _ = db.search("function", false, 50);
        });
    });

    group.finish();
}

/// Memory benchmark: reports post-operation RSS samples at various scales.
///
/// This is informational only; RSS delta sampling in a long-lived process is not
/// a reliable measure of peak memory. For README claim validation, use the
/// `memory_validation` tests or `/usr/bin/time -l` in a fresh process.
fn benchmark_memory(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory");
    group.sample_size(10); // Memory measurements are stable, fewer samples needed

    for num_files in [1000, 5000, 10000] {
        group.bench_with_input(BenchmarkId::new("index_rss", num_files), &num_files, |b, n| {
            let mut rss = RssSampler::new();
            b.iter_custom(|iters| {
                let mut total_duration = std::time::Duration::ZERO;
                let mut max_rss = 0.0f64;

                for _ in 0..iters {
                    let start = std::time::Instant::now();

                    let (dir, db) = create_benchmark_db(*n);
                    let config = IndexerConfig::default();
                    let mut indexer = Indexer::new(dir.path(), db, config);
                    let _ = indexer.index_directory();

                    total_duration += start.elapsed();
                    let rss_after = rss.sample_mb();
                    max_rss = max_rss.max(rss_after);
                }

                eprintln!("  [{n} files] RSS sample after index: {max_rss:.1} MB (not peak)");
                total_duration
            });
        });
    }

    // Search-only memory (no indexing, just query)
    group.bench_function("search_rss", |b| {
        let (dir, db) = create_benchmark_db(1000);
        let _indexer = index_files(&dir, db);
        let mut rss = RssSampler::new();
        let rss_baseline = rss.sample_mb();

        b.iter_custom(|iters| {
            let mut total_duration = std::time::Duration::ZERO;
            let mut max_rss = 0.0f64;
            for _ in 0..iters {
                let start = std::time::Instant::now();
                let db_path = dir.path().join(DB_NAME);
                let db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
                let _ = db.search("function", false, 50);
                total_duration += start.elapsed();
                let rss_current = rss.sample_mb();
                max_rss = max_rss.max(rss_current);
            }

            eprintln!("  Search RSS sample: {max_rss:.1} MB (baseline: {rss_baseline:.1} MB)");
            total_duration
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    benchmark_search,
    benchmark_index,
    benchmark_hash,
    benchmark_cold_start_10k,
    benchmark_memory
);
criterion_main!(benches);
