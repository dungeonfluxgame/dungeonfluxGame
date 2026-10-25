#!/usr/bin/env bash
# df-tools-test-matrix.sh: run the df-tools lib test suite as per-module shards and report pass,
# fail or crash for every module (master launch plan 12.6: a reliable test matrix).
#
# Why: one `cargo test -p df-tools --lib` process aborts on a stack overflow (and any abort kills
# every test still queued behind it), so the suite cannot be a CI gate as a single run. This script
# lists the tests, runs one shard per module in its own process with a large RUST_MIN_STACK, and
# when a shard crashes or times out it reruns that shard's tests one process each to name the
# exact test that crashed. A shard that crashes but whose every test then passes alone is an
# UNREPRODUCED_CRASH (an interaction between tests, a deadlock, an order dependence) and fails the
# run: it is never reported as passing. Tests on the known-failures list are tolerated and named,
# never hidden, and a tolerated failure must still fail for its recorded cause.
#
# Scope: the lib unit tests of df-tools only. The integration targets under crates/df-tools/tests
# are not run here (one of them does not compile on the baseline, TODO-GATE-01); until they are
# covered, the CI job that runs them keeps its own step. This runner is the single source of truth
# for the lib suite: the CI job must call it instead of keeping its own `--skip` list (request to
# A8 in artifacts/tmp/agents/S4/request.md).
#
# It needs only bash, coreutils (timeout, grep, sed, awk, sort) and the compiled test executable.
# It does not deploy, touch a database, start a server or make a network call.
#
# Exit status: 0 = nothing unexpected; 1 = an unexpected failure, a failure with a different cause
# than recorded, a crash, an unreproduced crash, a timeout or a count mismatch (and, with
# --strict-known, a stale, mislabelled or missing known entry); 2 = usage or setup.
#
# Usage:
#   development/df-tools-test-matrix.sh [options]
#     --exe PATH            df-tools lib test executable (default: build one with cargo, copy it)
#     --out DIR             output directory (default artifacts/test-matrix/df-tools-<timestamp>)
#     --known FILE          known-failures list (default development/df-tools-known-failures.txt)
#     --only REGEX          run only modules whose path matches this extended regex (smoke runs)
#     --jobs N              --test-threads inside each shard (default 4)
#     --stack BYTES         RUST_MIN_STACK for every run (default 33554432, 32 MiB)
#     --shard-timeout SEC   limit for one shard (default 1800)
#     --test-timeout SEC    limit for one isolated test (default 300)
#     --no-isolate          do not rerun a crashed shard test by test
#     --probe-known         also run every known-failures test alone at the default stack and at
#                           --stack, and report entries that are stale or mislabelled
#     --strict-known        fail on a stale, mislabelled or missing known entry; runs the `crash`
#                           entries alone (implies --probe-known for them)
#     --list-modules        print the shard plan and exit
#     --self-test           check this script against a stub executable and exit
#
# Known-failures list format, one entry per line, `|` separated, `#` starts a comment:
#   KIND | TEST | OWNER | PANIC_TEXT | NOTE
#   crash  a test that crashes or hangs at --stack. It is skipped in every shard so it cannot take
#          a shard down; PANIC_TEXT is `-`. --strict-known runs it alone and fails if it no longer
#          crashes (then it is a `fail` or stale).
#   fail   a test that fails without crashing. It still runs. PANIC_TEXT is required: a substring
#          of the test's panic summary, so the test is tolerated only for the cause the owner
#          recorded. The summary is one line, `path:line:col: message ~ left: x ~ right: y`: the
#          `panicked at` location, its message and the assertion lines after it, never the test's
#          other output (path separators are `/`). Record the whole summary from the file name on.
#          A failure with another cause or at another line is unexpected (DIFFERENT_CAUSE).
# TEST is a full test name. A module prefix is rejected: it would hide whole modules.
#
# Timeouts: every process runs under coreutils `timeout --kill-after=15`. On Linux this kills the
# test process. On Windows under Git Bash the signal may not reach a native child that ignores it,
# so a hung native test can outlive the shard timeout; the CI job-level timeout is the backstop
# there. The self-test proves the classification and the kill with a hanging stub process.
#
# CI use: one step, `bash development/df-tools-test-matrix.sh --strict-known`. It builds the lib
# test executable itself. A full run is about 25 minutes of test time on the 2026-10-24 baseline
# (adventure and npc_dossier shards dominate). `--only` makes a quick subset; `--probe-known` is
# for the owner who is clearing the known-failures list.
#
# Outputs in --out: shards.tsv, matrix.txt (the per-module table), passed.txt, failed.txt,
# crashed.txt, unexpected.txt, stale-known.txt, reclassify.txt, probe.txt (with probing),
# failure-text/ (the output of every failed test), failure-panic/ (its panic summary), and logs/.

set -u
set -o pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO=$(cd "$SCRIPT_DIR/.." && pwd)

EXE=""
OUT=""
KNOWN="$SCRIPT_DIR/df-tools-known-failures.txt"
ONLY=""
JOBS=4
STACK=33554432
SHARD_TIMEOUT=1800
TEST_TIMEOUT=300
ISOLATE=1
PROBE=0
PROBE_KINDS=all
STRICT_KNOWN=0
LIST_MODULES=0
SELF_TEST=0

