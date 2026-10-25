#!/usr/bin/env bash
# Tests of two security rules that live in scripts: (1) df-net-rules generates the INPUT and FORWARD rules it must, and fails CLOSED;
# (2) check-config-policy.sh requires the service-issuer pin on every client that verifies a service server. Needs bash only.
#   deploy/scripts/tests/test-net-and-pins.sh
set -uo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
failures=0
check() { local what=$1; shift; if "$@"; then echo "PASS $what"; else echo "FAIL $what"; failures=$((failures + 1)); fi; }

# --- (1) df-net-rules ---------------------------------------------------------------------------------------------------------------------------
work=$(mktemp -d)
state=$work/state
res=$work/resolve; : > "$work/empty"
printf 'primary.db.example.test 10.30.0.9\nstandby.db.example.test 10.30.0.10 10.30.0.11\n' > "$res"
rules() { bash "$root/infra/cloud-init/df-net-rules.sh" --dry-run --vpc-cidr 10.30.0.0/16 --state-file "$state" "$@" 2>/dev/null; }
dbrules() { rules --slots 3 --db-hosts primary.db.example.test,standby.db.example.test --resolve-file "$1"; }
out=$(dbrules "$res")
# INPUT: EVERY bridge of the stack, slot networks included
for bridge in dfpub0 dfin0 dfdata0 dfdsp0 dfmn0 dfobs0 dfrun0 dfctl0 dfegr0 dfdb0 dfmon0 dfslot1 dfslot2 dfslot3; do
  check "INPUT: $bridge cannot reach an address of the host itself" grep -qx -- "-A DF-INPUT -i $bridge -m addrtype --dst-type LOCAL -j DROP" <<<"$out"
  check "INPUT: replies to host-initiated connections on $bridge are let through first" grep -qx -- "-A DF-INPUT -i $bridge -m conntrack --ctstate ESTABLISHED,RELATED -j RETURN" <<<"$out"
done
check "the slot count follows DF_SLOTS: no bridge beyond the third" bash -c '! grep -q dfslot4 <<<"$1"' _ "$out"
# FORWARD: the bridges with a route out
for bridge in dfpub0 dfctl0 dfegr0 dfmon0; do
  check "FORWARD: $bridge cannot reach the metadata endpoint" grep -qx -- "-A DF-EGRESS -i $bridge -d 169.254.0.0/16 -j DROP" <<<"$out"
  check "FORWARD: $bridge cannot reach the VPC" grep -qx -- "-A DF-EGRESS -i $bridge -d 10.30.0.0/16 -j DROP" <<<"$out"
done
check "the public bridge lets replies to private clients (a load balancer) through before its drops" bash -c 'grep -- "DF-EGRESS -i dfpub0" <<<"$1" | head -n 1 | grep -q "ESTABLISHED,RELATED -j RETURN"' _ "$out"

