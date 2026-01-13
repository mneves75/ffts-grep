//! Project initialization with gitignore configuration.
//!
//! Provides idempotent initialization of ffts-grep in a project directory.
//!
//! # Design Principles
//!
//! 1. **Idempotent**: Safe to run multiple times (no duplicate entries)
//! 2. **Non-destructive**: Never overwrites existing content
//! 3. **Minimal**: Only adds what's missing
//! 4. **Atomic**: Uses temp file + rename for gitignore updates

use std::collections::HashSet;
use std::fs;
use std::io::Write;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use crate::error::Result;
use crate::fs_utils::sync_parent_dir;
use crate::{DB_NAME, DB_SHM_NAME, DB_TMP_GLOB, DB_WAL_NAME};

/// Required gitignore entries for ffts-grep.
/// Returns a static array with all database file patterns.
#[must_use]
pub const fn gitignore_entries() -> [&'static str; 4] {
    [DB_NAME, DB_SHM_NAME, DB_WAL_NAME, DB_TMP_GLOB]
}

/// Header comment for gitignore section.
const GITIGNORE_HEADER: &str = "# ffts-grep database files (auto-generated)";

#[cfg(windows)]
fn atomic_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let from_wide: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to_wide: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();

    let result = unsafe {
        MoveFileExW(
            from_wide.as_ptr(),
            to_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };

    if result == 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::rename(from, to)
}

/// Result of gitignore update operation.
#[derive(Debug, PartialEq, Eq)]
pub enum GitignoreResult {
    /// Created new .gitignore file with N entries
    Created(usize),
    /// Added N entries to existing file
    Updated(usize),
    /// All entries already present
    AlreadyComplete,
}

/// Result of init operation.
#[derive(Debug)]
pub struct InitResult {
    pub gitignore: GitignoreResult,
    pub database_created: bool,
    pub files_indexed: usize,
}

/// Check if all required gitignore entries are present.
///
/// Returns a list of missing entries (empty if all present).
#[must_use]
pub fn check_gitignore(project_dir: &Path) -> Vec<&'static str> {
    let gitignore_path = project_dir.join(".gitignore");

    let Ok(existing) = fs::read_to_string(&gitignore_path) else {
        return gitignore_entries().to_vec();
    };

    // Parse existing patterns (one per line, ignore comments/blanks)
    let existing_patterns: HashSet<&str> =
        existing.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();

    // Find missing entries
    gitignore_entries().iter().filter(|e| !existing_patterns.contains(*e)).copied().collect()
}

/// Update .gitignore with required entries (idempotent).
///
/// # Algorithm
///
/// 1. Read existing content (empty if file doesn't exist)
/// 2. Parse existing patterns into a set
/// 3. Find which required entries are missing
/// 4. If none missing, return `AlreadyComplete`
/// 5. Append header comment and missing entries
/// 6. Write to temp file, then atomic rename
///
/// # Errors
///
/// Returns error if file operations fail.
pub fn update_gitignore(project_dir: &Path) -> Result<GitignoreResult> {
    let gitignore_path = project_dir.join(".gitignore");

    // Read existing content (empty if file doesn't exist)
    let existing = fs::read_to_string(&gitignore_path).unwrap_or_default();
    let file_existed = !existing.is_empty() || gitignore_path.exists();

    // Parse existing patterns (one per line, ignore comments/blanks)
    let existing_patterns: HashSet<&str> =
        existing.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();

    // Find missing entries
    let missing: Vec<&str> =
        gitignore_entries().iter().filter(|e| !existing_patterns.contains(*e)).copied().collect();

    if missing.is_empty() {
        return Ok(GitignoreResult::AlreadyComplete);
    }

    // Build new content
    let mut new_content = existing.clone();

    // Ensure trailing newline before appending
    if !new_content.is_empty() && !new_content.ends_with('\n') {
        new_content.push('\n');
    }

    // Add comment header if this is first addition (header not already present)
    if !existing.contains(GITIGNORE_HEADER) {
        // Add blank line before header if there's existing content
        if !new_content.is_empty() {
            new_content.push('\n');
        }
        new_content.push_str(GITIGNORE_HEADER);
        new_content.push('\n');
    }

    // Add missing entries
    for entry in &missing {
        new_content.push_str(entry);
        new_content.push('\n');
    }

    // Atomic write: temp file + rename
    let tmp_path = gitignore_path.with_extension("gitignore.tmp");

    // Write to temp file
    let mut file = fs::File::create(&tmp_path)?;
    file.write_all(new_content.as_bytes())?;
    file.flush()?;
    file.sync_all()?;

    drop(file); // Ensure file is closed before rename

    // Atomic rename (Windows requires replace strategy)
    atomic_replace(&tmp_path, &gitignore_path)?;

    // Ensure the rename is durable on filesystems that require directory fsync.
    sync_parent_dir(&gitignore_path)?;

    let count = missing.len();
    if file_existed {
        Ok(GitignoreResult::Updated(count))
    } else {
        Ok(GitignoreResult::Created(count))
    }
}

