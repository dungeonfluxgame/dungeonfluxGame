#!/usr/bin/env bash
# Smoke assertions against a running staging mirror. STATUS: UNVERIFIED (never run: no Docker daemon on the
# authoring machine; the bash syntax and the rendered compose topology were checked statically).
#   deploy/scripts/staging-up.sh up --profile runtime && deploy/scripts/staging-smoke.sh
# Every check states what must be true and fails if it is not: a command that fails for the wrong reason
# (a missing password, a typo) does not count as the expected refusal, because refusals are matched on the
# error text. Reachability is tested from INSIDE each service's network namespace with a probe container
# (`--network container:<service>`), so it tests the real network split (decisions 18 and 23).
# Not covered (needs the production server and other owners): a join and a resume through the game flow
# (TODO(O1)), the section 17 OPS cases (S4 owns the assertions), credential separation between storage
# roles (the staging store has one credential).
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
secrets="$root/artifacts/tmp/staging/secrets"
compose=(docker compose
  --project-directory "$root/deploy/compose"
  --file "$root/deploy/compose/staging.compose.yml"
  --env-file "$root/deploy/images/images.lock"
  --env-file "$root/deploy/compose/staging.env"
  --profile runtime)
port=$(sed -n 's/^DF_STAGING_PORT=//p' "$root/deploy/compose/staging.env")
probe_image=$(sed -n 's/^AWSCLI_IMAGE=//p' "$root/deploy/images/images.lock")
failures=0
pass() { echo "PASS $*"; }
fail() { echo "FAIL $*"; failures=$((failures + 1)); }

cid() { "${compose[@]}" ps -q "$1"; }

# --- service state ------------------------------------------------------------------------------
for service in postgres db-gate object-store ingestor egress control-plane edge runtime-1 runtime-2; do
  id=$(cid "$service")
  if [ -z "$id" ]; then fail "$service is not running"; continue; fi
  state=$(docker inspect --format '{{.State.Status}}' "$id")
  health=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' "$id")
  if [ "$state" = running ] && { [ "$health" = healthy ] || [ "$health" = none ]; }; then
    pass "$service is running (health: $health)"
  else
    fail "$service state=$state health=$health"
  fi
  if [ "$(docker inspect --format '{{.HostConfig.ReadonlyRootfs}}' "$id")" = true ]; then
    pass "$service root filesystem is read-only"
  else
    fail "$service root filesystem is writable"
  fi
done
# Every service runs as its own numeric uid:gid from staging.env (critic round 1d, item 1); never 0 or 65532.
env_value() { sed -n "s/^$1=//p" "$root/deploy/compose/staging.env"; }
declare -A expected_user=(
  [control-plane]="$(env_value DF_UID_CONTROL_PLANE):$(env_value DF_GID_CONTROL_PLANE)"
  [egress]="$(env_value DF_UID_DISPATCHER):$(env_value DF_GID_DISPATCHER)"
  [ingestor]="$(env_value DF_UID_INGESTOR):$(env_value DF_GID_INGESTOR)"
  [edge]="$(env_value DF_UID_EDGE):$(env_value DF_UID_EDGE)"
  [db-gate]="$(env_value DF_UID_DBGATE):$(env_value DF_UID_DBGATE)"
  [runtime-1]="$(env_value DF_UID_RUNTIME_1):$(env_value DF_GID_RUNTIME)"
  [runtime-2]="$(env_value DF_UID_RUNTIME_2):$(env_value DF_GID_RUNTIME)"
)
for service in "${!expected_user[@]}"; do
  user=$(docker inspect --format '{{.Config.User}}' "$(cid "$service")")
  case "$user" in
    0:*|65532:*|"") fail "$service runs as '$user' (root, the image default, or unset)" ;;
    "${expected_user[$service]}") pass "$service runs as $user" ;;
    *) fail "$service runs as '$user', expected ${expected_user[$service]}" ;;
  esac
done

# --- the edge answers -----------------------------------------------------------------------------
code=$(curl --silent --max-time 5 --output /dev/null --write-out '%{http_code}' "http://127.0.0.1:$port/" || true)
case "$code" in [1-5][0-9][0-9]) pass "edge answers on 127.0.0.1:$port (HTTP $code)" ;; *) fail "edge gave no HTTP answer ($code)" ;; esac

