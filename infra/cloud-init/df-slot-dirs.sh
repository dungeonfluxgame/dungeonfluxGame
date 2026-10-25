#!/bin/bash
# df-slot-dirs: creates and owns the per-slot file tree the trusted runner fills and the Docker proxy checks (round 2a).
# Installed root-owned (0755) at /usr/local/sbin/df-slot-dirs; run as root by a boot unit (df-slot-dirs.service, before docker) and by
# `df-deploy release`. Never run by the runner, which cannot (it is not root and has no capability).
#
# WHY. The proxy lets a create through only if each bind source is a regular file or directory, owned by the slot's uid, on a path
# with no symlink. That is only worth anything if nothing the runner controls can replace an entry on that path between the check and
# the mount. So the tree is built by root and the runner cannot change its shape:
#   /run/dungeonflux/runtime                       root:root 0755
#   /run/dungeonflux/runtime/slot<N>               root:root 0755   (the runner can neither create nor rename nor replace an entry here)
#   .../slot<N>/descriptor.toml   uid 20000+N, gid 10004 (the runner)   0460  runner writes the content; the runtime reads it as owner
#   .../slot<N>/secrets/          uid 20000+N, gid 10004                0570  runner creates and removes files; others cannot enter
#   (no spool directory: the runtime's spool is a per-container tmpfs with a size, so nothing persists from one tenant of a slot to the
#    next and nothing is written under /run without a quota)
# Idempotent: existing entries keep their contents; a symlink or an entry of the wrong type is an error, never followed or replaced.
#
# usage (root): df-slot-dirs --slots N | --config RUNNER_TOML        (the number of [[slots]] tables is the slot count; 1..99)
# Test mode (non-root only): add --root SANDBOX --no-chown (the sandbox stands in for /; ownership is not set).
# STATUS: exercised in test mode on the authoring machine; chown and the boot unit are UNVERIFIED.
set -euo pipefail

slots=""
config=""
root=""
no_chown=0
while [ $# -gt 0 ]; do
  case "$1" in
    --slots) slots=$2; shift 2 ;;
    --config) config=$2; shift 2 ;;
    --root) root=$2; shift 2 ;;
    --no-chown) no_chown=1; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
if [ "$(id -u)" -eq 0 ]; then
  if [ -n "$root" ] || [ "$no_chown" -ne 0 ]; then echo "--root and --no-chown are test options and are refused as root" >&2; exit 2; fi
elif [ -z "$root" ] || [ "$no_chown" -ne 1 ]; then
  echo "not root: only --root SANDBOX --no-chown (test mode) is allowed" >&2
  exit 2
fi
if [ -n "$config" ]; then
  [ -r "$config" ] || { echo "cannot read $config" >&2; exit 2; }
  slots=$(grep -c '^\[\[slots\]\]' "$config" || true)
fi
[[ "$slots" =~ ^[0-9]{1,2}$ ]] && [ "$slots" -ge 1 ] && [ "$slots" -le 99 ] || { echo "the slot count must be 1..99 (got '$slots')" >&2; exit 2; }

uid_base=20000
runner_gid=10004
base="$root/run/dungeonflux/runtime"

check_clean() { # check_clean PATH TYPE   (type: d or f): not a symlink, and the right type when it exists
  local path=$1 type=$2
  if [ -L "$path" ]; then echo "refusing: $path is a symlink" >&2; exit 3; fi
  if [ -e "$path" ]; then
    case "$type" in
      d) [ -d "$path" ] || { echo "refusing: $path is not a directory" >&2; exit 3; } ;;
      f) [ -f "$path" ] || { echo "refusing: $path is not a regular file" >&2; exit 3; } ;;
    esac
  fi
}
own() { # own PATH UID GID MODE
  if [ "$no_chown" -eq 0 ]; then chown -h "$2:$3" "$1"; fi
  chmod "$4" "$1"
}

for parent in "$root/run" "$root/run/dungeonflux"; do
  if [ -L "$parent" ]; then echo "refusing: $parent is a symlink" >&2; exit 3; fi
  mkdir -p "$parent"
done
check_clean "$base" d
mkdir -p "$base"
own "$base" 0 0 0755

n=1
while [ "$n" -le "$slots" ]; do
  uid=$((uid_base + n))
  slot="$base/slot$n"
  check_clean "$slot" d
  mkdir -p "$slot"
  own "$slot" 0 0 0755
  check_clean "$slot/descriptor.toml" f
  [ -e "$slot/descriptor.toml" ] || : > "$slot/descriptor.toml"
  own "$slot/descriptor.toml" "$uid" "$runner_gid" 0460
  check_clean "$slot/secrets" d
  mkdir -p "$slot/secrets"
  own "$slot/secrets" "$uid" "$runner_gid" 0570
  # A spool directory left by an earlier version would carry a previous tenant's data: it is refused, not reused.
  if [ -e "$slot/spool" ] || [ -L "$slot/spool" ]; then echo "refusing: $slot/spool exists (the spool is a tmpfs now; remove the directory by hand after checking it)" >&2; exit 3; fi
  n=$((n + 1))
done
echo "slot directories ready for $slots slot(s) under $base"
