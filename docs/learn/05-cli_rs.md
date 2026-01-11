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

Let's examine the CLI structure at `cli.rs:19-82`:

```rust
#[derive(Debug, Parser)]
#[command(name = "ffts-grep", version, about, long_about = None)]
pub struct Cli {
    /// Query string for search (positional, captures all remaining args)
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub query: Vec<String>,

    /// Project directory to search (default: auto-detect)
    #[arg(long, short)]
    pub project_dir: Option<PathBuf>,

    /// Suppress status messages
    #[arg(long, short)]
    pub quiet: bool,

    /// Output format: 'plain' or 'json'
    #[arg(long, value_enum)]
    pub format: Option<OutputFormat>,

    /// SQLite cache size in KB (negative = KB, positive = pages)
    #[arg(long)]
    pub pragma_cache_size: Option<i64>,

    /// SQLite mmap size in bytes (0 to disable)
    #[arg(long)]
    pub pragma_mmap_size: Option<u64>,

    /// SQLite page size (512 to 65536, power of 2)
    #[arg(long)]
    pub pragma_page_size: Option<u32>,

    /// SQLite busy timeout in milliseconds
    #[arg(long)]
    pub pragma_busy_timeout: Option<u32>,

    /// SQLite synchronous mode: OFF, NORMAL, FULL, EXTRA
    #[arg(long)]
    pub pragma_synchronous: Option<String>,

    /// Command to run
    #[command(subcommand)]
    pub command: Option<Commands>,
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

See `cli.rs:86-129`:

```rust
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Index or reindex project files
    #[command(alias = "i")]
    Index {
        /// Force full reindex
        #[arg(long)]
        reindex: bool,
    },

    /// Search indexed files
    #[command(alias = "s")]
    Search {
        /// Search paths only (no content)
        #[arg(long)]
        paths_only: bool,

        /// Maximum number of results
        #[arg(long, short)]
        max_results: Option<u32>,
    },

    /// Run diagnostic checks
    #[command(alias = "d", alias = "check")]
    Doctor {
        /// Verbose output
        #[arg(long, short)]
        verbose: bool,
    },

    /// Initialize project for indexing
    #[command(aliases = ["init", "initialise"])]
    Init {
        /// Only update .gitignore
        #[arg(long, short)]
        gitignore_only: bool,

        /// Force reinitialization
        #[arg(long, short)]
        force: bool,
    },
}
```

### Subcommand Features

- **`#[command(alias = "i")]`** — Short form: `ffts-grep i`
- **`#[command(aliases = ...)]`** — Multiple aliases
- **`#[arg(long, short)]`** — Both `--verbose` and `-v` work

---

## 5.4 Output Format Enum

See `cli.rs:11-16`:

```rust
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum OutputFormat {
    /// Human-readable plain text output
    #[value(name = "plain")]
    Plain,

    /// Machine-readable JSON output
    #[value(name = "json")]
    Json,
}
```

The `ValueEnum` derive allows clap to automatically validate input:

```bash
ffts-grep search "main" --format plain    # ✓ Works
ffts-grep search "main" --format json     # ✓ Works
ffts-grep search "main" --format xml      # ✗ Error: invalid value
```

---

## 5.5 Validation Functions

See `cli.rs:131-191`:

### Cache Size Validation

```rust
fn validate_cache_size(s: &str) -> Result<i64, String> {
    let value: i64 = s.parse().map_err(|_| "must be an integer")?;

    if value == 0 || value < -1_000_000 || value > 1_000_000 {
        Err("must be between -1000000 and 1000000 (absolute value)".to_string())
    } else {
        Ok(value)
    }
}
```

### Memory Map Size Validation

```rust
fn validate_mmap_size(s: &str) -> Result<u64, String> {
    let value: u64 = s.parse().map_err(|_| "must be a non-negative integer")?;

    if value != 0 && !value.is_power_of_two() {
        Err("must be 0 or a power of 2".to_string())
    } else if value > 256 * 1024 * 1024 {
        Err("must not exceed 256 MB".to_string())
    } else {
        Ok(value)
    }
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
    /// Parse command-line arguments (uses Cli::parse() internally)
    #[inline]
    pub fn parse() -> Self {
        clap::Parser::parse()
    }

    /// Get effective project directory
    pub fn project_dir(&self) -> Result<PathBuf> {
        // Single-pass detection: try CLI arg, then env var, then auto-detect
        if let Some(dir) = &self.project_dir {
            return Ok(dir.clone());
        }

        if let Ok(env_dir) = std::env::var("CLAUDE_PROJECT_DIR") {
            return Ok(PathBuf::from(env_dir));
        }

        // Auto-detect via health module
        let current_dir = std::env::current_dir()
            .map_err(|e| IndexerError::Io { source: e })?;

        match health::find_project_root(&current_dir) {
            health::ProjectRoot { path, method } => {
                tracing::debug!(method = ?method, path = %path.display(), "Auto-detected project root");
                Ok(path)
            }
        }
    }

    /// Get combined query string from positional args
    #[inline]
    pub fn query_string(&self) -> Option<String> {
        if self.query.is_empty() {
            None
        } else {
            Some(self.query.join(" "))
        }
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
Usage: ffts-grep [OPTIONS] [QUERY]...

Arguments:
  [QUERY]...    Query string for search (positional, captures all remaining args)

Options:
  --project-dir <PROJECT_DIR>    Project directory to search (default: auto-detect)
  --quiet                        Suppress status messages
  --format <FORMAT>              Output format: 'plain' or 'json'
  --pragma-cache-size <N>        SQLite cache size in KB
  --pragma-mmap-size <N>         SQLite mmap size in bytes
  --pragma-page-size <N>         SQLite page size
  --pragma-busy-timeout <N>      SQLite busy timeout in ms
  --pragma-synchronous <MODE>    SQLite synchronous mode
  -h, --help                     Print help
  -V, --version                  Print version

Subcommands:
  index     Index or reindex project files
  search    Search indexed files
  doctor    Run diagnostic checks
  init      Initialize project for indexing
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
