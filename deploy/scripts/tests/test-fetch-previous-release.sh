#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT
bin="$temporary/bin"
mkdir -p "$bin"
version_one="sha256:$(printf '%064d' 1)"
version_two="sha256:$(printf '%064d' 2)"
revoked_one="sha256:$(printf '%064d' 11)"
revoked_two="sha256:$(printf '%064d' 22)"
cat > "$bin/docker" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
case "$1" in
  buildx)
    printf '%s\n' "$*" >> "${FAKE_DOCKER_LOG:-/dev/null}"
    if [[ "$*" == *":latest"* && -n "${FAKE_LATEST_DIGEST:-}" ]]; then
      if [[ "$*" == *"--format"* ]]; then printf '%s\n' "$FAKE_LATEST_DIGEST"; fi
      exit 0
    fi
    if [ "${FAKE_INSPECT:-missing}" = resolves ]; then exit 0; fi
    if [ "${FAKE_INSPECT:-missing}" = error ]; then echo "registry unavailable" >&2; exit 1; fi
    echo "manifest unknown" >&2
    exit 1
    ;;
  create)
    printf '%s\n' "$*" >> "${FAKE_DOCKER_LOG:-/dev/null}"
    printf 'release-container-%s\n' "${2##*sha256:}"
    ;;
  cp)
    printf '%s\n' "$*" >> "${FAKE_DOCKER_LOG:-/dev/null}"
    container=${2%%:*}
    case "$container" in
      *0000000000000000000000000000000000000000000000000000000000000001)
        printf '%s\n' "$FAKE_REVOKED_ONE" > "${@: -1}" ;;
      *0000000000000000000000000000000000000000000000000000000000000002)
        printf '%s\n' "$FAKE_REVOKED_TWO" > "${@: -1}" ;;
      *) exit 2 ;;
    esac
    ;;
  rm) printf '%s\n' "$*" >> "${FAKE_DOCKER_LOG:-/dev/null}" ;;
  *) exit 2 ;;
esac
MOCK
cat > "$bin/gh" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
[ "${1:-}" = api ] && [ "${2:-}" = --paginate ] || exit 2
endpoint=$3
if [[ "$endpoint" == *"/packages?package_type=container"* ]]; then
  case "${FAKE_GH:-versions}" in
    fail) echo "package pagination failed" >&2; exit 1 ;;
    invalid) printf 'null\n' ;;
    *) if [ "${FAKE_PACKAGE:-present}" = absent ]; then printf '[]\n'; else printf '[{"name":"dungeonflux-release"}]\n'; fi ;;
  esac
else
  [ "${FAKE_VERSION_LIST:-pass}" = pass ] || { echo "version pagination failed" >&2; exit 1; }
  case "${FAKE_GH:-versions}" in
    versions) printf '%s\n' "${FAKE_VERSIONS_JSON:-[]}" ;;
    paginated) printf '%s\n%s\n' "$FAKE_VERSIONS_JSON" "$FAKE_VERSIONS_JSON_2" ;;
    *) exit 2 ;;
  esac
fi
MOCK
cat > "$bin/cosign" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$FAKE_COSIGN_LOG"
case "${FAKE_COSIGN:-pass}" in
  pass) exit 0 ;;
  fail) exit 1 ;;
  fail_second) [[ "$*" != *"${FAKE_VERSION_TWO}"* ]] ;;
  *) exit 2 ;;
esac
MOCK
cat > "$bin/jq" <<'MOCK'
#!/usr/bin/env python
import json
import sys

arguments = sys.argv[1:]
if any(argument.startswith("-") and "s" in argument and "e" in argument for argument in arguments):
    raw = sys.stdin.read()
    decoder = json.JSONDecoder()
    arrays = []
    offset = 0
    while offset < len(raw):
        while offset < len(raw) and raw[offset].isspace():
            offset += 1
        if offset == len(raw):
            break
        value, offset = decoder.raw_decode(raw, offset)
        if not isinstance(value, list):
            sys.exit(4)
        arrays.extend(value)
    if not all(isinstance(item, dict) and isinstance(item.get("name"), str) for item in arrays):
        sys.exit(4)
    if "any(" not in " ".join(arguments):
        if not all(isinstance(item.get("created_at"), str) for item in arrays):
            sys.exit(4)
        print(json.dumps(arrays, separators=(",", ":")))
    else:
        print("true" if any(item["name"] == "dungeonflux-release" for item in arrays) else "false")
