use serde::Serialize;
use std::io::Write;

use crate::db::{Database, SearchResult};
use crate::error::Result;

pub use crate::cli::OutputFormat;

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
        Self { paths_only: false, format: OutputFormat::Plain, max_results: 15 }
    }
}

/// Search result for JSON output.
#[derive(Debug, Serialize)]
pub struct JsonSearchResult {
    pub path: String,
    pub rank: f64,
}

/// JSON output structure.
#[derive(Debug, Serialize)]
pub struct JsonOutput {
    pub results: Vec<JsonSearchResult>,
}

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

    /// Execute a search query.
    ///
    /// # Errors
    /// Returns `IndexerError` if:
    /// - Database query execution fails
    /// - FTS5 MATCH syntax is invalid (after sanitization)
    pub fn search(&mut self, query: &str) -> Result<Vec<SearchResult>> {
        let sanitized = Self::sanitize_query(query);

        if sanitized.trim().is_empty() {
            return Ok(vec![]);
        }

        self.db.search(&sanitized, self.config.paths_only, self.config.max_results)
    }

    /// Sanitize query for FTS5 MATCH.
    ///
    /// Replaces FTS5 special characters with spaces to prevent
    /// syntax errors while still allowing fuzzy matching.
    ///
    /// # Performance
    /// Called for every search query. Marked `#[inline]` for hot-path optimization.
    #[inline]
    fn sanitize_query(query: &str) -> String {
        // FTS5 special characters that need escaping or removal:
        // * - wildcard (remove to search for literal)
        // " - phrase delimiter (remove)
        // ( ) - grouping (remove)
        // : - column specifier (remove)
        // ^ - start of term (remove)
        // @ - phonetics (remove)
        // ~ - proximity (remove)
        // - - negation (replace with space)

        let mut result = String::with_capacity(query.len());

        for ch in query.chars() {
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
                    // Whitespace is already handled by FTS5
                    result.push(ch);
                }
                _ => {
                    result.push(ch);
                }
            }
        }
        result.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Format and output search results.
    ///
    /// # Errors
    /// Returns `IndexerError` if:
    /// - Writing to the output stream fails (wrapped as `IndexerError::Io`)
    /// - JSON serialization fails (when using JSON format)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DB_NAME;
    use crate::cli::OutputFormat;
    use crate::db::PragmaConfig;
    use tempfile::tempdir;

    #[test]
    fn test_sanitize_query_simple() {
        let config = SearchConfig::default();
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let mut db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let _searcher = Searcher::new(&mut db, config);

        let sanitized = Searcher::sanitize_query("hello world");
        assert_eq!(sanitized, "hello world");
    }

    #[test]
    fn test_sanitize_query_special_chars() {
        let config = SearchConfig::default();
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let mut db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let _searcher = Searcher::new(&mut db, config);

        let sanitized = Searcher::sanitize_query("test*query\"with()special");
        // * and " and ( ) should be replaced
        assert!(!sanitized.contains('*'));
        assert!(!sanitized.contains('"'));
        assert!(!sanitized.contains('('));
        assert!(!sanitized.contains(')'));
        assert!(!sanitized.contains("  "));
    }

    #[test]
    fn test_sanitize_query_collapses_whitespace() {
        let config = SearchConfig::default();
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let mut db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let _searcher = Searcher::new(&mut db, config);

        let sanitized = Searcher::sanitize_query("a*b\"c   d");
        assert_eq!(sanitized, "a b c d");
    }

    #[test]
    fn test_plain_output() {
        let config = SearchConfig { format: OutputFormat::Plain, ..Default::default() };
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let mut db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let searcher = Searcher::new(&mut db, config);

        let results = vec![
            SearchResult { path: "src/main.rs".to_string(), rank: -0.5 },
            SearchResult { path: "tests/main_test.rs".to_string(), rank: -0.3 },
        ];

        let mut output = Vec::new();
        searcher.format_results(&results, &mut output).unwrap();

        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("src/main.rs"));
        assert!(text.contains("tests/main_test.rs"));
    }

    #[test]
    fn test_json_output() {
        let config = SearchConfig { format: OutputFormat::Json, ..Default::default() };
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let mut db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let searcher = Searcher::new(&mut db, config);

        let results = vec![SearchResult { path: "src/main.rs".to_string(), rank: -0.5 }];

        let mut output = Vec::new();
        searcher.format_results(&results, &mut output).unwrap();

        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("src/main.rs"));
        assert!(text.contains("rank"));
        assert!(text.contains("results"));
    }

    #[test]
    fn test_json_escaping() {
        let config = SearchConfig::default();
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let mut db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();

        // Insert a file with special characters
        db.upsert_file("test\"with'quotes.rs", "content with \"quotes\" and 'apostrophes'", 0, 50)
            .unwrap();

        let mut searcher = Searcher::new(&mut db, config);

        let results = searcher.search("quotes").unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_empty_query_returns_empty() {
        let config = SearchConfig::default();
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let mut db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        let mut searcher = Searcher::new(&mut db, config);

        let results = searcher.search("").unwrap();
        assert!(results.is_empty());

        let results = searcher.search("   ").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_max_results() {
        let config = SearchConfig { max_results: 5, ..Default::default() };
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let mut db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();

        // Create many files first
        for i in 0..20 {
            db.upsert_file(&format!("file{i}.rs"), "test content", 0, 12).unwrap();
        }

        // Create searcher after inserting files
        let mut searcher = Searcher::new(&mut db, config);
        let results = searcher.search("test").unwrap();
        assert_eq!(results.len(), 5);
    }
}
