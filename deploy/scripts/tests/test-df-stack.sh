#!/usr/bin/env bash
# Exercises the activation state machine (df-deploy activate / rollback and df-stack) against a fake docker, in a sandbox, without root,
# Docker or a cloud. No flock is needed: a stand-in is put first on PATH. STATUS: run on the authoring machine; the REAL Docker/compose
# behaviour behind the fake is unverified.
#   deploy/scripts/tests/test-df-stack.sh
set -uo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
here=$root/deploy/scripts/tests
failures=0
check() { # check <description> <command...>
  local what=$1; shift
  if "$@"; then echo "PASS $what"; else echo "FAIL $what"; failures=$((failures + 1)); fi
}

new_sandbox() {
  S=$(mktemp -d)
  mkdir -p "$S/bin" "$S/sbin" "$S/state" "$S/etc/dungeonflux/allowed-images.d" "$S/run/dungeonflux/docker" "$S/run/dungeonflux/secrets/deploy"
  mkdir -p "$S/opt/dungeonflux/releases"
  printf '#!/bin/sh\nexit 0\n' > "$S/bin/flock"
  chmod +x "$S/bin/flock"
  # Stand-ins for the helper tools df-deploy calls; df-stack is the REAL script driven by the fake docker.
  for tool in df-slot-dirs df-net-rules df-keys-apply; do
    printf '#!/bin/sh\necho "%s $*" >> "%s/helpers.log"\nexit 0\n' "$tool" "$S/state" > "$S/sbin/$tool"
    chmod +x "$S/sbin/$tool"
  done
  cat > "$S/sbin/df-stack" <<EOF
#!/bin/bash
export DF_STACK_DOCKER="$here/fake-docker.sh" DF_STACK_SLEEP=true FAKE_STATE="$S/state" DF_STACK_CONF="$S/rollout.conf" DF_STACK_STATE="$S/state/stack"
exec bash "$root/infra/cloud-init/df-stack.sh" "\$@"
EOF
  chmod +x "$S/sbin/df-stack"
  printf 'canary_seconds=5\nready_checks=2\n' > "$S/rollout.conf"
  printf 'psql=postgres:18@sha256:%064d\n' 9 > "$S/etc/dungeonflux/allowed-images.d/allowed"
}

make_release() { # make_release NAME RUNTIME_DIGEST_NUMBER
  local name=$1 n=$2 dir=$S/opt/dungeonflux/releases/$1
  mkdir -p "$dir/config"
  echo 'services: {}' > "$dir/production.compose.yml"; echo 'x=1' > "$dir/production.env"; echo 'x=1' > "$dir/release.lock"
  printf 'DF_DOCKER_GID=999\nDF_DB_HOST=db.example.test\nDF_DB_PORT=25060\n' > "$dir/host.env"
  printf '[[slots]]\nid=1\n' > "$dir/config/runner.toml"
  echo 'same forwarder config' > "$dir/config/dbgate.cfg"
  printf 'control-plane=ghcr.io/o/dungeonflux-control-plane:1@sha256:%064d\ncampaign-runtime=ghcr.io/o/dungeonflux-campaign-runtime:1@sha256:%064d\n' "$n" "$n" > "$dir/allowed-images"
}

# The live allow-list as an activation of release NAME would have left it: its own list plus the host's psql line.
live_is() {
  { cat "$S/opt/dungeonflux/releases/$1/allowed-images"; printf 'psql=postgres:18@sha256:%064d\n' 9; } > "$S/etc/dungeonflux/allowed-images.d/allowed"
}

