# Asset storage, character library and recovery: design note (O2)

Date: 2026-10-24
Status: Round 0g (revision after the sixth critic REVISE, 3 items). No code.
Implementation rounds follow approval.
Owner: O2 (assets, characters and storage). Plan: `planning/master-launch-plan.md` sections 6.2-6.3, 9.12, 12.1,
13 (all), 14.1-14.4, 15.1-15.4, 17 (FILE-01..08, JOB-01, SAVE-03, AUTH-03, OPS-02, OPS-03), J11, J14.
Coordinator decisions: LAUNCH.md 1-27, cited as D1..D27. The ones that shape this note: D2 (durable paid admission
in one transaction), D7 (one hosted database), D8 (migrations 0010-0014), D9 (hosted players authenticate), D10 (a
storage quota is not a spend cap), D13 (no uploaded real-person photo as a generation reference; automated screen;
operator review queue), D14 (offline owner keys for backups and operator exports; customer exports under a wrapped
per-export key; no noncurrent versions in Exports), D15 (`local-dev` feature), D18 and D23 (provider credentials only
in the egress dispatcher; the dispatcher fetches provider output; all media bytes go through the control-plane media
gateway over internal mTLS, never presigned object-store URLs), D19 (offline importer), D20 (metrics through O1's
exporter), D21 (a sent call with no usable output is company loss), D22 (archived media counts against quota; O2
owns the single journal writer; settlement is F's under D27; one attempt id), D24 (save points in their own fenced
transaction; O2's contract takes the full asset list and returns the digest; `campaign_event` by gapless
`event_seq`; O1's `fence_current`), D25 (cross-owner lock order; `Host(c)` follows the current host role; the
explicit "share my portrait with this campaign" step; the money owner resolves attempts stuck at `OutcomeUnknown`),
D27 (agent F owns the money core: admission hook, `ProviderAttempt` and dispatch status, `cancel_unclaimed_effect`,
settlement, terminal outcomes, lock order; B keeps identity, accounts, the portal, the processor adapter and
operators). Every money contract below names F.

Approved in earlier rounds and kept: the crate split, the lifecycle, row-derived reference counts, re-encode
sanitization, magic-byte sniffing, opaque keys, the redacted signed value type, and the closed findings of round 0c.

## 1. Governing documents and how this note follows them

| Document | What governs this work |
| --- | --- |
| `planning/master-launch-plan.md` 6.2-6.3, 13 | Attempt states `Requested -> Quoted -> Reserved -> DispatchPrepared -> Sent -> ResultKnown / OutcomeUnknown -> Settled / CustomerReleased`; settlement; storage, lifecycle, uploads, delivery, templates, quotas, backups, export/deletion |
| `planning/c-df-assets-d01..d04-decision.md` | D01 bytes durable before metadata; D02 authorization on every open and chunk; D03 a cache is never the authority; D04 suppression generations and tombstones fence reads and completions |
| `planning/storage-architecture.md` | PostgreSQL is authoritative; no transaction spans an external call |
| `planning/service-operations.md` | Tenant scope; FORCE RLS; DeletionPlan; the recovery journal in `df-persistence` with a protected head watermark |
| `planning/erasure-retention-policy.md` | Retention classes; tombstones replayed before service on restore |
| `planning/asset-engine.md` | `df-media` owns generation admission; request keys include the audience; the resource hub is the one generation choke point |
| `planning/coding-style.md` | Typed enums, newtypes, typed errors, owned async, bounded collections, audit lines, dev CLI view, tunables in files, failing-before tests |
| `planning/account-service-research.md` (B) | ids, `operation_receipt`, account export, account deletion, portal, operator console |
| `planning/finance-ledger-research.md` (F, D27; in progress) | Paid admission transaction, `ProviderAttempt` states and dispatch status, `cancel_unclaimed_effect`, settlement of `FinalizeOutcome`, terminal resolution of `OutcomeUnknown` |
| `planning/campaigns-and-table-research.md` (S1) 4.6, 5.1, 6 | `MembershipReader::load`, `campaign_event` keyed (`campaign_id`, `event_seq`) with `member_revision` and `revoking`, `highest_revoking_revision`, `verify_display_ticket` (ticket plus device-secret cookies), `display_ticket_revoked` |
| `planning/hosted-runtime-research.md` (O1) 9 | Save points, `asset_manifest_digest`, branch, restore, compaction, `fence_current` |
| `artifacts/tmp/agents/audit-2026-10-24.md` | F01, F03, F04, F05, F06 |

## 2. What exists today (read 2026-10-24)

- `crates/df-assets`: synchronous byte contracts (`publication.rs`, `range.rs` per-chunk authorization,
  `native_file_store.rs`, `resolution.rs`, `outcomes.rs`), used by df-audio, df-render, df-client, df-server's
  registry. No catalog, lifecycle, object store or PostgreSQL metadata adapter. `df-model::checkpoint` has
  `AssetReference`, `AssetRequestKey` (with `audience`) and `AudienceScope { Shared, Members(Vec<MemberId>), Host }`.
- `crates/df-tools/src/gameplay/imagery.rs` (A2 editing): `request_key` (SHA-256 over newline-joined parts:
  `df-imagery/1`, kind, model, quality, rating, size, reference hashes, prompt; no audience); `Registry` bounded at
  512 (A2: re-register from the cache file); run-local disk cache; `imagery.jsonl` with full prompts (A2:
  record-wise reader); `serve_image` serves any registered key to anyone.
- `creator_portraits.rs`: A4 round 1 (4975bf39) per-draft 256-bit capability; drafts in memory. A4 round-2 prep:
  bounded `Grants` with classes `Room | Player | Withheld` and hashed grant lines in the manifest; creator drafts
  keep their own mechanism (A4 decision 5). A3 plan: `Passed | Rejected | Unjudged`; `QuotaExhausted` breaker.
- `df-persistence/src/hero_library.rs`: phone-key library in a local database, portraits as `bytea`.
- Lock file dependencies: `image 0.25.10`, `hmac 0.13.0`, `sha2 0.10.9`, `ring 0.17.14`, `reqwest 0.13.5`
  (rustls), `tokio-postgres 0.7.15`. No S3 SDK, no ZIP crate.

## 3. Decisions

1. **Crates.** `df-assets` (mine): object-store port, key layout, SigV4 header signing, fake, catalog domain,
   native catalog service and `CatalogStore` port, native-only modules behind `cfg(not(target_arch = "wasm32"))`
   with already pinned native dependencies (`image`, `serde`/`serde_json`, `tokio`, `ring`). `df-library` (new,
   mine): templates, campaign characters, S1's `CharacterDirectory`. `df-persistence`: `media_catalog.rs`,
   `character_library.rs`, `recovery_journal.rs`. `df-providers`: `spaces_object_store.rs` (new file).
   `NativeFileStore` implements the new port for local development and self-hosting.
