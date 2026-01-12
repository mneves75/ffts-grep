use serde::Serialize;
use std::collections::HashSet;
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

    /// Execute a search query with two-phase search.
    ///
    /// **Phase A**: SQL LIKE filename CONTAINS matches (absolute priority)
    /// **Phase B**: FTS5 BM25 search for remaining slots
    ///
    /// This two-phase approach solves FTS5's token matching limitation:
    /// FTS5 tokenizes "01-introduction.md" as ["01", "introduction"], so
    /// query "intro" doesn't match "introduction" (exact token match only).
    /// SQL LIKE '%intro%' bypasses tokenization and finds substring matches.
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

        let max = self.config.max_results as usize;

        // Phase A: Filename CONTAINS matches (absolute priority)
        // Use first token for filename search (most relevant for file lookup)
        let filename_query = sanitized.split_whitespace().next().unwrap_or(&sanitized);
        let filename_matches =
            self.db.search_filename_contains(filename_query, self.config.max_results)?;

        let mut seen: HashSet<String> = HashSet::with_capacity(max);
        let mut results: Vec<SearchResult> = Vec::with_capacity(max);

        // Add filename matches first with synthetic high-priority rank (-1000.0)
        // Lower rank = better match in BM25, so -1000.0 ensures filename matches come first
        for path in filename_matches {
            if results.len() >= max {
                break;
            }
            seen.insert(path.clone());
            results.push(SearchResult { path, rank: -1000.0 });
        }

        // Phase B: FTS5 BM25 for remaining slots (content/path matches)
        if results.len() < max {
            // Request more results to account for deduplication
            let fts_limit = (max - results.len() + seen.len()) as u32;
            let fts_results = self.db.search(&sanitized, self.config.paths_only, fts_limit)?;

            for result in fts_results {
                if results.len() >= max {
                    break;
                }
                // Deduplicate: skip if already in filename matches
                if !seen.contains(&result.path) {
                    seen.insert(result.path.clone());
                    results.push(result);
                }
            }
        }

        Ok(results)
    }

    /// Sanitize query for FTS5 MATCH with auto-prefix detection.
    ///
    /// Replaces FTS5 special characters with spaces to prevent
    /// syntax errors while still allowing fuzzy matching.
    ///
    /// **Auto-prefix**: Trailing `-` or `_` triggers FTS5 prefix query:
    /// - `"01-"` → `"01*"` (matches "01-introduction", "01-chapter")
    /// - `"test_"` → `"test*"` (matches "test_utils", "test_config")
    ///
    /// # Performance
    /// Called for every search query. Marked `#[inline]` for hot-path optimization.
    #[inline]
    pub fn sanitize_query(query: &str) -> String {
        let trimmed = query.trim();

        // Auto-prefix: trailing `-` or `_` triggers FTS5 prefix wildcard
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

        let mut result = String::with_capacity(query.len() + 1); // +1 for potential '*'

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
                    // Whitespace is already handled by FTS5
                    result.push(ch);
                }
                _ => {
                    result.push(ch);
                }
            }
        }

        let sanitized = result.split_whitespace().collect::<Vec<_>>().join(" ");

        // Apply auto-prefix: append '*' to enable FTS5 prefix matching
        if auto_prefix && !sanitized.is_empty() { format!("{sanitized}*") } else { sanitized }
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

    // ============================================
    // Auto-prefix tests
    // ============================================

    #[test]
    fn test_auto_prefix_trailing_hyphen() {
        // "01-" should become "01*" for FTS5 prefix matching
        let sanitized = Searcher::sanitize_query("01-");
        assert_eq!(sanitized, "01*");
    }

    #[test]
    fn test_auto_prefix_trailing_underscore() {
        // "test_" should become "test*" for FTS5 prefix matching
        let sanitized = Searcher::sanitize_query("test_");
        assert_eq!(sanitized, "test*");
    }

    #[test]
    fn test_no_auto_prefix_for_normal_query() {
        // Normal queries should not get a wildcard
        let sanitized = Searcher::sanitize_query("intro");
        assert!(!sanitized.contains('*'));
        assert_eq!(sanitized, "intro");
    }

    #[test]
    fn test_auto_prefix_multi_word_query() {
        // "hello world-" should become "hello world*"
        let sanitized = Searcher::sanitize_query("hello world-");
        assert!(sanitized.ends_with('*'));
        assert_eq!(sanitized, "hello world*");
    }

    #[test]
    fn test_auto_prefix_only_hyphen() {
        // Just "-" should become empty (stripped, then no content for prefix)
        let sanitized = Searcher::sanitize_query("-");
        assert!(sanitized.is_empty());
    }

    // ============================================
    // Two-phase search tests
    // ============================================

    #[test]
    fn test_filename_contains_priority() {
        // "intro" should find "01-introduction.md" via CONTAINS match
        // even though FTS5 tokenizes it as "01" + "introduction"
        let config = SearchConfig::default();
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let mut db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();

        // File with "intro" in filename (substring match)
        db.upsert_file("docs/learn/01-introduction.md", "chapter one content", 0, 20).unwrap();
        // File with "intro" in content only
        db.upsert_file("other.md", "intro content here unrelated", 0, 30).unwrap();

        let mut searcher = Searcher::new(&mut db, config);
        let results = searcher.search("intro").unwrap();

        assert!(!results.is_empty());
        // Filename CONTAINS match should come first
        assert_eq!(results[0].path, "docs/learn/01-introduction.md");
    }

    #[test]
    fn test_exact_filename_match_priority() {
        // Exact filename match should rank higher than substring match
        let config = SearchConfig::default();
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let mut db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();

        // Exact match
        db.upsert_file("config.rs", "configuration file", 0, 20).unwrap();
        // Substring match (longer filename)
        db.upsert_file("my-config-utils.rs", "utility functions", 0, 20).unwrap();

        let mut searcher = Searcher::new(&mut db, config);
        let results = searcher.search("config").unwrap();

        assert!(!results.is_empty());
        // Shorter/exact match should come first
        assert_eq!(results[0].path, "config.rs");
    }

    #[test]
    fn test_two_phase_deduplication() {
        // Results should not be duplicated between phases
        let config = SearchConfig { max_results: 10, ..Default::default() };
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let mut db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();

        // File that matches both filename AND content
        db.upsert_file("test.rs", "test content here", 0, 20).unwrap();
        db.upsert_file("other.rs", "test content", 0, 15).unwrap();

        let mut searcher = Searcher::new(&mut db, config);
        let results = searcher.search("test").unwrap();

        // Verify no duplicates
        let paths: Vec<&str> = results.iter().map(|r| r.path.as_str()).collect();
        let unique_paths: std::collections::HashSet<&str> = paths.iter().copied().collect();
        assert_eq!(paths.len(), unique_paths.len(), "Results contain duplicates");
    }

    #[test]
    fn test_filename_matches_have_priority_rank() {
        // Filename matches should have synthetic rank -1000.0
        let config = SearchConfig::default();
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let mut db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();

        db.upsert_file("intro.md", "introduction", 0, 12).unwrap();

        let mut searcher = Searcher::new(&mut db, config);
        let results = searcher.search("intro").unwrap();

        assert!(!results.is_empty());
        // First result (filename match) should have priority rank
        assert_eq!(results[0].rank, -1000.0);
    }

    #[test]
    fn test_content_only_match_when_no_filename_match() {
        // When no filename matches, content matches should still work
        let config = SearchConfig::default();
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);
        let mut db = Database::open(&db_path, &PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();

        // File with "unique" only in content, not filename
        db.upsert_file("random.rs", "this has uniquekeyword in it", 0, 30).unwrap();

        let mut searcher = Searcher::new(&mut db, config);
        let results = searcher.search("uniquekeyword").unwrap();

        assert!(!results.is_empty());
        assert_eq!(results[0].path, "random.rs");
        // This should be a BM25 rank, not the synthetic -1000.0
        assert!(results[0].rank > -1000.0);
    }
}
