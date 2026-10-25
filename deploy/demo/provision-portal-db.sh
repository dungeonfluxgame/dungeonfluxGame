#!/usr/bin/env bash
# Provision the hosted portal database on the demo droplet (files/host only;
# the coordinator runs this, never from CI). Idempotent: re-running keeps the
# passwords already in the env file, re-applies role limits and grants, and
# grants the runtime role on objects migrations created since the last run.
#
# Roles (crates/df-hosted/src/config.rs takes exactly two database URLs that
# must name the same host, port and database):
#   df_portal_owner    NOLOGIN, owns the database; never connects.
#   df_portal_migrator LOGIN, member of the owner, used only through
#                      DF_PORTAL_MIGRATOR_DATABASE_URL. df-portal-hosted opens
#                      the account store and database::migrate with it at
#                      startup (runtime.rs open_stores), so a service restart
#                      is the migration step. Owns migrated objects.
#   df_portal_runtime  LOGIN, used through DF_PORTAL_DATABASE_URL for every
#                      pooled/single runtime connection. No DDL: the account
#                      store skips its DDL once its ledger rows exist
#                      (account_store/pool.rs is_migrated), and default
#                      privileges below give it DML on migrator objects.
# Role defaults carry transaction_timeout=10s because non-development
# startup refuses an unset or longer role default (account_closure
# TimeoutPolicy::AtMostMs(10_000); campaign_store checks the same setting).
set -Eeuo pipefail

portal_db_name=${PORTAL_DB_NAME:-df_portal_demo}
portal_db_host=${PORTAL_DB_HOST:-127.0.0.1}
portal_db_port=${PORTAL_DB_PORT:-5432}
portal_env_file=${PORTAL_ENV_FILE:-/etc/dungeonflux/portal.env}
portal_owner_role=df_portal_owner
portal_migrator_role=df_portal_migrator
portal_runtime_role=df_portal_runtime
# Runtime connections today: account pool 4, campaign pool 2 + read pool 2,
# and one Connection each for finance, checkout, campaign accounts, sessions
# and funding, plus optional support (runtime.rs). 16 leaves headroom without
# exhausting the droplet cluster (max_connections=40, provision-demo.sh).
portal_runtime_connection_limit=16
portal_migrator_connection_limit=4

fail() { printf 'provision-portal-db: %s\n' "$1" >&2; exit 1; }

[[ $portal_db_name =~ ^[a-z_][a-z0-9_]*$ ]] || fail "invalid database name: $portal_db_name"
[[ $portal_db_port =~ ^[0-9]+$ ]] || fail "invalid database port: $portal_db_port"

admin_psql() {
  PGPASSFILE=${PGPASSFILE:-/root/.pgpass} psql -h "$portal_db_host" -p "$portal_db_port" \
    -U postgres -X -v ON_ERROR_STOP=1 "$@"
}

db_psql() {
  PGPASSFILE=${PGPASSFILE:-/root/.pgpass} psql -h "$portal_db_host" -p "$portal_db_port" \
    -U postgres -X -v ON_ERROR_STOP=1 -d "$portal_db_name" "$@"
}

env_value() {
  local key=$1 line
  [[ -f $portal_env_file ]] || return 0
  line=$(grep -E "^${key}=" "$portal_env_file" | tail -n 1 || true)
  [[ -n $line ]] && printf '%s' "${line#*=}"
  return 0
}

