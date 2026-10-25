#!/usr/bin/env bash
set -Eeuo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)
deploy_script=${DEPLOY_SCRIPT:-"$root/deploy/demo/deploy.sh"}

assert_contains() {
  grep -Fq -- "$1" "$deploy_script" || {
    printf 'missing deployment security contract: %s\n' "$1" >&2
    exit 1
  }
}

! grep -Fq '/tmp/df-demo-$run_token-' "$deploy_script" || {
  printf '%s\n' 'deployment still uploads to a predictable shared /tmp path' >&2
  exit 1
}
assert_contains 'mktemp -d /tmp/df-demo-upload.XXXXXX'
assert_contains "stat -c '%u:%a' -- \"\$incoming_dir\""
assert_contains "stat -c '%u %a' -- \"\$path\""
assert_contains '(( (8#$mode & 0022) == 0 ))'
assert_contains '[[ "$actual_sha256" == "$expected_sha256" ]]'
for file in compose renderer site app; do
  assert_contains "verify_upload_file \"\$incoming_$file\""
done
assert_contains 'trap cleanup_upload_dir EXIT'

printf '%s\n' 'demo deployment upload security checks passed'
