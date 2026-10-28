# Imagery service composition contract (IMG-SERVICE-COMPOSITION-A2)

Status: Proposed contract addressing every finding of the A1 independent review
(`artifacts/tmp/imagery-20261027/wave2/service-composition-review.json`, verdict
reject, SC1-SC7). No runtime code is changed or enabled by this document. Paid
dispatch remains blocked until the follow-on code lands and passes review.

Inspected in this session (2026-10-28, shell disabled so no commit hash observed;
root freezes the exact commit before implementation):
`crates/df-finance/src/admit.rs:101-177`, `ports.rs:53-84`, `held.rs:48-133`,
`service.rs:170-213`, `crates/df-assets/src/catalog_store.rs:36-54,285-382`,
`catalog_service.rs:235-284,589-623,732-771`, `catalog_ports.rs:50-64`,
`catalog_record.rs:103-193`, `quota.rs:8-37`, `object_store.rs:158-251`,
`media_request_key.rs:68-183`, `media_ids.rs:50-77`, `publication.rs:104-186`,
`crates/df-imagery/src/request.rs:74-313`, `output.rs:202-215`,
`journal.rs:196-233,337-500`, `transform.rs:22-61`,
`crates/df-persistence/src/imagery_store.rs:34-77`,
`imagery_sql/statements.rs:20-100`,
`crates/df-tools/src/gameplay/cost/port.rs:150-317`,
`crates/df-commerce/src/finance_ids.rs:64-80`, `settlement.rs:94-117`,
`planning/imagery-implementation/contract.md:11-17`,
`planning/imagery-implementation/operation-store.md:13-44`.

Base: accepted recipe keys (`MEDIA_RECIPE_SCHEMA = "df-imagery/3"`,
`media_request_key.rs:146`), request foundation (`request.rs`, `output.rs`,
`transform.rs`), journal candidate with transaction-taking statements
(`imagery_store.rs:34-77`), migration `0038_imagery_operations.sql`
(`imagery_store.rs:21-24`; A1 cited `0037`, which is wrong).

Corrections to A1 citations: `OutputProfile` lives in `output.rs:202-215`, not
`request.rs`. The journal lost-commit `Unknown` (`imagery_store.rs:112-121`)
covers only the store commit, never duplicate-dispatch safety. No test is cited
as passed; all tests below are required future work.

## 1. Authorities (freezes SC1)

`FinanceService::admit` owns its transaction (`admit.rs:101-177`): it starts the
transaction, locks (`finance.lock_flow`), decides (`decide_admission`),
commits the plan, then asks fence-then-media inside the same transaction
(`ask_fence_and_media`, `admit.rs:182-200`), then commits. There is no
`CommerceRepository::transact`; the A1 `transact` composition is deleted and not
revived. The image extension follows the same shape: a finance-owned entry
point sharing the existing admission core, invoking an injected
transaction-taking image link port before commit.

Lock order for one paid image-stage admission: finance rows first, then O1 fence
(existing D25 order), then quota/catalog rows in `QuotaScope` order
(accounts before campaigns, `quota.rs:8-37`), then the imagery job/stage row.
No provider I/O under any lock. Durable intent acceptance (journal `accept_in`
alone) is a separate earlier transaction that takes no finance or catalog lock
and dispatches nothing.

