# Error Type Hierarchy

Documents the `IndexerError` enum and error propagation paths.

## IndexerError Variants

```mermaid
flowchart TD
    subgraph "IndexerError enum"
        A["Database { source: rusqlite::Error }"]
        B["Io { source: std::io::Error }"]
        C["InvalidUtf8 { path: String }"]
        D["FileTooLarge { size: u64, max: u64 }"]
        E["PathTraversal { path: String }"]
    end

    A --> F["DB open, query, execute failures"]
    B --> G["File read, fs operations, mtime extraction"]
    C --> H["Non-UTF8 file content"]
    D --> I["File exceeds max_file_size"]
    E --> J["Path escapes project root"]
```

## Error Sources by Component

### Database (db.rs)
```mermaid
flowchart LR
    subgraph "Database Operations"
        A[open] --> E1[Database]
        B[init_schema] --> E1
        C[upsert_file] --> E1
        D[search] --> E1
        E[delete_file] --> E1
        F[get_all_files] --> E1
        G[get_file_count] --> E1
        H[optimize] --> E1
    end

    E1[IndexerError::Database]
```

### Indexer (indexer.rs)
```mermaid
flowchart LR
    subgraph "Indexer Operations"
        A[process_entry] --> E1[Io]
        A --> E2[InvalidUtf8]
        A --> E3[PathTraversal]
        A --> E4[Database]

        B[read_file_content] --> E1
        B --> E2
        B --> E5[FileTooLarge]

        C[index_directory] --> E4
    end

    E1[IndexerError::Io]
    E2[IndexerError::InvalidUtf8]
    E3[IndexerError::PathTraversal]
    E4[IndexerError::Database]
    E5[IndexerError::FileTooLarge]
```

## Error Handling Strategy

### Indexer: Graceful Degradation
```mermaid
flowchart TD
    A[process_entry error] --> B{Error type?}
    B -->|Any| C["warn! log the error"]
    C --> D[Increment files_skipped]
    D --> E[Continue to next file]

    F[Transaction error] --> G[Return Err immediately]
    G --> H[Abort indexing]
```

### Main: Exit Code Mapping
```mermaid
flowchart TD
    A[Error occurred] --> B{Which operation?}

    B -->|DB open| C[IOERR 3]
    B -->|Schema init| D[SOFTWARE 1]
    B -->|Indexing| D
    B -->|Search query| E[DATAERR 2]
    B -->|Output format| D
    B -->|Project dir| F[NOINPUT 4]
    B -->|Permission| G[NOPERM 5]
```

**Note:** Uses custom exit codes (1-5), not BSD sysexits.h values.

## Error Display Formatting

```rust
impl std::fmt::Display for IndexerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database { source } =>
                write!(f, "Database error: {source}"),
            Self::Io { source } =>
                write!(f, "I/O error: {source}"),
            Self::InvalidUtf8 { path } =>
                write!(f, "Invalid UTF-8 in file: {path}"),
            Self::FileTooLarge { size, max } =>
                write!(f, "File too large: {size} bytes (max: {max})"),
            Self::PathTraversal { path } =>
                write!(f, "Path traversal attempt: {path}"),
        }
    }
}
```

## Result Type Alias

```rust
pub type Result<T> = std::result::Result<T, IndexerError>;
```

## Error Recovery Patterns

| Error | Recovery | User Action |
|-------|----------|-------------|
| Database::Io (open fail) | None | Check path, permissions |
| Database::Corrupted | backup_and_reinit | Automatic or `init --force` |
| InvalidUtf8 | Skip file | None (binary file) |
| FileTooLarge | Skip file | Increase max_file_size or ignore |
| PathTraversal | Skip file | Check symlinks |
| Transaction fail | Abort operation | Retry or check disk space |
