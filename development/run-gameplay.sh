#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
ARTIFACT_ROOT=${DUNGEONFLUX_ARTIFACT_ROOT:-$ROOT/artifacts}
if test "${DF_GAMEPLAY_PACK_DIR+x}" != x; then
  export DF_GAMEPLAY_PACK_DIR="$ROOT/content/harbor-journey"
fi
if test "${DF_GAMEPLAY_ADVENTURE+x}" != x; then
  export DF_GAMEPLAY_ADVENTURE=random
fi
if test "${DF_GAMEPLAY_ADVENTURE_TABLES+x}" != x; then
  export DF_GAMEPLAY_ADVENTURE_TABLES="$ROOT/content/adventure-tables"
fi
if test "${DF_GAMEPLAY_VOICE_CASTING+x}" != x; then
  export DF_GAMEPLAY_VOICE_CASTING="$ROOT/content/voice-casting.json"
fi
# Content rating (mature by default, or restrained): passed through only when the user set it.
if test "${DF_GAMEPLAY_CONTENT_RATING+x}" = x; then
  export DF_GAMEPLAY_CONTENT_RATING
fi
export RUSTUP_HOME="$ARTIFACT_ROOT/cache/rustup"
export CARGO_HOME="$ARTIFACT_ROOT/cache/cargo"
export CARGO_TARGET_DIR=${DUNGEONFLUX_BUILD_ROOT:-$ARTIFACT_ROOT/build/play}
export TMPDIR=${DUNGEONFLUX_TMP_ROOT:-$ARTIFACT_ROOT/tmp/play}
export PATH="$CARGO_HOME/bin:$PATH"
PG=${DUNGEONFLUX_POSTGRES_BIN:-$ARTIFACT_ROOT/build/postgres-g05/install/bin}
PLAY="$ARTIFACT_ROOT/tmp/play"
# Storyteller (text model): export only OPENAI_API_KEY from the gitignored .env, never echoed.
if test -z "${OPENAI_API_KEY:-}" && test -f "$ROOT/.env"; then
  OPENAI_API_KEY=$(sed -n 's/^OPENAI_API_KEY=//p' "$ROOT/.env" | head -n 1 | tr -d "\"'\r")
  if test -n "$OPENAI_API_KEY"; then export OPENAI_API_KEY; else unset OPENAI_API_KEY; fi
fi
export DF_GAMEPLAY_TEXT_CACHE=${DF_GAMEPLAY_TEXT_CACHE:-$PLAY/text-cache}
DATABASE=
NO_BUILD=0
CAMPAIGN=
CAMPAIGN_SEED=
CAMPAIGN_STATUS=0
HARBOR_REQUESTED=0
ADVENTURE_SEED_REQUESTED=0

usage() {
  echo 'usage: run-gameplay.sh [--no-build] [--database <name>] [--seed <decimal-or-hex-u64>] [--harbor] [--campaign <id>] [--campaign-seed <n>] [--status]'
}

while test "$#" -gt 0; do
  case "$1" in
    --campaign)
      if test "$#" -lt 2; then usage >&2; exit 2; fi
      CAMPAIGN=$2; export DF_GAMEPLAY_ADVENTURE=campaign; export DF_GAMEPLAY_CAMPAIGN=$2; shift 2 ;;
    --campaign-seed)
      if test "$#" -lt 2; then usage >&2; exit 2; fi
      CAMPAIGN_SEED=$2; export DF_GAMEPLAY_CAMPAIGN_SEED=$2; shift 2 ;;
    --status) CAMPAIGN_STATUS=1; shift ;;
    --no-build) NO_BUILD=1; shift ;;
    --harbor) HARBOR_REQUESTED=1; export DF_GAMEPLAY_ADVENTURE=harbor; shift ;;
    --seed)
      if test "$#" -lt 2; then usage >&2; exit 2; fi
      ADVENTURE_SEED_REQUESTED=1
      export DF_GAMEPLAY_ADVENTURE_SEED=$2
      shift 2
      ;;
    --database)
      if test "$#" -lt 2; then usage >&2; exit 2; fi
      DATABASE=$2
      shift 2
      ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done


if test -z "$CAMPAIGN" && { test "$CAMPAIGN_STATUS" -eq 1 || test -n "$CAMPAIGN_SEED"; }; then
  echo '--status and --campaign-seed require --campaign' >&2; exit 2
