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
| Plate prompt and reference composition | `crates/df-tools/src/gameplay/imagery/plate_prompt.rs`, `photo_edit.rs`, `party_sheet.rs`, `scene_prompt.rs` | `PlateReferences`, `scene_plate`, `model_plate`, fixed-harbor photograph edit, graph edits |
| Scene graph and shot staging | `crates/df-tools/src/gameplay/scene_graph.rs`, `shot.rs` | graph revision, camera, speaker, actor spots, props and light |
| Plate lifecycle and timing | `crates/df-tools/src/gameplay/living_scenes/plates.rs` and `plates/` | request, render, head location, ready/failed, projection, stale result handling |
| Generated character and place references | `assets/ui/generated/` | checked-in reviewed references; runtime output never belongs here |
| Runtime generated cache | `DF_GAMEPLAY_IMAGERY_CACHE` or `artifacts/tmp/play/imagery-cache` | keyed image bytes and `<key>.layout.json` notes |
| Provenance | run-local `imagery.jsonl` | model, kind, prompt length, reference hashes/count/bytes, result and elapsed time |
| Server projection and visual summary | `crates/df-tools/src/gameplay/dev_control/reads.rs` | `/dev/view`, `/dev/scene`, `visual_assets` |
| Agent timeline | `crates/df-tools/src/gameplay/dev_control/timeline.rs`, `reads.rs` | `/dev/timeline?last=N`, bounded 4096-event in-memory mirror |
| Browser plate/source selection | `crates/df-tools/src/gameplay_browser/live_tv.rs`, `live_stage.rs` | `record_visual_source`, plate state, `data-visual-source-*`, image load events |
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
& 'C:\Program Files\Git\bin\bash.exe' ./development/run-gameplay.sh --harbor
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
  status, failure category, and elapsed milliseconds.
- `gameplay.living_scene.plate_requested`, `plate_render_requested`,
  `plate_ready`, `plate_failed`, and `plate_projected`: scene id, graph
  revision, plate id/image key identity, state, source selection, and request to
  projection timing.
- Browser `visual-source` presentation records: requested and effective source,
  selected shot, plate state, and page-local asset hashes.
- line, clip, scene, check, combat, and client presentation records that explain
  whether a visual change was delayed by the stage machine.

The resource filter and summary are available directly from the CLI:

```powershell
& '.\artifacts\build\play\debug\df-transport-fixture.exe' dev --pretty timeline --last 1000 --type image
& '.\artifacts\build\play\debug\df-transport-fixture.exe' dev --pretty timeline --last 1000 --type plate
& '.\artifacts\build\play\debug\df-transport-fixture.exe' dev --pretty timeline --last 1000 --type browser_media
& '.\artifacts\build\play\debug\df-transport-fixture.exe' dev --pretty timeline --last 1000 --type video --summary
```

The accepted resource types are `text_model`, `speech_input`, `voice`, `image`,
`plate`, `video`, `browser_media`, and `state`. Events report lifecycle stages
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

## Visual review loop

After every browser interaction, take a screenshot and record the current URL,
scene revision, `data-scene-state`, `data-plate-state`, selected source, and image
natural dimensions. DOM state proves wiring; the screenshot decides whether the
result looks photographic, medieval/old-world, readable, and free of overlaps.
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
