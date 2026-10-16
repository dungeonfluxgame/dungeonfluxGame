# Visual resource operations manual

This is the short path for inspecting generated art, scene plates, fallbacks, and
their timing in a local two-player dialogue run. It is an operational guide for
agents. The design contracts remain in `planning/asset-engine.md`,
`planning/observability.md`, and `planning/client-architecture.md`.

## Source map

| Concern | Current source of truth | What to inspect |
| --- | --- | --- |
| Image request identity, provider call, cache, manifest, HTTP bytes | `crates/df-tools/src/gameplay/imagery.rs` | `ImageRequest`, `ImageKey`, `ImageFailure`, `imagery.jsonl`, `GET /gameplay/imagery/<key>` |
| Reference loading and photographic allowlist | `crates/df-tools/src/gameplay/imagery/references.rs` | `ReferenceStore::photographic`, `load_photographic`, neutral carrier policy |
| Plate prompt and reference composition | `crates/df-tools/src/gameplay/imagery/plate_prompt.rs`, `photo_edit.rs`, `cast_sheet.rs`, `scene_prompt.rs` | `PlateReferences`, `scene_plate`, `model_plate`, numbered canonical cast contact sheet, photograph edit, graph edits |
| Scene graph and shot staging | `crates/df-tools/src/gameplay/scene_graph.rs`, `shot.rs` | graph revision, camera, speaker, actor spots, props and light |
| Plate lifecycle and timing | `crates/df-tools/src/gameplay/living_scenes/plates.rs` and `plates/` | request, render, head location, ready/failed, projection, stale result handling |
| Generated character and place references | `assets/ui/generated/` | checked-in reviewed references; runtime output never belongs here |
| Runtime generated cache | `DF_GAMEPLAY_IMAGERY_CACHE` or `artifacts/tmp/play/imagery-cache` | keyed image bytes and `<key>.layout.json` notes |
| Provenance | run-local `imagery.jsonl` | model, kind, prompt length, reference hashes/count/bytes, result and elapsed time |
| Server projection and visual summary | `crates/df-tools/src/gameplay/dev_control/reads.rs` | `/dev/view`, `/dev/scene`, `visual_assets` |
| Agent timeline | `crates/df-tools/src/gameplay/dev_control/timeline.rs`, `reads.rs` | `/dev/timeline?last=N`, bounded 4096-event in-memory mirror |
| Component lifecycle and actual visibility | `crates/df-observe/src/presentation.rs`, `gameplay_browser/presentation_timing.rs`, `presentation_ingress.rs` | closed component/phase types, readable duration, effective visibility and content-free correlation |
| Live placement contract and CLI | `crates/df-tools/src/layout_contract.rs`, `dev_cli/layout.rs` | typed geometry, IDs, revisions, limits, `dev layout` flags |
| Live placement authority and browser application | `crates/df-tools/src/gameplay/dev_control/layout.rs`, `gameplay_browser/layout.rs` | authenticated configuration, per-display inventory/acknowledgment, isolated override stylesheet |
| Browser plate/source selection | `crates/df-tools/src/gameplay_browser/live_tv.rs`, `live_stage.rs` | `record_visual_source`, plate state, `data-visual-source-*`, image load events |
| Tempo driven music plan | `crates/df-tools/src/music_score.rs`, `gameplay_browser/live_music.rs` | server `music_plan`, browser `data-music-state`, `/assets/music/<track>` |
| Fixed public assets and allowlisted routes | `crates/df-tools/src/gameplay/public_assets.rs`, `gameplay_browser/public_scene_assets.rs` | startup snapshots and exact route registration |
| Paid resource startup toggles | `crates/df-tools/src/gameplay/startup_configuration.rs`, `gameplay.rs` | imagery, plates, voice, STT, home/scene video summaries |

The server owns scene state, media admission, cache identity, and the effective
source sent to clients. The browser reports presentation and renders that source;
it must not invent a replacement asset or game outcome.

## Fast two-player dialogue run

