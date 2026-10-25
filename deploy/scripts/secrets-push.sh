#!/usr/bin/env bash
# Deliver runtime secrets from an operator-prepared directory to a host's tmpfs through the root-owned
# df-secrets-install (plan 12.7; critic round 1 items 4 and 5). STATUS: UNVERIFIED (jq absent here; the
# installer's test mode was exercised locally). Prints a plan by default and NEVER prints a secret value.
# Needs: jq, ssh, tar, grep.
#
# usage: deploy/scripts/secrets-push.sh --env staging|production --roles app[,worker] --host user@host \
#            --from-dir DIR [--as operator|ci] [--apply]
#   --as operator (default)  an individual operator with their own account and FIDO2 key: runs `sudo df-secrets-install`.
#   --as ci                  the CI deploy identity: the host account dfdeploy has the forced command df-deploy-ssh, so the
#                            client sends the word `secrets-install` and nothing else (coordinator ruling b: production
#                            deploys run only from CI, through the bastion, with that key).
#   DIR is laid out like the host: DIR/<consumer>/<file>, one directory per consumer in the manifest, exported
#   from the password manager by the operator (mode 700, on tmpfs or an encrypted volume; delete it afterwards).
#   --roles  the logical host roles this machine plays. While worker_droplet_count is 0 the application Droplet
#            also hosts the campaign runtimes: use --roles app,worker.
#   Without --apply: show what would be sent and what is missing. With --apply you must also set
#   DF_DEPLOY_CONFIRM=<env> so a typo in a shell history cannot reach production.
#   SSH goes through the bastion with your ssh config (ProxyJump) or SSH_OPTS="-J user@bastion".
# Refuses: a partial set, a source directory readable by others, and in STAGING any value that looks like a
# live key (manifest staging_policy.refuse_patterns; coordinator decision e).
# The host side is infra/cloud-init/df-secrets-install.sh: it validates the archive, installs each file as
# root:dfs-<consumer> 0440 by rename inside the existing directory (running containers keep their mount) and
# removes stale files of the pushed consumers. No cloud API is called.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
manifest="$root/deploy/secrets/secrets.manifest.json"
env_name=""; roles=""; host=""; from_dir=""; apply=0; as=operator
while [ $# -gt 0 ]; do
  case "$1" in
    --env) env_name=$2; shift 2 ;;
    --roles) roles=$2; shift 2 ;;
    --host) host=$2; shift 2 ;;
    --from-dir) from_dir=$2; shift 2 ;;
    --as) as=$2; shift 2 ;;
    --apply) apply=1; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$env_name" in staging|production) ;; *) echo "--env staging|production required" >&2; exit 2 ;; esac
case "$as" in operator|ci) ;; *) echo "--as operator|ci" >&2; exit 2 ;; esac
[ -n "$roles" ] && [ -n "$host" ] && [ -n "$from_dir" ] || { echo "--roles, --host and --from-dir required" >&2; exit 2; }
for tool in jq ssh tar grep; do command -v "$tool" >/dev/null 2>&1 || { echo "missing tool: $tool" >&2; exit 1; }; done
[ -d "$from_dir" ] || { echo "not a directory: $from_dir" >&2; exit 1; }

mode=$(stat -c %a "$from_dir" 2>/dev/null || stat -f %Lp "$from_dir")
case "$mode" in
  [0-7]00) ;;
  *) echo "refusing: $from_dir is accessible beyond its owner (mode $mode); chmod 700 it" >&2; exit 1 ;;
esac

# Every <consumer>/<file> this machine should receive: entries delivered to a host role it plays, for each
# consumer that exists on that role, plus the optional rotation-overlap ".previous" files.
roles_json=$(printf '%s' "$roles" | jq -R 'split(",")')
mapfile -t wanted < <(jq -r --arg env "$env_name" --argjson roles "$roles_json" '
  . as $m
  | $m.secrets[]
  | select(.delivery == "host" and (.environments | index($env)) and ((.optional // false) | not))
  | . as $s
  | $s.consumers[]
  | select(. as $c | any($roles[]; . as $r | ($s.hosts | index($r)) and ($m.consumers[$c].hosts | index($r))))
  | . + "/" + $s.file' "$manifest" | sort -u)
mapfile -t previous < <(jq -r --arg env "$env_name" --argjson roles "$roles_json" '
  . as $m
  | $m.secrets[]
  | select(.delivery == "host" and (.environments | index($env)) and (.rotation.previous_file != null))
  | . as $s
  | $s.consumers[]
  | select(. as $c | any($roles[]; . as $r | ($s.hosts | index($r)) and ($m.consumers[$c].hosts | index($r))))
  | . + "/" + $s.rotation.previous_file' "$manifest" | sort -u)
[ "${#wanted[@]}" -gt 0 ] || { echo "manifest lists nothing to deliver for roles '$roles' in $env_name" >&2; exit 1; }

missing=0
send=()
for path in "${wanted[@]}"; do
  if [ -s "$from_dir/$path" ]; then send+=("$path"); echo "send     $path"; else echo "MISSING  $path"; missing=$((missing + 1)); fi
done
for path in "${previous[@]}"; do
  if [ -s "$from_dir/$path" ]; then send+=("$path"); echo "send     $path (rotation overlap)"; fi
done
# Optional entries (for example the off-site backup credential) are sent only when present.
mapfile -t optional < <(jq -r --arg env "$env_name" --argjson roles "$roles_json" '
  . as $m
  | $m.secrets[]
  | select(.delivery == "host" and (.environments | index($env)) and (.optional // false))
  | . as $s
  | $s.consumers[]
  | select(. as $c | any($roles[]; . as $r | ($s.hosts | index($r)) and ($m.consumers[$c].hosts | index($r))))
  | . + "/" + $s.file' "$manifest" | sort -u)
for path in "${optional[@]}"; do
  if [ -s "$from_dir/$path" ]; then send+=("$path"); echo "send     $path (optional)"; fi
done
if [ "$missing" -gt 0 ]; then echo "$missing secret(s) missing; refusing to push a partial set" >&2; exit 1; fi

# Staging may only receive test-mode or sandbox credentials.
if [ "$env_name" = staging ]; then
  mapfile -t patterns < <(jq -r '.staging_policy.refuse_patterns[]' "$manifest")
  for path in "${send[@]}"; do
    for pattern in "${patterns[@]}"; do
      if grep -Eq -- "$pattern" "$from_dir/$path"; then
        echo "refusing: $path looks like a LIVE credential and staging accepts test-mode or sandbox keys only" >&2
        exit 1
      fi
    done
  done
fi

if [ "$apply" -ne 1 ]; then echo "dry run only; add --apply (and DF_DEPLOY_CONFIRM=$env_name) to push ${#send[@]} file(s)"; exit 0; fi
[ "${DF_DEPLOY_CONFIRM:-}" = "$env_name" ] || { echo "set DF_DEPLOY_CONFIRM=$env_name to confirm" >&2; exit 1; }

# shellcheck disable=SC2086
if [ "$as" = ci ]; then remote=secrets-install; else remote="sudo /usr/local/sbin/df-secrets-install"; fi
# shellcheck disable=SC2086
tar -C "$from_dir" -cf - "${send[@]}" | ssh -T -o RequestTTY=no -o ClearAllForwardings=yes ${SSH_OPTS:-} "$host" $remote
echo "pushed ${#send[@]} file(s) to $host"
