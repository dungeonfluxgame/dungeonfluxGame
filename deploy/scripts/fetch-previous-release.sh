#!/usr/bin/env bash
set -euo pipefail

[ "$#" -eq 7 ] || { echo "usage: fetch-previous-release.sh IMAGE OWNER_TYPE OWNER REPOSITORY OUTPUT_DIR FIRST_RELEASE NEW_VERSION" >&2; exit 2; }
image=$1
owner_type=$2
owner=$3
repository=$4
output_dir=$5
first_release=$6
new_version=$7
command -v jq >/dev/null 2>&1 || { echo "missing tool: jq" >&2; exit 1; }
[[ "$first_release" == true || "$first_release" == false ]] || { echo "first_release must be true or false" >&2; exit 2; }
[[ "$new_version" =~ ^[0-9]{14}$ ]] || { echo "new release version must be a UTC YYYYMMDDHHMMSS value" >&2; exit 2; }

if [ "$owner_type" = Organization ]; then
  packages_endpoint="orgs/$owner/packages?package_type=container&per_page=100"
  versions_endpoint="orgs/$owner/packages/container/dungeonflux-release/versions?per_page=100"
else
  packages_endpoint="users/$owner/packages?package_type=container&per_page=100"
  versions_endpoint="users/$owner/packages/container/dungeonflux-release/versions?per_page=100"
fi
versions_file=$(mktemp)
revocations_file=$(mktemp)
work_dir=$(mktemp -d)
trap 'rm -f "$versions_file" "$revocations_file"; rm -rf "$work_dir"' EXIT
packages=$(gh api --paginate "$packages_endpoint") || { echo "could not list owner packages" >&2; exit 1; }
package_exists=$(jq -ser 'add | if type == "array" and all(.[]; (.name | type == "string")) then any(.[]; .name == "dungeonflux-release") else error("invalid package listing") end' <<<"$packages") || {
  echo "could not parse owner package listing" >&2
  exit 1
}
if [ "$package_exists" = true ]; then
  versions=$(gh api --paginate "$versions_endpoint") || { echo "could not list every release package version" >&2; exit 1; }
elif [ "$package_exists" = false ]; then
  versions='[]'
else
  echo "could not determine whether the release package exists" >&2
  exit 1
fi
if ! jq -se 'add | if type == "array" and all(.[]; (.name | type == "string") and (.created_at | type == "string")) then . else error("invalid package versions") end' <<<"$versions" > "$versions_file"; then
  echo "could not parse every release package version" >&2
  exit 1
fi
if ! jq -e 'length == 0' "$versions_file" >/dev/null; then
  if [ "$first_release" = true ]; then
    echo "first_release=true was requested but release package already has versions" >&2
    exit 1
  fi
  jq -r '.[] | [.name, .created_at] | @tsv' "$versions_file" > "$work_dir/versions.tsv"
  newest_version=""
  while IFS=$'\t' read -r digest created_at; do
    [[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]] || { echo "release package version has an invalid digest: $digest" >&2; exit 1; }
    created_epoch=$(date -u -d "$created_at" +%s 2>/dev/null) || { echo "release package version has an invalid creation date: $created_at" >&2; exit 1; }
    created_version=$(date -u -d "@$created_epoch" +%Y%m%d%H%M%S)
    if [[ "$created_version" > "$newest_version" ]]; then newest_version=$created_version; fi

    release_ref="$image@$digest"
    if ! cosign verify --certificate-identity "https://github.com/$repository/.github/workflows/release.yml@refs/heads/main" \
      --certificate-oidc-issuer https://token.actions.githubusercontent.com "$release_ref" >/dev/null; then
      echo "release version failed signature verification: $digest" >&2
      exit 1
    fi
    container=$(docker create "$release_ref" /noop) || { echo "could not create verified release container: $digest" >&2; exit 1; }
    if ! docker cp "$container:/release/revoked-images" "$work_dir/revoked-$digest"; then
      docker rm -f "$container" >/dev/null 2>&1 || true
      echo "could not read revoked-images from verified release: $digest" >&2
      exit 1
    fi
    docker rm "$container" >/dev/null || { echo "could not remove verified release container: $digest" >&2; exit 1; }
    cat "$work_dir/revoked-$digest" >> "$revocations_file"
  done < "$work_dir/versions.tsv"
  [[ "$new_version" > "$newest_version" ]] || {
    echo "new release version $new_version is not newer than newest verified release $newest_version" >&2
    exit 1
  }
else
  if [ "$first_release" != true ]; then
    echo "release package has zero versions; first_release=true is required" >&2
    exit 1
  fi
  if inspect_output=$(docker buildx imagetools inspect "$image:latest" 2>&1); then
    echo "release package has no versions but latest resolves; refusing first-release mode" >&2
    exit 1
  elif [[ "$inspect_output" != *"manifest unknown"* ]]; then
    echo "could not confirm absent release manifest: $inspect_output" >&2
    exit 1
  fi
  : > "$revocations_file"
  touch "$output_dir/first-release"
fi

LC_ALL=C sort -u "$revocations_file" > "$output_dir/previous-revoked"
