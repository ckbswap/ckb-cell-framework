#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
bash scripts/build.sh framework-counter-script
npm --prefix examples/counter/web run deploy:testnet -- "$@"
