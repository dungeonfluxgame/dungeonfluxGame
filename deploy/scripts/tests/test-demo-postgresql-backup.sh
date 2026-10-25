#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
cd "$root"
work=$(mktemp -d "artifacts/tmp/demo-backup-test.XXXXXX")
trap 'rm -rf -- "$work"' EXIT
mkdir -p "$work/bin" "$work/backups"

cat > "$work/bin/id" <<'EOF'
#!/bin/sh
printf '0\n'
EOF
cat > "$work/bin/df" <<'EOF'
#!/bin/sh
printf 'Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/fake 100000 1 %s 1%% /\n' "${FAKE_FREE_KIB:-100000}"
EOF
cat > "$work/bin/flock" <<'EOF'
#!/bin/sh
exit "${FAKE_LOCK_RESULT:-0}"
EOF
cat > "$work/bin/install" <<'EOF'
#!/bin/sh
path=
while [ "$#" -gt 0 ]; do path=$1; shift; done
mkdir -p "$path"
chmod 0700 "$path"
EOF
cat > "$work/bin/chown" <<'EOF'
#!/bin/sh
exit 0
EOF
cat > "$work/bin/psql" <<'EOF'
#!/bin/sh
printf 'dungeonflux\ndungeonflux_demo_20261025\n'
EOF
cat > "$work/bin/pg_dump" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >> "$FAKE_LOG"
for arg in "$@"; do case "$arg" in --file=*) printf 'custom dump' > "${arg#--file=}" ;; esac; done
EOF
cat > "$work/bin/pg_dumpall" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >> "$FAKE_LOG"
printf 'CREATE ROLE dungeonflux_demo;\n'
EOF
cat > "$work/bin/aws" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >> "$FAKE_AWS_LOG"
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
run_backup() {
  if env PATH="$work/bin:$PATH" DF_BACKUP_DIR="$work/backups" DF_BACKUP_MIN_FREE_BYTES="${TEST_MIN_FREE_BYTES:-0}" \
    FAKE_LOG="$work/log" FAKE_AWS_LOG="$work/aws.log" DF_BACKUP_OFFSITE_URI="${TEST_OFFSITE_URI:-}" \
    DF_BACKUP_OFFSITE_ENDPOINT="${TEST_OFFSITE_ENDPOINT:-}" DF_BACKUP_OFFSITE_REGION="${TEST_OFFSITE_REGION:-}" \
    bash "$root/deploy/demo/backup/backup.sh" >"$work/out" 2>"$work/err"; then
    return 0
  else
    cat "$work/err" >&2
    return 1
  fi
}

: > "$work/log"
run_backup
check 'backup enforces root ownership and private directory permissions' bash -c 'grep -q "^chown root:root" "$1" && grep -q "^chmod 0700" "$1"' _ "$root/deploy/demo/backup/backup.sh"
check 'both DungeonFlux database names are dumped in custom format' bash -c 'grep -q -- "--format=custom" "$1" && grep -q "dungeonflux_demo_20261025" "$1" && grep -q "dungeonflux$" "$1"' _ "$work/log"
check 'global roles are dumped alongside database archives' bash -c 'grep -q -- "--globals-only" "$1" && test -s "$2"' _ "$work/log" "$(find "$work/backups" -name globals.sql | head -n 1)"
check 'dump files are owner-only' grep -q 'find "$stage_dir" -type f -exec chmod 0600' "$root/deploy/demo/backup/backup.sh"

if TEST_MIN_FREE_BYTES=200000000 run_backup; then result=0; else result=$?; fi
check 'free-space guard rejects a run below the configured threshold' test "$result" -ne 0
check 'free-space guard explains the failed precondition' grep -q 'insufficient free space' "$work/err"

TEST_OFFSITE_URI=s3://example-bucket/dungeonflux TEST_OFFSITE_ENDPOINT=https://nyc3.digitaloceanspaces.com TEST_OFFSITE_REGION=nyc3 run_backup
check 'off-host copy supports S3-compatible endpoint and region via environment settings' bash -c 'grep -q -- "--endpoint-url https://nyc3.digitaloceanspaces.com --region nyc3 s3 cp --recursive" "$1" && grep -q "s3://example-bucket/dungeonflux/" "$1"' _ "$work/aws.log"
check 'rotation keeps seven daily runs and four marked weekly runs' bash -c 'grep -q "all_runs\[@\]:0:7" "$1" && grep -q "weekly_kept.*-lt 4" "$1"' _ "$root/deploy/demo/backup/backup.sh"

check 'timer runs daily at 04:30 New York time' grep -qF 'OnCalendar=*-*-* 04:30:00 America/New_York' "$root/deploy/demo/backup/dungeonflux-postgresql-backup.timer"
check 'service runs with a restrictive umask and root-owned backup path' bash -c 'grep -q "User=root" "$1" && grep -q "UMask=0077" "$1" && grep -q "ReadWritePaths=/var/backups/dungeonflux/postgresql" "$1"' _ "$root/deploy/demo/backup/dungeonflux-postgresql-backup.service"

printf '%s checks, %s failure(s)\n' "$checks" "$failures"
[ "$failures" -eq 0 ]
