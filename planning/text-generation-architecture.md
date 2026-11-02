# Unified text-generation architecture

Date: 2026-10-28
Status: Contract frozen and partly implemented; see "Frozen contract and implementation status". Consumer migration is in progress.
Task: TEXT-SURFACE-PLAN-20261028-A1-SOL
Source: Desktop `dungeonfluxGame`, HEAD `6d0ce07e1d775d8bff9f569b23826a0503e13035`,
with concurrent uncommitted imagery work. See [surface map](text-generation-surface-map.md).

## Decision and scope

Consolidate every language-model request behind `df-ai::TextService`. Expand existing `df-ai`
rather than adding a competing `df-text` service. Game features, media judgments, support,
developer commands and test/evaluation tools use one consumer API. They select a purpose,
permitted context and output contract. Private backends own endpoint/wire/SDK/CLI details.
Feature owners keep prompt construction and semantic validation. TextService is proposed,
not an existing implementation.

The actual backend set is OpenAI Responses, OpenAI Chat Completions multimodal, TypeSafe/SystemOne
Jev (including batched support questions), Codex CLI text and fixtures/replay. No Anthropic,
Gemini or Ollama text implementation was found. New backends must implement the same contract.
Image/video synthesis, STT, TTS, embeddings, pixel transforms and deterministic text templates
keep their owners. Language-model prompt drafting or inspection still uses TextService;
deterministic prompt assembly does not become a model request merely because it returns text.

## Governing contracts and current gaps

Read [Coding style](coding-style.md), [Subsystem architecture](subsystem-architecture.md),
[Subsystem interfaces](subsystem-interfaces.md) AI/provider/media ports,
[Narrative engine](narrative-engine.md), [Interaction engine](interaction-engine.md) grounded
speech, [Runtime reliability](runtime-reliability.md), [Runtime directors](runtime-directors.md),
[Observability](observability.md), [Pricing and costs](pricing-and-costs.md),
[Commerce service](commerce-service.md), [Service operations](service-operations.md),
[Image architecture](image-generation-architecture.md) and
[Imagery consumer migration](imagery-implementation/consumer-migration.md).

Source-backed reasons for consolidation (paths below are relative to the repository):

- `crates/df-ai/src/lib.rs:1` exports admission, expression and lookup only. Preserve their
  checked expression, complete-record and authorized read contracts; they are not a live service.
- `crates/df-tools/src/gameplay/storyteller.rs:674,778,1082,1403,1596` owns expression, cache/flights,
  retries, dispatch, validation and accounting. OpenAI reaches `registered_text_provider` at 1619;
  CLI reaches `codex_cli::text` at 1459. Its binding at 1678 uses fixed local session/run/revision
  labels, not a current campaign authority fence.
- `approach_model.rs:457,901,915` owns a second wrapper, SystemOne. Creator startup constructs it
  separately (`creator_portrait_startup.rs:117`); dev photo-check constructs a plate judge during
  a paid diagnostic request (`dev_control/reads.rs:55`). These paths also need the shared API.
- `crates/df-hosted/src/support_ingest/providers.rs:199,219` directly sends SystemOne and Responses.
  Support needs batched questions, which the current single-q1 `SystemOneChoiceProvider` cannot
  represent. Include classification and generative drafts without automatic actions.
- `crates/df-providers/src/text_response.rs:443` sends `stream:false`. TextEvent currently means
  final candidate/completion (`df-provider-api/src/provider.rs:59`), not incremental tokens.
- `courier_ai.rs:401`, reached from `actor.rs:855`, is an actual prepared-only df-ai consumer;
  it does not prove that live Storyteller goes through df-ai.
- `crates/df-server/src/serving.rs:274` is campaign-runtime recovery/health startup. A registry
  string naming gameplay is not a wired executor. Actual playable startup is
  `crates/df-tools/src/main.rs:46 -> gameplay::serve` (`gameplay.rs:1178`). Hosted support is
  independently wired by `crates/df-hosted/src/runtime.rs:207`.

Unless a full prefix is shown, gameplay citations mean `crates/df-tools/src/gameplay/`.

Local and hosted financial policies differ. Coding style section 8 prohibits adding local spend
caps; commerce/service-operations require existing hosted grants/admission. Preserve both:
no new local wallet/cap and no bypass of hosted entitlement/liability authority. A diagnostic
counter is never spending authorization. This plan does not select new commercial prices.

## Ownership and dependencies

`df-ai` owns one public TextService facade, resolved configuration, queue/in-flight coordination,
complete-result cache/replay orchestration, attempt lifecycle, structural qualification and
backend selection. It owns no canonical gameplay, account identities, payment policy, asset
publication or automatic tools.

Keep current platform-neutral admission/expression/lookup modules. Place the native service and
private backends behind native feature/target boundaries so pure callers and WASM checks cannot
acquire reqwest, credentials or process execution. Backends reuse df-providers' bounded native
HTTP through minimal transport plumbing. Retire public executable text factories/constructors
there after migration. No `df-providers -> df-ai` cycle; neutral schema/value types may live in
`df-provider-api`, whose existence grants no execution authority.

Consumer-owned ports connect current authority, durable attempts/recordings, financial admission
and existing cost projection. `df-persistence` implements stores; composition supplies adapters.
Existing local SpendLedger/MeteringPort remains the local book; df-finance remains hosted money
authority. Never create a second ledger or import df-tools into df-ai. Temporary Storyteller and
ApproachModel facades delegate to TextService and lose their cache/dispatch/retry/metering owners.

