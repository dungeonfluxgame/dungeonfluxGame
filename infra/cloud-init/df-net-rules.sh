#!/bin/bash
# df-net-rules: the host firewall for the three NAT bridges that have a route out (critic round 2, change 5). Installed root-owned (0755)
# at /usr/local/sbin/df-net-rules; run as root by `df-deploy activate` and by a boot unit (df-net-rules.service, before docker).
#
# The compose file gives the networks with a route out fixed bridge names:
#   dfctl0  control-out  the control plane (object storage, identity and payment providers)
#   dfegr0  egress-out   the egress dispatcher (model providers; it also FETCHES provider output, so it is an SSRF path)
#   dfdb0   db-out       the db-gate (the managed PostgreSQL cluster, and nothing else)
# Without a rule, a NAT bridge reaches the metadata endpoint 169.254.169.254, every VPC host and any private address the host
# routes to. So, in DF-EGRESS (jumped to from DOCKER-USER, which Docker never rewrites):
#   dfctl0, dfegr0  DROP every non-public destination: link-local (the metadata endpoint), loopback, this VPC's CIDR, RFC 1918,
#                   carrier-grade NAT, multicast and reserved space. Public addresses stay reachable.
#   dfdb0           ACCEPT tcp to the cluster's address and port ONLY; DROP everything else.
# The chain is replaced atomically with iptables-restore --noflush (declaring a chain in the input empties it inside the same
# transaction), so there is no moment without rules. IPv4 only: the Docker networks have no IPv6.
#
# FAIL CLOSED. The drops for dfctl0, dfegr0 and dfdb0 are installed whatever else is wrong: with no database address (before the first
# release, or a name that does not resolve yet) dfdb0 denies everything. A systemd drop-in makes docker.service REQUIRE this unit and re-run
# it after every Docker start (ExecStartPost), so the rules also come back after a reboot and are refreshed when the managed cluster's
# address changes (a failover). The rules are kernel state: a Docker restart does not flush DOCKER-USER, a reboot or `iptables -F` does.
# Only a missing or malformed VPC CIDR is an error (the rules would not protect the VPC); Docker then does not start.
#
# usage (root): df-net-rules --auto                                        reads /opt/dungeonflux/current/host.env if there is one
#               df-net-rules --from HOST.ENV [--vpc-cidr-file FILE]        HOST.ENV holds DF_DB_HOST and DF_DB_PORT (written by df-deploy release)
#               df-net-rules --db-host H --db-port P --vpc-cidr C          explicit values
#               add --require-db to fail (after installing the safe rules) when the database address does not resolve: `activate` does
# Test mode (any user): add --dry-run to print the iptables-restore input instead of applying it.
# STATUS: the generated rules are exercised in --dry-run on the authoring machine; applying them (iptables, DOCKER-USER) is UNVERIFIED.
set -euo pipefail

from=""
db_host=""
db_port=""
vpc_cidr=""
vpc_file=/etc/dungeonflux/vpc-cidr
dry=0
require_db=0
while [ $# -gt 0 ]; do
  case "$1" in
    --from) from=$2; shift 2 ;;
    --db-host) db_host=$2; shift 2 ;;
    --db-port) db_port=$2; shift 2 ;;
    --vpc-cidr) vpc_cidr=$2; shift 2 ;;
    --vpc-cidr-file) vpc_file=$2; shift 2 ;;
    --dry-run) dry=1; shift ;;
    --require-db) require_db=1; shift ;;
    --auto) from=/opt/dungeonflux/current/host.env; [ -r "$from" ] || from=""; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
if [ "$dry" -eq 0 ] && [ "$(id -u)" -ne 0 ]; then echo "df-net-rules must run as root (or with --dry-run)" >&2; exit 2; fi

if [ -n "$from" ]; then
  [ -r "$from" ] || { echo "cannot read $from" >&2; exit 2; }
  db_host=$(sed -n 's/^DF_DB_HOST=//p' "$from" | head -n 1)
  db_port=$(sed -n 's/^DF_DB_PORT=//p' "$from" | head -n 1)
fi
if [ -z "$vpc_cidr" ] && [ -r "$vpc_file" ]; then vpc_cidr=$(head -n 1 "$vpc_file" | tr -d '[:space:]'); fi

cidr_re='^[0-9]{1,3}(\.[0-9]{1,3}){3}/[0-9]{1,2}$'
[[ "$vpc_cidr" =~ $cidr_re ]] || { echo "the VPC CIDR is missing or malformed ('$vpc_cidr'); the rules would not protect it" >&2; exit 4; }

# The database endpoint. Before the first release there is none: dfdb0 then reaches nothing at all.
db_ip=""
if [ -n "$db_host" ]; then
  if [[ "$db_host" =~ ^[A-Za-z0-9.-]{1,253}$ && "$db_port" =~ ^[0-9]{1,5}$ ]]; then
    if [[ "$db_host" =~ ^[0-9.]+$ ]]; then db_ip=$db_host; else db_ip=$(getent ahostsv4 "$db_host" 2>/dev/null | awk 'NR == 1 { print $1 }' || true); fi
    case "$db_ip" in
      0.*|127.*|169.254.*|22[4-9].*|23[0-9].*|24[0-9].*|25[0-5].*) echo "refusing to allow $db_ip" >&2; db_ip="" ;;
    esac
    [[ "$db_ip" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || db_ip=""
  else
    echo "malformed database host or port; the database bridge will deny everything" >&2
  fi
  if [ -z "$db_ip" ]; then echo "df-net-rules: the database address is unknown; dfdb0 denies everything until it resolves" >&2; fi
fi

rules() {
  echo '*filter'
  echo ':DF-EGRESS - [0:0]'
  local bridge net
  for bridge in dfctl0 dfegr0; do
    for net in 169.254.0.0/16 127.0.0.0/8 "$vpc_cidr" 10.0.0.0/8 172.16.0.0/12 192.168.0.0/16 100.64.0.0/10 224.0.0.0/3; do
      echo "-A DF-EGRESS -i $bridge -d $net -j DROP"
    done
  done
  if [ -n "$db_ip" ]; then echo "-A DF-EGRESS -i dfdb0 -p tcp -d $db_ip/32 --dport $db_port -j ACCEPT"; fi
  echo '-A DF-EGRESS -i dfdb0 -j DROP'
  echo 'COMMIT'
}

if [ "$dry" -eq 1 ]; then
  rules
  if [ "$require_db" -eq 1 ] && [ -z "$db_ip" ]; then echo "df-net-rules: --require-db and the database address is unknown" >&2; exit 4; fi
  exit 0
fi
command -v iptables >/dev/null 2>&1 && command -v iptables-restore >/dev/null 2>&1 || { echo "iptables is not installed" >&2; exit 4; }
iptables -N DOCKER-USER 2>/dev/null || true
iptables -N DF-EGRESS 2>/dev/null || true
for bridge in dfctl0 dfegr0 dfdb0; do
  iptables -C DOCKER-USER -i "$bridge" -j DF-EGRESS 2>/dev/null || iptables -I DOCKER-USER 1 -i "$bridge" -j DF-EGRESS
done
rules | iptables-restore --noflush
echo "egress rules installed (database ${db_ip:-none}:${db_port:-none}, VPC $vpc_cidr)"
if [ "$require_db" -eq 1 ] && [ -z "$db_ip" ]; then echo "df-net-rules: --require-db and the database address is unknown" >&2; exit 4; fi
