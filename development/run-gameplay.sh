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
RUN_DIR="$PLAY/run"
STOP_FILE="$RUN_DIR/stop.request"
# Windows under Git Bash reports MINGW64/MSYS/CYGWIN here; the game and
# PostgreSQL binaries are native Windows programs with a `.exe` suffix.
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) WINDOWS=1 ;;
  *) WINDOWS=0 ;;
esac
if test "$WINDOWS" -eq 1; then EXE=.exe; else EXE=; fi
# Storyteller (text model): export only OPENAI_API_KEY from the gitignored .env, never echoed.
if test -z "${OPENAI_API_KEY:-}" && test -f "$ROOT/.env"; then
  OPENAI_API_KEY=$(sed -n 's/^OPENAI_API_KEY=//p' "$ROOT/.env" | head -n 1 | tr -d "\"'\r")
  if test -n "$OPENAI_API_KEY"; then export OPENAI_API_KEY; else unset OPENAI_API_KEY; fi
fi
export DF_GAMEPLAY_TEXT_CACHE=${DF_GAMEPLAY_TEXT_CACHE:-$PLAY/text-cache}
DATABASE=
NO_BUILD=0
NEW_GAME=0
CAMPAIGN=
CAMPAIGN_SEED=
CAMPAIGN_STATUS=0
HARBOR_REQUESTED=0
ADVENTURE_SEED_REQUESTED=0

LAN=0

usage() {
  echo 'usage: run-gameplay.sh [--no-build] [--lan] [--new-game] [--database <name>] [--seed <decimal-or-hex-u64>] [--harbor] [--campaign <id>] [--campaign-seed <n>] [--status] [--test-fight <1-4>] [--test-foe <creature id>]'
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
    --test-fight)
      if test "$#" -lt 2; then usage >&2; exit 2; fi
      export DF_GAMEPLAY_TEST_FIGHT=$2; shift 2 ;;
    --test-foe)
      if test "$#" -lt 2; then usage >&2; exit 2; fi
      export DF_GAMEPLAY_TEST_FOE=$2; shift 2 ;;
    --no-build) NO_BUILD=1; shift ;;
    --new-game) NEW_GAME=1; shift ;;
    --lan) LAN=1; shift ;;
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
  if test "$NEW_GAME" -eq 1; then echo '--new-game cannot combine with --campaign' >&2; exit 2; fi
  if test "$HARBOR_REQUESTED" -eq 1 || test "$ADVENTURE_SEED_REQUESTED" -eq 1; then echo '--campaign cannot combine with --harbor or --seed; use --campaign-seed' >&2; exit 2; fi
  if test -n "$DATABASE"; then echo '--campaign chooses its database; omit --database' >&2; exit 2; fi
  mkdir -p "$TMPDIR"
  cd "$ROOT"
  if test "$NO_BUILD" -eq 0; then cargo build --offline --locked -j 3 -p df-tools --bin df-transport-fixture; fi
  if test "$CAMPAIGN_STATUS" -eq 1; then
    "$CARGO_TARGET_DIR/debug/df-transport-fixture$EXE" --campaign-status "$CAMPAIGN"
    exit 0
  fi
  if test -n "$CAMPAIGN_SEED"; then
    CAMPAIGN_SELECTION=$("$CARGO_TARGET_DIR/debug/df-transport-fixture$EXE" --campaign-run "$CAMPAIGN" "$DF_GAMEPLAY_ADVENTURE_TABLES" "$CAMPAIGN_SEED")
  else
    CAMPAIGN_SELECTION=$("$CARGO_TARGET_DIR/debug/df-transport-fixture$EXE" --campaign-run "$CAMPAIGN" "$DF_GAMEPLAY_ADVENTURE_TABLES")
  fi
  DATABASE=$(printf '%s\n' "$CAMPAIGN_SELECTION" | sed -n '1s/^DATABASE=//p')
  printf '%s\n' "$CAMPAIGN_SELECTION" | sed '1d'
fi

if test -z "$DATABASE"; then
  if test "$NEW_GAME" -eq 1; then
    DATABASE="df_gameplay_demo_play_$(date -u +%Y%m%d%H%M%S)_$$"
  else
    DATABASE="df_gameplay_demo_play_$(date -u +%Y%m%d%H%M%S)"
  fi
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
# lsof is absent on Windows, where netstat reports the owning PID instead.
if command -v lsof >/dev/null 2>&1; then HAVE_LSOF=1; else HAVE_LSOF=0; fi
if test "$HAVE_LSOF" -eq 0 && ! command -v netstat >/dev/null 2>&1; then
  echo 'missing lsof and netstat: required to check listening ports' >&2; exit 1
