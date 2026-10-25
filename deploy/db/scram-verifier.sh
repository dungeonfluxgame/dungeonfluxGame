#!/usr/bin/env bash
# Compute a PostgreSQL SCRAM-SHA-256 password verifier ON THE CLIENT, so the plaintext password is never sent
# to the server (critic round 1c, item 6). On managed PostgreSQL the text of an ALTER ROLE ... PASSWORD
# statement can land in pg_stat_activity, pg_stat_statements and error logs, and the administrator cannot
# switch statement logging off. A verifier is not a plaintext password: it cannot be used to log in as the
# role, only to check one (and, with a high-entropy generated password, it is not practically guessable).
#
#   deploy/db/scram-verifier.sh PASSWORD_FILE [ITERATIONS]     prints  SCRAM-SHA-256$4096:<salt>$<StoredKey>:<ServerKey>
#   source it and call scram_verifier PASSWORD_FILE [ITERATIONS] [SALT_BASE64]   (salt only for tests)
# Needs openssl 3 (`openssl kdf`, `openssl dgst -mac HMAC`), od, tr. Passwords are ASCII (SASLprep is the identity
# on them; deploy/scripts/gen-secret.sh produces only [A-Za-z0-9]). The password reaches `openssl kdf` as an
# argument on THIS machine for the duration of one process: run it on the operator workstation or CI runner,
# not on a shared host. The derivation follows RFC 5802/7677 and PostgreSQL's scram-common.c:
#   SaltedPassword = PBKDF2-HMAC-SHA-256(password, salt, iterations, 32)
#   StoredKey = SHA-256(HMAC(SaltedPassword, "Client Key")),  ServerKey = HMAC(SaltedPassword, "Server Key")
# deploy/db/test-scram-verifier.sh checks it against the RFC 7677 example exchange.
set -euo pipefail

scram_salted_hex() { # scram_salted_hex PASSWORD SALT_BASE64 ITERATIONS -> hex of SaltedPassword
  local pw=$1 salt_b64=$2 iterations=$3 salt_hex
  salt_hex=$(printf '%s' "$salt_b64" | openssl base64 -d -A | od -An -v -tx1 | tr -d ' \n')
  openssl kdf -keylen 32 -kdfopt digest:SHA256 -kdfopt "pass:$pw" -kdfopt "hexsalt:$salt_hex" \
    -kdfopt "iter:$iterations" PBKDF2 | tr -d ':\r\n' | tr 'A-F' 'a-f'
}

scram_hmac_bin() { # scram_hmac_bin HEXKEY MESSAGE -> raw HMAC-SHA-256 on stdout
  printf '%s' "$2" | openssl dgst -sha256 -mac HMAC -macopt "hexkey:$1" -binary
}

scram_verifier() { # scram_verifier PASSWORD_FILE [ITERATIONS] [SALT_BASE64]
  local file=$1 iterations=${2:-4096} salt_b64=${3:-} pw salted stored server
  [ -s "$file" ] || { echo "empty or missing password file: $file" >&2; return 1; }
  pw=$(cat "$file")
  [ -n "$salt_b64" ] || salt_b64=$(openssl rand -base64 16)
  salted=$(scram_salted_hex "$pw" "$salt_b64" "$iterations")
  stored=$(scram_hmac_bin "$salted" 'Client Key' | openssl dgst -sha256 -binary | openssl base64 -A)
  server=$(scram_hmac_bin "$salted" 'Server Key' | openssl base64 -A)
  printf 'SCRAM-SHA-256$%s:%s$%s:%s\n' "$iterations" "$salt_b64" "$stored" "$server"
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  command -v openssl >/dev/null 2>&1 || { echo "missing tool: openssl" >&2; exit 1; }
  scram_verifier "${1:?usage: scram-verifier.sh PASSWORD_FILE [ITERATIONS]}" "${2:-4096}"
fi
