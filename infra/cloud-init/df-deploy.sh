#!/bin/bash
# df-deploy: the ONLY way a person or CI starts containers or runs database administration from an application
# or worker host (critic rounds 1c, 1d and 1e). Installed root-owned (0755) at /usr/local/sbin/df-deploy by cloud-init.
# Nobody who logs in is in the docker group (docker socket access is host root: it can bind-mount /run/dungeonflux
# and read every consumer's secrets); the accounts may only run this script and df-secrets-install through sudo,
# with argument patterns fixed in /etc/sudoers.d/dungeonflux-deploy, and this script validates every argument again.
#
#   sudo df-deploy release <sha256:...>   STAGE a release (CI only, through the forced command df-deploy-ssh). The digest is that of the
#   sudo df-deploy <sha256:...>           signed release image <prefix>-release (a scratch image holding release.lock, the production
#                                         compose file, the edge files and the config templates). The tool verifies it with `cosign
#                                         verify` against the PINNED certificate identity (one workflow file on refs/heads/main or a
#                                         tag; never a pattern) and OIDC issuer from /etc/dungeonflux/cosign-identity, then verifies
#                                         each image the lock lists the same way, pulls by digest, renders the config templates, and
#                                         writes /opt/dungeonflux/releases/<digest> (including its own allowed-images) and the `staged`
#                                         pointer. It changes NOTHING that is live: not the allow-list, not `current`.
#   sudo df-deploy activate <sha256:...>  PROMOTE the staged release and START it: helpers first (slot tree, egress rules), then the live
#                                         allow-list (/etc/dungeonflux/allowed-images.d/allowed, replaced by rename) and the `current`
#                                         pointer change together, LAST; `previous` records the old release; df-stack brings the stack up
#                                         in order, gates the control plane's readiness and soaks it (the canary). A failed gate rolls
#                                         back to the previous release automatically: exit 8 = rolled back and healthy, 9 = the rollback
#                                         failed too (a person must look). The first activation has nothing to go back to: the stack is
#                                         stopped.
#   sudo df-deploy rollback               go back to `previous` on purpose (swaps the two pointers; no automatic fallback).
#   sudo df-deploy refresh-keys <sha256:...>
#                                         the SSH key set (CI key and one FIDO2 key per person): the digest of the signed image
#                                         <prefix>-keys. Verified exactly like a release, unpacked, and handed to df-keys-apply,
#                                         which validates it and rewrites /etc/ssh/authorized_keys.d, sudoers and AllowUsers. This is
#                                         the tool whose absence was the launch gate: keys are revoked or changed on a RUNNING host
#                                         only through a signed, reviewed key set, never by hand and never by Terraform.
#   sudo df-deploy psql --host H --port P --dbname D --user U
#                                         stdin: line 1 = the database administrator password, the rest = SQL.
#                                         Runs the pinned one-shot psql image (the allow-list, key "psql") on a read-only,
#                                         capability-free container whose ONLY way out is a host-side forwarder to the
#                                         database (see below); the password goes to a pgpass file on the container's tmpfs
#                                         and never touches a command line, an environment variable, `docker inspect` or
#                                         the host disk.
#
# THE psql CONTAINER'S NETWORK AND THE FORWARDER. The container runs on a dedicated INTERNAL docker network (df-psql: no
# default route, no internet, no metadata endpoint, no other VPC host). The only thing it can reach is a socat forwarder
# this script starts on the network's gateway address, relaying to the database. So the forwarder is the whole attack
# surface, and it is locked down (critic round 1e):
#   * the target host must END WITH the root-owned suffix in /etc/dungeonflux/db-host-suffix (cloud-init writes
#     .db.ondigitalocean.com), must be a NAME (IP literals are refused), and the port must be 25060 or 25061; the name is
#     resolved ONCE and the forwarder relays to that address (a link-local, loopback or multicast answer is refused), so
#     `--host 169.254.169.254` or any internet host is impossible;
#   * the whole command holds the one flock /run/dungeonflux/deploy.lock (a second run is refused, never silently sharing the first
#     run's forwarder), the forwarder must be alive and LISTENING before the container starts, and the run fails if it dies;
#   * an INPUT chain drop for the df-psql bridge on everything except tcp/5432 to the gateway address, so the container
#     cannot reach sshd or any other host service listening on all addresses.
# THE CA FILE. The database CA is public data but lives in a root:dfs-control-plane 0440 file; this script copies it to a
# root-created 0444 temporary file on tmpfs and mounts only that, so the container needs no group and cannot read any secret.
#
# Until release verification exists, the deploy SSH key is ROOT-EQUIVALENT in effect (it can drive this tool and the
# secrets installer) and must be handled as such; see the manifest entry ssh_deploy_key.
# STATUS: UNVERIFIED (never run as root; bash -n only). Unverified: that a host process can bind an internal bridge's
# gateway address on the pinned Docker version, the bridge name br-<12 id characters>, and libpq hostaddr with host.
set -euo pipefail

