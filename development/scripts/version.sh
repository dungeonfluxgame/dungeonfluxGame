#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
manifest="$repo_root/Cargo.toml"
export PYTHONDONTWRITEBYTECODE=1
if py -3 -c 'import sys' >/dev/null 2>&1; then
    python_command=(py -3)
elif python3 -c 'import sys' >/dev/null 2>&1; then
    python_command=(python3)
elif python -c 'import sys' >/dev/null 2>&1; then
    python_command=(python)
else
    echo "Python 3 is required to edit the workspace version" >&2
    exit 1
fi

python_version() {
    "${python_command[@]}" -I - "$manifest" "$@" <<'PY'
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
action = sys.argv[2]
text = path.read_bytes().decode("utf-8")
match = re.search(r"(?ms)^\[workspace\.package\]\s*$([^\[]*)", text)
if match is None:
    raise SystemExit("[workspace.package] not found in root Cargo.toml")
section = match.group(1)
version_match = re.search(r'^version\s*=\s*"([^"]+)"\s*$', section, re.M)
if version_match is None:
    raise SystemExit("workspace package version not found")
current = version_match.group(1)

if action == "show":
    print(current)
    raise SystemExit(0)

bump = sys.argv[3]
stable = re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)", current)
pre = re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)-(alpha|beta|rc)\.(0|[1-9]\d*)", current)

if bump == "pre":
    if pre:
        major, minor, patch, stage, number = pre.groups()
        next_version = f"{major}.{minor}.{patch}-{stage}.{int(number) + 1}"
    elif stable:
        major, minor, _, = stable.groups()
        next_version = f"{major}.{int(minor) + 1}.0-alpha.1"
    else:
        raise SystemExit("version must be stable SemVer or alpha.N, beta.N, or rc.N")
else:
    if not stable:
        raise SystemExit("finish or explicitly resolve the current prerelease before a stable bump")
    major, minor, patch = map(int, stable.groups())
    if bump == "major":
        if major == 0:
            raise SystemExit("major is not used before 1.0.0; use minor for a breaking 0.x change")
        next_version = f"{major + 1}.0.0"
    elif bump == "minor":
        next_version = f"{major}.{minor + 1}.0"
    elif bump == "patch":
        next_version = f"{major}.{minor}.{patch + 1}"
    else:
        raise SystemExit("usage: version.sh show | bump <major|minor|patch|pre>")

replacement = section[:version_match.start(1)] + next_version + section[version_match.end(1):]
updated = text[:match.start(1)] + replacement + text[match.end(1):]
path.write_bytes(updated.encode("utf-8"))
print(f"{current} -> {next_version}")
PY
}

case "${1:-}" in
    show)
        python_version show
        ;;
    bump)
        if [[ $# -ne 2 || ! "$2" =~ ^(major|minor|patch|pre)$ ]]; then
            echo "usage: development/scripts/version.sh show | bump <major|minor|patch|pre>" >&2
            exit 2
        fi
        if [[ -n "$(git -C "$repo_root" status --porcelain -- Cargo.toml)" ]]; then
            echo "refusing to edit dirty Cargo.toml" >&2
            exit 1
        fi
        python_version bump "$2"
        cat <<'NEXT_STEPS'
Next: review the Cargo.toml diff, update docs/changelog/ for player-visible changes,
update CHANGELOG.md through the coordinator, run release checks, commit, then have
the owner create and push the annotated vX.Y.Z tag.
NEXT_STEPS
        ;;
    *)
        echo "usage: development/scripts/version.sh show | bump <major|minor|patch|pre>" >&2
        exit 2
        ;;
esac
