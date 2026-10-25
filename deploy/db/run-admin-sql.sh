#!/usr/bin/env bash
# Run SQL as the cluster administrator against managed PostgreSQL (critic round 1c, item 7). STATUS: UNVERIFIED.
#
# THE PATH. The managed database trusts only the application and worker Droplets (modules/database), those hosts
# have no psql and no SSH forwarding, and the bastion is not trusted by the database. So the SQL is executed by a
# PINNED ONE-SHOT psql CONTAINER on the app host, started by the root-owned deploy tool:
#     operator workstation --ssh (via the bastion)--> dfdeploy@APP_HOST --sudo--> df-deploy psql --> docker run <pinned psql image>
# stdin carries the administrator password as the FIRST LINE, then the SQL. ssh runs with -T and RequestTTY=no
# (a configured PTY would echo the password line to the terminal and its scrollback) and with every forwarding cleared. The password never appears on a
# command line, in an environment variable, in `docker inspect`, or on the host's disk: df-deploy hands it to the
# container, which writes it to a pgpass file on a container tmpfs. Whoever may run this must be an individual
# operator with their own bastion account and a hardware key (the CI deploy key cannot: its forced command only
# runs `deploy`; coordinator ruling b).
#
# usage: run-admin-sql.sh --ssh-host user@APP_HOST --admin-password-file FILE --host DB_PRIVATE_HOST --port 25060 \
#            --dbname dungeonflux --user doadmin  FILE.sql [FILE.sql ...]
#   SSH_OPTS="-J you@bastion" selects the bastion. FILE.sql is typically the output of create-roles.sh, or
#   verify-privileges.sql (preceded by `\set db dungeonflux`, which this script adds when --set-db is given).
#   Production verification passes no check_default_acl override: verify-privileges.sql defaults it to ON, so a database
#   without the bootstrap migration FAILS the check.
#   --set-db NAME  prepend `\set db 'NAME'` (needed by verify-privileges.sql and grants.sql).
set -euo pipefail

ssh_host=""; admin_file=""; db_host=""; db_port=""; dbname=""; db_user=""; set_db=""
files=()
while [ $# -gt 0 ]; do
  case "$1" in
    --ssh-host) ssh_host=$2; shift 2 ;;
    --admin-password-file) admin_file=$2; shift 2 ;;
    --host) db_host=$2; shift 2 ;;
    --port) db_port=$2; shift 2 ;;
    --dbname) dbname=$2; shift 2 ;;
    --user) db_user=$2; shift 2 ;;
    --set-db) set_db=$2; shift 2 ;;
    -*) echo "unknown argument: $1" >&2; exit 2 ;;
    *) files+=("$1"); shift ;;
  esac
done
for v in ssh_host admin_file db_host db_port dbname db_user; do
  [ -n "${!v}" ] || { echo "missing --${v//_/-}" >&2; exit 2; }
done
[ "${#files[@]}" -gt 0 ] || { echo "give at least one SQL file" >&2; exit 2; }
[ -s "$admin_file" ] || { echo "empty or missing --admin-password-file" >&2; exit 1; }
for f in "${files[@]}"; do [ -s "$f" ] || { echo "empty or missing SQL file: $f" >&2; exit 1; }; done
case "$set_db" in *[!A-Za-z0-9_]*) echo "bad --set-db name" >&2; exit 2 ;; esac

# stdin = password line, then the SQL.
{
  tr -d '\r\n' < "$admin_file"; printf '\n'
  if [ -n "$set_db" ]; then printf "\\\\set db '%s'\n" "$set_db"; fi
  cat "${files[@]}"
} | ssh -T -o RequestTTY=no -o ClearAllForwardings=yes ${SSH_OPTS:-} "$ssh_host" sudo /usr/local/sbin/df-deploy psql \
      --host "$db_host" --port "$db_port" --dbname "$dbname" --user "$db_user"
