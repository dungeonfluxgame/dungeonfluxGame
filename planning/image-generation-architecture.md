# Project-wide image generation: architecture and migration plan

Date: 2026-10-27

Status: Proposed architecture and implementation plan. No runtime changes made.

Source inspected: `C:/Users/mreca/Desktop/dungeonfluxGame`. This document is saved in the chat workspace, separately from that checkout. Findings describe the inspected source, not a passing build or a live deployment. Freeze the exact commit and working-tree changes before implementation.

## Decision

Create **`df-imagery`**, a native Rust crate with one consumer-facing `ImageService` API. Every project-owned raster generation or generative edit must enter through this API, including creator portraits, scene artwork, party murals, combat stills, animation reference frames, development tools, and future image consumers.

The same API also owns local raster transformations through typed **Generate, Edit, and Transform** operations. The [companion transformation plan](image-transformations-architecture.md) specifies resizing, cropping, brightness/color adjustments, masks, composition, encoding, immutable variants, local resource accounting and migration of existing raster helpers.

Display-time swipe, zoom, fades, camera motion and gameplay animation are defined in the [visual motion plan](visual-motion-architecture.md). Existing presentation/UI/render owners animate ready assets; they do not execute image-generation calls per frame or add another asset pipeline.

The supported backend architecture includes **OpenAI API, fal, BFL, and Codex CLI**. Codex CLI is a normal selectable image backend behind the same API, not a separate consumer workflow or a local-only experimental exception. Its implementation is part of this plan.

This is a crate boundary, not a new deployed service. Local play runs it in the existing process. Hosted deployment uses the same implementation behind the existing authorized egress boundary; credentials remain there. Callers never choose deployment placement.

The aim is to make regressions harder to introduce and easier to detect. A crate alone cannot guarantee their absence: the completion criteria include deletion of alternate dispatch paths, enforced dependency/call-site rules, behavior contracts, fault tests, and observed browser acceptance.

## Scope and governing architecture

This proposal explicitly refines `planning/asset-engine.md` and `planning/subsystem-architecture.md`, which currently assign general asset-engine orchestration to `df-media`. `df-media` retains cross-media demand coordination. It delegates all image generation to `df-imagery`; it must not maintain a second image queue, retry loop, or provider registry.

Keep the existing authority boundaries from `planning/runtime-reliability.md`, `planning/storage-architecture.md`, `planning/asset-storage-research.md`, and the finance/commerce plans:

- Gameplay and the session owner decide what has happened and which facts may appear in an image.
- `df-imagery` owns image execution and returns cosmetic results; it never commits game facts, decides rules, or attaches a portrait to a character on its own.
- `df-assets` owns immutable bytes, catalog visibility, audience constraints, retention, and publication.
- The existing finance authority owns paid admission, reservations, provider-attempt liability, settlement, and reconciliation. Storage quota is not a monetary budget.
- Authentication and delivery remain in the authorized API/media gateway. Knowing an image ID or request hash never grants access.
- `df-observe` remains the instrumentation boundary.

Image-specific paid classification, visual checking, and alpha extraction that are required by a generation recipe are centrally orchestrated stages. Independent spatial analysis such as head location and SAM masks may stay outside initially, operating on authorized asset references. They may not generate replacement artwork or start hidden image calls.

Video, speech, music, uploaded-image admission, display-only renderer effects, and full prompt redesign are outside this migration. **Still frames generated for video and reusable raster derivatives are in scope.** Deterministic crops/resizes and other effects are covered by the companion plan, superseding their earlier exclusion.

## Current inventory and migration destinations

| Current owner | Current behavior | Destination |
| --- | --- | --- |
| `gameplay/imagery.rs` | Routes, image requests, cache keys, flights, semaphores, retries, quota breaker, quality checks, files, provenance | Core `df-imagery`; byte publication delegates to `df-assets` |
| `living_scenes/canonical.rs` | Canonical heroes, NPCs, creatures, rooms | Keeps domain intent and reference selection; submits through `ImageService` |
| `living_scenes.rs` | Refreshes, expressions, combat kill stills | Same API; existing presentation behavior retained |
| `living_scenes/plates.rs` and `plates/{direct,state_images,ahead}.rs` | Current/successor scenes, edits, directed plates, item/held-item views | Keep scene selection and relevance; remove image execution ownership |
| `living_scenes/mural.rs` | Party mural | Same API and versioned roster binding |
| `creator_portraits.rs` | Private draft operation, polling, acceptance, generation | Keeps authentication, draft UI state and acceptance; generation workflow delegates |
| `creator_classification.rs` and `creator_portrait_startup.rs` | Image routing classification and dedicated provider configuration | Image-specific classification/route policy in `df-imagery`; bootstrap supplies typed config |
| `character_clips.rs` | Direct OpenAI guard/fallen frame generation, own cache and prior-attempt fence | Frames use the same API; video encoding/generation stays with clips |
| `df-providers/image_edit.rs` | OpenAI generation/edit | Private image backend inside `df-imagery` |
| `df-providers/fal_image.rs` | FLUX Ultra/Kontext; BiRefNet alpha extraction | Private image backend/stage inside `df-imagery` |
| `df-providers/bfl_image.rs` | BFL submit/poll/download | Private image backend inside `df-imagery` |
| `df-providers/fal_media.rs::submit_image` | Generic FLUX Schnell path; current callers found only in tests | Private disabled-by-default backend or delete if no supported use remains; never retain a public bypass |
| `gameplay/codex_cli.rs` and `codex_cli/windows_process.rs` | Existing text-only CLI wrapper and process runner; image generation explicitly disabled | New private `df-imagery/backends/codex_cli.rs` for images; reuse suitable process primitives without importing `df-tools` |
| Development fixtures and provider tests | Loopback providers and explicit live checks | Central service/backend test seams; live tests remain opt-in |

Paths above are under `crates/df-tools/src/` unless another crate is named. The audited runtime has three image vendors, plus a generic fal adapter. BiRefNet and SAM are processing services, not additional scene-art generators. Codex CLI image generation is explicitly disabled in the inspected storyteller path; the new CLI backend adds image execution inside `df-imagery`. This describes planned support, not an already-working project integration.

### Preserve character creation as a distinct consumer

The migration must preserve the following chain:

1. Public appearance, species, class/background and portrait scenery form a validated design.
2. The authenticated request carries a stable operation and draft revision. Polling never creates work.
3. One image-specific classification selects the configured general route or BFL-compatible route, or refuses/returns uncertainty. Classification is recorded and reused for the same operation.
4. An opaque 1024 x 1536 portrait is generated; this must not become the transparent canonical-cutout profile.
5. A draft stays private to the authorized player. Ordinary repeated Generate/Confirm reuses the matching design; explicit regeneration has a new generation identity.
6. Character acceptance matches the current public design and attaches the verified image reference/digest/revision through the existing character/session path.
7. Accepted portrait sharing follows existing audience authorization. Personal portraits, canonical stage depictions and scene compositions remain different uses of the same identity.
8. A hero can be created without a portrait. Failures leave this flow usable.