deploy() { # deploy <command...>   (runs df-deploy in the sandbox with the given environment)
  env PATH="$S/bin:$PATH" DF_DEPLOY_TEST_PREFIX="$S" DF_DEPLOY_TEST_SBIN="$S/sbin" FAKE_STATE="$S/state" \
    bash "$root/infra/cloud-init/df-deploy.sh" "$@" 2>"$S/err.log" >"$S/out.log"
  local rc=$?
  if [ -n "${DF_TEST_VERBOSE:-}" ]; then echo "--- df-deploy $* -> $rc"; cat "$S/err.log" "$S/out.log"; fi
  return $rc
}
link() { cat "$S/opt/dungeonflux/$1" 2>/dev/null | sed "s,.*/,,"; }
ptr() { printf "%s" "$1" > "$2"; }
last_up_release() { grep '^up ' "$S/state/order.log" | tail -n 1 | awk '{print $2}'; }
digest_of() { printf 'sha256:%064d' "$1"; }
runtime_lines() { grep -c '^campaign-runtime=' "$S/etc/dungeonflux/allowed-images.d/allowed"; }

# --- 1. a good activation ----------------------------------------------------------------------------------------------------------
new_sandbox
B=$(printf '%064d' 2)
make_release aaa 1; make_release "$B" 2
ptr "$S/opt/dungeonflux/releases/aaa" "$S/opt/dungeonflux/current"
ptr "$S/opt/dungeonflux/releases/$B" "$S/opt/dungeonflux/staged"
live_is aaa
deploy activate "$(digest_of 2)"; rc=$?
check "a good activation exits 0" test "$rc" -eq 0
check "current is the new release" test "$(link current)" = "$B"
check "the old release is recorded as previous" test "$(link previous)" = aaa
check "the live allow-list is the new release's own list: the old runtime digest is NOT carried over" test "$(runtime_lines)" -eq 1
check "and it is exactly that list plus the psql line" bash -c 'cmp -s <(cat "$1/opt/dungeonflux/releases/$2/allowed-images"; printf "psql=postgres:18@sha256:%064d\n" 9) "$1/etc/dungeonflux/allowed-images.d/allowed"' _ "$S" "$B"
check "the helpers ran before the stack started" test "$(head -n 1 "$S/state/helpers.log" | cut -d' ' -f1)" = df-slot-dirs
order=$(awk '{print $3}' "$S/state/order.log" | tr '\n' ' ')
check "services started in dependency order, the control plane before the edge" \
  bash -c '[[ "$1" == "db-gate ingestor docker-proxy egress runner control-plane edge prometheus alertmanager host-textfile "* ]]' _ "$order"
check "every service is running the new release" test "$(last_up_release)" = "$B"
rm -rf "$S"

# --- 2. the new release fails its readiness gate: automatic rollback ---------------------------------------------------------
new_sandbox
make_release aaa 1; B=$(printf '%064d' 2); make_release "$B" 2
ptr "$S/opt/dungeonflux/releases/aaa" "$S/opt/dungeonflux/current"
ptr "$S/opt/dungeonflux/releases/$B" "$S/opt/dungeonflux/staged"
live_is aaa; cp "$S/etc/dungeonflux/allowed-images.d/allowed" "$S/allowed.before"
FAKE_FAIL_RELEASES=$B deploy activate "$(digest_of 2)"; rc=$?
check "a failed readiness gate exits 8 (rolled back)" test "$rc" -eq 8
check "current is back on the old release" test "$(link current)" = aaa
check "no previous pointer was left behind" test -z "$(link previous)"
check "the allow-list was restored byte for byte" cmp -s "$S/allowed.before" "$S/etc/dungeonflux/allowed-images.d/allowed"
check "the old release was started again" test "$(last_up_release)" = aaa
rm -rf "$S"

# --- 3. the rollback fails too -------------------------------------------------------------------------------------------------
new_sandbox
make_release aaa 1; B=$(printf '%064d' 2); make_release "$B" 2
ptr "$S/opt/dungeonflux/releases/aaa" "$S/opt/dungeonflux/current"
ptr "$S/opt/dungeonflux/releases/$B" "$S/opt/dungeonflux/staged"
FAKE_FAIL_RELEASES="$B,aaa" deploy activate "$(digest_of 2)"; rc=$?
check "when the rollback also fails the exit is 9" test "$rc" -eq 9
rm -rf "$S"

