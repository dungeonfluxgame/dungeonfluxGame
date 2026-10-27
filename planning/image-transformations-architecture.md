# Image transformations: architecture and implementation plan

Date: 2026-10-27

Status: Proposed design; no runtime implementation or provider calls performed.

Companion to [the unified image architecture](image-generation-architecture.md). Source inspected: `C:/Users/mreca/Desktop/dungeonfluxGame`; these plans live separately in the chat workspace. Freeze the source commit and working-tree changes before implementation.

## Decision and scope

Extend the planned `df-imagery` crate and its existing `ImageService` lifecycle with three typed operations: **Generate**, **Edit**, and **Transform**. Transform executes an ordered local raster pipeline. It does not select an AI backend, require provider credentials, or fall back to a model when an effect is unsupported. Generate and Edit can use the same local pipeline for reference preparation and output variants.

Cover resizing, cropping, padding, rotation, flipping, brightness/exposure, contrast, saturation, hue, grayscale, blur, sharpening, masks, compositing, and format/quality conversion. Character portraits and thumbnails, display plates, item views, reference sheets, and animation stills all use this facility. Preserve source images and create immutable derived assets; an effect never overwrites an accepted portrait or canonical plate.

This refines the parent plan's earlier exclusion of deterministic crops/resizes. It follows the source project's `planning/subsystem-architecture.md`, `planning/subsystem-interfaces.md`, `planning/asset-engine.md`, `planning/runtime-reliability.md`, `planning/storage-architecture.md`, `planning/observability.md`, and `planning/coding-style.md`. Existing asset publication, authorization, retention, finance, and telemetry authorities stay with their established owners.

Interactive camera transforms, fades, display calibration, CSS/GPU styling, and video frame encoding remain in their rendering/video owners. They do not create reusable raster assets. Pure analysis that reads pixels without producing an image also remains with its analysis owner. Do not move all uses of the `image` crate indiscriminately.

The [visual motion plan](visual-motion-architecture.md) specifies these display-time transitions, swipe/zoom gestures, camera effects and their configuration/lifecycle. A zoom animation reuses an image; a saved crop/resize creates a new Transform asset. Motion preferences and image execution modes remain separate controls.

## Current implementation and migration map

Paths below are relative to the inspected source checkout, not this planning workspace. These are observed call sites, not evidence of working integration with the proposed crate.

| Current code | Observed behavior | Planned ownership |
| --- | --- | --- |
| `crates/df-tools/src/gameplay/imagery.rs` near 2023 | BFL output resized exactly with Lanczos3 and re-encoded as JPEG | Private output normalization recipe in `df-imagery`; preserve existing behavior initially, record any stretch explicitly |
| `gameplay/imagery/party_sheet.rs` | Grey flattening, triangle resize, ordered party composition, JPEG encoding; `vision_picture` resizes/encodes; `fit_reference` attempts byte fitting | Local pipeline executes pixels; domain caller keeps roster selection, slot meaning and legend |
| `gameplay/imagery/cast_sheet.rs` | Canonical-ID ordering, 512 x 768 cells, thumbnail/grey composition, JPEG90, identity legend | Caller keeps identity order/legend; service owns layout execution and source lineage |
| `gameplay/hero_library.rs::thumbnail` | JPEG portrait thumbnail within 96 x 144, quality 65 and 32 KiB limit | Transform variant of the accepted authorized portrait; library owns when/how it is attached |
| `gameplay/character_clips.rs` near 307 | Alpha overlay and 768 x 1152 Lanczos3 preparation | Prepared still through Transform; clip orchestration and video remain outside |
| `gameplay/plate_segments/crops.rs` | Resizing and rectangular crops for segmented plate imagery | Analysis owner chooses regions; Transform produces image derivatives with coordinate mapping |
| `gameplay/imagery/figure_masks.rs` near 596 | Alpha-edge blur | Local mask application/feathering moves with raster preparation; segmentation/model inference remains a separately declared analysis stage |
| `gameplay/home_video.rs`, `gameplay/splat_world.rs` | Raster resize/thumbnail preparation | Move reusable source-image preparation through Transform; leave video encoding and 3D execution with their owners; freeze exact boundary during inventory |
| `gameplay/home_theme/analysis.rs` | Thumbnail used for pixel sampling | Keep private analysis sampling; no generated/published image or provider dispatch |
| `crates/df-assets/src/validation.rs` | Upload type/size/pixel checks, animation rejection, metadata-stripping re-encoding | Keep upload admission in `df-assets`; do not create a dependency cycle or bypass validation via Transform |

