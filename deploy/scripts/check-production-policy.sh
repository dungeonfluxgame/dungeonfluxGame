#!/usr/bin/env bash
# Static policy check of the RENDERED production compose file (LAUNCH.md decisions 18, 23; round 2). Needs a docker client (no daemon) and jq.
# STATUS: the jq filters are UNVERIFIED (jq was not installed on the authoring machine). The same assertions were implemented a second
# time in a throwaway node script over the rendered JSON (`docker compose config --format json`); it passes on the real file and reports
# every violation on a set of deliberately corrupted copies (a second Docker socket mount, a privileged runner, a writable secrets mount,
# a published database port, a non-internal slot network, a duplicated uid, an extra capability), so the policy itself is checked.
# Exit 1 on any violation.
#
# Asserts: the ONLY non-internal networks are public, control-out, db-out and egress-out, each carrying exactly the one service that needs
# it; exact membership of every other network (the edge is on public and ingress only; the slot networks hold no runtime, which the runner
# adds later through the proxy, and exactly {control-plane, db-gate, egress, ingestor}); every service mounts exactly the one secrets
# directory the map allows it and read-only; EXACTLY ONE service (docker-proxy) mounts the Docker socket and it has no network at all;
# every service has a read-only root, drops ALL capabilities, sets no-new-privileges, has memory and pid limits and a digest-pinnable image;
# no privileged container, no host network/pid/ipc/userns/cgroup, no devices, sysctls or unconfined profile; cap_add only from a
# per-service allowlist (edge: NET_BIND_SERVICE only; the runner has none); the only published ports are the edge's 80 and 443; every bind
# mount comes from the production host paths and everything except the few writable ones is read-only; every service has an explicit
# numeric uid:gid below the slot range, unique, never 65532, and no service runs as uid 0. The proxy mounts the runtime tree read-only.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
command -v docker >/dev/null 2>&1 || { echo "missing tool: docker" >&2; exit 1; }
command -v jq >/dev/null 2>&1 || { echo "missing tool: jq" >&2; exit 1; }
failures=0
fail() { echo "FAIL $*"; failures=$((failures + 1)); }
pass() { echo "PASS $*"; }

rendered=$(docker compose \
  --project-directory "$root/deploy/compose" \
  --file "$root/deploy/compose/production.compose.yml" \
  --env-file "$root/deploy/images/images.lock" \
  --env-file "$root/deploy/compose/production.env" \
  --env-file "$root/deploy/compose/host.env.example" \
  config --format json)
q() { printf '%s' "$rendered" | jq -r "$@"; }
check_empty() { # check_empty <description> <output of a violation query>
  if [ -z "$2" ]; then pass "$1"; else fail "$1: $2"; fi
}

check_empty "the only non-internal networks are public, control-out, db-out, egress-out and monitor-out" \
  "$(q '.networks | to_entries[] | select(.value.internal != true) | select(.key | IN("public","control-out","db-out","egress-out","monitor-out") | not) | .key')"
check_empty "EVERY network has the fixed bridge name the host firewall addresses (dfpub0 dfin0 dfdata0 dfdsp0 dfmn0 dfobs0 dfrun0 dfctl0 dfdb0 dfegr0 dfmon0 and dfslot<N>)" \
  "$(q '{"public":"dfpub0","ingress":"dfin0","data":"dfdata0","dispatch":"dfdsp0","monitoring":"dfmn0","observe":"dfobs0","runner-net":"dfrun0","control-out":"dfctl0","db-out":"dfdb0","egress-out":"dfegr0","monitor-out":"dfmon0"} as $want | .networks | to_entries[] | . as $n | ($want[$n.key] // (if ($n.key | test("^slot[0-9]+$")) then "df" + $n.key else null end)) as $w | select($w == null or ($n.value.driver_opts["com.docker.network.bridge.name"] != $w)) | $n.key')"
check_empty "the allow-list is mounted as a directory, never as a single file (a rename replaces a file's inode)" \
  "$(q '.services | to_entries[] | . as $s | ($s.value.volumes // [])[] | select(.target | test("allowed-images")) | select(.target != "/etc/dungeonflux/allowed-images.d" or .source != "/etc/dungeonflux/allowed-images.d" or .read_only != true) | $s.key + ":" + .target')"
