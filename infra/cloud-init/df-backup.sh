#!/bin/bash
# df-backup: the nightly encrypted logical backup of PostgreSQL to object storage (round 3). Installed root-owned (0755) at
# /usr/local/sbin/df-backup; run by df-backup.timer (daily, with a random delay) as df-backup.service. Admins may also run
# `sudo df-backup record-drill <epoch-seconds> <pass|fail>` after an operator restore drill.
#
#   df-backup run                        dump, encrypt, upload, verify, record status
#   df-backup record-drill EPOCH RESULT  record a restore drill (deploy/scripts/restore-drill.sh prints the exact command)
#
# WHAT THIS IS. Managed PostgreSQL has its own automated backups and point-in-time recovery (the primary recovery path, RPO and RTO in
# service-operations.md). This job adds a PORTABLE, independently encrypted copy: pg_dump in custom format, encrypted with age to the
# public recipient(s) in the backup consumer's directory. The matching age IDENTITY is operator-only (manifest backup_age_identity) and is
# never on a host, so a host compromise yields ciphertext only. The primary destination's storage key is NOT write-only
# (a Spaces key cannot be restricted to writing: the best it can be is limited to the one bucket). What protects the primary copy is the
# bucket's versioning and lifecycle rule (23 days current + 7 noncurrent, deploy/storage/buckets.csv), so an overwrite or delete by a stolen
# key is recoverable for seven days. The real protection against a stolen key is the OFF-ACCOUNT copy below, which must be made safe by
# OBJECT LOCK (a compliance-mode retention the writer cannot shorten) or by a credential that really is write-only at that provider.
#
# HOW. Two one-shot containers of the signed backup image (allow-list key "backup"), read-only, capability-free, no secret in an argument
# or environment variable:
#   1. dump   on the internal data network, to the db-gate (TLS end to end, verify-full), as the backup role; pg_dump | age writes a file
#             under /var/lib/dungeonflux/backup/staging (ciphertext only: plaintext never touches the disk);
#   2. upload on the control-out network (public destinations only; df-net-rules enforces that) with the bucket-limited key, to the primary
#             bucket, then to the OFF-ACCOUNT target when one is configured (below). Each upload is verified by size.
# A manifest (timestamp, size, sha256 of the ciphertext, tool versions) is uploaded next to each dump.
#
# THE OFF-ACCOUNT TARGET IS AN INTERFACE WITH NO PROVIDER. If OFFSITE_ENDPOINT/OFFSITE_BUCKET/OFFSITE_REGION are absent from the release
# environment, or the offsite_* keys are not pushed, the second copy is skipped and df_backup_offsite_configured is 0, which the alert rules
# turn into a standing P2 reminder, never into success. Whoever chooses the provider only sets those values and pushes a credential that CANNOT delete or overwrite (write-only at the provider, or the bucket under object lock) and an https endpoint (a plain http or other scheme is refused); nothing
# in this script names a provider.
#
# THE BACKUP DATABASE ROLE. df_backup is OPEN (manifest db_backup_password, ruling g): with FORCE ROW LEVEL SECURITY a dump needs a role
# that bypasses row security, which only a superuser can grant and a managed cluster may not allow. Until the owner decides, the role does
# not exist, this job fails at the database step and says why, and the alert rules say BackupFailed/BackupNeverSucceeded. The managed
# cluster's own backups are the recovery path in the meantime.
#
# STATUS FILES. /var/lib/dungeonflux/textfile/df_backup.prom and df_restore_drill.prom, in Prometheus text format, served to Prometheus by
# the host-textfile exporter.
# Test mode (non-root only): DF_BACKUP_TEST_PREFIX=<dir> moves every path under it, DF_BACKUP_DOCKER=<command> replaces docker.
# STATUS: UNVERIFIED against a real host; the state machine (status files, failure handling) is exercised by deploy/scripts/tests/test-df-backup.sh.
set -euo pipefail

if [ "$(id -u)" -eq 0 ]; then
  [ -z "${DF_BACKUP_TEST_PREFIX:-}${DF_BACKUP_DOCKER:-}" ] || { echo "the test options are refused as root" >&2; exit 2; }
  P=""
  docker_cmd=docker
else
  [ -n "${DF_BACKUP_TEST_PREFIX:-}" ] && [ -n "${DF_BACKUP_DOCKER:-}" ] || { echo "df-backup must run as root" >&2; exit 2; }
  P=$DF_BACKUP_TEST_PREFIX
  docker_cmd=$DF_BACKUP_DOCKER
