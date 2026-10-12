#!/bin/sh
# usage: own-postgres.sh <start|stop>
# A PostgreSQL instance of this checkout's browser checks, with its own data directory and port
# (DF_CHECK_PG_PORT, default 55531), so checks never share the launcher's instance on 55517.
# Evaluator tooling, not application code.
set -eu
WT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
R=${DUNGEONFLUX_ARTIFACT_ROOT:-$WT/artifacts}
PG=${DUNGEONFLUX_POSTGRES_BIN:-$R/build/postgres-g05/install/bin}
DIR=$R/tmp/browser-checks
PORT=${DF_CHECK_PG_PORT:-55531}
SOCK=${DF_CHECK_PG_SOCKET_DIR:-$DIR/pgsock}
mkdir -p "$SOCK"
case "$1" in
  start)
    if test ! -d "$DIR/pgdata"; then
      "$PG/initdb" -D "$DIR/pgdata" -U df_gameplay_demo_admin_20261004 --auth-host=trust --auth-local=trust -E UTF8 --locale=C > "$DIR/initdb.log" 2>&1
    fi
    if ! "$PG/pg_ctl" -D "$DIR/pgdata" status > /dev/null 2>&1; then
      "$PG/pg_ctl" -D "$DIR/pgdata" -l "$DIR/postgres.log" -o "-p $PORT -h 127.0.0.1 -k $SOCK -c max_connections=60" -w start
    fi
    ;;
  stop)
    "$PG/pg_ctl" -D "$DIR/pgdata" -m fast -w stop || true
    ;;
esac