Start from the repository root in PowerShell. Use a fresh database for a clean
asset and timing record. Keep the server revision, build directory, database name,
and run directory together in the test notes.

```powershell
$env:DF_GAMEPLAY_LISTEN='127.0.0.1:63319'
$env:DF_GAMEPLAY_PUBLIC_URL='http://127.0.0.1:63319'
$env:DF_GAMEPLAY_DEV_CONTROL='on'
$env:DF_GAMEPLAY_STORYTELLER='on'
$env:DF_GAMEPLAY_VOICE='on'
$env:DF_GAMEPLAY_IMAGERY='on'
$env:DF_GAMEPLAY_PLATES='on'
$env:DF_GAMEPLAY_HOME_VIDEO='on'
$env:DF_GAMEPLAY_SCENE_VIDEO='on'
$env:DF_GAMEPLAY_STT='on'
& 'C:\Program Files\Git\bin\bash.exe' ./development/run-gameplay.sh --harbor --new-game
```

Copy the `DEV CONTROL` URL and token file from startup, then run the fixture
binary. This joins both seats, creates the fixture heroes from the live offers,
and stops in dialogue before combat.

```powershell
$env:DF_DEV_CONTROL_URL='http://127.0.0.1:63319'
$env:DF_DEV_CONTROL_TOKEN_FILE='C:\path\from\DEV CONTROL\dev-control-token'
& '.\artifacts\build\play\debug\df-transport-fixture.exe' dev --timeout 600 play --to dialogue
```

If the fixture is not built, use `cargo run --locked -p df-tools --bin
df-transport-fixture -- dev --server $env:DF_DEV_CONTROL_URL --token-file
$env:DF_DEV_CONTROL_TOKEN_FILE play --to dialogue`. The game still applies its
ordinary presentation gates; a quickstart is not a bypass of server authority.

Checkpoints are pinned to their build ID. Resume an old database only with its
matching retained native binary and WASM files. A `Checkpoint(BuildMismatch)`
refusal after installing another build is expected; start a new quick test with
`--new-game` and preserve the old database. Do not relabel a different binary to
match the old checkpoint. `--no-build` reuses an already installed matching build.

For dialogue tests, inspect the `disposition` returned by `dev say`. `status: said`
alone does not mean a social question was spoken: an input such as "I study the
courier's face" can be classified as an action and perform an Investigation check.
Use an explicit question addressed to the NPC, then confirm the hero and reply
in `dev view --as tv` and the actual display before measuring reading time.

## Inspect the complete resource path

The token is a bearer credential. Never print it into a retained log or commit.
The following requests are read-only and return JSON:

```powershell
$token=(Get-Content -Raw $env:DF_DEV_CONTROL_TOKEN_FILE).Trim()
$auth=@{ Authorization = "Bearer $token" }
Invoke-RestMethod "$env:DF_DEV_CONTROL_URL/dev/describe" -Headers $auth
Invoke-RestMethod "$env:DF_DEV_CONTROL_URL/dev/state" -Headers $auth
Invoke-RestMethod "$env:DF_DEV_CONTROL_URL/dev/view?as=tv" -Headers $auth
Invoke-RestMethod "$env:DF_DEV_CONTROL_URL/dev/scene?name=Brakka" -Headers $auth
Invoke-RestMethod "$env:DF_DEV_CONTROL_URL/dev/timeline?last=250" -Headers $auth
```

`dev describe` is the live command contract. `dev view --as tv` includes
`visual_assets` with `requested_source`, `server_effective_source`,
`plate_state`, and a local identity hash for the effective asset. `dev scene`
also exposes scene, backdrop, plate, cast sprite and item states. These summaries
intentionally omit prompt text, image bytes, credentials, and asset URLs.

The timeline is the first diagnostic to retain. Each record has sequence,
wall-clock time, process-relative elapsed time, event name, readable line, and
typed `fields`. Relevant event families include:

