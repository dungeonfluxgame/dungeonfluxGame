#!/usr/bin/env bash
# Tests of the database-only resolver stub's configuration (infra/cloud-init/df-db-dns.sh): the stub forwards a name ONLY when it is inside
# the configured database suffix and refuses every other name. dnsmasq itself is not run here; the forwarding decision is evaluated from the
# generated configuration with a small matcher that follows dnsmasq's documented rule for `server=/domain/ip` (the domain and its
# subdomains, on a label boundary) and its documented default for a name with no matching server (REFUSED when `no-resolv` is set).
#   deploy/scripts/tests/test-db-dns.sh
set -uo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
failures=0
check() { local what=$1; shift; if "$@"; then echo "PASS $what"; else echo "FAIL $what"; failures=$((failures + 1)); fi; }
t=$(mktemp -d)
printf '.db.example.test\n' > "$t/suffix"
printf 'search example.test\nnameserver 67.207.67.2\nnameserver 127.0.0.53\nnameserver 67.207.67.3\n' > "$t/resolv"
gen() { bash "$root/infra/cloud-init/df-db-dns.sh" --print --suffix-file "$t/suffix" --resolv-conf "$1" 2>/dev/null; }
conf=$(gen "$t/resolv")

# decision NAME -> "forward <ip>" or "refuse", from the configuration alone
decide() {
  local name=${1%.} line domain ip
  name=${name,,}
  while read -r line; do
    case "$line" in
      server=/*/*) domain=${line#server=/}; ip=${domain#*/}; domain=${domain%%/*}
        if [ "$name" = "$domain" ] || [[ "$name" == *".$domain" ]]; then echo "forward $ip"; return; fi ;;
    esac
  done <<<"$conf"
  echo refuse
}

check "a cluster name inside the suffix is forwarded to the host's upstream resolvers" bash -c '[[ "$1" == "forward 67.207.67.2" ]]' _ "$(decide private-abc-do-user-1.db.example.test)"
check "a standby name inside the suffix is forwarded" bash -c '[[ "$1" == forward* ]]' _ "$(decide replica-abc.db.example.test)"
for name in example.com www.example.test attacker.net 169.254.169.254.example.com db.example.test.attacker.net xdb.example.test notdb.example.test data.exfil.example.org localhost control-plane ingestor; do
  check "a non-database name is REFUSED: $name" bash -c '[[ "$1" == refuse ]]' _ "$(decide "$name")"
done
check "the suffix match is on a label boundary" bash -c '[[ "$1" == refuse && "$2" == refuse ]]' _ "$(decide xdb.example.test)" "$(decide db.example.test.attacker.net)"
check "no loopback stub resolver is used as an upstream" bash -c '! grep -q 127.0.0.53 <<<"$1"' _ "$conf"
check "every forward is domain-restricted: no bare server= line and no wildcard" bash -c '! grep -E "^server=[^/]" <<<"$1" | grep -q . && ! grep -Eq "^(address|ptr-record|local)=" <<<"$1" && ! grep -q "server=/#/" <<<"$1"' _ "$conf"
check "no default upstream (no-resolv) and no hosts file" bash -c 'grep -qx no-resolv <<<"$1" && grep -qx no-hosts <<<"$1"' _ "$conf"
check "it listens on the database bridge gateway only" bash -c 'grep -qx "listen-address=172.30.8.1" <<<"$1" && ! grep -q "^interface=\|listen-address=0.0.0.0" <<<"$1"' _ "$conf"
check "it caches nothing, so a failover is seen at once" grep -qx "cache-size=0" <<<"$conf"
conf=$(gen /dev/null)
check "with no upstream resolver every query is refused" bash -c '[[ "$1" == refuse ]]' _ "$(decide private-abc.db.example.test)"
printf 'db.example.test\n' > "$t/suffix"
check "a suffix without the leading dot is refused" bash -c '! bash "$1" --print --suffix-file "$2" >/dev/null 2>&1' _ "$root/infra/cloud-init/df-db-dns.sh" "$t/suffix"
printf '.com\n' > "$t/suffix"
check "a one-label suffix (which would cover a whole top-level domain) is refused" bash -c '! bash "$1" --print --suffix-file "$2" >/dev/null 2>&1' _ "$root/infra/cloud-init/df-db-dns.sh" "$t/suffix"
rm -rf "$t"

echo "$failures failure(s)"
[ "$failures" -eq 0 ]
