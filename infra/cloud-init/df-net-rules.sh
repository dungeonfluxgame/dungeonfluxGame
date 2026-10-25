#!/bin/bash
# df-net-rules: the host firewall for the Docker bridges of the production stack. Installed root-owned (0755) at /usr/local/sbin/df-net-rules;
# run as root by `df-deploy activate`, by a boot unit (df-net-rules.service, before docker), after every Docker start (a docker.service
# drop-in) and EVERY 30 SECONDS by df-net-rules-refresh.timer. FAIL CLOSED: the drops are installed whatever else is wrong; only a missing or
# malformed VPC CIDR is an error (the rules would not protect it), and then Docker does not start.
#
# EVERY compose network has a FIXED bridge name (deploy/compose/production.compose.yml, enforced by check-production-policy.sh):
#   with a route out   dfpub0 public (the edge)   dfctl0 control-out   dfegr0 egress-out   dfdb0 db-out   dfmon0 monitor-out
#   internal           dfin0 ingress   dfdata0 data   dfdsp0 dispatch   dfmn0 monitoring   dfobs0 observe   dfrun0 runner-net
#                      dfslot1 .. dfslotN  the runtime slot networks (N = the number of [[slots]] tables of the active release: DF_SLOTS)
#
# TWO CHAINS, because a container reaches two different things: other hosts through FORWARD (DOCKER-USER, DF-EGRESS) and the HOST ITSELF
# through INPUT (DF-INPUT). Without the INPUT rules any container on any bridge can connect to a service listening on the host's addresses.
#   DF-INPUT   EVERY bridge above: after letting replies to host-initiated connections through, drop everything addressed to the host
#              (-m addrtype --dst-type LOCAL). The ONE exception: the database bridge may send DNS (udp and tcp 53) to the host's database-only
#              resolver stub at the bridge's own gateway address 172.30.8.1 (infra/cloud-init/df-db-dns.sh), and to nothing else on the host.
#   DF-EGRESS  dfpub0 dfctl0 dfegr0 dfmon0  DROP every non-public destination (link-local = the metadata endpoint, loopback, this VPC, RFC 1918,
#              carrier-grade NAT, multicast, reserved). dfpub0 first lets replies through (a client behind a private load balancer is a
#              legitimate destination of a reply). Public addresses stay reachable: ACME for the edge, object storage, providers, the alert receiver.
#              dfdb0  ACCEPT tcp 25060-25061 to the cluster's RESOLVED ADDRESSES ONLY (/32 each: the primary and any standby host the release
#              names, DF_DB_HOSTS in host.env), nothing else; DROP everything else. No DNS and no VPC-wide rule: a compromised db-gate can reach
#              exactly the database nodes, and its only DNS is the stub, which answers for the database suffix and refuses every other name.
#
# THE ADDRESSES ARE REFRESHED. Each run resolves the names in DF_DB_HOSTS on the host (getent), requires every address to be an ordinary
# unicast address INSIDE the VPC, and replaces the chains atomically (iptables-restore --noflush declares a chain empty inside the same
# transaction). A failover that moves the primary is followed within one timer period (30 s). If a name does not resolve in a run the LAST
# KNOWN good addresses for it (the state file, /run/dungeonflux/db-addresses) are kept rather than dropping the database; a name that has
# never resolved contributes nothing, so with no known address dfdb0 reaches nothing at all.
#
# usage (root): df-net-rules --auto                                   reads the active release's host.env (DF_SLOTS, DF_DB_HOSTS) if there is one
#               df-net-rules --from HOST.ENV | --slots N [--db-hosts a,b]   [--vpc-cidr C | --vpc-cidr-file FILE]
# Test mode (any user): --dry-run prints the iptables-restore input instead of applying it; --resolve-file FILE (name address... per line)
#               stands in for getent and --state-file FILE for the last-known-good state, both only with --dry-run.
# STATUS: the generated rules are exercised in --dry-run (deploy/scripts/tests/test-net-and-pins.sh); applying them is UNVERIFIED.
set -euo pipefail

