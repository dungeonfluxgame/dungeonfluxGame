#!/usr/bin/env bash
# Static policy check of the TOML configuration files under deploy/config (critic round 1 items 9 and 18; the
# hosted binary reads one TOML file and no environment variables, and every network hop is TLS).
#   deploy/scripts/check-config-policy.sh
# Needs bash and grep only. Exit 1 on any violation. CI-safe.
# Asserts: no plaintext http:// or ws:// endpoint to a non-loopback host; no tls mode other than verify-full
# and no loopback-plaintext/disable; every key that names a credential ends in _file (a value never appears
# in a config); no [providers] mode = "live" without allow_live; a runtime descriptor names no provider,
# payment, identity or object-store setting; staging TOML never selects a live mode.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
failures=0
fail() { echo "FAIL $*"; failures=$((failures + 1)); }
pass() { echo "PASS $*"; }

shopt -s nullglob
files=("$root"/deploy/config/*/*.toml)
[ "${#files[@]}" -gt 0 ] || { echo "no TOML files found under deploy/config" >&2; exit 1; }

for f in "${files[@]}"; do
  name=${f#"$root"/}
  code=$(sed 's/[[:space:]]*#.*$//' "$f")

  if printf '%s\n' "$code" | grep -Eq '(http|ws)://' ; then
    if printf '%s\n' "$code" | grep -E '(http|ws)://' | grep -Evq '(http|ws)://(127\.0\.0\.1|localhost|\[::1\])'; then
      fail "$name: plaintext http:// or ws:// endpoint to a non-loopback host"; continue
    fi
  fi
  if printf '%s\n' "$code" | grep -Eq 'mode[[:space:]]*=[[:space:]]*"(loopback-plaintext|disable|allow|prefer|require)"'; then
    fail "$name: a TLS mode other than verify-full"; continue
  fi
  # No listener binds a wildcard address: the control plane is multi-homed, and 0.0.0.0 would put its public and internal
  # listeners on the dispatch and runtime networks too (critic round 1d, item 12).
  if printf '%s\n' "$code" | grep -Eq '(^|[^0-9.])0\.0\.0\.0:|\[::\]:|"::"|"\*:'; then
    fail "$name: a wildcard bind address (0.0.0.0, [::], *) is not allowed; bind each listener to one network's address"; continue
  fi
  # A credential key must be a file path. Keys that look like credentials but do not end in _file are values.
  if printf '%s\n' "$code" | grep -Eiq '^[[:space:]]*[a-z_]*(password|secret|token|api_key|private_key|signing_key|access_key)[a-z_]*[[:space:]]*=' \
     && printf '%s\n' "$code" | grep -Ei '^[[:space:]]*[a-z_]*(password|secret|token|api_key|private_key|signing_key|access_key)[a-z_]*[[:space:]]*=' | grep -Evq '_file[[:space:]]*='; then
    fail "$name: a credential setting that is not a *_file path"; continue
  fi
  case "$name" in
    */staging/*)
      if printf '%s\n' "$code" | grep -Eq 'mode[[:space:]]*=[[:space:]]*"live"'; then fail "$name: staging selects a live mode"; continue; fi
      if printf '%s\n' "$code" | grep -Eq 'allow_live[[:space:]]*=[[:space:]]*true'; then fail "$name: staging sets allow_live"; continue; fi
      ;;
  esac
  case "$name" in
    *descriptor.toml)
      if printf '%s\n' "$code" | grep -Eq '^\[(providers|payments|identity|objects|email)\]'; then
        fail "$name: a runtime descriptor must not configure providers, payments, identity, objects or email"; continue
      fi
      ;;
  esac
  # Mutual TLS and caller identity (critic round 1c, items 2 and 4; coordinator ruling c).
  has() { printf '%s\n' "$code" | grep -Eq "$1"; }
  count() { printf '%s\n' "$code" | grep -Ec "$1" || true; }
  case "$name" in
    *egress.toml)
      if ! { has '^\[dispatch_tls\]' && has 'client_ca_file' && has 'client_certificate_uri_san'; }; then
        fail "$name: the dispatch listener must require client certificates and take the placement id from the URI SAN"; continue
      fi
      if ! { has '^\[control_plane\]' && has 'client_certificate_file'; }; then
        fail "$name: the dispatcher must present a client certificate to the control plane gateway"; continue
      fi
      ;;
    *ingestor.toml)
      metrics_net=$(printf '%s\n' "$code" | sed -n 's/^metrics *= *"\([0-9]*\.[0-9]*\.[0-9]*\)\..*/\1/p')
      otlp_nets=$(printf '%s\n' "$code" | grep '^otlp' | grep -o '[0-9]*\.[0-9]*\.[0-9]*\.[0-9]*:4317' | sed -E 's/\.[0-9]+:4317$//' | sort -u || true)
      if [ -z "$metrics_net" ] || printf '%s\n' "$otlp_nets" | grep -qx "$metrics_net"; then
        fail "$name: the metrics listener must be on a network of its own (the monitoring network), not one that carries OTLP intake"; continue
      fi
      if has 'allowed_prefixes'; then
        fail "$name: the ingestor must list exact allowed_identities for fixed services, not allowed_prefixes"; continue
      fi
      if ! { has 'allowed_identities' && has 'placement_prefix'; }; then
        fail "$name: the ingestor needs exact allowed_identities plus a placement_prefix"; continue
      fi
      if ! { has '^\[tls\]' && has 'client_ca_file' && has 'client_certificate_uri_san'; }; then
        fail "$name: the ingestor must require client certificates and label records by certificate identity"; continue
      fi
      ;;
    *descriptor.toml)
      if [ "$(count 'client_certificate_file')" -lt 3 ]; then
        fail "$name: the runtime must present its placement certificate to the dispatcher, the media gateway and the ingestor"; continue
      fi
      if ! { has '^\[listen_tls\]' && has 'allowed_identities'; }; then
        fail "$name: the game listener must authorize its caller by certificate identity"; continue
      fi
      ;;
    *control-plane.toml)
      if ! { has '^\[internal_tls\.media_gateway\]' && has '^\[internal_tls\.dispatcher_gateway\]' && has 'allowed_identities'; }; then
        fail "$name: the gateways must authorize callers by certificate identity"; continue
      fi
      pub=$(printf '%s\n' "$code" | sed -n 's/^public *= *"\([0-9]*\.[0-9]*\.[0-9]*\)\..*/\1/p')
      bad_listener=""
      for key in internal dispatcher_gateway; do
        addr=$(printf '%s\n' "$code" | sed -n "s/^$key *= *\"\([0-9]*\.[0-9]*\.[0-9]*\)\..*/\1/p")
        if [ -z "$addr" ] || [ "$addr" = "$pub" ]; then bad_listener=$key; fi
      done
      if [ -n "$bad_listener" ]; then
        fail "$name: the $bad_listener listener must be bound to a different network than the public listener"; continue
      fi
      if ! has '^\[listen_tls\]'; then fail "$name: the control plane's non-loopback listeners must be TLS ([listen_tls])"; continue; fi
      if ! has '^ready *= *"'; then fail "$name: the edge needs a readiness-only listener of its own"; continue; fi
      ;;
  esac
  pass "$name"