die() {
  echo "df-tools-test-matrix: $*" >&2
  exit 2
}

usage() {
  sed -n '2,/^set -u/p' "${BASH_SOURCE[0]}" | sed '$d' | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
  case $1 in
    --exe) EXE=${2:?--exe needs a path}; shift 2 ;;
    --out) OUT=${2:?--out needs a directory}; shift 2 ;;
    --known) KNOWN=${2:?--known needs a file}; shift 2 ;;
    --only) ONLY=${2:?--only needs a regex}; shift 2 ;;
    --jobs) JOBS=${2:?--jobs needs a number}; shift 2 ;;
    --stack) STACK=${2:?--stack needs a byte count}; shift 2 ;;
    --shard-timeout) SHARD_TIMEOUT=${2:?needs seconds}; shift 2 ;;
    --test-timeout) TEST_TIMEOUT=${2:?needs seconds}; shift 2 ;;
    --no-isolate) ISOLATE=0; shift ;;
    --probe-known) PROBE=1; shift ;;
    --strict-known) STRICT_KNOWN=1; shift ;;
    --list-modules) LIST_MODULES=1; shift ;;
    --self-test) SELF_TEST=1; shift ;;
    -h | --help) usage; exit 0 ;;
    *) die "unknown option: $1 (try --help)" ;;
  esac
done

# run_limited STACK SECONDS LOG ARGS...: one test-executable process, output to LOG, its exit code
# returned. An empty STACK runs at the platform default stack.
run_limited() {
  local stack=$1 seconds=$2 log=$3
  shift 3
  if [ -n "$stack" ]; then
    RUST_MIN_STACK=$stack timeout --kill-after=15 "$seconds" "$EXE" "$@" >"$log" 2>&1
  else
    env -u RUST_MIN_STACK timeout --kill-after=15 "$seconds" "$EXE" "$@" >"$log" 2>&1
  fi
}

# classify EXIT LOG: PASS, FAIL (tests failed, harness finished), CRASH (no harness summary or an
# abnormal exit) or TIMEOUT.
classify() {
  local code=$1 log=$2
  if [ "$code" = 124 ] || [ "$code" = 137 ]; then
    echo TIMEOUT
  elif ! grep -q '^test result:' "$log"; then
    echo CRASH
  elif [ "$code" = 0 ]; then
    echo PASS
  elif [ "$code" = 101 ]; then
    echo FAIL
  else
    echo CRASH
  fi
}

# sanitize TEXT: TEXT as a file-name-safe token.
sanitize() { printf '%s' "$1" | tr -c 'A-Za-z0-9_' '_'; }

