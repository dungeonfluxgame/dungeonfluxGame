#!/bin/bash
# df-db-dns: the DATABASE-ONLY resolver stub (round 3c). Installed root-owned (0755) at /usr/local/sbin/df-db-dns and run by
# df-db-dns.service as `dnsmasq` with the configuration this script writes. Why: the db-gate must resolve the managed cluster's name itself
# (so a failover needs no release), but the database bridge may not have general DNS: a compromised db-gate could otherwise use it to
# exfiltrate data in query names. So the database bridge's ONLY DNS is this stub, listening on the bridge's own gateway address
# 172.30.8.1 (df-net-rules lets dfdb0 reach exactly that address on port 53 and nothing else on the host; the db-gate container is
# configured with `dns: [172.30.8.1]`), and the stub
#   * forwards a query ONLY when the name is inside the configured database suffix (/etc/dungeonflux/db-host-suffix, for example
#     .db.ondigitalocean.com), and only to the host's own upstream resolvers;
#   * REFUSES every other name: with no default server and `no-resolv`, dnsmasq answers REFUSED, and there is no wildcard forward;
#   * reads no hosts file, caches nothing (cache-size=0, so a failover is seen at once), and answers no local names.
# The suffix match is on a label boundary: `db.ondigitalocean.com.attacker.net` and `xdb.ondigitalocean.com` are not inside
# `.db.ondigitalocean.com`. What remains is a query for a name under the provider's own database domain, answered by the provider's
# resolvers: nothing an attacker controls sees it.
#
# usage:  df-db-dns --write FILE        write the dnsmasq configuration (the unit's ExecStartPre)
#         df-db-dns --print             print it
# Options for tests: --suffix-file FILE, --resolv-conf FILE.
# NEW PACKAGE: dnsmasq-base (the binary only, without dnsmasq's own service); the cloud-init template installs it.
# STATUS: the generator and the forwarding decision are exercised by deploy/scripts/tests/test-db-dns.sh; dnsmasq itself never ran.
set -euo pipefail

suffix_file=/etc/dungeonflux/db-host-suffix
resolv=""
out=""
mode=""
listen=172.30.8.1
while [ $# -gt 0 ]; do
  case "$1" in
    --write) mode=write; out=$2; shift 2 ;;
    --print) mode=print; shift ;;
    --suffix-file) suffix_file=$2; shift 2 ;;
    --resolv-conf) resolv=$2; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[ -n "$mode" ] || { echo "usage: df-db-dns --write FILE | --print" >&2; exit 2; }

[ -r "$suffix_file" ] || { echo "no $suffix_file" >&2; exit 4; }
suffix=$(head -n 1 "$suffix_file" | tr -d '[:space:]')
[[ "$suffix" =~ ^\.[a-z0-9-]+(\.[a-z0-9-]+)+$ ]] || { echo "bad suffix in $suffix_file" >&2; exit 4; }
domain=${suffix#.}

# The host's UPSTREAM resolvers (systemd-resolved's real list first, then resolv.conf); loopback stubs are skipped.
if [ -z "$resolv" ]; then
  for candidate in /run/systemd/resolve/resolv.conf /etc/resolv.conf; do [ -r "$candidate" ] && { resolv=$candidate; break; }; done
fi
upstreams=()
if [ -n "$resolv" ] && [ -r "$resolv" ]; then
  while read -r keyword address _; do
    [ "$keyword" = nameserver ] || continue
    [[ "$address" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || continue
    case "$address" in 0.*|127.*|169.254.*|22[4-9].*|23[0-9].*|24[0-9].*|25[0-5].*) continue ;; esac
    upstreams+=("$address")
  done < "$resolv"
fi
[ "${#upstreams[@]}" -gt 0 ] || echo "df-db-dns: no upstream resolver found; every query will be refused" >&2

conf() {
  echo "# Written by df-db-dns. Answers ONLY names inside .$domain; everything else is REFUSED."
  echo "no-resolv"
  echo "no-hosts"
  echo "no-poll"
  echo "domain-needed"
  echo "bogus-priv"
  echo "cache-size=0"
  echo "port=53"
  echo "bind-dynamic"
  echo "listen-address=$listen"
  echo "user=nobody"
  echo "log-facility=-"
  local up
  for up in "${upstreams[@]}"; do echo "server=/$domain/$up"; done
}

case "$mode" in
  print) conf ;;
  write) conf > "$out.new"; chmod 0644 "$out.new"; mv -f "$out.new" "$out" ;;
esac
