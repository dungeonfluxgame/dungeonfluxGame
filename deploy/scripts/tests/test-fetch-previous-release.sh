#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT
bin="$temporary/bin"
mkdir -p "$bin"
cat > "$bin/docker" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
case "$1" in
  buildx)
    if [ "${FAKE_LATEST:-present}" = absent ]; then exit 1; fi
    if [ "${5:-}" = --format ]; then printf '%s\n' "${FAKE_DIGEST:-sha256:$(printf '%064d' 7)}"; fi
    ;;
  create) echo release-container ;;
  cp) printf 'sha256:%064d\n' 1 > "${@: -1}" ;;
  rm) ;;
  *) exit 2 ;;
esac
MOCK
cat > "$bin/gh" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
case "${FAKE_GH:-empty}" in
  fail) exit 1 ;;
  empty) printf '[]\n' ;;
  invalid) printf 'null\n' ;;
  package) printf '[{"name":"dungeonflux-release"}]\n' ;;
  versions) printf '[{"name":"dungeonflux-release"}]\n' ;;
  *) exit 2 ;;
esac
MOCK
cat > "$bin/cosign" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$FAKE_COSIGN_LOG"
[ "${FAKE_COSIGN:-pass}" = pass ]
MOCK
cat > "$bin/jq" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
input=$(cat)
case "$*" in
  *"any("*)
    [[ "$input" == \[*\] ]] || exit 4
    if [[ "$input" == *'"name":"dungeonflux-release"'* ]]; then echo true; else echo false; fi
    ;;
  *"length == 0"*) [ "$input" = '[]' ] ;;
  *) exit 2 ;;
esac
MOCK
chmod +x "$bin/docker" "$bin/gh" "$bin/cosign" "$bin/jq"

failures=0
check() {
  label=$1
  shift
  if "$@"; then printf 'PASS %s\n' "$label"; else printf 'FAIL %s\n' "$label"; failures=$((failures + 1)); fi
}
run_helper() {
  PATH="$bin:$PATH" FAKE_COSIGN_LOG="$temporary/cosign.log" \
    bash "$root/deploy/scripts/fetch-previous-release.sh" \
      ghcr.io/example/dungeonflux-release Organization example example/repo "$1"
}
expect_failure() { ! "$@" >/dev/null 2>&1; }

latest="$temporary/latest"
mkdir "$latest"
check latest_tag_selects_its_verified_digest run_helper "$latest"
grep -q "ghcr.io/example/dungeonflux-release@sha256:$(printf '%064d' 7)" "$temporary/cosign.log"
grep -q "sha256:$(printf '%064d' 1)" "$latest/previous-revoked"
test ! -e "$latest/first-release"

missing="$temporary/missing"
mkdir "$missing"
check absent_tag_with_no_package_allows_first_release env PATH="$bin:$PATH" FAKE_LATEST=absent FAKE_GH=empty \
  FAKE_COSIGN_LOG="$temporary/cosign.log" bash "$root/deploy/scripts/fetch-previous-release.sh" \
  ghcr.io/example/dungeonflux-release Organization example example/repo "$missing"
test -e "$missing/first-release"

existing="$temporary/existing"
mkdir "$existing"
check absent_tag_with_existing_release_version_fails expect_failure env PATH="$bin:$PATH" FAKE_LATEST=absent FAKE_GH=package \
  FAKE_COSIGN_LOG="$temporary/cosign.log" bash "$root/deploy/scripts/fetch-previous-release.sh" \
  ghcr.io/example/dungeonflux-release Organization example example/repo "$existing"
test ! -e "$existing/first-release"

bad_signature="$temporary/bad-signature"
mkdir "$bad_signature"
check invalid_latest_signature_fails expect_failure env PATH="$bin:$PATH" FAKE_COSIGN=fail \
  FAKE_COSIGN_LOG="$temporary/cosign.log" bash "$root/deploy/scripts/fetch-previous-release.sh" \
  ghcr.io/example/dungeonflux-release Organization example example/repo "$bad_signature"

unreadable="$temporary/unreadable"
mkdir "$unreadable"
check unreadable_package_listing_fails expect_failure env PATH="$bin:$PATH" FAKE_LATEST=absent FAKE_GH=fail \
  FAKE_COSIGN_LOG="$temporary/cosign.log" bash "$root/deploy/scripts/fetch-previous-release.sh" \
  ghcr.io/example/dungeonflux-release Organization example example/repo "$unreadable"

invalid_listing="$temporary/invalid-listing"
mkdir "$invalid_listing"
check malformed_package_listing_fails expect_failure env PATH="$bin:$PATH" FAKE_LATEST=absent FAKE_GH=invalid \
  FAKE_COSIGN_LOG="$temporary/cosign.log" bash "$root/deploy/scripts/fetch-previous-release.sh" \
  ghcr.io/example/dungeonflux-release Organization example example/repo "$invalid_listing"

test "$failures" -eq 0
printf '%s failure(s)\n' "$failures"
