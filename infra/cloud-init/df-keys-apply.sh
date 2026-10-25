#!/bin/bash
# df-keys-apply: rewrite the SSH access of an application or worker host from a REVIEWED, SIGNED key set (the round 2
# blocker and launch gate `refresh-keys`). Installed root-owned (0755) at /usr/local/sbin/df-keys-apply. It is never run by
# hand: `sudo df-deploy refresh-keys sha256:<digest>` first verifies the key-set image with cosign (the pinned CI identity
# and issuer, so only a build of the reviewed main branch can sign), unpacks it, and then hands the directory to this tool.
#
# KEY-SET DIRECTORY (what CI packages from infra/keys/ in the repository, after review):
#   ci.pub                one or more `ssh-ed25519 <base64> [comment]` lines: the CI deploy key
#   admin/<name>.pub      one file per person: one or more `sk-ssh-ed25519@openssh.com <base64> [comment]` lines (FIDO2 keys)
# The files hold RAW PUBLIC KEYS only. This tool writes every option itself, so a key set cannot carry a command, a
# permitopen or an environment option:
#   dfdeploy  command="/usr/local/sbin/df-deploy-ssh",restrict <key>
#   <admin>   verify-required <key>                     (touch AND PIN)
# It then: writes /etc/ssh/authorized_keys.d/<account> (root-owned 0644; sshd reads only these files), creates missing admin
# accounts (locked, no password), regenerates /etc/sudoers.d/dungeonflux-admins (df-secrets-install and `df-deploy psql`
# only), rewrites /etc/ssh/sshd_config.d/05-allowusers.conf, checks `sshd -t`, and reloads sshd. An account that is no
# longer in the set loses its authorized_keys file and its AllowUsers entry (its account and files are left, locked).
# Everything is validated BEFORE anything is written, and the tool refuses a set that would lock everyone out (no CI key, no
# admin) or that names a reserved account. The bastion is not managed by this tool: it has no registry access and is
# replaced monthly instead.
#
# usage (root): df-keys-apply --from DIR
# Test mode (non-root only): df-keys-apply --from DIR --root SANDBOX --no-system
# STATUS: exercised locally in test mode on the authoring machine; the root path (useradd, visudo, sshd) is UNVERIFIED.
set -euo pipefail

from=""
root=""
no_system=0
while [ $# -gt 0 ]; do
  case "$1" in
    --from) from=$2; shift 2 ;;
    --root) root=$2; shift 2 ;;
    --no-system) no_system=1; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[ -d "$from" ] || { echo "--from DIR required" >&2; exit 2; }
if [ "$(id -u)" -eq 0 ]; then
  if [ -n "$root" ] || [ "$no_system" -ne 0 ]; then echo "--root and --no-system are test options and are refused as root" >&2; exit 2; fi
elif [ -z "$root" ] || [ "$no_system" -ne 1 ]; then
  echo "not root: only --root SANDBOX --no-system (test mode) is allowed" >&2
  exit 2
fi

keys_dir="$root/etc/ssh/authorized_keys.d"
sudoers_file="$root/etc/sudoers.d/dungeonflux-admins"
allow_file="$root/etc/ssh/sshd_config.d/05-allowusers.conf"

ci_re='^ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA[A-Za-z0-9+/]{44}( [A-Za-z0-9@._-]{1,64})?$'
admin_re='^sk-ssh-ed25519@openssh\.com [A-Za-z0-9+/]+={0,2}( [A-Za-z0-9@._-]{1,64})?$'
name_re='^[a-z][a-z0-9_-]{1,31}$'
reserved=" root dfdeploy dfci admin daemon bin sys sync games man lp mail news uucp proxy www-data backup list irc nobody ubuntu sshd docker "

fail() { echo "refusing key set: $*" >&2; exit 1; }

# A regular expression does not prove a key parses: a truncated or malformed base64 body would pass it and could lock everyone out.
# So every key line is also given to ssh-keygen, which must accept it and report the expected type.
command -v ssh-keygen >/dev/null 2>&1 || fail "ssh-keygen is not installed: keys cannot be validated"
keycheck_dir=$(mktemp -d)
trap 'rm -rf "$keycheck_dir"' EXIT
keygen_ok() { # keygen_ok LINE EXPECTED_TYPE_IN_PARENTHESES
  local line=$1 type=$2 out
  printf '%s\n' "$line" > "$keycheck_dir/key.pub"
  out=$(ssh-keygen -l -f "$keycheck_dir/key.pub" 2>/dev/null) || return 1
  [ "$(printf '%s\n' "$out" | wc -l)" -eq 1 ] && [[ "$out" == *"($type)" ]]
}

# --- validate everything first -----------------------------------------------------------------------------------------
[ -s "$from/ci.pub" ] || fail "ci.pub is missing or empty"
ci_count=0
while IFS= read -r line || [ -n "$line" ]; do
  [ -n "$line" ] || continue
  [[ "$line" =~ $ci_re ]] || fail "ci.pub has a line that is not a plain ssh-ed25519 public key"
  keygen_ok "$line" ED25519 || fail "ci.pub has a key that ssh-keygen does not accept as ED25519"
  ci_count=$((ci_count + 1))
done < "$from/ci.pub"
[ "$ci_count" -ge 1 ] || fail "ci.pub holds no key"
[ "$ci_count" -le 4 ] || fail "ci.pub holds more than 4 keys"

