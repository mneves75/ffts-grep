//! Diagnostic checks for installation verification.
//!
//! Following patterns from `brew doctor`, `flutter doctor`, `npm doctor`.
//!
//! # Design Principles
//!
//! 1. **Non-destructive**: Opens database in read-only mode when possible (FTS5 integrity requires write access)
//! 2. **Verbose option**: `-v` / `--verbose` for detailed diagnostics
//! 3. **JSON output**: `--format json` for CI/automation integration
//! 4. **Actionable**: Every failure includes specific fix command
//! 5. **Fast**: No file system walking for basic checks

use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::Path;

use crate::cli::OutputFormat;
use crate::constants::EXPECTED_APPLICATION_ID;
use crate::db::Database;
use crate::init;
use crate::{DB_NAME, DB_SHM_SUFFIX, DB_WAL_SUFFIX};

/// Check severity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Pass,
    Info,
    Warning,
    Error,
}

/// Result of a single diagnostic check.
#[derive(Debug, Serialize)]
pub struct CheckResult {
    pub name: &'static str,
    pub status: Severity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// Summary of all doctor checks.
#[derive(Debug, Serialize)]
pub struct DoctorSummary {
    pub pass: usize,
    pub info: usize,
    pub warn: usize,
    pub fail: usize,
}

impl DoctorSummary {
    /// Create summary from checks.
    #[must_use]
    pub fn from_checks(checks: &[CheckResult]) -> Self {
        let mut summary = Self { pass: 0, info: 0, warn: 0, fail: 0 };

        for check in checks {
            match check.status {
                Severity::Pass => summary.pass += 1,
                Severity::Info => summary.info += 1,
                Severity::Warning => summary.warn += 1,
                Severity::Error => summary.fail += 1,
            }
        }

        summary
    }

    /// Returns true if any errors were found.
    #[must_use]
    pub const fn has_errors(&self) -> bool {
        self.fail > 0
    }

    /// Returns true if any warnings were found.
    #[must_use]
    pub const fn has_warnings(&self) -> bool {
        self.warn > 0
    }
}

/// Doctor diagnostic output (for JSON format).
#[derive(Debug, Serialize)]
pub struct DoctorOutput {
    pub version: &'static str,
    pub project_dir: String,
    pub checks: Vec<CheckResult>,
    pub summary: DoctorSummary,
    pub exit_code: u8,
}

/// Doctor diagnostic runner.
pub struct Doctor<'a> {
    project_dir: &'a Path,
    verbose: bool,
    checks: Vec<CheckResult>,
    exe_name: String,
}

impl<'a> Doctor<'a> {
    /// Create a new Doctor for the given project directory.
    #[must_use]
    pub fn new(project_dir: &'a Path, verbose: bool) -> Self {
        let exe_name = std::env::current_exe()
            .ok()
            .and_then(|path| {
                path.file_name().and_then(|name| name.to_str()).map(ToString::to_string)
            })
            .unwrap_or_else(|| "ffts-grep".to_string());

        Self { project_dir, verbose, checks: Vec::with_capacity(10), exe_name }
    }

    /// Run all diagnostic checks.
    pub fn run(&mut self) -> DoctorSummary {
        self.check_database_exists();
        self.check_database_readable();
        self.check_application_id();
        self.check_schema_complete();
        self.check_fts_integrity();
        self.check_journal_mode();
        self.check_file_count();
        self.check_gitignore();
        self.check_binary_available();
        self.check_orphan_wal_files();

        DoctorSummary::from_checks(&self.checks)
    }

    /// Get the checks after running.
    #[must_use]
    pub fn checks(&self) -> &[CheckResult] {
        &self.checks
    }

