#!/usr/bin/env bash
# Well-formedness of the staging qualification checklist (deploy/qualification/staging-checks.csv): eight fields on every row, unique ids, a
# status that is unverified, passed or failed, and the two managed-PostgreSQL assumptions of migration 0018 (Q01 pg_signal_backend, Q02 the
# migrator's membership revoked under doadmin) present, with the SQL that checks them. Needs awk only.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
file=$root/deploy/qualification/staging-checks.csv
sql=$root/deploy/db/qualify-managed-postgres.sql
fail=0
bad() { echo "FAIL $*"; fail=1; }
[ -s "$file" ] || { echo "FAIL no $file"; exit 1; }
awk 'BEGIN { FS = "" }
  { n = 1; q = 0
    for (i = 1; i <= NF; i++) { c = $i; if (c == "\"") q = !q; else if (c == "," && !q) n++ }
    if (n != 8) { printf "FAIL line %d has %d fields\n", NR, n; bad = 1 } }
  END { exit bad }' "$file" || fail=1
ids=$(awk -F, 'NR > 1 { print $1 }' "$file")
[ "$(printf '%s\n' "$ids" | sort | uniq -d | wc -l | tr -d ' ')" = 0 ] || bad "duplicate ids"
for id in Q01 Q02; do printf '%s\n' "$ids" | grep -qx "$id" || bad "checklist item $id is missing"; done
grep -q 'pg_signal_backend' "$file" || bad "Q01 does not mention pg_signal_backend"
grep -q 'REVOKE df_runtime_definer' "$file" || bad "Q02 does not mention the REVOKE"
if tail -n +2 "$file" | grep -vE ',(unverified|passed|failed)$' | grep -q .; then bad "a row has a status other than unverified, passed or failed"; fi
if ! { grep -q 'Q01 ok' "$sql" && grep -q 'Q02 ok' "$sql"; }; then bad "the qualification SQL does not print Q01 ok and Q02 ok"; fi
[ "$fail" -eq 0 ] && echo "qualification checklist ok"
exit "$fail"
