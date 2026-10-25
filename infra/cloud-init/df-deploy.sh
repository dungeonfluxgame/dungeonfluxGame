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
#   sudo df-deploy activate <sha256:...>  PROMOTE the staged release: the live allow-list (/etc/dungeonflux/allowed-images.d/allowed,
#                                         replaced by rename) and the `current` pointer change together, `previous` records the old
#                                         release, the slot tree and the egress rules are rebuilt. It does NOT start the stack (canary
#                                         and rollback are the next round).
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
#   * the whole function holds flock /run/dungeonflux/psql.lock (a second run is refused, never silently sharing the first
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
allow_dir=/etc/dungeonflux/allowed-images.d
allowed=$allow_dir/allowed
identity_file=/etc/dungeonflux/cosign-identity
source_file=/etc/dungeonflux/release-source
env_file=/run/dungeonflux/secrets/deploy/release_environment
suffix_file=/etc/dungeonflux/db-host-suffix
lock_file=/run/dungeonflux/deploy.lock
export DOCKER_CONFIG=/run/dungeonflux/docker

[ "$(id -u)" -eq 0 ] || { echo "df-deploy must run through sudo" >&2; exit 2; }

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
  local token_file=/run/dungeonflux/secrets/deploy/registry_pull_token user host=${1%%/*}
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

stage_dir() { mktemp -d /run/dungeonflux/docker/stage.XXXXXX; }

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
           config/control-plane.toml config/egress.toml config/ingestor.toml config/runner.toml config/dbgate.cfg; do
    [ -s "$tmp/release/$f" ] || { echo "the release image lacks $f" >&2; exit 4; }
  done
  # Fill the @NAME@ tokens of the config templates from the non-secret release environment. A missing or unsafe value fails the
  # release HERE, before anything is pulled, allow-listed or staged.
  render_config "$tmp/release/config" "$tmp/rendered" "$env_file"

  # Every image the release starts must be listed by digest, from the pinned repository family, and verified itself. The one
  # exception is the haproxy database forwarder, trusted through the signed release.lock and pulled by digest.
  declare -A refs
  declare -a third_party=()
  local third_seen=""
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
      DBGATE_IMAGE)
        [[ "$image" =~ ^[a-z0-9./_-]+:[A-Za-z0-9._-]+@sha256:[0-9a-f]{64}$ && "$image" != *TODO* ]] \
          || { echo "release.lock: $key must be repo:tag@sha256:<digest>" >&2; exit 4; }
        [ -z "$third_seen" ] || { echo "release.lock lists $key twice" >&2; exit 4; }
        third_seen=1; third_party+=("$image"); continue ;;
      *) echo "release.lock has an unknown key: $key" >&2; exit 4 ;;
    esac
    expected="$prefix-$name:"
    [[ "$image" == "$expected"* && "$image" =~ @sha256:[0-9a-f]{64}$ && "$image" != *TODO* ]] \
      || { echo "release.lock: $key must be $expected<version>@sha256:<digest>" >&2; exit 4; }
    [ -z "${refs[$name]:-}" ] || { echo "release.lock lists $key twice" >&2; exit 4; }
    refs[$name]=$image
  done < "$tmp/release/release.lock"
  [ -n "$third_seen" ] || { echo "release.lock must list DBGATE_IMAGE" >&2; exit 4; }
  for name in control-plane campaign-runtime egress runner docker-proxy edge; do
    [ -n "${refs[$name]:-}" ] || { echo "release.lock does not list $name" >&2; exit 4; }
    verify_ref "${refs[$name]}" identity
    docker pull --quiet "${refs[$name]}" >/dev/null
  done
  for image in "${third_party[@]}"; do docker pull --quiet "$image" >/dev/null; done

  # STAGE ONLY. Nothing below touches the live allow-list or the `current` pointer: the running proxy keeps allowing exactly what it
  # allowed, and anything that re-creates a container keeps taking the active release's files, until `activate` promotes both.
  # The release directory is content-addressed; an existing one is never rewritten (it may be the active release).
  local dest=/opt/dungeonflux/releases/${digest#sha256:}
  if [ -e "$dest" ]; then
    echo "release $digest is already present at $dest; not rewritten"
  else
    rm -rf "$dest.new"
    install -d -m 0755 -o root -g root "$dest.new"
    cp -a "$tmp/release/." "$dest.new/"
    rm -rf "$dest.new/config"
    cp -a "$tmp/rendered" "$dest.new/config"
    {
      for name in control-plane campaign-runtime egress runner docker-proxy edge; do echo "$name=${refs[$name]}"; done
    } > "$dest.new/allowed-images"
    # Host facts the compose file needs and the release cannot know: the docker group's id (the proxy reaches the daemon through
    # it), and the database endpoint the egress rules allow.
    local gid dbhost dbport
    gid=$(getent group docker | cut -d: -f3)
    [[ "$gid" =~ ^[0-9]+$ ]] || { echo "no docker group on this host" >&2; exit 4; }
    dbhost=$(sed -n 's/^DB_HOST=//p' "$env_file" | head -n 1)
    dbport=$(sed -n 's/^DB_PORT=//p' "$env_file" | head -n 1)
    printf 'DF_DOCKER_GID=%s\nDF_DB_HOST=%s\nDF_DB_PORT=%s\n' "$gid" "$dbhost" "$dbport" > "$dest.new/host.env"
    chown -R root:root "$dest.new"
    mv -T "$dest.new" "$dest"
  fi
  swap_link /opt/dungeonflux/staged "$dest"
  echo "release $digest verified and STAGED at $dest; NOT PROMOTED (df-deploy activate), NOT STARTED"
}

# Replace a symlink by a rename, never by remove-then-create.
swap_link() { # swap_link LINK TARGET
  ln -sfn "$2" "$1.new"
  mv -T "$1.new" "$1"
}

# Promote the staged release: the allow-list and the `current` pointer change TOGETHER, and the old pointer is recorded as `previous`
# so a rollback has a target. Starting the stack (canary, health gate, rollback) is the next round.
activate() {
  [ "$#" -eq 1 ] || { echo "activate takes exactly one digest" >&2; exit 2; }
  local digest=$1 dest staged cur live_runtime
  [[ "$digest" =~ $digest_re ]] || { echo "not a sha256 digest: $digest" >&2; exit 2; }
  dest=/opt/dungeonflux/releases/${digest#sha256:}
  staged=$(readlink /opt/dungeonflux/staged 2>/dev/null || true)
  [ "$staged" = "$dest" ] || { echo "$digest is not the staged release (staged: ${staged:-none})" >&2; exit 4; }
  [ -s "$dest/allowed-images" ] && [ -s "$dest/host.env" ] || { echo "the staged release is incomplete" >&2; exit 4; }
  cur=$(readlink /opt/dungeonflux/current 2>/dev/null || true)
  # The idempotent helpers come FIRST, so a failure leaves the host exactly as it was: the per-slot tree the runner fills and the proxy
  # checks (one slot per [[slots]] table of the new runner.toml), and the egress rules for the new database address.
  /usr/local/sbin/df-slot-dirs --config "$dest/config/runner.toml"
  /usr/local/sbin/df-net-rules --from "$dest/host.env" --require-db
  # The new live allow-list: the staged images, plus the previous campaign-runtime digest (a rollback and the placements already
  # running keep working), plus the host's psql line.
  live_runtime=$(sed -n 's/^campaign-runtime=//p' "$allowed" 2>/dev/null | head -n 1 || true)
  {
    cat "$dest/allowed-images"
    if [ -n "$live_runtime" ] && ! grep -qxF "campaign-runtime=$live_runtime" "$dest/allowed-images"; then echo "campaign-runtime=$live_runtime"; fi
    sed -n 's/^psql=/psql=/p' "$allowed" 2>/dev/null || true
  } > "$allow_dir/allowed.new"
  chmod 0644 "$allow_dir/allowed.new"
  chown root:root "$allow_dir/allowed.new"
  # Re-activating the current release must not overwrite `previous` with itself (it would destroy the rollback target).
  if [ -n "$cur" ] && [ "$cur" != "$dest" ]; then swap_link /opt/dungeonflux/previous "$cur"; fi
  # The two changes that make the promotion visible go LAST, together: the allow-list, then the pointer.
  mv -f "$allow_dir/allowed.new" "$allowed"
  swap_link /opt/dungeonflux/current "$dest"
  echo "release $digest PROMOTED (previous: ${cur:-none}); the stack is NOT started by this command"
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
  /usr/local/sbin/df-keys-apply --from "$tmp/keys"
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

run_psql() {
  local host="" port="" dbname="" user=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --host) host=$2; shift 2 ;;
      --port) port=$2; shift 2 ;;
      --dbname) dbname=$2; shift 2 ;;
      --user) user=$2; shift 2 ;;
      *) echo "unknown psql argument: $1" >&2; exit 2 ;;
    esac
  done
  [[ "$host" =~ $host_re ]] && [[ "$port" =~ $port_re ]] && [[ "$dbname" =~ $name_re ]] && [[ "$user" =~ $name_re ]] \
    || { echo "bad --host/--port/--dbname/--user value" >&2; exit 2; }

  # The relay target: a database NAME under the root-owned suffix, on a managed-database port.
  [[ "$host" =~ ^[0-9.]+$ ]] && { echo "an IP literal is not allowed as --host" >&2; exit 2; }
  [ -r "$suffix_file" ] || { echo "no $suffix_file" >&2; exit 4; }
  local suffix
  suffix=$(head -n 1 "$suffix_file" | tr -d '[:space:]')
  [[ "$suffix" =~ ^\.[a-z0-9.-]+\.[a-z]+$ ]] || { echo "bad suffix in $suffix_file" >&2; exit 4; }
  [[ "$host" == *"$suffix" && "$host" != "$suffix" ]] || { echo "--host must end with $suffix" >&2; exit 2; }
  { [ "$port" = 25060 ] || [ "$port" = 25061 ]; } || { echo "--port must be 25060 or 25061" >&2; exit 2; }

  local ca=/run/dungeonflux/secrets/control-plane/db_ca_certificate image tmp forwarder="" target bridge net_id
  [ -r "$ca" ] || { echo "database CA certificate not installed ($ca)" >&2; exit 4; }
  image=$(allowed_image psql)
  command -v socat >/dev/null 2>&1 || { echo "socat is not installed" >&2; exit 4; }
  command -v iptables >/dev/null 2>&1 || { echo "iptables is not installed" >&2; exit 4; }
  target=$(resolve_once "$host")

  tmp=$(mktemp -d /run/dungeonflux/docker/psql.XXXXXX)
  chmod 0755 "$tmp"
  install -m 0444 "$ca" "$tmp/ca.crt"
  docker network inspect "$psql_net" >/dev/null 2>&1 \
    || docker network create --internal --driver bridge --subnet "$psql_subnet" --gateway "$psql_gateway" "$psql_net" >/dev/null
  trap 'kill "$forwarder" 2>/dev/null || true; rm -rf "$tmp"' EXIT

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

  # shellcheck disable=SC2016
  docker run --rm -i --pull=missing --network "$psql_net" --read-only --cap-drop ALL --security-opt no-new-privileges \
    --pids-limit 64 --memory 256m --user 999:999 \
    --tmpfs /pgpw:rw,mode=0700,uid=999,gid=999,size=1m \
    --volume "$tmp/ca.crt:/ca.crt:ro" \
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

# ONE lock for every mutating command: a release, a promotion, a key refresh and a psql run never overlap (two CI runs would otherwise race
# on the staged pointer, the allow-list and the key files). Children that outlive a command (the psql forwarder) close this descriptor.
exec 9> "$lock_file"
flock -n 9 || { echo "another df-deploy command is running; refusing" >&2; exit 5; }

case "${1:-}" in
  psql) shift; run_psql "$@" ;;
  release) shift; release "$@" ;;
  activate) shift; activate "$@" ;;
  refresh-keys) shift; refresh_keys "$@" ;;
  sha256:*) release "$@" ;;
  *) echo "usage: df-deploy release <digest> | activate <digest> | refresh-keys <digest> | psql --host H --port P --dbname D --user U" >&2; exit 2 ;;
esac
