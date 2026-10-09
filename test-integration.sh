#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"
cargo build --locked --bin tracer
exec python3 evals/p0.py --binary target/debug/tracer "$@"
