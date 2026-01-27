# Chapter 4: error.rs - Error Handling

> "Errors should never pass silently. Unless explicitly silenced." — The Zen of Python

## 4.1 What Does This File Do? (In Simple Terms)

The `error.rs` file defines all the things that can go wrong. In Rust, we use an `enum` to represent different error types, and the `thiserror` crate makes it easy to define errors with nice error messages.

### The Flight Status Board Analogy

Think of an airport's flight status board:

| Flight Status | What It Tells You |
|---------------|-------------------|
| On Time | Everything is fine |
| Delayed | Something went wrong, but we know what |
| Cancelled | Big problem, can't fly |
| Diverted | Going somewhere else |

Error types are like flight statuses—they tell you what went wrong and how serious it is.

---

## 4.2 The Actual Source Code: Error Enum

Here is the actual error enum from `error.rs:1-69`:

```rust
use thiserror::Error;

/// Centralized error types for the FTS5 indexer.
///
/// All errors are explicit enum variants (no Box<dyn Error>) for
/// maximum performance and actionable error messages.
#[derive(Error, Debug)]
pub enum IndexerError {
    /// `SQLite` database operation failed
    #[error("database error: {source}")]
    Database {
        #[from]
        source: rusqlite::Error,
    },

    /// File system I/O operation failed
    #[error("IO error: {source}")]
    Io {
        #[from]
        source: std::io::Error,
    },

    /// Path traversal attempt detected (symlink escape)
    #[error("path '{path}' is outside project root")]
    PathTraversal { path: String },

    /// File exceeds maximum allowed size
    #[error("file too large: {size} bytes (max: {max})")]
    FileTooLarge { size: u64, max: u64 },

    /// File contains invalid UTF-8 encoding
    #[error("invalid UTF-8 in file: {path}")]
    InvalidUtf8 { path: String },

    /// Gitignore parsing error
    #[error("gitignore parse error in '{path}': {source}")]
    GitignoreParse {
        path: String,
        #[source]
        source: ignore::Error,
    },

    /// Invalid CLI configuration value
    #[error("invalid {field}: {value} ({reason})")]
    ConfigInvalid { field: String, value: String, reason: String },

    /// FTS5 index integrity check failed
    #[error("index corrupted, run --reindex")]
    IndexCorrupted,

    /// Database belongs to a different application (never auto-delete)
    #[error("database belongs to different application (app_id: {app_id:#x})")]
    ForeignDatabase { app_id: u32 },

    /// Query parsing failed
    #[error("invalid query: {0}")]
    QueryParse(String),

    /// Empty search query
    #[error("empty query")]
    EmptyQuery,

    /// JSON serialization error
    #[error("JSON error: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
}
```

### Understanding the Derive

```rust
#[derive(Debug, thiserror::Error)]
```

- `Debug` — Allows printing the error for debugging
- `thiserror::Error` — Generates the `Error` trait implementation automatically

### Understanding #[error] Attributes

The `#[error(...)]` attribute defines the user-facing error message:

| Pattern | Example | Output |
|---------|---------|--------|
| `"{0}"` | `QueryParse(String)` | Uses Display trait of String |
| `"{path}"` | `PathTraversal { path }` | Field name |
| `"{source}"` | `Database { source }` | Inner error (transparent) |
| Literal text | `"Empty search query"` | Static message |

---

## 4.3 Exit Codes: System Integration

Here is the actual exit code implementation from `error.rs:80-105`:

```rust
/// Exit codes for the CLI application.
///
/// Based on BSD sysexits.h conventions for meaningful exit statuses.
/// Use `ExitCode::into()` to convert to `std::process::ExitCode`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    /// Successful execution
    Ok = 0,
    /// General software error (internal error, unexpected state)
    Software = 1,
    /// Invalid input data (malformed query, corrupted database)
    DataErr = 2,
    /// I/O error (file not found, permission denied on files)
    IoErr = 3,
    /// No input provided (missing required arguments)
    NoInput = 4,
    /// Permission denied (access control failure)
    NoPerm = 5,
}

impl From<ExitCode> for std::process::ExitCode {
    fn from(code: ExitCode) -> Self {
        Self::from(code as u8)
    }
}
```

