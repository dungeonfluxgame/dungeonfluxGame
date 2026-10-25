#!/usr/bin/env bash
# Print the SQL that creates (or re-passwords) the application database roles and applies grants.sql. It
# CONNECTS TO NOTHING: it reads operator-supplied password FILES and writes a psql script on stdout, in which
# every password is a SCRAM-SHA-256 verifier computed on this machine (deploy/db/scram-verifier.sh). The
# plaintext password is never in the SQL, so it cannot reach pg_stat_activity, pg_stat_statements or a server
# log (critic round 1c, item 6). STATUS: the generator and the verifier derivation were run locally; the SQL
# was never run against PostgreSQL.
#
# TODO(owner, coordinator ruling g): the backup role df_backup (manifest entry db_backup_password) is NOT created here yet. Whether it
# needs BYPASSRLS on managed PostgreSQL is pending the owner's check; when decided, this file, grants.sql and verify-privileges.sql
# change together.
#
# usage: create-roles.sh --password-dir DIR [--database dungeonflux] > roles.sql
#   DIR holds db_<role>_password files: db_migrator_password, db_control_password, db_runner_password,
#   db_campaign_runtime_password, db_egress_password, db_accounts_password, db_campaigns_password,
#   db_finance_password, db_support_password
#   (made with deploy/scripts/gen-secret.sh). Application passwords never pass through Terraform.
#
# HOW AN OPERATOR RUNS IT AGAINST MANAGED POSTGRESQL (critic round 1c, item 7). The database firewall trusts
# only the application and worker Droplets, and those hosts have no psql and no SSH forwarding. So the SQL is
# run by a pinned one-shot psql container on the app host, started by the root-owned deploy tool, with the
# administrator password as the first line of stdin:
#     deploy/db/run-admin-sql.sh --ssh-host dfdeploy@APP_HOST --admin-password-file FILE \
#         --host CLUSTER_PRIVATE_HOST --port 25060 --dbname dungeonflux --user doadmin  roles.sql
# (see run-admin-sql.sh and infra/cloud-init/df-deploy.sh). The staging mirror pipes the same output into the
# local psql from deploy/compose/postgres-init/10-roles.sh.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
dir=""
database=dungeonflux
while [ $# -gt 0 ]; do
  case "$1" in
    --password-dir) dir=$2; shift 2 ;;
    --database) database=$2; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[ -d "$dir" ] || { echo "--password-dir DIR required" >&2; exit 2; }
command -v openssl >/dev/null 2>&1 || { echo "missing tool: openssl" >&2; exit 1; }
case "$database" in *[!A-Za-z0-9_]*|"") echo "bad database name: $database" >&2; exit 2 ;; esac
# shellcheck disable=SC1091
. "$here/scram-verifier.sh"

roles="migrator control runner campaign_runtime egress accounts campaigns finance support"
for r in $roles; do
  file="$dir/db_${r}_password"
  [ -s "$file" ] || { echo "missing or empty password file: db_${r}_password" >&2; exit 1; }
  # A password below 24 characters is refused: these are machine-generated.
  [ "$(wc -c < "$file")" -ge 24 ] || { echo "password file db_${r}_password is shorter than 24 characters" >&2; exit 1; }
done

echo '\set ON_ERROR_STOP on'
echo '\set QUIET on'
for r in $roles; do
  verifier=$(scram_verifier "$dir/db_${r}_password" 4096)
  printf "SELECT 'CREATE ROLE df_%s LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS NOREPLICATION INHERIT' WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'df_%s') \\\\gexec\n" "$r" "$r"
  printf "ALTER ROLE df_%s PASSWORD '%s';\n" "$r" "$verifier"
done
printf "\\\\set db '%s'\n" "$database"
cat "$here/grants.sql"