fi
etc=$P/etc/dungeonflux
run=$P/run/dungeonflux
state=$P/var/lib/dungeonflux
allowed=$etc/allowed-images.d/allowed
env_file=$run/secrets/deploy/release_environment
secrets=$run/secrets/backup
textfile=$state/textfile
staging=$state/backup/staging
lock_file=$run/backup.lock
dk() { "$docker_cmd" "$@"; }
now() { date +%s; }

write_prom() { # write_prom NAME < content   (atomic)
  mkdir -p "$textfile"
  cat > "$textfile/.$1.tmp"
  chmod 0644 "$textfile/.$1.tmp"
  mv -f "$textfile/.$1.tmp" "$textfile/$1"
}

prev_value() { # prev_value METRIC -> the value in the last status file, or empty
  sed -n "s/^$1 //p" "$textfile/df_backup.prom" 2>/dev/null | head -n 1 || true
}

setting() { sed -n "s/^$1=//p" "$env_file" 2>/dev/null | head -n 1 || true; }

record_drill() {
  [ "$#" -eq 2 ] || { echo "usage: df-backup record-drill EPOCH_SECONDS pass|fail" >&2; exit 2; }
  [[ "$1" =~ ^[0-9]{9,11}$ ]] || { echo "EPOCH_SECONDS must be a unix time in seconds" >&2; exit 2; }
  case "$2" in pass|fail) ;; *) echo "result must be pass or fail" >&2; exit 2 ;; esac
  # A drill that failed does not move the "last successful drill" time forward; it is recorded separately.
  local last_pass
  last_pass=$(sed -n 's/^df_restore_drill_last_timestamp_seconds //p' "$textfile/df_restore_drill.prom" 2>/dev/null | head -n 1 || true)
  [ "$2" != pass ] || last_pass=$1
  {
    echo '# TYPE df_restore_drill_last_timestamp_seconds gauge'
    [ -z "$last_pass" ] || echo "df_restore_drill_last_timestamp_seconds $last_pass"
    echo '# TYPE df_restore_drill_last_result gauge'
    echo "df_restore_drill_last_result $([ "$2" = pass ] && echo 1 || echo 0)"
  } | write_prom df_restore_drill.prom
  echo "restore drill recorded: $2 at $1"
}

write_status() { # write_status OK(0|1) SIZE DURATION OFFSITE_CONFIGURED OFFSITE_OK
  local ok=$1 size=$2 duration=$3 offsite=$4 offsite_ok=$5 last_success
  last_success=$(prev_value df_backup_last_success_timestamp_seconds)
  [ "$ok" != 1 ] || last_success=$(now)
  {
    echo '# TYPE df_backup_last_run_ok gauge'
    echo "df_backup_last_run_ok $ok"
    echo '# TYPE df_backup_last_run_timestamp_seconds gauge'
    echo "df_backup_last_run_timestamp_seconds $(now)"
    echo '# TYPE df_backup_last_success_timestamp_seconds gauge'
    [ -z "$last_success" ] || echo "df_backup_last_success_timestamp_seconds $last_success"
    echo '# TYPE df_backup_last_size_bytes gauge'
    echo "df_backup_last_size_bytes $size"
    echo '# TYPE df_backup_last_duration_seconds gauge'
    echo "df_backup_last_duration_seconds $duration"
    echo '# TYPE df_backup_offsite_configured gauge'
    echo "df_backup_offsite_configured $offsite"
    echo '# TYPE df_backup_offsite_last_ok gauge'
    echo "df_backup_offsite_last_ok $offsite_ok"
  } | write_prom df_backup.prom
}

# The pinned backup image from the root-owned allow-list (a digest from a verified release, never a tag).
backup_image() {
  local ref
  ref=$(sed -n 's/^backup=//p' "$allowed" 2>/dev/null | head -n 1)
  [[ "$ref" =~ @sha256:[0-9a-f]{64}$ ]] || { echo "no digest-pinned backup image in $allowed (activate a release first)" >&2; return 1; }
  printf '%s' "$ref"
}

# Common container flags: read-only, no capability, no new privileges, bounded, a numeric uid of its own, a private tmpfs.
harden=(--rm --read-only --cap-drop ALL --security-opt no-new-privileges --pids-limit 128 --memory 1g --user 10005:10005)

