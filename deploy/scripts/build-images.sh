#!/usr/bin/env bash
# Build the control-plane, campaign-runtime, egress, runner, docker-proxy and edge images (plan 12.6: build Linux artifacts in CI, publish
# immutable digests). STATUS: UNVERIFIED (no Docker daemon on the authoring machine).
#
# usage: deploy/scripts/build-images.sh [--push] [--sign]
#   default   build locally for linux/amd64 and load into the local Docker (no registry, no network push)
#   --push    push to DF_REGISTRY_PREFIX/<name> (for example ghcr.io/OWNER) and print the image lines
#             (KEY=repo:version@sha256:digest) for the CI release job to commit into images.lock
#             (coordinator decision b). CI only; the registry login happens in the CI job, not here.
#   --sign    cosign keyless signing of each pushed digest through the CI job's OIDC identity; needs --push.
#             Deploy-time verification (df-deploy: cosign verify with --certificate-identity and
#             --certificate-oidc-issuer pinned, from /etc/dungeonflux/cosign-identity) is infra/cloud-init/df-deploy.sh.
#
# Refuses: a dirty working tree, a .env anywhere in the tree, any TODO-CI digest (local build: the two bases;
# --push: every line of images.lock, so a release never ships with a placeholder), an unpinned base.
# Timestamps: SOURCE_DATE_EPOCH is the commit time and the image exporter rewrites layer timestamps to it.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
lock="$root/deploy/images/images.lock"
push=0
sign=0
for arg in "$@"; do
  case "$arg" in
    --push) push=1 ;;
    --sign) sign=1 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done
if [ "$sign" -eq 1 ] && [ "$push" -eq 0 ]; then echo "--sign needs --push" >&2; exit 2; fi

need() { command -v "$1" >/dev/null 2>&1 || { echo "missing tool: $1" >&2; exit 1; }; }
need docker
need git
if [ "$sign" -eq 1 ]; then need cosign; fi

cd "$root"
if [ -n "$(git status --porcelain)" ]; then
  echo "refusing to build from a dirty working tree" >&2
  exit 1
fi
if find . -path ./.git -prune -o -path ./artifacts -prune -o \( -name '.env' -o -name '.env.*' \) -print | grep -q .; then
  echo "refusing to build: a .env file exists in the tree (it is excluded by .dockerignore, but remove it from CI checkouts)" >&2
  exit 1
fi

# Placeholder refusal: the shared rule for every script that reads images.lock.
if [ "$push" -eq 1 ]; then
  if grep -v '^[[:space:]]*#' "$lock" | grep -q 'TODO-CI'; then
    echo "images.lock still has TODO-CI placeholders; resolve every digest before a release build" >&2
    exit 1
  fi
fi
# shellcheck disable=SC1090
set -a; . "$lock"; set +a
for key in RUST_BASE RUNTIME_BASE EDGE_BUILDER_BASE EDGE_BASE; do
  value=${!key}
  case "$value" in
    *TODO-CI*|"") echo "images.lock: $key has no pinned digest yet" >&2; exit 1 ;;
    *:*@sha256:*) ;;
    *) echo "images.lock: $key must carry an exact tag and @sha256:" >&2; exit 1 ;;
  esac
done

vcs_ref=$(git rev-parse HEAD)
version=${DF_VERSION:-0.0.0-$(git rev-parse --short=12 HEAD)}
source_url=${DF_SOURCE_URL:-unknown}
SOURCE_DATE_EPOCH=$(git log -1 --format=%ct)
export SOURCE_DATE_EPOCH

metadata_dir=$(mktemp -d)
trap 'rm -rf "$metadata_dir"' EXIT

build_target() {
  local target=$1 image_var=$2
  local args=(
    buildx build
    --platform linux/amd64
    --file deploy/images/Dockerfile
    --target "$target"
    --build-arg "RUST_BASE=$RUST_BASE"
    --build-arg "RUNTIME_BASE=$RUNTIME_BASE"
    --build-arg "EDGE_BUILDER_BASE=$EDGE_BUILDER_BASE"
    --build-arg "EDGE_BASE=$EDGE_BASE"
    --build-arg "CADDY_VERSION=$CADDY_VERSION"
    --build-arg "CADDY_RATELIMIT_REF=$CADDY_RATELIMIT_REF"
    --build-arg "VCS_REF=$vcs_ref"
    --build-arg "VERSION=$version"
    --build-arg "SOURCE_URL=$source_url"
    --build-arg "SOURCE_DATE_EPOCH=$SOURCE_DATE_EPOCH"
    --provenance=mode=max
    --sbom=true
    --metadata-file "$metadata_dir/$target.json"
  )
  if [ "$push" -eq 1 ]; then
    local repo="${DF_REGISTRY_PREFIX:?set DF_REGISTRY_PREFIX, for example ghcr.io/OWNER}/dungeonflux-$target"
    docker "${args[@]}" \
      --output "type=image,name=$repo:$version,push=true,rewrite-timestamp=true" .
    local digest
    digest=$(sed -n 's/.*"containerimage.digest": *"\(sha256:[0-9a-f]*\)".*/\1/p' "$metadata_dir/$target.json" | head -n 1)
    [ -n "$digest" ] || { echo "no digest in build metadata for $target" >&2; exit 1; }
    echo "$image_var=$repo:$version@$digest"
    if [ "$sign" -eq 1 ]; then cosign sign --yes "$repo@$digest"; fi
  else
    docker "${args[@]}" \
      --output "type=docker,name=dungeonflux-$target:local,rewrite-timestamp=true" .
    echo "built dungeonflux-$target:local (local only, not deployable: no registry digest)"
  fi
}

build_target control-plane CONTROL_PLANE_IMAGE
build_target campaign-runtime CAMPAIGN_RUNTIME_IMAGE
build_target egress EGRESS_IMAGE
build_target runner RUNNER_IMAGE
build_target docker-proxy DOCKER_PROXY_IMAGE
build_target edge EDGE_IMAGE