# --- reachability matrix, from inside each network namespace ----------------------------------------
probe() { # probe <service> <address> <port>: exit 0 when a TCP connection succeeds
  docker run --rm --network "container:$(cid "$1")" --entrypoint bash "$probe_image"     -c "timeout 3 bash -c '</dev/tcp/$2/$3'" >/dev/null 2>&1
}
reach() { if probe "$1" "$2" "$3"; then pass "$1 reaches $2:$3 ($4)"; else fail "$1 cannot reach $2:$3 ($4)"; fi; }
blocked() { if probe "$1" "$2" "$3"; then fail "$1 CAN reach $2:$3 ($4)"; else pass "$1 cannot reach $2:$3 ($4)"; fi; }

# A runtime reaches exactly four things (decision 23).
reach   runtime-1 172.30.11.2 5432 "db-gate, the only database path"
reach   runtime-1 172.30.11.4 8082 "egress dispatcher"
reach   runtime-1 172.30.11.3 8083 "control-plane media gateway (mTLS)"
reach   runtime-1 172.30.11.5 4317 "telemetry ingestor"
blocked runtime-1 172.30.11.3 8080 "control-plane public listener"
blocked runtime-1 172.30.11.3 8084 "control-plane readiness listener"
blocked runtime-1 172.30.11.3 9090 "control-plane internal listener"
blocked runtime-1 172.30.11.3 9093 "control-plane dispatcher gateway"
blocked runtime-1 172.30.11.4 9092 "egress health listener"
blocked runtime-1 172.30.2.2  5432 "PostgreSQL directly"
blocked runtime-1 172.30.2.3  9000 "object store"
blocked runtime-1 172.30.1.2  8080 "edge"
blocked runtime-1 172.30.12.10 8081 "the other runtime"
blocked runtime-1 172.30.12.2 5432 "the other slot's db-gate"
blocked runtime-1 172.30.3.2  443  "the dispatcher's provider network"
blocked runtime-1 1.1.1.1     443  "the internet"
blocked runtime-2 172.30.11.10 8081 "the first runtime"
# Only the edge is public. It reaches the control plane's public and READINESS-ONLY listeners and nothing else:
# not the internal listener (metrics, drain), not the dispatcher gateway, not the data tier, not a runtime.
reach   edge 172.30.1.3 8080 "control-plane public listener"
reach   edge 172.30.1.3 8084 "control-plane readiness-only listener"
blocked edge 172.30.1.3 9090 "control-plane internal listener (metrics, drain)"
blocked edge 172.30.1.3 9093 "control-plane dispatcher gateway"
blocked edge 172.30.2.4 9090 "control-plane internal listener on the data network"
blocked edge 172.30.2.2 5432 "PostgreSQL"
blocked edge 172.30.2.3 9000 "object store"
blocked edge 172.30.11.10 8081 "a runtime"
blocked edge 172.30.11.3 8083 "the media gateway"
blocked runtime-1 172.30.11.5 9464 "ingestor metrics listener"
blocked runtime-1 172.30.2.8 4317 "ingestor on the data network"
# The control plane sits on five networks; each listener is bound to ONE address, so from the dispatch network the
# dispatcher reaches only the dispatcher gateway (critic round 1d, item 5).
blocked egress 172.30.4.3 8080 "control-plane public listener"
blocked egress 172.30.4.3 8084 "control-plane readiness listener"
blocked egress 172.30.4.3 9090 "control-plane internal listener"
blocked egress 172.30.4.3 8083 "control-plane media gateway"
blocked egress 172.30.4.5 9464 "ingestor metrics listener"
blocked egress 172.30.2.8 9464 "ingestor metrics on the data network"
blocked egress 172.30.4.4 9092 "its own health listener from the network (it is loopback-only)"
# The dispatcher reaches PostgreSQL (through the db-gate), the control plane's dispatcher gateway and the
# ingestor; in staging it has no route to a provider. It is not on the data network.
reach   egress 172.30.4.2 5432 "db-gate"
reach   egress 172.30.4.3 9093 "control-plane dispatcher gateway (mTLS)"
reach   egress 172.30.4.5 4317 "telemetry ingestor"
blocked egress 172.30.2.2 5432 "PostgreSQL directly"
blocked egress 172.30.2.3 9000 "object store"
blocked egress 172.30.2.4 9090 "control-plane internal listener"
blocked egress 1.1.1.1    443  "the internet (staging never reaches a real provider)"

