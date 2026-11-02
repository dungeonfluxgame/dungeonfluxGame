#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"

PACKAGE_DIRECTORY="$ROOT/artifacts/playground/pkg"
# The generator must match the wasm-bindgen crate in Cargo.lock.
WASM_BINDGEN_VERSION=0.2.129

usage() {
  cat <<'USAGE'
Usage: development/playground.sh COMMAND

Commands:
  build   Compile crates/df-playground for the browser into artifacts/playground/pkg
  serve   Start the canvas host, which also serves the playground at /playground
          (extra arguments go to `df-table serve`, for example --port 7979)

The page, theme and fixtures are plain files in assets/playground. Edit them while the
page is open and it redraws by itself; only a change to the Rust needs `build` again, and
the page reloads itself when the package changes.
USAGE
}

command=${1:-}
if [[ -z "$command" || "$command" == "-h" || "$command" == "--help" ]]; then
  usage
  exit 0
fi
shift

case "$command" in
  build)
    generator=${WASM_BINDGEN:-$ROOT/artifacts/cache/cargo/bin/wasm-bindgen}
    if [[ ! -x "$generator" && ! -x "$generator.exe" ]]; then
      echo "missing $generator; see development/start-s00.md (cargo install wasm-bindgen-cli --version $WASM_BINDGEN_VERSION)" >&2
      exit 1
    fi
    found=$("$generator" --version | tr -d '\r')
    if [[ "$found" != "wasm-bindgen $WASM_BINDGEN_VERSION" ]]; then
      echo "wrong wasm-bindgen: expected $WASM_BINDGEN_VERSION from Cargo.lock, found $found" >&2
      exit 1
    fi
    cargo build -p df-playground --lib --target wasm32-unknown-unknown --locked
    target_directory=${CARGO_TARGET_DIR:-$ROOT/target}
    mkdir -p "$PACKAGE_DIRECTORY"
    "$generator" --target web --no-typescript --out-dir "$PACKAGE_DIRECTORY" \
      "$target_directory/wasm32-unknown-unknown/debug/df_playground.wasm"
    ;;
  serve)
    exec cargo run -p df-table-host --bin df-table --locked -- serve "$@"
    ;;
  *)
    echo "unknown command: $command" >&2
    usage >&2
    exit 2
    ;;
esac