elif "-e" in arguments:
    with open(arguments[-1], encoding="utf-8") as source:
        sys.exit(0 if len(json.load(source)) == 0 else 1)
elif "-r" in arguments:
    with open(arguments[-1], encoding="utf-8") as source:
        for item in json.load(source):
            print(f'{item["name"]}\t{item["created_at"]}')
else:
    sys.exit(2)
MOCK
chmod +x "$bin/docker" "$bin/gh" "$bin/cosign" "$bin/jq"

failures=0
check() {
  label=$1
  shift
  if "$@"; then printf 'PASS %s\n' "$label"; else printf 'FAIL %s\n' "$label"; failures=$((failures + 1)); fi
}
expect_failure() { ! "$@" >/dev/null 2>&1; }
versions='[{"name":"sha256:0000000000000000000000000000000000000000000000000000000000000001","created_at":"2026-09-01T00:00:00Z"},{"name":"sha256:0000000000000000000000000000000000000000000000000000000000000002","created_at":"2026-10-01T00:00:00Z"}]'

version_one_json="[{\"name\":\"$version_one\",\"created_at\":\"2026-09-01T00:00:00Z\"}]"
version_two_json="[{\"name\":\"$version_two\",\"created_at\":\"2026-10-01T00:00:00Z\"}]"
union="$temporary/union"
mkdir "$union"
check moved_latest_tag_still_uses_union_of_all_verified_versions env PATH="$bin:$PATH" FIRST_RELEASE=false FAKE_GH=paginated \
  FAKE_LATEST_DIGEST="$version_one" FAKE_VERSIONS_JSON="$version_one_json" FAKE_VERSIONS_JSON_2="$version_two_json" \
  FAKE_DOCKER_LOG="$temporary/docker.log" \
  FAKE_COSIGN_LOG="$temporary/cosign.log" FAKE_VERSION_TWO="$version_two" \
  FAKE_REVOKED_ONE="$revoked_one" FAKE_REVOKED_TWO="$revoked_two" bash "$root/deploy/scripts/fetch-previous-release.sh" \
  ghcr.io/example/dungeonflux-release Organization example example/repo "$union" false 20261002000000
grep -q "$version_one" "$temporary/cosign.log"
grep -q "$version_two" "$temporary/cosign.log"
grep -q "$revoked_one" "$union/previous-revoked"
grep -q "$revoked_two" "$union/previous-revoked"
! grep -q 'latest' "$temporary/docker.log"
test ! -e "$union/first-release"

unverifiable="$temporary/unverifiable"
mkdir "$unverifiable"
check unverifiable_package_version_fails expect_failure env PATH="$bin:$PATH" FAKE_GH=versions FAKE_VERSIONS_JSON="$versions" \
  FAKE_COSIGN=fail_second FAKE_COSIGN_LOG="$temporary/cosign.log" FAKE_VERSION_TWO="$version_two" \
  FAKE_REVOKED_ONE="$revoked_one" FAKE_REVOKED_TWO="$revoked_two" bash "$root/deploy/scripts/fetch-previous-release.sh" \
  ghcr.io/example/dungeonflux-release Organization example example/repo "$unverifiable" false 20261002000000

first_without_input="$temporary/first-without-input"
mkdir "$first_without_input"
check zero_versions_without_first_release_input_fails expect_failure env PATH="$bin:$PATH" FAKE_GH=versions FAKE_VERSIONS_JSON='[]' \
  FAKE_COSIGN_LOG="$temporary/cosign.log" FAKE_INSPECT=missing bash "$root/deploy/scripts/fetch-previous-release.sh" \
  ghcr.io/example/dungeonflux-release Organization example example/repo "$first_without_input" false 20261002000000
test ! -e "$first_without_input/first-release"

