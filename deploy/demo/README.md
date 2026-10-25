# Demo host setup

The demo web edge is nginx. `provision-demo.sh` installs nginx, Certbot and its
nginx plugin from the Ubuntu archive, then installs
`nginx/dungeonflux.conf` as the site configuration. The Docker bridge PostgreSQL
listener is optional because the demo Compose service uses host networking.

On the host PostgreSQL instance, create the default gameplay role and database
in `psql` as an administrator. Enter the role password at the interactive
prompt:

```sql
CREATE ROLE df_gameplay_demo_admin_20261004 LOGIN;
\password df_gameplay_demo_admin_20261004
CREATE DATABASE df_gameplay_demo_20261004_r03 OWNER df_gameplay_demo_admin_20261004;
```

Keep PostgreSQL listening on localhost. Create `/etc/dungeonflux/db_password`
with the same password as its first line, then set owner `root:root` and mode
`0600` (for example, `sudo install -o root -g root -m 0600 /dev/null
/etc/dungeonflux/db_password` followed by `sudoedit`). Compose mounts it read-only
into the game container.

Create `/etc/dungeonflux/db_password` with that same password as its first line.
Set the file owner to `root:root` and permissions to `0600`. Compose mounts it
read-only into the game container.

Configure the Resend credential and any mail overrides in a root-owned file under
`/etc/dungeonflux/*.env`. Set `DF_RESEND_API_KEY` from the Resend dashboard; optionally set
`DF_MAIL_FROM`, `DF_MAIL_REPLY_TO`, and `DF_MAIL_DAILY_CAP` there. Keep the human support mailbox
in Google Workspace.

## Hosted account portal

Deployment acceptance is pending the privileged-migration prerequisite described
below. These are coordinator-run files, not evidence of a deployed portal.

The portal (`df-portal-hosted`, crate `df-hosted`) runs as a host binary under
`df-portal-hosted.service`, not as a container. `demo-release.yml` builds a
second image, `ghcr.io/dungeonfluxgame/dungeonflux-portal`, from
`Dockerfile.portal` in the same run as the game image (one commit, a matched
pair, independent digests and rollback lines). A second image, rather than a
target inside the game image, because the game image's entrypoint and
`check-profile.sh` inspection are the game fixture's, and the portal must roll
back without redeploying the game.

Provision its database once (and re-run after any reprovision; it is
idempotent and keeps passwords already in the env file). The host provisioner
creates the `dungeonflux` user/group and `/root/.pgpass`; this script uses that
root-only PostgreSQL administrator password file, never an argv password:

```bash
sudo deploy/demo/provision-portal-db.sh
```

That creates database `df_portal_demo` owned by the NOLOGIN
`df_portal_owner`, the LOGIN migrator `df_portal_migrator` (connection limit
4) and the LOGIN runtime `df_portal_runtime` (connection limit 16), generates
any missing passwords on the host into `/etc/dungeonflux/portal.env`
(`root:dungeonflux 0640`, never in this repo), sets the role
`transaction_timeout`/`statement_timeout` defaults hosted startup requires,
and grants the runtime role DML on the schemas it uses. Fill the remaining
names in `portal.env.names` (WorkOS, Resend, Stripe test keys, campaign and
trial HMAC keys) from the host secret store. Accounts stay closed:
`DF_ACCOUNTS_OPEN=false` is required and the deploy refuses `true`.

Deploy or roll back (the restart runs pending migrations as the migrator;
the deploy health-checks `http://127.0.0.1:8099/app/healthz` and restores the
previous binary, bundle, unit and digest on failure). Deployment uses an exclusive
host lock, a private staging directory and atomic binary replacement. It stops
the old service before swapping the bundle and accepts only an active service
with an exact `ok` health body. Each of 18 probes is limited to three seconds,
with five seconds between probes (at most 144 seconds plus local service work):

```bash
deploy/demo/deploy-portal.sh latest
deploy/demo/deploy-portal.sh sha256:<digest>
deploy/demo/deploy-portal.sh --rollback
```

Accounts stay closed even if an env file tries to select development mode:
the systemd command fixes `DF_ACCOUNTS_OPEN=false`, `DF_PROFILE=demo`, the
loopback listener and the installed bundle path. The shared telemetry sink and
spool use `/var/lib/dungeonflux-portal/telemetry`, writable through the service's
private `StateDirectory` despite `ProtectSystem=strict`.

Admin access stays off. `ADMINFLOW_HOOK` follows
`artifacts/tmp/agents/ADMINFLOW/design.md`: `DF_OPERATOR_TUNNEL=false`, later
a distinct loopback-only operator listener plus loopback TLS reached by SSH.
No public peer-IP exemption is allowed. The unit fixes the flag false and deploy
refuses true until the listener, authorization and TLS runbook are accepted.

### Privileged migration prerequisite

The current startup migrator is `NOCREATEROLE`, while the included persistence
migration `0018_runtime_roles.sql` creates/alters roles and grants
`pg_signal_backend`. It cannot succeed under that migrator on a fresh database.
Do not grant the serving process superuser or cluster role-management privileges
to work around this. The coordinator must resolve the crate-owner request in
`artifacts/tmp/agents/HOSTEDDEPLOY/request.md` before deployment acceptance:
separate privileged host bootstrap from ordinary migrations and provide an
offline migration command. The current startup path retains migration credentials
in the serving process, unlike the service-operations target of offline-only
migration credentials. No crate changes are part of this lane.

Rollback restores application files and checks the previous service; it does
not reverse database migrations. Only backward-compatible migrations are safe
for automatic binary rollback. A failed recovery leaves the private staging
directory for the coordinator and reports failure; it never claims a healthy
rollback. Take a verified database backup before the first migration run.

## Memory budget (512 MB droplet)

| Consumer | Cap | Basis |
| --- | --- | --- |
| PostgreSQL 18 | No hard cap | `shared_buffers=64MB`, global `work_mem=2MB`, max 40 connections; portal runtime overrides `work_mem=512kB`, parallel query workers 0 |
| Game container | Stopped for portal acceptance | Its separate 300m cap does not fit this portal budget concurrently |
| Portal service | 128M hard, 96M high, no swap | `MemoryMax`/`MemoryHigh`/`MemorySwapMax`; `MALLOC_ARENA_MAX=2` |
| nginx + Docker + OS | Remaining RAM | `worker_processes 1`; host swap is spill, not extra physical memory |

The portal's fixed Rust pools are accounts 4, campaign write 2 + read 2;
finance, checkout, campaign accounts, sessions, funding and optional support use
one connection each (14 runtime connections total). Runtime role limit 16 leaves
two spare; migrator limit 4 covers its temporary account pool 2 plus migration
connection 1. These are PostgreSQL connection limits, not pool-size knobs.
Smaller pools require a crate-owner configuration change. Two simultaneous
production password hashes use 38 MiB (`19 * 1024` KiB each) before application
overhead. Role work memory applies per sort/hash, so connection limits do not
constitute a PostgreSQL memory cap. Measure RSS/cgroup peaks during migrations
and sign-in on the droplet before claiming the 512 MB host qualified. Running
the game concurrently requires a separately measured budget or a larger host.

In the Resend dashboard, copy the DNS records for the `mail.dungeonfluxdnd.com` sending domain
into the DNS provider. Add the SPF TXT value using the dashboard-provided `include:` target, the
DKIM CNAME records using the dashboard-provided names and targets, and the return-path record
using the dashboard-provided name and target under `mail.dungeonfluxdnd.com`. These values are
account-specific; do not substitute sample values. Wait for Resend to verify the records.
