#!/usr/bin/env bash
set -Eeuo pipefail
export DEBIAN_FRONTEND=noninteractive

if [[ ${EUID} -ne 0 ]]; then
  printf '%s\n' 'Run this script as root.' >&2
  exit 1
fi

if dpkg-query -W -f='${db:Status-Abbrev}' caddy 2>/dev/null | grep -Eq '^(ii|rc)'; then
  apt-get purge -y caddy
fi
rm -f /etc/apt/sources.list.d/caddy-stable.list /usr/share/keyrings/caddy-stable-archive-keyring.gpg

apt-get update
apt-get -y upgrade
apt-get install -y ca-certificates curl gnupg tzdata unattended-upgrades ufw fail2ban jq openssl

timedatectl set-timezone UTC
cat > /etc/apt/apt.conf.d/20auto-upgrades <<'EOF'
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
EOF
cat > /etc/apt/apt.conf.d/52unattended-upgrades-local <<'EOF'
Unattended-Upgrade::Automatic-Reboot "false";
EOF
rm -f /etc/apt/apt.conf.d/52unattended-upgrades-local
systemctl enable --now unattended-upgrades

if ! swapon --show=NAME --noheadings | grep -Fxq /swapfile; then
  if [[ ! -e /swapfile ]]; then
    fallocate -l 2G /swapfile || dd if=/dev/zero of=/swapfile bs=1M count=2048 status=progress
    chmod 600 /swapfile
    mkswap /swapfile
  fi
  swapon /swapfile
fi
grep -Eq '^[[:space:]]*/swapfile[[:space:]]' /etc/fstab || printf '%s\n' '/swapfile none swap sw 0 0' >> /etc/fstab
cat > /etc/sysctl.d/60-dungeonflux-memory.conf <<'EOF'
vm.swappiness=10
EOF
sysctl --system >/dev/null

install -d -m 0755 /usr/share/postgresql-common/pgdg
curl -fsSL https://www.postgresql.org/media/keys/ACCC4CF8.asc | gpg --dearmor --yes -o /usr/share/postgresql-common/pgdg/apt.postgresql.org.gpg
chmod 0644 /usr/share/postgresql-common/pgdg/apt.postgresql.org.gpg
. /etc/os-release
printf 'deb [signed-by=/usr/share/postgresql-common/pgdg/apt.postgresql.org.gpg] https://apt.postgresql.org/pub/repos/apt %s-pgdg main\n' "$VERSION_CODENAME" > /etc/apt/sources.list.d/pgdg.list

install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
chmod a+r /etc/apt/keyrings/docker.asc
printf 'Types: deb\nURIs: https://download.docker.com/linux/ubuntu\nSuites: %s\nComponents: stable\nArchitectures: %s\nSigned-By: /etc/apt/keyrings/docker.asc\n' "${UBUNTU_CODENAME:-$VERSION_CODENAME}" "$(dpkg --print-architecture)" > /etc/apt/sources.list.d/docker.sources
apt-get update

apt-get install -y --no-install-recommends postgresql-18 postgresql-18-pgvector nginx certbot python3-certbot-nginx
apt-get purge -y postgresql-18-jit libllvm19
systemctl enable postgresql

# Docker
apt-get remove -y docker.io docker-compose docker-compose-v2 docker-doc docker-buildx podman-docker containerd runc >/dev/null 2>&1 || true
apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
install -d -m 0755 /etc/docker
cat > /etc/docker/daemon.json <<'EOF'
{
  "log-driver": "local",
  "log-opts": {
    "max-size": "10m",
    "max-file": "3"
  },
  "live-restore": true,
  "userland-proxy": false,
  "bip": "172.31.255.1/24",
  "default-address-pools": [
    { "base": "172.30.0.0/16", "size": 24 }
  ]
}
EOF
python3 -m json.tool /etc/docker/daemon.json >/dev/null
systemctl enable docker.service containerd.service
systemctl reset-failed docker.service docker.socket >/dev/null 2>&1 || true
systemctl enable --now docker.socket containerd.service
systemctl restart docker.service
ip link set dev docker0 up
docker run --rm hello-world
docker image rm hello-world
install -d -o root -g root -m 0750 /opt/dungeonflux/demo
install -o root -g root -m 0640 /dev/null /opt/dungeonflux/demo/current.digest
install -o root -g root -m 0640 /dev/null /opt/dungeonflux/demo/previous.digest
ufw allow in on docker0 from 172.31.255.0/24 to 172.31.255.1 port 5432 proto tcp