- `gameplay.imagery.render_requested`, `render_ready`, and `render_failed`:
  image kind/key, prompt character count, reference count and byte lengths,
  status, failure category, and elapsed milliseconds. Request records also carry
  `request_schema`, `prompt_sha256`, and ordered `reference_sha256`, including in
  fixed harbor runs without an imagery manifest.
- `gameplay.living_scene.plate_requested`, `plate_render_requested`,
  `plate_ready`, `plate_failed`, and `plate_projected`: scene id, graph
  revision, plate id/image key identity, state, source selection, and request to
  projection timing.
- `gameplay.living_scene.plate_story_observed` and `plate_refresh_decision`:
  committed NPC reply sequence/hash, pending settle, queue admission, suppression,
  rate limiting and in-flight state. An unchanged reply does not request an edit.
- `gameplay.living_scene.head_located` and `actor_heads`: per-batch and aggregate
  head measurement outcomes under `--type plate`. Inspect the exact `image_key`,
  `target_ids`, `located_ids`, per-batch `actor_id_mapping`, `cache_source`,
  `elapsed_ms`, failure/partial/truncation fields, stage and status. Aggregate
  records use `located_ids`; mappings belong to each batch. Provider, durable-note
  and in-process cache results are distinct. Failed provider batches are not
  written as durable layout notes; an otherwise successful partial result can
  retain valid heads and leave missing actors in the named caption lane.
- `gameplay.npc_target`: input ID, canonical actor ID, decision source, model,
  latency, reason and validation. Use `--type text_model` for Jev decisions and
  `--type state` for exact-name selection and the server's current-cast validation.
  A model selection is a proposal; publication still checks the current scene and
  cast. An absent or ambiguous target must not become an unrelated focus reply.
- Browser `visual-source` presentation records: requested and effective source,
  selected shot, plate state, and page-local asset hashes.
- `gameplay.music.plan`: the public tempo source, phase/intensity, suite, location
  bed, scene colour, combat-layer eligibility, and projected mix gains. It is
  emitted when `dev view --as tv` snapshots the display; the browser's
  `data-music-state` remains the delivery/playback observation.
- line, clip, scene, check, combat, and client presentation records that explain
  whether a visual change was delayed by the stage machine.

The resource filter and summary are available directly from the CLI:

```powershell
& '.\artifacts\build\play\debug\df-transport-fixture.exe' dev --pretty timeline --last 1000 --type image
& '.\artifacts\build\play\debug\df-transport-fixture.exe' dev --pretty timeline --last 1000 --type plate
& '.\artifacts\build\play\debug\df-transport-fixture.exe' dev --pretty timeline --last 1000 --type browser_media
& '.\artifacts\build\play\debug\df-transport-fixture.exe' dev --pretty timeline --last 1000 --type video --summary
```

The accepted resource types are `text_model`, `speech_input`, `voice`, `music`, `image`,
`plate`, `video`, `browser_media`, and `state`. Tempo and music lifecycle records
can be filtered with `--type music`; the plan itself is always available from
`dev view --as tv`. Events report lifecycle stages
including `requested`, `queued`, `ready`, `failed`, `played`, `projected`, and
`delivered`; browser observations can also report `observed` or `hidden`.
The summary reports `matched`,
`summary.window_events`, distinct `resource_count`, and counts by resource type,
stage, and status. Status values can also identify `cached`, `fallback`,
`partial`, or `cancelled` outcomes. `--summary` returns counts with an empty
`events` array, which is useful for a quick pass over a long run.

The corresponding HTTP request is
`/dev/timeline?last=1000&type=plate&summary=true`; invalid types or a non-boolean
summary value are rejected by the server.

For NPC placement, compare `.view.scene.plate_actor_heads` from `dev view --as tv`
with the display's active image key. Every head record belongs to its `image_path`;
coordinates from an earlier edited photo are invalid for a new photo. Canonical
NPC bubble layout IDs are `bubbles-courier`, `bubbles-bandit`, or the corresponding
current cast ID. A same-image measurement may arrive after a line opens. The
current readable line retains its position; the next line uses the latched head.
Verify both with screenshots and browser component events, since valid server
coordinates alone do not prove that the browser applied them.

