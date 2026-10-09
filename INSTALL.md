# Installation Guide

Tracer installs two command names, **`tracer`** and **`tr`**, with the same CLI.
There is no `trace` command. No crates.io package, Homebrew tap, or distribution
package is currently advertised; use this repository or verified build artifacts.

## Supported platform

Linux x86-64 is the supported and tested platform. The downloadable archive targets
`x86_64-unknown-linux-musl` and contains statically linked binaries, including
SQLite; no Rust installation or system SQLite is required to run them.

macOS, Windows, ARM, and other targets are **not currently supported downloads**.
In particular, snapshot publication opens and fsyncs the containing directory after
atomic rename. That implementation needs platform-specific validation (and changes
on Windows), not just a successful cross-compile. Tests do not simulate hardware
power loss. Use a local filesystem; network filesystem durability/locking has not
been validated.

## Install from source

Install [Rust](https://rustup.rs/) **1.89 or newer** and a C toolchain. Rust 1.89 is
required for the standard-library file locking used by synchronization.
On Debian/Ubuntu, the build tools can be installed with:

```bash
sudo apt-get update
sudo apt-get install -y build-essential
```

Then clone and install both commands using the committed dependency lockfile:

```bash
git clone https://github.com/Abil-Shrestha/tracer.git
cd tracer
cargo install --locked --path . --bins
export PATH="$HOME/.cargo/bin:$PATH"
tracer --version
tr --version
```

For a particular version, check out its reviewed tag or commit before installing.
The source build bundles SQLite and does not require OpenSSL or a SQLite development
package. A native Linux source build may depend on the host's libc; it is not the
static download build described below.

## Install a binary artifact

The **CI** and manually dispatched **Prepare release artifacts** workflows produce
an Actions artifact named `tracer-linux-x86_64-<commit>`. They do **not** create or
publish a GitHub Release. Download the artifact from a successful, trusted workflow
run for the commit you intend to install; GitHub may require you to sign in. Artifacts
expire after 14 days. Do not assume a `releases/latest/download` URL exists.

After unzipping the Actions download, it contains:

```text
tracer-v0.2.0-x86_64-unknown-linux-musl.tar.gz
SHA256SUMS
```

From that directory, verify the archive **before** extracting it. Change `version`
to match the downloaded archive; do not mix files from different workflow runs.

```bash
version=0.2.0
archive="tracer-v${version}-x86_64-unknown-linux-musl"
sha256sum --check SHA256SUMS
tar -xzf "${archive}.tar.gz"
mkdir -p "$HOME/.local/bin"
install -m 755 "${archive}/tracer" "${archive}/tr" "$HOME/.local/bin/"
export PATH="$HOME/.local/bin:$PATH"
tracer --version
tr --version
```

Stop if checksum verification fails. SHA-256 detects corruption; it is not a
signature or proof that an untrusted workflow is safe. Inspect the run's source
commit and provenance before executing its binaries.

The archive also includes `LICENSE`, `README.md`, this guide, `BUILD-INFO` (source
commit, target, Rust version, and source timestamp), and the agent skill at
`.agents/skills/tracking-work-with-tracer/SKILL.md`. See the README's skill
installation instructions. Keep these files when redistributing the archive.

## Verify and start

```bash
tracer --help
tracer init
tracer create "My first task" --json
tracer ready --json
```

`tr` accepts the same commands. For behavioral validation in a source checkout,
the [P0 evals](evals/README.md) accept an absolute `--binary` path and use disposable
data, never your existing tracker.

## Update or uninstall

For a source installation, fetch and review the desired revision, check it out,
then run `cargo install --locked --path . --bins --force`. Uninstall both commands
with `cargo uninstall tracer`.

For a binary installation, repeat the checksum/extract/install steps for the new
artifact. To uninstall, remove only the `tracer` and `tr` files you installed in
`~/.local/bin`. Neither uninstall method removes your tracker data.

## Troubleshooting

- **Command not found:** add the installation directory (`~/.cargo/bin` or
  `~/.local/bin`) to your shell's PATH. Use `command -v tracer` and `command -v tr`
  to check for an older installation shadowing the new one.
- **Old Rust:** run `rustup update stable` and check `rustc --version` is at least
  1.89. See [CONTRIBUTING.md](CONTRIBUTING.md) for the exact CI toolchain.
- **C compiler missing:** install `build-essential` before building bundled SQLite.
- **Sync/publication failure:** do not discard the database or assume the mutation
  did not happen. Follow the recovery guidance in the [README](README.md).

See [CONTRIBUTING.md](CONTRIBUTING.md) for checks, packaging, and release preparation.
