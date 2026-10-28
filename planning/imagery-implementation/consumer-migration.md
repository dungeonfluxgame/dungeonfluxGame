# Consumer migration map: all imagegen/raster consumers to df-imagery (IMG-CONSUMER-MIGRATION-MAP-A2)

Source: `703267289d580d933e07b623fbea6fbba4ec4578`.
A1 was rejected per `artifacts/tmp/imagery-20261027/wave2/consumer-migration-review.json`;
this revision fixes every listed defect. The checker reaches zero bypass on the
current tree in default mode (legacy rows reported, nonfatal) and fails strict
until the section 10 rows are migrated and their budgets removed.
Governing: `planning/image-generation-architecture.md`, `planning/image-transformations-architecture.md`,
`planning/imagery-implementation/contract.md`, `planning/imagery-implementation/operation-store.md`.
`planning/imagery-implementation/service-composition.md` is an unapproved draft: it is read
for ordering context only, and no call shape in this map is sourced from it.
Base: accepted `df-imagery` request foundation (`crates/df-imagery/src/request.rs`),
recipe keys `df-imagery/3` (`crates/df-assets/src/media_request_key.rs:74-183`),
journal candidate (`crates/df-imagery/src/journal.rs`, `crates/df-persistence/src/imagery_store.rs`).

Status: inventory + boundary checker only. No app source edits. Service contract owned elsewhere;
no caller is edited against an invented API. Root owns all `lib.rs`/parent module registrations,
manifests/lock, and runtime migration harness; exact snippets for those are in section 8.

## 0. Preserved invariants (do not migrate away)

1. A hero can be created with no portrait; failures leave creation usable.
2. Creator polling never creates work (`handle` with `dispatch=false` is a read).
3. Private drafts stay private: only the owning seat's grant reads the draft image;
   accepted sharing follows existing audience authorization.
4. Video work stays with clips: `character_clips.rs` keeps clip orchestration/encoding;
   only its guard/fallen still frames move to `ImageService`.
5. No cross-provider fallback after refusal/uncertainty; no silent custom-rules change;
   classification categories, rating behavior, models, prompt text, quality, formats stay
   versioned compatibility policies in the first migration.

## 1. Creator chain (distinct consumer, kept distinct)

### 1.1 `creator_portraits.rs`: `Portraits::{handle, accepted, image, approved_reference}`

- Method/identity: `handle(service, Request<HeroPortraitRequest>, dispatch: bool)` binds
  `operation_id` + `draft_revision` to a stable operation hash
  (`client_diagnostics::operation_hash`); `dispatch=true` admits one operation binding,
  `dispatch=false` only polls. Entry keyed by operation string in `requests: BTreeMap<String, Entry>`;
  `Entry { diagnostic_operation, fingerprint, revision, resource: ResourceKey, binding: [u8;32],
  appearance, sequence, progress, grant: [u8;32] }`.
- Prompt: `prepare_portrait` builds `(public, design)` via
  `creator_catalog::portrait_context` + `with_scenery`; fingerprint is
  `SHA256(public)`. Generation prompt is `imagery::prompt::creator_portrait`
  (opaque 1024x1536 portrait; never the transparent canonical-cutout profile).
- Output: opaque portrait bytes through `Imagery::generate`; `accepted()` attaches only when
  the matching public design's resource is `ResourceStatus::Ready`, bytes non-empty and
  `<= MAXIMUM_PORTRAIT_BYTES (6 MiB)`, returning `Attachment { key, digest, bytes, revision }`.
  Oversize/vanished cache returns `Ok(None)` with `attach outcome=none` audit, never blocks creation.
- Default: portraits disabled (`Portraits::new(None, None, None, cache)`) when
  `DF_GAMEPLAY_CREATOR_PORTRAITS` is absent/`off`; startup errors on other values.
- Privacy: `grant` capability per draft; `accepted()` filters by `binding` + exact `appearance`;
  private draft unreadable by another seat/display; audit lines carry `seat_hash`, never prompts.
- Tests: Generate/poll/reconnect/repeat/explicit-regenerate; edit-while-pending;
  stale completion; no-portrait join; matching-design acceptance; cross-seat read refused;
  `creator_portrait_route_tests.rs` routing matrix.
