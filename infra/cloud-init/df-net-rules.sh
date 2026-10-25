#!/bin/bash
# df-net-rules: the host firewall for the Docker bridges of the production stack. Installed root-owned (0755) at /usr/local/sbin/df-net-rules;
# run as root by `df-deploy activate` and by a boot unit (df-net-rules.service, before docker), and re-run after every Docker start by a
# docker.service drop-in. FAIL CLOSED: the drops are installed whatever else is wrong; only a missing or malformed VPC CIDR is an error
# (the rules would not protect it), and then Docker does not start.
#
# EVERY compose network has a FIXED bridge name (deploy/compose/production.compose.yml, enforced by check-production-policy.sh):
#   with a route out   dfpub0 public (the edge)   dfctl0 control-out   dfegr0 egress-out   dfdb0 db-out   dfmon0 monitor-out
#   internal           dfin0 ingress   dfdata0 data   dfdsp0 dispatch   dfmn0 monitoring   dfobs0 observe   dfrun0 runner-net
#                      dfslot1 .. dfslotN  the runtime slot networks (N = the number of [[slots]] tables of the active release: DF_SLOTS)
#
# TWO CHAINS, because a container reaches two different things: other hosts through FORWARD (DOCKER-USER, DF-EGRESS) and the HOST ITSELF
# through INPUT (DF-INPUT). Without the INPUT rules any container on any bridge can connect to a service listening on the host's addresses
# (sshd, a metrics port, the psql forwarder's gateway): the destination is a LOCAL address, so DOCKER-USER never sees it.
#   DF-INPUT   EVERY bridge above: after letting replies to host-initiated connections through, drop everything addressed to the host
#              (-m addrtype --dst-type LOCAL). No port is excepted: every legitimate destination is reached by forwarding. The df-psql bridge of
#              \`df-deploy psql\` is not in this list; its ACCEPT for the forwarder sits at the very top of INPUT and stays above these jumps.
#   DF-EGRESS  dfpub0 dfctl0 dfegr0 dfmon0  DROP every non-public destination (link-local = the metadata endpoint, loopback, this VPC, RFC 1918,
#              carrier-grade NAT, multicast, reserved). dfpub0 first lets replies through (a client behind a private load balancer is a legitimate
#              destination of a reply). Public addresses stay reachable: ACME for the edge, object storage, providers, the alert receiver.
#              dfdb0  ACCEPT only DNS to the host's own upstream resolvers (udp and tcp 53) and tcp to THIS VPC's CIDR on the managed database
#              ports 25060 and 25061; DROP everything else. The db-gate's haproxy RESOLVES the cluster's name itself (resolvers, init-addr none),
#              so a failover that changes the cluster's address needs no firewall change and no new release.
# The chains are replaced atomically with iptables-restore --noflush (declaring a chain empties it inside the same transaction). IPv4 only.
#
# usage (root): df-net-rules --auto                                   reads the active release's host.env (DF_SLOTS) if there is one
#               df-net-rules --from HOST.ENV | --slots N   [--vpc-cidr C | --vpc-cidr-file FILE]   [--resolv-conf FILE]
# Test mode (any user): add --dry-run to print the iptables-restore input instead of applying it.
# STATUS: the generated rules are exercised in --dry-run (deploy/scripts/tests/test-net-and-pins.sh); applying them is UNVERIFIED.
set -euo pipefail

from=""
slots=2
vpc_cidr=""
vpc_file=/etc/dungeonflux/vpc-cidr
resolv=""
dry=0
while [ $# -gt 0 ]; do
  case "$1" in
    --from) from=$2; shift 2 ;;
    --slots) slots=$2; shift 2 ;;
    --vpc-cidr) vpc_cidr=$2; shift 2 ;;
    --vpc-cidr-file) vpc_file=$2; shift 2 ;;
    --resolv-conf) resolv=$2; shift 2 ;;
    --dry-run) dry=1; shift ;;
    --auto) from=/opt/dungeonflux/current/host.env; [ -r "$from" ] || from=""; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
if [ "$dry" -eq 0 ] && [ "$(id -u)" -ne 0 ]; then echo "df-net-rules must run as root (or with --dry-run)" >&2; exit 2; fi

