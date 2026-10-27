# Game-wide image transitions and animation

Date: 2026-10-27

Status: Proposed architecture and migration plan. Source inspection only; no runtime, visual or device acceptance performed.

Source: `C:/Users/mreca/Desktop/dungeonfluxGame`. This document lives in the separate chat workspace with the [image generation plan](image-generation-architecture.md) and [image transformation plan](image-transformations-architecture.md). Freeze the source revision and working-tree changes before implementation.

## Decision

Create one typed motion contract and reusable execution primitives **inside the existing presentation, UI and render crates**. Consumers request semantic transitions and camera/UI effects through those contracts instead of starting unrelated timers, CSS animations and frame loops. Keep the established display conductor as the integration starting point; consolidate responsibilities incrementally rather than layering a second director on top.

`df-imagery` produces images and immutable raster variants. It does not animate the browser. Swipe, zoom, pan, dissolve, reveal and UI motion run against already authorized assets, normally without creating new files or making provider calls. Generated video/animated media is a separate asset workflow with its own cost and playback lifecycle. Applying a zoom to a still does not require generating a video.

Follow the source project's `planning/client-presentation.md`, `client-architecture.md`, `subsystem-interfaces.md`, `tempo-engine.md`, `g12-d02-tempo-accessibility-policy.md`, `campaign-cinematics.md`, `runtime-reliability.md`, `observability.md`, and `coding-style.md`. In particular, persistent Rust/WASM clients, explicit resource ownership, server-authoritative gameplay, and audience-safe presentation remain the architecture. No new animation microservice, application scripting language, or crate is required by this plan.

## Player-facing motion inventory

Motion should explain what changed, direct attention and maintain spatial continuity. A new snapshot alone is not a reason to restart an effect. Provide a complete small vocabulary, then choose deliberate recipes by surface rather than random transitions.

| Surface/event | Normal presentation | Quiet/static equivalent and failure behavior |
| --- | --- | --- |
| TV scene/location change | Ready-to-ready dissolve; optional directional wipe or travel slide for an explicit location transition; title/caption timing coordinated with narration | Stationary new scene and title; keep an authorized current image or prepared fallback until ready |
| Same-scene plate refresh | Subtle crossfade of the image layer; preserve framing, captions, controls and speaker context | Direct replacement; no repeated location intro |
| Establishing still | Slow bounded pan/zoom across a supplied focal region, then settle | Still composition; no endless camera drift by default |
| Speaker emphasis | Brief focus/pan/zoom toward an authorized subject or a restrained portrait emphasis | Stable outline/name emphasis; never focus a hidden subject |
| Dialogue/captions | Bubble/caption entrance, speaker change and readable exit coordinated with actual speech | Complete readable text; decorative text reveal cannot gate access to the line or interrupt it |
| Character creation | Pending indicator, new-portrait reveal, selected-option feedback, smooth step/panel transitions | Text pending status and direct portrait replacement; draft input/focus retained |
| Character library/party portraits | Swipe between cards, directional slide, selected portrait enlargement/shared-element transition where identity is stable | Previous/next controls and direct selection; never require a gesture |
| Portrait/item inspection | User pinch/wheel/buttons to zoom; drag to pan; reset/fit; gallery swipe when not panning an enlarged image | Same manual controls remain usable; disabling automatic motion does not disable inspection |
| Item discovery/reward | Image reveal, restrained scale/outline emphasis, detail panel entrance | Image, item name and award text appear directly; award already committed |
| Item upgrade/equipment change | Old-to-new image dissolve only when both belong to the same explicit item binding | Direct new image; avoid morphing arbitrary artwork into a different item |
| Combat entry/exit | Establish battlefield framing and ready assets, transition between flat/3D scenes; readable initiative/victory/defeat treatment | Flat ready scene plus text; never wait for optional GPU/media readiness to expose legal actions |
| Actor movement/attack | Interpolate committed path/pose; existing approach, lunge, impact, recoil, miss, fall and healing cues | Final permitted position/status plus outcome text; no local movement/rules simulation |
| Spell/projectile/area effect | Prepared effect along a supplied path/area, with explicit origin, target, impact and expiration | Stable affected-area/target indication and committed outcome; visual particles do not decide hits, collision or visibility |
| Dice/check result | Existing prepared clip/renderer with server-supplied result and an explicit settle point | Static die face and result card; same result and readable history |
| Health/status/turn | Bar/chip interpolation, status entrance, current-turn outline, optional brief impact treatment | Exact numeric state, labels and stable emphasis; moving controls are excluded |
| Progression/achievement | Brief level/reward emphasis and panel reveal after a committed update | Persistent level/reward text; animation never grants or consumes anything |
| Dialog/card/modal/navigation | Fade/slide/scale from a consistent anchor; shared-element expansion when available | Direct open/close with correct focus restoration and selection |
| Loading/reconnect/error | Small owned activity cue, recoverable error/status transition | Honest text status; no fabricated percentage or provider completion estimate |
| Ambient scene | Optional subtle prepared layers/particles/parallax, constrained by scene assets and profile | Still scene; ambience stops when hidden and yields to input/readability |
| Recap/critical cinematic | Bounded sequence of authorized shots with existing audio/captions and skip | Still/caption version; skip changes presentation only |

