# Landing ambience loop: video provider research

Research snapshot: **2026-10-17**. USD. The first part of the day's work had no working
fal key: the key then in `.env` was rejected with 401, and no video was generated. The
owner then supplied a new key, and three paid proofs were run the same day (about USD 1.01
of a USD 2.00 cap). This note records what was read, what was measured, and what is still
unmeasured. Prices in the model table are list prices; the proof costs are the live list
prices of the three endpoints, not a billed statement.

## Summary

- The first key in `.env` was rejected by fal with `401 invalid key credentials` at both the
  pricing API and the queue. That alone makes every video request fail, and it is the most
  likely cause of every logged failure. The server's own failure detail was never found, so
  the logged 10 s and 20 s timings are explained by inference, not by a reproduction.
- With the new key, the game's exact Kling O1 request (data URIs, `start_image_url` and
  `end_image_url`, `duration` "5") was accepted in 1.3 s and produced a clip in 109 s. The
  clip is 1920 by 1080, 24 fps, 5.083 s, so it passes the game's size and duration checks.
  The request body and the size check are therefore not the cause of the logged failures.
- Measured proofs on the landing still, with the same still as first and last frame:

  | Model | Result |
  |---|---|
  | `fal-ai/kling-video/o1/image-to-video` | photographic, gentle, seamless loop; 109 s; $0.56 |
  | `fal-ai/pixverse/v5/transition` at 720p | stylised smoke plumes and whole-frame edge shimmer; 53 s; $0.05 |
  | `fal-ai/wan-flf2v` at 720p | almost no motion and a small pop at the wrap; 250 s; $0.40 |

- **Recommendation: keep Kling O1 for the landing loop.** It is the only one of the three
  that is both photographic and gently animated, and its loop seam is smaller than the
  largest ordinary frame step. PixVerse is the cheapest by a wide margin but does not look
  photographic. Wan costs nearly as much as Kling and moves almost nothing.
- Other parts of the path still need work: `FILM_LOOK` invites people into an empty room,
  the same still is sent twice as a data URI, a failed submit is reported as
  `QueueIdentity`, and the TV client's still keeps drifting under the video.
- The TV client already cross-fades two stacked clips, but the still keeps drifting and
  carries a lobby grade the video lacks, and the loop can freeze permanently if the tab
  is throttled through the last half second.

## Measured results

All three runs used the same input: the landing still `assets/ui/scenes/mara-harbor-v4.png`
resized to 1920 by 1080 and encoded as a JPEG at quality 95 (534 KB), sent as a base64 data
URI for both the first and the last frame. The scripts are under `artifacts/tmp/video-diag`
(see the tools table below). All three submits were accepted in about 1.1 to 1.3 s, so fal
accepts data URIs for all three endpoints. Times run from submit to the `COMPLETED` status,
polled every 4 s; downloads took about 1 to 2 s more.

| Measure | Kling O1 | PixVerse v5 transition | Wan FLF2V |
|---|---|---|---|
| Endpoint | `fal-ai/kling-video/o1/image-to-video` | `fal-ai/pixverse/v5/transition` | `fal-ai/wan-flf2v` |
| Body fields | `prompt`, `start_image_url`, `end_image_url`, `duration` "5" | `prompt`, `first_image_url`, `end_image_url`, `aspect_ratio` "16:9", `resolution` "720p", `duration` "5"; no `style` sent | `prompt`, `start_image_url`, `end_image_url`, `resolution` "720p", `aspect_ratio` "16:9", `num_frames` 81, `frames_per_second` 16 |
| Prompt | the game's prompt as first read (push-in and `FILM_LOOK`), exactly as the game sent it | the ambience prompt in `prompts.py` | the same ambience prompt |
| Submit to completed | 109.3 s | 53.3 s | 250.2 s |
| List price per clip | $0.56 (5 s at $0.112 per second) | $0.05 | $0.40 |
| File size | 3.99 MB | 1.31 MB | 0.86 MB |
| Output size | 1920 by 1080 | 1280 by 720 | 1280 by 720 |
| Frame rate and frames | 24 fps, 122 frames | 24 fps, 121 frames | 16 fps, 81 frames |
| Duration | 5.083 s | 5.042 s | 5.063 s |
| First frame against the source still (PSNR) | 37.8 dB | 36.7 dB | 34.8 dB |
| Camera shift, first to peak and first to last | 0 px | 1 px at peak, 0 at last | 0 px |
| Frame step, mean and maximum (mean absolute difference, 0 to 255) | 0.24 and 1.18 | 1.27 and 2.82 | 0.20 and 0.96 |
| Seam: last frame against first frame | 0.88 | 2.15 | 1.88 |
| Seam as a fraction of the largest frame step | 0.75 | 0.76 | 1.96 |
| Pixels moving noticeably (temporal deviation above 4) | 8.3% | 18.5% | 0.4% |