2. **PostgreSQL is the authority for existence, in every deployment**; in-memory fakes exist only in tests.
3. **One attempt id (D22).** `AttemptId` (16 random bytes) is minted once at admission and is the same value in
   F's `ProviderAttempt` (D27), the hub's job token (A6's F01 fix), the catalog row and every `PrivateMedia` key it
   writes (each key also carries a random write id, 5).
4. **One request key.** `MediaRequestKey` = SHA-256 of a canonical length-prefixed encoding (6.4), schema
   `df-imagery/2`, covering today's `imagery.rs` fields and the full output audience value. `df-model::AssetRequestKey`
   maps into it; the catalog stores only `MediaRequestKey`.
5. **Who serves bytes (D23).** The control-plane media gateway serves `/media/*` and `/gameplay/imagery/*` and
   receives provider output from the dispatcher over internal mTLS. No presigned object-store URL exists anywhere.
   Runtimes serve no media bytes, hold no object-store credential and keep no media cache. The gateway's cache is
   in memory only (`gateway_cache_bytes`, default 256 MiB, least recently used), keyed by `(asset, attempt, write,
   version)`, holds only whole objects up to `cache_entry_max_bytes` (default 8 MiB), never holds export bytes, and
   drops entries on tombstone and erasure events (8.3). Larger objects and every `Range` response are streamed from
   the object store and never cached. A re-audienced asset needs only authorization, not cache invalidation.
6. **Delivery is by grant bound to a principal** in hosted builds; capability-only delivery only behind
   `local-dev` (D15).
7. **Uploads go through the server** with the 13.4 ticket semantics.
8. **Sanitization is decode and re-encode** after applying EXIF orientation.
9. **Referenced/Unreferenced are derived** from `asset_reference` rows.
10. **Settlement is F's (D22, D27, plan 6.3).** Only the transaction that makes an attempt's terminal transition
    reports `FinalizeOutcome { attempt, outcome }` (`Ready`, `Superseded`, `Rejected(reason)`, `Invalid(reason)`),
    through a `media.settlement_outbox` row F consumes; O2 never decides a charge, release or credit (D21 through F).
11. **Every limit, retention period, quota and lifetime is configuration** (`content/storage.json`, validated at
    load, plan values as defaults); retention periods, the backup destination and storage-expansion pricing are
    owner decisions.
12. **Hosted dedupe scope is the campaign** (13.9); reviewed public library photographs are the only global scope.
13. **Cross-owner lock order (D25, D27).** Finance rows (F; B's account rows where B's transaction starts), then O1
    directory and session rows, then `media.quota_usage` (ordered by scope kind, `Account` before `Campaign`, then
    scope id), then `media.asset`
    (ordered by `asset_id`), then reference, grant and other media rows. A transaction that starts in media never
    afterwards locks finance, account or O1 rows. Every quota-changing transition (the list in section 10) locks the
    quota row before the asset row, including media-started ones such as a user delete. `delete_account_media` runs
    inside B's account-deletion transaction, after B's and F's locks; the save functions run inside O1's.

## 4. Requirement map

| Plan requirement | Crate / module | Main types | Tables (migration) | Tests (section 18) |
| --- | --- | --- | --- | --- |
| 13.2 domains, opaque keys | `df-assets::object_store`, `object_key` | `ObjectStore`, `StorageDomain`, `ObjectKey`, `ObjectVersion` | none | key layout incl. attempt; domain refusal |
| Faithful fake | `df-assets::fake_object_store` | `FakeObjectStore`, `Fault` | none | contract suite on fake, NativeFileStore, Spaces (ignored) |
| 13.3 metadata, lifecycle | `df-assets::catalog` | `AssetRecord`, `AssetId`, `AttemptId`, `AssetState`, `Audience`, `RetentionClass` | `media.asset`, `media.asset_variant`, `media.provider_lineage` (0010) | transition table, enum round trips |
| 13.3 copied, validated, retryable finalize | `catalog_service`, `sweeper` | `StagedObject`, `StepLease`, `FinalizeOutcome`, `LeaseLost` | `media.asset` step and staged columns | lease loss; sweeper vs finalize; JOB-01 media |
| 6.2, D2, D22, D27 paid admission | `catalog_service::admit_generation` inside F's transaction | `MediaAdmission`, `NotAdmitted` | `media.asset`, `media.quota_reservation` | concurrent admissions |
| D18, D23 output path | gateway internal staging endpoint | `StagingWrite` | `media.asset.staged_*` | overwrite refused; late result accepted |
| F03 no 512-entry authority | hub job + `CatalogStore::by_request_key` | `CatalogLookup` | partial unique index | FILE-03 |
| 13.4 bounds, still images, EXIF | `df-assets::validation` | `ImagePolicy`, `Rejection`, `SanitizedMaster` | none | FILE-01 |
| 13.4 scan; D13 screen | `df-assets::screen` via the dispatcher | `ScreenOutcome` | `media.upload.screen_attempt` | screen attempt rule |
| 13.4 ticket, quota, finalize | `df-assets::upload` | `UploadTicket`, `RightsAttestation` | `media.upload` | FILE-02, J11 |
| 13.5 delivery | `df-assets::delivery` (gateway) | `Grant`, `GrantPrincipal` | `media.asset_grant` | AUTH-03, FILE-07 |
| 13.5 per-chunk fence | `delivery::fence` | `RevocationFence` | none | revoking event fences the next chunk |
| 13.6 templates | `df-library` | `CharacterTemplate`, `TemplateRevision`, `CampaignCharacter` | `library.*` (0011) | two-tab conflict |
| 13.8 quotas | `df-assets::quota` | `QuotaPolicy`, `QuotaRefusal` | `media.quota_usage`, `media.quota_reservation` | FILE-05; interleaving races |
| 13.11 references, GC | `references`, `gc` | `Holder`, `Presentation`, `GcLease` | `media.asset_reference`, `media.tombstone` | FILE-06 |
| 13.11, J14, D14 export | `df-assets::export` | `ExportJob`, `ExportEntry`, `WrappedExportKey` | `media.export_job`, `media.export_access` (0012) | FILE-08, key destroyed |
| 13.10, D22 journal | `df-persistence::recovery_journal` | `JournalWriter`, `PendingEntry`, `JournalHead` | `journal.*` (0013) | crash windows, OPS-02, OPS-03 |
| D20 metrics | through O1's exporter | `StorageMetrics` | none | metric names present |
| 15.2 idempotency | all mutations | `OperationId` | `operation_receipt`, namespace `media` | same key other payload conflicts |

## 5. Object store port and adapters (13.2)

```rust
pub enum StorageDomain { PublicStatic, PrivateMedia, Quarantine, Exports, Backups }
pub struct ObjectKey { domain: StorageDomain, path: String }   // built only by the layout functions
pub struct ObjectVersion(String);
pub struct StoredObject { pub key: ObjectKey, pub version: ObjectVersion, pub byte_len: u64, pub sha256: [u8; 32] }

pub trait ObjectStore {   // native, async, generic (not dyn); every call carries a Deadline
    async fn put_new(&self, key: &ObjectKey, body: BoundedBody, expected: AssetManifest, deadline: Deadline)
        -> Result<StoredObject, ObjectStoreError>;                     // never overwrites
    async fn put_if_match(&self, key: &ObjectKey, current: &ObjectVersion, body: BoundedBody,
        expected: AssetManifest, deadline: Deadline) -> Result<StoredObject, ObjectStoreError>;
    fn conditional_writes(&self) -> ConditionalWrites;
    async fn head(&self, key: &ObjectKey, deadline: Deadline) -> Result<Option<StoredObject>, ObjectStoreError>;
    async fn get_version(&self, object: &StoredObject, range: Option<ByteRange>, deadline: Deadline)
        -> Result<VerifiedBody, ObjectStoreError>;                  // reads that exact version, digest-checked
    async fn list_versions(&self, key: &ObjectKey, deadline: Deadline) -> Result<Vec<ObjectVersion>, VersionListing>;
    async fn delete_version(&self, key: &ObjectKey, version: &ObjectVersion, deadline: Deadline)
        -> Result<(), ObjectStoreError>;
    async fn list_page(&self, prefix: &ObjectKey, after: Option<&str>, deadline: Deadline)
        -> Result<ListPage, ObjectStoreError>;
    async fn versioning_status(&self, domain: StorageDomain, deadline: Deadline)
        -> Result<VersioningStatus, ObjectStoreError>;              // NeverEnabled | Enabled | Suspended
}
```

- No presign methods (D23). SigV4 is header signing only (`df-assets::sigv4`), shared by the Spaces adapter and
  the fake, tested with the AWS example vectors.
- Layout functions are the only constructors, and every `PrivateMedia` key carries the attempt and a random write
  id: `character_asset(user, character, revision, asset, attempt, write, variant)`, `campaign_asset(campaign, asset,
  attempt, write, variant)`, `campaign_checkpoint(campaign, epoch, revision)` (O1), `export_archive(user, export)`,
  `staged(asset, attempt, write)` and `quarantine_source(upload, write)` in `Quarantine` (`write` is a random id per
  write), `backup(dataset, date, backup)`, `journal_segment(epoch, sequence)`, `journal_head(epoch)`. An environment
  prefix comes from configuration. Ids are hex of 16 random bytes. Two attempts, or two lease holders of one attempt,
  can therefore never write the same key.
- `get_version` reads the recorded version and checks length and SHA-256 against the recorded manifest; a mismatch
  is `IntegrityMismatch`, never served or published.
- `ConditionalWrites { create_if_absent, replace_if_match }`: the fake and NativeFileStore enforce both; the Spaces
  adapter sends `If-None-Match: *` on `put_new` and `If-Match` on `put_if_match` and relies on them only where live
  qualification shows Spaces enforces them. Without enforcement no correctness depends on them: every `PrivateMedia`,
  `Quarantine` and `Exports` key carries a random id, and the journal uses numbered heads under a writer lease (13).
- `Purged` means every version from `list_versions` was deleted; where listing is unsupported, every recorded
  version was deleted and a noncurrent-version expiry lifecycle rule (S2) removes the rest; the help page says so.
- `Exports` (D14): created never-versioned by S2's IaC; excluded from object replication (13) and from S2's object
  backups. Before issuing each export the control plane checks `versioning_status(Exports)` (cached for at most
  `exports_versioning_check_s`, default 60 s) and refuses with an alert unless it is `NeverEnabled` (on S3-style stores
  turning versioning off only suspends it). S2's qualification list includes Spaces `GetBucketVersioning` answers
  (absent status, `Enabled`, `Suspended`).
- `ObjectStoreError`: `NotFound`, `AlreadyExists`, `PreconditionFailed`, `IntegrityMismatch`, `TooLarge`,
  `DeadlineExceeded`, `Unavailable`, `Unknown` (sent, outcome unknown: head before any retry), `Denied`.
- Access by process: the control plane (catalog service, gateway, sweeper, GC, export worker) holds the media and
  export credentials; the journal writer holds the `Backups` credential; the dispatcher and the runtimes hold none.
  This supersedes "object-store scope (O2)" in O1's `RuntimeSpec` (relayed).

## 6. Catalog, lifecycle and the generation path (13.3)

### 6.1 States

| From | To | Trigger | Guard |
| --- | --- | --- | --- |
| (none) | Uploading | upload ticket issued, or generation admitted inside F's admission transaction | audience not `Withheld`; quota reserved; `AttemptId` persisted |
| Uploading | Quarantined | first complete staging write (upload or dispatcher output) | attempt current and not terminal; records staged key, version, length, SHA-256 |
| Uploading / Quarantined | Rejected | validation or screen rejection; abandoned upload; `cancel_unclaimed_effect` succeeded; supplier-side failure; F's terminal failure (D25); the dispatcher's positive `OutputGone`; `quarantine_max_age` reached (`review_expired`, `screen_unavailable`, `staged_expired`); recorded staged object `NotFound` or `IntegrityMismatch` (`staged_lost`) | in the terminal transaction: quota row locked first, reservation released, settlement outbox row (paid output: company loss, D21); never on `CustomerReleased` or a dispatcher outage alone |
| Quarantined | Validated | decode, bounds, sanitize; screen `Passed` (uploads) or judgement policy (generated) | step lease held; staged bytes read by recorded version and digest-checked |
| Validated | Ready | master and variants written under the attempt's write ids, verified by `head` | quota row locked first, then the asset row `FOR UPDATE`; every named winner `written` and none `deleting`; compare-and-set on `(asset, attempt, state, step_owner)` |
| Ready | Archived | owner or host archive; regenerate of the same key | still counts against quota (D22) |
| Archived | Ready | restore | no other live row for the key; no quota change (D22) |
| Ready / Archived | Deleted (tombstone) | delete, account deletion, retention, erasure | quota row locked first, `committed_bytes` released; no `live` references, or placeholder consent (11) |
| Deleted | Purging | GC claims after `gc_delay` | GC lease; no `live` references; save-point manifest scan clean |
| Purging | Purged | all versions deleted (5); journal entries confirmed (13) | GC lease current; no quota change (released at tombstone) |

Each row carries `step` (`stage`, `validate`, `screen`, `publish`, `purge`), `step_owner`, `step_lease_until_ms`.
`step_owner` is a fresh random 16-byte claim token minted on every claim (a second claim by the same process gets a
new token), so "owner is not the current `step_owner`" always identifies a lost or older claim. Every step is
claimed by compare-and-set on `(asset_id, attempt, state, step_owner IS NULL OR step_lease_until_ms < now)` and
renewed while working; a sweeper or GC worker takes a row only after its lease expires.

### 6.2 Admission, output, finalize and the sweeper

- **Admission (D2, D7, D22, D27).** Paid generation: F's admission transaction (F's finance rows locked first, D25)
  calls `media_catalog::admit_generation(&tx, ...)`, which refuses a `Withheld` audience (`NotAdmitted(AudienceEmpty)`,
  before any reservation or provider call), locks the quota row, reserves the kind's maximum bytes (10), and runs
  `INSERT INTO media.asset ... ON CONFLICT (dedupe_scope, request_key) WHERE state IN ('uploading', 'quarantined',
  'validated', 'ready') DO NOTHING RETURNING asset_id`. No row returned: `NotAdmitted(InFlight | Ready)`; F rolls back
  its whole transaction (reservation and `ProviderAttempt` included) and the caller re-reads `by_request_key` in a
  new transaction. A unique violation never aborts F's transaction. Non-paid admissions run the catalog part alone.