from=""
slots=2
db_hosts=""
vpc_cidr=""
vpc_file=/etc/dungeonflux/vpc-cidr
state_file=/run/dungeonflux/db-addresses
resolve_file=""
dry=0
while [ $# -gt 0 ]; do
  case "$1" in
    --from) from=$2; shift 2 ;;
    --slots) slots=$2; shift 2 ;;
    --db-hosts) db_hosts=$2; shift 2 ;;
    --vpc-cidr) vpc_cidr=$2; shift 2 ;;
    --vpc-cidr-file) vpc_file=$2; shift 2 ;;
    --resolve-file) resolve_file=$2; shift 2 ;;
    --state-file) state_file=$2; shift 2 ;;
    --dry-run) dry=1; shift ;;
    --auto) from=/opt/dungeonflux/current/host.env; [ -r "$from" ] || from=""; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
if [ "$dry" -eq 0 ] && [ "$(id -u)" -ne 0 ]; then echo "df-net-rules must run as root (or with --dry-run)" >&2; exit 2; fi
if [ "$dry" -eq 0 ] && [ -n "$resolve_file" ]; then echo "--resolve-file is a test option (with --dry-run only)" >&2; exit 2; fi

if [ -n "$from" ]; then
  [ -r "$from" ] || { echo "cannot read $from" >&2; exit 2; }
  value=$(sed -n 's/^DF_SLOTS=//p' "$from" | head -n 1)
  [ -z "$value" ] || slots=$value
  value=$(sed -n 's/^DF_DB_HOSTS=//p' "$from" | head -n 1)
  [ -z "$value" ] || db_hosts=$value
fi
[[ "$slots" =~ ^[0-9]{1,2}$ ]] && [ "$slots" -ge 1 ] && [ "$slots" -le 99 ] || { echo "the slot count must be 1..99 (got '$slots')" >&2; exit 2; }
if [ -z "$vpc_cidr" ] && [ -r "$vpc_file" ]; then vpc_cidr=$(head -n 1 "$vpc_file" | tr -d '[:space:]'); fi
cidr_re='^([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,3})/([0-9]{1,2})$'
[[ "$vpc_cidr" =~ $cidr_re ]] || { echo "the VPC CIDR is missing or malformed ('$vpc_cidr'); the rules would not protect it" >&2; exit 4; }

ip_to_int() { local IFS=.; read -r a b c d <<<"$1"; echo $(( (a << 24) + (b << 16) + (c << 8) + d )); }
net_int=$(ip_to_int "${BASH_REMATCH[1]}.${BASH_REMATCH[2]}.${BASH_REMATCH[3]}.${BASH_REMATCH[4]}")
prefix=${BASH_REMATCH[5]}
[ "$prefix" -ge 8 ] && [ "$prefix" -le 30 ] || { echo "implausible VPC prefix /$prefix" >&2; exit 4; }
mask=$(( (0xFFFFFFFF << (32 - prefix)) & 0xFFFFFFFF ))
in_vpc() { [ $(( $(ip_to_int "$1") & mask )) -eq $(( net_int & mask )) ]; }

# --- the database addresses -------------------------------------------------------------------------------------------------------------------
declare -A known=()   # name -> "addr addr ..." from the previous run
if [ -r "$state_file" ]; then
  while read -r name addrs; do [ -z "${name:-}" ] || known[$name]=$addrs; done < "$state_file"
