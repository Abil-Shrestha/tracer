#!/usr/bin/env bash
# Build the one supported download target. Requires GNU tar, gzip, musl-gcc,
# Python 3, Git, and the Rust toolchain/target documented in CONTRIBUTING.md.
set -euo pipefail
export LC_ALL=C
export TZ=UTC
export RUSTUP_TOOLCHAIN=1.89.0

if [[ $# -gt 1 || $(uname -s) != Linux || $(uname -m) != x86_64 ]]; then
  echo "Usage: bash scripts/package.sh [output-directory] (Linux x86-64 only)" >&2
  exit 1
fi

root=$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)
cd "$root"
target=x86_64-unknown-linux-musl
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}
[[ $SOURCE_DATE_EPOCH =~ ^[0-9]+$ ]] || { echo "Invalid SOURCE_DATE_EPOCH" >&2; exit 1; }
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-"$root/target"}
mkdir -p "$CARGO_TARGET_DIR" "${1:-target/dist}"
CARGO_TARGET_DIR=$(realpath "$CARGO_TARGET_DIR")
output=$(realpath "${1:-target/dist}")
export CC_x86_64_unknown_linux_musl=musl-gcc
# Rust supplies its own musl CRT. The host driver supports -static-pie, whereas
# Debian's musl-gcc specs inject a dynamic interpreter even for that link mode.
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=cc
# Remove checkout and registry locations from embedded Rust/C source paths.
export RUSTFLAGS="--remap-path-prefix=$root=. --remap-path-prefix=$HOME/.cargo=/cargo"
export CFLAGS_x86_64_unknown_linux_musl="-ffile-prefix-map=$root=. -ffile-prefix-map=$HOME/.cargo=/cargo"

version=$(cargo metadata --locked --no-deps --format-version 1 |
  python3 -c 'import json, sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "tracer"))')
name="tracer-v$version-$target"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
skill=.agents/skills/tracking-work-with-tracer/SKILL.md
mkdir -p "$stage/$name/$(dirname "$skill")" "$stage/$name/docs"
install -m 644 LICENSE README.md INSTALL.md "$stage/$name/"
install -m 644 "$skill" "$stage/$name/$skill"
install -m 644 docs/agent-cli.md "$stage/$name/docs/"

cargo build --locked --release --target "$target" --bins
for binary in tracer tr; do
  install -m 755 "$CARGO_TARGET_DIR/$target/release/$binary" "$stage/$name/$binary"
done
printf 'source=%s\ntarget=%s\nrustc=%s\nsource_date_epoch=%s\n' \
  "$(git rev-parse HEAD)" "$target" "$(rustc --version)" "$SOURCE_DATE_EPOCH" > "$stage/$name/BUILD-INFO"

# Normalize order, owners, modes, mtimes, and gzip headers. No wall-clock time or
# absolute build paths enter the archive metadata.
tar --sort=name --format=gnu --owner=0 --group=0 --numeric-owner \
  --mtime="@$SOURCE_DATE_EPOCH" --mode='u=rwX,go=rX' \
  -C "$stage" -cf - "$name" | gzip -n -9 > "$output/$name.tar.gz"
(
  cd "$output"
  sha256sum "$name.tar.gz" > SHA256SUMS
)
echo "Packaged $output/$name.tar.gz"