This inventory includes both scripted slide/wipe transitions and interactive swipe navigation. They share geometry/easing primitives but have different input and completion contracts. It also includes manual inspection zoom and director-selected camera zoom; they must not fight over the same transform.

Phase one delivers the reusable base vocabulary and existing effects. Depth-based parallax, mesh deformation, simulated fluid, complex page curls and new generated character/video animation are optional follow-ups requiring their own assets and evidence. They are not prerequisites for the game's normal transition system. Existing battle animation remains in scope for integration.

## Current implementation map

Paths below are under `crates/` in the inspected checkout. Some design documents still describe a future client; actual current source contains mounted browser presentation and a battle renderer. Treat code as evidence of implementation presence, not of measured correctness.

| Owner | Observed implementation | Planned migration |
| --- | --- | --- |
| `df-tools/src/presentation.rs` | Diffs audience-projected views; identifies scene/combat/turn/outcome changes | Preserve valid source bindings; replace ad hoc effect selection with typed semantic cues as contracts migrate |
| `df-tools/src/stage_{conductor,director,machines}.rs` | Display sequencing, line/scene/actor state and timing | Reuse/factor pure lifecycle and sequencing; one active sequencing owner per surface |
| `df-tools/src/gameplay_browser/live_motion.rs` | Converts director commands to DOM/CSS/SFX; frame clock and throttling fallback; phone does not wait for TV | Thin executor bridge into shared motion primitives; preserve current outcome/line ordering |
| `gameplay_browser/live_scene_stage.rs`, `live_stage.rs`, `live_tv.rs` | Plate/image stage lifecycle, actor effects, image layers and framing | Shared image swap/camera lifecycle and resource ownership; retain domain-specific view composition |
| `gameplay_browser/live_plate.css`, `live_imagery.css` | Plate opacity swaps, focus/zoom wrappers, outgoing figure fade | Versioned recipes/tokens, scoped execution; CSS remains a rendering backend |
| `gameplay_browser/hero_creator.css` | Pending orbit/breathing, roughly 700 ms portrait reveal, library feedback | Creator recipes driven by asset/draft revision, not remounts or repeated status polls |
| `gameplay_browser/hero_portrait_state.rs` | Pending shadow hides but retains the previous portrait for restoration | Preserve this explicit pending/restore behavior during extraction; replacing it with a visible old portrait is a separate design change |
| `gameplay_browser/live_home_swap.rs`, `live_home_video.rs` | A/B video swap with outgoing coverage retained during incoming fade | Preserve coverage/readiness invariant; integrate playback observations and motion profile |
| `gameplay_browser/live_dice.rs`, `live_health.rs` | Dice clips/canvas fallback; bar lag/chips, shake, tremor and health cues | Preserve semantic outcomes; unify preference handling, cue identity and cleanup |
| `gameplay_browser/presentation_timing.rs`, `presentation_ingress.rs` | Transition/timing observation and correlation | Extend existing diagnostics rather than add competing timing logs |
| `df-render/src/battle/{camera,stage,director,feel,host,pacing}.rs` | Camera pan/whip/zoom punch/shake, battle choreography, owned frame loop/cadence | Renderer adapter consumes shared cue/profile semantics while retaining renderer-specific math/resources |
| `df-ui/src/*_phase.rs`, `scene_transition_phase.rs` | Typed mounted UI/view components, transition surface and lifecycle | Reusable panel/card/image/gesture primitives in `df-ui`; role shells compose them |
| `df-presentation/src/` | Source-approved critical/moment/forecast contracts | Pure semantic cue/recipe planning belongs here; not a complete current motion runtime |

