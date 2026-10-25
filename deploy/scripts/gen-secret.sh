#!/usr/bin/env bash
# Generate a random secret value into a file with mode 0600, for the operator to store in the password manager
# and to place in the push directory. Prints only the file name, never the value. Needs openssl.
#   deploy/scripts/gen-secret.sh DIR NAME [BYTES]      -> DIR/NAME  (default 36 random bytes, base64 without padding)
# Application database passwords generated this way are fed to deploy/db/create-roles.sh; they never pass
# through Terraform. Refuses to overwrite an existing file.
set -euo pipefail

dir=${1:?usage: gen-secret.sh DIR NAME [BYTES]}
name=${2:?usage: gen-secret.sh DIR NAME [BYTES]}
bytes=${3:-36}
command -v openssl >/dev/null 2>&1 || { echo "missing tool: openssl" >&2; exit 1; }
case "$name" in *[!A-Za-z0-9_.-]*|""|.*) echo "bad secret name: $name" >&2; exit 2 ;; esac
[ "$bytes" -ge 24 ] 2>/dev/null || { echo "BYTES must be a number of at least 24" >&2; exit 2; }
umask 077
mkdir -p "$dir"
chmod 700 "$dir" 2>/dev/null || true
[ ! -e "$dir/$name" ] || { echo "refusing to overwrite $dir/$name" >&2; exit 1; }
openssl rand -base64 "$bytes" | tr -d '\n=+/' > "$dir/$name"
echo "wrote $dir/$name"
