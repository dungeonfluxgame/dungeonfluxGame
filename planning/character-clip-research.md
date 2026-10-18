# Character video clips at the battle home angle

Research: 2026-10-18, lane A1. **Provider experiment blocked; animation acceptance pending.**
No game source, content, dependency, server, or public contract was changed.

## Result and scope

Brakka and Sigrun's current generated full-body photographs were found and preserved.
The bounded fal authoring/generation runner and a working offline chromakey, alpha
WebM, contact-sheet and atlas pipeline are under `artifacts/tmp/char-clips/`.
Two five-second **synthetic translation controls**, made from the existing cutouts,
were processed and measured. They are technical controls, not guard, attack or fall
animations, and do not depict the requested home view.

The pricing request and all three guard-image submissions failed before HTTP with
`WinError 10061`. A separate curl HEAD failed with code 7 through the configured
localhost proxy. No provider response, queue identity, image or video was received;
zero paid jobs were accepted. The runtime network restriction was not bypassed.
Evidence: `artifacts/tmp/char-clips/network-blocker.json` and each character's
`guard.job.json`. The exact live error was connection refusal, not bad credentials.

Consequently there is no filmed angle, no generated guard or fallen frame, and no
measured Kling identity drift, camera drift, idle seam, attack impact or fall duration.
The full nine-clip comparison remains required. This report does not approve it.

## Governing decisions and evidence

- Owner/coordinator task dated 2026-10-18: return-home view, photographic characters,
  green background, idle/attack/last hit, Brakka/Sigrun/Skeleton; live fal explicitly
  authorized for these clips with at most one retry each. That narrow authorization
  takes precedence over the generic lane prohibition on provider calls.
- [Asset engine](asset-engine.md), **Control plane and latency classes**, **Models,
  public operations and lifecycle**, **Nonblocking presentation and acceptance** and
  **Runtime scene imagery provider**: cosmetic prepared media, identity/reference
  dependencies, canonical keys, retained provenance and a nonblocking still fallback.
- [Tempo engine](tempo-engine.md), **State and server output** and **Presentation
  profile and cues**: server anchored presentation epochs and time; no replay of
  expired one-shots, no animation changing game rules or withholding accepted outcomes.
- [Video provider research](video-loop-provider-research.md), **Measured results**:
  yesterday's landscape Kling O1 proof cost $0.56 for five seconds and took 109.3 s.
  Its 1920x1080, 24 fps, 122-frame result is historical landscape evidence, not a
  character or portrait benchmark.
- `crates/df-providers/src/fal_media.rs`, `fal_image.rs`: existing Kling first/last
  body, FLUX Kontext Max multi-reference edit and BiRefNet background-removal path.
- `crates/df-render/src/battle/figure_art.rs`: shared 2:3 canvas and visible-alpha
  bounds; `art_contract.rs`: photographic depictions only.
- [Coding style](coding-style.md), [Observability](observability.md), ADR 0003/0004/0005:
  bounded work, accurate failure evidence, independent running-output acceptance.
  Research JSON is retained experiment evidence, not a competing gameplay logging path.
- Play evidence: `artifacts/tmp/play-r6/battle-findings.txt` B-05/B-07/B-08/B-13/B-18;
  `findings.txt` P6-68 and character continuity findings; `startup36.txt` identifies
  snapshot build `abc42a23`, the canonical hero resource handles and the cache reuse.
  B-08 identifies stock hero substitution; B-18 records missing Skeleton figures.

## Authoring view, not yet filmed

Proposed home-angle revision: **`battle-home-a1-v1-pending-c5`**.

| Parameter | Authoring request |
|---|---|
| Heading | Scene front; relative yaw 0 degrees |
| Downward pitch | 15 degrees |
| Vertical FOV | 38 degrees; provisional interpretation of coordinator's FOV |
| Distance | Scene's combat-establishing reference distance; merchant shop 10 m, riverbed 8 m |
| Figure | Full body at three-quarter view, facing screen-right, both feet visible |
| Background | Flat chroma green RGB 0,255,0, including ground; no horizon, shadows or scenery |
| Master canvas | 768x1152 PNG inputs; retain native provider outputs separately |
| Initial framing | Figure at most 65% canvas height, feet around 88%; room for the complete weapon and fall |
| Motion | Locked camera; no pan, zoom, orbit, reframe or refocus |