# trim TEXT: TEXT without leading and trailing whitespace.
trim() {
  local value=$1
  value=${value#"${value%%[![:space:]]*}"}
  value=${value%"${value##*[![:space:]]}"}
  printf '%s' "$value"
}

# count_field LOG WORD: the number before WORD in the last `test result:` line, 0 if absent.
count_field() {
  local value
  value=$(grep '^test result:' "$1" | tail -1 | sed -nE "s/.* ([0-9]+) $2[;.].*/\1/p")
  echo "${value:-0}"
}

# panic_summary NAME LOG: the first panic of test NAME in a libtest log, on one line:
# `path:line:col: first message line ~ left: x ~ right: y`. Only the `panicked at` location, its
# message line and the `left:` / `right:` assertion lines after it are kept; the test's other
# output (log lines, prompts, anything it printed) can never satisfy a recorded panic text.
panic_summary() {
  tr -d '\r' <"$2" | awk -v header="---- $1 stdout ----" '
    $0 == header { on = 1; next }
    on && (/^---- .* stdout ----$/ || /^failures:$/) { on = 0 }
    !on { next }
    state == 0 && match($0, /panicked at /) {
      location = substr($0, RSTART + RLENGTH); sub(/:$/, "", location); gsub(/\\/, "/", location)
      state = 1; next
    }
    state == 1 { message = $0; state = 2; next }
    state == 2 && /^ +(left|right):/ { line = $0; sub(/^ +/, "", line); message = message " ~ " line; next }
    state == 2 { state = 3 }
    END { if (state >= 1) print location ": " message }'
}

# save_failure_text LOG: for each failed test, failure-text/NAME.txt (its whole libtest block, for
# reading) and failure-panic/NAME.txt (its panic summary, which is what known entries match).
save_failure_text() {
  local name
  mkdir -p "$OUT/failure-text" "$OUT/failure-panic"
  tr -d '\r' <"$1" | sed -nE 's/^test (.*) \.\.\. FAILED$/\1/p' | while read -r name; do
    tr -d '\r' <"$1" | awk -v header="---- $name stdout ----" '
      $0 == header { on = 1; next }
      on && (/^---- .* stdout ----$/ || /^failures:$/) { on = 0 }
      on { print }' >"$OUT/failure-text/$(sanitize "$name").txt"
    panic_summary "$name" "$1" >"$OUT/failure-panic/$(sanitize "$name").txt"
  done
}

# collect LOG: append the per-test lines of a libtest log to passed, failed and ignored.
collect() {
  tr -d '\r' <"$1" | sed -nE 's/^test (.*) \.\.\. ok$/\1/p' >>"$OUT/passed.txt"
  tr -d '\r' <"$1" | sed -nE 's/^test (.*) \.\.\. FAILED$/\1/p' >>"$OUT/failed.txt"
  tr -d '\r' <"$1" | sed -nE 's/^test (.*) \.\.\. ignored.*$/\1/p' >>"$OUT/ignored.txt"
  save_failure_text "$1"
}

# load_known: fills known_crash.txt and known_fail.txt (exact test names) and known_owner.tsv
# (name, kind, owner, panic text).
load_known() {
  : >"$OUT/known_crash.txt"
  : >"$OUT/known_fail.txt"
  : >"$OUT/known_owner.tsv"
  [ -f "$KNOWN" ] || die "known-failures file not found: $KNOWN"
  local kind name owner needle note
  while IFS='|' read -r kind name owner needle note; do
    kind=$(trim "$kind")
    case $kind in '' | '#'*) continue ;; esac
    name=$(trim "$name")
    owner=$(trim "$owner")
    needle=$(trim "$needle")
    case $kind in crash | fail) ;; *) die "bad kind '$kind' in $KNOWN" ;; esac
    [ -n "$name" ] && [ -n "$owner" ] || die "entry needs a test and an owner in $KNOWN: $kind | $name"
    case $name in
      *::) die "module prefix entries are rejected (they hide whole modules): $name" ;;
    esac
    if [ "$kind" = fail ] && { [ -z "$needle" ] || [ "$needle" = - ]; }; then
      die "a fail entry needs the panic text its failure must contain: $name"
    fi
    if ! grep -qxF -- "$name" "$OUT/all-tests.txt"; then
      printf '%s\t%s\t%s\tnot in this build\n' "$kind" "$name" "$owner" >>"$OUT/known_missing.tsv"
      continue
    fi
    printf '%s\n' "$name" >>"$OUT/known_$kind.txt"
    printf '%s\t%s\t%s\t%s\n' "$name" "$kind" "$owner" "$needle" >>"$OUT/known_owner.tsv"
  done <"$KNOWN"
  sort -u -o "$OUT/known_crash.txt" "$OUT/known_crash.txt"
  sort -u -o "$OUT/known_fail.txt" "$OUT/known_fail.txt"
}

# sorted_unique FILE...: sorted unique lines of the files.
sorted_unique() { cat "$@" | sort -u; }