- Migration: keep `handle`/`accepted`/grant/poll shape; replace `Imagery::generate` +
  `hub().status` with `ImageService::submit/status` using
  `Purpose::CreatorPortrait/Primary`, `Generate`, `ImageOwner::Draft { account }`,
  `Freshness::ReuseCompatible` (explicit regeneration uses `Regenerate { nonce }`),
  `ExecutionMode::Live`. Prompt/template version + route/quality frozen in recipe.

### 1.2 `creator_classification.rs`: `Classifier::classify`

- Method/identity: one Jev choice per owned creator operation:
  `classify(public: &str, operation: &str)`; `VERSION = "creator-content-v3-jev-latest"`;
  7 closed `CRITERIA`; `Assessment { Category }` maps to `Route::{General, Bfl, Revise, Uncertain}`
  via `route_for_rating(mature: bool)`; `>8 KiB` public text refuses.
- Prompt: fixed `INSTRUCTIONS` (quoted appearance is data, never instructions); state is
  `{ public_appearance, classification_contract, operation }` only (no name/backstory).
- Output: `Route::General` (general appearance) or `Route::Bfl` (adult-detail/nudity/violence
  with mature flag, else `Err`), `Revise` (unsupported), `Uncertain`. Recorded + reused for
  the same operation; refusal/uncertain stops before generation.
- Default: missing dedicated Jev credential fails startup (`creator_portrait_startup.rs:112-116`);
  classifier unavailability surfaces `classifier_unavailable`, never a guess.
- Privacy: public appearance only; audit logs `category` + `ms`, never the description.
- Tests: closed-category routing incl. choice 6 needs no mature flag; out-of-range choice errs;
  loopback Jev asserts model `jev-latest`, exact state fields, 7 criteria, no name/backstory.
- Migration: classification becomes the admitted paid stage inside the frozen recipe
  (own admission/attempt, durably attached, replay reuses decision). No free preflight;
  no direct `SystemOneChoiceProvider::choose` outside the service after migration.

### 1.3 `creator_portrait_startup.rs`: `start(hub, current)`

- Method/identity: composes isolated providers; requests no paid work. Reads
  `DF_GAMEPLAY_CREATOR_PORTRAITS`, `DF_GAMEPLAY_IMAGERY_PROVIDER`,
  `DF_GAMEPLAY_CREATOR_FAL_KEY/_OPENAI_KEY/_BFL_KEY/_JEV_*`, `DF_GAMEPLAY_DISABLE_OPENAI_API`,
  `DF_GAMEPLAY_CODEX_MODE`. Today constructs `NativeImageEditProvider`,
  `NativeFalImageProvider`, `NativeBflImageProvider`, `SystemOneChoiceProvider` directly
  and `Imagery::new_in_hub` twice (primary + alternate) with `Limits { timeout: 210s, attempts: 1 }`.
- Prompt/output/default/privacy: inherits 1.1-1.2; general/BFL routing under rating rules;
  one logical creator attempt; dedicated credentials per route.
- Tests: startup refusal matrix (bad gate values, guarded OpenAi, missing keys).
- Migration: emit typed `df-imagery::ImageConfig` snapshot (backend/account bindings,
  creator route policy, `creator_portrait` purpose enablement) at bootstrap; delete all four
  direct provider constructors here. This file is a legacy allowance in the checker until removed.

### 1.4 Private acceptance (`accepted` + `approved_reference`)

- `accepted(binding, character)`: matches committed public design + scenery, picks max-sequence
  ready entry for `(binding, appearance)`, verifies bytes via `image(key)`, returns digest-bound
  `Attachment`; mismatch/miss returns `Ok(None)`.
- `approved_reference(asset)` / `approved_reference_at(asset, root)`: reads only the committed
  `AssetReference` (`64-hex key`, `AssetKind::Image`, `0 < bytes <= 6 MiB`), length + `SHA256`
  verified, `spawn_blocking` file read; returns `ReferenceImage(Jpeg, bytes)`.