Current authored cameras differ: merchant `combat_est` is pitch 16/FOV 40/distance 10,
riverbed is 17/40/8; both `turn_focus` pitches are 13 and FOVs 38. The task's 15/38
choice is deliberate, provisional and not an assertion that those JSON cameras changed.
The prompt includes the angles, but a generated image is not calibrated camera metadata.
The runner fixes portrait framing rather than claiming a physical distance from a flat
green image. C5's final home view must be captured and compared before production filming.
Change the home-angle version and regenerate if that view changes.

Green has no stationary spatial features: camera translation/rotation cannot be
independently solved from it. Body/foot/weapon scale and centroid changes provide
stability proxies, not a measured zero-degree camera drift. Use a matching camera
reference render plus visual review; reject apparent zoom/orbit or changed perspective.

## Source identity provenance

Both actual cached PNGs decode to **832x1248**, despite recovered manifest records
reporting 1024x1536. Record the actual bytes and their hashes; a historic manifest
dimension or a logical imagery key alone does not prove the file revision.

| Character | Logical imagery key | Actual PNG SHA-256 | Bytes |
|---|---|---|---:|
| Brakka | `4ca48c4111bfa06e766f3a3ef08817f58d2db34efb222aac6646add83c4405af` | `787bdfb94a9917177be747b74b76747f496f78cf14cf9725b2e5b498d6f9e1ea` | 1,196,075 |
| Sigrun | `f8b8abb5c85c0633719cf905c276df2a697f2e1a3a0422041b8e3c2346304177` | `07352e08298e01705625596ab875efb6d30a001b9dec0c29c2a7cf383475b82f` | 1,308,705 |

Original cache: `artifacts/tmp/play-src/artifacts/tmp/play/imagery-cache/`.
Retained copies: `artifacts/tmp/char-clips/brakka/source.png` and
`artifacts/tmp/char-clips/sigrun/source.png`. `source-records.json` retains the recovered
public prompts and manifest locations; `references.json` pins the copied bytes.

Brakka is the adult male dwarf with copper-red hair/beard, chain mail, dark blue
cloak, leather belt/bracers and boots. Sigrun is the adult female dwarf with auburn
braids, chain mail, dark blue cloak and leather belt/bracers/boots. Both reference
cutouts hold swords. The requested Sigrun edit changes her held prop to a javelin;
it is an authoring experiment, not an inventory mutation.

No Skeleton depiction was found in the snapshot run manifests or reviewed assets.
`content/adventure-tables/art-manifest.json` maps `ant-skeleton` to the live canonical
photo token rather than actual art bytes. The runner proposes a new practical-film
skeleton reference, then uses that reference consistently; it cannot validate identity
against a nonexistent current cutout. Its creation and owner review remain pending.

## Requested nine-clip experiment