- **Output path (D18, D23).** The dispatcher fetches the provider output and streams it to the gateway's internal
  staging endpoint (mTLS) with `{asset, attempt}`. The dispatcher keeps the output durably (its own spool) or keeps
  the provider fetch handle until the staging endpoint acknowledges the write (O1's dispatcher contract). The
  control plane accepts a write only while that attempt is current and not terminal and the row is `Uploading`. It
  reads at most `maximum_bytes_per_kind + 1` bytes, hashing as it goes; at the limit it aborts, deletes the partial
  object and makes the terminal transition `Invalid(over_maximum_bytes)` (outbox row; customer not charged, D21).
  Otherwise it `put_new`s `staged(asset, attempt, write)` and moves the row to `Quarantined` by compare-and-set,
  recording key, version, length and SHA-256, then acknowledges. A second write for the same attempt after that is
  refused (`AlreadyStaged`) and its object deleted, so validated bytes can never be replaced. A late result (plan
  6.3 reconciliation, hours later) uses the same endpoint while the attempt is still current. A provider URL never
  leaves the dispatcher; its logs and spans redact provider output URLs and query strings.
- **Write order and write ids:** staged -> validated -> `PrivateMedia` -> `Ready` compare-and-set -> gateway memory
  cache on first read. Every `PrivateMedia` object key carries the attempt and a random `write` id. Before writing,
  the worker inserts its `media.asset_variant` row (`asset, attempt, variant, write, owner` with `owner` = its claim
  token, state `intended`) with an insert conditional on the asset row: `INSERT ... SELECT ... WHERE EXISTS (SELECT
  1 FROM media.asset WHERE asset_id = $1 AND attempt = $2 AND step_owner = $3 AND step_lease_until_ms > now)`.
  Zero rows means the attempt was superseded or the lease was lost: the worker stops before writing. After writing
  it marks the row `written`.
- **Finalize:** in one transaction, lock the quota row, then the asset row (`SELECT ... FOR UPDATE`); check that
  every write it is about to name as a winner is `written` (not `intended`, not `deleting`), else
  `WinnerMissing { variant }` and the worker rewrites that variant under a new write id; then `UPDATE media.asset
  SET state = 'ready', winning_writes = $4, ... WHERE asset_id = $1 AND attempt = $2 AND state = 'validated' AND
  step_owner = $3`. `Ready` names exactly one winning write per variant. On zero rows, re-read the row:
  - attempt unchanged, owner different: `LeaseLost`. The worker stops; it deletes nothing and reports nothing (the
    new lease holder continues from the recorded staged object, with its own write ids, so the keys never collide).
  - attempt changed, or row terminal: `Superseded` locally. It deletes nothing and reports nothing: the transaction
    that superseded or terminated the attempt already wrote the settlement outbox row.
  Only a terminal transition writes the outbox row.
- **Cleanup** is a recurring orphan sweep, not a one-time schedule. For the sweep a row is terminal when it is
  `Rejected` or has reached `Ready` (`ready_at_ms` set: `Ready`, `Archived`, `Deleted`, `Purging`, `Purged`). A
  variant write is deletable only when one of these holds: its attempt is not the asset's current attempt; the row
  never reached `Ready`, is not `Rejected`, and the write's `owner` is not the current `step_owner`; or the row is
  terminal and the write is not among `winning_writes`. The winning writes of a row that reached `Ready` are never
  deleted by the sweep, whatever its later state or lease; only the GC purge (11) removes them. A write of the
  current owner on a row that never reached `Ready` is never deleted, however old. For each candidate the sweep, in one
  transaction, locks the asset row `FOR SHARE`, re-evaluates the rule, and moves the variant to `deleting` by
  compare-and-set; after commit it deletes the object and then the variant row. Because finalize holds the same
  asset row `FOR UPDATE` while it checks its winners, either the sweep marks first (finalize sees `deleting` and
  answers `WinnerMissing`) or finalize commits first (the write is a winner and no longer deletable). Staged objects
  are deletable once their attempt is not current, the row is `Rejected`, or the row has reached `Ready`.
  `cleanup_grace` only paces the sweep. A write that raced past a supersede is removed on the next sweep.
- **Sweeper** (owned task, bounded batches, expired leases only), reading F's `ProviderAttempt` state:
  - `Reserved` (not yet claimed): rejection runs in one transaction that first calls F's conditional
    `cancel_unclaimed_effect(attempt)` (F rows, D25 order), then locks the quota and asset rows and moves the row to
    `Rejected(not_dispatched)`. If F answers `AlreadyClaimed`, the transaction rolls back and the row stays live.
  - `DispatchPrepared`, `Sent`, `OutcomeUnknown`: possibly sent. The row stays live and `by_request_key` answers
    `InFlight`; no new dispatch. A late result arrives through the staging endpoint and finalizes.
  - `CustomerReleased` (plan 6.3: the customer hold is released after about 15 minutes) is a money event, not a
    supplier outcome. While the supplier outcome is unknown the row stays `InFlight`; a late result is staged and
    finalized (company-funded, F's settlement); the key is never re-bought meanwhile.
  - Only a supplier-side terminal moves a generation row to `Rejected`: `ResultKnown` with a failed result, or F's
    terminal failure after its reconciliation limit (D25, proposed 24 h; F credits the customer per D21). F's outcome
    event moves the row to `Rejected(unknown_outcome_failed)` in the consumer transaction (F rows first, then quota,
    then asset). A result arriving after that is refused at the staging endpoint and discarded.
  - `ResultKnown` with success but nothing staged after `staging_deadline`: the sweeper sends the dispatcher a
    re-fetch request carrying the `AttemptId`; re-fetch is idempotent per attempt on the dispatcher side (concurrent
    requests share one fetch and one staging write) and never calls the provider again. Only a positive answer
    `OutputGone { attempt }` (no spool and an expired fetch handle) makes the terminal transition
    `Rejected(output_lost)` with a settlement outbox row (company loss under D21; F settles). Dispatcher
    unavailable, a timeout or any unknown answer leaves the row `InFlight`; the sweeper retries with backoff
    (`refetch_backoff`, capped), so a dispatcher outage never frees a key while spooled output may still exist.
  - `quarantine_max_age` (configuration, default 20 h; validated at load to be strictly below
    `quarantine_lifecycle_expiry`, the value S2's lifecycle rule for `Quarantine` is generated from, default 24 h; a
    value that is not falls back to the default and is named in an audit line). The age counts from the recorded
    staged object's `staged_at_ms` (the time of the first complete staging write, which is also the creation time
    S2's lifecycle rule counts from), never from admission, so a late result staged hours after admission gets the
    full window. A row still `Quarantined` at `staged_at_ms + quarantine_max_age` goes terminal with a settlement
    outbox row for paid output
    (D21): `Rejected(review_expired)` for a flagged upload never reviewed, `Rejected(screen_unavailable)` for a
    screen still `Unjudged`, `Rejected(staged_expired)` for anything else not yet validated. The operator console
    shows each queued item's review deadline, `review_deadline_ms = staged_at_ms + quarantine_max_age`.
  - A recorded staged object answering `NotFound` or `IntegrityMismatch` at any step is the terminal
    `Rejected(staged_lost)` (outbox row for paid output), never a retry loop.
  - Uploads: completes only rows with `finalize_requested_ms` set; others expire at ticket expiry as
    `Rejected(abandoned)`.
  - Staged bytes recorded: resume the next step from them; no provider call is repeated.
- **Screen attempts (D13)** follow the same rule: each content-screen call is its own `AttemptId` with F's exposure
  record, stored as `media.upload.screen_attempt`. A sweeper that takes over a `screen` step reads that attempt's
  state first and admits a new screen attempt only when the previous one is supplier-terminal (or cancelled through
  `cancel_unclaimed_effect`); while it is `DispatchPrepared`, `Sent`, `OutcomeUnknown` or `CustomerReleased` it waits.
- **F01 replacement.** One statement moves the row from attempt A1 to A2 (`... SET attempt = A2 WHERE asset_id = $1
  AND attempt = A1 AND state IN (uploading, quarantined, validated)`); that transaction writes A1's `Superseded`
  outbox row. A1's late output is refused at staging or fails its conditional insert or compare-and-set, and the
  orphan sweep removes anything it wrote. If A1 was already `Ready`, the regenerate archives that asset and A2 gets
  a new row.

### 6.3 `by_request_key` outcomes and precedence

`Ready(asset)` | `InFlight(attempt)` | `Archived(asset)` | `Suppressed` | `Deferred(QuotaFull)` | `Absent`. The live
row (at most one) wins; otherwise the newest `Archived` row (neutral chrome, restore offered, never re-bought);
otherwise a tombstoned row answers `Suppressed` until an explicit regenerate; otherwise `Absent`. A catalog outage is
a transient failure with no dispatch.

### 6.4 Audience and the request key (asset-engine.md, 14.4, D25)

`Audience` (closed): `PublicStatic`, `CampaignShared(c)`, `Members(c, MemberSet)` (sorted, non-empty), `Host(c)`,
`OwnerPrivate(account)`, `Withheld`. An output's audience is the meet of the requested audience and every reference
input's audience, from this static table (no lookup; `Withheld` absorbs; two campaigns meet to `Withheld`):

| meet | PublicStatic | CampaignShared(c) | Members(c, T) | Host(c) | OwnerPrivate(b) |
| --- | --- | --- | --- | --- | --- |
| PublicStatic | PublicStatic | CampaignShared(c) | Members(c, T) | Host(c) | OwnerPrivate(b) |
| CampaignShared(c) | CampaignShared(c) | CampaignShared(c) | Members(c, T) | Host(c) | Withheld |
| Members(c, S) | Members(c, S) | Members(c, S) | Members(c, S and T), Withheld if empty | Withheld | Withheld |
| Host(c) | Host(c) | Host(c) | Withheld | Host(c) | Withheld |
| OwnerPrivate(a) | OwnerPrivate(a) | Withheld | Withheld | Withheld | OwnerPrivate(a) if a = b, else Withheld |

- A `Withheld` result is refused before admission (`AudienceEmpty`): nothing nobody may see is ever bought.
- `Host(c)` follows the current host role, like a game master's notes (D25): after a host transfer the new host
  sees host media and the old one does not. `OwnerPrivate` media never follows any role.
- D25 share step: a player's portrait stays `OwnerPrivate` until the character-approval step "share my portrait with
  this campaign" (`share_to_campaign`, 11). Until then a scene that would use it meets to `Withheld`, is not
  admitted, and shows neutral chrome.
- D13: an uploaded photograph is refused as a generation reference (`ReferenceRefusal::UploadedPhoto`).
- `MediaRequestKey` encoding: SHA-256 over `"df-imagery/2"` followed by fields, each a big-endian `u32` length and
  its bytes, in a fixed order (kind, model, quality, rating, width, height, transparency, reference count and each
  reference hash, prompt, scene evidence when present, audience). The audience is a tag byte, then the campaign id
  (16 bytes) where it has one, then for `Members` a `u32` count and the sorted member ids (16 bytes each), for
  `OwnerPrivate` the account id. No two different field lists encode the same bytes.

### 6.5 The hub's generation path

- A runtime view reads only an in-process set of catalog-confirmed `Ready` asset ids (no I/O, bounded); a miss
  shows neutral chrome and queues through `Hub::get_or_request`; the miss itself never generates.
- The set is fenced by `media.tombstone` and `media.asset_audience_event` (cursor), so a deleted or re-audienced
  asset falls back to neutral chrome.
- The worker's job first awaits `by_request_key`; only `Absent` leads to admission.

### 6.6 The record (13.3)

Owner scope, campaign and template ids, `Audience`, `AssetKind`, MIME, length, dimensions, duration, SHA-256, the
staged object (key, version, length, digest), variants keyed by attempt and write id (winning writes named),
provenance (`Generated`, `Uploaded`, `Reviewed`, `ClaimedLibrary`), provider lineage (`prompt_sha256` only), attempt,
`charge_scope`, `RightsAttestation`, `judgement` (A3), `screen`, `RetentionClass`, state, step lease,
`suppression_generation`.

## 7. Upload pipeline (13.4, J11)

1. `request_upload(principal, purpose, declared_mime, declared_bytes, declared_sha256, attestation, idempotency)`:
   authenticated (D9); bounds first (10 MiB image, 1 MiB JSON); rate limit (default 20 per hour); in-flight count
   and quota on the account's quota row; `RightsAttestation` recorded; `Uploading` row, `AttemptId`, `UploadTicket`
   (account, purpose, size, expiry; default 15 minutes).
2. Bytes through the server; at most `declared_bytes + 1` read, hashed while streaming, `put_new` to
   `quarantine_source(upload, write)`, staged object recorded at `Quarantined` as in 6.2. A ticket from another
   account or for another upload is refused (FILE-02).
3. `finalize(ticket, idempotency)` records `finalize_requested_ms`, then: length, SHA-256, unexpired reservation;
   decode on a bounded blocking pool (the permit is moved into the `spawn_blocking` closure and released when the
   thread returns); magic bytes; 20 megapixels checked from the header; animated WebP and APNG refused;
   `image::Limits`; EXIF orientation; re-encode master and display variant; content screen per 6.2's rule (`Passed`
   -> `Validated`; `Flagged` -> review queue with a deadline (6.2); `Rejected`; `Unjudged` -> a new screen
   attempt later, until `quarantine_max_age`); publish;
   `Ready` with the quota commit. A rejection records the reason and leaves the character untouched (J11).
4. Character JSON: `CharacterImportV1` (`df-character/1`, `deny_unknown_fields`, bounded), a proposal validated by
   the rules engine; no URL or path field.

## 8. Private delivery (13.5, F05, 14.1, D23)

The control-plane gateway serves `/media/<capability>` and `/gameplay/imagery/<capability>` with one handler, always
streaming the bytes itself (`Range` included). There is no redirect mode.

### 8.1 Grant issue, binding and renewal

- `issue_grant` and `renew_grant` each call `MembershipReader::load(account, campaign)` fresh, check the audience
  against that snapshot (`Members` needs the current `MemberId` in the set; `Host` needs the current host role), and
  store `sha256(capability)`, asset, audience, `GrantPrincipal` (`Member { account, member_id }`, `Display {
  ticket_id }`, `Owner { account }`), `member_revision`, room epoch, issued, expiry, revoked.
- Origin access (plan 13.6): the account an asset was uploaded by or originated from (`origin_account`: the
  uploader, or the template owner whose portrait it is) may always view it through an `Owner` grant, and include it
  in its account export, while that account holds a `TemplateRevision` reference to it, whatever the asset's
  audience or charge scope and even after leaving the campaign it was shared with. The check is the reference row,
  read in the grant's PostgreSQL read; tombstones and suppression still apply.
- Every request presents the principal's own credential: the account session cookie, or for displays S1's
  `verify_display_ticket` with both the ticket and device-secret cookies. Mismatch: concealed 404. A display grant
  covers only `CampaignShared` and expires no later than its ticket.
- Lifetime `grant_lifetime` (default 15 minutes); view rebuilds renew on-screen grants under half their lifetime,
  each renewal doing the fresh load.
- Hosted creator drafts keep A4's capability and add a session-owner check against the draft's `OwnerPrivate` row;
  capability-only only behind `local-dev` (D15).

### 8.2 Headers and FILE-07

- `Referrer-Policy: no-referrer`; `Cache-Control: private, max-age=<min(remaining grant, browser_cache_seconds)>`
  (default 60 s; never `public` or `immutable` for restricted media); `X-Content-Type-Options: nosniff`;
  `Content-Type` from the catalog row.
- FILE-07: no bearer object-store URL exists; after revocation no new grant is issued and the next request or chunk
  is refused; bytes already in a browser's private cache cannot be recalled and expire with `max-age` (documented on
  the help page).
