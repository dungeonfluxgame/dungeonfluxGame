#!/bin/bash
# df-deploy: the ONLY way a person or CI starts containers or runs database administration from an application
# or worker host (critic rounds 1c, 1d and 1e). Installed root-owned (0755) at /usr/local/sbin/df-deploy by cloud-init.
# Nobody who logs in is in the docker group (docker socket access is host root: it can bind-mount /run/dungeonflux
# and read every consumer's secrets); the accounts may only run this script and df-secrets-install through sudo,
# with argument patterns fixed in /etc/sudoers.d/dungeonflux-deploy, and this script validates every argument again.
#
#   sudo df-deploy <sha256:digest>        a release (CI only, through the forced command df-deploy-ssh). NOT IMPLEMENTED
#   sudo df-deploy release <sha256:...>   YET: round 2 adds `cosign verify` with a pinned --certificate-identity and
#                                         --certificate-oidc-issuer, writes the verified digest into
#                                         /etc/dungeonflux/allowed-images, and starts only allow-listed digests with the
#                                         hardening in the run definition. Until then it REFUSES.
#   sudo df-deploy psql --host H --port P --dbname D --user U
#                                         stdin: line 1 = the database administrator password, the rest = SQL.
#                                         Runs the pinned one-shot psql image (allowed-images, key "psql") on a read-only,
#                                         capability-free container whose ONLY way out is a host-side forwarder to the
#                                         database (see below); the password goes to a pgpass file on the container's tmpfs
#                                         and never touches a command line, an environment variable, `docker inspect` or
#                                         the host disk.
#
# THE psql CONTAINER'S NETWORK AND THE FORWARDER. The container runs on a dedicated INTERNAL docker network (df-psql: no
# default route, no internet, no metadata endpoint, no other VPC host). The only thing it can reach is a socat forwarder
# this script starts on the network's gateway address, relaying to the database. So the forwarder is the whole attack
# surface, and it is locked down (critic round 1e):
#   * the target host must END WITH the root-owned suffix in /etc/dungeonflux/db-host-suffix (cloud-init writes
#     .db.ondigitalocean.com), must be a NAME (IP literals are refused), and the port must be 25060 or 25061; the name is
#     resolved ONCE and the forwarder relays to that address (a link-local, loopback or multicast answer is refused), so
#     `--host 169.254.169.254` or any internet host is impossible;
#   * the whole function holds flock /run/dungeonflux/psql.lock (a second run is refused, never silently sharing the first
#     run's forwarder), the forwarder must be alive and LISTENING before the container starts, and the run fails if it dies;
#   * an INPUT chain drop for the df-psql bridge on everything except tcp/5432 to the gateway address, so the container
#     cannot reach sshd or any other host service listening on all addresses.
# THE CA FILE. The database CA is public data but lives in a root:dfs-control-plane 0440 file; this script copies it to a
# root-created 0444 temporary file on tmpfs and mounts only that, so the container needs no group and cannot read any secret.
#
# Until release verification exists, the deploy SSH key is ROOT-EQUIVALENT in effect (it can drive this tool and the
# secrets installer) and must be handled as such; see the manifest entry ssh_deploy_key.
# STATUS: UNVERIFIED (never run as root; bash -n only). Unverified: that a host process can bind an internal bridge's
# gateway address on the pinned Docker version, the bridge name br-<12 id characters>, and libpq hostaddr with host.
set -euo pipefail

allowed=/etc/dungeonflux/allowed-images
suffix_file=/etc/dungeonflux/db-host-suffix
lock_file=/run/dungeonflux/psql.lock
export DOCKER_CONFIG=/run/dungeonflux/docker

[ "$(id -u)" -eq 0 ] || { echo "df-deploy must run through sudo" >&2; exit 2; }

digest_re='^sha256:[0-9a-f]{64}$'
name_re='^[A-Za-z0-9_.-]{1,64}$'
host_re='^[A-Za-z0-9.-]{1,253}$'
port_re='^[0-9]{1,5}$'
psql_net=df-psql
psql_subnet=172.29.250.0/29
psql_gateway=172.29.250.1

release() {
  [ "$#" -eq 1 ] || { echo "release takes exactly one digest" >&2; exit 2; }
  local digest=$1
  [[ "$digest" =~ $digest_re ]] || { echo "not a sha256 digest: $digest" >&2; exit 2; }
  echo "df-deploy: release verification (cosign verify, identity and issuer pinned, then start of allow-listed digests) is not implemented yet; refusing" >&2
  exit 3
}

allowed_image() { # allowed_image KEY -> image reference with a digest from the root-owned allow-list
  local key=$1 ref
  [ -f "$allowed" ] || { echo "no $allowed" >&2; exit 4; }
  ref=$(sed -n "s/^${key}=//p" "$allowed" | head -n 1)
  [[ "$ref" =~ @sha256:[0-9a-f]{64}$ ]] || { echo "no digest-pinned '$key' image in $allowed" >&2; exit 4; }
  printf '%s' "$ref"
}

# Resolve the (already allow-listed) database name ONCE; refuse anything that is not an ordinary unicast address.
resolve_once() {
  local host=$1 ip
  ip=$(getent ahostsv4 "$host" | awk 'NR == 1 { print $1 }')
  [[ "$ip" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "cannot resolve $host to an IPv4 address" >&2; exit 4; }
  case "$ip" in
    0.*|127.*|169.254.*|22[4-9].*|23[0-9].*|24[0-9].*|25[0-5].*) echo "refusing to relay to $ip" >&2; exit 4 ;;
  esac
  printf '%s' "$ip"
}