Focused searches found no reusable image-gallery swipe/pinch controller in these paths. Confirm all gesture/animation dispatch paths during baseline inventory, including embedded CSS and Rust browser bindings; do not infer total absence from a keyword search.

The current dice module explicitly plays motion regardless of the device's reduced-motion preference, and health cues document the same bypass. That conflicts with the governing presentation/accessibility design. Plan an explicit behavior change: honor the effective motion preference, retain the authoritative die result and health/status information, and provide static emphasis. Record before/after acceptance rather than calling this unchanged extraction.

## Ownership and contracts

| Concern | Owner |
| --- | --- |
| Committed state, permitted facts, legal input and outcomes | Existing engine/session/rules authority |
| Game-linked intensity, eligible cue/shot selection, narration pacing | `df-tempo` and pure `df-presentation` under session authority |
| Asset generation/editing/local derivatives | `df-imagery`; immutable assets/authorization remain `df-assets` |
| Ordered views, active binding and presentation epoch | `df-client` and persistent shell |
| UI transitions, image viewers, gestures, shared easing/cue lifecycle | Existing `df-ui` modules with private browser implementation |
| Camera, scene layering, battlefield/GPU animation | `df-render`; shared contract types live in an existing dependency-neutral owner selected at contract freeze |
| Surface composition | `df-player`, `df-display`, current `df-tools` bridges during migration |
| Audio playback and actual timeline observations | `df-audio`/current playback bridge; motion follows reported playback where required |
| Diagnostics | Existing `df-observe` presentation path |

Expose a small host-facing lifecycle such as `apply(plan)`, `observe(readiness/playback)`, `set_profile(profile)`, and `dispose()`. Renderer adapters receive typed commands; consumers do not manipulate another component's animation handles. Do not force UI DOM transforms and GPU cameras into one physical renderer or one new global singleton.

`df-ui` already depends on `df-render`; never add the reverse dependency for shared motion. Place pure shared types/math in an existing lower-level owner compatible with native presentation and WASM execution, or a dependency-safe module of `df-render` if the dependency audit permits it. M0 freezes that location and validates the dependency graph before consumer work. DOM bindings stay out of pure/server contracts.

Use two explicit cue origins: **game-linked cues** carry a server-authorized event/view binding; **local interaction cues** come from permitted UI gestures/navigation and cannot create gameplay facts. Both use the same execution semantics. Avoid a server round trip for every drag frame or local panel animation.

An illustrative cue contract contains:

- Cue ID, origin, session/member binding, presentation epoch and sequence, target/component key, asset/source revision and schema/profile revision.
- Semantic recipe (scene swap, portrait reveal, inspect zoom, attack impact, panel entrance), channel, priority and compatible typed parameters.
- Timebase, start/offset, duration/easing, any actual-audio anchor, expiry and late-arrival behavior.
- Readiness requirements, fallback, interruption policy, accessibility equivalent and completion state.

Supported primitive operations include opacity, translate, scale/zoom, bounded camera pan, clip/wipe, rotation, highlight, and renderer-owned impact/ambient effects. Finite typed sequences/parallel groups support normal choreography; no arbitrary executable expressions, raw server CSS, or unbounded nested timelines. A recipe is allowed because a client implements it, not because a server sends a new string.

Resolve a versioned recipe once into executable parameters. Freeze easing definitions, units, coordinate frames, duration meaning, transform order, stacking order, mask clipping and interaction hit-testing. Dynamic user gestures supply continuous local progress within the active recipe; they do not rewrite its authority or content binding.

## Image swap, zoom and swipe behavior

**Asset handoff.** Separate requested, fetched, decoded, uploaded/ready, displaying and retired states. Validate active asset/binding/epoch at every async completion. Keep at most the bounded outgoing/incoming image layers needed by the selected recipe; discard obsolete pending arrivals. A source may remain visible only while it is authorized and valid for the current presentation. A scene change can retain neutral scenery where permitted, but must remove stale actors/status/labels immediately; otherwise use a neutral prepared fallback. Revocation clears pixels without waiting for a fade.

**Dissolve/slide/wipe.** Start only when the incoming asset is displayable; do not fade the outgoing image to an empty frame. Maintain full background coverage (for an ordinary opaque swap, hold outgoing opaque while incoming covers it). Transparent character sprites need a separate compositing recipe that avoids double silhouettes. Clip slide/wipe layers to their viewport; outgoing layers are never interactive. Replaced handlers/resources retire exactly once, including when completion events are missing.