### Why Exit Codes Matter

Exit codes allow the program to integrate with shell scripts and other tools:

```bash
if ffts-grep search "main"; then
    echo "Found results"
else
    exit_code=$?
    case $exit_code in
        2) echo "Data error" ;;
        3) echo "File not found" ;;
        5) echo "Permission denied" ;;
    esac
fi
```

### BSD sysexits.h Convention

These exit codes follow the BSD `sysexits.h` convention:

| Code | Name | Meaning |
|------|------|---------|
| 0 | `EX_OK` | Success |
| 1 | `EX_SOFTWARE` | Internal software error |
| 2 | `EX_DATAERR` | Data format error |
| 3 | `EX_IOERR` | I/O error |
| 4 | `EX_NOINPUT` | No input |
| 5 | `EX_NOPERM` | Permission denied |

---

## 4.4 Error Conversion: The ? Operator

Here is the actual From implementation from `error.rs:71-75`:

```rust
impl From<ignore::Error> for IndexerError {
    fn from(source: ignore::Error) -> Self {
        Self::GitignoreParse { path: String::new(), source }
    }
}
```

**Note**: `Database` and `Io` errors use `#[from]` attribute in the enum definition, which automatically implements `From`. Only `ignore::Error` needs a manual implementation because it has an additional `path` field.

The `From` trait enables the `?` operator for automatic conversion:

```rust
// Without From: Manual conversion
fn read_file() -> Result<String, IndexerError> {
    std::fs::read_to_string("file.txt")
        .map_err(IndexerError::from)
}

// With From: Automatic conversion
fn read_file() -> Result<String, IndexerError> {
    std::fs::read_to_string("file.txt")?  // Auto-converts!
}
```

---

## 4.5 The Result Type

Here is the actual type alias from `error.rs:77-78`:

```rust
/// Result type alias for indexer operations.
pub type Result<T> = std::result::Result<T, IndexerError>;
```

This is a type alias—shorthand for `Result<T, IndexerError>`:

```rust
// These are equivalent:
fn foo() -> Result<String> { ... }
fn foo() -> Result<String, IndexerError> { ... }
```

### Why Use Type Aliases?

1. **Shorter code** — Less typing
2. **Consistency** — All functions use the same error type
3. **Easy changes** — Change the error type in one place

---

## 4.6 Designing Good Error Messages

### Principles from thiserror

1. **Be specific** — Don't just say "error", say what kind
2. **Be helpful** — Include relevant information
3. **Be actionable** — Tell users what to do

### Good vs. Bad Examples

| Bad | Good |
|-----|------|
| "File error" | "Cannot open file 'config.txt': Permission denied" |
| "Invalid input" | "Cache size must be between -1000000 and -1000 (got: 500)" |
| "Query failed" | "Empty search query - please provide a search term" |

---

## 4.7 Error Handling in Practice

See how errors are used in `main.rs:57-189`:

```rust
fn main() -> Result<()> {
    let args = Cli::parse();

    tracing_subscribers::fmt::init();

    let project_dir = args.project_dir()?;

    match args.command {
        Commands::Index { reindex } => {
            run_indexing(&project_dir, reindex, args.quiet)?;
        }
        Commands::Search { query, paths_only, format, max_results } => {
            run_search(&project_dir, query, paths_only, format, max_results, args.quiet)?;
        }
        Commands::Doctor => {
            run_doctor(&project_dir, args.format, args.verbose)?;
        }
        Commands::Init { gitignore_only, force } => {
            run_init(&project_dir, gitignore_only, force, args.quiet)?;
        }
    }

    Ok(())
}
```

The `?` operator propagates errors up the call stack.

---

## 4.8 The Complete error.rs File

