#!/bin/bash
# df-host-metrics: writes the host facts Prometheus cannot see from inside the stack, as a Prometheus text file that the host-textfile
# exporter serves (round 3). Installed root-owned (0755) at /usr/local/sbin/df-host-metrics; run every minute by df-host-metrics.timer.
# The backup job writes its own file (df_backup.prom) and the restore-drill recorder another (df_restore_drill.prom) in the same directory.
#   df_host_disk_used_ratio{mount}        use of /var/lib/dungeonflux (the state disk) and /run (tmpfs: secrets and per-slot files)
#   df_host_secrets_present{consumer}     1 when the consumer's secrets directory exists and holds a file, else 0 (a service cannot start)
#   df_host_alert_receiver_present        1 when the alert receiver URL has been pushed (otherwise alerts go nowhere)
#   df_host_release_current               1 when a release is active
# Test mode (non-root only): DF_HOST_METRICS_PREFIX=<dir> moves every path under it.
set -euo pipefail

if [ "$(id -u)" -eq 0 ]; then
  [ -z "${DF_HOST_METRICS_PREFIX:-}" ] || { echo "the test option is refused as root" >&2; exit 2; }
  P=""
else
  [ -n "${DF_HOST_METRICS_PREFIX:-}" ] || { echo "df-host-metrics must run as root" >&2; exit 2; }
  P=$DF_HOST_METRICS_PREFIX
fi
textfile=$P/var/lib/dungeonflux/textfile
secrets=$P/run/dungeonflux/secrets
mkdir -p "$textfile"

ratio() { # ratio PATH -> used fraction of the filesystem holding PATH, or empty
  df -P "$1" 2>/dev/null | awk 'NR == 2 { gsub("%", "", $5); printf "%.4f", $5 / 100 }'
}

out=$textfile/.df_host.tmp
{
  echo '# TYPE df_host_disk_used_ratio gauge'
  for mount in /var/lib/dungeonflux /run; do
    value=$(ratio "$P$mount" || true)
    [ -z "$value" ] || echo "df_host_disk_used_ratio{mount=\"$mount\"} $value"
  done
  echo '# TYPE df_host_secrets_present gauge'
  # The consumers whose absence stops a service that runs on THIS kind of host (/etc/dungeonflux/host-role: app or worker).
  role=$(head -n 1 "$P/etc/dungeonflux/host-role" 2>/dev/null | tr -d "[:space:]" || true)
  if [ "$role" = worker ]; then consumers="dispatcher runner ingestor deploy"; else consumers="control-plane dispatcher runner ingestor edge monitoring backup deploy"; fi
  for consumer in $consumers; do
    if [ -d "$secrets/$consumer" ] && [ -n "$(find "$secrets/$consumer" -maxdepth 1 -type f -print -quit 2>/dev/null)" ]; then v=1; else v=0; fi
    echo "df_host_secrets_present{consumer=\"$consumer\"} $v"
  done
  echo '# TYPE df_host_alert_receiver_present gauge'
  if [ -s "$secrets/monitoring/alert_receiver_url" ]; then echo 'df_host_alert_receiver_present 1'; else echo 'df_host_alert_receiver_present 0'; fi
  echo '# TYPE df_host_release_current gauge'
  if [ -e "$P/opt/dungeonflux/current" ]; then echo 'df_host_release_current 1'; else echo 'df_host_release_current 0'; fi
} > "$out"
chmod 0644 "$out"
mv -f "$out" "$textfile/df_host.prom"
