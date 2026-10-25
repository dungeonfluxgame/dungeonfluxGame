#!/usr/bin/env bash
# Generate THROWAWAY secrets for the local staging mirror, laid out like production: one directory per
# consumer (control-plane, runtime-1, runtime-2, dispatcher, ingestor, migrate, postgres, object-store,
# bucket-init), so a service mounts only its own directory. Also a private CA and the certificates the TLS paths
# need: the PostgreSQL server, the object store, the control plane's gateway server certificate, the dispatcher's
# and the ingestor's server certificates, and CLIENT certificates whose URI SAN carries the caller's identity:
#   urn:dungeonflux:control-plane   urn:dungeonflux:dispatcher   urn:dungeonflux:placement:<placement id>
# Each runtime gets its OWN certificate with its own placement id (runtime-1: ...0003, runtime-2: ...0013, the ids in
# deploy/config/staging/runtime-N.descriptor.toml), so a server can prove it takes the placement id from the certificate.
# Writes only under artifacts/tmp/staging/secrets (gitignored). Nothing here is ever a real credential; files are
# mode 0444 because containers run as other users on a developer workstation (production: 0440 root:dfs-<consumer>,
# and runner-issued per-slot files 0400 to the slot's uid). It contains NO payment, identity, email or provider key:
# staging uses fakes (decision e). Needs: openssl.
#
# usage: deploy/scripts/staging-secrets.sh [--force]
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
dir=${DF_STAGING_SECRETS_DIR_ABS:-$root/artifacts/tmp/staging/secrets}
force=0
if [ "${1:-}" = "--force" ]; then force=1; fi

command -v openssl >/dev/null 2>&1 || { echo "missing tool: openssl" >&2; exit 1; }
# Git Bash on Windows rewrites arguments that start with a slash (the -subj values); stop it.
export MSYS_NO_PATHCONV=1
trap 'echo "staging-secrets.sh failed at line $LINENO" >&2' ERR

if [ -e "$dir/postgres/db_admin_password" ] && [ "$force" -eq 0 ]; then
  echo "staging secrets already exist in $dir (use --force to regenerate)"
  exit 0
fi