# --- the database bridge: tcp 25060-25061 to the cluster's RESOLVED addresses only; DNS only to the stub -----------------------------------------
check "dfdb0: the primary's address, /32, the managed ports" grep -qx -- "-A DF-EGRESS -i dfdb0 -p tcp -d 10.30.0.9/32 --dport 25060:25061 -j ACCEPT" <<<"$out"
check "dfdb0: every address of the standby host too" bash -c 'grep -qx -- "-A DF-EGRESS -i dfdb0 -p tcp -d 10.30.0.10/32 --dport 25060:25061 -j ACCEPT" <<<"$1" && grep -qx -- "-A DF-EGRESS -i dfdb0 -p tcp -d 10.30.0.11/32 --dport 25060:25061 -j ACCEPT" <<<"$1"' _ "$out"
check "dfdb0: NO VPC-wide rule and NO resolver rule in the forward chain" bash -c '! grep -E -- "-i dfdb0.*(10.30.0.0/16|--dport 53)" <<<"$1" | grep -v DF-INPUT | grep -q .' _ "$out"
check "dfdb0: exactly three ACCEPTs (primary plus the standby's two addresses)" bash -c 'test "$(grep -c -- "DF-EGRESS.*-j ACCEPT" <<<"$1")" -eq 3' _ "$out"
check "dfdb0: everything else is dropped, and that DROP comes after the ACCEPTs" bash -c 'grep -- "DF-EGRESS -i dfdb0" <<<"$1" | tail -n 1 | grep -qx -- "-A DF-EGRESS -i dfdb0 -j DROP"' _ "$out"
check "dfdb0: its DNS goes to the database-only stub at the bridge gateway, udp and tcp, and nowhere else on the host" bash -c 'grep -qx -- "-A DF-INPUT -i dfdb0 -p udp -d 172.30.8.1/32 --dport 53 -j RETURN" <<<"$1" && grep -qx -- "-A DF-INPUT -i dfdb0 -p tcp -d 172.30.8.1/32 --dport 53 -j RETURN" <<<"$1" && test "$(grep -c -- "-i dfdb0 .*-j RETURN" <<<"$1")" -eq 3' _ "$out"
check "no other bridge may reach the stub" bash -c '! grep -E -- "-i (dfctl0|dfegr0|dfmon0|dfpub0|dfslot1).* --dport 53" <<<"$1" | grep -q .' _ "$out"

# --- a FAILOVER: the primary moves to another address ---------------------------------------------------------------------------------------------
printf 'primary.db.example.test 10.30.0.77\nstandby.db.example.test 10.30.0.10 10.30.0.11\n' > "$work/resolve-failover"
out2=$(dbrules "$work/resolve-failover")
check "after a failover the NEW primary address is accepted" grep -qx -- "-A DF-EGRESS -i dfdb0 -p tcp -d 10.30.0.77/32 --dport 25060:25061 -j ACCEPT" <<<"$out2"
check "and the OLD primary address is no longer accepted (the chain is replaced, not appended to)" bash -c '! grep -q "10.30.0.9/32" <<<"$1"' _ "$out2"
check "the standby is unchanged" grep -qx -- "-A DF-EGRESS -i dfdb0 -p tcp -d 10.30.0.10/32 --dport 25060:25061 -j ACCEPT" <<<"$out2"

# --- bad answers ----------------------------------------------------------------------------------------------------------------------------------
printf 'primary.db.example.test 169.254.169.254 8.8.8.8 127.0.0.1 10.31.0.5\n' > "$work/resolve-bad"
out3=$(rules --slots 1 --db-hosts primary.db.example.test --resolve-file "$work/resolve-bad")
check "addresses outside the VPC, link-local, loopback and public ones are never accepted, whatever the resolver says" bash -c '! grep -q -- "-j ACCEPT" <<<"$1"' _ "$out3"
check "...so the database bridge reaches nothing (fail closed)" grep -qx -- "-A DF-EGRESS -i dfdb0 -j DROP" <<<"$out3"
out4=$(rules --slots 1 --db-hosts primary.db.example.test --resolve-file "$work/empty")
check "a name that never resolved contributes nothing" bash -c '! grep -q -- "-j ACCEPT" <<<"$1"' _ "$out4"
check "no database hosts at all: still the full set of drops" bash -c 'grep -qx -- "-A DF-EGRESS -i dfdb0 -j DROP" <<<"$1" && grep -qx -- "-A DF-INPUT -i dfctl0 -m addrtype --dst-type LOCAL -j DROP" <<<"$1"' _ "$(rules)"
check "a malformed host name is ignored" bash -c 'bash "$1" --dry-run --vpc-cidr 10.30.0.0/16 --state-file "$3" --db-hosts "a b;c" --resolve-file "$2" 2>&1 | grep -q "malformed database host name"' _ "$root/infra/cloud-init/df-net-rules.sh" "$res" "$state"
# the last known good addresses survive a transient resolver failure (the state file is what a real run leaves behind)
printf 'primary.db.example.test 10.30.0.9\n' > "$state"
out5=$(rules --slots 1 --db-hosts primary.db.example.test --resolve-file "$work/empty")
check "a transient resolution failure keeps the last known address" grep -qx -- "-A DF-EGRESS -i dfdb0 -p tcp -d 10.30.0.9/32 --dport 25060:25061 -j ACCEPT" <<<"$out5"
rm -f "$state"

