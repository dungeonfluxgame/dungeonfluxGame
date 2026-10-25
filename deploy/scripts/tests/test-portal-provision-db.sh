#!/usr/bin/env bash
set -Eeuo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)
provision_script=${PORTAL_PROVISION_SCRIPT:-"$root/deploy/demo/provision-portal-db.sh"}
[[ -f $provision_script ]] || { printf 'missing portal provisioning script: %s\n' "$provision_script" >&2; exit 1; }

temporary_dir=$(mktemp -d)
trap 'rm -rf "$temporary_dir"' EXIT
export PORTAL_ENV_FILE="$temporary_dir/portal.env"
export PORTAL_PROVISION_SOURCE_ONLY=1
# shellcheck disable=SC1090
source "$provision_script"

assert_contains() {
  grep -Fq -- "$1" <<< "$2" || { printf 'missing provisioning contract: %s\n' "$1" >&2; exit 1; }
}

sql=$(provision_sql migratorpw runtimepw)
assert_contains 'CREATE ROLE df_portal_owner NOLOGIN' "$sql"
assert_contains 'CREATE ROLE df_portal_migrator LOGIN' "$sql"
assert_contains 'CREATE ROLE df_portal_runtime LOGIN' "$sql"
assert_contains 'GRANT df_portal_owner TO df_portal_migrator' "$sql"
assert_contains 'CREATE DATABASE df_portal_demo OWNER df_portal_owner' "$sql"
assert_contains 'REVOKE ALL ON DATABASE df_portal_demo FROM PUBLIC' "$sql"
assert_contains 'CONNECTION LIMIT 16' "$sql"
assert_contains 'CONNECTION LIMIT 4' "$sql"
for role in df_portal_migrator df_portal_runtime; do
  assert_contains "ALTER ROLE $role SET transaction_timeout = '10s'" "$sql"
done

db_sql=$(database_sql)
assert_contains 'CREATE EXTENSION IF NOT EXISTS vector' "$db_sql"
assert_contains 'ALTER DEFAULT PRIVILEGES FOR ROLE df_portal_migrator IN SCHEMA %I GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO df_portal_runtime' "$db_sql"
assert_contains 'GRANT USAGE ON SCHEMA %I TO df_portal_runtime' "$db_sql"
assert_contains 'REVOKE CREATE ON SCHEMA public FROM PUBLIC' "$db_sql"
assert_contains "ALTER ROLE df_portal_runtime SET work_mem = '512kB'" "$sql"
assert_contains 'ALTER ROLE df_portal_runtime SET max_parallel_workers_per_gather = 0' "$sql"

# Env writing: provider lines survive, URLs are written, accounts stay closed.
printf '%s\n' 'DF_WORKOS_API_KEY=fixture-only' 'DF_PROFILE=demo' > "$PORTAL_ENV_FILE"
write_portal_env runtimepw migratorpw
grep -Fxq 'DF_WORKOS_API_KEY=fixture-only' "$PORTAL_ENV_FILE"
grep -Fxq 'DF_PORTAL_DATABASE_URL=postgres://df_portal_runtime:runtimepw@127.0.0.1:5432/df_portal_demo' "$PORTAL_ENV_FILE"
grep -Fxq 'DF_PORTAL_MIGRATOR_DATABASE_URL=postgres://df_portal_migrator:migratorpw@127.0.0.1:5432/df_portal_demo' "$PORTAL_ENV_FILE"
grep -Fxq 'DF_ACCOUNTS_OPEN=false' "$PORTAL_ENV_FILE"
grep -Fq 'ADMINFLOW_HOOK' "$PORTAL_ENV_FILE"
grep -Fxq 'DF_OPERATOR_TUNNEL=false' "$PORTAL_ENV_FILE"
grep -Fxq 'DF_TELEMETRY_DIR=/var/lib/dungeonflux-portal/telemetry' "$PORTAL_ENV_FILE"

# Re-running keeps the deployed passwords instead of rotating them.
[[ $(portal_password DF_PORTAL_DATABASE_URL) == runtimepw ]]
[[ $(portal_password DF_PORTAL_MIGRATOR_DATABASE_URL) == migratorpw ]]
write_portal_env "$(portal_password DF_PORTAL_DATABASE_URL)" "$(portal_password DF_PORTAL_MIGRATOR_DATABASE_URL)"
grep -Fxq 'DF_PORTAL_DATABASE_URL=postgres://df_portal_runtime:runtimepw@127.0.0.1:5432/df_portal_demo' "$PORTAL_ENV_FILE"
[[ $(grep -c '^DF_PORTAL_DATABASE_URL=' "$PORTAL_ENV_FILE") -eq 1 ]]
[[ $(grep -c '^# ADMINFLOW_HOOK:' "$PORTAL_ENV_FILE") -eq 1 ]]

# A fresh password is 64 lowercase hex characters and differs per role.
fresh=$(portal_password DF_PORTAL_DATABASE_CA_FILE)
[[ $fresh =~ ^[a-f0-9]{64}$ ]]

# Open accounts are refused.
printf '%s\n' 'DF_ACCOUNTS_OPEN=true' > "$PORTAL_ENV_FILE"
if (assert_accounts_closed) >/dev/null 2>&1; then
  printf '%s\n' 'provisioning accepted DF_ACCOUNTS_OPEN=true' >&2
  exit 1
fi

# The script never carries a literal generated password or provider secret.
! grep -Eq 'postgres://[^ ]+:[a-f0-9]{64}@' "$provision_script" || {
  printf '%s\n' 'provisioning script embeds a database password' >&2
  exit 1
}

printf '%s\n' 'portal provisioning checks passed'