fi
port_listener() {
  # Prints what holds a listening TCP port, or nothing when it is free.
  if test "$HAVE_LSOF" -eq 1; then
    lsof -nP -iTCP:"$1" -sTCP:LISTEN 2>/dev/null || true
  else
    netstat -ano 2>/dev/null | grep -E "[:.]$1[[:space:]]" | grep LISTENING || true
  fi
}
if test "$NO_BUILD" -eq 0; then
  if test ! -x "$CARGO_HOME/bin/wasm-bindgen"; then echo "missing wasm-bindgen: $CARGO_HOME/bin/wasm-bindgen" >&2; exit 1; fi
  # Native Windows tools print CRLF; strip it before comparing.
  WASM_BINDGEN_VERSION=$("$CARGO_HOME/bin/wasm-bindgen" --version | tr -d '\r')
  if test "$WASM_BINDGEN_VERSION" != 'wasm-bindgen 0.2.129'; then echo "wrong wasm-bindgen version: expected wasm-bindgen 0.2.129, found $WASM_BINDGEN_VERSION" >&2; exit 1; fi
fi
for PG_TOOL in initdb pg_ctl createdb psql pg_isready; do
  if test ! -x "$PG/$PG_TOOL"; then echo "missing PostgreSQL tool: $PG/$PG_TOOL" >&2; exit 1; fi
done

# --lan: the game server listens on every interface, and phones on this network join through this
# machine's address (DF_GAMEPLAY_PUBLIC_URL). Without a LAN address it stays on loopback.
if test "$LAN" -eq 1; then
  LAN_ADDRESS=
  if test "${DF_GAMEPLAY_PUBLIC_URL+x}" != x; then
    case "$(uname -s)" in
      MINGW*|MSYS*|CYGWIN*)
        # First private IPv4 from Windows ipconfig (CRLF stripped).
        LAN_ADDRESS=$(ipconfig 2>/dev/null | tr -d '\r' | awk '/IPv4 Address/ {print $NF}' | grep -E '^(192\.168\.|10\.|172\.(1[6-9]|2[0-9]|3[01])\.)' | head -n 1 || true)
        ;;
      Linux*)
        LAN_ADDRESS=$(hostname -I 2>/dev/null | awk '{print $1}' || true)
        ;;
      *)
        LAN_ADDRESS=$(ipconfig getifaddr en0 2>/dev/null || true)
        if test -z "$LAN_ADDRESS"; then LAN_ADDRESS=$(ipconfig getifaddr en1 2>/dev/null || true); fi
        ;;
    esac
    if test -z "$LAN_ADDRESS"; then echo 'no LAN address found: the server stays on loopback and phones cannot join' >&2; fi
  fi
  if test -n "$LAN_ADDRESS" || test "${DF_GAMEPLAY_PUBLIC_URL+x}" = x; then
    if test "${DF_GAMEPLAY_LISTEN+x}" != x; then export DF_GAMEPLAY_LISTEN=0.0.0.0:63309; fi
  fi
  if test -n "$LAN_ADDRESS"; then
    export DF_GAMEPLAY_PUBLIC_URL="http://$LAN_ADDRESS:${DF_GAMEPLAY_LISTEN##*:}"
    echo "LAN: phones on this network join through $DF_GAMEPLAY_PUBLIC_URL"
  fi
fi

LISTEN=${DF_GAMEPLAY_LISTEN:-127.0.0.1:63309}
GAME_PORT=${LISTEN##*:}
GAME_HOLDER=$(port_listener "$GAME_PORT")
if test -n "$GAME_HOLDER"; then
  echo "game port $GAME_PORT is already in use:" >&2
  echo "$GAME_HOLDER" >&2
  exit 1
fi

PG_HOLDER=$(port_listener 55517)
PG_ALREADY_RUNNING=0
if test -n "$PG_HOLDER"; then
  if test "$HAVE_LSOF" -eq 1; then
    case "$PG_HOLDER" in
      *postgres*)
        if "$PG/pg_ctl" -D "$PLAY/pgdata" status >/dev/null 2>&1; then PG_ALREADY_RUNNING=1
        else echo 'PostgreSQL port 55517 is held by another process:' >&2; echo "$PG_HOLDER" >&2; exit 1
        fi
        ;;
      *) echo 'PostgreSQL port 55517 is held by another process:' >&2; echo "$PG_HOLDER" >&2; exit 1 ;;
    esac
  elif "$PG/pg_ctl" -D "$PLAY/pgdata" status >/dev/null 2>&1; then
    # netstat names no process, so our own running cluster is the decider.
    PG_ALREADY_RUNNING=1
  else
    echo 'PostgreSQL port 55517 is held by another process:' >&2; echo "$PG_HOLDER" >&2; exit 1
  fi
fi

mkdir -p "$TMPDIR" "$PLAY"
chmod 700 "$PLAY"
mkdir -p "$RUN_DIR"
chmod 700 "$RUN_DIR"
# A previous interrupted run may leave the marker as an empty directory (some
# Windows/MSYS file operations create it that way). Remove only that directory;
# never recurse through the run directory. A non-empty directory is left intact
# and reported so an operator can inspect it.
if test -d "$STOP_FILE" && test ! -L "$STOP_FILE"; then
  if ! rmdir "$STOP_FILE"; then
    echo "stop marker is a non-empty directory: $STOP_FILE" >&2
    exit 1
  fi