Classification categories, content-rating behavior, dedicated credentials, selected models, prompt text, quality, output formats and no-cross-provider-fallback behavior remain versioned compatibility policies in the first migration. Do not use extraction to change them silently.

## Target ownership and dependencies

```text
Gameplay / creator RPC / character clips / df-media / development tools
       |  authorized ImageRequest; no vendor client or credential
       v
df-imagery::ImageService
       |-- request validation, immutable recipe and policy resolution
       |-- scoped request identity and duplicate-operation handling
       |-- bounded scheduling and owned worker lifecycle
       |-- classify -> generate/edit -> optional transform -> inspect
       |-- one retry policy, typed outcomes, recovery and diagnostics
       |
       |-- private API image backends -> shared native HTTP/egress transport
       |-- private Codex CLI backend -> owned CLI process -> image artifacts
       |-- existing finance admission/attempt/settlement authority
       |-- df-assets catalog + authorized references + byte publication
       `-- df-observe

Completed AssetRef + quality evidence + source binding
       -> session/consumer selects or attaches it
       -> existing authorized media routes
       -> browser preload / display / player UI
```

`df-imagery` must not depend on `df-tools`, browser crates, `df-session`, or concrete PostgreSQL adapters. Composition supplies existing persistence and finance adapters. A small consumer-owned port is justified only where an existing contract cannot express the required transaction; do not invent a second storage or money authority.

Image backends become private modules of `df-imagery`. Shared native HTTP, output-download controls, and unrelated text/audio/video adapters remain in `df-providers`; it does not depend on `df-imagery`. The Codex backend owns its image-specific invocation and event adapter. Reusable process lifecycle primitives can be factored into existing native infrastructure, without moving storyteller logic into the image crate or creating a dependency on `df-tools`. Remove the old public image constructors and dispatch functions after their callers migrate. Shared format/value types can stay in a neutral existing contract crate where needed; they must not expose execution.

Suggested internal modules, created only with their implementation:

```text
df-imagery/src/
  lib.rs              narrow public exports
  request.rs          validated inputs and profiles
  service.rs          submit/status/watch/cancel facade
  recipe.rs           frozen stage plans and route policy
  scheduler.rs        bounded queues, permits, worker ownership
  attempts.rs         retry/recovery decisions using existing authority
  quality.rs          declared inspection outcomes and acceptance policy
  publication.rs      df-assets integration
  transform/          private local geometry, color, composition and encoding
  compatibility.rs    bounded legacy import/key lookup, removed when retired
  backends/           private OpenAI, fal, BFL, Codex CLI implementations
