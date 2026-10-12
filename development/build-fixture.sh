#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
ARTIFACT_ROOT=${DUNGEONFLUX_ARTIFACT_ROOT:-$ROOT/artifacts}
export RUSTUP_HOME="$ARTIFACT_ROOT/cache/rustup"
export CARGO_HOME="$ARTIFACT_ROOT/cache/cargo"
export CARGO_TARGET_DIR=${DUNGEONFLUX_BUILD_ROOT:-$ARTIFACT_ROOT/build/start-s00}
export TMPDIR=${DUNGEONFLUX_TMP_ROOT:-$ARTIFACT_ROOT/tmp/start-s00}
export PATH="$CARGO_HOME/bin:$PATH"
export CARGO_BUILD_JOBS=1
# Windows under Git Bash reports MINGW64/MSYS/CYGWIN here; the fixture binary
# is a native Windows program with a `.exe` suffix, staged without it.
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) WINDOWS=1 ;;
  *) WINDOWS=0 ;;
esac
if test "$WINDOWS" -eq 1; then EXE=.exe; else EXE=; fi
native_path() {
  # Paths handed to the native fixture binary must be native absolute paths on
  # Windows; convert explicitly when cygpath exists.
  if test "$WINDOWS" -eq 1 && command -v cygpath >/dev/null 2>&1; then
    cygpath -w -- "$1"
  else
    printf '%s\n' "$1"
  fi
}
DF_FIXTURE_BUILD=$(git -C "$ROOT" rev-parse HEAD)
if test -n "$(git -C "$ROOT" status --porcelain -- Cargo.toml Cargo.lock rust-toolchain.toml .cargo crates development/build-fixture.sh)"; then
  DF_FIXTURE_BUILD="$DF_FIXTURE_BUILD-dirty"
fi
export DF_FIXTURE_BUILD
mkdir -p "$TMPDIR"
chmod 700 "$TMPDIR"
cd "$ROOT"
case "${1:-build}" in
  cargo) shift; exec cargo "$@" ;;
  build)
    # Public, nonsecret inputs are explicit. Observed provenance is not inferred
    # from filenames; assets are the exact resolved closure in the content file.
    : "${DUNGEONFLUX_BUILD_EXPECTED_IDENTITY:?required public identity record}"
    : "${DUNGEONFLUX_BUILD_OBSERVED_IDENTITY:?required observed identity record}"
    : "${DUNGEONFLUX_BUILD_CONFIGURATION:?required public configuration bytes}"
    : "${DUNGEONFLUX_BUILD_CONTENT_MANIFEST:?required resolved content manifest}"
    : "${DUNGEONFLUX_BUILD_ASSETS_ROOT:?required resolved static asset directory}"
    cargo build --locked -p df-tools --bin df-transport-fixture
    cargo build --locked -p df-tools --lib --target wasm32-unknown-unknown
    # Native Windows tools print CRLF; strip it before comparing.
    test "$(wasm-bindgen --version | tr -d '\r')" = "wasm-bindgen 0.2.129"
    STAGING=$(mktemp -d "$TMPDIR/build-set-input.XXXXXXXX")
    # mkdir -m has no effect on Windows, where it errors out instead.
    if test "$WINDOWS" -eq 1; then
      mkdir "$STAGING/native" "$STAGING/web"
    else
      mkdir -m 700 "$STAGING/native" "$STAGING/web"
    fi
    cp "$CARGO_TARGET_DIR/debug/df-transport-fixture$EXE" "$STAGING/native/df-transport-fixture"
    cp "$CARGO_TARGET_DIR/wasm32-unknown-unknown/debug/df_tools.wasm" "$STAGING/compiler.wasm"
    wasm-bindgen --target web --out-name df_tools --out-dir "$STAGING/web" "$STAGING/compiler.wasm"
    cp "$DUNGEONFLUX_BUILD_OBSERVED_IDENTITY" "$STAGING/identity.txt"
    cp "$DUNGEONFLUX_BUILD_CONFIGURATION" "$STAGING/web/configuration.bin"
    cp "$DUNGEONFLUX_BUILD_CONTENT_MANIFEST" "$STAGING/web/content-manifest.txt"
    cp -R "$DUNGEONFLUX_BUILD_ASSETS_ROOT" "$STAGING/web/assets"
    MANIFEST="$TMPDIR/$(basename "$STAGING").manifest"
    STAGING_NATIVE=$(native_path "$STAGING")
    IDENTITY_NATIVE=$(native_path "$DUNGEONFLUX_BUILD_EXPECTED_IDENTITY")
    MANIFEST_NATIVE=$(native_path "$MANIFEST")
    PUBLISHED_NATIVE=$(native_path "$CARGO_TARGET_DIR/published")
    # The staged binary keeps its extensionless manifest name on every
    # platform; Windows starts it by full path (see D7 in the task brief).
    "$STAGING/native/df-transport-fixture" --seal-build-set "$STAGING_NATIVE" "$IDENTITY_NATIVE" "$MANIFEST_NATIVE"
    "$STAGING/native/df-transport-fixture" --publish-build-set "$PUBLISHED_NATIVE" "$STAGING_NATIVE" "$MANIFEST_NATIVE"
    ;;
  serve) exec "$CARGO_TARGET_DIR/debug/df-transport-fixture$EXE" --launch-build-set "$(native_path "$CARGO_TARGET_DIR/published")" "${DUNGEONFLUX_PREVIEW_PORT:-43180}" ;;
  *) echo 'usage: build-fixture.sh [build|serve|cargo arguments...]' >&2; exit 2 ;;
esac