### Interoperability with the concurrent imagery service

Imagery may depend on df-ai or receive a narrow supplied text-inspection port whose composition
adapter calls TextService. The dependency direction is imagery -> text; df-ai never imports
`df-imagery` to obtain a cost, image-part or process type. Use neutral df-provider-api usage/parts/
port contracts where shared value types are needed, mapped explicitly at each boundary. Existing
`df-provider-api/Cargo.toml` lists df-ai as a dev-dependency for contract tests, not a normal
production reverse dependency; do not misdiagnose that source-only test edge as a production cycle.

During T4, freeze the classification/multimodal inspection port with the imagery integration owner.
Creator classification, photo/visible-feature judgment, staging and head location call TextService;
pixel generation/editing/publication stays ImageService-owned. Remove or quarantine the old direct
judge handles at cutover rather than retaining a raw-provider escape behind the new port. A disabled
or unsupported text dependency returns a typed stage outcome, with explicit existing image quality
policy; no silent direct Chat call or replacement generation route.

Neutral usage observations feed the same current book/finance authority. Do not reuse an
image-specific stage envelope as a text charge or make df-ai depend on imagery's cost adapter.
The root composition maps the neutral text usage/admission port into the existing local
MeteringPort/SpendLedger and hosted finance. Each actual inspection attempt has one booking owner;
image orchestration can link the child attempt without recording a second supplier charge.

Codex image and text adapters remain separate semantic/event modules. Extract suitable bounded
process-lifecycle primitives from the existing private runners into neutral native infrastructure
in df-providers (or a coordinator-approved existing native infrastructure owner) before both need
them. This is process I/O/timeout/tree-ownership plumbing, with no prompt, tool policy, retry,
provider choice, queue or money authority. Composition resolves common credential/account policy;
each service supplies its request's frozen allowed capabilities and owns exactly one process
supervisor tree. No df-ai -> df-imagery import, parallel supervision of one child, duplicate
restart/retry logic or second CLI ledger. T0 freezes that primitive interface with both owners;
T2 migrates text only and T4 changes only agreed inspection integration points, preserving ongoing
image implementation work.

Hosted credentials/sockets stay in the native egress dispatcher under Service operations. A
campaign service uses its authorized dispatcher port, not a key loaded in the campaign runtime.
Local composition may use an in-process executor with the same attempt/recovery contract. One
logical API does not mean one global process or one provider client per campaign.

## Consumer API to freeze before implementation

The names below are proposed contracts. Freeze concrete enums, newtypes and serialization in T0.
No public provider-JSON passthrough or raw adapter handle.

| Contract | Required guarantee |
| --- | --- |
| `submit(context, request)` | Ready, Joined(existing job), Accepted(new job), Deferred or typed refusal; same operation can be looked up after a lost response. No game commit. |
| `qualify(candidate_id, basis, verdict)` | Authorized feature acceptance/rejection bound to exact candidate digest, job/generation, source/audience/validator basis; prerequisite to reusable/public publication. Stale verdict cannot evict or qualify a newer candidate. |
| `status/subscribe/cancel/shutdown` | Bounded progress/result pages, explicit owner cancellation, drain and unresolved outcome; no detached worker. |
| `TextRequest` | Purpose/profile, trusted scope, real source basis, operation/job/generation, permitted input, locale, output contract, validation profile, mode and deadline. |
| `PermittedInput` | Ordered role-tagged text and authorized image parts with digest/size/MIME/detail/provenance/rights; no arbitrary remote URL dereference. |
| `OutputContract` | Bounded prose slots, typed structured JSON schema, single/batched closed choices, grounded clauses or typed tool proposals; schema revision binds identity. |
| `TextOutcome` | Qualified candidate with basis/provenance/usage; explicit partial-batch acceptance; typed unavailable/unsupported/disabled/refused/invalid/stale/cancelled/deadline/Unknown. |
| `ToolProposal` | Stable proposal id, registered tool/schema version, checked arguments and basis/dependencies; no executable capability. |
| `Qualification` | Structural acceptance distinct from feature semantic pending/passed/rejected/uncertain and named compatibility degradation. Model-critic PASS is never authority. |
| `AttemptObservation` | Stable attempt/parent identity, sent/unsent/unknown disposition, provider ids and exact usage evidence; no private payload in Debug. |

Register stable purpose ids for every surface in the companion map, including imagery and
support. Test composition may register fixture purposes. A feature submits one logical request,
then applies existing domain checks before committing or speaking. TextService owns structural
checks; the feature supplies semantic qualification. Never add keyword meaning fallback.

Semantic qualification is an explicit two-stage handshake. TextService retains a structurally
checked candidate privately and supplies it only to the authorized feature validation port (or
returns a private owner-only candidate handle). The feature returns Accepted, Rejected or Uncertain
through `qualify`, including exact candidate digest/id, operation/job/generation and current
source/audience/validator basis. Revalidate that basis atomically before marking reusable/Ready
or emitting captions/TTS; unqualified candidates are never shared cache hits. The feature's
separate canonical commit does not become a TextService state write. Lost/expired qualification
leaves explicit pending/rejected output and original paid liability; it never automatically
regenerates the request.