if [ -n "$from" ]; then
  [ -r "$from" ] || { echo "cannot read $from" >&2; exit 2; }
  value=$(sed -n 's/^DF_SLOTS=//p' "$from" | head -n 1)
  [ -z "$value" ] || slots=$value
fi
[[ "$slots" =~ ^[0-9]{1,2}$ ]] && [ "$slots" -ge 1 ] && [ "$slots" -le 99 ] || { echo "the slot count must be 1..99 (got '$slots')" >&2; exit 2; }
if [ -z "$vpc_cidr" ] && [ -r "$vpc_file" ]; then vpc_cidr=$(head -n 1 "$vpc_file" | tr -d '[:space:]'); fi
cidr_re='^[0-9]{1,3}(\.[0-9]{1,3}){3}/[0-9]{1,2}$'
[[ "$vpc_cidr" =~ $cidr_re ]] || { echo "the VPC CIDR is missing or malformed ('$vpc_cidr'); the rules would not protect it" >&2; exit 4; }

# The host's UPSTREAM resolvers (what Docker's embedded DNS forwards to, from inside the container's network namespace). systemd-resolved's
# real list first, then resolv.conf; loopback stubs are skipped. None found: dfdb0 gets no DNS (and says so), which fails closed.
resolvers=()
if [ -z "$resolv" ]; then
  for candidate in /run/systemd/resolve/resolv.conf /etc/resolv.conf; do [ -r "$candidate" ] && { resolv=$candidate; break; }; done
fi
if [ -n "$resolv" ] && [ -r "$resolv" ]; then
  while read -r keyword address _; do
    [ "$keyword" = nameserver ] || continue
    [[ "$address" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || continue
    case "$address" in 0.*|127.*|169.254.*|22[4-9].*|23[0-9].*|24[0-9].*|25[0-5].*) continue ;; esac
    resolvers+=("$address")
  done < "$resolv"
fi
if [ "${#resolvers[@]}" -eq 0 ]; then echo "df-net-rules: no upstream resolver found; the database bridge gets no DNS" >&2; fi

bridges_internal=(dfin0 dfdata0 dfdsp0 dfmn0 dfobs0 dfrun0)
n=1
while [ "$n" -le "$slots" ]; do bridges_internal+=("dfslot$n"); n=$((n + 1)); done
bridges_out=(dfpub0 dfctl0 dfegr0 dfdb0 dfmon0)
bridges_all=("${bridges_internal[@]}" "${bridges_out[@]}")

rules() {
  echo '*filter'
  echo ':DF-EGRESS - [0:0]'
  local bridge net resolver
  echo '-A DF-EGRESS -i dfpub0 -m conntrack --ctstate ESTABLISHED,RELATED -j RETURN'
  for bridge in dfpub0 dfctl0 dfegr0 dfmon0; do
    for net in 169.254.0.0/16 127.0.0.0/8 "$vpc_cidr" 10.0.0.0/8 172.16.0.0/12 192.168.0.0/16 100.64.0.0/10 224.0.0.0/3; do
      echo "-A DF-EGRESS -i $bridge -d $net -j DROP"
    done
  done
  for resolver in "${resolvers[@]}"; do
    echo "-A DF-EGRESS -i dfdb0 -p udp -d $resolver/32 --dport 53 -j ACCEPT"
    echo "-A DF-EGRESS -i dfdb0 -p tcp -d $resolver/32 --dport 53 -j ACCEPT"
  done
  echo "-A DF-EGRESS -i dfdb0 -p tcp -d $vpc_cidr --dport 25060:25061 -j ACCEPT"
  echo '-A DF-EGRESS -i dfdb0 -j DROP'
  # INPUT: nothing on any bridge may talk to the host itself (replies to host-initiated connections excepted).
  echo ':DF-INPUT - [0:0]'
  for bridge in "${bridges_all[@]}"; do
    echo "-A DF-INPUT -i $bridge -m conntrack --ctstate ESTABLISHED,RELATED -j RETURN"
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
echo "firewall rules installed ($slots runtime slot bridges, ${#resolvers[@]} upstream resolver(s), VPC $vpc_cidr)"