- Redaction: audit lines carry asset id, grant hash prefix, outcome, bytes; never a capability. S2's edge redacts
  `/media/*` and `/gameplay/imagery/*` path segments and query strings.

### 8.3 Per-chunk fence and the memory cache

- One PostgreSQL read per request; no query per 64 KiB chunk.
- `RevocationFence`: `highest_revoking_revision[member]` and revoked ticket ids from an owned consumer of S1's
  `campaign_event` (per campaign, `event_seq > acked_seq` in order; rows with `revoking = true`), plus asset
  suppression generations from `media.tombstone` and `media.asset_audience_event`. A chunk is refused when
  `highest_revoking_revision[grant.member_id] > grant.member_revision`, the ticket is revoked or expired, or the
  asset's generation moved.
- Lag is the age of the last successful poll; above `revocation_lag_limit_ms` (default 5 s) new private requests are
  refused and open streams fenced. Entries exist only for principals with a live grant on this gateway, at most
  `MAXIMUM_FENCED_PRINCIPALS`; at the bound new private streams are refused, never admitted unfenced.
- The same consumer drops memory-cache entries on tombstone and erasure events (entries are keyed by `(asset,
  attempt, write, version)`, so an audience change needs only the per-request authorization).

## 9. Character templates and campaign characters (13.6, 9.12, 15.1)