Preserve caller-rejection invalidation with the same handshake and one cache owner. Current
`place_creation.rs:1241,1464,1474` calls `Storyteller::forget` (`storyteller.rs:750`) after feature
rejection. Map that to rejection/invalidation of the exact retained candidate and compatible cache
version, not eviction by request hash alone. A late rejection for candidate A cannot evict a newer
accepted candidate B. Qualification/rejection receipts are idempotent; semantic changes invalidate
by validator/source revision, and any retry is the same centrally bounded correction policy.
T0 freezes this handshake; T2/T5 acceptance must test caller rejection, repeated cache lookup,
qualification loss and stale acceptance/rejection races without a second feature cache.

Image inputs are required today by photo quality, identity, staging and head-location calls.
Use authorized bytes/asset handles and bind image digest/detail policy. Imagery retains identity
selection, verdict interpretation and segmentation. TextService neither generates images nor
edits pixels. Add audio/video parts only for a real selected consumer, not because STT/TTS/video
providers exist. Unsupported part/schema/backend pairs fail before dispatch.

## Context, authority and output

Bind tenant/account/campaign as applicable, session/run/recovery epoch, operation/job/generation,
relevant state/knowledge/content/source/rights revisions, audience/projection, prompt/schema/
validator/locale revisions, mode, purpose and frozen profile. Support binds mailbox/case ingestion
identity and permission instead of fake game ids; account drafts bind account/draft revision.
OperationContext carries correlation, never authorization.

Feature projection happens before generation. Speaker-private policy and listener-safe expression
are distinct contexts; public prompts never receive hidden truth or raw secret ids to hide later.
Existing withheld-word checks supplement projection. Preserve paired-hidden-state invariance for
prompts/output/cache/streams/media cues/replay. Reauthorize before stored reads, egress, delivery
and canonical commit; honor deletion/revocation tombstones on reuse.

Bound context with explicit source selection, byte/token estimates and reserved output/reasoning.
Record source digests and included/omitted counts. Never silently drop necessary grounding or
rules outcomes: return ContextTooLarge or run an explicitly supported summarization purpose.
Summaries cite canonical records and stay derived. No models in pure director/rules functions
or view rendering.

Completions carry the admitted basis. Only the current owning feature rechecks and accepts them.
Late output can settle its original supplier charge but cannot commit canon, reveal facts, mutate
inventory or release a newer busy flag. The session owner alone commits game changes through
existing source/rules validation. Support drafts remain human-review suggestions and cannot
send replies, issue refunds, create identity grants or act on repositories. TextService never
executes tool proposals.

Preserve df-ai::expression and df-interaction grounded speech. Schema/closed fields/lengths/
locale and transport consistency are central checks; rules legality, claims/entities, knowledge,
consent, reveals and state transitions stay feature-owned. A model critic is an explicitly metered
child request under its own purpose, with a finite nonrecursive validation-stage bound.

## Streaming and final validation

Expose bounded lifecycle events even for buffered backends: Admitted, Queued, Started, Progress,
Qualified/Rejected/Failed and UsageCorrection. Candidate token fragments stay internal. Public
subscriptions receive permitted progress and approved output, never raw tokens to captions/TTS.

Default current sensitive/structured features to whole-response buffering. Exact identity,
EOF/sequence/JSON/schema/final semantic checks precede publication. JSON prefix, HTTP 200,
provider completed marker and process exit are insufficient; trailing bytes/events reject.
Preserve partial-adventure behavior by accepting only independently complete validated slots or
groups, with explicit rejected slots and no unfinished-slot publication.

Stable-clause streaming is a separately qualified profile: each clause has stable sequence/hash,
approved claim/slot scope, locale and current audience basis and cannot need retraction. Until
qualified, only progress streams publicly. Revocation/cancel/slow-subscriber overflow stops new
delivery. Report supplier completion separately from client delivery. SSE fixtures cover UTF-8
and JSON across chunks, reasoning/tool events, duplicated/out-of-order events, truncation,
disconnect, malformed termination and late completion. Backend streaming support is not claimed
until its event adapter and fixtures exist.

## One lifecycle, cache and recovery

Move current Storyteller cache/flights into TextService. Reuse the resource hub through a supplied
coordination port or replace its text-specific coordination once; never let both own retries,
reservation or the same flight. Domain stores such as accepted story dossiers are derived feature
data, not alternate provider-flight caches.

Canonical request identity includes ordered permitted input/reference digests, scope and relevant
source/audience revisions, purpose, prompt/validator/schema/locale versions, provider account,
backend/model/effective parameters and mode. Preserve `storyteller.rs:1900` key coverage and
strengthen authority binding. Operation-id conflict checks differ from content reuse. No cross-
tenant reuse by equal strings. Bound cache eviction; never evict durable Unknown history as cache.

1. Validate current scope, operation and capabilities. Prepared/replay uses complete compatible
   records only; missing/redacted/stale returns a typed outcome without network/CLI.
2. Deduplicate/join, then queue with count/bytes/age/deadline bounds, foreground capacity and a
   bounded background share. Defer/reject explicitly on overflow.
3. Persist intent, frozen config and stable attempt; obtain existing local/hosted admission once.
   Cache hits reserve no generation cost, but retained paid critics are distinct child attempts.
4. Confirm durable dispatch marker/permit and irreversible journal where required before possible
   send; check current grant/owner fence. Use provider idempotency only where actually supported.
