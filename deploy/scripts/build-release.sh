#!/usr/bin/env bash
# Assemble, publish and sign the RELEASE image and the KEY-SET image that `df-deploy` verifies on a host (round 2). CI only; never run on a
# production machine. STATUS: UNVERIFIED (no Docker daemon, cosign or registry on the authoring machine); the CI job that calls it is round 3.
#
#   deploy/scripts/build-release.sh release --built FILE (--previous-revoked FILE | --first-release) [--push] [--sign]
#   deploy/scripts/build-release.sh keys [--push] [--sign]
#
# release  FILE holds the KEY=repo:version@sha256:digest lines that `deploy/scripts/build-images.sh --push` printed (CONTROL_PLANE_IMAGE,
#          CAMPAIGN_RUNTIME_IMAGE, EGRESS_IMAGE, RUNNER_IMAGE, DOCKER_PROXY_IMAGE, EDGE_IMAGE, BACKUP_IMAGE). The script adds the third-party images (DBGATE, PROMETHEUS, ALERTMANAGER, NODE_EXPORTER; pinned by digest) from
#          deploy/images/images.lock, writes release.lock, and packages it with production.compose.yml, production.env, the edge files and the
#          config TEMPLATES of deploy/config/production into <prefix>-release. Before anything is built it runs the static checks that
#          need no daemon (config policy; the production compose policy when jq is installed).
# keys     packages infra/keys/ (ci.pub and admin/*.pub; raw public keys only) into <prefix>-keys. The key set is validated with
#          infra/cloud-init/df-keys-apply.sh in test mode first, so a set the host would refuse never gets signed.
#
# Both images are `FROM scratch`: files only. --push pushes $DF_REGISTRY_PREFIX/dungeonflux-release and -keys (the family
# $DF_REGISTRY_PREFIX/dungeonflux is what the hosts pin in /etc/dungeonflux/release-source as prefix) and prints `RELEASE_DIGEST=` / `KEYS_DIGEST=`; --sign signs that digest with
# cosign keyless through the CI job's OIDC identity (the identity the hosts pin: the workflow file on refs/heads/main). Refuses: a dirty
# tree, a TODO-CI or placeholder digest, a built-image line from a different repository family.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
mode=${1:-}
[ -n "$mode" ] && shift || { echo "usage: build-release.sh release --built FILE | keys  [--push] [--sign]" >&2; exit 2; }
built=""
previous_revoked=""
first_release=0
push=0
sign=0
while [ $# -gt 0 ]; do
  case "$1" in
    --built) built=$2; shift 2 ;;
    --previous-revoked) previous_revoked=$2; shift 2 ;;
    --first-release) first_release=1; shift ;;
    --push) push=1; shift ;;
    --sign) sign=1; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[ "$sign" -eq 0 ] || [ "$push" -eq 1 ] || { echo "--sign needs --push" >&2; exit 2; }
case "$mode" in release|keys) ;; *) echo "mode must be release or keys" >&2; exit 2 ;; esac

need() { command -v "$1" >/dev/null 2>&1 || { echo "missing tool: $1" >&2; exit 1; }; }
need git
if [ "$push" -eq 1 ]; then need docker; fi
if [ "$sign" -eq 1 ]; then need cosign; fi
cd "$root"
if [ -n "$(git status --porcelain)" ]; then echo "refusing to build from a dirty working tree" >&2; exit 1; fi
prefix=${DF_REGISTRY_PREFIX:-}
if [ "$push" -eq 1 ] && [ -z "$prefix" ]; then echo "set DF_REGISTRY_PREFIX, for example ghcr.io/OWNER (the same variable build-images.sh uses)" >&2; exit 1; fi
family="$prefix/dungeonflux"

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
version=${DF_VERSION:-0.0.0-$(git rev-parse --short=12 HEAD)}
SOURCE_DATE_EPOCH=$(git log -1 --format=%ct)
export SOURCE_DATE_EPOCH