- Migration: `accepted` keeps its match rule; reference reads become authorized `SourceRef`
  pins (`AssetId` + digest + admitted audience) with catalog authorization, never raw paths.

### 1.5 Primary imagery bootstrap (`imagery.rs` + shared registry)

- `ImageRoute::configured(&configuration)` (`imagery.rs:731-760`): reads
  `DF_GAMEPLAY_IMAGERY_PROVIDER` through `media_policy::image_provider` plus `FAL_KEY`
  presence. Unset/empty selects OpenAI; `flux` requires a non-empty `FAL_KEY`, otherwise the
  media-policy gate refuses. OpenAI is the default route; FLUX is explicit only. The
  configuration comes from `startup_configuration::imagery_settings` (`ImageryConfiguration
  { credential, model, quality, profile, interval, deadline, cache_dir, rating }`):
  invalid `DF_GAMEPLAY_IMAGERY*` values refuse without echo, empty counts as unset, missing
  credential with no independent provider yields `Off(NoCredential)`.
- `Imagery::start/start_in_hub` (`imagery.rs:876-927`): `Off` returns a disabled service that
  registers nothing; `On` builds one `ImageRoute::configured` provider with the validated
  `deadline`, attaches the spend ledger, and wires optional head-locator/photo-check stages
  behind `DF_GAMEPLAY_DISABLE_OPENAI_API`. These two `imagery.rs` constructors are the section
  10 legacy rows for that file; their fingerprints are the exact live configs in the call.
- `registered_fal_media_provider` (`df-providers/src/registry.rs:380-388`): the selected FAL
  adapter factory (`FalMediaConfig`, credential, `NativeHttpLimits`). The generic FLUX Schnell
  image entry migrates to the service and is then deleted; the same factory remains available
  for video (`character_clips.rs:1497-1511` builds `FalMediaFamily::Video` with `KLING_O1_IMAGE_TO_VIDEO`
  through this factory). No image bypass is retained for the migrated entry.
- Tests/removal: `imagery_tests.rs:931` settings, `imagery_live_tests.rs:56` opt-in live
  acceptance (`FluxPerson`, `#[ignore]`), and the `character_clips.rs:1510` video path pin the
  image/video split. Removing a row means migrating its call site and deleting its budget entry
  in the same change; the video factory use is not an image row.
- Migration: emit one typed `ImageConfig` snapshot (backend/account bindings, creator route
  policy, `creator_portrait` purpose enablement, model/quality/profile/deadline/cache/rating)
  at bootstrap; replace `ImageRoute::configured` provider construction and
  `registered_fal_media_provider` image dispatch with `ImageService::submit/status` using
  `Purpose::{CreatorPortrait, CanonicalPerson, CanonicalRoom, ScenePlate, ...}/Primary`,
  `Generate`, `ImageOwner::Run`, `Freshness::ReuseCompatible`, `ExecutionMode::Live`.

## 2. Canonical worker (`living_scenes/canonical.rs` + `mural.rs`)

- Method/identity: run-owned worker; `References::select(target, source, revision)` binds
  `canonical-reference-v1|scope|target|revision` (SHA256); `take_jobs_up_to(available)` priority
  current-room > hero > present NPC > room > other NPC > mural; `Job::Canonical(Work { target,
  scope, binding, source, regenerate })`. Targets: `Mural`, `Hero(EntityId)`, `Npc(String)`,
  `Room { place, revision }`, plus `ReviewedRoom { asset }` and on-demand `Cast { member, canon }`.
- Prompt: `imagery::canonical_prompt::{canonical_character, canonical_npc, canonical_room_content,
  canonical_room_with_context, canonical_cast_member}` from accepted identity/appearance/canon;
  only committed harmless `Backfill/HarmlessMissingDetail` canon enters; secret terms filtered.
- Output: canonical person/room stills; `hero_path` falls back to `/gameplay/creator-portrait/<key>`
  when a portrait asset exists; room retry only after `ROOM_RETRY_COOLDOWN (60s)` via explicit
  invalidation, never a side retry loop.