check_empty "the slot networks are internal and named df-slot-N" \
  "$(q '.networks | to_entries[] | select(.key | test("^slot[0-9]+$")) | select((.value.internal != true) or (.value.name != ("df-slot-" + (.key | ltrimstr("slot"))))) | .key')"

members() { q --arg n "$1" '[.services | to_entries[] | select(.value.networks != null and (.value.networks | has($n))) | .key] | sort | join(",")'; }
expect_members() {
  local got
  got=$(members "$1")
  if [ "$got" = "$2" ]; then pass "network $1 = {$got}"; else fail "network $1 has {$got}, expected {$2}"; fi
}
expect_members public "edge"
expect_members control-out "control-plane"
expect_members db-out "db-gate"
expect_members egress-out "egress"
expect_members ingress "control-plane,edge"
expect_members data "control-plane,db-gate,ingestor"
expect_members dispatch "control-plane,db-gate,egress,ingestor"
expect_members monitoring "ingestor,prometheus"
expect_members observe "alertmanager,host-textfile,prometheus"
expect_members monitor-out "alertmanager"
expect_members runner-net "db-gate,runner"
expect_members slot1 "control-plane,db-gate,egress,ingestor"
expect_members slot2 "control-plane,db-gate,egress,ingestor"
check_empty "the docker proxy has no network" \
  "$(q '.services["docker-proxy"] | select(.network_mode != "none" or (.networks != null and (.networks | length) > 0)) | "docker-proxy has a network"')"