```

Pure prompt builders can remain alongside their domain callers initially. They submit a bounded, versioned prompt and typed facts/reference bindings, not arbitrary HTTP payloads. Centralized prompts are not required for centralized generation. Provider-specific rewrites, output adaptation, classifier instructions, and retry corrections belong inside the image crate.

Deterministically composed party/cast reference sheets currently exist as in-memory bytes. The companion plan moves their pixel execution into Transform, while callers retain identity selection and positional legends. During migration, register existing composed bytes through the asset staging/publication boundary with a digest, inherited audience, and bounded lifetime. The final path submits authorized sources and a typed composition recipe. Do not force callers to invent filesystem paths or add a public raw-reference escape hatch. Registration cannot turn an unapproved upload into an approved reference.

## Public API contract

One service API means one route into generation, generative editing and local transformation, with a small lifecycle surface. It does not mean hiding polling or cancellation inside a single long blocking function. Proposed signatures are design sketches, not a frozen new duplicate type system:

```rust
impl ImageService {
    async fn submit(&self, request: AuthorizedImageRequest)
        -> Result<Submission, SubmitError>;
    async fn status(&self, access: ImageAccess, job: ImageJobId)
        -> Result<ImageStatus, LookupError>;
    async fn watch(&self, access: ImageAccess, job: ImageJobId)
        -> Result<ImageUpdates, LookupError>;
    async fn cancel(&self, access: ImageAccess, interest: ImageInterestId, reason: CancelReason)
        -> Result<CancelOutcome, CancelError>;
}
```

The composition root separately starts and drains the service. `watch` is bounded/coalesced state notification with revision numbers; `status` is the recovery path. Dropping a watcher does not cancel the job. `Submission` includes a consumer-interest identity bound to its authenticated operation and lifetime. Consumer cancellation withdraws only that interest; another authorized consumer's interest keeps shared work alive. Owner-wide shutdown/revocation is a separately authorized administrative operation. There is no public `render_creator`, `raw_edit`, vendor factory, arbitrary callback executor, or special frame bypass.

### Request contents

| Field | Contract |
| --- | --- |
| Operation identity | Authenticated scope + stable operation ID + fingerprint. Same ID/different request is a conflict. |
| Operation | Generate, Edit, or Transform; Transform uses authorized image/canvas inputs and an ordered local recipe, with no prompt or provider requirement. |
| Owner | Run/generation or authorized pre-character draft owner; expected source/identity revision. Drafts do not require an already-created hero. |
| Purpose | Closed variants: creator portrait, canonical person, canonical room, scene plate, scene edit, expression, item close-up, party mural, combat still, animation frame; add reference/analysis preparation and imported-asset derivative purposes during contract freeze. Variants inherit the originating domain purpose plus a typed role such as thumbnail or mask. |
| Content | Generate/Edit: bounded prompt with template version, applicable public appearance/scene data, and source binding. Transform: typed ordered effects and source binding as defined in the companion plan. |
| References | Authorized immutable asset references, ordered and digest-bound; no arbitrary file paths or remote URLs. |
| Output profile | Named purpose-compatible dimensions/format/alpha/profile and typed overrides; generated source size and delivered variant size are distinct. Cannot override provider policy or credentials. |
| Audience | Authenticated requested audience intersected with reference audiences; withheld/empty scope is refused. |
| Urgency | Visible, current presence, or speculative; authorized limits and recipe determine actual admission. |
| Execution mode | Live, prepared-only, or replay, constrained by global mode. A miss in the latter two starts neither paid work nor new local transforms. |
| Freshness | Reuse compatible result, or explicit regeneration with a new operation/nonce. Polling never changes it. |

For provider-backed stages, the service freezes the resolved provider-account identity, backend/profile, controllable model/version settings, route/classification version, effective output settings, quality policy and retry policy before each affected dispatch. Local stages freeze their transform/executor profile without a supplier identity. For CLI-managed image models, freeze the capability contract and record the actual image-model identity afterward when exposed; unknown identity remains unknown. Callers select the intended picture, not a supplier. Existing CLI/settings provider choices are mapped into versioned service configuration at bootstrap.

Creator classification is itself a potentially paid stage, not a free admission preflight. Submission first freezes a classification-stage plan without network I/O. That stage gets its own recorded admission/attempt, and its result is durably attached to the job. The service then resolves and freezes the selected generation route and obtains that stage's admission. A refusal/uncertain classification stops there. Repeated polling or duplicate submissions reuse the recorded decision. Directly controlled classification, inspection and background-removal calls require their corresponding dispatch record/accounting admission. Opaque work inside Codex CLI uses the admitted execution-envelope contract below; observable internal calls are evidence, not falsely claimed independent admissions.

### Submission and result

`Submission` distinguishes a ready cache result, an existing job, a newly accepted job, and deferred capacity. Typed refusals distinguish authorization, invalid request, operation conflict, unsupported profile, disabled capability, unavailable references, and admission refusal.

`ImageStatus` exposes queued/processing/ready/failed/cancelled/awaiting-reconciliation with a revision and a safe reason. A ready result contains a primary `AssetRef`, dimensions, MIME, alpha facts, digest, source binding, and explicit quality evidence. For recipes with variants, it also contains named required/optional variant statuses and authorized references as defined in the companion plan. Ready requires all required outputs; optional failure does not discard the primary result, and pending optional work retains an owned lifecycle visible in status. Source and variant quality evidence remain distinct. It contains no provider credentials, signed remote supplier URL, or raw private prompt.

Quality is separate from availability: `Passed`, `Unjudged(reason)`, or `AcceptedWithKnownDefect(reason)` may describe a ready result under the selected compatibility policy; rejected candidates are not ready. Strict verification can be a separately selected later policy. Preserve today's fail-open and visible-text exceptions explicitly during extraction, rather than relabeling them as passed or changing behavior unnoticed.

## Identity, cache and durable attempts

There are three different identities:

1. **Consumer operation**: idempotent intent such as a player's Generate click.
2. **Image request key**: reuse-compatible resolved recipe, inputs, audience and scope.
3. **Provider attempt/stage**: one potentially billable controlled dispatch, plus its provider job ID if returned. Direct classification, generation, alpha extraction, inspection and a corrective generation each have distinct durable child attempt IDs under the parent image job. Polling the same supplier job retains its child attempt ID. A CLI invocation with opaque internal work is one admitted execution envelope; recorded inner calls are linked observations and are not charged again as independent attempts.

Use the existing `df-assets::MediaRequestKey` length-prefixed, audience-aware scheme as the base. Extend/version its schema where necessary for output format, effective dimensions/quality, template/route/inspection policy revisions, stage recipe, and regeneration identity. Do not assume the existing v2 struct already contains every required field. Centralize derivation; callers do not hand-roll hashes. Distinguish a reusable base content key from an explicitly fresh generation key so a new operation can still reuse ordinary cached content.

The catalog's storage lifecycle (`Uploading/Quarantined/Validated/Ready/...`) is not the supplier attempt lifecycle. Preserve each authority. Compose the existing media admission and finance attempt records under the established lock/transaction contract; extend missing fields in those owners after an implementation audit. The current catalog has one asset-attempt field: phase 1 must freeze an explicit parent job/catalog-attempt to child supplier-attempt mapping before dispatch is enabled. Do not overload that single field to settle several billed calls, assume currently separate Rust ID types are interchangeable, or silently override the existing single-attempt correlation contract. Record the necessary multi-stage extension in the ownership ADR and persistence/finance contract. Do not create an independent imagery SQLite journal or monetary ledger.

Required execution sequence for provider-backed recipes follows below. Local stages have durable job/stage and publication identity, but no dummy supplier attempt, reservation or settlement. They use the companion plan's local execution and retry contract; a mixed recipe applies financial admission only to its paid stages.

1. Validate current scope and look up the operation, retained stage decisions and compatible ready result without network I/O; do not reserve supplier spend on a hit. Resolve deterministic recipe stages now; paid classification follows the admitted-stage sequence below.
2. Atomically bind the operation, media work and paid admission, or return the existing outcome. Where current APIs cannot do this, extend the shared transaction boundary before enabling the hosted path. A pair of best-effort writes is insufficient.
3. Claim work with an owner/fence. Persist each controlled child attempt (or the CLI execution envelope) and its dispatch-prepared state before network I/O/process launch. That ID is consistently linked through its dispatch, financial and stage-result records; the parent catalog attempt retains its separate publication identity under the frozen mapping. Do not collapse independently controlled paid stages into one settlement identity or double-charge observed CLI internals already covered by their envelope.
4. Revalidate dispatch authority; execute outside the database transaction. Save provider job IDs as soon as known. Repeated polling/downloads use the same provider job.
5. Validate output bytes and recipe requirements, stage and publish through `df-assets`; publish ready metadata only after complete bytes are durable.
6. Settle each billed stage through the existing finance path. Ready output and unknown billing are distinct facts; a settlement retry must not regenerate an image.
7. Recover by recorded state. Resume known jobs, repair publication from staged bytes, or enter outcome-unknown reconciliation. Never automatically resubmit a potentially accepted request because a lease expired or the process restarted.

Providers without an idempotent submit or lookup cannot support an absolute exactly-once guarantee across a network crash. The contract is **no automatic duplicate paid dispatch after uncertainty**. A fresh explicit retry requires a new admitted attempt, while the old liability remains recorded. Preserve the character-frame prior-attempt fence throughout migration.

Local development preserves its current accounting policy via an explicit local adapter; the extraction does not silently add a user spend cap, require a customer wallet, or enable hosted billing. **Local and hosted execution both require durable attempt identity and the same conservative unknown-outcome recovery semantics.** Their financial admission policies may differ. Neither mode may regress an existing durable prior-attempt guard. Project-wide completion requires proving restart protection in both modes, not just providing a hosted interface.

## Scheduling, cancellation and retry ownership

- One image scheduler per runtime/service owner. Provider pools are keyed by actual supplier account identity and workload class, not by each `Imagery` object; dedicated creator credentials remain separate accounts where applicable. Local transforms use separate CPU/memory permits under this scheduler, without taking provider permits.
- Start from explicit compatibility settings for foreground/presence/background capacity (currently 3/3/6 per engine). Consolidating per-engine capacity into account-wide bounds is a measured policy change; record effective limits and compare latency before activation. No implied global account limit from a local semaphore.
- Enforce bounded queue count, queued reference bytes, active memory, age and deadline. Foreground retains capacity; speculative work has a bounded share and cannot masquerade as foreground after a short delay.
- Duplicate callers attach to one job and consume neither another dispatch attempt nor another monetary reservation. Coalescing requires compatible authenticated scope, audience, source and lifetime policies; byte reuse alone does not authorize sharing a job. Attempt count changes only at the defined dispatch transition.
- A single owned worker tree handles generation, polling, transforms, checks and publication. `shutdown/drain` reports completed versus unresolved work. No detached image task escapes service ownership.
- Caller disconnection stops waiting. Explicit cancellation before dispatch releases only proven-unsent reservations. Cancellation after dispatch suppresses stale publication as appropriate and preserves billing uncertainty; it does not promise a refund.
- Consumer-interest cancellation leaves the shared job running for other authorized interests. Only after the last interest disappears may the service retire unsent work under its lifetime policy. An owner-authorized run shutdown or revocation can stop all affected interests; cached output still follows its own authorization/retention rules.
- Stale speculative output may remain authorized reusable bytes, but it cannot replace the current scene. Source/run/identity fences are rechecked before selection or attachment.
- One retry policy controls safe 429 retries, transport failures, quality corrections and explicit user regeneration as different events. A quality correction is another billed attempt. Poll retries are not new generations. Moderation/refusal never automatically reroutes to another provider.
- One absolute job deadline bounds classification, queue waits, generation, backoff, transforms and inspection; shorter per-stage deadlines cannot reset it. Financial reconciliation may continue as separately owned bounded maintenance after that deadline, without dispatching a replacement generation or treating expiry as proof of no charge.
- A shared quota breaker is scoped by supplier account. Its cooldown does not erase an unknown paid attempt.

## Provider and processing policy

Keep initial profiles behavior-compatible:

| Profile | Initial policy to preserve |
| --- | --- |
| General imagery | Configured OpenAI default or explicit fal FLUX; existing supported model/configuration validation |
| Creator portrait | Dedicated configured account; one recorded Jev classification; general/BFL routing under current rating rules; one logical creator attempt |
| Animation frame | Existing OpenAI compatibility profile keeps Sunburst transparent PNG and medium-dev/high-production settings. Explicit CLI backend selection may use its own image model while preserving required dimensions/alpha, validation and explicit retry after a prior attempt. |
| Scene/room quality | Existing one-correction policy and explicit unjudged/known-defect handling |
| FLUX transparency | BiRefNet is an explicit metered recipe stage, not an invisible extra call |
| Generic Schnell | Not selected by any migrated gameplay profile; internal only if retained |
| Codex CLI | Normal configured backend for compatible image purposes; supported CLI image generation/editing with the same references, output validation, cache, lifecycle and publication contract |

The private backend contract must distinguish unsent/rejected submit, accepted with job ID, completed bytes, and unknown dispatch. An adapter must not erase submit/poll stages into a retryable generic HTTP failure. Native HTTP safety, bounded downloads, format validation and current provider restrictions remain mandatory.

Keep raw supplier responses out of consumer APIs. Account IDs in telemetry are nonsecret handles; keys are supplied only at trusted composition/egress. No network or live model call is required by normal CI.

## Codex CLI backend

### Configuration and compatibility

Add `codex-cli` to the central image backend selection, including the compatibility mapping for `DF_GAMEPLAY_IMAGERY_PROVIDER`. Selecting it must work through ordinary service configuration for scene images, creator portraits, murals, items and animation frames when the required output capabilities are supported. It has no additional experimental-enable flag, hard-coded local-only rule, or forced API-key requirement. The existing creator classification/rating decision still applies; a general portrait route can use Codex CLI, while an explicitly selected BFL route remains BFL. Selecting CLI images alone does not replace Jev or other independent services.

Use the configured Codex executable and its supported authentication, including ChatGPT-managed authentication. The application does not rewrite the user's account configuration or silently change the billing/authentication mode. Account-specific scheduling and cache provenance distinguish the CLI account from API credentials. Hosted composition can use this backend through its normal configured execution boundary; local support does not depend on deploying the hosted service.

Detect the actual capabilities required by the request: image generation, reference/edit input, output artifacts, format/alpha handling, and machine-readable completion. Record the CLI version and capability-adapter revision for diagnostics and reproducibility. **Do not require one exact CLI version, executable hash, fixed model name, or a bespoke empty CLI home.** A newer CLI is not rejected merely because its version changed. An actual unsupported capability returns the existing typed unsupported outcome for that operation.

Read-only inspection on 2026-10-27 found installed `codex-cli 0.164.0`, with `image_generation` listed as enabled. The project text wrapper separately hard-codes `0.150.1`. These observations demonstrate a version mismatch, not a reason to carry that pin into the new backend. Local help confirms `exec`, JSONL output, image attachments, working-directory selection and structured final responses; it does not establish the full image-artifact event contract. Verify that contract during implementation with recorded fixtures and an authorized end-to-end image test.

### Execution through the shared API

```text
ImageService::submit(authorized request)
    -> normal operation lookup / stage admission / backend selection
    -> resolve and stage authorized reference images
    -> Codex CLI invocation owned by the image worker
    -> parse progress + collect generated image artifacts
    -> normal byte/format/quality validation
    -> df-assets publication
    -> ordinary ImageStatus::Ready and AssetRef
