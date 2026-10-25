#!/usr/bin/env bash
set -euo pipefail
umask 077

backup_root=${DF_BACKUP_DIR:-/var/backups/dungeonflux/postgresql}
minimum_free_bytes=${DF_BACKUP_MIN_FREE_BYTES:-1073741824}
offsite_uri=${DF_BACKUP_OFFSITE_URI:-}
offsite_endpoint=${DF_BACKUP_OFFSITE_ENDPOINT:-}
offsite_region=${DF_BACKUP_OFFSITE_REGION:-}

if [ "$(id -u)" -ne 0 ]; then
  echo "backup must run as root" >&2
  exit 1
fi
if ! [[ "$minimum_free_bytes" =~ ^[0-9]+$ ]]; then
  echo "DF_BACKUP_MIN_FREE_BYTES must be a non-negative integer" >&2
  exit 2
fi
command -v flock >/dev/null 2>&1 || { echo "missing tool: flock" >&2; exit 1; }
command -v psql >/dev/null 2>&1 || { echo "missing tool: psql" >&2; exit 1; }
command -v pg_dump >/dev/null 2>&1 || { echo "missing tool: pg_dump" >&2; exit 1; }
command -v pg_dumpall >/dev/null 2>&1 || { echo "missing tool: pg_dumpall" >&2; exit 1; }
command -v df >/dev/null 2>&1 || { echo "missing tool: df" >&2; exit 1; }

install -d -o root -g root -m 0700 "$backup_root"
chown root:root "$backup_root"
chmod 0700 "$backup_root"
exec 9>"$backup_root/.backup.lock"
flock -n 9 || { echo "another PostgreSQL backup is running" >&2; exit 1; }

available_kib=$(df -Pk "$backup_root" | awk 'NR == 2 { print $4 }')
[[ "$available_kib" =~ ^[0-9]+$ ]] || { echo "could not read free space for $backup_root" >&2; exit 1; }
if (( available_kib * 1024 < minimum_free_bytes )); then
  echo "insufficient free space: ${available_kib} KiB available; ${minimum_free_bytes} bytes required" >&2
  exit 1
fi

database_names=$(psql --no-psqlrc --tuples-only --no-align --dbname=postgres --command="SELECT datname FROM pg_database WHERE datallowconn AND NOT datistemplate AND (datname = 'dungeonflux' OR datname LIKE 'dungeonflux\\_%' ESCAPE '\\') ORDER BY datname")
if [ -z "$database_names" ]; then
  echo "no DungeonFlux databases found" >&2
  exit 1
fi

stamp=$(date -u +%Y%m%dT%H%M%S%NZ)
stage_dir=$(mktemp -d "$backup_root/.stage.XXXXXXXX")
chmod 0700 "$stage_dir"
final_dir="$backup_root/$stamp"
cleanup() {
  if [ -n "${stage_dir:-}" ] && [ -d "$stage_dir" ]; then
    rm -rf -- "$stage_dir"
  fi
}
trap cleanup EXIT

database_count=0
while IFS= read -r database_name; do
  [ -n "$database_name" ] || continue
  if ! [[ "$database_name" =~ ^dungeonflux([_][A-Za-z0-9_]+)?$ ]]; then
    echo "unexpected database name returned by PostgreSQL" >&2
    exit 1
  fi
  pg_dump --format=custom --no-owner --no-acl --file="$stage_dir/database-$database_name.dump" "$database_name"
  database_count=$((database_count + 1))
done <<< "$database_names"
[ "$database_count" -gt 0 ] || { echo "no DungeonFlux databases found" >&2; exit 1; }
pg_dumpall --globals-only > "$stage_dir/globals.sql"
printf '%s\n' "$database_names" > "$stage_dir/databases.txt"

if [ "$(TZ=America/New_York date +%u)" = 7 ]; then
  : > "$stage_dir/.weekly"
fi
find "$stage_dir" -type f -exec chmod 0600 {} +
mv -- "$stage_dir" "$final_dir"
stage_dir=
chmod 0700 "$final_dir"

if [ -n "$offsite_uri" ]; then
  command -v aws >/dev/null 2>&1 || { echo "DF_BACKUP_OFFSITE_URI is set but aws is missing" >&2; exit 1; }
  aws_options=()
  [ -z "$offsite_endpoint" ] || aws_options+=(--endpoint-url "$offsite_endpoint")
  [ -z "$offsite_region" ] || aws_options+=(--region "$offsite_region")
  aws "${aws_options[@]}" s3 cp --recursive "$final_dir/" "${offsite_uri%/}/$stamp/"
fi

mapfile -t all_runs < <(find "$backup_root" -mindepth 1 -maxdepth 1 -type d -name '20*' -printf '%T@ %p\n' | sort -nr | cut -d' ' -f2-)
declare -A keep_runs=()
for run in "${all_runs[@]:0:7}"; do keep_runs["$run"]=1; done
weekly_kept=0
for run in "${all_runs[@]}"; do
  if [ -f "$run/.weekly" ] && [ "$weekly_kept" -lt 4 ]; then
    keep_runs["$run"]=1
    weekly_kept=$((weekly_kept + 1))
  fi
done
for run in "${all_runs[@]}"; do
  if [ -z "${keep_runs[$run]:-}" ]; then
    rm -rf -- "$run"
  fi
done

echo "PostgreSQL backup complete: $database_count databases, globals, $final_dir"
