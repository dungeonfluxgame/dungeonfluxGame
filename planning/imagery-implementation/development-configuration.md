# Imagery development configuration (IMG-DEVELOPMENT-CONFIG-A1)

Date: 2026-10-28. Status: Proposed architecture amendment, no runtime change.

Scope: development-time configuration of the planned `df-imagery`
`ImageService` boundary only. It does not change image semantics,
domain intent, finance, catalog, or game rules. It preserves the full
target in [the unified image plan](../image-generation-architecture.md),
[the transform plan](../image-transformations-architecture.md), and
[the service composition contract](service-composition.md). Planning
and review only; root integrates and dispatches.

Source inspected (shell disabled, no commit hash observed; root freezes
the exact commit before implementation):

- `crates/df-imagery/src/config.rs:1-291` (actual `ImageConfig`,
  `GlobalMode`, `BackendId`, `AccountHandle`, `PurposeGate`,
  `BackendGate`, `TransformLimits`, `SchedulerBounds`, nonzero-only
  validation)
- `crates/df-imagery/src/config_overlay.rs:1-449` (actual supplied-input
  overlay: `resolve`, `apply_runtime`, `RuntimePatch`, `Provenance`,
  `OverlaySource`, `OverlayError`)
- `crates/df-imagery/src/request.rs:1-383` (`Purpose` x12,
  `ImageOperation`, `OutputProfile` split, `ExecutionMode`,
  `Freshness`, `Urgency`, injected `RequestLimits`)
- `crates/df-imagery/src/output.rs:1-250` (`Dimensions`,
  `RasterOutputSpec`, `OutputProfile::Provider/Local`, `Encoding`)
- `crates/df-imagery/src/transform.rs:1-269+` (`TransformSource`,
  `TransformSpec`, typed steps)
- `crates/df-imagery/src/policy.rs:1-99` (pure `decide` over a frozen
  `ImageConfig`)
- `crates/df-imagery/src/service.rs:1-320+` (`ImageService<J,C,I,A>`
  binds one snapshot at `new`; request/read/watch/cancel only, no
  `Ready`, no scheduler)
- `crates/df-imagery/src/journal.rs:1-120+` (durable receipts, caller
  `now_ms`, caller-minted ids)
- `crates/df-imagery/src/raster.rs:1-120` (private sync executor,
  injected `ResolveBytes`/`CancelCheck`, `TransformLimits` enforced)
- `crates/df-imagery/src/backends.rs:1-2` (`api`, `codex_cli` private
  modules named)
- `crates/df-imagery/src/lib.rs:1-77` (public surface; no config-owner
  export)
- Evidence `artifacts/tmp/imagery-20261027/wave2/config-a7-review.json`
  (`approve_bounded_overlay_robustness`, full-config feature **not**
  approved): 76 overlay tests pass, 25/26 independent probes pass; the
  one retained failure is the unchanged historical shipped-example
  `unwrap` that now correctly receives
  `InvalidField(pictures.picture_profile)`. Parsing and refusal passing
  is **not** runtime implementation.

## 1. Non-goals and user constraint

Flexible development configuration. Tuning defaults are defaults, not
hard ceilings: validation refuses only meaningless values (zero bounds
that refuse all work, stage deadline beyond job deadline, non-finite or
over-ceiling pixels) and genuinely unsupported capabilities. There is:

- No mandatory development restriction profile.
- No extra CLI enable gate, no hard-coded local-only rule, no mandatory
  API key for CLI use.
- No surprise spend cap from this amendment; cost tracking stays with
  the existing finance/cost owners. Optional operator alerts are a
  separate decision with explicit thresholds.
- No automatic live fallback from fixtures. A fixture miss is a typed
  miss, never a silent provider call.
- No test artifact may present fixture provenance as a live result.

Real capabilities and invalid values are still typed and refused.
Missing cost stays `unknown`, never zero.

## 2. Field to owner to consumer map

`Owner` is the crate/module that validates and enforces the field.
`Consumer` is who reads the frozen value. `Status` is current source
truth, not a plan claim.