elif test -e "$STOP_FILE" || test -L "$STOP_FILE"; then
  rm -f "$STOP_FILE"
fi

# Approach model: take only its credential from the ignored .env, never printing it.
if test -z "${DF_JEV_API_KEY:-}" && test -f "$ROOT/.env"; then
  DF_JEV_API_KEY=$(sed -n 's/^DF_JEV_API_KEY=//p' "$ROOT/.env" | head -n 1 | tr -d "\"'\r")
  if test -n "$DF_JEV_API_KEY"; then export DF_JEV_API_KEY; else unset DF_JEV_API_KEY; fi
fi
# Home video (PLAY-V10): take only the fal key from the ignored .env, never printing it.
if test -z "${FAL_KEY:-}" && test -f "$ROOT/.env"; then
  FAL_KEY=$(sed -n 's/^FAL_KEY=//p' "$ROOT/.env" | head -n 1 | tr -d "\"'\r")
  if test -z "$FAL_KEY"; then
    FAL_KEY=$(sed -n 's/^DF_FAL_KEY=//p' "$ROOT/.env" | head -n 1 | tr -d "\"'\r")
  fi
  if test -n "$FAL_KEY"; then export FAL_KEY; else unset FAL_KEY; fi
fi
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

if test ! -x "$CARGO_TARGET_DIR/debug/df-transport-fixture$EXE"; then echo "missing gameplay server binary: $CARGO_TARGET_DIR/debug/df-transport-fixture$EXE" >&2; exit 1; fi
if test ! -d "$CARGO_TARGET_DIR/play-web" || test -z "$(find "$CARGO_TARGET_DIR/play-web" -type f -print -quit)"; then echo "missing gameplay web files: $CARGO_TARGET_DIR/play-web" >&2; exit 1; fi

SERVER_PID=
PG_STARTED=0
CLEANED=0
SERVER_STATUS=0
cleanup() {
  if test "$CLEANED" -eq 1; then return; fi
  CLEANED=1
  if test -n "$SERVER_PID" && kill -0 "$SERVER_PID" 2>/dev/null; then
    : > "$STOP_FILE"
    STOP_WAIT=0
    while kill -0 "$SERVER_PID" 2>/dev/null && test "$STOP_WAIT" -lt 20; do
      sleep 1
      STOP_WAIT=$((STOP_WAIT + 1))
    done
    if kill -0 "$SERVER_PID" 2>/dev/null; then
      echo 'Graceful stop timed out; hard kill means the game will not be resumable for five minutes.' >&2
      kill -KILL "$SERVER_PID" 2>/dev/null || true
    fi
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
  # The game connects over TCP, so on Windows the Unix socket directory is
  # left out: its path would not survive Git Bash quoting into `-o` anyway.
  if test "$WINDOWS" -eq 1; then
    PG_SOCKET_OPTS=
  else
    mkdir -p "$PLAY/sock"
    PG_SOCKET_OPTS=" -k $PLAY/sock"
  fi
  "$PG/pg_ctl" -D "$PLAY/pgdata" -l "$PLAY/postgres.log" -o "-p 55517 -h 127.0.0.1$PG_SOCKET_OPTS -c max_connections=120" -w start
  PG_STARTED=1
fi

DATABASE_EXISTS=$("$PG/psql" -h 127.0.0.1 -p 55517 -U df_gameplay_demo_admin_20261004 -d postgres -Atc "SELECT 1 FROM pg_database WHERE datname='$DATABASE'")
if test -n "$DATABASE_EXISTS"; then
  if test "$NEW_GAME" -eq 1; then echo "new game database already exists: $DATABASE" >&2; exit 1; fi
else
  "$PG/createdb" -h 127.0.0.1 -p 55517 -U df_gameplay_demo_admin_20261004 -T template0 "$DATABASE"
fi

echo "Playing with database: $DATABASE"
if test "$NEW_GAME" -eq 1; then
  echo 'A new game will start in a fresh database.'
elif test "$DF_GAMEPLAY_ADVENTURE" = harbor; then
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
DF_GAMEPLAY_DEMO_DATABASE=$DATABASE DF_GAMEPLAY_STOP_FILE="$STOP_FILE" "$CARGO_TARGET_DIR/debug/df-transport-fixture$EXE" --gameplay-demo "$CARGO_TARGET_DIR/play-web" "$ROOT/assets/ui" &
SERVER_PID=$!
wait "$SERVER_PID" || SERVER_STATUS=$?
SERVER_ENDED_AT=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
echo "Server ended at $SERVER_ENDED_AT with exit code $SERVER_STATUS"
SERVER_PID=
exit "$SERVER_STATUS"