```rust
// df-finance/src/image_stage.rs (new file; root registers the module).
// df-finance may add a native df-assets dependency for the Admission type.
// It must not import df-imagery or df-persistence.
use df_commerce::finance_ids::{AttemptId, EffectId, OperationKey};

pub struct ImageStageLink {
    pub job: [u8; 16],
    pub stage: [u8; 16],
    pub asset: [u8; 16],
    pub publication_attempt: [u8; 16],
    pub owner_basis_digest: [u8; 32],
    pub claim_generation: i64,
    pub expected_job_revision: u64,
    pub child_attempt: AttemptId,
}

pub struct FinanceChild {
    pub operation: OperationKey,
    pub attempt: AttemptId,
    pub effect: EffectId,
}

pub enum StageAction {
    Deferred,
    Admit(df_assets::Admission),
    VerifyExisting { asset: df_assets::AssetId, publication_attempt: df_assets::AttemptId },
}

pub struct ImageStageAdmission {
    pub link: ImageStageLink,
    pub child: FinanceChild,
    pub publication: StageAction,
    pub intent_fingerprint: [u8; 32],
}

pub enum ImagePhase { New, Replay }

pub struct StoredImageLink {
    pub link: ImageStageLink,
    pub child: FinanceChild,
    pub intent_fingerprint: [u8; 32],
}

pub enum ImageLinkVerdict {
    Bound,
    Existing { stored: StoredImageLink },
    Conflict,
    Stale,
    ParentUnavailable,
}

pub trait ImageStagePort: Send + Sync {
    fn bind_image_stage(
        &self,
        tx: &tokio_postgres::Transaction<'_>,
        request: &ImageStageAdmission,
        phase: ImagePhase,
    ) -> impl std::future::Future<Output = Result<ImageLinkVerdict, crate::ports::PortUnavailable>> + Send;
}

pub enum ImageAdmissionError {
    Finance(crate::service::FinanceError),
    LinkConflict,
    LinkStale,
    ParentUnavailable,
    Catalog(df_assets::NotAdmitted),
    DualMedia,
    IdentityMismatch,
    PortUnavailable,
}
```

`owner_basis_digest` is SHA-256 over the canonical `owner_scope_bytes`
(`journal.rs:196-214`); it binds the exact stored owner. `claim_generation`
and `expected_job_revision` are CAS guards, never part of any admission
fingerprint. The immutable child tuple
(job, stage, asset, publication attempt, child operation/attempt/effect, owner
digest, intent fingerprint) is what both new admission and `AlreadyAdmitted`
replay verify exactly, plus the payload digest below.

```rust
impl<F: FencePort, M: MediaPort, S: MemberPort, I: ImageStagePort> FinanceService<F, M, S, I> {
    pub async fn admit_image_stage(
        &mut self,
        command: &AdmitCommand,
        media: &ImageStageAdmission,
    ) -> Result<AdmissionOutcome, ImageAdmissionError>;
}
```

Rules: `command.media` must be `None` on this branch (`DualMedia` otherwise, so
one admission never charges twice). Finance identities in `command`
(operation/attempt/effect, fence campaign/session) must equal the linkage in
`media` (`IdentityMismatch` otherwise). The body reuses the existing admission
core: lock, `parse_admission_snapshot`, authorized payers, `decide_admission`,
`commit_plan`, fence, then `bind_image_stage(tx, media, phase)` under the same
`port_deadline_ms` bound, then commit. The phase is `New` when no linkage row
exists for the child operation under the lock, else `Replay`. On `Replay` the
service compares the stored receipt, intent fingerprint and full linkage inside
the same finance transaction before returning `AlreadyAdmitted`; a replay never
creates a send permit and stays readable after later owner retirement. Catalog
rows are never reinserted on replay. Port outcomes map: `Bound` to `Admitted`,
`Existing` (after exact comparison) to `AlreadyAdmitted`, `Conflict` to
`LinkConflict`, `Stale` to `LinkStale`, `ParentUnavailable` to
`ParentUnavailable`, catalog refusal to `Catalog(reason)`. A commit failure is
`Unknown`: reread before any retry, never resend. The existing non-image
`admit` path is unchanged.

## 2. Submission lifecycle (freezes SC2, SC4)

Live submissions accept durable intent first. Paid classification precedes the
final recipe freeze: classification is itself an admitted stage, its result is
durably attached to the job, and only then is the output route frozen.

| Step | Action | Writes |
| --- | --- | --- |
| Accept | Journal `accept_in` alone (`Create` or explicit `Join`) | Receipt + interest, `Planning` job |
| Allocate | Mint stable publication asset id + publication attempt id on the job | Job row fields, no asset row claimed |
| Classify | `admit_image_stage` with `StageAction::Deferred` | Finance hold + bound link, no catalog row |
| Retain | Persist classifier result under the existing attempt | Stage row result bytes |
| Freeze | Journal `freeze_in` with resolved key + recipe snapshot | `RecipeFrozen`, revision bump |
| Produce | `admit_image_stage` with `StageAction::Admit(full Admission)` for the first output stage | Finance hold + catalog row |
| Extend | Later stages use `StageAction::VerifyExisting` | Linkage check only |