```rust
use thiserror::Error;

/// Centralized error types for the FTS5 indexer.
///
/// All errors are explicit enum variants (no Box<dyn Error>) for
/// maximum performance and actionable error messages.
#[derive(Error, Debug)]
pub enum IndexerError {
    /// `SQLite` database operation failed
    #[error("database error: {source}")]
    Database {
        #[from]
        source: rusqlite::Error,
    },

    /// File system I/O operation failed
    #[error("IO error: {source}")]
    Io {
        #[from]
        source: std::io::Error,
    },

    /// Path traversal attempt detected (symlink escape)
    #[error("path '{path}' is outside project root")]
    PathTraversal { path: String },

    /// File exceeds maximum allowed size
    #[error("file too large: {size} bytes (max: {max})")]
    FileTooLarge { size: u64, max: u64 },

    /// File contains invalid UTF-8 encoding
    #[error("invalid UTF-8 in file: {path}")]
    InvalidUtf8 { path: String },

    /// Gitignore parsing error
    #[error("gitignore parse error in '{path}': {source}")]
    GitignoreParse {
        path: String,
        #[source]
        source: ignore::Error,
    },

    /// Invalid CLI configuration value
    #[error("invalid {field}: {value} ({reason})")]
    ConfigInvalid { field: String, value: String, reason: String },

    /// FTS5 index integrity check failed
    #[error("index corrupted, run --reindex")]
    IndexCorrupted,

    /// Database belongs to a different application (never auto-delete)
    #[error("database belongs to different application (app_id: {app_id:#x})")]
    ForeignDatabase { app_id: u32 },

    /// Query parsing failed
    #[error("invalid query: {0}")]
    QueryParse(String),

    /// Empty search query
    #[error("empty query")]
    EmptyQuery,

    /// JSON serialization error
    #[error("JSON error: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
}

impl From<ignore::Error> for IndexerError {
    fn from(source: ignore::Error) -> Self {
        Self::GitignoreParse { path: String::new(), source }
    }
}

/// Result type alias for indexer operations.
pub type Result<T> = std::result::Result<T, IndexerError>;

/// Exit codes for the CLI application.
///
/// Based on BSD sysexits.h conventions for meaningful exit statuses.
/// Use `ExitCode::into()` to convert to `std::process::ExitCode`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    /// Successful execution
    Ok = 0,
    /// General software error (internal error, unexpected state)
    Software = 1,
    /// Invalid input data (malformed query, corrupted database)
    DataErr = 2,
    /// I/O error (file not found, permission denied on files)
    IoErr = 3,
    /// No input provided (missing required arguments)
    NoInput = 4,
    /// Permission denied (access control failure)
    NoPerm = 5,
}

impl From<ExitCode> for std::process::ExitCode {
    fn from(code: ExitCode) -> Self {
        Self::from(code as u8)
    }
}
```

---

## 4.9 Chapter Summary

| Concept | What We Learned |
|---------|-----------------|
| Error enum | All possible failures, one type |
| thiserror | Derive macros for error types |
| Exit codes | Integration with shell/scripts |
| From trait | Automatic error conversion |
| ? operator | Error propagation |
| Result type | Type alias for ergonomics |

---

## Exercises

### Exercise 4.1: Analyze Error Types

List all error variants and explain what situation causes each:

```rust
Database { source }
Io { source }
PathTraversal { path }
```

**Deliverable:** For each error, describe a scenario that would trigger it.

### Exercise 4.2: Create an Error Enum

Create an error enum for a file processor that handles:
- File not found
- Permission denied
- Invalid format
- File too large
- Encoding error

**Deliverable:** Write the enum with thiserror, then use it in a function.

### Exercise 4.3: Exit Codes

Why do we use different exit codes (0, 1, 2, 3, 4, 5) instead of just 0 and 1?

**Deliverable:** Write a shell script that uses exit codes to handle different errors.

### Exercise 4.4: Error Message Design

Rewrite these bad error messages to be better:

| Bad | Your Version |
|-----|--------------|
| "Error opening file" | |
| "Invalid config" | |
| "Search failed" | |

**Deliverable:** Explain what makes your messages better.

---

**Next Chapter**: [Chapter 5: cli.rs - Argument Parsing](05-cli_rs.md)
