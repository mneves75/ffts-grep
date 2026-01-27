# Chapter 5: cli.rs - Argument Parsing

> "A well-designed CLI feels like a conversation with the computer." — Unknown

## 5.1 What Does This File Do? (In Simple Terms)

The `cli.rs` file uses the `clap` crate to define and parse command-line arguments. When you run `ffts-grep search "main"`, this file:
1. Defines what arguments are valid
2. Parses the arguments from the command line
3. Validates the input (e.g., numbers must be positive)

### The Restaurant Order Analogy

Think of CLI parsing like taking a restaurant order:

| CLI Parsing | Restaurant |
|-------------|------------|
| Arguments | Order items |
| Validation | "Sorry, we don't have that" |
| Subcommands | Appetizer, Main, Dessert |
| Options | "No onions", "Extra crispy" |
| Help | "What do you recommend?" |

---

## 5.2 The Code: clap Derive API

Let's examine the CLI structure at `cli.rs:53-91`:

```rust
#[derive(Debug, Parser)]
#[command(name = "ffts-grep", version, about, long_about = None)]
pub struct Cli {
    /// Search query (triggers search mode if provided)
    #[arg(index = 1)]
    pub query: Vec<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Suppress status messages (for CI/scripting)
    #[arg(long, short = 'q')]
    pub quiet: bool,

    /// Project directory (defaults to current directory)
    #[arg(long, env = "CLAUDE_PROJECT_DIR")]
    pub project_dir: Option<PathBuf>,

    /// Follow symlinks while indexing (disabled by default for safety)
    #[arg(long)]
    pub follow_symlinks: bool,

    /// `SQLite` cache size in `KB` (negative) or `pages` (positive)
    #[arg(long, default_value = "-32000", value_parser = validate_cache_size)]
    pub pragma_cache_size: i64,

    /// Memory-mapped I/O size in bytes (0 = disabled on macOS)
    #[arg(long, default_value_t = DEFAULT_MMAP_SIZE, value_parser = validate_mmap_size)]
    pub pragma_mmap_size: i64,

    /// Database page size in bytes (must be power of 2, 512-65536)
    #[arg(long, default_value = "4096", value_parser = validate_page_size)]
    pub pragma_page_size: i64,

    /// Busy timeout in milliseconds (0 = disabled)
    #[arg(long, default_value = "5000", value_parser = validate_busy_timeout)]
    pub pragma_busy_timeout: i64,

    /// `SQLite` synchronous mode (`OFF`, `NORMAL`, `FULL`, `EXTRA`)
    #[arg(long, default_value = "NORMAL", value_parser = validate_synchronous)]
    pub pragma_synchronous: String,
}
```

### Understanding #[derive(Debug, Parser)]

The `Parser` derive comes from clap and automatically:
- Generates a CLI parser from struct fields
- Creates `--help` and `--version` automatically
- Handles type conversion (String → PathBuf, etc.)

### Field Types and Attributes

| Field Type | What It Handles | Example |
|------------|-----------------|---------|
| `String` | Text arguments | `--format json` |
| `PathBuf` | File paths | `--project-dir ./src` |
| `bool` | Flags | `--quiet` |
| `Option<T>` | Optional values | `--max-results 100` |
| `Vec<String>` | Multiple values | `search a b c` |
| `enum` | Fixed choices | `plain` or `json` |

---

## 5.3 Subcommands: The Commands Enum

See `cli.rs:94-138`:

```rust
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Index or reindex files in the project directory.
    Index {
        /// Force full reindex (atomic replace)
        #[arg(long)]
        reindex: bool,
    },
    /// Run diagnostic checks on installation health.
    Doctor {
        /// Verbose output for diagnostics
        #[arg(long, short = 'v')]
        verbose: bool,
        /// JSON output format
        #[arg(long)]
        json: bool,
    },
    /// Initialize project with .gitignore and database.
    Init {
        /// Only update .gitignore, skip database creation
        #[arg(long)]
        gitignore_only: bool,
        /// Force reinitialization even if already initialized
        #[arg(long)]
        force: bool,
    },
    /// Search indexed files (this is the default when a query is provided).
    Search {
        /// Search query
        #[arg(index = 1)]
        query: Vec<String>,
        /// Search paths only (no content)
        #[arg(long)]
        paths: bool,
        /// Output format
        #[arg(long, value_enum)]
        format: Option<OutputFormat>,
        /// Run performance benchmark
        #[arg(long)]
        benchmark: bool,
        /// Disable auto-initialization on search (fail if no database)
        #[arg(long)]
        no_auto_init: bool,
    },
}
```