`PreparedOnly` and `Replay` perform authorized lookups only: request-key lookup
plus `lookup_in`/`read_in` coherent reads. A hit returns the ready receipt (or
joins a read-only interest on an in-flight job without changing attempt state);
a miss returns typed `PreparedMiss`/`ReplayMiss` and writes no job, stage,
admission or catalog row, so a miss never becomes runnable work.

Public facade (existing `AuthorizedImageRequest` wrapper and `ImageAccess` are
deleted; they invented a second identity and unnamed states):

```rust
impl<A: ImageAuthorizer, J: ImageJournal, ..> ImageService<A, J, ..> {
    pub async fn submit(&self, caller: &A::Caller, request: ImageRequest)
        -> Result<Submission, SubmitError>;
    pub async fn status(&self, caller: &A::Caller, job: ImageJobId)
        -> Result<ImageStatus, LookupError>;
    pub async fn watch(&self, caller: &A::Caller, job: ImageJobId)
        -> Result<ImageUpdates, LookupError>;
    pub async fn cancel(
        &self, caller: &A::Caller, interest: ImageInterestId,
        capability: InterestCapability, reason: CancelReason,
    ) -> Result<CancelOutcome, CancelError>;
}
```

`Caller` is an associated type of the injected authorizer, never a minted
permission token. Authorization privately constructs one canonical
request/snapshot pair from caller context plus the existing `ImageRequest`
(`request.rs:300-313`); the constructor takes no ambient environment and all
ids/times are injected:

```rust
pub trait ImageAuthorizer: Send + Sync {
    type Caller: Clone + Send + Sync;
    fn authorize(
        &self, caller: &Self::Caller, request: &ImageRequest, now_ms: i64,
    ) -> Result<Authorization, AuthzError>;
}

pub struct Authorization {
    pub owner: ImageOwner,
    pub audience_meet: Audience,
    pub readable_sources: Vec<SourceRef>,
    pub sharing: SharingIdentity,
    pub intent: VersionedSnapshot,
    pub config: VersionedSnapshot,
}
```

Source resolution: every `SourceRef` pin is rechecked against the catalog and
the auth owner at submit; the audience is the requested audience intersected
with each reference audience and `Withheld` is refused before the journal is
touched. Fresh regeneration (`Freshness::Regenerate`) uses a new
operation/nonce identity and can never join old work. `submit` validates
`ImageRequest::validate` (`request.rs:321-351`): `Generate`/`Edit` require an
`OutputProfile::Provider` profile, `Transform` requires `OutputProfile::Local`
(`output.rs:202-215`); `Transform` spec validation comes from `transform.rs`.

Read/execution model: `ImageStatus { job, revision, state:
Queued/Processing/Ready/Failed/Cancelled/AwaitingReconciliation, reason,
ready: Option<ReadyBinding>, outstanding_liability: bool }`. `Ready` requires
the durable job-result row (section 6) with primary `AssetRef`, dimensions,
MIME, alpha facts, digest, source binding and quality evidence
(`Passed`/`Unjudged`/`AcceptedWithKnownDefect`); rejected candidates are never
ready. `watch` returns bounded coalesced revision notifications; dropping a
watcher never cancels. `ImageUpdates { revision, state, ready: Option<...> }`
with monotone revision per job. `cancel` withdraws only the caller's interest
(`withdraw_in`); the last withdrawal retires the job while preserving facts.
`InterestCapability` is minted by the service at accept, stored alongside the
interest row (root adds the column), and required with the caller on
status/watch/cancel; a mismatch returns `NotFound`, never an oracle.

