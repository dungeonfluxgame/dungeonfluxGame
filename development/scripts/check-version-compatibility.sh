#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: check-version-compatibility.sh <base-commit>" >&2
    exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
base_commit="$1"
git -C "$repo_root" rev-parse --verify "$base_commit^{commit}" >/dev/null

mapfile -t published_migrations < <(
    git -C "$repo_root" ls-tree -r --name-only "$base_commit" -- crates/df-persistence/migrations
)

if ((${#published_migrations[@]} == 0)); then
    echo "no migrations in base commit; migration immutability check skipped"
    exit 0
fi

if ! git -C "$repo_root" diff --quiet "$base_commit"...HEAD -- "${published_migrations[@]}"; then
    echo "published migration changed or was removed; add a new numbered migration instead" >&2
    git -C "$repo_root" diff --name-status "$base_commit"...HEAD -- "${published_migrations[@]}" >&2
    exit 1
fi

echo "published migrations are unchanged; new numbered migrations are allowed"