- Default: imagery off yields `ImageFailure::Off`; missing lore yields empty canon, not a guess.
- Privacy: scope+revision fenced selection; stale completion rejected before attach.
- Tests: `canonical_tests.rs`, mural tests below, `hero_identity_fixture`.
- Migration: keep domain intent + reference selection; submit as `Purpose::{CanonicalPerson,
  CanonicalRoom}/Primary` `Generate` with `ImageOwner::Run`, frozen source binding; mural keeps
  versioned roster binding (section 3).

## 3. Party mural (`living_scenes/mural.rs`: `admit_mural/refresh_mural/render_mural/project_mural`)

- Method/identity: `admit_mural` settles party from a real scene projection; `refresh_mural`
  requires full roster (`mural_expected_count`), every hero ready, non-empty; revision is
  `party-mural-v1|binding:key|...`. Roster change removes the old entry (never shows stale party).
- Prompt: `party_mural(&[FluxPerson], binding)` over a locally composed `party_sheet(&references)`;
  `ScenePlate` carries `SceneParty` metadata through the photo/count/custody gate.
- Output: one mural plate; reconnect/restart reuses (`take_jobs` empty when bound); depiction
  revision creates exactly one new mural.
- Default/privacy: waits for every hero; approved hero references only; model gate requires
  `Gpt25Flare` today (compatibility policy, versioned, not silently changed).
- Tests: `hs54_transient_mural_reference_miss_requeues_only_after_cooldown`,
  `party_mural_requests_once_reuses_after_reconnect_and_restart_and_changes_with_roster`,
  `party_mural_waits_for_every_hero_and_rejects_old_roster_completions`.
- Migration: `Purpose::PartyMural/Primary` `Generate` (reference-prep sheet = local pipeline
  stage); roster/binding rule unchanged.

## 4. Plates (`living_scenes/plates*.rs`)

### 4.1 Direct (`plates/direct.rs`: `Plates::direct/clear_direction/render_directed/finish_directed`)

- `direct(text, keep)` admits via session owner only: requires imagery on, current scene,
  not in combat, stageable scene; bounded `jobs.try_send(Job::Directed)`; publishes pending
  `Updating scene...` at once. `render_directed` uses `fresh_request(..., Some(text))`,
  `key_for`, `generate("plate:<scene_key>")`; speaker plates add measured/predicted placement.
  Combat or scene change converts the result to `Interrupted`.
- Migration: `Purpose::ScenePlate/Primary` `Generate` with the same admission/bounds.
  The directed fresh path (`living_scenes/plates.rs:3708`) builds `ImageKind::ScenePlate`
  with the direction folded into the prompt (`with_development_direction`); there is no
  base still to edit, so prescribing `Edit` would invent a base. Plate edits stay with
  the settled-edit path in 4.2 (`ImageKind::ScenePlateEdit`).

### 4.2 State (`plates/state_images.rs`): settled edits, item/held-item views, expressions

- Settled visible changes share one edit (`Pending` + `SETTLE 3s`, `BAKE_INTERVAL 15s`,
  `SCENE_CADENCE 30s`, `MINIMUM_PRESENTATION 25s`, `MAXIMUM_PLATE_RENDERS 3`,
  `CHAIN_REFRESH_COOLDOWN 60s`); `state_edit/graph_edit/story_beat_edit` prompts.
- Items: `item_image_request(rating, thing, detail)` one request per object state;
  `enrich_item_card` decorates only the durable selection; stale purchase/examination fenced.
  Held-item/inspection-hands variants use `ImagePriority` + `ItemFocusPose::{Recorded, InspectionHands}`.
- Expressions: `prompt::{cast_expression, hero_expression}` + `hero_cutout/hero_portrait`.
- Combat: kill stills via the same plate path; `in_combat` plates are refused in `direct`
  (battlefield owns the fight view).
- Migration: `Purpose::{ScenePlate, SceneEdit, Expression, ItemCloseUp}` with matching roles;
  keep settle/bake/cadence/presentation/chain-refresh semantics byte-for-byte initially.

### 4.3 Ahead (`plates/ahead.rs`): speculative successor plates

- `queue_ahead`/`replace_beat_ahead`/`poll_ahead_locked`/`reject_stale_ahead`/`finish_ahead`/
  `note_arrival`; bounded `MAXIMUM_SUCCESSORS`; Beat vs Travel/Generated/Outcome sources;
  current always admitted before speculative work; stale branches cancelled, never promoted.
