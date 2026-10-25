#!/usr/bin/env bash
set -Eeuo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)
config="$root/deploy/demo/nginx/dungeonflux.conf"
[[ $(grep -Fc 'error_page 502 503 504 = @accounts_closed;' "$config") -eq 2 ]]
[[ $(grep -Fc 'location @accounts_closed {' "$config") -eq 2 ]]
[[ $(grep -Fc 'root /opt/dungeonflux/site;' "$config") -ge 2 ]]
[[ $(grep -Fc 'try_files $uri $uri/ $uri/index.html /app/index.html;' "$config") -eq 2 ]]
printf '%s\n' 'nginx account fallback config test passed'