fi
resolve() { # resolve NAME -> addresses, one per line
  if [ -n "$resolve_file" ]; then
    awk -v n="$1" '$1 == n { for (i = 2; i <= NF; i++) print $i }' "$resolve_file"
  else
    getent ahostsv4 "$1" 2>/dev/null | awk '{ print $1 }' | sort -u
  fi
}
db_addrs=()
declare -A fresh=()
if [ -n "$db_hosts" ]; then
  IFS=, read -r -a names <<<"$db_hosts"
  for name in "${names[@]}"; do
    [[ "$name" =~ ^[A-Za-z0-9.-]{1,253}$ ]] || { echo "df-net-rules: ignoring a malformed database host name" >&2; continue; }
    good=()
    while read -r addr; do
      [[ "$addr" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || continue
      # An ordinary unicast address INSIDE this VPC: a resolver that answers anything else (or is made to) opens nothing.
      in_vpc "$addr" || { echo "df-net-rules: $name resolved to $addr, outside the VPC; ignored" >&2; continue; }
      case "$addr" in 0.*|127.*|169.254.*|22[4-9].*|23[0-9].*|24[0-9].*|25[0-5].*) continue ;; esac
      good+=("$addr")
    done < <(resolve "$name")
    if [ "${#good[@]}" -gt 0 ]; then
      fresh[$name]="${good[*]}"
    elif [ -n "${known[$name]:-}" ]; then
      echo "df-net-rules: $name did not resolve; keeping its last known addresses (${known[$name]})" >&2
      fresh[$name]=${known[$name]}
    else
      echo "df-net-rules: $name has never resolved; the database bridge reaches nothing for it" >&2
    fi
  done
  for name in "${!fresh[@]}"; do for addr in ${fresh[$name]}; do db_addrs+=("$addr"); done; done
fi
mapfile -t db_addrs < <(printf '%s\n' "${db_addrs[@]:-}" | sed '/^$/d' | sort -u)

bridges_internal=(dfin0 dfdata0 dfdsp0 dfmn0 dfobs0 dfrun0)
n=1
while [ "$n" -le "$slots" ]; do bridges_internal+=("dfslot$n"); n=$((n + 1)); done
bridges_out=(dfpub0 dfctl0 dfegr0 dfdb0 dfmon0)
bridges_all=("${bridges_internal[@]}" "${bridges_out[@]}")
db_dns_stub=172.30.8.1

rules() {
  echo '*filter'
  echo ':DF-EGRESS - [0:0]'
  local bridge net addr
  echo '-A DF-EGRESS -i dfpub0 -m conntrack --ctstate ESTABLISHED,RELATED -j RETURN'
  for bridge in dfpub0 dfctl0 dfegr0 dfmon0; do
    for net in 169.254.0.0/16 127.0.0.0/8 "$vpc_cidr" 10.0.0.0/8 172.16.0.0/12 192.168.0.0/16 100.64.0.0/10 224.0.0.0/3; do
      echo "-A DF-EGRESS -i $bridge -d $net -j DROP"
    done
  done
  for addr in "${db_addrs[@]}"; do
    echo "-A DF-EGRESS -i dfdb0 -p tcp -d $addr/32 --dport 25060:25061 -j ACCEPT"
  done
  echo '-A DF-EGRESS -i dfdb0 -j DROP'
  # INPUT: nothing on any bridge may talk to the host itself (replies to host-initiated connections excepted), except the database
  # bridge's DNS to the database-only stub.
  echo ':DF-INPUT - [0:0]'
  for bridge in "${bridges_all[@]}"; do
    echo "-A DF-INPUT -i $bridge -m conntrack --ctstate ESTABLISHED,RELATED -j RETURN"
    if [ "$bridge" = dfdb0 ]; then
      echo "-A DF-INPUT -i dfdb0 -p udp -d $db_dns_stub/32 --dport 53 -j RETURN"
      echo "-A DF-INPUT -i dfdb0 -p tcp -d $db_dns_stub/32 --dport 53 -j RETURN"
    fi
    echo "-A DF-INPUT -i $bridge -m addrtype --dst-type LOCAL -j DROP"
  done
  echo 'COMMIT'
}

if [ "$dry" -eq 1 ]; then
  rules
  exit 0
fi
command -v iptables >/dev/null 2>&1 && command -v iptables-restore >/dev/null 2>&1 || { echo "iptables is not installed" >&2; exit 4; }
iptables -N DOCKER-USER 2>/dev/null || true
iptables -N DF-EGRESS 2>/dev/null || true
iptables -N DF-INPUT 2>/dev/null || true
for bridge in dfpub0 dfctl0 dfegr0 dfdb0 dfmon0; do
  iptables -C DOCKER-USER -i "$bridge" -j DF-EGRESS 2>/dev/null || iptables -I DOCKER-USER 1 -i "$bridge" -j DF-EGRESS
done
# The jumps go to position 1 per bridge; none of these interfaces is the df-psql bridge, whose ACCEPT/DROP pair `df-deploy psql` keeps at the top.
for bridge in "${bridges_all[@]}"; do
  iptables -C INPUT -i "$bridge" -j DF-INPUT 2>/dev/null || iptables -I INPUT 1 -i "$bridge" -j DF-INPUT
done
rules | iptables-restore --noflush
# Remember what resolved, for the next run (tmpfs: gone at reboot, when everything is resolved afresh).
mkdir -p "$(dirname "$state_file")"
for name in "${!fresh[@]}"; do echo "$name ${fresh[$name]}"; done > "$state_file.new"
mv -f "$state_file.new" "$state_file"
echo "firewall rules installed ($slots runtime slot bridges, ${#db_addrs[@]} database address(es), VPC $vpc_cidr)"
