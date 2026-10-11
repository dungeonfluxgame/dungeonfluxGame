#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
ARTIFACT_ROOT=${DUNGEONFLUX_ARTIFACT_ROOT:-$ROOT/artifacts}
export RUSTUP_HOME="$ARTIFACT_ROOT/cache/rustup"
export CARGO_HOME="$ARTIFACT_ROOT/cache/cargo"
export CARGO_TARGET_DIR=${DUNGEONFLUX_BUILD_ROOT:-$ARTIFACT_ROOT/build/play}
export TMPDIR=${DUNGEONFLUX_TMP_ROOT:-$ARTIFACT_ROOT/tmp/play}
export PATH="$CARGO_HOME/bin:$PATH"
PG=${DUNGEONFLUX_POSTGRES_BIN:-$ARTIFACT_ROOT/build/postgres-g05/install/bin}
PLAY="$ARTIFACT_ROOT/tmp/play"
DATABASE=
NO_BUILD=0

usage() {
  echo 'usage: run-gameplay.sh [--no-build] [--database <name>]'
}

while test "$#" -gt 0; do
  case "$1" in
    --no-build) NO_BUILD=1; shift ;;
    --database)
      if test "$#" -lt 2; then usage >&2; exit 2; fi
      DATABASE=$2
      shift 2
      ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done

if test -z "$DATABASE"; then
  DATABASE="df_gameplay_demo_play_$(date -u +%Y%m%d%H%M%S)"
fi
case "$DATABASE" in
  df_gameplay_demo_*) ;;
  *) echo 'invalid database name: expected df_gameplay_demo_<name> with only letters, digits, or underscores' >&2; exit 1 ;;
esac
case "$DATABASE" in
  *[!A-Za-z0-9_]*) echo 'invalid database name: expected df_gameplay_demo_<name> with only letters, digits, or underscores' >&2; exit 1 ;;
esac
if test "${#DATABASE}" -gt 63 || test "$DATABASE" = df_gameplay_demo_; then
  echo 'invalid database name: must be 1-63 characters and match df_gameplay_demo_<name>' >&2
  exit 1
fi

if test ! -x "$CARGO_HOME/bin/cargo"; then echo "missing cargo: $CARGO_HOME/bin/cargo" >&2; exit 1; fi
if ! command -v lsof >/dev/null 2>&1; then echo 'missing lsof: required to check listening ports' >&2; exit 1; fi
if test "$NO_BUILD" -eq 0; then
  if test ! -x "$CARGO_HOME/bin/wasm-bindgen"; then echo "missing wasm-bindgen: $CARGO_HOME/bin/wasm-bindgen" >&2; exit 1; fi
  WASM_BINDGEN_VERSION=$("$CARGO_HOME/bin/wasm-bindgen" --version)
  if test "$WASM_BINDGEN_VERSION" != 'wasm-bindgen 0.2.129'; then echo "wrong wasm-bindgen version: expected wasm-bindgen 0.2.129, found $WASM_BINDGEN_VERSION" >&2; exit 1; fi
fi
for PG_TOOL in initdb pg_ctl createdb psql pg_isready; do
  if test ! -x "$PG/$PG_TOOL"; then echo "missing PostgreSQL tool: $PG/$PG_TOOL" >&2; exit 1; fi
done

LISTEN=${DF_GAMEPLAY_LISTEN:-127.0.0.1:63309}
GAME_PORT=${LISTEN##*:}
GAME_HOLDER=$(lsof -nP -iTCP:"$GAME_PORT" -sTCP:LISTEN 2>/dev/null || true)
if test -n "$GAME_HOLDER"; then
  echo "game port $GAME_PORT is already in use:" >&2
  echo "$GAME_HOLDER" >&2
  exit 1
fi

PG_HOLDER=$(lsof -nP -iTCP:55517 -sTCP:LISTEN 2>/dev/null || true)
PG_ALREADY_RUNNING=0
if test -n "$PG_HOLDER"; then
  case "$PG_HOLDER" in
    *postgres*)
      if "$PG/pg_ctl" -D "$PLAY/pgdata" status >/dev/null 2>&1; then PG_ALREADY_RUNNING=1
      else echo 'PostgreSQL port 55517 is held by another process:' >&2; echo "$PG_HOLDER" >&2; exit 1
      fi
      ;;
    *) echo 'PostgreSQL port 55517 is held by another process:' >&2; echo "$PG_HOLDER" >&2; exit 1 ;;
  esac
