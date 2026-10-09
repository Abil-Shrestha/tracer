# Contributing to Tracer

Thank you for your interest in contributing to Tracer! This document provides guidelines and instructions for contributing.

## Code of Conduct

Be respectful, constructive, and collaborative. We're all here to make Trace better.

## Getting Started

Use Linux x86-64, a C compiler (SQLite is bundled), Python 3.9+, and Rust 1.89+.
CI pins Rust 1.89.0, the minimum supported version, for repeatable checks and builds:

```bash
rustup toolchain install 1.89.0 --profile minimal --component clippy --component rustfmt
export RUSTUP_TOOLCHAIN=1.89.0
```

1. **Fork the repository** on GitHub
2. **Clone your fork** locally:
   ```bash
git clone https://github.com/Abil-Shrestha/tracer.git
cd tracer
   ```
3. **Build the project**:
   ```bash
   cargo build --locked --bins
   ```
4. **Run tests**:
   ```bash
   cargo test --locked
   ```

Both `tracer` and `tr` are intentional binary targets sharing `src/main.rs`.
Cargo may warn about this; do not suppress compiler/Clippy warnings to hide it.
Use `cargo run --locked --bin tracer -- --help` to select a target explicitly.

## Development Workflow

### Making Changes

1. **Create a branch** for your feature or bugfix:

   ```bash
   git checkout -b feature/my-amazing-feature
   ```

2. **Make your changes** following the coding standards below

3. **Test your changes**:

   ```bash
   cargo build --locked --bins
   cargo test --locked
   cargo clippy --locked --all-targets --all-features -- -D warnings
   cargo fmt --all -- --check
   python3 evals/p0.py --binary target/debug/tracer --rounds 5 --json
   ```

4. **Commit with clear messages**:
   ```bash
   git commit -m "feat: add amazing feature"
   ```

### Commit Message Format

