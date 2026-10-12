#!/bin/sh
# usage: tv-layout.sh [frames-dir]
# PLAY-V14: the layout check of the shared display in one command, for every merge. Evaluator tooling, not
# application code. It starts this checkout's own PostgreSQL (own-postgres.sh, DF_CHECK_PG_PORT), plays each
# scenario of tv-layout.mjs against a fresh server on PORT (default 63342 as serve.sh), and stops them again.
# It does not build: build the server and the wasm display first (development/run-gameplay.sh does both, or
# cargo build -p df-tools --bin df-transport-fixture and --lib --target wasm32-unknown-unknown, then
# serve.sh bindgen). Exit status 1 when any text overlaps, is covered, is clipped, is cut with an ellipsis or lies
# outside the 5% TV-safe margin, or when a screen state could not be reached. ONLY=lobby,story,fight,... picks
# scenarios; VIEWPORTS=1920x1080 narrows the sizes; DF_DEBUG=1 lists every text box read.
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export PORT=${PORT:-63342}
export DF_BASE=${DF_BASE:-http://127.0.0.1:$PORT}
"$HERE/own-postgres.sh" start
status=0
node "$HERE/tv-layout.mjs" "$@" || status=$?
"$HERE/serve.sh" stop
"$HERE/own-postgres.sh" stop
exit $status
