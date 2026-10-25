# Service operations, security and recovery

Status: concrete proposed defaults and qualification procedures; nothing measured or deployed.

## Initial and scaled deployment

Begin with these processes on one application host, each in its own read-only container with a numeric uid of its own: a **control plane** (accounts, memberships, routing, billing, webhooks, the media gateway, the static client), an **egress dispatcher** (the only holder of provider credentials), a **telemetry ingestor**, a trusted **runner**, a body-checking **Docker proxy** (the only process that touches the Docker socket), a Caddy **edge**, and **one native campaign-runtime process per active campaign** (a container the runner starts through the proxy; never a thread or actor inside another campaign's process). Each runtime hosts exactly one fenced campaign actor and serves that campaign's table sessions; a runtime reaches only PostgreSQL (through a TCP forwarder), the dispatcher, the control plane's media gateway and the ingestor, over mutual TLS where it is not PostgreSQL, and a campaign's isolation from every other campaign is a process, network, uid and database-row-security boundary, not a convention. Managed PostgreSQL and durable object-backed media (Spaces) are outside the host. The ownership directory is durable, not a sticky browser assumption. Native bridge terminates at the runtime. Asset RPC remains authorized server relay through the media gateway. Upload complete immutable bytes/manifest to durable backing before referencing them; local artifact/cache files never establish availability. Protected local SQLite telemetry volume and durable spool are distinct from media and PostgreSQL. PostgreSQL migration and privileged maintenance credentials are separate from runtime: the migrator role exists only in the CI migrate step and never on a serving host.

Later further hosts (each with its own runner, dispatcher and ingestor) use the same PostgreSQL owner directory and durable media store. Gateway routes a session to its current owner using owner epoch; a wrong/stale route resolves directory, returns bounded reconnect/redirect guidance and never creates another actor writer. Acquire/renew fences with DB clock, strict expiry and transactional state revisions. Missing DB/quorum stops new commits and paid dispatch; a partitioned actor cannot publish new native/private views or accept authority;
only previously received permitted client cache can remain visibly stale. Reconnect crosses instances using existing operation lookup and snapshots, never Join again. Each producer instance owns its own SQLite writer/spool and immutable segment identity; review federation reads bounded indexed copies/segments through df-telemetry, no shared network SQLite writable DB. Losing an instance invokes the tested segment/spool restore procedure with visible missing ranges, not fictional lossless failover.

## Effect send boundary and uncertainty

`EffectRepository::claim_dispatch(effect_id, attempt_id, owner_fence, reservation_id)` atomically validates current unexpired campaign owner, effect status and hierarchical spend permit and records Dispatching plus unique provider idempotency key. The **native egress dispatcher** owns provider credentials and sockets and serializes the bounded send with that recorded permit. Actors/adapters cannot bypass it. Its lease renewal/revocation is checked immediately before egress; dispatch ownership is independent of actor takeover. Once marked Dispatching, recovery never considers the intent safely unsent merely because an actor lease expired. A race after last DB check can still send an old admitted request: fence cannot revoke a packet already in flight. Treat it as possibly sent and retain worst-case liability. At most one authorized dispatcher attempt may proceed for a reservation; takeover/retry queries provider by stable key or enters Unknown, never creates a new paid key.

For providers without reliable idempotency/status lookup, disallow automatic retransmission after Dispatching or ambiguous timeout. Persist UnknownLiability and use prepared fallback; manual reconciliation does not invent success/refund. Cancel before dispatch records UnsentCanceled only if dispatcher verifies no socket send began. Cancel after dispatch requests provider cancellation best effort, settles known/unknown charges and fences gameplay result by run/job generation. Old-owner successful response can settle accounting under attempt identity but only the current actor can commit an eligible result. Partition/crash fixtures cut execution before claim, after claim-before-send, during send, after response-before-ledger and during takeover; assert no duplicate deliberate sends, no lost liability, no old-run game commit. Exact external exactly-once delivery cannot be promised.

## Proposed workload and service targets

Reference qualification workload: **100 concurrent campaigns**, 8 interactive clients plus one display per scenario, four 3-hour sessions/month, burst 2 legal inputs/sec/campaign, 90 submitted audio minutes and 30 generated narration minutes/session, 12 new images/session. Also test 16/32 member campaigns and private parallel speech; these are load fixtures, not maximum allowed seats. Admit any supported group only when negotiated measured stream/capture/CPU/spend envelope fits; reject overload transparently rather than invent a D&D player maximum. This workload replaces the misleading assumption that a cheap VM is proven to serve 100 campaigns.

Candidate controlled-launch targets: p95 accepted input-to-durable decision **150ms** without provider dependency at regional RTT<=80ms; p95 permitted view publication after commit **100ms**; p95 ready audio scheduling **100ms** after browser receipt, with generation latency measured separately; reconnect permitted snapshot p95 **2sec** after network/auth is available. Availability target **99.5% monthly** for admission/read/game decision at qualified load, excluding no customer-visible incident arbitrarily. Alert error budget burn, actual session stalled progress and log ingestion gap. These targets are not benchmarks or an SLA sold today.

Bounded defaults: game-command payload64KiB; snapshot1MiB; asset/audio chunk64KiB; decoded WebSocket message256KiB; compressed-header block16KiB; 64 concurrent RPC streams/connection; 8MiB application receive/send budget/connection; per-actor inbox256 items/2MiB; every provider result <=approved schema/prompt8MiB; text generation output<=4,096tokens/call, context<=32,768tokens/call initially; asset video<=8sec and64MiB/job under explicit quote. Oversize rejection never parses/allocates the full malicious declared size. Control queue128 frames/256KiB reserved; rate cap100controlframes/sec/connection closes abuse. Snapshot oversize uses bounded manifest/page/resync plan, never silently truncates character/rules state. Limits apply before decompression and afterward; no arbitrary unlimited exceptions.

Telemetry initial envelope: producer enabled logs100events/sec sustained,1KiB average, burst1,000/sec for10sec; protected spool2GiB and SQLite active segment4GiB per instance, retention7days by rotating bounded immutable segments with a30GiB per-instance quota. These numbers imply quota pressure before seven days at continuous high rate; quotas win, pinning consumes reserved capacity and alerts before eviction. No claim all logs last seven days under every load. Export/ingest backlog age>30sec warns, >120sec degrades readiness for new campaign/provider admissions; gameplay already admitted reaches safe checkpoint. Disk hard-full emergency stderr records explicit loss/gap ranges and stops new high-volume admissions. Unsampled enabled logs remain the rule; reduce enabled verbose configuration explicitly if necessary, never covert sampling.

## Recovery and operator access

Proposed controlled-launch recovery objective: PostgreSQL acknowledged decisions RPO **<=5min** only where selected managed backup/PITR actually supports it, restore RTO **<=60min**; durable referenced media RPO0 for confirmed publication under qualified backing durability, restoration availability<=60min; telemetry/spool RPO<=5min recovery with explicit loss ranges. Until a restore drill demonstrates each target, deployment remains blocked, no contractual promise. Acknowledged decisions are durable to configured database fsync/replication, not immune to regional disaster. Initial single node fails availability on restart; HA target requires distinct validated multi-node mode, not claiming same uptime.

Runbook: freeze new admissions; establish incident epoch and preserve DB/media/segment backups; restore to isolated environment; replay deletion/revocation ledger first; reconcile payment/provider Unknown without resending; verify owner fences and canonical hashes; restore permitted assets; confirm account/recovery/entitlements; reopen read-only then new admissions; capture lost acknowledged-operation range and notify affected parties only when human authorization permits. Rolling deploy rejects incompatible checkpoint/source versions before ownership handoff, drains requests, transfers actor via fresh fence, handles outstanding dispatch attempts and keeps old protobuf compatible. No provider calls during deterministic game restore.

Operators authenticate separately with MFA, time-limited scoped role and reason/ticket. Default support sees redacted state, receipt IDs and diagnostic aggregates, not private dialogue/credentials. Break-glass access requires approved audit trail, expires15min and cannot alter game facts or wallets except typed reviewed commands. Refund, deletion, restore and rights-unlock are separate permissions. Alert owner is designated human on-call with fallback; unresolved operator coverage blocks sale of availability promises. Support workload tracks ticket arrival, resolution time and wage burden by paying campaign, excluding player dialogue content.

## Release, access, restore steps and DNS

Status: written as code and scripts under `deploy/` and `infra/`; none of it has run against a cloud, a registry or a real host. Everything below that says "verifies" is a script that exists and has not yet been exercised on the staging environment.

**Releases.** CI builds each image from the pinned Rust toolchain and distroless base, pushes it by digest and signs it keyless with cosign through the GitHub Actions OIDC identity of ONE workflow, `.github/workflows/release.yml` on `refs/heads/main` (`deploy/scripts/build-images.sh`, `deploy/scripts/build-release.sh`). The edge is our own image: Caddy built with xcaddy, with a rate-limit module pinned by commit, signed like the rest. A **release** is one signed `FROM scratch` image holding `release.lock` (the digests of the control-plane, campaign-runtime, egress, runner, docker-proxy and edge images, plus the pinned database-forwarder image), the production compose file, the edge files and the config templates. Two commands, so that staging a release changes nothing that is live:

- `sudo df-deploy release sha256:<digest>` **stages**. It verifies the release image and every image the lock lists with `cosign verify` against the exact certificate identity and OIDC issuer pinned in root-owned `/etc/dungeonflux/cosign-identity` (one workflow file on the main branch or a tag; a pattern, another host or a pull-request ref is refused before cosign runs), pulls by digest, fills the `@NAME@` tokens of the config templates from the non-secret release environment, and writes `/opt/dungeonflux/releases/<digest>` (its files, its own `allowed-images` and a `host.env` with the docker group id and the database endpoint) and the `staged` pointer. It never rewrites an existing release directory and never touches the live allow-list or `current`. If the packages are private it logs in first with the pushed `registry_pull_token`, into a tmpfs Docker config.
- `sudo df-deploy activate sha256:<digest>` **promotes** the staged release: the live allow-list (`/etc/dungeonflux/allowed-images.d/allowed`, the only image references the Docker proxy lets the runner start, replaced by rename in a directory that the containers mount, so a revoked digest stops being startable at once) and the `current` pointer change together, LAST: the slot tree and the egress firewall rules are rebuilt first (a failure there leaves the host as it was). The old pointer is recorded as `previous` unless the release is already current, and the previous campaign-runtime digest stays allow-listed for rollback and running placements. It does not start the stack: canary, health gate and rollback are the next round.

One `flock` is shared by `release`, `activate`, `refresh-keys` and `psql`, so two CI runs never race. A tag is never deployed.

**Network containment.** The three networks with a route out are bridges with fixed names (`dfctl0` control plane, `dfegr0` dispatcher, `dfdb0` database forwarder). `df-net-rules` installs DOCKER-USER drops so that the control plane and the dispatcher cannot reach the metadata endpoint, the VPC or any private range (the dispatcher fetches provider output, so this is its SSRF containment), and the database forwarder reaches only the cluster's address and port (and nothing at all while that address is unknown). It FAILS CLOSED: `docker.service` requires its unit, which runs before Docker, and re-applies the rules after every Docker start (a managed-cluster failover that changes the address is covered by the next start or by re-running `activate`). Internal mTLS has two intermediate CAs under an offline root, both `pathlen:0`: the service issuer (key offline) signs the fixed identities and server certificates, the placement issuer (key on the runner) signs only per-placement runtime certificates, and every receiver accepts a placement identity only through the placement issuer and a fixed identity only through the service issuer.

**Access.** People reach a host only through the bastion, one account each, with a FIDO2 `sk-ssh-ed25519` key that requires touch and PIN; CI has a separate key whose forced command can only run `release`, `activate`, `refresh-keys` and `secrets-install`. Nobody is in the `docker` group. Root actions are three sudo tools with fixed argument patterns: `df-secrets-install` (per-consumer secrets into tmpfs), `df-deploy` (`release`, `activate`, `refresh-keys`, `psql`) and `df-keys-apply`. Database administration (roles, grants, privilege verification, the restore step below) runs as `df-deploy psql`: a pinned one-shot psql container whose only route is a locked-down forwarder to the managed cluster's private name and port.

**Changing keys.** `sudo df-deploy refresh-keys sha256:<digest>` verifies a signed key-set image (the CI key and one FIDO2 key per person, from `infra/keys/` after review) like a release but against a DIFFERENT pinned identity, `.github/workflows/keys.yml`, whose job runs in a GitHub environment named `keys` with the owner as required reviewer (a key set is root-equivalent, so a release signature does not count), and hands it to `df-keys-apply`, which validates the whole set before it writes anything (every key must also parse with `ssh-keygen`), refuses a set that would lock everyone out, writes root-owned `authorized_keys` files with every option added by the tool, reloads sshd and ends the open sessions of any removed person. Removing a person is a reviewed change to `infra/keys/` plus this command; Terraform never changes keys on a running host. The bastion is not covered (no registry access): **replace it monthly** with `terraform apply -replace` of the bastion droplet, and in the same change as any application or worker host replacement, because its `permitopen` list holds the literal private addresses of those hosts. **Launch gates:** `refresh-keys` and release verification are built but unexercised; production is not applied, and takes no traffic or money, until each has been exercised on staging (a good set applied, a person removed, a tampered signature refused, the wrong identity refused).

**Database provisioning and the restore step.** `deploy/db/create-roles.sh` and `grants.sql` create every service role and set `transaction_timeout` (10 s, `feed_txn_timeout`) **as a role default** on every role that can bump a campaign's `event_seq`: the role default is the guarantee, because the timer starts at a transaction's first statement and `SET LOCAL` does not shorten one that is running. Requires PostgreSQL 17 or later; `grants.sql` refuses an older server, and `deploy/db/verify-privileges.sql` fails the run if any listed role has no positive role-level value at most 10 s. After a restore from backup, and after a failover that lost committed transactions, run `deploy/db/restore-feed-epochs.sql` (through `df-deploy psql`) **before service resumes**: it calls `campaigns.restore_feed_epochs()`, giving every campaign a fresh random 128-bit `feed_epoch`, and aborts unless every epoch changed and all are distinct. It refuses unless the service is explicitly halted or no application role is connected. Order for a restore: stop every application process, restore, raise the recovery epoch through the journal-head compare-and-set (the owner of that tool is the runtime design; the command is still to be named), run the epoch script, verify privileges, start services. Order for a lost-commit failover: the service is already halted by the regression check; run the epoch script in the same operator step that raises the recovery epoch and sets `serving` again. A counter would give two restores from one backup the same epoch; that is why it is random. `deploy/db/test-feed-epochs.sql` checks the shape (16 bytes, distinct, not a counter) and that two restores from one backup differ, in a transaction it rolls back.

**DNS and the GitHub Pages cutover.** Agents never change DNS or registrar settings. The owner enters the records listed in `deploy/dns/records.csv` by hand at GoDaddy, in the order of its `phase` column: lower the apex and `www` TTL to 300 s at least 48 hours ahead; add `staging` and rehearse the whole edge on it; add the mail subdomain's SPF, DKIM and DMARC; at the cutover replace the four GitHub Pages `A` records and delete the `AAAA` records (keep a copy of the old values for rollback) with the one edge address, and point `www` at the apex (the edge answers it with a permanent redirect). The edge obtains its certificate automatically once the name resolves to it, so expect a short window in which browsers see a certificate error until issuance succeeds. Before destroying a staging or production environment, delete its DNS records first and wait out the TTL (`down-1`): a record that outlives its reserved address can be taken over by whoever receives that address next.

## Tenant isolation and hostile input

Every repository/asset/history/commerce operation requires trusted TenantScope plus audience; lookup by opaque ID alone is insufficient. Runtime PostgreSQL role is neither owner nor superuser/BYPASSRLS; use FORCE ROW LEVEL SECURITY where applicable and explicit tenant predicates, transaction-local scoped settings reset on every pooled transaction. Administrative migrations/backups use separate roles. Unique/FK errors return nonexistence-safe public codes because constraints can leak across rows. Paired cross-tenant fixtures exercise joins, search, pagination, cache keys, temporary URLs, restore and foreign IDs. PostgreSQL documents owner/BYPASSRLS exceptions; RLS is defense in depth, not replacement for application auth. [PostgreSQL row security](https://www.postgresql.org/docs/18/ddl-rowsecurity.html).

Imported packages accept uploaded bounded bytes only, never arbitrary external import URLs. Approved provider/object-media retrieval goes through one native allowlist fetch policy: HTTPS only, allowed vendor/object hosts/ports, no user credentials, limit redirects<=2 and revalidate every hop and resolved IP, block loopback/private/link-local/metadata and DNS rebinding with connect-to-validated IP/TLS hostname verification; timeout10sec, max32MiB/import, decompressed max64MiB, no executable HTML/SVG/script or archive path traversal. Provider-supplied URLs are untrusted too. Local/private deployment endpoints require explicit operator config separate from public fetch. Prompt inputs/content/modeltext are data, never instructions granting tools, retrieval or access. Rate-limit anonymous invites/account challenges/capture/text/import/export and expensive admission by IP+principal+tenant with platform caps; abuse flags deny paid jobs without corrupting committed state. No raw credentials/player speech in default logs.

## Retention, deletion and provenance

Use a versioned retention class manifest: credentials revoked immediately; challenge tokens15min; anonymous bootstrap dedupe24h then retired-key tombstone; operation receipt90days then nonreplayable namespace tombstone; raw capture disabled persistence by default, approved diagnostic sample<=24h; private creative speech/episodic data campaign lifetime plus30days deletion window; canonical replay retains pseudonymous minimal facts/catalog revisions while personal identifiers/private payload are redacted or access-restricted; billing/tax records retained under selected jurisdiction policy, rights-cleared attribution kept separately. A replay needing deleted personal text returns Redacted/Unavailable, never reconstructs from model memory. Exact statutory periods are legal launch decisions, not invented legal advice.

`DeletionPlan` names subject/tenant/rights basis, affected credentials/rows/assets/index/cache/log segments/export manifests/backup generations, legal holds, completion stage and deletion tombstone. Execute prospective denial immediately; bounded deletion jobs remove permitted data and crypto keys where appropriate, invalidate derived indexes/summaries and audience-safe projections, record irreversible minimal audit. Backups expire within30days candidate window, restore MUST replay deletion/revocation tombstones before serving data; backup physical removal is not claimed instantaneous. Downloaded/shared exports cannot be retracted; explain limitation with explicit consent. Do not rewrite shared game truth to pretend another player's committed decision never happened. Fixtures cover restore of pre-deletion backup, rights hold, departed member, shared campaign, canceled account and stale private cache.

## Additional security enforcement details

Media decode/transcode runs in a resource-limited native worker subprocess, isolated
from actor/credentials:2threads,256MiB memory,10sec CPU/job,15sec walltime, no network, unprivileged UID, read-only restricted filesystem, image4096x4096 maximum16M decoded pixels, video8sec/240frames, bounded64MiB outputs;
unsupported codecs reject before decode. MIME sniff and decoded dimensions/duration
must agree with approved manifest. No active content rendering or shell interpretation.
Secret rotation maintains explicit issuer/key epoch and narrowly bounded verification
overlap; new sends use current keys, revoked tokens/bindings stop regardless of overlap.
Origin allowlist and secure HttpOnly same-site cookie/admission ticket policy prevent
cross-site tunnel use; CSRF/state protection applies customer mutations/redirects.
Default rate ceilings: anonymous bootstrap/invite/recovery10/min/IP plus3/min target,
text20/min/principal, capture one active lease/person with a distinct abuse cap of10submitted minutes
per15wall-clock minutes/principal, irrespective of campaign paid allowance, client telemetry10batches/sec at64KiB/batch with per-tenant
byte cap. Legitimate groups may require configured measured adjustment; denial is
safe explicit capacity and never frees another tenant's spend/history.

## Nonregressing irreversible recovery journal

Gameplay state stays PostgreSQL authoritative with the declared game RPO. Money,
possible external sends and privacy suppression cannot use an old Pg backup's
absence as proof. df-persistence implements existing repositories with a separately
protected **append-only recovery journal in durable object backing**, keyed by
installation/recovery epoch, tenant, monotonic journal sequence and stable intent/
event/deletion identity. This journal is evidence for restoring Pg/holding authority,
not a second runtime game writer. Entries hold minimal encrypted permitted metadata:
provider request/idempotency key/hash, max liability and Dispatching, verified
payment event/final result/credit identity, deletion/revocation/rights suppression
and retired allocation/operation namespaces, no copied private speech. Integrity
links/hashes and a protected published head watermark are independently retained.
Object versioning/immutable retention, encryption key custody and head nonregression
are required selected storage qualification; added requests/storage enter cost model.

Native choreography: reserve/claim in Pg -> append and confirm durable irreversible
journal entry/head -> mark journal-confirmed in Pg -> dispatcher may send or payment
policy may expose new credit/entitlement; deletion acknowledgement and webhook durable
acceptance likewise require protected journal confirmation. If any step is ambiguous,
close affected paid/private admissions and reconcile by the same stable ID; no
send/credit success/deletion-complete ack on guessed persistence. An extra journal
entry for work not sent is conservatively possibly sent, not an instruction to resend.
The egress dispatcher requires current journal-confirmed permit and current fence.
Receipt namespaces bind installation/recovery epoch; a restore allocates fresh epoch
and permanently retires old unknown command namespaces, while exact retained receipts
remain lookup-only. Old client retry cannot allocate a new account/member/job.

Restore authenticates the latest protected head against off-host watermark before
serving private or commercial data, replays suppressions first and overlays all
post-backup irreversible entries as hold/reconciliation records. It then reconciles
provider/payment receipts by stable key and rebuilds exact Pg grants/ledger only from
known source observations; unproven sends keep worst-case liability. Missing journal,
key, head or unverifiable latest completeness is fail-closed for affected scopes,
not an empty clean database. Already acknowledged financial/erasure transitions
require journal RPO0 under qualified backing; loss of that guarantee blocks paid
release and triggers incident reconstruction rather than advertising five-minute
financial loss tolerance. Game snapshot may lose declared five-minute range and
reports it; paid exposure/private suppression cannot silently regress with it.

Mandatory restore fixture cuts Pg backup before a recent socket send/payment credit/
deletion ack, then restores old game data with latest protected journal. Assert no
redisclosure/index rebuild of removed private data, no duplicate charge/credit,
no resend of Unknown, no implied wallet refund, retired old allocation retries and
explicit lost game-revision range. Remove newest journal/head/key separately: affected
admissions stay closed until verified recovery. This is a proposed protocol; durable
object consistency/head integrity/fault recovery and request-cost overhead are
unperformed G05/X02 gates, not existing RPO0 proof.

Campaign submitted-audio allowance is a separate versioned paid PriceVersion unit
grant, atomically admitted in commerce and selected from qualification before sale.
The reference90aggregate submitted minutes/campaign/session,360per four-session
scenario period, is workload data only and never an implicit per-person free quota
or promised subscription allowance. Capture abuse limits are independently adjustable
with measured noise/continuous-voice evidence and do not bypass aggregate spend.

## Composite revision after disaster restore

Allocate a strictly greater RecoveryEpoch from the authenticated nonregressing
protected head using conditional compare-and-swap and persisted identity, before
reopening any restored session. Missing/unverified head blocks allocation/serving.
SessionRevision=(RecoveryEpoch,in_epoch_sequence) orders lexicographically; ordinary
run resets/owner restart without rollback continue sequence, disaster restore starts
a new epoch. G03 freezes native/WASM/protobuf compare encoding and G05/X02 qualify
restore. Publish a full permitted snapshot declaring the lost game-revision range,
retire old expected revisions/operation/allocation namespaces, and return old receipt
lookup as known retained or expired/indeterminate, never reaccept old commands.
The protected journal records epoch issuance and irreversible liabilities/suppression,
not each game action; gameplay retains honest RPO5min instead of fictional RPO0.
Concurrent recovery head updates, stale writer and missing latest head fixtures
ensure epoch cannot regress or fork valid ownership.
