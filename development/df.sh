#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"

usage() {
  cat <<'USAGE'
Usage: development/df.sh COMMAND [arguments]

Commands:
  check          Check all workspace targets (native)
  test ARGS...   Run a scoped Cargo test command; pass -p and a filter as needed
  fmt            Check workspace formatting
  lint           Run workspace Clippy with warnings denied
  wasm           Check df-tools for wasm32-unknown-unknown
  run ARGS...    Forward to development/run-gameplay.sh
  test-tools     Run the df-tools shard matrix with --strict-known
  python-check   Compile development Python tools without writing source caches
  deny           Run cargo-deny when both the tool and reviewed config exist
USAGE
}

command=${1:-}
if [[ -z "$command" || "$command" == "-h" || "$command" == "--help" ]]; then
  usage
  exit 0
fi
shift

case "$command" in
  check)
    cargo check --workspace --all-targets --locked "$@"
    ;;
  test)
    if [[ $# -eq 0 ]]; then
      echo "test needs Cargo test arguments, for example: -p df-types --lib <filter>" >&2
      exit 2
    fi
    cargo test --locked "$@"
    ;;
  fmt)
    cargo fmt --all -- --check
    ;;
  lint)
    cargo clippy --workspace --all-targets --locked -- -D warnings
    ;;
  wasm)
    cargo check -p df-tools --target wasm32-unknown-unknown --locked "$@"
    ;;
  run)
    exec bash "$ROOT/development/run-gameplay.sh" "$@"
    ;;
  test-tools)
    exec bash "$ROOT/development/df-tools-test-matrix.sh" --strict-known "$@"
    ;;
  python-check)
    python_cache="$ROOT/artifacts/tmp/df-python-cache"
    mapfile -d '' -t python_files < <(find "$ROOT/development" -type f -name '*.py' -print0)
    if command -v py >/dev/null 2>&1; then
      py -3 -I -X "pycache_prefix=$python_cache" -m py_compile "${python_files[@]}"
    elif command -v python3 >/dev/null 2>&1; then
      python3 -I -X "pycache_prefix=$python_cache" -m py_compile "${python_files[@]}"
    else
      echo "python-check needs the Python launcher (Windows) or python3 (Linux)" >&2
      exit 2
    fi
    ;;
  deny)
    if ! command -v cargo-deny >/dev/null 2>&1; then
      echo "SKIP: cargo-deny is not installed; see development/tooling.md" >&2
      exit 0
    fi
    if [[ ! -f "$ROOT/development/deny.toml" ]]; then
      echo "SKIP: development/deny.toml is not approved yet; see development/tooling.md" >&2
      exit 0
    fi
    cargo deny --manifest-path "$ROOT/Cargo.toml" --config "$ROOT/development/deny.toml" check "$@"
    ;;
  *)
    echo "unknown command: $command" >&2
    usage >&2
    exit 2
    ;;
esac