| Field group | Actual type | Owner | Consumer | Status |
| --- | --- | --- | --- | --- |
| `mode` (`Live`/`LocalOnly`/`CacheOnly`/`Off`) | `config.rs: GlobalMode` | `policy::decide` | service admission | Supported: parsing + enforcement |
| `default_backend`, per-purpose `backend_override` | `BackendId` (`openai`/`fal`/`bfl`/`codex-cli`), `PurposeGate` | `policy::decide` | route selection (later) | Supported: selection record; provider dispatch not yet |
| purpose `enabled` x12, per-purpose output-profile selection | `PurposeGate` + `OutputProfileName` | `policy::decide`, caller/composition seam | all demand, recipe freeze (later) | Supported: typed + validated; `output_profile_for` resolves the concrete `OutputProfile` for `ImageRequest` construction; dispatch wiring is later executor work |
| backend `enabled`, `account` (nonsecret handle), `max_concurrent_jobs`, `model`, `generation_model`, `edit_model`, `alpha_model`, `endpoint` | `BackendGate`, `AccountHandle`, `ModelName`, `ApiEndpoint` | `policy::decide`, scheduler (later), private backends (later) | dispatch gate, private dispatch | Supported: gate + typed nonsecret model/endpoint; scheduling bounds pending scheduler lane; endpoint refused on `codex-cli`, which uses the Codex executable instead; private-backend consumption is later work |
| Codex `executable`, `workdir`, `cli_profile`, `agent_model` | `CodexSettings`, `CodexExecutable`, `JobWorkDir`, `CliProfileRevision`, `ModelName` | private Codex backend (later) | CLI dispatch | Supported: typed + validated; no executable allowlist, no version pin, no hash check, no mandatory API key; private-backend consumption is later work |
| named output profiles + `outputs.default_profile` | `NamedOutputProfile`, `OutputProfileName` over `OutputProfile` | caller/composition seam, recipe freeze (later) | output selection | Supported: typed + validated; requests keep their explicit `OutputProfile`; resolver maps purpose to profile |
| provider quality default | `QualityDefaults` (`jpeg_quality: LossyQuality`, `provider_quality: Option<GenerationQuality>`) | config validation and private backend composition | delivered encoding and provider generation | Typed and validated separately; named generation quality is `low`, `medium`, `high`, or `auto`. `none` clears it. Backend consumption requires explicit integration verification. |
| retry bounds | `RetryPolicy` (`max_attempts`, `retry_deadline_secs`, `backoff_secs`) | scheduler/worker (later) | safe retries | Supported: typed + validated; refusals are never retried with other wording |
| accounting mode/settings | `Accounting`, `AccountingMode`, `PriceFileRef` | finance/cost owners, private backends (later) | usage accounting | Supported: typed `record-only`/`hosted` mode plus nonsecret price-file reference; tracking never imposes a spend cap |
| `transforms.*` (`enabled`, `max_concurrency`, `max_decoded_bytes`, `max_output_bytes`, `queue_capacity`, `deadline_secs`) | `TransformLimits` | `policy::decide`, `raster::execute_transform` | local worker (later) | Supported: validation + executor bounds |
| `scheduler.*` (foreground/presence/background, pending jobs/bytes, queue age, job/stage deadlines) | `SchedulerBounds` | `config::validate` | scheduler (later) | Supported: typed + validated; durable enforcement is scheduler-lane work |
| `request_limits.*` (prompt chars, references, transform steps, composite layers) | `RequestLimits` | `request::validate`, `service::request` | all requests | Supported |
| output profile (`Provider` generated+delivered vs `Local` delivered) | `output.rs: OutputProfile` | `request::validate` | recipe freeze (later) | Supported: shape validation |
| per-purpose output overrides (quality, size, format/alpha, profile) | typed `RasterOutputSpec`/`Dimensions`/`Encoding` | request boundary | recipe freeze (later) | Supported as typed request content and named config-file profiles |
| per-request urgency, execution mode, freshness | `request.rs` enums | `policy::decide`, service | admission | Supported |
| quality policy (acceptance, corrective count, retry/backoff) | none in `config.rs` | unassigned | inspection (later) | Missing: typed owner required |
| credentials, secrets, API keys | never in `ImageConfig` | composition/egress | private backends at dispatch | Intentionally absent; see section 6 |

`ImageConfig` carries no secrets or credentials. Nonsecret development
settings — backend model names, API base endpoints for development
transport, the Codex executable command or absolute path plus job
workdir, quality/output defaults, named output profiles, retry bounds,
and accounting mode/settings — are typed fields consumed by private
executors. Overlay inputs naming anything outside the typed surface are
explicitly refused (`InvalidField` / `SecretRefused` / `UnknownField`);
that refusal is correct behavior, not a consumption gap to paper over.

## 3. Development profiles

Profiles are named starting points over the same typed `ImageConfig`.
Selecting one never grants capability, bypasses authorization, spends
money, or relaxes validation. No profile is mandatory. These
development-mode presets are TARGET: no mode-preset table or selection
flag exists in `ImageConfig` today; the rows below name intended
starting points only, and no test may present a preset name as
supported. They are distinct from named output profiles
(`outputs.<name>` plus per-purpose selection), which are supported and
typed (see section 5).

