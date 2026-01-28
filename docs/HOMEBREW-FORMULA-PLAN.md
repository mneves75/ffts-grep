# Plan: Add ffts-grep Formula to Existing Homebrew Tap

## Overview

Add ffts-grep formula to existing tap `mneves75/homebrew-tap` alongside the existing `healthsync` formula.

## Production Standards Applied

Following Carmack/Steinberger standards and 2025+ Homebrew best practices:

| Standard | Implementation |
|----------|----------------|
| `livecheck` block | Automated version tracking via GitHub releases |
| `--locked` builds | `std_cargo_args` includes `--locked` for reproducibility |
| Functional tests | Test actual search behavior, not just `--version` |
| Subdirectory build | `cd "rust-fts5-indexer"` block for cargo workspace |
| Apache-2.0 license | Explicit SPDX identifier |

## Current Tap Structure

```
homebrew-tap/
├── Formula/
│   └── healthsync.rb   # Existing formula
├── README.md
└── SETUP.md
```

## Target Structure

```
homebrew-tap/
├── Formula/
│   ├── healthsync.rb   # Existing
│   └── ffts-grep.rb    # NEW
├── README.md           # Update to list both formulas
└── SETUP.md
```

## Implementation

Pick a release version and use it consistently below (example: `0.11.4`).

### Step 1: Calculate SHA256

```bash
curl -sL https://github.com/mneves75/ffts-grep/archive/refs/tags/v<VERSION>.tar.gz | shasum -a 256
```

### Step 2: Create Formula File

**File: `Formula/ffts-grep.rb`**

```ruby
class FftsGrep < Formula
  desc "Fast full-text search file indexer using SQLite FTS5"
  homepage "https://github.com/mneves75/ffts-grep"
  url "https://github.com/mneves75/ffts-grep/archive/refs/tags/v<VERSION>.tar.gz"
  sha256 "SHA256_HASH_HERE"
  license "Apache-2.0"
  head "https://github.com/mneves75/ffts-grep.git", branch: "master"

  livecheck do
    url :stable
    strategy :github_latest
  end

  depends_on "rust" => :build

  def install
    cd "rust-fts5-indexer" do
      system "cargo", "install", *std_cargo_args
    end
  end

  test do
    # Test version output
    assert_match "ffts-grep", shell_output("#{bin}/ffts-grep --version")

    # Test actual search functionality
    (testpath/"hello.txt").write("Hello World from ffts-grep test!")
    system bin/"ffts-grep", "init", "--project-dir", testpath
    output = shell_output("#{bin}/ffts-grep search --project-dir #{testpath} Hello")
    assert_match "hello.txt", output
  end
end
```

**Formula conventions followed:**
- Filename: lowercase with hyphens (`ffts-grep.rb`)
- Class name: CamelCase (`FftsGrep`)
- `head` block for development builds from master branch
- `cd "rust-fts5-indexer"` for subdirectory build
- `livecheck` block for automated version tracking (ripgrep pattern)
- Functional test: creates file, initializes index, verifies search works
- `std_cargo_args` includes `--locked` for reproducible builds

### Step 3: Update README.md

Add ffts-grep to the tap's README alongside healthsync:

```markdown
## Available Formulas

| Formula | Description |
|---------|-------------|
| healthsync | Secure sync of Apple HealthKit data |
| ffts-grep | Fast full-text search file indexer using SQLite FTS5 |

## Installation

```bash
brew tap mneves75/tap
brew install ffts-grep
```
```

### Step 4: Audit Formula (Best Practice)

```bash
brew audit --strict --online ffts-grep
```

### Step 5: Commit and Push

```bash
git add Formula/ffts-grep.rb README.md
git commit -m "Add ffts-grep formula v<VERSION>"
git push
```

## User Installation

```bash
brew tap mneves75/tap      # If not already tapped
brew install ffts-grep

# Verify
ffts-grep --version
ffts-grep doctor
```