We follow [Conventional Commits](https://www.conventionalcommits.org/):

- `feat:` - New features
- `fix:` - Bug fixes
- `docs:` - Documentation changes
- `test:` - Test additions or modifications
- `refactor:` - Code refactoring
- `perf:` - Performance improvements
- `chore:` - Maintenance tasks

Examples:

```
feat: add markdown import for bulk issue creation
fix: resolve database locking issue on concurrent access
docs: update README with performance benchmarks
```

### Pull Request Process

1. **Update documentation** if needed (README, inline docs)
2. **Add tests** for new features
3. **Ensure tests and P0 evals pass** using the commands above
4. **Format code**: `cargo fmt --all`
5. **Lint code**: `cargo clippy --locked --all-targets --all-features -- -D warnings`
6. **Push to your fork**:
   ```bash
   git push origin feature/my-amazing-feature
   ```
7. **Open a Pull Request** on GitHub

#### PR Checklist

- [ ] Code builds without errors
- [ ] All tests pass
- [ ] New tests added for new features
- [ ] Documentation updated
- [ ] Code formatted with `cargo fmt`
- [ ] No new clippy warnings
- [ ] Commit messages follow convention

### CI and binary packaging

`.github/workflows/ci.yml` runs on pull requests and pushes to `main`. A Linux
check job builds both binaries with the lockfile, runs Rust tests and the P0 evals
(five scenarios × five rounds), and requires strict Clippy and formatting. The
package job runs only after those checks pass. It builds a static Linux x86-64
musl archive, checks that packaging it twice gives the same SHA-256, validates the
contents and executable modes, checks static linkage, and runs all P0 scenarios
five times through **each extracted binary** before uploading the artifact.

Run the same packaging steps locally from the repository on Linux x86-64:

```bash
sudo apt-get update
sudo apt-get install -y build-essential musl-tools binutils
rustup toolchain install 1.89.0 --profile minimal --target x86_64-unknown-linux-musl
bash scripts/package.sh
python3 scripts/verify-package.py target/dist --rounds 5
```

The scripts also require Python 3.9+, GNU tar, gzip, Git, and `sha256sum`.
Output is `target/dist/tracer-v<VERSION>-x86_64-unknown-linux-musl.tar.gz` and
`target/dist/SHA256SUMS`. Pass a different output directory as the first argument
to `package.sh`; use a separate directory for each version. The archive contains
`tracer`, `tr`, `LICENSE`, `README.md`, `INSTALL.md`, `BUILD-INFO`, and
the skill and CLI contract at `.agents/skills/tracking-work-with-tracer/SKILL.md`
and `docs/agent-cli.md` under a versioned directory.

Packaging pins Rust 1.89.0, uses `Cargo.lock`, remaps source paths, normalizes tar
order/ownership/modes/timestamps, and omits gzip timestamps. `SOURCE_DATE_EPOCH`
defaults to the checked-out commit's timestamp; it can be set to a nonnegative
Unix timestamp. `BUILD-INFO` records the commit, target, Rust version, and epoch.
Build from a clean reviewed checkout for distributable artifacts; local worktree
changes are included but are not described by the commit recorded in `BUILD-INFO`.

Reproducibility means identical archives for identical source, toolchain, build
environment, and epoch. It is **not** a claim of bit-identical binaries across C
compiler versions or host distributions. The hosted jobs use Ubuntu 22.04;
runner images and apt packages can change. To compare independent builds, keep
those inputs fixed and set distinct `CARGO_TARGET_DIR` and output directories.
Checksums detect corruption, not malicious builds; no signing or attestation is
currently provided.

### Release preparation does not publish

The manual **Prepare release artifacts** workflow (`release.yml`) reuses the exact
CI gates and artifact upload. A maintainer can select the reviewed branch/tag
when manually dispatching it. Neither workflow has `contents: write`, creates a
GitHub Release, pushes tags, publishes crates, nor deploys anything. Checkout does
not persist credentials; external actions are pinned to commits. Updating the Rust
pin requires changing `ci.yml`, `scripts/package.sh`, and `.agents/setup`, then
rerunning the gates.

Download the `tracer-linux-x86_64-<commit>` artifact from its successful Actions
run within 14 days. It is an Actions ZIP containing the tarball and `SHA256SUMS`;
the tarball preserves Unix executable modes. Public release publication remains
a separate, explicitly approved maintainer action. See [INSTALL.md](INSTALL.md)
for checksum verification and installation instructions.

Do not add Windows/macOS/ARM downloads on compilation evidence alone. Snapshot
publication includes file fsync, atomic rename, and directory fsync; validate that
sequence and the locking/recovery tests on the target OS first. Current CI covers
Linux only and does not simulate hardware power loss or network filesystems.

## Coding Standards

### Rust Style

- Follow the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- Use `cargo fmt` for consistent formatting
- Fix all `cargo clippy` warnings
- Write clear, descriptive variable names
- Add inline documentation for public APIs

### Code Organization

```
src/
├── types.rs        # Core data structures
├── storage/        # Storage layer (trait + implementations)
├── cli/            # CLI commands (one file per command group)
├── utils.rs        # Helper functions
├── lib.rs          # Public library API
└── main.rs         # CLI entry point
```

### Documentation

- Add doc comments (`///`) for public functions and types
- Include examples in doc comments when helpful
- Update README.md for user-facing changes
- Add inline comments for complex logic

Example:

````rust
/// Creates a new issue in the database.
///
/// # Arguments
///
/// * `issue` - The issue to create
/// * `actor` - Name of the user/agent creating the issue
///
/// # Example
///
/// ```
/// let issue = Issue { ... };
/// storage.create_issue(&issue, "agent")?;
/// ```
pub fn create_issue(&mut self, issue: &Issue, actor: &str) -> Result<()> {
    // Implementation
}
````

### Testing

- Write unit tests for new functions
- Add integration tests for new commands
- Test edge cases and error conditions
- Use descriptive test names

Example:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_issue_generates_id() {
        let mut storage = SqliteStorage::new(":memory:").unwrap();
        let id = storage.generate_id("test").unwrap();
        assert!(id.starts_with("test-"));
        assert_ne!(id, storage.generate_id("test").unwrap());
    }

    #[test]
    fn test_ready_work_excludes_blocked_issues() {
        // Test implementation
    }
}
```

## Areas for Contribution

### High Priority

- **Markdown import** - Bulk issue creation from markdown files
- **Performance optimization** - SQLite query improvements
- **Windows testing** - Ensure cross-platform compatibility
- **Error handling** - Better error messages and recovery

### Medium Priority

- **Query language** - Advanced filtering syntax
- **Custom fields** - User-defined issue fields
- **Export formats** - CSV, Markdown, etc.
- **Git hooks** - Example scripts for automation

### Low Priority / Future

- **Web UI** - Optional visualization layer
- **Plugin system** - Extensibility framework
- **GitHub integration** - Sync with GitHub Issues
- **TUI** - Terminal UI for interactive use

## Project Architecture

### Storage Layer

The storage layer is abstracted via the `Storage` trait in `src/storage/mod.rs`. Currently, we have:

- `SqliteStorage` - SQLite implementation (main)

Future storage backends could include PostgreSQL, MongoDB, etc.

### CLI Layer

Each command is implemented as a module in `src/cli/`:

- `init.rs` - Database initialization
- `create.rs` - Issue creation
- `list.rs` - List issues
- `show.rs` - Show details
- `update.rs` - Update/close issues
- `ready.rs` - Ready/blocked work
- `dep.rs` - Dependency management
- `export.rs` - Export/import JSONL
- `stats.rs` - Statistics

### Data Flow

1. **CLI** parses arguments via `clap`
2. **Storage** is initialized (auto-discovery or explicit path)
3. **Command** executes via storage trait
4. **Output** formatted as text or JSON
5. **Auto-sync** exports to JSONL if changes made

## Questions?

- Open an issue for questions
- Check existing issues and PRs
- Reach out to maintainers

## Thank You!

Every contribution, no matter how small, makes Trace better for everyone. Thank you for being part of this project!
