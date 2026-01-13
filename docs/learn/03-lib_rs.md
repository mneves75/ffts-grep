# Chapter 3: lib.rs - The Library Root

> "A good API is like a well-designed sign. It tells you exactly what you need to know." — Unknown

## 3.1 What Does This File Do? (In Simple Terms)

Think of `lib.rs` as the **table of contents** and **public face** of the library. When someone wants to use your library, this is the first file they look at. It answers:

- "What modules are available?"
- "What types can I use?"
- "What constants do I need?"

### The Restaurant Menu Analogy

Imagine a restaurant:
- **lib.rs** = The menu listing all available dishes
- **Other .rs files** = The kitchen where dishes are prepared
- **User of the library** = The customer reading the menu

The customer doesn't need to know how the chef cooks—they just need to know what's available and how to order.

---

## 3.2 The Actual Source Code

Here is the complete actual source code from `lib.rs`:

```rust
//! ffts-indexer - Fast full-text search file indexer using `SQLite` FTS5
//!
//! This library provides the core functionality for indexing files
//! in a directory and searching them using `SQLite` FTS5.
//!
//! # Example
//!
//! ```rust
//! use ffts_indexer::{Database, Indexer, IndexerConfig, PragmaConfig, DB_NAME};
//! use std::path::Path;
//! use std::time::{SystemTime, UNIX_EPOCH};
//!
//! let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
//! let root = std::env::temp_dir().join(format!("ffts-indexer-doctest-{unique}"));
//! std::fs::create_dir_all(&root)?;
//! std::fs::write(root.join("main.rs"), "fn main() {}")?;
//!
//! let db_path = root.join(DB_NAME);
//! let db = Database::open(&db_path, &PragmaConfig::default())?;
//! db.init_schema()?;
//!
//! let config = IndexerConfig::default();
//! let mut indexer = Indexer::new(Path::new(&root), db, config);
//! indexer.index_directory()?;
//!
//! drop(indexer);
//! let _ = std::fs::remove_dir_all(&root);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

/// Default database filename.
pub const DB_NAME: &str = ".ffts-index.db";

/// WAL mode shm file suffix.
pub const DB_SHM_SUFFIX: &str = "-shm";

/// WAL mode wal file suffix.
pub const DB_WAL_SUFFIX: &str = "-wal";

/// Temporary file suffix during reindex.
pub const DB_TMP_SUFFIX: &str = ".tmp";

/// WAL mode shm file name.
pub const DB_SHM_NAME: &str = ".ffts-index.db-shm";

/// WAL mode wal file name.
pub const DB_WAL_NAME: &str = ".ffts-index.db-wal";

/// Temporary file name during reindex.
pub const DB_TMP_NAME: &str = ".ffts-index.db.tmp";

/// Temporary file glob for gitignore entries (covers suffix variants).
pub const DB_TMP_GLOB: &str = ".ffts-index.db.tmp*";

pub mod cli;
pub mod db;
pub mod doctor;
pub mod error;
pub mod health;
pub mod indexer;
pub mod init;
pub mod search;

pub use cli::OutputFormat;
pub use db::{Database, PragmaConfig, SchemaCheck, SearchResult};
pub use doctor::{
    CheckResult, Doctor, DoctorOutput, DoctorSummary, EXPECTED_APPLICATION_ID, Severity,
};
pub use error::{ExitCode, IndexerError, Result};
pub use health::{
    DatabaseHealth, DetectionMethod, ProjectRoot, auto_init, auto_init_with_config,
    backup_and_reinit, backup_and_reinit_with_config, check_health_fast, find_project_root,
};
pub use indexer::{IndexStats, Indexer, IndexerConfig};
pub use init::{GitignoreResult, InitResult, check_gitignore, gitignore_entries, update_gitignore};
pub use search::{SearchConfig, Searcher};
```

### Key Observations

**Lines 1-29**: Module documentation with a complete working example using `tempdir` for doctest safety.

**Lines 31-53**: Database constants - both names and suffixes for all database-related files.

**Lines 55-62**: Public module declarations using `pub mod`.

**Lines 64-76**: Public re-exports using `pub use` to provide a clean API surface.

---

## 3.3 Key Concepts: Library Design

### Why Have a lib.rs?

In Rust, `lib.rs` is the root of a library crate. It defines:

1. **What modules exist** — The structure of your code
2. **What's public** — The API you expose
3. **What's private** — Implementation details users shouldn't depend on

### The Public/Private Boundary

| What's Public | What's Private |
|--------------|----------------|
| `Database`, `Indexer` types | Internal helper functions |
| `Result<T>`, `Error` types | Private module contents |
| Constants like `DB_NAME` | Helper modules not in `pub mod` |

**The principle**: Expose what users need, hide what they don't.

### Semantic Versioning for APIs

The library follows semantic versioning:
- **Breaking changes** → Major version bump
- **New features** → Minor version bump
- **Bug fixes** → Patch version bump

By being explicit about what's public, users know what they can depend on.

---

## 3.4 Design Decision: Constants vs. Config

Notice that constants like `DB_NAME` are hardcoded, not configurable:

```rust
pub const DB_NAME: &str = ".ffts-index.db";
```

**Why not make this configurable?**

1. **Simplicity** — Users don't need to configure it
2. **Consistency** — All instances use the same name
3. **Discoverability** — Easy to find the database file

If users needed different names, we'd make it a configuration option. But for this tool, one name works everywhere.

---

## 3.5 File Structure Overview

Here's the complete lib.rs structure:

```rust
//! ffts-grep documentation (lines 1-17)
//! - What the library does
//! - Usage example
//! - Requirements