| Profile | `mode` | Backends | Notes |
| --- | --- | --- | --- |
| `dev-live` | `Live` | operator's configured default | Real dispatch once backends land; spend policy unchanged |
| `dev-fixture` | `Live` with fixture transport | fixture only | Deterministic, no network; default for unit development |
| `dev-offline` | `LocalOnly` | none required | Local transforms of authorized ready sources only |
| `dev-replay` (TARGET) | `CacheOnly` | none required | Reuse compatible ready results; a cache-only miss is `CacheOnlyMiss`, never `PreparedMiss` (`PreparedMiss` is the `PreparedOnly` execution-mode miss) |
| `dev-off` | `Off` | none | New submissions `Disabled`; reads and reporting still work |

A profile is TOML plus optional env plus an explicit launch/runtime
selection (section 4). Adding a profile adds no new enforcement
branch: it writes values the existing `decide` already understands.

## 4. Precedence

Low to high: built-in `ImageConfig::defaults` -> operator TOML file
text -> legacy launcher env pairs -> new `DF_IMAGERY_*` env pairs ->
explicit launch patch -> cumulative runtime patch. Within the file
layer, contradictory new and legacy spellings for one field conflict
only when their values differ; legacy and new environment are separate
layers, so a present new value wins silently over a legacy one and
records its provenance. This amends the unified plan's looser
same-layer wording deliberately: same-layer conflict applies to the
file layer only, never across the two env layers.

`resolve(config_text, env, runtime, revision)` in
`config_overlay.rs` implements the file/env/runtime order over supplied
inputs only: it never reads ambient environment, the filesystem, or the
network. `apply_runtime` applies a patch all-or-nothing against an
already-resolved snapshot. The section 8 owner (`config_control.rs`
`ConfigControl`) stores the validated base layers, the explicit launch
patch, and the cumulative runtime state separately, applying launch
between new-env and runtime and recording it as `OverlaySource::Launch`.
`Provenance { revision, fields, ignored }` records the winning
`OverlaySource` per tracked field and acknowledged no-counterpart
inputs; values and secrets never appear in it. Every error leaves no
partial result behind.

Legacy `[pictures]` aliases map to the same canonical fields at the
same layer; the existing mapping table in the unified plan stays
authoritative. The launcher (not the imagery crate) owns file discovery
(`config/dungeonflux.toml`), ambient env reads, and secret delivery.

## 5. Per-area settings (supported vs target)

Supported today means a typed field exists, validates, and a named
owner enforces it. Target means the field is named here with its owner
and acceptance test, but refused as unknown until its lane lands.
Never claim parsing a target name as supporting it.

- Purpose: `purposes.<label>.enabled`, `backend_override`. Supported.
- Backend: `backends.<id>.enabled`, `.account` handle,
  `.max_concurrent_jobs`, plus nonsecret `model` and API base
  `endpoint`. Supported as typed validated values. Credential binding
  stays at composition, never in config; `endpoint` is refused on
  `codex-cli`, which uses the Codex executable instead. Private-backend
  consumption is later work.
- Output: per-request `OutputProfile` + `RasterOutputSpec` fully typed,
  and named per-purpose config defaults (`outputs.<name>` bodies with
  generated/delivered dimensions, format/alpha, encoding quality, plus
  `outputs.default_profile` and per-purpose `output_profile`
  selection) are supported on every layer (file, environment, launch,
  runtime). Partial edits inherit unmentioned fields from the seeded
  body; `none` selects lossless WebP quality, drops a generated side
  (both sides together make a local profile), or clears the matte
  color. Provenance records the winning source per edited field
  (`outputs.<name>.<field>`). `ImageRequest.output` stays explicit;
  the `output_profile_for` resolver produces the concrete
  `OutputProfile` at the caller/composition seam. Wiring it into
  dispatch is later executor work. The named profile table has no
  fixed count ceiling; file, environment, and codec byte bounds are
  the memory bound.
- Quality/retries: supported as values. The global lossy default
  (`quality.jpeg_quality`, delivered-side only) plus the optional
  provider-generation default (`quality.provider_quality`, `none`
  clears back to the provider deciding) and safe retry bounds
  (`retry.max_attempts`, `retry.retry_deadline_secs`,
  `retry.backoff_secs`) are typed and validated; a refusal is never
  retried with other wording. The retry deadline is a finite budget
  capped by the job deadline at use
  (`ImageConfig::effective_retry_deadline_secs`), never a cross-field
  refusal, so historical short-deadline snapshots stay valid.
  Inspection acceptance policy and corrective-generation counts stay
  with the inspection/retry lane.
- Resolution: supported through named profiles. Generated size and
  delivered variant stay distinct keys; a local profile carries no
  generated size, and a local upscale never claims generated detail.
- CLI: supported. `codex.executable` (bare command or absolute path),
  `codex.cli_profile` revision, optional `codex.agent_model` (never an
  image-model guarantee), `codex.workdir`, concurrency (already typed
  per backend gate), account handle. No executable allowlist, no
  version pin, no executable-hash check, no bespoke home requirement,
  no mandatory API key for CLI use. The executable names `argv[0]`
  for a native argument array: an absolute path may carry spaces,
  apostrophes, or other shell punctuation without shell meaning, while
  a bare command keeps its strict spelling and a relative path with
  separators stays ambiguous and refused; NUL and control characters
  are always refused.
