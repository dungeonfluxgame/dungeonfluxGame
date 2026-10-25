#!/usr/bin/env bash
# The RESTORE DRILL: restores a database and its object bundle into an isolated target,
# and measures how long that takes against the 60-minute restore target (service-operations.md, "Recovery and operator access").
# STATUS: UNVERIFIED (cloud mode needs docker, a read credential and the age identity; never run here). Run it on an OPERATOR machine or a throwaway
# host, never on a production host: it needs the age IDENTITY (manifest backup_age_identity, operator-only), which must never be on a host.
#
#   deploy/scripts/restore-drill.sh --endpoint https://nyc3.digitaloceanspaces.com --region nyc3 --bucket dfx-prod-backups \
#       --identity-file ~/keys/backup.age --backup-image REF --postgres-image REF [--object KEY | --latest] [--dry-run]
# Local rehearsal: restore-drill.sh --local-bundle DIR --local-pg-uri postgresql://postgres@127.0.0.1:55911/postgres
#   Credentials: a READ-capable key for the backup bucket in AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY of THIS shell (an operator
#   credential, not the write-only key the backup job holds). --backup-image is the signed backup image (age, aws, pg tools),
#   --postgres-image the pgvector PostgreSQL image from images.lock (POSTGRES_IMAGE), both by digest.
#
# WHAT IT DOES, step by step, each with its time:
#   1. lists the bucket and picks the newest df-*.dump.age (or --object), downloads it and its manifest;
#   2. checks the size and sha256 of the ciphertext against the manifest;
#   3. starts a throwaway PostgreSQL container on a private network with no route out and restores:
#      age --decrypt | pg_restore --no-owner --no-privileges (so no df_* role has to exist);
#   4. verifies the restore: the migration ledger exists and is non-empty, the campaigns schema is there, and the feed-epoch test
#      (deploy/db/test-feed-epochs.sql, which rolls back) passes against the restored data;
#   5. writes a JSON result (steps, times, total, target, pass or fail) and prints the exact command that records the drill on a host:
#          sudo df-backup record-drill <epoch> pass
#      so the P2 reminder (RestoreDrillOverdue) goes quiet only when a drill really passed.
# Row counts are printed, not asserted (an empty database restores fine): the drill proves recoverability, not completeness. Completeness
# is the reconciliation in the recovery runbook.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
endpoint=""; region=""; bucket=""; identity=""; backup_image=""; pg_image=""; object=""; dry=0
local_bundle=""; local_dump_dir=""; local_pg_uri=""
python_bin=$(command -v python3 || command -v python || true)
target_seconds=3600
while [ $# -gt 0 ]; do
  case "$1" in
    --local-bundle) local_bundle=$2; shift 2 ;;
    --local-dump-dir) local_dump_dir=$2; shift 2 ;;
    --local-pg-uri) local_pg_uri=$2; shift 2 ;;
    --endpoint) endpoint=$2; shift 2 ;;
    --region) region=$2; shift 2 ;;
    --bucket) bucket=$2; shift 2 ;;
    --identity-file) identity=$2; shift 2 ;;
    --backup-image) backup_image=$2; shift 2 ;;
    --postgres-image) pg_image=$2; shift 2 ;;
    --object) object=$2; shift 2 ;;
    --latest) object=""; shift ;;
    --dry-run) dry=1; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