    /// Output results in plain format.
    ///
    /// # Errors
    /// Returns `std::io::Error` if writing to the output stream fails.
    pub fn output_plain<W: Write>(
        &self,
        writer: &mut W,
        summary: &DoctorSummary,
    ) -> std::io::Result<()> {
        // Header line
        writeln!(writer)?;

        let check_count = self.checks.len();

        for (i, check) in self.checks.iter().enumerate() {
            if self.verbose {
                // Verbose format: [N/10] Check name
                writeln!(writer, "[{}/{}] {}", i + 1, check_count, check.name)?;

                // Details if present
                if let Some(details) = &check.details {
                    if let Some(obj) = details.as_object() {
                        for (key, value) in obj {
                            writeln!(writer, "       {key}: {value}")?;
                        }
                    }
                }

                // Status
                let status_symbol = match check.status {
                    Severity::Pass => "PASS",
                    Severity::Info => "INFO",
                    Severity::Warning => "WARN",
                    Severity::Error => "FAIL",
                };
                writeln!(writer, "       {} {}", status_symbol, check.message)?;

                // Remediation if present
                if let Some(remediation) = &check.remediation {
                    writeln!(writer, "       -> {remediation}")?;
                }

                writeln!(writer)?;
            } else {
                // Compact format
                let symbol = match check.status {
                    Severity::Pass => '\u{2713}', // ✓
                    Severity::Info => '\u{2139}', // ℹ
                    Severity::Warning => '!',
                    Severity::Error => '\u{2717}', // ✗
                };

                writeln!(writer, "{} {}", symbol, check.message)?;

                if let Some(remediation) = &check.remediation {
                    writeln!(writer, "  -> {remediation}")?;
                }
            }
        }

        // Summary
        writeln!(writer)?;
        let issue_count = summary.fail + summary.warn;
        if issue_count == 0 {
            writeln!(writer, "All checks passed.")?;
        } else if issue_count == 1 {
            writeln!(writer, "1 issue found.")?;
        } else {
            writeln!(writer, "{issue_count} issues found.")?;
        }

        Ok(())
    }

    /// Output results in JSON format.
    ///
    /// # Errors
    /// Returns `std::io::Error` if:
    /// - JSON serialization fails (converted from `serde_json::Error`)
    /// - Writing to the output stream fails
    pub fn output_json<W: Write>(
        &self,
        writer: &mut W,
        summary: &DoctorSummary,
    ) -> std::io::Result<()> {
        // Exit code follows BSD sysexits(3) convention: 0=OK, 1=WARNING, 2=ERROR
        // Not a simple bool-to-int conversion - three distinct states
        #[allow(clippy::bool_to_int_with_if)]
        let exit_code = if summary.has_errors() {
            2 // DATAERR
        } else if summary.has_warnings() {
            1 // SOFTWARE
        } else {
            0 // OK
        };

        let output = DoctorOutput {
            version: env!("CARGO_PKG_VERSION"),
            project_dir: self.project_dir.display().to_string(),
            checks: self.checks.clone(),
            summary: DoctorSummary::from_checks(&self.checks),
            exit_code,
        };

        let json = serde_json::to_string_pretty(&output).map_err(std::io::Error::other)?;
        writeln!(writer, "{json}")?;

        Ok(())
    }

    /// Output results based on format.
    ///
    /// # Errors
    /// Returns `std::io::Error` if the underlying output method fails.
    /// See [`output_plain`](Self::output_plain) and [`output_json`](Self::output_json).
    pub fn output<W: Write>(
        &self,
        writer: &mut W,
        format: OutputFormat,
        summary: &DoctorSummary,
    ) -> std::io::Result<()> {
        match format {
            OutputFormat::Json => self.output_json(writer, summary),
            OutputFormat::Plain => self.output_plain(writer, summary),
        }
    }

    // -------------------------------------------------------------------------
    // Individual checks
    // -------------------------------------------------------------------------

    fn db_path(&self) -> std::path::PathBuf {
        self.project_dir.join(DB_NAME)
    }