5. Execute one bounded attempt; persist observations/usage/candidate receipt even after owner
   turnover. Validate structure/basis and return candidate for feature acceptance.
6. Publish complete reusable output atomically with source/access checks; partial/failed output
   cannot replace good records. Persist final receipt and one settlement/correction projection.
7. Restart inspects durable attempt state first. PossiblySent/Unknown never becomes unsent when
   process/lease/waiter/deadline expires. Reconcile by supported provider identity or retain
   Unknown and prepared fallback. Never auto-resend a possibly paid non-idempotent request.

Cancel before dispatch releases only proven-unsent admission. After dispatch cancellation is best
effort plus publication/game fences; usage remains payable. Caller disconnect stops waiting,
not durable accepted work. Own all tasks/process trees; shutdown drains or records unresolved
attempts. Deadline covers queue/connect/headers/body/parse/validation and child critics. Bound
stdout/stderr, subscribers and queued bytes, not only semaphore concurrency.

Centralize finite retry/correction policy. Storyteller currently permits one validation correction
(`express_online`, 1139 onward), and cached prose can invoke fresh SRD review (`judge_values`,1319).
Preserve or explicitly version those semantics. Refusal is terminal; never reword or switch routes
around it. A known completed invalid result may produce a separately billed child correction under
its deadline. Unknown cannot be retransmitted as that correction. Remove nested feature retry
owners or represent their finite stages centrally.

Local execution needs durable minimal attempts too; a success-cache file cannot prevent resend
after crash during send. This adds no money cap. Hosted recovery follows the existing finance
and nonregressing journal contract. Reuse neutral primitives with imagery only where contracts
actually align; image-specific identity/publication remains df-imagery-owned.

## Usage, costs and configuration

One authoritative supplier attempt feeds the existing financial/cost projection. Reuse local
`cost/port.rs` pinned start/correction (331,349), SpendLedger and hosted `df-finance/src/admit.rs:101`
through composition adapters. Move no balance/pricing policy into TextService. Legacy LedgerMeter,
Storyteller begin_call, support MeteredTransport and the new service must not all book one call.
Select exactly one booking owner at each migration cut.

Record attempt/parent/purpose/provider/account/model, frozen price revision and evidence class.
Preserve input/output/total, cached input/cache write and reasoning output tokens. Subset token
categories are not additional totals. `df-providers/src/text_response.rs:195,635` already retains
these; Jev/Chat currently report less. Missing fields remain unknown/estimated, not reported zero.
Retain overruns rather than clamping to quotes. Pin dispatch rate version; append idempotent usage
corrections against original attempt/rate. Use integer micro-dollars.

CLI subscription usage, tokens and API dollar charges are distinct. Capture observed token/cache/
reasoning quantities and account class; absent tariff is unpriced/unknown, not zero-dollar API.
Existing local policy allows unpriced generation; hosted required quote policy still governs.
Rejected-but-billed text, child critics, corrections, late/cancelled/unused output and Unknown
remain attributable. No second text totals store or diagnostic counter as admission authority.

Resolve a typed snapshot at submission: documented defaults < file < environment < launch/CLI <
authorized runtime patch. Within merged config, purpose settings override provider defaults and
feature selection chooses registered purpose/profile. Mandatory account/security/disable policy
constrains all overlays; purpose settings cannot re-enable a disabled provider. Store each field's
source provenance. No environment reads per line. Migrate launcher mappings explicitly
(`config/dungeonflux.example.toml`, `crates/df-launcher/src/settings.rs`).

Expose master/provider/feature/purpose enable and mode, default backend/profile, credential/account
reference, endpoint/model/version, supported reasoning/temperature, strict output/schema/tool
proposal settings, context/output bounds, deadline, concurrency/queue, quality/validator/correction
policy, cache/replay/fixture sources, retention and price/report policy. Validate all fields.
Unsupported parameters are errors, never silently ignored or routed elsewhere. Quality cannot
weaken required privacy/source/rules checks. Runtime patches increment generation; admitted
work keeps its snapshot unless explicitly canceled/reissued. Emergency deny can stop new sends/
delivery without removing existing liability.

| Current setting | Unified meaning |
| --- | --- |
| `DF_GAMEPLAY_STORYTELLER`, launcher `story.storyteller` | Narrative expression gate |
| `DF_GAMEPLAY_TEXT_MODEL`, `DF_GAMEPLAY_TEXT_DEADLINE_SECONDS`, `DF_GAMEPLAY_TEXT_CACHE` | Narrative model/deadline and one cache adapter with explicit key migration |
| `OPENAI_API_KEY` | Composition credential reference, never consumer payload or logged value |
| `DF_JEV_API_KEY`, `DF_JEV_BASE_URL`, `DF_GAMEPLAY_JEV` | Jev account/endpoint and semantic gate; current jev-latest becomes documented profile default |
| `DF_GAMEPLAY_MEMORY`, launcher story memory | Embedding/summary feature dependency; summaries still enter TextService |
| `DF_GAMEPLAY_CODEX_MODE` and executable/workdir/home/hash settings | CLI backend/launch configuration; replace fixed-version/model restrictions with configurable capability preflight; fixtures qualify adapter/release evidence, not permitted development model choices |
| `DF_GAMEPLAY_DISABLE_OPENAI_API` | Direct API exclusion independent from ChatGPT CLI; unavailable auxiliary stage is reported |
| Pictures/plates/creator gates and dedicated creator credentials | Media purpose/account dependencies; preserve classification/rating behavior |
| `DF_SUPPORT_MODELS_APPROVED`, `DF_SUPPORT_JEV_*`, `DF_SUPPORT_DRAFT_*` | Existing support model permission, separate judgment/draft profiles/account bindings |
| `DF_GAMEPLAY_CONTENT_RATING` | Versioned feature prompt/validator policy |