Placement maps the head through the active image's measured cover frame, then
converts viewport coordinates into the bubble layer's local origin and scale.
Nearby positions on both sides and above/below compete by distance to that
speaker; the authored side breaks ties. All measured heads and interface
obstacles still constrain placement. The bubble host exposes `data-side` as its
lateral preference and `data-tail` as the actual pointer edge, with `--tail-x`
and `--tail-y` locating the head relative to the bubble. Use the pointer edge,
image key, canonical actor ID and screenshot together when checking association.

For a compact event table in PowerShell:

```powershell
$timeline=Invoke-RestMethod "$env:DF_DEV_CONTROL_URL/dev/timeline?last=1000" -Headers $auth
$timeline.events | Where-Object {
  $_.event -match 'imagery|plate|visual|presentation|scene'
} | Select-Object sequence,elapsed_ms,event,line | Format-Table -AutoSize
```

The same checks are available through the fixture CLI and keep the output in the
agent's normal JSON stream:

```powershell
& '.\artifacts\build\play\debug\df-transport-fixture.exe' dev --pretty view --as tv
& '.\artifacts\build\play\debug\df-transport-fixture.exe' dev --pretty scene Brakka
& '.\artifacts\build\play\debug\df-transport-fixture.exe' dev --pretty timeline --last 1000
```

`dev view --as tv` also includes `music_plan` for the shared display. It is the
server-derived public plan: the tempo cue phase/intensity, cue sequence, 84 BPM
beat grid, suite, location ambience, scene colour, scene profile, layer gains, filter and
transition timing. Social dialogue uses the `social-restrained` profile: tempo still
drives its phase and intensity, while the bed/motif are softened, the tension pulse
is reduced, and the combat layer remains silent. The browser's `data-music-state` attribute reports the applied
AudioContext state, loaded/failed track names, arrangement, duck reason and
current bus levels. Compare both when diagnosing a music mismatch: the plan is
authoritative; the DOM state proves delivery and application.

`dropped` and `capacity` are part of the response. A nonzero `dropped` means the
bounded process-local mirror no longer contains the earliest events. The timeline
is diagnostic evidence, not authoritative game state; preserve a run-local copy
with the build and database identity when investigating a regression.

The intended correlation is the same scene id and graph revision from request
through projection. Current live runs can show different local identity hashes
between the plate request/render records and the projected display asset because
the former identifies the pack scene path while the latter identifies the
effective served asset path. Treat that as the known identity-alignment gap and
compare scene id, graph revision, plate id, and image key together until the
diagnostics follow-up makes the domains explicit and joins them in one record.

There are three additional coverage gaps to account for during diagnosis:

- Some server generation records do not carry room and scene context. Scope them
  with the run/build, event sequence, `/dev/state`, `/dev/scene`, and nearby
  revision-bearing events rather than assuming a resource id identifies a room.
- Browser presentation records carry resource type/id, stage, status, source
  details, and hashed room/run scope where available, but do not carry the
  gameplay revision. Read the server revision from `dev view --as tv` and use
  the browser event sequence for presentation ordering.
- Home-video still preparation happens before the clip key is derived. Early
  `prepare_still` failures can therefore have no video resource key; inspect the
  failure category and the painting/scene context from adjacent state events.
  Once preparation succeeds, the clip key is the SHA-256 of the prepared still,
  model, prompt, duration, and recipe.

## Move display elements live through the agent CLI

Saved research for a future image-localization layer is in
[Segment Anything feasibility](../planning/segment-anything-feasibility.md).
It includes the official Meta SAM API, coordinate/provenance requirements,
published provider options, and the local billing/TLS probe outcomes. The owner
deferred SAM integration; exact DOM geometry remains available through layout
readback below.

