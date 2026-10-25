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

In the Resend dashboard, copy the DNS records for the `mail.dungeonfluxdnd.com` sending domain
into the DNS provider. Add the SPF TXT value using the dashboard-provided `include:` target, the
DKIM CNAME records using the dashboard-provided names and targets, and the return-path record
using the dashboard-provided name and target under `mail.dungeonfluxdnd.com`. These values are
account-specific; do not substitute sample values. Wait for Resend to verify the records.