run_psql() {
  local host="" port="" dbname="" user=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --host) host=$2; shift 2 ;;
      --port) port=$2; shift 2 ;;
      --dbname) dbname=$2; shift 2 ;;
      --user) user=$2; shift 2 ;;
      *) echo "unknown psql argument: $1" >&2; exit 2 ;;
    esac
  done
  [[ "$host" =~ $host_re ]] && [[ "$port" =~ $port_re ]] && [[ "$dbname" =~ $name_re ]] && [[ "$user" =~ $name_re ]] \
    || { echo "bad --host/--port/--dbname/--user value" >&2; exit 2; }

  # The relay target: a database NAME under the root-owned suffix, on a managed-database port.
  [[ "$host" =~ ^[0-9.]+$ ]] && { echo "an IP literal is not allowed as --host" >&2; exit 2; }
  [ -r "$suffix_file" ] || { echo "no $suffix_file" >&2; exit 4; }
  local suffix
  suffix=$(head -n 1 "$suffix_file" | tr -d '[:space:]')
  [[ "$suffix" =~ ^\.[a-z0-9.-]+\.[a-z]+$ ]] || { echo "bad suffix in $suffix_file" >&2; exit 4; }
  [[ "$host" == *"$suffix" && "$host" != "$suffix" ]] || { echo "--host must end with $suffix" >&2; exit 2; }
  { [ "$port" = 25060 ] || [ "$port" = 25061 ]; } || { echo "--port must be 25060 or 25061" >&2; exit 2; }

  local ca=/run/dungeonflux/secrets/control-plane/db_ca_certificate image tmp forwarder="" target bridge net_id
  [ -r "$ca" ] || { echo "database CA certificate not installed ($ca)" >&2; exit 4; }
  image=$(allowed_image psql)
  command -v socat >/dev/null 2>&1 || { echo "socat is not installed" >&2; exit 4; }
  command -v iptables >/dev/null 2>&1 || { echo "iptables is not installed" >&2; exit 4; }
  target=$(resolve_once "$host")

  # One run at a time: two runs would race for the gateway address and the second psql would land on the first run's
  # forwarder, which may point at another host.
  exec 9> "$lock_file"
  flock -n 9 || { echo "another df-deploy psql is running; refusing" >&2; exit 5; }

  tmp=$(mktemp -d /run/dungeonflux/docker/psql.XXXXXX)
  chmod 0755 "$tmp"
  install -m 0444 "$ca" "$tmp/ca.crt"
  docker network inspect "$psql_net" >/dev/null 2>&1 \
    || docker network create --internal --driver bridge --subnet "$psql_subnet" --gateway "$psql_gateway" "$psql_net" >/dev/null
  trap 'kill "$forwarder" 2>/dev/null || true; rm -rf "$tmp"' EXIT

  # The container may reach the host on tcp/5432 of the gateway address (the forwarder) and nothing else: no sshd, no other
  # service listening on all addresses. DROP first and ACCEPT above it, both at the top of INPUT, idempotently.
  net_id=$(docker network inspect -f '{{.Id}}' "$psql_net")
  bridge="br-${net_id:0:12}"
  iptables -C INPUT -i "$bridge" -j DROP 2>/dev/null || iptables -I INPUT 1 -i "$bridge" -j DROP
  iptables -C INPUT -i "$bridge" -p tcp -d "$psql_gateway" --dport 5432 -j ACCEPT 2>/dev/null \
    || iptables -I INPUT 1 -i "$bridge" -p tcp -d "$psql_gateway" --dport 5432 -j ACCEPT

  socat -T 300 "TCP-LISTEN:5432,bind=$psql_gateway,reuseaddr,fork,max-children=2" "TCP:$target:$port" &
  forwarder=$!
  local waited=0
  until (exec 3<> "/dev/tcp/$psql_gateway/5432") 2>/dev/null; do
    kill -0 "$forwarder" 2>/dev/null || { echo "the forwarder died while starting" >&2; exit 6; }
    waited=$((waited + 1))
    [ "$waited" -le 25 ] || { echo "the forwarder did not start listening" >&2; exit 6; }
    sleep 0.2
  done
  kill -0 "$forwarder" 2>/dev/null || { echo "the forwarder died" >&2; exit 6; }

  # shellcheck disable=SC2016
  docker run --rm -i --pull=missing --network "$psql_net" --read-only --cap-drop ALL --security-opt no-new-privileges \
    --pids-limit 64 --memory 256m --user 999:999 \
    --tmpfs /pgpw:rw,mode=0700,uid=999,gid=999,size=1m \
    --volume "$tmp/ca.crt:/ca.crt:ro" \
    --entrypoint sh "$image" -c '
      umask 077
      IFS= read -r pw
      pw=$(printf "%s" "$pw" | sed "s/\\\\/\\\\\\\\/g; s/:/\\\\:/g")
      printf "*:*:*:%s:%s\n" "$1" "$pw" > /pgpw/pgpass
      unset pw
      export PGPASSFILE=/pgpw/pgpass
      exec psql -X --no-psqlrc -v ON_ERROR_STOP=1 "host=$2 hostaddr=$5 port=5432 dbname=$4 user=$1 sslmode=verify-full sslrootcert=/ca.crt"
    ' sh "$user" "$host" "$port" "$dbname" "$psql_gateway"
}

case "${1:-}" in
  psql) shift; run_psql "$@" ;;
  release) shift; release "$@" ;;
  sha256:*) release "$@" ;;
  *) echo "usage: df-deploy <sha256:digest> | release <digest> | psql --host H --port P --dbname D --user U" >&2; exit 2 ;;
esac
