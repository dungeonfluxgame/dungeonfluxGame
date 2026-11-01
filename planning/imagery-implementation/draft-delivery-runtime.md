# Hosted private draft delivery

The hosted account app mounts `GET /app/api/draft-images/{asset}` (and `HEAD`)
alongside `app_api::compose` before the common private/proxy/account/telemetry
middleware. The default selector is `display`; `?variant=master` selects the
exact winning master. The delivery path reads persisted account-owned drafts;
it performs no synthesis, supplier call, reservation or charge.

## Configuration

| Environment setting | Meaning |
| --- | --- |
| `DF_DRAFT_DELIVERY_ENABLED` | `true` enables configured delivery; missing or `false` mounts the disabled source. Other values fail configuration. |
| `DF_MEDIA_OBJECT_ROOT` | Existing absolute native object-store root used by the writer. No temporary-root fallback or path blacklist. |
| `DF_MEDIA_CATALOG_DATABASE_URL` | Existing shared writer catalog database, accessed through an ordinary media reader login. |
| `DF_DRAFT_DISCLOSURE_DATABASE_URL` | Separate narrow disclosure-reader login for the same database and endpoint as accounts and catalog. |
| `DF_DRAFT_MAX_IMAGE_BYTES` | Positive bound for one image buffer and the native store read. |
| `DF_DRAFT_MAX_DELIVERY_BYTES` | Positive total buffered-byte bound, at least one maximum image. The singleton slot count is `floor(total / image)`, conservatively reserving one maximum image per response allocation. |
| `DF_DRAFT_OPERATION_TIMEOUT_MS` | Positive finite total request deadline, including authentication, buffering, reauthentication and the final snapshot. |

All enabled settings are required; the delivery connections use the configured
`DF_GAMEPLAY_DATABASE_TLS` transport and CA. Database hostname, host address,
port and database name must match the account connection. Configure the writer
with these same shared media settings; deployment remains responsible for
mounting the correct writer volume. There is no second delivery-only root.
Startup refuses a missing volume, missing catalog schema or inaccessible reader.
It runs no catalog DDL: the host migrator/provisioner installs schema and grants.
The catalog and disclosure logins must differ. Startup rejects a disclosure
login with administrative role flags, asset/account/session mutation rights, or
access to session token digests or CSRF secrets. Neither URL appears in errors.

Disabled delivery authenticates normally: an anonymous request gets an empty
404; an authenticated request gets an empty 503. Every result includes
`Cache-Control: private, no-store`, `X-Content-Type-Options: nosniff` and
`Cross-Origin-Resource-Policy: same-origin`. Common middleware preserves that
cache policy and adds the normal referrer policy. Enabled but invalid settings
fail startup instead of silently serving another store.

## Authority and ownership

`Hosted::open` constructs one `Tunables` value, passes it into `CustomerService`
and copies its `sessions` policy into the delivery adapter. The adapter retains
the disclosure connection and its owned driver; the catalog retains its bounded
pool. The singleton delivery budget is retained through unpolled HTTP bodies
and any aliases of emitted `Bytes`; consumption, cancellation or dropping the
last alias releases the slot. Holding an authorized response does not authorize
another read.

The final disclosure decision is one PostgreSQL statement joining session,
account, asset and the exact winning variant. It compares the live session
policy and absolute expiry, account status/generation, owner audience,
publication MIME/aggregate manifest, selected variant manifest/version,
attempt/write and suppression generation against the buffered basis. All bytes
are already buffered and verified. There is no subsequent storage await before
response construction. A revocation/tombstone committed before that statement's
snapshot denies; a commit after the snapshot cannot retract already authorized
buffered bytes. This is the explicit disclosure linearization contract.

## Provisioning grants

Create fresh deployment logins through the deployment provisioner. Roles must
not inherit broader privileges, and these schema/table grants run in the actual
application database. The ordinary catalog reader needs `CONNECT` on that
database, `USAGE ON SCHEMA media` and `SELECT ON media.schema_migration,
media.asset, media.asset_variant`. It needs no account access or mutation/DDL.
The existing catalog migration versions must already be installed.

The separate disclosure reader needs `CONNECT`, `USAGE ON SCHEMA accounts,
media`, and these column reads only:

```sql
GRANT SELECT (session_id, account_id, credential_generation, created_at_ms,
 last_seen_at_ms, revoked_at_ms, absolute_expires_at_ms)
 ON accounts.web_session TO draft_disclosure_reader;
GRANT SELECT (account_id, status, credential_generation)
 ON accounts.account TO draft_disclosure_reader;
GRANT SELECT (asset_id, state, kind, provenance, campaign_id, charge_scope_kind,
 charge_scope_id, origin_account, audience_kind, audience_account, attempt,
 winning_writes, mime, byte_length, sha256, suppression_generation)
 ON media.asset TO draft_disclosure_reader;
GRANT SELECT (asset_id, attempt, write_id, variant, state, object_domain,
 object_version, byte_length, sha256)
 ON media.asset_variant TO draft_disclosure_reader;
```

Do not grant session token/CSRF columns, account email, object keys, finance
access, table-wide account SELECT or any mutation privilege to this login.
Production migration/grant registration belongs to the coordinator.

## Focused verification

`cargo test -p df-hosted --lib draft_composition` covers configuration.
Build the hosted test executable with `cargo test -p df-hosted --lib --no-run`
and run that printed executable from the repository root with
`draft_images_tests --test-threads=1`. The existing hosted finance startup reads
`content/customer-prices.json` relative to the working directory, so invoking
the full hosted composition case from Cargo's package working directory fails
before delivery. The direct executable run exercises real PostgreSQL, native bytes, privacy middleware, two read roles, exact
master/display variants, denied mutation/secret access, revocation, expiry,
account closure, suppression and manifest changes, blocked-query deadline and
retained HTTP allocation permits. Set `REQUIRE_SCRATCH_PG=1` (or the legacy
`DUNGEONFLUX_REQUIRE_SCRATCH_PG=1`) so unavailable PostgreSQL fails the run.
The owned fixture accepts only loopback port `55471` (host and user via
`DUNGEONFLUX_RUNTIME_PG_HOST` / `DUNGEONFLUX_RUNTIME_PG_USER`); each test owns a
fresh database, closes its connections, drops that database and then drops only
the fresh roles it successfully created. No preemptive role deletion is used.
Independent running-output review remains the coordinator's acceptance gate.

The bounded implementation run passed all 16 PostgreSQL cases (including full
`Config::parse` / `Hosted::open` and preservation of published bytes through
legacy hosted migration adoption), all three delivery configuration cases and
all 14 existing hosted configuration cases. Evidence is retained in
`artifacts/tmp/imagery-20261027/wave2/hosted-draft-sol-final-pg.log`,
`hosted-draft-sol-final-config.log` and `hosted-draft-sol-existing-config.log`.
Hosted package linting passed with
`cargo clippy -p df-hosted --all-targets --no-deps -j 2 -- -D warnings`; evidence
is `hosted-draft-sol-owned-clippy.log`. The required strict command including
workspace dependencies remains an integration gate: the attempted run stopped
in concurrently changing `df-imagery`; see `hosted-draft-sol-final-clippy.log`.
The scoped lint result does not replace that gate. No assertion was disabled.
