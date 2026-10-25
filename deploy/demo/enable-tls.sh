#!/usr/bin/env bash
set -Eeuo pipefail

readonly expected_ip='157.230.52.8'
readonly names=(dungeonfluxdnd.com www.dungeonfluxdnd.com play.dungeonfluxdnd.com tv.dungeonfluxdnd.com)

if [[ ${EUID} -ne 0 ]]; then
  printf '%s\n' 'Run this script as root.' >&2
  exit 1
fi

for name in "${names[@]}"; do
  if ! getent ahostsv4 "$name" | awk -v expected="$expected_ip" '$1 == expected { found = 1 } END { exit !found }'; then
    printf 'DNS for %s does not resolve to %s.\n' "$name" "$expected_ip" >&2
    exit 1
  fi
done

certbot certonly --nginx --dry-run \
  -d dungeonfluxdnd.com \
  -d www.dungeonfluxdnd.com \
  -d play.dungeonfluxdnd.com \
  -d tv.dungeonfluxdnd.com \
  --agree-tos \
  --register-unsafely-without-email

certbot --nginx \
  --reinstall \
  -d dungeonfluxdnd.com \
  -d www.dungeonfluxdnd.com \
  -d play.dungeonfluxdnd.com \
  -d tv.dungeonfluxdnd.com \
  --redirect \
  --agree-tos \
  --register-unsafely-without-email

python3 - <<'PY'
from pathlib import Path

config_path = Path('/etc/nginx/sites-available/dungeonflux')
lines = config_path.read_text(encoding='utf-8').splitlines()
lines = [
    'client_max_body_size 10m;' if line.strip().startswith('client_max_body_size ') else line
    for line in lines
]
redirects = {
    'dungeonfluxdnd.com': 'https://dungeonfluxdnd.com$request_uri',
    'www.dungeonfluxdnd.com': 'https://dungeonfluxdnd.com$request_uri',
    'play.dungeonfluxdnd.com': 'https://dungeonfluxdnd.com/play',
    'tv.dungeonfluxdnd.com': 'https://dungeonfluxdnd.com/tv',
}
updated = []
replaced = set()
index = 0
while index < len(lines):
    if lines[index].strip() != 'server {':
        updated.append(lines[index])
        index += 1
        continue
    start = index
    depth = 0
    while index < len(lines):
        depth += lines[index].count('{') - lines[index].count('}')
        index += 1
        if depth == 0:
            break
    block = lines[start:index]
    configured_names = set()
    server_names = set()
    for line in block:
        stripped = line.strip()
        if stripped.startswith('server_name '):
            configured = stripped.removeprefix('server_name ').rstrip(';').split()
            configured_names.update(configured)
            server_names.update(set(configured) & set(redirects))
    if 'listen 80;' in [line.strip() for line in block] and server_names:
        if len(server_names) != 1:
            raise SystemExit('Refusing an ambiguous nginx HTTP redirect server block.')
        name = server_names.pop()
        if name in replaced:
            raise SystemExit(f'Refusing duplicate HTTP redirect block for {name}.')
        replaced.add(name)
        updated.extend(
            [
                'server {',
                '    listen 80;',
                '    listen [::]:80;',
                f'    server_name {name};',
                '    location / {',
                f'        return 301 {redirects[name]};',
                '    }',
                '}',
            ]
        )
    elif '157.230.52.8' in configured_names:
        updated.extend(
            line
            for line in block
            if line.strip() != 'include /etc/nginx/snippets/dungeonflux-hsts.conf;'
        )
    else:
        updated.extend(block)

if replaced != set(redirects):
    missing = ', '.join(sorted(set(redirects) - replaced))
    raise SystemExit(f'Expected four Certbot HTTP redirect blocks; missing: {missing}')

if not any(line.strip() == 'server_name 157.230.52.8;' for line in updated):
    updated.extend(
        [
            '',
            'server {',
            '    listen 80;',
            '    listen [::]:80;',
            '    server_name 157.230.52.8;',
            '    root /opt/dungeonflux/site;',
            '    index index.html;',
            '    add_header X-Content-Type-Options nosniff always;',
            '    add_header Referrer-Policy no-referrer always;',
            '    add_header Content-Security-Policy "frame-ancestors \'none\'" always;',
            '    add_header X-Frame-Options DENY always;',
            '    location = /healthz {',
            '        access_log off;',
            '        default_type text/plain;',
            '        return 200 "ok\\n";',
            '    }',
            '    location ^~ /app/ {',
            '        alias /opt/dungeonflux/app/;',
            '        index index.html;',
            '        try_files $uri $uri/ /app/index.html;',
            '    }',
            '    location = /app {',
            '        return 301 /app/;',
            '    }',
            '    location / {',
            '        try_files $uri $uri/ $uri/index.html =404;',
            '    }',
            '}',
        ]
    )

config_path.write_text('\n'.join(updated) + '\n', encoding='utf-8')
PY

install -d -m 0755 /etc/nginx/snippets
printf '%s\n' 'add_header Strict-Transport-Security "max-age=31536000" always;' > /etc/nginx/snippets/dungeonflux-hsts.conf
nginx -t
systemctl reload nginx

for name in "${names[@]}"; do
  curl --fail --silent --show-error --output /dev/null "https://${name}/"
done

test "$(curl --silent --show-error --output /dev/null --write-out '%{http_code}' https://www.dungeonfluxdnd.com/test-path)" = 301
test "$(curl --silent --show-error --output /dev/null --write-out '%{redirect_url}' https://www.dungeonfluxdnd.com/test-path)" = 'https://dungeonfluxdnd.com/test-path'
test "$(curl --silent --show-error --output /dev/null --write-out '%{redirect_url}' http://www.dungeonfluxdnd.com/test-path)" = 'https://dungeonfluxdnd.com/test-path'
test "$(curl --silent --show-error --output /dev/null --write-out '%{redirect_url}' http://play.dungeonfluxdnd.com/)" = 'https://dungeonfluxdnd.com/play'
test "$(curl --silent --show-error --output /dev/null --write-out '%{redirect_url}' http://tv.dungeonfluxdnd.com/)" = 'https://dungeonfluxdnd.com/tv'
test "$(curl --silent --show-error --output /dev/null --write-out '%{redirect_url}' https://play.dungeonfluxdnd.com/)" = 'https://dungeonfluxdnd.com/play'
test "$(curl --silent --show-error --output /dev/null --write-out '%{redirect_url}' https://tv.dungeonfluxdnd.com/)" = 'https://dungeonfluxdnd.com/tv'
