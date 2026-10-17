# World Labs Marble splat worlds: research record

Status: research record, reconstructed on 2026-10-17 from files on disk and the
live World Labs documentation. The original research agent's report was lost, so
this document rebuilds its findings from the proof artifacts, scripts, logs and
saved API docs under `artifacts/tmp/worldlabs`. Anything that could not be
confirmed from those files is marked **unconfirmed**. Writing this record made no
World Labs API calls. The two generation runs on disk were paid calls made by the
earlier research pass. This is not a purchased integration. Revalidate prices,
limits and terms before implementation.

Related planning: [asset provider research](asset-provider-research.md) (no World
Labs row yet), [scene combat binding](scene-combat-binding.md) (the NPC binding
that a splat arena must respect), [issue triage](issue-triage.md) (WIP-08 and the
owner decisions quoted in section 8), and the [narrative engine](narrative-engine.md)
(has no fight-likelihood or arena section yet).

Paths in this document are relative to the repository root
`C:\Users\mreca\Desktop\dungeonfluxGame`.

## 1. What Marble is

Marble is World Labs' generative 3D world service. From a text prompt, a single
image, several images or a video, it generates a photographic 360 panorama and
then a 3D world from that panorama. The world is delivered as Gaussian splat
files, a coarse collider mesh (GLB), a panorama image, a thumbnail and a caption.
Optional high-quality meshes are a separate, paid export.

For DungeonFlux the relevant property is that a generated world is a real-scale
room that can be walked on. The collider mesh gives the walkable geometry, and
the splats give the photographic look. The art contract is photographic only, so
Marble's output fits it.