- `library.character_template` and immutable `library.template_revision` (identity record, digest, rules and
  content versions, `portrait_asset`, provenance). `save` is a compare-and-set on `current_revision`
  (`RevisionConflict { current }`); the template count (default 10) is on the account's quota row.
- `library.campaign_character` binds a campaign character to its account and origin revision; instantiation is
  idempotent (S1 O2-2); progression lives only in game state; a template edit never reaches the instance.
- `CharacterTemplateId`, `CampaignCharacterId` into `df-types` through its lock holder (S1 O2-1).
- Hero-library claims read hosted-database rows only (screen skipped, sanitizer kept); local or self-hosted
  libraries and the D19 importer go through section 7 with the screen.

## 10. Quotas (13.8, FILE-05, D10, D22)

- One usage row per scope: `committed_bytes` (Ready and Archived), `reserved_bytes`, `in_flight_uploads`,
  `template_count`; `INSERT ... ON CONFLICT DO NOTHING` then `SELECT ... FOR UPDATE`, in the D25 order.
- Uploads reserve the worst case (declared plus variant and growth bounds), rechecked at commit; an excess that does
  not fit rejects the upload. Finalize needs an unexpired reservation.
- Generation reserves the kind's configured maximum bytes, a hard limit enforced while streaming (6.2): output
  over it is `Invalid(over_maximum_bytes)` (reported to F; customer not charged); output within it always commits.
  The reservation does not expire while the attempt is `DispatchPrepared`, `Sent`, `OutcomeUnknown` or
  `CustomerReleased` with the supplier outcome unknown; it is released only by the terminal transition.
