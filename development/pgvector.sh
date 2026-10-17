#!/bin/bash
# Installs the pgvector extension into the bundled development PostgreSQL on Windows.
#
# Target: the EDB PostgreSQL 18.6 x64 binaries that run-gameplay.sh starts from
# artifacts/build/postgres-g05/install (override with DUNGEONFLUX_POSTGRES_BIN, the same
# variable run-gameplay.sh honors). The EDB tree ships bin/ lib/ share/ in the install tree but
# its include/ headers only exist in the pinned zip, so this script builds against a throwaway
# copy of include/ and lib/postgres.lib and installs only vector.dll, vector.control and the
# sql files. Usage: pgvector.sh {install|status}. install is idempotent: it does nothing when
# the pinned version is already installed. A first install is safe while the cluster is running
# (nothing has loaded vector.dll yet); changing the pinned version needs the cluster stopped,
# because Windows will not replace a loaded DLL.
#
# Where CREATE EXTENSION happens: NOT here. The extension is created by the game's own schema
# bootstrap (CREATE EXTENSION IF NOT EXISTS vector), because
#   - run-gameplay.sh makes every game database with createdb -T template0, so an extension
#     created at provisioning time (in postgres or template1) would never exist in a game
#     database, and the game owns df_local_demo and its other DDL, with no external migrations;
#   - the game connects as the initdb superuser df_gameplay_demo_admin_20261004, which may create
#     it; pgvector is not marked trusted, so a NOSUPERUSER runtime role (the production shape in
#     planning/c-df-persistence-d03-decision.md) cannot, and its migration role must.
set -euo pipefail

PGVECTOR_VERSION=0.8.7
PGVECTOR_SHA256=cac0b10c360f05b2d521200105ba3697e773d4cd3731f5a915a7e37ebe0bea85
PGVECTOR_URL="https://github.com/pgvector/pgvector/archive/refs/tags/v${PGVECTOR_VERSION}.tar.gz"
PG_VERSION=18.6
PG_ZIP_NAME=postgresql-18.6-5-windows-x64-binaries.zip
PG_ZIP_SHA256=e2246ba91d22345bc3d017586c09ede52d9df180b1eeb480f050445f1cad84e2
SETUP_DEADLINE=900
# Visual Studio component that provides the x64 MSVC compiler and linker.
VS_COMPONENT=Microsoft.VisualStudio.Component.VC.Tools.x86.x64

fail() { printf 'pgvector-g05: %s\n' "$*" >&2; exit 1; }

no_symlink_path() {
  local path=$1 current= part
  local IFS=/
  for part in $path; do
    [[ -n "$part" ]] || continue
    current="$current/$part"
    if [[ -L "$current" ]]; then fail "refusing symlink component: $current"; fi
  done
}