Config and dispatch linearization: the service holds the effective
`ImageConfig` revision. A disable (`off`, `cache-only`, purpose/backend off)
sets a pending gate, fences every dispatch owner (scheduler, local workers,
hosted egress), and each owner reports its applied revision. The disable ack is
returned only after all affected owners applied or fenced it; before that,
submissions report `Disabling { revision }`. A runtime-local update that has not
propagated to hosted egress returns `AppliedLocal { pending_propagation }`,
never a false global success. Once acknowledged, no queued stage starts under
the old snapshot; started stages drain without beginning another effect stage.
Queue/deadline/concurrency changes never invalidate content keys; effective
output/prompt/route/acceptance changes do.

## 3. Catalog admission mapping and account drafts (freezes SC3)

The async `CatalogStore`/`ObjectStore` lifecycle is the canonical path:
`record_staged` -> `mark_validated` -> `intend_variant` -> immutable
`ObjectStore::put_new` -> `mark_variant_written` -> `finalize`. The generic
`publication::publish` receipt (`publication.rs:154-186`) is not used and never
substitutes for `FinalizeResult::Ready`; no `Version`/`Metadata`/`AssetMetadata
Store` adapter is retained for imagery. Every `Admission` field
(`catalog_store.rs:36-54`) is mapped below; the service builds the payload and
the finance transaction admits it.

| Admission field | Run scope | Account draft scope |
| --- | --- | --- |
| `asset` / `attempt` | Pre-allocated job publication ids | Same |
| `kind` | `AssetKind::Image` | Same |
| `provenance` | `Generated` | `Generated` (pixel-derived sources keep lineage; no silent relabel) |
| `charge_scope` | `QuotaScope::Campaign(campaign)` | `QuotaScope::Account(account)` |
| `origin_account` | Caller account | Caller account (required) |
| `campaign` | `Some(campaign)` | `None` (allowed only under the draft rule below) |
| `audience` | Requested meet, `Withheld` refused earlier | Same; a private draft meets only its account audience |
| `dedupe_scope` | `DedupeScope::campaign` | `DedupeScope::account` |
| `request_key` | `Some(MediaRequestKey::derive_recipe)` over frozen recipe | Same; owner scope string is the draft account |
| `retention` | `CampaignMedia` (or `Intermediate` for pure prep) | `Personal` |
| `reserve_bytes` | `CatalogConfig.maximum_bytes.for_kind(Image)` | Same kind bound under the account quota |
| `byte_limit` | `quota.byte_limit(Campaign)` | `quota.byte_limit(Account)` |
| `in_flight_upload_limit` | `None` (generated input uses no upload count) | `None` |

Catalog refusal mapping: `AudienceEmpty` (refused before catalog),
`InFlight { attempt }` (join or transfer only an authorized compatible
interest; already-incurred classification liability is retained, never
recharged or overwritten), `Ready { asset }` (reuse receipt, no new dispatch),
`QuotaFull` (typed refusal; storage quota is not a spend cap), `UploadLimit`
(unreachable for generation; `None` above), `OwnerUnsupported` (only until the
draft extension lands; never bypassed by fabricating a campaign or template).

Account-owned creator drafts are explicitly refused today and cannot be solved
by renaming: `CatalogService::admit_generation` rejects `campaign=None`
(`catalog_service.rs:246-254`), and `advance`/`publish` return
`UnsupportedOwner` without a campaign (`catalog_service.rs:601-602,732-740`).
Personal object layout additionally requires `CharacterRevision`
(`object_store.rs:150-192`), while `ImageOwner::Draft` carries only an account
(`request.rs:74-79`), and hosted `AdmitCommand` requires campaign/session
(`admit.rs:26-39`). The required source-backed extensions (follow-on code, not
claimed here):

1. `df-assets`: `KeyLayout::draft_asset(account, asset, attempt, write,
   variant)` under `PrivateMedia` as
   `users/<hex>/drafts/<asset>/<attempt>/<write>/<variant>`; admit/advance/
   publish accept `campaign=None` exactly when `provenance==Generated`,
   `retention==Personal`, `charge_scope==Account(account)==origin_account`,
   `dedupe_scope==DedupeScope::account(account)`; delivery authorization checks
   the account capability independent of campaign membership.