How I judged each:

- **Kling O1.** Photographic and faithful: the composition, the lanterns, the lake and the
  floor reflections match the still, and the grain and lighting look like the source. The
  motion is confined to the lake window (water shimmer and moonlight) and the lamp
  reflections on the floor, which is exactly what was asked for. It is subtle but visible
  over five seconds. The seam (0.88) is smaller than the largest ordinary step (1.18), so
  the wrap is as smooth as any other moment in the clip.
- **PixVerse v5 transition.** Not photographic in motion. It adds pastel white and pink
  smoke plumes that rise from the lantern tops and look rendered, not like real smoke. The
  motion map shows the whole frame's edges shimmering (about 18% of pixels move), so the
  textures crawl. The first-frame fidelity is acceptable (36.7 dB). The seam is within the
  largest step (0.76), so it loops, but the look fails the "photographic, gentle" brief. The
  `style` field was left unset, so this is the model's default look. At $0.05 it is the
  cheapest by far, and a prompt without the word "smoke" might calm it, but I did not test
  that.
- **Wan FLF2V.** Photographic, but nearly static: under 0.5% of pixels move and the frame
  step averages 0.20. The last frame is the furthest from the first (1.88 against a
  largest step of 0.96), so the wrap has a small pop even though the clip barely changes.
  Its first frame is the least faithful to the source (34.8 dB), and it is the slowest.
  Not worth $0.40 as a landing loop.

Contact sheets for the owner (all under `artifacts/tmp/video-diag`):

| File | Shows |
|---|---|
| `owner_contact_sheet.jpg` | best sheet: the source still, then four frames (first, one third, two thirds, last) from each model, 2560 by 1552 |
| `motion_maps.jpg` | where each clip moves (temporal deviation, amplified 12 times); Kling moves the lake window and floor reflections, PixVerse moves everything plus the smoke, Wan almost nothing |
| `pixverse_smoke_crop.jpg` | a full-resolution crop of the PixVerse smoke plumes, to judge the stylised look |
| `kling_mirror_contact.jpg`, `pixverse_720p_contact.jpg`, `wan_720p_contact.jpg` | five-frame strips per clip from `seam.py` |

What the Kling run says about the old failures. The request the game sends is accepted, the
clip passes the 1920 by 1080 and 4.5 to 5.5 s checks, and the result is on `v3b.fal.media`,
one of the two pinned hosts. So the request shape, the data URI, the duration and the
size check are all fine. The logged 10 s and 20 s failures are therefore most likely the
old key, but I could not read which key those servers used, and a 401 alone does not give
10 s, so I cannot rule out something in the Rust path I could not run. One environment
note: Python's certificate store could not verify `v3b.fal.media` (a missing issuer), so
the scratch downloader uses curl. The Rust client uses its own store and I did not test
it. If the first request with a good key still fails in the game, check TLS to
`v3b.fal.media` first.

## Key evidence

This section describes the first key, which was in `.env` before the owner replaced it
later the same day. The key was read from `.env` inside a scratch process and never
printed or logged. The
header scheme is the game's own (`Authorization: Key <key>`, `Content-Type:
application/json`, `X-Fal-No-Retry: 1`). The key has the normal `id:secret` shape: 69
characters with one colon. `development/run-gameplay.sh` line 245 extracts it from `.env`
the same way (`DF_FAL_KEY=` line, quotes and carriage returns stripped).

| Call | Result | Time |
|---|---|---|
| `POST https://queue.fal.run/fal-ai/kling-video/o1/image-to-video`, the game's exact body (1.4 MB) | `401 {"detail":"invalid key credentials"}` | 0.68 s |
| Same POST with body `{}` | `401 {"detail":"invalid key credentials"}` | 0.36 s |
| `GET https://api.fal.ai/v1/models/pricing?endpoint_id=fal-ai/kling-video/o1/image-to-video` with the key | `401 {"error":{"type":"authorization_error","message":"Invalid API key"}}` | under 1 s |
| Same pricing GET without a key | `401 API key authentication required` | under 1 s |
| Pricing GET for `fal-ai/wan-flf2v`, and a queue status GET for a dummy request id | both 401 | 0.5 s, 0.3 s |

