#!/usr/bin/env bash
set -Eeuo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)
deploy_script=${PORTAL_DEPLOY_SCRIPT:-"$root/deploy/demo/deploy-portal.sh"}
temporary_dir=$(mktemp -d)
trap 'rm -rf "$temporary_dir"' EXIT
old_digest="sha256:$(printf '%064d' 1)"
previous_digest="sha256:$(printf '%064d' 2)"
next_digest="sha256:$(printf '%064d' 3)"
mkdir "$temporary_dir/bin"

# Execute the actual remote payload with only filesystem roots rebased. Every
# host/network/process boundary is fake; no SSH, Docker daemon or service runs.
sed -n "/<<'REMOTE'/,/^REMOTE$/p" "$deploy_script" | sed '1d;$d' > "$temporary_dir/remote.template"
cat > "$temporary_dir/bin/docker" <<'FAKE'
#!/usr/bin/env bash
set -eu
printf 'docker %s\n' "$*" >> "$CASE_ROOT/events"
case $1 in
  pull) [[ $FAILURE != pull ]] ;;
  create) printf '%s\n' "${*: -1}" > "$CASE_ROOT/image" ;;
  cp)
    if [[ $2 == *:/usr/local/bin/df-portal-hosted ]]; then
      printf '#!/bin/sh\nNEXT\n' > "$3"
      chmod +x "$3"
    elif [[ $FAILURE != bundle ]]; then
      printf 'NEXT BUNDLE\n' > "$3/index.html"
    fi ;;
  rm) exit 0 ;;
  *) exit 99 ;;
esac
FAKE
cat > "$temporary_dir/bin/systemctl" <<'FAKE'
#!/usr/bin/env bash
set -eu
printf 'systemctl %s\n' "$*" >> "$CASE_ROOT/events"
if [[ $1 == restart && $FAILURE == restart ]] && grep -q NEXT "$CASE_ROOT/opt/dungeonflux/bin/df-portal-hosted"; then
  exit 1
fi
FAKE
cat > "$temporary_dir/bin/curl" <<'FAKE'
#!/usr/bin/env bash
set -eu
printf 'curl %s\n' "$*" >> "$CASE_ROOT/events"
if [[ $FAILURE == rollback_health ]]; then exit 22; fi
if [[ $FAILURE == health || $FAILURE == first_health ]] && grep -q NEXT "$CASE_ROOT/opt/dungeonflux/bin/df-portal-hosted"; then
  exit 22
fi
if [[ $FAILURE == health_body ]] && grep -q NEXT "$CASE_ROOT/opt/dungeonflux/bin/df-portal-hosted"; then
  printf 'coming soon\n'
else
  printf 'ok\n'
fi
FAKE
cat > "$temporary_dir/bin/stat" <<'FAKE'
#!/usr/bin/env bash
case $2 in
  '%u:%a') printf '0:700\n' ;;
  '%u %a') printf '0 644\n' ;;
  *) exit 99 ;;
esac
FAKE
cat > "$temporary_dir/bin/flock" <<'FAKE'
#!/usr/bin/env bash
[[ $FAILURE != concurrent ]]
FAKE
cat > "$temporary_dir/bin/sleep" <<'FAKE'
#!/usr/bin/env bash
exit 0
FAKE
cat > "$temporary_dir/bin/install" <<'FAKE'
#!/usr/bin/env bash
set -eu
# Git Bash's sandbox cannot mkdir -p through /c/Users. Preserve install's
# filesystem behavior using paths relative to this case's existing root.
arguments=()
for argument in "$@"; do
  arguments+=("${argument#"$CASE_ROOT/"}")
done
cd "$CASE_ROOT"
/usr/bin/install "${arguments[@]}"
FAKE
cat > "$temporary_dir/bin/mkdir" <<'FAKE'
#!/usr/bin/env bash
set -eu
arguments=()
skip_mode=false
for argument in "$@"; do
  if [[ $skip_mode == true ]]; then skip_mode=false; continue; fi
  # Windows ACLs do not implement mkdir's Unix mode option. The private-upload
  # ownership/mode boundary is represented by the stat fake above.
  if [[ $argument == -m ]]; then skip_mode=true; continue; fi
  arguments+=("${argument#"$CASE_ROOT/"}")
done
cd "$CASE_ROOT"
/usr/bin/mkdir "${arguments[@]}"
FAKE
chmod +x "$temporary_dir/bin/"*

