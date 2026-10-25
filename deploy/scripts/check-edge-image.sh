#!/usr/bin/env bash
# Checks the BUILT edge image (Caddy with the rate-limit module) against the real Caddyfiles. CI only: needs docker, curl and openssl and
# runs the image on loopback; it reaches nothing else and starts no application component. STATUS: UNVERIFIED (never run).
#
#   deploy/scripts/check-edge-image.sh EDGE_IMAGE_REFERENCE
#
# 1. `caddy validate` of Caddyfile.production and Caddyfile.staging with the environment set normally, with DF_ALLOWED_CIDRS EMPTY and with
#    it UNSET. This settles an open question: whether Caddy's `{$DF_ALLOWED_CIDRS:127.0.0.1/32}` default applies to an empty variable, or
#    the parse fails. Either way the edge is closed (a failed parse means it does not start), but the answer decides what the staging
#    Terraform default (an empty list) means, so the result is printed.
# 2. LIVE rate limit. Runs the image with the production Caddyfile (no control plane behind it: a request that passes the limiter gets a
#    502) and sends a burst at the join endpoint in each path spelling an attacker would try. The limiter must answer 429 for every
#    spelling: `/app/api/join/x`, `//app/api/join/x`, `/app//api/join/x`, `/app/api/join%2Fx`, `/app/api/%6Aoin/x`, `/APP/API/JOIN/x`.
#    A spelling that only ever gets 502 bypasses the limiter, and the check fails. The same for /app/api/display/*.
set -euo pipefail

image=${1:-}
[ -n "$image" ] || { echo "usage: check-edge-image.sh EDGE_IMAGE_REFERENCE" >&2; exit 2; }
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
for tool in docker curl openssl; do command -v "$tool" >/dev/null 2>&1 || { echo "missing tool: $tool" >&2; exit 1; }; done

work=$(mktemp -d)
container=""
cleanup() { [ -z "$container" ] || docker rm -f "$container" >/dev/null 2>&1 || true; rm -rf "$work"; }
trap cleanup EXIT
failures=0
fail() { echo "FAIL $*"; failures=$((failures + 1)); }
pass() { echo "PASS $*"; }

# A throwaway CA certificate stands in for the internal CA the Caddyfile trusts; an empty docs tree and an empty extra directory.
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj "/CN=check-edge" -keyout "$work/k.pem" -out "$work/internal_ca_certificate" >/dev/null 2>&1
mkdir -p "$work/docs" "$work/extra" "$work/data"
chmod -R a+rX "$work"

run_caddy() { # run_caddy <caddyfile> [docker args...] -- <command...>
  local caddyfile=$1; shift
  docker run --rm --user 10010:10010 \
    --volume "$root/deploy/edge/$caddyfile:/etc/caddy/Caddyfile:ro" \
    --volume "$work/extra:/etc/caddy/extra:ro" \
    --volume "$work/internal_ca_certificate:/run/secrets/internal_ca_certificate:ro" \
    --volume "$work/docs:/srv/docs:ro" \
    "$@"
}

# --- 1. caddy validate --------------------------------------------------------------------------------------------------------------
for caddyfile in Caddyfile.production Caddyfile.staging; do
  for mode in set empty unset; do
    envargs=(--env DF_PUBLIC_HOST=check.example.test --env DF_ACME_EMAIL=ops@example.test)
    case "$mode" in
      set) envargs+=(--env "DF_ALLOWED_CIDRS=10.0.0.0/8 ::/0") ;;
      empty) envargs+=(--env DF_ALLOWED_CIDRS=) ;;
      unset) ;;
    esac
    if out=$(run_caddy "$caddyfile" "${envargs[@]}" "$image" caddy validate --config /etc/caddy/Caddyfile 2>&1); then
      pass "$caddyfile validates with DF_ALLOWED_CIDRS $mode"
    else
      fail "$caddyfile does not validate with DF_ALLOWED_CIDRS $mode: $(printf '%s' "$out" | tail -n 3 | tr '\n' ' ')"
    fi
  done
done

# --- 2. the rate limiter really sits in front of every spelling of the path ------------------------------------------------------
container=$(docker run -d --user 10010:10010 -p 127.0.0.1:18443:443 \
  --volume "$root/deploy/edge/Caddyfile.production:/etc/caddy/Caddyfile:ro" \
  --volume "$work/extra:/etc/caddy/extra:ro" \
  --volume "$work/internal_ca_certificate:/run/secrets/internal_ca_certificate:ro" \
  --volume "$work/docs:/srv/docs:ro" \
  --volume "$work/data:/data" \
  --env DF_PUBLIC_HOST=localhost --env DF_ALLOWED_CIDRS="0.0.0.0/0 ::/0" \
  --env DF_RATE_JOIN_EVENTS=5 --env DF_RATE_DISPLAY_EVENTS=5 --env DF_RATE_WINDOW=3s \
  "$image")
ready=0
for _ in $(seq 1 40); do
  if curl -ks -o /dev/null --resolve localhost:18443:127.0.0.1 https://localhost:18443/healthz-probe; then ready=1; break; fi
  sleep 0.5
done
[ "$ready" -eq 1 ] || { docker logs "$container" 2>&1 | tail -n 20; fail "the edge did not start"; echo "$failures failure(s)"; exit 1; }

limited() { # limited <path>: 12 requests in a burst; passes when at least one is answered 429
  local path=$1 code seen=0
  for _ in $(seq 1 12); do
    code=$(curl -ks -o /dev/null -w '%{http_code}' --path-as-is --resolve localhost:18443:127.0.0.1 "https://localhost:18443$path" || true)
    [ "$code" != 429 ] || seen=$((seen + 1))
  done
  [ "$seen" -ge 1 ]
}
for path in /app/api/join/x //app/api/join/x /app//api/join/x /app/api/join%2Fx /app/api/%6Aoin/x /APP/API/JOIN/x \
            /app/api/display/x //app/api/display/x /app//api/display/x /app/api/display%2Fx; do
  sleep 4 # one client address: let the 3 s window of the previous burst pass
  if limited "$path"; then pass "the limiter answers 429 for $path"; else fail "the limiter is bypassed by $path (no 429 in a burst of 12)"; fi
done

echo "$failures failure(s)"
[ "$failures" -eq 0 ]