- Workdir: supported as an operator absolute or relative base
  (Unix, drive, or UNC root; spaces and apostrophes allowed; dot and
  dot-dot segments never escape). Job-owned input/output directories
  stage beneath it per job at runtime and stay worker-owned, never
  operator-addressed per job.
- Account/transport: nonsecret `account` handle supported; secrets and
  credential binding stay at intentional deployment transport policy
  (section 6), never in `ImageConfig` or domain logic.
- Timing: `scheduler.*` bounds supported as validated values; durable
  enforcement and the absolute job-deadline choke belong to the
  scheduler lane.
- Concurrency: `backend.max_concurrent_jobs`, `transforms.max_*`,
  `scheduler.*_limit` supported as values; permit ownership belongs to
  the scheduler/worker lanes.
- Transform: `transforms.*` fully supported as values and executor
  bounds. Named transform profiles (`hero_thumbnail` etc.) are target
  of the transform-config lane with codec-version compatibility keys.
  Raster itself enforces only its present enabled/bytes/pixel/cancel
  checks; queue, concurrency, and deadline ownership stays with the
  worker/scheduler lane.
- Cost: money policy stays with the finance/cost owners. The
  nonsecret accounting surface in this crate (`accounting.mode` plus
  an optional `accounting.price_file` reference for projections)
  is supported; report defaults and alert thresholds (alert-only,
  never a hidden cap) stay with the finance/cost owners. Tracking
  never imposes a spend cap.

## 6. Endpoints are deployment policy, not domain logic

No credential appears in `df-imagery` request/config types. Transport
selection stays intentional deployment policy at composition: the
launcher/operator channel names which transport a private backend may
open (fixture, loopback fixture server, authorized egress), and the
private backend opens only that. `ImageConfig` may carry a nonsecret
API base endpoint (`backends.<id>.endpoint`) so development
composition can point a private backend at a custom development
endpoint such as a loopback fixture server; the value parses as a
structural `http(s)` base URL with a validated host and optional
port, and userinfo, query, fragment, whitespace, and control
characters are refused, so credentials are never parsed out of it.
`ApiEndpoint` debug output stays redacted. Domain logic names
purposes, references, and output contracts; it never names production
hosts. User image-prompt text is content, not policy: it is never
scanned for URL substrings, and a configured endpoint or reference
value must never be confused with prompt content. Tests for this
rule: a fixture asserting no credential-bearing literal in
`config.rs`, `request.rs`, `output.rs`, `transform.rs`; endpoint
refusal tests for credentials, query, fragment, and bad hosts/ports;
a composition test proving the same frozen recipe resolves to the
fixture transport in `dev-fixture`.

## 7. Deterministic fixture and offline composition

Dev fakes compose behind the same `ImageService` public API through
crate-native seams, never through a public executor bypass:

- `ImageService<J,C,I,A>` already injects `ImageJournal`,
  `ServiceClock` (`now_ms`), `ServiceIds`, and `ImageAuthorizer`
  (`service.rs:56-88`). Deterministic tests inject a fake clock, a
  seeded id source, and an in-memory journal; no ambient time or
  randomness enters the crate.
- Local pixel execution injects `ResolveBytes` and `CancelCheck`
  (`raster.rs:29-33`); tests supply canned bytes and a scripted
  cancellation, never the filesystem.
- Provider dispatch (when its lane lands) arrives through the private
  `backends/` seam with a fake transport implementing the same private
  trait; the fake records dispatch identity (job, stage, attempt,
  config revision) and replays scripted bytes/usage. No public
  `raw_edit`, vendor factory, or callback-executor escape hatch is
  added; the regression guard from the unified plan covers it.
- Fixture provenance is explicit: fixture-derived bytes carry a
  `fixture` origin in diagnostics and test assertions, and cost
  projection records them as unpriced with `unknown` supplier usage,
  never as zero-dollar actuals. Fixture provenance never collides with
  live provenance: a fixture result is never recorded or asserted as a
  live result. Auth/account compatibility is retained: the nonsecret
  `account` handle still identifies the scheduling account on fixture
  and live paths alike. Two configs with the same content key
  share scheduling identity; different material content gets a different
  key (section 10). Cache-result reuse and fixture transport
  composition are TARGET, not implemented: no ready-result claim may be
  presented until the scheduler lane lands it.

## 8. Runtime configuration owner contract

Implemented crate-native owner: `crates/df-imagery/src/config_control.rs`
`ConfigControl`, beside `config_overlay.rs` (never inside `df-assets` or
`df-tools`), with exactly this surface:

```rust
fn build(config_text: &str, env: &[(&str, &str)], launch: &RuntimePatch, initial_revision: u64)
    -> Result<ConfigControl, OverlayError>;
fn read_effective(&self) -> ResolvedConfig;
fn validate(&self, candidate: &ImageConfig) -> Result<(), ConfigError>;
fn preview(&self, patch: &RuntimePatch) -> Result<ResolvedConfig, ConfigApplyError>;
fn apply(&mut self, patch: RuntimePatch, expected_revision: u64)
    -> Result<AppliedLocal, ConfigApplyError>;
fn reset(&mut self, expected_revision: u64) -> Result<AppliedLocal, ConfigApplyError>;
fn prepare_export(&self) -> PreparedExport;
fn save_prepared(prepared: &PreparedExport,
    persist: impl FnOnce(&ImageConfig, &Provenance, u64) -> Result<String, E>)
    -> Result<PersistedExport, ConfigApplyError>;
impl PreparedExport {
    fn revision(&self) -> u64;
    fn config(&self) -> &ImageConfig;
    fn provenance(&self) -> &Provenance;
}
```

`ConfigApplyError`: `StaleRevision { expected, current }`,
`Invalid(OverlayError)`, `RevisionOverflow`, `PersistFailed`. Semantics:

- `build` validates the supplied TOML/env/launch layers up front and
  stores base, launch, and runtime state separately. It reads nothing
  ambient; an invalid layer yields no owner.
- `read_effective` returns the live snapshot plus its `Provenance`
  (winning source per field, revision) as an independent copy; later
  updates never mutate a returned snapshot. Credentials redacted
  (absent by construction).
- `validate` and `preview` are pure: they mutate nothing and leave the
  complete state unchanged on any error.
- `apply` merges a partial typed runtime patch cumulatively (`None`
  leaves a prior runtime value in place; `Some(None)` on a purpose
  backend override explicitly clears it), recomputes base plus launch
  plus runtime, validates, and only then commits at a checked
  `expected_revision + 1`. Invalid input, a stale fence, or a counter
  at `u64::MAX` leaves the complete state unchanged.
- `expected_revision` is an optimistic fence: a stale caller gets
  `StaleRevision` and rereads; it never overwrites a newer operator
  change. The revision counter is checked and never wraps.
- `reset` clears runtime overrides back to base plus launch at a fresh
  revision; launch overrides and their `Launch` source stay.
- `AppliedLocal` reports LOCAL application only: the new revision is
  committed locally and visible to later reads, but no scheduler or
  egress owner has acknowledged it. A disable receipted this way must
  not be presented as globally propagated; see the fence below.
- Save ownership belongs to the launcher: `prepare_export` captures
  config, provenance, and the exact revision as one tear-proof triple,
  and `save_prepared` persists only that capture through an injected
  callback that receives the exact captured revision, never a reread of
  newer state. A callback failure surfaces `PersistFailed`; only a
  callback success reports a `PersistedExport`. The crate never rewrites
  an operator TOML file directly. A runtime-only change is temporary
  until the launcher saves it.
- Configuration-only changes never require a rebuild; services pick up
  new revisions once the service-binding lane consumes them (see gap).

Gap (current source truth): `ImageService::new` still binds one
snapshot for the service lifetime (`service.rs:71-88`); binding service
snapshots to owner revisions is later integration work owned by root.
There is no `Ready`, no provider dispatch, and no global dispatch
acknowledgement yet. Claiming global propagation on an `AppliedLocal`
receipt would contradict the code. Existing fixed `PROTOCOL_LIMITS`
(`service.rs:39-51`) still cap request values at admission and bound
exact replay however large the configured `request_limits` grow; that
protocol ceiling is a separate compatibility fact, not a tunable, and
this amendment adds no caps.

Dispatch fence for disable. Configuration application and stage
dispatch share one serialized gate: once a disable acknowledgement is
returned, no queued stage may start under an older enable snapshot.
In-flight jobs hold an immutable snapshot reference; new admissions
use the new revision. Started stages drain without beginning another
effect stage; publication completes only under the frozen acceptance
contract. In hosted deployment every affected dispatch owner
(scheduler, local workers, hosted egress) reports its applied revision
before the ack; an update not yet propagated returns the local
`AppliedLocal` receipt with global propagation pending, never a false
global success. `PropagationPending` is the target global status name
for exactly that state: local revision committed, global ack pending.
A scoped disable (one purpose or backend) never stops coalesced
interests that remain enabled; only the disabled scope refuses new
stages. A separate explicit stop-active action terminates owned CLI
processes and marks uncertain remote outcomes `unknown`; it makes no
refund claim. Re-enable with `live`; cancelled/stale/unknown attempts
are never automatically replayed.

