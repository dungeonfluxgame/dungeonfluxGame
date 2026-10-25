#!/usr/bin/env bash
# Static policy check of the RENDERED staging compose file (LAUNCH.md decisions 18, 23; critic rounds 1 and 1c).
# Needs a docker client (no daemon) and jq. STATUS: the jq filters are UNVERIFIED (jq was not installed on the
# authoring machine). The same assertions were implemented a second time in a throwaway node script over the
# rendered JSON (`docker compose config --format json`); it passes on the real file and reports 10 violations on
# a deliberately corrupted copy (privileged, SYS_ADMIN, host network, devices, published port, duplicated and
# 65532 uids, a non-internal network, a runtime on two networks), so the policy itself is checked.
# Exit 1 on any violation.
#
# Asserts: every network but `public` is internal; EVERY service mounts exactly the one secrets directory the map allows it (egress ->
# dispatcher, edge -> edge, ...) and no other; exact membership of every network (the edge is on public and
# ingress only; each slot network has exactly {control-plane, db-gate, egress, ingestor, its runtime}; the dispatcher
# network has {control-plane, db-gate, egress, ingestor}; only egress is on egress-out); each runtime is on one
# slot network and mounts only its own secrets directory; no service mounts more than one secrets directory;
# every service but bucket-init has a read-only root, drops ALL capabilities and sets no-new-privileges; NO
# privileged container, no host network/pid/ipc/userns/cgroup, no devices, no sysctls, no unconfined profile;
# cap_add only from a per-service allowlist (postgres, edge); NO published port except the edge on 127.0.0.1;
# no Docker socket and no bind mount outside the repository; every service except postgres and the object store
# has an explicit numeric uid:gid, unique, never 0 or 65532, runtime uids in the slot range (>= 20000) and every
# other service below it; a runtime's secrets directory (if generated) holds no foreign credential.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
command -v docker >/dev/null 2>&1 || { echo "missing tool: docker" >&2; exit 1; }
command -v jq >/dev/null 2>&1 || { echo "missing tool: jq" >&2; exit 1; }
failures=0
fail() { echo "FAIL $*"; failures=$((failures + 1)); }
pass() { echo "PASS $*"; }

rendered=$(docker compose \
  --project-directory "$root/deploy/compose" \
  --file "$root/deploy/compose/staging.compose.yml" \
  --env-file "$root/deploy/images/images.lock" \
  --env-file "$root/deploy/compose/staging.env" \
  --profile runtime config --format json)
q() { printf '%s' "$rendered" | jq -r "$@"; }
check_empty() { # check_empty <description> <output of a violation query>
  if [ -z "$2" ]; then pass "$1"; else fail "$1: $2"; fi
}

check_empty "every network except public is internal" \
  "$(q '.networks | to_entries[] | select(.key != "public" and (.value.internal != true)) | .key')"

members() { q --arg n "$1" '[.services | to_entries[] | select(.value.networks | has($n)) | .key] | sort | join(",")'; }
expect_members() {
  local got
  got=$(members "$1")
  if [ "$got" = "$2" ]; then pass "network $1 = {$got}"; else fail "network $1 has {$got}, expected {$2}"; fi
}
expect_members public "edge"
expect_members ingress "control-plane,edge"
expect_members data "bucket-init,control-plane,db-gate,ingestor,migrate,object-store,postgres"
expect_members dispatch "control-plane,db-gate,egress,ingestor"
expect_members slot1 "control-plane,db-gate,egress,ingestor,runtime-1"
expect_members slot2 "control-plane,db-gate,egress,ingestor,runtime-2"
expect_members egress-out "egress"

for runtime in runtime-1 runtime-2; do
  nets=$(q --arg s "$runtime" '.services[$s].networks | keys | join(",")')
  case "$nets" in slot[0-9]) pass "$runtime is on exactly one slot network ($nets)" ;; *) fail "$runtime networks: $nets" ;; esac
done