run_case() {
  local name=$1 expected=$2 mode=${3:-deploy}
  export FAILURE=$name CASE_ROOT="$temporary_dir/$name"
  cd "$temporary_dir"
  mkdir -p "$name/opt/dungeonflux/"{bin,app,demo} \
    "$name/etc/"{dungeonflux,systemd/system} "$name/tmp/df-portal-upload.fixture"
  printf OLD > "$CASE_ROOT/opt/dungeonflux/bin/df-portal-hosted"
  printf 'OLD BUNDLE' > "$CASE_ROOT/opt/dungeonflux/app/index.html"
  printf 'OLD UNIT' > "$CASE_ROOT/etc/systemd/system/df-portal-hosted.service"
  printf '%s\n' "$old_digest" > "$CASE_ROOT/opt/dungeonflux/demo/portal-current"
  printf '%s\n' "$previous_digest" > "$CASE_ROOT/opt/dungeonflux/demo/portal-previous"
  printf 'NEW UNIT' > "$CASE_ROOT/tmp/df-portal-upload.fixture/df-portal-hosted.service"
  printf '%s\n' DF_ACCOUNTS_OPEN=false DF_PORTAL_DATABASE_URL=fixture \
    DF_PORTAL_MIGRATOR_DATABASE_URL=fixture > "$CASE_ROOT/etc/dungeonflux/portal.env"
  : > "$CASE_ROOT/etc/dungeonflux/demo.env"
  if [[ $name == operator ]]; then
    printf 'DF_OPERATOR_TUNNEL=true\n' >> "$CASE_ROOT/etc/dungeonflux/portal.env"
  fi
  if [[ $name == first_health ]]; then
    rm -f "$CASE_ROOT/opt/dungeonflux/bin/df-portal-hosted" \
      "$CASE_ROOT/etc/systemd/system/df-portal-hosted.service" \
      "$CASE_ROOT/opt/dungeonflux/app/index.html" \
      "$CASE_ROOT/opt/dungeonflux/demo/portal-"{current,previous}
    rmdir "$CASE_ROOT/opt/dungeonflux/app"
  fi
  sed -e "s|/opt/dungeonflux|$CASE_ROOT/opt/dungeonflux|g" \
    -e "s|/etc/dungeonflux|$CASE_ROOT/etc/dungeonflux|g" \
    -e "s|/etc/systemd/system|$CASE_ROOT/etc/systemd/system|g" \
    -e "s|/tmp/df-portal-upload|$CASE_ROOT/tmp/df-portal-upload|g" \
    -e "s|/tmp/df-portal-deploy-|$CASE_ROOT/tmp/df-portal-deploy-|g" \
    "$temporary_dir/remote.template" > "$CASE_ROOT/remote.sh"
  local unit_hash result=0 requested_digest=$next_digest
  if [[ $name == same_digest ]]; then requested_digest=$old_digest; fi
  unit_hash=$(sha256sum "$CASE_ROOT/tmp/df-portal-upload.fixture/df-portal-hosted.service")
  PATH="$temporary_dir/bin:$PATH" bash "$CASE_ROOT/remote.sh" "$mode" "$requested_digest" \
    fixture "$CASE_ROOT/tmp/df-portal-upload.fixture" "${unit_hash%% *}" \
    > "$CASE_ROOT/output" 2>&1 || result=$?
  if [[ $expected == success ]]; then
    [[ $result == 0 ]] || { cat "$CASE_ROOT/output" >&2; return 1; }
    grep -Fxq NEXT "$CASE_ROOT/opt/dungeonflux/bin/df-portal-hosted"
    grep -Fxq 'NEXT BUNDLE' "$CASE_ROOT/opt/dungeonflux/app/index.html"
    if [[ $name == same_digest ]]; then
      grep -Fxq "$previous_digest" "$CASE_ROOT/opt/dungeonflux/demo/portal-previous"
    else
      grep -Fxq "$old_digest" "$CASE_ROOT/opt/dungeonflux/demo/portal-previous"
    fi
    if [[ $mode == rollback ]]; then
      grep -Fxq "$previous_digest" "$CASE_ROOT/opt/dungeonflux/demo/portal-current"
      grep -Fq "@$previous_digest" "$CASE_ROOT/image"
    else
      grep -Fxq "$requested_digest" "$CASE_ROOT/opt/dungeonflux/demo/portal-current"
    fi
  else
    [[ $result != 0 ]] || { echo "unexpected success: $name" >&2; return 1; }
    if [[ $name == first_health ]]; then
      [[ ! -e "$CASE_ROOT/opt/dungeonflux/bin/df-portal-hosted" ]]
      [[ ! -e "$CASE_ROOT/opt/dungeonflux/app" ]]
      [[ ! -e "$CASE_ROOT/etc/systemd/system/df-portal-hosted.service" ]]
      [[ ! -e "$CASE_ROOT/opt/dungeonflux/demo/portal-current" ]]
      [[ ! -e "$CASE_ROOT/opt/dungeonflux/demo/portal-previous" ]]
      grep -Fq 'systemctl disable df-portal-hosted.service' "$CASE_ROOT/events"
    else
      grep -Fxq OLD "$CASE_ROOT/opt/dungeonflux/bin/df-portal-hosted" || {
        echo "$name did not restore the old binary" >&2
        return 1
      }
      grep -Fxq 'OLD BUNDLE' "$CASE_ROOT/opt/dungeonflux/app/index.html"
      grep -Fxq 'OLD UNIT' "$CASE_ROOT/etc/systemd/system/df-portal-hosted.service"
      grep -Fxq "$old_digest" "$CASE_ROOT/opt/dungeonflux/demo/portal-current"
      grep -Fxq "$previous_digest" "$CASE_ROOT/opt/dungeonflux/demo/portal-previous"
      if [[ $name == health* || $name == restart ]]; then
        grep -Fq 'systemctl restart df-portal-hosted.service' "$CASE_ROOT/events"
      fi
    fi
  fi
  [[ ! -d "$CASE_ROOT/tmp/df-portal-upload.fixture" ]]
  if [[ $name == rollback_health ]]; then
    [[ -n $(find "$CASE_ROOT/opt/dungeonflux/demo" -name '.portal-deploy.*' -print) ]]
    grep -Fq 'Portal rollback failed; recovery files retained' "$CASE_ROOT/output"
  else
    [[ -z $(find "$CASE_ROOT/opt/dungeonflux/demo" -name '.portal-deploy.*' -print) ]]
  fi
  printf 'PASS %s\n' "$name"
}

run_case success success
run_case same_digest success
run_case rollback success rollback
for failure in pull bundle restart health health_body operator concurrent first_health rollback_health; do
  run_case "$failure" failure
done
printf 'portal lifecycle: 12 cases passed (all host commands faked)\n'