# The live allow-list lives in its OWN DIRECTORY and is replaced by rename: containers mount the directory, so a new file is seen at once
# (a bind-mounted single file would keep showing the old inode, and a revoked digest would stay startable).
# Test mode (non-root only): DF_DEPLOY_TEST_PREFIX=<dir> moves every path under that directory and DF_DEPLOY_TEST_SBIN=<dir> names the
# directory holding stand-ins for the helper tools, so the activation state machine can be exercised without root or Docker
# (deploy/scripts/tests/test-df-stack.sh). Both are refused as root.
if [ "$(id -u)" -eq 0 ]; then
  [ -z "${DF_DEPLOY_TEST_PREFIX:-}${DF_DEPLOY_TEST_SBIN:-}" ] || { echo "the test options are refused as root" >&2; exit 2; }
  P=""
  sbin=/usr/local/sbin
elif [ -n "${DF_DEPLOY_TEST_PREFIX:-}" ] && [ -n "${DF_DEPLOY_TEST_SBIN:-}" ]; then
  P=$DF_DEPLOY_TEST_PREFIX
  sbin=$DF_DEPLOY_TEST_SBIN
else
  echo "df-deploy must run through sudo" >&2
  exit 2
fi
etc=$P/etc/dungeonflux
opt=$P/opt/dungeonflux
run=$P/run/dungeonflux

allow_dir=$etc/allowed-images.d
allowed=$allow_dir/allowed
identity_file=$etc/cosign-identity
source_file=$etc/release-source
env_file=$run/secrets/deploy/release_environment
suffix_file=$etc/db-host-suffix
lock_file=$run/deploy.lock
revoked_file=$etc/revoked-images
export DOCKER_CONFIG=$run/docker

digest_re='^sha256:[0-9a-f]{64}$'
name_re='^[A-Za-z0-9_.-]{1,64}$'
host_re='^[A-Za-z0-9.-]{1,253}$'
port_re='^[0-9]{1,5}$'
psql_net=df-psql
psql_subnet=172.29.250.0/29
psql_gateway=172.29.250.1

pinned() { # pinned FILE KEY -> the value of KEY= in a root-owned file
  local file=$1 key=$2 value
  [ -r "$file" ] || { echo "no $file" >&2; exit 4; }
  value=$(sed -n "s/^${key}=//p" "$file" | head -n 1)
  [ -n "$value" ] || { echo "no $key in $file" >&2; exit 4; }
  printf '%s' "$value"
}

