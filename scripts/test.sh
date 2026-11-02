#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all -- --check
cargo test --locked
python3 scripts/test-tools.py
python3 scripts/test-contracts.py
