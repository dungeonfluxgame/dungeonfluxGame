#!/usr/bin/env bash
# Check deploy/db/scram-verifier.sh against the RFC 7677 SCRAM-SHA-256 example exchange (user "user", password
# "pencil", salt W22ZaJ0SNY7soEsUEjb6gQ==, 4096 iterations). The RFC publishes the client proof for that exchange;
# recomputing it from our SaltedPassword/ClientKey/StoredKey proves the derivation behind the verifier is right,
# and a second check proves the verifier string has PostgreSQL's shape. Needs openssl 3, od, sed. No network.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck disable=SC1091
. "$here/scram-verifier.sh"

hex2bin() { printf "$(printf '%s' "$1" | sed 's/../\\x&/g')"; }
bin2hex() { od -An -v -tx1 | tr -d ' \n'; }

salt='W22ZaJ0SNY7soEsUEjb6gQ=='
salted=$(scram_salted_hex pencil "$salt" 4096)
client_key=$(scram_hmac_bin "$salted" 'Client Key' | bin2hex)
stored_key=$(hex2bin "$client_key" | openssl dgst -sha256 -binary | bin2hex)
auth_message='n=user,r=rOprNGfwEbeRWgbNEkqO,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096,c=biws,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0'
signature=$(scram_hmac_bin "$stored_key" "$auth_message" | bin2hex)

proof_hex=""
for i in $(seq 0 31); do
  a=$((16#${client_key:$((i * 2)):2}))
  b=$((16#${signature:$((i * 2)):2}))
  proof_hex+=$(printf '%02x' $((a ^ b)))
done
proof=$(hex2bin "$proof_hex" | openssl base64 -A)
expected='dHzbZapWIk4jUhN+Ute9ytag9zjfMHgsqmmiz7AndVQ='
failures=0
if [ "$proof" = "$expected" ]; then
  echo "PASS RFC 7677 client proof reproduced"
else
  echo "FAIL client proof $proof, expected $expected"; failures=$((failures + 1))
fi

tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
printf '%s' pencil > "$tmp"
verifier=$(scram_verifier "$tmp" 4096 "$salt")
case "$verifier" in
  'SCRAM-SHA-256$4096:W22ZaJ0SNY7soEsUEjb6gQ==$'*:*) echo "PASS verifier shape: ${verifier%%\$*}\$4096:<salt>\$<StoredKey>:<ServerKey>" ;;
  *) echo "FAIL verifier shape: $verifier"; failures=$((failures + 1)) ;;
esac
stored_b64=$(hex2bin "$stored_key" | openssl base64 -A)
case "$verifier" in
  *"\$$stored_b64:"*) echo "PASS verifier carries the StoredKey of the RFC example" ;;
  *) echo "FAIL verifier StoredKey differs"; failures=$((failures + 1)) ;;
esac
[ "$failures" -eq 0 ] || exit 1
echo "scram verifier ok"