Per-request output overrides are limited to the already-typed content
fields (`OutputProfile`, `RasterOutputSpec`, `Dimensions`, `Encoding`,
`TransformSpec`) within the caller's purpose and authorization.
Overrides can never select providers, accounts, credentials,
endpoints, or money policy; those resolve from the frozen
configuration at composition.

## 9. Diagnostics and replay

Status/reporting surfaces expose, with credentials absent: effective
revision and winning source per field, backend readiness, queue/active
counts, reasons for disabled/unavailable purposes, fixture vs live
provenance, cache-hit vs fresh-launch counts, coalesced interests, and
unknown/unpriced usage. Reports distinguish ready-hit reuse (zero new
generation cost, original cost retained by lineage) from new dispatch.
Replay (`ExecutionMode::Replay`, `PreparedOnly`) is lookup-only: a miss
returns the typed miss without minting ids or touching the journal
(actual service behavior today). Diagnostics never log prompt text,
reference bytes, or secrets; `TextPrompt` already reports length only.

## 10. Minimal examples

Both examples parse through the current `resolve` only where noted.
Labels are normative: do not present the target example as passing
today.

Current-supported (passes today: typed gates, overlay precedence,
provenance, refusal preservation):

```toml
[imagery]
mode = "local-only"
default_backend = "codex-cli"

[imagery.purposes.scene_plate]
enabled = true

[imagery.purposes.animation_frame]
enabled = false

[imagery.backends.codex_cli]
enabled = true
account = "dev-cli"
max_concurrent_jobs = 2

[imagery.codex]
executable = "codex"
workdir = "artifacts/tmp/imagery-jobs"
cli_profile = "codex-images-v1"

[imagery.backends.openai]
enabled = false
model = "gpt-image-1"

[imagery.outputs]
default_profile = "hero-thumbnail"

[imagery.outputs.hero-thumbnail]
width_px = 96
height_px = 144
format = "jpeg"
quality = 65

[imagery.transforms]
enabled = true
max_concurrency = 2
max_decoded_bytes = 268435456
max_output_bytes = 16777216
queue_capacity = 64
deadline_secs = 30

[imagery.retry]
max_attempts = 2
retry_deadline_secs = 170
backoff_secs = 2

[imagery.accounting]
mode = "record-only"

[imagery.quality]
jpeg_quality = 70
provider_quality = "high"

[imagery.outputs.hero-lossless]
width_px = 64
height_px = 64
format = "webp"
quality = "none"
```

Target, not yet supported (each unsupported name is refused today;
each needs its typed lane; do not simplify the example to hide them):

```toml
[imagery]
mode = "live"
default_backend = "codex-cli"

[imagery.costs]
# TARGET (finance owner, alert-only thresholds): remains outside this crate
```

Tests validating the semantics (no live calls, no ambient I/O):

- Overlay: file/env/launch/runtime precedence with provenance on every
  winning field; legacy alias equality and contradictory-alias conflict;
  unknown/duplicate/secret inputs refused without value echo; invalid
  patch preserves the prior snapshot byte-for-byte.
- Policy: each knob reaching its owner (`mode`, per-purpose enable,
  per-backend enable, `transforms.enabled`, execution modes) returns its
  typed refusal; disabled backend never reroutes; queue/deadline/
  concurrency edits leave content keys unchanged while output/prompt/
  route/acceptance edits change them.
- Identity: two configs with identical material content derive the same
  scheduling key; any material change (dimensions, encoding, ordered
  steps, route revision) derives a different key.
- Fixture: deterministic fake transport behind the same `ImageService`
  API replays scripted bytes/usage across restart; fixture provenance
  recorded; no live fallback on miss; unpriced usage stays `unknown`,
  never zero.
- Fence: disable acknowledged only after every dispatch owner reports
  the new revision; a queued stage admitted pre-disable never starts
  post-ack; in-flight snapshot immutable; stale `expected_revision`
  rejected.

## 11. Staged atomic tasks

Dependency order; each task names exact source scope and acceptance
evidence. Codec/version compatibility is preserved on every typed
expansion (new variants/fields are additive; canonical serialization
rejects non-finite floats, normalizes negative zero, and versions
algorithm/profile revisions). Root assigns owners and numbers
migrations.

1. T1 Config-file owner mapping. Scope: launcher file discovery plus
   `config_overlay::file_layer` names only; no `ImageConfig` widening.
   Accept: the current-supported example resolves with per-field
   provenance; contradictory alias conflict and secret refusal tests
   pass; unsupported target names still refused.
2. T2 Named output profiles (config-owner portion landed: typed
   `outputs.<name>` tables with generated/delivered dimensions,
   format/alpha/encoding quality, `outputs.default_profile` plus
   per-purpose selection, and the `output_profile_for` resolver).
   Scope: new typed profile tables in `df-imagery` config plus
   `output.rs` validation; config codec at v3 with v1/v2 snapshot
   decoding. Transform profiles stay with the transform-config lane.
   Accept: complete example round trips, meaningful nondefault values,
   per-purpose profile resolution tests, v1/v2 compatibility and v3
   noncollision tests, JPEG-matte and pixel-ceiling refusal tests.