first_with_input="$temporary/first-with-input"
mkdir "$first_with_input"
check first_release_input_with_existing_versions_fails expect_failure env PATH="$bin:$PATH" FAKE_GH=versions \
  FAKE_VERSIONS_JSON="$versions" FAKE_COSIGN_LOG="$temporary/cosign.log" FAKE_VERSION_TWO="$version_two" \
  FAKE_REVOKED_ONE="$revoked_one" FAKE_REVOKED_TWO="$revoked_two" bash "$root/deploy/scripts/fetch-previous-release.sh" \
  ghcr.io/example/dungeonflux-release Organization example example/repo "$first_with_input" true 20261002000000
test ! -e "$first_with_input/first-release"

listing_error="$temporary/listing-error"
mkdir "$listing_error"
check package_version_listing_error_fails expect_failure env PATH="$bin:$PATH" FAKE_GH=fail \
  FAKE_COSIGN_LOG="$temporary/cosign.log" bash "$root/deploy/scripts/fetch-previous-release.sh" \
  ghcr.io/example/dungeonflux-release Organization example example/repo "$listing_error" true 20261002000000

version_listing_error="$temporary/version-listing-error"
mkdir "$version_listing_error"
check version_pagination_error_fails expect_failure env PATH="$bin:$PATH" FAKE_GH=versions FAKE_VERSION_LIST=fail \
  FAKE_COSIGN_LOG="$temporary/cosign.log" bash "$root/deploy/scripts/fetch-previous-release.sh" \
  ghcr.io/example/dungeonflux-release Organization example example/repo "$version_listing_error" false 20261002000000

malformed_listing="$temporary/malformed-listing"
mkdir "$malformed_listing"
check malformed_package_version_listing_fails expect_failure env PATH="$bin:$PATH" FAKE_GH=invalid \
  FAKE_COSIGN_LOG="$temporary/cosign.log" bash "$root/deploy/scripts/fetch-previous-release.sh" \
  ghcr.io/example/dungeonflux-release Organization example example/repo "$malformed_listing" true 20261002000000

first_release="$temporary/first-release"
mkdir "$first_release"
check explicit_first_release_with_zero_versions_passes env PATH="$bin:$PATH" FAKE_GH=versions FAKE_VERSIONS_JSON='[]' \
  FAKE_COSIGN_LOG="$temporary/cosign.log" FAKE_INSPECT=missing bash "$root/deploy/scripts/fetch-previous-release.sh" \
  ghcr.io/example/dungeonflux-release Organization example example/repo "$first_release" true 20261002000000
test -e "$first_release/first-release"
test ! -s "$first_release/previous-revoked"

absent_package="$temporary/absent-package"
mkdir "$absent_package"
check explicitly_confirmed_first_release_with_no_package_passes env PATH="$bin:$PATH" FAKE_GH=versions FAKE_PACKAGE=absent \
  FAKE_COSIGN_LOG="$temporary/cosign.log" FAKE_INSPECT=missing bash "$root/deploy/scripts/fetch-previous-release.sh" \
  ghcr.io/example/dungeonflux-release Organization example example/repo "$absent_package" true 20261002000000
test -e "$absent_package/first-release"

inspect_error="$temporary/inspect-error"
mkdir "$inspect_error"
check non_manifest_unknown_first_release_inspection_error_fails expect_failure env PATH="$bin:$PATH" FAKE_GH=versions \
  FAKE_VERSIONS_JSON='[]' FAKE_INSPECT=error FAKE_COSIGN_LOG="$temporary/cosign.log" \
  bash "$root/deploy/scripts/fetch-previous-release.sh" ghcr.io/example/dungeonflux-release Organization example example/repo "$inspect_error" true 20261002000000

old_version="$temporary/old-version"
mkdir "$old_version"
check new_release_date_not_newer_than_verified_release_fails expect_failure env PATH="$bin:$PATH" FAKE_GH=versions \
  FAKE_VERSIONS_JSON="$versions" FAKE_COSIGN_LOG="$temporary/cosign.log" FAKE_VERSION_TWO="$version_two" \
  FAKE_REVOKED_ONE="$revoked_one" FAKE_REVOKED_TWO="$revoked_two" bash "$root/deploy/scripts/fetch-previous-release.sh" \
  ghcr.io/example/dungeonflux-release Organization example example/repo "$old_version" false 20260930000000

test "$failures" -eq 0
printf '%s failure(s)\n' "$failures"
