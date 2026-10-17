# SAM 3.1 and hosted segmentation for DungeonFlux

Date: 2026-10-17
Status: Research and measured proof; no repository source changed; integration not started
Supersedes in part: [Segment Anything feasibility](segment-anything-feasibility.md) (2026-10-15)

Owner instruction for this pass: use SAM 3.1 over a hosted API, not locally. The hosted measurements
below are real calls on two game images (12 calls, USD 0.125 in total, cap USD 0.50). One local
attempt before that instruction was abandoned and its install and weights were deleted (see the last
line of "How to run it").

## What SAM 3.1 is

- Meta's Segment Anything Model 3 family. SAM 3 (November 2025, 848 million parameters) added
  promptable concept segmentation: a short noun phrase such as "person" returns every matching
  instance in an image, each with a mask, a box and a score. It also takes point, box and mask
  prompts and tracks objects in video. [SAM 3 repository](https://github.com/facebookresearch/sam3)
- SAM 3.1 was released on 2026-03-27. The single change is "Object Multiplex": tracked objects are
  grouped into fixed-capacity buckets and processed jointly, about 7 times faster at 128 objects on
  one H100, plus less CPU/GPU synchronisation. Meta reports mixed video results (SA-V cgF1 30.3 to
  30.5, YT-Temporal-1B 50.8 to 52.9, LVVIS 36.3 to 34.3) and better video object segmentation on 6
  of 7 benchmarks. [Release notes](https://github.com/facebookresearch/sam3/blob/main/RELEASE_SAM3p1.md)
- The change is in the video tracker. The 3.1 checkpoint (`sam3.1_multiplex.pt`, 3.5 GB) still holds
  the same kind of image detector (about 853 million parameters under `detector.`, 19.6 million
  under `tracker.`). Measured here: on a still plate, SAM 3 and SAM 3.1 returned the same five people
  with the same boxes and scores within 0.01 (table below). For stills there is no quality reason to
  pay for 3.1.
- Variants: one size per generation. No small or large tiers are published. Hosted services expose
  separate image, image-RLE, video and video-RLE endpoints.

## Licence and weights access

- Licence: the Meta "SAM License" (last updated 2025-11-19). It grants a non-exclusive, worldwide,
  royalty-free right to use, copy, modify and distribute the materials and to make derivative works.
  It does not address commercial use in words and it does not forbid it. Conditions: pass the licence
  text on with redistributed copies; acknowledge the materials in published research; comply with
  trade-control and privacy law; no use for ITAR, military or warfare, nuclear, espionage or weapons
  end uses; no reverse engineering. Meta may terminate for breach, and the licence ends if you sue
  Meta over the materials. It is not an OSI licence. [LICENSE](https://github.com/facebookresearch/sam3/blob/main/LICENSE)
- A tabletop game is not a listed prohibited end use, but a lawyer, not this note, should confirm
  the commercial position before launch.
- Weights: `facebook/sam3.1` on Hugging Face is gated (manual approval, contact details). Unofficial
  mirrors of the 3.1 checkpoint and SAM 3 ONNX exports exist and carry the same licence. The hosted
  route needs no weights and no gate.
- Hosted use is governed by each host's terms (fal, Meta Model API), not by the SAM License. fal
  labels its SAM pages "Commercial use"; that label is not a terms review.

## How to run it

Hosted options, prices checked 2026-10-17:

| Host | Endpoint | Price | Notes |
| --- | --- | --- | --- |
| fal (measured) | `fal-ai/sam-3/image` | USD 0.005 per request | SAM 3, PNG cut-outs per mask |
| fal (measured) | `fal-ai/sam-3/image-rle` | USD 0.005 per request | SAM 3, run-length masks |
| fal (measured) | `fal-ai/sam-3-1/image` | USD 0.01 per request | SAM 3.1, PNG cut-outs |
| fal (measured) | `fal-ai/sam-3-1/image-rle` | USD 0.01 per request | SAM 3.1, run-length masks |
| fal (not run) | `fal-ai/sam-3/video`, `video-rle` | USD 0.005 per 16 frames | SAM 3 tracking |
| fal (not run) | `fal-ai/sam-3-1/video`, `video-rle` | USD 0.01 per 16 frames (page) | pricing API reports unit "units" |
| Meta Model API | model `sam-3.1`, `POST https://api.meta.ai/v1/responses` | USD 2.50 per 1,000 images (0.0025 each); video USD 0.20 per 1,000 frames | Official; public preview; 3,000 requests per minute per team; one concept per request; special-token output; content not used for training in Standard mode. A 2026-10-15 probe got HTTP 402 `billing_not_configured`, so not run. |
| Roboflow Serverless | `POST https://serverless.roboflow.com/sam3/concept_segment` | not published (credits) | SAM 3; up to 16 text prompts in one request; polygon or RLE output; not run |
| Replicate | community `lucataco/sam3-video` | about USD 0.0031 per run (page estimate), L40S, about 4 s | Third-party SAM 3 video model; no official image model found; not run |

Sources: fal pricing API (`GET https://api.fal.ai/v1/models/pricing?endpoint_id=<id>`, run today for
all eight image and video endpoints), [fal sam-3-1 image](https://fal.ai/models/fal-ai/sam-3-1/image/api),
[fal sam-3 image](https://fal.ai/models/fal-ai/sam-3/image),
[fal sam-3-1 video](https://fal.ai/models/fal-ai/sam-3-1/video),
[Meta pricing](https://dev.meta.ai/docs/pricing-rate-limits),
[Roboflow SAM 3](https://docs.roboflow.com/deploy/supported-models/sam3),
[Replicate lucataco/sam3-video](https://replicate.com/lucataco/sam3-video).

fal API flow (the one measured):

1. `POST https://queue.fal.run/<endpoint>` with `Authorization: Key <key>` and a JSON body:
   `image_url` (a public URL or a `data:image/jpeg;base64,...` URI; 0.32 MB and 0.55 MB bodies worked),
   `prompt` (one short noun phrase), `include_scores`, `include_boxes`, `return_multiple_masks`,
   `max_masks` (1 to 32, default 3), and for the PNG endpoint `output_format` (png, jpeg, webp). Reply:
   `request_id`, `status_url`, `response_url`.
2. Poll `status_url` (about 0.5 s) until `COMPLETED`; then `GET response_url`.
3. Result, PNG endpoint: `masks[]` (url, width, height, content_type), `scores[]`, `boxes[]`
   (normalised `[cx, cy, w, h]`), `metadata[]` (index, score, box), `image` (preview). Result, RLE
   endpoint: `rle[]` (one string per mask), `scores[]`, `boxes[]`, `metadata[]`.
4. PNG masks need one download each (0.4 to 0.9 s each, 22 to 680 KB). RLE needs none.

Measured output formats:

- PNG endpoint: RGBA PNG at the full source resolution (1,672x941 and 2,560x1,440 were sent and
  returned). The RGB channels are the original pixels and the alpha channel is hard 0 or 255, so each
  mask file is already a transparent cut-out of that instance. No soft edge.
- RLE endpoint: a space-separated list of `start length` integer pairs over the row-major flattened
  mask, with 1-based starts (pixel index = start - 1 = y * width + x; read 0-based, runs spill one pixel into the next row, found in the plate integration). Decoded and checked: the pixel totals equal the PNG alpha counts
  exactly (353,181, 218,848, 48,379, 106,413, 130,604). A person mask is 3.5 to 17.5 KB of text.
- Boxes are normalised centre/size, not corner pairs. Masks come back in score order and `metadata.index`
  is the original instance index.

Local attempt, abandoned on the owner's instruction: the SAM 3.1 detector weights loaded in the
official code on this ARM64 CPU only after removing CUDA and Triton assumptions and took 205 s per
image in fp32 (948 s in bf16 autocast) against 11 s for an int8 SAM 3 ONNX export, with the same
detections; the NPU path did not finish compiling; all installs and weights were deleted.

## Measured results (12 calls, USD 0.125)

Images: `assets/ui/scenes/mara-harbor-v4.png` (empty harbor hut, sent as a 1,672x941 JPEG, 323 KB) and
a generated shop plate (`plate_marek.jpg`, 2,560x1,440, 549 KB). Latency is submit to result JSON,
end to end, from this machine; the submit step (upload of the data URI) was 0.5 to 0.9 s of it.

| Image | Endpoint | Prompt | Found (scores) | Total s | USD |
| --- | --- | --- | --- | --- | --- |
| harbor | sam-3-1/image | lantern | 8 (0.89, 0.86, 0.82, 0.82, 0.81, 0.79, 0.70, 0.64) | 1.72 | 0.01 |
| harbor | sam-3-1/image | barrel | 5 (0.93, 0.92, 0.87, 0.71, 0.57) | 1.92 | 0.01 |
| harbor | sam-3-1/image | boat | 0 | 1.67 | 0.01 |
| harbor | sam-3-1/image | wooden boat | 1 (0.57) | 2.18 | 0.01 |
| harbor | sam-3-1/image | person | 0 (correct, empty room) | 1.71 | 0.01 |
| shop | sam-3-1/image | person | 5 (0.96, 0.95, 0.95, 0.94, 0.87) | 3.25 | 0.01 |
| shop | sam-3/image | person | 5 (0.97, 0.96, 0.95, 0.94, 0.87) | 2.56 | 0.005 |
| shop | sam-3-1/image-rle | person | 5 (same scores as above) | 1.69 | 0.01 |
| shop | sam-3-1/image | head | 5 (0.94, 0.94, 0.91, 0.91, 0.78) | 2.35 | 0.01 |
| shop | sam-3-1/image | wooden counter | 1 (0.95) | 2.59 | 0.01 |
| shop | sam-3-1/image | wooden floor | 1 (0.98) | 1.74 | 0.01 |
| shop | sam-3-1/image | blue cloth | 6 (0.86, 0.85, 0.68, 0.64, 0.54, 0.50) | 3.40 | 0.01 |

What the images showed:

- People: all five in the plate (merchant, two cast members, two foreground heroes seen from behind),
  boxes within 1 to 3 pixels of the local run. Cut-outs are clean at full resolution. Where an
  occluder hides part of a figure, the cut-out ends there (the merchant stops at the counter, a cast
  member's legs at the rack and pack, a held cloak is excluded from its holder), so a cut-out is a
  partial figure, not a full body. Edges are hard with slight stair-stepping and ragged fragments at
  occlusion borders.
- Heads, "head" prompt, compared with head boxes I drew by hand on the plate (an estimate, not ground
  truth): best-match IoU 0.84, 0.81, 0.73, 0.87, 0.83 for the five people; centre error at most 2 percent
  of frame height, mostly vertical because SAM's box runs down to the chin, neck or hood. The existing
  `measured_layout` grows the measured box by 1.3 and clamps it, so that slack is within what it
  already absorbs. I could not measure the current gpt-5.5 locator on the same plates (a paid call, and
  the owner's rule is no paid tests in verification); its 25 s deadline failure from the server log
  is the only evidence for it.
- Things: lanterns (8 of 8 visible, including two small distant ones on the boat), barrels, the counter
  and the floor were found reliably. The floor mask excludes people, the counter and the pack, and
  stops at the door frame: usable as the walkable region of a plate.
- Prompt sensitivity is real. "boat" returned nothing and "wooden boat" returned one match at 0.57;
  in the local run "blue linen bolt" and "fitting mirror" returned nothing while "blue cloth" and
  "mirror" matched. "blue cloth" also matched shelf stacks and cloaks. Relations ("on the counter")
  are not understood; the server must filter by box position using the scene facts.
- Named identity is not returned. "head" and "person" give instances, not who. Described prompts
  ("woman in a grey tunic holding a cloak") scored one instance at 0.94 in the local run, while a
  merchant description also scored a hero at 0.42, so assignment must be exclusive across the
  instances, with the heroes taken from their known slots.

Evidence under `artifacts/tmp/sam31/` (gitignored):

- `fal/*.result.json` (raw results), `fal/run_*.json` (timings), `fal/masks/` (downloaded PNG masks),
  `fal/sheet_person_cutouts_hosted.jpg`, `fal/overlay_harbor_lantern.jpg`,
  `fal/marek_head_zoom_hosted.jpg`;
- `scripts/fal_sam.py` (the call script; reads the key from an environment variable and never stores it);
- `out/` and `plates_contact.jpg` (the earlier local run, nine plates of the same shop);
- `plate_marek.jpg` (the shop plate, copied from the imagery cache, key `d7de8b05...`).

## Uses for DungeonFlux

Verdicts, with the current code each would touch.

1. Head and face location on plates (`living_scenes/plates.rs`, `df-providers/src/head_locate.rs`).
   Good fit as a second tier. Today one gpt-5.5 vision call finds up to four heads per batch under a
   25 s deadline, and when it fails the director's predicted layout stands (a 25,007 ms failed batch is
   the case in the log). SAM's "head" prompt located all five heads in 2.35 s for USD 0.01, with the
   boxes above. It cannot say whose head is whose, so the identity step stays: heroes by slot, cast by
   exclusive assignment of described-person prompts, or the existing vision call kept as the first
   tier. Expected gain: fewer plates left on the predicted layout, and a result in about 2 s instead of
   a 4 to 25 s call. Not yet shown: that it beats the vision call when both work.
2. Figure cut-outs. Works for isolating a figure from a plate that already exists, which keeps the
   figure identical to the plate (the current plates drift: the same cast member looks different in
   each of nine plates). Limits: partial bodies at occlusions, hard alpha, no matting of hair. It does
   not replace the transparent canonical-character cut-outs for walking, because a figure removed from
   a plate leaves a hole that needs inpainting. Use cases that fit: focus a speaker by dimming
   the rest, parallax between the figure layer and the plate, and a highlight outline. Edges need a
   1 to 2 pixel erode and feather in Rust before compositing.
3. Interactable hotspots. Good fit. Prompts come from the structured scene facts (`things[].name`);
   masks give TV and phone highlight shapes and tap regions. Plan for misses: ask Jev or an LLM for up
   to three plain noun phrases per fact and try them in order, and filter by box position against the
   fact's stated position. Hits beyond the score floor still need a vision check for important things.
4. Walkable grid for fights (SCN-01). Marginal. SCN-01 is fixed from 3D geometry (`d29a3a4f`, voxel
   collider ported from PlayCanvas), which is more exact than any 2D mask. SAM on a source photo or a
   rendered view of a splat gives the floor region (0.98 on the shop plate) and could label what a
   blocked cell is (water, furniture, door) for the narrator, but turning a 2D mask into grid cells
   needs camera and depth data the arena work owns. Coordinate with the Marble/splat research: this
   is a classification layer over their geometry, not a grid source. Not tested on a splat view.
5. Video. Not tested. SAM 3.1's gain is here. fal prices 3.1 video at USD 0.01 per 16 frames: a 5 s
   loop at 24 fps is 120 frames, 8 blocks, USD 0.08 (SAM 3: USD 0.04). Possible use: follow heads
   through the MOT-02 ambient loop so bubbles and outlines stay on the person. Wait until MOT-02 clips
   exist; bubbles anchored to a baked still need no tracking.

## Hub resource kinds

Owner rules: every derived resource goes through the one generation hub (`gameplay/resources.rs`:
typed spec, stable key, cache, single flight, audit, dev CLI), on demand only, no keyword tables,
photographic only.

- `PlateSegments` (provider call). Spec: plate image key (the imagery cache key of the plate bytes),
  endpoint id (`fal-ai/sam-3/image-rle` today), model version string, prompt (one noun phrase),
  score floor, max masks, output kind `rle`, contract version. Canonical JSON of those fields gives the
  key, so a new model, prompt or contract is a new key. Handle: a segments note kept beside the cached
  plate, the way `<key>.layout.json` is, holding image width and height, per-mask score, normalised
  box and RLE string (3 to 18 KB per person). Limits: timeout 30 s, 2 attempts, transient on 5xx or
  timeout, permanent on a 4xx or a refusal. A zero-mask answer is a successful, cached result.
- Spend: add `Unit::SegmentCall`. The call reserves from the ledger before it is sent, commits when
  sent, never refunds a sent call, and reconciles to the fal price (5,000 or 10,000 micro-dollars).
  `Capped` leaves the layout as it is today.
- `PlateHeads` (derived, no provider call). Spec: a `PlateSegments` key for "head" plus one for
  "person". Output: `HeadBox` per instance in fractions of the plate, the same type
  `head_locate.rs` returns, so `plates.rs` consumes either.
- `FigureCutout` (derived, local CPU). Spec: `PlateSegments` key plus instance index. Output: WebP
  with alpha built from the plate bytes and the RLE; no second provider call.
- `PlateHotspots` (derived). Spec: plate key, scene-facts revision. The prompts are chosen from the
  structured facts by Jev or an LLM, never from a word list. Output: one mask handle per interactable
  thing.
- `ArenaFloorMask` (later, splat views): same `PlateSegments` spec with a rendered-view key.
- Dev CLI: `resources list --kind plate_segments`, `resources show <key>` (prints scores, boxes and
  writes an overlay PNG under `artifacts/`), `resources regenerate <key>`, `resources invalidate <subject>`,
  plus a one-off `segment <plate-key> <prompt>` that calls `hub.render`. The adapter logs safe ids,
  latency, mask count, cache hit and spend through the shared audit path, never the image or the
  prompt text beyond the noun phrase.

## Recommendation

Adopt hosted segmentation as an optional, cached, on-demand hub resource, and start with
`fal-ai/sam-3/image-rle` at USD 0.005 per call. Reasons: it is the cheapest working route today; SAM 3
and 3.1 gave the same still-image results; the RLE reply is a few kilobytes with no mask downloads
(1.7 s end to end); and the key already works. Keep the model id in the spec so `sam-3-1` is a
one-field change when video tracking is wanted. Move to Meta's official `sam-3.1` endpoint
(USD 0.0025 per image) once its billing is configured and its special-token output has a Rust parser.

Per plate, "person" plus "head" is two calls (USD 0.01 on SAM 3, USD 0.02 on 3.1) against a plate that
costs about USD 0.06, and each extra concept is one more call, so hotspots are the cost driver: bound
them to the few things a scene marks interactable and cache by plate key.

First implementation step: a `PlateSegments` spec, a `fal_segment.rs` adapter in `df-providers` on the
same `NativeHttp` path as `fal_media.rs`, `Unit::SegmentCall`, the dev-CLI read, and one consumer: the
head fallback in `plates.rs` when `locate_heads` fails or times out, before the predicted layout. That
is a measurable change (plates on the predicted layout versus measured) with no new UI.

## Gaps

- Not run: Meta Model API (billing not configured on 2026-10-15), Roboflow, Replicate, and any video
  endpoint. Their prices are from the pages cited above.
- The head comparison uses boxes I drew by eye on one plate. The gpt-5.5 locator was not measured. A fair
  comparison needs annotated heads on the nine stored plates and the same plates through both paths.
- Identity assignment (which person is which cast member) was only checked on local runs on one plate.
- Two images only, both photographic scenes without gore or nudity; fal's safety layer on mature plates
  is untested.
- fal's limits on `data:` URI size, result URL lifetime and rate are not documented on the pages read;
  0.55 MB bodies worked. The earlier TLS failure in `segment-anything-feasibility.md` did not recur with
  Python 3.11 `urllib` against `queue.fal.run`, so it was probably specific to that client.
- No splat or arena view was segmented; the Marble work owns the camera and depth data a grid needs.
- Licence: the SAM License text was read, not reviewed by counsel; fal's and Meta's service terms and
  image retention were not read.
