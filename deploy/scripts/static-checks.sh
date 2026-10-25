#!/usr/bin/env bash
# The static checks of everything under deploy/ and infra/, as ONE entry point for CI and for a developer (round 3). Nothing here starts a
# service, contacts a cloud or needs a secret; the only network use is `terraform init` downloading providers (skipped with --no-network).
#
#   deploy/scripts/static-checks.sh [--no-network] [--skip-slow]
#   DF_REQUIRE_TOOLS=jq,shellcheck,terraform deploy/scripts/static-checks.sh     (a missing tool listed here is a FAILURE, not a skip)
#
# Each step prints PASS, FAIL or SKIP (a tool that is not installed). In CI list the tools you installed in DF_REQUIRE_TOOLS so that a
# broken runner image cannot turn the whole job into skips. Exit 1 if any step failed. STATUS: the steps that need jq, shellcheck,
# terraform, promtool, amtool and docker were never run on the authoring machine (none is installed there).
set -uo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"
network=1; slow=1
for arg in "$@"; do
  case "$arg" in
    --no-network) network=0 ;;
    --skip-slow) slow=0 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done
required=",${DF_REQUIRE_TOOLS:-},"
failures=0
passes=0
skips=0

have() { command -v "$1" >/dev/null 2>&1; }
need_tool() { # need_tool TOOL: 0 if present; otherwise records a SKIP (or a FAIL when required) and returns 1
  if have "$1"; then return 0; fi
  case "$required" in
    *",$1,"*) echo "FAIL $2: required tool $1 is not installed"; failures=$((failures + 1)) ;;
    *) echo "SKIP $2 (needs $1)"; skips=$((skips + 1)) ;;
  esac
  return 1
}
step() { # step NAME COMMAND...
  local name=$1; shift
  local out
  if out=$("$@" 2>&1); then echo "PASS $name"; passes=$((passes + 1)); else echo "FAIL $name"; printf '%s\n' "$out" | tail -n 25 | sed 's/^/     /'; failures=$((failures + 1)); fi
}

mapfile -t scripts < <(find deploy infra -type f -name '*.sh' -not -path '*/target/*' | sort)

# --- shell ---------------------------------------------------------------------------------------------------------------------------
syntax() { local f; for f in "${scripts[@]}"; do bash -n "$f" || { echo "syntax error in $f"; return 1; }; done; }
step "bash -n of every shell script (${#scripts[@]})" syntax
if need_tool shellcheck "shellcheck"; then
  # Warnings and errors; the scripts use bash features on purpose, so the shell dialect is declared.
  step "shellcheck (severity warning)" shellcheck --shell=bash --severity="${DF_SHELLCHECK_SEVERITY:-warning}" "${scripts[@]}"
fi

# --- policy checks that need only bash/grep/sed -------------------------------------------------------------------------------------------
step "config policy (TOML, templates, issuer pins)" bash deploy/scripts/check-config-policy.sh
step "DNS record list" bash deploy/scripts/check-dns-records.sh
step "staging qualification checklist" bash deploy/scripts/check-qualification.sh
step "edge log redaction" bash deploy/scripts/check-edge-redaction.sh
step "secrets: no secret-looking content in deploy/ and infra/" bash deploy/scripts/secrets-audit.sh repo

# --- manifest and compose policy: jq scripts and their node twins ----------------------------------------------------------------------------
if need_tool jq "secrets manifest (jq)"; then step "secrets manifest invariants (jq)" bash deploy/scripts/secrets-audit.sh manifest; fi
if need_tool node "manifest twin"; then step "secrets manifest invariants (node twin)" node deploy/scripts/twins/twin-manifest.js; fi
if have docker && docker compose version >/dev/null 2>&1; then
  if have jq; then
    step "staging compose policy (jq)" bash deploy/scripts/check-compose-policy.sh
    step "production compose policy (jq)" bash deploy/scripts/check-production-policy.sh
  else
    need_tool jq "compose policy (jq)" || true
  fi
  if have node; then
    step "staging compose policy (node twin)" node deploy/scripts/twins/twin-staging-policy.js
    step "production compose policy (node twin, with mutation tests)" node deploy/scripts/twins/twin-production-policy.js
  fi
else
  need_tool docker "compose policy checks (needs the docker client with the compose plugin; no daemon)" || true
fi

# --- tests of the tools themselves, against fakes -------------------------------------------------------------------------------------------
step "df-backup against a fake docker" bash deploy/scripts/tests/test-df-backup.sh
step "revocation list superset rule" bash deploy/scripts/tests/test-revoked-superset.sh
step "egress firewall rules and issuer pins" bash deploy/scripts/tests/test-net-and-pins.sh
step "database-only DNS stub (forwards the database suffix, refuses every other name)" bash deploy/scripts/tests/test-db-dns.sh
if [ "$slow" -eq 1 ]; then step "activation state machine against a fake docker" bash deploy/scripts/tests/test-df-stack.sh; else echo "SKIP activation state machine (--skip-slow)"; skips=$((skips + 1)); fi
if need_tool openssl "SCRAM verifier and PKI tests"; then
  step "SCRAM verifier derivation" bash deploy/db/test-scram-verifier.sh
  step "staging PKI (pathlen, EKU, name constraints)" bash deploy/scripts/tests/test-pki.sh
fi

# --- the Docker proxy crate ---------------------------------------------------------------------------------------------------------------------
if need_tool cargo "docker-proxy crate"; then
  step "docker-proxy rustfmt" bash -c 'cd deploy/docker-proxy && cargo fmt --check'
  step "docker-proxy tests (--locked)" bash -c 'cd deploy/docker-proxy && cargo test --locked'
  step "docker-proxy clippy (warnings are errors)" bash -c 'cd deploy/docker-proxy && cargo clippy --locked --all-targets -- -D warnings'
fi

# --- monitoring ------------------------------------------------------------------------------------------------------------------------------------
if need_tool promtool "prometheus config and alert rule checks"; then
  step "prometheus.yml" promtool check config deploy/config/production/prometheus.yml
  step "alert rules" promtool check rules deploy/config/production/alert-rules.yml
  step "alert rule unit tests" promtool test rules deploy/monitoring/alert-rules.test.yml
fi
if need_tool amtool "alertmanager config check"; then step "alertmanager.yml" amtool check-config deploy/config/production/alertmanager.yml; fi

# --- terraform ----------------------------------------------------------------------------------------------------------------------------------------
if need_tool terraform "terraform fmt and validate"; then
  step "terraform fmt -check -recursive" terraform -chdir=infra/terraform fmt -check -recursive
  if [ "$network" -eq 1 ]; then
    for env in staging production; do
      step "terraform validate ($env)" bash -c "cd infra/terraform/envs/$env && terraform init -backend=false -input=false >/dev/null && terraform validate"
    done
  else
    echo "SKIP terraform validate (--no-network)"; skips=$((skips + 1))
  fi
fi

echo "---"
echo "$passes passed, $failures failed, $skips skipped"
[ "$failures" -eq 0 ]