do_dump() { # do_dump IMAGE OUTFILE -> pg_dump | age, in a container on the data network
  local image=$1 out=$2 host port name server tmp
  host=172.30.2.7   # the db-gate's fixed address on the data network (deploy/compose/production.compose.yml)
  port=5432
  name=$(setting DB_NAME); server=$(setting DB_SERVER_NAME)
  if ! [[ "$name" =~ ^[A-Za-z0-9_]{1,63}$ && "$server" =~ ^[A-Za-z0-9.-]{1,253}$ && "$host" =~ ^[0-9.]{7,15}$ ]]; then
    echo "DB_NAME or DB_SERVER_NAME is missing from the release environment" >&2
    return 1
  fi
  for f in db_backup_password db_ca_certificate backup_age_recipient; do
    if ! [ -s "$secrets/$f" ]; then
      echo "backup secret $f is missing (db_backup_password: the df_backup role is an OPEN owner decision)" >&2
      return 1
    fi
  done
  tmp=$(mktemp -d "$run/docker/backup.XXXXXX") || return 1
  install -m 0444 "$secrets/db_ca_certificate" "$tmp/ca.crt"
  # The recipient file must hold PUBLIC recipients (age1...) and comments only. An age SECRET key in it would put the decrypting identity on a
  # production host, which defeats the whole design: refuse it before anything is dumped.
  if grep -q 'AGE-SECRET-KEY-' "$secrets/backup_age_recipient" \
     || grep -vE '^(#.*|[[:space:]]*|age1[0-9a-z]+)$' "$secrets/backup_age_recipient" | grep -q .; then
    echo "backup_age_recipient must contain public age recipients (age1...) and comments only; an AGE-SECRET-KEY line was found or a line is not a recipient" >&2
    rm -rf "$tmp"
    return 1
  fi
  install -m 0444 "$secrets/backup_age_recipient" "$tmp/recipients.txt"
  # The password goes to the container on stdin (first line) and lands in a pgpass file on the container's own tmpfs.
  if ! head -n 1 "$secrets/db_backup_password" | dk run -i "${harden[@]}" --network "${DF_BACKUP_DATA_NETWORK:-dungeonflux_data}" \
      --tmpfs /pgpw:rw,mode=0700,uid=10005,gid=10005,size=1m \
      --volume "$tmp/ca.crt:/ca.crt:ro" --volume "$tmp/recipients.txt:/recipients.txt:ro" \
      --volume "$(dirname "$out"):/out" \
      --entrypoint bash "$image" -c '
        set -euo pipefail
        umask 077
        IFS= read -r pw
        pw=$(printf "%s" "$pw" | sed "s/\\\\/\\\\\\\\/g; s/:/\\\\:/g")
        printf "*:*:*:df_backup:%s\n" "$pw" > /pgpw/pgpass
        unset pw
        export PGPASSFILE=/pgpw/pgpass
        pg_dump --format=custom --compress=6 --no-owner --no-privileges \
          "host=$2 hostaddr=$3 port=$4 dbname=$5 user=df_backup sslmode=verify-full sslrootcert=/ca.crt" \
          | age --recipients-file /recipients.txt --output "/out/$1"
      ' bash "$(basename "$out")" "$server" "$host" "$port" "$name"; then
    rm -rf "$tmp"
    return 1
  fi
  rm -rf "$tmp"
}

# upload IMAGE ENDPOINT REGION BUCKET KEYFILE_ID KEYFILE_SECRET FILE KEY NETWORK
upload() {
  local image=$1 endpoint=$2 region=$3 bucket=$4 idfile=$5 secretfile=$6 file=$7 key=$8 net=$9
  { head -n 1 "$idfile"; head -n 1 "$secretfile"; } | dk run -i "${harden[@]}" --network "$net" \
    --tmpfs /aws:rw,mode=0700,uid=10005,gid=10005,size=1m --env HOME=/aws \
    --volume "$(dirname "$file"):/in:ro" \
    --entrypoint bash "$image" -c '
      set -euo pipefail
      umask 077
      IFS= read -r id; IFS= read -r secret
      mkdir -p /aws/.aws
      printf "[default]\naws_access_key_id = %s\naws_secret_access_key = %s\n" "$id" "$secret" > /aws/.aws/credentials
      unset id secret
      aws --endpoint-url "$1" --region "$2" s3 cp "/in/$4" "s3://$3/$5" --only-show-errors
      size=$(aws --endpoint-url "$1" --region "$2" s3api head-object --bucket "$3" --key "$5" --query ContentLength --output text)
      test "$size" = "$(stat -c %s "/in/$4")"
    ' bash "$endpoint" "$region" "$bucket" "$(basename "$file")" "$key"
}

