# Landing ambience loop: video provider research

Research snapshot: **2026-10-17**. USD. No video was generated: the only fal key in
the project is rejected with 401, so no paid call was possible and none was made. Every
price below is a list price read from a page or search snippet, not a billed amount.
This note records what was read, what was measured, and what is still unmeasured.

## Summary

- Every request in the server logs fails before any clip exists. The `DF_FAL_KEY` in
  `.env` is rejected by fal with `401 invalid key credentials` at both the pricing API
  and the queue. That alone makes every video request fail.
- The request body the game sends matches fal's current schema for
  `fal-ai/kling-video/o1/image-to-video`. No field is wrong.
- The logged elapsed times of about 10 s and about 20 s are not explained by a 401, which
  returns in under a second. The server's own failure detail was not found on disk, so the
  logged failures are not yet reproduced.
- Once a clip does arrive, several other parts of the path will reject or degrade it:
  the prompt I first read asked for a push-in that contradicts a seamless loop (since
  rewritten in the current source), `FILM_LOOK` invites people into an empty room, the output size check is exact 1920 by 1080, and the same
  still is sent twice as a data URI.
- Two cheaper first/last-frame models are worth a trial: `fal-ai/wan-flf2v` and
  `fal-ai/pixverse/v5/transition`. Neither has been run.
- The TV client already cross-fades two stacked clips, but the still keeps drifting and
  carries a lobby grade the video lacks, and the loop can freeze permanently if the tab
  is throttled through the last half second.

## Key evidence

The key was read from `.env` inside a scratch process and never printed or logged. The
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
(`p.width.min(p.height) == 1080` for "1080p") repeat it. Kling O1 documents no output
resolution. If it returns 1280 by 720, the clip is paid for and then discarded. The
alternatives below return other sizes (Wan at 480p or 720p, PixVerse at 360p to 1080p), so
the check must accept the provider's real output size, or the profile must say what that
model produces.

### Data URI duplication

The same 712 KB base64 string is sent as both `start_image_url` and `end_image_url`, so the
request body is about 1.4 MB. If fal limits or times out on data URIs (its start image
fetch timeout is 20 s), the failure is hard to diagnose. Upload the still once to fal
storage and send the same URL twice.

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
| `fal-ai/kling-video/o1/image-to-video` | about $0.112 per second, so $0.56 for 5 s and $1.12 for 10 s; the fal page is 319 days old | "3" to "10" s | `start_image_url`, `end_image_url` (optional) | works as designed; the most expensive here |
| `fal-ai/wan-flf2v` | $0.20 at 480p, $0.40 at 720p, per video; 1.25 times more above the default frame count | no `duration` field; `num_frames` 81 to 100 at `frames_per_second` 5 to 24, so about 5.1 s at the default 16 fps | `start_image_url`, `end_image_url`, both required; `aspect_ratio` "auto" follows the image; images are resized and centre-cropped | real first and last frame support and clearly cheaper; try first |
| `fal-ai/pixverse/v5/transition` | about $0.15 at 360p or 540p, $0.20 at 720p, $0.40 at 1080p for 5 s; a third-party figure for PixVerse generally, not for this endpoint | "5" or "8" s | `first_image_url`, `end_image_url`, both required; has a `style` enum (anime, 3d_animation, clay, comic, cyberpunk) | cheapest; may stylise, so check against the photographic look |
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

Recommendation: trial `fal-ai/wan-flf2v` at 720p ($0.40) first, then
`fal-ai/pixverse/v5/transition` at 720p (about $0.20) as the cheaper second test. There is
no quality or seam evidence for either yet. The 1080p size check has to change for both.

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

The landing still is `assets/ui/scenes/mara-harbor-v4.png` (1672 by 941), a dark
lamp-lit harbour boat shed. `public_assets.rs` serves it as `ART_PATH`.

`seam.py` decodes the clip to 480 by 270, computes the mean absolute pixel difference
between consecutive frames, and the difference between the last frame and the first. It
prints the mean and maximum frame step, the seam difference, the middle-versus-first
difference and the ratio of seam to mean step, and writes a five-frame contact sheet
(first, quarter, middle, three quarters, last).

Acceptance rule: the last-versus-first difference should be within about 3 times the mean
frame-to-frame difference. A clip that meets it loops without a visible pop when played
back to back. Look at the contact sheet as well, since a metric can pass while a lamp or
water highlight jumps.

Treatment, in order of preference:

1. Use the provider's same-still first and last frame alone, if the measured seam passes.
2. If it fails, cross-dissolve the last `f` seconds (about 0.5 s) into the first `f`
   seconds and drop the overlap from the length. In ffmpeg, split the clip into head
   (0 to `f`), body (`f` to `D-f`) and tail (`D-f` to `D`), blend the tail into the head with
   `xfade` (duration `f`, offset 0), and concatenate the blend followed by the body. The
   result is `D-f` long, and its end continues into its start with no jump.
3. Ping-pong (forward then reverse) is not needed when the two ends match, and it makes
   flickering light run backwards, so it is a last resort.

The method was tested on a synthetic clip only. It has not been run on a provider clip.

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

## Still unmeasured until a working key exists

- Whether the old key (before 08:21) was valid, and what the logged 10 s and 20 s failures
  were. The OTEL `gameplay.home_video.failure` detail would answer it.
- Whether Kling O1 accepts a data URI for `start_image_url` and `end_image_url`.
- Kling O1's output resolution and frame rate, and whether the exact 1920 by 1080 check
  rejects it.
- Real generation time and cost for Kling O1, Wan FLF2V and PixVerse transition.
- The seam difference and contact sheet for any provider clip, and whether the same-still
  first and last frame is enough or a tail cross-dissolve is needed.
- Whether PixVerse's style bias, Wan's 16 fps or Vidu's amplitude setting suits a
  photographic ambience loop.
- Live per-second prices from fal's pricing API; the figures above are page and search
  snippets, and the PixVerse and Vidu ones are third-party.
- How the client transition looks and behaves on the TV page in a real browser.
