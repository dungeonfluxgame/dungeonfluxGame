#!/usr/bin/env bash
# Tests of two security rules that live in scripts: (1) df-net-rules generates the INPUT and FORWARD rules it must, and fails CLOSED;
# (2) check-config-policy.sh requires the service-issuer pin on every client that verifies a service server. Needs bash only.
#   deploy/scripts/tests/test-net-and-pins.sh
set -uo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
failures=0
check() { local what=$1; shift; if "$@"; then echo "PASS $what"; else echo "FAIL $what"; failures=$((failures + 1)); fi; }
rules() { bash "$root/infra/cloud-init/df-net-rules.sh" --dry-run --vpc-cidr 10.30.0.0/16 "$@" 2>/dev/null; }

# --- (1) df-net-rules ---------------------------------------------------------------------------------------------------------------------------
resolv=${TMPDIR:-/tmp}/df-net-resolv.conf
printf 'search example.test\nnameserver 67.207.67.2\nnameserver 127.0.0.53\nnameserver 67.207.67.3\n' > "$resolv"
out=$(rules --slots 3 --resolv-conf "$resolv")
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
# the database bridge: DNS to the host's real upstream resolvers, the managed ports inside the VPC, nothing else; NO fixed database address
check "dfdb0: DNS (udp and tcp 53) to each upstream resolver" bash -c 'for r in 67.207.67.2 67.207.67.3; do for p in udp tcp; do grep -qx -- "-A DF-EGRESS -i dfdb0 -p $p -d $r/32 --dport 53 -j ACCEPT" <<<"$1" || exit 1; done; done' _ "$out"
check "dfdb0: a loopback stub resolver is not allowed" bash -c '! grep -q 127.0.0.53 <<<"$1"' _ "$out"
check "dfdb0: tcp 25060-25061 into the VPC CIDR" grep -qx -- "-A DF-EGRESS -i dfdb0 -p tcp -d 10.30.0.0/16 --dport 25060:25061 -j ACCEPT" <<<"$out"
check "dfdb0: everything else is dropped, and that DROP is the last rule for it" bash -c 'grep -- "-i dfdb0" <<<"$1" | grep -v DF-INPUT | tail -n 1 | grep -qx -- "-A DF-EGRESS -i dfdb0 -j DROP"' _ "$out"
check "the only ACCEPTs are DNS to the resolvers and the database ports (a failover changes no rule)" bash -c 'test "$(grep -c -- "-j ACCEPT" <<<"$1")" -eq 5' _ "$out"
check "no resolver found: the database bridge gets no DNS but the drops are still installed" bash -c 'out=$(bash "$1" --dry-run --vpc-cidr 10.30.0.0/16 --resolv-conf /dev/null 2>/dev/null); grep -qx -- "-A DF-EGRESS -i dfdb0 -j DROP" <<<"$out" && ! grep -q -- "--dport 53" <<<"$out"' _ "$root/infra/cloud-init/df-net-rules.sh"
printf 'DF_DOCKER_GID=1\nDF_SLOTS=4\n' > "${TMPDIR:-/tmp}/df-net-test.env"
check "DF_SLOTS of the release's host.env sets the number of slot bridges" grep -qx -- "-A DF-INPUT -i dfslot4 -m addrtype --dst-type LOCAL -j DROP" <<<"$(rules --from "${TMPDIR:-/tmp}/df-net-test.env" --resolv-conf "$resolv")"
rm -f "${TMPDIR:-/tmp}/df-net-test.env" "$resolv"
check "a missing VPC CIDR is refused (the rules would not protect it)" bash -c '! bash "$1" --dry-run >/dev/null 2>&1' _ "$root/infra/cloud-init/df-net-rules.sh"
check "a malformed slot count is refused" bash -c '! bash "$1" --dry-run --vpc-cidr 10.30.0.0/16 --slots 0 >/dev/null 2>&1' _ "$root/infra/cloud-init/df-net-rules.sh"

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