# --- mutual TLS: the caller's certificate is required, and its identity is the placement ------------------
# From inside runtime-1's namespace, with runtime-1's own secrets directory mounted: a handshake WITH its placement
# certificate must succeed against each server, and WITHOUT a client certificate it must be refused. (Whether the
# server then maps the certificate's URI SAN to the right placement id and rejects a request body that claims a
# different one is O1's test: it needs the real server.)
tls_probe() { # tls_probe <description> <address:port> <servername> <with-client-cert yes|no>
  local out cert_args=""
  if [ "$4" = yes ]; then cert_args="-cert /s/internal_tls_runtime_certificate -key /s/internal_tls_runtime_key"; fi
  out=$(docker run --rm --network "container:$(cid runtime-1)" --volume "$secrets/runtime-1:/s:ro" --entrypoint bash "$probe_image"     -c "echo | timeout 5 openssl s_client -connect $2 -servername $3 -verify_hostname $3 -CAfile /s/internal_ca_certificate -verify_return_error $cert_args 2>&1" || true)
  if [ "$4" = yes ]; then
    if printf '%s' "$out" | grep -q 'Verification: OK' && ! printf '%s' "$out" | grep -qi 'alert'; then pass "$1: handshake with the placement certificate succeeds"; else fail "$1: handshake with the placement certificate failed"; fi
  else
    if printf '%s' "$out" | grep -qiE 'alert (certificate required|handshake failure|bad certificate)|certificate required'; then pass "$1: a client without a certificate is refused"; else fail "$1: a client without a certificate was NOT refused"; fi
  fi
}
for hop in "dispatcher 172.30.11.4:8082 egress" "media gateway 172.30.11.3:8083 control-plane" "telemetry ingestor 172.30.11.5:4317 ingestor"; do
  name=${hop% * *}; rest=${hop#"$name" }; address=${rest%% *}; servername=${rest##* }
  tls_probe "$name" "$address" "$servername" yes
  tls_probe "$name" "$address" "$servername" no
done

# --- PostgreSQL: TLS, role pinning, least privilege -------------------------------------------------------
# Each connection is made from the network namespace of the service that legitimately owns the role, with the real psql
# client of the PostgreSQL image, because pg_hba.conf pins roles to source addresses (df_control to the control plane's
# data address; df_campaign_runtime and df_egress to the db-gate). The password goes by stdin into a pgpass file on a
# container tmpfs, never an argument.
postgres_image=$(sed -n 's/^POSTGRES_IMAGE=//p' "$root/deploy/images/images.lock")
psql_from() { # psql_from <service whose network to use> <hostaddr> <role> <extra conninfo> <sql>
  local service=$1 addr=$2 role=$3 extra=$4 sql=$5
  printf '*:*:*:%s:%s\n' "$role" "$(cat "$secrets/postgres/db_${role#df_}_password")" |
    docker run --rm -i --network "container:$(cid "$service")" --tmpfs /tmp --volume "$secrets/postgres/db_ca_certificate:/ca.crt:ro" \
      --entrypoint sh "$postgres_image" -c 'umask 077; cat > /tmp/pgpass; PGPASSFILE=/tmp/pgpass exec psql -X -At -v ON_ERROR_STOP=1 "$0" -c "$1"' \
      "host=postgres hostaddr=$addr port=5432 dbname=dungeonflux user=$role $extra" "$sql" 2>&1 || true
}
expect_output() { # expect_output <description> <regex> <output>
  if printf '%s' "$3" | grep -Eq "$2"; then pass "$1"; else fail "$1 (got: $(printf '%s' "$3" | head -c 200))"; fi
}
tls="sslmode=verify-full sslrootcert=/ca.crt"
expect_output "df_control connects over verify-full TLS from the control plane" '^df_control$' \
  "$(psql_from control-plane 172.30.2.2 df_control "$tls" 'select current_user')"
expect_output "df_campaign_runtime connects through the db-gate from a runtime" '^df_campaign_runtime$' \
  "$(psql_from runtime-1 172.30.11.2 df_campaign_runtime "$tls" 'select current_user')"
expect_output "df_egress connects through the db-gate from the dispatcher" '^df_egress$' \
  "$(psql_from egress 172.30.4.2 df_egress "$tls" 'select current_user')"
expect_output "a plaintext connection is refused by pg_hba.conf" 'no encryption|pg_hba.conf rejects' \
  "$(psql_from control-plane 172.30.2.2 df_control 'sslmode=disable' 'select 1')"
expect_output "the administrator cannot log in over the network" 'pg_hba.conf rejects' \
  "$(printf '*:*:*:df_admin:%s\n' "$(cat "$secrets/postgres/db_admin_password")" |
     docker run --rm -i --network "container:$(cid control-plane)" --tmpfs /tmp --volume "$secrets/postgres/db_ca_certificate:/ca.crt:ro" \
       --entrypoint sh "$postgres_image" -c 'umask 077; cat > /tmp/pgpass; PGPASSFILE=/tmp/pgpass exec psql -X -At "host=postgres hostaddr=172.30.2.2 dbname=dungeonflux user=df_admin sslmode=require" -c "select 1"' 2>&1 || true)"
# Role pinning: a role is accepted only from the source address of its own service.
expect_output "df_control is refused when it arrives through the db-gate (a runtime holding its password gains nothing)" 'pg_hba.conf rejects' \
  "$(psql_from runtime-1 172.30.11.2 df_control "$tls" 'select 1')"
expect_output "df_campaign_runtime is refused when it connects directly from the control plane" 'pg_hba.conf rejects' \
  "$(psql_from control-plane 172.30.2.2 df_campaign_runtime "$tls" 'select 1')"
expect_output "df_campaign_runtime cannot create a table" 'permission denied' \
  "$(psql_from runtime-1 172.30.11.2 df_campaign_runtime "$tls" 'create table public.smoke_probe (i int)')"
expect_output "df_campaign_runtime cannot create a schema" 'permission denied' \
  "$(psql_from runtime-1 172.30.11.2 df_campaign_runtime "$tls" 'create schema smoke_probe')"
expect_output "df_egress cannot create a table" 'permission denied' \
  "$(psql_from egress 172.30.4.2 df_egress "$tls" 'create table public.smoke_probe (i int)')"
# The default-ACL check is passed off here ONLY until O1 ships deploy/db/bootstrap-migration.sql as the first migration;
# production verification never passes it (the verifier defaults it ON).
verify_out=$("${compose[@]}" exec -T postgres psql -X -U df_admin -d dungeonflux -v ON_ERROR_STOP=1 -v db=dungeonflux -v check_default_acl=off \
  -f /opt/dungeonflux/db/verify-privileges.sql 2>&1 || true)
expect_output "verify-privileges.sql: no app role is powerful, a member of another role, an owner or able to run DDL" 'privileges ok' "$verify_out"

# --- object store: TLS only ------------------------------------------------------------------------------
bucket_check() { # bucket_check <endpoint>: lists buckets with the staging credentials
  "${compose[@]}" run --rm --no-deps -T --entrypoint /bin/sh bucket-init -c \
    "AWS_ACCESS_KEY_ID=\$(cat /run/secrets/storage_access_key_id) AWS_SECRET_ACCESS_KEY=\$(cat /run/secrets/storage_secret_key) aws --endpoint-url $1 s3api list-buckets --query 'Buckets[].Name' --output text" 2>&1 || true
}
prefix=dfx-stage
buckets=$(bucket_check https://object-store:9000)
while IFS=, read -r suffix _rest; do
  expect_output "bucket $prefix-$suffix exists over TLS" "$prefix-$suffix" "$buckets"
done < <(tail -n +2 "$root/deploy/storage/buckets.csv")
plain=$(bucket_check http://object-store:9000)
if printf '%s' "$plain" | grep -q "$prefix-"; then fail "the object store answered a plaintext request"; else pass "the object store refuses plaintext requests"; fi

# --- edge logging ------------------------------------------------------------------------------------------
if bash "$root/deploy/scripts/check-edge-redaction.sh" --live >/dev/null; then pass "edge logs redact capabilities (live)"; else fail "edge log redaction (run check-edge-redaction.sh --live)"; fi

if [ "$failures" -ne 0 ]; then echo "$failures check(s) failed"; exit 1; fi
echo "all smoke checks passed"