The development layout surface is enabled by `DF_GAMEPLAY_DEV_CONTROL=on`.
After installing a build containing it, changing an element uses the existing
authenticated CLI and does not require recompilation, a server restart, a page
reload, or a gameplay action.

```powershell
$fixture='.\artifacts\build\play\debug\df-transport-fixture.exe'
& $fixture dev layout list
& $fixture dev layout get scene-context
& $fixture dev layout transform scene-context --translate-y -24 --unit px
& $fixture dev layout transform scene-context --scale 0.96 --rotate 0
& $fixture dev layout get scene-context
& $fixture dev layout reset scene-context
```

Use the server URL and token-file environment variables from the quick-start
section. Never paste a bearer token into commands, screenshots, or reports.
`layout list` discovers the current game DOM inventory. Stable semantic aliases
include `scene-context`, `dm-overlay`, `narration`, `subtitle`, `background`,
`plate`, `choices`, `hud`, and `item-card` when those elements exist. Other nodes
receive runtime IDs; inspect their classes, owner metadata, and measured bounds
before selecting one. The registry also covers descendants such as individual
images, labels, figures, cards, loaders, and controls. A composed photograph is
one image; people painted into its pixels are not independent DOM elements.

`transform` accepts translation, uniform or separate-axis scale, and rotation in
degrees. Translation units are `px` or `percent` (percentage of that element's
own size). `move <id> --x <n> --y <n> --unit px|percent` sets the translation
offset from the element's natural layout; it is not a world-coordinate move.
Partial updates compose with the existing override. Omitting an ID from
`layout reset` clears every override. Reset removes only layout-owned rules and
reveals the application's current styles and animations.

An accepted command is pending until the browser reports the matching layout
revision. `layout get` returns per-display measured rectangles, viewport,
configured and effective transforms, sample/revision, and applied/absent/stale
status. Re-read it after a command and inspect a real screenshot. Layout revision
is independent of gameplay revision. Runtime overrides survive connection
recovery in the same server process; they are development settings rather than
saved campaign state. Missing nodes stay explicit instead of pretending they
moved. The inventory and overrides have fixed capacities and report overflow;
unknown or stale IDs, unknown flags, and invalid geometry are refused.

Layout changes and browser acknowledgments use the existing structured timeline.
Inspect `dev timeline --type browser_media` for `gameplay.layout.*` records.
Component lifetimes can be filtered separately, for example:

```powershell
& $fixture dev timeline --last 1000 --component speech_bubble
& $fixture dev timeline --last 1000 --component check_card
& $fixture dev timeline --last 1000 --component plate_incoming
```

Component records distinguish requested/opened/readable/closing/hidden phases,
effective visibility, and measured visible duration. A logical speaking state or
a mounted node alone does not prove that a person could see its text.

Keep the timeline mirror's `dropped` count separate from `DISPLAY gap` records.
The former measures the bounded server mirror; the latter carries client-reported
capture loss. In RAGURA, the mirror reported zero drops while browser batches
reported one lost record. The current aggregate cannot identify its exact loss
source. Preserve these gaps and do not certify a complete lifetime from an
incomplete capture. A 332-character regression in that run had a 24,080 ms reading
budget and a 27,206 ms logical lifetime, while both caption components reported
zero visible time; screenshots confirmed the missing question.

## Visual review loop

### Same-scene story refresh and caption acceptance

Authored harbor scenes now derive subtle plate edits from completed public NPC
replies. The current photograph is the first reference, followed by the canonical
cast sheet and the matching speaker reference when available. This is cosmetic
continuation: speech cannot open or transfer objects, invent participants, or
commit a game outcome. Changes settle for 3 seconds and use a single latest
pending slot with at least the configured interval (20 seconds in the local run).
Repeated identical speaker/text is suppressed; stale scene/story/revision results
cannot replace the current plate.

Compare ordered reference hashes between request records. Keep the cast-sheet
digest stable across edits and verify the first digest identifies the previously
shown photograph. `request_to_projection_ms` is frozen for the original request;
edited/directed projections report null when no matching request clock exists.
`scene_request_age_ms` is separate and must not be described as provider latency.

