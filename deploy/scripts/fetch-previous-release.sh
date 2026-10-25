#!/usr/bin/env bash
set -euo pipefail

[ "$#" -eq 5 ] || { echo "usage: fetch-previous-release.sh IMAGE OWNER_TYPE OWNER REPOSITORY OUTPUT_DIR" >&2; exit 2; }
image=$1
owner_type=$2
owner=$3
repository=$4
output_dir=$5
command -v jq >/dev/null 2>&1 || { echo "missing tool: jq" >&2; exit 1; }

if docker buildx imagetools inspect "$image:latest" >/dev/null 2>&1; then
  digest=$(docker buildx imagetools inspect "$image:latest" --format '{{.Manifest.Digest}}')
  [[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]] || { echo "latest release has an invalid digest" >&2; exit 1; }
  release_ref="$image@$digest"
  cosign verify --certificate-identity "https://github.com/$repository/.github/workflows/release.yml@refs/heads/main" \
    --certificate-oidc-issuer https://token.actions.githubusercontent.com "$release_ref" >/dev/null
  container=$(docker create "$release_ref" /noop)
  trap 'docker rm -f "$container" >/dev/null 2>&1 || true' EXIT
  docker cp "$container:/release/revoked-images" "$output_dir/previous-revoked"
  docker rm "$container" >/dev/null
  trap - EXIT
  exit 0
fi

if [ "$owner_type" = Organization ]; then
  packages_endpoint="orgs/$owner/packages?package_type=container&per_page=100"
  versions_endpoint="orgs/$owner/packages/container/dungeonflux-release/versions?per_page=1"
else
  packages_endpoint="users/$owner/packages?package_type=container&per_page=100"
  versions_endpoint="users/$owner/packages/container/dungeonflux-release/versions?per_page=1"
fi
packages=$(gh api --paginate "$packages_endpoint") || { echo "could not list owner packages" >&2; exit 1; }
package_exists=$(jq -s -r 'add | if type == "array" then any(.[]; .name == "dungeonflux-release") | tostring else error("expected package arrays") end' <<<"$packages") || {
  echo "could not parse owner package listing" >&2
  exit 1
}
if [ "$package_exists" = true ]; then
  versions=$(gh api "$versions_endpoint") || { echo "could not inspect release package versions" >&2; exit 1; }
  jq -e 'length == 0' <<<"$versions" >/dev/null || {
    echo "release package has versions but no latest tag; refusing first-release mode" >&2
    exit 1
  }
elif [ "$package_exists" != false ]; then
  echo "could not determine whether the release package exists" >&2
  exit 1
fi

: > "$output_dir/previous-revoked"
touch "$output_dir/first-release"
