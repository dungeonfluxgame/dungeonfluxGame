#!/usr/bin/env bash
# The RESTORE DRILL (round 3): proves that the newest encrypted backup can be fetched, decrypted and restored into a scratch PostgreSQL,
# and measures how long that takes against the 60-minute restore target (service-operations.md, "Recovery and operator access").
# STATUS: UNVERIFIED (needs docker, a read credential and the age identity; never run here). Run it on an OPERATOR machine or a throwaway
# host, never on a production host: it needs the age IDENTITY (manifest backup_age_identity, operator-only), which must never be on a host.
#
#   deploy/scripts/restore-drill.sh --endpoint https://nyc3.digitaloceanspaces.com --region nyc3 --bucket dfx-prod-backups \
#       --identity-file ~/keys/backup.age --backup-image REF --postgres-image REF [--object KEY | --latest] [--dry-run]
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
target_seconds=3600
while [ $# -gt 0 ]; do
  case "$1" in
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