Build `layout-b6cb696d`, room PODOBU, supplied live acceptance of automatic story
edits and the caption ownership repair. A repeated 332-character social question
was visibly held for 26,950 ms at 1460 pixels, above its 24,080 ms reading budget.
At 1024x768 the named fallback retained the full text and reported 26,808 ms visible
time. An independent Chrome observer measured 27,236 ms for the full question.
The named lane used a documented live `subtitle` translation of -96 px to clear
the existing scene-context overlap; this is not acceptance of the default layout.

For a fresh reply, approximately 100 ms DOM sampling captured incoming opacity
from zero through six intermediate values to one in about 1.02 seconds. The
outgoing plate remained fully opaque throughout the fade, then retired. Screenshots
and timeline evidence are retained by the coordinator. Neither this evidence nor
a successful provider response certifies dialogue quality or audible playback.

Build `routing-0187f83e`, room GOSASU, subsequently accepted canonical NPC routing
and physical speaker placement in the actual Codex display and independent
Chrome observation. Jev selected the background Bandit in 132 ms; his box stayed
below his own head after an automatic photo refresh, with a head-facing pointer.
Courier's box stayed below Courier. A 332-character question remained visibly
held for 26,726–26,983 ms across the two browser lifecycle producers, above the
24,080 ms reading budget. Seven crossfade samples showed no blank frame. A
47-sample zoom check reached 1.16x and returned to 1x without a speaker/head/HUD
collision. The waiting indicator followed Bandit as well. The game remained in
NPC dialogue with two heroes and no combat. Evidence is indexed by the
coordinator's `gosasu-routing-acceptance.json`; the detailed history and remaining
default-layout, capture-gap and dialogue-quality limitations are in
`planning/issue-triage.md`. The -96 px subtitle override still applied in this run.

After every browser interaction, take a screenshot and record the current URL,
scene revision, `data-scene-state`, `data-plate-state`, selected source, and image
natural dimensions. DOM state proves wiring; the screenshot decides whether the
result looks photographic, medieval/old-world, readable, and free of overlaps.
Compare each visible hero and NPC with the reference art and canonical cast list:
a photographic image can still replace a party member or change a speaker's face.
Inspect the shared display and the player view that matters for the issue. A plate
can be HTTP-ready while the visible stage still shows its previous source.

For an empty or suspicious plate, check in this order:

1. `dev view --as tv`: is `plate_state` `READY`, and is
   `server_effective_source` `live_plate`?
2. `dev timeline`: did the same scene id and graph revision reach
   `plate_requested`, `plate_ready`, and `plate_projected`? Compare elapsed
   times and image key identity.
3. `dev scene`: is the plate URL/state present, and are cast references in the
   expected source category?
4. Browser screenshot and network-visible image dimensions: did the requested
   image load, and did the stage activate it after decode?
5. Provenance and cache: when the run uses a generated adventure, inspect its
   run directory's `imagery.jsonl` for cache-hit/provider-result status, reference
   hashes, and prompt size. A fixed `--harbor` run may have no `imagery.jsonl`:
   `gameplay.rs` passes a manifest directory only when a prepared generated
   adventure exists. For fixed harbor, use the timeline's image key and inspect
   that key's cache file and `<key>.layout.json` sidecar instead. Do not copy
   prompts or image bytes into agent output.

For a CGI or illustrated regression, inspect the actual bytes named by the
provenance entry, or by the timeline image key for fixed harbor, with an image
viewer. A filename containing `photo` is not proof of photographic content.
Confirm the selected reference files are the reviewed documentary variants.
For the fixed exterior harbor, the first provider reference is the complete
`scene-salt-quay-candid-documentary-photo.webp` frame; check its byte length
against the first `reference_byte_lengths` value in the image-request event.
`ReferenceStore::load_photographic` accepts the explicit
`-documentary-photo` suffix and deliberately does not fall back to a painted base
or older nominal photo stem. Do not reintroduce an illustrated fallback to hide a
provider failure; keep the failed/pending state visible and retain the previous
known-good scene according to the stage contract.