printf 'DF_DOCKER_GID=1\nDF_SLOTS=4\nDF_DB_HOSTS=primary.db.example.test\n' > "$work/host.env"
out6=$(rules --from "$work/host.env" --resolve-file "$res")
check "host.env sets the slot bridges and the database names" bash -c 'grep -qx -- "-A DF-INPUT -i dfslot4 -m addrtype --dst-type LOCAL -j DROP" <<<"$1" && grep -qx -- "-A DF-EGRESS -i dfdb0 -p tcp -d 10.30.0.9/32 --dport 25060:25061 -j ACCEPT" <<<"$1" && ! grep -q "10.30.0.10" <<<"$1"' _ "$out6"
check "a missing VPC CIDR is refused (the rules would not protect it)" bash -c '! bash "$1" --dry-run >/dev/null 2>&1' _ "$root/infra/cloud-init/df-net-rules.sh"
check "a malformed slot count is refused" bash -c '! bash "$1" --dry-run --vpc-cidr 10.30.0.0/16 --slots 0 >/dev/null 2>&1' _ "$root/infra/cloud-init/df-net-rules.sh"
check "--resolve-file is refused outside --dry-run" bash -c '! bash "$1" --vpc-cidr 10.30.0.0/16 --resolve-file "$2" >/dev/null 2>&1' _ "$root/infra/cloud-init/df-net-rules.sh" "$res"
rm -rf "$work"

# --- (2) the issuer pin check ---------------------------------------------------------------------------------------------------------------------
tmp=$(mktemp -d)
mkdir -p "$tmp/deploy/scripts" "$tmp/deploy/config/staging" "$tmp/deploy/config/production" "$tmp/deploy/compose"
cp "$root/deploy/scripts/check-config-policy.sh" "$tmp/deploy/scripts/"
: > "$tmp/deploy/compose/production.compose.yml"
cfg() { bash "$tmp/deploy/scripts/check-config-policy.sh" 2>&1; }
cat > "$tmp/deploy/config/staging/egress.toml" <<'EOF'
[dispatch_tls]
certificate_file = "/run/secrets/dispatch_tls_certificate"
key_file = "/run/secrets/dispatch_tls_key"
client_ca_file = "/run/secrets/internal_ca_certificate"
identity = "client_certificate_uri_san"