# --- 4. the first activation fails: nothing to roll back to ----------------------------------------------------------------------
new_sandbox
B=$(printf '%064d' 2); make_release "$B" 2
ptr "$S/opt/dungeonflux/releases/$B" "$S/opt/dungeonflux/staged"
FAKE_FAIL_RELEASES=$B deploy activate "$(digest_of 2)"; rc=$?
check "a failed first activation exits 10 (stopped, nothing current), not 8" test "$rc" -eq 10
check "...and leaves only the psql client in the allow-list" bash -c 'test "$(grep -c "^campaign-runtime=" "$1/etc/dungeonflux/allowed-images.d/allowed")" -eq 0' _ "$S"
check "no release is current" test ! -e "$S/opt/dungeonflux/current" -a ! -L "$S/opt/dungeonflux/current"
check "the stack was stopped" grep -q '^down ' "$S/state/order.log"
rm -rf "$S"

# --- 5. an unhealthy `up --wait` is a failure too; a crash loop during the soak is a failure -------------------------------------------
new_sandbox
make_release aaa 1; B=$(printf '%064d' 2); make_release "$B" 2
ptr "$S/opt/dungeonflux/releases/aaa" "$S/opt/dungeonflux/current"
ptr "$S/opt/dungeonflux/releases/$B" "$S/opt/dungeonflux/staged"
FAKE_UNHEALTHY_RELEASES=$B deploy activate "$(digest_of 2)"; rc=$?
check "a service that does not become healthy exits 8" test "$rc" -eq 8
check "and the old release is current again" test "$(link current)" = aaa
rm -rf "$S"
new_sandbox
make_release aaa 1; B=$(printf '%064d' 2); make_release "$B" 2
ptr "$S/opt/dungeonflux/releases/aaa" "$S/opt/dungeonflux/current"
ptr "$S/opt/dungeonflux/releases/$B" "$S/opt/dungeonflux/staged"
FAKE_RESTART_SERVICE=egress deploy activate "$(digest_of 2)"; rc=$?
check "a service that restarts during the soak is caught (exit 8 or 9, never 0)" bash -c '[ "$1" -eq 8 ] || [ "$1" -eq 9 ]' _ "$rc"
rm -rf "$S"

# --- 6. a missing image stops everything before any service starts -------------------------------------------------------------------
new_sandbox
make_release aaa 1; B=$(printf '%064d' 2); make_release "$B" 2
ptr "$S/opt/dungeonflux/releases/aaa" "$S/opt/dungeonflux/current"
ptr "$S/opt/dungeonflux/releases/$B" "$S/opt/dungeonflux/staged"
FAKE_MISSING_IMAGE="example/image-$B@sha256:$(printf '%064d' 1)" deploy activate "$(digest_of 2)"; rc=$?
check "a missing image is a failed activation that rolls back" test "$rc" -eq 8
check "no service of the new release was started" bash -c '! grep -q "^up $1 " "$2/order.log" 2>/dev/null' _ "$B" "$S/state"
rm -rf "$S"

# --- 7. re-activating the current release keeps the rollback target; a manual rollback swaps the pointers ---------------------------------
new_sandbox
make_release aaa 1; B=$(printf '%064d' 2); make_release "$B" 2
ptr "$S/opt/dungeonflux/releases/aaa" "$S/opt/dungeonflux/current"
ptr "$S/opt/dungeonflux/releases/$B" "$S/opt/dungeonflux/staged"
deploy activate "$(digest_of 2)"
deploy activate "$(digest_of 2)"; rc=$?
check "re-activating the current release works" test "$rc" -eq 0
check "and does not overwrite previous with itself" test "$(link previous)" = aaa
deploy rollback; rc=$?
check "a manual rollback succeeds" test "$rc" -eq 0
check "it makes the old release current" test "$(link current)" = aaa
check "and records the other one as previous" test "$(link previous)" = "$B"
rm -rf "$S"

# --- 8. only the staged release can be activated -------------------------------------------------------------------------------------
new_sandbox
make_release aaa 1; B=$(printf '%064d' 2); make_release "$B" 2
ptr "$S/opt/dungeonflux/releases/aaa" "$S/opt/dungeonflux/current"
deploy activate "$(digest_of 2)"; rc=$?
check "an unstaged release is refused before anything changes" test "$rc" -eq 4
check "current did not move" test "$(link current)" = aaa
rm -rf "$S"