- Migration: `Urgency::Speculative` with bounded share; disable via
  `[imagery.prefetch].enabled=false`; no late prefetch replaces the current plate.

## 5. Character clips (`character_clips.rs`)

- Guard/fallen stills only: `Generation::frame(spec, source, fallen, explicit_retry)` builds
  `FrameSpec { clip, fallen }` (all three clips share the guard key), `hub.render` + `frame_gates`
  dedupe; `green_frame` validates `1024x1536 + alpha`, overlays chroma `RGB(0,177,64)`,
  Lanczos3 to `768x1152`, PNG-encodes. `CLIP_SECONDS=5`, `IMAGE_EDIT_TIMEOUT=180s`.
- Retry fence (preserved): `cost.image_calls > 0 && !explicit_retry` returns
  `clip_frame_prior_attempt_requires_explicit_retry`; same for video
  (`clip_prior_attempt_requires_explicit_retry`); restart recovers dispatch evidence, never
  auto-resubmits. `FrameProvider::frame` / `VideoProvider::video` are injected traits today.
- Prompt: `frame_prompt(fighter, fallen)` (guard vs fallen poses, transparent background);
  `motion_prompt(spec)` is video-orchestration text, stays with clips.
- Output/privacy/default: alpha + green stills under `directory/<key>/`; identity from server
  roster only; `DF_GAMEPLAY_CHARACTER_VIDEO=off` starts no clip-only frame demand.
- Tests: cache hit; prior-attempt-without-bytes requires explicit retry; explicit retry works;
  alpha/model/format checks; video-disabled starts nothing.
- Migration: frames become `Purpose::AnimationFrame/Reference` `Generate`
  through `ImageService` with the fence expressed as admitted-attempt state; video stays out.
  Preserve the reference-based edit character, the Sunburst transparent-PNG requirement,
  medium-dev/high-production settings, output validation, and explicit retry after a prior
  attempt. No unresolved Generate-or-ReferencePrep choice: it is `Generate` until a frozen
  contract says otherwise.

## 6. Reference prep, masks, thumbs, sheets (local Transform scope)

| Current code | Behavior | Destination |
| --- | --- | --- |
| `imagery/party_sheet.rs` (`party_sheet`, `fit_reference`, `vision_picture`) | grey flatten, triangle resize, ordered composition, JPEG encode; byte-fitting loop | `Transform` ordered recipe + `ReferencePrep/Reference` role; replace fitting with bounded monotonic dimension/quality sequence + typed `CannotFit` |
| `imagery/cast_sheet.rs` (`cast_sheet`) | canonical-ID order, `512x768` cells, thumbnail/grey JPEG90 + legend | caller keeps order/legend; service executes layout |
| `hero_library.rs:1313 thumbnail` | JPEG decode, `thumbnail(96,144)`, quality 65, `<=32768` bytes else unavailable | `Transform` variant of the accepted portrait (`ImportedDerivative/Thumbnail`) |
| `character_clips.rs:301 green_frame` | alpha overlay + `768x1152` Lanczos3 prep | `Transform` prepared still |
| `plate_segments/crops.rs` | resizing + rectangular crops | `Transform` derivatives; analysis owner keeps region choice + coordinate mapping |
| `imagery/figure_masks.rs` (alpha-edge blur ~596) | mask feathering | local mask apply/feather; segmentation inference stays a separate analysis stage |
| `home_video.rs`, `splat_world.rs` | raster resize/thumbnail prep | reusable source prep via `Transform`; encoding/3D stay with owners |
| `home_theme/analysis.rs` | thumbnail for pixel sampling | stays private analysis sampling; no published image |
| `df-assets/validation.rs` | upload admission, re-encode | stays in `df-assets`; never bypassed via `Transform` |
| `imagery.rs:2023` BFL output norm | Lanczos3 resize + JPEG re-encode | private output-normalization recipe; record any stretch explicitly |
| `imagery/photo_check.rs` + `NativePlateJudge` judge stages (`imagery.rs:906-912`, `dev_control/reads.rs:56-62`) | scene/room judgement, visible-feature extraction, one corrective generation, visible-text exception kept explicit | inspection/quality stage inside the service with its own admitted attempt; preserve pass/unjudged/known-defect outcomes, the one-correction policy, and the dev `photo-check` read-only shape (no regeneration, no state change) |
| `fal_image.rs` BiRefNet background removal (`BACKGROUND_REMOVAL`, `cost.rs:778-784`) | transparent-background fal edit billed as a second metered stage | explicit metered recipe stage with both receipts; never a single merged charge or an invisible extra call |
| `cost.rs` image attribution (`image_call`, `image_route_model`, `settle_image_call`, `image_token_usage`, `BilledCall` unknown-outcome on drop) + `cost/port.rs` `MeteringPort` + `cost_charge_guard_tests.rs` AST owner | per-route maxima/estimates, provider-reported vs estimated usage, unsettled-drop booked at maxima as unknown | migrate attribution onto the service attempt/envelope identity; keep the guard AST owner; unknown usage stays outstanding, never zero |