SCRIPT_INPUT=${BASH_SOURCE[0]}
[[ "$SCRIPT_INPUT" = /* ]] || SCRIPT_INPUT="$PWD/$SCRIPT_INPUT"
SCRIPT_DIR=$(cd -- "$(dirname -- "$SCRIPT_INPUT")" && pwd -P)
ROOT=$(cd -- "$SCRIPT_DIR/.." && pwd -P)
ARTIFACT_ROOT=${DUNGEONFLUX_ARTIFACT_ROOT:-$ROOT/artifacts}
PG_BIN=${DUNGEONFLUX_POSTGRES_BIN:-$ARTIFACT_ROOT/build/postgres-g05/install/bin}
PG_ROOT=$(dirname -- "$PG_BIN")
CACHE="$ARTIFACT_ROOT/cache/postgres-g05"
WORK="$ARTIFACT_ROOT/tmp/pgvector-build"
TARBALL="$CACHE/pgvector-${PGVECTOR_VERSION}.tar.gz"
PG_ZIP="$CACHE/$PG_ZIP_NAME"
LIB_DIR="$PG_ROOT/lib"
EXT_DIR="$PG_ROOT/share/extension"

setup_started=0
bounded() {
  local command_limit=$1
  shift
  local remaining=$((SETUP_DEADLINE - (SECONDS - setup_started)))
  (( remaining > 0 )) || fail "install exceeded its ${SETUP_DEADLINE}s total deadline"
  (( command_limit <= remaining )) || command_limit=$remaining
  local status=0
  timeout --kill-after=5 "$command_limit" "$@" || status=$?
  if (( status == 124 )); then fail "command exceeded its ${command_limit}s deadline: $1"; fi
  return "$status"
}

check_sha256() {
  local expected=$1 file=$2 actual
  actual=$(bounded 120 sha256sum "$file") || fail "could not hash $file"
  actual=${actual%% *}
  [[ "$actual" == "$expected" ]] || fail "sha256 mismatch for $file: expected $expected, got $actual"
}

installed_version() {
  # Prints the installed extension default_version, or nothing when pgvector is incomplete.
  local control="$EXT_DIR/vector.control" version
  [[ -f "$LIB_DIR/vector.dll" && -f "$control" ]] || return 0
  version=$(sed -n "s/^default_version = '\\(.*\\)'.*/\\1/p" "$control" | tr -d '\r')
  [[ -f "$EXT_DIR/vector--${version}.sql" ]] || return 0
  printf '%s\n' "$version"
}

# The first two bytes after the PE signature are the machine type; x64 is 0x8664 (bytes 64 86).
require_x64_dll() {
  local dll=$1 offset machine
  offset=$(od -An -t u4 -j 60 -N 4 "$dll" | tr -d ' ')
  machine=$(od -An -t x1 -j $((offset + 4)) -N 2 "$dll" | tr -d ' \n')
  [[ "$machine" == 6486 ]] || fail "built vector.dll is not x64 (PE machine bytes $machine)"
}

find_msvc() {
  # Sets VCVARSALL and VCARCH from the newest Visual Studio with the x64 MSVC tools.
  local pf86 vswhere vs_path host_arch=${PROCESSOR_ARCHITEW6432:-${PROCESSOR_ARCHITECTURE:-}}
  pf86=$(printenv 'ProgramFiles(x86)' || true)
  [[ -n "$pf86" ]] || fail 'ProgramFiles(x86) is not set; cannot locate vswhere'
  vswhere="$(cygpath -u "$pf86")/Microsoft Visual Studio/Installer/vswhere.exe"
  [[ -x "$vswhere" ]] || fail "vswhere.exe not found at $vswhere: install Visual Studio Build Tools with the component $VS_COMPONENT"
  vs_path=$(bounded 30 "$vswhere" -latest -products '*' -requires "$VS_COMPONENT" -property installationPath | tr -d '\r') || fail 'vswhere failed'
  [[ -n "$vs_path" ]] || fail "no Visual Studio with the component $VS_COMPONENT (MSVC x64/x86 build tools); not installing it automatically"
  vs_path=$(cygpath -u "$vs_path")
  VCVARSALL="$vs_path/VC/Auxiliary/Build/vcvarsall.bat"
  [[ -f "$VCVARSALL" ]] || fail "vcvarsall.bat missing under $vs_path: the Visual Studio C++ workload is incomplete"
  local tools
  tools=$(ls -d "$vs_path"/VC/Tools/MSVC/*/ 2>/dev/null | sort | tail -n 1)
  [[ -n "$tools" ]] || fail "no MSVC toolset under $vs_path/VC/Tools/MSVC"
  # An ARM64 host cross-compiles to x64 natively (arm64_x64); an x64 host uses x64.
  if [[ "$host_arch" == ARM64 && -f "${tools}bin/Hostarm64/x64/cl.exe" ]]; then
    VCARCH=arm64_x64
  elif [[ -f "${tools}bin/Hostx64/x64/cl.exe" ]]; then
    VCARCH=x64
  else
    fail "no x64-targeting cl.exe under ${tools}bin (need Hostarm64/x64 or Hostx64/x64): missing component $VS_COMPONENT"
  fi
}

install() {
  setup_started=$SECONDS
  no_symlink_path "$PG_ROOT"
  no_symlink_path "$CACHE"
  no_symlink_path "$WORK"
  for tool in postgres initdb pg_ctl psql; do
    [[ -x "$PG_BIN/$tool" ]] || fail "missing PostgreSQL tool: $PG_BIN/$tool"
  done
  local pg_version
  pg_version=$(bounded 30 "$PG_BIN/postgres" --version | tr -d '\r') || fail 'could not query postgres version'
  [[ "$pg_version" == *"PostgreSQL) $PG_VERSION" ]] || fail "installed PostgreSQL is not $PG_VERSION: $pg_version"
  local current
  current=$(installed_version)
  if [[ "$current" == "$PGVECTOR_VERSION" ]]; then
    printf 'pgvector %s already installed in %s\n' "$PGVECTOR_VERSION" "$PG_ROOT"
    return
  fi
  [[ -d "$EXT_DIR" && -d "$LIB_DIR" ]] || fail "PostgreSQL tree has no lib/ and share/extension: $PG_ROOT"
  [[ -f "$LIB_DIR/postgres.lib" ]] || fail "missing $LIB_DIR/postgres.lib needed to link extensions"

  find_msvc
  printf 'pgvector %s: building with %s (vcvarsall %s)\n' "$PGVECTOR_VERSION" "$VCVARSALL" "$VCARCH"

  mkdir -p "$CACHE"
  if [[ ! -f "$TARBALL" ]]; then
    no_symlink_path "$TARBALL.part"
    bounded 95 curl --proto '=https' --tlsv1.2 -fsSL --max-time 90 "$PGVECTOR_URL" -o "$TARBALL.part"
    check_sha256 "$PGVECTOR_SHA256" "$TARBALL.part"
    mv "$TARBALL.part" "$TARBALL"
  else
    check_sha256 "$PGVECTOR_SHA256" "$TARBALL"
  fi
  [[ -f "$PG_ZIP" ]] || fail "missing pinned PostgreSQL archive (source of include/ headers): $PG_ZIP"
  check_sha256 "$PG_ZIP_SHA256" "$PG_ZIP"

  # Own scratch tree: replace any leftover from an earlier failed build.
  rm -rf "$WORK"
  mkdir -p "$WORK/pgroot" "$WORK/src"
  # The system bsdtar reads zip archives; the Git Bash tar does not.
  bounded 120 /c/Windows/System32/tar.exe -xf "$PG_ZIP" -C "$WORK/pgroot" --strip-components=1 pgsql/include pgsql/lib/postgres.lib
  [[ -f "$WORK/pgroot/include/server/pg_config.h" && -f "$WORK/pgroot/lib/postgres.lib" ]] || fail 'could not extract PostgreSQL headers from the pinned archive'
  bounded 60 tar -xzf "$TARBALL" -C "$WORK/src" --strip-components=1
  [[ -f "$WORK/src/Makefile.win" ]] || fail 'pgvector source archive has no Makefile.win'
  grep -q "^EXTVERSION = ${PGVECTOR_VERSION}\$" "$WORK/src/Makefile.win" || fail "pgvector source is not version $PGVECTOR_VERSION"

  cat > "$WORK/build.bat" <<EOF_BAT
@echo off
call "$(cygpath -w "$VCVARSALL")" $VCARCH || exit /b 1
set "PGROOT=$(cygpath -w "$WORK/pgroot")"
cd /d "$(cygpath -w "$WORK/src")" || exit /b 1
nmake /nologo /F Makefile.win || exit /b 1
EOF_BAT
  local started=$SECONDS
  if ! MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*' bounded 600 cmd.exe /c "$(cygpath -w "$WORK/build.bat")" > "$WORK/build.log" 2>&1; then
    tail -n 25 "$WORK/build.log" >&2 || true
    fail "pgvector build failed; full log: $WORK/build.log"
  fi
  printf 'pgvector %s: built in %ss\n' "$PGVECTOR_VERSION" "$((SECONDS - started))"

  local dll="$WORK/src/vector.dll"
  [[ -f "$dll" && -f "$WORK/src/vector.control" && -f "$WORK/src/sql/vector--${PGVECTOR_VERSION}.sql" ]] || fail "build produced no vector.dll/sql; log: $WORK/build.log"
  require_x64_dll "$dll"

  # Stage next to the destination, then rename, so a failed copy never leaves a partial file.
  cp "$dll" "$LIB_DIR/vector.dll.new"
  mv -f "$LIB_DIR/vector.dll.new" "$LIB_DIR/vector.dll"
  local file
  for file in "$WORK/src/vector.control" "$WORK"/src/sql/vector--*.sql; do
    cp "$file" "$EXT_DIR/$(basename "$file").new"
    mv -f "$EXT_DIR/$(basename "$file").new" "$EXT_DIR/$(basename "$file")"
  done
  [[ $(installed_version) == "$PGVECTOR_VERSION" ]] || fail 'installed files do not report the pinned version after install'
  rm -rf "$WORK"
  printf 'pgvector %s installed into %s\n' "$PGVECTOR_VERSION" "$PG_ROOT"
}

status() {
  local current
  current=$(installed_version)
  if [[ -z "$current" ]]; then
    printf 'pgvector missing from %s\n' "$PG_ROOT"
    return 3
  fi
  printf 'pgvector %s installed in %s (pinned %s)\n' "$current" "$PG_ROOT" "$PGVECTOR_VERSION"
}

case "${1:-}" in
  install) install ;;
  status) status ;;
  *) printf 'usage: %s {install|status}\n' "$0" >&2; exit 2 ;;
esac
