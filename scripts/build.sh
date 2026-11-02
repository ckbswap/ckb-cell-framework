#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ -z "${CLANG:-}" ]]; then
  for candidate in clang-22 clang-21 clang-20 clang-19 /opt/homebrew/opt/llvm@18/bin/clang /opt/homebrew/opt/llvm/bin/clang /usr/local/opt/llvm/bin/clang clang; do
    if command -v "$candidate" >/dev/null 2>&1 && "$candidate" --print-targets 2>/dev/null | grep -q riscv64; then
      export CLANG="$candidate"
      break
    fi
  done
fi
if [[ -z "${CLANG:-}" ]]; then
  echo 'Install Clang with RISC-V support or set CLANG.' >&2
  exit 1
fi

contract_package="${1:-framework-counter-script}"
if [[ ! "$contract_package" =~ ^[a-z][a-z0-9-]*$ ]]; then
  echo 'Invalid contract package name.' >&2
  exit 1
fi
cargo build --locked -p "$contract_package" --bin "$contract_package" --features vm --release --target riscv64imac-unknown-none-elf
mkdir -p build
contract_target_dir="${CARGO_TARGET_DIR:-target}"
cp "$contract_target_dir/riscv64imac-unknown-none-elf/release/$contract_package" "build/$contract_package"
echo "Built: build/$contract_package"