/// Output init results.
///
/// # Errors
/// Returns `std::io::Error` if writing to the output stream fails.
pub fn output_init_result<W: Write>(
    writer: &mut W,
    result: &InitResult,
    quiet: bool,
) -> std::io::Result<()> {
    if quiet {
        return Ok(());
    }

    writeln!(writer)?;

    // Gitignore status
    match &result.gitignore {
        GitignoreResult::Created(n) => {
            writeln!(writer, "\u{2713} .gitignore: Created with {n} entries")?;
        }
        GitignoreResult::Updated(n) => {
            writeln!(writer, "\u{2713} .gitignore: Added {n} entries")?;
        }
        GitignoreResult::AlreadyComplete => {
            writeln!(writer, "\u{2713} .gitignore: Already configured")?;
        }
    }

    // Database status
    if result.database_created {
        writeln!(
            writer,
            "\u{2713} Database: Created {} ({} files)",
            DB_NAME, result.files_indexed
        )?;
    } else if result.files_indexed > 0 {
        writeln!(writer, "\u{2713} Database: Already exists ({} files)", result.files_indexed)?;
    }

    // Integration hint (generic)
    if result.database_created || matches!(result.gitignore, GitignoreResult::Created(_)) {
        writeln!(writer)?;
        writeln!(writer, "To use with file explorers or editors, run:")?;
        writeln!(writer, "  ffts-grep search <query>")?;
    }

    writeln!(writer)?;

    // Summary
    if result.database_created || !matches!(result.gitignore, GitignoreResult::AlreadyComplete) {
        writeln!(writer, "Initialization complete.")?;
    } else {
        writeln!(writer, "Already initialized.")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_check_gitignore_no_file() {
        let dir = tempdir().unwrap();
        let missing = check_gitignore(dir.path());

        // All entries should be missing
        assert_eq!(missing.len(), gitignore_entries().len());
    }

    #[test]
    fn test_check_gitignore_empty_file() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "").unwrap();

        let missing = check_gitignore(dir.path());
        assert_eq!(missing.len(), gitignore_entries().len());
    }

    #[test]
    fn test_check_gitignore_partial() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), format!("{DB_NAME}\n{DB_NAME}-shm\n")).unwrap();

        let missing = check_gitignore(dir.path());
        assert_eq!(missing.len(), 2);
        assert!(missing.contains(&DB_WAL_NAME));
        assert!(missing.contains(&DB_TMP_GLOB));
    }

    #[test]
    fn test_check_gitignore_complete() {
        let dir = tempdir().unwrap();
        let content = gitignore_entries().join("\n") + "\n";
        fs::write(dir.path().join(".gitignore"), content).unwrap();

        let missing = check_gitignore(dir.path());
        assert!(missing.is_empty());
    }

    #[test]
    fn test_update_gitignore_creates_file() {
        let dir = tempdir().unwrap();

        let result = update_gitignore(dir.path()).unwrap();
        assert_eq!(result, GitignoreResult::Created(4));

        // Verify file contents
        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains(GITIGNORE_HEADER));
        for entry in gitignore_entries() {
            assert!(content.contains(entry));
        }
    }

    #[test]
    fn test_update_gitignore_appends_to_existing() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "node_modules/\n").unwrap();

        let result = update_gitignore(dir.path()).unwrap();
        assert_eq!(result, GitignoreResult::Updated(4));

        // Verify original content preserved
        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains("node_modules/"));
        assert!(content.contains(GITIGNORE_HEADER));
        for entry in gitignore_entries() {
            assert!(content.contains(entry));
        }
    }

    #[test]
    fn test_update_gitignore_idempotent() {
        let dir = tempdir().unwrap();

        // First update
        let result1 = update_gitignore(dir.path()).unwrap();
        assert_eq!(result1, GitignoreResult::Created(4));

        let content1 = fs::read_to_string(dir.path().join(".gitignore")).unwrap();

        // Second update
        let result2 = update_gitignore(dir.path()).unwrap();
        assert_eq!(result2, GitignoreResult::AlreadyComplete);

        // Content should be identical
        let content2 = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert_eq!(content1, content2);
    }

    #[test]
    fn test_update_gitignore_partial_update() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), format!("{DB_NAME}\n")).unwrap();

        let result = update_gitignore(dir.path()).unwrap();
        assert_eq!(result, GitignoreResult::Updated(3)); // Only 3 new entries

        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();

        // Count occurrences of DB_NAME (should only appear once)
        let count = content.matches(DB_NAME).count();
        // The base entry appears, plus 3 more with suffixes/glob
        assert_eq!(count, 4); // base + shm + wal + tmp*
    }

    #[test]
    fn test_update_gitignore_preserves_trailing_newline() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "node_modules/").unwrap(); // No trailing newline

        let result = update_gitignore(dir.path()).unwrap();
        assert_eq!(result, GitignoreResult::Updated(4));

        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        // Should have newline after node_modules/ and before header
        assert!(content.starts_with("node_modules/\n"));
    }

    #[test]
    fn test_update_gitignore_with_comments() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "# My project ignores\nnode_modules/\n").unwrap();

        let result = update_gitignore(dir.path()).unwrap();
        assert_eq!(result, GitignoreResult::Updated(4));

        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains("# My project ignores"));
        assert!(content.contains(GITIGNORE_HEADER));
    }

    #[test]
    fn test_gitignore_result_output() {
        let result = InitResult {
            gitignore: GitignoreResult::Created(4),
            database_created: true,
            files_indexed: 100,
        };

        let mut output = Vec::new();
        output_init_result(&mut output, &result, false).unwrap();
        let output_str = String::from_utf8(output).unwrap();

        assert!(output_str.contains("Created with 4 entries"));
        assert!(output_str.contains("100 files"));
        assert!(output_str.contains(DB_NAME));
        assert!(output_str.contains("ffts-grep search"));
    }

    #[test]
    fn test_gitignore_result_quiet() {
        let result = InitResult {
            gitignore: GitignoreResult::Created(4),
            database_created: true,
            files_indexed: 100,
        };

        let mut output = Vec::new();
        output_init_result(&mut output, &result, true).unwrap();

        // Quiet mode should produce no output
        assert!(output.is_empty());
    }

    #[test]
    fn test_gitignore_entries_match_constants() {
        // Verify that hardcoded strings in gitignore_entries() match the expected
        // DB_NAME + suffix pattern. This guards against divergence.
        use crate::{DB_NAME, DB_SHM_SUFFIX, DB_TMP_GLOB, DB_WAL_SUFFIX};

        let entries = gitignore_entries();

        assert_eq!(entries[0], DB_NAME);
        assert_eq!(entries[1], format!("{DB_NAME}{DB_SHM_SUFFIX}"));
        assert_eq!(entries[2], format!("{DB_NAME}{DB_WAL_SUFFIX}"));
        assert_eq!(entries[3], DB_TMP_GLOB);
    }
}