3. T3 CLI/backend composition settings (config-owner portion landed:
   typed nonsecret CLI table and per-backend model/endpoint settings).
   Scope: typed nonsecret CLI table (`codex.executable` as a bare
   command or absolute path with no allowlist, CLI profile revision,
   workdir policy, agent-model field) plus per-backend model and
   development endpoint settings; credentials stay at composition.
   Private backend wiring is later work. Accept: compatible newer CLI
   accepted, no version/hash admission criteria, no executable
   allowlist, no mandatory key; workdir-escape fixture refused;
   endpoint on `codex-cli` refused.
4. T4 Runtime owner + fence. Scope: local owner module implemented
   (`config_control.rs`: `build`/`read_effective`/`validate`/`preview`/
   `apply`/`reset`/`prepare_export`/`save_prepared`, revision fence,
   `Launch` provenance, launcher-owned revision-consistent save).
   Remaining target: service snapshot binding plus scheduler/worker/
   egress applied-revision reports and the global dispatch fence.
   Accept: invalid update rollback, stale-revision, overflow, reset,
   export, and `AppliedLocal`-is-local-only tests in
   `config_control_tests.rs`; disable-ack and drain tests on real PG
   adapters with fake transports stay with the fence lane.
5. T5 Fixture/offline composition + diagnostics. Scope: crate-native
   fake transport, seeded clock/ids, provenance + cost-projection
   labels, status/reporting views. Accept: deterministic replay,
   explicit fixture provenance, unknown-not-zero, no-fallback-on-miss
   tests; diagnostics show revision, readiness, and disable reasons
   with secrets absent.
6. T6 Quality/retry/cost-alert tables with their owners. Scope:
   inspection/retry lane types plus finance-owned alert thresholds
   (alert-only). Accept: corrective-count and safe-retry tests;
   no hidden cap; unknown liability retained and visible.

## 12. Handoff

Results, evidence, and follow-ups are recorded in
`artifacts/tmp/imagery-20261027/wave2/development-config-handoff.md`.
No test or check was executed in this planning session (shell
disabled); every verdict above cites inspected source lines or the
named review JSON, and every target is labeled as not-yet-supported.

## 13. Typed-config implementation status (2026-10-28)

The owned config, overlays, controller, and codec implement native operator
paths, operation-specific API models, named generation quality, named output
profiles, per-field provenance, retry limits, and immutable export snapshots.
`ConfigControl` remains a local configuration owner; production scheduler and
private executor composition must verify consumption and global acknowledgement
at their own boundary. This lane does not claim live provider execution.

The A2 implementation was rejected by independent running-output review. Its
claims about numeric provider quality, complete matte clearing, and the absence
of a need for distinct Fal routes are superseded by the actual A3 contracts below.
A3 regression evidence and the full handoff are retained in
`artifacts/tmp/imagery-20261027/wave2/typed-development-config-a3-sol-handoff.md`.

## A3 configuration migration and boundary rules

Generation quality uses `low`, `medium`, `high`, `auto`, or `none` in file,
environment and runtime overlays. JPEG/WebP compression quality remains an
independent integer from 1 to 100. New generation-quality overlays reject
numbers. Config codec version 3 encodes the named enum and per-operation model
selections; versions 1 and 2 remain readable. Historical version 2 numbers
round-trip as `GenerationQuality::LegacyNumeric(LossyQuality)` without guessing
a corresponding named setting. A backend that does not support such numbers
must refuse them explicitly before dispatch. Operators migrate by selecting a
named quality; snapshot decoding alone never spends money or calls a backend.

`backends.<id>.generation_model`, `.edit_model`, and `.alpha_model` each override
one operation. Composition checks the operation-specific selection first, the
legacy `.model` fallback second, and the backend's built-in default last. Fal's
three distinct routes must be mapped separately to its existing `with_models`
constructor. Environment names end in `_GENERATION_MODEL`, `_EDIT_MODEL`, or
`_ALPHA_MODEL`; dotted runtime paths use the same file field names. Higher layers
can clear optional model selections with `none`. The Codex agent model remains
`codex.agent_model`, which does not imply an image-model guarantee. Generic API
image-model overrides are not interpreted as Codex agent-model selections. They
remain lossless snapshot data for historical replay; Codex composition must
explicitly refuse unsupported API model fields before starting a process.

Codex executable and workdir selections preserve native Unicode, spaces in
absolute executable paths, inert argv punctuation, and long path segments. They
remain bounded to 4096 bytes and reject NUL/control characters. Bare executable
commands contain no whitespace; path executables must be absolute. Workdir
bases reject empty, dot, and dot-dot segments. There is no ASCII allowlist or
64-byte segment ceiling. These strings are process arguments, never shell code.