## Verification Checklist

1. `brew audit --strict ffts-grep` - passes linting
2. `brew install ffts-grep` - builds successfully
3. `brew test ffts-grep` - functional test passes (creates file, indexes, searches)
4. `ffts-grep --version` - shows version <VERSION>
5. `ffts-grep doctor` - runs diagnostics
6. Both formulas coexist: `brew list --formula | grep -E 'healthsync|ffts-grep'`
7. `brew livecheck ffts-grep` - version tracking works

## Technical Notes

### Why `std_cargo_args`?
- Includes `--locked` for reproducible builds from Cargo.lock
- Includes `--root #{prefix}` for correct installation path
- Includes `--path .` for current directory

### Why functional test?
- Version check alone doesn't verify FTS5 works
- Real test: create file → init index → search → verify result
- Catches linking issues, missing SQLite features

### Why `livecheck`?
- Enables `brew livecheck ffts-grep` for version monitoring
- CI/automation can detect new releases
- `strategy :github_latest` follows GitHub releases

---

## Part 2: Automated Formula Updates (GitHub Actions)

### Architecture: Two-Repository Pattern

```
┌─────────────────────────────────┐         ┌─────────────────────────────────┐
│       ffts-grep repo            │         │      homebrew-tap repo          │
│                                 │         │                                 │
│  git tag v<VERSION>                │         │  Formula/ffts-grep.rb           │
│         │                       │         │         ▲                       │
│         ▼                       │         │         │                       │
│  .github/workflows/             │         │  .github/workflows/             │
│    release.yml                  │         │    bump-formula.yml             │
│         │                       │  PAT    │         │                       │
│         └──────────────────────────────────────────▶│                       │
│           gh workflow run       │         │    workflow_dispatch            │
└─────────────────────────────────┘         └─────────────────────────────────┘
```

### Step 1: Create PAT (Personal Access Token)

1. Go to GitHub → Settings → Developer settings → Personal access tokens → Fine-grained tokens
2. Create token with:
   - Name: `homebrew-tap-updater`
   - Repository access: Select `mneves75/homebrew-tap`
   - Permissions: `contents: write`, `actions: write`
3. Copy token

### Step 2: Add Secret to ffts-grep Repo

1. Go to `mneves75/ffts-grep` → Settings → Secrets → Actions
2. Add secret: `HOMEBREW_TAP_TOKEN` = (paste token)

### Step 3: Create Workflow in ffts-grep Repo

**File: `ffts-grep/.github/workflows/release.yml`**

```yaml
name: Release and Update Homebrew

on:
  push:
    tags:
      - "v*"

jobs:
  release:
    runs-on: ubuntu-latest
    permissions:
      contents: write
    steps:
      - uses: actions/checkout@v4

      - name: Get version
        id: version
        run: echo "VERSION=${GITHUB_REF#refs/tags/}" >> "$GITHUB_ENV"

      - name: Create GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          generate_release_notes: true

      - name: Trigger Homebrew tap update
        run: |
          gh workflow run bump-formula.yml \
            --repo mneves75/homebrew-tap \
            -f version=${{ env.VERSION }}
        env:
          GH_TOKEN: ${{ secrets.HOMEBREW_TAP_TOKEN }}
```

### Step 4: Create Workflow in homebrew-tap Repo

**File: `homebrew-tap/.github/workflows/bump-formula.yml`**