2. `df-finance`: account-scoped hosted funding gate for `AdmitCommand`
   (finance owner defines the session binding). Until it lands, hosted Draft
   generation is refused with typed `AccountFundingUnsupported`; local Draft
   uses the same durable attempt identity with record-only cost policy, never a
   fabricated campaign, wallet or spend entry.
3. Local metering (section 6) stays record-only under the existing policy.

## 4. Publication commit and stage schema (freezes SC3, SC5)

`finalize` stays `Validated`-to-`Ready` in one transaction: quota row then asset
row, every winner `written`, compare-and-set on `(asset, attempt, state,
owner)` (`catalog_store.rs:375-382`). Job-ready additionally requires the
imagery job-result row, written atomically by the new transaction-taking
function (implemented in `df-persistence`, specified here for the finance-owned
composition to call):

```rust
pub struct ImageResultBinding {
    pub job: [u8; 16],
    pub stage: [u8; 16],
    pub winners: Vec<df_assets::Winner>,
    pub mime: String,
    pub width_px: Option<u32>,
    pub height_px: Option<u32>,
    pub sha256: [u8; 32],
    pub quality: QualityEvidence,
    pub recipe_digest: [u8; 32],
}

pub enum QualityEvidence { Passed, Unjudged { reason: &'static str }, AcceptedWithKnownDefect { reason: &'static str } }

pub async fn finalize_image_in(
    tx: &tokio_postgres::Transaction<'_>,
    held: df_assets::HeldStep,
    request: &df_assets::FinalizeRequest,
    result: &ImageResultBinding,
    now_ms: i64,
) -> Result<(df_assets::FinalizeResult, JobResultOutcome), ImageCommitError>;

pub enum JobResultOutcome { Recorded, NotRecorded { finalize: df_assets::FinalizeResult } }
pub enum ImageCommitError { Catalog(df_assets::CatalogError), Conflict, Stale, InvalidRecord, Unavailable, Unknown }
```

Semantics: run the catalog finalize checks under the row locks; only on
`FinalizeResult::Ready { .. }` insert the job-result row in the same
transaction and return `Recorded`. On `WinnerMissing`, `LeaseLost`,
`Superseded` or `Rejected`, write no job-result row and return `NotRecorded`.
No external byte writes occur inside the transaction: bytes are immutable in the
object store (`put_new` + `mark_variant_written`) before this call. The caller
maps `LeaseLost`/`Superseded` to stop-without-deleting and `Rejected` to the
terminal outbox path. The recipe digest binds the frozen
(`MediaRequestKey`, recipe snapshot, variant winners, output bytes digest);
replay compares it exactly.

Proposed stage schema (new migration; root numbers and registers):

```sql
CREATE TABLE imagery.image_stage (
    job_id bytea NOT NULL, stage_id bytea NOT NULL,
    owner_scope bytea NOT NULL, claim_generation bigint NOT NULL DEFAULT 0,
    job_revision bigint NOT NULL, stage_kind text NOT NULL,
    state text NOT NULL, publication_asset bytea, publication_attempt bytea,
    finance_operation text NOT NULL, finance_attempt text NOT NULL,
    finance_effect text NOT NULL, owner_digest bytea NOT NULL,
    intent_fingerprint bytea NOT NULL, recipe_digest bytea,
    result_bytes bytea, created_at_ms bigint NOT NULL,
    updated_at_ms bigint NOT NULL,
    PRIMARY KEY (job_id, stage_id),
    CONSTRAINT image_stage_kind CHECK (stage_kind IN
      ('classification','generation','extraction','inspection','correction','local_transform')),
    CONSTRAINT image_stage_state CHECK (state IN
      ('planned','admitted','dispatch_prepared','sent','succeeded','failed','unknown')),
    CONSTRAINT image_stage_ids CHECK
      (octet_length(job_id)=16 AND octet_length(stage_id)=16
       AND octet_length(owner_digest)=32 AND octet_length(intent_fingerprint)=32)
);
CREATE TABLE imagery.image_dispatch_attempt (
    finance_attempt text PRIMARY KEY, job_id bytea NOT NULL, stage_id bytea NOT NULL,
    dispatch_seq integer NOT NULL, supplier_job_id text,
    usage_revision integer NOT NULL DEFAULT 0,
    settlement text NOT NULL DEFAULT 'held',
    created_at_ms bigint NOT NULL, updated_at_ms bigint NOT NULL,
    CONSTRAINT image_dispatch_settlement CHECK (settlement IN
      ('held','settled','unusable_settled','released','unknown_outstanding'))
);
CREATE TABLE imagery.image_job_result (
    job_id bytea PRIMARY KEY, stage_id bytea NOT NULL,
    publication_asset bytea NOT NULL, publication_attempt bytea NOT NULL,
    mime text NOT NULL, width_px integer, height_px integer,
    sha256 bytea NOT NULL, quality text NOT NULL, recipe_digest bytea NOT NULL,
    ready_at_ms bigint NOT NULL
);
CREATE INDEX image_stage_job_state ON imagery.image_stage (job_id, state);
```

