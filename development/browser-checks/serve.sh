#!/bin/sh
# usage: serve.sh <start|stop|bindgen> [database-name]
# Runs this checkout's gameplay demo on port 63342 against the checks' own PostgreSQL (see own-postgres.sh,
# port 55531), never the launcher's on 55517. Starts nothing else. Evaluator tooling for the browser checks in
# this directory, not application code. Layout overrides: DUNGEONFLUX_ARTIFACT_ROOT (build, cache and
# PostgreSQL tools), DUNGEONFLUX_BUILD_ROOT (cargo target), DUNGEONFLUX_ENV_FILE (keys, never printed).
set -eu
WT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
R=${DUNGEONFLUX_ARTIFACT_ROOT:-$WT/artifacts}
O=$R/tmp/browser-checks
mkdir -p "$O"
ENV_FILE=${DUNGEONFLUX_ENV_FILE:-$WT/.env}
TARGET=${DUNGEONFLUX_BUILD_ROOT:-$R/build/play}
export RUSTUP_HOME="$R/cache/rustup" CARGO_HOME="$R/cache/cargo"
export PATH="$CARGO_HOME/bin:$PATH"
PG="${DUNGEONFLUX_POSTGRES_BIN:-$R/build/postgres-g05/install/bin}"
PORT=${PORT:-63342}
PGPORT=${DF_CHECK_PG_PORT:-55531}
export DF_GAMEPLAY_DATABASE_PORT=$PGPORT
WEB="$TARGET/f05-web"
case "$1" in
  bindgen)
    rm -rf "$WEB"; mkdir -p "$WEB"
    wasm-bindgen --target web --out-name df_tools --out-dir "$WEB" "$TARGET/wasm32-unknown-unknown/debug/df_tools.wasm"
    ;;
  stop)
    if test -f "$O/server.pid"; then kill -INT "$(cat "$O/server.pid")" 2>/dev/null || true; rm -f "$O/server.pid"; fi
    ;;
  start)
    DB=${2:-df_gameplay_demo_f05_$(date -u +%H%M%S)}
    if test -z "$("$PG/psql" -h 127.0.0.1 -p "$PGPORT" -U df_gameplay_demo_admin_20261004 -d postgres -Atc "SELECT 1 FROM pg_database WHERE datname='$DB'")"; then
      "$PG/createdb" -h 127.0.0.1 -p "$PGPORT" -U df_gameplay_demo_admin_20261004 -T template0 "$DB"
    fi
    if test "${WITH_STORYTELLER:-0}" = 1; then
      OPENAI_API_KEY=$(sed -n 's/^OPENAI_API_KEY=//p' "$ENV_FILE" | head -n 1 | tr -d "\"'\r")
      export OPENAI_API_KEY
    fi
    if test "${WITH_ELEVEN:-0}" = 1; then
      ELEVENLABS_API_KEY=$(sed -n 's/^ELEVENLABS_API_KEY=//p' "$ENV_FILE" | head -n 1 | tr -d "\"'\r")
      export ELEVENLABS_API_KEY
    fi
    cd "$WT"
    export DF_GAMEPLAY_PACK_DIR="${DF_GAMEPLAY_PACK_DIR:-$WT/content/harbor-journey}"
    export DF_GAMEPLAY_ADVENTURE="${DF_GAMEPLAY_ADVENTURE:-harbor}"
    export DF_GAMEPLAY_VOICE_CASTING="$WT/content/voice-casting.json"
    export DF_GAMEPLAY_DEMO_DATABASE="$DB" DF_GAMEPLAY_LISTEN="127.0.0.1:$PORT"
    export DF_GAMEPLAY_TEXT_CACHE="$O/text-cache" DF_GAMEPLAY_VOICE_CACHE="$O/voice-cache"
    nohup "$TARGET/debug/df-transport-fixture" --gameplay-demo "$WEB" "$WT/assets/ui" > "$O/server-$PORT.log" 2>&1 &
    echo $! > "$O/server.pid"
    for i in $(seq 1 60); do grep -q '^READY' "$O/server-$PORT.log" 2>/dev/null && break; sleep 0.5; done
    grep -E '^(READY|STORYTELLER|VOICE|GAMEPLAY PACK|ADVENTURE)' "$O/server-$PORT.log" | cut -c1-200
    echo "DB=$DB"
    ;;
esac
