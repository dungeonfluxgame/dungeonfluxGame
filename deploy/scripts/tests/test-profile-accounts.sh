#!/usr/bin/env bash
set -Eeuo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)
temporary_dir=$(mktemp -d "$root/artifacts/tmp/agents/ACCOUNTSCLOSED-profile.XXXXXX")
trap 'rm -rf "$temporary_dir"' EXIT
settings="$temporary_dir/profile.env"

printf '%s\n' 'DF_ACCOUNTS_OPEN=false' > "$settings"
bash "$root/deploy/profiles/check-profile.sh" demo "$settings" >/dev/null

printf '%s\n' 'DF_ACCOUNTS_OPEN=true' > "$settings"
if bash "$root/deploy/profiles/check-profile.sh" demo "$settings" >/dev/null 2>&1; then
  printf '%s\n' 'demo accepted open accounts without launch confirmation' >&2
  exit 1
fi
printf '%s\n' 'DF_ACCOUNTS_OPEN=true' 'DF_ACCOUNTS_OPEN_CONFIRMED=yes' > "$settings"
bash "$root/deploy/profiles/check-profile.sh" demo "$settings" >/dev/null

printf '%s\n' 'DF_ACCOUNTS_OPEN=maybe' > "$settings"
if bash "$root/deploy/profiles/check-profile.sh" demo "$settings" >/dev/null 2>&1; then
  printf '%s\n' 'demo accepted an invalid DF_ACCOUNTS_OPEN value' >&2
  exit 1
fi

printf '%s\n' 'DF_ACCOUNTS_OPEN=true' > "$settings"
bash "$root/deploy/profiles/check-profile.sh" development "$settings" >/dev/null
printf '%s\n' 'profile account guard tests passed'
