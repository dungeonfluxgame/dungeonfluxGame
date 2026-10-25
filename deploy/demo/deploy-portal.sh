#!/usr/bin/env bash
# Deploy or roll back the hosted account portal on the demo droplet.
# Usage: deploy/demo/deploy-portal.sh <sha256:digest|latest|--rollback>
#
# The portal runs as a binary under df-portal-hosted.service, not as a
# container: the GHCR portal image is the digest-pinned transport for the
# binary and the account-app bundle (Dockerfile.portal). Migrations are not
# a separate step here: df-portal-hosted opens the account store and runs
# database::migrate with DF_PORTAL_MIGRATOR_DATABASE_URL at startup
# (crates/df-hosted/src/runtime.rs open_stores), so the restart below is the
# migration step and a failed migration fails the health check and rolls back.
set -Eeuo pipefail

readonly host='root@157.230.52.8'
readonly ssh_key="${HOME}/.ssh/id_ed25519_dungeonfluxdnd"
readonly script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly temporary_dir="$(mktemp -d)"
remote_upload_dir=
cleanup() {
  if [[ -n "$remote_upload_dir" ]]; then
    ssh -i "$ssh_key" "$host" bash -s -- "$remote_upload_dir" >/dev/null 2>&1 <<'CLEANUP' || true
set -Eeuo pipefail
case $1 in
  /tmp/df-portal-upload.*) rm -rf -- "$1" ;;
  *) exit 1 ;;
esac
CLEANUP
  fi
  rm -rf "$temporary_dir"
}
trap cleanup EXIT