**Rapid A -> B -> C.** Keep a bounded current/incoming state; replace pending B if it never displayed. If B is partly displayed, retarget from the observed visual state or settle it according to the recipe before transitioning to C. Never create an unbounded stack of fading nodes. At the concurrency boundary, prefer a direct update to the newest authorized state over a backlog of decorative transitions.

**Camera/zoom.** Define focal points in normalized source coordinates and map through orientation, fit/cover, viewport aspect and any derivative crop. Supply safe framing bounds and preserve the subject/caption safe regions. With no valid focal metadata, use a documented center/fit fallback. Clamp automatic zoom to avoid empty edges or excessive pixel enlargement; exact limits are tunable profile values. Manual zoom remains under the user's control with explicit bounds, fit/reset and visible feedback. Viewport resize reprojects the current framing rather than jumping to an unrelated origin.

**Transform composition.** Separate image-fit geometry, camera framing, manual inspection and transient impact layers. Give each property/channel one writer; combine them in a defined order or nested wrappers. Health shake, scene pan and selection scale must not overwrite each other's `transform`. Manual inspection suspends conflicting automatic camera motion until the user resets/exits or explicitly resumes it.

**Swipe/drag/pinch.** Use pointer identity/capture and direction/intent thresholds. A horizontal gallery swipe yields to vertical page scrolling and ordinary browser accessibility zoom; do not disable page zoom globally. Pan an enlarged image before offering gallery navigation, with an explicit edge/gesture policy. Pinch focuses around its centroid; pointer cancellation, lost capture, resize and disposal end the gesture deterministically. Physical travel direction and logical next/previous are mapped deliberately for reading direction.

**Commit and alternatives.** Drag progress previews a local card transition; release commits selection only after its threshold, otherwise it returns to the selected card. This is not a gameplay action or item equip/portrait accept. Those continue through existing explicit commands. Provide previous/next, zoom in/out, fit/reset, keyboard and dismiss controls with the same results. Keep focus and selected-card semantics correct during animation; outgoing content cannot remain tabbable or submit stale actions.

## Timing, interruption and multi-screen behavior

Use elapsed monotonic browser time, never frame counts. Game-linked cues interpolate from the accepted server timeline/epoch; audio-linked visual emphasis follows actual playback observations. Local navigation uses a local monotonic anchor. The TV and phones share event identity, not a fragile lockstep animation barrier: a slow TV must not delay a player's input or game state.

Define an explicit clock mapping from each server timebase to the local monotonic clock, using the existing synchronization/receipt data and recording offset uncertainty. Raw server timestamps and `performance.now()` are not interchangeable. Audio mappings use observed playback position/start/stall observations, not fetch completion. M0 freezes offset estimation and bounded drift correction; prefer gradual correction for ongoing continuous cues and seek/skip for expired one-shots. Reconnect, discontinuity or a new epoch invalidates old mappings and rebuilds them from the current anchor. Until a mapping is usable, show current readable state and omit optional timed motion rather than guess or replay history.

Define interruption per recipe: replace pending, retarget from current pose, finish a bounded visible segment, or cancel/snap to current state. Higher-priority readability/input/status can suppress cosmetic motion. Keep control/cancel messages reliable when coalescing views. Preserve authoritative outcomes in the current UI/history even when an obsolete one-shot animation is skipped. Do not use animation completion as the only mechanism that reveals a result or advances a gameplay command.

Same ID/epoch updates adjust the current cue, not replay it. A newer accepted revision supersedes stale work. Hidden tabs pause unnecessary loops and retain only bounded state; reconnect/visibility return seeks or skips according to the current snapshot and cue policy. Never replay a queue of old hits/rewards. Run/member/permission changes dispose old surfaces and callbacks.

One browser adapter owns each scoped animation clock/handle. Reuse existing frame schedulers where appropriate; CSS/Web Animations may execute DOM motion while renderer frame loops evaluate camera/GPU work. They share timeline semantics and disposal, not competing polling timers. Handle zero-duration/reduced-motion completion without relying on `animationend`. A bounded completion fallback checks current ownership before cleanup and never authorizes gameplay.

