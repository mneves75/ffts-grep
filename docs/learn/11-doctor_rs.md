# Chapter 11: doctor.rs - Diagnostics

> "An ounce of prevention is worth a pound of cure." — Benjamin Franklin

## 11.1 What Does This File Do? (In Simple Terms)

The `doctor.rs` file implements a **diagnostic tool** that checks the health of the installation. It runs multiple checks and reports any issues found, similar to how `brew doctor` works on macOS or `flutter doctor` works for Flutter.

### The Medical Checkup Analogy

Think of a routine medical checkup:

| Medical Checkup | This doctor.rs |
|-----------------|----------------|
| Blood pressure test | Database exists check |
| Heart rhythm test | Database readable check |
| Temperature check | Schema complete check |
| Doctor's diagnosis | Overall health summary |
| Prescriptions | Remediation suggestions |

The doctor module doesn't fix problems—it just diagnoses them and tells you what to do.

---

## 11.2 The Doctor Struct

See `doctor.rs:99-118`:

```rust
/// Doctor diagnostic runner.
pub struct Doctor<'a> {
    project_dir: &'a Path,
    verbose: bool,
    checks: Vec<CheckResult>,
    exe_name: String,
}

impl<'a> Doctor<'a> {
    /// Create a new Doctor for the given project directory.
    pub fn new(project_dir: &'a Path, verbose: bool) -> Self {
        let exe_name = std::env::current_exe()
            .ok()
            .and_then(|path| {
                path.file_name().and_then(|name| name.to_str()).map(ToString::to_string)
            })
            .unwrap_or_else(|| "ffts-grep".to_string());

        Self { project_dir, verbose, checks: Vec::with_capacity(10), exe_name }
    }
}
```

---

## 11.3 Severity Levels

See `doctor.rs:27-34`:

```rust
/// Check severity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Pass,      // Check succeeded
    Info,      // Informational
    Warning,   // Non-critical issue
    Error,     // Critical problem
}
```

| Severity | Meaning | Exit Code |
|----------|---------|-----------|
| `Pass` | Everything is fine | 0 |
| `Info` | FYI, not a problem | 0 |
| `Warning` | Should fix eventually | 1 |
| `Error` | Must fix now | 2 |

---

## 11.4 The 10 Diagnostic Checks

See `doctor.rs:121-134`:

```rust
/// Run all diagnostic checks.
pub fn run(&mut self) -> DoctorSummary {
    self.check_database_exists();      // Check 1
    self.check_database_readable();    // Check 2
    self.check_application_id();       // Check 3
    self.check_schema_complete();      // Check 4
    self.check_fts_integrity();        // Check 5
    self.check_journal_mode();         // Check 6
    self.check_file_count();           // Check 7
    self.check_gitignore();            // Check 8
    self.check_binary_available();     // Check 9
    self.check_orphan_wal_files();     // Check 10

    DoctorSummary::from_checks(&self.checks)
}
```

| # | Check | What It Verifies |
|---|-------|------------------|
| 1 | Database exists | `.ffts-index.db` is present |
| 2 | Database readable | Can open the database file |
| 3 | Application ID | Correct ID (`0xA17E_6D42`) |
| 4 | Schema complete | All tables, triggers, indexes exist |
| 5 | FTS5 integrity | FTS5 virtual table works |
| 6 | Journal mode | WAL mode is enabled |
| 7 | File count | Has indexed files |
| 8 | Gitignore | DB files are ignored |
| 9 | Binary available | Executable exists |
| 10 | Orphan WAL files | No WAL files without main DB |

---

## 11.5 Example Check: Database Exists

See `doctor.rs:278-305`:

```rust
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
```

### CheckResult Structure

```rust
pub struct CheckResult {
    pub name: &'static str,              // Check name
    pub status: Severity,                // Result
    pub message: String,                 // Human-readable message
    pub remediation: Option<String>,     // How to fix
    pub details: Option<serde_json::Value>, // Technical details
}
```

---

## 11.6 Output Formatting

See `doctor.rs:146-214`:

### Compact Output

```
✓ Database: .ffts-index.db (1.2 MB)
✓ Schema: 2 tables, 3 triggers, 3 indexes
✓ Journal mode: WAL
! Gitignore: 2 entries missing
  -> Run: ffts-grep init
```

### Verbose Output

```
[1/10] Database exists
       PASS ✓ Database: .ffts-index.db (1.2 MB)
       path: /path/to/project/.ffts-index.db
       size_bytes: 1258291

[2/10] Database readable
       PASS ✓ Database readable (SQLITE_OPEN_READ_ONLY)
```

### JSON Output

```json
{
  "version": "0.11.4",
  "project_dir": "/path/to/project",
  "checks": [...],
  "summary": {
    "pass": 8,
    "info": 0,
    "warn": 1,
    "fail": 1
  },
  "exit_code": 2
}
```

---

## 11.7 Chapter Summary

| Concept | What We Learned |
|---------|-----------------|
| Doctor pattern | Multi-check diagnostic system |
| Severity levels | Pass/Info/Warning/Error |
| Remediation | Every failure has a fix suggestion |
| Multiple outputs | Compact/verbose/JSON formats |
| Exit codes | Integration with CI/CD |

---

## Exercises

### Exercise 11.1: Run Doctor

Run the doctor and observe the output:

```bash
ffts-grep doctor
ffts-grep doctor --verbose
ffts-grep doctor --format json
```

**Deliverable:** Compare the three output formats.

### Exercise 11.2: Intentionally Break Something

Temporarily rename the database file, then run doctor:

```bash
mv .ffts-index.db .ffts-index.db.backup
ffts-grep doctor
```

**Deliverable:** Show the diagnostic output.

### Exercise 11.3: Add a New Check

Add a check for SQLite version compatibility.

**Deliverable:** Show the code changes needed.

### Exercise 11.4: Design a Check

Design a check that verifies the database is not corrupted.

**Deliverable:** Write pseudocode for the check.

---

**Next Chapter**: [Chapter 12: health.rs - Health Checking](12-health_rs.md)