    /// Check 1: Database exists
    fn check_database_exists(&mut self) {
        let db_path = self.db_path();

        if db_path.exists() {
            let size_bytes = fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);

            self.checks.push(CheckResult {
                name: "Database exists",
                status: Severity::Pass,
                message: format!("Database: {} ({})", DB_NAME, format_bytes(size_bytes)),
                remediation: None,
                details: Some(serde_json::json!({
                    "path": db_path.display().to_string(),
                    "size_bytes": size_bytes,
                })),
            });
        } else {
            self.checks.push(CheckResult {
                name: "Database exists",
                status: Severity::Error,
                message: format!("Database not found: {DB_NAME}"),
                remediation: Some("Run: ffts-grep init".to_string()),
                details: Some(serde_json::json!({
                    "path": db_path.display().to_string(),
                })),
            });
        }
    }

    /// Check 2: Database readable
    fn check_database_readable(&mut self) {
        let db_path = self.db_path();

        if !db_path.exists() {
            // Skip if database doesn't exist (already reported)
            return;
        }

        match Database::open_readonly(&db_path) {
            Ok(_) => {
                self.checks.push(CheckResult {
                    name: "Database readable",
                    status: Severity::Pass,
                    message: "Database readable (SQLITE_OPEN_READONLY)".to_string(),
                    remediation: None,
                    details: None,
                });
            }
            Err(e) => {
                self.checks.push(CheckResult {
                    name: "Database readable",
                    status: Severity::Error,
                    message: format!("Cannot read database: {e}"),
                    remediation: Some("Check file permissions".to_string()),
                    details: None,
                });
            }
        }
    }

    /// Check 3: Application ID
    fn check_application_id(&mut self) {
        let db_path = self.db_path();

        if !db_path.exists() {
            return;
        }

        let Ok(db) = Database::open_readonly(&db_path) else { return };

        match db.get_application_id() {
            Some(id) if id == EXPECTED_APPLICATION_ID => {
                self.checks.push(CheckResult {
                    name: "Application ID",
                    status: Severity::Pass,
                    message: format!("Application ID: 0x{id:08X}"),
                    remediation: None,
                    details: Some(serde_json::json!({
                        "expected": format!("0x{:08X}", EXPECTED_APPLICATION_ID),
                        "actual": format!("0x{:08X}", id),
                    })),
                });
            }
            Some(id) => {
                self.checks.push(CheckResult {
                    name: "Application ID",
                    status: Severity::Error,
                    message: format!(
                        "Wrong application ID: 0x{id:08X} (expected 0x{EXPECTED_APPLICATION_ID:08X})"
                    ),
                    remediation: Some(
                        "This database was not created by ffts-grep".to_string(),
                    ),
                    details: Some(serde_json::json!({
                        "expected": format!("0x{:08X}", EXPECTED_APPLICATION_ID),
                        "actual": format!("0x{:08X}", id),
                    })),
                });
            }
            None => {
                self.checks.push(CheckResult {
                    name: "Application ID",
                    status: Severity::Error,
                    message: "Cannot read application ID".to_string(),
                    remediation: Some("Database may be corrupted".to_string()),
                    details: None,
                });
            }
        }
    }

    /// Check 4: Schema complete
    fn check_schema_complete(&mut self) {
        let db_path = self.db_path();

        if !db_path.exists() {
            return;
        }

        let Ok(db) = Database::open_readonly(&db_path) else { return };

        let schema = db.check_schema();

        if schema.is_complete() {
            self.checks.push(CheckResult {
                name: "Schema complete",
                status: Severity::Pass,
                message: format!(
                    "Schema: {} tables, {} triggers, {} indexes",
                    schema.table_count(),
                    schema.trigger_count(),
                    schema.index_count()
                ),
                remediation: None,
                details: Some(serde_json::json!({
                    "tables": schema.table_count(),
                    "triggers": schema.trigger_count(),
                    "indexes": schema.index_count(),
                })),
            });
        } else {
            let missing = schema.missing_objects();
            self.checks.push(CheckResult {
                name: "Schema complete",
                status: Severity::Error,
                message: format!("Schema incomplete: {} objects missing", missing.len()),
                remediation: Some("Run: ffts-grep index --reindex".to_string()),
                details: Some(serde_json::json!({
                    "missing": missing,
                })),
            });
        }
    }

    /// Check 5: FTS5 integrity
    fn check_fts_integrity(&mut self) {
        let db_path = self.db_path();

        if !db_path.exists() {
            return;
        }

        // Need write access for integrity-check (it's an INSERT command)
        // Open with regular mode but don't init schema
        let Ok(db) = Database::open(&db_path, &crate::db::PragmaConfig::default()) else { return };

        // Check schema first to avoid errors on missing FTS table
        let schema = db.check_schema();
        if !schema.has_fts_table {
            return;
        }

        if db.check_fts_integrity() {
            self.checks.push(CheckResult {
                name: "FTS5 integrity",
                status: Severity::Pass,
                message: "FTS5 integrity: OK".to_string(),
                remediation: None,
                details: None,
            });
        } else {
            self.checks.push(CheckResult {
                name: "FTS5 integrity",
                status: Severity::Error,
                message: "FTS5 integrity check failed".to_string(),
                remediation: Some(format!("Run: {} index --reindex", self.exe_name)),
                details: None,
            });
        }
    }

    /// Check 6: Journal mode
    fn check_journal_mode(&mut self) {
        let db_path = self.db_path();

        if !db_path.exists() {
            return;
        }

        let Ok(db) = Database::open_readonly(&db_path) else { return };

        match db.get_journal_mode() {
            Some(mode) if mode.to_lowercase() == "wal" => {
                self.checks.push(CheckResult {
                    name: "Journal mode",
                    status: Severity::Pass,
                    message: "Journal mode: WAL".to_string(),
                    remediation: None,
                    details: Some(serde_json::json!({
                        "mode": mode,
                    })),
                });
            }
            Some(mode) => {
                self.checks.push(CheckResult {
                    name: "Journal mode",
                    status: Severity::Warning,
                    message: format!("Journal mode: {mode} (expected WAL)"),
                    remediation: Some(
                        "Database may have been copied without WAL files".to_string(),
                    ),
                    details: Some(serde_json::json!({
                        "mode": mode,
                        "expected": "wal",
                    })),
                });
            }
            None => {
                self.checks.push(CheckResult {
                    name: "Journal mode",
                    status: Severity::Warning,
                    message: "Cannot read journal mode".to_string(),
                    remediation: None,
                    details: None,
                });
            }
        }
    }

    /// Check 7: File count
    fn check_file_count(&mut self) {
        let db_path = self.db_path();

        if !db_path.exists() {
            return;
        }

        let Ok(db) = Database::open_readonly(&db_path) else { return };

        match db.get_file_count() {
            Ok(0) => {
                self.checks.push(CheckResult {
                    name: "File count",
                    status: Severity::Warning,
                    message: "Database is empty (0 files indexed)".to_string(),
                    remediation: Some("Run: ffts-grep index".to_string()),
                    details: Some(serde_json::json!({
                        "count": 0,
                    })),
                });
            }
            Ok(count) => {
                self.checks.push(CheckResult {
                    name: "File count",
                    status: Severity::Pass,
                    message: format!("Indexed files: {count}"),
                    remediation: None,
                    details: Some(serde_json::json!({
                        "count": count,
                    })),
                });
            }
            Err(_) => {
                self.checks.push(CheckResult {
                    name: "File count",
                    status: Severity::Warning,
                    message: "Cannot read file count".to_string(),
                    remediation: None,
                    details: None,
                });
            }
        }
    }

    /// Check 8: Gitignore entries
    fn check_gitignore(&mut self) {
        let missing = init::check_gitignore(self.project_dir);

        if missing.is_empty() {
            self.checks.push(CheckResult {
                name: "Gitignore",
                status: Severity::Pass,
                message: "Gitignore: All entries present".to_string(),
                remediation: None,
                details: None,
            });
        } else {
            self.checks.push(CheckResult {
                name: "Gitignore",
                status: Severity::Warning,
                message: format!("Gitignore: {} entries missing", missing.len()),
                remediation: Some("Run: ffts-grep init".to_string()),
                details: Some(serde_json::json!({
                    "missing": missing,
                })),
            });
        }
    }

    /// Check 9: Binary availability
    fn check_binary_available(&mut self) {
        let Ok(exe_path) = std::env::current_exe() else {
            self.checks.push(CheckResult {
                name: "Binary availability",
                status: Severity::Warning,
                message: "Cannot determine executable path".to_string(),
                remediation: None,
                details: None,
            });
            return;
        };

        let exe_name = exe_path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown");
        let exe_dir =
            exe_path.parent().map_or_else(|| ".".to_string(), |p| p.display().to_string());

        self.checks.push(CheckResult {
            name: "Binary availability",
            status: Severity::Pass,
            message: format!("{exe_name} is available at: {exe_dir}"),
            remediation: None,
            details: Some(serde_json::json!({
                "path": exe_path.display().to_string(),
            })),
        });
    }

    /// Check 10: Orphan WAL files
    fn check_orphan_wal_files(&mut self) {
        let db_path = self.db_path();
        // Construct correct WAL/SHM filenames by appending suffix to DB_NAME
        let shm_filename = format!("{DB_NAME}{DB_SHM_SUFFIX}");
        let wal_filename = format!("{DB_NAME}{DB_WAL_SUFFIX}");
        let shm_path = self.project_dir.join(&shm_filename);
        let wal_path = self.project_dir.join(&wal_filename);

        let db_exists = db_path.exists();
        let shm_exists = shm_path.exists();
        let wal_exists = wal_path.exists();

        if !db_exists && (shm_exists || wal_exists) {
            let mut orphans = Vec::new();
            if shm_exists {
                orphans.push(format!("{DB_NAME}{DB_SHM_SUFFIX}"));
            }
            if wal_exists {
                orphans.push(format!("{DB_NAME}{DB_WAL_SUFFIX}"));
            }

            self.checks.push(CheckResult {
                name: "Orphan WAL files",
                status: Severity::Warning,
                message: format!(
                    "Found {} orphan WAL file(s) without main database",
                    orphans.len()
                ),
                remediation: Some(format!("Delete orphan files: {orphans:?}")),
                details: Some(serde_json::json!({
                    "orphans": orphans,
                })),
            });
        } else {
            self.checks.push(CheckResult {
                name: "Orphan WAL files",
                status: Severity::Pass,
                message: "No orphan WAL files".to_string(),
                remediation: None,
                details: None,
            });
        }
    }
}

