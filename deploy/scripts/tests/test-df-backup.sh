#!/usr/bin/env bash
# Exercises df-backup (status files, failure handling, the off-account interface, drill recording) against a fake docker in a sandbox,
# without root or a cloud. STATUS: run on the authoring machine; the real containers (pg_dump, age, aws) are unverified.
#   deploy/scripts/tests/test-df-backup.sh
set -uo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
failures=0
check() { local what=$1; shift; if "$@"; then echo "PASS $what"; else echo "FAIL $what"; failures=$((failures + 1)); fi; }

new_sandbox() {
  S=$(cd "$root" && { mkdir -p artifacts/tmp && temp=$(mktemp -d artifacts/tmp/df-backup.XXXXXX) && printf '%s' "${temp#"$root"/}"; })
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

# 7. local restore bundle validation failures are isolated from PostgreSQL and cloud access
local_root="$S/local"
mkdir -p "$local_root/bin"
cat > "$local_root/bin/createdb" <<'EOF'
#!/bin/sh
echo "$*" >> "$FAKE_CREATEDB_LOG"
exit 0
EOF
cat > "$local_root/bin/pg_restore" <<'EOF'
#!/bin/sh
[ "$1" != -f ] || { [ "$2" != - ] || printf '%s' "${FAKE_RESTORE_SQL:-}"; exit 0; }
[ "${FAKE_RESTORE_FAIL:-}" != 1 ] || exit 1
exit 0
EOF
cat > "$local_root/bin/dropdb" <<'EOF'
#!/bin/sh
[ "${FAKE_DROPDB_FAIL:-}" != 1 ] || exit 1
exit 0
EOF
cat > "$local_root/bin/psql" <<'EOF'
#!/bin/sh
case "$*" in
  *"SELECT version"*) echo "${FAKE_VERSION:-0}" ;;
  *) exit 0 ;;