Facts around it:

- There is no `.mp4` anywhere under `artifacts`, so no fal call has ever succeeded in
  this project.
- `.env` was last modified 2026-10-17 08:21. The `merchant-r4` and `merchant-r5` servers
  started around 06:00 and were still running, so they launched with whatever key existed
  before 08:21. Whether that earlier key was valid is unknown.
- The game's first step is the pricing lookup (`video_price_per_second`,
  `home_video.rs` around line 1091), so a rejected key stops every request there.

### What the 401 does not explain

The server logs show failures at `elapsed_ms` of about 10 s, 10.4 s, 10.9 s, 18.4 s and 9.5 s,
with `provider_ms=none` and `failure_category=provider_failure`. A 401 answers in under a
second, and a pricing failure would be categorised `price_unavailable` in the source as
read. Possible reasons, none verified:

- The running binary differs from the source read, since another worker is editing
  `home_video.rs` and the fal provider files.
- The 10 s and 20 s match `connect_timeout(10 s)` in `limits()` and `PRICE_DEADLINE`
  (20 s). A reset during the 1.4 MB upload is a candidate. A Python client that sent the
  same body saw `SSLEOFError` during the upload, because the server answers 401 and closes
  the connection early.
- `provider_ms=none` means the failure came before the paid call, which points at
  provider construction, request building or the price read rather than the queue.

The full wording of each failure is written by `record_failure_detail`
(`home_video.rs` around line 1877) as the OTEL event `gameplay.home_video.failure`, field
`gameplay.home_video.detail`. The telemetry database was not found under `artifacts`.
Reading that field for one failed run settles this.

## Request fields against fal's schema