/// Format bytes as human-readable string.
///
/// Safety: u64→f64 casts for display purposes only
/// Precision loss is acceptable for human-readable output (e.g., 1.2 GB vs exact bytes)
#[allow(clippy::cast_precision_loss)]
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} bytes")
    }
}

// Allow cloning CheckResult for DoctorOutput
impl Clone for CheckResult {
    fn clone(&self) -> Self {
        Self {
            name: self.name,
            status: self.status,
            message: self.message.clone(),
            remediation: self.remediation.clone(),
            details: self.details.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_severity_serialize() {
        assert_eq!(serde_json::to_string(&Severity::Pass).unwrap(), "\"pass\"");
        assert_eq!(serde_json::to_string(&Severity::Error).unwrap(), "\"error\"");
    }

    #[test]
    fn test_doctor_summary_from_checks() {
        let checks = vec![
            CheckResult {
                name: "test1",
                status: Severity::Pass,
                message: "ok".to_string(),
                remediation: None,
                details: None,
            },
            CheckResult {
                name: "test2",
                status: Severity::Warning,
                message: "warn".to_string(),
                remediation: None,
                details: None,
            },
            CheckResult {
                name: "test3",
                status: Severity::Error,
                message: "err".to_string(),
                remediation: None,
                details: None,
            },
        ];

        let summary = DoctorSummary::from_checks(&checks);
        assert_eq!(summary.pass, 1);
        assert_eq!(summary.warn, 1);
        assert_eq!(summary.fail, 1);
        assert!(summary.has_errors());
        assert!(summary.has_warnings());
    }

    #[test]
    fn test_doctor_no_database() {
        let dir = tempdir().unwrap();
        let mut doctor = Doctor::new(dir.path(), false);
        let summary = doctor.run();

        // Should have error for missing database
        assert!(summary.has_errors());
        assert!(
            doctor
                .checks()
                .iter()
                .any(|c| c.name == "Database exists" && c.status == Severity::Error)
        );
    }

    #[test]
    fn test_doctor_healthy_database() {
        use crate::DB_NAME;
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(DB_NAME);

        // Create a valid database
        let db = Database::open(&db_path, &crate::db::PragmaConfig::default()).unwrap();
        db.init_schema().unwrap();
        db.upsert_file("test.rs", "content", 0, 7).unwrap();
        drop(db);

        let mut doctor = Doctor::new(dir.path(), false);
        let summary = doctor.run();

        // Should not have errors (may have warnings for gitignore/settings)
        assert!(!summary.has_errors());
        assert!(
            doctor
                .checks()
                .iter()
                .any(|c| c.name == "Database exists" && c.status == Severity::Pass)
        );
        assert!(
            doctor
                .checks()
                .iter()
                .any(|c| c.name == "Schema complete" && c.status == Severity::Pass)
        );
    }

    #[test]
    fn test_doctor_verbose_output() {
        let dir = tempdir().unwrap();
        let mut doctor = Doctor::new(dir.path(), true);
        let summary = doctor.run();

        let mut output = Vec::new();
        doctor.output_plain(&mut output, &summary).unwrap();
        let output_str = String::from_utf8(output).unwrap();

        // Verbose output should contain [N/10] format
        assert!(output_str.contains("[1/"));
    }

    #[test]
    fn test_doctor_json_output() {
        let dir = tempdir().unwrap();
        let mut doctor = Doctor::new(dir.path(), false);
        let summary = doctor.run();

        let mut output = Vec::new();
        doctor.output_json(&mut output, &summary).unwrap();
        let output_str = String::from_utf8(output).unwrap();

        // Should be valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&output_str).unwrap();
        assert!(parsed.get("version").is_some());
        assert!(parsed.get("checks").is_some());
        assert!(parsed.get("summary").is_some());
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 bytes");
        assert_eq!(format_bytes(512), "512 bytes");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GB");
    }

    #[test]
    fn test_check_orphan_wal_files() {
        use crate::{DB_NAME, DB_WAL_SUFFIX};
        let dir = tempdir().unwrap();

        // Create orphan WAL file without main database
        let wal_filename = format!("{DB_NAME}{DB_WAL_SUFFIX}");
        let wal_path = dir.path().join(&wal_filename);
        fs::write(&wal_path, "orphan").unwrap();

        let mut doctor = Doctor::new(dir.path(), false);
        doctor.check_orphan_wal_files();

        let orphan_check = doctor.checks().iter().find(|c| c.name == "Orphan WAL files").unwrap();
        assert_eq!(orphan_check.status, Severity::Warning);
    }
}