if [ "$mode" = release ]; then
  [ -f "$built" ] || { echo "release needs --built FILE (the lines build-images.sh --push printed)" >&2; exit 2; }
  bash deploy/scripts/check-config-policy.sh
  # The revocation list only grows: it must be a superset of the PREVIOUS signed release's list (extracted from that release image by the
  # CI job). A release with neither the previous list nor --first-release is refused.
  if [ "$first_release" -eq 1 ]; then
    [ -z "$previous_revoked" ] || { echo "--first-release and --previous-revoked exclude each other" >&2; exit 2; }
  else
    [ -n "$previous_revoked" ] || { echo "release needs --previous-revoked FILE (the revoked-images of the previous signed release) or --first-release" >&2; exit 2; }
    bash deploy/scripts/check-revoked-superset.sh "$previous_revoked" deploy/images/revoked-images
  fi
  if command -v jq >/dev/null 2>&1 && command -v docker >/dev/null 2>&1; then bash deploy/scripts/check-production-policy.sh; else echo "SKIP production compose policy (needs docker and jq)"; fi

  lock="$stage/release.lock"
  : > "$lock"
  for key in CONTROL_PLANE_IMAGE CAMPAIGN_RUNTIME_IMAGE EGRESS_IMAGE RUNNER_IMAGE DOCKER_PROXY_IMAGE EDGE_IMAGE BACKUP_IMAGE; do
    line=$(grep -E "^$key=" "$built" | tail -n 1 || true)
    [ -n "$line" ] || { echo "$built has no $key line" >&2; exit 1; }
    value=${line#*=}
    [[ "$value" =~ @sha256:[0-9a-f]{64}$ ]] && [[ "$value" != *TODO* && "$value" != *REPLACE_ME* ]] || { echo "$key has no real digest: $value" >&2; exit 1; }
    if [ -n "$prefix" ] && [[ "$value" != "$family"-* ]]; then echo "$key is not from $family-*: $value" >&2; exit 1; fi
    echo "$line" >> "$lock"
  done
  for key in DBGATE_IMAGE PROMETHEUS_IMAGE ALERTMANAGER_IMAGE NODE_EXPORTER_IMAGE; do
    line=$(grep -E "^$key=" deploy/images/images.lock | head -n 1 || true)
    [ -n "$line" ] || { echo "deploy/images/images.lock has no $key" >&2; exit 1; }
    [[ "$line" == *@sha256:[0-9a-f]* && "$line" != *TODO* ]] || { echo "$key has no real digest in images.lock" >&2; exit 1; }
    echo "$line" >> "$lock"
  done
  install -m 0644 deploy/compose/production.compose.yml "$stage/production.compose.yml"
  install -m 0644 deploy/compose/production.env "$stage/production.env"
  install -m 0644 deploy/images/revoked-images "$stage/revoked-images"
  install -d "$stage/edge/extra" "$stage/config"
  install -m 0644 deploy/edge/Caddyfile.production "$stage/edge/Caddyfile.production"
  find deploy/edge/extra -type f -print0 | while IFS= read -r -d '' f; do install -m 0644 "$f" "$stage/edge/extra/$(basename "$f")"; done
  find deploy/config/production -type f -print0 | while IFS= read -r -d '' f; do install -m 0644 "$f" "$stage/config/$(basename "$f")"; done
  dockerfile=deploy/images/Dockerfile.release
  repo_name="$family-release"
  var=RELEASE_DIGEST
else
  keys_src="$root/infra/keys"
  [ -s "$keys_src/ci.pub" ] || { echo "infra/keys/ci.pub is missing: the key set needs the CI key" >&2; exit 1; }
  install -m 0644 "$keys_src/ci.pub" "$stage/ci.pub"
  if [ -d "$keys_src/admin" ]; then
    install -d "$stage/admin"
    find "$keys_src/admin" -type f -name '*.pub' -print0 | while IFS= read -r -d '' f; do install -m 0644 "$f" "$stage/admin/$(basename "$f")"; done
  fi
  # The host would refuse a bad set; refuse it here first (test mode: no root, a sandbox, no system changes).
  sandbox=$(mktemp -d)
  bash infra/cloud-init/df-keys-apply.sh --from "$stage" --root "$sandbox" --no-system
  rm -rf "$sandbox"
  dockerfile=deploy/images/Dockerfile.keys
  repo_name="$family-keys"
  var=KEYS_DIGEST
fi

if [ "$push" -eq 0 ]; then
  echo "staged $mode files (not built, not pushed):"
  (cd "$stage" && find . -type f | sort)
  exit 0
fi

meta=$(mktemp)
docker buildx build --platform linux/amd64 --file "$dockerfile" \
  --provenance=mode=max --sbom=true --metadata-file "$meta" \
  --output "type=image,name=$repo_name:$version,push=true,rewrite-timestamp=true" "$stage"
digest=$(sed -n 's/.*"containerimage.digest": *"\(sha256:[0-9a-f]*\)".*/\1/p' "$meta" | head -n 1)
rm -f "$meta"
[ -n "$digest" ] || { echo "no digest in build metadata" >&2; exit 1; }
echo "$var=$digest"
if [ "$sign" -eq 1 ]; then cosign sign --yes "$repo_name@$digest"; fi