Candidate tuning bands for playtesting: UI feedback 120-220 ms; panels/cards 180-350 ms; portrait reveal 350-700 ms; scene dissolve 600-1000 ms; establishing pan/zoom 4-8 seconds then rest. These are initial design candidates, not measured limits or mandated replacements for current timings. Freeze calibrated profile values after observation. Dialogue readability and actual voice duration are independent constraints; do not shorten a spoken line because a camera move ended. Essential controls and current results remain available throughout.

## Profiles, tuning and disable controls

Add a separate proposed `[presentation.motion]` configuration; image-generation enablement does not control whether existing art can animate. Use typed validated settings, effective-source/profile revision diagnostics and the same project configuration precedence conventions as the image plans. Keep controls scoped to their owner: deployment defaults, authorized session/display profile and personal device preference. One player's local preference must not silently change other players' screens.

Session/display tuning belongs in the launcher's existing Settings surface, consistent with the project's start-flow contract; do not add a second gameplay-settings page to the account website. Personal motion/inspection controls belong in the existing authorized player/device preference flow.

Profiles: `full`, `reduced`, and `off`, with per-family toggles for scene transitions, automatic camera, ambient effects, impact effects and UI transitions. `off` removes automatic/decorative motion but preserves manual inspection/navigation and direct state changes. `reduced` substitutes stationary readable emphasis for shake, forced zoom, parallax and major spatial travel. Include separate flash avoidance and manual camera reset; do not implement reduced motion by merely making spatial motion faster.

Tune duration/easing presets, maximum simultaneous layers/cues, camera travel/zoom amplitude, impact strength/refractory interval, ambient intensity, decode/GPU memory and hidden-tab behavior. Avoid exposing a giant unvalidated bag of per-element CSS. Freeze normal cue parameters on start; changes apply to new cues, while stricter accessibility/disable changes cancel or settle affected active effects immediately into the current readable state.

User/device reduced-motion preference is respected by default. A deliberate personal full-motion preference may override the detected device default where supported, but cannot relax a stricter session/accessibility constraint. Persist explicit preference separately from detection and show the effective value/reason. Updating an OS preference re-evaluates the effective profile. All executors, including dice, health, CSS, Web Animations and GPU cameras, consume the same result.

For game-linked cues, apply the authorized audience/profile constraints server-side before composition, serialization and asset forecasting. Local preferences/capability checks may clamp further immediately; they do not authorize a less restrictive server plan. Propagate a persistent preference update through the existing authorized settings path while applying its stricter local effect immediately. Unknown/invalid profile data retains essential readable state and omits optional effects.

Decorative home/character video loops use their authorized poster/still fallback in reduced/off mode; stop unnecessary playback/decode while retaining the selected asset identity. Required narration/captions follow their independent audio/text settings and are not removed just because their decorative visual stopped. A clip without a usable still gets the existing prepared static fallback, not a new generation request. Explicit user-requested media playback remains a separate play/stop action with its own controls; automatic profile changes do not silently start it. Dice use the static result equivalent already specified above.

Example proposed settings (illustrative values, not an existing supported schema):

```toml
[presentation.motion]
profile = "full"
respect_system_preference = true
scene_transitions = true
automatic_camera = true
ambient_effects = true
impact_effects = true
ui_transitions = true
scene_swap_ms = 800
panel_ms = 240
camera_max_zoom = 1.12
```

The motion setting never toggles image generation, transforms, video generation or audio spending. If an effect would benefit from a new crop/mask/video, its owning presentation/media planner may request that asset through the normal API and configured execution mode; the motion runtime uses a ready fallback until then. It cannot make a provider call on a frame tick or silently regenerate an image to achieve an effect.

## Performance, privacy and accounting

Reuse decoded assets and bounded GPU resources; account for both sides of a transition. Prefer transform/opacity motion for ordinary layers; measure blur/filter/masking and large offscreen surfaces before enabling them broadly. Do not permanently retain `will-change`, hidden videos, stale canvases, duplicate listeners or per-cue frame loops. Bound expensive preparation and prioritize input, readable UI and current frames over decoration.

Measure representative real phone and shared-display configurations, including the existing flat and GPU renderer paths. Choose device budgets for input latency, frame-time tails, peak decoded/GPU memory, asset readiness and audio drift from evidence. On overload, remove ambient effects/expensive filters and simplify transitions before degrading essential state/input. Unsupported motion primitives have a tested direct/static fallback.