esac
EOF
chmod +x "$local_root/bin/createdb" "$local_root/bin/pg_restore" "$local_root/bin/dropdb" "$local_root/bin/psql"
make_bundle() {
  (cd "$local_root" && env PATH="$PWD/bin:$PATH" FAKE_CREATEDB_LOG=createdb.log "$root/deploy/scripts/restore-drill.sh" --local-bundle bundle \
    --local-pg-uri postgresql://postgres@127.0.0.1:55911/postgres >out 2>err
  )
}
new_bundle() {
  mkdir -p "$local_root/bundle/media"
  printf 'dump fixture\n' > "$local_root/bundle/database.dump"
  printf 'media fixture\n' > "$local_root/bundle/media/plate.bin"
  python - "$local_root/bundle" <<'PY'
import hashlib, json, pathlib, sys
root = pathlib.Path(sys.argv[1])
dump = (root / "database.dump").read_bytes()
media = (root / "media/plate.bin").read_bytes()
table = {"class": "finance", "rows": 0, "sha256": hashlib.sha256(b"").hexdigest()}
value = {"format_version": 1, "database_dump": "database.dump", "database_sha256": hashlib.sha256(dump).hexdigest(),
         "migration_version": "24", "table_checksums": {"finance.entries": table},
         "table_classes": {"finance": {"rows": 0, "sha256": hashlib.sha256(b"finance.entries\0" + table["sha256"].encode() + b"\n").hexdigest()}},
         "media": [{"key": "plate.bin", "sha256": hashlib.sha256(media).hexdigest()}]}
(root / "backup.json").write_text(json.dumps(value), encoding="utf-8")
PY
}
mkdir -p "$local_root"
new_bundle
rm "$local_root/bin/dropdb"
make_bundle; rc=$?
check "missing dropdb fails before creating a restore database" bash -c 'test "$1" -ne 0 && grep -q "createdb, pg_restore, psql and dropdb are required" "$2" && test ! -s "$3"' _ "$rc" "$local_root/err" "$local_root/createdb.log"
cat > "$local_root/bin/dropdb" <<'EOF'
#!/bin/sh
[ "${FAKE_DROPDB_FAIL:-}" != 1 ] || exit 1
exit 0
EOF
chmod +x "$local_root/bin/dropdb"
printf 'corrupted dump fixture\n' > "$local_root/bundle/database.dump"
make_bundle; rc=$?
check "corrupted dump fails with a checksum reason" bash -c 'test "$1" -ne 0 && grep -q "corrupted dump" "$2"' _ "$rc" "$local_root/err"
new_bundle
rm "$local_root/bundle/media/plate.bin"
make_bundle; rc=$?
check "missing media object fails with a catalog reason" bash -c 'test "$1" -ne 0 && grep -q "catalog object is missing" "$2"' _ "$rc" "$local_root/err"
new_bundle
printf 'changed media fixture\n' > "$local_root/bundle/media/plate.bin"
make_bundle; rc=$?
check "media digest mismatch fails clearly" bash -c 'test "$1" -ne 0 && grep -q "digest mismatch" "$2"' _ "$rc" "$local_root/err"
new_bundle
python - "$local_root/bundle/backup.json" <<'PY'
import json, pathlib, sys
path = pathlib.Path(sys.argv[1]); value = json.loads(path.read_text()); value["migration_version"] = "wrong"
path.write_text(json.dumps(value))
PY
FAKE_VERSION=0 make_bundle; rc=$?
check "restored migration version mismatch fails clearly" bash -c 'test "$1" -ne 0 && grep -q "does not match backup wrong" "$2"' _ "$rc" "$local_root/err"
new_bundle
python - "$local_root/bundle" <<'PY'
import hashlib, json, pathlib, sys
root = pathlib.Path(sys.argv[1]); dump = b"api_key=supersecret\n"
(root / "database.dump").write_bytes(dump)
path = root / "backup.json"; value = json.loads(path.read_text()); value["database_sha256"] = hashlib.sha256(dump).hexdigest()
path.write_text(json.dumps(value))
PY
make_bundle; rc=$?
check "secret in database dump fails clearly" bash -c 'test "$1" -ne 0 && grep -q "secret material found" "$2"' _ "$rc" "$local_root/err"
new_bundle
python - "$local_root/bundle" <<'PY'
import base64, hashlib, json, pathlib, sys
root = pathlib.Path(sys.argv[1]); dump = base64.b64encode(b"api_key=encodedsecret\n")
(root / "database.dump").write_bytes(dump)
path = root / "backup.json"; value = json.loads(path.read_text()); value["database_sha256"] = hashlib.sha256(dump).hexdigest()
path.write_text(json.dumps(value))
PY
make_bundle; rc=$?
check "base64-encoded secret in dump fails clearly" bash -c 'test "$1" -ne 0 && grep -q "secret material found" "$2"' _ "$rc" "$local_root/err"
new_bundle
FAKE_RESTORE_SQL='-- AGE-secret-key-1abc\n' make_bundle; rc=$?
check "case-variant age secret marker in pg_restore output fails clearly" bash -c 'test "$1" -ne 0 && grep -q "secret material found" "$2"' _ "$rc" "$local_root/err"
new_bundle
FAKE_RESTORE_SQL='-- api_key=decodedsecret\n' make_bundle; rc=$?
check "secret found only in decoded custom-format dump text fails clearly" bash -c 'test "$1" -ne 0 && grep -q "secret material found" "$2"' _ "$rc" "$local_root/err"
new_bundle
FAKE_RESTORE_FAIL=1 FAKE_DROPDB_FAIL=1 make_bundle; rc=$?
check "failed cleanup reports the isolated database and fails the drill" bash -c 'test "$1" -ne 0 && grep -Eq "CLEANUP FAILURE: could not drop restore-drill database df_s2_restore_[A-Za-z0-9_]+" "$2"' _ "$rc" "$local_root/err"
new_bundle
FAKE_VERSION=24 make_bundle; rc=$?
report=$(sed -n 's/^restore drill incomplete: \([^ ]*\).*/\1/p' "$local_root/err")
check "report step writes from a non-project working directory and returns partial" bash -c 'test "$1" -eq 3 && test -s "$2" && test -s "${2%.json}.txt" && test -s "$2.sha256"' _ "$rc" "$report"
report_stamp=${report##*drill-}; report_stamp=${report_stamp%.json}
(cd "$root" && rm -f "$report" "$report.sha256" "${report%.json}.txt" && rm -rf "artifacts/restore-drill/media-$report_stamp")
rm -rf "$local_root"
rm -rf "$S"

echo "$failures failure(s)"
[ "$failures" -eq 0 ]
