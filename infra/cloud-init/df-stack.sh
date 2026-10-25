#!/bin/bash
# df-stack: starts, gates and stops the production compose stack of ONE release on this host (round 3). Installed root-owned (0755) at
# /usr/local/sbin/df-stack. Called only by `df-deploy activate` and `df-deploy rollback` (and by the Docker boot hook); nobody else has a
# reason to run it, and it is not in any sudoers file.
#
#   df-stack up RELEASE_DIR      bring the stack of that release up, gate it, soak it; exit 0 only when it is healthy
#   df-stack down                stop and remove the compose services (runtime containers are not compose services and are left alone)
#   df-stack restart-proxy       restart the Docker proxy and the runner (the Docker boot hook: see below)
#
# THE ROLLOUT, in dependency order, each step gated before the next starts (a step that does not become healthy within the wait ends
# the whole thing with a non-zero exit, and df-deploy then rolls back to the previous release):
#   1. validate  `docker compose config` of the release, and every image it names already present locally (df-deploy pulled them by digest);
#   2. base      db-gate, ingestor, docker-proxy;
#   3. services  egress (healthy), runner;
#   4. CANARY    control-plane, gated on its READINESS listener: `df-server healthcheck --signal ready` (the same check as /healthz/ready on
#                the readiness-only listener) must succeed ready_checks times in a row, ready_interval seconds apart;
#   5. edge      the Caddy edge, which routes only to a ready control plane;
#   6. monitoring  Prometheus, Alertmanager and the textfile exporter;
#   7. SOAK      for canary_seconds every service must stay running, never turn unhealthy and never restart.
# One host is one canary by itself; across hosts the order is the caller's (CI activates host by host and stops at the first failure).
# Runtime containers are created by the runner through the proxy, not by compose, so recreating the control plane does not touch a running
# campaign; the runner and the proxy are recreated and the runner adopts or ends what it finds (O1).
#
# THE DB-GATE IS RECREATED ONLY WHEN ITS CONFIGURATION CHANGED. Every other service is recreated on each activation (a changed bind-mounted
# config or image is always picked up), but recreating the database forwarder drops the database connections of every LIVE runtime, which
# keep running across an activation. So it is left alone when its rendered config is byte-identical to the one it was last started with
# (compose still recreates it if its image or definition changed).
#
# Settings (root-owned, optional) in /etc/dungeonflux/rollout.conf: wait_seconds=180 canary_seconds=60 ready_checks=3 ready_interval=5
# Test mode (non-root only): DF_STACK_DOCKER=<command standing in for docker>, DF_STACK_SLEEP=<command standing in for sleep>.
# STATUS: the state machine is exercised against a fake docker (deploy/scripts/tests/test-df-stack.sh); real Docker is UNVERIFIED.
set -euo pipefail

if [ "$(id -u)" -eq 0 ]; then
  [ -z "${DF_STACK_DOCKER:-}${DF_STACK_SLEEP:-}" ] || { echo "test options are refused as root" >&2; exit 2; }
  docker_cmd=docker
  sleep_cmd=sleep
  conf=/etc/dungeonflux/rollout.conf
  state_dir=/var/lib/dungeonflux
else
  [ -n "${DF_STACK_DOCKER:-}" ] || { echo "df-stack must run as root" >&2; exit 2; }
  docker_cmd=$DF_STACK_DOCKER
  sleep_cmd=${DF_STACK_SLEEP:-sleep}
  conf=${DF_STACK_CONF:-/nonexistent}
  state_dir=${DF_STACK_STATE:-/nonexistent}
fi

wait_seconds=180
canary_seconds=60
ready_checks=3
ready_interval=5
if [ -r "$conf" ]; then
  while IFS='=' read -r key value; do
    [[ "$value" =~ ^[0-9]{1,4}$ ]] || continue
    case "$key" in
      wait_seconds) wait_seconds=$value ;;
      canary_seconds) canary_seconds=$value ;;
      ready_checks) ready_checks=$value ;;
      ready_interval) ready_interval=$value ;;
    esac
  done < "$conf"
fi

project=dungeonflux
say() { echo "df-stack: $*" >&2; }
dk() { "$docker_cmd" "$@"; }

release_dir=""
compose() { dk compose --project-name "$project" --project-directory "$release_dir" --file "$release_dir/production.compose.yml" \
  --env-file "$release_dir/release.lock" --env-file "$release_dir/production.env" --env-file "$release_dir/host.env" "$@"; }

container_of() { # container_of SERVICE -> container id (empty if none)
  dk ps -aq --filter "label=com.docker.compose.project=$project" --filter "label=com.docker.compose.service=$1" | head -n 1
}

# Start the given services (without their dependencies, in the order the caller chose), recreating them so a changed bind-mounted config
# or a changed image is always picked up, and wait for them to be healthy (running, for services without a health check).
start_group() { # start_group SERVICE...
  say "starting: $*"
  compose up -d --force-recreate --no-deps --no-build --pull never --wait --wait-timeout "$wait_seconds" "$@" \
    || { say "FAILED to start or become healthy: $*"; return 1; }
}

