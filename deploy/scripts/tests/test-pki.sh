#!/usr/bin/env bash
# Checks the staging PKI that deploy/scripts/staging-secrets.sh generates against the rules of the production design (manifest procedure
# internal-pki): two intermediate CAs with pathlen:0, the PLACEMENT intermediate limited to client authentication with a name constraint
# that permits no DNS name, so it can never sign a certificate that verifies as a server. Needs openssl. Generates the throwaway staging
# secrets if they do not exist (it never overwrites them).
#   deploy/scripts/tests/test-pki.sh
set -uo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
command -v openssl >/dev/null 2>&1 || { echo "missing tool: openssl" >&2; exit 1; }
cd "$root"
d=artifacts/tmp/staging/secrets  # relative: a native openssl on Windows cannot open MSYS absolute paths
[ -d "$d/dispatcher" ] || bash "$root/deploy/scripts/staging-secrets.sh" >/dev/null
export MSYS_NO_PATHCONV=1
failures=0
check() { local what=$1; shift; if "$@"; then echo "PASS $what"; else echo "FAIL $what"; failures=$((failures + 1)); fi; }
ca=$d/control-plane/internal_ca_certificate
placement=$d/dispatcher/placement_issuer_certificate
service=$d/dispatcher/service_issuer_certificate
text() { openssl x509 -in "$1" -noout -text; }

check "the placement issuer is a CA with pathlen:0" bash -c 'openssl x509 -in "$1" -noout -text | grep -q "CA:TRUE, pathlen:0"' _ "$placement"
check "the service issuer is a CA with pathlen:0" bash -c 'openssl x509 -in "$1" -noout -text | grep -q "CA:TRUE, pathlen:0"' _ "$service"
check "the placement issuer may only authenticate clients" bash -c 'openssl x509 -in "$1" -noout -text | grep -A1 "Extended Key Usage" | grep -q "TLS Web Client Authentication" && ! openssl x509 -in "$1" -noout -text | grep -A1 "Extended Key Usage" | grep -q "Server"' _ "$placement"
check "the placement issuer has a critical name constraint that permits one impossible DNS name" bash -c 'openssl x509 -in "$1" -noout -text | grep -A2 "Name Constraints" | grep -q "permit-nothing.invalid"' _ "$placement"
check "a runtime certificate verifies as a CLIENT through the placement issuer" openssl verify -purpose sslclient -CAfile "$ca" -untrusted "$placement" "$d/runtime-1/internal_tls_runtime_certificate"
check "a runtime certificate does NOT verify as a SERVER" bash -c '! openssl verify -purpose sslserver -CAfile "$1" -untrusted "$2" "$3" >/dev/null 2>&1' _ "$ca" "$placement" "$d/runtime-1/internal_tls_runtime_certificate"
check "a service server certificate verifies as a server through the service issuer" openssl verify -purpose sslserver -CAfile "$ca" -untrusted "$service" "$d/dispatcher/dispatch_tls_certificate"
check "a service certificate does NOT verify through the placement issuer" bash -c '! openssl verify -CAfile "$1" -untrusted "$2" "$3" >/dev/null 2>&1' _ "$ca" "$placement" "$d/dispatcher/dispatch_tls_certificate"
check "the pinned service issuer is the issuer of every service certificate" bash -c 'openssl x509 -in "$1" -noout -issuer | grep -q service-issuer' _ "$d/dispatcher/dispatch_tls_certificate"

# --- the runtime-server issuer (decision 33): online, held by the control plane, serverAuth only ----------------------------------------
rsi=$d/control-plane/runtime_server_issuer_certificate
listener=$d/runtime-1/game_listener_certificate
check "the runtime-server issuer is a CA with pathlen:0" bash -c 'openssl x509 -in "$1" -noout -text | grep -q "CA:TRUE, pathlen:0"' _ "$rsi"
check "the runtime-server issuer may only authenticate servers" bash -c 'openssl x509 -in "$1" -noout -text | grep -A1 "Extended Key Usage" | grep -q "TLS Web Server Authentication" && ! openssl x509 -in "$1" -noout -text | grep -A1 "Extended Key Usage" | grep -q "Client"' _ "$rsi"
check "the runtime-server issuer permits no DNS name" bash -c 'openssl x509 -in "$1" -noout -text | grep -A2 "Name Constraints" | grep -q "permit-nothing.invalid"' _ "$rsi"
check "a game-listener certificate carries exactly the placement-server URI SAN" bash -c 'openssl x509 -in "$1" -noout -text | grep -A1 "Subject Alternative Name" | grep -q "URI:urn:dungeonflux:placement-server:00000000-0000-0000-0000-000000000003" && ! openssl x509 -in "$1" -noout -text | grep -q "DNS:"' _ "$listener"
check "a game-listener certificate verifies as a SERVER through the runtime-server issuer" openssl verify -purpose sslserver -CAfile "$ca" -untrusted "$rsi" "$listener"
check "...and does NOT verify as a CLIENT" bash -c '! openssl verify -purpose sslclient -CAfile "$1" -untrusted "$2" "$3" >/dev/null 2>&1' _ "$ca" "$rsi" "$listener"
check "...and does NOT verify through the service issuer or the placement issuer" bash -c '! openssl verify -CAfile "$1" -untrusted "$2" "$4" >/dev/null 2>&1 && ! openssl verify -CAfile "$1" -untrusted "$3" "$4" >/dev/null 2>&1' _ "$ca" "$service" "$placement" "$listener"
check "a placement client certificate does NOT verify through the runtime-server issuer" bash -c '! openssl verify -CAfile "$1" -untrusted "$2" "$3" >/dev/null 2>&1' _ "$ca" "$rsi" "$d/runtime-1/internal_tls_runtime_certificate"
check "the runtime-server issuer KEY is in the control plane's directory and in no other consumer's" bash -c 'test -s "$1/control-plane/runtime_server_issuer_key" && test -z "$(find "$1" -name runtime_server_issuer_key -not -path "$1/control-plane/*")"' _ "$d"
check "no other directory holds a private key of any issuer" bash -c 'test -z "$(find "$1" \( -name "*issuer_key" \) -not -path "$1/control-plane/runtime_server_issuer_key")"' _ "$d"

# --- no certificate carries a hostname-like common name: identity is the SAN, never the CN (a verifier that falls back to the CN could be
# talked into accepting a service name that only appears there) ---------------------------------------------------------------------------
bad_cn=""
for cert in $(find "$d" -type f \( -name '*_certificate' -o -name '*.crt' -o -name public.crt \) | sort); do
  cn=$(openssl x509 -in "$cert" -noout -subject -nameopt multiline 2>/dev/null | sed -n 's/^ *commonName *= *//p' | head -n 1)
  [ -z "$cn" ] || ! [[ "$cn" =~ ^[A-Za-z0-9.-]+$ ]] || bad_cn="$bad_cn $cert($cn)"
done
check "no certificate carries a hostname-like common name" test -z "$bad_cn"

echo "$failures failure(s)"
[ "$failures" -eq 0 ]