Codex CLI is a normal selectable text backend with platform process support and configurable model.
Validate actual stdin/structured-output/event/usage/auth/cancel capabilities, not one exact CLI
version/hash, empty home, ChatGPT-only account class or unrelated repository ancestry. Normal
configured accounts and operator process policy govern it. Retain the no-side-effect constraints
needed for text-only execution; text cannot invoke image generation. Use arguments/stdin, owned
process trees and hidden Windows helpers. Record version/adapter revision and reject actual
unsupported capability. Do not copy image-tool permissions into text merely because both invoke
Codex. Tool results stay proposals. Share suitable process lifecycle primitives through neutral
native infrastructure while retaining separate text/image event adapters.

### Development flexibility contract (R3)

Development must use the same API without incidental model, path, endpoint or fixture bans.
Named profiles are operator-defined data, not a hardcoded approved-model list. Profile names and credential references also accept valid Unicode/spaces; environment aliases
follow host OS syntax. Layered overrides resolve one effective configuration with per-field
provenance and a redacted
inspection/diff before activation. Purpose registration selects a typed contract and validation
owner; adding a profile or model does not require a new consumer API or raw-provider escape.

| Adjustable surface | Development guarantee |
| --- | --- |
| Backend/profile/model | Select registered backends and arbitrary provider-valid model ids; no baked model-name whitelist. New typed backend implementations extend the registry behind TextService. |
| Endpoint/account | Configure full provider base endpoint and credential reference, including explicit local/private/loopback routes. Hosted egress policy remains separately enforced; never accept an untrusted player URL as operator configuration. |
| CLI invocation | Configure executable, working directory, account/config profile and model. Preserve valid Unicode/spaces in paths and supported identifiers using native paths/argument arrays/stdin, without shell splitting, ASCII-only checks or fixed directory ancestry rules. |
| Provider options | Versioned backend-specific typed options with registered schemas extend common options. Capability negotiation validates fields before dispatch; this is not arbitrary request JSON, header or tool-execution passthrough. |
| Resource envelope | Configure context/input/output tokens and bytes, slot/batch counts, image size/count, queue/concurrency/memory/process-output bounds, deadlines, cache and fixture limits. Defaults are documented starting values, not hardcoded development ceilings. |
| Modes and inputs | Text plus authorized image inputs, structured/choice/tool-proposal output, buffered or qualified streaming, local/loopback, scripted fixtures, replay and fault injection all enter TextService. Select only capabilities the chosen backend implements. |
| Gates | Global/backend/purpose enable-disable and existing feature gates are inspectable. A narrower profile cannot override an applicable global/backend deny; fixture/offline profiles remain available without enabling a live provider. |

Distinguish three limits in preflight diagnostics: actual backend/protocol capability; configured
product/security policy; configured resource envelope. Return typed `UnsupportedCapability`,
`UnsupportedOption`, `PolicyDenied` or `LimitExceeded` with the safe field, effective bound and
configuration source. Never silently substitute model/provider, drop an option, truncate context
or downgrade output. Negotiated provider maxima, schema/type representability and host OS limits
are real constraints, reported explicitly; adjustable profile limits can be raised within them.
Existing code constants (for example storyteller byte/slot/deadline and CLI version/model checks)
are migration inputs, not new API allowlists. Keep bounded owned work while allowing larger finite
operator-selected development envelopes; no universal hardcoded cap merely for test convenience.

Capability declarations may come from adapter evidence, supported introspection or explicitly
supplied local fixture/configuration. Record that provenance and report unknown/unverified support
honestly. Engineering qualification evidence governs confidence and release claims, not a hidden
ban on configuring another compatible model/CLI version in development. A real unsupported CLI
protocol fails typed preflight; neither this plan nor an operator model string proves installed
CLI support. Creator consent, audience/source authorization, secret handling and hosted finance
remain product authority, independent of provider capability negotiation.

T0 documents each field's activation class: `NextAdmission`, immediate gate denial, or
`RestartRequired` for an implementation that cannot reload that field safely. Validate an entire
reload/patch atomically, retain the last good snapshot on failure, and show which fields require
restart. New jobs capture the selected profile/config revision; existing jobs retain their
admitted settings unless explicitly canceled/reissued. A changed credential reference applies
through current account/key policy, not by rewriting an admitted attempt or losing its liability.

Offline/fixture/replay profiles need no real credentials or hosted balance. Local paid development
uses the existing local accounting policy without a new hosted wallet requirement. Both preserve
usage evidence, pinned-rate corrections where priced and conservative Unknown recovery for actual
possible sends; fault injection cannot manufacture a refund or erase an attempt. No-live profiles
cannot fall through to network/CLI on a miss. Normal hosted paid dispatch still requires current
account/campaign admission. These are composition policies through the same API, not bypasses.