umask 022
# rm -rf takes its path from an environment variable, so refuse anything but the staging secrets directory (critic round 1d, item 14).
case "$dir" in
  /*/artifacts/tmp/staging/secrets) ;;
  *) echo "refusing: DF_STAGING_SECRETS_DIR_ABS must be an absolute path ending in /artifacts/tmp/staging/secrets (got: $dir)" >&2; exit 1 ;;
esac
case "$dir" in *..*|*'*'*|*' '*) echo "refusing: suspicious path: $dir" >&2; exit 1 ;; esac
rm -rf "$dir"
work="$dir.work"
rm -rf "$work"
mkdir -p "$work"
trap 'rm -rf "$work"; echo "staging-secrets.sh failed at line $LINENO" >&2' ERR
mkdir -p "$dir"
for consumer in control-plane runtime-1 runtime-2 dispatcher ingestor edge migrate postgres object-store object-store/certs bucket-init; do
  mkdir -p "$dir/$consumer"
done

random_value() { openssl rand -base64 36 | tr -d '\n=+/'; }
put() { # put <consumer> <file> <value>
  printf '%s' "$3" > "$dir/$1/$2"
}
copy() { # copy <from consumer/file> <to consumer> [new name]
  cp "$dir/$1" "$dir/$2/${3:-$(basename "$1")}"
}

# --- passwords and keys -------------------------------------------------------------------------
for role in migrator control runner campaign_runtime egress billing support; do
  put postgres "db_${role}_password" "$(random_value)"
done
put postgres db_admin_password "$(random_value)"
copy postgres/db_migrator_password migrate
copy postgres/db_control_password control-plane
copy postgres/db_egress_password dispatcher
for runtime in runtime-1 runtime-2; do
  copy postgres/db_campaign_runtime_password "$runtime"
  put "$runtime" placement_token "$(random_value)"
done
put control-plane session_signing_key "$(random_value)"

# S3 access key ids are short identifiers. Staging has ONE object-store root credential behind every role.
objstore_key=$(openssl rand -hex 10)
objstore_secret=$(random_value)
put object-store objstore_root_user "$objstore_key"
put object-store objstore_root_password "$objstore_secret"
put bucket-init storage_access_key_id "$objstore_key"
put bucket-init storage_secret_key "$objstore_secret"
for name in storage_media storage_exports; do
  put control-plane "${name}_access_key_id" "$objstore_key"
  put control-plane "${name}_secret_key" "$objstore_secret"
done

# --- private CA and certificates ------------------------------------------------------------------
# A native openssl on Windows cannot open MSYS paths: work with relative names inside the work directory.
cd "$work"
openssl req -x509 -newkey rsa:2048 -nodes -days 30 -keyout "ca.key" -out "ca.crt" \
  -subj "/CN=dungeonflux-staging-ca" >/dev/null 2>&1

issue() { # issue <name> <subjectAltName list> <extendedKeyUsage>
  local name=$1 sans=$2 usage=$3
  openssl req -newkey rsa:2048 -nodes -keyout "$name.key" -out "$name.csr" -subj "/CN=$name" >/dev/null 2>&1
  printf 'subjectAltName=%s\nextendedKeyUsage=%s\nbasicConstraints=CA:FALSE\n' "$sans" "$usage" > "$name.ext"
  openssl x509 -req -in "$name.csr" -CA "ca.crt" -CAkey "ca.key" -CAcreateserial \
    -out "$name.crt" -days 30 -extfile "$name.ext" >/dev/null 2>&1
}
issue postgres "DNS:postgres,DNS:localhost,IP:172.30.2.2" serverAuth
issue object-store "DNS:object-store,IP:172.30.2.3" serverAuth
# One gateway certificate for both control-plane gateway listeners (media gateway on the slot networks, dispatcher
# gateway on the dispatch network).
issue media-gateway "DNS:control-plane,IP:172.30.1.3,IP:172.30.2.4,IP:172.30.11.3,IP:172.30.12.3,IP:172.30.4.3" serverAuth
issue dispatch "DNS:egress,IP:172.30.11.4,IP:172.30.12.4" serverAuth
issue ingestor "DNS:ingestor,IP:172.30.2.8,IP:172.30.4.5,IP:172.30.11.5,IP:172.30.12.5" serverAuth
issue control-client "URI:urn:dungeonflux:control-plane" clientAuth,serverAuth
issue dispatcher-client "URI:urn:dungeonflux:dispatcher" clientAuth
issue runtime-1-client "URI:urn:dungeonflux:placement:00000000-0000-0000-0000-000000000003" clientAuth,serverAuth
issue runtime-2-client "URI:urn:dungeonflux:placement:00000000-0000-0000-0000-000000000013" clientAuth,serverAuth
cd "$root"

put_file() { cp "$work/$1" "$dir/$2/$3"; }
for consumer in control-plane runtime-1 runtime-2 dispatcher migrate postgres; do
  put_file ca.crt "$consumer" db_ca_certificate
done
put_file postgres.crt postgres pg_server_certificate
put_file postgres.key postgres pg_server_key
put_file object-store.crt object-store/certs public.crt
put_file object-store.key object-store/certs private.key
put_file ca.crt control-plane object_store_ca_certificate
put_file ca.crt bucket-init object_store_ca_certificate
for consumer in control-plane runtime-1 runtime-2 dispatcher ingestor edge; do
  put_file ca.crt "$consumer" internal_ca_certificate
done
put_file media-gateway.crt control-plane media_gateway_certificate
put_file media-gateway.key control-plane media_gateway_key
put_file control-client.crt control-plane internal_tls_control_certificate
put_file control-client.key control-plane internal_tls_control_key
put_file dispatch.crt dispatcher dispatch_tls_certificate
put_file dispatch.key dispatcher dispatch_tls_key
put_file dispatcher-client.crt dispatcher dispatcher_client_certificate
put_file dispatcher-client.key dispatcher dispatcher_client_key
put_file ingestor.crt ingestor ingestor_tls_certificate
put_file ingestor.key ingestor ingestor_tls_key
for n in 1 2; do
  put_file "runtime-$n-client.crt" "runtime-$n" internal_tls_runtime_certificate
  put_file "runtime-$n-client.key" "runtime-$n" internal_tls_runtime_key
done

# The role statements (passwords as SCRAM verifiers computed here, on the client) for the staging init script.
bash "$root/deploy/db/create-roles.sh" --password-dir "$dir/postgres" --database dungeonflux > "$dir/postgres/roles.sql"

find "$dir" -type f -exec chmod 0444 {} +
rm -rf "$work"
echo "wrote throwaway staging secrets, one directory per consumer, to $dir"