Hosted-financed dispatch transitions (guards are CAS on
`(stage, claim_generation)` or the finance attempt state; every row is updated,
never deleted):

| Command/event | Guard | Write | Outcome |
| --- | --- | --- | --- |
| Admit stage | Job `RecipeFrozen` (or `Planning` for classification), expected revision | `planned`->`admitted`, bind link | `Admitted` |
| Prepare dispatch | `admitted`, finance `prepare_dispatch` ok | `admitted`->`dispatch_prepared` | Dispatchable |
| Mark sent | Supplier id returned, finance `mark_attempt_sent` ok | `dispatch_prepared`->`sent` + supplier id | In flight |
| Bytes complete | Staged + validated + variants written | `sent`->`succeeded` candidate | Await finalize |
| Finalize `Ready` | `finalize_image_in` returns `Recorded` | Job-result row | `Ready` |
| Output unusable | Metering marks unusable | `settle_unusable` + `unusable_settled` | Customer not charged |
| Cancel before send | Finance `release` ok (proven unsent) | `admitted`->released | Reservation released |
| Timeout/unknown | No conclusive answer | `sent`->`unknown` | Liability retained, no resend |
| Fresh retry | Explicit new child attempt | New linkage + rows | Old liability preserved |

Local-accounted dispatch (same tables, no finance rows): admit records the
attempt identity with record-only policy; cancel before start deletes nothing
and writes a terminal local row; crash recovery reruns from the durable source
without paid regeneration; `unknown` never triggers paid source regeneration.

CAS ownership: step leases stay `HeldStep { asset, attempt, owner: ClaimToken }`
(`catalog_record.rs:138-145`); stage claims mint a fresh token per claim and a
write carrying an older token is a lost lease. Claim generation increments on
every supersede; retry and claim values that may change are excluded from the
immutable admission fingerprint.

## 5. Cost projection over the existing engine (freezes SC6)

No new money ledger, wallet, price table or spend cap. The existing cost engine
stays the only bookkeeping: `MeteringPort::start/settle/not_billed`
(`port.rs:150-317`), `SpendLedger` reserve/commit/refund/reconcile, and the
finance hold/settle/release/credit flows (`held.rs:48-158`). Calling `start`
again creates a new book call identity, so the imagery composition must map one
durable child attempt plus usage revision to exactly one existing cost entry:
the service keeps `(finance_attempt, usage_revision) -> call_id` and starts at
most once per key; recovery rereads the key instead of starting again.

```rust
// df-tools owned (consumer side); injected into the imagery composition.
pub trait ImageCostProjection: Send + Sync {
    fn key_for(&self, attempt: &str, usage_revision: u32) -> String;
    fn record_reported(&self, key: &str, route: &PortRoute, units: &[MeteredUnits]);
    fn record_unpriced(&self, key: &str, route: &str, detail: &str);
}
```

Settlement mapping (finance rows are authoritative; the local book is a
projection and never overrides them):

