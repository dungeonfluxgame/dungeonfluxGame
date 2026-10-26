#!/bin/bash
# build-web.sh: builds the account app bundle (index.html, assets/app.css, boot.js, the wasm module).
# Run from the repo root: bash crates/df-account-app/build-web.sh
# OUT overrides the output directory (default artifacts/build/account-app-web). The finished bundle
# replaces the old one atomically: the old directory is renamed aside, the staged one renamed into
# place, then the old one removed, so a server never sees a half-written bundle.
# If the bundled rust-lld cannot execute, set CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER to an
# installed wasm-ld executable, for example: CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER='C:/Program Files/LLVM/bin/wasm-ld.exe'.
set -euo pipefail

R=/c/Users/mreca/Desktop/dungeonfluxGame
cd "$R"
APP=crates/df-account-app
OUT=${OUT:-artifacts/build/account-app-web}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-C:/Users/mreca/Desktop/dungeonfluxGame/artifacts/build/agents-p}
export RUSTUP_HOME=C:/Users/mreca/Desktop/dungeonfluxGame/artifacts/cache/rustup
export CARGO_HOME=C:/Users/mreca/Desktop/dungeonfluxGame/artifacts/cache/cargo
export CARGO_BUILD_JOBS=2
# Apply size settings to this browser build without changing native release defaults.
export CARGO_PROFILE_RELEASE_OPT_LEVEL=${CARGO_PROFILE_RELEASE_OPT_LEVEL:-z}
export CARGO_PROFILE_RELEASE_LTO=${CARGO_PROFILE_RELEASE_LTO:-true}
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=${CARGO_PROFILE_RELEASE_CODEGEN_UNITS:-1}
export CARGO_PROFILE_RELEASE_PANIC=${CARGO_PROFILE_RELEASE_PANIC:-abort}
export CARGO_PROFILE_RELEASE_STRIP=${CARGO_PROFILE_RELEASE_STRIP:-true}
BINDGEN=artifacts/cache/cargo/bin/wasm-bindgen
WANTED_BINDGEN=0.2.129
BUDGET_BYTES=$((700 * 1024))

fail() { echo "build-web: $*" >&2; exit 1; }

# 1. The tool must match the wasm-bindgen crate in Cargo.lock.
version=$("$BINDGEN" --version | awk '{print $2}')
[ "$version" = "$WANTED_BINDGEN" ] || fail "wasm-bindgen is $version, the lockfile pins $WANTED_BINDGEN"

# 2. Compile. Until the crate is a workspace member it builds in the private shadow workspace.
if grep -q '"crates/df-account-app"' Cargo.toml; then
  cargo build -p df-account-app --lib --release --target wasm32-unknown-unknown
else
  bash artifacts/tmp/agents/P/shadow-app.sh build --lib --release --target wasm32-unknown-unknown
fi
wasm="$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/df_account_app.wasm"
[ -f "$wasm" ] || fail "missing $wasm"

# 3. Stage next to the final directory so the last step is a rename on one filesystem.
mkdir -p "$(dirname "$OUT")"
stage=$(mktemp -d "${OUT}.stage.XXXXXX")
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/assets"

"$BINDGEN" --target web --no-typescript --out-name account_app --out-dir "$stage/assets" "$wasm"

if command -v wasm-opt >/dev/null 2>&1; then
  # The feature flags are what rustc emits for wasm32-unknown-unknown by default, plus GC
  # type merging (Chrome/Edge 119+, Firefox 120+, Safari 18.4+), which the size budget needs.
  # Unlimited single-caller inlining grows the large view and reply dispatchers.
  if wasm-opt -Oz --one-caller-inline-max-function-size=32 \
    --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
    --enable-mutable-globals --enable-reference-types --enable-multivalue --enable-gc \
    "$stage/assets/account_app_bg.wasm" -o "$stage/assets/account_app_bg.opt.wasm"; then
    mv "$stage/assets/account_app_bg.opt.wasm" "$stage/assets/account_app_bg.wasm"
  else
    rm -f "$stage/assets/account_app_bg.opt.wasm"
    echo "build-web: wasm-opt failed, keeping the unoptimised module" >&2
  fi
else
  echo "build-web: wasm-opt not on PATH, skipping -Oz"
fi

cat $APP/web/css/base.css $APP/web/css/customer.css $APP/web/css/campaign.css \
  $APP/web/css/admin.css > "$stage/assets/app.css"

# Generated glue, never hand-edited, never committed: the CSP forbids inline script.
printf 'import init from "/app/assets/account_app.js";\ninit({ module_or_path: "/app/assets/account_app_bg.wasm" });\n' \
  > "$stage/assets/boot.js"

cp $APP/web/index.html "$stage/index.html"
cp $APP/web/google-g.svg "$stage/assets/google-g.svg"
cp docs/assets/establishing-harbor-canal-rowboat-bridge.webp "$stage/assets/landing.webp"

# 4. Size budget.
size=$(stat -c %s "$stage/assets/account_app_bg.wasm")
echo "build-web: account_app_bg.wasm $size bytes (budget $BUDGET_BYTES)"
if [ "$size" -gt "$BUDGET_BYTES" ]; then
  echo "build-web: OVER BUDGET. Inspect the wasm profile overrides and use Twiggy to locate size regressions." >&2
  fail "wasm exceeds the $BUDGET_BYTES byte budget"
fi

# 5. Publish atomically.
old=
if [ -e "$OUT" ]; then
  old="${OUT}.old.$$"
  mv "$OUT" "$old"
fi
if ! mv "$stage" "$OUT"; then
  [ -z "$old" ] || mv "$old" "$OUT"
  fail "could not move the staged bundle into place"
fi
trap - EXIT
[ -z "$old" ] || rm -rf "$old"

echo "build-web: bundle at $OUT"
( cd "$OUT" && find . -type f -printf '%8s  %p\n' | sort -k2 )
