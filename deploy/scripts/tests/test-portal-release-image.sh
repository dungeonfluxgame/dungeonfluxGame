#!/usr/bin/env bash
set -Eeuo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)
workflow=${DEMO_RELEASE_WORKFLOW:-"$root/.github/workflows/demo-release.yml"}
dockerfile=${PORTAL_DOCKERFILE:-"$root/deploy/demo/Dockerfile.portal"}
[[ -f $workflow && -f $dockerfile ]] || { printf '%s\n' 'missing portal release workflow or Dockerfile' >&2; exit 1; }

assert_contains() {
  grep -Fq -- "$1" "$2" || { printf 'missing portal release contract: %s (in %s)\n' "$1" "$2" >&2; exit 1; }
}

# A second image in the existing workflow, by digest, with its own artifact.
assert_contains 'ghcr.io/dungeonfluxgame/dungeonflux-portal' "$workflow"
assert_contains '--file deploy/demo/Dockerfile.portal' "$workflow"
assert_contains '--platform linux/amd64' "$workflow"
assert_contains 'name: portal-digest' "$workflow"
assert_contains 'portal-digest.txt' "$workflow"
assert_contains '/usr/local/bin/df-portal-hosted' "$workflow"
assert_contains '/opt/dungeonflux/app/index.html' "$workflow"

# Every action stays pinned to a full commit SHA, as the workflow already does.
while IFS= read -r line; do
  [[ $line =~ @[a-f0-9]{40}[[:space:]] ]] || { printf 'unpinned action: %s\n' "$line" >&2; exit 1; }
done < <(grep -E '^[[:space:]]*(- )?uses:' "$workflow")

# The image builds the real binary and the account-app bundle, and runs
# neither a fixture nor a development entrypoint.
assert_contains 'cargo build --locked --release -p df-hosted --bin df-portal-hosted' "$dockerfile"
assert_contains 'cargo build --locked --release -p df-account-app --lib --target wasm32-unknown-unknown' "$dockerfile"
assert_contains 'account_app_bg.wasm' "$dockerfile"
assert_contains 'ENTRYPOINT ["/usr/local/bin/df-portal-hosted"]' "$dockerfile"
assert_contains 'http://127.0.0.1:8099/app/healthz' "$dockerfile"
assert_contains 'USER 10001:10001' "$dockerfile"
! grep -Eq 'df-transport-fixture|fake|DF_PORTAL_OUTBOX' "$dockerfile" || {
  printf '%s\n' 'portal image carries a fixture or development setting' >&2
  exit 1
}

printf '%s\n' 'portal release image checks passed'