Color/alpha contract (companion plan): EXIF-first orientation, sRGB-linear working,
premultiplied filtering, explicit JPEG matte, PNG/JPEG/WebP only as verified, metadata stripped.

## 7. Dev fixtures, generic fal, Codex

- `dev_control/direct.rs`: `image_fixture` (loopback-only behind dev gate) + `start_imagery`
  (`ImageEditConfig::loopback_fixture` + `NativeImageEditProvider::new(..., "test-credential")`).
  Legacy allowance until migrated to the central fake-backend seam.
- `imagery.rs` `DirectionFixture::photographic`, `register_reviewed`, `Imagery::disabled`;
  `hero_identity_fixture::portrait_bytes`; cost/metering/imagery/plates tests using
  `loopback_fixture(127.0.0.1:*)`. Test-only allowances, loopback + port required.
- `df-providers/fal_media.rs::submit_image` (generic FLUX Schnell): only test callers found;
  migrate to private disabled-by-default backend or delete; never retain a public bypass.
- `gameplay/codex_cli.rs` + `codex_cli/windows_process.rs`: text-only wrapper today, image
  generation explicitly disabled; new private `df-imagery/backends/codex_cli.rs` owns image
  invocation/event adapter. Ordinary non-image Codex text calls are never a violation.

## 8. Exact integration outline for root (worker writes no shared files)

Migration patch shape per consumer (one commit per consumer, in this order:
creator chain -> canonical/mural -> plates direct/state/ahead/items/expressions/combat ->
clips frames -> reference/mask/thumb/sheet prep -> dev fixtures -> delete bypasses).
The sketch below is illustrative only: field names follow the frozen contract, and
`service-composition.md` (unapproved draft) contributes no call shape. Root owns the
service surface, `lib.rs` registrations, manifests, and the runtime migration harness.

```rust
// At each call site (consumer crate, owned by this map; service contract owned elsewhere):
let submission = image_service
    .submit(AuthorizedImageRequest {
        identity: OperationIdentity { owner, operation, fingerprint },
        purpose: PurposedUse { purpose: Purpose::ScenePlate, role: VariantRole::Primary },
        operation: ImageOperation::Generate(GenerateSpec { prompt, references }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible, // explicit regen: Regenerate { nonce }
        urgency: Urgency::Visible,             // ahead: Urgency::Speculative
        audience,
        expected_source_revision,
        output,
    })
    .await?;
```

- Recipe key base: extend `MediaRequestKey::derive_recipe` (`df-imagery/3`), never the
  `df-imagery/2` legacy struct; pipeline bytes are canonical `df-imagery` bytes, not `Debug`.
- Root-owned snippets needed (worker does not edit): `df-imagery/src/lib.rs` narrow
  `pub use service::{ImageService, ...}`; `df-persistence` migration registration +
  `lib.rs` store wiring; `Cargo.toml` edges (`df-imagery` must not gain `df-tools`,
  `df-session`, browser crates, or PostgreSQL adapters; composition injects adapters).
  Dependencies: reuse `image`, existing native HTTP/egress, `df-assets`, finance authority,
  `df-observe`; no new image-only wallet, SQLite journal, or second cache/retry/registry.