# Passwords already deployed keep working across a re-run; only a missing
# password is generated, on the host, and it is written only to the env file.
existing_password() {
  local url=$1 rest
  [[ $url == postgres://*:*@* || $url == postgresql://*:*@* ]] || return 0
  rest=${url#*://}
  rest=${rest%%@*}
  [[ $rest == *:* ]] && printf '%s' "${rest#*:}"
  return 0
}

portal_password() {
  local key=$1 existing generated
  existing=$(existing_password "$(env_value "$key")")
  if [[ -n $existing ]]; then
    printf '%s' "$existing"
    return 0
  fi
  generated=$(openssl rand -hex 32)
  [[ $generated =~ ^[a-f0-9]{64}$ ]] || fail 'openssl did not return a 64-hex password'
  printf '%s' "$generated"
}

sql_password() { printf '%s' "${1//\'/\'\'}"; }

provision_sql() {
  local migrator_password_sql=$1 runtime_password_sql=$2
  cat <<SQL
DO \$\$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '$portal_owner_role') THEN
    CREATE ROLE $portal_owner_role NOLOGIN;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '$portal_migrator_role') THEN
    CREATE ROLE $portal_migrator_role LOGIN;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '$portal_runtime_role') THEN
    CREATE ROLE $portal_runtime_role LOGIN;
  END IF;
END
\$\$;
ALTER ROLE $portal_owner_role NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
ALTER ROLE $portal_migrator_role LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS
  CONNECTION LIMIT $portal_migrator_connection_limit PASSWORD '$migrator_password_sql';
ALTER ROLE $portal_runtime_role LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS
  CONNECTION LIMIT $portal_runtime_connection_limit PASSWORD '$runtime_password_sql';
GRANT $portal_owner_role TO $portal_migrator_role;
ALTER ROLE $portal_migrator_role SET transaction_timeout = '10s';
ALTER ROLE $portal_migrator_role SET statement_timeout = '30s';
ALTER ROLE $portal_migrator_role SET idle_in_transaction_session_timeout = '10s';
ALTER ROLE $portal_runtime_role SET transaction_timeout = '10s';
ALTER ROLE $portal_runtime_role SET statement_timeout = '30s';
ALTER ROLE $portal_runtime_role SET idle_in_transaction_session_timeout = '10s';
ALTER ROLE $portal_runtime_role SET work_mem = '512kB';
ALTER ROLE $portal_runtime_role SET max_parallel_workers_per_gather = 0;
SELECT 'CREATE DATABASE $portal_db_name OWNER $portal_owner_role'
  WHERE NOT EXISTS (SELECT 1 FROM pg_database WHERE datname = '$portal_db_name')\\gexec
ALTER DATABASE $portal_db_name OWNER TO $portal_owner_role;
REVOKE ALL ON DATABASE $portal_db_name FROM PUBLIC;
GRANT CONNECT ON DATABASE $portal_db_name TO $portal_migrator_role, $portal_runtime_role;
GRANT CREATE ON DATABASE $portal_db_name TO $portal_migrator_role;
SQL
}

database_sql() {
  cat <<SQL
CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS btree_gist;
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
DO \$\$
DECLARE
  v_schema text;
BEGIN
  FOREACH v_schema IN ARRAY ARRAY['accounts', 'campaigns', 'finance', 'hosted']
  LOOP
    EXECUTE format('CREATE SCHEMA IF NOT EXISTS %I AUTHORIZATION $portal_migrator_role', v_schema);
    EXECUTE format('GRANT USAGE ON SCHEMA %I TO $portal_runtime_role', v_schema);
    EXECUTE format('GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA %I TO $portal_runtime_role', v_schema);
    EXECUTE format('GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA %I TO $portal_runtime_role', v_schema);
    EXECUTE format('GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA %I TO $portal_runtime_role', v_schema);
    EXECUTE format('ALTER DEFAULT PRIVILEGES FOR ROLE $portal_migrator_role IN SCHEMA %I GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO $portal_runtime_role', v_schema);
    EXECUTE format('ALTER DEFAULT PRIVILEGES FOR ROLE $portal_migrator_role IN SCHEMA %I GRANT USAGE, SELECT, UPDATE ON SEQUENCES TO $portal_runtime_role', v_schema);
    EXECUTE format('ALTER DEFAULT PRIVILEGES FOR ROLE $portal_migrator_role IN SCHEMA %I GRANT EXECUTE ON FUNCTIONS TO $portal_runtime_role', v_schema);
  END LOOP;
END
\$\$;
SQL
}

portal_database_url() {
  local role=$1 password=$2
  printf 'postgres://%s:%s@%s:%s/%s' "$role" "$password" "$portal_db_host" "$portal_db_port" "$portal_db_name"
}

