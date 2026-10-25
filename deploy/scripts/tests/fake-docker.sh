#!/usr/bin/env bash
# A fake `docker` for deploy/scripts/tests/test-df-stack.sh: just enough of `docker compose`, `ps`, `inspect`, `exec`, `image inspect` and
# `restart` to drive df-stack and df-deploy through their state machines. State lives in $FAKE_STATE; behaviour is set by environment:
#   FAKE_FAIL_RELEASES      comma list of release directory NAMES whose control-plane never becomes ready (exec healthcheck fails)
#   FAKE_UNHEALTHY_RELEASES comma list of release names whose control-plane `up --wait` fails
#   FAKE_RESTART_SERVICE    a service whose RestartCount grows on every inspect (a crash loop during the soak)
#   FAKE_MISSING_IMAGE      an image reference that `image inspect` reports as absent
#   FAKE_BAD_COMPOSE=1      `compose config` fails
set -u
state=${FAKE_STATE:?}
mkdir -p "$state/svc"
echo "docker $*" >> "$state/calls.log"

in_list() { case ",$2," in *",$1,"*) return 0 ;; esac; return 1; }

case "${1:-}" in
  compose)
    shift
    dir=""
    while [ $# -gt 0 ]; do
      case "$1" in
        --project-directory) dir=$2; shift 2 ;;
        --project-name|--file|--env-file) shift 2 ;;
        *) break ;;
      esac
    done
    rel=$(basename "$dir")
    sub=${1:-}; shift || true
    case "$sub" in
      config)
        [ -z "${FAKE_BAD_COMPOSE:-}" ] || exit 1
        case "${1:-}" in --images) printf 'example/image-%s@sha256:%064d\n' "$rel" 1 ;; esac
        exit 0 ;;
      up)
        services=()
        while [ $# -gt 0 ]; do
          case "$1" in
            --pull|--wait-timeout) shift 2 ;;
            -*) shift ;;
            *) services+=("$1"); shift ;;
          esac
        done
        for s in "${services[@]}"; do
          echo "$rel" > "$state/svc/$s"
          echo "up $rel $s" >> "$state/order.log"
          if [ "$s" = control-plane ] && in_list "$rel" "${FAKE_UNHEALTHY_RELEASES:-}"; then exit 1; fi
        done
        exit 0 ;;
      down)
        rm -f "$state"/svc/*
        echo "down $rel" >> "$state/order.log"
        exit 0 ;;
    esac
    exit 0 ;;
  image)
    [ "${2:-}" = inspect ] || exit 0
    [ "${3:-}" != "${FAKE_MISSING_IMAGE:-@none@}" ] || exit 1
    exit 0 ;;
  ps)
    svc=""
    for a in "$@"; do case "$a" in label=com.docker.compose.service=*) svc=${a#label=com.docker.compose.service=} ;; esac; done
    [ -z "$svc" ] || [ ! -f "$state/svc/$svc" ] || echo "cid-$svc"
    exit 0 ;;
  inspect)
    fmt=$3; id=$4; svc=${id#cid-}
    case "$fmt" in
      *State.Status*) echo running ;;
      *State.Health*) if [ "$svc" = control-plane ] && in_list "$(cat "$state/svc/$svc")" "${FAKE_FAIL_RELEASES:-}"; then echo unhealthy; else echo healthy; fi ;;
      *RestartCount*)
        n=$(cat "$state/restarts.$svc" 2>/dev/null || echo 0)
        if [ "$svc" = "${FAKE_RESTART_SERVICE:-@none@}" ]; then n=$((n + 1)); echo "$n" > "$state/restarts.$svc"; fi
        echo "$n" ;;
    esac
    exit 0 ;;
  exec)
    id=$2; svc=${id#cid-}
    if in_list "$(cat "$state/svc/$svc" 2>/dev/null)" "${FAKE_FAIL_RELEASES:-}"; then exit 1; fi
    exit 0 ;;
  restart) exit 0 ;;
esac
exit 0