The inspected helpers often use fixed dimensions, quality, and encoded-channel alpha arithmetic. A migration must not silently change their appearance while claiming it only moved code. Capture current synthetic fixture outputs, then preserve them in explicitly versioned compatibility profiles. New color-correct behavior is a separate profile revision with visual comparison.

`fit_reference` currently reuses the original decode while its size estimate can remain unchanged across attempts. Replace the fitting strategy with a bounded, monotonic dimension/quality sequence and a typed failure when the byte target cannot be met. Record actual dimensions and encoding settings, not only the requested byte limit.

## One public API and internal execution path

Keep `submit`, `status`, `watch`, and `cancel` from the parent plan. Add the operation discriminant to the existing authorized request rather than adding a separate effects service or public raw pixel executor. Illustrative contracts, to be reconciled with existing shared types before implementation:

```rust
enum ImageOperation {
    Generate(GenerateSpec),
    Edit(EditSpec),
    Transform(TransformSpec),
}

struct TransformSpec {
    source: TransformSource, // Authorized image, canvas, or bound mask coverage.
    steps: Vec<ImageTransform>,
    output: RasterOutputSpec,
}

enum ImageTransform {
    Resize(ResizeSpec),
    Crop(PixelRect),
    Pad(PadSpec),
    Rotate(RotateSpec),
    Flip(FlipAxis),
    Adjust(ColorAdjustment),
    Blur(BlurSpec),
    Sharpen(SharpenSpec),
    ApplyMask(MaskSpec),
    Composite(CompositeSpec),
}
```

Masks and composite layers contain additional authorized immutable references. The final source type must include both an authorized image and an explicitly sized/color-filled canvas; a sheet does not need a fake file or empty uploaded image. Include typed mask rasterization from bounded immutable coverage/run data linked to its authorized source and analysis revision, so existing SAM mask materialization has a supported path without exposing arbitrary pixel callbacks. Generate/Edit may declare named derived variants and reference-preparation recipes using these same typed steps. This is a finite recipe, not an arbitrary script, plugin runtime, or recursive workflow language.

Prompts and image references remain typed Generate/Edit inputs. Transform needs image/canvas inputs and effect parameters; it does not reinterpret natural-language text as effect commands. Adding local text drawing later would require an explicit font/layout operation, not an implicit model call.

```text
Authorized request -> validate/freeze recipe -> cache lookup -> stage scheduler
    Generate/Edit -> selected private API or Codex CLI backend -> source asset
    Transform     -> authorized source/canvas -----------------> local executor
                           |                                      |
                           +---- shared local variant steps ------+
                                                                  |
                                 df-assets validation/publication <-+
                                             |
                              authorized source + variant references
```

Private modules `transform/{plan,execute,geometry,color,composite,encode}.rs` are introduced as needed inside `df-imagery`. Reuse the project's raster library after checking its actual supported formats, allocation limits, color behavior and target support. Do not create a second crate solely for these helpers. Any genuinely shared validation primitive belongs in an existing lower-level owner; `df-assets` must not depend on `df-imagery`.

The result identifies the canonical source, each named variant and its status, actual dimensions/format/alpha, pipeline/profile revision, and lineage. A failed optional thumbnail does not discard a completed portrait. Required output normalization failing prevents that output from being marked ready. A persisted source can be used to retry a local stage without repeating paid generation. Internal intermediate checkpoints need not be player-visible assets.

Required variants, final-image checks, and source publication order are frozen in the recipe. Perform pixel-sensitive final checks on the actual delivered variant. Do not copy a source's visual-quality verdict onto a changed crop or composite as though it was checked. Record inherited evidence separately from newly performed checks; ordinary local transforms must not implicitly initiate a paid critic.

## Ordered operations and exact semantics

Steps execute in listed order. Crop-then-resize and resize-then-crop are different requests. Coordinates use the current oriented raster at that step; masks/layers carry explicit alignment and extent rules. Validate dimensions and resource estimates after every step before allocating. Freeze the following semantics in the first contract milestone:

| Operation | Contract |
| --- | --- |
| Resize | `contain` preserves aspect ratio within bounds; `cover` fills exact bounds with an explicit crop anchor; `stretch` deliberately changes aspect ratio; `pad` preserves content and adds an explicit background. Specify target pixels, interpolation and whether upscaling is allowed. |
| Crop/pad | Integer pixel rectangle and canvas dimensions, explicit anchor/color; reject out-of-bounds crop unless padding is requested. Apply EXIF orientation before interpreting coordinates. |
| Rotate/flip | Exact right-angle rotations and flips first; arbitrary angle uses explicit interpolation, expanded/fixed canvas choice and fill color. |
| Brightness | Add a signed normalized offset in the declared working RGB space, then clamp at the specified output boundary. Neutral is zero. Do not overload this parameter with exposure. |
| Exposure | Multiply linear-light RGB by `2^stops`; neutral is zero stops. Preserve alpha. |
| Contrast | Scale RGB around an explicit pivot in the declared working space; neutral gain is one. |
| Saturation/hue/grayscale | Use one named, versioned color model with explicit transfer conversion, coefficients and units; freeze these equations before coding. Neutral saturation is one; hue is degrees. |
| Blur/sharpen | Explicit kernel/algorithm and radius in pixels of the current raster; sharpen includes amount and threshold. Define boundary sampling and alpha behavior. |
| Mask | Choose alpha channel or declared luminance extraction; specify alignment, optional inversion, feather radius and multiply/replace mode. No automatic semantic segmentation. |
| Composite | Ordered layers, positions, extents, opacity, and blend mode. Source-over is the initial default. Extra blend modes need defined equations and fixtures, not backend-dependent names. |
| Encode | One final encode with explicit format and relevant quality/compression/alpha/matte options. Re-encoding alone is a valid transform. |

The normal pipeline decodes once, performs operations in memory, and encodes each final variant once. Byte-target fitting is the explicit exception: it encodes a bounded number of candidates and retains the selected result. Branch variants from a decoded canonical source when resource accounting permits, without encoding each intermediate effect. Preserve precision between color operations and define rounding/clamping explicitly. An optimization may fuse operations only when it preserves the frozen semantics.

Resolution has two independent meanings: provider-native generation size and delivered variant size. Both appear in provenance and costs. A local upscale does not claim additional generated detail. AI super-resolution, inpainting, semantic background removal, and other model-powered changes are Edit or explicitly billable provider stages, never disguised as local Resize/Mask. Unsupported backend input/output capabilities return a typed outcome or use an explicitly configured local adaptation whose result is reported.

Byte-target reference fitting specifies maximum bytes, minimum dimensions, permitted quality range and format/alpha requirements. It uses a bounded deterministic search, reports the actual chosen settings and fails with `CannotFit` when no permitted candidate fits. Never silently remove alpha, crop content, or degrade below the declared bounds to meet a byte budget.

## Color, alpha, orientation and encoding

New standard profiles normalize orientation and color before effects. Honor supported embedded color profiles and convert to a defined sRGB output; choose and version the actual color-management implementation during the contract milestone. Unprofiled images use the declared sRGB assumption. Unsupported profiles must produce a typed error unless the caller explicitly selected a documented compatibility assumption; never strip a profile while pretending the pixels were converted.

Use a defined linear-light working representation for new resampling and compositing profiles, with premultiplied alpha during filtering. Convert/unpremultiply safely at encoding boundaries, including fully transparent pixels. Color adjustments leave alpha unchanged unless the operation explicitly targets it. Masks are scalar coverage, not color images to which gamma/exposure is accidentally applied. Include transparent-edge fixtures to prevent black/white fringes.

PNG, JPEG and WebP are the initial output formats because they are already represented in the project. Expose only encoder capabilities actually verified in the selected implementation. JPEG requires an explicit matte for transparent input. Separate lossy quality from lossless compression; do not pretend the same numerical quality means the same result for every codec. Persist actual output MIME, dimensions and alpha state.

Normalize orientation once before discarding EXIF; strip location/private metadata. Existing admitted uploads may already have lost orientation/profile metadata in `df-assets::validate_image`; the transform stage cannot recover it. Review upload normalization as a coordinated `df-assets` change, preserve admission ordering, and never retroactively invent metadata for old assets. Derivatives cannot be used to launder rejected or unapproved uploads.

## Identity, ownership and publication

Derive a versioned transform key through the existing asset request-key facility. Include authorized scope/audience, all source and mask/layer digests with ordered bindings, orientation interpretation, ordered normalized parameters, working/output color profiles, encoder settings, and executor/algorithm compatibility revision. Canonical serialization defines float handling; reject non-finite values, normalize negative zero and defaults, and avoid hashing debug strings. Budget, queue priority and wall-clock deadlines do not affect pixel identity.

Separate request identity from output content digest. The same transform key must resolve consistently within its declared compatibility revision. Changes to algorithms, encoding behavior or color conversion that can change bytes require a new revision. Do not promise byte-for-byte identity across arbitrary libraries/CPUs; verify supported targets, pin the relevant implementation behavior, and revise the compatibility key when necessary.

