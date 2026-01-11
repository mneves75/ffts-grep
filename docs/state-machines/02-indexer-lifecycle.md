# Indexer Lifecycle State Machine

Shows the complete indexing process including **conditional transaction strategy**.

## Key Design: Conditional Transactions

```
THRESHOLD = 50 files   → Start transaction after this many files
BATCH_SIZE = 500 files → Commit and restart transaction at this count
```

**Why?** Transaction overhead dominates for small operations. Below 50 files, autocommit is faster.

## Main Indexer Flow

```mermaid
stateDiagram-v2
    [*] --> New: Indexer::new(root, db, config)

    New --> Initialize: index_directory()

    state Initialize {
        [*] --> SetupWalk
        SetupWalk --> InitStats: WalkBuilder::new().standard_filters(true).same_file_system(true)
        InitStats --> SetCounters: stats = IndexStats::default()
        SetCounters --> [*]: batch_count=0, transaction_started=false
    }

    Initialize --> WalkLoop

    state WalkLoop {
        [*] --> GetNextEntry
        GetNextEntry --> ProcessEntry: Ok(entry)
        GetNextEntry --> LogWalkError: Err(e)
        LogWalkError --> GetNextEntry: warn and continue

        ProcessEntry --> HandleResult
        HandleResult --> IncrementBatch: Ok(true) needs_commit
        HandleResult --> GetNextEntry: Ok(false) skipped
        HandleResult --> LogFileError: Err(e)
        LogFileError --> IncrementSkipped
        IncrementSkipped --> GetNextEntry

        IncrementBatch --> CheckThreshold
        CheckThreshold --> StartTransaction: batch_count == 50 && !transaction_started
        CheckThreshold --> CheckBatchSize: already in transaction OR below threshold

        StartTransaction --> BeginImmediate: BEGIN IMMEDIATE
        BeginImmediate --> SetStarted: transaction_started = true
        SetStarted --> CheckBatchSize

        CheckBatchSize --> CommitAndRestart: transaction_started && batch_count >= 500
        CheckBatchSize --> GetNextEntry: continue walking

        CommitAndRestart --> Commit: COMMIT
        Commit --> BeginAgain: BEGIN IMMEDIATE
        BeginAgain --> ResetToThreshold: batch_count = 50 (NOT 0!)
        ResetToThreshold --> GetNextEntry

        GetNextEntry --> WalkComplete: no more entries
    }

    WalkComplete --> Finalize

    state Finalize {
        [*] --> CheckTransaction
        CheckTransaction --> FinalCommit: transaction_started
        CheckTransaction --> RunAnalyze: !transaction_started
        FinalCommit --> RunAnalyze: COMMIT
        RunAnalyze --> RunOptimize: ANALYZE (non-fatal)
        RunOptimize --> RunFtsOptimize: PRAGMA optimize (non-fatal)
        RunFtsOptimize --> [*]: optimize_fts() (non-fatal)
    }

    Finalize --> ReturnStats: Ok(IndexStats)
```

## CRITICAL: batch_count Reset Logic

```rust
// CORRECT: Reset to THRESHOLD, not 0
batch_count = TRANSACTION_THRESHOLD; // 50

// WHY: We're still in a transaction, so we don't want to
// immediately trigger another commit. We set it to 50 so
// the next 450 files will trigger the next commit.
```

## File Processing State Machine

```mermaid
flowchart TD
    A[process_entry] --> B{is_database_file?}
    B -->|yes .db/.sqlite/-shm/-wal/.db.tmp| C[return Ok false]
    B -->|no| D{is_directory?}

    D -->|yes| C
    D -->|no| E{is_symlink?}

    E -->|yes| F{config.follow_symlinks?}
    F -->|false| G[files_skipped++]
    G --> C
    F -->|true| H[fs::canonicalize]
    H -->|Err| G
    H -->|Ok resolved| I{is_within_root?}
    I -->|no| J[warn + files_skipped++]
    J --> C
    I -->|yes| K

    E -->|no| K{metadata.len > max_file_size?}
    K -->|yes 1MB default| G
    K -->|no| L[read_file_content]

    L -->|InvalidUtf8| M[warn + files_skipped++]
    M --> C
    L -->|Ok content| N[strip_prefix to get rel_path]
    N -->|Err| O[return Err PathTraversal]
    N -->|Ok| P[calculate mtime as i64]
    P --> Q[db.upsert_file]
    Q -->|Err| R[return Err]
    Q -->|Ok| S[files_indexed++, bytes_indexed += size]
    S --> T[return Ok true]
```

## Atomic Reindex Flow

```mermaid
sequenceDiagram
    participant CLI as run_indexing(force_reindex=true)
    participant TMP as .ffts-index.db.tmp
    participant DB as .ffts-index.db
    participant FS as Filesystem

    CLI->>FS: Remove existing tmp file (ignore error)
    CLI->>TMP: Database::open(tmp_path)
    CLI->>TMP: init_schema()
    CLI->>TMP: Indexer::new() + index_directory()
    CLI->>TMP: optimize_fts()
    CLI->>TMP: PRAGMA wal_checkpoint(TRUNCATE)
    Note over TMP: Log checkpoint stats (busy, log, checkpointed)

    CLI->>FS: atomic_replace(tmp -> db)
    Note over FS: Unix: fs::rename<br/>Windows: MoveFileExW(REPLACE|WRITE_THROUGH)

    CLI->>FS: Remove .ffts-index.db-shm
    CLI->>FS: Remove .ffts-index.db-wal
    Note over FS: Cleanup AFTER rename (not before!)
```

## Error Handling Strategy

| Error Type | Handling | Continues? |
|------------|----------|------------|
| Walk error | `warn!` and continue | Yes |
| File process error | `warn!`, increment `files_skipped` | Yes |
| Transaction error | Return `Err(IndexerError::Database)` | No |
| ANALYZE/optimize error | `.ok()` ignore | Yes |

## Configuration Defaults

| Parameter | Default | Purpose |
|-----------|---------|---------|
| `max_file_size` | 1MB | Skip files larger than this |
| `batch_size` | 500 | Files per transaction commit |
| `follow_symlinks` | true | Resolve and follow symlinks |
| `TRANSACTION_THRESHOLD` | 50 | Files before starting transaction |