done

# Production templates (round 2): every @TOKEN@ must be a key of the release environment (manifest entry release_environment), no fake mode
# may be selected, payments stay in test mode until the owner's phase A, and every file the production compose file mounts must exist.
allowed_tokens=" INSTANCE_ID ENVIRONMENT PUBLIC_HOST DB_HOST DB_PORT DB_NAME DB_SERVER_NAME SPACES_HOST SPACES_REGION BUCKET_PREFIX "
before=$failures
for f in "$root"/deploy/config/production/*; do
  name=${f#"$root"/}
  for token in $(sed 's/[[:space:]]*#.*$//' "$f" | grep -o '@[A-Z][A-Z0-9_]*@' | tr -d '@' | sort -u); do
    case "$allowed_tokens" in *" $token "*) ;; *) fail "$name: unknown template token @${token}@ (not a release environment key)" ;; esac
  done
  case "$f" in
    *.toml)
      code=$(sed 's/[[:space:]]*#.*$//' "$f")
      if printf '%s\n' "$code" | grep -Eq 'mode[[:space:]]*=[[:space:]]*"fake"'; then fail "$name: production selects a fake mode"; fi
      if printf '%s\n' "$code" | grep -Eq '^\[payments\]' && printf '%s\n' "$code" | sed -n '/^\[payments\]/,/^\[/p' | grep -Eq 'mode[[:space:]]*=[[:space:]]*"live"'; then
        fail "$name: payments must stay in test mode until the owner completes the payment-processor phase"
      fi
      ;;
  esac
done
for mounted in $(grep -o '/opt/dungeonflux/current/config/[a-z][a-z.-]*' "$root/deploy/compose/production.compose.yml" | sort -u); do
  [ -f "$root/deploy/config/production/$(basename "$mounted")" ] || fail "production.compose.yml mounts $mounted but deploy/config/production has no such template"
done
if [ "$failures" -eq "$before" ]; then pass "production templates (tokens, no fake mode, payments in test mode, mounted files exist)"; fi

if [ "$failures" -ne 0 ]; then echo "$failures file(s) violate the config policy"; exit 1; fi
echo "config policy ok"