API base endpoints permit only structural HTTP(S) host/port and URI path
characters, with well-formed percent escapes and real IPv6 literals. Credentials,
queries, fragments, controls, backslashes, and malformed escapes are refused.
Endpoint diagnostics redact the value.

Profile edits merge per field over the same named lower-layer profile. Explicit
`quality = "none"` selects lossless WebP; clearing both generated dimensions
selects a local output profile. `matte = "none"` clears inherited matte handling
to opaque unless an explicit alpha selection supplies the resulting contract;
explicit `alpha = "matte"` still requires a color. Reset removes runtime edits
and restores launch/file/environment values, including encoding and alpha.


## Mounted gameplay authority and custody controls (2026-10-29)

The native gameplay host owns one live `ConfigControl`, one image runtime and one supplier
quota registry. Each demand captures the current effective configuration and its acknowledged
revision. Configuration rebuilds retain the same quota registry; cached successes, reads and
process-local display state cannot reset supplier quota. The existing developer view exposes
only aggregate closed/open/probing account counts, with unavailable state explicit. The display
notice aggregates weak registrations owned by actual live hosts and has no admission authority.

`DF_IMAGERY_MAX_CURRENT_GRANTS` is a host deployment setting, separate from semantic image
configuration. It defaults to 512 and accepts any positive value representable as a native
`usize`. Changing it requires a host restart. `/dev/imagery` exposes the configured maximum,
registered count and capacity-refusal count. A full registry returns the typed
`AuthorityCapacity` availability failure for a new owner. Updating an existing owner's proof
still requires fresh canonical authorization and remains permitted at capacity. No entry is
evicted: pending work and unknown supplier outcomes keep their captured proof custody. This
setting introduces no new price, payer, credit, dispatch or image-cache identity.

The legacy parsed provider deadline seeds `scheduler.stage_deadline_secs` before file,
environment and runtime operator layers. Explicit legacy environment selection retains its
legacy provenance, and the new scheduler setting follows normal precedence. Credential values
are not retained in legacy imagery settings; the native host resolves them through trusted
account custody when required. The shared installed content rating is passed once to the host,
then frozen into each request's versioned policy.

Generic Fal and BFL accounts use their generic deployment keys. They never automatically
prefer creator-only keys. Explicit `creator-fal` and `creator-bfl` account aliases may use their
dedicated key, with the existing generic-key fallback. `creator.bfl_account` selects the typed
account for the classified alternate BFL route; `none` uses the configured BFL backend account,
never the primary OpenAI account. `DF_IMAGERY_CREATOR_BFL_ACCOUNT` is its environment spelling.
The host may seed the dedicated alias only below operator layers when that dedicated key
exists; explicit operator `none` remains authoritative. `DF_IMAGERY_CREDENTIAL_MAP` maps account
handles to environment-variable names, never to values stored in `ImageConfig` or diagnostics.
Configuration snapshot version 12 preserves the explicit alternate account, version 11 quota
policy and every prior snapshot layout/default.

Mural and room cooldowns reobserve the same durable operation. Domain cooldowns do not create
another paid demand or invalidate a second image queue. The unified runtime owns configured
provider retry policy and attempt budgets; an unknown send is not permission to resend. Only
an explicit caller regeneration intent supplies a stable `Freshness::Regenerate` nonce.

## Retained image bounds and public artwork identity (2026-10-29)

`transforms.max_output_bytes` is frozen with each admitted demand. New demands use the
acknowledged current configuration; older admitted work keeps its validated write bound.
The native publisher passes that retained bound to the object store for each write without
changing the ordinary store cap. Uploads and reviewed imports retain their own limits.
Authorized generated Ready reads resolve the exact winning publication/allocation/job and
use its immutable output bound, including after a lower setting and process restart. Missing
or corrupt generated lineage is unavailable; it never borrows the reviewed-import allowance.

Initial public canonical person requests use prompt revision DF02_0001 and item requests with
public artwork metadata use DF02_0005. The complete public prompt, references, output policy
and configuration remain semantic inputs. The typed public specification is distinct from
the current run selection fence, which remains in the full intent and operation identity.
Explicit person regeneration preserves the existing full-binding v1 path and is not eligible
for artifact reuse. Shared artifacts use an asset alias with a separate checksum domain;
reads authorize the current campaign and exact retained sources and expose no foreign job,
operation, interest or supplier usage. Reuse stays within the same actual catalog/database;
local demo databases are not federated.

These source contracts are not a claim of observed acceptance. The coordinator records the
actual native/PG checks separately. The proposed campaign-funded hosted factory is still
unmounted pending specific approval and actual trusted startup context; local operation does
not fabricate canonical customer sessions, machine grants or creator consent.