[Kling O1's current schema](https://fal.ai/models/fal-ai/kling-video/o1/image-to-video/api)
accepts start and end image URLs, including data-URI file input, with duration `"5"`.
The existing adapter's field names are correct. The script sends guard->guard for
idle and attack, guard->fallen for last hit. No alternative animation model was used.

| Character | Clip | Action requested between bookends | Result |
|---|---|---|---|
| Brakka | idle | Breathing and small weight shift, greatsword ready | Blocked before guard |
| Brakka | attack | Two-handed greatsword diagonal cut, recover | Blocked before guard |
| Brakka | last hit | Recoil, stagger, fall, settle on side | Blocked before guard |
| Sigrun | idle | Breathing and small weight shift, javelin ready | Blocked before guard |
| Sigrun | attack | Overarm javelin throw, draw a second javelin, recover | Blocked before guard |
| Sigrun | last hit | Recoil, stagger, fall, settle on side | Blocked before guard |
| Skeleton | idle | Subtle balance/weight shift, shortsword ready | Blocked before guard |
| Skeleton | attack | Shortsword thrust, withdraw, recover | Blocked before guard |
| Skeleton | last hit | Recoil, stagger, collapse, settle | Blocked before guard |

For **each of these nine**, actual clip cost, provider wall time, file size, FPS,
frame count, identity drift, camera drift, loop seam, attack impact frame, fall
duration and canvas containment are **unmeasured**. No retry was made. Historical
Kling output does not fill those cells. A javelin leaving the frame can conflict
with a strict all-props-in-canvas requirement; inspect that case and define whether
the projectile is separately rendered before integrating. A recovered second
javelin may also morph: reject the result if it does.

## Alpha experiment and measured controls

The offline test starts with known source alpha, adds a two-pixel sinusoidal
horizontal translation, composites onto green, encodes an H.264 4:2:0 input,
then applies this [FFmpeg chromakey/despill pipeline](https://www.ffmpeg.org/ffmpeg-filters.html#chromakey):

```text
chromakey=0x00ff00:0.10:0.08,format=rgba,despill=type=green:mix=0.5:expand=0:alpha=0
```

It exports VP9 `yuva420p`, CRF 28, `auto-alt-ref=0`, no audio, and decodes using
`libvpx-vp9` explicitly. Merely seeing `yuv420p` from a default ffprobe decoder
does not establish alpha loss. The test inspects decoded transparent and opaque
pixels. A direct-source-alpha VP9 control separates codec error from keying error.

All controls are 256x384, 24 fps, 120 frames, exactly 5.000 s, inside the 2:3
canvas with no detected edge-touch frame. Local paid cost is $0.

| Control measurement | Brakka | Sigrun |
|---|---:|---:|
| H.264 green input bytes | 28,074 | 30,660 |
| Keyed VP9 alpha WebM bytes | 29,653 | 34,951 |
| Direct-source-alpha VP9 bytes | 18,392 | 21,427 |
| Eight keyed atlas PNG pages, compressed bytes | 1,701,177 | 1,961,314 |
| Eight pages decoded RGBA bytes, no mipmaps | 51,650,560 | 51,650,560 |
| Local processing wall time, second run | 10.094 s | 8.172 s |
| Full-image alpha MAE, 0..255 | 1.634 | 1.839 |
| Partially transparent source-edge alpha MAE | 45.916 | 47.401 |
| Direct-alpha VP9 edge MAE | 1.251 | 1.183 |
| Opaque source pixels remaining alpha >240 | 99.9998% | 99.9999% |
| Empty source pixels becoming alpha >16 | 0.4754% | 0.4911% |
| Premultiplied RGB last/first MAD, 0..255 | 0.1027 | 0.1417 |
| Seam / largest adjacent-frame MAD | 0.0600 | 0.0630 |

These are low-motion controls; their byte sizes and seams cannot predict fast
strikes, hair motion, generated edges, fall clips or Kling's frame count. Contact
sheets were visually inspected: faces/costumes and swords remain readable at this
size, while edges look hard and the source itself has a brownish hair fringe.
The original preexisting fringe must not be credited to the keyer. The large edge
alpha error relative to direct-alpha encoding isolates substantial keying/input
compression damage in this control; it does not prove which neural matte wins.

Pictures and videos, relative to repository root:

- `artifacts/tmp/char-clips/controls/brakka/alpha-comparison.png`
- `artifacts/tmp/char-clips/controls/sigrun/alpha-comparison.png`
- `artifacts/tmp/char-clips/controls/{brakka,sigrun}/synthetic-translation-control-key-contact.jpg`
- `artifacts/tmp/char-clips/controls/{brakka,sigrun}/synthetic-translation-control.mp4`
- `artifacts/tmp/char-clips/controls/{brakka,sigrun}/synthetic-translation-control-key.webm`
- `artifacts/tmp/char-clips/controls/{brakka,sigrun}/direct-source-alpha-control.webm`
- Per-control `summary.json`, keyed metrics JSON and `*-atlas/layout.json`/PNG pages.

**Matting comparison still pending.** [BiRefNet v2](https://fal.ai/models/fal-ai/birefnet/v2/api)
supports heavy and matting variants. A better bounded comparison for these nine
clips is the existing [Bria video background-removal endpoint](https://fal.ai/models/bria/video/background-removal/api),
which supports transparent VP9 output. The runner is prepared to process every
Kling output with Bria, preserving the original video and keyed counterpart.
Neither endpoint could be called here. Compare hair strands, bone gaps, sword
tips/javelin shafts, motion-blurred edges and temporal matte flicker on bright and
dark backgrounds. Also inspect original unkeyed green to distinguish missing prop
generation from matting damage. No matting winner is claimed.

Provisional production choice: prefer a temporally stable neural matte **if it
preserves all weapons and gaps in the measured comparison**; retain chroma keying
as the deterministic baseline. The control's hard-edge damage makes chroma alone
an insufficient basis for approving photographic hair and motion blur.

## Measurement method for the real clips

`analyze.py clips` probes native output size/FPS/duration/count/bytes, keys each
video, produces contact sheets, emits alpha bounds for every delivery frame and
exports atlas pages. Its numerical motion metric compares premultiplied RGB so
hidden background RGB cannot dominate. The largest adjacent-frame motion index
is a **candidate strike frame**, not proof of weapon contact; a recovery, camera
shift or matte failure can be the largest peak. Mark the actual weapon strike
and release by viewing consecutive frames at real speed and slow speed.

For idle, assess both pose and velocity at the wrap, including repeated playback;
do not auto-pass because last/first image error is small. Drop a duplicate last
frame only if measured. For fall, record recoil start, first ground contact and
settled hold timestamps; the script's remaining-frame MAD <0.35 heuristic is a
settle candidate, not a fall-duration measurement. At least half a second of
stable final pose is requested. Inspect every clip's full native frames: reduced
resolution alpha bounds can miss a thin tip leaving the canvas. Neural matting
that erases an escaping tip can also create false containment success.

Identity review must match the guard and canonical source: face, braids/beard,
costume, limb anatomy, weapon length and grip, no extra person or prop. Never use
full-frame pixel differences as a face-identity score. Require an independent
running-output evaluator after the actual clips exist; still sheets alone do
not approve motion, timing or looping. No audio is generated or claimed.

## Delivery recommendation, pending renderer proof

Start with **RGBA frame atlases at 256x384 and native 24 fps**, with a retained
768x1152 authoring master and native video. This is an initial quality tier, not
a claim that it resolves photographic detail on every TV. Validate projected
figure size at the home camera; supply a 384x576 tier only if the measured display
needs it. Do not silently halve attack FPS: 24 fps gives a 41.7 ms timing interval.

The prototype packs 16 frames/page, four columns/four rows, 2 px extruded gutters:
1040x1552 RGBA pages. Its 120-frame control uses eight pages (last partly empty),
49.26 MiB decoded without mipmaps. Raw 120-frame texels alone are 45 MiB. Three
fully resident clips would approach 148 MiB before other textures. Preload only
the selected/near-use clips, bound resident pages, reuse idle resources and retain
the approved still when a page cannot load. Use a page/array layout supported by
the actual device limits; do not build one huge strip.

Alpha VP9 WebM is the attractive bandwidth/storage alternative (29-35 KB on these
controls versus 1.7-2.0 MB atlas PNGs), but **not yet the verified renderer format**.
[WebGPU external textures](https://gpuweb.github.io/gpuweb/explainer/) can expose
video frames; this does not establish VP9 alpha support, correct blending or
precise synchronization on the target browser/device. Test repeated idle loops,
alpha over light/dark rooms, mirroring, seek/start latency, dropped frames,
visibility changes, reconnect and concurrent actors in the actual Rust/WASM
renderer before selecting WebM. No browser or WebGPU playback test occurred here.

For both formats, use a fixed registered canvas and ground/root anchor shared
across the clip family. Compute a union alpha bound once for admission/culling;
do not rescale or replant from each frame's changing alpha box: the figure would
slide, grow or pop during a sword swing and fall. Keep root/ground anchoring when
switching to the fallen hold. Existing still alpha-box planting needs an explicit
clip registration contract before connecting animated bytes.

Idle repeats on the server's presentation timebase. Attack plays once, then returns
to the identical guard; last hit plays once, then holds the fallen end frame.
Mirror UV X for the other side without mirroring the world anchor; define
handedness/weapon consequences in the approved style. A guard pose with a javelin
cannot stand in for a greatsword family. Neither render format chooses rules or
replays a past impact on reconnect. Camera returns home before presenting these
view-dependent clips; off-angle views use an approved still or separately filmed
angle family. Scheduling that return is coordinator/renderer work, outside A1.

## Resource hub proposal, not a new public contract

Use the existing reserved `ResourceKind::VideoClip`, with a concrete spec owned
and frozen by the shared resource owner before implementation. Keep all clip
dependencies and output derivatives under one generation ownership path.
The resource key is the existing scheme: first 16 bytes of
SHA-256(`video_clip|<contract-version>|<canonical-spec-json>`), lowercase hex.
Stable field ordering and serialized units must match Rust's canonical spec.

Required spec inputs:

- Audience-safe character identity ID, appearance revision, canonical source
  logical handle **and actual source PNG SHA-256**.
- Clip kind (`guard_idle`, `attack`, `last_hit`), weapon style (`greatsword_cut`,
  `javelin_throw_rearm`, `shortsword_thrust`) and approved gear revision.
- `battle-home-a1-v1-pending-c5` or its replacement: scene-front convention,
  pitch/FOV, framing/registration policy and camera-reference revision.
- Guard/fallen frame hashes as applicable; reference identity dependency;
  fixed canvas size and anchor; master format and color/alpha interpretation.
- Model endpoint and revision when known, duration, prompt-template revision,
  safety/style policy, matte model/version and key/despill settings.
- Delivery derivative size/FPS/codec/alpha/gutter layout and generator revision.

Output manifest: opaque ready handle, byte hashes/sizes, actual native dimensions,
FPS and frame PTS, loop/one-shot policy, visually approved impact/release/fall
markers, fixed root anchor, union bounds, contained/not-contained verdict,
identity/home-angle/reference revisions and review status. Unreviewed generation
is not ready approved art. Mirroring can reuse the clip bytes; include facing in
a derivative key only if pixels are actually rebaked. Stale dependencies invalidate
future requests without deleting historical assets or automatically respending.
No guessed resource is registered by the research scripts.

When integrated, use shared OTEL operation/job correlation and hub SPEND/RESOURCE
events for submit/accepted/completed/failed/cancelled, generation/matting/export
elapsed time, cost/usage and derivative bytes. Log presentation epoch, clip start,
scheduled/actual impact, camera-home arrival, fall contact/settle, end/switch and
fallback reasons. Developer JSON here is not authoritative gameplay telemetry.

## Cost bounds and reproduction

Actual known paid spend for this attempt: **$0**. Live pricing API verification
was blocked; prices below are dated research inputs, not invoice reconciliation.

| Planned calls, no retries | Dated estimate |
|---|---:|
| Two hero guards + three fallen edits, Kontext Max at $0.08 | $0.40 |
| One Skeleton guard, FLUX Ultra page estimate | $0.06 |
| Nine five-second Kling clips at historical $0.56 | $5.04 |
| Nine five-second Bria video mattes at page $0.14/s | $6.30 |
| Total initial experiment | $11.80 |

Sources read 2026-10-18: [Kontext Max](https://fal.ai/models/fal-ai/flux-pro/kontext/max/multi),
[Ultra](https://fal.ai/models/fal-ai/flux-pro/v1.1-ultra),
[Bria pricing](https://fal.ai/models/bria/video/background-removal);
Kling rate is yesterday's authenticated measurement in the linked local research.
The Ultra page also mentions megapixel rounding: its advertised $0.06 needs live
pricing confirmation for the exact output. Bria's actual output duration may
differ slightly from requested five seconds. Treat $11.80 as a planning estimate.
One retry for each of nine Kling clips adds up to $5.04, plus re-matting costs if
needed. The runner reserves **estimated** cost against $20; it cannot enforce a
provider invoice cap and has no automatic paid retry. Reprice before live work.

From repository PowerShell, after the runtime provides permitted fal connectivity:

```powershell
Set-Location 'C:\Users\mreca\Desktop\dungeonfluxGame'; python artifacts/tmp/char-clips/prototype.py prices
Set-Location 'C:\Users\mreca\Desktop\dungeonfluxGame'; python artifacts/tmp/char-clips/prototype.py archive-connect-failures
Set-Location 'C:\Users\mreca\Desktop\dungeonfluxGame'; python artifacts/tmp/char-clips/prototype.py guards
# Inspect all three generated guards against the source and home-view reference.
Set-Location 'C:\Users\mreca\Desktop\dungeonfluxGame'; python artifacts/tmp/char-clips/prototype.py fallen
# Inspect fixed-scale, fully contained fallen bookends before spending on video.
Set-Location 'C:\Users\mreca\Desktop\dungeonfluxGame'; python artifacts/tmp/char-clips/prototype.py videos
Set-Location 'C:\Users\mreca\Desktop\dungeonfluxGame'; python artifacts/tmp/char-clips/prototype.py mattes
Set-Location 'C:\Users\mreca\Desktop\dungeonfluxGame'; python artifacts/tmp/char-clips/analyze.py clips
```

The archive command refuses a job with a queue identity or an unclassified failure.
Accepted jobs resume by polling their saved queue identity; do not resubmit an
unknown outcome. Provider deadlines are 900 s/job, polls every 5 s, up to three
independent jobs in flight. The key is read from `.env` `DF_FAL_KEY` in-process and
not printed, written into scripts or exposed to browsers. Download files using
the existing curl/OS certificate path; no TLS verification was disabled.

Offline reproduction: `python artifacts/tmp/char-clips/analyze.py controls`.
No dependencies were installed. Python/Pillow/numpy, ffmpeg and ffprobe were
already available. `local-checks.log` retains the numeric results. Script syntax
compilation passed; the local control assertions passed for both characters.
`cargo check -p df-tools --lib` passed in the prescribed lane target, with 177
existing warnings; log: `artifacts/tmp/lanes/A1/cargo-check.log`. No Rust files
were authored, so no Rust formatter, affected Clippy/WASM or Rust test delta exists.
No browser smoke test, neural matte, live animation playback or independent
evaluator acceptance is claimed. Coordinator-issued devlog context was not
provided: `artifacts/tmp/char-clips/devlog-entry.json` is retained for ingestion.

## Live results, 2026-10-18 (coordinator run)

The A1 lane had no network, so the coordinator ran the prototype from its own shell. Everything below
was generated and measured; files are under `artifacts/tmp/char-clips/gpt/<character>/`.

**Frames.** The owner asked for the latest ChatGPT image model. `GET /v1/models` lists
`gpt-image-2.5-flare` and `gpt-image-2.5-sunburst` (both 2026-09-04) as the newest image models;
`chatgpt-image-latest` dates from 2025-12-16. Frames use `gpt-image-2.5-sunburst` (edits, 1024x1536,
quality high, `background: transparent`, PNG), then are composited by us onto flat green (0,177,64).
Generated with transparency, the subject carries no green spill (the FLUX Kontext skeleton had green
bones). Each guard and fallen frame took 28-31 s. The guard frames hold real ready poses (Brakka a
two-handed greatsword raised, Sigrun a javelin raised to throw, the skeleton a shortsword), identity
matches the source cutouts, and the fallen frames are limp, at the same scale, weapon dropped beside.
The first FLUX Kontext frames kept the source's lowered sword, ignored the javelin, and made Brakka
recline rather than fall; they were abandoned.

**Clips.** Kling O1 (`fal-ai/kling-video/o1/image-to-video`, start and end frame, 5 s) for idle
(guard->guard), attack (guard->guard) and last hit (guard->fallen), nine clips: 110-154 s each, three in
flight, 1176x1764, 24 fps, 122 frames, 5.08 s, $0.56 each list. The camera never moved; identity held in
every clip. Idle is a subtle breathing guard. Attack: Brakka's wind-up and diagonal cut and the
skeleton's thrust both land about 1.0-1.4 s in and are back in guard by about 2.5 s, so the second half of
each attack clip is idle. Last hit: recoil, stagger and fall, settled by about 2.5-3 s, then still.
Defects: Sigrun's "throw" never releases the javelin (it reads as a thrust, then she reaches for a second
one); her javelin leaves the top of the frame for two frames in the fall; the skeleton's sword warps
briefly mid-fall.

**Alpha.** Kling kept the plate uniform at about (2,172,64). `chromakey=0x02AC40:0.13:0.05` with
`despill=type=green:mix=0.6:expand=0.1` gives clean edges on hair, beard and cloak at full size and no
green fringe over the riverbed scan (`keyed/*.webm`, VP9 with alpha, about 1.9 MB per 5 s). Defect:
despill tints steel slightly purple; a gentler despill on near-neutral pixels is needed.

**Recommendation (updated).** Frames with gpt-image-2.5-sunburst on transparency, composited on flat
green; Kling O1 first/last frame for the three clips; trim attack to its active window (about 0-2.6 s) and
hold the last-hit's settled frame; key with chroma key + neutral-aware despill. Deliver the green-screen
clip and key it in the renderer's figure shader (one chroma key + despill pass) rather than shipping VP9
alpha, which avoids per-browser alpha codec support and any server-side encoder; the attack carries its
measured contact time so the hit lands on the server's dice result. Per character: two image calls plus
three clips, about $1.7 in clips, about 2.5 minutes wall time in parallel. The home angle is the authoring
angle; clips must be remade if it changes.