# Verify a registry reference BY DIGEST against the pinned CI identity and issuer. KIND is the key in the pinned file: `identity` for
# releases and the images they list, `keys_identity` for key sets (a different workflow, behind a GitHub environment with required
# reviewers: a key set is root-equivalent). The identity is compared exactly, and it must name ONE workflow file on the main branch
# or on a release tag; a pattern, another host or a pull-request ref is refused here, before cosign is run.
verify_ref() {
  local ref=$1 kind=${2:-identity} identity issuer
  [[ "$ref" =~ @sha256:[0-9a-f]{64}$ ]] || { echo "refusing to verify a reference without a digest: $ref" >&2; exit 2; }
  command -v cosign >/dev/null 2>&1 || { echo "cosign is not installed" >&2; exit 4; }
  identity=$(pinned "$identity_file" "$kind")
  issuer=$(pinned "$identity_file" issuer)
  [[ "$identity" =~ ^https://github\.com/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/\.github/workflows/[A-Za-z0-9_.-]+\.ya?ml@(refs/heads/main|refs/tags/[A-Za-z0-9_.-]+)$ ]] \
    || { echo "the pinned $kind must be https://github.com/OWNER/REPO/.github/workflows/FILE.yml@refs/heads/main (or @refs/tags/TAG)" >&2; exit 4; }
  [[ "$issuer" == "https://token.actions.githubusercontent.com" ]] || { echo "the pinned issuer must be GitHub Actions' OIDC issuer" >&2; exit 4; }
  if ! cosign verify --certificate-identity "$identity" --certificate-oidc-issuer "$issuer" "$ref" >/dev/null 2>&1; then
    echo "SIGNATURE VERIFICATION FAILED for $ref (identity $identity, issuer $issuer)" >&2
    exit 7
  fi
}

# Log in to the registry when the packages are private: the pull token (manifest entry registry_pull_token, consumer deploy) comes
# from tmpfs and the login lands in DOCKER_CONFIG on tmpfs, so it never reaches the Droplet disk or its backups. No token, no login.
registry_login() { # registry_login PREFIX
  local token_file=$run/secrets/deploy/registry_pull_token user host=${1%%/*}
  [ -r "$token_file" ] || return 0
  user=$(pinned "$source_file" registry_user)
  docker login "$host" --username "$user" --password-stdin < "$token_file" >/dev/null 2>&1 \
    || { echo "registry login to $host failed" >&2; exit 4; }
}

# Copy a directory out of a (scratch) image without running it.
extract_dir() { # extract_dir REF SRC DEST
  local ref=$1 src=$2 dest=$3 cid
  docker pull --quiet "$ref" >/dev/null
  cid=$(docker create "$ref" /noop)
  docker cp "$cid:$src" "$dest" >/dev/null || { docker rm "$cid" >/dev/null; echo "cannot copy $src out of $ref" >&2; exit 4; }
  docker rm "$cid" >/dev/null
}

stage_dir() { mktemp -d "$run/docker/stage.XXXXXX"; }

# Render the release's config TEMPLATES. Files hold @NAME@ tokens (upper case, digits, underscore); the values are the
# NON-SECRET lines KEY=VALUE of the release environment (consumer "deploy" of df-secrets-install: root-only, never mounted by
# a container). The value alphabet is fixed so a value can never carry sed or TOML syntax, and the result must leave no token.
render_config() { # render_config SRC_DIR DEST_DIR ENV_FILE
  local src=$1 dest=$2 envf=$3 line key val n=0 sedscript f base out
  [ -r "$envf" ] || { echo "no $envf: push the release environment first (df-secrets-install, consumer deploy)" >&2; exit 4; }
  sedscript=$(mktemp "$dest.sed.XXXXXX" 2>/dev/null || mktemp)
  while IFS= read -r line || [ -n "$line" ]; do
    n=$((n + 1))
    [[ -z "$line" || "$line" == \#* ]] && continue
    if [[ "$line" =~ ^([A-Z][A-Z0-9_]{0,63})=([A-Za-z0-9._:/-]{1,253})$ ]]; then
      key=${BASH_REMATCH[1]}
      val=${BASH_REMATCH[2]}
      printf 's|@%s@|%s|g\n' "$key" "$val" >> "$sedscript"
    else
      rm -f "$sedscript"
      echo "release environment line $n is not KEY=VALUE with the safe alphabet [A-Za-z0-9._:/-]" >&2
      exit 4
    fi
  done < "$envf"
  mkdir -p "$dest"
  for f in "$src"/*; do
    [ -f "$f" ] && [ ! -L "$f" ] || { rm -f "$sedscript"; echo "unexpected entry in the release config directory: $f" >&2; exit 4; }
    base=$(basename "$f")
    [[ "$base" =~ ^[a-z0-9][a-z0-9._-]*$ ]] || { rm -f "$sedscript"; echo "bad config file name: $base" >&2; exit 4; }
    out="$dest/$base"
    sed -f "$sedscript" "$f" > "$out"
    if grep -q '@[A-Z][A-Z0-9_]*@' "$out"; then
      rm -f "$sedscript"
      echo "$base still has an unfilled token: $(grep -o '@[A-Z][A-Z0-9_]*@' "$out" | sort -u | tr '\n' ' ')" >&2
      exit 4
    fi
    chmod 0644 "$out"
  done
  rm -f "$sedscript"
}

release() {
  [ "$#" -eq 1 ] || { echo "release takes exactly one digest" >&2; exit 2; }
  local digest=$1 prefix ref tmp key line image name expected
  [[ "$digest" =~ $digest_re ]] || { echo "not a sha256 digest: $digest" >&2; exit 2; }
  prefix=$(pinned "$source_file" prefix)
  registry_login "$prefix"
  ref="$prefix-release@$digest"
  verify_ref "$ref" identity
  tmp=$(stage_dir)
  trap 'rm -rf "$tmp"' EXIT
  extract_dir "$ref" /release "$tmp/release"
  for f in release.lock production.compose.yml production.env edge/Caddyfile.production \
           config/control-plane.toml config/egress.toml config/ingestor.toml config/runner.toml config/dbgate.cfg \
           config/prometheus.yml config/alert-rules.yml config/alertmanager.yml config/migrate.toml; do
    [ -s "$tmp/release/$f" ] || { echo "the release image lacks $f" >&2; exit 4; }
  done
  # Fill the @NAME@ tokens of the config templates from the non-secret release environment. A missing or unsafe value fails the
  # release HERE, before anything is pulled, allow-listed or staged.
  check_revoked_format "$tmp/release/revoked-images"
  render_config "$tmp/release/config" "$tmp/rendered" "$env_file"

  # Every image the release starts must be listed by digest, from the pinned repository family, and verified itself. The one
  # exception is the haproxy database forwarder, trusted through the signed release.lock and pulled by digest.
  declare -A refs
  declare -a third_party=()
  declare -A third_seen=()
  while IFS= read -r line; do
    [[ -z "$line" || "$line" == \#* ]] && continue
    key=${line%%=*}
    image=${line#*=}
    case "$key" in
      CONTROL_PLANE_IMAGE) name=control-plane ;;
      CAMPAIGN_RUNTIME_IMAGE) name=campaign-runtime ;;
      EGRESS_IMAGE) name=egress ;;
      RUNNER_IMAGE) name=runner ;;
      DOCKER_PROXY_IMAGE) name=docker-proxy ;;
      EDGE_IMAGE) name=edge ;;
      BACKUP_IMAGE) name=backup ;;
      DBGATE_IMAGE|PROMETHEUS_IMAGE|ALERTMANAGER_IMAGE|NODE_EXPORTER_IMAGE)
        [[ "$image" =~ ^[a-z0-9./_-]+:[A-Za-z0-9._-]+@sha256:[0-9a-f]{64}$ && "$image" != *TODO* ]] \
          || { echo "release.lock: $key must be repo:tag@sha256:<digest>" >&2; exit 4; }
        [ -z "${third_seen[$key]:-}" ] || { echo "release.lock lists $key twice" >&2; exit 4; }
        third_seen[$key]=1; third_party+=("$image"); continue ;;
      *) echo "release.lock has an unknown key: $key" >&2; exit 4 ;;
    esac
    expected="$prefix-$name:"
    [[ "$image" == "$expected"* && "$image" =~ @sha256:[0-9a-f]{64}$ && "$image" != *TODO* ]] \
      || { echo "release.lock: $key must be $expected<version>@sha256:<digest>" >&2; exit 4; }
    [ -z "${refs[$name]:-}" ] || { echo "release.lock lists $key twice" >&2; exit 4; }
    refs[$name]=$image
  done < "$tmp/release/release.lock"
  for key in DBGATE_IMAGE PROMETHEUS_IMAGE ALERTMANAGER_IMAGE NODE_EXPORTER_IMAGE; do
    [ -n "${third_seen[$key]:-}" ] || { echo "release.lock must list $key" >&2; exit 4; }
  done
  for name in control-plane campaign-runtime egress runner docker-proxy edge backup; do
    [ -n "${refs[$name]:-}" ] || { echo "release.lock does not list $name" >&2; exit 4; }
    verify_ref "${refs[$name]}" identity
    docker pull --quiet "${refs[$name]}" >/dev/null
  done
  for image in "${third_party[@]}"; do docker pull --quiet "$image" >/dev/null; done

  # STAGE ONLY. Nothing below touches the live allow-list or the `current` pointer: the running proxy keeps allowing exactly what it
  # allowed, and anything that re-creates a container keeps taking the active release's files, until `activate` promotes both.
  # The release directory is content-addressed; an existing one is never rewritten (it may be the active release).
  local dest=$opt/releases/${digest#sha256:}
  if [ -e "$dest" ]; then
    echo "release $digest is already present at $dest; not rewritten"
  else
    rm -rf "$dest.new"
    install -d -m 0755 -o root -g root "$dest.new"
    cp -a "$tmp/release/." "$dest.new/"
    rm -rf "$dest.new/config"
    cp -a "$tmp/rendered" "$dest.new/config"
    {
      for name in control-plane campaign-runtime egress runner docker-proxy edge backup; do echo "$name=${refs[$name]}"; done
    } > "$dest.new/allowed-images"
    # Host facts the compose file needs and the release cannot know: the docker group's id (the proxy reaches the daemon through
    # it), and the number of runtime slot bridges the firewall rules must cover.
    local gid
    gid=$(getent group docker | cut -d: -f3)
    [[ "$gid" =~ ^[0-9]+$ ]] || { echo "no docker group on this host" >&2; exit 4; }
    local slots
    slots=$(grep -c '^\[\[slots\]\]' "$dest.new/config/runner.toml" || true)
    [[ "$slots" =~ ^[0-9]+$ ]] && [ "$slots" -ge 1 ] || { echo "runner.toml lists no [[slots]]" >&2; exit 4; }
    # The database NAMES the firewall resolves (the primary, and a standby host if the provider exposes one): the rules allow tcp 25060-25061
    # to exactly their addresses, refreshed every 30 s.
    local dbhosts standby
    dbhosts=$(sed -n 's/^DB_HOST=//p' "$env_file" | head -n 1)
    standby=$(sed -n 's/^DB_STANDBY_HOST=//p' "$env_file" | head -n 1)
    [[ "$dbhosts" =~ ^[A-Za-z0-9.-]{1,253}$ ]] || { echo "DB_HOST is missing or malformed in the release environment" >&2; exit 4; }
    if [ -n "$standby" ]; then
      [[ "$standby" =~ ^[A-Za-z0-9.-]{1,253}$ ]] || { echo "DB_STANDBY_HOST is malformed" >&2; exit 4; }
      dbhosts="$dbhosts,$standby"
    fi
    printf 'DF_DOCKER_GID=%s\nDF_SLOTS=%s\nDF_DB_HOSTS=%s\n' "$gid" "$slots" "$dbhosts" > "$dest.new/host.env"
    chown -R root:root "$dest.new"
    mv -T "$dest.new" "$dest"
  fi
  swap_link "$opt/staged" "$dest"
  echo "release $digest verified and STAGED at $dest; NOT PROMOTED (df-deploy activate), NOT STARTED"
}

# Replace a symlink by a rename, never by remove-then-create.
# Pointers are symlinks. In test mode (no root, a sandbox that may not allow symlinks) they are one-line files; only these three tiny
# functions differ, everything else is the code that runs for real.
ptr_get() { if [ -n "$P" ]; then cat "$1" 2>/dev/null || true; else readlink "$1" 2>/dev/null || true; fi; }
swap_link() { # swap_link LINK TARGET
  if [ -n "$P" ]; then printf '%s' "$2" > "$1.new"; mv -f "$1.new" "$1"; return 0; fi
  ln -sfn "$2" "$1.new"
  mv -T "$1.new" "$1"
}

# REVOCATION. A signed release may carry `revoked-images`: image digests (sha256:<64 hex>, one per line) that must never be allowed again,
# normally the campaign-runtime digests of releases found to be bad. The host keeps the union of every revocation it has ever seen in the
# root-owned append-only-by-this-tool /etc/dungeonflux/revoked-images, and EVERY allow-list this tool writes (activate, automatic
# rollback, manual rollback) is filtered through it: a release or a rollback target that lists a revoked digest is refused, never silently
# trimmed. The live allow-list is exactly the active release's own list plus the host's psql line: nothing is carried over from an earlier
# release, so a revoked runtime cannot stay startable. (Running campaigns are not affected by the list; it is read only when a runtime is
# created.) The rollback target's list is its own file in its release directory, outside the live allow-list.
check_revoked_format() { # check_revoked_format FILE
  local line
  [ -f "$1" ] || return 0
  while IFS= read -r line || [ -n "$line" ]; do
    [[ -z "$line" || "$line" == \#* ]] && continue
    [[ "$line" =~ ^sha256:[0-9a-f]{64}$ ]] || { echo "revoked-images has a line that is not a sha256 digest" >&2; exit 4; }
  done < "$1"
}

record_revocations() { # record_revocations RELEASE_DIR: add the release's revocations to the host's
  local src=$1/revoked-images
  check_revoked_format "$src"
  [ -s "$src" ] || return 0
  { cat "$revoked_file" 2>/dev/null || true; grep -E '^sha256:[0-9a-f]{64}$' "$src" || true; } | sort -u > "$revoked_file.new"
  chmod 0644 "$revoked_file.new"
  mv -f "$revoked_file.new" "$revoked_file"
}

filter_revoked() { # filter_revoked SRC: SRC without any line that names a revoked digest
  if [ -s "$revoked_file" ]; then grep -vF -f "$revoked_file" "$1" || true; else cat "$1"; fi
}

# check_allowed_clean RELEASE_DIR: succeeds only if the release's own allow-list names no revoked image and lists a campaign-runtime.
check_allowed_clean() {
  local rel=$1 wanted got
  wanted=$(grep -c . "$rel/allowed-images" || true)
  got=$(filter_revoked "$rel/allowed-images" | grep -c . || true)
  [ "$wanted" = "$got" ] || { echo "REFUSED: $rel lists an image that has been revoked" >&2; return 4; }
  grep -q '^campaign-runtime=' "$rel/allowed-images" || { echo "REFUSED: $rel lists no campaign-runtime image" >&2; return 4; }
}

# write_allowed RELEASE_DIR: the live allow-list for that release: its own list plus the host's psql line (checked first).
write_allowed() {
  local rel=$1
  check_allowed_clean "$rel" || return 4
  {
    cat "$rel/allowed-images"
    sed -n 's/^psql=/psql=/p' "$allowed" 2>/dev/null || true
  } > "$allow_dir/allowed.new"
  chmod 0644 "$allow_dir/allowed.new"
  if [ -z "$P" ]; then chown root:root "$allow_dir/allowed.new"; fi
  mv -f "$allow_dir/allowed.new" "$allowed"
}

# The allow-list with NO runtime image: only the host's psql client line. Written when nothing may start (first activation failed, or a
# rollback was refused).
psql_only_allowed() {
  sed -n 's/^psql=/psql=/p' "$allowed" 2>/dev/null > "$allow_dir/allowed.new" || true
  chmod 0644 "$allow_dir/allowed.new"
  mv -f "$allow_dir/allowed.new" "$allowed"
}

# Make a staged (or previous) release the live one AND start it. EXIT CODES: 0 activated and healthy; 8 the new release failed its gates and
# the previous one is running again; 9 it failed and the previous one could not be brought back (or was refused because it is revoked): the
# failed stack is stopped where that applies and a person must act; 10 the first activation failed: the stack is stopped and NO release is
# current; 11 a retry of the release that was already current failed (not a rollback: the stack is as it is); other non-zero: refused before
# anything changed.
#
# Order: (1) VALIDATE EVERYTHING first: the release's revocations are recorded, its allow-list is checked against them, the idempotent
# helpers run (slot tree, firewall rules); (2) only then the change: the allow-list, then `previous` (unless it is the same release), then
# `current` LAST; (3) df-stack brings the new release up in order, gates the control plane's readiness (the canary) and soaks it;
# (4) on a failed gate the previous release is put back (pointers, its OWN allow-list, helpers) and started with the same gates; with
# nothing to go back to the stack is stopped. Database migrations are expand-then-contract, so the previous binary still runs on the
# migrated schema.
switch_to() { # switch_to DEST AUTO_ROLLBACK(1|0)
  local dest=$1 auto=$2 cur oldprev
  [ -s "$dest/allowed-images" ] && [ -s "$dest/host.env" ] && [ -s "$dest/production.compose.yml" ] \
    || { echo "the release at $dest is incomplete" >&2; exit 4; }
  cur=$(ptr_get "$opt/current")
  oldprev=$(ptr_get "$opt/previous")

  # --- 1. VALIDATE EVERYTHING before any pointer or the live allow-list is touched ---
  record_revocations "$dest"
  check_allowed_clean "$dest" || exit 4
  "$sbin/df-slot-dirs" --config "$dest/config/runner.toml"
  "$sbin/df-net-rules" --from "$dest/host.env"

  # --- 2. THE CHANGE: the allow-list, then `previous`, then `current` LAST ---
  write_allowed "$dest" || exit 4
  # Re-activating the current release must not overwrite `previous` with itself (it would destroy the rollback target).
  if [ -n "$cur" ] && [ "$cur" != "$dest" ]; then swap_link "$opt/previous" "$cur"; fi
  swap_link "$opt/current" "$dest"

  if "$sbin/df-stack" up "$dest"; then
    echo "release $dest ACTIVATED and healthy (previous: ${cur:-none})"
    return 0
  fi
  echo "ACTIVATION OF $dest FAILED its health gates" >&2
  if [ "$auto" != 1 ]; then echo "no automatic rollback was requested; the stack may be unhealthy" >&2; exit 9; fi

  # --- 3. AUTOMATIC ROLLBACK ---
  if [ "$cur" = "$dest" ]; then
    # A retry of the release that was already current: there is nothing older to go back to, and `previous` is untouched.
    echo "RETRY OF THE CURRENT RELEASE FAILED: $dest is still current and the stack is as it is (possibly unhealthy); this was not a rollback" >&2
    exit 11
  fi
  if [ -n "$oldprev" ]; then swap_link "$opt/previous" "$oldprev"; else rm -f "$opt/previous"; fi
  if [ -z "$cur" ]; then
    rm -f "$opt/current"
    psql_only_allowed
    "$sbin/df-stack" down "$dest" || true
    echo "FIRST ACTIVATION FAILED: nothing to roll back to; the stack was stopped and no release is current" >&2
    exit 10
  fi
  if ! check_allowed_clean "$cur" || ! write_allowed "$cur"; then
    # The failed release's revocations cover the release that was running: it must not be started again. Stop what is half-started, leave
    # no runtime image allowed, and say so. A person decides.
    rm -f "$opt/current"
    psql_only_allowed
    "$sbin/df-stack" down "$dest" || true
    echo "ROLLBACK TO $cur REFUSED: its images are revoked. The failed stack was stopped, no release is current and the allow-list holds only the psql client. A person must act." >&2
    exit 9
  fi
  swap_link "$opt/current" "$cur"
  "$sbin/df-slot-dirs" --config "$cur/config/runner.toml" || true
  "$sbin/df-net-rules" --from "$cur/host.env" || true
  if "$sbin/df-stack" up "$cur"; then
    echo "ROLLED BACK to $cur; it is up and healthy" >&2
    exit 8
  fi
  echo "ROLLBACK TO $cur FAILED: the stack is down or unhealthy and needs a person" >&2
  exit 9
}

activate() {
  [ "$#" -eq 1 ] || { echo "activate takes exactly one digest" >&2; exit 2; }
  local digest=$1 dest staged
  [[ "$digest" =~ $digest_re ]] || { echo "not a sha256 digest: $digest" >&2; exit 2; }
  dest=$opt/releases/${digest#sha256:}
  staged=$(ptr_get "$opt/staged")
  [ "$staged" = "$dest" ] || { echo "$digest is not the staged release (staged: ${staged:-none})" >&2; exit 4; }
  switch_to "$dest" 1
}

# Go back to the release recorded as `previous` (a manual rollback after a release that passed its gates turned out bad). The target's
# own allow-list is used, filtered by every revocation the host has seen, so it cannot bring back a revoked runtime. It swaps the two
# pointers, so running it twice returns to where you started. No automatic fallback: a person asked for it.
rollback() {
  [ "$#" -eq 0 ] || { echo "rollback takes no argument" >&2; exit 2; }
  local prev
  prev=$(ptr_get "$opt/previous")
  [ -n "$prev" ] && [ -d "$prev" ] || { echo "there is no previous release to go back to" >&2; exit 4; }
  switch_to "$prev" 0
}

refresh_keys() {
  [ "$#" -eq 1 ] || { echo "refresh-keys takes exactly one digest" >&2; exit 2; }
  local digest=$1 prefix ref tmp
  [[ "$digest" =~ $digest_re ]] || { echo "not a sha256 digest: $digest" >&2; exit 2; }
  prefix=$(pinned "$source_file" prefix)
  registry_login "$prefix"
  ref="$prefix-keys@$digest"
  verify_ref "$ref" keys_identity
  tmp=$(stage_dir)
  trap 'rm -rf "$tmp"' EXIT
  extract_dir "$ref" /keys "$tmp/keys"
  "$sbin/df-keys-apply" --from "$tmp/keys"
}

allowed_image() { # allowed_image KEY -> image reference with a digest from the root-owned allow-list
  local key=$1 ref
  [ -f "$allowed" ] || { echo "no $allowed" >&2; exit 4; }
  ref=$(sed -n "s/^${key}=//p" "$allowed" | head -n 1)
  [[ "$ref" =~ @sha256:[0-9a-f]{64}$ ]] || { echo "no digest-pinned '$key' image in $allowed" >&2; exit 4; }
  printf '%s' "$ref"
}

# Resolve the (already allow-listed) database name ONCE; refuse anything that is not an ordinary unicast address.
resolve_once() {
  local host=$1 ip
  ip=$(getent ahostsv4 "$host" | awk 'NR == 1 { print $1 }')
  [[ "$ip" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "cannot resolve $host to an IPv4 address" >&2; exit 4; }
  case "$ip" in
    0.*|127.*|169.254.*|22[4-9].*|23[0-9].*|24[0-9].*|25[0-5].*) echo "refusing to relay to $ip" >&2; exit 4 ;;
  esac
  printf '%s' "$ip"
}

# --- the database forwarder, shared by `psql` and `migrate` ---------------------------------------------------------------------------------
# The relay target is a database NAME under the root-owned suffix, on a managed-database port. The container's network (df-psql) is
# internal; its only way out is a socat forwarder this script starts on the network's gateway address; an INPUT drop for that bridge
# lets the container reach only tcp/5432 of the gateway address. Sets: fw_tmp (a root-made temp directory, removed on exit), forwarder.
fw_tmp=""
forwarder=""
db_forwarder_up() { # db_forwarder_up HOST PORT
  local host=$1 port=$2 suffix ca target bridge net_id
  [[ "$host" =~ $host_re ]] && [[ "$port" =~ $port_re ]] || { echo "bad database host or port" >&2; exit 2; }
  [[ "$host" =~ ^[0-9.]+$ ]] && { echo "an IP literal is not allowed as the database host" >&2; exit 2; }
  [ -r "$suffix_file" ] || { echo "no $suffix_file" >&2; exit 4; }
  suffix=$(head -n 1 "$suffix_file" | tr -d '[:space:]')
  [[ "$suffix" =~ ^\.[a-z0-9.-]+\.[a-z]+$ ]] || { echo "bad suffix in $suffix_file" >&2; exit 4; }
  [[ "$host" == *"$suffix" && "$host" != "$suffix" ]] || { echo "the database host must end with $suffix" >&2; exit 2; }
  { [ "$port" = 25060 ] || [ "$port" = 25061 ]; } || { echo "the database port must be 25060 or 25061" >&2; exit 2; }

  ca=$run/secrets/control-plane/db_ca_certificate
  [ -r "$ca" ] || { echo "database CA certificate not installed ($ca)" >&2; exit 4; }
  command -v socat >/dev/null 2>&1 || { echo "socat is not installed" >&2; exit 4; }
  command -v iptables >/dev/null 2>&1 || { echo "iptables is not installed" >&2; exit 4; }
  target=$(resolve_once "$host")

  fw_tmp=$(mktemp -d "$run/docker/fw.XXXXXX")
  chmod 0755 "$fw_tmp"
  install -m 0444 "$ca" "$fw_tmp/ca.crt"
  docker network inspect "$psql_net" >/dev/null 2>&1 \
    || docker network create --internal --driver bridge --subnet "$psql_subnet" --gateway "$psql_gateway" "$psql_net" >/dev/null
  trap 'kill "$forwarder" 2>/dev/null || true; rm -rf "$fw_tmp"' EXIT

  # The container may reach the host on tcp/5432 of the gateway address (the forwarder) and nothing else: no sshd, no other
  # service listening on all addresses. DROP first and ACCEPT above it, both at the top of INPUT, idempotently.
  net_id=$(docker network inspect -f '{{.Id}}' "$psql_net")
  bridge="br-${net_id:0:12}"
  iptables -C INPUT -i "$bridge" -j DROP 2>/dev/null || iptables -I INPUT 1 -i "$bridge" -j DROP
  iptables -C INPUT -i "$bridge" -p tcp -d "$psql_gateway" --dport 5432 -j ACCEPT 2>/dev/null \
    || iptables -I INPUT 1 -i "$bridge" -p tcp -d "$psql_gateway" --dport 5432 -j ACCEPT

  socat -T 300 "TCP-LISTEN:5432,bind=$psql_gateway,reuseaddr,fork,max-children=2" "TCP:$target:$port" 9>&- &
  forwarder=$!
  local waited=0
  until (exec 3<> "/dev/tcp/$psql_gateway/5432") 2>/dev/null; do
    kill -0 "$forwarder" 2>/dev/null || { echo "the forwarder died while starting" >&2; exit 6; }
    waited=$((waited + 1))
    [ "$waited" -le 25 ] || { echo "the forwarder did not start listening" >&2; exit 6; }
    sleep 0.2
  done
  kill -0 "$forwarder" 2>/dev/null || { echo "the forwarder died" >&2; exit 6; }
}

run_psql() {
  local host="" port="" dbname="" user="" image
  while [ $# -gt 0 ]; do
    case "$1" in
      --host) host=$2; shift 2 ;;
      --port) port=$2; shift 2 ;;
      --dbname) dbname=$2; shift 2 ;;
      --user) user=$2; shift 2 ;;
      *) echo "unknown psql argument: $1" >&2; exit 2 ;;
    esac
  done
  [[ "$dbname" =~ $name_re ]] && [[ "$user" =~ $name_re ]] || { echo "bad --dbname/--user value" >&2; exit 2; }
  image=$(allowed_image psql)
  db_forwarder_up "$host" "$port"

  # shellcheck disable=SC2016
  docker run --rm -i --pull=missing --network "$psql_net" --read-only --cap-drop ALL --security-opt no-new-privileges \
    --pids-limit 64 --memory 256m --user 999:999 \
    --tmpfs /pgpw:rw,mode=0700,uid=999,gid=999,size=1m \
    --volume "$fw_tmp/ca.crt:/ca.crt:ro" \
    --entrypoint sh "$image" -c '
      umask 077
      IFS= read -r pw
      pw=$(printf "%s" "$pw" | sed "s/\\\\/\\\\\\\\/g; s/:/\\\\:/g")
      printf "*:*:*:%s:%s\n" "$1" "$pw" > /pgpw/pgpass
      unset pw
      export PGPASSFILE=/pgpw/pgpass
      exec psql -X --no-psqlrc -v ON_ERROR_STOP=1 "host=$2 hostaddr=$5 port=5432 dbname=$4 user=$1 sslmode=verify-full sslrootcert=/ca.crt"
    ' sh "$user" "$host" "$port" "$dbname" "$psql_gateway"
}

# df-deploy migrate sha256:<digest>   stdin: line 1 = the df_migrator password.
# Runs `df-server migrate apply` from the STAGED release's control-plane image (verified and pulled at staging) as df_migrator, on THIS host,
# BEFORE the canary. Run it on ONE host per release: migrations are serialised by an advisory lock in the database anyway, but only one
# host should ever try. The password lives for the run in a 0400 file on the host's tmpfs, owned by the container's uid, and is removed on exit.
run_migrate() {
  [ "$#" -eq 1 ] || { echo "migrate takes exactly one digest" >&2; exit 2; }
  local digest=$1 dest staged image host port name pwfile
  [[ "$digest" =~ $digest_re ]] || { echo "not a sha256 digest: $digest" >&2; exit 2; }
  dest=$opt/releases/${digest#sha256:}
  staged=$(ptr_get "$opt/staged")
  [ "$staged" = "$dest" ] || { echo "$digest is not the staged release (staged: ${staged:-none})" >&2; exit 4; }
  [ -s "$dest/config/migrate.toml" ] || { echo "the release has no migrate.toml" >&2; exit 4; }
  image=$(sed -n 's/^control-plane=//p' "$dest/allowed-images" | head -n 1)
  [[ "$image" =~ @sha256:[0-9a-f]{64}$ ]] || { echo "the staged release names no control-plane image" >&2; exit 4; }
  host=$(sed -n 's/^DB_HOST=//p' "$env_file" | head -n 1)
  port=$(sed -n 's/^DB_PORT=//p' "$env_file" | head -n 1)
  name=$(sed -n 's/^DB_NAME=//p' "$env_file" | head -n 1)
  [[ "$name" =~ $name_re ]] || { echo "DB_NAME is missing from the release environment" >&2; exit 4; }
  db_forwarder_up "$host" "$port"
  pwfile=$fw_tmp/db_migrator_password
  ( umask 077; IFS= read -r pw; printf '%s' "$pw" > "$pwfile" )
  [ -s "$pwfile" ] || { echo "no password on stdin" >&2; exit 2; }
  if [ -z "$P" ]; then chown 10009:10009 "$pwfile"; fi
  chmod 0400 "$pwfile"
  docker run --rm --pull=never --network "$psql_net" --read-only --cap-drop ALL --security-opt no-new-privileges \
    --pids-limit 128 --memory 512m --user 10009:10009 --tmpfs /tmp:rw,size=64m,uid=10009,gid=10009 \
    --volume "$dest/config/migrate.toml:/etc/dungeonflux/migrate.toml:ro" \
    --volume "$pwfile:/run/secrets/db_migrator_password:ro" \
    --volume "$fw_tmp/ca.crt:/run/secrets/db_ca_certificate:ro" \
    "$image" migrate apply --config /etc/dungeonflux/migrate.toml
  echo "migrations applied for release $digest"
}

# ONE lock for every mutating command: a release, a promotion, a key refresh and a psql run never overlap (two CI runs would otherwise race
# on the staged pointer, the allow-list and the key files). Children that outlive a command (the psql forwarder) close this descriptor.
exec 9> "$lock_file"
flock -n 9 || { echo "another df-deploy command is running; refusing" >&2; exit 5; }

case "${1:-}" in
  psql) shift; run_psql "$@" ;;
  migrate) shift; run_migrate "$@" ;;
  release) shift; release "$@" ;;
  activate) shift; activate "$@" ;;
  rollback) shift; rollback "$@" ;;
  refresh-keys) shift; refresh_keys "$@" ;;
  sha256:*) release "$@" ;;
  *) echo "usage: df-deploy release <digest> | migrate <digest> | activate <digest> | rollback | refresh-keys <digest> | psql --host H --port P --dbname D --user U" >&2; exit 2 ;;
esac