Schema source: the fal model page for Kling O1 image-to-video, read 2026-10-17
([fal.ai/models/fal-ai/kling-video/o1/image-to-video/api](https://fal.ai/models/fal-ai/kling-video/o1/image-to-video/api)).
The game's body is built at `crates/df-providers/src/fal_media.rs` lines 354 to 358.

| Field | fal schema | Game sends | Match |
|---|---|---|---|
| `prompt` | string, required; `@Image1` and `@Image2` can refer to the frames | `FILM_LOOK` plus `HOME_MOTION`, about 790 characters | yes (see prompt problems) |
| `start_image_url` | string, required, first frame | the still as a `data:image/jpeg;base64,` URI | yes |
| `end_image_url` | string, optional, last frame | the same URI | yes |
| `duration` | enum string "3" to "10", default "5" | `"5"` as a string | yes |
| `enable_safety_checker` | boolean, default true | not sent | default applies |

Start image requirements from the page: at most 50 MB, at least 300 by 300 px, aspect ratio
0.40 to 2.50, and a 20 s fetch timeout. The still is 1920 by 1080 (aspect 1.78) and 534 KB
as a JPEG, so it meets them. Output schema: `video.url` (required), `content_type`,
`file_name`, `file_size`. The adapter's `output_files` expects `video/mp4`, which matches.
Queue URLs: fal names a request under `fal-ai/kling-video/requests/{id}`, and
`queue_url_confined` accepts that form.

The page does not list accepted image formats, so whether fal accepts a base64 data URI in
these fields is unverified for this endpoint.

## Problems that will bite once a clip arrives

### Prompt

The `HOME_MOTION` I first read began "Locked camera with a slow, almost imperceptible
optical push-in". With the same still forced as the last frame, a push-in means the model
must push in and pull back out, which creates drift or a visible return. A loop wants a
fully locked-off camera and ambient motion only: lamp flicker, drifting smoke or dust,
cloth or rope stirring, water shimmer. When I rechecked on 2026-10-17, `home_video.rs`
line 73 had already been rewritten by the worker who owns the file as "Locked-off camera:
the frame does not move, zoom or drift", with ambient motion only and "the last frame
matches the first". So the push-in is fixed in the current source, and only the
`FILM_LOOK` issue below remains. The rewritten text still allows "anyone in the frame",
which fits `FILM_LOOK` but not an empty room.

`FILM_LOOK` (`imagery/prompt.rs` lines 23 to 27) is written for people: "real adult
performers, simple sewn wool and linen costumes… skin pores". Sent with an empty harbour
room, it invites people to appear. `HOME_MOTION` says no new people, but the two sentences
pull against each other. An ambience-loop prompt should use a landscape-only look clause
(documentary photograph, natural practical light, film grain) without cast language.

### Exact 1920 by 1080 size check

`run_job` (`home_video.rs` lines 1173 to 1177) rejects any clip whose size is not exactly
1920 by 1080 or whose length is outside 4.5 to 5.5 s. `validate_output` and the profile
(`p.width.min(p.height) == 1080` for "1080p") repeat it. Measured: Kling O1 returns
1920 by 1080 at 24 fps and 5.083 s, so it passes. The alternatives return other sizes
(1280 by 720 for both PixVerse and Wan at 720p), so if the game ever uses one of them the
check must accept that provider's real output size, or the profile must say what the model
produces. The check is not the cause of the logged failures.

### Data URI duplication

The same 712 KB base64 string is sent as both `start_image_url` and `end_image_url`, so the
request body is about 1.4 MB. Measured: all three endpoints accepted this body and answered
the submit in 1.1 to 1.3 s, so it works today. It still doubles the upload for no reason, so
upload the still once to fal storage and send the same URL twice if the body ever becomes a
problem.

### Error attribution

An error at submit, such as the 401, is parsed as a queue reply that has no `request_id`
and is reported as `QueueIdentity` (`fal_media.rs` lines 447 to 458), not as an HTTP
status. A 401, a 422 and a 5xx all look the same in the log. The adapter should keep the
status code and a short, key-free body excerpt on the error and write them into the
failure detail.

### Adapter allow-list

`with_video_model` (`fal_media.rs` line 152) accepts only the family default and Kling O1.
Any model in the table below needs its own body, its own duration and size rules, and its
own output checks.

## Cheaper first and last frame models

Sources: fal model pages read 2026-10-17, the fal pricing page, and one web search the same
day. fal's pricing API needs a valid key, so live unit prices were not read. Generation time
is not measured for any model.

| Model id | Price per clip | Durations | First and last frame fields | Verdict |
|---|---|---|---|---|
| `fal-ai/kling-video/o1/image-to-video` | about $0.112 per second, so $0.56 for 5 s and $1.12 for 10 s; the fal page is 319 days old | "3" to "10" s | `start_image_url`, `end_image_url` (optional) | measured: photographic, gentle, seamless; the most expensive here, and the one to keep |
| `fal-ai/wan-flf2v` | $0.20 at 480p, $0.40 at 720p, per video; 1.25 times more above the default frame count | no `duration` field; `num_frames` 81 to 100 at `frames_per_second` 5 to 24, so about 5.1 s at the default 16 fps | `start_image_url`, `end_image_url`, both required; `aspect_ratio` "auto" follows the image; images are resized and centre-cropped | measured at 720p: nearly static, small pop at the wrap, slowest; not recommended |
| `fal-ai/pixverse/v5/transition` | about $0.15 at 360p or 540p, $0.20 at 720p, $0.40 at 1080p for 5 s; a third-party figure for PixVerse generally, not for this endpoint | "5" or "8" s | `first_image_url`, `end_image_url`, both required; has a `style` enum (anime, 3d_animation, clay, comic, cyberpunk) | measured at 720p: cheapest by far at $0.05, but adds rendered-looking smoke and edge shimmer; not photographic |
| `fal-ai/vidu/start-end-to-video` | about $0.035 per second at 360p or 540p, 2.2 times more at 720p or 1080p; that figure is for Vidu Q3 | not listed on the page | `start_image_url`, `end_image_url`; `movement_amplitude` auto, small, medium, large | worth a trial with "small" |
| `fal-ai/kling-video/v2.1/standard/image-to-video` | not listed | "5" or "10" s | none: no tail field in the current schema; the page says it is no longer supported | ruled out |
| `fal-ai/bytedance/seedance/v1/lite/image-to-video` | not listed | "2" to "12" s | `end_image_url` exists but is deprecated and ignored, since requests redirect to Seedance 1.0 Pro Fast, which has no end frame | ruled out |
| `fal-ai/ltx-2/image-to-video` | not listed | "6", "8", "10" s | none: no end frame field | ruled out |

Page sources:
[Wan FLF2V](https://fal.ai/models/fal-ai/wan-flf2v),
[PixVerse v5 transition](https://fal.ai/models/fal-ai/pixverse/v5/transition),
[Vidu start-end](https://fal.ai/models/fal-ai/vidu/start-end-to-video),
[Kling 2.1 standard](https://fal.ai/models/fal-ai/kling-video/v2.1/standard/image-to-video),
[Seedance lite](https://fal.ai/models/fal-ai/bytedance/seedance/v1/lite/image-to-video),
[LTX-2](https://fal.ai/models/fal-ai/ltx-2/image-to-video),
[fal pricing](https://fal.ai/pricing).
Price sources: the Wan figures and the Kling O1 figure are the fal model pages as shown in
a web search on 2026-10-17. The fal pricing page itself listed no price for these models.
The PixVerse and Vidu figures come from a third-party guide, not from fal.

Live prices read from fal's pricing API with the new key on 2026-10-17: Kling O1 $0.112 per
second, Wan FLF2V $0.40 per video, PixVerse v5 transition $0.05 per video segment. They
replace the page and third-party figures in the table above where they differ; in particular
PixVerse is $0.05, not the $0.15 to $0.20 a third-party guide gave.

Recommendation, after the measured proofs: keep Kling O1. Neither cheaper model matched it
on the landing still (see the measured results above). If a cheaper loop is wanted later,
the first thing to try is PixVerse with a prompt that has no smoke in it, since its cost is
one eleventh of Kling's, but that is untested.

## Loop seam method

Tools, all under `artifacts/tmp/video-diag`:

| File | Purpose |
|---|---|
| `fal.py` | scratch fal queue client: reads the key from `.env`, prepares the still, submits, polls, downloads; prints status codes and bodies, never the key |
| `call1_mirror.py` | the one mirror call of the game's exact request; its output went to `call1.log` (a 401) |
| `still_1920.jpg` | the landing still prepared as the game does it: 1920 by 1080, JPEG quality 95 |
| `harbor_preview.jpg` | a 960 by 540 preview of the still |
| `seam.py` | seam measurement and contact sheet, usage `python seam.py clip.mp4 [out_prefix]` |
| `synth_test.mp4`, `synth_test_contact.jpg` | a 2 s synthetic clip used only to check that `seam.py` runs |
| `prompts.py`, `run_pixverse.py`, `run_wan.py` | the ambience prompt and the PixVerse and Wan proof calls |
| `call1_mirror.mp4`, `pixverse_720p.mp4`, `wan_720p.mp4` | the three proof clips (4.0, 1.3 and 0.9 MB) |
| `pixverse.log`, `wan.log`, `call1.log` | status polls, result and timing for each run |
| `compare.py` | per-clip fidelity, motion and seam numbers against the source still, and `owner_contact_sheet.jpg` |
| `owner_contact_sheet.jpg`, `motion_maps.jpg`, `pixverse_smoke_crop.jpg` | the sheets to show the owner |

The landing still is `assets/ui/scenes/mara-harbor-v4.png` (1672 by 941), a dark
lamp-lit harbour boat shed. `public_assets.rs` serves it as `ART_PATH`.

`seam.py` decodes the clip to 480 by 270, computes the mean absolute pixel difference
between consecutive frames, and the difference between the last frame and the first. It
prints the mean and maximum frame step, the seam difference, the middle-versus-first
difference and the ratio of seam to mean step, and writes a five-frame contact sheet
(first, quarter, middle, three quarters, last).

Acceptance rule: the last-versus-first difference should be no larger than the largest
frame-to-frame difference in the same clip, so the wrap is no rougher than any other
moment. I first proposed 3 times the mean frame step, but the measured clips show that is
too strict for gentle clips: Kling's mean step is 0.24, so a mean-based rule rejects its
perfectly smooth seam (0.88 against a largest step of 1.18). Measured against the largest
step, Kling is 0.75 and PixVerse is 0.76 (both pass) and Wan is 1.96 (fails). Look at the
contact sheet as well, since a metric can pass while a lamp or water highlight jumps.

Treatment, in order of preference:

1. Use the provider's same-still first and last frame alone, if the measured seam passes.
2. If it fails, cross-dissolve the last `f` seconds (about 0.5 s) into the first `f`
   seconds and drop the overlap from the length. In ffmpeg, split the clip into head
   (0 to `f`), body (`f` to `D-f`) and tail (`D-f` to `D`), blend the tail into the head with
   `xfade` (duration `f`, offset 0), and concatenate the blend followed by the body. The
   result is `D-f` long, and its end continues into its start with no jump.
3. Ping-pong (forward then reverse) is not needed when the two ends match, and it makes
   flickering light run backwards, so it is a last resort.

The method has now been run on the three provider clips; the results are in the measured
results section. Kling and PixVerse need no tail cross-dissolve. Wan would need one, but it
is not recommended anyway.

## TV client transition design

Code read: `crates/df-tools/src/gameplay_browser/live_home_video.rs`, `live_tv.rs`, and the
stylesheets `live_home.css`, `live_tv.css`, `live_fx.css`.

### How it works today

- The still is `.tv-art`, a CSS background (`background-size: cover`, position
  `center 40%`). The loop layer `.tv-loop` is a sibling above it.
- The layer holds two stacked muted `<video>` elements with no `loop` attribute. The first
  fades in on `canplaythrough` (900 ms when `data-first` is set, otherwise 500 ms linear).
  Near the end (0.5 s before), the second restarts from 0 and the two cross-fade, driven by
  `requestAnimationFrame`.
- Any error, a refused `play()`, or leaving the lobby releases both and the still stays on
  screen. The still is never removed, so there is no black flash.
- The loop plays only in the lobby (`live_tv.rs` around line 2799). Both elements load the
  same file, so it is fetched twice.

### Problems

1. Framing and grade mismatch. `.tv-art` runs `df-drift` (scale 1.02 to 1.07 with a small
   translate, 70 s, alternate) and, in the lobby, a `brightness`, `contrast` and
   `saturate` filter from the `--home-*` variables. The video has neither. The fade-in
   therefore changes framing and colour.
2. The loop can die. The swap happens only in a frame callback and only if the front clip
   is near its end and not paused. If the tab is hidden or throttled through the last 0.5 s,
   the clip ends, `paused` becomes true, and the condition never passes again. The video
   freezes on its last frame, which equals the first frame, so it looks like a still and
   never moves again.
3. The first-frame gate is `canplaythrough`, which says the data is buffered, not that a
   frame has painted.

### Proposed transition

- When to start: only in the lobby, after `canplaythrough` on the first clip, after
  `play()` resolves, and after the first video frame is presented
  (`requestVideoFrameCallback`, falling back to a `timeupdate` with time above 0). Until
  then the still stays fully visible and opaque underneath.
- Match before fading: at fade start, read the still's computed transform and copy it onto
  `.tv-loop`, then pause the still's animation (`animation-play-state: paused`). Apply the
  same lobby filter to `.tv-loop-clip` so the grade is identical. The video then moves in
  place over a frozen still.
- Fade: opacity from 0 to 1 over about 1.2 s with ease-in-out, with the still untouched
  underneath. Do not fade the still out until the video is on, and never fade through black.
- Loop: keep the two-element cross-fade (0.5 s overlap) rather than native `loop`, which can
  hitch at the wrap in some browsers. Because the clip's first and last frames match, the
  cross-fade blends near-identical images. Add an `ended` listener on each clip that starts
  the other clip immediately, so a throttled tab recovers.
- Release: on error, a refused `play()`, or leaving the lobby, release both elements and
  clear the frozen transform and filter, so the still resumes its drift.
- Verify in a real browser. The browser pane starves timers and has no focus events, so it
  cannot judge fade timing or the `ended` recovery.

## Still unmeasured

- Which key the `merchant-r4` and `merchant-r5` servers actually used, and the exact reason
  for the logged 10 s and 20 s failures. The Kling run shows the request is accepted and the
  clip passes the game's checks, so the old key is the likely cause. The OTEL
  `gameplay.home_video.failure` detail on one failed run would settle it.
- Whether the Rust client verifies `v3b.fal.media` and `v3.fal.media` correctly. The Python
  scratch client could not verify the certificate issuer, and curl could.
- Kling O1 with the rewritten `HOME_MOTION` (locked-off, last frame matches the first) and a
  landscape-only look clause. The Kling proof used the old prompt, so a second run with the
  new prompt would cost another $0.56 and show whether the gentle result improves.
- Whether PixVerse with a smoke-free prompt, or Vidu start-end with `movement_amplitude`
  "small", can reach a photographic look at a lower price. Neither was run.
- Whether the Kling clip's wrap looks right on the TV with the proposed client transition.
  The numbers say the seam is smooth, but nobody has watched it loop in a real browser.
- The billed cost of each run. The costs above are fal's live list prices, and the total of
  about USD 1.01 is their sum; no billing statement was read.
- How the client transition looks and behaves on the TV page in a real browser.