- Every quota-changing transition locks the quota row first (3.13), then the asset row:
  - reserve: generation admission (inside F's transaction), upload request;
  - commit (reserved to committed): finalize to `Ready`;
  - release reserved: every terminal `Rejected`, `Invalid` or `Superseded` transition, upload ticket expiry, the
    sweeper's `cancel_unclaimed_effect` rejection, F's outcome-event consumer;
  - release committed: tombstone (user delete, account deletion, retention, erasure), so a deletion frees space at
    once (13.8);
  - add committed: an operator's explicit reversal of a tombstone after a restore (13);
  - scope transfer: `share_to_campaign` (coordinator ruling) releases the asset's committed bytes from the account
    scope and adds them to the campaign scope in one transaction, locking both quota rows in scope order (account,
    then campaign; both within D25's `media.quota_usage` step), then the asset row; a campaign without room refuses
    the share with `QuotaRefusal` and the portrait stays private;
  - copy to a second campaign: sharing an already shared portrait with another campaign creates a new asset; its
    bytes are added to that campaign's committed bytes (that campaign's quota row, then the new asset row; refused
    with `QuotaRefusal` when it lacks room); the original asset's charge is unchanged;
  - template count (account row): `+1` on template create, duplicate, import and hero-library claim (each refused at
    the 10-template limit); `-1` on template delete (tombstone); `+1` on restoring a deleted template within its
    deletion window (refused at the limit); archived templates keep counting (as archived media does, D22);
  - recompute: after a restore, bytes and template counts from the rows (13);
  - no change: archive and archive restore (archived media counts, D22), purge (released at tombstone).
- Refusal is `QuotaRefusal { scope, needed_bytes, available_bytes }`; the hub sees `Deferred(QuotaFull)`, neither
  an attempt nor a remembered failure (request to A6).

## 11. References, deletion, retention and GC (13.11, 9.12, D04)

- `media.asset_reference(asset, holder_kind, holder_id, presentation)` with `presentation` `live | placeholder`;
  holders `SavePoint`, `TemplateRevision`, `CampaignCharacter`, `CampaignShare { campaign }`, `ExportJob`.
- Reference inserts lock asset rows `FOR SHARE` in `asset_id` order (after the caller's finance, account or O1 rows,
  D25);
  deletion and erasure lock `FOR UPDATE` in the same order. `live` needs `Ready`/`Archived`; tombstoned assets are
  accepted only as `placeholder`; unknown, foreign-scope or `Rejected` assets are refused.
- Delete: `live`-referenced is refused (`ReferencedBySave { holders }`); with placeholder consent one transaction
  tombstones the asset and turns references to `placeholder`; purgeable after `gc_delay` (FILE-06).
- `share_to_campaign` (the D25 approval step) adds a `CampaignShare` holder (`CampaignMedia` retention), an audience
  event to `CampaignShared(c)`, and moves the asset's `charge_scope` from the account to the campaign (the quota scope
  transfer of 10). From then on the campaign carries the charge: deleting the account leaves the campaign's charge and
  the asset in place (the uploader stays recorded as `origin_account` for erasure rights and origin access, 8.1).
  Sharing the same portrait with a second campaign creates a new asset for that campaign (the control plane copies
  the master into that campaign's keys; the copy keeps the same `origin_account`), because an asset has exactly one
  charge scope. Erasure of a shared upload suppresses it across holders and
  releases its bytes from the campaign scope (the tombstone transition of 10).
- GC lease per row; purge deletes every version (5); `Purged` after journal confirmation (quota was released at
  the tombstone). Retention classes are configuration (`Quarantine` 24 h lifecycle with the 20 h
  `quarantine_max_age` below it, `Intermediate` 48 h, `Personal`, `CampaignMedia`, `Export` 7 days, `Backup` 30
  days).
- "Deleted" is shown only after journal confirmation; access is denied from the tombstone commit.

## 12. Exports (13.11, J14, FILE-08, D14)

- `request_export` requires B's recent-authentication check; an owned worker builds it from `ExportSource`
  sections: B's account JSON (profile), F's personal usage statements, templates and revisions, owned upload
  masters (including shared ones under origin access, 8.1), O1's permitted campaign excerpts. Each `ExportEntry`
  carries its `Audience` (or the origin-access basis); the builder drops entries
  the requester may not see.
- FILE-08: (a) mine, failing first, with a fake source returning another player's secret note and a hidden GM fact;
  (b) S4's test against O1's real projection.
- Names from ids only (`templates/<template-id>/<revision>.json`, `uploads/<asset-id>.<ext>`,
  `attachment; filename="dungeonflux-export-<export-id>.zip"`).
- Stored ZIP, size bound, `put_new` to `Exports` (never versioned and checked before each export (5), not replicated,
  not in object backups, never in the gateway cache), download through an `Owner` grant, each access in
  `media.export_access`.
- Keys (D14): a random per-export AES-256-GCM data key encrypts the archive and is stored only wrapped under S2's
  key-encryption key (random nonce). Expiry deletes the archive and the wrapped key row and writes an
  `ExportExpired` journal entry. Residual (stated on the export page): database backups and PITR keep the wrapped
  data key for their retention, with no archive copy left anywhere to decrypt.

## 13. Backups and the recovery journal (13.10, OPS-02, OPS-03, D22)

- **One writer:** `df-persistence::recovery_journal::JournalWriter` in the control plane (D22). Submitters (catalog
  tombstones, purges, export issue and expiry, F's financial entries, the dispatcher's dispatch entries, O1's epoch
  issuance) insert `journal.pending_entry` (`entry_id`, kind, stable id, minimal metadata) in their own transaction.
- **Drain, step 1:** in one PostgreSQL commit, the writer stamps the next batch of unstamped pending rows with the
  next `sequence`. The batch for a sequence is fixed from then on.
- **Step 2:** seal each stamped entry with AES-256-GCM under the journal key (S2 secrets) and a random 96-bit nonce
  per seal; chain SHA-256 over ciphertexts; `put_new` `journal_segment(epoch, sequence)`. `AlreadyExists` means a
  previous run wrote it: read, decrypt and adopt it if it holds exactly the stamped rows of that sequence; a
  difference stalls the journal and closes private delivery and paid admission (alert).
- **Step 3:** advance the head (`put_if_match`; or numbered heads with `put_new`, a PostgreSQL writer lease, and the
  off-host watermark), then mark the stamped rows confirmed.
- **Start:** read the head and mark every stamped batch at or below it confirmed (a crash after the head advanced);
  never rewrite those batches under a later sequence; continue from the journal head, not from the database's
  `confirmed_head`.
- **O2 entry kinds:** `AssetTombstoned`, `AssetPurged`, `SuppressionAdvanced`, `ExportIssued`, `ExportExpired`.
- **Restore with PostgreSQL older than the journal (OPS-02):** verify the head against the watermark; replay
  post-backup tombstones and suppressions into PostgreSQL (a missing asset row gets a tombstone stub); skip any
  restored pending row whose `entry_id` already appears in the journal after the database's high-water mark;
  recompute `media.quota_usage`; run the operator-held orphan reconciliation of `PrivateMedia` and `Quarantine`
  against the catalog (report only); then reopen delivery. Any mismatch keeps private delivery and paid admission
  closed.
- **Objects:** `PrivateMedia` keys are immutable; versioning plus replication of new keys to a backups destination
  outside production's destructive authority (owner decision); `Exports` and the gateway's memory cache are never
  part of it. OPS-03 media half: a purged asset restored from backup stays refused by its journal tombstone.

## 14. Contracts with O1 (D24)

- **Save point:** inside O1's own fenced save transaction (O1 rows and `fence_current` first, D25),
  `media_catalog::record_save_references(&tx, save_point, &[AssetRef])` takes the full asset list (`AssetRef {
  asset, audience }`), locks asset rows `FOR SHARE` in `asset_id` order, inserts `live` holders for `Ready`/`Archived`
  and `placeholder` holders for tombstoned assets, refuses only unknown, foreign-scope or `Rejected` assets (which
  fails the save, never drops a reference), and returns the digest O1 stores as `asset_manifest_digest`.
- **Branch:** `copy_save_references(&tx, from, to)` copies holders with their `presentation`. **Compaction:**
  `drop_save_references(&tx, save_point)`.
- **Checkpoint bundles:** `CheckpointBundle` assets written by the control plane; their `Ready` event reports
  `backup_copied_revision`.
- **Room end:** `end_room(campaign, epoch)`. **Runtimes:** no object-store credential, no media cache, no media
  serving (D23). **Importer (D19):** section 15. **Metrics (D20):** section 17.
- Open: Q-O1-2 (`AssetReference.key` carrying the `AssetId`), Q-O1-4 (`permitted_excerpts`).

## 15. Migration of today's imagery path (A2, A3, A4 kept)

| Today (after the audit fixes) | Becomes | What carries over |
| --- | --- | --- |
| A2: `registered()` re-registers from the cache file | the hub job's `by_request_key`; `Registry` becomes the fenced in-process set | A2's F03 test as FILE-03 |
| A2: record-wise manifest reader | O2's part of the D19 offline importer: A2's reader, idempotent upsert re-keyed under `df-imagery/2`, `prompt_sha256` only. Audience: `CampaignShared(c)` only where an A4 `Room` grant line proves the image was shown to the table; images with no grant line import as `Host(c)` (the host reviews and may share them); A4 `Player` or `Withheld` lines import as `Withheld` | A2's test as FILE-04 |
| `Engine::store` | runtime asks; dispatcher fetches and streams to the gateway's staging endpoint; control plane finalizes | size bound; a failed write never fails the scene |
| A3: `Passed / Rejected / Unjudged` | `judgement`; `Rejected` -> `Rejected`; `Unjudged` never accepted evidence | A3's tests |
| A4 round 2: `Grants` API | same API over `media.asset_grant`, principal-bound, served by the gateway | A4's planned tests |
| A4 round 1: draft capability | kept plus the hosted owner check (8.1) | AUTH-03 draft cases |
| Hub (A6) | unchanged choke point; its token is the shared `AttemptId` | A6's stale-completion test |

Order on `imagery.rs`: A2 -> A3 -> A4 round 2 -> O2 round 5.

## 16. Schema and migrations (D7, D8)

B's conventions (16-byte ids, millisecond times, closed enums with CHECK, append-only triggers, explicit tenant
predicates, FORCE ROW LEVEL SECURITY, `schema_migration`).