fi
if test -n "$CAMPAIGN"; then
  if test "$HARBOR_REQUESTED" -eq 1 || test "$ADVENTURE_SEED_REQUESTED" -eq 1; then echo '--campaign cannot combine with --harbor or --seed; use --campaign-seed' >&2; exit 2; fi
  if test -n "$DATABASE"; then echo '--campaign chooses its database; omit --database' >&2; exit 2; fi
  mkdir -p "$TMPDIR"
  cd "$ROOT"
  if test "$NO_BUILD" -eq 0; then cargo build --offline --locked -j 3 -p df-tools --bin df-transport-fixture; fi
  if test "$CAMPAIGN_STATUS" -eq 1; then
    "$CARGO_TARGET_DIR/debug/df-transport-fixture" --campaign-status "$CAMPAIGN"
    exit 0
  fi
  if test -n "$CAMPAIGN_SEED"; then
    CAMPAIGN_SELECTION=$("$CARGO_TARGET_DIR/debug/df-transport-fixture" --campaign-run "$CAMPAIGN" "$DF_GAMEPLAY_ADVENTURE_TABLES" "$CAMPAIGN_SEED")
  else
    CAMPAIGN_SELECTION=$("$CARGO_TARGET_DIR/debug/df-transport-fixture" --campaign-run "$CAMPAIGN" "$DF_GAMEPLAY_ADVENTURE_TABLES")
  fi
  DATABASE=$(printf '%s\n' "$CAMPAIGN_SELECTION" | sed -n '1s/^DATABASE=//p')
  printf '%s\n' "$CAMPAIGN_SELECTION" | sed '1d'
fi

if test -z "$DATABASE"; then
  DATABASE="df_gameplay_demo_play_$(date -u +%Y%m%d%H%M%S)"
fi
case "$DATABASE" in
  df_gameplay_demo_*) ;;
  *) echo 'invalid database name: expected df_gameplay_demo_<name> with only letters, digits, or underscores' >&2; exit 1 ;;
esac
case "$DATABASE" in
  *[!A-Za-z0-9_-]*) echo 'invalid database name: expected df_gameplay_demo_<name> with only letters, digits, or underscores' >&2; exit 1 ;;
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

# Spoken narration: take only the ElevenLabs key from the ignored .env, never printing it.
if test -z "${ELEVENLABS_API_KEY:-}" && test -f "$ROOT/.env"; then
  ELEVENLABS_API_KEY=$(sed -n 's/^ELEVENLABS_API_KEY=//p' "$ROOT/.env" | head -n 1)
  if test -n "$ELEVENLABS_API_KEY"; then export ELEVENLABS_API_KEY; else unset ELEVENLABS_API_KEY; fi
fi
export DF_GAMEPLAY_VOICE_CACHE=${DF_GAMEPLAY_VOICE_CACHE:-$ARTIFACT_ROOT/tmp/play/voice-cache}
# Optional scan settings, passed through to the server when set: serve downloaded splat scans from
# DF_GAMEPLAY_SPLAT_DIR and stage every fight on the scene DF_GAMEPLAY_BATTLE_SCENE (see development/README.md).
if test "${DF_GAMEPLAY_SPLAT_DIR+x}" = x; then export DF_GAMEPLAY_SPLAT_DIR; echo "Splat scans are served from $DF_GAMEPLAY_SPLAT_DIR"; fi
if test "${DF_GAMEPLAY_BATTLE_SCENE+x}" = x; then export DF_GAMEPLAY_BATTLE_SCENE; echo "Every fight is staged on battle scene $DF_GAMEPLAY_BATTLE_SCENE"; fi
if test "${DF_GAMEPLAY_BATTLE_FPS+x}" = x; then export DF_GAMEPLAY_BATTLE_FPS; echo "The battlefield presents at $DF_GAMEPLAY_BATTLE_FPS fps"; fi
cd "$ROOT"
if test "$NO_BUILD" -eq 0; then
  cargo build --locked --offline -j 3 -p df-tools --bin df-transport-fixture
  cargo build --locked --offline -j 3 -p df-tools --lib --target wasm32-unknown-unknown
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
  "$PG/pg_ctl" -D "$PLAY/pgdata" -l "$PLAY/postgres.log" -o "-p 55517 -h 127.0.0.1 -k $PLAY/sock -c max_connections=120" -w start
  PG_STARTED=1
fi

if test -z "$("$PG/psql" -h 127.0.0.1 -p 55517 -U df_gameplay_demo_admin_20261004 -d postgres -Atc "SELECT 1 FROM pg_database WHERE datname='$DATABASE'")"; then
  "$PG/createdb" -h 127.0.0.1 -p 55517 -U df_gameplay_demo_admin_20261004 -T template0 "$DATABASE"
fi

echo "Playing with database: $DATABASE"
if test "$DF_GAMEPLAY_ADVENTURE" = harbor; then
  echo "Edit content/harbor-journey/tuning.json (or pack.json), then re-run without --database for a new game with the new values."
else
  echo "A fresh database rolls a new adventure; --seed reproduces the chapter. Existing databases resume their stored chapter."
fi
if test -n "$CAMPAIGN"; then
  echo "To resume this campaign later: development/run-gameplay.sh --no-build --campaign $CAMPAIGN"
elif test "$DF_GAMEPLAY_ADVENTURE" = harbor; then
  echo "To resume this game later: development/run-gameplay.sh --no-build --harbor --database $DATABASE"
else
  echo "To resume this game later: development/run-gameplay.sh --no-build --database $DATABASE"
fi
DF_GAMEPLAY_DEMO_DATABASE=$DATABASE "$CARGO_TARGET_DIR/debug/df-transport-fixture" --gameplay-demo "$CARGO_TARGET_DIR/play-web" "$ROOT/assets/ui" &
SERVER_PID=$!
wait "$SERVER_PID" || SERVER_STATUS=$?
SERVER_PID=
exit "$SERVER_STATUS"