fail() { printf 'deploy-portal: %s\n' "$1" >&2; exit 1; }
if [[ $# -ne 1 ]]; then
  printf '%s\n' 'usage: deploy/demo/deploy-portal.sh <sha256:digest|latest|--rollback>' >&2
  exit 2
fi

mode=deploy
digest=$1
if [[ "$digest" == --rollback ]]; then
  mode=rollback
  digest=previous
elif [[ "$digest" == latest ]]; then
  printf '%s\n' 'Resolving latest successful demo-release run on main.'
  run_id="$(gh run list --workflow demo-release.yml --branch main --status success --limit 1 --json databaseId --jq '.[0].databaseId // empty')"
  [[ -n "$run_id" ]] || fail 'No successful demo-release run exists on main.'
  mkdir -p "$temporary_dir/digest"
  gh run download "$run_id" --name portal-digest --dir "$temporary_dir/digest"
  digest="$(<"$temporary_dir/digest/portal-digest.txt")"
fi
if [[ "$mode" == deploy ]]; then
  [[ "$digest" =~ ^sha256:[a-f0-9]{64}$ ]] || fail 'Expected a sha256 digest, latest or --rollback.'
fi
[[ -r "$ssh_key" ]] || fail "SSH key is missing or unreadable: $ssh_key"

run_token="$(date -u +%Y%m%d%H%M%S)-$$"
remote_upload_dir="$(ssh -i "$ssh_key" "$host" bash -s <<'PREPARE'
set -Eeuo pipefail
umask 077
upload_dir="$(mktemp -d /tmp/df-portal-upload.XXXXXX)"
chmod 0700 "$upload_dir"
[[ "$(stat -c '%u:%a' "$upload_dir")" == '0:700' ]]
printf '%s\n' "$upload_dir"
PREPARE
)"
[[ "$remote_upload_dir" =~ ^/tmp/df-portal-upload\.[A-Za-z0-9]+$ ]] || fail 'Host returned an invalid private upload directory.'
scp -i "$ssh_key" "$script_dir/df-portal-hosted.service" "$host:$remote_upload_dir/df-portal-hosted.service"
unit_sha256="$(sha256sum -- "$script_dir/df-portal-hosted.service" | cut -d ' ' -f 1)"

printf 'Deploying portal to %s.\n' "$host"
ssh -i "$ssh_key" "$host" bash -s -- "$mode" "$digest" "$run_token" "$remote_upload_dir" "$unit_sha256" <<'REMOTE'
set -Eeuo pipefail
mode=$1
requested_digest=$2
run_token=$3
incoming_dir=$4
unit_sha256=$5
image='ghcr.io/dungeonfluxgame/dungeonflux-portal'
directory='/opt/dungeonflux/demo'
current_file="$directory/portal-current"
previous_file="$directory/portal-previous"
binary='/opt/dungeonflux/bin/df-portal-hosted'
bundle='/opt/dungeonflux/app'
unit='/etc/systemd/system/df-portal-hosted.service'
staging=
incoming_unit="$incoming_dir/df-portal-hosted.service"
container="df-portal-extract-$run_token"

# ADMINFLOW_HOOK: artifacts/tmp/agents/ADMINFLOW/design.md defines a separate
# loopback ingress. Enablement needs its implemented listener and TLS runbook.
apply_admin_tunnel_hook() {
  if grep -Eq '^DF_OPERATOR_TUNNEL=(true|"true"|\x27true\x27)' /etc/dungeonflux/portal.env /etc/dungeonflux/demo.env; then
    echo 'DF_OPERATOR_TUNNEL must stay false until ADMINFLOW implementation is accepted.' >&2
    exit 1
  fi
  printf '%s\n' 'ADMINFLOW_HOOK: DF_OPERATOR_TUNNEL=false; dedicated operator ingress pending.'
}

verify_upload_directory() {
  local owner_mode
  [[ "$incoming_dir" =~ ^/tmp/df-portal-upload\.[A-Za-z0-9]+$ && -d "$incoming_dir" && ! -L "$incoming_dir" ]] || {
    echo 'Private upload directory is missing or invalid.' >&2
    exit 1
  }
  owner_mode=$(stat -c '%u:%a' -- "$incoming_dir")
  [[ "$owner_mode" == '0:700' ]] || { echo 'Private upload directory ownership or mode is invalid.' >&2; exit 1; }
}

verify_upload_file() {
  local path=$1 expected_sha256=$2 owner mode actual_sha256
  [[ -f "$path" && ! -L "$path" && "$expected_sha256" =~ ^[a-f0-9]{64}$ ]] || {
    echo 'Uploaded file is missing or invalid.' >&2
    exit 1
  }
  read -r owner mode < <(stat -c '%u %a' -- "$path")
  [[ "$owner" == 0 ]] || { echo 'Uploaded file is not owned by root.' >&2; exit 1; }
  (( (8#$mode & 0022) == 0 )) || { echo 'Uploaded file is group or other writable.' >&2; exit 1; }
  actual_sha256=$(sha256sum -- "$path")
  actual_sha256=${actual_sha256%% *}
  [[ "$actual_sha256" == "$expected_sha256" ]] || { echo 'Uploaded file checksum mismatch.' >&2; exit 1; }
}

verify_upload_directory
cleanup_upload_dir() {
  docker rm -f "$container" >/dev/null 2>&1 || true
  rm -rf -- "$incoming_dir"
}
trap cleanup_upload_dir EXIT
verify_upload_file "$incoming_unit" "$unit_sha256"

[[ -r /etc/dungeonflux/portal.env ]] || { echo 'Missing /etc/dungeonflux/portal.env; run provision-portal-db.sh first.' >&2; exit 1; }
[[ -r /etc/dungeonflux/demo.env ]] || { echo 'Missing owner-managed /etc/dungeonflux/demo.env.' >&2; exit 1; }
# Accounts stay closed on the demo droplet: refuse any env file that opens
# them, and require the explicit closed value in portal.env.
if grep -Eq '^DF_ACCOUNTS_OPEN=true' /etc/dungeonflux/portal.env /etc/dungeonflux/demo.env; then
  echo 'DF_ACCOUNTS_OPEN=true is refused on the demo droplet.' >&2
  exit 1
fi
grep -Eq '^DF_ACCOUNTS_OPEN=false$' /etc/dungeonflux/portal.env || {
  echo 'DF_ACCOUNTS_OPEN=false is required in /etc/dungeonflux/portal.env.' >&2
  exit 1
}
grep -Eq '^DF_PORTAL_MIGRATOR_DATABASE_URL=.+' /etc/dungeonflux/portal.env || {
  echo 'DF_PORTAL_MIGRATOR_DATABASE_URL is required so startup migrations run as the migrator.' >&2
  exit 1
}
grep -Eq '^DF_PORTAL_DATABASE_URL=.+' /etc/dungeonflux/portal.env || {
  echo 'DF_PORTAL_DATABASE_URL is required.' >&2
  exit 1
}
if grep -Eq '^DF_PORTAL_(OUTBOX|FAKE_PORTS|FAKE_CHECKOUT)=(on|true|1)' /etc/dungeonflux/portal.env; then
  echo 'Development-only portal settings are refused on the demo droplet.' >&2
  exit 1
fi
apply_admin_tunnel_hook

install -d -m 0755 "$directory" /opt/dungeonflux/bin
# Serialize deployments before reading the rollback pointers or changing files.
exec 9>"$directory/portal-deploy.lock"
flock -n 9 || { echo 'Another portal deployment is active.' >&2; exit 1; }
committed=false
changed=false
if [[ "$mode" == rollback ]]; then
  [[ -s "$previous_file" ]] || { echo 'No previous portal image digest is recorded.' >&2; exit 1; }
  target_digest=$(cat "$previous_file")
else
  target_digest=$requested_digest
fi
[[ "$target_digest" =~ ^sha256:[a-f0-9]{64}$ ]] || { echo 'Invalid portal image digest.' >&2; exit 1; }
old_current=
old_previous=
[[ ! -s "$current_file" ]] || old_current=$(cat "$current_file")
[[ ! -s "$previous_file" ]] || old_previous=$(cat "$previous_file")
for value in "$old_current" "$old_previous"; do
  [[ -z "$value" || "$value" =~ ^sha256:[a-f0-9]{64}$ ]] || { echo 'Recorded portal digest is malformed.' >&2; exit 1; }
done
portal_is_healthy() {
  local body
  body=$(curl --fail --silent --show-error --connect-timeout 2 --max-time 3 \
    http://127.0.0.1:8099/app/healthz) || return 1
  [[ "$body" == ok ]] && systemctl is-active --quiet df-portal-hosted.service
}

rollback_on_error() {
  systemctl stop df-portal-hosted.service || return 1
  if [[ -f "$staging/binary.old" ]]; then
    install -m 0755 "$staging/binary.old" "$binary.next" || return 1
    mv -f "$binary.next" "$binary" || return 1
  else
    rm -f "$binary" || return 1
  fi
  rm -rf "$bundle" || return 1
  if [[ -d "$staging/app.old" ]]; then mv "$staging/app.old" "$bundle" || return 1; fi
  if [[ -f "$staging/unit.old" ]]; then
    cp "$staging/unit.old" "$unit" || return 1
  else
    systemctl disable df-portal-hosted.service || return 1
    rm -f "$unit" || return 1
  fi
  if [[ -n "$old_current" ]]; then
    printf '%s\n' "$old_current" > "$current_file" || return 1
  else
    rm -f "$current_file" || return 1
  fi
  if [[ -n "$old_previous" ]]; then
    printf '%s\n' "$old_previous" > "$previous_file" || return 1
  else
    rm -f "$previous_file" || return 1
  fi
  systemctl daemon-reload || return 1
  if [[ -n "$old_current" ]]; then
    systemctl restart df-portal-hosted.service || return 1
    portal_is_healthy || return 1
  fi
}

finish_deploy() {
  local result=$? restored=true
  trap - EXIT
  if [[ "$committed" != true && "$changed" == true ]]; then
    if rollback_on_error; then
      echo 'Portal files and previous service restored; database migrations are not reversed.' >&2
    else
      restored=false
      echo "Portal rollback failed; recovery files retained at $staging" >&2
    fi
    result=1
  fi
  cleanup_upload_dir
  if [[ "$restored" == true ]]; then rm -rf "$staging"; fi
  exit "$result"
}
# EXIT covers explicit validation exits as well as failed commands.
trap finish_deploy EXIT
staging=$(mktemp -d "$directory/.portal-deploy.XXXXXX")
[[ ! -f "$binary" ]] || cp -a "$binary" "$staging/binary.old"
[[ ! -d "$bundle" ]] || cp -a "$bundle" "$staging/app.old"
[[ ! -f "$unit" ]] || cp -a "$unit" "$staging/unit.old"

printf 'Pulling %s@%s\n' "$image" "$target_digest"
docker pull "$image@$target_digest"
docker create --name "$container" "$image@$target_digest" >/dev/null
mkdir "$staging/app.next"
docker cp "$container:/usr/local/bin/df-portal-hosted" "$staging/binary.next"
docker cp "$container:/opt/dungeonflux/app/." "$staging/app.next/"
docker rm -f "$container" >/dev/null
[[ -x "$staging/binary.next" && -s "$staging/app.next/index.html" ]] || {
  echo 'Portal image did not contain an executable binary and bundle index.' >&2
  exit 1
}
chmod -R a+rX "$staging/app.next"

changed=true
systemctl stop df-portal-hosted.service
install -m 0755 "$staging/binary.next" "$binary.next"
mv -f "$binary.next" "$binary"
rm -rf "$bundle"
mv "$staging/app.next" "$bundle"
install -m 0644 "$incoming_unit" "$unit"
# Portal startup applies pending migrations as the migrator, then serves.
systemctl daemon-reload
systemctl enable df-portal-hosted.service >/dev/null
systemctl restart df-portal-hosted.service
healthy=false
for attempt in $(seq 1 18); do
  if portal_is_healthy; then
    healthy=true
    break
  fi
  sleep 5
done
[[ "$healthy" == true ]] || { echo 'Portal did not become healthy after 18 bounded probes.' >&2; exit 1; }
systemctl is-active --quiet df-portal-hosted.service
if [[ -n "$old_current" && "$old_current" != "$target_digest" ]]; then
  printf '%s\n' "$old_current" > "$previous_file"
fi
printf '%s\n' "$target_digest" > "$current_file"
committed=true
printf 'Portal deployment complete: %s@%s\n' "$image" "$target_digest"
REMOTE
printf '%s\n' 'Portal deployment command finished.'
