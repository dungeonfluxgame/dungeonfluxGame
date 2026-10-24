#!/bin/sh
# Pure configuration resolution. Do not read credentials or touch the filesystem.
resolve_database_port() {
  for PORT_NAME in DF_GAMEPLAY_DATABASE_PORT DF_GAMEPLAY_DEMO_DATABASE_PORT; do
    if test "$PORT_NAME" = DF_GAMEPLAY_DATABASE_PORT; then
      PORT_VALUE=${DF_GAMEPLAY_DATABASE_PORT-55517}
    else
      PORT_VALUE=${DF_GAMEPLAY_DEMO_DATABASE_PORT-55517}
    fi
    case "$PORT_VALUE" in
      ''|*[!0-9]*) echo "$PORT_NAME invalid: expected port 1024-65535" >&2; exit 2 ;;
    esac
    if ! test "$PORT_VALUE" -ge 1024 2>/dev/null || ! test "$PORT_VALUE" -le 65535 2>/dev/null; then
      echo "$PORT_NAME invalid: expected port 1024-65535" >&2; exit 2
    fi
  done
  if test "${DF_GAMEPLAY_DATABASE_PORT+x}" = x && test "${DF_GAMEPLAY_DEMO_DATABASE_PORT+x}" = x; then
    if test "$DF_GAMEPLAY_DATABASE_PORT" -ne "$DF_GAMEPLAY_DEMO_DATABASE_PORT"; then
      echo 'database port settings conflict' >&2; exit 2
    fi
  fi
  DATABASE_PORT=${DF_GAMEPLAY_DATABASE_PORT-${DF_GAMEPLAY_DEMO_DATABASE_PORT-55517}}
  export DF_GAMEPLAY_DATABASE_PORT=$DATABASE_PORT
}