# --- 9. REVOCATION: nothing is carried over, a revoked runtime cannot come back by rollback, a release naming one is refused ------------
new_sandbox
make_release aaa 1; B=$(printf '%064d' 2); make_release "$B" 2
ptr "$S/opt/dungeonflux/releases/aaa" "$S/opt/dungeonflux/current"
ptr "$S/opt/dungeonflux/releases/$B" "$S/opt/dungeonflux/staged"
live_is aaa
printf '# the runtime digest of release aaa is bad\nsha256:%064d\n' 1 > "$S/opt/dungeonflux/releases/$B/revoked-images"
deploy activate "$(digest_of 2)"; rc=$?
check "a release that revokes the running runtime still activates (its own list is clean)" test "$rc" -eq 0
check "the host recorded the revocation" grep -q "$(printf 'sha256:%064d' 1)" "$S/etc/dungeonflux/revoked-images"
deploy rollback; rc=$?
check "a manual rollback onto a revoked runtime is refused" test "$rc" -eq 4
check "and current did not move" test "$(link current)" = "$B"
check "the live allow-list never listed the revoked digest" bash -c '! grep -q "$2" "$1/etc/dungeonflux/allowed-images.d/allowed"' _ "$S" "$(printf 'sha256:%064d' 1)"
rm -rf "$S"
new_sandbox
make_release aaa 1; B=$(printf '%064d' 2); make_release "$B" 2
ptr "$S/opt/dungeonflux/releases/aaa" "$S/opt/dungeonflux/current"
ptr "$S/opt/dungeonflux/releases/$B" "$S/opt/dungeonflux/staged"
live_is aaa; cp "$S/etc/dungeonflux/allowed-images.d/allowed" "$S/allowed.before"
printf 'sha256:%064d\n' 2 > "$S/etc/dungeonflux/revoked-images"
deploy activate "$(digest_of 2)"; rc=$?
check "a release whose own list names a revoked image is refused before anything changes" test "$rc" -eq 4
check "current did not move and the allow-list is untouched" bash -c '[ "$(cat "$1/opt/dungeonflux/current" | sed "s,.*/,,")" = aaa ] && cmp -s "$2" "$1/etc/dungeonflux/allowed-images.d/allowed"' _ "$S" "$S/allowed.before"
rm -rf "$S"
new_sandbox
make_release aaa 1; B=$(printf '%064d' 2); make_release "$B" 2
ptr "$S/opt/dungeonflux/releases/aaa" "$S/opt/dungeonflux/current"
ptr "$S/opt/dungeonflux/releases/$B" "$S/opt/dungeonflux/staged"
live_is aaa
printf 'sha256:%064d\n' 1 > "$S/opt/dungeonflux/releases/$B/revoked-images"
FAKE_FAIL_RELEASES=$B deploy activate "$(digest_of 2)"; rc=$?
check "a failed activation whose revocation covers the running runtime cannot roll back onto it (exit 9)" test "$rc" -eq 9
check "...the failed stack was stopped" grep -q '^down ' "$S/state/order.log"
check "...no release is current" test ! -e "$S/opt/dungeonflux/current"
check "...and the allow-list holds no runtime image, only the psql client" bash -c 'test "$(grep -c "^campaign-runtime=" "$1/etc/dungeonflux/allowed-images.d/allowed")" -eq 0 && grep -q "^psql=" "$1/etc/dungeonflux/allowed-images.d/allowed"' _ "$S"
rm -rf "$S"
new_sandbox
make_release aaa 1; B=$(printf '%064d' 2); make_release "$B" 2
ptr "$S/opt/dungeonflux/releases/aaa" "$S/opt/dungeonflux/current"
ptr "$S/opt/dungeonflux/releases/$B" "$S/opt/dungeonflux/staged"
live_is aaa
printf 'not-a-digest
' > "$S/opt/dungeonflux/releases/$B/revoked-images"
deploy activate "$(digest_of 2)"; rc=$?
check "a malformed revocation line is refused (exit 4)" test "$rc" -eq 4
rm -rf "$S"

