#!/usr/bin/env bash
# Config check of the Caddy edge files (critic rounds 1 to 1e):
#   * no bearer capability in a URL (asset capabilities in /assets/* and /media/*, join tokens, any future route) may reach
#     a proxy log, and capability responses must carry Referrer-Policy: no-referrer (O2 design review, plan 13.5);
#   * the static docs site is served with a strict CSP and without dotfiles or source maps;
#   * the production file carries the DF_ALLOWED_CIDRS access gate that fails closed (staging is a public name in front of a
#     fake identity provider).
#
#   deploy/scripts/check-edge-redaction.sh          static: reads both Caddyfiles, then runs the log
#                                                   redaction pattern FROM the file against sample targets
#   deploy/scripts/check-edge-redaction.sh --live   also sends requests through a running staging mirror
#                                                   (including one while the upstream is down, which makes
#                                                   Caddy write an error-log entry) and greps the edge log
# The first segment ends at '/', '?', '%' or ';', so an encoded slash (/join%2FTOKEN) or a path parameter cannot
# carry a token into the log (critic round 1c, item 12).
# Needs bash, grep, sed. STATUS: the static part was run on the authoring machine. The pattern is evaluated
# with POSIX ERE (sed -E) while Caddy uses Go RE2; the pattern uses only syntax both share and its two
# alternatives are mutually exclusive, so leftmost-first and leftmost-longest agree. The Caddy configuration
# itself has NOT been run through `caddy validate`, and --live is UNVERIFIED.
# Exit 1 on any failure. CI-safe: no secrets, no network.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
live=0
if [ "${1:-}" = "--live" ]; then live=1; fi
failures=0
fail() { echo "FAIL $*"; failures=$((failures + 1)); }
pass() { echo "PASS $*"; }