### Subcommand Features

- **`#[arg(index = 1)]`** — Positional query for search
- **`#[arg(long)]`** — Long-form flags like `--paths` and `--no-auto-init`
- **`#[arg(long, short = 'v')]`** — Short alias for verbose (`-v`)

---

## 5.4 Output Format Enum

See `cli.rs:11-16`:

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    /// Human-readable plain text output
    #[default]
    Plain,

    /// Machine-readable JSON output
    Json,
}
```

The `ValueEnum` derive allows clap to automatically validate input:

```bash
ffts-grep search "main" --format plain    # ✓ Works
ffts-grep search "main" --format json     # ✓ Works
ffts-grep search "main" --format xml      # ✗ Error: invalid value
```

Note: On non-macOS platforms the `--pragma-mmap-size` default displays as `268435456`
(256MB) instead of `0`.

---

## 5.5 Validation Functions

See `cli.rs:131-191`:

### Cache Size Validation

```rust
fn validate_cache_size(s: &str) -> Result<i64, String> {
    let val: i64 = s.parse().map_err(|_| "invalid integer".to_string())?;

    match val {
        v if v > 0 => Ok(v),                              // pages
        v if (-1_000_000..=-1_000).contains(&v) => Ok(v), // KB range
        _ => Err("must be positive (pages) or -1000 to -1000000 (KB)".to_string()),
    }
}
```

### Memory Map Size Validation

`pragma_mmap_size` defaults to `0` on macOS and `256MB` on other platforms via
`DEFAULT_MMAP_SIZE`.

```rust
fn validate_mmap_size(s: &str) -> Result<i64, String> {
    const MAX_MMAP: i64 = 256 * 1024 * 1024;
    let val: i64 = s.parse().map_err(|_| "invalid integer".to_string())?;

    if val < 0 {
        return Err("must be >= 0".to_string());
    }

    if val > MAX_MMAP {
        return Err(format!("must be <= {MAX_MMAP} (256MB)"));
    }

    Ok(val)
}
```

### Synchronous Mode Validation

```rust
fn validate_synchronous(s: &str) -> Result<String, String> {
    let mode = s.to_uppercase();
    match mode.as_str() {
        "OFF" | "NORMAL" | "FULL" | "EXTRA" => Ok(mode),
        _ => Err("must be OFF, NORMAL, FULL, or EXTRA".to_string()),
    }
}
```

---

## 5.6 Helper Methods

See `cli.rs:193-277`:

```rust
impl Cli {
    /// Resolve the effective project directory.
    pub fn project_dir(&self) -> Result<PathBuf> {
        match &self.project_dir {
            Some(path) => self.expand_tilde(path),
            None => {
                let cwd = std::env::current_dir().map_err(|e| IndexerError::ConfigInvalid {
                    field: "project_dir".to_string(),
                    value: "current_dir".to_string(),
                    reason: e.to_string(),
                })?;
                Ok(find_project_root(&cwd).path)
            }
        }
    }

    /// Expand tilde (`~`) to home directory in path.
    fn expand_tilde(&self, path: &Path) -> Result<PathBuf> {
        if let Some(stripped) = path.to_str().and_then(|s| s.strip_prefix('~')) {
            let home = dirs::home_dir().ok_or_else(|| IndexerError::ConfigInvalid {
                field: "project_dir".to_string(),
                value: path.to_string_lossy().to_string(),
                reason: "Could not determine home directory".to_string(),
            })?;
            if stripped.is_empty() {
                return Ok(home);
            }
            if stripped.starts_with('/') || stripped.starts_with('\\') {
                return Ok(home.join(&stripped[1..]));
            }
        }
        Ok(path.to_path_buf())
    }

    /// Get the query as a single string (if present).
    pub fn query_string(&self) -> Option<String> {
        if self.query.is_empty() { None } else { Some(self.query.join(" ")) }
    }

    /// Check if --index was passed
    pub fn wants_index(&self) -> bool {
        matches!(self.command, Some(Commands::Index { .. }))
    }

    /// Check if --reindex was passed
    pub fn wants_reindex(&self) -> bool {
        match &self.command {
            Some(Commands::Index { reindex }) => *reindex,
            _ => false,
        }
    }

    /// Check if --doctor was passed
    pub fn wants_doctor(&self) -> bool {
        matches!(self.command, Some(Commands::Doctor { .. }))
    }

    /// Check if --init was passed
    pub fn wants_init(&self) -> bool {
        matches!(self.command, Some(Commands::Init { .. }))
    }
}
```


---

## 5.7 The Help System

clap automatically generates help text:

```bash
$ ffts-grep --help
Fast full-text search file indexer using SQLite FTS5