R3 implementation acceptance uses offline fakes to change profiles/model ids/endpoints/options,
Unicode/spaced CLI paths and finite bounds without a rebuild; inspect merged provenance; toggle
global/backend/purpose gates; test reload rollback/restart-required reporting; exercise text/image,
structured/streaming/fixture/replay/fault cases; and assert typed unsupported/policy/limit outcomes
with zero silent substitution. Retain qualification/invalidation, privacy and cost tests. No live
provider or installed CLI capability was tested for this amendment.

An illustrative future file configuration (not accepted by current code) can select CLI text,
keep Jev for judgments and disable speculative wording without changing consumers:

```toml
[text]
enabled = true
mode = "live"
[text.profiles.npc_reply]
backend = "codex-cli"
model = "operator-selected-compatible-model"
[text.profiles.closed_judgment]
backend = "systemone"
model = "jev-latest"
[text.purposes.speculative_narration]
enabled = false
```

The T0 schema must document exact field names and validation; this example defines intent rather
than a second configuration parser. File, environment, launch and runtime controls resolve the
same fields and expose redacted source provenance.

## Observability and development/test controls

Logging refinement (2026-10-28): [Shared generation logging](generation-logging.md) governs the typed event envelope, event/severity ownership, one audit/OTEL bridge, asynchronous context, redaction, finite capture/loss and read-only causal review. Its source-gap table distinguishes current string/span diagnostics from proposed typed TextService logs; neutral df-observe types never depend back on text/audio services.

Emit one lifecycle per real job/attempt, linked waiter/delivery records, distinct structural/semantic qualification and correction evidence. Preserve enabled logs without sampled-trace dependence; never log raw output, prompts, stderr or private-content hashes. T0-T6 below include logging gates; no logging implementation or fault acceptance ran for this amendment.

Use df-observe OTEL and existing audit path. Correlate safe scope, session/run/operation/job/attempt/
parent/trace and build/config/prompt/schema/validator/source revisions. Emit queue/admission/send/
response/validation/publication/cancel/stale/Unknown/reconcile/cache/replay outcomes with durations
and evidence class. Measure queued bytes/age, active work, first-approved-output latency, rejection/
correction, unknown usage/cost and prepared hits. No private prompts, hidden facts, player text,
image bytes, credentials, raw stderr or arbitrary URLs in default logs; no high-cardinality metrics.

Expose authorized `dev text` inspection of redacted effective config, jobs/attempts, provenance,
validation decisions and existing usage/cost linkage. Invalidate/regenerate/fixture/fault/config
patch commands retain current operator permission and operation ownership. A read-style HTTP
photo-check that spends still submits an admitted TextService operation.

Fixtures/replay are API execution profiles with supplied fakes and no credentials/live fallback.
Support recorded wire responses, loopback HTTP, scripted chunks, fake executables and controlled
source/clock/dice. Tests may exercise every valid purpose/model/payload/schema; do not hard-code
fixture wording, request count, provider version or production-only endpoint constraints.
Player input cannot select operator fixture/fault registration. Existing paid tests stay ignored;
this planning task authorizes no paid/live calls.

## Buildable migration and implementation ownership

Freeze public contracts before parallel implementation. Each phase is a bounded separate
submission after dependencies. Coordinator assigns nonoverlapping edit areas and integrates
shared files sequentially. This task dispatches no workers and implements no source changes.

| Phase | Cut and dependencies | Buildable acceptance |
| --- | --- | --- |
| T0 | Freeze API, parts, validator split, attempt/admission/cost ports, registry, config and capabilities; centrally own df-ai/provider-api/manifests. | Existing pure df-ai callers compile; source/financial ownership fixed; no second execution API.  Logging gate: Freeze shared logging envelope/vocabulary/owner and redaction types. |
| T1 | Native lifecycle/config/store ports with fixture backend; local durable attempt store and existing cost adapter. T0. | Zero-network submit/join/cancel/replay/Unknown/restart fixtures reachable from composition.  Logging gate: Required logs remain queryable with no sampled trace; one producer/reader fixture. |
| T2 | Private Responses and Codex text backends; storyteller facade loses dispatch/cache/flights/bookings. T1. | Existing slot/partial/withheld/SRD/cost behavior, one booking, fake CLI fixtures and explicit failure semantics.  Logging gate: One audit/log owner for Responses/CLI; normalized process and Unknown events. |
| T3 | Private SystemOne single/batched; ApproachModel/creator/support adapters. T1 + T2: support drafts require the Responses backend. | All Jev purposes and support five-question classification plus drafts use API; support approval/semantic checks retained.  Logging gate: Jev/batch/feature verdicts preserve captured context and safe stage links. |
| T4 | Private multimodal Chat and plate/head adapters; dev photo-check. T1; coordinate frozen imagery classification/judge ports. | Same authorized image parts/detail/schema/verdict/cost; image service uses one TextService port; no image implementation overlap.  Logging gate: Inspection child attempts join image parent without duplicate logs/charges. |
| T5 | Migrate all surface-map groups to real purpose/basis/profile; prepared courier, fakes, evaluation/smoke tools. T2-T4 facades remain only until callers move. | Reachable feature tests preserve behavior; no new cache/retry/spend owner; prepared courier remains keyless/no-live.  Logging gate: All listed consumers emit typed context; redaction and coalesced-waiter cases. |
| T6 | Wire actual local/hosted roots; retire raw support HTTP/public factories/old CLI wrappers/facades; strict guard. All prior phases. | Zero legacy bypass entries, required native/WASM/lint and integrated offline recovery/cost evidence.  Logging gate: Shared logging fault/redaction/cardinality/read-only gates pass; no duplicate bridge. |