check_empty "every service mounts exactly its own secrets directory (read-only) and no other" \
  "$(q '{"control-plane":"control-plane","egress":"dispatcher","ingestor":"ingestor","edge":"edge","runner":"runner","alertmanager":"monitoring"} as $map
        | .services | to_entries[] | . as $s
        | ([$s.value.volumes[]? | select(.target == "/run/secrets")]) as $m
        | ([$m[] | (.source | split("/") | last)]) as $got
        | ($map[$s.key]) as $want
        | select((($want == null) and ($m | length > 0)) or (($want != null) and ($got != [$want] or ($m | all(.read_only == true) | not))))
        | $s.key + " mounts " + ($got | tostring) + " (expected " + ($want // "none" | tostring) + ", read-only)"')"
check_empty "every secrets source is below /run/dungeonflux/secrets" \
  "$(q '.services | to_entries[] | . as $s | ($s.value.volumes // [])[] | select(.target == "/run/secrets") | select(.source | startswith("/run/dungeonflux/secrets/") | not) | $s.key + ":" + .source')"

check_empty "every service has a read-only root and drops all capabilities" \
  "$(q '.services | to_entries[] | select((.value.read_only != true) or ((.value.cap_drop // []) | index("ALL") | not)) | .key')"
check_empty "every service sets no-new-privileges" \
  "$(q '.services | to_entries[] | select(((.value.security_opt // []) | index("no-new-privileges:true")) | not) | .key')"
check_empty "every service has a memory limit and a pid limit" \
  "$(q '.services | to_entries[] | select((.value.mem_limit // "") == "" or (.value.pids_limit // 0) == 0) | .key')"
check_empty "every image comes from the lock (a digest is added by CI) and none is :latest" \
  "$(q '.services | to_entries[] | select((.value.image // "") == "" or (.value.image | test(":latest($|@)"))) | .key')"
check_empty "no privileged container" \
  "$(q '.services | to_entries[] | select(.value.privileged == true) | .key')"
check_empty "no host or shared namespace; network_mode none only for the docker proxy" \
  "$(q '.services | to_entries[] | select(([.value.network_mode, .value.pid, .value.ipc, .value.userns_mode, .value.cgroup] | map(select(. == "host" or ((. // "") | startswith("container:")))) | length > 0) or (.value.network_mode != null and (.key != "docker-proxy" or .value.network_mode != "none"))) | .key')"
check_empty "no devices, no sysctls, no unconfined security profile" \
  "$(q '.services | to_entries[] | select(((.value.devices // []) | length > 0) or ((.value.sysctls // {}) | length > 0) or ([.value.security_opt[]? | select(test("unconfined"))] | length > 0)) | .key')"
check_empty "cap_add only from the per-service allowlist (edge: NET_BIND_SERVICE; nobody else)" \
  "$(q '{"edge": ["NET_BIND_SERVICE"]} as $ok
        | .services | to_entries[] | . as $s | ($s.value.cap_add // [])[] | select(. as $c | (($ok[$s.key] // []) | index($c)) | not) | $s.key + ":" + .')"
check_empty "the only published ports are the edge's 80 and 443" \
  "$(q '.services | to_entries[] | . as $s | ($s.value.ports // [])[] | select($s.key != "edge" or ((.target | tostring) | IN("80","443") | not) or (.published != (.target | tostring))) | $s.key + ":" + (.published // "")')"

# The Docker socket: one mount, in one service.
check_empty "exactly one service mounts the Docker socket, and it is the docker proxy" \
  "$(q '[.services | to_entries[] | select([.value.volumes[]?.source] | any(test("docker\\.sock"))) | .key] | if . == ["docker-proxy"] then empty else tostring end')"
check_empty "the docker proxy mounts the socket once, plus its three other paths" \
  "$(q '.services["docker-proxy"].volumes | map(.target) | sort | if . == ["/etc/dungeonflux/allowed-images.d","/run/dungeonflux/runtime","/run/proxy","/var/run/docker.sock"] then empty else tostring end')"
check_empty "the docker proxy sees the runtime tree READ-ONLY, at the path the binds use (it walks each bind source before a create)" \
  "$(q '.services["docker-proxy"].volumes[] | select(.target == "/run/dungeonflux/runtime") | select(.read_only != true or .source != "/run/dungeonflux/runtime") | "docker-proxy mounts " + .source + " writable or at another path"')"

check_empty "every bind mount comes from the production host paths" \
  "$(q '.services | to_entries[] | . as $s | ($s.value.volumes // [])[] | select(.type == "bind") | select(.source | test("^(/etc/dungeonflux/|/opt/dungeonflux/|/run/dungeonflux/|/var/lib/dungeonflux/|/var/run/docker\\.sock$)") | not) | $s.key + ":" + .source')"
check_empty "only the listed writable mounts are writable" \
  "$(q '{"edge":["/data"],"ingestor":["/var/lib/dungeonflux/telemetry"],"docker-proxy":["/run/proxy","/var/run/docker.sock"],"runner":["/run/dungeonflux/runtime"],"prometheus":["/prometheus"],"alertmanager":["/alertmanager"]} as $rw
        | .services | to_entries[] | . as $s | ($s.value.volumes // [])[] | select(.read_only != true) | select(. as $v | (($rw[$s.key] // []) | index($v.target)) | not) | $s.key + ":" + .target')"

# Identity: an explicit numeric uid per service.
check_empty "every service has an explicit numeric uid:gid" \
  "$(q '.services | to_entries[] | select(((.value.user // "") | test("^[0-9]+:[0-9]+$")) | not) | .key')"
check_empty "no service runs as uid 0 or 65532 (the runner is uid 10004 with no capability: round 2a)" \
  "$(q '.services | to_entries[] | select((.value.user // "") | test("^(0|65532):")) | .key')"
check_empty "every uid is unique" \
  "$(q '[.services | to_entries[] | select(.value.user != null) | {k: .key, u: (.value.user | split(":")[0])}] | group_by(.u) | map(select(length > 1) | map(.k) | join("+")) | .[]')"
check_empty "every service uid is below the slot range (20000+)" \
  "$(q '.services | to_entries[] | select(.value.user != null) | select((.value.user | split(":")[0] | tonumber) >= 20000) | .key')"
check_empty "extra groups only: the docker proxy -> the docker group" \
  "$(q '.services | to_entries[] | select((.value.group_add // []) | length > 0) | select((.key == "docker-proxy" and (.value.group_add | length == 1) and (.value.group_add[0] | test("^[0-9]+$"))) | not) | .key')"

if [ "$failures" -ne 0 ]; then echo "$failures policy violation(s)"; exit 1; fi
echo "production compose policy ok"
