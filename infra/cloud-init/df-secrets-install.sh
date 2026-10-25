#!/bin/bash
# df-secrets-install: the ONLY way secrets reach a host (plan 12.7). Installed root-owned (0755) at
# /usr/local/sbin/df-secrets-install by cloud-init; the deploy account and individual operators may run exactly this
# command and df-deploy through sudo (sudoers rules written by the same cloud-init). It reads a tar stream on stdin,
# validates it, and installs
#   /run/dungeonflux/secrets/<consumer>/<file>   as root:dfs-<consumer> mode 0440
# into a 0750 root:dfs-<consumer> directory. /run is tmpfs: nothing reaches the Droplet disk, so Droplet backups and
# snapshots never contain a secret; after a reboot the directories are empty until the next push.
#
# Each consumer is a separate Unix group, so a container (or a breakout from it) running with one consumer's group
# cannot read another consumer's files. A campaign runtime's group has no identity, payment, provider, storage or
# journal secrets. Containers mount their consumer DIRECTORY (never single files): files are replaced by rename
# inside the existing directory, so the directory inode a running container mounted is never replaced and a
# rotation takes effect without breaking the mount.
#
# ALL OR NOTHING (critic round 1c, item 11). The whole archive is validated first (regular files only, member name
# exactly <consumer>/<file> with ONE slash, consumer from the fixed list, file name [A-Za-z0-9_][A-Za-z0-9_.-]*,
# no duplicate member, at most 1 MiB); then EVERY file is staged as .new.<file> in its directory with its final owner
# and mode; only when all of them are staged are they renamed into place, and a failure while staging removes the
# staged files and installs nothing. Ownership and permissions recorded in the archive are ignored.
#
# usage (as root, via sudo):  tar -C DIR -cf - consumer/file ... | sudo df-secrets-install
#        read-only audit:       sudo df-secrets-install --audit    prints  <type> <mode> <owner:group> <path>  for every entry under the
#                              secrets root, and a line `docker-config-on-disk <path>` for any docker config.json found in a home
#                              directory (a registry token on the disk would reach Droplet backups). deploy/scripts/secrets-audit.sh
#                              host calls it, because the 0750 root:dfs-* directories are unreadable to the operator.
# Test mode (non-root only): df-secrets-install --root DIR --no-chown < archive.tar
# STATUS: exercised locally in test mode on the authoring machine; the root path (chown, groups) is UNVERIFIED.
set -euo pipefail

declare -A GROUP_GID=(
  [control-plane]=10001 [runtime]=10002 [dispatcher]=10003 [runner]=10004
  [backup]=10005 [monitoring]=10006 [billing-worker]=10007 [ingestor]=10008 [edge]=10010
)
root=/run/dungeonflux/secrets
no_chown=0
audit=0
while [ $# -gt 0 ]; do
  case "$1" in
    --root) root=$2; shift 2 ;;
    --no-chown) no_chown=1; shift ;;
    --audit) audit=1; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
if [ "$(id -u)" -eq 0 ]; then
  if [ "$root" != /run/dungeonflux/secrets ] || [ "$no_chown" -ne 0 ]; then
    echo "--root and --no-chown are test options and are refused when running as root" >&2
    exit 2
  fi
elif [ "$no_chown" -ne 1 ]; then
  echo "not root: only --root DIR --no-chown (test mode) is allowed" >&2
  exit 2
fi

if [ "$audit" -eq 1 ]; then
  find "$root" -mindepth 1 -printf '%y %m %u:%g %P
' 2>/dev/null || true
  for f in /root/.docker/config.json /home/*/.docker/config.json; do
    if [ -e "$f" ]; then echo "docker-config-on-disk $f"; fi
  done
  exit 0
fi

base=$(dirname "$root")
incoming=$(mktemp -d "$base/.incoming.XXXXXX")
staged=()
cleanup() {
  local f
  for f in "${staged[@]}"; do rm -f "$f"; done
  rm -rf "$incoming"
}
trap cleanup EXIT
chmod 0700 "$incoming"

# Bounded read of the archive.
head -c 1048577 > "$incoming/archive.tar"
if [ "$(wc -c < "$incoming/archive.tar")" -gt 1048576 ]; then echo "archive larger than 1 MiB" >&2; exit 1; fi

# Validate every member BEFORE extracting or installing anything.
while IFS= read -r type; do
  [ "$type" = "-" ] || { echo "refusing archive: a member is not a regular file" >&2; exit 1; }
done < <(tar -tvf "$incoming/archive.tar" | awk '{ print substr($1,1,1) }')
members=()
declare -A seen_member
while IFS= read -r name; do
  case "$name" in
    */*/*|/*|*..*|"") echo "refusing archive: bad member name: $name" >&2; exit 1 ;;
  esac
  [[ "$name" == */* ]] || { echo "refusing archive: member without a consumer directory: $name" >&2; exit 1; }
  consumer=${name%%/*}
  file=${name#*/}
  [ -n "${GROUP_GID[$consumer]:-}" ] || { echo "refusing archive: unknown consumer: $consumer" >&2; exit 1; }
  [[ "$file" =~ ^[A-Za-z0-9_][A-Za-z0-9_.-]*$ ]] || { echo "refusing archive: bad file name: $file" >&2; exit 1; }
  [ -z "${seen_member[$name]:-}" ] || { echo "refusing archive: duplicate member: $name" >&2; exit 1; }
  seen_member[$name]=1
  members+=("$name")
done < <(tar -tf "$incoming/archive.tar")
[ "${#members[@]}" -gt 0 ] || { echo "empty archive" >&2; exit 1; }

mkdir "$incoming/x"
tar -xf "$incoming/archive.tar" -C "$incoming/x" --no-same-owner --no-same-permissions

mkdir -p "$root"
declare -A seen_consumer
# Phase 1: stage EVERY file next to its final name, with its final owner and mode. Nothing is visible yet.
for name in "${members[@]}"; do
  consumer=${name%%/*}
  file=${name#*/}
  dir="$root/$consumer"
  if [ -z "${seen_consumer[$consumer]:-}" ]; then
    seen_consumer[$consumer]=1
    mkdir -p "$dir"
    if [ "$no_chown" -eq 0 ]; then
      chown "root:dfs-$consumer" "$dir"
      chmod 0750 "$dir"
    fi
  fi
  src="$incoming/x/$name"
  chmod 0440 "$src"
  if [ "$no_chown" -eq 0 ]; then chown "root:dfs-$consumer" "$src"; fi
  mv -f "$src" "$dir/.new.$file"
  staged+=("$dir/.new.$file")
done
# Phase 2: rename into place (same directory, same filesystem: atomic per file, directory inode unchanged).
declare -A keep
for name in "${members[@]}"; do
  consumer=${name%%/*}
  file=${name#*/}
  mv -f "$root/$consumer/.new.$file" "$root/$consumer/$file"
  keep["$name"]=1
done
staged=()

# Remove files of the pushed consumers that are not in the new set (a rotation overlap file disappears when the
# push no longer carries it).
removed=0
for consumer in "${!seen_consumer[@]}"; do
  while IFS= read -r existing; do
    if [ -z "${keep["$consumer/$(basename "$existing")"]:-}" ]; then rm -f "$existing"; removed=$((removed + 1)); fi
  done < <(find "$root/$consumer" -maxdepth 1 -type f)
done
echo "installed ${#members[@]} file(s) for ${#seen_consumer[@]} consumer(s); removed $removed stale file(s)"