admins=()
if [ -d "$from/admin" ]; then
  for file in "$from"/admin/*.pub; do
    [ -e "$file" ] || continue
    name=$(basename "$file" .pub)
    [[ "$name" =~ $name_re ]] || fail "admin account name '$name' is not allowed"
    case "$reserved" in *" $name "*) fail "admin account name '$name' is reserved" ;; esac
    count=0
    while IFS= read -r line || [ -n "$line" ]; do
      [ -n "$line" ] || continue
      [[ "$line" =~ $admin_re ]] || fail "admin/$name.pub has a line that is not a plain FIDO2 sk-ssh-ed25519 public key (no options, one key per line)"
      keygen_ok "$line" ED25519-SK || fail "admin/$name.pub has a key that ssh-keygen does not accept as ED25519-SK (a FIDO2 key)"
      count=$((count + 1))
    done < "$file"
    [ "$count" -ge 1 ] || fail "admin/$name.pub holds no key"
    [ "$count" -le 4 ] || fail "admin/$name.pub holds more than 4 keys"
    admins+=("$name")
  done
fi
[ "${#admins[@]}" -ge 1 ] || fail "no admin account: the set would leave only the CI key (add at least one person, or use the console)"
[ "${#admins[@]}" -le 20 ] || fail "more than 20 admin accounts"
# Other files in the set are not allowed: nothing but ci.pub and admin/*.pub is read, so refuse surprises.
for entry in "$from"/* "$from"/admin/*; do
  [ -e "$entry" ] || continue
  case "$entry" in "$from/ci.pub"|"$from/admin"|"$from"/admin/*.pub) ;; *) fail "unexpected entry $(basename "$entry") in the key set" ;; esac
done

# --- stage the new files ----------------------------------------------------------------------------------------------------
stage=$(mktemp -d)
trap 'rm -rf "$stage" "$keycheck_dir"' EXIT
mkdir -p "$stage/keys"
while IFS= read -r line || [ -n "$line" ]; do
  [ -n "$line" ] || continue
  printf 'command="/usr/local/sbin/df-deploy-ssh",restrict %s\n' "$line"
done < "$from/ci.pub" > "$stage/keys/dfdeploy"
for name in "${admins[@]}"; do
  while IFS= read -r line || [ -n "$line" ]; do
    [ -n "$line" ] || continue
    printf 'verify-required %s\n' "$line"
  done < "$from/admin/$name.pub" > "$stage/keys/$name"
done
{
  echo "# Written by df-keys-apply from a signed key set. Do not edit."
  for name in "${admins[@]}"; do
    printf '%s ALL=(root) NOPASSWD: /usr/local/sbin/df-secrets-install "", /usr/local/sbin/df-secrets-install --audit, /usr/local/sbin/df-deploy psql *\n' "$name"
  done
} > "$stage/sudoers"
printf 'AllowUsers dfdeploy %s\n' "${admins[*]}" > "$stage/allowusers"

if [ "$no_system" -eq 0 ]; then
  visudo -cf "$stage/sudoers" >/dev/null || fail "the generated sudoers file does not validate"
  for name in "${admins[@]}"; do
    id "$name" >/dev/null 2>&1 || useradd --create-home --shell /bin/bash --user-group "$name"
    passwd -l "$name" >/dev/null 2>&1 || true
  done
fi

# --- install: authorized_keys files, then sudoers, then AllowUsers -------------------------------------------------------------
install -d -m 0755 "$keys_dir" "$(dirname "$sudoers_file")" "$(dirname "$allow_file")"
declare -A wanted
wanted[dfdeploy]=1
for name in "${admins[@]}"; do wanted[$name]=1; done
removed=0
removed_accounts=()
for existing in "$keys_dir"/*; do
  [ -e "$existing" ] || continue
  account=$(basename "$existing")
  if [ -z "${wanted[$account]:-}" ]; then rm -f "$existing"; removed=$((removed + 1)); removed_accounts+=("$account"); fi
done
for account in "${!wanted[@]}"; do
  install -m 0644 "$stage/keys/$account" "$keys_dir/.new.$account"
  mv -f "$keys_dir/.new.$account" "$keys_dir/$account"
done
install -m 0440 "$stage/sudoers" "$sudoers_file.new"
mv -f "$sudoers_file.new" "$sudoers_file"
[ ! -f "$allow_file" ] || cp -p "$allow_file" "$stage/allowusers.old"
install -m 0644 "$stage/allowusers" "$allow_file.new"
mv -f "$allow_file.new" "$allow_file"

if [ "$no_system" -eq 0 ]; then
  if ! sshd -t; then
    if [ -f "$stage/allowusers.old" ]; then cp -p "$stage/allowusers.old" "$allow_file"; fi
    fail "sshd -t rejected the new AllowUsers file; the previous one was restored (the authorized_keys files are already updated)"
  fi
  systemctl reload ssh
  # A removed person's already-open SSH sessions survive the reload and the key removal: end them now (after sshd accepts the new
  # AllowUsers, so a failed change leaves everyone's access as it was). The account itself stays, locked, with its files.
  for account in "${removed_accounts[@]}"; do
    id "$account" >/dev/null 2>&1 || continue
    passwd -l "$account" >/dev/null 2>&1 || true
    loginctl terminate-user "$account" >/dev/null 2>&1 || true
    pkill -KILL -u "$account" >/dev/null 2>&1 || true
  done
fi
echo "key set applied: ci keys $ci_count, admin accounts ${#admins[@]}, removed $removed stale account file(s)"