T5 can split by feature after T0 only. Storyteller, ApproachModel, imagery composition, support,
shared schemas/manifests remain explicit edit conflicts. Re-scan current source before each cut
because concurrent imagery work may move call sites. Do not claim hosted gameplay wiring from
composition-registry strings or a tested helper without a caller.

## Frozen contract and implementation status

Updated 2026-11-01. This replaces the first-slice notes. It records what exists, so later phases
build on it rather than on the proposal text above. Proposal text that differs is superseded here.

### Contract (T0, frozen for text)

All in `df-ai`, platform-neutral, with no provider, credential or process dependency.

- `TextRequest { purpose, profile, instructions, facts, output_name, output_schema, deadline }`.
  `TextPurpose` is a closed enum (`Storyteller`, `DevProbe`; features add variants when they
  adopt the service). `TextProfile` is an operator-defined label for backend, model and mode; it
  is part of the request fingerprint, so one operation under two profiles conflicts rather than
  replaying another model's answer. Typed image and audio parts are not in the contract yet; they
  arrive with T4 for a real consumer.
- `TextBackend::complete(&TextRequest) -> TextReceipt { outcome, usage }`.
  `TextOutcome` is `Answered | Unavailable(reason) | Deadline | Invalid | Stale | Unknown`.
  `Unknown` means the request may have reached the supplier and no result is known; it is never
  settled as a failure and never resent. `Stale` is produced by the service, never by a backend.
- `TextUsage { total_tokens, input/output/cached_input/cache_write/reasoning_output: Option }`.
  Unreported categories stay unknown, never zero. Provider counts convert with `try_from`; an
  overflow drops the usage instead of clamping it.
- `OperationKey { scope, operation }`: scoped labels, so equal operation ids in different scopes
  never share an attempt.
- `DurableTextService<B, S, A, O>` is the only path to a backend (`TextService` alone cannot run a
  request). `submit`, `qualify(key, request, Verdict)`, `invalidate`, `cancel`.
  - `Submission`: `Settled` (reusable: a qualified answer or a non-answer outcome), `Candidate`
    (a structurally valid answer that is **not** a cache hit until the feature qualifies it),
    `Rejected`, `InFlight`, `Unknown`, `Cancelled`.
  - Attempt phases: accepted, dispatched, candidate, settled, rejected, unknown, cancelled. Each
    transition is one compare-and-set on the stored phase. A rejected answer is never regenerated
    under its key. A verdict whose basis moved after dispatch records `Stale` in the same step.
- `TextAuthority::current_basis(key)` is the owner fence. The service asks before it touches the
  store, immediately before dispatch, after the backend answers, and when a verdict arrives.
- `TextAttemptStore` is the persistence port: PostgreSQL (`df-persistence::text_attempts`, schema
  `df_text.text_attempt`, loopback only) and an in-memory fixture store. Store errors are typed
  (`Unreachable`, `TimedOut`, `Rejected`, `Invalid`) and carry their cause label into the log.
- Debug output of answers and candidates is withheld.

### Composition (gameplay process)

- `text_runtime.rs` owns the shared store, a local authority (one basis per run, released at
  shutdown), the observer and the owner token. Callers supply only a backend and request;
  `TextRuntime::service(backend)` builds the durable service. Without a reachable database the
  store is in memory (still one send per operation per process, not across a restart) and the
  `TEXT` start-up line says so.
- Backends are private composition adapters: `CodexTextBackend` (`codex_cli.rs`),
  `ResponsesTextBackend` (`responses_text.rs`) and `SystemOneTextBackend` (`systemone_text.rs`).
  The static guard (`development/check-text-boundary.py`) restricts raw egress to them.
- Cost: callers keep their existing bookings at each cut and feed them from `TextUsage`; the
  service books nothing, so there is exactly one booking owner per migrated call.

### Logging

`text_telemetry.rs` emits one record per call to the shared diagnostic sink, shaped by
[generation logging](generation-logging.md): schema `df.generation/1`, `generation.*` event names
(`request`, `cache`, `coalesced`, `provider_result`, `recovery`, `text_qualification`, `cancel`,
`stale`), closed outcome, reason and send-state labels, operation and scope references, duration
with clock domain. Prompts, answers, credentials and content hashes never appear. Not yet carried:
attempt and parent ids, record ids and config revisions (they need the typed producer path).

### Development control

`dev text`: `list`, `attempt` (sizes and usage, never the answer), `config`, `cancel` (pre-dispatch
only), `prune --days` (finished attempts only; unknown sends are liability history and stay),
`probe`, `qualify`, `invalidate`. A probe runs one scripted fixture request through the real
service under its own purpose and profile, so rows and log records can be checked without a
provider; a run without the telemetry sink refuses to probe.

### Status by phase

| Phase | State |
| --- | --- |
| T0 | Text contract, logging vocabulary, authority and qualification semantics frozen as above. Config layering, purpose registry beyond two purposes and typed media parts are open. |
| T1 | Durable attempts, replay, Unknown recovery, cancel, retention, dev control and logging are built and tested (PostgreSQL scratch server). Open: cost adapter, bounded pool, multi-instance owner leases (a second live instance can mark a live dispatch Unknown, which is conservative), output size bound on stored answers, layered configuration. |
| T2 | Responses and Codex backends exist. Storyteller dispatch migration in progress; its disk cache and flights move into the service in a later cut. |
| T3 | SystemOne backend and ApproachModel seam in progress. Support (df-hosted) batched questions not started. |
| T4 | Multimodal Chat backend, plate/head adapters, photo-check: not started (needs typed image parts). |
| T5, T6 | Remaining surface groups, root wiring, removal of legacy wrappers: not started. |