fi

mkdir -p "$TMPDIR" "$PLAY"
chmod 700 "$PLAY"
cd "$ROOT"
if test "$NO_BUILD" -eq 0; then
  cargo build --locked --offline -p df-tools --bin df-transport-fixture
  cargo build --locked --offline -p df-tools --lib --target wasm32-unknown-unknown
  STAGING=$(mktemp -d "$TMPDIR/wasm-bindgen.XXXXXXXX")
  wasm-bindgen --target web --out-name df_tools --out-dir "$STAGING" "$CARGO_TARGET_DIR/wasm32-unknown-unknown/debug/df_tools.wasm"
  OLD_WEB="$TMPDIR/play-web.old.$$"
  if test -e "$CARGO_TARGET_DIR/play-web"; then mv "$CARGO_TARGET_DIR/play-web" "$OLD_WEB"; fi
  if mv "$STAGING" "$CARGO_TARGET_DIR/play-web"; then
    rm -rf "$OLD_WEB"
  else
    if test -e "$OLD_WEB"; then mv "$OLD_WEB" "$CARGO_TARGET_DIR/play-web"; fi
    exit 1
  fi
fi

if test ! -x "$CARGO_TARGET_DIR/debug/df-transport-fixture"; then echo "missing gameplay server binary: $CARGO_TARGET_DIR/debug/df-transport-fixture" >&2; exit 1; fi
if test ! -d "$CARGO_TARGET_DIR/play-web" || test -z "$(find "$CARGO_TARGET_DIR/play-web" -type f -print -quit)"; then echo "missing gameplay web files: $CARGO_TARGET_DIR/play-web" >&2; exit 1; fi

SERVER_PID=
PG_STARTED=0
CLEANED=0
SERVER_STATUS=0
cleanup() {
  if test "$CLEANED" -eq 1; then return; fi
  CLEANED=1
  if test -n "$SERVER_PID" && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill -INT "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  if test "$PG_STARTED" -eq 1; then "$PG/pg_ctl" -D "$PLAY/pgdata" -m fast -w stop || true; fi
}
on_signal() {
  SIGNAL_STATUS=$1
  cleanup
  exit "$SIGNAL_STATUS"
}
trap cleanup EXIT
trap 'on_signal 130' INT
trap 'on_signal 143' TERM

if test "$PG_ALREADY_RUNNING" -eq 0; then
  if test ! -d "$PLAY/pgdata"; then
    "$PG/initdb" -D "$PLAY/pgdata" -U df_gameplay_demo_admin_20261004 --auth-host=trust --auth-local=trust -E UTF8 --locale=C
  fi
  mkdir -p "$PLAY/sock"
  "$PG/pg_ctl" -D "$PLAY/pgdata" -l "$PLAY/postgres.log" -o "-p 55517 -h 127.0.0.1 -k $PLAY/sock -c max_connections=24" -w start
  PG_STARTED=1
fi

if test -z "$("$PG/psql" -h 127.0.0.1 -p 55517 -U df_gameplay_demo_admin_20261004 -d postgres -Atc "SELECT 1 FROM pg_database WHERE datname='$DATABASE'")"; then
  "$PG/createdb" -h 127.0.0.1 -p 55517 -U df_gameplay_demo_admin_20261004 -T template0 "$DATABASE"
fi

echo "Playing with database: $DATABASE"
echo "To resume this game later: development/run-gameplay.sh --no-build --database $DATABASE"
DF_GAMEPLAY_DEMO_DATABASE=$DATABASE "$CARGO_TARGET_DIR/debug/df-transport-fixture" --gameplay-demo "$CARGO_TARGET_DIR/play-web" "$ROOT/assets/ui" &
SERVER_PID=$!
wait "$SERVER_PID" || SERVER_STATUS=$?
SERVER_PID=
exit "$SERVER_STATUS"