The known regression shape is `requested_source=scene_plate`,
`server_effective_source=live_plate`, and `PLATE_STATE_READY`, while the screenshot
still looks like polished CGI. That is a content acceptance failure, not a loading
failure. Keep the key, prompt/reference counts, reference hashes, commit, and
screenshot together; then inspect the selected source bytes and the shot/prompt
clauses before changing cache or stage code. A ready HTTP response cannot certify
the photographic look.

## Cache identity and invalidation

An image key is the lowercase SHA-256 identity of the request. The request key
includes image kind, prompt, model, quality, content rating, output format and
reference bytes/hashes. Replacing a reviewed reference or changing the prompt
therefore creates a new key; an old byte remains valid for an older run/reference.
The plate layout note lives beside the image as `<key>.layout.json`.

Runtime generated output belongs under `artifacts/` or the explicit
`DF_GAMEPLAY_IMAGERY_CACHE` directory. For a generated-adventure run, the
prepared adventure directory can also contain the run-local `imagery.jsonl`
provenance manifest; startup uses that manifest to re-register retained keys.
Fixed `--harbor` runs currently pass no manifest directory, so absence of
`imagery.jsonl` is expected. Their diagnostic source is `/dev/timeline` plus the
cache key and `<key>.layout.json` sidecar. Scene plates are JPEG cache files;
other image kinds may use WebP or PNG according to their output specification.
Checked-in source references under `assets/` are immutable project inputs and
should not be cleaned as cache.

To clear one stopped run's disposable imagery cache, resolve and verify the exact
directory first, then remove only that directory. Keep `imagery.jsonl` when a
generated-adventure run has one and its provenance is needed for review; remove it
only as part of intentionally retiring the same run. Fixed harbor runs have no
manifest to preserve. Never clear an active run or a broad `artifacts/` parent.

## Safe stop and retained evidence

Stop through the authenticated dev surface after the final screenshot:

```powershell
$body='{}'
Invoke-RestMethod "$env:DF_DEV_CONTROL_URL/dev/shutdown" -Method Post -Headers $auth -Body $body -ContentType 'application/json'
```

If the process is owned by the launcher, Ctrl-C is also supported. Confirm the
server exits, the browser shows its reconnect/stop state, and no test process is
left listening on the preview port. Preserve the final timeline, `imagery.jsonl`,
database name, commit, configuration summary, and screenshots under the retained
evidence location required by the active attempt. Do not treat a quiet log as a
successful visual run.

## Ownership and staged consolidation

The current implementation has a safe split but no single visual-resource
contract: `imagery.rs` owns request hashing/cache/provider publication,
`references.rs` filters input files, `plates.rs` owns scene timing and projection,
and the browser owns activation/reporting. Moving these files together now would
collide with active plate/style and diagnostics work and would blur server timing
with browser presentation.

The recommended owner is a small server-side `df-tools::gameplay::visual_resource`
contract. It should own only:

1. `VisualResourceId` and its provenance fields (kind, source category, identity
   revision, reference digest, model/config revision);
2. `VisualSource` and the explicit fallback policy (`scene_plate`,
   `live_backdrop`, `scene_asset`, or unavailable);
3. cache/manifest lookup results and typed invalidation reasons.

The migration should be staged: first add the contract and parity tests around
the existing `ImageKey` and `visual_asset_summary`; then make `imagery.rs` and
`plates.rs` consume it without changing timing; then make `reads.rs` and browser
diagnostics serialize the same identity/source values. Keep provider/cache bytes
owned by imagery, scene timing owned by living scenes, and DOM activation owned by
the browser. An integration test must prove that a ready plate, a failed plate,
and a stale completion produce the same effective source and identity before and
after the move. Until those stages land, this manual and the existing typed
summaries are the authoritative navigation guide.
