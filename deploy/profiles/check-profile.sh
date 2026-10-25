#!/usr/bin/env bash
set -Eeuo pipefail

fail() { printf 'profile guard: %s\n' "$1" >&2; exit 1; }
[[ $# -ge 2 && $# -le 3 ]] || fail 'usage: check-profile.sh <production|development|demo> <env-file-or-dir> [image-or-binary]'
profile=$1
settings=$2
artifact=${3:-}
case "$profile" in production|development|demo) ;; *) fail "unknown profile: $profile" ;; esac
[[ -e "$settings" ]] || fail "settings path not found: $settings"

# Read key names and values without evaluating shell syntax or printing secret values.
declare -A values=()
read_env_file() {
  local file=$1 line key value
  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ "$line" =~ ^[[:space:]]*(#|$) ]] && continue
    [[ "$line" == *=* ]] || fail "malformed environment line in $file"
    key=${line%%=*}
    value=${line#*=}
    key=${key//[[:space:]]/}
    [[ "$key" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]] || fail "invalid environment key in $file"
    if [[ -v "values[$key]" && ${values[$key]} != "$value" ]]; then fail "conflicting $key values in settings"; fi
    values["$key"]=$value
  done < "$file"
}
if [[ -d "$settings" ]]; then
  while IFS= read -r -d '' file; do read_env_file "$file"; done < <(find "$settings" -type f -name '*.env' -print0)
else
  read_env_file "$settings"
fi

is_on() { [[ ${values[$1]-} == on || ${values[$1]-} == true || ${values[$1]-} == 1 ]]; }
is_off() { [[ ${values[$1]-} == off || ${values[$1]-} == false || ${values[$1]-} == 0 ]]; }
if [[ "$profile" == production ]]; then
  for key in DF_GAMEPLAY_DEV_CONTROL DF_GAMEPLAY_DEV_CONTROL_PUBLIC DF_GAMEPLAY_DEV_OVERRIDES DF_DEV_ACCOUNTS DF_FIXTURE_BUILD DF_DEV_CONTROL DF_DEVCTL_ENABLED; do
    is_on "$key" && fail "$key is enabled"
  done
  for key in "${!values[@]}"; do
    if [[ "$key" =~ ^DF_.*_QUALIFICATION$ || "$key" == DF_PLAY_* || "$key" == DF_N06_* || "$key" == DF_L01_* ]]; then
      fail "$key is set"
    fi
    if [[ "$key" == DF_GAMEPLAY_DEV_* || "$key" == DF_DEVCTL_* || "$key" == DF_DEV_CONTROL_* || "$key" == DF_GAMEPLAY_TEST_* || "$key" == DF_*_FIXTURE* ]]; then
      [[ -z ${values[$key]} ]] || is_off "$key" || fail "$key is set to a development value"
    fi
  done
  [[ ${values[DF_GAMEPLAY_PROVIDER_MODE]-} != fake* ]] || fail 'DF_GAMEPLAY_PROVIDER_MODE selects fakes'
  [[ ${values[DF_GAMEPLAY_IDENTITY]-} == workos ]] || fail 'DF_GAMEPLAY_IDENTITY must be workos'
  [[ ${values[DF_GAMEPLAY_MAIL_MODE]-} == real ]] || fail 'DF_GAMEPLAY_MAIL_MODE must be real'
  [[ ${values[DF_LOG_LEVEL]-} == info ]] || fail 'DF_LOG_LEVEL must be info'
  [[ ${values[DF_GAMEPLAY_PUBLIC_URL]-} == https://* ]] || fail 'DF_GAMEPLAY_PUBLIC_URL must use HTTPS'
  [[ ${values[DF_GAMEPLAY_PUBLIC_URL]-} != *localhost* && ${values[DF_GAMEPLAY_PUBLIC_URL]-} != *127.0.0.1* && ${values[DF_GAMEPLAY_PUBLIC_URL]-} != *\[::1\]* ]] || fail 'DF_GAMEPLAY_PUBLIC_URL is loopback-only'
  [[ -n ${values[DF_GAMEPLAY_ALLOWED_ORIGINS]-} ]] || fail 'DF_GAMEPLAY_ALLOWED_ORIGINS is missing'
  [[ ${values[DF_GAMEPLAY_ALLOWED_ORIGINS]} == https://* && ${values[DF_GAMEPLAY_ALLOWED_ORIGINS]} != *http://* ]] || fail 'DF_GAMEPLAY_ALLOWED_ORIGINS must contain HTTPS origins only'
  [[ ${values[DF_GAMEPLAY_DATABASE_TLS]-} == verify-full ]] || fail 'DF_GAMEPLAY_DATABASE_TLS must be verify-full'
  [[ ${values[DF_GAMEPLAY_LOAD_DOTENV]-} == off ]] || fail 'DF_GAMEPLAY_LOAD_DOTENV must be off'
  [[ ${values[DF_PUBLIC_TLS]-} == enabled ]] || fail 'DF_PUBLIC_TLS must be enabled'
fi

if [[ -n "$artifact" ]]; then
  image_command=
  if [[ -f "$artifact" ]]; then
    binary=$artifact
  elif command -v docker >/dev/null 2>&1 && docker image inspect "$artifact" >/dev/null 2>&1; then
    image_command=$(docker image inspect --format '{{json .Config.Entrypoint}} {{json .Config.Cmd}}' "$artifact")
    [[ "$image_command" != *--dev-accounts* ]] || fail 'image entrypoint contains --dev-accounts'
    [[ "$image_command" != *account_app_preview* ]] || fail 'image entrypoint selects account_app_preview'
    [[ "$image_command" != *portal_preview* ]] || fail 'image entrypoint selects portal_preview'
    binary=$(docker create "$artifact") || fail "cannot create container for image $artifact"
    temporary_binary=$(mktemp)
    trap 'docker rm -f "$binary" >/dev/null 2>&1 || true; rm -f "$temporary_binary"' EXIT
    docker cp "$binary:/usr/local/bin/df-transport-fixture" "$temporary_binary" >/dev/null 2>&1 || fail 'image has no /usr/local/bin/df-transport-fixture'
    binary=$temporary_binary
  else
    fail "artifact is neither a readable binary nor a local Docker image: $artifact"
  fi
  command -v strings >/dev/null 2>&1 || fail 'strings is required to inspect build feature markers'
  markers=$(strings -a "$binary") || fail "cannot inspect binary: $artifact"
  if [[ "$profile" == production ]]; then
    for marker in DF_BUILD_INFO_V1 FEATURE:local-dev=0 FEATURE:test-fakes=0 FEATURE:fixture-build=0; do
      [[ "$markers" == *"$marker"* ]] || fail "binary build metadata cannot prove $marker (build-info marker missing)"
    done
    [[ "$markers" != *'FEATURE:local-dev=1'* ]] || fail 'binary contains FEATURE:local-dev=1'
    [[ "$markers" != *'FEATURE:test-fakes=1'* ]] || fail 'binary contains FEATURE:test-fakes=1'
    [[ "$markers" != *'FEATURE:fixture-build=1'* ]] || fail 'binary contains FEATURE:fixture-build=1'
    [[ "$markers" != *'BINARY:df-transport-fixture'* ]] || fail 'production binary is df-transport-fixture'
  elif [[ "$profile" == demo ]]; then
    [[ ${values[DF_FIXTURE_BUILD]-} == demo-exception:* ]] || fail 'demo profile does not declare its fixture exception'
    if [[ -n "$image_command" ]]; then
      [[ "$image_command" == *df-transport-fixture* && "$image_command" == *--gameplay-demo* ]] || fail 'demo image must run df-transport-fixture --gameplay-demo'
    else
      [[ "${artifact##*/}" == df-transport-fixture* ]] || fail 'demo binary is not df-transport-fixture'
    fi
  fi
  [[ "$markers" != *account_app_preview* ]] || fail 'binary contains account_app_preview example marker'
  [[ "$markers" != *portal_preview* ]] || fail 'binary contains portal_preview example marker'
fi
printf 'profile guard: %s settings accepted%s\n' "$profile" "${artifact:+ and artifact inspected}"