build_executable() {
  command -v cargo >/dev/null || die "cargo not found and --exe not given"
  local built
  built=$(cd "$REPO" && cargo test -p df-tools --lib --no-run 2>&1 |
    tee "$OUT/build.log" | sed -nE 's/^ *Executable unittests .* \((.*)\)$/\1/p' | tail -1)
  [ -n "$built" ] || die "could not find the test executable in $OUT/build.log"
  case $built in /* | [A-Za-z]:*) ;; *) built="$REPO/$built" ;; esac
  local extension=""
  case $built in *.exe) extension=.exe ;; esac
  EXE="$OUT/df-tools-tests$extension"
  cp "$built" "$EXE"
}

# isolate_shard EXPECTED_FILE: run each test of a crashed shard in its own process; returns 0 when
# at least one of them crashed or timed out alone, 1 when none did (an unreproduced crash).
isolate_shard() {
  local test_name log code status result=1
  while read -r test_name; do
    [ -n "$test_name" ] || continue
    log="$OUT/logs/isolated-$(sanitize "$test_name").log"
    run_limited "$STACK" "$TEST_TIMEOUT" "$log" --test-threads 1 --exact "$test_name"
    code=$?
    status=$(classify "$code" "$log")
    case $status in
      PASS | FAIL) collect "$log" ;;
      *)
        printf '%s\t%s\n' "$test_name" "$status" >>"$OUT/crashed.txt"
        result=0
        ;;
    esac
  done <"$1"
  return $result
}

run_matrix() {
  mkdir -p "$OUT/logs"
  : >"$OUT/passed.txt"
  : >"$OUT/failed.txt"
  : >"$OUT/ignored.txt"
  : >"$OUT/crashed.txt"
  : >"$OUT/unreproduced.txt"
  : >"$OUT/shards.tsv"
  : >"$OUT/known_missing.tsv"
  : >"$OUT/reclassify.txt"
  : >"$OUT/stale-probe.txt"

  [ -n "$EXE" ] || build_executable
  [ -x "$EXE" ] || die "test executable not found or not executable: $EXE"

  "$EXE" --list 2>/dev/null | tr -d '\r' | sed -nE 's/^(.*): test$/\1/p' | sort -u \
    >"$OUT/all-tests.txt"
  local total
  total=$(wc -l <"$OUT/all-tests.txt" | tr -d ' ')
  [ "$total" -gt 0 ] || die "the executable lists no tests: $EXE"
  load_known

  # Crash-kind entries are skipped in every shard so one cannot take its shard down.
  local skip_args=()
  local name
  while read -r name; do
    [ -n "$name" ] && skip_args+=(--skip "$name")
  done <"$OUT/known_crash.txt"

  # Shard plan: a module is a test's path without its last segment; tests with no module are
  # their own shard. libtest filters match substrings, so module M runs with filter `M::` and
  # skips every other module whose path contains `M::` (its descendants, and look-alikes such as
  # `x::M`), which are shards of their own. A count check after each shard catches any overlap.
  grep -vxFf "$OUT/known_crash.txt" "$OUT/all-tests.txt" |
    awk '{ n = $0; if (sub(/::[^:]*$/, "", n)) print n; else print "(root) " $0 }' |
    sort -u >"$OUT/modules.txt" || true
  sed -E 's/::[^:]*$//' "$OUT/all-tests.txt" | sort -u >"$OUT/all-modules.txt"
  if [ -n "$ONLY" ]; then
    grep -E -- "$ONLY" "$OUT/modules.txt" >"$OUT/modules.sel" || true
    mv "$OUT/modules.sel" "$OUT/modules.txt"
  fi
  local shard_count
  shard_count=$(wc -l <"$OUT/modules.txt" | tr -d ' ')
  echo "tests listed: $total; known crash (skipped): $(wc -l <"$OUT/known_crash.txt" | tr -d ' ');" \
    "known fail: $(wc -l <"$OUT/known_fail.txt" | tr -d ' '); shards: $shard_count"
  if [ "$LIST_MODULES" = 1 ]; then
    cat "$OUT/modules.txt"
    return 0
  fi

  local index=0 module log code status expected passed failed ignored seconds started
  while read -r module; do
    index=$((index + 1))
    local expected_tests="$OUT/expected.tmp"
    local shard_args=()
    case $module in
      "(root) "*)
        name=${module#"(root) "}
        printf '%s\n' "$name" >"$expected_tests"
        shard_args=(--exact "$name")
        ;;
      *)
        awk -v m="$module" '{ n = $0; sub(/::[^:]*$/, "", n); if (n == m) print $0 }' \
          "$OUT/all-tests.txt" | grep -vxFf "$OUT/known_crash.txt" >"$expected_tests" || true
        shard_args=("$module::")
        local other
        while read -r other; do
          shard_args+=(--skip "$other::")
        done < <(awk -v m="$module::" -v self="$module" '$0 != self && index($0 "::", m) > 0' \
          "$OUT/all-modules.txt")
        ;;
    esac
    expected=$(wc -l <"$expected_tests" | tr -d ' ')
    log="$OUT/logs/$(sanitize "$module").log"
    started=$SECONDS
    run_limited "$STACK" "$SHARD_TIMEOUT" "$log" --test-threads "$JOBS" \
      "${shard_args[@]}" ${skip_args[@]+"${skip_args[@]}"}
    code=$?
    status=$(classify "$code" "$log")
    passed=$(count_field "$log" passed)
    failed=$(count_field "$log" failed)
    ignored=$(count_field "$log" ignored)
    collect "$log"

    if [ "$status" = CRASH ] || [ "$status" = TIMEOUT ]; then
      if [ "$ISOLATE" = 1 ]; then
        if isolate_shard "$expected_tests"; then
          status="$status+isolated"
        else
          printf '%s\n' "$module" >>"$OUT/unreproduced.txt"
          status="$status+UNREPRODUCED"
        fi
      else
        sed 's/$/\tshard-'"$status"'/' "$expected_tests" >>"$OUT/crashed.txt"
      fi
    elif [ "$((passed + failed + ignored))" != "$expected" ]; then
      status="$status+COUNT_MISMATCH(ran $((passed + failed + ignored)) expected $expected)"
    fi
    seconds=$((SECONDS - started))
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$status" "$module" "$expected" "$passed" "$failed" \
      "$ignored" "$seconds" >>"$OUT/shards.tsv"
    printf '[%3d/%d] %-34s %-40s passed=%s failed=%s ignored=%s %ss\n' "$index" "$shard_count" \
      "$status" "$module" "$passed" "$failed" "$ignored" "$seconds"
  done <"$OUT/modules.txt"
  rm -f "$OUT/expected.tmp"
}

# probe_known: run known-failures tests alone at the default stack and at the matrix stack (the
# stack CI uses), restricted to PROBE_KINDS (`all` or `crash`).
probe_known() {
  : >"$OUT/probe.txt"
  : >"$OUT/stale-probe.txt"
  : >"$OUT/reclassify.txt"
  local test_name log default_status matrix_status kind note
  while IFS=$'\t' read -r test_name kind _owner _needle; do
    [ "$PROBE_KINDS" = all ] || [ "$PROBE_KINDS" = "$kind" ] || continue
    log="$OUT/logs/probe-$(sanitize "$test_name")"
    run_limited "" "$TEST_TIMEOUT" "$log.default.log" --test-threads 1 --exact "$test_name"
    default_status=$(classify $? "$log.default.log")
    run_limited "$STACK" "$TEST_TIMEOUT" "$log.stack.log" --test-threads 1 --exact "$test_name"
    matrix_status=$(classify $? "$log.stack.log")
    [ "$matrix_status" = FAIL ] && save_failure_text "$log.stack.log"
    note=""
    if [ "$kind" = fail ] && { [ "$default_status" = CRASH ] || [ "$default_status" = TIMEOUT ]; } &&
      [ "$matrix_status" = FAIL ]; then
      note="	crashes at the default stack only"
    fi
    printf '%s\t%s\tdefault-stack=%s\tmatrix-stack=%s%s\n' "$kind" "$test_name" "$default_status" \
      "$matrix_status" "$note" >>"$OUT/probe.txt"
    if [ "$default_status" = PASS ] && [ "$matrix_status" = PASS ]; then
      echo "$test_name" >>"$OUT/stale-probe.txt"
    fi
    case "$kind:$matrix_status" in
      crash:FAIL | crash:PASS)
        printf '%s\tlisted crash, but at the matrix stack it does not crash: make it a fail entry or remove it\n' \
          "$test_name" >>"$OUT/reclassify.txt" ;;
      fail:CRASH | fail:TIMEOUT)
        printf '%s\tlisted fail, but it crashes or hangs at the matrix stack: make it a crash entry\n' \
          "$test_name" >>"$OUT/reclassify.txt" ;;
    esac
  done <"$OUT/known_owner.tsv"
}

# find_different_causes: failed tests on the list whose output lacks the recorded panic text.
find_different_causes() {
  : >"$OUT/different-cause.txt"
  local name kind _owner needle
  while IFS=$'\t' read -r name kind _owner needle; do
    [ "$kind" = fail ] || continue
    grep -qxF -- "$name" "$OUT/failed.txt" || continue
    if ! grep -qF -- "$needle" "$OUT/failure-panic/$(sanitize "$name").txt" 2>/dev/null; then
      printf '%s\t%s\n' "$name" "$needle" >>"$OUT/different-cause.txt"
    fi
  done <"$OUT/known_owner.tsv"
}

report() {
  sort -u -o "$OUT/passed.txt" "$OUT/passed.txt"
  sort -u -o "$OUT/failed.txt" "$OUT/failed.txt"
  sort -u -o "$OUT/ignored.txt" "$OUT/ignored.txt"
  cut -f1 "$OUT/crashed.txt" | sort -u >"$OUT/crashed.names"

  # Unexpected: failed and not on the list as fail; failed for another cause than recorded; any
  # crash (a listed crash is skipped, so a crash seen here is new); an unreproduced shard crash.
  grep -vxFf "$OUT/known_fail.txt" "$OUT/failed.txt" >"$OUT/unexpected-failed.txt" || true
  find_different_causes
  # Known fail entries that ran and passed are stale.
  grep -xFf "$OUT/known_fail.txt" "$OUT/passed.txt" >"$OUT/stale-known.txt" || true
  [ -s "$OUT/stale-probe.txt" ] && cat "$OUT/stale-probe.txt" >>"$OUT/stale-known.txt"
  sort -u -o "$OUT/stale-known.txt" "$OUT/stale-known.txt"
  {
    sed 's/^/FAILED   /' "$OUT/unexpected-failed.txt"
    awk -F'\t' '{print "DIFFERENT_CAUSE  " $1 " (expected output containing: " $2 ")"}' \
      "$OUT/different-cause.txt"
    sed 's/^/CRASHED  /' "$OUT/crashed.names"
    sed 's/^/UNREPRODUCED_CRASH  /' "$OUT/unreproduced.txt"
    awk -F'\t' '$1 ~ /COUNT_MISMATCH/ {print "COUNT     " $2 " " $1}' "$OUT/shards.tsv"
    if [ "$STRICT_KNOWN" = 1 ]; then
      awk -F'\t' '{print "MISSING_KNOWN  " $2 " (" $1 ")"}' "$OUT/known_missing.tsv"
    fi
  } >"$OUT/unexpected.txt"

  local passed_total failed_total crashed_total known_failed_seen
  passed_total=$(wc -l <"$OUT/passed.txt" | tr -d ' ')
  failed_total=$(wc -l <"$OUT/failed.txt" | tr -d ' ')
  crashed_total=$(wc -l <"$OUT/crashed.names" | tr -d ' ')
  known_failed_seen=$(grep -cxFf "$OUT/known_fail.txt" "$OUT/failed.txt" || true)

  {
    echo "df-tools test matrix  $(date -Iseconds 2>/dev/null || date)"
    echo "executable: $EXE"
    echo "stack: RUST_MIN_STACK=$STACK  test-threads: $JOBS"
    echo
    echo "status | module | expected | passed | failed | ignored | seconds"
    sort -t "$(printf '\t')" -k1,1 -k2,2 "$OUT/shards.tsv" | tr '\t' '|'
    echo
    echo "tests passed: $passed_total; failed: $failed_total (known: $known_failed_seen);" \
      "crashed: $crashed_total"
    echo "known crash list, skipped in every shard:"
    awk -F'\t' '$2 == "crash" {print "  " $1 "  owner=" $3}' "$OUT/known_owner.tsv"
    echo "known fail list, still run (for the recorded cause only):"
    awk -F'\t' '$2 == "fail" {print "  " $1 "  owner=" $3}' "$OUT/known_owner.tsv"
    if [ -s "$OUT/known_missing.tsv" ]; then
      echo "known entries not found in this build:"
      sed 's/^/  /' "$OUT/known_missing.tsv"
    fi
    if [ -s "$OUT/stale-known.txt" ]; then
      echo "STALE known entries (they no longer fail; remove them from $KNOWN):"
      sed 's/^/  /' "$OUT/stale-known.txt"
    fi
    if [ -s "$OUT/reclassify.txt" ]; then
      echo "RECLASSIFY (probe):"
      sed 's/^/  /' "$OUT/reclassify.txt"
    fi
    if [ -s "$OUT/unexpected.txt" ]; then
      echo "UNEXPECTED:"
      sed 's/^/  /' "$OUT/unexpected.txt"
    else
      echo "UNEXPECTED: none"
    fi
  } >"$OUT/matrix.txt"
  sed -n '/^tests passed/,$p' "$OUT/matrix.txt"

  if [ -s "$OUT/unexpected.txt" ]; then return 1; fi
  if [ "$STRICT_KNOWN" = 1 ] && { [ -s "$OUT/stale-known.txt" ] || [ -s "$OUT/reclassify.txt" ]; }; then
    return 1
  fi
  return 0
}

# ---------------------------------------------------------------------------------------------
# Self-test: a stub executable that behaves like a libtest binary. a::sub::fails fails with a
# panic text; b::tests::crashes always crashes; b::tests::needs_stack crashes only at the default
# stack; the other tests pass unless an environment switch makes them misbehave:
#   STUB_CAUSE=text     a::sub::fails panics with this text instead of cause-one
#   STUB_LEFT=n         a::sub::fails reports `left: n` instead of 2
#   STUB_INTERACTION=1  i::pair::* crash only when both run in one process
#   STUB_HANG=1         h::hangs::sleeper sleeps far longer than the timeouts
#   STUB_DROP=1         m::drop::silent is listed but silently not run

write_stub() {
  cat >"$1" <<'STUB'
#!/usr/bin/env bash
tests=(a::ok_one a::sub::ok_two a::sub::fails b::tests::crashes b::tests::needs_stack
  b::tests::fine c::lonely d::deep::x::passes p::a::look_alike i::pair::first i::pair::second
  h::hangs::sleeper m::drop::silent)
mode=run; exact=0; filters=(); skips=(); args=("$@"); i=0
while [ "$i" -lt "${#args[@]}" ]; do
  case ${args[$i]} in
    --list) mode=list ;;
    --exact) exact=1 ;;
    --skip) i=$((i + 1)); skips+=("${args[$i]}") ;;
    --test-threads) i=$((i + 1)) ;;
    --*) ;;
    *) filters+=("${args[$i]}") ;;
  esac
  i=$((i + 1))
done
selected=()
for t in "${tests[@]}"; do
  keep=1
  if [ "${#filters[@]}" -gt 0 ]; then
    keep=0
    for f in "${filters[@]}"; do
      if [ "$exact" = 1 ]; then [ "$t" = "$f" ] && keep=1; else case $t in *"$f"*) keep=1 ;; esac; fi
    done
  fi
  for s in ${skips[@]+"${skips[@]}"}; do
    if [ "$exact" = 1 ]; then [ "$t" = "$s" ] && keep=0; else case $t in *"$s"*) keep=0 ;; esac; fi
  done
  [ "$keep" = 1 ] && selected+=("$t")
done
if [ "$mode" = list ]; then
  for t in "${selected[@]}"; do echo "$t: test"; done
  exit 0
fi
if [ -n "${STUB_DROP:-}" ]; then
  kept=()
  for t in ${selected[@]+"${selected[@]}"}; do [ "$t" = m::drop::silent ] || kept+=("$t"); done
  selected=(${kept[@]+"${kept[@]}"})
fi
pair_count=0
for t in ${selected[@]+"${selected[@]}"}; do case $t in i::pair::*) pair_count=$((pair_count + 1)) ;; esac; done
echo "running ${#selected[@]} tests"
passed=0; failed=0; failed_names=()
for t in ${selected[@]+"${selected[@]}"}; do
  case $t in
    *::crashes) echo "test $t ... "; exit 134 ;;
    *::needs_stack)
      if [ -z "${RUST_MIN_STACK:-}" ]; then echo "test $t ... "; exit 134; fi
      echo "test $t ... ok"; passed=$((passed + 1)) ;;
    i::pair::*)
      if [ -n "${STUB_INTERACTION:-}" ] && [ "$pair_count" -ge 2 ]; then echo "test $t ... "; exit 134; fi
      echo "test $t ... ok"; passed=$((passed + 1)) ;;
    *::sleeper)
      if [ -n "${STUB_HANG:-}" ]; then sleep 30; fi
      echo "test $t ... ok"; passed=$((passed + 1)) ;;
    *::fails) echo "test $t ... FAILED"; failed=$((failed + 1)); failed_names+=("$t") ;;
    *) echo "test $t ... ok"; passed=$((passed + 1)) ;;
  esac
done
if [ "$failed" -gt 0 ]; then
  echo; echo "failures:"
  for t in "${failed_names[@]}"; do
    echo; echo "---- $t stdout ----"
    echo "noise: cause-zzz"
    echo "thread '$t' panicked at stub.rs:1:1:"
    echo "boom: ${STUB_CAUSE:-cause-one}"
    echo "  left: ${STUB_LEFT:-2}"
    echo " right: 3"
  done
  echo; echo "failures:"
  for t in "${failed_names[@]}"; do echo "    $t"; done
  echo
  verdict=FAILED
else
  verdict=ok
fi
echo "test result: $verdict. $passed passed; $failed failed; 0 ignored; 0 measured; 0 filtered out"
exit $(( failed > 0 ? 101 : 0 ))
STUB
  chmod +x "$1"
}

self_test() {
  local work me
  work=$(mktemp -d) || die "mktemp failed"
  me="${BASH_SOURCE[0]}"
  write_stub "$work/stub-tests"
  local failures=0
  expect() { # expect DESCRIPTION EXPECTED_EXIT ACTUAL_EXIT
    if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1 (exit $3, wanted $2)"; failures=$((failures + 1)); fi
  }
  expect_in() { # expect_in DESCRIPTION FILE PATTERN
    if grep -q -- "$3" "$2"; then echo "ok   $1"; else echo "FAIL $1 (no '$3' in $2)"; failures=$((failures + 1)); fi
  }
  expect_not_in() { # expect_not_in DESCRIPTION FILE PATTERN
    if grep -q -- "$3" "$2"; then echo "FAIL $1 ('$3' found in $2)"; failures=$((failures + 1)); else echo "ok   $1"; fi
  }
  run() { # run OUTNAME KNOWNFILE [options]: the script against the stub
    local name=$1 known=$2
    shift 2
    bash "$me" --exe "$work/stub-tests" --out "$work/$name" --known "$known" "$@" >"$work/$name.log" 2>&1
  }

  cat >"$work/known-all.txt" <<'EOF'
crash | b::tests::crashes | N | - | always aborts
crash | b::tests::needs_stack | N | - | overflows at the default stack
fail | a::sub::fails | W1 | stub.rs:1:1: boom: cause-one ~ left: 2 ~ right: 3 | recorded cause
EOF
  run o1 "$work/known-all.txt"
  expect "listed crash and listed failure with its recorded cause are tolerated" 0 $?
  expect_in "the table names the known crash owner" "$work/o1/matrix.txt" "owner=N"
  expect_not_in "look-alike module paths do not overlap a shard" "$work/o1/shards.tsv" "COUNT_MISMATCH"
  expect_in "the look-alike module ran as its own shard" "$work/o1/passed.txt" "p::a::look_alike"

  STUB_CAUSE=cause-two run o1b "$work/known-all.txt"
  expect "a listed failure with another cause fails the run" 1 $?
  expect_in "the different cause is named" "$work/o1b/unexpected.txt" "DIFFERENT_CAUSE  a::sub::fails"

  STUB_LEFT=5 run o1c "$work/known-all.txt"
  expect "the same failure with a different left value is a different cause" 1 $?
  expect_in "the different assertion value is named" "$work/o1c/unexpected.txt" "DIFFERENT_CAUSE  a::sub::fails"
  expect_in "the panic summary holds the location, message and assertion lines" "$work/o1c/failure-panic/a__sub__fails.txt" "stub.rs:1:1: boom: cause-one ~ left: 5 ~ right: 3"

  printf 'fail | a::sub::fails | W1 | cause-zzz | text only in the test output\n' >"$work/known-noise.txt"
  run o1d "$work/known-noise.txt" --only '^a::'
  expect "text printed by the test cannot satisfy a recorded panic text" 1 $?
  expect_in "the noise match is a different cause" "$work/o1d/unexpected.txt" "DIFFERENT_CAUSE  a::sub::fails"

  printf 'fail | a::sub::fails | W1 | stub.rs:1:1: boom: cause-one ~ left: 2 ~ right: 3 | recorded cause\n' >"$work/known-fail-only.txt"
  run o2 "$work/known-fail-only.txt"
  expect "an unlisted crash fails the run" 1 $?
  expect_in "the crash is pinned to its exact test" "$work/o2/unexpected.txt" "CRASHED  b::tests::crashes"
  expect_in "the sibling test in the crashed shard still passed" "$work/o2/passed.txt" "b::tests::fine"
  expect_in "a stack-sensitive test passes with the matrix stack" "$work/o2/passed.txt" "b::tests::needs_stack"

  printf '# nothing known\n' >"$work/known-none.txt"
  run o3 "$work/known-none.txt"
  expect "an unlisted failure and crash fail the run" 1 $?
  expect_in "the unlisted failure is reported" "$work/o3/unexpected.txt" "FAILED   a::sub::fails"

  printf 'fail | a::ok_one | X | boom | pretend this failed once\ncrash | b::tests::crashes | N | - | always aborts\nfail | a::sub::fails | W1 | stub.rs:1:1: boom: cause-one ~ left: 2 ~ right: 3 | x\n' >"$work/known-stale.txt"
  run o4 "$work/known-stale.txt"
  expect "a stale known entry is a warning by default" 0 $?
  expect_in "the stale entry is named" "$work/o4/stale-known.txt" "a::ok_one"
  run o5 "$work/known-stale.txt" --strict-known
  expect "a stale known entry fails under --strict-known" 1 $?

  run o6 "$work/known-all.txt" --probe-known
  expect_in "the probe shows the crash only at the default stack" "$work/o6/probe.txt" "needs_stack.*default-stack=CRASH.*matrix-stack=PASS"
  expect_in "a crash entry that passes at the matrix stack is reclassified" "$work/o6/reclassify.txt" "needs_stack.*does not crash"

  printf 'crash | a::sub::fails | W1 | - | mislabelled\nfail | b::tests::crashes | N | boom | mislabelled\n' >"$work/known-wrong.txt"
  run o8 "$work/known-wrong.txt" --probe-known --only '^$' --strict-known
  expect "mislabelled entries fail under --strict-known" 1 $?
  expect_in "a crash entry that only fails is reclassified" "$work/o8/reclassify.txt" "a::sub::fails.*make it a fail entry"
  expect_in "a fail entry that crashes is reclassified" "$work/o8/reclassify.txt" "b::tests::crashes.*make it a crash entry"

  run o9 "$work/known-all.txt" --strict-known
  expect "strict mode runs crash entries alone and fails when one no longer crashes at the matrix stack" 1 $?
  expect_in "strict mode catches the crash entry that no longer crashes at the matrix stack" "$work/o9/reclassify.txt" "needs_stack"

  printf 'crash | b::tests::crashes | N | - | always aborts\nfail | a::sub::fails | W1 | stub.rs:1:1: boom: cause-one ~ left: 2 ~ right: 3 | recorded\nfail | no::such::test | W1 | x | gone\n' >"$work/known-missing.txt"
  run o10 "$work/known-missing.txt"
  expect "a missing known entry is only reported by default" 0 $?
  run o11 "$work/known-missing.txt" --strict-known
  expect "a missing known entry fails under --strict-known" 1 $?
  expect_in "the missing entry is named" "$work/o11/unexpected.txt" "MISSING_KNOWN  no::such::test"

  printf 'fail | a:: | W1 | boom | a whole module\n' >"$work/known-prefix.txt"
  run o12 "$work/known-prefix.txt"
  expect "a module prefix entry is rejected" 2 $?
  printf 'fail | a::sub::fails | W1 | | no panic text\n' >"$work/known-notext.txt"
  run o13 "$work/known-notext.txt"
  expect "a fail entry without panic text is rejected" 2 $?

  STUB_INTERACTION=1 run o14 "$work/known-none.txt" --only '^i::'
  expect "a crash that no test reproduces alone fails the run" 1 $?
  expect_in "the shard is named UNREPRODUCED_CRASH" "$work/o14/unexpected.txt" "UNREPRODUCED_CRASH  i::pair"
  expect_in "its tests still passed alone" "$work/o14/passed.txt" "i::pair::first"

  STUB_DROP=1 run o15 "$work/known-none.txt" --only '^m::'
  expect "a shard that runs fewer tests than listed fails the run" 1 $?
  expect_in "the mismatch is recorded" "$work/o15/shards.tsv" "COUNT_MISMATCH(ran 0 expected 1)"
  expect_in "the mismatch is reported as unexpected" "$work/o15/unexpected.txt" "COUNT     m::drop"

  STUB_HANG=1 run o16 "$work/known-none.txt" --only '^h::' --shard-timeout 2 --test-timeout 2
  expect "a hanging test is killed and fails the run" 1 $?
  expect_in "the shard is classified as a timeout" "$work/o16/shards.tsv" "^TIMEOUT"
  expect_in "the hanging test is named" "$work/o16/crashed.txt" "h::hangs::sleeper.TIMEOUT"

  run o7 "$work/known-all.txt" --only '^d::'
  expect "--only runs a subset" 0 $?
  expect_in "the subset ran the nested module" "$work/o7/passed.txt" "d::deep::x::passes"

  rm -rf "$work"
  if [ "$failures" = 0 ]; then echo "self-test passed"; return 0; fi
  echo "self-test: $failures check(s) failed"
  return 1
}

if [ "$SELF_TEST" = 1 ]; then
  self_test
  exit $?
fi

[ -n "$OUT" ] || OUT="$REPO/artifacts/test-matrix/df-tools-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT" || die "cannot create $OUT"
if [ "$STRICT_KNOWN" = 1 ] && [ "$PROBE" = 0 ]; then
  PROBE=1
  PROBE_KINDS=crash
fi
run_matrix || exit $?
[ "$LIST_MODULES" = 1 ] && exit 0
[ "$PROBE" = 1 ] && probe_known
report
exit $?