install -d -m 0755 /etc/systemd/system/apt-daily.timer.d /etc/systemd/system/apt-daily-upgrade.timer.d /etc/apt/apt.conf.d
cat > /etc/systemd/system/apt-daily.timer.d/override.conf <<'EOF'
[Timer]
OnCalendar=
OnCalendar=Sun *-*-* 05:30:00 America/New_York
EOF
cat > /etc/systemd/system/apt-daily-upgrade.timer.d/override.conf <<'EOF'
[Timer]
OnCalendar=
OnCalendar=Sun *-*-* 06:00:00 America/New_York
EOF
cat > /etc/apt/apt.conf.d/52dungeonflux-maintenance <<'EOF'
Unattended-Upgrade::Automatic-Reboot "true";
Unattended-Upgrade::Automatic-Reboot-WithUsers "true";
Unattended-Upgrade::Automatic-Reboot-Time "now";
EOF
systemctl daemon-reload
systemctl enable --now apt-daily.timer apt-daily-upgrade.timer

pg_ctlcluster 18 main start
pg_conf_dir=/etc/postgresql/18/main
cat > "$pg_conf_dir/conf.d/60-dungeonflux.conf" <<'EOF'
shared_buffers=64MB
effective_cache_size=192MB
work_mem=2MB
maintenance_work_mem=32MB
max_connections=40
# Compose uses host networking; the Docker bridge listener is optional.
listen_addresses='localhost,172.31.255.1'
password_encryption=scram-sha-256
max_prepared_transactions=0
jit=off
EOF
cat > "$pg_conf_dir/pg_hba.conf" <<'EOF'
local all all scram-sha-256
host all all 127.0.0.1/32 scram-sha-256
host all all ::1/128 scram-sha-256
host all all 172.31.255.0/24 scram-sha-256
EOF

if ! grep -q '^127\.0\.0\.1:5432:\*:postgres:' /root/.pgpass 2>/dev/null; then
  postgres_password=$(openssl rand -hex 32)
  install -m 0600 /dev/null /root/.pgpass
  sed -i '/^127\.0\.0\.1:5432:\*:postgres:/d' /root/.pgpass
  printf '127.0.0.1:5432:*:postgres:%s\n' "$postgres_password" >> /root/.pgpass
  chown root:root /root/.pgpass
  runuser -u postgres -- psql -v ON_ERROR_STOP=1 -c "ALTER ROLE postgres WITH PASSWORD '$postgres_password'"
  unset postgres_password
fi
chmod 0600 /root/.pgpass
chown root:root /root/.pgpass
systemctl restart postgresql

for attempt in $(seq 1 30); do
  if pg_isready -h 127.0.0.1 -p 5432 -q; then break; fi
  sleep 1
done
pg_isready -h 127.0.0.1 -p 5432
if ! psql -h 127.0.0.1 -U postgres -d postgres -tAc "SELECT 1 FROM pg_database WHERE datname='dungeonflux'" | grep -qx 1; then
  psql -h 127.0.0.1 -U postgres -d postgres -v ON_ERROR_STOP=1 -c 'CREATE DATABASE dungeonflux'
fi
psql -h 127.0.0.1 -U postgres -d dungeonflux -v ON_ERROR_STOP=1 -c 'CREATE EXTENSION IF NOT EXISTS vector'

id -u dungeonflux >/dev/null 2>&1 || useradd --system --home-dir /var/lib/dungeonflux --shell /usr/sbin/nologin dungeonflux
install -d -o dungeonflux -g dungeonflux -m 0755 /opt/dungeonflux/bin /opt/dungeonflux/web /opt/dungeonflux/assets/ui /opt/dungeonflux/content
install -d -o root -g dungeonflux -m 0750 /etc/dungeonflux
install -d -o dungeonflux -g dungeonflux -m 0750 /var/lib/dungeonflux /var/log/dungeonflux
cat > /etc/dungeonflux/server.env <<'EOF'
DF_GAMEPLAY_DATABASE_PORT=5432
DF_GAMEPLAY_LISTEN=127.0.0.1:63309
DF_GAMEPLAY_PACK_DIR=/opt/dungeonflux/content/harbor-journey
DF_GAMEPLAY_ADVENTURE_TABLES=/opt/dungeonflux/content/adventure-tables
DF_GAMEPLAY_LOAD_DOTENV=off
EOF
chown root:dungeonflux /etc/dungeonflux/server.env
chmod 0640 /etc/dungeonflux/server.env