A high-performance file indexer that provides sub-10ms queries after initial indexing.
Uses SQLite FTS5 with BM25 ranking for relevant search results.

Version: 0.11.2
Repository: https://github.com/mneves75/ffts-grep

SUBCOMMANDS:
  search     Search indexed files (default when query provided)
  index      Index or reindex the project directory
  doctor     Run diagnostic checks on installation health
  init       Initialize project with .gitignore and database

EXIT CODES:
  0   Success
  1   Warnings (non-fatal issues found)
  2   Errors (diagnostic failures)

For more information, see: https://github.com/mneves75/ffts-grep

Usage: 

Commands:
  index   Index or reindex files in the project directory
  doctor  Run diagnostic checks on installation health
  init    Initialize project with .gitignore and database
  search  Search indexed files (this is the default when a query is provided)
  help    Print this message or the help of the given subcommand(s)

Arguments:
  [QUERY]...
          Search query (triggers search mode if provided)

Options:
  -q, --quiet
          Suppress status messages (for CI/scripting)

      --project-dir <PROJECT_DIR>
          Project directory (defaults to current directory)
          
          [env: CLAUDE_PROJECT_DIR=]

      --follow-symlinks
          Follow symlinks while indexing (disabled by default for safety)

      --refresh
          Refresh index before search (search-only)

      --pragma-cache-size <PRAGMA_CACHE_SIZE>
          `SQLite` cache size in `KB` (negative) or `pages` (positive)
          
          [default: -32000]

      --pragma-mmap-size <PRAGMA_MMAP_SIZE>
          Memory-mapped I/O size in bytes (0 = disabled on macOS)
          
          [default: 0]

      --pragma-page-size <PRAGMA_PAGE_SIZE>
          Database page size in bytes (must be power of 2, 512-65536)
          
          [default: 4096]

      --pragma-busy-timeout <PRAGMA_BUSY_TIMEOUT>
          Busy timeout in milliseconds (0 = disabled)
          
          [default: 5000]

      --pragma-synchronous <PRAGMA_SYNCHRONOUS>
          `SQLite` synchronous mode (`OFF`, `NORMAL`, `FULL`, `EXTRA`)
          
          [default: NORMAL]

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version
```

---

## 5.8 Design Patterns Used

### 1. Single-Pass Detection

The `project_dir()` method checks multiple sources in order:

1. CLI argument (`--project-dir`)
2. Environment variable (`CLAUDE_PROJECT_DIR`)
3. Auto-detection (walk up directories)

### 2. Validation at Boundaries

Input validation happens early, at the "boundary" between user and code. This is a security principle:

```bash
# Bad: Validate late, after using the value
value = read_from_user()
if value.is_invalid() { ... }  # Too late!

# Good: Validate immediately
value = parse_user_input()?
if value.is_invalid() { ... }  # Catch errors early!
```

### 3. Builder Pattern via Derive

clap's derive API is a form of the Builder pattern—you specify what you want, clap builds the parser.

---

## 5.9 Chapter Summary

| Concept | What We Learned |
|---------|-----------------|
| clap derive | Declarative CLI definition |
| Struct fields | Arguments and options |
| Subcommands | Major commands (index, search, doctor, init) |
| Validation | Ensuring input is valid early |
| Helper methods | Convenience functions for CLI state |
| Auto-generated help | clap builds documentation |

---

## Exercises

### Exercise 5.1: Explore CLI Arguments

Run all these commands and observe the output:

```bash
ffts-grep --help
ffts-grep search --help
ffts-grep index --help
ffts-grep doctor --help
ffts-grep init --help
```

**Deliverable:** List all available options for the `search` command.

### Exercise 5.2: Invalid Input

Try these invalid commands and observe the error messages:

```bash
ffts-grep search --format invalid
ffts-grep search --pragma-mmap-size abc
ffts-grep search --pragma-cache-size 9999999999
```

**Deliverable:** Are the error messages helpful? How could they be better?

### Exercise 5.3: Add a New Option

Add a `--max-file-size` option that limits the maximum file size to index.

**Deliverable:** Modify cli.rs and show the code changes.

### Exercise 5.4: Design a CLI

Design a CLI for a simple note-taking app with:
- `add "note text"` — Add a note
- `list` — List all notes
- `delete <id>` — Delete a note by ID
- `--format json` — JSON output

**Deliverable:** Write the CLI struct using clap derive.

---

**Next Chapter**: [Chapter 6: main.rs - The Entry Point](06-main_rs.md)