- Deletion gate: remove `ImageRoute::{OpenAi, Flux, Bfl}` public constructors,
  `submit_image` generic bypass, redundant flights/caches/retries, temporary shims;
  keep read-only legacy import (`compatibility.rs`) only until phase 5 guard passes.

## 9. Checker + verification

- `development/check-imagery-boundary.py`: whole-stream scanner over stripped Rust
  (comments/strings/chars blanked, newlines kept, escaped newlines preserved for line
  mapping). Default mode reports the narrow section 10 legacy rows nonfatally and fails
  on anything else; `--strict` fails on the legacy rows too. Any non-allowlisted direct
  executor always fails; no directory blankets.
- Test scope is syntax-scoped: a whole test path (`tests/` dir, `*_tests.rs`) or an
  offset inside a `cfg(test)` braced item counts; production after a closed `cfg(test)`
  module does not. `df-providers` self-tests use the same rule, so the registry factory
  stays checked. `submit_image` calls and `ImageRoute` constructions in test scope are
  test-only wiring and exempt.
- Fixture association is call-associated and binding-associated: a forbidden constructor
  in test scope is exempt only when its own argument list carries `loopback_fixture(...)`,
  or the exact variable passed to it was bound by `let name = ...loopback_fixture(...)`
  within the 12 preceding lines without a later non-fixture reassignment. A bare
  `loopback_fixture` identifier or an unrelated nearby fixture call bound to another
  variable exempts nothing.
- `ImageRoute::X(_)` with an exact wildcard, match-arm patterns before `=>`,
  `matches!` patterns, and `let` destructuring are inspection, never execution. Only
  construction is flagged, including multiline payloads and `_prefixed` payloads such as
  `ImageRoute::Flux(_provider)` in expression position. `submit_image` requires its
  dispatch dot, so the provider definition is not a finding. Route-enum wrapping
  (`Self::Flux`, `.map(Self::OpenAi)`) cannot evade alone: every route still needs its
  provider built through a flagged `::new`.
- `crates/df-imagery/src/backends/` is the exact intended private execution boundary and
  is skipped by the constructor scan; the rest of `df-imagery` is still checked. Export
  audit fails direct bare-`pub` backend exposure (`pub mod backends`, `pub use ... backends
  ...`, multiline included) and same-file indirect exposure through module aliases
  (`use crate::backends as implementation; pub use implementation::Runner`) and grouped
  item aliases; `pub(crate)` and private declarations pass. Dependency audit parses
  `crates/df-imagery/Cargo.toml` with stdlib `tomllib`, covering `[dependencies]`,
  `[dev-dependencies]`, `[build-dependencies]`, `[target.*]` subtables,
  `[dependencies.<name>]` subtables, `{ package = "<name>" }` renames, and
  `workspace = true` inheritance resolved against root `[workspace.dependencies]` for
  `df-tools` / `df-session`.
- `use X as Y` aliasing is resolved per `use` statement including grouped/nested trees
  (`use df_providers::{NativeImageEditProvider as Edit, NativeFalImageProvider as Flux}`:
  both `Edit::new` and `Flux::new` fail as `alias`). Bare renamed identifiers with no
  `use ... as` binding in the file, `type` aliases, wrapper functions, raw HTTP image
  endpoints, and image CLI/script launches are outside this guard; the map claims no
  universal bypass deletion for them.
- `development/tests/test_imagery_boundary.py`: failure-focused unittest covering strip
  fidelity (comments, strings, lifetimes, escaped newlines), multiline and
  comment-separated constructors/dispatch, cfg-scope leaks (production after a closed
  `cfg(test)` module still fails), fixture binding (unrelated and reassigned fixtures
  still fail; inline and exact-variable fixtures pass), route payloads (`_provider`
  construction fails; named match/`let`/`matches!` patterns pass), submit definition vs
  call, the private-backend prefix, budget growth and owning-function staleness, all Cargo
  subtable/workspace forms, direct and indirect export forms, grouped aliases, and
  byte-raw literal noise.