```yaml
name: Bump ffts-grep Formula

on:
  workflow_dispatch:
    inputs:
      version:
        description: 'Version tag (e.g., v0.11.4)'
        required: true
        type: string

jobs:
  update-formula:
    runs-on: ubuntu-latest
    permissions:
      contents: write
    steps:
      - uses: actions/checkout@v4

      - name: Update formula
        run: |
          VERSION="${{ github.event.inputs.version }}"
          VERSION_NUM="${VERSION#v}"  # Strip 'v' prefix

          # Calculate SHA256
          SHA256=$(curl -sL "https://github.com/mneves75/ffts-grep/archive/refs/tags/${VERSION}.tar.gz" | shasum -a 256 | cut -d' ' -f1)

          # Generate formula
          cat > Formula/ffts-grep.rb << 'FORMULA'
          class FftsGrep < Formula
            desc "Fast full-text search file indexer using SQLite FTS5"
            homepage "https://github.com/mneves75/ffts-grep"
            url "https://github.com/mneves75/ffts-grep/archive/refs/tags/VERSION_PLACEHOLDER.tar.gz"
            sha256 "SHA256_PLACEHOLDER"
            license "Apache-2.0"
            head "https://github.com/mneves75/ffts-grep.git", branch: "master"

            livecheck do
              url :stable
              strategy :github_latest
            end

            depends_on "rust" => :build

            def install
              cd "rust-fts5-indexer" do
                system "cargo", "install", *std_cargo_args
              end
            end

            test do
              assert_match "ffts-grep", shell_output("#{bin}/ffts-grep --version")
              (testpath/"hello.txt").write("Hello World from ffts-grep test!")
              system bin/"ffts-grep", "init", "--project-dir", testpath
              output = shell_output("#{bin}/ffts-grep search --project-dir #{testpath} Hello")
              assert_match "hello.txt", output
            end
          end
          FORMULA

          # Replace placeholders
          sed -i "s|VERSION_PLACEHOLDER|${VERSION}|g" Formula/ffts-grep.rb
          sed -i "s|SHA256_PLACEHOLDER|${SHA256}|g" Formula/ffts-grep.rb

      - name: Commit and push
        run: |
          git config user.name "github-actions[bot]"
          git config user.email "github-actions[bot]@users.noreply.github.com"
          git add Formula/ffts-grep.rb
          git commit -m "Bump ffts-grep to ${{ github.event.inputs.version }}"
          git push
```

### How It Works

1. **You tag a release**: `git tag v<VERSION> && git push --tags`
2. **ffts-grep workflow runs**: Creates GitHub release, triggers tap update
3. **homebrew-tap workflow runs**: Downloads tarball, calculates SHA256, regenerates formula, commits
4. **Users get update**: `brew upgrade ffts-grep`

### Manual Trigger (Alternative)

If automation fails, manually trigger from homebrew-tap:
```bash
gh workflow run bump-formula.yml --repo mneves75/homebrew-tap -f version=v<VERSION>
```

---

## Implementation Order

| # | Task | Repo |
|---|------|------|
| 1 | Calculate SHA256 for v<VERSION> | Local |
| 2 | Create `Formula/ffts-grep.rb` | homebrew-tap |
| 3 | Update tap README | homebrew-tap |
| 4 | Audit and test formula | Local |
| 5 | Create PAT and add secret | GitHub |
| 6 | Add `release.yml` workflow | ffts-grep |
| 7 | Add `bump-formula.yml` workflow | homebrew-tap |
| 8 | Test automation with v<VERSION> release | Both |

---

## Sources

- [How to Create and Maintain a Tap](https://docs.brew.sh/How-to-Create-and-Maintain-a-Tap)
- [Formula Cookbook](https://docs.brew.sh/Formula-Cookbook)
- [Taps Documentation](https://docs.brew.sh/Taps)
- [ripgrep formula](https://github.com/Homebrew/homebrew-core/blob/master/Formula/r/ripgrep.rb) - Production Rust formula reference
- [Automating Homebrew Tap Updates](https://builtfast.dev/blog/automating-homebrew-tap-updates-with-github-actions/) - Two-repo pattern
- [Simon Willison's Homebrew Automation](https://til.simonwillison.net/homebrew/auto-formulas-github-actions) - Formula generation
- [josh.fail Homebrew Automation](https://josh.fail/2023/automate-updating-custom-homebrew-formulae-with-github-actions/) - Cross-repo triggering
