#!/usr/bin/env bash
# Usage: run-load.sh <generated-web-root> <fresh-output-directory> [seconds, default 600]
set -euo pipefail
root=$(cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$root"
export CARGO_TARGET_DIR="$root/artifacts/build/agents"
export RUSTUP_HOME="$root/artifacts/cache/rustup"
export CARGO_HOME="$root/artifacts/cache/cargo"
export CARGO_BUILD_JOBS=2
cargo build -p df-tools --example load_client --bin df-transport-fixture
exec python3 -B deploy/scripts/load/run_load.py --web-root "${1:?generated web root required}" \
  --out "${2:?fresh output directory required}" --seconds "${3:-600}" \
  --psql "${PSQL:-psql}" --database-port "${PGPORT:-55911}"