- Root runs (worker shell is disabled; root executes): the unittest file, then
  `python development/check-imagery-boundary.py --root .` (must PASS with only legacy
  rows) and `... --strict` (must FAIL until every section 10 row is migrated and its
  budget removed), plus independent bypass fixtures. Tests are never weakened to force
  green. This document contains no truncation markers; a `[truncated ...]` marker in a
  submission is a defect.

## 10. Exact legacy allowance and removal table

Each row is an exact (file, symbol) budget with one occurrence and an owning-function
callsite fingerprint. Only an occurrence whose enclosing `fn` body carries the fingerprint
counts as legacy; any other occurrence of the same pair fails as `bypass` even when the
token appears elsewhere in the file. A present file with zero qualifying occurrences fails
as `stale`. Absent files are skipped (fixture trees do not carry the whole workspace).
Default mode reports these rows; strict fails on them. Removing a row means migrating its
call site and deleting its budget entry in the same change.

| File | Symbol | Count | Fingerprint | Removal |
| --- | --- | --- | --- | --- |
| `crates/df-tools/src/gameplay/imagery.rs` | `NativeFalImageProvider::new` | 1 | `FalImageConfig::live` | phase 3 startup migration, then phase 5 deletion |
| `crates/df-tools/src/gameplay/imagery.rs` | `NativeImageEditProvider::new` | 1 | `ImageEditConfig::live` | phase 3 startup migration, then phase 5 deletion |
| `crates/df-providers/src/registry.rs` | `NativeFalMediaProvider::new` | 1 | `registered_fal_media_provider` | phase 3 generic-Schnell migration, then phase 5 deletion |
| `crates/df-tools/src/gameplay/creator_portrait_startup.rs` | `NativeImageEditProvider::new` | 1 | `ImageEditConfig::live` | phase 3 creator chain, then phase 5 deletion |
| `crates/df-tools/src/gameplay/creator_portrait_startup.rs` | `NativeFalImageProvider::new` | 1 | `FalImageConfig::live` | phase 3 creator chain, then phase 5 deletion |
| `crates/df-tools/src/gameplay/creator_portrait_startup.rs` | `NativeBflImageProvider::new` | 1 | `BflImageConfig::live` | phase 3 creator chain, then phase 5 deletion |
| `crates/df-tools/src/gameplay/creator_portrait_startup.rs` | `ImageRoute::OpenAi` | 1 | `ImageProvider::OpenAi` | phase 3 creator chain, then phase 5 deletion |
| `crates/df-tools/src/gameplay/creator_portrait_startup.rs` | `ImageRoute::Flux` | 1 | `ImageProvider::Flux` | phase 3 creator chain, then phase 5 deletion |
| `crates/df-tools/src/gameplay/creator_portrait_startup.rs` | `ImageRoute::Bfl` | 1 | `alt_settings` | phase 3 creator chain, then phase 5 deletion |
| `crates/df-tools/src/gameplay/character_clips.rs` | `NativeImageEditProvider::new` | 1 | `IMAGE_EDIT_TIMEOUT` | phase 3 frames with the prior-attempt fence, then phase 5 deletion |
| `crates/df-tools/src/gameplay/dev_control/direct.rs` | `NativeImageEditProvider::new` | 1 | `loopback_fixture` | phase 3 dev fixtures to the central fake-backend seam |
| `crates/df-tools/src/gameplay/imagery/figure_masks.rs` | `NativeImageEditProvider::new` | 1 | `for_test` | phase 3 test helper to the central fake-backend seam |
| `crates/df-tools/src/gameplay/imagery_live_tests.rs` | `NativeFalImageProvider::new` | 1 | `FluxPerson` | opt-in live acceptance (`#[ignore]`); migrate to service-level live acceptance |

Deliberately not rows: `cost.rs` / `imagery.rs` `ImageRoute::X(_)` pattern arms and
`matches!` reads (inspection, section 9); `fal_media.rs::submit_image` definition
(dispatch dot required); `df-providers` self-tests and loopback-backed test wiring
(call-associated fixture proof); `crates/df-imagery/src/backends/` (private execution
boundary, paired with the export audit).