### Known deviations still open

No tenant-scoped authority beyond the local run; no financial admission port; no queue or lifecycle
event stream; retry and correction policy still lives in features; the request deadline is enforced around every
backend by `DeadlineBackend` in the gameplay composition (`df-ai` has no timer), not inside `df-ai`; `clippy -D warnings` cannot run on
`df-persistence` and `df-tools` while `df-imagery` has unrelated warnings. `df-ai` builds for
`wasm32-unknown-unknown`.

## Boundary guard and acceptance

Implement `development/check-text-boundary.py` with adversarial scanner tests. Cover source/test
roots, scripts/build tools, Cargo dependencies/imports, HTTP/SDK and subprocess execution. Keep
literal endpoints as evidence but tokenize Rust for executable imports/calls/aliases, multiline/
qualified forms and macros. Check process targets/argv/stdin, shell/curl/PowerShell/Python HTTP and
provider SDK imports; recognize aliases and split URL constants. Unaccounted egress code that constructs a dynamic endpoint or executable outside the registered
backend/configuration boundary requires code review. Ordinary custom endpoint/model/CLI path
values supplied through that boundary require no new guard exception or per-change human approval;
preflight applies the selected configuration and capability policy. Endpoint regex alone cannot
enforce this boundary.

Only exact private backend modules and designated shared transport may perform raw text egress.
Shared transport permission grants no arbitrary feature dispatch. Ban executable text re-exports
and migrated consumer imports of NativeTextProvider/registered_text_provider/
SystemOneChoiceProvider/NativePlateJudge/NativeHeadLocator/old Codex process wrappers.
Check native/WASM dependency reachability. Raw-egress fixture harness plumbing may have narrow
structural exceptions identifying its source module/symbol and permitted local transport purpose;
never tests/** or df-providers/** blanket permits. Ordinary same-API fixture datasets, schemas,
cases, fault schedules and configured endpoints/CLI paths need no exception edits. Scanner
occurrence checks constrain raw-egress source code, never runtime request counts or dataset size.
Temporary legacy source exceptions bind exact caller/symbol/removal phase, reject code bypass
increases, and reach zero at T6. Negative fixtures cover aliases, cfg(test), nested modules, scripts, split URLs,
indirect Command, factory re-export and unrelated OAuth/payment/media HTTP false positives.

Required implementation evidence, unperformed by this documentation task:

- Faithful Responses, multimodal Chat and SystemOne single/batched wire fixtures; fake CLI event/
  process fixtures; exact model/params/schema, refusal, malformed JSON/schema/tool output,
  deadline/disconnect/EOF/trailing bytes, chunk bounds and redacted errors.
- Controlled queue/clock, concurrent dedupe, same-id conflict, pre/post-send cancel, waiter loss,
  shutdown, source/audience/rights/generation races and restart at every dispatch/publication/
  settlement cut. Unknown never auto-resends.
- Prepared/replay/courier misses produce zero egress; deleted input remains unavailable; paired
  hidden-state prompts/streams/views are equal; captions/TTS never receive raw candidate tokens.
- Existing hosted account/campaign grant gates and uncapped local bookkeeping; one attempt/one
  usage projection, pinned rates, cached/reasoning subset arithmetic, estimated/unknown usage,
  duplicate/corrected receipts and rejected/late/canceled billable work.
- Existing NPC/social/addressee/merchant/physical/investigation/place/fact/memory/adventure/emotion/
  music/creator/media/support flows, with deterministic source/rules outcomes and malformed/
  disabled fallback cases from the map. No provider failure withholds required rewards.
- Pinned-toolchain fmt and clippy `--all-targets -- -D warnings` for changed crates; native checks
  for df-ai/provider-api/providers and affected roots; filtered df-tools suites; `cargo check -p
  df-tools --target wasm32-unknown-unknown` and affected browser targets. Follow tooling.md;
  never run paid ignored tests without separate scope.
- Actual offline local runtime and hosted support fixture execution; browser captions/reconnect
  and audible matching speech when affected; independent output review. Strict guard zero;
  cost totals match fixed fixture denominator. Record exact source/config/build.

## Remaining gates and limits

Decision: consolidate df-ai with private backends, one lifecycle and one cost projection.
Implementation must freeze public types and source revisions. Actual unresolved policy gates:
existing financial admission/profile for hosted platform-paid support and account-draft text;
hosted/release evidence for CLI account/capability profiles; approved model/locale stable-clause scope if streaming
is selected; rights/retention for private stored input/fixtures. These do not block completing this
audit. Default buffered output, existing approval gates and local/hosted accounting stay explicit
migration policy while launch qualifications are decided.

Counts/citations describe a moving shared tree, not compiled reachability proof. Lexical searches
cannot prove absence of generated/dynamic dispatch; implementation guard needs syntax/dependency
checks plus runtime egress assertions. Private/ignored source and deployment wiring need explicit
follow-up if selected. No application implementation, provider/CLI/network calls, runtime tests,
financial mutations or workflow-database writes were performed.
