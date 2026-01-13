# Chapter 9: search.rs - Query Execution

> "Search is the core of the experience." — Google Design Philosophy

## 9.1 What Does This File Do? (In Simple Terms)

The `search.rs` file is responsible for **executing searches** and **formatting results**. It takes a user's search query, sanitizes it (removes dangerous characters), sends it to the database, and formats the results for display.

### The Librarian Analogy

When you ask a librarian "Where can I find books about programming?":

| Librarian | This Application |
|-----------|------------------|
| Understands your question | Parses query |
| Removes inappropriate words | Sanitizes input |
| Checks the card catalog | Queries FTS5 |
| Formats the answer | Outputs plain/JSON |

The librarian doesn't store the books (that's the database's job)—they just help you find them!

---

## 9.2 SearchConfig: Search Configuration

See `search.rs:9-24`:

```rust
/// Configuration for search operations.
#[derive(Debug, Clone)]
pub struct SearchConfig {
    /// Search paths only (no content)
    pub paths_only: bool,

    /// Output format
    pub format: OutputFormat,

    /// Maximum results to return
    pub max_results: u32,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            paths_only: false,
            format: OutputFormat::Plain,
            max_results: 15,
        }
    }
}
```

---

## 9.3 The Searcher Struct

See `search.rs:39-49`:

```rust
/// FTS5 search executor.
pub struct Searcher<'a> {
    db: &'a mut Database,
    config: SearchConfig,
}

impl<'a> Searcher<'a> {
    /// Create a new searcher.
    pub const fn new(db: &'a mut Database, config: SearchConfig) -> Self {
        Self { db, config }
    }
}
```

The searcher holds:
- A mutable database reference (to execute queries)
- Configuration (how to search and format)

---

## 9.4 Query Sanitization: The Most Important Function

See `search.rs:67-109`:

```rust
/// Sanitize query for FTS5 MATCH with auto-prefix detection.
///
/// Replaces FTS5 special characters with spaces to prevent syntax errors while
/// still allowing fuzzy matching. If the query ends with `-` or `_`, we append
/// `*` to enable FTS5 prefix matching (e.g., "01-" → "01*").
///
/// # Performance
/// Called for every search query. Marked `#[inline]` for hot-path optimization.
#[inline]
fn sanitize_query(query: &str) -> String {
    let trimmed = query.trim();
    let auto_prefix = trimmed.ends_with('-') || trimmed.ends_with('_');

    // FTS5 special characters that need escaping or removal:
    // * - wildcard (remove to search for literal)
    // " - phrase delimiter (remove)
    // ( ) - grouping (remove)
    // : - column specifier (remove)
    // ^ - start of term (remove)
    // @ - phonetics (remove)
    // ~ - proximity (remove)
    // - - negation (replace with space)

    let mut result = String::with_capacity(query.len() + 1);

    for ch in trimmed.chars() {
        match ch {
            '*' | '"' | '(' | ')' | ':' | '^' | '@' | '~' => {
                // Remove special operators
                result.push(' ');
            }
            '-' | '_' | '.' | '/' | '\\' | '[' | ']' | '{' | '}' | '+' | '!' | '=' | '>'
            | '<' | '&' | '|' => {
                // Replace with space to avoid FTS5 syntax
                result.push(' ');
            }
            '\n' | '\r' | '\t' => {
                // Whitespace is handled by FTS5
                result.push(ch);
            }
            _ => {
                result.push(ch);
            }
        }
    }

    let sanitized = result.split_whitespace().collect::<Vec<_>>().join(" ");

    if auto_prefix && !sanitized.is_empty() { format!("{sanitized}*") } else { sanitized }
}
```

### Why Sanitize?

FTS5 has special characters that change search behavior:

| Character | FTS5 Meaning | Our Response |
|-----------|--------------|--------------|
| `*` | Wildcard | Remove (search literally) |
| `"` | Phrase | Remove (avoid syntax errors) |
| `(` `)` | Grouping | Remove (avoid syntax errors) |
| `-` | NOT operator | Replace with space |
| `:` | Column specifier | Remove (avoid syntax errors) |

### Example

```
User input:  "main -test"
After sanitize:  "main test"  (NOT "main" AND NOT "test")
```

Without sanitization, `-test` would mean "NOT test", which is probably not what the user meant!

---

## 9.5 Executing the Search (Two-Phase)

See `search.rs:48-118`:

```rust
/// Execute a search query with two-phase search.
///
/// Phase A: filename CONTAINS (SQL LIKE) for substring matches
/// Phase B: FTS5 BM25 for remaining slots
pub fn search(&mut self, query: &str) -> Result<Vec<SearchResult>> {
    let sanitized = Self::sanitize_query(query);

    if sanitized.trim().is_empty() {
        return Ok(vec![]);
    }

    let max = self.config.max_results as usize;

    // Phase A: filename CONTAINS (substring match)
    let filename_query = sanitized.split_whitespace().next().unwrap_or(&sanitized);
    let filename_matches =
        self.db.search_filename_contains(filename_query, self.config.max_results)?;

    let mut seen: HashSet<String> = HashSet::with_capacity(max);
    let mut results: Vec<SearchResult> = Vec::with_capacity(max);

    for path in filename_matches {
        if results.len() >= max {
            break;
        }
        seen.insert(path.clone());
        results.push(SearchResult { path, rank: -1000.0 });
    }

    // Phase B: FTS5 BM25
    if results.len() < max {
        let fts_limit = (max - results.len() + seen.len()) as u32;
        let fts_results = self.db.search(&sanitized, self.config.paths_only, fts_limit)?;

        for result in fts_results {
            if results.len() >= max {
                break;
            }
            if !seen.contains(&result.path) {
                seen.insert(result.path.clone());
                results.push(result);
            }
        }
    }

    Ok(results)
}
```

### Why Two Phases?

FTS5 matches whole tokens, so `"intro"` does not match `"introduction"`.
By running a filename substring search first (SQL `LIKE`), we capture user expectations
for file names, then fill the remaining slots with BM25-ranked FTS5 results.

### Literal `%` and `_` in Filename Queries

`search_filename_contains` escapes SQL `LIKE` wildcards (`%` and `_`) so these
characters are treated literally in filename searches.

---

## 9.6 Formatting Results

See `search.rs:111-144`:

```rust
/// Format and output search results.
pub fn format_results<W: Write>(&self, results: &[SearchResult], output: &mut W) -> Result<()> {
    match self.config.format {
        OutputFormat::Plain => Self::format_plain(results, output),
        OutputFormat::Json => Self::format_json(results, output),
    }
}

/// Format results as plain text (one path per line).
fn format_plain<W: Write>(results: &[SearchResult], output: &mut W) -> Result<()> {
    for result in results {
        writeln!(output, "{}", result.path)?;
    }
    Ok(())
}

/// Format results as JSON.
fn format_json<W: Write>(results: &[SearchResult], output: &mut W) -> Result<()> {
    let json_results: Vec<JsonSearchResult> = results
        .iter()
        .map(|r| JsonSearchResult { path: r.path.clone(), rank: r.rank })
        .collect();

    let output_struct = JsonOutput { results: json_results };

    let json = serde_json::to_string_pretty(&output_struct)?;
    writeln!(output, "{json}")?;

    Ok(())
}
```

### Plain Output

```
src/main.rs
tests/main_test.rs
examples/main.rs
```

### JSON Output

```json
{
  "results": [
    {
      "path": "src/main.rs",
      "rank": -0.5
    },
    {
      "path": "tests/main_test.rs",
      "rank": -0.3
    }
  ]
}
```

---

## 9.7 Chapter Summary

| Concept | What We Learned |
|---------|-----------------|
| Query sanitization | Remove FTS5 special characters |
| BM25 ranking | Results sorted by relevance |
| Plain output | Simple one-per-line format |
| JSON output | Structured machine-readable format |
| Hot-path optimization | #[inline] on frequently-called functions |

---

## Exercises

### Exercise 9.1: Test Sanitization

Run these searches and observe the sanitization:

```bash
ffts-grep search "main"
ffts-grep search "main -test"
ffts-grep search "main test"
ffts-grep search 'main "test"'
ffts-grep search "main*test"
```

**Deliverable:** Which ones work? Which fail? Why?

### Exercise 9.2: JSON Output

Compare plain and JSON output:

```bash
ffts-grep search "main" --format plain
ffts-grep search "main" --format json
```

**Deliverable:** Show both outputs and explain when you'd use each.

### Exercise 9.3: Path-Only Search

Compare regular and path-only search:

```bash
ffts-grep search "main"
ffts-grep search "main" --paths-only
```

**Deliverable:** What's the difference in results?

### Exercise 9.4: Add a New Output Format

Add XML output format to the search.

**Deliverable:** Show the code changes needed.

---

**Next Chapter**: [Chapter 10: init.rs - Project Initialization](10-init_rs.md)