Sources: [World generation examples, live](https://docs.worldlabs.ai/api/world-generation-examples)
(fetched 2026-10-17); saved copy `artifacts/tmp/worldlabs/docs/api_world-generation-examples.md`.

## 2. The API flow

The flow is one submit, a poll loop, then a free PLY export and asset downloads.
The scripts that did this are in `artifacts/tmp/worldlabs/scripts`.

| Step | Call | What the research runs recorded |
| --- | --- | --- |
| Check balance | `GET /marble/v1/credits` (`scripts/credits.py`) | Balance 7000 before the draft run, 5190 after the marble-1.1 run. |
| Submit | `POST /marble/v1/worlds:generate` (`scripts/generate.py`) | Returns an `operation_id` at once. Round trip 22.6 s (draft) and 18.7 s (marble-1.1). |
| Poll | `GET /marble/v1/operations/{id}` every 5 s in the script | Draft: 5 poll records. marble-1.1: 63 poll records. Progress only says `IN_PROGRESS` until `done` is true. |
| Result | `response` on the completed operation | Holds `world_id`, `assets`, `semantics_metadata`, `caption`, `model` and `cost`. |
| Export | `POST /marble/v1/worlds/{id}:export` with `{"asset_type":"splats","format":"ply","resolution":...}` (`scripts/fetch.py`) | Synchronous for PLY, and free. Returns a signed URL in `response.url`. |
| Download | Plain GET on the signed URLs (no key header) | Collider GLB, thumbnail and pano are fetched from the asset URLs in the world response. |

Request shape used by `scripts/generate.py`: `display_name`, `model`,
`permission: {public: false}`, tags `["dungeonflux-poc"]`, and a `world_prompt`
of type `image` with `image_prompt` as `data_base64` (JPEG), `is_pano: false` and
a short `text_prompt`. The API's `is_pano` default is `auto`, which skips the pano
step when the image is already an equirectangular 360 panorama.

Error behaviour from the saved docs: `400` covers invalid JSON, unfetchable
`uri` inputs and content policy. `402` means insufficient API credits. `422` is a
schema error. `429` is a rate limit, and a `429` start has not been accepted, so
the operation should be polled rather than resubmitted. Generation failures show
up on the operation (`done` true with `error` set), not on the submit call. The
scripts did not hit any error on disk.

Sources: [Generate, saved](artifacts/tmp/worldlabs/docs/api_reference_worlds_generate.md),
[Operations get, saved](artifacts/tmp/worldlabs/docs/api_reference_operations_get.md),
[Export, saved](artifacts/tmp/worldlabs/docs/api_reference_worlds_export.md),
[Errors, saved](artifacts/tmp/worldlabs/docs/api_errors.md),
[Live errors](https://docs.worldlabs.ai/api/errors) (fetched 2026-10-17, not separately
re-read here; the saved copy was used).

## 3. Models, cost and timing

### Models

| Marble model | API `model` | World generation credits | Notes |
| --- | --- | ---: | --- |
| Marble 1.0 Draft | `marble-1.0-draft` | 150 | Fastest. The draft world has no metric scale (see section 5). |
| Marble 1.0 | `marble-1.0` | 1,500 | Legacy, kept for existing work. |
| Marble 1.1 | `marble-1.1` | 1,500 | Recommended starting point by the docs. Used for the proof world. |
| Marble 1.1 Plus | `marble-1.1-plus` | 1,500 base, plus 0 to 1,500 variable | Expands coverage for larger worlds. Not tested here. |

The API default model is `marble-1.0`, but the docs say it will move to
`marble-1.1`. The scripts always pass `model` explicitly.

Sources: [Models, saved](artifacts/tmp/worldlabs/docs/api_models.md),
[Marble models, saved](artifacts/tmp/worldlabs/docs/marble_models.md),
[API pricing, live](https://docs.worldlabs.ai/api/pricing) (fetched 2026-10-17).

### Credits and USD

The API bills in credits. The documented rate is **$1.00 per 1,250 credits**, with a
6,250 credit ($5.00) minimum purchase. Credits bought in the Marble web app cannot
be used through the API.

Usage events and totals, from the saved pricing page and matched to the
operation cost lines in the on-disk JSON:

| Input | Pano step | World step | Total credits | USD at $1 per 1,250 |
| --- | ---: | ---: | ---: | ---: |
| Draft, non-pano image (draft run) | 80 | 150 | 230 | $0.184 |
| marble-1.1, non-pano image (proof run) | 80 | 1,500 | 1,580 | $1.264 |
| marble-1.1, existing pano | 0 | 1,500 | 1,500 | $1.200 |
| HQ mesh export (not run) | n/a | 3,500 | 3,500 | $2.80 |
| PLY splat export | n/a | free | 0 | $0 |

The owner's figure of "about $1.26" for marble-1.1 is confirmed: 1,580 credits is
$1.264. The on-disk records show the balance moving 7,000 to 6,770 (draft run,
230 credits) and 6,770 to 5,190 (marble-1.1 run, 1,580 credits). Both fetch
steps left the balance unchanged, which matches the free PLY export. Total spend
recorded on disk is 1,810 credits, or $1.448. Any earlier spend on this account is
**unconfirmed**, because no log for it is on disk.

Overage: the pricing page says the API admits requests on a low-balance threshold,
not a cost estimate, so a request can take the balance below zero. Overage is
billed at the end of the monthly period, and auto-refill does not cap usage.

### Timing

| Run | Model | Submit to done | Polls | Source |
| --- | --- | ---: | ---: | --- |
| Draft proof | `marble-1.0-draft` | 53.6 s | 5 | `artifacts/tmp/worldlabs/out/draft.log.jsonl` |
| Proof world | `marble-1.1` | 362.7 s (about 6.0 min) | 63 | `artifacts/tmp/worldlabs/out/m11.log.jsonl` and `m11.stdout.txt` |

The docs say a standard world takes "about 5 minutes" (rate limits page, saved and
live). Only two runs exist, so no distribution can be stated. The owner's "about 6
minutes per place" is consistent with the one marble-1.1 run on disk.

### Rate limits

Defaults are about 3 world generation starts per minute and 60 per hour per API
user. Approved accounts can request about 30 per minute for standard models and
about 90 per minute for draft. Limits are approximate, and they apply to starts,
not to running jobs.

Sources: [Rate limits, saved](artifacts/tmp/worldlabs/docs/api_rate-limits.md),
[Live rate limits](https://docs.worldlabs.ai/api/rate-limits) (not re-fetched; saved copy used).

## 4. Outputs and their formats

What the completed operation carries, checked against `out/*.operation.json`:

| Output | Field | On disk for the proof world | Format and notes |
| --- | --- | --- | --- |
| Splats, SPZ | `assets.splats.spz_urls` (100k, 150k on marble-1.1, 500k, full_res) | URLs only, not downloaded | Marble-native, compact. The project rule forbids SPZ, so the pipeline never downloads or reads these. |
| Splats, PLY | `:export` with `asset_type: splats`, `format: ply`, `resolution` (`full_res`, `500k`, `150k`, `100k`) | `m11_100k.ply`, `m11_500k.ply`; draft `draft_100k.ply` | Binary little endian 3DGS PLY. Free. |
| Collider mesh | `assets.mesh.collider_mesh_url` | `m11_collider.glb`, `draft_collider.glb` | GLB, one mesh, u32 indices and float positions. The walkable source. |
| High-quality mesh | `:export` with `asset_type: mesh`, `format: glb` | not requested | Paid (3,500 credits), up to an hour, rate-limited. Not needed for the plan. |
| Panorama | `assets.imagery.pano_url` | `m11_pano_small.jpg` only (see section 7) | Equirectangular PNG, 2560 by 1280 per the export specs page. Draft returned `pano_url: null`. |
| Thumbnail | `assets.thumbnail_url` | `m11_thumbnail.webp`, `draft_thumbnail.webp` | WebP. |
| Caption | `assets.caption` | in operation JSON | Text description. |
| Hosted viewer | `world_marble_url` | in operation JSON | Marble web link. |

The PLY file is 14 float properties per splat, in this order: `x y z`, `f_dc_0..2`,
`opacity` (logit), `scale_0..2` (log), `rot_0..3` (w x y z). There is no
`f_rest_*`. The header was checked directly. Stride is 56 bytes per splat, and the
file sizes agree with that stride plus a header of about 361 to 362 bytes:

| File | Vertices (header) | Bytes on disk | Check |
| --- | ---: | ---: | --- |
| `out/draft/draft_100k.ply` | 99,846 | 5,591,737 | 99,846 x 56 = 5,591,376, plus 361 header bytes |
| `out/m11/m11_100k.ply` | 98,304 | 5,505,385 | 98,304 x 56 = 5,505,024, plus 361 header bytes |
| `out/m11/m11_500k.ply` | 500,000 | 28,000,362 | 500,000 x 56 = 28,000,000, plus 362 header bytes |

The docs describe the low-res PLY as about 500k splats and the full PLY as about
2M. The full_res export was not run, so its size and count are **unconfirmed**.

The collider GLB on disk, read from the JSON chunk only:

| File | Positions | Index count | Triangles | Bytes |
| --- | ---: | ---: | ---: | ---: |
| `out/draft/draft_collider.glb` | 15,228 | 88,170 | 29,390 | 780,356 |
| `out/m11/m11_collider.glb` | 48,058 | 285,864 | 95,288 | 2,490,376 |

The export specs page says the collider is 100k to 200k triangles. The proof
collider has about 95k, just under that range. The source of the mismatch is not
known from disk. The GLB also carries a VEC4 unsigned-byte accessor (probably
vertex colour) and a second VEC3 float accessor. Their meaning is not confirmed.

Sources: [Export file specs, live](https://docs.worldlabs.ai/marble/export/specs) (fetched
2026-10-17; saved copy `artifacts/tmp/worldlabs/docs/marble_export_specs.md`),
[Export a world, saved](artifacts/tmp/worldlabs/docs/api_reference_worlds_export.md),
[Mesh export, saved](artifacts/tmp/worldlabs/docs/marble_export_mesh.md).

## 5. The coordinate frame and scale

### What the docs say

- Generated worlds use the **OpenCV** convention, with +y down and +z forward. The
  export FAQ also says +x is left, which is not the usual OpenCV +x right. This
  wording is recorded as written and **unconfirmed**.
- `metric_scale_factor` converts raw units to metres. `ground_plane_offset` is in
  metres and is subtracted from Y after scaling, which places the floor at y = 0.
  It applies to Gaussian centres only, not to Gaussian sizes.
- Sizes are multiplied by the scale. If a decoder gives log scales, add
  `ln(metric_scale_factor)` instead of multiplying.
- The web viewer applies a 180 degree rotation about the X axis for generated
  assets. To move to OpenGL or Three.js, scale Y and Z by -1 and keep X.

Sources: [Rendering SPZ files, saved](artifacts/tmp/worldlabs/docs/api_rendering-spz.md),
[FAQ, saved](artifacts/tmp/worldlabs/docs/api_faq.md),
[Export specs FAQ, live](https://docs.worldlabs.ai/marble/export/specs).

### A conflict in the release notes

The release notes say that from 11 December 2025, "World generations now export in
the OpenGL coordinate system (previously OpenCV) for both splats and meshes". The
1 January 2026 entry says there is an option to choose OpenGL or OpenCV. The API
docs still describe OpenCV for generated assets. Which frame the API returns
today is therefore **unconfirmed**. The proof code assumes OpenCV. If the assumption
is wrong, the 180 degree flip would put the room upside down. This should be
checked on a PLY with a known floor before the integration relies on it.

Source: [Release notes, saved](artifacts/tmp/worldlabs/docs/marble_release-notes.md).

### What the code does

The owner's note says "flip X by 180 degrees". The docs and the code both mean a
half turn about the X axis. That keeps X and negates Y and Z. This document uses
that meaning.

- `crates/df-render/src/battle/ply.rs`, `marble_world_transform(metric_scale_factor,
  ground_plane_offset)`. It builds a `SplatTransform` with `scale` set to the
  metric scale, `offset_y` set to the ground offset, and the orientation
  `Quat(1, 0, 0, 0)`, which is the half turn about X. Bounds are checked: scale in
  (0, 100], offset within 1,000 m.
- `crates/df-render/src/battle/splat.rs`, `SplatTransform::to_world`:
  `rotate(scan * scale) + offset`. With the half turn, a scan point (x, y, z)
  maps to world (x, -y, -z) times scale, plus the offset in Y.
- In the scene file, this is `scale`, `offset_y` and `orient_deg: [180, 0, 0]`
  (`ply.rs` doc comment).

Sign check: docs say `aligned_y = y_scan * scale - offset` in OpenCV. After the
half turn, world y is `-y_scan * scale + offset`, which matches `to_world`.

### Proof values

| World | `metric_scale_factor` | `ground_plane_offset` | Source |
| --- | ---: | ---: | --- |
| marble-1.1 proof world (`14303792-4851-4bb8-ab28-ae93eb84d706`) | 1.5014322 | 1.2088797 | `out/m11.operation.json`, `out/m11/fetch_report.json` |
| draft proof world (`a288aa3a-ae42-4bea-b9ba-05ecaa50b90f`) | none (`semantics_metadata: null`) | none | `out/draft.operation.json` |

The draft world has no scale, so its numbers are not metric. Use it for preview
only. The harness and the repo test both use the marble-1.1 values.

## 6. How the walkable grid is derived

The harness is `artifacts/tmp/worldlabs/harness/src/main.rs`. It is a scratch
Rust program outside the repo. It includes the df-render modules by absolute
path (`battle/math.rs`, `splat.rs`, `collider.rs`, `terrain.rs`) and does not
build the wasm-only parts. The harness and its scripts use absolute Windows paths.

Steps:

1. **Load.** A PLY loader reads the 14-property binary PLY. It checks that the
   header is `binary_little_endian` and that every property is a float. It applies
   `exp` to the scale fields and a sigmoid to opacity.
2. **Floor.** The floor height in raw units is either the given
   `ground_plane_offset / metric_scale_factor` or, if none is given, the peak of a
   24-bin histogram of raw Y over opaque splats (between the 1st and 99th
   percentile).
3. **Transform.** The `SplatTransform` above, with the half turn about X, is used
   to bake the cloud into metric world space.
4. **Extent.** The 2nd to 98th percentile of opaque splat centres gives the room's
   X and Z extent, with the floor at y = 0.
5. **Voxel grid (SCN-01 path).** `VoxelBuilder` fills a volume at 0.1 m voxels,
   padded by 0.5 m on each side, 3.6 m tall. Each grid cell at 1.5 m or 0.5 m is
   then tested. A cell is `?` if there is no floor within plus or minus 0.35 m
   (`ground_height`). It is `#` if a capsule of radius 0.35 m from y 0.6 to 1.35 m
   touches the collider. Otherwise it is `.`.
6. **Splat-density estimator.** `terrain.rs` builds a height field at 0.5 m and
   blocked cells at 1.0 m from splat density. This is a comparison, not the grid
   used for play.
7. **Mesh grid.** Triangles of the collider GLB are sampled at 0.5 m cells. A cell
   is blocked (`#`) if its wall or object area in the 0.3 to 1.7 m band is at least
   0.15 m2. It is `?` if there is no floor surface (up-facing and near y = 0).

The walkable grid is therefore a grid of cells marked open, blocked or no floor,
derived from the collider. The splats are used for the look and the floor
estimate.

Results: the harness printed grids and counts to the console. No stdout file for
the harness is on disk, so the grids cannot be checked from text. The only
numbers on record are in the repo test `crates/df-render/tests/world_scan.rs`,
which says the research harness found **82 open, 87 blocked and 21 without floor,
out of 190** half-metre squares over the splat extent. The test's own grid covers
the whole mesh, including the street outside, so its totals are larger. The test
expects the shop's open squares to agree. Those counts are **unconfirmed** from
disk. The test and its expected values have not been re-run for this record.

The proof world's collider includes the cobbled street beyond the door, as the
caption says. The walkable grid therefore needs a region to limit an arena to.

Sources: harness and scripts under `artifacts/tmp/worldlabs`,
`crates/df-render/src/battle/collider_mesh.rs` (GLB reader),
`crates/df-render/tests/world_scan.rs` (expected values),
`crates/df-render/src/battle/ply.rs`.

## 7. The proof runs on disk

All paths are under `artifacts/tmp/worldlabs/`. Images and large binaries are
listed by size only and were not opened, as the brief asked.

### Draft run (`marble-1.0-draft`, tag `draft`)

| File | Bytes | Note |
| --- | ---: | --- |
| `out/draft.operation.json` | 3,942 | Operation with `world_id` `a288aa3a-ae42-4bea-b9ba-05ecaa50b90f`. |
| `out/draft.log.jsonl` | 1,512 | Submit, 5 polls, done. Cost 230. |
| `out/draft/draft_100k.ply` | 5,591,737 | Verified header, 99,846 splats. |
| `out/draft/draft_500k.ply` | not on disk | Listed in `out/draft/fetch_report.json` at 27,869,546 bytes. **Missing from disk.** |
| `out/draft/draft_collider.glb` | 780,356 | 29,390 triangles. |
| `out/draft/draft_thumbnail.webp` | 46,754 | Thumbnail. |
| `out/draft/fetch_report.json` | 1,024 | Export and download record. |
| `out/draft/h500_view_source.png` | 395,513 | Harness render, image. Suffix meaning **unconfirmed**. |
| `out/draft/h500_view_tactical.png` | 291,540 | Harness render, image. |
| `out/draft/h500_view_right.png` | 391,554 | Harness render, image. |

### Proof run (`marble-1.1`, tag `m11`)

| File | Bytes | Note |
| --- | ---: | --- |
| `out/m11.operation.json` | 4,514 | Operation with `world_id` `14303792-4851-4bb8-ab28-ae93eb84d706`, cost 1,580. |
| `out/m11.log.jsonl` | 11,212 | Submit, 63 polls, done. |
| `out/m11.stdout.txt` | 7,903 | Console transcript of the generate script. |
| `out/m11/m11_100k.ply` | 5,505,385 | 98,304 splats, header verified. |
| `out/m11/m11_500k.ply` | 28,000,362 | 500,000 splats, header verified. |
| `out/m11/m11_collider.glb` | 2,490,376 | 95,288 triangles. |
| `out/m11/m11_thumbnail.webp` | 44,286 | Thumbnail. |
| `out/m11/m11_pano_small.jpg` | 143,052 | Smaller JPEG of the pano. The method used to make it is **unconfirmed**. |
| `out/m11/m11_pano.png` | not on disk | Listed in `out/m11/fetch_report.json` at 12,833,453 bytes. **Missing from disk.** |
| `out/m11/fetch_report.json` | 1,266 | Export and download record. |
| `out/m11/proof_contact_sheet.jpg` | 224,572 | Contact sheet, image. Content not reviewed. |
| `out/m11/h500_view_source.png`, `h500_view_tactical.png`, `h500_view_right.png` | 650,240; 389,979; 610,559 | Harness renders, image. |
| `out/m11/h500c_view_source.png`, `h500c_view_tactical.png`, `h500c_view_right.png` | 566,365; 804,655; 595,828 | Harness renders, image. The `c` suffix is **unconfirmed**. |
| `out/m11/h100c_view_source.png`, `h100c_view_tactical.png`, `h100c_view_right.png` | 372,035; 556,483; 428,813 | Harness renders, image. |

The renders are CPU rasterised splat views at 960 by 540, with a 75 degree
horizontal field of view by default. The harness uses a source view from the
photo's own viewpoint, a tactical view 2.3 m high (overridable with `WL_EYE_H`) and
a view turned 35 degrees right. The `WL_CLIP_M` and `WL_MAXSCALE_M` variables
clip outliers. The `h100c` and `h500c` names probably mean 100k and 500k
splats with a clip, but **unconfirmed**. The `.ppm` sources are not on disk.

### Input

| File | Bytes |
| --- | ---: |
| `input/cloth_shop_canonical_room.jpg` | 256,943 |

### Scripts and harness

| File | Bytes | Role |
| --- | ---: | --- |
| `scripts/wl.py` | 1,781 | Shared client. Reads the key from the repo `.env` and never prints it. Also the download and log helpers. |
| `scripts/credits.py` | 187 | Balance check. |
| `scripts/generate.py` | 2,384 | Submit and poll, `python generate.py <tag> <model>`. |
| `scripts/fetch.py` | 2,354 | PLY exports (default 100k and 500k), collider, thumbnail, pano. |
| `harness/src/main.rs` | 21,107 | Loader, transform, renders, walkable grid. |
| `harness/Cargo.toml` | 223 | Standalone crate. |

### API docs saved

Saved under `docs/` and matched against the live pages on the points checked (export
specs, pricing, world generation examples). The saved copies carry no retrieval
date beyond their file timestamps (2026-10-17). They include `openapi.yaml` (68 KB),
the reference pages for generate, operations, export, credits, worlds get and
media-assets upload, and the marble export, model, release-notes and support pages.
Terms of service pages are saved but **have not been read** for this record.

### Repo references to the proof

- `crates/df-render/tests/world_scan.rs` loads `out/m11` and skips if the files are
  absent. It does not call World Labs.
- `crates/df-render/src/battle/ply.rs`, `collider_mesh.rs`, `splat.rs`, `scene_file.rs`
  carry the loader, the transform, the GLB reader and the scene-file format.

## 8. The owner's decisions

Recorded this session and in `planning/issue-triage.md` (WIP-08):

1. "The narrative engine needs to flag it based on the conversation, otherwise it'll
   be too late or too expensive." The narrative engine decides WHEN to generate a
   world, from the likelihood of a fight.
2. "The narrative engine can say whether the shop is too big/small and might move
   the combat outside the shop for example, leave it to the narrative engine." The
   narrative engine decides WHERE the fight happens.
3. Judgements are made by Jev or an LLM, never by keyword rules. No keyword tables,
   not even as fallbacks.
4. A world becomes a `SplatWorld` resource on the game's single resource hub
   (`crates/df-tools/src/gameplay/resources.rs`).
5. Fights that start before the world is ready fall back to the flat photographic
   view.
6. Splat formats in this project are PLY and SOG only, never SPZ.
7. Coordinates are in the OpenCV frame; the collider GLB gives the walkable grid.
8. The API key is in `.env` as `WORLDLABS_API_KEY`. It is not to be read, printed
   or used in this record, and none was.

Also from the same issue list: every generated resource follows one contract
(ResourceSpec, then ResourceKey, then `get_or_request`). The hub owns the spend
ledger, single-flight, the RESOURCE audit line and the dev-CLI hooks. Art is
photographic only.

## 9. Integration plan

This is a proposal built on the decisions above. Items marked **open** are
unresolved.

### Hub resource

- Add a `SplatWorld` variant to `ResourceKind` in
  `crates/df-tools/src/gameplay/resources.rs`. The enum today has `Depiction`,
  `ScenePlate`, `PlateEdit`, `ItemCloseUp`, and the reserved `MusicSet`,
  `VoiceLine`, `VideoClip`. There is no splat kind yet (verified). Give it a label
  such as `splat_world` and its own `CONTRACT_VERSION`.
- The spec carries the place or scene ID, the model (default `marble-1.1`), the
  source image reference and the narrative judgement that asked for it. Its key is
  the hash of those, so a repeat request for the same place is idempotent.
- The worker does submit, poll, PLY export at 100k or 500k, collider GLB and pano
  downloads. The ready entry stores the PLY, the collider, and the
  `metric_scale_factor` and `ground_plane_offset`. An SPZ URL is never stored or
  read.
- A validator checks that a ready entry is still fresh. A permanent failure (for
  example, a `400`) and a transient one (`429`, `500`, timeout) are kept distinct.
  The 429 path honours `Retry-After`.
- Every credit spend goes through the hub's ledger. Set a hard cap per session
  (**open**: value to be chosen by the owner).

### Narrative engine: when

- A Jev or LLM judgement, reading the conversation, gives a likelihood that a fight
  will start and when. It does not use keywords. The judgement carries the place,
  the likely participants and a confidence.
- Lead time matters. The observed times are 54 s for draft and 6.0 min for
  marble-1.1, plus export and download. Two runs are too few for a rule, so the
  threshold and lead time are **open**. The narrative engine should flag early
  enough that the world is usually ready.
- Only flagged places generate a world. Each marble-1.1 world costs 1,580 credits
  ($1.264).

### Narrative engine: where

- Given the walkable grid, the narrative engine chooses the arena. It may choose the
  shop interior, or a street or yard outside the shop. The choice is a region in
  world coordinates, named by the judgement.
- The grid checks the choice geometrically: the number of open cells and whether the
  bound NPC's cell is open. The grid is a validator, not a keyword rule.
- The arena must exclude the street beyond the door unless the judgement chose it.
- NPC binding stays as in [scene combat binding](scene-combat-binding.md). A splat
  arena does not change which NPC may fight.

### Fallback

- A fight that starts before the `SplatWorld` is ready uses the flat photographic
  view. **Proposal:** the fight stays flat for its whole length. Switching mid-fight
  is not planned.
- The battle layer requests the resource at the moment a fight is judged likely,
  not at the start of the fight, so most fights should have the world ready.

### Dev CLI

- Following the owner's rule that every system has logs and CRUD in the dev CLI,
  add read commands for splat worlds: list, status, spend, and the judgement that
  requested each one. Writes go through the hub.

### Renderer

- Use `marble_world_transform` and the PLY loader in `df-render`. Wire the splat
  scene through `battle/scene_file.rs`, `stage.rs`, `layout.rs`, `hud.rs` and
  `gpu/renderer.rs`, as WIP-08 lists.

## 10. Risks and gaps

| Area | Risk or gap | Status |
| --- | --- | --- |
| Cost | 1,580 credits per marble-1.1 world. Overage is billed after the month ends and is not capped by auto-refill. | Known from docs. Needs a hub cap. |
| Rate limits | Default about 3 starts per minute and 60 per hour. A burst of flags could be throttled. | Known from docs. |
| Time | One marble-1.1 run took 6.0 min. Docs say about 5. Slow runs will miss fights. | Fallback covers it. |
| Frame | Release notes say exports moved to OpenGL in December 2025. API docs say OpenCV. | **Unconfirmed.** Check on a PLY with a known floor. |
| Scale | The draft has no metric scale. | Draft is preview only. |
| Formats | Marble has no SOG output. The project lists SOG as a splat format. Any SOG path needs a converter that has not been researched. | **Gap.** |
| SPZ | The API returns SPZ URLs in every world, and the project forbids SPZ. The pipeline must never store or read them. | Rule to enforce in the adapter. |
| Export plan | Docs say splat export needs the Standard plan in the Marble app. The API export worked on this account. The plan that applies to the API account is **unconfirmed**. | Check on the platform billing page. |
| Full resolution | `full_res` PLY was not run. Its size and count are **unconfirmed**. Mobile and browser memory limits are untested. | Test before choosing a default. |
| Collider | Proof collider has about 95k triangles, under the documented 100k to 200k. | Observed, cause unknown. |
| Grid | Harness grids were printed to a console with no file kept. Counts exist only in a repo test comment. | **Unconfirmed.** Re-derive from the saved GLB before relying on numbers. |
| Floor | Without a given offset, the harness falls back to a histogram peak. | Prefer `ground_plane_offset`; the histogram is a fallback. |
| Files | `draft_500k.ply` and `m11_pano.png` are in the fetch reports but not on disk. | **Missing.** Explain or re-fetch (costs nothing to re-export). |
| Terms | World Labs terms are saved but not read for this record. Shipping generated worlds in a game may need specific rights. | **Not reviewed.** |
| Reproducibility | Scripts and harness hard-code absolute Windows paths and sit outside the repo. | Acceptable for research; move before integration. |
| Sample size | Two generation runs. No distribution of times or costs. | Stated as observed, not as a rule. |
| Planning | `asset-provider-research.md` has no World Labs row. `narrative-engine.md` has no fight-likelihood or arena section. This document is not yet linked from `planning/README.md`. | Add when the owner approves. |
| Flat fallback | No planning doc describes the flat photographic battle view. | Needs its own reference before implementation. |

Open decisions for the owner: the threshold and lead time for the "when" judgement,
the per-session spend cap, which splat resolution (100k or 500k) the renderer uses by
default, and whether a fight stays flat once it has started.