[control_plane]
address = "172.30.4.3:9093"
ca_file = "/run/secrets/internal_ca_certificate"
server_name = "control-plane"
client_certificate_file = "/run/secrets/dispatcher_client_certificate"
EOF
out=$(cfg)
check "a client section that trusts the root CA without the issuer pin is a violation" grep -q "without server_issuer_file" <<<"$out"
sed -i 's#^server_name = "control-plane"#server_name = "control-plane"\nserver_issuer_file = "/run/secrets/service_issuer_certificate"#' "$tmp/deploy/config/staging/egress.toml"
check "pinning the SERVICE issuer clears it" bash -c '! grep -q "without server_issuer_file" <<<"$1"' _ "$(cfg)"
sed -i 's#service_issuer_certificate#placement_issuer_certificate#' "$tmp/deploy/config/staging/egress.toml"
check "pinning any other certificate does not" grep -q "without server_issuer_file" <<<"$(cfg)"
rm -f "$tmp/deploy/config/staging/egress.toml"
cat > "$tmp/deploy/config/staging/upstream.toml" <<'EOF'
[runtime_upstream]
ca_file = "/run/secrets/internal_ca_certificate"
server_issuer_file = "/run/secrets/service_issuer_certificate"
expected_identity = "urn:dungeonflux:placement-server:{placement_id}"
EOF
check "a runtime upstream pinned to the SERVICE issuer is refused" grep -q "must pin runtime_server_issuer_certificate" <<<"$(cfg)"
sed -i 's#service_issuer_certificate#runtime_server_issuer_certificate#' "$tmp/deploy/config/staging/upstream.toml"
check "pinned to the runtime-server issuer it is accepted" bash -c '! grep -q "FAIL" <<<"$1"' _ "$(cfg)"
sed -i 's#^expected_identity.*#expected_identity = "any"#' "$tmp/deploy/config/staging/upstream.toml"
check "an upstream that does not check the placement id is refused" grep -q "must check the URI SAN" <<<"$(cfg)"
cat > "$tmp/deploy/config/staging/misc.toml" <<'EOF'
[x]
key_file = "/run/secrets/runtime_server_issuer_key"
EOF
check "a config other than the control plane's that names the runtime-server key is refused" grep -q "only the control plane may reference runtime_server_issuer_key" <<<"$(cfg)"
# --- (3) the issuer-to-identity-class pairing -------------------------------------------------------------------------------------------------
rm -f "$tmp"/deploy/config/staging/*.toml
cp "$root/deploy/config/staging/control-plane.toml" "$tmp/deploy/config/staging/control-plane.toml"
cp "$root/deploy/config/staging/egress.toml" "$tmp/deploy/config/staging/egress.toml"
cp "$root/deploy/config/staging/ingestor.toml" "$tmp/deploy/config/staging/ingestor.toml"
cp "$root/deploy/config/staging/runtime-1.descriptor.toml" "$tmp/deploy/config/staging/runtime-1.descriptor.toml"
check "the shipped staging configs pair every receiver with the right issuer" bash -c '! grep -q "FAIL" <<<"$1"' _ "$(cfg)"
# swap: the media gateway (placement identities) pointed at the SERVICE issuer
sed -i '/^\[internal_tls.media_gateway\]/,/^\[/ s#issuer_file = "/run/secrets/placement_issuer_certificate"#issuer_file = "/run/secrets/service_issuer_certificate"#' "$tmp/deploy/config/staging/control-plane.toml"
check "a media gateway that takes placement identities through the service issuer is refused" grep -q "internal_tls.media_gateway. must set issuer_file = placement_issuer_certificate" <<<"$(cfg)"
cp "$root/deploy/config/staging/control-plane.toml" "$tmp/deploy/config/staging/control-plane.toml"
# swap: the dispatcher gateway (a fixed identity) pointed at the PLACEMENT issuer
sed -i '/^\[internal_tls.dispatcher_gateway\]/,/^\[/ s#issuer_file = "/run/secrets/service_issuer_certificate"#issuer_file = "/run/secrets/placement_issuer_certificate"#' "$tmp/deploy/config/staging/control-plane.toml"
check "a dispatcher gateway that takes the dispatcher identity through the placement issuer is refused" grep -q "internal_tls.dispatcher_gateway. must set issuer_file = service_issuer_certificate" <<<"$(cfg)"
cp "$root/deploy/config/staging/control-plane.toml" "$tmp/deploy/config/staging/control-plane.toml"
# a missing pairing line
sed -i '/^placement_issuer_file/d' "$tmp/deploy/config/staging/ingestor.toml"
check "an ingestor without the placement-issuer pin is refused" grep -q "identity. must set placement_issuer_file" <<<"$(cfg)"
cp "$root/deploy/config/staging/ingestor.toml" "$tmp/deploy/config/staging/ingestor.toml"
sed -i '/^\[listen_tls\]/,/^\[/ s#client_issuer_file = "/run/secrets/service_issuer_certificate"#client_issuer_file = "/run/secrets/placement_issuer_certificate"#' "$tmp/deploy/config/staging/runtime-1.descriptor.toml"
check "a runtime listener that admits the control plane through the placement issuer is refused" grep -q "listen_tls. must set client_issuer_file = service_issuer_certificate" <<<"$(cfg)"
rm -rf "$tmp"

echo "$failures failure(s)"
[ "$failures" -eq 0 ]