```

The backend assembles the CLI prompt and arguments from the frozen request. Use native argument arrays and stdin, not interpolated shell commands. Treat user-supplied appearance/scene text as image content. Reference files are exact authorized, digest-verified inputs staged in the job's working area. Deterministic reference-sheet composition and output normalization remain supported.

Allow the tools and helper processes required by the supported CLI image workflow. Do not inherit the text wrapper's blanket tool-disable list or one-process limit. Use normal configured execution permissions and explicit job-owned input/output locations; never require bypassing platform access controls. Track the complete process tree, including image helpers, so cancellation and shutdown can clean up owned work. Start Windows helpers without visible windows. Support the host platforms through their normal process APIs rather than making the existing Windows-only wrapper a product requirement.

Use a dedicated image-event adapter; do not treat the text parser's rejection of non-text events as the image protocol. Keep the storyteller's direct image-generation path disabled so all project images enter `ImageService`. If the storyteller requests an image, it emits an image intent through that API.

Accept actual image artifacts, not a successful exit code or prose claiming that a file was written. Validate ownership of the returned path/handle, complete bytes, MIME, decoded pixel bounds, dimensions and required alpha before publication. The CLI may return artifacts via its documented output mechanism; copy verified bytes into the owned publication area rather than demanding one invented output filename. A textual final-response schema can describe a manifest but does not prove that the referenced image exists. Cleanup targets are service-owned paths, never arbitrary paths from generated text.

### Attempts, usage and recovery

A CLI launch is an execution envelope, not proof of exactly one image generation. Record its admitted envelope before launch and link any available thread, turn, tool and artifact IDs. Capture observable inner image calls and their usage beneath it; do not double-charge those calls and the envelope. Where the CLI exposes dispatch control, bind inner calls to pre-recorded child attempts. Otherwise the envelope is the admitted accounting unit and opaque internal activity is recorded as such, rather than falsely claiming per-tool admission. Use existing local or hosted accounting policy for that unit; do not introduce a new CLI-only wallet, hard-coded generation quota, or arbitrary process-count cap.

Use available CLI usage evidence. Subscription usage, text tokens, image-tool usage and API dollar charges are different measures. Missing image cost is unknown/unreported, not zero; do not price subscription usage as if it were a direct Image API response. A deployment whose existing financial policy requires a quote must obtain a supported quote/accounting policy before dispatch, just as for other backends.

The shared job deadline, cancellation ownership, duplicate-operation behavior and durable restart protection apply unchanged. Terminating a process tree does not prove that remote image generation stopped or consumed no usage. On timeout, interruption or lost output after possible dispatch, retain an unknown outcome and do not automatically start a new CLI turn or fall back to an API backend. Recover verified complete artifacts when possible. An explicit fresh generation uses the existing new-attempt flow.

Cache identity includes the `codex-cli` backend profile, request/reference digests, effective output policy and capability-adapter revision. Record the observed CLI/agent/image model identities separately where exposed; `--model` selecting the agent is not evidence of which image model generated the artifact. CLI version changes alone do not invalidate all existing cached images. Compatibility-affecting behavior changes get a new profile/adapter revision.

### CLI acceptance evidence

- Fake-executable fixtures cover argument/reference handling, image and progress events, authentication/capability failure, malformed or absent artifacts, helper-process ownership, timeout, restart and unknown usage without calling a provider.
- Verify generation and reference-based editing, required portrait/frame formats and alpha, multiple reported image calls, cache-hit zero launches, duplicate request one launch, explicit regeneration and cancellation with another consumer still interested.
- Check path ownership, invalid image bytes, stdout/stderr bounds and cleanup after failed publication. Needed helper processes must work; they must not escape lifecycle ownership.
- Run the existing creator and scene acceptance flows through this backend. Test that storyteller mode still cannot create an image outside `ImageService`.
- Add a bounded live CLI image acceptance run when implementation is ready and provider usage is authorized. A feature listing or successful text turn alone is not image integration evidence.

Official documentation checked for this design: [non-interactive execution and JSONL output](https://learn.chatgpt.com/docs/non-interactive-mode), [CLI commands and reference-image input](https://learn.chatgpt.com/docs/developer-commands?surface=cli), and [image generation guidance](https://learn.chatgpt.com/docs/image-generation). These establish general CLI/image workflows, not a verified project-specific artifact parser or an exact model-control contract.

## Configuration, tuning and disabling generation

### One configuration model

`df-imagery::ImageConfig` is the canonical validated configuration. The launcher, run scripts, server bootstrap and development control surface all translate into that model; consumers never independently read image environment variables or decide whether to start a provider. The crate reads a supplied snapshot, not ambient environment on every call.

Use the existing `config/dungeonflux.toml` as the operator-facing file, with an `[imagery]` section. Keep the old `[pictures]` fields and environment variables as documented compatibility aliases while callers migrate. Configuration fields and examples below are **proposed**, not already implemented. The existing launcher serializes settings into environment variables today; that adapter must preserve explicit overrides and their source rather than accidentally giving generated defaults higher priority.

Precedence, low to high: built-in defaults -> saved TOML -> explicitly supplied environment overrides -> explicit launch arguments -> authorized runtime override. Legacy aliases are normalized within their source layer; contradictory new/legacy values in the same layer produce a useful field-level configuration error. Read effective configuration with its source and revision so an environment override cannot silently defeat a launcher setting.

The development/configuration interface exposes read-effective, validate, apply-with-expected-revision, reset-runtime-overrides, and save-to-file actions through the existing authenticated control surface. This is an administrative configuration API alongside `ImageService`, not another generation method. Runtime changes apply without a rebuild. Saving persists them for future starts; a runtime-only change is identified as temporary. Launcher controls and CLI tooling use the same validator and clearly show overridden values. Hosted deployment uses its existing operator configuration channel.

### Settings to expose

| Area | Tunables | Meaning |
| --- | --- | --- |
| Service | `mode = live / local-only / cache-only / off`, default backend, default output profile | Enable execution, allow local derivatives only, reuse-only, or stop accepting new image demand |
| Local transforms | Enabled state, profiles, CPU concurrency, decoded-memory/output limits, queue/deadline | Separate local resource permits; no supplier calls; see companion plan |
| Purpose | Enabled state and optional backend/output overrides for every `ImagePurpose` | Independently control portraits, canonical art, scene plates/edits, expressions, items, murals, combat stills and animation frames |
| Backend | Enabled state, account/auth reference, backend-supported model/profile settings, concurrency | API and CLI backends use the same selection mechanism; no hidden secondary enable switch |
| CLI | Executable, supported CLI profile, optional agent model, job working directory, concurrency | Authentication uses the configured CLI account; agent model selection is not an image-model guarantee |
| Scheduling | Foreground/presence/background limits, pending count/bytes, queue age, job/stage deadlines | Finite resource bounds with configurable defaults rather than hard-coded product ceilings |
| Refresh/prefetch | Minimum refresh interval, speculative enablement, lead time, successor count | Tune image churn and preparation separately from gameplay/presentation timing |
| Output | Quality, size, format/alpha and development/production profile by purpose | Checked against backend capabilities and the consumer's actual output contract |
| Quality/retries | Inspection acceptance policy, corrective-generation count, safe retry/backoff policy | Explicitly controls additional calls; uncertain dispatch is never made safe by a retry setting |
| Storage | Cache/staging locations and existing asset retention policy references | Changing a location does not delete or invalidate accepted images |
| Costs | Price-file reference, report defaults, optional alerts | Tracking is always active; no new spend cap is imposed by this refactor |

The current 20-second imagery interval, 170-second provider deadline, concurrency allocations and profile presets are migration defaults, not immutable minimums/maximums. Validate meaningful values, actual provider limitations and output invariants; do not retain arbitrary tuning restrictions merely because they exist in the old parser.

Example: choose CLI images, allow portraits, turn off speculative generation and animation-frame generation. Values are illustrative operator choices, not measured capacity recommendations; unshown purposes retain their configured/default values.

```toml
[imagery]
mode = "live"
backend = "codex-cli"
profile = "production"
job_deadline_seconds = 240