# The EXACT map from service to the one secrets directory it may mount (critic round 1d, item 6): any other directory, a
# second one, or a directory for a service that should have none is a violation, for every service and not only runtimes.
check_empty "every service mounts exactly its own secrets directory and no other" \
  "$(q '{"control-plane":"control-plane","egress":"dispatcher","ingestor":"ingestor","edge":"edge","migrate":"migrate","postgres":"postgres","object-store":"object-store","bucket-init":"bucket-init","runtime-1":"runtime-1","runtime-2":"runtime-2"} as $map
        | .services | to_entries[] | . as $s
        | ([$s.value.volumes[]? | select(.target == "/run/secrets") | (.source | gsub("\\\\"; "/") | split("/") | last)]) as $got
        | ($map[$s.key]) as $want
        | select((($want == null) and ($got | length > 0)) or (($want != null) and ($got != [$want])))
        | $s.key + " mounts " + ($got | tostring) + " (expected " + ($want // "none" | tostring) + ")"')"
check_empty "every service but bucket-init has a read-only root and drops all capabilities" \
  "$(q '.services | to_entries[] | select(.key != "bucket-init") | select((.value.read_only != true) or ((.value.cap_drop // []) | index("ALL") | not)) | .key')"
check_empty "every service sets no-new-privileges" \
  "$(q '.services | to_entries[] | select(((.value.security_opt // []) | index("no-new-privileges:true")) | not) | .key')"
check_empty "no privileged container" \
  "$(q '.services | to_entries[] | select(.value.privileged == true) | .key')"
check_empty "no host network, pid, ipc, userns or cgroup namespace" \
  "$(q '.services | to_entries[] | select([.value.network_mode, .value.pid, .value.ipc, .value.userns_mode, .value.cgroup] | map(select(. == "host" or ((. // "") | startswith("container:")))) | length > 0) | .key')"
check_empty "no devices, no sysctls, no unconfined security profile" \
  "$(q '.services | to_entries[] | select(((.value.devices // []) | length > 0) or ((.value.sysctls // {}) | length > 0) or ([.value.security_opt[]? | select(test("unconfined"))] | length > 0)) | .key')"
check_empty "cap_add only from the per-service allowlist (postgres: CHOWN DAC_OVERRIDE FOWNER SETGID SETUID; edge: NET_BIND_SERVICE)" \
  "$(q '{"postgres": ["CHOWN","DAC_OVERRIDE","FOWNER","SETGID","SETUID"], "edge": ["NET_BIND_SERVICE"]} as $ok
        | .services | to_entries[] | . as $s | ($s.value.cap_add // [])[] | select(. as $c | (($ok[$s.key] // []) | index($c)) | not) | $s.key + ":" + .')"
check_empty "no published port except the edge, and the edge only on 127.0.0.1" \
  "$(q '.services | to_entries[] | . as $s | ($s.value.ports // [])[] | select($s.key != "edge" or .host_ip != "127.0.0.1") | $s.key + ":" + (.published // "")')"
check_empty "no Docker socket mounted" \
  "$(q '.services | to_entries[] | select([.value.volumes[]?.source] | any(test("docker\\.sock"))) | .key')"
check_empty "no bind mount outside the repository" \
  "$(q --arg root "$root" '.services | to_entries[] | . as $s | ($s.value.volumes // [])[] | select(.type == "bind") | select((.source | gsub("\\\\"; "/")) | startswith($root + "/") | not) | $s.key + ":" + .source')"

# Identity: an explicit numeric uid per service.
check_empty "every service except postgres and the object store has an explicit numeric uid:gid" \
  "$(q '.services | to_entries[] | select(.key != "postgres" and .key != "object-store") | select(((.value.user // "") | test("^[0-9]+:[0-9]+$")) | not) | .key')"
check_empty "no service runs as uid 0 or 65532" \
  "$(q '.services | to_entries[] | select((.value.user // "") | test("^(0|65532):")) | .key')"
check_empty "every uid is unique" \
  "$(q '[.services | to_entries[] | select(.value.user != null) | {k: .key, u: (.value.user | split(":")[0])}] | group_by(.u) | map(select(length > 1) | map(.k) | join("+")) | .[]')"
check_empty "runtime uids are in the slot range (>= 20000); every other uid is below it" \
  "$(q '.services | to_entries[] | select(.value.user != null) | (.value.user | split(":")[0] | tonumber) as $u | select((.key | startswith("runtime-")) != ($u >= 20000)) | .key')"

secrets_dir="$root/artifacts/tmp/staging/secrets"
if [ -d "$secrets_dir/runtime-1" ]; then
  for runtime in runtime-1 runtime-2; do
    bad=$(ls "$secrets_dir/$runtime" | grep -E 'session|storage|provider|payments|workos|email|journal|media_gateway|dispatch_tls|dispatcher_client|internal_tls_control|ingestor_tls|objstore|pg_server|db_control|db_egress|db_admin|db_migrator|db_runner|db_billing|db_support' || true)
    check_empty "$runtime secrets directory holds no foreign credential" "$bad"
  done
else
  echo "SKIP secrets directory content (run deploy/scripts/staging-secrets.sh first)"
fi

if [ "$failures" -ne 0 ]; then echo "$failures policy violation(s)"; exit 1; fi
echo "compose policy ok"
