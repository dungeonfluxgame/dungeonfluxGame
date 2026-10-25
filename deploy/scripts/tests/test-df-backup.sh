#!/usr/bin/env bash
# Exercises df-backup (status files, failure handling, the off-account interface, drill recording) against a fake docker in a sandbox,
# without root or a cloud. STATUS: run on the authoring machine; the real containers (pg_dump, age, aws) are unverified.
#   deploy/scripts/tests/test-df-backup.sh
set -uo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
failures=0
check() { local what=$1; shift; if "$@"; then echo "PASS $what"; else echo "FAIL $what"; failures=$((failures + 1)); fi; }

new_sandbox() {
  S=$(mktemp -d)
  mkdir -p "$S/bin" "$S/run/dungeonflux/secrets/backup" "$S/run/dungeonflux/secrets/deploy" "$S/run/dungeonflux/docker" \
           "$S/etc/dungeonflux/allowed-images.d" "$S/var/lib/dungeonflux"
  printf '#!/bin/sh\nexit 0\n' > "$S/bin/flock"; chmod +x "$S/bin/flock"
  cat > "$S/docker" <<'EOF'
#!/usr/bin/env bash
# a fake docker for df-backup: `run` only. FAKE_FAIL=dump|upload|offsite selects the failing step.
cat > /dev/null
args=("$@")
mode=upload
for a in "$@"; do case "$a" in *pg_dump*) mode=dump ;; esac; done
echo "$mode ${args[*]: -5}" >> "$FAKE_LOG"
if [ "$mode" = dump ]; then
  [ "${FAKE_FAIL:-}" != dump ] || exit 1
  n=${#args[@]}
  file=${args[$((n - 5))]}
  for a in "$@"; do case "$a" in *:/out) dir=${a%:/out}; echo ciphertext > "$dir/$file" ;; esac; done
  exit 0
fi
[ "${FAKE_FAIL:-}" != upload ] || exit 1
if [ "${FAKE_FAIL:-}" = offsite ]; then
  case "${args[$(( ${#args[@]} - 5 ))]}" in *offsite*) exit 1 ;; esac
fi
exit 0
EOF
  chmod +x "$S/docker"
  printf 'backup=ghcr.io/o/dungeonflux-backup:1@sha256:%064d\npsql=x@sha256:%064d\n' 1 2 > "$S/etc/dungeonflux/allowed-images.d/allowed"
  printf 'DB_NAME=dungeonflux\nDB_SERVER_NAME=db.example.test\nDB_ADDR=10.0.0.9\nSPACES_HOST=nyc3.example.test\nSPACES_REGION=nyc3\nBUCKET_PREFIX=dfx-test\n' > "$S/run/dungeonflux/secrets/deploy/release_environment"
  for f in db_backup_password db_ca_certificate storage_backups_access_key_id storage_backups_secret_key; do echo "value-$f" > "$S/run/dungeonflux/secrets/backup/$f"; done
  printf '# public recipient
age1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq
' > "$S/run/dungeonflux/secrets/backup/backup_age_recipient"
  : > "$S/log"
}
backup() { env PATH="$S/bin:$PATH" DF_BACKUP_TEST_PREFIX="$S" DF_BACKUP_DOCKER="$S/docker" FAKE_LOG="$S/log" bash "$root/infra/cloud-init/df-backup.sh" "$@" 2>"$S/err.log" >"$S/out.log"; }
metric() { sed -n "s/^$1 //p" "$S/var/lib/dungeonflux/textfile/$2" 2>/dev/null | head -n 1; }

# 1. a good run, no off-account target configured
new_sandbox
backup run; rc=$?
check "a good run exits 0" test "$rc" -eq 0
check "last_run_ok is 1" test "$(metric df_backup_last_run_ok df_backup.prom)" = 1
check "a success time is recorded" test -n "$(metric df_backup_last_success_timestamp_seconds df_backup.prom)"
check "the off-account copy is reported as NOT configured (0), never as success" test "$(metric df_backup_offsite_configured df_backup.prom)" = 0
check "one dump and two uploads (dump file and manifest) ran" bash -c 'test "$(grep -c "^dump" "$1")" -eq 1 && test "$(grep -c "^upload" "$1")" -eq 2' _ "$S/log"
check "no staged ciphertext is left on the disk" bash -c 'test -z "$(ls "$1/var/lib/dungeonflux/backup/staging")"' _ "$S"
first=$(metric df_backup_last_success_timestamp_seconds df_backup.prom)