service_ok() { # service_ok SERVICE -> 0 when it is running and not unhealthy
  local id state health
  id=$(container_of "$1")
  [ -n "$id" ] || { say "no container for $1"; return 1; }
  state=$(dk inspect --format '{{.State.Status}}' "$id")
  health=$(dk inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' "$id")
  [ "$state" = running ] && [ "$health" != unhealthy ]
}

restarts_of() { dk inspect --format '{{.RestartCount}}' "$(container_of "$1")"; }

ready_gate() { # the control plane's readiness, ready_checks times in a row
  local id n=0
  id=$(container_of control-plane)
  [ -n "$id" ] || { say "no control-plane container"; return 1; }
  while [ "$n" -lt "$ready_checks" ]; do
    if dk exec "$id" /usr/local/bin/df-server healthcheck --address 172.30.1.3:8084 --signal ready \
         --ca-file /run/secrets/service_issuer_certificate --server-name control-plane >/dev/null 2>&1; then
      n=$((n + 1))
    else
      say "the control plane is not ready ($n of $ready_checks consecutive checks)"
      return 1
    fi
    [ "$n" -ge "$ready_checks" ] || "$sleep_cmd" "$ready_interval"
  done
  say "the control plane answered ready $ready_checks times in a row"
}

all_services="db-gate ingestor docker-proxy egress runner control-plane edge prometheus alertmanager host-textfile"

soak() {
  local s elapsed=0 base_restarts
  declare -A base=()
  for s in $all_services; do base[$s]=$(restarts_of "$s"); done
  say "soak: $canary_seconds s with every service running, healthy and not restarting"
  while [ "$elapsed" -lt "$canary_seconds" ]; do
    "$sleep_cmd" 5
    elapsed=$((elapsed + 5))
    for s in $all_services; do
      service_ok "$s" || { say "SOAK FAILED: $s is not running and healthy"; return 1; }
      base_restarts=$(restarts_of "$s")
      [ "$base_restarts" = "${base[$s]}" ] || { say "SOAK FAILED: $s restarted ($base_restarts, was ${base[$s]})"; return 1; }
    done
  done
}

start_db_gate() {
  local want have flags=(--force-recreate)
  want=$(sha256sum "$release_dir/config/dbgate.cfg" | cut -d' ' -f1)
  have=$(cat "$state_dir/stack-db-gate.sha256" 2>/dev/null || true)
  if [ "$want" = "$have" ] && [ -n "$(container_of db-gate)" ]; then
    say "db-gate: configuration unchanged, NOT force-recreated (that would drop every live runtime's database connections)"
    flags=()
  fi
  say "starting: db-gate"
  compose up -d "${flags[@]}" --no-deps --no-build --pull never --wait --wait-timeout "$wait_seconds" db-gate \
    || { say "FAILED to start or become healthy: db-gate"; return 1; }
  mkdir -p "$state_dir" 2>/dev/null || true
  printf '%s\n' "$want" > "$state_dir/stack-db-gate.sha256" 2>/dev/null || true
}

up() {
  release_dir=${1:-}
  [ -d "$release_dir" ] && [ -s "$release_dir/production.compose.yml" ] && [ -s "$release_dir/host.env" ] \
    || { say "not a staged release directory: $release_dir"; return 2; }
  say "validating $release_dir"
  compose config -q || { say "the compose file does not validate"; return 1; }
  local image
  while IFS= read -r image; do
    [ -n "$image" ] || continue
    dk image inspect "$image" >/dev/null 2>&1 || { say "image not present locally (df-deploy release pulls by digest): $image"; return 1; }
  done < <(compose config --images)

  start_db_gate || return 1
  start_group ingestor docker-proxy || return 1
  start_group egress runner || return 1
  start_group control-plane || return 1
  ready_gate || return 1
  start_group edge || return 1
  start_group prometheus alertmanager host-textfile || return 1
  ready_gate || return 1
  soak || return 1
  say "the stack of $release_dir is up and healthy"
}

down() {
  release_dir=${1:-/opt/dungeonflux/current}
  [ -s "$release_dir/production.compose.yml" ] || { say "no compose file in $release_dir; nothing to stop"; return 0; }
  compose down --timeout 100 || true
  rm -f "$state_dir/stack-db-gate.sha256" 2>/dev/null || true
}

restart_proxy() {
  local s id
  for s in docker-proxy runner; do
    id=$(container_of "$s")
    [ -z "$id" ] || dk restart "$id" >/dev/null 2>&1 || true
  done
}

case "${1:-}" in
  up) shift; up "$@" ;;
  down) shift; down "$@" ;;
  restart-proxy) restart_proxy ;;
  *) echo "usage: df-stack up RELEASE_DIR | down [RELEASE_DIR] | restart-proxy" >&2; exit 2 ;;
esac