check_file() {
  local file=$1 name
  name=$(basename "$file")

  if grep -Eq '^[[:space:]]*@capability[[:space:]]+path[[:space:]]+.*/assets/\*' "$file" \
     && grep -Eq '^[[:space:]]*@capability[[:space:]]+path[[:space:]]+.*/media/\*' "$file"; then
    pass "$name: @capability matcher covers /assets/* and /media/*"
  else
    fail "$name: @capability matcher must list /assets/* and /media/*"
  fi

  if grep -Eq '^[[:space:]]*header[[:space:]]+@capability[[:space:]]+>Referrer-Policy[[:space:]]+"no-referrer"' "$file"; then
    pass "$name: capability responses get Referrer-Policy no-referrer (deferred)"
  else
    fail "$name: missing 'header @capability >Referrer-Policy \"no-referrer\"'"
  fi

  if grep -Eq '^[[:space:]]*admin[[:space:]]+unix//run/caddy/admin\.sock' "$file"; then
    pass "$name: admin API on a unix socket (reload without dropping connections)"
  else
    fail "$name: admin must be 'unix//run/caddy/admin.sock|0600', not off and not a TCP address"
  fi

  # A global default logger (carries http.log.error and handler logs) AND the site access logger: at least
  # two log blocks, and every block filters the URI, Referer, Sec-WebSocket-Protocol and Location.
  local logs filters referers protocols locations plain
  logs=$(grep -Ec '^[[:space:]]*log[[:space:]]*\{' "$file" || true)
  filters=$(grep -Ec '^[[:space:]]*request>uri[[:space:]]+regexp[[:space:]]+"' "$file" || true)
  referers=$(grep -Ec '^[[:space:]]*request>headers>Referer[[:space:]]+delete' "$file" || true)
  protocols=$(grep -Ec '^[[:space:]]*request>headers>Sec-WebSocket-Protocol[[:space:]]+delete' "$file" || true)
  locations=$(grep -Ec '^[[:space:]]*resp_headers>Location[[:space:]]+delete' "$file" || true)
  plain=$({ grep -E '^[[:space:]]*format[[:space:]]' "$file" || true; } | { grep -Evc 'format[[:space:]]+filter' || true; })
  if [ "$logs" -ge 2 ] && [ "$logs" -eq "$filters" ] && [ "$logs" -eq "$referers" ] \
     && [ "$logs" -eq "$protocols" ] && [ "$logs" -eq "$locations" ] && [ "$plain" -eq 0 ]; then
    pass "$name: every logger (global default and site) filters URI, Referer, Sec-WebSocket-Protocol, Location ($logs)"
  else
    fail "$name: loggers=$logs uri-filters=$filters referer=$referers ws-protocol=$protocols location=$locations non-filter-formats=$plain (need >=2 loggers, all equal, none unfiltered)"
  fi
  if grep -Eq 'log_credentials|log_skip' "$file"; then fail "$name: log_credentials/log_skip must not be used"; fi
  if grep -Eq '^[[:space:]]*(level|exclude|include)[[:space:]]' "$file"; then
    fail "$name: no include/exclude/level: a logger must not be able to drop the filter or hide errors"
  fi

  # The static docs site's security headers and file hiding.
  if grep -Fq 'Content-Security-Policy' "$file" && grep -Fq "default-src 'none'" "$file" && grep -Fq "script-src 'none'" "$file" \
     && grep -Fq "frame-ancestors 'none'" "$file" && grep -Fq "object-src 'none'" "$file"; then
    pass "$name: the docs site sends a strict CSP (default-src and script-src none, frame-ancestors none)"
  else
    fail "$name: the docs site must send a CSP with default-src 'none', script-src 'none', object-src 'none' and frame-ancestors 'none'"
  fi
  if grep -Eq "unsafe-eval" "$file" || grep -Eq "(^|[; \"])(script-src|style-src|default-src) [^;\"]*'unsafe-inline'" "$file"; then
    fail "$name: the CSP allows unsafe-eval or unsafe-inline for script or style elements (only style-src-attr may)"
  else
    pass "$name: the CSP has no unsafe-eval and no unsafe-inline outside style-src-attr"
  fi
  if grep -Eq '^[[:space:]]*hide[[:space:]]+\.\*[[:space:]]+\*\.map[[:space:]]*$' "$file" \
     && ! grep -Eq '^[[:space:]]*file_server[[:space:]]*$' "$file"; then
    pass "$name: file_server hides dotfiles and source maps"
  else
    fail "$name: every file_server must be 'file_server { hide .* *.map }'"
  fi
  if [ "$name" = Caddyfile.production ]; then
    local gate_line handle_denied handle_dynamic
    gate_line=$(grep -nF '@denied not client_ip {$DF_ALLOWED_CIDRS:127.0.0.1/32}' "$file" | head -n 1 | cut -d: -f1 || true)
    handle_denied=$(grep -nE '^[[:space:]]*handle @denied[[:space:]]*\{' "$file" | head -n 1 | cut -d: -f1 || true)
    handle_dynamic=$(grep -nE '^[[:space:]]*handle @always_dynamic[[:space:]]*\{' "$file" | head -n 1 | cut -d: -f1 || true)
    if [ -n "$gate_line" ] && [ -n "$handle_denied" ] && [ -n "$handle_dynamic" ] && [ "$handle_denied" -lt "$handle_dynamic" ] \
       && grep -Eq '^[[:space:]]*respond "Forbidden" 403[[:space:]]*$' "$file"; then
      pass "$name: access gate (client_ip allowlist, default loopback only, 403) comes before every route"
    else
      fail "$name: missing the DF_ALLOWED_CIDRS access gate (client_ip, default 127.0.0.1/32, respond 403, before the other handles)"
    fi
  fi

  # Evaluate the pattern that is actually in the file.
  local line pattern replacement sed_replacement
  line=$(grep -E '^[[:space:]]*request>uri[[:space:]]+regexp[[:space:]]+"' "$file" | head -n 1)
  pattern=$(printf '%s' "$line" | sed -E 's/^[^"]*"([^"]*)".*$/\1/')
  replacement=$(printf '%s' "$line" | sed -E 's/^[^"]*"[^"]*"[[:space:]]+"([^"]*)".*$/\1/')
  if [ -z "$pattern" ] || [ -z "$replacement" ]; then fail "$name: cannot read the redaction pattern"; return; fi
  sed_replacement=$(printf '%s' "$replacement" | sed -E 's/\$([0-9])/\\\1/g')

  redact() { printf '%s\n' "$1" | sed -E "s#${pattern//#/\\#}#${sed_replacement}#"; }
  expect() { # expect <request target> <expected logged value>
    local got
    got=$(redact "$1")
    if [ "$got" = "$2" ]; then pass "$name: '$1' -> '$got'"; else fail "$name: '$1' -> '$got' (expected '$2')"; fi
  }
  expect '/assets/CAPSECRET123/hero.webp?X-Sig=SIGSECRET' '/assets'
  expect '/ASSETS/CAPSECRET124/hero.webp' '/ASSETS'
  expect '//assets/CAPSECRET125/hero.webp' '/'
  expect '/media/CAPSECRET456/clip.mp4' '/media'
  expect '/media/CAPSECRET789/a/b/c?range=1' '/media'
  expect '/join/JOINSECRET9' '/join'
  expect '/join%2FJOINSECRET10' '/join'
  expect '/JOIN%2fJOINSECRET11' '/JOIN'
  expect '/assets%2FCAPSECRET127/x' '/assets'
  expect '/join;jsessionid=JOINSECRET12' '/join'
  expect '/play/ROOM42?token=INVITESECRET' '/play'
  expect '/api/health' '/api'
  expect '/assets' '/assets'
  expect '/' '/'
  expect 'http://host/assets/CAPSECRET126/x?k=SIGSECRET' ''
  expect 'host:443' ''
  local sample
  for sample in '/assets/CAPSECRET1/x' '/ASSETS/CAPSECRET2/y?z=1' '//media/CAPSECRET3' '/join/JOINSECRET4' \
                'http://h/x/CAPSECRET5?a=SIGSECRET' '/some/new/route/CAPSECRET6?t=SIGSECRET' \
                '/join%2FJOINSECRET7' '/JOIN%2fJOINSECRET8' '/join;k=JOINSECRET9'; do
    case "$(redact "$sample")" in
      *SECRET*) fail "$name: a secret survives in '$sample'" ;;
    esac
  done
}

