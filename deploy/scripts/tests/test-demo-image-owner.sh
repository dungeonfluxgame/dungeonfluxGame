#!/usr/bin/env bash
set -Eeuo pipefail

readonly repository_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)"
readonly image='ghcr.io/dungeonfluxgame/dungeonflux-demo'
readonly files=(
  .github/workflows/demo-release.yml
  deploy/demo/compose.yml
  deploy/demo/deploy.sh
  deploy/profiles/README.md
)

for file in "${files[@]}"; do
  grep -Fq -- "$image" "$repository_dir/$file" || {
    printf 'demo image owner check: expected %s in %s\n' "$image" "$file" >&2
    exit 1
  }
done

if grep -Fq 'ghcr.io/monstercameron/dungeonflux-demo' \
  "$repository_dir/.github/workflows/demo-release.yml" \
  "$repository_dir/deploy/demo/compose.yml" \
  "$repository_dir/deploy/demo/deploy.sh" \
  "$repository_dir/deploy/profiles/README.md"; then
  printf '%s\n' 'demo image owner check: stale package path remains' >&2
  exit 1
fi

printf '%s\n' 'demo image owner checks passed'