| Evidence | Finance call | Book projection |
| --- | --- | --- |
| Usable, supplier-reported usage | `settle(attempt, units, Reported)` | `settle(Reported)` |
| Usable, counted/estimated usage | `settle(attempt, units, Estimated)` | `settle(Estimated)` |
| Billed but unusable output | `settle_unusable(attempt, units, source, outcome)` with `UnusableOutcome::{InvalidOutput, FailedNoOutput, Duplicate, Superseded}` (`settlement.rs:109-117`); customer never charged, supplier cost is company loss | `settle_unusable`, `usable=false` |
| Proven unsent (refused submit, cancel before dispatch) | `release(attempt, reason)`; a sent attempt cannot be released | `not_billed`, nothing recorded |
| Ambiguous timeout / lost receipt | Neither settle nor release; liability retained, reconciliation continues; never an automatic resend | Drop records `Unknown` at maxima only when the send marker exists |
| Codex envelope, priced lines | Envelope attempt settled on reported lines | Priced lines settle; subscription units listed separately |
| Codex envelope, no tariff line | No invented dollar figure | `unpriced=true`, amount from priced lines only, never zeroed |
| Missing usage evidence | Outstanding, never zero | `Unknown`, counted as unresolved exposure |

A finance reservation dropped pre-send via `release` is conclusive, not
uncertain liability: `CannotReleaseSent` proves sent attempts cannot take that
path. A dropped `PortCall` recording `Unknown` maxima (`port.rs:181-186`) is
local bookkeeping for an actually-sent call; it never releases finance state
and never authorizes a resend. `UsageSource` in finance admits only
`Reported`/`Estimated` (`settlement.rs:94-98`); unknown stays outstanding.

## 6. Paid classification detail

Classification is a `stage_kind='classification'` row admitted with
`StageAction::Deferred` against the pre-allocated publication identity; it
claims no asset row. Its finance child (operation/attempt/effect) is stable per
job: repeated polling and duplicate submissions reuse it, and the classifier
result bytes are retained under that attempt before `freeze_in`. A
refusal/uncertain classification stops the job; no output stage is admitted.
After freeze, the output route is resolved and the first output-producing stage
uses `StageAction::Admit` with the full `Admission` payload; catalog
`InFlight`/`Ready` on that key transfers or joins only authorized compatible
interests and never overwrites or recharges the retained classification
liability. Local-only stages (`local_transform`, reference preparation without
a paid check) carry durable job/stage and publication identity with zero
supplier attempt, reservation or settlement, per the transformations plan.

## 7. First full integration and file/task dependencies

Three worked integrations prove the whole path on real adapters (no local-only
substitute, no invented protocol):

1. Generate, creator portrait, run scope: `submit(Live, ReuseCompatible)` ->
   accept -> allocate -> classify (`Deferred`) -> retain route -> freeze ->
   `Admit` generation stage -> dispatch (OpenAI/fal/BFL/Codex envelope) ->
   `record_staged` -> validate -> `intend_variant` + `put_new` variants ->
   `mark_variant_written` -> `finalize_image_in` (`Ready` + `Recorded`) ->
   `settle(Reported)` -> `status Ready` with private draft readable only by its
   account audience.
2. Edit, scene plate: same path with `EditSpec` base pin rechecked at submit;
   quality correction is a second admitted `correction` stage with its own child
   identities, never an overwrite of the first attempt.
3. Transform, hero thumbnail, `local-only`: `submit(Transform, Live)` ->
   accept -> `Admit` catalog row with no finance admission -> local worker
   (bounded permits, per-step dimension/memory check, EXIF-first, sRGB-linear
   working, premultiplied filter, matte on JPEG) -> variants ->
   `finalize_image_in` (`Ready` + `Recorded`) -> record-only cost projection.
   `PreparedOnly` miss returns `PreparedMiss` with zero finance calls.