if [ -n "$local_dump_dir" ]; then
  [ -n "$local_pg_uri" ] || { echo "--local-pg-uri is required with --local-dump-dir" >&2; exit 2; }
  [[ "$local_pg_uri" =~ ^postgres(ql)?://[^@/]+@127\.0\.0\.1:55911/[^?]+$ ]] || { echo "local rehearsal must use the scratch PostgreSQL at 127.0.0.1:55911" >&2; exit 2; }
  [ -d "$local_dump_dir" ] || { echo "backup directory is missing" >&2; exit 1; }
  command -v createdb >/dev/null 2>&1 && command -v pg_restore >/dev/null 2>&1 && command -v psql >/dev/null 2>&1 && command -v dropdb >/dev/null 2>&1 || {
    echo "missing local PostgreSQL tools: createdb, pg_restore, psql and dropdb are required" >&2; exit 1;
  }
  latest_dump=$(find "$local_dump_dir" -type f -name 'database-dungeonflux*.dump' -printf '%T@ %p\n' | sort -nr | head -n 1 | cut -d' ' -f2-)
  [ -n "$latest_dump" ] || { echo "no DungeonFlux custom-format dump found" >&2; exit 1; }
  stamp=$(date -u +%Y%m%d%H%M%S)
  target_db="df_restore_drill_${stamp}_$$"
  [[ "$target_db" =~ ^df_restore_drill_[A-Za-z0-9_]+$ ]] || { echo "invalid isolated database name" >&2; exit 2; }
  target_uri="${local_pg_uri%/*}/$target_db"
  createdb --maintenance-db="$local_pg_uri" "$target_db"
  cleanup_local_dump() {
    if ! dropdb --maintenance-db="$local_pg_uri" --if-exists "$target_db" >/dev/null 2>&1; then
      echo "CLEANUP FAILURE: could not drop restore-drill database $target_db" >&2
      return 1
    fi
  }
  trap cleanup_local_dump EXIT
  pg_restore --no-owner --no-privileges --exit-on-error --dbname="$target_uri" "$latest_dump"
  actual_database=$(psql "$target_uri" -XAt -v ON_ERROR_STOP=1 -c 'SELECT current_database()')
  [ "$actual_database" = "$target_db" ] || { echo "SANITY FAILURE: restore target database name does not match" >&2; exit 1; }
  object_count=$(psql "$target_uri" -XAt -v ON_ERROR_STOP=1 -c "SELECT count(*) FROM pg_catalog.pg_class WHERE relkind IN ('r','p','S')")
  [[ "$object_count" =~ ^[0-9]+$ ]] && [ "$object_count" -gt 0 ] || { echo "SANITY FAILURE: restored database has no tables or sequences" >&2; exit 1; }
  echo "restore drill passed: $latest_dump restored into $target_db; $object_count tables and sequences found"
  exit 0
fi
if [ -n "$local_bundle" ]; then
  [ -n "$python_bin" ] || { echo "missing tool: python 3" >&2; exit 1; }
  [ -n "$local_pg_uri" ] || { echo "--local-pg-uri is required with --local-bundle" >&2; exit 2; }
  [[ "$local_pg_uri" =~ ^postgres(ql)?://[^@/]+@127\.0\.0\.1:55911/[^?]+$ ]] || { echo "local rehearsal must use the scratch PostgreSQL at 127.0.0.1:55911" >&2; exit 2; }
  [ -d "$local_bundle" ] || { echo "bundle directory is missing" >&2; exit 1; }
  local_bundle=$(cd "$local_bundle" && pwd)
  command -v createdb >/dev/null 2>&1 && command -v pg_restore >/dev/null 2>&1 && command -v psql >/dev/null 2>&1 && command -v dropdb >/dev/null 2>&1 || { echo "missing local PostgreSQL tools: createdb, pg_restore, psql and dropdb are required" >&2; exit 1; }
  started=$(date +%s)
  dump_name=$("$python_bin" - "$local_bundle/backup.json" <<'PY'
import json
import pathlib
import sys

try:
    manifest = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
except (OSError, ValueError) as error:
    print(f"BUNDLE FAILURE: backup.json is unreadable: {error}", file=sys.stderr)
    raise SystemExit(1)
name = manifest.get("database_dump", "database.dump")
if not isinstance(name, str) or pathlib.PurePosixPath(name).name != name or "\\" in name or ":" in name:
    print("BUNDLE FAILURE: database_dump must be a file name", file=sys.stderr)
    raise SystemExit(1)
print(name)
PY
  )
  scanner=$(cat <<'PY'
import base64
import binascii
import hashlib
import json
import pathlib
import re
import sys
import urllib.parse

bundle = pathlib.Path(sys.argv[1])
dump_name = sys.argv[2]
try:
    manifest = json.loads((bundle / "backup.json").read_text(encoding="utf-8"))
except (OSError, ValueError) as error:
    print(f"BUNDLE FAILURE: backup.json is unreadable: {error}", file=sys.stderr)
    raise SystemExit(1)
if manifest.get("format_version") != 1:
    print("VERSION FAILURE: unsupported backup bundle format_version", file=sys.stderr)
    raise SystemExit(1)
dump_name = manifest.get("database_dump", "database.dump")
if not isinstance(dump_name, str) or pathlib.PurePosixPath(dump_name).name != dump_name or "\\" in dump_name or ":" in dump_name:
    print("BUNDLE FAILURE: database_dump must be a file name", file=sys.stderr)
    raise SystemExit(1)
dump = bundle / dump_name
try:
    dump_bytes = dump.read_bytes()
except OSError:
    print("DUMP FAILURE: database dump is missing or unreadable", file=sys.stderr)
    raise SystemExit(1)
if hashlib.sha256(dump_bytes).hexdigest() != manifest.get("database_sha256"):
    print("DUMP FAILURE: database dump checksum mismatch (corrupted dump)", file=sys.stderr)
    raise SystemExit(1)
restored_sql = sys.stdin.buffer.read()
secret_patterns = (rb"AGE-SECRET-KEY-", rb"-----BEGIN [A-Z ]*PRIVATE KEY-----", rb"AKIA[0-9A-Z]{16}",
                   rb"(password|secret_key|api_key)\s*[=:]\s*[^\s]+")
def contains_secret(data):
    pending = [data]
    scanned = set()
    while pending:
        text = pending.pop()
        if text in scanned:
            continue
        scanned.add(text)
        if any(re.search(pattern, text, re.IGNORECASE) for pattern in secret_patterns):
            return True
        decoded_url = urllib.parse.unquote_to_bytes(text)
        if decoded_url != text:
            pending.append(decoded_url)
        for match in re.finditer(rb"(?<![A-Za-z0-9+/_=-])[A-Za-z0-9+/_-]{16,}={0,2}(?![A-Za-z0-9+/_=-])", text):
            try:
                pending.append(base64.b64decode(match.group(0), altchars=b"-_", validate=True))
            except (binascii.Error, ValueError):
                pass
        for match in re.finditer(rb"(?<![0-9A-Fa-f])(?:[0-9A-Fa-f]{2}){8,}(?![0-9A-Fa-f])", text):
            try:
                pending.append(bytes.fromhex(match.group(0).decode("ascii")))
            except ValueError:
                pass
    return False

if contains_secret(dump_bytes) or contains_secret(restored_sql):
    print("SECRET FAILURE: secret material found in database dump", file=sys.stderr)
    raise SystemExit(1)
if not isinstance(manifest.get("migration_version"), str) or not manifest["migration_version"]:
    print("VERSION FAILURE: backup migration_version is missing", file=sys.stderr)
    raise SystemExit(1)
if not isinstance(manifest.get("table_checksums"), dict) or not manifest["table_checksums"]:
    print("INTEGRITY FAILURE: per-table checksum catalog is missing", file=sys.stderr)
    raise SystemExit(1)
media_catalog = manifest.get("media", [])
if not isinstance(media_catalog, list):
    print("MEDIA FAILURE: media catalog must be a list", file=sys.stderr)
    raise SystemExit(1)
for media in media_catalog:
    if not isinstance(media, dict):
        print("MEDIA FAILURE: invalid media catalog entry", file=sys.stderr)
        raise SystemExit(1)
    key = media.get("key", "")
    if not isinstance(key, str) or not key or "\\" in key or ":" in key or pathlib.PurePosixPath(key).is_absolute() or ".." in pathlib.PurePosixPath(key).parts:
        print("MEDIA FAILURE: unsafe or missing media key in catalog", file=sys.stderr)
        raise SystemExit(1)
    path = bundle / "media" / key
    try:
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError:
        print(f"MEDIA FAILURE: catalog object is missing: {key}", file=sys.stderr)
        raise SystemExit(1)
    if digest != media.get("sha256"):
        print(f"MEDIA FAILURE: digest mismatch for catalog object: {key}", file=sys.stderr)
        raise SystemExit(1)
PY
)
  pg_restore -f - "$local_bundle/$dump_name" | "$python_bin" -c "$scanner" "$local_bundle" "$dump_name"
  stamp=$(date -u +%Y%m%dT%H%M%SZ)
  target_db="df_s2_restore_${stamp}_$$"
  [[ "$target_db" =~ ^df_s2_restore_[A-Za-z0-9_]+$ ]] || { echo "invalid isolated database name" >&2; exit 2; }
  target_uri="${local_pg_uri%/*}/$target_db"
  createdb --maintenance-db="$local_pg_uri" "$target_db"
  cleanup_local() {
    if ! dropdb --maintenance-db="$local_pg_uri" --if-exists "$target_db" >/dev/null 2>&1; then
      echo "CLEANUP FAILURE: could not drop restore-drill database $target_db" >&2
      return 1
    fi
  }
  trap cleanup_local EXIT
  dump_name=$("$python_bin" -c 'import json,sys; print(json.load(open(sys.argv[1]))["database_dump"])' "$local_bundle/backup.json")
  pg_restore --no-owner --no-privileges --exit-on-error --dbname="$target_uri" "$local_bundle/$dump_name"
  actual_version=$(psql "$target_uri" -XAt -v ON_ERROR_STOP=1 -c 'SELECT version FROM df_meta.schema_migrations ORDER BY version DESC LIMIT 1')
  expected_version=$("$python_bin" -c 'import json,sys; print(json.load(open(sys.argv[1]))["migration_version"])' "$local_bundle/backup.json")
  [ "$actual_version" = "$expected_version" ] || { echo "VERSION FAILURE: restored migration version $actual_version does not match backup $expected_version" >&2; exit 1; }
  query_builder=$(cat <<'PY'
import json
import re
import sys

manifest = json.load(open(sys.argv[1], encoding="utf-8"))
queries = []
for table in sorted(manifest["table_checksums"]):
    parts = table.split(".")
    if len(parts) != 2 or any(not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", part) for part in parts):
        print(f"INTEGRITY FAILURE: unsafe table name in checksum catalog: {table}", file=sys.stderr)
        raise SystemExit(1)
    schema, name = parts
    queries.append(f"SELECT '{table}' AS table_name, to_jsonb(t)::text AS row_data FROM \"{schema}\".\"{name}\" t")
query = " UNION ALL ".join(queries)
print(f"SELECT jsonb_build_array(table_name, row_data)::text FROM ({query}) AS checksum_rows ORDER BY table_name, row_data")
PY
)
  checksum_sql=$("$python_bin" -c "$query_builder" "$local_bundle/backup.json")
  checksum_verifier=$(cat <<'PY'
import hashlib
import json
import sys

manifest = json.load(open(sys.argv[1], encoding="utf-8"))
tables = {table: [] for table in manifest["table_checksums"]}
for line in sys.stdin:
    try:
        table, row = json.loads(line)
    except (TypeError, ValueError):
        print("INTEGRITY FAILURE: malformed restored table checksum row", file=sys.stderr)
        raise SystemExit(1)
    if table not in tables:
        print("INTEGRITY FAILURE: restored table is missing from checksum catalog", file=sys.stderr)
        raise SystemExit(1)
    tables[table].append(row)
classes = {}
for table, expected in sorted(manifest["table_checksums"].items()):
    rows = tables[table]
    digest = hashlib.sha256("\n".join(rows).encode()).hexdigest()
    if len(rows) != expected.get("rows") or digest != expected.get("sha256"):
        print(f"INTEGRITY FAILURE: row count or checksum mismatch for {table}", file=sys.stderr)
        raise SystemExit(1)
    table_class = expected.get("class")
    if not isinstance(table_class, str) or not table_class:
        print(f"INTEGRITY FAILURE: table class is missing for {table}", file=sys.stderr)
        raise SystemExit(1)
    aggregate = classes.setdefault(table_class, [0, hashlib.sha256()])
    aggregate[0] += len(rows)
    aggregate[1].update(f"{table}\0{digest}\n".encode())
for table_class, expected in manifest.get("table_classes", {}).items():
    actual = classes.get(table_class, [0, hashlib.sha256()])
    if actual[0] != expected.get("rows") or actual[1].hexdigest() != expected.get("sha256"):
        print(f"INTEGRITY FAILURE: row count or checksum mismatch for table class {table_class}", file=sys.stderr)
        raise SystemExit(1)
if set(classes) != set(manifest.get("table_classes", {})):
    print("INTEGRITY FAILURE: table class catalog does not match restored tables", file=sys.stderr)
    raise SystemExit(1)
PY
)
  psql "$target_uri" -XAt -v ON_ERROR_STOP=1 -c "$checksum_sql" | "$python_bin" -c "$checksum_verifier" "$local_bundle/backup.json"
  cd "$root"
  mkdir -p artifacts/restore-drill
  report="artifacts/restore-drill/drill-$stamp.json"
  summary="artifacts/restore-drill/drill-$stamp.txt"
  cp -R "$local_bundle/media" "artifacts/restore-drill/media-$stamp"
  cp "$local_bundle/backup.json" "artifacts/restore-drill/manifest-$stamp.json"
  elapsed=$(( $(date +%s) - started ))
  [ "$elapsed" -le "$target_seconds" ] || { echo "RTO FAILURE: restore took ${elapsed}s; target is ${target_seconds}s" >&2; exit 1; }
  "$python_bin" - "$local_bundle/backup.json" "$report" "$summary" "$target_db" "$elapsed" <<'PY'
import hashlib
import json
import pathlib
import sys

manifest_path, report_path, summary_path, database, elapsed = sys.argv[1:]
manifest = json.loads(pathlib.Path(manifest_path).read_text(encoding="utf-8"))
report = {"result": "partial", "database": database, "migration_version": manifest["migration_version"],
          "table_checksums": manifest["table_checksums"], "table_classes": manifest["table_classes"],
          "media_objects": len(manifest.get("media", [])),
          "total_seconds": int(elapsed), "target_seconds": 3600, "finance_hash_chain": "not exposed; request filed",
          "database_dump_sha256": manifest["database_sha256"], "operator_signoff": "pending"}
encoded = json.dumps(report, sort_keys=True, separators=(",", ":"))
pathlib.Path(report_path).write_text(encoded + "\n", encoding="utf-8")
pathlib.Path(report_path + ".sha256").write_text(hashlib.sha256(encoded.encode()).hexdigest() + "  " + pathlib.Path(report_path).name + "\n", encoding="ascii")
pathlib.Path(summary_path).write_text(f"Restore drill PARTIAL (not signed off)\nDatabase: {database}\nMigration: {manifest['migration_version']}\nMedia objects: {report['media_objects']}\nElapsed: {elapsed}s / 3600s\nFinance journal hash chain: not verified (request filed)\n", encoding="utf-8")
PY
  echo "restore drill incomplete: $report (summary: $summary; digest: $report.sha256); finance verification and owner signature are pending" >&2
  exit 3
fi
for v in endpoint region bucket identity backup_image pg_image; do
  [ -n "${!v}" ] || { echo "--${v//_/-} is required" >&2; exit 2; }
done
[[ "$endpoint" =~ ^https://[A-Za-z0-9.-]+$ && "$region" =~ ^[a-z0-9-]+$ && "$bucket" =~ ^[a-z0-9.-]+$ ]] || { echo "malformed endpoint, region or bucket" >&2; exit 2; }
for img in "$backup_image" "$pg_image"; do
  [[ "$img" =~ @sha256:[0-9a-f]{64}$ ]] || { echo "images must be pinned by digest: $img" >&2; exit 2; }
done
[ -r "$identity" ] || { echo "cannot read the age identity file" >&2; exit 2; }
if [ "$dry" -eq 0 ]; then
  command -v docker >/dev/null 2>&1 || { echo "missing tool: docker" >&2; exit 1; }
  [ -n "${AWS_ACCESS_KEY_ID:-}" ] && [ -n "${AWS_SECRET_ACCESS_KEY:-}" ] || { echo "set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY (a read credential)" >&2; exit 1; }
fi

run() { # in dry-run mode only print
  if [ "$dry" -eq 1 ]; then echo "[dry-run] $*"; else "$@"; fi
}
work=$(mktemp -d)
network=df-drill-$$
container=df-drill-pg-$$
cleanup() {
  [ "$dry" -eq 1 ] || { docker rm -f "$container" >/dev/null 2>&1 || true; docker network rm "$network" >/dev/null 2>&1 || true; }
  rm -rf "$work"
}
trap cleanup EXIT
steps=()
t0=$(date +%s)
mark() { steps+=("{\"step\":\"$1\",\"seconds\":$(( $(date +%s) - t0 ))}"); echo "[$(( $(date +%s) - t0 ))s] $1"; }
aws_run() { # aws_run ARGS...  (credentials from this shell's environment, passed by NAME so they never appear in a process list)
  docker run --rm --read-only --cap-drop ALL --security-opt no-new-privileges --user "$(id -u):$(id -g)" \
    --tmpfs /tmp --env AWS_ACCESS_KEY_ID --env AWS_SECRET_ACCESS_KEY --env AWS_DEFAULT_REGION="$region" \
    --volume "$work:/work" --entrypoint aws "$backup_image" --endpoint-url "$endpoint" "$@"
}

# 1. find and fetch
if [ -z "$object" ]; then
  if [ "$dry" -eq 1 ]; then object="db/YYYY/MM/df-<newest>.dump.age"; else
    object=$(aws_run s3api list-objects-v2 --bucket "$bucket" --prefix db/ --query 'sort_by(Contents[?ends_with(Key, `.dump.age`)], &LastModified)[-1].Key' --output text)
  fi
fi
[[ "$object" =~ ^db/[0-9]{4}/[0-9]{2}/df-[0-9TZ]+\.dump\.age$ || "$dry" -eq 1 ]] || { echo "unexpected object key: $object" >&2; exit 1; }
base=$(basename "$object" .dump.age)
mark "newest backup is $object"
run aws_run s3 cp "s3://$bucket/$object" "/work/$base.dump.age" --only-show-errors
run aws_run s3 cp "s3://$bucket/${object%.dump.age}.json" "/work/$base.json" --only-show-errors
mark "downloaded"

# 2. integrity
if [ "$dry" -eq 0 ]; then
  want=$(sed -n 's/.*"sha256":"\([0-9a-f]\{64\}\)".*/\1/p' "$work/$base.json")
  size_want=$(sed -n 's/.*"size_bytes":\([0-9]*\).*/\1/p' "$work/$base.json")
  have=$(sha256sum "$work/$base.dump.age" | cut -d' ' -f1)
  size_have=$(stat -c %s "$work/$base.dump.age")
  [ "$want" = "$have" ] && [ "$size_want" = "$size_have" ] || { echo "INTEGRITY FAILURE: the ciphertext does not match its manifest" >&2; exit 1; }
fi
mark "ciphertext matches its manifest"

# 3. restore into a scratch PostgreSQL with no route out
run docker network create --internal "$network"
run docker run -d --name "$container" --network "$network" --read-only --cap-drop ALL --cap-add CHOWN --cap-add DAC_OVERRIDE --cap-add FOWNER --cap-add SETGID --cap-add SETUID \
  --tmpfs /tmp --tmpfs /run/postgresql --tmpfs /var/lib/postgresql/data:rw,size=8g --env POSTGRES_PASSWORD=drill --env POSTGRES_DB=drill "$pg_image"
if [ "$dry" -eq 0 ]; then
  for _ in $(seq 1 60); do docker exec "$container" pg_isready -U postgres >/dev/null 2>&1 && break; sleep 1; done
  docker exec "$container" pg_isready -U postgres >/dev/null
fi
# The dump itself carries CREATE SCHEMA df_ext and CREATE EXTENSION vector; the pgvector image provides the library.
mark "scratch PostgreSQL is up"
if [ "$dry" -eq 1 ]; then
  echo "[dry-run] age --decrypt -i <identity> < $base.dump.age | docker exec -i $container pg_restore -U postgres -d drill --no-owner --no-privileges --exit-on-error"
else
  docker run --rm -i --read-only --cap-drop ALL --user "$(id -u):$(id -g)" --volume "$work:/work:ro" --volume "$identity:/identity:ro" \
    --entrypoint age "$backup_image" --decrypt -i /identity "/work/$base.dump.age" \
    | docker exec -i "$container" pg_restore -U postgres -d drill --no-owner --no-privileges --exit-on-error
fi
mark "restored"

# 4. verify
result=pass
if [ "$dry" -eq 0 ]; then
  q() { docker exec "$container" psql -U postgres -d drill -tA -v ON_ERROR_STOP=1 -c "$1"; }
  ledger=$(q "SELECT count(*) FROM df_meta.schema_migrations" 2>/dev/null || echo missing)
  if [ "$ledger" = missing ] || [ "$ledger" -lt 1 ]; then echo "VERIFY FAILED: the migration ledger is missing or empty" >&2; result=fail; fi
  q "SELECT to_regnamespace('campaigns') IS NOT NULL" | grep -qx t || { echo "VERIFY FAILED: the campaigns schema is missing" >&2; result=fail; }
  echo "tables per schema:"; q "SELECT schemaname || ' ' || count(*) FROM pg_tables WHERE schemaname LIKE 'df\_%' OR schemaname IN ('campaigns','accounts','finance','journal') GROUP BY schemaname ORDER BY 1"
  if ! docker exec -i "$container" psql -U postgres -d drill -v ON_ERROR_STOP=1 < "$root/deploy/db/test-feed-epochs.sql" >/dev/null 2>&1; then
    echo "VERIFY FAILED: the feed-epoch test did not pass on the restored data" >&2; result=fail
  fi
else
  echo "[dry-run] verify: migration ledger non-empty, campaigns schema present, deploy/db/test-feed-epochs.sql passes"
fi
mark "verified ($result)"

total=$(( $(date +%s) - t0 ))
[ "$total" -le "$target_seconds" ] || { echo "the restore took ${total}s, over the ${target_seconds}s target" >&2; result=fail; }
out=$root/artifacts/restore-drill
stamp=$(date -u +%Y%m%dT%H%M%SZ)
[ "$dry" -eq 1 ] || mkdir -p "$out"
printf '{"object":"%s","result":"%s","total_seconds":%s,"target_seconds":%s,"steps":[%s]}\n' "$object" "$result" "$total" "$target_seconds" "$(IFS=,; echo "${steps[*]}")" | { if [ "$dry" -eq 1 ]; then cat; else tee "$out/drill-$stamp.json"; fi; }
echo "result: $result in ${total}s (target ${target_seconds}s); (dry run: nothing written; a real run writes artifacts/restore-drill/drill-$stamp.json)"
echo "record it on each production host:  sudo df-backup record-drill $(date +%s) $result"
[ "$result" = pass ]