# --- 10. the database forwarder is recreated only when its configuration changed --------------------------------------------------------
new_sandbox
make_release aaa 1; B=$(printf '%064d' 2); make_release "$B" 2
ptr "$S/opt/dungeonflux/releases/aaa" "$S/opt/dungeonflux/current"
ptr "$S/opt/dungeonflux/releases/$B" "$S/opt/dungeonflux/staged"
deploy activate "$(digest_of 2)"; rc=$?
check "the first start of the forwarder is a forced recreate" bash -c 'grep "^docker compose.* up .* db-gate" "$1/calls.log" | head -n 1 | grep -q -- --force-recreate' _ "$S/state"
C=$(printf '%064d' 3); make_release "$C" 3
printf '%s' "$S/opt/dungeonflux/releases/$C" > "$S/opt/dungeonflux/staged"
deploy activate "$(digest_of 3)"; rc=$?
check "a second release with the SAME forwarder config does not force-recreate it" bash -c 'grep "^docker compose.* up .* db-gate" "$1/calls.log" | tail -n 1 | grep -vq -- --force-recreate' _ "$S/state"
echo 'a different forwarder config' > "$S/opt/dungeonflux/releases/$C/config/dbgate.cfg"
D=$(printf '%064d' 4); make_release "$D" 4; echo 'a different forwarder config' > "$S/opt/dungeonflux/releases/$D/config/dbgate.cfg"
printf '%s' "$S/opt/dungeonflux/releases/$D" > "$S/opt/dungeonflux/staged"
deploy activate "$(digest_of 4)"; rc=$?
check "a release with a CHANGED forwarder config force-recreates it" bash -c 'grep "^docker compose.* up .* db-gate" "$1/calls.log" | tail -n 1 | grep -q -- --force-recreate' _ "$S/state"
rm -rf "$S"

# --- 11. a failed RETRY of the current release is reported as a failed retry, not as a rollback ----------------------------------------------
new_sandbox
A=$(printf '%064d' 1); make_release "$A" 1; B=$(printf '%064d' 2); make_release "$B" 2
ptr "$S/opt/dungeonflux/releases/$A" "$S/opt/dungeonflux/current"
ptr "$S/opt/dungeonflux/releases/$B" "$S/opt/dungeonflux/previous"
ptr "$S/opt/dungeonflux/releases/$A" "$S/opt/dungeonflux/staged"
live_is "$A"
FAKE_FAIL_RELEASES=$A deploy activate "$(digest_of 1)"; rc=$?
check "a failed retry of the current release exits 11" test "$rc" -eq 11
check "...says it was not a rollback" grep -q "not a rollback" "$S/err.log"
check "...leaves current and previous exactly as they were" bash -c 'test "$(sed "s,.*/,," "$1/opt/dungeonflux/current")" = "$2" && test "$(sed "s,.*/,," "$1/opt/dungeonflux/previous")" = "$3"' _ "$S" "$A" "$B"
rm -rf "$S"

# --- 12. everything is validated before any pointer moves: a refused release leaves previous alone too -----------------------------------------
new_sandbox
make_release aaa 1; B=$(printf '%064d' 2); make_release "$B" 2
ptr "$S/opt/dungeonflux/releases/aaa" "$S/opt/dungeonflux/current"
ptr "$S/opt/dungeonflux/releases/$B" "$S/opt/dungeonflux/staged"
live_is aaa
printf 'sha256:%064d\n' 2 > "$S/etc/dungeonflux/revoked-images"
deploy activate "$(digest_of 2)"; rc=$?
check "a release refused for a revoked image moves no pointer, previous included" bash -c 'test "$1" -eq 4 && test ! -e "$2/opt/dungeonflux/previous" && test "$(sed "s,.*/,," "$2/opt/dungeonflux/current")" = aaa' _ "$rc" "$S"
rm -rf "$S"

echo "$failures failure(s)"
[ "$failures" -eq 0 ]