Always authorize before cache disclosure. Derivative visibility is no broader than the intersection of source permissions and the requested audience. Cropping a private image is not declassification. Apply the existing rights/retention and revocation model to every source, including masks and composition layers; lineage lets the catalog invalidate descendants when required. A lost source entitlement cannot be bypassed through a formerly cached variant.

Publish atomically through `df-assets`; preserve immutable originals and variant-to-source relationships. Retention follows the existing catalog's rules for accepted portraits, attached plates/items, temporary references and unreferenced derivatives. Account for the storage of variants; do not build a second file cache or retention daemon. Source attachment to a character or scene remains the session/domain owner's decision.

## Scheduling, cancellation and recovery

Use the same service job ownership and interest/coalescing rules as generation, with separate local CPU/memory permits. A local thumbnail must not wait for a provider semaphore. Reserve estimated peak decoded/intermediate memory, queue capacity and output capacity before execution; account for all branches and composite sources. Limits are operator-tunable within the deployment's resource allocation, not arbitrary artistic restrictions.

Schedule recipe stages directly; do not recursively submit child jobs while holding parent execution permits. Release provider/CPU execution permits before awaiting a different stage's capacity. Retained decoded buffers still hold their memory reservation; include the next stage's peak in admission or persist/release buffers before waiting. Define an acquisition order and a minimum-capacity test so reference preparation and final variants cannot deadlock their parent jobs.

Run CPU work on bounded owned workers outside async reactor threads. Cooperative cancellation is checked between stages/tiles where supported. Dropping a future does not prove a decoder stopped or free its resource reservation. For hard termination requirements, use owned isolated workers; do not claim in-process codec calls are preemptible. Report cancellation pending until the worker completes or is confirmed stopped.

Coalesce identical authorized transforms. Retry failed local work only under a bounded policy; resume from durable source/checkpoints after a crash. Local recomputation cannot create a supplier charge, but it still consumes CPU and storage. Never turn recovery of a failed transform into paid source regeneration. Publication retries remain idempotent through the asset catalog.

## Configuration and disable semantics

Extend the parent `ImageConfig` with transform defaults/profiles, algorithm revisions, local concurrency, decoded-memory/output-byte limits and bounded queue/deadline settings. Resolve file/environment/launcher/runtime changes with the parent's existing precedence, validation and revision mechanism. Named profiles provide sensible defaults; callers can request typed overrides allowed by their purpose and authorization. Invalid/unsupported choices produce field errors without silently dropping effects.

Add `local-only` to the global modes. This is the explicit way to keep local image work available while disabling every provider-backed image stage, including Codex CLI and paid analysis/checks. A local source need not have been generated by this service, but must be admitted and authorized through assets.

| Global mode | Ready compatible result | New local transform | New provider stage |
| --- | --- | --- | --- |
| `live` | Reuse | Allowed | Allowed by selected backend/purpose configuration |
| `local-only` | Reuse | Allowed from ready authorized sources | Disabled, including classifications, checks and CLI launch |
| `cache-only` | Reuse | No; miss returns `PreparedMiss` | No |
| `off` | Existing assets remain readable; new submissions return `Disabled` | No | No |

No mode silently changes Generate/Edit into Transform. In `local-only`, a generation recipe can complete local variants from an already valid source/checkpoint only if its required checks are already satisfied and no provider stage is needed; otherwise report the unmet stage. Request-level prepared-only/replay remains stricter: it cannot execute a new transform merely because the global mode allows one.

`[imagery.transforms].enabled = false` prevents new local stages; cached complete variants remain reusable in modes that allow submissions. Generate/Edit recipes that require a disabled local stage fail before paid dispatch instead of returning malformed output. Per-purpose disabling also covers its transform demand. Backend disabling does not disable independent local transformations.

Example proposed configuration, illustrative values rather than hard-coded limits:

```toml
[imagery]
mode = "local-only"

[imagery.transforms]
enabled = true
max_concurrency = 2
max_decoded_bytes = 268435456
max_output_bytes = 16777216
queue_capacity = 64
deadline_seconds = 30

[imagery.transform_profiles.hero_thumbnail]
width = 96
height = 144
resize = "contain"
allow_upscale = false
format = "jpeg"
matte = "#808080"
quality = 85
color_profile = "srgb-linear-v1"
```

These values must be reconciled with asset limits and the launcher configuration schema in implementation. Profile changes affect new jobs and variant keys; existing jobs keep frozen pixel semantics. Disabling takes effect at the parent's serialized dispatch gate, including local workers. After acknowledgement, no unstarted stage begins. Started stages drain; an off/cache-only transition may finish publication of already completed acceptable output but cannot start the next effect stage. Track that distinction explicitly. Source checkpoints remain available to an authorized explicit later request.