# 2. the dump fails: the failure is recorded, the earlier success time is KEPT
FAKE_FAIL=dump backup run; rc=$?
check "a failed dump exits non-zero" test "$rc" -ne 0
check "last_run_ok is 0" test "$(metric df_backup_last_run_ok df_backup.prom)" = 0
check "the previous success time is preserved" test "$(metric df_backup_last_success_timestamp_seconds df_backup.prom)" = "$first"

# 3. a failed primary upload is a failed backup
FAKE_FAIL=upload backup run; rc=$?
check "a failed upload exits non-zero and records failure" bash -c 'test "$1" -ne 0 && test "$2" = 0' _ "$rc" "$(metric df_backup_last_run_ok df_backup.prom)"

# 4. an off-account target configured and working, then failing
for kv in 'OFFSITE_ENDPOINT=https://offsite.example.test' 'OFFSITE_BUCKET=offsite-bucket' 'OFFSITE_REGION=eu'; do echo "$kv" >> "$S/run/dungeonflux/secrets/deploy/release_environment"; done
echo id > "$S/run/dungeonflux/secrets/backup/offsite_access_key_id"; echo secret > "$S/run/dungeonflux/secrets/backup/offsite_secret_key"
backup run; rc=$?
check "with an off-account target the run succeeds" test "$rc" -eq 0
check "offsite configured is 1 and its last result is ok" bash -c 'test "$1" = 1 && test "$2" = 1' _ "$(metric df_backup_offsite_configured df_backup.prom)" "$(metric df_backup_offsite_last_ok df_backup.prom)"
FAKE_FAIL=offsite backup run; rc=$?
check "an off-account failure keeps the primary backup ok but reports the copy as failed" bash -c 'test "$1" = 1 && test "$2" = 0' _ "$(metric df_backup_last_run_ok df_backup.prom)" "$(metric df_backup_offsite_last_ok df_backup.prom)"

# 5. the backup role/password missing (the OPEN owner decision) is a clear failure
rm -f "$S/run/dungeonflux/secrets/backup/db_backup_password"
backup run; rc=$?
check "a missing db_backup_password fails and says why" bash -c 'test "$1" -ne 0 && grep -q "OPEN owner decision" "$2"' _ "$rc" "$S/err.log"

# 5b. the recipient file must hold PUBLIC recipients only
printf 'value\n' > "$S/run/dungeonflux/secrets/backup/db_backup_password"
printf '# a comment\nage1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq\nAGE-SECRET-KEY-1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ\n' > "$S/run/dungeonflux/secrets/backup/backup_age_recipient"
backup run; rc=$?
check "a recipient file with an AGE-SECRET-KEY line is refused" bash -c 'test "$1" -ne 0 && grep -q "AGE-SECRET-KEY" "$2"' _ "$rc" "$S/err.log"
printf 'not a recipient\n' > "$S/run/dungeonflux/secrets/backup/backup_age_recipient"
backup run; rc=$?
check "a recipient file with any other line is refused" test "$rc" -ne 0
printf '# ok\nage1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq\n' > "$S/run/dungeonflux/secrets/backup/backup_age_recipient"
sed -i 's#^OFFSITE_ENDPOINT=.*#OFFSITE_ENDPOINT=http://offsite.example.test#' "$S/run/dungeonflux/secrets/deploy/release_environment"
backup run; rc=$?
check "a non-https off-account endpoint is refused and reported as a failed copy" bash -c 'test "$1" = 1 && test "$2" = 1 && test "$3" = 0 && grep -q "must be an https URL" "$4"' _ "$(sed -n "s/^df_backup_last_run_ok //p" "$S/var/lib/dungeonflux/textfile/df_backup.prom")" "$(sed -n "s/^df_backup_offsite_configured //p" "$S/var/lib/dungeonflux/textfile/df_backup.prom")" "$(sed -n "s/^df_backup_offsite_last_ok //p" "$S/var/lib/dungeonflux/textfile/df_backup.prom")" "$S/err.log"

# 6. recording drills
backup record-drill 1700000000 pass; rc=$?
check "a passed drill is recorded" bash -c 'test "$1" -eq 0 && test "$2" = 1700000000 && test "$3" = 1' _ "$rc" "$(metric df_restore_drill_last_timestamp_seconds df_restore_drill.prom)" "$(metric df_restore_drill_last_result df_restore_drill.prom)"
backup record-drill 1700100000 fail
check "a failed drill does not move the last successful drill forward" bash -c 'test "$1" = 1700000000 && test "$2" = 0' _ "$(metric df_restore_drill_last_timestamp_seconds df_restore_drill.prom)" "$(metric df_restore_drill_last_result df_restore_drill.prom)"
backup record-drill 12 pass; rc=$?
check "a malformed time is refused" test "$rc" -eq 2
rm -rf "$S"

echo "$failures failure(s)"
[ "$failures" -eq 0 ]