- `0010_media_catalog.sql`: `media.asset` (attempt, step lease, staged key/version/length/digest,
  `finalize_requested_ms`, `winning_writes`, `charge_scope`, `origin_account`, `staged_at_ms`, `ready_at_ms`),
  `media.asset_variant` (pk `asset, attempt, write, variant`; `owner`; `intended`/`written`/`deleting`),
  `media.provider_lineage`, `media.upload` (screen, `screen_attempt`, attestation, `review_deadline_ms`),
  `media.quota_usage`,
  `media.quota_reservation`, `media.asset_reference`, `media.asset_grant` (`member_id`, `member_revision`,
  `ticket_id`), `media.asset_audience_event`, `media.tombstone`, `media.settlement_outbox`; the partial unique index.
- `0011_character_library.sql`: `library.*`.
- `0012_media_exports.sql`: `media.export_job` (wrapped data key), `media.export_access`.
- `0013_recovery_journal.sql`: `journal.pending_entry` (with `sequence` null until stamped, `confirmed_ms`),
  `journal.writer_lease`, `journal.confirmed_head`.
- 0014 reserved.

## 17. Audit lines, metrics, dev CLI and configuration

- `ASSET <upload_issued|staged|staging_refused|finalize_requested|validated|screened|flagged|rejected|ready|
  lease_lost|winner_missing|superseded|invalid|shared|scope_transferred|archived|restored|tombstoned|purging|purged|
  variant_deleting|refetch_requested|orphan_reported> asset= scope=
  kind= attempt= bytes= reason= ms=`
- `MEDIA_GRANT <issued|renewed|revoked|served|refused|expired|fenced> asset= grant=<8 hex of hash> audience= status=`
- `STORAGE <quota_reserved|quota_committed|quota_released|quota_refused|export_ready|export_downloaded|
  export_expired|journal_stamped|journal_confirmed|journal_adopted|journal_stalled|revocation_feed_lagging> scope=
  bytes= reason=`