## Usage and cost reporting

Report local queue/wall time, measured CPU time where available, peak accounted memory, input/output bytes, source/variant counts, operation mix, retries, coalesced interests and cache hits through the existing telemetry/reporting path. Keep measured and estimated resource values distinguishable.

A wholly local transform has zero incremental AI supplier usage/charge; create no dummy supplier attempt or money reservation. That does not mean zero infrastructure/storage cost or zero historical cost for its source. Attribute the original generation's cost through lineage without recording it again as new spend. Optional infrastructure estimates use separately configured rates and are labeled estimates; they never enter supplier settlement as actual charges.

Paid masks, AI upscale or provider background removal retain their own admitted attempt and usage evidence under the parent cost contract. Show stage-level local processing and provider amounts separately, with parent totals deduplicated. Transform failures after paid generation preserve that generation's cost and reusable source. Reports remain readable when all image execution is disabled.

## Implementation sequence and acceptance

| Phase | Deliverable | Required evidence |
| --- | --- | --- |
| T0: Inventory and contract | Freeze source revision, classify asset-producing vs analysis/render-only helpers, finalize operation equations/color implementation/output types and mode semantics | Exact caller map; current synthetic golden fixtures; public contracts reconcile with assets and parent API |
| T1: Local core | Typed validation, ordered geometry/color/composition/encoding, bounded workers and resource estimation | Synthetic pixel/alpha/orientation fixtures; malformed/oversize inputs; no provider credentials or dispatch required |
| T2: Service and assets | Transform via normal submit/status/watch/cancel, cache keys, source/variant lineage, publication/recovery and config/cost projection | Authorization, coalescing, disable-race, cancellation, restart and publication-failure tests on actual adapters |
| T3: Existing consumers | Migrate party/cast/vision references, BFL normalization, hero thumbnails, clip still prep and plate/mask derivatives; classify home-video/splat preparation | Existing profile compatibility; no provider calls on local requests; purpose/identity binding retained |
| T4: New effects and variants | Configurable brightness/exposure/contrast/saturation etc; explicit output variants for portraits, plates and item images | Visual reference comparisons, ordered-operation results, no mutation of originals or duplicated source charges |
| T5: Integration and enforcement | Update main architecture, configs, call-site guard and consumer tests; remove migrated helper bypasses | Native/WASM gates for affected crates, independent actual browser observation and source/asset/build records |

T0 coordinates with the parent plan's public-contract phase; T2 depends on its service/catalog integration. T3 can follow consumers incrementally without waiting for every backend. Do not postpone the existing caller migration behind a generic effects editor: no new UI editor is required by this plan.

The verification matrix must include:

- Portrait -> thumbnail and accepted-character display; current plate -> crop/brightness variant; item source -> close-up variant; transparent clip frame -> prepared still. Source assets remain unchanged and all attachments preserve current audience/revision rules.
- Geometry/aspect/crop anchors, operation ordering, round-trip metadata/orientation, non-square masks, composite layer order, alpha edge halos, grey/black/white and known color fixtures, explicit JPEG matte, lossy/lossless settings, and byte fitting that either progresses or fails clearly.
- No-op/neutral effects, zero dimensions, overflow, non-finite parameters, oversized decoded images, too many layers, malformed codecs, queue saturation, cancellation during decode, and memory reservations retained until work actually stops.
- Cold/warm cache, shared interests, cross-user cache denial, source revocation and derivative serving, algorithm/profile key changes, optional variant failure, crash before/after publication, and local retry without paid regeneration.
- Every global mode and transform/backend/purpose toggle; off/cache-only races between local stages; local-only with no provider keys; required disabled transforms rejected before a paid call; prepared-only never computes on a miss.
- Cost/usage projection: one paid source plus several local variants counts the supplier attempt once, local cache hits add no execution charge, and unavailable CPU measurements/infra rates remain unknown rather than invented.

Extend the parent regression guard to detect image derivative creation/publication outside the approved boundary. Keep a reviewed list of upload-validation, video-codec, renderer and analysis-only exceptions. A blanket ban on `image::` imports would hide the actual architectural intent. Tests must include a forbidden derivative-publication fixture to demonstrate the guard detects bypasses.

Planning completion means this document is internally reviewed and linked into the parent plan. Implementation completion additionally requires migrated callers, enforced boundaries, configuration/reporting integration and observed consumer behavior on the recorded build. No runtime acceptance is implied by this document.