Only already permitted assets/subjects enter a cue or prefetch manifest. Hidden entities cannot leak through a zoom target, thumbnail, future-shot identifier, preload or timing difference. Wipe/crop/opacity is not access control; private pixels must never be loaded on the shared display. Retain existing asset-revocation and stale-epoch checks through decoding and rendering.

Local animation has no incremental AI supplier charge. Track frame/decode/upload/memory usage separately from provider costs and link any requested/generated asset to its existing cost record. Changing zoom/transition settings does not create a new image-generation cache key; a genuinely requested raster derivative has its own transform key. Video playback reuses an asset without billing generation again.

Extend existing presentation telemetry with cue/epoch/recipe/profile, readiness, request-to-first-display, actual start/end, interruption/skip/fallback reason, active layer count, long frames and audio drift. Record logical transition events separately from observed frames; a DOM attribute change is not proof that a user saw it. Keep private image/prompt contents out of ordinary diagnostics.

## Delivery sequence

| Phase | Deliverable | Exit evidence |
| --- | --- | --- |
| M0: Inventory and design freeze | Enumerate active effect/gesture/clock paths, source authority and current timing; select shared type owner; freeze semantic recipes, origin/timebase and accessibility behavior | Source/build/config inventory, current behavior fixtures; deliberate dice/health preference change recorded |
| M1: Shared lifecycle and primitives | Typed cue/profile contract, channel arbitration, elapsed-time lifecycle, disposal; existing conductor integration | Controlled-clock order/interruption/dedupe tests; no second sequencing owner or leaked handles |
| M2: Image transitions and inspection | Ready image swap, portrait reveal, slide/wipe/dissolve, focal zoom/pan, swipe/pinch/buttons | A/B/C churn, slow/failed assets, gesture cancellation, focus/scroll and manual-vs-auto camera acceptance |
| M3: Existing game integration | Creator, TV plate/figure swaps, item views, home loops, health/dice/turn cues, battle cameras and phase changes | Existing outcomes preserved; effective profiles reach every execution path; flat/GPU/video fallbacks work |
| M4: Tuning and diagnostics | Config/runtime controls, profile changes, performance metrics, optional ambient/cinematic recipes | Disable/reduced-motion mid-cue, real device results, correct cost/asset attribution and cleanup |
| M5: Independent integration acceptance | Current build exercised on TV/player/creator/combat flows, repeated updates, reconnect, visibility and asset failures | Human-paced visual/audio observation, retained evidence and measured budgets; old competing paths removed |

M1/M2 can use prepared assets and proceed before image-backend migration completes. Integration must eventually use the image plans' stable asset references/variants. No paid image/video generation is required to prove swipe, zoom, lifecycle or motion preferences. Keep one active execution path per migrated surface; shadow comparisons may compare pure plans, not play duplicate animations/audio.

Acceptance includes:

- Same-phase polling does not restart portrait/plate reveal, clear drafts or move focus. Creator generation failure leaves usable controls and the prior valid image.
- Scene transitions never expose a blank hole or unauthorized/stale subject; delayed B cannot replace newer C. Asset decode/upload failure uses the prepared fallback.
- Swipe/zoom works with touch, mouse and keyboard; scroll, browser zoom, pointer cancellation, resize and RTL navigation remain correct. Selecting a card cannot equip/accept an item/portrait.
- Simultaneous camera/health/selection cues respect channel ownership; no transform stomping. Manual zoom wins over automatic focus until released.
- Repeated combat snapshots/reconnect never replay a hit or die roll; static/reduced presentation retains exact results, status and legal input. Motion never shortens a rules window.
- Off/reduced/system preference changes affect active CSS, Web Animations, video dice and GPU motion; essential text/result remains readable. Hidden tabs stop unnecessary work and return to current state.
- Long narration/caption, barge-in, late audio, autoplay blocking and skipped cinematic preserve audible/readable content under existing playback semantics; observe actual audio as well as visuals.
- Repeated navigation and asset churn leave bounded nodes, animation handles, textures, decoded buffers and listeners. Measure input responsiveness and frame-time tails on recorded devices/builds.
- Runtime fixtures detect unowned animation launches, duplicate sequencing or provider calls from motion code. Audit direct CSS/DOM/GPU paths with reviewed renderer exceptions rather than banning animation keywords.

Planning approval establishes the intended contract only. Implementation, visual feel, audio synchronization and device performance remain unverified until the corresponding integration phases produce evidence.
