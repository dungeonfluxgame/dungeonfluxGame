#!/usr/bin/env bash
# Prunes stale generated output from the shared cargo target and old scratch target directories.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"

TARGET="$ROOT/artifacts/build/start-s00"
TMP_ROOT="$ROOT/artifacts/tmp"
DEPS_DIRS=("$TARGET/debug/deps" "$TARGET/wasm32-unknown-unknown/debug/deps")

usage() {
  cat <<'USAGE'
Usage: development/prune-artifacts.sh [--dry-run] [--hours N] [--force]

Deletes stale output from the shared cargo target artifacts/build/start-s00 and
stale *-target / *_target scratch directories under artifacts/tmp.

Options:
  --dry-run    Print the count and MB per category; delete nothing
  --hours N    Age threshold in hours for the deps files (default 2)
  --force      Run even when cargo.exe or rustc.exe is running
  -h, --help   Show this help

Categories:
  1. df_* and df-* files in debug/deps and wasm32-unknown-unknown/debug/deps
     with mtime older than N hours
  2. .exe and .pdb files in those deps directories whose sibling .d file lists a
     crates\df-* source (workspace test and bin targets), older than N hours
  3. Directories directly under artifacts/tmp named *-target or *_target with no
     entry modified in the last 24 hours

Never a candidate: third-party crate files, artifacts/cache, artifacts/playground,
artifacts/build/postgres-*, artifacts/build/imagery-*, development/, and anything
outside artifacts/. Symlinks are not followed or deleted. Each candidate is
rechecked immediately before deletion.
USAGE
}

