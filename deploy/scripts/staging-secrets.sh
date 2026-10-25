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
for role in migrator control runner campaign_runtime egress accounts campaigns finance support; do
  put postgres "db_${role}_password" "$(random_value)"
done
put postgres db_admin_password "$(random_value)"
copy postgres/db_migrator_password migrate
copy postgres/db_control_password control-plane
copy postgres/db_accounts_password control-plane
copy postgres/db_campaigns_password control-plane
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
  -subj "/CN=DungeonFlux staging root CA" >/dev/null 2>&1

# Two intermediate CAs under the root, as in production (manifest procedure internal-pki): the SERVICE issuer signs the fixed identities and
# the server certificates, the PLACEMENT issuer signs only per-placement runtime client certificates. Both are CA:TRUE, pathlen:0.
make_issuer() { # make_issuer <name>
  local name=$1
  openssl req -newkey rsa:2048 -nodes -keyout "$name.key" -out "$name.csr" -subj "/CN=DungeonFlux staging issuer $name" >/dev/null 2>&1
  # The PLACEMENT issuer is limited twice: its EKU is clientAuth only and its name constraints permit no DNS name at all, so even if its key
  # leaks it can never sign a certificate that verifies as a server (the EKU stops Go and OpenSSL; the constraint stops a verifier that
  # ignores EKU). Runtime leaves carry URI names only, which a DNS constraint does not touch. The SERVICE issuer is not constrained so.
  if [ "$name" = runtime-server-issuer ]; then
    # The RUNTIME-SERVER issuer (decision 33): ONLINE, held by the control plane, it signs each placement's game-listener certificate. serverAuth
    # only, pathlen:0, and a DNS constraint that permits nothing, so it can sign neither a client certificate nor a certificate for any
    # service host name; the listener certificates carry URI names only (urn:dungeonflux:placement-server:<id>).
    printf 'basicConstraints=critical,CA:TRUE,pathlen:0\nkeyUsage=critical,keyCertSign,cRLSign\nextendedKeyUsage=serverAuth\nnameConstraints=critical,permitted;DNS:permit-nothing.invalid\n' > "$name.ext"
  elif [ "$name" = placement-issuer ]; then
    printf 'basicConstraints=critical,CA:TRUE,pathlen:0\nkeyUsage=critical,keyCertSign,cRLSign\nextendedKeyUsage=clientAuth\nnameConstraints=critical,permitted;DNS:permit-nothing.invalid\n' > "$name.ext"
  else
    printf 'basicConstraints=critical,CA:TRUE,pathlen:0\nkeyUsage=critical,keyCertSign,cRLSign\n' > "$name.ext"
  fi
  openssl x509 -req -in "$name.csr" -CA "ca.crt" -CAkey "ca.key" -CAcreateserial \
    -out "$name.crt" -days 30 -extfile "$name.ext" >/dev/null 2>&1
}
make_issuer service-issuer
make_issuer placement-issuer
make_issuer runtime-server-issuer

issue() { # issue <name> <subjectAltName list> <extendedKeyUsage> [issuer name; default: the root]
  local name=$1 sans=$2 usage=$3 signer=${4:-ca}
  openssl req -newkey rsa:2048 -nodes -keyout "$name.key" -out "$name.csr" -subj "/CN=DungeonFlux staging leaf $name" >/dev/null 2>&1
  printf 'subjectAltName=%s\nextendedKeyUsage=%s\nbasicConstraints=CA:FALSE\n' "$sans" "$usage" > "$name.ext"
  openssl x509 -req -in "$name.csr" -CA "$signer.crt" -CAkey "$signer.key" -CAcreateserial \
    -out "$name.crt" -days 30 -extfile "$name.ext" >/dev/null 2>&1
  # A leaf issued by an intermediate is presented with that intermediate behind it: the receiver checks the chain passes through
  # the issuer it pins.
  if [ "$signer" != ca ]; then cat "$signer.crt" >> "$name.crt"; fi
}
issue postgres "DNS:postgres,DNS:localhost,IP:172.30.2.2" serverAuth
issue object-store "DNS:object-store,IP:172.30.2.3" serverAuth
# One gateway certificate for both control-plane gateway listeners (media gateway on the slot networks, dispatcher
# gateway on the dispatch network).
issue media-gateway "DNS:control-plane,IP:172.30.1.3,IP:172.30.2.4,IP:172.30.11.3,IP:172.30.12.3,IP:172.30.4.3" serverAuth service-issuer
issue dispatch "DNS:egress,IP:172.30.11.4,IP:172.30.12.4" serverAuth service-issuer
issue ingestor "DNS:ingestor,IP:172.30.2.8,IP:172.30.4.5,IP:172.30.11.5,IP:172.30.12.5" serverAuth service-issuer
issue control-client "URI:urn:dungeonflux:control-plane" clientAuth,serverAuth service-issuer
issue dispatcher-client "URI:urn:dungeonflux:dispatcher" clientAuth service-issuer
issue runtime-1-client "URI:urn:dungeonflux:placement:00000000-0000-0000-0000-000000000003" clientAuth placement-issuer
issue runtime-2-client "URI:urn:dungeonflux:placement:00000000-0000-0000-0000-000000000013" clientAuth placement-issuer
# The game-listener certificates (decision 33): serverAuth, URI SAN urn:dungeonflux:placement-server:<id>, signed by the runtime-server issuer.
# In production the control plane signs them at placement time (the runtime keeps its own key); staging pre-issues one per runtime slot.
issue runtime-1-listener "URI:urn:dungeonflux:placement-server:00000000-0000-0000-0000-000000000003" serverAuth runtime-server-issuer
issue runtime-2-listener "URI:urn:dungeonflux:placement-server:00000000-0000-0000-0000-000000000013" serverAuth runtime-server-issuer
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
for consumer in runtime-1 runtime-2 edge; do
  put_file service-issuer.crt "$consumer" service_issuer_certificate
done
for consumer in control-plane dispatcher ingestor; do
  put_file placement-issuer.crt "$consumer" placement_issuer_certificate
  put_file service-issuer.crt "$consumer" service_issuer_certificate
done
# The runtime-server issuer: the KEY is the control plane's alone; the certificate is public, for whoever verifies a game listener.
put_file runtime-server-issuer.crt control-plane runtime_server_issuer_certificate
put_file runtime-server-issuer.key control-plane runtime_server_issuer_key
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
  put_file "runtime-$n-listener.crt" "runtime-$n" game_listener_certificate
  put_file "runtime-$n-listener.key" "runtime-$n" game_listener_key
  put_file "runtime-$n-client.crt" "runtime-$n" internal_tls_runtime_certificate
  put_file "runtime-$n-client.key" "runtime-$n" internal_tls_runtime_key
done

# The role statements (passwords as SCRAM verifiers computed here, on the client) for the staging init script.
bash "$root/deploy/db/create-roles.sh" --password-dir "$dir/postgres" --database dungeonflux > "$dir/postgres/roles.sql"

find "$dir" -type f -exec chmod 0444 {} +
rm -rf "$work"
echo "wrote throwaway staging secrets, one directory per consumer, to $dir"