| File (new unless noted) | Owner | Depends on |
| --- | --- | --- |
| `df-imagery/src/service.rs`, `auth.rs`, `status.rs` + tests | Service implementer | This contract, journal, request/output/transform |
| `df-finance/src/image_stage.rs` (section 1 types + `admit_image_stage`) | Finance implementer | New `df-assets` dep (root adds) |
| `df-persistence/src/imagery_stages.rs` + migration (section 4 SQL) + `ImageStagePort` impl + `admit_generation_in(tx, &Admission)` | Persistence implementer | `CatalogStore` PG statements, `imagery_store` tx pattern |
| `df-assets`: `KeyLayout::draft_asset`, account draft admit/advance/publish path, draft delivery auth | Assets implementer | Section 3 rule |
| `df-tools`: `ImageCostProjection` adapter over `MeteringPort` | Consumer implementer | Section 5 keying |
| Root only: `lib.rs`/`mod.rs` registrations, `Cargo.toml`/lock, migration numbering/runtime wiring, interest capability column | Root | Snippets below |

Root integration snippets (specification only; root applies):

```toml
# df-finance/Cargo.toml (root-owned)
df-assets = { path = "../df-assets" }  # native only; no wasm image execution
```

```rust
// df-finance/src/lib.rs (root-owned)
mod image_stage;
pub use image_stage::{FinanceChild, ImageAdmissionError, ImagePhase, ImageStageAdmission, ImageStageLink, ImageStagePort, StageAction, StoredImageLink};
```

```rust
// df-persistence migration registration (root-owned): number the section-4
// schema migration after 0038 and register it beside imagery_store.rs:21-24.
```

No `lib.rs`, manifest, lock or migration-harness edit is made by contributors.

## 8. Failure-focused tests (required once implemented)

Each test fails before the fix and passes after; fakes only, injected clocks,
no sleeps, no live calls: replay verifies full linkage and rejects a changed
job/stage/asset/publication-attempt/child-operation/fingerprint with
`LinkConflict`; `DualMedia` refused when `command.media` is `Some`;
`IdentityMismatch` when finance and image identities differ; `PreparedOnly`
and `Replay` misses write no rows; double `submit` creates one job and one
classification attempt; classification refusal admits no output stage;
`InFlight` catalog response joins without recharging; `finalize_image_in`
records the job result only on `Ready`; `WinnerMissing`/`LeaseLost`/
`Superseded`/`Rejected` record nothing; cancel before dispatch releases only
proven-unsent holds; timeout retains `unknown` with no resend; stale CAS
returns `Stale` with the current revision; wrong-owner status/watch returns
`NotFound`; draft submit without the section-3 extension returns
`OwnerUnsupported`/`AccountFundingUnsupported`, never a fabricated campaign;
unusable output settles without customer charge; unpriced Codex usage stays
visible and nonzero-chargeable lines are never zeroed; restart recovers from
recorded rows without duplicate dispatch or double cost entries.

## 9. Resolved, remaining, and retained evidence

Resolved by this contract: finance-owned admission with an injected
transaction-taking hook (SC1); intent-first ordering with paid classification
before freeze and non-creating `PreparedOnly`/`Replay` misses (SC2); exact
catalog `Admission` mapping, async publication path, `finalize_image_in`, and
the explicit account-draft extensions (SC3); caller-typed facade with private
canonical construction and a typed execution/read model (SC4); stage schema,
CAS, linkage, transition tables and local/hosted dispatch rules (SC5);
durable-attempt cost projection with full settlement mapping and no new ledger
(SC6); exact source anchors including `output.rs` and migration `0038` (SC7).

Remaining for root/owners, none of them an open blocker in this design:
numbering/registering the migration, adding the `df-assets` dependency to
`df-finance`, the interest capability column, the three section-3 draft code
extensions, and the section-7 implementation tasks in dependency order.

Retained evidence from A1 (no repeated scan): the A1 handoff reported no
deletion performed and nothing proven eligible for cleanup --
`artifacts/tmp/imagery-20261027/wave2/` is current-wave active evidence and
builds/evidence/DBs plus the durable workflow DB stay retained per ADR0002;
eligibility needs approved root plus terminal/abandoned state, 30 minutes,
no active claim, superseded/reproducible content and no retained-evidence
status, none of which was established. That assessment is carried forward
unchanged.
