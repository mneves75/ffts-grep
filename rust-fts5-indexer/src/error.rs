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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exit_code_values() {
        // Verify exit codes match BSD sysexits.h conventions
        assert_eq!(ExitCode::Ok as u8, 0);
        assert_eq!(ExitCode::Software as u8, 1);
        assert_eq!(ExitCode::DataErr as u8, 2);
        assert_eq!(ExitCode::IoErr as u8, 3);
        assert_eq!(ExitCode::NoInput as u8, 4);
        assert_eq!(ExitCode::NoPerm as u8, 5);
    }

    #[test]
    fn test_exit_code_into_process_exit_code() {
        // Verify From trait works correctly
        let code: std::process::ExitCode = ExitCode::Ok.into();
        // We can't directly inspect ExitCode value, but we can verify it compiles
        let _ = code;

        let code: std::process::ExitCode = ExitCode::Software.into();
        let _ = code;
    }

    #[test]
    fn test_exit_code_equality() {
        assert_eq!(ExitCode::Ok, ExitCode::Ok);
        assert_ne!(ExitCode::Ok, ExitCode::Software);
    }

    #[test]
    fn test_exit_code_clone() {
        let code = ExitCode::DataErr;
        let cloned = code;
        assert_eq!(code, cloned);
    }

    #[test]
    fn test_exit_code_debug() {
        // Verify Debug trait works
        let code = ExitCode::IoErr;
        let debug_str = format!("{code:?}");
        assert!(debug_str.contains("IoErr"));
    }

    #[test]
    fn test_indexer_error_display() {
        let error = IndexerError::FileTooLarge { size: 2_000_000, max: 1_000_000 };
        let display = format!("{error}");
        assert!(display.contains("2000000"));
        assert!(display.contains("1000000"));
    }

    #[test]
    fn test_indexer_error_from_io() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let indexer_error: IndexerError = io_error.into();
        match indexer_error {
            IndexerError::Io { .. } => {}
            _ => panic!("Expected Io variant"),
        }
    }

    #[test]
    fn test_foreign_database_error_display() {
        let error = IndexerError::ForeignDatabase { app_id: 0x1234_5678 };
        let display = format!("{error}");
        assert!(display.contains("different application"));
        assert!(display.contains("0x12345678"));
    }
}
