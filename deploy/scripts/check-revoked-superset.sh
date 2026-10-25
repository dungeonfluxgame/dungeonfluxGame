#!/usr/bin/env bash
# CI rule: the revocation list of a NEW release must be a SUPERSET of the previous signed release's list (round 3b). A host that has seen a
# revocation keeps it forever (/etc/dungeonflux/revoked-images is a union), but a fresh host, a rebuilt host and a host that never saw the
# older release learn revocations only from the release they activate; a release that quietly drops an old digest would let such a host
# start a runtime that was revoked months ago.
#
#   deploy/scripts/check-revoked-superset.sh PREVIOUS_REVOKED_FILE NEW_REVOKED_FILE
#
# WHAT REVOCATION CAN AND CANNOT DO (also in service-operations.md): it covers the images in a release's allowed-images list, i.e. the built
# images, above all the campaign-runtime image. THIRD-PARTY images (the database forwarder, Prometheus, Alertmanager, the textfile exporter)
# are pinned by digest in release.lock but are not in that list and cannot be revoked: to stop using one, ship a new release. Revocation
# also does not stop RUNNING runtimes: the list is read only when a runtime is created, so live campaigns keep running on a revoked image
# until they end or an operator stops them on purpose.
set -euo pipefail
[ "$#" -eq 2 ] || { echo "usage: check-revoked-superset.sh PREVIOUS_REVOKED_FILE NEW_REVOKED_FILE" >&2; exit 2; }
prev=$1; new=$2
[ -f "$prev" ] && [ -f "$new" ] || { echo "both files must exist" >&2; exit 2; }
digests() { grep -E '^sha256:[0-9a-f]{64}$' "$1" | sort -u || true; }
bad=$(grep -vE '^(#.*|[[:space:]]*|sha256:[0-9a-f]{64})$' "$new" || true)
[ -z "$bad" ] || { echo "the new revocation list has a malformed line: $bad" >&2; exit 1; }
missing=$(comm -23 <(digests "$prev") <(digests "$new"))
if [ -n "$missing" ]; then
  echo "the new revocation list DROPS digests that the previous release revoked (a revocation list only grows):" >&2
  echo "$missing" >&2
  exit 1
fi
echo "revocation list ok: $(digests "$new" | wc -l | tr -d ' ') digest(s), a superset of the previous $(digests "$prev" | wc -l | tr -d ' ')"
