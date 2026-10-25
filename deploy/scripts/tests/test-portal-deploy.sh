#!/usr/bin/env bash
set -Eeuo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)
deploy_script=${PORTAL_DEPLOY_SCRIPT:-"$root/deploy/demo/deploy-portal.sh"}
service_unit=${PORTAL_SERVICE_UNIT:-"$root/deploy/demo/df-portal-hosted.service"}
[[ -f $deploy_script ]] || { printf 'missing portal deploy script: %s\n' "$deploy_script" >&2; exit 1; }

assert_contains() {
  grep -Fq -- "$1" "$2" || { printf 'missing portal deploy contract: %s (in %s)\n' "$1" "$2" >&2; exit 1; }
}

assert_contains 'ghcr.io/dungeonfluxgame/dungeonflux-portal' "$deploy_script"
assert_contains 'portal-current' "$deploy_script"
assert_contains 'portal-previous' "$deploy_script"
assert_contains 'docker pull "$image@$target_digest"' "$deploy_script"
assert_contains 'docker cp "$container:/usr/local/bin/df-portal-hosted"' "$deploy_script"
assert_contains 'docker cp "$container:/opt/dungeonflux/app/."' "$deploy_script"
assert_contains 'http://127.0.0.1:8099/app/healthz' "$deploy_script"
assert_contains 'rollback_on_error' "$deploy_script"
assert_contains 'systemctl restart df-portal-hosted.service' "$deploy_script"
assert_contains '^DF_ACCOUNTS_OPEN=false$' "$deploy_script"
assert_contains 'DF_PORTAL_MIGRATOR_DATABASE_URL' "$deploy_script"
assert_contains 'apply_admin_tunnel_hook' "$deploy_script"
assert_contains 'ADMINFLOW_HOOK' "$deploy_script"
assert_contains 'mktemp -d /tmp/df-portal-upload.XXXXXX' "$deploy_script"
assert_contains 'verify_upload_file "$incoming_unit"' "$deploy_script"
! grep -Fq '63309' "$deploy_script" || { printf '%s\n' 'portal deploy touches the gameplay port' >&2; exit 1; }

# Systemd memory caps for the 512 MB droplet live on the unit.
assert_contains 'MemoryMax=128M' "$service_unit"
assert_contains 'MemoryHigh=96M' "$service_unit"
assert_contains 'ExecStart=/usr/bin/env DF_ACCOUNTS_OPEN=false DF_PROFILE=demo' "$service_unit"
assert_contains 'DF_OPERATOR_TUNNEL=false /opt/dungeonflux/bin/df-portal-hosted' "$service_unit"
assert_contains 'MemorySwapMax=0' "$service_unit"
assert_contains 'StateDirectory=dungeonflux-portal' "$service_unit"
assert_contains 'ReadWritePaths=/var/lib/dungeonflux-portal' "$service_unit"

# Argument handling fails before any SSH work.
if bash "$deploy_script" >/dev/null 2>&1; then
  printf '%s\n' 'portal deploy accepted no arguments' >&2
  exit 1
fi
if bash "$deploy_script" not-a-digest >/dev/null 2>&1; then
  printf '%s\n' 'portal deploy accepted a malformed digest' >&2
  exit 1
fi
if bash "$deploy_script" sha256:xyz >/dev/null 2>&1; then
  printf '%s\n' 'portal deploy accepted a short digest' >&2
  exit 1
fi

printf '%s\n' 'portal deploy checks passed'