run_backup() {
  local started image bucket endpoint region stamp base out manifest size sha version offsite=0 offsite_ok=0 ok=0
  started=$(now)
  mkdir -p "$staging" "$run/docker"
  if [ -z "$P" ]; then chown 10005:10005 "$staging"; chmod 0700 "$staging"; fi
  stamp=$(date -u +%Y%m%dT%H%M%SZ)
  base="df-$stamp"
  out=$staging/$base.dump.age
  manifest=$staging/$base.json
  trap "rm -f '$out' '$manifest'" EXIT   # expanded now: the locals are gone when the shell exits

  if image=$(backup_image) && do_dump "$image" "$out"; then
    size=$(stat -c %s "$out")
    sha=$(sha256sum "$out" | cut -d' ' -f1)
    version=$(setting DB_NAME)
    printf '{"format":"pg_dump-custom+age","created_utc":"%s","database":"%s","size_bytes":%s,"sha256":"%s"}\n' "$stamp" "$version" "$size" "$sha" > "$manifest"
    endpoint=https://$(setting SPACES_HOST); region=$(setting SPACES_REGION); bucket=$(setting BUCKET_PREFIX)-backups
    local key="db/${stamp:0:4}/${stamp:4:2}/$base"
    if [ -s "$secrets/storage_backups_access_key_id" ] && [ -s "$secrets/storage_backups_secret_key" ] \
       && upload "$image" "$endpoint" "$region" "$bucket" "$secrets/storage_backups_access_key_id" "$secrets/storage_backups_secret_key" "$out" "$key.dump.age" "${DF_BACKUP_UPLOAD_NETWORK:-dungeonflux_control-out}" \
       && upload "$image" "$endpoint" "$region" "$bucket" "$secrets/storage_backups_access_key_id" "$secrets/storage_backups_secret_key" "$manifest" "$key.json" "${DF_BACKUP_UPLOAD_NETWORK:-dungeonflux_control-out}"; then
      ok=1
    else
      echo "the upload to the primary backup bucket failed" >&2
    fi
    # The off-account copy: an interface only. Configured iff the target settings AND a credential are present.
    local o_endpoint o_bucket o_region
    o_endpoint=$(setting OFFSITE_ENDPOINT); o_bucket=$(setting OFFSITE_BUCKET); o_region=$(setting OFFSITE_REGION)
    if [ -n "$o_endpoint" ] && ! [[ "$o_endpoint" =~ ^https://[A-Za-z0-9.-]+(:[0-9]{1,5})?$ ]]; then
      echo "OFFSITE_ENDPOINT must be an https URL (host and optional port); refusing to send backups elsewhere" >&2
      offsite=1
    elif [ -n "$o_endpoint" ] && [ -n "$o_bucket" ] && [ -s "$secrets/offsite_access_key_id" ] && [ -s "$secrets/offsite_secret_key" ]; then
      offsite=1
      if [ "$ok" = 1 ] \
         && upload "$image" "$o_endpoint" "$o_region" "$o_bucket" "$secrets/offsite_access_key_id" "$secrets/offsite_secret_key" "$out" "$key.dump.age" "${DF_BACKUP_UPLOAD_NETWORK:-dungeonflux_control-out}" \
         && upload "$image" "$o_endpoint" "$o_region" "$o_bucket" "$secrets/offsite_access_key_id" "$secrets/offsite_secret_key" "$manifest" "$key.json" "${DF_BACKUP_UPLOAD_NETWORK:-dungeonflux_control-out}"; then
        offsite_ok=1
      else
        echo "the off-account copy failed" >&2
      fi
    else
      echo "offsite: not configured" >&2
    fi
  else
    size=0
  fi
  write_status "$ok" "${size:-0}" "$(( $(now) - started ))" "$offsite" "$offsite_ok"
  [ "$ok" = 1 ] || exit 1
  echo "backup $base uploaded ($size bytes, sha256 $sha); off-account: $([ "$offsite_ok" = 1 ] && echo ok || echo "$([ "$offsite" = 1 ] && echo FAILED || echo 'not configured')")"
}

case "${1:-}" in
  run)
    mkdir -p "$run"
    exec 9> "$lock_file"
    flock -n 9 || { echo "another df-backup is running" >&2; exit 5; }
    run_backup ;;
  record-drill) shift; record_drill "$@" ;;
  *) echo "usage: df-backup run | record-drill EPOCH_SECONDS pass|fail" >&2; exit 2 ;;
esac