// Database constants (lines 22-40)
// - DB_NAME, DB_SHM_NAME, etc.
// - Suffix constants

// Module declarations (lines 42-49)
// - pub mod cli
// - pub mod db
// - ... (8 modules total)

// Type re-exports (lines 51-63)
// - Database, Indexer, Searcher
// - Doctor, Error, ExitCode, Result
// - DatabaseHealth, IndexerConfig
```

---

## 3.6 The Complete lib.rs File

```rust
//! ffts-grep: Fast FTS5 file indexer with sub-10ms queries.
//!
//! # Example
//!
//! ```rust,no_run
//! use ffts_indexer::{Database, Indexer, IndexerConfig, PragmaConfig, DB_NAME};
//!
//! let db = Database::open(Path::new(DB_NAME), &PragmaConfig::default())?;
//! db.init_schema()?;
//!
//! let mut indexer = Indexer::new(Path::new("."), db, IndexerConfig::default());
//! indexer.index_directory()?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! <small>Note: This library requires Rust 1.85+ (Edition 2024).</small>

use std::path::Path;

// Database constants (lib.rs:22-40)
pub const DB_NAME: &str = ".ffts-index.db";
pub const DB_SHM_NAME: &str = ".ffts-index.db-shm";
pub const DB_WAL_NAME: &str = ".ffts-index.db-wal";
pub const DB_TMP_NAME: &str = ".ffts-index.db.tmp";
pub const DB_TMP_GLOB: &str = ".ffts-index.db.tmp*";

pub const DB_SHM_SUFFIX: &str = "-shm";
pub const DB_WAL_SUFFIX: &str = "-wal";
pub const DB_TMP_SUFFIX: &str = ".tmp";

// Re-export modules (lib.rs:42-49)
pub mod cli;
pub mod db;
pub mod doctor;
pub mod error;
pub mod health;
pub mod indexer;
pub mod init;
pub mod search;

// Re-export types (lib.rs:55-78)
pub use db::{Database, PragmaConfig, SchemaCheck, SearchResult};
pub use doctor::{
    CheckResult, Doctor, DoctorOutput, DoctorSummary, EXPECTED_APPLICATION_ID, Severity,
};
pub use error::{ExitCode, IndexerError, Result};
pub use health::{
    DatabaseHealth, DetectionMethod, ProjectRoot, auto_init, auto_init_with_config,
    backup_and_reinit, backup_and_reinit_with_config, check_health_fast, find_project_root,
};
pub use indexer::{IndexStats, Indexer, IndexerConfig};
pub use init::{GitignoreResult, InitResult, check_gitignore, gitignore_entries, update_gitignore};
pub use search::{SearchConfig, Searcher};
```

---

## 3.7 Using the Library

Users can import types in several ways:

### Minimal Import

```rust
use ffts_indexer::{Database, Indexer, Searcher};
```

### Explicit Imports

```rust
use ffts_indexer::db::Database;
use ffts_indexer::indexer::Indexer;
use ffts_indexer::search::Searcher;
```

### Wildcard Import (Generally Avoid)

```rust
use ffts_indexer::*;  // Imports everything public
```

The first approach (curly-brace imports) is recommended—it's explicit about what's used.

---

## 3.8 Chapter Summary

| Aspect | What We Learned |
|--------|-----------------|
| Purpose | lib.rs is the library's public API |
| Doc comments | First thing users see on crates.io |
| Constants | DB_NAME, file suffixes |
| Module re-exports | Making modules part of public API |
| Type re-exports | Cleaner imports for users |
| Public/private | Expose what's needed, hide the rest |

---

## Exercises

### Exercise 3.1: Explore the Exports

Run this to see what's exported:

```bash
cargo doc --no-deps --open
# Then check target/doc/ffts_indexer/index.html
```

**Deliverable:** List all public types and constants.

### Exercise 3.2: Create Your Own Library

Create a new library:

```bash
cargo new --lib my_library
```

Add modules and exports:

```rust
// src/lib.rs
pub mod greet;
pub mod math;
pub use crate::greet::hello;
pub use crate::math::add;
```

**Deliverable:** Create a library that exports at least 3 functions.

### Exercise 3.3: Design an API

Design a library for a simple calculator. What would your lib.rs look like?

**Deliverable:** Write the lib.rs file and explain your design choices.

### Exercise 3.4: Constants vs Config

When should you use constants vs. configurable values? Give examples.

**Deliverable:** Write 3 examples of each approach and explain the trade-offs.

---

**Next Chapter**: [Chapter 4: error.rs - Error Handling](04-error_rs.md)