dry_run=0
force=0
hours=2
while (( $# > 0 )); do
  case "$1" in
    --dry-run) dry_run=1; shift ;;
    --force) force=1; shift ;;
    --hours)
      if [[ $# -lt 2 ]]; then
        echo "prune-artifacts.sh: --hours needs a value" >&2
        exit 2
      fi
      hours=$2
      shift 2
      ;;
    --hours=*) hours=${1#--hours=}; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "prune-artifacts.sh: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done
if ! [[ "$hours" =~ ^[0-9]+$ ]]; then
  echo "prune-artifacts.sh: --hours needs a whole number, got: $hours" >&2
  exit 2
fi
cutoff_min=$(( hours * 60 ))

free_space() {
  df -Pk "$ROOT" | awk 'NR == 2 { printf "%.1f GB", $4 / 1048576 }'
}

# Prints cargo.exe or rustc.exe process lines. Returns 1 when no process lister exists.
running_cargo_lines() {
  if command -v tasklist.exe >/dev/null 2>&1; then
    MSYS_NO_PATHCONV=1 tasklist.exe /FO CSV /NH 2>/dev/null | tr -d '\r' \
      | grep -i -E '^"(cargo|rustc)(\.exe)?"' || true
  elif command -v ps >/dev/null 2>&1; then
    ps -W 2>/dev/null | grep -i -E '(cargo|rustc)(\.exe)?[[:space:]]*$' || true
  else
    return 1
  fi
}

# Emits "size<TAB>path" NUL records for df_ and df- files in the deps directories.
# Files that cargo renames or deletes during the scan are skipped silently.
emit_prefixed_files() {
  local dir
  for dir in "${DEPS_DIRS[@]}"; do
    [[ -d "$dir" ]] || continue
    find "$dir" -maxdepth 1 -type f -mmin "+$cutoff_min" \( -name 'df_*' -o -name 'df-*' \) \
      -printf '%s\t%p\0' 2>/dev/null || true
  done
}

# Emits records for .exe and .pdb files built from workspace sources. The sibling .d file
# (dep-info) lists workspace sources as relative paths such as "src\lib.rs" or
# "crates\df-api\tests\x.rs"; registry and git dependencies list only absolute paths.
emit_workspace_binaries() {
  local dir size path depfile
  for dir in "${DEPS_DIRS[@]}"; do
    [[ -d "$dir" ]] || continue
    while IFS=$'\t' read -r -d '' size path; do
      depfile="${path%.*}.d"
      if [[ -f "$depfile" ]] && grep -q -E '(^|[[:space:]])[^[:space:]:\\/.][^[:space:]:]*[\\/][^[:space:]]*\.rs([[:space:]]|$)' "$depfile"; then
        printf '%s\t%s\0' "$size" "$path"
      fi
    done < <(find "$dir" -maxdepth 1 -type f -mmin "+$cutoff_min" \
      \( -name '*.exe' -o -name '*.pdb' \) ! -name 'df_*' ! -name 'df-*' -printf '%s\t%p\0' 2>/dev/null || true)
  done
}

# Emits "size<TAB>path" NUL records for scratch directories with no entry modified in 24 hours.
emit_stale_scratch_dirs() {
  local path kb
  [[ -d "$TMP_ROOT" ]] || return 0
  while IFS= read -r -d '' path; do
    if [[ -n "$(find "$path" -mmin -1440 -print -quit 2>/dev/null)" ]]; then
      continue
    fi
    kb=$(du -sk "$path" 2>/dev/null | cut -f1 || true)
    printf '%s\t%s\0' "$(( ${kb:-0} * 1024 ))" "$path"
  done < <(find "$TMP_ROOT" -mindepth 1 -maxdepth 1 -type d \( -name '*-target' -o -name '*_target' \) -print0)
}

sum_bytes() {
  local rec total=0
  for rec in "$@"; do
    total=$(( total + ${rec%%$'\t'*} ))
  done
  printf '%s' "$total"
}

to_mb() {
  awk -v b="$1" 'BEGIN { printf "%.1f", b / 1048576 }'
}

# Deletes stale deps files after rechecking each one. Sets deleted_count and freed_bytes.
delete_files() {
  local rec size path
  deleted_count=0
  freed_bytes=0
  for rec in "$@"; do
    size=${rec%%$'\t'*}
    path=${rec#*$'\t'}
    case "$path" in
      "$TARGET"/*) ;;
      *) echo "  skip (outside target): $path" >&2; continue ;;
    esac
    if [[ -L "$path" || ! -f "$path" ]]; then
      continue
    fi
    if [[ -z "$(find "$path" -maxdepth 0 -type f -mmin "+$cutoff_min" -print 2>/dev/null)" ]]; then
      echo "  skip (changed since scan): $path" >&2
      continue
    fi
    if rm -f -- "$path"; then
      deleted_count=$(( deleted_count + 1 ))
      freed_bytes=$(( freed_bytes + size ))
    else
      echo "  warning: could not delete $path" >&2
    fi
  done
}

# Deletes stale scratch directories after rechecking each one. Sets deleted_count and freed_bytes.
delete_dirs() {
  local rec size path
  deleted_count=0
  freed_bytes=0
  for rec in "$@"; do
    size=${rec%%$'\t'*}
    path=${rec#*$'\t'}
    case "$path" in
      "$TMP_ROOT"/*) ;;
      *) echo "  skip (outside artifacts/tmp): $path" >&2; continue ;;
    esac
    if [[ -L "$path" || ! -d "$path" ]]; then
      continue
    fi
    if [[ -n "$(find "$path" -mmin -1440 -print -quit 2>/dev/null)" ]]; then
      echo "  skip (recently modified): $path" >&2
      continue
    fi
    if rm -rf -- "$path"; then
      deleted_count=$(( deleted_count + 1 ))
      freed_bytes=$(( freed_bytes + size ))
    else
      echo "  warning: could not fully delete $path" >&2
    fi
  done
}

mode="delete"
if (( dry_run )); then
  mode="dry-run"
fi
echo "prune-artifacts: mode=$mode hours=$hours"
echo "target: $TARGET"
echo "free disk before: $(free_space)"

blocked=""
if running=$(running_cargo_lines); then
  if [[ -n "$running" ]]; then
    blocked="cargo or rustc is running (it may be using the target):"$'\n'"$running"
  fi
else
  blocked="cannot check for running cargo or rustc (no tasklist.exe or ps)"
fi
if [[ -n "$blocked" ]]; then
  if (( dry_run )); then
    echo "warning: $blocked"
    echo "warning: a real run would refuse here; nothing is deleted in dry-run"
  elif (( force )); then
    echo "warning: $blocked"
    echo "warning: continuing because --force was given"
  else
    echo "refusing to delete: $blocked" >&2
    echo "rerun with --dry-run to inspect, or --force to override" >&2
    exit 3
  fi
fi

mapfile -d '' -t files_prefixed < <(emit_prefixed_files)
mapfile -d '' -t files_binaries < <(emit_workspace_binaries)
mapfile -d '' -t scratch_dirs < <(emit_stale_scratch_dirs)

bytes_prefixed=$(sum_bytes "${files_prefixed[@]+"${files_prefixed[@]}"}")
bytes_binaries=$(sum_bytes "${files_binaries[@]+"${files_binaries[@]}"}")
bytes_scratch=$(sum_bytes "${scratch_dirs[@]+"${scratch_dirs[@]}"}")

echo "categories:"
printf '  1. df_/df- deps files        %6d files  %10s MB\n' "${#files_prefixed[@]}" "$(to_mb "$bytes_prefixed")"
printf '  2. workspace .exe/.pdb       %6d files  %10s MB\n' "${#files_binaries[@]}" "$(to_mb "$bytes_binaries")"
printf '  3. stale *-target scratch    %6d dirs   %10s MB\n' "${#scratch_dirs[@]}" "$(to_mb "$bytes_scratch")"
if [[ ${#scratch_dirs[@]} -gt 0 ]]; then
  for rec in "${scratch_dirs[@]}"; do
    echo "     ${rec#*$'\t'}"
  done
fi

if (( dry_run )); then
  total_items=$(( ${#files_prefixed[@]} + ${#files_binaries[@]} + ${#scratch_dirs[@]} ))
  total_bytes=$(( bytes_prefixed + bytes_binaries + bytes_scratch ))
  echo "dry run: would delete $total_items items, $(to_mb "$total_bytes") MB (nothing deleted)"
else
  total_deleted=0
  total_freed=0
  if [[ ${#files_prefixed[@]} -gt 0 ]]; then
    delete_files "${files_prefixed[@]}"
    total_deleted=$(( total_deleted + deleted_count ))
    total_freed=$(( total_freed + freed_bytes ))
  fi
  if [[ ${#files_binaries[@]} -gt 0 ]]; then
    delete_files "${files_binaries[@]}"
    total_deleted=$(( total_deleted + deleted_count ))
    total_freed=$(( total_freed + freed_bytes ))
  fi
  if [[ ${#scratch_dirs[@]} -gt 0 ]]; then
    delete_dirs "${scratch_dirs[@]}"
    total_deleted=$(( total_deleted + deleted_count ))
    total_freed=$(( total_freed + freed_bytes ))
  fi
  echo "deleted $total_deleted items, about $(to_mb "$total_freed") MB"
fi

echo "free disk after: $(free_space)"
