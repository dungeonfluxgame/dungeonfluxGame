# Imagery service composition contract (IMG-SERVICE-COMPOSITION-A3)

Status: Implementable source-backed contract repairing every A2 finding
SC-A2-1 through SC-A2-9
(`artifacts/tmp/imagery-20261027/wave2/service-composition-a2-review.json`,
verdict reject). No runtime code is changed or enabled by this document. Paid
dispatch stays blocked until the lanes in section 11 land with the evidence in
section 12 and pass independent review. This document is not feature
acceptance; root reviews and dispatches the executable lanes.

Inspected in this session (shell disabled, so no commit hash observed; root
freezes the exact commit before implementation):
`crates/df-finance/src/image_stage.rs:1-377` (actual hook, binding, digest,
errors),
`crates/df-finance/src/admit.rs:98-242` (`admit_with_hook`, digest fold,
asset stamp, replay verify, commit mapping),
`crates/df-finance/src/service.rs:34-125` (actual `FinanceError` variants),
`crates/df-finance/src/held.rs:47-210` (settle/settle_unusable/release/
prepare_dispatch/mark_attempt_sent),
`crates/df-finance/src/ports.rs:27-84` (Fence/Media/Member ports),
`crates/df-finance/src/lib.rs:26-27` (module registration),
`crates/df-finance/src/host_funding.rs:1-125,156-290` (session/campaign
consent, open/close),
`crates/df-finance/tests/image_stage.rs:1-150,277-491` (linkage-table hook
pattern, digest/rollback/replay coverage),
`crates/df-commerce/src/lifecycle.rs:155-222` (`plan_prepare_dispatch`,
`plan_mark_sent`: `DispatchPrepared -> Sent` committed before any byte
leaves),
`crates/df-commerce/src/settlement.rs:27-117` (`SettleRefusal::
CannotReleaseSent`, `UsageSource::Reported/Estimated`,
`UnusableOutcome` four variants),
`crates/df-commerce/src/admission.rs:27-76` (`Route`, `WorkClass`,
`AdmissionRequest` with tenant/campaign/session/operation/attempt/effect),
`crates/df-commerce/src/finance_ids.rs:64-131` (16-byte ids),
`crates/df-assets/src/catalog_store.rs:33-131` (`Admission`, `NotAdmitted`,
`admission_owner`/`record_owner` with the `AccountDraft` rule),
`crates/df-assets/src/catalog_service.rs:70-91,235-284,589-623,732-771`
(`GenerationRequest`, no-money admission, `advance`/`publish` owner gates),
`crates/df-assets/src/object_store.rs:158-263` (`StorageOwner::
Campaign/AccountDraft`, `account_draft_asset` under
`users/<hex>/draft-assets/...`),
`crates/df-assets/src/catalog_ports.rs:17-86` (`FinanceAttempts`,
`Dispatcher`, O2-01..O2-04 gaps),
`crates/df-assets/src/catalog_sweep.rs:163-259` (unstaged/refetch consumers
of the catalog attempt),
`crates/df-persistence/src/media_catalog_sql/admission.rs:14-80`
(`admit_in`: owner gate, quota lock, dedupe conflict),
`crates/df-persistence/src/imagery_store.rs:1-121` (migration `0038`,
transaction-taking `accept_in`/`lookup_in`/`read_in`/`freeze_in`/
`withdraw_in`, commit-only `Unknown`),
`crates/df-persistence/src/imagery_sql/statements.rs:20-115`
(READ COMMITTED, advisory scoped-accept lock, job-before-interest locking),
`crates/df-imagery/src/journal.rs:24-500` (owner scopes, snapshots,
`JobState`, `ImageJournal`),
`crates/df-imagery/src/request.rs:74-351` (owners, purposes, modes,
validation),
`crates/df-imagery/src/output.rs:202-241` (`OutputProfile`
Provider/Local),
`crates/df-imagery/src/transform.rs:22-71,478-500` (claimed sources,
ordered steps, bounds),
`crates/df-assets/src/media_request_key.rs:68-183` (`derive_recipe`,
`MEDIA_RECIPE_SCHEMA = "df-imagery/3"`),
`crates/df-tools/src/gameplay/cost/port.rs:150-318` (`MeteringPort`,
consumed `PortCall`, `not_billed`),
`planning/imagery-implementation/operation-store.md`,
`artifacts/tmp/imagery-20261027/wave2/service-boundary-brief.md` (the
request/read/watch/cancel boundary another worker is now implementing;
authoritative for section 2 and not duplicated here),
`planning/imagery-implementation/consumer-migration.md:32-139`
(creator/plate/canonical/mural consumers and provider constructors),
`planning/commerce-service.md:19-33` (reservation lifecycle, unknown
liability, optional-video gate).

## 0. What A2 got wrong and what replaces it

The A2 document invented signatures that the landed code does not have. The
replacements below are the only contract; the invented names are deleted and
not revived.

