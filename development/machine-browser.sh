#!/usr/bin/env bash
# Repeatable browser round trip of the machine view and intent (plan O.16 W1).
#
# Builds the Rust/WASM client (crates/df-rpc-bridge/examples/machine_browser.rs) and the native
# server that carries it, serves them on an ephemeral loopback port, loads the page in a headless
# Chromium browser and prints PASS or FAIL from the verdict the page reports to the server. Exit status 0 is a
# pass. The browser is Edge or Chrome, found on the usual Windows and Linux paths; set BROWSER to
# use another. Nothing is installed and no network is used beyond loopback.
#
# Usage: development/machine-browser.sh            build, serve, load, report
#        development/machine-browser.sh --keep     also leave the server running and print its URL
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"

export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/artifacts/build/start-s00}
export CARGO_NET_OFFLINE=${CARGO_NET_OFFLINE:-true}
OUT_DIRECTORY="$ROOT/artifacts/tmp/machine-browser"
LOG="$OUT_DIRECTORY/server.log"
PROFILE="$OUT_DIRECTORY/profile"
# The generator must match the wasm-bindgen crate in Cargo.lock.
WASM_BINDGEN_VERSION=0.2.129
# How long the page may take to finish its five steps, in seconds.
PAGE_TIMEOUT_SECONDS=${PAGE_TIMEOUT_SECONDS:-60}
KEEP=false
if [[ "${1:-}" == "--keep" ]]; then KEEP=true; fi

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

generator=${WASM_BINDGEN:-$ROOT/artifacts/cache/cargo/bin/wasm-bindgen}
if [[ ! -x "$generator" && ! -x "$generator.exe" ]]; then generator=$(command -v wasm-bindgen || true); fi
[[ -n "$generator" ]] || fail "wasm-bindgen $WASM_BINDGEN_VERSION not found (see development/playground.sh)"
found=$("$generator" --version | tr -d '\r')
[[ "$found" == "wasm-bindgen $WASM_BINDGEN_VERSION" ]] ||
  fail "wrong wasm-bindgen: expected $WASM_BINDGEN_VERSION from Cargo.lock, found $found"

browser=${BROWSER:-}
if [[ -z "$browser" ]]; then
  for candidate in \
    "/c/Program Files (x86)/Microsoft/Edge/Application/msedge.exe" \
    "/c/Program Files/Microsoft/Edge/Application/msedge.exe" \
    "/c/Program Files/Google/Chrome/Application/chrome.exe" \
    "/c/Program Files (x86)/Google/Chrome/Application/chrome.exe" \
    "$(command -v google-chrome || true)" "$(command -v chromium || true)" \
    "$(command -v chromium-browser || true)" "$(command -v microsoft-edge || true)"; do
    if [[ -n "$candidate" && -x "$candidate" ]]; then browser=$candidate; break; fi
  done
fi
[[ -n "$browser" ]] || fail "no Chromium browser found; set BROWSER"

mkdir -p "$OUT_DIRECTORY"
echo "building the WASM client and the server"
cargo build --offline -p df-rpc-bridge --example machine_browser --target wasm32-unknown-unknown
cargo build --offline -p df-rpc-bridge --example machine_bridge_server
"$generator" --target web --no-typescript --out-dir "$OUT_DIRECTORY" \
  "$CARGO_TARGET_DIR/wasm32-unknown-unknown/debug/examples/machine_browser.wasm"

server_binary="$CARGO_TARGET_DIR/debug/examples/machine_bridge_server"
[[ -x "$server_binary" ]] || server_binary="$server_binary.exe"
: >"$LOG"
"$server_binary" "$OUT_DIRECTORY" >"$LOG" 2>&1 &
server_pid=$!
stop_server() {
  if [[ "$KEEP" == false ]]; then kill "$server_pid" 2>/dev/null || true; fi
}
trap stop_server EXIT

url=""
for _ in $(seq 1 100); do
  url=$(sed -n 's/^machine bridge ready at \(http:[^ ]*\).*/\1/p' "$LOG" | tr -d '\r' | head -n 1)
  if [[ -n "$url" ]]; then break; fi
  kill -0 "$server_pid" 2>/dev/null || fail "the server stopped: $(cat "$LOG")"
  sleep 0.1
done
[[ -n "$url" ]] || fail "the server did not report an address"
echo "serving $url"

profile_argument=$PROFILE
if command -v cygpath >/dev/null 2>&1; then profile_argument=$(cygpath -w "$PROFILE"); fi
rm -rf "$PROFILE" 2>/dev/null || true
# The page tells the server its verdict (GET /report/pass or /report/fail), which the server prints,
# so no browser console or stdout is needed. The browser is started in the background and only its
# own process tree is stopped afterwards.
"$browser" --headless=new --disable-gpu --no-first-run --no-default-browser-check \
  --user-data-dir="$profile_argument" "$url" >/dev/null 2>&1 &
browser_pid=$!
stop_browser() {
  # Edge hands its window to a child process, so the process started above may already be gone.
  # Stop exactly the processes that carry this run's profile directory, and no other browser.
  if command -v powershell.exe >/dev/null 2>&1; then
    powershell.exe -NoProfile -Command 'Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -like "*--user-data-dir=*machine-browser*profile*" -and $_.ProcessId -ne $PID } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }' >/dev/null 2>&1 || true
  else
    pkill -f "$PROFILE" 2>/dev/null || true
  fi
  kill "$browser_pid" 2>/dev/null || true
}
trap 'stop_browser; stop_server' EXIT
verdict=""
for _ in $(seq 1 $((PAGE_TIMEOUT_SECONDS * 10))); do
  verdict=$(sed -n 's/^machine bridge report: \(pass\|fail\).*/\1/p' "$LOG" | tr -d '\r' | head -n 1)
  if [[ -n "$verdict" ]]; then break; fi
  sleep 0.1
done
stop_browser
sleep 1
rm -rf "$PROFILE" 2>/dev/null || true

if [[ "$verdict" == "pass" ]]; then
  echo "PASS: the machine view and intent crossed the bridge in $(basename "$browser")"
  if [[ "$KEEP" == true ]]; then echo "server left running at $url (pid $server_pid)"; fi
  exit 0
fi
fail "the page reported '${verdict:-nothing within ${PAGE_TIMEOUT_SECONDS}s}'; server log: $(cat "$LOG")"