[imagery.backends.codex_cli]
enabled = true
executable = "codex"
max_concurrent_jobs = 2

[imagery.purposes.creator_portrait]
enabled = true

[imagery.purposes.scene_plate]
enabled = true
quality = "medium"
minimum_refresh_interval_seconds = 20

[imagery.purposes.animation_frame]
enabled = false

[imagery.prefetch]
enabled = false

[imagery.costs]
price_file = "content/pricing.json"
```

### Existing configuration mapping

| Existing input | Central destination / preserved meaning |
| --- | --- |
| `[pictures].picture_provider`, `DF_GAMEPLAY_IMAGERY_PROVIDER` | Default backend; add `codex-cli` as a normal value |
| `picture_quality`, `picture_profile`, `DF_GAMEPLAY_IMAGERY_MODEL/QUALITY/PROFILE` | Output/backend defaults; unsupported API-specific model settings do not silently become CLI agent models |
| `scene_pictures`, `DF_GAMEPLAY_IMAGERY` | Existing scene-art purpose group; retain its current scope instead of silently turning it into the new global switch |
| `scene_plates`, `DF_GAMEPLAY_PLATES` | Scene plate and plate-edit purpose enablement, retaining their scene-art dependency |
| `creator_portraits`, `DF_GAMEPLAY_CREATOR_PORTRAITS` | Creator-portrait purpose enablement |
| `creator_alternate_route` and dedicated creator credentials | Creator route policy and backend account bindings; primary CLI portraits need no OpenAI API key |
| `DF_GAMEPLAY_CHARACTER_VIDEO` | Video dependency: when off, no clip-only frame demand; frame purpose may also be disabled independently |
| `DF_GAMEPLAY_IMAGERY_INTERVAL_SECONDS/DEADLINE_SECONDS/CACHE` | Refresh, backend-stage deadline and cache settings, preserving units |
| `DF_GAMEPLAY_CODEX_EXECUTABLE/HOME/WORKDIR` | Compatible CLI executable, auth and working-directory inputs; new TOML settings can specify them directly |
| `DF_GAMEPLAY_CODEX_SHA256` and old fixed-version check | Not required by the image backend; do not use old text-wrapper pins as image admission criteria |
| `DF_GAMEPLAY_CODEX_MODE` | Storyteller selection only; image backend is independently selectable |
| `DF_GAMEPLAY_DISABLE_OPENAI_API` | Existing direct-API exclusion remains meaningful; it does not disable ChatGPT-authenticated CLI generation. Required auxiliary API stages report the dependency, rather than secretly calling a disabled API. |
| `DF_GAMEPLAY_PRICING` | Existing price-file override for the shared cost engine |
| New `DF_IMAGERY_MODE` | Explicit global `live`, `local-only`, `cache-only`, or `off` control over image execution |

Disable the entire image interface with `[imagery].mode = "off"` or `DF_IMAGERY_MODE=off`. Use `cache-only` to reuse matching completed images without new generation or local computation. Use `local-only` to allow local transforms of authorized ready images while disabling every provider-backed stage. Disable just CLI dispatch with `[imagery.backends.codex_cli].enabled = false`, local stages with `[imagery.transforms].enabled = false`, or one feature under `[imagery.purposes.<purpose>]`. Disabling one backend never silently routes to another. Required disabled transform stages are detected before paid dispatch.

### Runtime change semantics

| Change | New/queued work | Already dispatched work | Existing authorized images |
| --- | --- | --- | --- |
| `mode = off` | New submissions return `Disabled`; retire unstarted stages, including classifier, checks and transforms | Finish the already-started stage without starting another effect/provider stage; publish only if the frozen acceptance contract can be satisfied | Remain readable; current displays keep their fallback/current image |
| `mode = cache-only` | Matching ready results may be reused; misses return `PreparedMiss`; no new provider or local transform stage, including from the old queue | Same drain rule as off | Remain readable |
| `mode = local-only` | Ready results and local transforms allowed; no new provider-backed stage or CLI launch | Started provider stages drain; local completion allowed only when no further provider stage is required | Remain readable |
| Purpose disabled | New demand for that purpose returns `Disabled`; remove its queued interest | Other coalesced interests may still need the work; no new stages solely for the disabled purpose | Remain readable; disablement does not revoke sharing |
| Backend disabled | No new dispatch to that backend; unresolved demand reports its disabled route | Drain already-started work; preserve result/usage evidence | Valid cache hits remain reusable without dispatch |
| Backend/quality/profile changed | New operations use the new config revision | Existing jobs keep their frozen recipe; no mid-job reroute | Compatible cache remains; changed content settings get different keys |
| Concurrency reduced | Scheduler respects the lower limit before admitting further work | Existing calls finish; temporary excess is reported rather than killed | Unchanged |

Configuration application and stage dispatch share a serialized gate: once a disable response is acknowledged, no previously queued stage can slip through using an old enable snapshot. In hosted deployment, the actual egress dispatchers must enforce the new revision; acknowledge a completed disable only after every affected dispatch owner has applied it or been fenced from dispatch. A runtime-local update alone returns pending propagation, not a false global success. Disabling the whole interface also stops prefetch and clip-frame/classifier loopholes. Runtime disable may drain and publish a completed image with its recorded acceptance state; it must not label an unfinished required validation as passed. A separate explicit stop-active action terminates owned CLI processes/cancels supported requests and marks uncertain remote outcomes appropriately. It makes no refund claim.

Changing settings never cancels a committed game action, rewrites rules or blocks character creation without a portrait. Re-enable by setting `live`; consumers re-evaluate current demand. Cancelled/stale/unknown attempts are not automatically replayed. Queue/deadline/concurrency settings do not invalidate image content keys; effective output/prompt/route/acceptance changes do. Invalid live updates preserve the last valid snapshot and return field errors. Diagnostics show effective revision, winning source, backend readiness, queue/active counts and reasons for disabled or unavailable purposes, with credentials redacted.

## Cost tracking and reporting

### Reuse the existing cost engine

The project already has `gameplay/cost.rs`, a metered `SpendLedger`, dated rates in `content/pricing.json`, durable run reports in `development/runs/<database>/cost.jsonl`, `dev cost` / `GET /dev/cost`, and provider-bill comparison. It also already distinguishes `codex_subscription`. Integrate with these facilities and the existing hosted finance authority; do not build an image-only wallet or a second set of totals.

Move/re-export only the required cost contract into a dependency-neutral existing owner, or inject its adapter at composition, so `df-imagery` never imports `df-tools`. The existing image call-site guard must check API calls, classifier/check/transform stages and CLI execution envelopes. Reporting data is a projection of recorded attempts and settlements; JSONL/telemetry is not a substitute for durable dispatch identity. Preserve report history when the implementation moves.

Every image job has a cost summary available through the authorized job-status/reporting flow. Operator views contain supplier evidence and internal accounting; player/customer views expose only the amounts and status allowed by existing customer-pricing policy. Permission to read an image does not grant permission to read supplier accounts, rates, reservations or internal liability. A cache hit reports zero **new generation** cost and references the existing asset; it does not erase the original cost. Coalesced callers see the permitted projection of the same job summary, and rollups count the underlying attempt once. Keep billing ownership distinct from the set of consumers that used an image.

Wholly local transforms have no supplier attempt or charge. Report local processing/storage usage separately; any infrastructure estimate is labeled and does not enter supplier settlement. Link source generation costs through lineage without charging or summing them again for every derivative.

### Record for every controlled call or CLI envelope

- Parent job, consumer operation/purpose, attempt/envelope ID, causal stage, source/config revision, account handle, backend, model identity when known, run/session and campaign scope.
- Pre-dispatch estimates, any existing-authority reservation, recorded send state, timestamps, usage evidence, terminal/unknown outcome and associated published asset.
- Usage quantities with units and evidence class: reported, measured/estimated, or unknown. Include applicable text/image tokens, images/megapixels, provider credits, CLI turns/tool observations and subscription usage separately.
- Exact monetary amounts in integer micro-dollars for USD rate tables, with rate/price-version provenance. Other currencies, credits and subscription units are separate typed amounts; do not add incomparable units into one total.
- Independent settlement evidence: reported usage priced by a rate table, provider-reported charge, or bill-reconciled charge. A token-based estimate is not an invoice-confirmed actual charge.
- Correction/retry reason; generated-but-rejected, unused speculative, cancelled-after-dispatch and failed-but-billed work remain attributable costs. Unknown usage stays outstanding until evidence resolves it.

Local unpriced usage remains allowed under the existing policy and is visible as unpriced. Do not invent a dollar figure or block local generation because a tariff entry is absent. Existing hosted admission/quote policy remains the authority for hosted spending. Cost tracking does not add a new hard cap; optional operator alerts can notify without silently disabling generation.

For Codex CLI, report subscription consumption and available token/tool counts even when a monetary charge cannot be observed. API tariff rates must not be presented as actual subscription charges. An optional allocation of a recurring subscription fee may be shown as an operator-supplied planning estimate, clearly separated from marginal per-image cost and known charges. Missing data is never displayed as free or zero. Observable internal tool calls under an opaque envelope are not charged again.

### Views and reconciliation

Extend existing `dev cost` and `/dev/cost` with image filters and breakdowns rather than adding another reporting surface:

| View | Show |
| --- | --- |
| Job/image detail | Consumer, backend/profile, every stage/envelope, attempts, estimate, reported/rate-derived/bill-confirmed amounts, unknown/unpriced usage, quality result and resulting asset |
| Run/session/campaign | Known charges, rate-derived estimates, unresolved liability, unpriced activity, subscription units and total image jobs as separately labeled measures |
| Backend/account/purpose | Spend and usage by scene plates, portraits, frames, murals, etc.; corrections, failures, speculative waste and top spenders |
| Efficiency | Cache hits, coalesced requests, generated images actually selected/displayed, cost per generated/used image where computable; unknown portions explicitly excluded and counted |
| Provider comparison | Existing bill-import comparison by attempt/provider/model/date; discrepancies and corrected settlement revisions |
| Operational health | Cost-writer/export lag or failure, unsettled attempts, missing tariffs and missing CLI usage fields |

Reports should answer: "What did this portrait cost?", "What did this run spend on pictures?", "Which provider or feature used most?", and "How much was retries or unused prefetch?" without requiring prompt logs. Supplier details remain in authorized operator views; the existing customer pricing policy determines customer-facing amounts. Export structured JSON/CSV from the same canonical aggregation for investigation.

Settlement and correction events are idempotent by attempt/envelope identity and revision. On restart, rebuild the report without adding duplicate calls; late bills reconcile existing entries rather than append another charged generation. Keep reserved/unknown exposure separate from incurred expense. A failure to persist/export reporting data is visible as degraded cost tracking; recover from durable attempt/usage records instead of claiming complete totals from an in-memory fallback. Disabling generation still permits status reads, cost reporting and reconciliation of existing work.

## Compatibility and rollout

The existing image HTTP URLs, grants, creator operation behavior, browser presentation clocks, identity selection, fallback images and accepted portrait references remain stable during caller migration. Delivery handlers resolve a generated `AssetRef`; they do not start work on GET.

Legacy cache support is read-only and authority-aware. Import verified bytes/metadata into the catalog only when a scoped owner can establish their provenance and authorization. A v1 filename or shared-cache presence is not sufficient. Do not bulk regenerate old images because a request schema changed. Preserve old accepted references, notes, masks and character-frame attempt records; unknown legacy quality is `Unjudged`, not `Passed`.

Rollout selects exactly one execution owner for a request. Shadow comparisons may compare pure normalized requests, route decisions and keys using fixtures; never send both old and new live provider requests. Migration flags cannot permit a fallback from an uncertain new attempt to an old generator. Rolling back code must keep the journal/import fence readable, or stop new generation while existing attempts are resolved.

## Implementation sequence and acceptance gates

| Phase | Deliverable | Gate before proceeding |
| --- | --- | --- |
| 0. Freeze baseline | Exact revision/configuration; inventory of every constructor/call site; recorded current creator/frame/plate behaviors; audit existing catalog, finance and request-key capabilities; inspect CLI image invocation/artifact contract | Every consumer/provider in this document mapped to reachable code or explicitly test-only; existing failures recorded separately; CLI text-only behavior distinguished from new image support |
| 1. Freeze contract | Add `df-imagery` with narrow validated types, service facade, fake backend, configuration schema/precedence, shared cost contract, ownership rules and architecture guard | Contract tests prove operation conflict/reuse, prepared-only behavior, unauthorized input refusal and bounded status/watch; validate legacy setting mapping and cost identity; no new live feature |
| 2. Central execution | Move image backends, routing, normalization, retries, quota breaker, inspection policy and worker ownership; add the Codex CLI backend/event adapter; adapt current storage/metering | Loopback/fake-process tests for each backend and failure stage; old imagery facade only delegates; one dispatch owner; no exact-version CLI pin or inherited one-process restriction |
| 3. Migrate every consumer | Canonical/scene/item/mural/kill stills, creator workflow, animation frames, directed dev paths; launcher/run-script/config controls and image cost reporting | Each consumer uses `submit`; creator private preview/accept and frame retry fence preserved; global disable reaches every path; cost attribution complete; no live shadow requests |
| 4. Durable integration | Reuse/extend asset catalog, finance admission, attempt records, request key and publication; import legacy cache/attempt metadata safely | Crash matrix below passes on actual local PostgreSQL adapters; local and hosted policy modes remain distinct; no unknown-outcome resend |
| 5. Delete bypasses | Remove old public image adapters and `submit_image`, redundant image flights/cache/retry owners and temporary execution shims; keep necessary read compatibility only | Dependency/AST/endpoint guards enforced with no migration exemptions; all known consumers pass focused integration checks |
| 6. Integrated acceptance | Native/WASM checks, browser creator/TV/phone flows, cold/warm cache and restart runs, Codex CLI generation/edit acceptance, scoped live-provider visual sample | Independent evaluation against current build; no acceptance based only on unit-test counts; source/docs map updated; CLI support demonstrated through the same consumer API |

Phase 4 contract discovery is part of phase 0/1. Existing durable guards must remain in place through phases 2/3; this sequence does not authorize a temporary weaker implementation. Migrate animation frames only when their attempt fence is expressible in the central service. Do not call the consolidation complete until phases 5 and 6 pass.

Required documentation changes at implementation: crate/dependency map, subsystem interfaces, Asset Engine ownership, runtime reliability, image configuration/reference, provider inventory, cost instrumentation and the README. Record the governing ownership decision in the next numbered ADR. Do not overwrite historical playtest evidence.

## Regression prevention and verification matrix

### Enforced architecture

1. Private backends: application code cannot import a live image-provider constructor from any public crate.
2. Cargo dependency check: no `df-imagery -> df-tools` cycle; no WASM client dependency on native image execution.
3. AST/call-site audit: enumerate generation/edit dispatch, image route factories, known endpoint/model construction and image-producing CLI process launches; allow execution only inside private image backends. Test that the guard rejects a deliberately forbidden fixture. Ordinary non-image Codex text calls are not a violation. Do not rely only on string grep.
4. Scan scripts and raw HTTP call sites too. Extend the existing cost/call-site guard rather than create competing allowlists. Shared OpenAI transport used for text is not itself an image violation; classify the operation/endpoint.
5. Runtime fixture transport records every dispatch with job, stage, attempt and cost identity. Unexpected or unmetered image calls fail integration tests.
6. No perpetual allowlist entries for creator or character clips. A migration exception has a removal phase and cannot remain at completion.

### Behavior and fault cases

| Boundary | Required evidence |
| --- | --- |
| All consumers | Expected purpose, source binding, references, route/profile and audience recorded; zero direct backend calls |
| Creator | Generate/poll/reconnect/repeat/explicit-regenerate; edit while pending; stale completion; no-portrait join; matching portrait acceptance; private draft cannot be read by another seat/display |
| Identity | Accepted appearance preserved into canonical depiction, party mural and scene references; regenerated draft does not change an accepted character without commit |
| Frames | Cache hit; prior attempted frame without bytes; explicit retry; correct alpha/model/format; video-disabled starts no frame calls |
| Codex CLI | Generation/edit artifact fixtures; compatible newer CLI accepted; needed helpers allowed; missing capability explained; private reference handling; actual artifact validation; usage uncertainty; duplicate request/cache/restart behavior; no API fallback after uncertain CLI dispatch |
| Configuration | File/env/launch/runtime precedence; aliases; launcher override visibility; invalid update preserves prior revision; hot tuning; disable acknowledged before any further stage dispatch; off vs cache-only; purpose/backend disable; independent storyteller/image selection |
| Costs | Every backend/stage/CLI envelope attributed; cache hit zero new calls; coalesced result counted once; correction/failure/unused prefetch retained; unknown/unpriced/subscription distinguished; restart and late settlement idempotent; bill reconciliation; report persistence failure visible |
| Dedupe | Concurrent identical callers cause one classification and one generation plan; retry budget unaffected by watcher count; same operation/different fingerprint conflicts; cancelling one interest preserves another |
| Scope | Identical content in different private audiences never authorizes cross-access; reference revocation/tombstone fences later reads and publication |
| Replay | Ready hit performs zero provider calls; prepared-only/replay miss is typed unavailable |
| Provider failures | Proven unsent failure, rejected submit, ambiguous timeout, accepted job/poll failure, download failure, output decoding failure; no accidental cross-provider fallback |
| Corrections | One allowed corrective generation; every stage accounted once; failed checker stays unjudged; compatibility visible-text exception remains explicit |
| Lifecycle | Cancel before/after dispatch; waiter disconnect; owner/run reset; stale completion; bounded drain/shutdown; quota breaker recovery; all stages/backoff obey the absolute job deadline |
| Crash recovery | In both local and hosted modes: crash before admission commit, after commit/before send, after send/before receipt, after job ID, after classification/before route freeze, after bytes staged, after publication/before settlement; recover without blind paid resend or repeated paid classification |
| Scheduling | Foreground remains responsive with speculative backlog; bounded memory and queue age; per-account concurrency; no late prefetch replacing current plate |
| Compatibility | Existing accepted portrait URLs and stored images survive import/restart; legacy attempt evidence blocks automatic paid replay |

Use controlled clocks, faithful fake providers and fault-injected stores for ordinary tests. Run targeted native tests and Clippy for affected crates, native compilation, and the established `df-tools` WASM check for consumer changes. Classify inherited failing/skipped tests; never claim a clean baseline by hiding them.

Browser acceptance must cover the creator on a phone-sized view, accepted portrait on player/party/turn surfaces, TV scene transitions, late image arrival and fallback. Measure queue/provider/inspection/publish/display latency separately. Paid visual checks require an explicitly bounded test configuration and spend authorization when implementation reaches that gate; this planning task authorizes no provider spending.

## Observability and completion criteria

One trace relates consumer operation -> image job -> recipe stage -> provider attempt -> asset publication. Record cache hit, coalesced callers, queue wait, supplier duration, check result, correction count, unknown liability, publication time, stale completion, and first authorized display. Include template/config/build revisions and request/asset digests; keep prompts, credentials and private descriptions out of ordinary logs.

Completion means:

- Every inventoried production image-generation consumer uses `ImageService`.
- Every image-generating backend is private to `df-imagery`; no public generic bypass remains.
- Codex CLI is selectable through the same API and passes image generation/edit and consumer acceptance; it is not omitted as a future optional adapter.
- Launcher, config file, environment and runtime controls resolve to one effective image configuration; global disable stops new generation through every backend and consumer while preserving access to existing images.
- Every image job has attributable usage/cost evidence, the existing reports show backend/purpose/stage breakdowns, and unknown or subscription-only amounts are never disguised as zero-dollar actual charges.
- Local transformations follow the companion plan: one API, immutable derivatives, explicit ordered effects/color semantics, local-only configuration, resource accounting, and no hidden supplier calls or duplicated source charges.
- There is one execution owner, one request-key derivation, one attempt/retry policy and one existing money/storage authority per concern.
- Character creation and animation-frame safeguards survive, including audience and retry behavior.
- Unknown provider outcomes never trigger automatic duplicate paid generation.
- Old assets remain usable without a regeneration bill or broader access.
- Regression guards, focused integration/fault tests and independent browser acceptance pass on the same recorded build.
- Remaining policy changes, such as strict quality gates or new local spend caps, are separate decisions rather than hidden in this refactor.

## Source anchors

- [Current imagery execution](C:/Users/mreca/Desktop/dungeonfluxGame/crates/df-tools/src/gameplay/imagery.rs:1291)
- [Creator classification and generation](C:/Users/mreca/Desktop/dungeonfluxGame/crates/df-tools/src/gameplay/creator_portraits.rs:502)
- [Creator startup and dedicated accounts](C:/Users/mreca/Desktop/dungeonfluxGame/crates/df-tools/src/gameplay/creator_portrait_startup.rs:20)
- [Animation frame prior-attempt guard](C:/Users/mreca/Desktop/dungeonfluxGame/crates/df-tools/src/gameplay/character_clips.rs:385)
- [Animation direct image provider](C:/Users/mreca/Desktop/dungeonfluxGame/crates/df-tools/src/gameplay/character_clips.rs:1235)
- [Generic fal image adapter](C:/Users/mreca/Desktop/dungeonfluxGame/crates/df-providers/src/fal_media.rs:338)
- [Existing audience-aware request key](C:/Users/mreca/Desktop/dungeonfluxGame/crates/df-assets/src/media_request_key.rs)
- [Existing durable catalog contract](C:/Users/mreca/Desktop/dungeonfluxGame/crates/df-assets/src/catalog_store.rs)
- [Existing finance admission](C:/Users/mreca/Desktop/dungeonfluxGame/crates/df-finance/src/admit.rs)
- [Existing provider outcome classification](C:/Users/mreca/Desktop/dungeonfluxGame/crates/df-provider-api/src/outcome.rs)
- [Existing call-site/cost checks](C:/Users/mreca/Desktop/dungeonfluxGame/crates/df-tools/src/gameplay/cost_tests.rs)
- [Existing Codex text invocation and disabled image feature](C:/Users/mreca/Desktop/dungeonfluxGame/crates/df-tools/src/gameplay/codex_cli.rs:372)
- [Existing Codex process ownership primitives](C:/Users/mreca/Desktop/dungeonfluxGame/crates/df-tools/src/gameplay/codex_cli/windows_process.rs)
- [Existing launcher picture settings](C:/Users/mreca/Desktop/dungeonfluxGame/config/dungeonflux.example.toml:45)
- [Existing generation feature gates](C:/Users/mreca/Desktop/dungeonfluxGame/crates/df-tools/src/gameplay/media_policy.rs)
- [Existing launcher environment and credentials](C:/Users/mreca/Desktop/dungeonfluxGame/crates/df-launcher/src/server.rs:244)
- [Existing cost engine](C:/Users/mreca/Desktop/dungeonfluxGame/crates/df-tools/src/gameplay/cost.rs)
- [Existing cost report and bill comparison](C:/Users/mreca/Desktop/dungeonfluxGame/crates/df-tools/src/gameplay/dev_control/cost.rs)