| A2 invented | Actual current source | A3 contract |
| --- | --- | --- |
| `ImageStageLink` + `owner_basis_digest` + `claim_generation` + `expected_job_revision` | `ImageStageBinding` (`image_stage.rs:131-144`): `owner_scope: OwnerScope` (bounded 128 bytes, `MAX_OWNER_SCOPE_BYTES`), `job/stage/asset/publication_attempt` ids, `tenant/campaign/session/operation/attempt/effect` | Bind exactly this struct; equality data comes from fields present in it, never from absent digests or generations |
| `ImageStagePort::bind_image_stage(tx, request, phase)` returning `ImageLinkVerdict` | `ImageStagePort` with `admit_image_link` / `verify_image_link` (`image_stage.rs:257-271`); `ImageStageHook::on_image_stage` dispatches on `ImageStagePhase::New/Replay` (`:244-287`); outcomes `Accepted/Existing` (`:223-224`) | Persistence implements the two port methods; the phase comes from `decide_admission` (`admit.rs:175-203`), never from link-row presence |
| `ImageAdmissionError` with `LinkConflict/LinkStale/DualMedia/...` and no `Unknown` | Actual `FinanceError` (`service.rs:34-125`): `ImageStageBinding(refusal)`, `ImageStageRefusal`, `ImageStageConflict`, `ImageStageMissingLink`, `ImageStageUnavailable`/`ImageStageUncertain`/`ImageStageCommitUnknown` (each keeps SQLSTATE, never a payload) | Map only these variants; commit failure is `ImageStageCommitUnknown`: reread before retry, never release or resend |
| `FinanceService<F, M, S, I>` with a fourth generic and `admit_image_stage(&mut self, command, media)` | `admit_image_stage<P: ImageStagePort>(&mut self, command: &ImageStageAdmission, port: &P)` on the existing `FinanceService<F, M, S>` (`image_stage.rs:359-377`); `ImageStageAdmission { admit, binding }` (`:350-357`) | Keep the existing service shape; pass `&impl ImageStagePort` per call. No constructor migration |
| `StageAction::{Deferred, Admit, VerifyExisting}` carrying `df_assets::Admission` | No such type exists. Preflight is `check_against` (`image_stage.rs:146-174`): shared ids must equal the `AdmitCommand`, and a legacy `media` beside a binding is `LegacyMediaPresent` refused | Classification-without-catalog-row is section 2 admission policy (deferred catalog admission), not a finance enum |
| `finalize_image_in` / `admit_generation_in` attributed to existing crates | Neither function exists in `df-persistence` or `df-assets` | Section 6 specifies them as new composition-owned functions with exact verification order; root assigns the owning crate at dispatch |
| Stage schema with `finance_* text` columns, one finance child per stage, no FKs | `finance_ids` are 16-byte values (`finance_ids.rs:64-131`); retries create several finance attempts per stage | Section 4 separates logical stage from dispatch attempts with `bytea` ids, composite FKs, and nullable hosted finance tuples |
| `ImageCostProjection` in `df-tools` referencing `PortRoute`/`MeteredUnits` | `port.rs` is the cost owner's public surface; `start` returns a consumed `PortCall` with no idempotency input (`port.rs:306-317`); `df-imagery` cannot depend on `df-tools` vocabulary | Section 8 keeps a neutral projection contract in `df-imagery` and specifies the two pinned-id functions the `df-tools` cost owner must add |
| `submit(caller, ...)` facade with `Caller`, `InterestCapability`, `ImageStatus` readiness | The request/read/watch/cancel boundary is owned by the in-flight brief (`service-boundary-brief.md`): `Principal`/`OperationHandle`, `authorize_request`/`authorize_owner`/`authorize_stored`, no `Ready` until a validated result binding exists | Section 2 consumes that brief; this document defines no competing facade, token, or capability |
| Mark-sent guarded on supplier id returned | `plan_mark_sent` commits `DispatchPrepared -> Sent` before any byte leaves (`lifecycle.rs:183-222`); `dispatcher_instance` is a finance input, not a post-egress receipt | Section 5 commits the finance Sent marker before transport launch and persists supplier ids in a separate idempotent update |

## 1. Finance admission hook (repairs SC-A2-2)

`FinanceService::admit` owns its transaction (`admit.rs:101-125`): lock
(`finance.lock_flow`), authorized payers, `decide_admission`, `commit_plan`,
then fence-then-media in D25 order (`admit.rs:245-265`), then commit. The
image path reuses that core through `admit_with_hook` (`admit.rs:108-242`):
the request digest is folded with `image_stage_request_digest`
(`admit.rs:148-154`; domain `df-finance/image-stage-v1` over the binding
plus caller payload digest, `image_stage.rs:176-200`), the asset is stamped
into the single `provider_attempt` insert (`admit.rs:206-211`,
`stamp_media_asset`), and the hook runs on both the new and the replayed
decision (`admit.rs:177-201` for replay, `:215-230` for new) under the
`port_deadline_ms` bound (`bounded_hook`, `image_stage.rs:322-332`).

Preflight runs before any transaction opens (`check_against`,
`image_stage.rs:146-174`): binding tenant/campaign/session/operation/
attempt/effect must equal the `AdmitCommand` fields, and any legacy
`command.media` beside a binding is `BindingRefusal::LegacyMediaPresent`.
A2's `DualMedia`/`IdentityMismatch` names are replaced by these actual
variants.

New admission runs the finance plan, fence, stamp, then
`ImageStagePhase::New`; replay runs `ImageStagePhase::Replay` against the
stored receipt and returns `AlreadyAdmitted` without a fresh send permit,
readable after later owner retirement. The persistence port implementation
admits or verifies the catalog linkage in the same transaction on `New`
(insert-or-exact-match keyed by the finance operation, as the
`image_stage.rs` test linkage table demonstrates in
`crates/df-finance/tests/image_stage.rs:145-275`) and performs a read-only
exact verification on `Replay`, returning `ImageStageMissingLink` when no
linkage exists rather than admitting one. Failure mapping uses only actual
variants: `ImageStageBinding` (preflight), `ImageStageRefusal(StageRefusal)`
(catalog refusal: `InFlight/Ready/AudienceEmpty/QuotaFull/Other`),
`ImageStageConflict` (stored linkage names another mapping),
`ImageStageMissingLink`, `ImageStageUnavailable`/`ImageStageUncertain`
(SQLSTATE kept, payloads redacted), `ImageStageCommitUnknown` (reread before
any retry; never release, never resend).

Lock order for one paid image-stage admission: finance rows first, then the
O1 fence (existing D25 order), then quota/catalog rows in `QuotaScope`
order, then the imagery linkage row. No provider I/O under any lock.
Durable intent acceptance (journal `accept_in` alone) is a separate earlier
transaction that takes no finance or catalog lock and dispatches nothing.
The immutable catalog/finance tuple pinned in the receipt digest is the
binding plus payload digest; retry and claim values that may change
(dispatch sequence, claim token/generation, supplier ids) are never part
of any admission fingerprint.

## 2. Submission lifecycle (repairs SC-A2-6; facade owned elsewhere)

The public request/read/watch/cancel boundary is the in-flight
`service-boundary-brief.md`, implemented by another worker now. That brief
is authoritative for the facade; this composition consumes it and defines
no competing surface:

- One `ImageService` over the accepted journal and canonical codec, with
  `request` / `read` / `watch` / `cancel` taking the trusted host
  `Principal` plus the existing `ImageRequest` (`request.rs:300-313`) or a
  stored `OperationHandle` (owner plus operation identity; never a
  caller-chosen job/interest, never a Bearer [REDACTED].
- `authorize_request` (submit permission for owner/purpose/audience and
  reference pins), `authorize_owner` (coarse scoped gate before journal
  access), `authorize_stored` (`Read`/`Cancel` against the actual persisted
  job and interest). Reference `audience`/`digest` claims are verified
  against asset authority; verification is not permission to fetch remote
  bytes. Reauthorize stored audience/owner rather than trusting the
  caller's original request.
- `PreparedOnly`, `Replay`, and `CacheOnly` are strictly lookup-only: no
  `accept`, no interest, no freeze, no job or stage ids, no finance or
  local dispatch. A miss returns a typed miss and writes nothing. A hit is
  impossible to certify today: no accepted durable job-to-Ready-asset
  mapping exists yet, so compatible prepared-result reuse stays explicitly
  outstanding (section 9) rather than served by an always-miss adapter or
  an intent-hash catalog key. Request replay on an existing receipt
  reuses the retained snapshots/deadline and returns the original view;
  retired/expired receipts stay readable after authorization.
- Ready facts derive only from the validated result binding in section 6,
  never from request fields. The journal persists no requesting principal,
  so member-level cancel inside a shared run needs the deliberate
  principal-binding migration the brief names; this document does not
  promise it.

Live execution ordering inside this composition, once the boundary admits
genuinely new `Live` demand:

1. Accept durable intent first (`accept_in`: receipt plus `Attached`
   interest on a `Planning` job). Paid classification precedes the final
   recipe freeze: classification is itself an admitted stage, its result
   is durably attached to the job, and only then is the output route
   frozen (`freeze_in` with the resolved `MediaRequestKey` plus exact
   recipe snapshot).
2. Allocate the stable publication identity (asset plus publication
   attempt) on the job before classification, so the classification stage
   links to the retained identity without claiming an asset row.
3. Freeze once; the first output-producing stage admits the catalog row
   through the section 1 hook; later stages verify the existing linkage.
   Catalog `InFlight`/`Ready` on the frozen key transfers or joins only
   authorized compatible interests and never overwrites or recharges the
   retained classification liability.
4. Local-only stages (`local_transform`, reference preparation without a
   paid check) carry durable job/stage and publication identity with zero
   supplier attempt, reservation, or settlement, per the transformations
   plan.

Config and dispatch linearization follows the brief: the service holds the
effective `ImageConfig` revision; a disable (`off`, `cache-only`,
purpose/backend off) sets a pending gate, fences every dispatch owner
(scheduler, local workers, hosted egress), and each owner reports its
applied revision before the ack. An update not yet propagated to hosted
egress returns `AppliedLocal`, never a false global success. Once
acknowledged, no queued stage starts under the old snapshot; started
stages drain without beginning another effect stage. Queue/deadline/
concurrency changes never invalidate content keys; effective output/
prompt/route/acceptance changes do.

## 3. Catalog admission mapping and account drafts (repairs SC-A2-3)

The canonical path is the async `CatalogStore`/`ObjectStore` lifecycle:
`record_staged` -> `mark_validated` -> `intend_variant` -> immutable
`put_new` -> `mark_variant_written` -> `finalize`. The generic
`publication::publish` receipt is not used and never substitutes for
`FinalizeResult::Ready`. Every `Admission` field
(`catalog_store.rs:33-54`) maps as below; the service builds the payload
and the finance transaction admits it through `admit_in`
(`media_catalog_sql/admission.rs:14-80`), which enforces the owner gate,
locks quota first, and treats a live dedupe conflict as a join/reuse
answer rather than an overwrite.

| `Admission` field | Run scope | Account draft scope |
| --- | --- | --- |
| `asset` / `attempt` | Pre-allocated job publication ids (attempt doubles as the catalog attempt) | Same |
| `kind` | `AssetKind::Image` | Same |
| `provenance` | `Generated` | `Generated` (pixel-derived sources keep lineage; no silent relabel) |
| `charge_scope` | `QuotaScope::Campaign(campaign)` | `QuotaScope::Account(account)` |
| `origin_account` | Caller account | Caller account (required) |
| `campaign` | `Some(campaign)` | `None` |
| `audience` | Requested meet; `Withheld` refused before the catalog is touched (`catalog_service.rs:241-247`) | `Audience::OwnerPrivate(account)` only |
| `dedupe_scope` | Campaign dedupe | `DedupeScope::account(account)` |
| `request_key` | `Some(MediaRequestKey::derive_recipe)` over the frozen recipe | Same; owner scope string is the draft account |
| `retention` | `CampaignMedia` (or `Intermediate` for pure prep) | `Personal` |
| `reserve_bytes` | `CatalogConfig.maximum_bytes.for_kind(Image)` | Same kind bound under the account quota |
| `byte_limit` | `quota.byte_limit(Campaign)` | `quota.byte_limit(Account)` |
| `in_flight_upload_limit` | `None` (generated input uses no upload count) | `None` |

Catalog refusal mapping: `AudienceEmpty` (refused before catalog),
`InFlight { attempt }` (join or transfer only an authorized compatible
interest; already-incurred classification liability is retained, never
recharged or overwritten), `Ready { asset }` (reuse receipt, no new
dispatch), `QuotaFull` (typed refusal; storage quota is not a spend cap),
`UploadLimit` (unreachable for generation), `OwnerUnsupported` (a
mismatched or legacy no-campaign row, never bypassed by fabricating a
campaign or template).

Account-draft storage has materially landed since A2 and A2's refusal
citations are stale for well-formed drafts: `admission_owner` accepts
`campaign=None` exactly when `provenance==Generated`, `kind==Image`,
`charge_scope==Account(account)==origin_account`,
`audience==OwnerPrivate(account)`, `dedupe_scope==account`,
`request_key` present, `retention==Personal`
(`catalog_store.rs:80-131`), and `record_owner` rechecks the same rule on
recovery (`:134-154`). Object bytes for drafts resolve under
`users/<hex>/draft-assets/<asset>/<attempt>/<write>/<variant>`
(`object_store.rs:202-221`) through the `StorageOwner::AccountDraft` arm
(`object_store.rs:233-241`). `advance` (`catalog_service.rs:601-603`) and
`publish` (`:738-740`) return `UnsupportedOwner` only for rows whose
revalidated owner is `None`, so well-formed drafts now flow through staging,
validation, variant writes, and finalization with the owner threaded
through `write_variant`/`finalize_with_rewrites`. What remains on the
storage side is delivery authorization: key existence must not grant
delivery; serving a draft byte requires the existing account-capability
check independent of campaign membership (lane L2 verifies and pins the
call sites).

## 4. Hosted draft funding: governing sources and the genuine gap

Hosted funding today is session- and campaign-scoped only.
`crates/df-finance/src/host_funding.rs` implements a host-only pilot:
dated versioned consent (`record_consent`, `:83-125`), atomic open plus
allocation through the existing posting boundary (`open`, `:157-290`),
release of only unreserved credit on close (`close`, `:293-343`); consent
itself moves no money (`host_funding.rs:1-2`). Every consent, session, and
allocation names a session, campaign, tenant, and payer account. The
creator draft owner is `ImageOwner::Draft { account }` (`request.rs:76-79`)
with no session, no campaign, and no payer binding, so no existing
`host_funding` path can fund it. The pure admission decision likewise
prices a tenant/campaign/session/operation tuple
(`admission.rs:66-76`); there is no account-scoped hosted admission, price
authority, quota/consent check, idempotency namespace, or account fence in
the planner, locks, or settlement.

`planning/commerce-service.md` (proposed, not live) constrains any future
answer rather than supplying one: no customer grant above their explicit
quoted cap, optional video disabled until funded approval (`:29-33`), and
unknown spend keeps its worst-case liability reserved (`:29-31`). The
creator consumer (`consumer-migration.md:34-96`) submits drafts with
`Purpose::CreatorPortrait`, `ImageOwner::Draft`, and explicit regeneration
nonces, but names no payer.

Decision for this contract: hosted Draft generation stays refused with a
typed funding refusal until root approves an account admission/funding
policy naming the actual payer source (host grant extension, account
wallet, or trial grant), price authority, accounting scope,
quotas/consent, idempotency namespace, and account authority fence, plus
the corresponding finance planner/lock/settlement path. Until then, local
Draft work uses the same durable attempt identity with record-only cost
projection and never a fabricated campaign, wallet, or spend entry. The
single genuinely unresolved user-policy item is who pays for
account-private hosted generation; everything around it (storage layout,
admission mapping, attempt identity, record-only metering) is specified
below and in the lanes. This document claims no resolution of that item.

## 5. Durable stage vs dispatch attempt state (repairs SC-A2-1, SC-A2-4)

One logical stage produces output at most once per dispatch, but retries
create several supplier attempts. The schema therefore separates the
logical stage (one row per job stage, ownable and claimable) from dispatch
attempts (one row per send, each with a stable neutral id and an optional
hosted finance tuple). Pure local stages have no supplier or finance
attempt at all: their dispatch row carries the local checkpoint instead.

Proposed imagery-schema tables (new migration; root numbers it after
`0038` and registers it beside `imagery_store.rs:21-24`):

```sql
CREATE TABLE imagery.image_stage (
    job_id bytea NOT NULL REFERENCES imagery.image_job (job_id),
    stage_id bytea NOT NULL,
    owner_scope bytea NOT NULL,
    claim_generation bigint NOT NULL DEFAULT 0,
    job_revision bigint NOT NULL,
    stage_kind text NOT NULL,
    state text NOT NULL,
    publication_asset bytea,
    publication_attempt bytea,
    local_accounting boolean NOT NULL DEFAULT false,
    created_at_ms bigint NOT NULL,
    updated_at_ms bigint NOT NULL,
    PRIMARY KEY (job_id, stage_id),
    CONSTRAINT image_stage_kind CHECK (stage_kind IN
      ('classification','generation','edit','extraction','inspection',
       'correction','local_transform')),
    CONSTRAINT image_stage_state CHECK (state IN
      ('planned','admitted','dispatch_prepared','sent','succeeded',
       'failed','unknown','released')),
    CONSTRAINT image_stage_ids CHECK
      (octet_length(job_id) = 16 AND octet_length(stage_id) = 16
       AND octet_length(owner_scope) BETWEEN 1 AND 128
       AND (publication_asset IS NULL OR octet_length(publication_asset) = 16)
       AND (publication_attempt IS NULL OR octet_length(publication_attempt) = 16))
);
CREATE TABLE imagery.image_dispatch_attempt (
    dispatch_id bytea NOT NULL PRIMARY KEY,
    job_id bytea NOT NULL,
    stage_id bytea NOT NULL,
    finance_attempt bytea,
    finance_operation bytea,
    finance_effect bytea,
    dispatch_seq integer NOT NULL,
    supplier_job_id text,
    usage_revision integer NOT NULL DEFAULT 0,
    book_call_id text,
    settlement text NOT NULL DEFAULT 'held',
    claim_token bytea NOT NULL,
    claim_generation bigint NOT NULL,
    lease_until_ms bigint NOT NULL,
    created_at_ms bigint NOT NULL,
    updated_at_ms bigint NOT NULL,
    FOREIGN KEY (job_id, stage_id) REFERENCES imagery.image_stage (job_id, stage_id),
    CONSTRAINT image_dispatch_attempt_seq UNIQUE (job_id, stage_id, dispatch_seq),
    CONSTRAINT image_dispatch_finance_op UNIQUE (finance_operation),
    CONSTRAINT image_dispatch_attempt_ids CHECK
      (octet_length(dispatch_id) = 16
       AND (finance_attempt IS NULL OR octet_length(finance_attempt) = 16)
       AND (finance_operation IS NULL OR octet_length(finance_operation) = 16)
       AND (finance_effect IS NULL OR octet_length(finance_effect) = 16)
       AND (finance_attempt IS NULL) = (finance_operation IS NULL)
       AND (finance_operation IS NULL) = (finance_effect IS NULL)
       AND dispatch_seq >= 0 AND usage_revision >= 0
       AND octet_length(claim_token) = 16),
    CONSTRAINT image_dispatch_settlement CHECK (settlement IN
      ('held','settled','unusable_settled','released','unknown_outstanding'))
);
CREATE TABLE imagery.image_job_result (
    job_id bytea PRIMARY KEY REFERENCES imagery.image_job (job_id),
    stage_id bytea NOT NULL,
    publication_asset bytea NOT NULL,
    publication_attempt bytea NOT NULL,
    mime text NOT NULL,
    width_px integer,
    height_px integer,
    sha256 bytea NOT NULL,
    quality text NOT NULL,
    recipe_digest bytea NOT NULL,
    execution_revision bigint NOT NULL DEFAULT 1,
    ready_at_ms bigint NOT NULL,
    CONSTRAINT image_job_result_ids CHECK
      (octet_length(publication_asset) = 16
       AND octet_length(publication_attempt) = 16
       AND octet_length(sha256) = 32 AND octet_length(recipe_digest) = 32)
);
CREATE INDEX image_stage_job_state ON imagery.image_stage (job_id, state);
```

No `finance_* text` identities: every id column is `bytea` with an
`octet_length` check, matching the 16-byte `finance_ids` and the
32-byte digests. The finance tuple is all-or-nothing NULL (local-only
dispatches) or all present (hosted dispatches). Publication allocation
(asset plus attempt) lives on the stage row, constrained to one binding;
retries add dispatch rows with increasing `dispatch_seq` and immutable
retry lineage, never a second stage row. Claim ownership is a durable
`(claim_token, claim_generation, lease_until_ms)` triple: a stage claim
mints a fresh 16-byte token, increments generation on every supersede,
and a write carrying an older token or generation is a lost lease that
stops without deleting. CAS guards are `(stage, claim_generation)` for
claims and `(finance attempt, revision)` inside finance; the job revision
is checked at admit against the frozen job. Flags that may change
(dispatch sequence, claim values, supplier ids, usage revision) stay out
of every admission fingerprint.

Hosted-financed dispatch ordering (every row is updated, never deleted;
CAS on the guards above):

| Step | Finance call | Stage/dispatch write | Outcome |
| --- | --- | --- | --- |
| Admit stage | Section 1 hook in the admission transaction | `planned` -> `admitted`, bind publication ids, `local_accounting=false` | `Admitted` |
| Prepare dispatch | `prepare_dispatch` (`Reserved -> DispatchPrepared`) | `admitted` -> `dispatch_prepared` with fresh claim token | Dispatchable |
| Commit send marker | `mark_attempt_sent` (`DispatchPrepared -> Sent`) with the dispatcher instance id | `dispatch_prepared` -> `sent` on the dispatch row | In flight; this finance transition is the durable Sent marker and commits before any transport launch |
| Launch egress | None (transport reads the committed marker) | Record launch against `dispatch_id` | Bytes in flight |
| Supplier receipt | None | Idempotent `supplier_job_id` update on the same dispatch row | Correlated, not re-sent |
| Bytes complete | None | Staged, validated, variants written; `sent` -> `succeeded` candidate | Await finalization |
| Finalize `Ready` | Section 6 atomic publication | Job-result row inserted | `Ready` |
| Output unusable | `settle_unusable` with an `UnusableOutcome` | `unusable_settled`; customer never charged | Company loss recorded |
| Cancel before send | `release` (only when proven unsent; a sent attempt answers `CannotReleaseSent`) | `admitted`/`dispatch_prepared` -> `released` | Reservation released |
| Timeout/ambiguous | Neither settle nor release; liability retained | `sent` -> `unknown` | Reconcile; never auto-resend |
| Fresh retry | New child finance attempt | New dispatch row, old rows preserved | Old liability intact |

Crash between the Sent marker and egress (or between egress and the
supplier-id update) leaves finance `Sent` with a dispatch row lacking a
supplier id: recovery rereads the finance attempt state and the dispatch
row, reconciles by the stable `dispatch_id` idempotency key against the
provider (or awaits the late result), and never blindly dispatches again.
A reservation released pre-send via `release` is conclusive, never
uncertain liability: `CannotReleaseSent` proves sent attempts cannot take
that path. A dropped `PortCall` recording `Unknown` at its maxima
(`port.rs:181-186`) is local bookkeeping for an actually-sent call; it
never releases finance state and never authorizes a resend.

Local-accounted dispatch (same tables, `local_accounting=true`, finance
tuple NULL): admit records the attempt identity with record-only policy;
cancel before start writes a terminal local row; crash recovery reruns
from the durable source without paid regeneration; `unknown` never
triggers paid source regeneration and never mints a finance row.

## 6. Atomic catalog/job publication (repairs SC-A2-7)

A2's `finalize_image_in` never existed. The composition needs one new
transaction-taking publication function (root assigns the owning crate at
dispatch; it runs wherever the catalog PG statements live and must share
the caller's transaction so the finance-owned admission composition in
section 1 can call it). Specified behavior, no invented API beyond these
names:

- Inputs: the open transaction, the `HeldStep` (asset, attempt, owner
  claim token), the `FinalizeRequest` (winners, MIME, dimensions, byte
  limit), the resolved job binding (job, stage, publication asset plus
  attempt, owner scope, claim generation, frozen recipe digest), and
  `now_ms`.
- Under the fixed lock order (quota row, then asset row, then job/stage
  rows, all READ COMMITTED), verify before writing anything: the
  `HeldStep` names the asset's current attempt; the job's stage binding
  still names this publication asset plus attempt; the stage owner scope
  and claim generation match; the frozen recipe digest equals the
  supplied one; every named winner is `written` in the catalog with a
  manifest the caller can read.
- Derive the result byte facts (MIME, dimensions, digest, alpha facts,
  source binding, quality evidence) from the validated catalog writes,
  never from caller strings: a supplied metadata field that conflicts
  with a winner manifest rejects the publication before commit.
- Only on `FinalizeResult::Ready` insert the `image_job_result` row in
  the same transaction and return `Recorded`. On `WinnerMissing`,
  `LeaseLost`, `Superseded`, or `Rejected`, write no job-result row and
  return `NotRecorded` with the catalog outcome. No external byte writes
  occur inside the transaction: bytes are immutable in the object store
  (`put_new` plus `mark_variant_written`) before this call.
- Replay compares the full stored binding (job, stage, publication
  asset/attempt, recipe digest, winner manifests) and rereads it as
  already recorded; a mismatched replay is a conflict that writes
  nothing. Successful publication increments the job's execution
  revision atomically; row-level foreign keys from the result row to the
  job and from the dispatch rows to the stage make a forged binding
  (result attached to an arbitrary job) a constraint violation, not a
  silent misattach. `LeaseLost`/`Superseded` stop without deleting;
  `Rejected` takes the terminal outbox path.

Outbox discipline: only terminal transitions write the settlement outbox
row (`catalog_store.rs` rejection path). The publication function never
writes an outbox row for a non-terminal outcome, and section 7 routes
consumed outbox entries by their job binding so one stage settlement is
never mistaken for whole-job terminal publication.

## 7. Publication recovery adapter (repairs SC-A2-5)

Today one catalog attempt is one finance attempt: `GenerationRequest.
attempt` is the attempt F minted at admission (`catalog_service.rs:74`),
and the sweeper reads that same attempt from F (`catalog_sweep.rs:170-176`)
and asks the dispatcher to refetch it (`:239-242`). Per-stage finance
child attempts break that 1:1 identity: one publication (asset plus
publication attempt, which remains the catalog attempt) now maps to one
job/stage binding and to N finance attempts across retries. The existing
consumers (`FinanceAttempts::attempt_state` /
`cancel_unclaimed_effect`, `Dispatcher::request_refetch`, and the catalog
settlement outbox readers in `pending_outbox`/`outbox_entry`,
`catalog_store.rs:529-539`) must not treat one child as the parent and
must never mint dummy finance rows for local work. The bridge:

- `resolve_publication(asset, publication_attempt)` reads the stage
  binding and answers the owning job/stage plus the current output
  dispatch (highest `dispatch_seq` not superseded). Unknown pairs answer
  not-found; nothing is created.
- Hosted status aggregation reads the catalog row state, the dispatch
  row settlement, and the current dispatch's finance attempt state, in
  that order, and reports the conjunction: a finance `Sent` with a
  catalog `Uploading` row is in flight, not failed; a catalog terminal
  state with finance still `held` is a recovery obligation, not a second
  settlement.
- Cancellation routes by proof: only a dispatch proven unsent (no Sent
  marker on its finance attempt, no supplier id) may call finance
  `release` plus catalog reject; anything sent or uncertain takes the
  catalog-side stop path and leaves finance state to reconciliation.
  `cancel_unclaimed_effect` is therefore answered per the current
  dispatch's finance attempt, never per the publication attempt as if it
  were a finance row.
- Refetch selection persists the chosen dispatch for
  `Dispatcher::request_refetch`: the adapter records which dispatch the
  refetch was requested for, so a late result for a superseded dispatch
  is discarded (stale completion rejection) instead of finalizing the
  wrong attempt.
- Local-only publications resolve to local checkpoints (staged object
  keys, validation verdicts), never to finance rows; local recovery
  replays from those checkpoints without paid regeneration.
- Outbox routing consumes catalog `pending_outbox` entries and delivers
  each to its bound job: a stage settlement updates its dispatch row and
  the job's execution revision, and only a terminal publication outcome
  for the job's bound output stage may mark the job result. One child's
  settlement is never whole-job terminal publication.

The adapter is specified here and implemented once (root assigns the
owner; it sits beside the finance bridge, not inside `df-assets`, and it
performs no provider I/O). Until it lands, the sweeper's finance and
dispatcher calls keep their current 1:1 meaning and multi-attempt jobs
must not run hosted dispatch.

## 8. Same-attempt cost correction and upsert bridge (repairs SC-A2-8)

The existing cost engine stays the only bookkeeping: `MeteringPort::
start` opens a call, the consumed `PortCall` settles once with
`Reported`/`Estimated` units or drops to record `Unknown` at maxima, and
`not_billed` records nothing for work refused before billable action
(`port.rs:150-230,306-318`). Calling `start` again creates a new book
call identity, so the composition must map one durable dispatch attempt
plus usage revision to exactly one existing cost entry. The neutral
projection contract lives in `df-imagery` (consumer-owned; `df-imagery`
never imports `df-tools` vocabulary), and the `df-tools` cost owner adds
exactly two pinned-id functions beside `start` (new code in the cost
owner, specified here):

```rust
// df-imagery owned (neutral; no df-tools types).
pub trait ImageCostProjection: Send + Sync {
    fn call_id_for(&self, dispatch: &[u8; 16]) -> String;
    fn record_reported(&self, call_id: &str, route: &CostRoute, units: &[CostUnits]);
    fn record_unpriced(&self, call_id: &str, route: &str, detail: &str);
}

// `CostRoute`/`CostUnits` are plain data defined beside the trait
// (provider/model/cause/detail strings plus category/quantity pairs),
// not df-tools vocabulary.

// df-tools gameplay cost owner additions (specified; implemented in lane L6).
// start_pinned(call_id, route): idempotent per call_id; replay while the
//   call is open returns the same open call, replay after settle answers
//   AlreadySettled instead of opening a second charge identity.
// correct(call_id, source, units): idempotent per (call_id, usage_revision);
//   applies the newer supplier report as a correction event on the same
//   call identity, never a new generation charge.
```

Rules: the dispatch row stores `book_call_id` plus `usage_revision`;
recovery rereads the row before starting anything, so a crash between
supplier report and book write corrects rather than double-counts. Usage
revisions are idempotent updates on the one stable call id, never another
`start`. Local record-only accounting uses the same path with an
unpriced/local cause and never touches finance settlement. The finance
rows stay authoritative and the local book is a projection that never
overrides them.

Settlement mapping (finance calls from `held.rs`, projection effects):

| Evidence | Finance call | Book projection |
| --- | --- | --- |
| Usable, supplier-reported usage | `settle(attempt, units, Reported)` | Settle the pinned call `Reported` |
| Usable, counted/estimated usage | `settle(attempt, units, Estimated)` | Settle the pinned call `Estimated` |
| Billed but unusable output | `settle_unusable` with an `UnusableOutcome` (`InvalidOutput`/`FailedNoOutput`/`Duplicate`/`Superseded`); customer never charged, supplier cost is company loss | `settle_unusable`, `usable=false` on the same call id |
| Proven unsent (refused submit, cancel before dispatch) | `release(attempt, reason)`; a sent attempt cannot be released | `not_billed`; nothing recorded |
| Ambiguous timeout / lost receipt | Neither settle nor release; liability retained, reconciliation continues; never an automatic resend | `Unknown` recorded only for an actually-sent call at its maxima |
| Codex envelope, priced lines | Envelope attempt settled on reported lines | Priced lines settle; subscription units listed separately |
| Codex envelope, no tariff line | No invented dollar figure (`settlement.rs` admits only `Reported`/`Estimated`) | `unpriced=true`, amount from priced lines only, never zeroed |
| Missing usage evidence | Outstanding, never zero | `Unknown`, counted as unresolved exposure |

## 9. Resolved-recipe sharing, prepared lookup, bounded scheduling

Recipe identity is `MediaRequestKey::derive_recipe`
(`media_request_key.rs:74-133`) over operation kind, purpose, owner
scope, backend profile, model, quality, rating, dimensions, output
format, alpha policy, template/route/acceptance/executor revisions,
source binding, ordered reference digests, prompt, pipeline bytes, the
regeneration nonce, and the audience, under schema `df-imagery/3`
(`:146`). Queue position, deadlines, and concurrency limits are execution
state and stay out of the key. The freeze payload is exactly this recipe;
the journal already stores the resolved key plus recipe snapshot per job
(`FreezeCommand`, `journal.rs:379-389`), and replay compares them whole.

Prepared and compatible-reuse lookup, once the section 6 result binding
exists: resolve the frozen recipe to its key, `lookup` the catalog dedupe
scope, and join only an authorized compatible interest whose owner,
sharing identity, intent, config, and resolved key all match exactly.
`Freshness::Regenerate` carries a new nonce and can never join old work.
`FailedUnknown`/unknown outcomes are never reused. Until the result
binding lane lands, compatible prepared-result reuse stays outstanding
and the boundary brief's always-miss behavior stands; this document
installs no interim cache adapter and derives no catalog key from an
intent hash.

Bounded durable scheduling (lane L4/L7, not this document): the journal
does not enforce `max_pending_jobs`/`max_pending_bytes`, and a
process-local counter is not capacity proof (boundary brief). The
scheduler lane adds a durable PG queue with per-purpose in-flight bounds
(the commerce-service proposal orients at four in-flight image/video jobs
globally, one video job per campaign, `commerce-service.md:29-33`),
durable stage claims with the section 5 token/generation/lease triple,
config-revision fencing at claim time, and recovery that reclaims only
expired leases. Scheduler, local worker, and hosted egress each report
their applied config revision before any disable ack.

## 10. Preserved full target: consumers, inputs, providers, cost

Every consumer in `consumer-migration.md` stays in scope and keeps its
contract; this composition replaces only the dispatch seam
(`ImageService::request`/`read` with the purpose, operation, owner, and
freshness each consumer already names), never the domain intent:

- Creator chain (`consumer-migration.md:34-96`): `Purpose::
  CreatorPortrait` with `Generate`, `ImageOwner::Draft`, `ReuseCompatible`
  (explicit `Regenerate` nonce for a new portrait), `ExecutionMode::Live`.
  Classification is the admitted paid stage with its own durable attempt
  attached before freeze; refusal or uncertain classification admits no
  output stage. Grant-gated `accepted`/`approved_reference` reads stay
  private to the draft account audience.
- Canonical worker and mural (`:140-158`): `CanonicalPerson`/
  `CanonicalRoom` `Generate` with `ImageOwner::Run` and frozen source
  binding; mural keeps its versioned roster binding.
- Plates (direct, state-ahead, item, expression, combat), clip frames,
  reference/mask/thumbnail prep, and dev fixtures (`:160-279`): each
  keeps its purpose/role, source pins, and poll/reconnect/retry semantics;
  bypass constructors (`NativeImageEditProvider`,
  `NativeFalImageProvider`, `NativeBflImageProvider`,
  `SystemOneChoiceProvider`, `ImageRoute::OpenAi/Flux`) migrate to
  service submission in the documented order, one commit per consumer.

All inputs resolve from the frozen recipe: text prompts (`TextPrompt`,
`request.rs:238-275`, bounded by injected `RequestLimits`), image pins
(`SourceRef` asset/digest/audience, `:203-211`, claims verified at
submit), resolution (`OutputProfile::Provider` with generated plus
delivered sizes, `Local` with delivered only, `output.rs:202-241`;
`Generate`/`Edit` require `Provider`, `Transform` requires `Local`,
`request.rs:326-350`), and ordered local transforms (`TransformSource::
Image/Canvas/Coverage` over claimed pins, `transform.rs:22-71`, pixel
bounds against `MAXIMUM_OUTPUT_PIXELS`, EXIF-first sRGB-linear working
space per the transformations plan). Private Codex/OpenAI/fal/BFL
execution stays behind the provider seam with per-route credentials;
a Codex envelope settles on its reported priced lines with subscription
units listed separately (section 8). Actual cost tracking is the section
8 projection over the existing engine; no new ledger, wallet, price
table, or spend cap appears anywhere in this design.

## 11. Exact bounded implementation lanes (repairs SC-A2-9 dispatch)

Root dispatches these in dependency order; each lane names its exclusive
files and its owner, and no lane edits shared registrations, manifests,
migrations harnesses, or another lane's paths (root integrates):

- L1 Campaign finance image-admission port impl. Files: new
  `df-persistence` imagery-stages module implementing `ImageStagePort::
  admit_image_link`/`verify_image_link` over `admit_in` plus the section
  5 stage insert (same transaction), plus PG tests. `df-finance` is
  untouched (the hook, digest, stamp, and error variants already exist).
  Proves atomic rollback and replay verification on real PG.
- L2 Account draft delivery authorization. Files: `crates/df-assets`
  delivery/call sites only (storage and admission already land per
  section 3). Serves draft bytes only on the account capability,
  independent of campaign membership; wrong-account denial tests on real
  PG plus native object store.
- L3 Service boundary. In progress by another worker under
  `service-boundary-brief.md`. This composition consumes it; no lane
  here duplicates its facade.
- L4 Stage/dispatch schema, recovery adapter, bounded scheduler. Files:
  new imagery migration (section 5 SQL) plus transaction-taking stage
  functions, the section 7 adapter (beside the finance bridge, owner
  assigned by root), and the durable queue/claim scheduler. Real PG
  crash/replay/lease-expiry tests; fake transport only.
- L5 Atomic publication and result binding. The section 6 function plus
  `image_job_result` writes, winner-manifest equality, forged-binding
  rollback tests on real PG.
- L6 Cost bridge. Neutral `ImageCostProjection` in `df-imagery` plus the
  two pinned-id functions in the `df-tools` cost owner; same-attempt
  correction and crash-reread tests with fake clock, no live calls.
- L7 Execution lane. Classification-to-freeze-to-dispatch over Codex/
  OpenAI/fal/BFL fake transports with mark-sent-before-egress ordering,
  supplier-id idempotent update, unknown recovery without resend, and
  the section 10 consumers migrated in the documented order. No live
  provider calls except a separately scoped, deadline- and spend-bound
  task.

## 12. Failure-focused tests (required once implemented)

Each test fails before the fix and passes after; fakes only for
providers, clocks, and ids; real PostgreSQL on owned scratch databases
for every persistence claim (fake persistence alone is not evidence);
no sleeps; no live calls; no test cited as passed until root runs it:

- Admission hook (real PG finance schema plus a transaction-taking test
  linkage table as in `crates/df-finance/tests/image_stage.rs`):
  rollback removes both receipt and linkage; new/replay linkage mismatch
  is `Conflict`; changed binding field changes the digest; replay after
  owner retirement verifies without a fresh send permit; commit
  uncertainty surfaces `Unknown` and a reread resolves it.
- Scope/fence races: concurrent same-operation accepts leave one job,
  one receipt, one interest; incompatible recipe freeze has one winner;
  stale revision refused; wrong-owner status/watch returns `NotFound`.
- Send ordering (real PG plus fake transport with a crash seam): kill
  between the Sent marker and egress, and between egress and the
  supplier-id update; assert no second dispatch, liability retained, and
  reconciliation by stable `dispatch_id`. Supplier id recorded twice is
  one update.
- Cancel/release proof: cancel before dispatch releases only proven
  unsent holds; a sent attempt answers `CannotReleaseSent`; a dropped
  book call recording `Unknown` never releases finance state.
- Publication: job-result row written only on `Ready`; `WinnerMissing`/
  `LeaseLost`/`Superseded`/`Rejected` write none; conflicting winner
  metadata rejects; forged job binding violates a constraint;
  superseded-dispatch late results are discarded.
- Recovery adapter: one publication resolves to the current dispatch;
  outbox entries route to bound jobs; local publications resolve to
  checkpoints with no finance rows; refetch selection persists its
  dispatch.
- Cost: one dispatch plus two usage revisions is one book call with a
  correction, never two charges; unpriced Codex usage stays visible and
  priced lines are never zeroed; crash between report and write
  corrects on reread.
- Authorization: deny before any mutation; forged owner/source claims
  refused; stored-read denial on replay; prepared/replay/cache-only
  misses write no rows and mint no ids.

## 13. Resolved, remaining, and retained evidence

Resolved by this contract: finance-owned admission with the actual
hook/port/digest/replay/commit semantics (SC-A2-2); intent-first ordering
with paid classification before freeze and lookup-only
`PreparedOnly`/`Replay`/`CacheOnly` (SC-A2-6 read path, SC2); exact
catalog `Admission` mapping over the canonical async path with the
landed account-draft storage rule (SC-A2-3 storage half, SC3); no
competing facade (SC-A2-6 facade half, owned by the boundary worker);
logical-stage vs dispatch-attempt schema with bytea types, FKs, CAS
claim/lease, publication allocation, and the full hosted/local
transition tables (SC-A2-4); mark-sent-before-egress with supplier-id
update and uncertain-send recovery (SC-A2-1); the publication recovery
adapter with parent-to-child routing and outbox discipline (SC-A2-5);
atomic catalog/job publication with winner-manifest equality and
forged-binding constraints (SC-A2-7); the neutral projection contract
with the same-attempt correction/upsert bridge and full settlement
mapping (SC-A2-8); real-PG plus fake-provider crash/replay/
authorization evidence with no unrun-pass claims (SC-A2-9).

Remaining and honestly open: the account funding user-policy decision
(section 4: payer source, price authority, accounting scope,
quotas/consent, idempotency namespace, account fence, and the
corresponding planner/lock/settlement path); the boundary worker's
facade completion (L3); draft delivery-authorization call-site proof
(L2); and the code in lanes L1 and L4-L7 with their section 12 evidence.
Hosted multi-attempt dispatch, compatible prepared-result reuse, and paid
creator drafts stay blocked on those items. Nothing in this document
restates them as solved, and no further generic planning round is
requested: root dispatches the lanes above.

Retained evidence: no runtime or test result is claimed in this document
(shell disabled for this session). Prior handoffs
(`service-contract-a2-handoff.md` and the wave2 review JSON files) stay
in `artifacts/tmp/imagery-20261027/wave2/` as current-wave active
evidence per ADR 0002/0003; no deletion was performed and none is
proposed here. Root freezes the exact commit, runs the checks, and owns
integration.
