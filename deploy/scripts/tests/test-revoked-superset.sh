#!/usr/bin/env bash
# Tests of deploy/scripts/check-revoked-superset.sh. Needs bash only.
set -uo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
t=$(mktemp -d)
failures=0
check() { local what=$1; shift; if "$@"; then echo "PASS $what"; else echo "FAIL $what"; failures=$((failures + 1)); fi; }
d() { printf 'sha256:%064d\n' "$1"; }
run() { bash "$root/deploy/scripts/check-revoked-superset.sh" "$@" >"$t/out" 2>&1; }
{ echo '# old'; d 1; d 2; } > "$t/prev"
{ echo '# new'; d 1; d 2; d 3; } > "$t/grown"
{ echo '# new'; d 1; d 3; } > "$t/dropped"
: > "$t/empty"
check "a list that keeps every old digest and adds one passes" run "$t/prev" "$t/grown"
check "an identical list passes" run "$t/prev" "$t/prev"
check "dropping an old digest fails" bash -c '! bash "$1" "$2" "$3" >/dev/null 2>&1' _ "$root/deploy/scripts/check-revoked-superset.sh" "$t/prev" "$t/dropped"
check "...and names the dropped digest" bash -c 'bash "$1" "$2" "$3" 2>&1 | grep -q "$4"' _ "$root/deploy/scripts/check-revoked-superset.sh" "$t/prev" "$t/dropped" "$(d 2)"
check "an empty new list fails against a non-empty previous one" bash -c '! bash "$1" "$2" "$3" >/dev/null 2>&1' _ "$root/deploy/scripts/check-revoked-superset.sh" "$t/prev" "$t/empty"
check "an empty previous list lets anything through" run "$t/empty" "$t/grown"
printf 'not-a-digest\n' > "$t/bad"
check "a malformed line in the new list fails" bash -c '! bash "$1" "$2" "$3" >/dev/null 2>&1' _ "$root/deploy/scripts/check-revoked-superset.sh" "$t/empty" "$t/bad"
rm -rf "$t"
echo "$failures failure(s)"
[ "$failures" -eq 0 ]
