#!/usr/bin/env bash
# Commit gate: fmt, clippy (native, plus wasm when df-playground is included) and lib tests.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"

DEFAULT_CRATES="df-table df-table-host df-playground"

usage() {
  cat <<'USAGE'
Usage: development/gate.sh [--quick] [crate ...]

Runs the commit gate in order and stops at the first failing stage:
  fmt          cargo fmt -p <crates> -- --check
  clippy       cargo clippy -p <crates> --offline --no-deps --all-targets -- -D warnings
  wasm-clippy  cargo clippy -p df-playground --offline --target wasm32-unknown-unknown -- -D warnings
               (only when df-playground is among the crates)
  tests        cargo test -p <crates> --offline --lib [-- GATE_TEST_ARGS]

Crates default to: df-table df-table-host df-playground

Options:
  --quick      Skip the tests stage
  -h, --help   Show this help

Environment:
  CARGO_TARGET_DIR  Used when already set; otherwise artifacts/build/start-s00
  GATE_TEST_ARGS    Extra arguments passed after "--" to the test harness,
                    for example: GATE_TEST_ARGS="--test-threads=1 some_filter"

Stage logs are written to artifacts/tmp/gate/<stage>.log. Exit status is 0 on
success, 1 when a stage fails, and 2 on a usage error.
USAGE
}

quick=0
crates=()
for arg in "$@"; do
  case "$arg" in
    -h|--help) usage; exit 0 ;;
    --quick) quick=1 ;;
    -*) echo "gate.sh: unknown option: $arg" >&2; usage >&2; exit 2 ;;
    *) crates+=("$arg") ;;
  esac
done
if [[ ${#crates[@]} -eq 0 ]]; then
  read -r -a crates <<< "$DEFAULT_CRATES"
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "gate.sh: cargo was not found on PATH" >&2
  exit 2
fi

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/artifacts/build/start-s00}"
export CARGO_NET_OFFLINE=true

gate_dir="$ROOT/artifacts/tmp/gate"
mkdir -p "$gate_dir"

pkg_args=()
has_playground=0
for crate in "${crates[@]}"; do
  pkg_args+=(-p "$crate")
  if [[ "$crate" == "df-playground" ]]; then
    has_playground=1
  fi
done

# Prints the first 20 lines of error output, starting at the first error-like line.
error_excerpt() {
  local log=$1 first
  first=$(grep -n -m1 -E '^error|panicked|FAILED|Diff in|failures:' "$log" 2>/dev/null | cut -d: -f1 || true)
  if [[ -z "$first" ]]; then
    first=1
  fi
  sed -n "${first},$((first + 19))p" "$log" | sed 's/^/    /'
}

# Runs one stage, logs its output, and exits 1 on failure with a short excerpt.
run_stage() {
  local name=$1 log rc secs start
  shift
  log="$gate_dir/$name.log"
  start=$(date +%s)
  printf '[gate] %-12s running: %s\n' "$name" "$*"
  rc=0
  "$@" >"$log" 2>&1 || rc=$?
  secs=$(( $(date +%s) - start ))
  if [[ $rc -ne 0 ]]; then
    printf '[gate] FAILED: stage %s (exit %s, %ss). Full log: %s\n' "$name" "$rc" "$secs" "$log"
    error_excerpt "$log"
    exit 1
  fi
  printf '[gate] %-12s ok (%ss)\n' "$name" "$secs"
}

# Sums the "N passed" and "N failed" counts over all cargo "test result:" lines.
test_totals() {
  awk '
    /^test result:/ {
      for (i = 1; i <= NF; i++) {
        if ($i == "passed;") passed += $(i - 1)
        if ($i == "failed;") failed += $(i - 1)
      }
    }
    END { printf "%d %d\n", passed + 0, failed + 0 }
  ' "$1"
}

overall_start=$(date +%s)
echo "[gate] crates: ${crates[*]}"
echo "[gate] target: $CARGO_TARGET_DIR"

run_stage fmt cargo fmt "${pkg_args[@]}" -- --check
run_stage clippy cargo clippy "${pkg_args[@]}" --offline --no-deps --all-targets -- -D warnings

if [[ $has_playground -eq 1 ]]; then
  run_stage wasm-clippy cargo clippy -p df-playground --offline --target wasm32-unknown-unknown -- -D warnings
fi

if [[ $quick -eq 1 ]]; then
  echo "[gate] tests        skipped (--quick)"
else
  test_cmd=(cargo test "${pkg_args[@]}" --offline --lib)
  if [[ -n "${GATE_TEST_ARGS:-}" ]]; then
    read -r -a test_extra <<< "$GATE_TEST_ARGS"
    test_cmd+=(-- "${test_extra[@]}")
  fi
  run_stage tests "${test_cmd[@]}"
  read -r passed failed < <(test_totals "$gate_dir/tests.log")
  echo "[gate] tests: $passed passed, $failed failed"
fi

echo "[gate] PASS in $(( $(date +%s) - overall_start ))s"