# Rewrites the two database URLs, adds missing non-secret demo defaults, and
# refuses to open accounts. Other lines (provider keys) pass through untouched.
write_portal_env() {
  local runtime_password=$1 migrator_password=$2 temporary key value
  temporary=$(mktemp "$(dirname -- "$portal_env_file")/.portal.env.XXXXXX")
  if [[ -f $portal_env_file ]]; then
    grep -vE '^(DF_PORTAL_DATABASE_URL|DF_PORTAL_MIGRATOR_DATABASE_URL)=' "$portal_env_file" > "$temporary" || true
  fi
  printf 'DF_PORTAL_DATABASE_URL=%s\n' "$(portal_database_url "$portal_runtime_role" "$runtime_password")" >> "$temporary"
  printf 'DF_PORTAL_MIGRATOR_DATABASE_URL=%s\n' "$(portal_database_url "$portal_migrator_role" "$migrator_password")" >> "$temporary"
  while IFS='=' read -r key value; do
    if [[ -z $(env_value_in "$temporary" "$key") ]]; then
      printf '%s=%s\n' "$key" "$value" >> "$temporary"
    fi
  done <<'DEFAULTS'
DF_ACCOUNTS_OPEN=false
DF_PROFILE=demo
DF_PORTAL_LISTEN=127.0.0.1:8099
DF_PORTAL_BUNDLE_DIR=/opt/dungeonflux/app
DF_PORTAL_TRUSTED_PROXIES=127.0.0.1/32,::1/128
DF_GAMEPLAY_DATABASE_TLS=loopback-plaintext
DF_TELEMETRY_DIR=/var/lib/dungeonflux-portal/telemetry
DF_OPERATOR_TUNNEL=false
DEFAULTS
  if ! grep -q '^# ADMINFLOW_HOOK:' "$temporary"; then
    printf '%s\n' '# ADMINFLOW_HOOK: DF_OPERATOR_TUNNEL=false until the dedicated listener and TLS runbook are accepted.' >> "$temporary"
  fi
  chmod 0640 "$temporary"
  if getent group dungeonflux >/dev/null 2>&1; then
    chown root:dungeonflux "$temporary"
  fi
  mv "$temporary" "$portal_env_file"
}

env_value_in() {
  local file=$1 key=$2 line
  line=$(grep -E "^${key}=" "$file" | tail -n 1 || true)
  [[ -n $line ]] && printf '%s' "${line#*=}"
  return 0
}

assert_accounts_closed() {
  local value
  value=$(env_value DF_ACCOUNTS_OPEN)
  [[ -z $value || $value == false ]] || fail "DF_ACCOUNTS_OPEN must stay false on the demo droplet (found: $value)"
}

main() {
  [[ ${EUID} -eq 0 ]] || fail 'run this script as root on the droplet'
  command -v psql >/dev/null 2>&1 || fail 'psql is missing'
  command -v openssl >/dev/null 2>&1 || fail 'openssl is missing'
  getent group dungeonflux >/dev/null 2>&1 || fail 'dungeonflux group is missing; provision the host first'
  assert_accounts_closed
  local runtime_password migrator_password
  runtime_password=$(portal_password DF_PORTAL_DATABASE_URL)
  migrator_password=$(portal_password DF_PORTAL_MIGRATOR_DATABASE_URL)
  [[ $runtime_password != "$migrator_password" ]] || fail 'runtime and migrator passwords must differ'
  admin_psql -q <<< "$(provision_sql "$(sql_password "$migrator_password")" "$(sql_password "$runtime_password")")"
  db_psql -q <<< "$(database_sql)"
  install -d -o root -g dungeonflux -m 0750 "$(dirname -- "$portal_env_file")" 2>/dev/null \
    || install -d -m 0750 "$(dirname -- "$portal_env_file")"
  write_portal_env "$runtime_password" "$migrator_password"
  chmod 0640 "$portal_env_file"
  if getent group dungeonflux >/dev/null 2>&1; then
    chown root:dungeonflux "$portal_env_file"
  fi
  printf 'provision-portal-db: database %s ready; runtime limit %s, migrator limit %s; env %s\n' \
    "$portal_db_name" "$portal_runtime_connection_limit" "$portal_migrator_connection_limit" "$portal_env_file"
}

if [[ ${PORTAL_PROVISION_SOURCE_ONLY:-} != 1 ]]; then
  main "$@"
fi
