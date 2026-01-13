use clap::{Parser, Subcommand};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

#[derive(Parser)]
#[command(name = "release-tools")]
#[command(about = "Release tooling for ffts-grep", long_about = None)]
struct Args {
    #[command(subcommand)]
    command: ReleaseCommand,
}

#[derive(Subcommand)]
enum ReleaseCommand {
    /// Verify README badge version matches Cargo.toml version.
    CheckVersion,

    /// Print release notes extracted from a changelog section.
    ReleaseNotes {
        /// Path to changelog (defaults to repo CHANGELOG.md).
        #[arg(long)]
        changelog: Option<PathBuf>,

        /// Version to extract (e.g., 0.10). Defaults to latest non-Unreleased.
        #[arg(long)]
        version: Option<String>,
    },

    /// Print a release checklist; optionally verify repository state.
    Checklist {
        /// Version heading to verify in changelog (e.g., 0.10). Defaults to Cargo version (major.minor).
        #[arg(long)]
        version: Option<String>,

        /// Run automated checks (clean git, version match, changelog entry).
        #[arg(long)]
        verify: bool,
    },
}

fn main() -> ExitCode {
    let args = Args::parse();

    match args.command {
        ReleaseCommand::CheckVersion => match check_version_consistency() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("{err}");
                ExitCode::FAILURE
            }
        },
        ReleaseCommand::ReleaseNotes { changelog, version } => {
            match release_notes(changelog.as_deref(), version.as_deref()) {
                Ok(notes) => {
                    println!("{notes}");
                    ExitCode::SUCCESS
                }
                Err(err) => {
                    eprintln!("{err}");
                    ExitCode::FAILURE
                }
            }
        }
        ReleaseCommand::Checklist { version, verify } => {
            let version = version.or_else(|| cargo_version_major_minor().ok());
            print_checklist(version.as_deref());
            if verify {
                if let Err(err) = run_release_checks(version.as_deref()) {
                    eprintln!("{err}");
                    return ExitCode::FAILURE;
                }
            }
            ExitCode::SUCCESS
        }
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn cargo_manifest_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
}

fn readme_path() -> PathBuf {
    repo_root().join("README.md")
}

fn default_changelog_path() -> PathBuf {
    repo_root().join("CHANGELOG.md")
}

fn read_to_string(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|err| format!("Failed to read {}: {err}", path.display()))
}

fn parse_cargo_version(contents: &str) -> Option<String> {
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("version =") {
            let value = trimmed.split('=').nth(1)?.trim();
            return value.strip_prefix('"')?.strip_suffix('"').map(ToString::to_string);
        }
    }
    None
}

fn parse_readme_badge_version(contents: &str) -> Option<String> {
    let needle = "badge/version-";
    let start = contents.find(needle)? + needle.len();
    let rest = &contents[start..];
    let end = rest.find("-blue")?;
    Some(rest[..end].to_string())
}

fn versions_match(cargo_version: &str, badge_version: &str) -> bool {
    cargo_version == badge_version || cargo_version.starts_with(&format!("{badge_version}."))
}

fn cargo_version_major_minor() -> Result<String, String> {
    let cargo_contents = read_to_string(&cargo_manifest_path())?;
    let cargo_version = parse_cargo_version(&cargo_contents)
        .ok_or_else(|| "Could not parse version from Cargo.toml".to_string())?;

    let mut parts = cargo_version.split('.');
    let major = parts.next().unwrap_or("0");
    let minor = parts.next().unwrap_or("0");
    Ok(format!("{major}.{minor}"))
}

fn check_version_consistency() -> Result<(), String> {
    let cargo_contents = read_to_string(&cargo_manifest_path())?;
    let readme_contents = read_to_string(&readme_path())?;

    let cargo_version = parse_cargo_version(&cargo_contents)
        .ok_or_else(|| "Could not parse version from Cargo.toml".to_string())?;
    let badge_version = parse_readme_badge_version(&readme_contents)
        .ok_or_else(|| "Could not parse version badge from README".to_string())?;

    if versions_match(&cargo_version, &badge_version) {
        println!(
            "Version check OK: Cargo.toml {cargo_version} matches README badge {badge_version}"
        );
        Ok(())
    } else {
        Err(format!("Version mismatch: Cargo.toml {cargo_version} != README badge {badge_version}"))
    }
}

fn latest_version(contents: &str) -> Option<String> {
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## [") {
            let remainder = trimmed.strip_prefix("## [")?;
            if let Some(end) = remainder.find(']') {
                let version = &remainder[..end];
                if version != "Unreleased" {
                    return Some(version.to_string());
                }
            }
        }
    }
    None
}

fn extract_section(contents: &str, version: &str) -> Option<String> {
    let header = format!("## [{version}]");
    let start = contents.find(&header)?;
    let rest = &contents[start + header.len()..];
    let end = rest.find("\n## [").map(|idx| start + header.len() + idx).unwrap_or(contents.len());

    Some(contents[start..end].trim_end().to_string())
}

fn release_notes(changelog: Option<&Path>, version: Option<&str>) -> Result<String, String> {
    let changelog_path = changelog.map_or_else(default_changelog_path, Path::to_path_buf);
    let contents = read_to_string(&changelog_path)?;

    let version = match version {
        Some(v) => v.to_string(),
        None => latest_version(&contents)
            .ok_or_else(|| "Could not determine latest release in changelog".to_string())?,
    };

    extract_section(&contents, &version)
        .ok_or_else(|| format!("Changelog section for {version} not found"))
}

fn print_checklist(version: Option<&str>) {
    println!("Release checklist");
    println!("- Ensure working tree is clean");
    println!("- Run full test suite: cargo fmt, cargo test, cargo clippy");
    println!("- Verify README badge matches Cargo.toml version");
    if let Some(version) = version {
        println!("- Confirm changelog has section for {version}");
    }
    println!("- Generate release notes from changelog");
    println!("- Tag release and push tags");
}

fn run_release_checks(version: Option<&str>) -> Result<(), String> {
    ensure_git_clean()?;
    check_version_consistency()?;

    let changelog_contents = read_to_string(&default_changelog_path())?;
    let version = match version {
        Some(v) => v.to_string(),
        None => latest_version(&changelog_contents)
            .ok_or_else(|| "Could not determine changelog version".to_string())?,
    };

    if extract_section(&changelog_contents, &version).is_none() {
        return Err(format!("Changelog section for {version} not found"));
    }

    Ok(())
}

fn ensure_git_clean() -> Result<(), String> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_root())
        .output()
        .map_err(|err| format!("Failed to run git status: {err}"))?;

    if !output.status.success() {
        return Err("git status failed".to_string());
    }

    if output.stdout.is_empty() { Ok(()) } else { Err("Working tree is not clean".to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_versions_match() {
        assert!(versions_match("0.10.0", "0.10"));
        assert!(versions_match("0.10", "0.10"));
        assert!(!versions_match("0.11.0", "0.10"));
    }
}