edge_dir=${DF_EDGE_DIR:-$root/deploy/edge}
for file in "$edge_dir/Caddyfile.production" "$edge_dir/Caddyfile.staging"; do
  if [ -f "$file" ]; then check_file "$file"; else fail "missing $file"; fi
done

# Extra site files (the www redirect) must not define a logger: every logger lives in the two files checked above, where
# the filter is verified, so a site added later cannot start logging unfiltered request URIs.
shopt -s nullglob
for extra in "$edge_dir"/extra/*.caddy; do
  if grep -Eq '^[[:space:]]*log([[:space:]]|\{|$)' "$extra"; then
    fail "$(basename "$extra"): an extra site file must not define a logger"
  else
    pass "$(basename "$extra"): defines no logger"
  fi
done

if [ "$live" -eq 1 ]; then
  port=$(sed -n 's/^DF_STAGING_PORT=//p' "$root/deploy/compose/staging.env")
  compose=(docker compose
    --project-directory "$root/deploy/compose"
    --file "$root/deploy/compose/staging.compose.yml"
    --env-file "$root/deploy/images/images.lock"
    --env-file "$root/deploy/compose/staging.env")
  marker="LIVECAP$(date +%s)$RANDOM"
  base="http://127.0.0.1:$port"
  headers=$(curl --silent --show-error --max-time 5 --dump-header - --output /dev/null \
    --header "Referer: $base/assets/$marker/page" \
    "$base/assets/$marker/x.webp?sig=$marker" || true)
  if printf '%s' "$headers" | grep -iq '^referrer-policy: no-referrer'; then
    pass "live: Referrer-Policy no-referrer on /assets/*"
  else
    fail "live: no Referrer-Policy no-referrer on /assets/*"
  fi
  curl --silent --max-time 5 --output /dev/null "$base/ASSETS/$marker/upper" || true
  curl --silent --max-time 5 --output /dev/null "$base/join/$marker" || true
  curl --silent --max-time 5 --output /dev/null "$base//assets/$marker/double" || true
  curl --silent --max-time 5 --output /dev/null "$base/join%2F$marker" || true
  # With the upstream down, Caddy writes an error-log entry that carries the request: the default logger filter must cover it.
  "${compose[@]}" stop control-plane >/dev/null
  curl --silent --max-time 8 --output /dev/null --header "Referer: $base/join/$marker" "$base/join/$marker?x=$marker" || true
  sleep 1
  logs=$("${compose[@]}" logs --no-color edge 2>&1)
  "${compose[@]}" start control-plane >/dev/null
  if printf '%s' "$logs" | grep -q "$marker"; then
    fail "live: the capability marker appears in the edge log (access or error)"
  else
    pass "live: the capability marker is absent from the edge log, access and error entries included"
  fi
  if printf '%s' "$logs" | grep -q '"/assets"'; then
    pass "live: the edge log records the redacted path /assets"
  else
    fail "live: no redacted '/assets' access entry found in the edge log"
  fi
fi

if [ "$failures" -ne 0 ]; then echo "$failures check(s) failed"; exit 1; fi
echo "edge redaction checks passed"
