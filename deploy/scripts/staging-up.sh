#!/usr/bin/env bash
# Start / stop / check the local staging mirror. STATUS: UNVERIFIED (no Docker daemon on the authoring machine).
#   deploy/scripts/staging-up.sh [up|down|config|ps|logs] [extra docker compose args, e.g. --profile runtime]
# `up` REFUSES to run while images.lock still holds a TODO-CI placeholder: nothing is started from an unpinned
# image. `config` renders the effective compose file without needing a daemon and is allowed with placeholders.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
command -v docker >/dev/null 2>&1 || { echo "missing tool: docker" >&2; exit 1; }
action=${1:-up}
if [ $# -gt 0 ]; then shift; fi

compose=(docker compose
  --project-directory "$root/deploy/compose"
  --file "$root/deploy/compose/staging.compose.yml"
  --env-file "$root/deploy/images/images.lock"
  --env-file "$root/deploy/compose/staging.env")

case "$action" in
  up)
    if grep -v '^[[:space:]]*#' "$root/deploy/images/images.lock" | grep -q 'TODO-CI'; then
      echo "refusing to start: deploy/images/images.lock still has TODO-CI placeholders (CI fills the digests)" >&2
      exit 1
    fi
    [ -f "$root/artifacts/tmp/staging/secrets/postgres/db_admin_password" ] \
      || { echo "run deploy/scripts/staging-secrets.sh first" >&2; exit 1; }
    "${compose[@]}" up --detach --wait "$@"
    ;;
  down) "${compose[@]}" down "$@" ;;
  config) "${compose[@]}" config "$@" ;;
  ps) "${compose[@]}" ps "$@" ;;
  logs) "${compose[@]}" logs "$@" ;;
  *) echo "unknown action: $action" >&2; exit 2 ;;
esac
