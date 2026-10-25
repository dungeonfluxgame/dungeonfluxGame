#!/usr/bin/env bash
# Structural check of the documented DNS record list deploy/dns/records.csv (LAUNCH.md decision 32). It reads
# files only: it never queries or changes DNS, and agents never change DNS or registrar settings (the owner enters
# the records by hand at GoDaddy, the registrar, unless DNS later moves to DigitalOcean).
#   deploy/scripts/check-dns-records.sh
# Asserts: exactly 9 comma-separated columns on every row (no embedded commas); unique ids; known phases and
# actions; the apex is never a CNAME; the required first-cutover rows exist (apex A, www CNAME, staging A, mail
# SPF, DKIM, DMARC); the hostnames in the file are the configured ones (default dungeonfluxdnd.com); no example
# or placeholder host (example.com, REPLACE_ME) appears. Needs bash, awk, grep.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
file="$root/deploy/dns/records.csv"
failures=0
fail() { echo "FAIL $*"; failures=$((failures + 1)); }
pass() { echo "PASS $*"; }

[ -f "$file" ] || { echo "missing $file" >&2; exit 1; }

bad_columns=$(awk -F, 'NR > 1 && NF != 9 { print NR ": " NF " columns" }' "$file")
if [ -z "$bad_columns" ]; then pass "every row has 9 columns"; else fail "rows with the wrong column count: $bad_columns"; fi

dups=$(awk -F, 'NR > 1 { c[$1]++ } END { for (k in c) if (c[k] > 1) print k }' "$file")
if [ -z "$dups" ]; then pass "ids are unique"; else fail "duplicate ids: $dups"; fi

bad_phase=$(awk -F, 'NR > 1 && $2 !~ /^(pre-cutover|cutover|post-cutover|pre-teardown|always)$/ { print $1 }' "$file")
if [ -z "$bad_phase" ]; then pass "phases are known"; else fail "unknown phase: $bad_phase"; fi
bad_action=$(awk -F, 'NR > 1 && $3 !~ /^(add|replace|remove|change|keep|decide)$/ { print $1 }' "$file")
if [ -z "$bad_action" ]; then pass "actions are known"; else fail "unknown action: $bad_action"; fi

apex_cname=$(awk -F, 'NR > 1 && $5 == "@" && $4 == "CNAME" { print $1 }' "$file")
if [ -z "$apex_cname" ]; then pass "the apex is not a CNAME"; else fail "apex CNAME rows: $apex_cname"; fi

need() { # need <description> <awk condition on type,name>
  if awk -F, -v t="$2" -v n="$3" 'NR > 1 && $4 == t && $5 == n && $8 == "yes" { found = 1 } END { exit found ? 0 : 1 }' "$file"; then
    pass "required row: $1"
  else
    fail "missing required row: $1"
  fi
}
need "apex A" A '@'
need "www CNAME" CNAME www
need "staging A" A staging
need "mail SPF" TXT mail
need "mail DKIM" 'CNAME or TXT' '<dkim selector>._domainkey.mail'
need "mail DMARC" TXT '_dmarc.mail'
# The staging A record must be removed BEFORE staging is destroyed or its address released (subdomain takeover).
if awk -F, 'NR > 1 && $2 == "pre-teardown" && $3 == "remove" && $4 == "A" && $5 == "staging" && $8 == "yes" { found = 1 } END { exit found ? 0 : 1 }' "$file"; then
  pass "required row: pre-teardown removal of the staging A record"
else
  fail "missing required row: a pre-teardown row that removes the staging A record"
fi

if grep -Eqi 'example\.(com|org|net)|REPLACE_ME' "$file"; then fail "an example or placeholder host appears in the record list"; else pass "no example or placeholder host"; fi
if grep -q 'dungeonfluxdnd\.com' "$file"; then pass "the configured domain dungeonfluxdnd.com is named"; else fail "dungeonfluxdnd.com is not mentioned"; fi

if [ "$failures" -ne 0 ]; then echo "$failures check(s) failed"; exit 1; fi
echo "dns records ok"
