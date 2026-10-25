#!/usr/bin/env bash
# Audit the secrets layout. STATUS: UNVERIFIED as a script (jq and a host were not available when written).
# The `repo` mode was run locally. The manifest invariants below were also evaluated against the generated
# manifest by a throwaway node script and hold; they encode LAUNCH.md decisions 18 and 23.
#   deploy/scripts/secrets-audit.sh manifest          check secrets.manifest.json invariants
#   deploy/scripts/secrets-audit.sh repo              scan deploy/ and infra/ for secret-looking content
#   deploy/scripts/secrets-audit.sh host user@host    check /run/dungeonflux/secrets modes, groups and strays over ssh
# Prints names and findings only; never a secret value. Exit 1 on any finding.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
manifest="$root/deploy/secrets/secrets.manifest.json"
findings=0
finding() { echo "FINDING: $*"; findings=$((findings + 1)); }

mode=${1:-}
case "$mode" in
  manifest)
    command -v jq >/dev/null 2>&1 || { echo "missing tool: jq" >&2; exit 1; }
    out=$(jq -r '.secrets | map(.name) | group_by(.) | map(select(length > 1) | .[0]) | .[]' "$manifest")
    [ -z "$out" ] || finding "duplicate names: $out"
    out=$(jq -r '.secrets[] | select((.store_of_record // "") == "" or (.rotation.procedure // "") == "" or (.on_compromise // "") == "") | .name' "$manifest")
    [ -z "$out" ] || finding "entries without store of record, rotation procedure or compromise action: $out"
    out=$(jq -r '. as $m | .secrets[] | select(($m.procedures | has(.rotation.procedure)) | not) | .name' "$manifest")
    [ -z "$out" ] || finding "entries naming an undefined rotation procedure: $out"
    out=$(jq -r '. as $m | .secrets[] | select(.delivery == "host") | . as $s | $s.consumers[] | select(($m.consumers | has(.)) | not) | $s.name + ":" + .' "$manifest")
    [ -z "$out" ] || finding "entries naming an unknown consumer: $out"
    out=$(jq -r '. as $m | .secrets[] | select(.delivery == "host") | . as $s | $s.consumers[] | . as $c | select([$s.hosts[] | . as $h | select($m.consumers[$c].hosts | index($h))] | length == 0) | $s.name + ":" + $c' "$manifest")
    [ -z "$out" ] || finding "entries delivered to a consumer on no host that consumer runs on: $out"
    out=$(jq -r '.secrets[] | select(.delivery != "host" and .delivery != "runner-issued" and (.consumers | length) > 0) | .name' "$manifest")
    [ -z "$out" ] || finding "ci-only, operator-only or terraform-input entries must not name host consumers: $out"
    # Decision 18: provider keys reach the dispatcher and nothing else.
    out=$(jq -r '.secrets[] | select(.name | startswith("provider_")) | select(.consumers != ["dispatcher"]) | .name' "$manifest")
    [ -z "$out" ] || finding "provider keys outside the dispatcher: $out"
    # Decision 18 and 23: the runtime consumer holds one database password, public certificates and runner-issued items only.
    out=$(jq -r '.secrets[] | select(.consumers | index("runtime")) | select((.delivery == "runner-issued") or (.class == "config") or (.name == "db_campaign_runtime_password") | not) | .name' "$manifest")
    [ -z "$out" ] || finding "the runtime consumer is given secrets it must not hold: $out"
    # Decision 18: the dispatcher holds no object-store, identity-minting or payment secret.
    out=$(jq -r '.secrets[] | select(.consumers | index("dispatcher")) | select(.class | IN("storage", "journal", "backup", "payments")) | .name' "$manifest")
    [ -z "$out" ] || finding "the dispatcher is given storage, journal, backup or payment secrets: $out"
    # One consumer, one group, everywhere: the manifest, the installer's group map and the cloud-init groupadd list
    # must agree on every consumer name and gid (critic round 1c, item 9).
    while read -r consumer gid; do
      grep -q "[$consumer]=$gid" "$root/infra/cloud-init/df-secrets-install.sh"         || finding "consumer $consumer gid $gid missing from infra/cloud-init/df-secrets-install.sh"
      grep -q "groupadd, -g, \"$gid\", dfs-$consumer]" "$root/infra/cloud-init/droplet.yaml.tftpl"         || finding "consumer $consumer gid $gid missing from the cloud-init groupadd list"
    done < <(jq -r '.consumers | to_entries[] | .key + " " + (.value.gid | tostring)' "$manifest")
    # The recorded launch gates and recurring operations must be present (critic round 1e rulings).
    out=$(jq -r 'if ([.launch_gates[]? | select(.id == "refresh-keys")] | length) == 0 then "launch gate refresh-keys" else empty end,
                 if ([.recurring_operations[]? | select(.id == "replace-bastion" and .cadence_days <= 30 and (.dependency | length > 0))] | length) == 0 then "recurring replace-bastion (cadence <= 30 days with its permitopen dependency)" else empty end' "$manifest")
    [ -z "$out" ] || finding "missing from the manifest: $out"
    # Operators-only items never reach a host.
    out=$(jq -r '.secrets[] | select(.name | IN("backup_age_identity", "db_admin_password", "db_support_password")) | select(.delivery != "operator-only") | .name' "$manifest")
    [ -z "$out" ] || finding "must be operator-only: $out"
    # Staging may not carry a live key: entries with a staging policy must say so.
    out=$(jq -r '.secrets[] | select(.class | IN("payments")) | select((.staging_policy // "") == "") | .name' "$manifest")
    [ -z "$out" ] || finding "payment entries without a staging_policy: $out"
    ;;
  repo)
    cd "$root"
    patterns='-----BEGIN [A-Z ]*PRIVATE KEY-----|dop_v1_[0-9a-f]{20,}|AKIA[0-9A-Z]{16}|(sk|rk|pk)_(live|test)_[0-9A-Za-z]{16,}|xox[abp]-[0-9A-Za-z-]{10,}|gh[pousr]_[0-9A-Za-z]{30,}|DO00[0-9A-Z]{16,}'
    # File names only: the matching line itself is never printed.
    hits=$(grep -rElI --exclude="secrets-audit.sh" -e "$patterns" deploy infra || true)
    [ -z "$hits" ] || finding "files with secret-looking content:
$hits"
    stray=$(find deploy infra \( -name '*.tfstate' -o -name '*.tfstate.*' -o -name '*.tfvars' -o -name '.env' -o -name '.env.*' -o -name '*.pem' -o -name '*.key' \) -not -path '*/.terraform/*' -print 2>/dev/null || true)
    [ -z "$stray" ] || finding "files that must never be committed:
$stray"
    ;;
  host)
    command -v jq >/dev/null 2>&1 || { echo "missing tool: jq" >&2; exit 1; }
    host=${2:?usage: secrets-audit.sh host user@host}
    # The consumer directories are 0750 root:dfs-<consumer>, unreadable to the operator, so the host side lists them
    # itself: `sudo df-secrets-install --audit` (read-only, root) prints  <type> <mode> <owner:group> <path>  per entry and
    # a docker-config-on-disk line for any registry token on the disk. Each directory must be 750 root:dfs-<consumer>,
    # each file 440 root:dfs-<consumer>; no other directory may exist.
    out=$(ssh -T -o RequestTTY=no -o ClearAllForwardings=yes ${SSH_OPTS:-} "$host" sudo /usr/local/sbin/df-secrets-install --audit) \
      || { finding "cannot run the secrets audit on $host"; out=""; }
    mapfile -t known < <(jq -r '.consumers | keys[]' "$manifest")
    while read -r kind perm owner path; do
      [ -n "${path:-}" ] || continue
      if [ "$kind" = docker-config-on-disk ]; then finding "a docker config.json exists on the Droplet disk: $perm (use DOCKER_CONFIG=/run/dungeonflux/docker)"; continue; fi
      consumer=${path%%/*}
      if ! printf '%s\n' "${known[@]}" | grep -qx "$consumer"; then finding "unknown consumer directory: $path"; continue; fi
      if [ "$kind" = d ]; then
        { [ "$perm" = 750 ] && [ "$owner" = "root:dfs-$consumer" ]; } || finding "$path is $perm $owner, expected 750 root:dfs-$consumer"
      else
        { [ "$perm" = 440 ] && [ "$owner" = "root:dfs-$consumer" ]; } || finding "$path is $perm $owner, expected 440 root:dfs-$consumer"
      fi
    done <<< "$out"
    # The audit tool prints docker-config-on-disk with the path in the second field.
    ;;
  *)
    echo "usage: secrets-audit.sh manifest|repo|host user@host" >&2
    exit 2
    ;;
esac

if [ "$findings" -ne 0 ]; then echo "$findings finding(s)"; exit 1; fi
echo "ok: $mode"