- Metrics (D20, O1's exporter): stored bytes by scope and state, objects by state, upload rejections by reason,
  delivered bytes per campaign, grants live, gateway memory cache bytes, revocation feed age, journal pending count
  and age, sweeper and GC backlog, `InFlight` rows by attempt state.
- `dev assets`: read only (states, transitions, quotas, grants, review queue, sweeper, GC, feed lag, journal).
- `content/storage.json`: upload bounds, rate and in-flight limits, decode permits and deadline, ticket and grant
  lifetimes, renewal threshold, browser cache seconds, revocation lag limit, fenced-principal bound, quotas,
  reservation bounds, per-kind maximum bytes, retention periods, GC delay, export lifetime and size, JPEG quality,
  gateway memory cache bytes, `cache_entry_max_bytes` (8 MiB), `exports_versioning_check_s` (60), `staging_deadline`,
  `cleanup_grace`, `refetch_backoff`, `quarantine_max_age` (20 h) and `quarantine_lifecycle_expiry` (24 h, also the
  input to S2's lifecycle rule), sweeper batch size.

## 18. Tests

Fakes and injected clocks only; PostgreSQL tests on the coordinator-owned scratch server; Spaces qualification
`#[ignore = "live Spaces qualification"]`. Race tests use controlled interleaving: a second connection runs the
competing statement, the test asserts through `pg_locks` (`granted = false`) that it waits, then releases the first;
no sleeps.

| Plan test | Arrange / act / assert |
| --- | --- |
| FILE-01 | oversize; 30000x30000 header; PNG sent as JPEG; animated WebP; APNG: `Rejected` before promotion, nothing in `PrivateMedia`, reservation released |
| FILE-02 | account X's ticket finalized by account Y refused; a staging write for another asset or a non-current attempt refused |
| FILE-03 | 600 generated assets, revisit the first 100 after eviction: served to a member with zero provider calls; refused to a non-member |
| FILE-04 | 5 MiB manifest with junk and an overlong line; offline import; restart; every key owned with the audience its grant lines prove |
| FILE-05 | campaign at quota: save commits; optional plate `Deferred` (zero calls); space freed, next need generates; upload refused |
| FILE-06 | live reference: delete refused; placeholder consent: tombstone, neutral chrome, a new save point commits, a branch from the old save commits, purge after the delay |
| FILE-07 | the next chunk after a revoking `campaign_event` is refused; renewal after revocation refused by the fresh load; no new grant |
| FILE-08 | section 12 (a) failing first; (b) in S4's suite |
| JOB-01 (media) | dispatch `Sent` then timeout; sweep runs; next need: zero provider calls (`InFlight`); customer hold released at 15 minutes (`CustomerReleased`): the next need still makes zero calls; a result at 2 hours is staged and finalizes; separately, after F's 24 h failure the row is `Rejected` and a later result is refused |
| AUTH-03 | another member's session with a valid draft capability; no session; content key: concealed 404; owner 200 |
| J11 | request, upload, screen pass, finalize, revision updated; failed validation leaves revision and quota unchanged |
| J14 | reauth; archive; access recorded; expiry deletes archive and wrapped key, journals `ExportExpired` |
| SAVE-03 (asset half) | branch copies references incl. placeholders; zero provider calls |
| OPS-02 (media half) | old PostgreSQL; journal replay inserts tombstones (stub when missing) and skips already-journaled pending rows before delivery reopens; quotas recomputed |
| OPS-03 (media half) | purged asset restored from backup stays refused |

Invariants (each failing before its fix):
- A purge claim on a `Deleted` row (fresh GC claim token as `step_owner`), then an orphan sweep: the winning writes
  stay in place until the purge itself deletes them; the sweep deletes only non-winners.
- A generation result staged 21 h after admission validates and finalizes (the quarantine window counts from
  `staged_at_ms`); its `review_deadline_ms` is `staged_at_ms + quarantine_max_age`.
- An uploader shares a portrait with a campaign and then leaves it: while the uploader's template revision references
  the portrait, the uploader's `Owner` grant serves it and the account export includes it; after the template
  revision is deleted, neither does (unless another basis applies).
- Sharing a shared portrait with a second campaign adds its bytes to that campaign only, under that campaign's quota
  row first.
- A sweep run between finalize's `head` and its compare-and-set (controlled interleaving: the sweep's `FOR SHARE`
  waits on finalize's `FOR UPDATE`, asserted through `pg_locks`) leaves the winner intact; a sweep that marks first
  makes finalize answer `WinnerMissing`; a write of the current owner on a `Validated` row older than
  `cleanup_grace` is never deleted.
- Dispatcher unreachable for longer than `staging_deadline`: the row stays `InFlight`, the key is not re-bought, and
  it later finalizes from the spool; two concurrent re-fetch requests for one attempt produce one staging write; only
  `OutputGone` yields `Rejected(output_lost)` with a company-loss outbox row.
- `quarantine_max_age` reached: a flagged upload becomes `Rejected(review_expired)`, an unjudged one
  `Rejected(screen_unavailable)`, a paid generation not yet validated `Rejected(staged_expired)` with an outbox row;
  a configured `quarantine_max_age` not below the lifecycle expiry falls back to the default with an audit line.
- A recorded staged object deleted or altered behind the catalog yields `Rejected(staged_lost)` once, with no retry.
- Template count: create, duplicate, import and claim each add one and are refused at the limit; delete subtracts
  one; restore within the window adds one and is refused at the limit; each locks the account quota row first.
- `share_to_campaign` moves the bytes from the account scope to the campaign scope under both quota rows (account
  first); a campaign without room refuses the share and nothing moves; deleting the account afterwards leaves the
  campaign's charge; erasing the shared upload releases it from the campaign scope; sharing with a second campaign
  creates a separate asset charged to that campaign.
- Worker W1's lease expires mid-publish and the sweeper finalizes: W1's late finalize returns `LeaseLost`, deletes
  nothing, and F's outbox holds exactly one outcome for the attempt; W1's and the sweeper's objects have different
  write ids, and the orphan sweep removes only W1's.
- F01 replacement: attempt A2's keys differ from A1's; a write by A1 after the supersede fails its conditional
  `intended` insert, and one that slipped past it is removed by the next orphan sweep: no object of A1 remains.
- A staging stream of `maximum_bytes_per_kind + 1` bytes is aborted at the limit, stores nothing, and makes the
  terminal transition `Invalid(over_maximum_bytes)`.
- `ResultKnown` success with nothing staged after `staging_deadline`: the dispatcher re-fetches (zero new provider
  calls) while the handle is valid; with an expired handle the row becomes `Rejected(output_lost)` with a company-loss
  outbox row.
- Sweeper `cancel_unclaimed_effect` racing the dispatcher's claim (controlled interleaving): exactly one wins; if
  the claim wins the row stays live and the output finalizes; never paid twice.
- `DispatchPrepared` after a crash is treated as possibly sent (no rejection, no new dispatch).
- A screen step taken over while its screen call is `Sent`: no second screen call until the first is terminal.
- A second staging write after `Quarantined` is refused; a staged object altered behind the catalog fails the
  digest check at publish.
- Two concurrent `admit_generation` calls for one key (two connections): one reservation, one `ProviderAttempt`,
  the loser gets `NotAdmitted(InFlight)` and F rolls back cleanly.
- A `Withheld` meet is refused before admission with zero provider calls; one test per audience-table cell.
- `MediaRequestKey`: two field lists that concatenate to the same text hash differently.
- Reservation expiry during a slow `Sent` job still finalizes; over-maximum output is `Invalid`.
- Lock order: save (O1 rows, then assets) versus erasure inside B's account-deletion transaction (account and
  finance rows, then assets) under controlled interleaving: no deadlock; a user delete (media-started) locks the
  quota row before the asset row and never locks finance, account or O1 rows (static check of the call graph plus a
  test); each quota-changing transition of section 10 has a test that it locks the quota row first.
- Concurrent first reservations of a new scope; a reference insert racing a delete (controlled interleaving).
- Journal: crash after stamping, new pending rows arrive, restart adopts the segment; crash after head advance,
  restart confirms without rewriting; restored pending rows already journaled are skipped; nonces never repeat.
- GC lease takeover; decode permit held until the thread returns; tombstone event drops the runtime set and gateway
  cache entries; exports are never cached; an export is refused when `Exports` versioning changes to `Enabled` or
  `Suspended` after start (within the 60 s check window); an object over `cache_entry_max_bytes` and every `Range`
  response are streamed and never enter the cache.
- Display grant needs both cookies; no audit line or dispatcher span contains a capability or provider URL; object
  keys and export names never contain user text; SigV4 example vectors.

## 19. Order of work (one round each, critic after each)

1. Object-store port, key layout with attempts, SigV4, fake, NativeFileStore, contract suite.
2. Catalog domain, `CatalogStore` with a test fake, migration 0010, `df-persistence` adapter, O1 reference functions.
3. Upload pipeline with the screen behind a fake dispatcher (FILE-01, -02, -05 storage half, J11).
4. Delivery gateway: grants, renewal, revocation fence, memory cache, headers (AUTH-03, FILE-07). Needs S1.
5. Imagery migration after A4 round 2, staging endpoint, sweeper rules (FILE-03, -04, -05 generation half, JOB-01
   media). Needs F's admission hook, `cancel_unclaimed_effect`, outcome events and settlement outbox consumer, and
   O1's dispatcher (durable output hold, re-fetch).
6. `df-library` (0011).
7. References, deletion, placeholders, GC (FILE-06). Needs O1's save transaction.
8. Exports (0012), J14, FILE-08 (a).
9. Spaces adapter; S2 receives bucket, versioning, replication-exclusion, lifecycle and redaction requirements.
10. Journal writer (0013), OPS-02/OPS-03 media drills.

## 20. Questions

Answered: Q-O1-1 (D7), Q-B3 (D9), Q-B4 (D8), Q-C3 (D10), Q-C4 (D13), Q-C5 and journal ownership (D22), Q-C6 and
Q-C6' (D14), archived quota and settlement (D22), presigned delivery (D23: none), save contract (D24), lock order,
`Host(c)`, the share step and unknown-outcome resolution (D25).

Still open:
- O1: Q-O1-2; Q-O1-4; the dispatcher's staging-endpoint call, late-result delivery, durable hold of the provider
  output (or its fetch handle) until staging acknowledges, re-fetch on request idempotent per `AttemptId`, and the
  positive `OutputGone { attempt }` answer, which the dispatcher must give once its spool retention for that
  attempt ends and the fetch handle has expired (so no row waits forever); confirm the D25 order in your save path;
  remove the object-store scope from `RuntimeSpec`.
- F (D27; `planning/finance-ledger-research.md`): `admit_generation` inside your admission transaction with rollback
  on `NotAdmitted`; `ProviderAttempt` state reads; `cancel_unclaimed_effect`; confirm that `CustomerReleased` never
  implies the supplier outcome is known; the settlement outbox consumer (including `output_lost`,
  `staged_expired`, `staged_lost`, `review_expired`, `screen_unavailable` and `over_maximum_bytes` as company loss
  under D21); your outcome event for D25 terminal failures; the screen-call
  settlement class; the personal usage statements as an `ExportSource` section; confirm the D25 order (finance
  rows, then media quota, then asset).
- B: recent-authentication for exports; mounts for the characters, storage and export pages; the account export
  JSON section; the account-deletion hook (`delete_account_media` inside your transaction).
- P (D28, `df-account-app`): the operator console's review-queue screen over `CatalogService::review_queue`, showing
  each item's `review_deadline_ms`, with approve and reject actions.
- S1: `campaign_event` (`event_seq`, `member_revision`, `revoking`) as the fence O2 consumes; a cross-campaign read
  for the gateway (one query over a per-campaign cursor table, so the lag figure covers every campaign) as part of
  the `campaign_event` contract; `verify_display_ticket` for media requests.
- S2: buckets or permission domains, one key per domain, `Exports` never versioned and excluded from replication
  and backups, Spaces `GetBucketVersioning` semantics, lifecycle rules (the `Quarantine` expiry generated from
  `quarantine_lifecycle_expiry`, strictly above `quarantine_max_age`), the export key-encryption key, the journal
  key, the off-host watermark, edge log redaction, internal mTLS between dispatcher and gateway.
- Coordinator: Q-C1 crate layout; Q-C2 campaign dedupe scope.
- A6: the `Deferred` hub outcome.

## 21. Risks and deviations

- Uploads and all media bytes flow through the control plane (D23): bounded server bandwidth in exchange for one
  enforcement point and per-chunk revocation.
- JPEG re-encoding is lossy; refusing animation is stricter than first-frame extraction.
- Spaces conditional writes, version listing and versioning status semantics are unverified; each has a stated
  fallback and nothing claims WORM.
- Export residual: wrapped data keys survive in database backups for their retention, with no archive to decrypt.
- Bytes already in a browser's private cache outlive a revocation by at most `browser_cache_seconds`.
- `/gameplay/imagery/*` GET routes remain a known deviation from gRPC-only transport, principal-bound and served by
  the gateway.