install -d -o root -g root -m 0755 /opt/dungeonflux/site /opt/dungeonflux/app
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
nginx_template="$script_dir/nginx/dungeonflux.conf"
if [[ ! -f "$nginx_template" ]]; then
  nginx_template=/tmp/df-demo-dungeonflux.conf
fi
if [[ ! -f "$nginx_template" ]]; then
  printf '%s\n' 'Missing deploy/demo/nginx/dungeonflux.conf; upload it to /tmp/df-demo-dungeonflux.conf when piping this script.' >&2
  exit 1
fi
sed 's/__GAME_PORT__/63309/g' "$nginx_template" > /etc/nginx/sites-available/dungeonflux
ln -sfn /etc/nginx/sites-available/dungeonflux /etc/nginx/sites-enabled/dungeonflux
rm -f /etc/nginx/sites-enabled/default
install -d -m 0755 /etc/nginx/snippets
if [[ ! -e /etc/nginx/snippets/dungeonflux-hsts.conf ]]; then
  install -m 0644 /dev/null /etc/nginx/snippets/dungeonflux-hsts.conf
fi
sed -i -E 's/^[[:space:]]*worker_processes[[:space:]]+[^;]+;/worker_processes 1;/' /etc/nginx/nginx.conf
cat > /etc/nginx/conf.d/dungeonflux-tuning.conf <<'EOF'
client_body_buffer_size 16k;
client_header_buffer_size 1k;
large_client_header_buffers 2 4k;
EOF
cat > /etc/logrotate.d/dungeonflux-nginx <<'EOF'
/var/log/nginx/dungeonflux.access.log /var/log/nginx/dungeonflux.error.log {
    daily
    rotate 7
    compress
    delaycompress
    missingok
    notifempty
    create 0640 www-data adm
    sharedscripts
    postrotate
        systemctl kill -s USR1 nginx.service >/dev/null 2>&1 || true
    endscript
}
EOF
nginx -t
systemctl enable --now nginx
systemctl restart nginx

cat > /etc/systemd/system/dungeonflux-server.service <<'EOF'
[Unit]
Description=DungeonFlux demo game server
After=network-online.target postgresql.service
Wants=network-online.target

[Service]
Type=simple
User=dungeonflux
Group=dungeonflux
WorkingDirectory=/opt/dungeonflux
EnvironmentFile=-/etc/dungeonflux/server.env
ExecStart=/opt/dungeonflux/bin/df-transport-fixture --gameplay-demo /opt/dungeonflux/web /opt/dungeonflux/assets/ui
Restart=on-failure
RestartSec=5s
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
MemoryMax=320M
ReadWritePaths=/var/lib/dungeonflux /var/log/dungeonflux

[Install]
WantedBy=multi-user.target
EOF
systemctl daemon-reload
systemctl disable dungeonflux-server.service >/dev/null 2>&1 || true

cat > /etc/fail2ban/jail.d/dungeonflux-sshd.local <<'EOF'
[sshd]
enabled = true
backend = systemd
EOF
systemctl enable --now fail2ban
ufw default deny incoming
ufw default allow outgoing
ufw limit 22/tcp
ufw allow 80/tcp
ufw allow 443/tcp
ufw --force enable

printf '\n-- Installed package versions --\n'
dpkg-query -W -f='${binary:Package} ${Version}\n' ca-certificates curl gnupg tzdata unattended-upgrades ufw fail2ban jq openssl postgresql-18 postgresql-18-pgvector nginx certbot python3-certbot-nginx docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin 2>/dev/null | sort
printf '\n-- Memory --\n'
free -m
printf '\n-- Disk --\n'
df -h /
printf '\n-- PostgreSQL vector extension --\n'
psql -h 127.0.0.1 -U postgres -d dungeonflux -c "SELECT extversion FROM pg_extension WHERE extname='vector'"
printf '\n-- Service states --\n'
systemctl is-active postgresql nginx docker containerd do-agent fail2ban
printf '\n-- Firewall --\n'
ufw status
printf '\n-- PostgreSQL, nginx, Docker and containerd idle RSS (KiB) --\n'
ps -C postgres,nginx,dockerd,containerd -o pid=,comm=,rss= --sort=comm
