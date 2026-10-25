#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
cd "$root"
work=$(mktemp -d "artifacts/tmp/restore-scratch-test.XXXXXX")
trap 'rm -rf -- "$work"' EXIT
mkdir -p "$work/bin" "$work/dumps/old" "$work/dumps/new"
printf 'older custom dump' > "$work/dumps/old/database-dungeonflux.dump"
printf 'latest custom dump' > "$work/dumps/new/database-dungeonflux_demo.dump"
touch -d '2026-10-24 04:00:00 UTC' "$work/dumps/old/database-dungeonflux.dump"
touch -d '2026-10-25 04:00:00 UTC' "$work/dumps/new/database-dungeonflux_demo.dump"

cat > "$work/bin/createdb" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >> "$FAKE_LOG"
EOF
cat > "$work/bin/pg_restore" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >> "$FAKE_LOG"
EOF
cat > "$work/bin/psql" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >> "$FAKE_LOG"
case "$*" in
  *current_database*)
    if [ -n "${FAKE_DATABASE:-}" ]; then printf '%s\n' "$FAKE_DATABASE"; else
      for arg in "$@"; do case "$arg" in postgresql://*/df_restore_drill_*) printf '%s\n' "${arg##*/}"; exit 0 ;; esac; done
    fi ;;
  *pg_catalog.pg_class*) printf '%s\n' "${FAKE_OBJECT_COUNT:-3}" ;;
  *) exit 2 ;;
esac
EOF
cat > "$work/bin/dropdb" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >> "$FAKE_LOG"
EOF
chmod +x "$work/bin/"*

failures=0
checks=0
check() {
  local description=$1
  shift
  checks=$((checks + 1))
  if "$@"; then
    printf 'PASS %s\n' "$description"
  else
    printf 'FAIL %s\n' "$description"
    failures=$((failures + 1))
  fi
}
run_drill() {
  env PATH="$work/bin:$PATH" FAKE_LOG="$work/log" FAKE_DATABASE="${FAKE_DATABASE:-}" \
    FAKE_OBJECT_COUNT="${FAKE_OBJECT_COUNT:-3}" bash "$root/deploy/scripts/restore-drill.sh" \
    --local-dump-dir "$work/dumps" --local-pg-uri postgresql://postgres@127.0.0.1:55911/postgres >"$work/out" 2>"$work/err"
}

: > "$work/log"
run_drill
check 'latest custom dump is restored into an isolated scratch database' bash -c 'grep -Fq "$1" "$2" && grep -q -- "--dbname=postgresql://postgres@127.0.0.1:55911/df_restore_drill_" "$2"' _ "$work/dumps/new/database-dungeonflux_demo.dump" "$work/log"
check 'scratch database runs the database identity and restored-object sanity queries' bash -c 'grep -q "current_database" "$1" && grep -q "pg_catalog.pg_class" "$1"' _ "$work/log"
check 'scratch database is dropped after the drill' grep -q -- '--if-exists df_restore_drill_' "$work/log"

if FAKE_DATABASE=wrong_database run_drill; then result=0; else result=$?; fi
check 'database identity mismatch fails the drill' test "$result" -ne 0
check 'database identity mismatch is reported' grep -q 'SANITY FAILURE' "$work/err"

if FAKE_OBJECT_COUNT=0 run_drill; then result=0; else result=$?; fi
check 'a restored database with no tables or sequences fails sanity' test "$result" -ne 0
check 'empty restored object catalog is reported' grep -q 'no tables or sequences' "$work/err"

printf '%s checks, %s failure(s)\n' "$checks" "$failures"
[ "$failures" -eq 0 ]
