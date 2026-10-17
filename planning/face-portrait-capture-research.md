# Face portrait capture: research

Date: 2026-10-17
Status: research and light prototype. No provider call was made, no spend, no server or build of the game was started, and nothing under `crates/` or `content/` was changed. Governing documents: [AGENTS](../AGENTS.md), [client architecture](client-architecture.md), [asset engine](asset-engine.md), [RPC transport](rpc-transport.md), [RPC API](rpc-api.md), [character backstory creation](character-backstory-creation.md), and the queue task PLAY-P01 in `development/operational-tasks.json`.

## 1. Summary and recommendation

- Capture: the browser client takes the photo with `getUserMedia` (front camera preferred, a laptop webcam accepted), draws it to a canvas at 768 px on the long side and encodes JPEG at quality 0.85. The file-chooser fallback (`<input type="file" accept="image/*" capture="user">`) uses the same downscale path. Camera access needs a secure context (HTTPS or localhost), so phones on a LAN address need HTTPS; the file fallback works over plain HTTP.
- Transport: a client-streaming RPC (Start, ordered chunks of at most 48 KiB, End, receipt) over the existing gRPC-over-WebSocket bridge. The photo never goes over HTTP. The current RPC message ceiling is 64 KiB (`RPC_MESSAGE_BYTES`), and a 768 px JPEG is roughly 80 to 200 KB, so it takes 2 to 5 chunks.
- Likeness model: `fal-ai/flux-pulid` (PuLID Flux, a FLUX.1-based identity model). It is the only identity model on fal that fits the owner's FLUX-only photographic rule. Price on the fal page is $0.0333 per megapixel, rounded up, so a 768 by 1024 portrait bills as one megapixel, about $0.033. The fallback is the already-wired `fal-ai/flux-pro/kontext/max/multi` at $0.08 per image, which edits the photo rather than matching the face.
- Use of the result: the approved portrait becomes the hero's portrait depiction and the identity figure for plates through the existing cast-sheet path. The raw photo is never a plate reference. The reference loader stays closed to everything except reviewed photographs and approved likenesses.
- Privacy: an explicit consent screen before the camera opens; the photo held in memory only for one generation call; the fal request sent with `X-Fal-Store-IO: 0` and a short object lifecycle, because fal keeps request payloads 90 days by default; withdrawal deletes the likeness and its derived images. Biometric and minor-related rules are in section 6.
- Integration: a new resource kind `Likeness` keyed by hero and a random consent identifier (never a hash of the photo), a consent record in PostgreSQL, a `dev likeness show|set|approve|clear|regenerate <hero>` command group through the existing dev control surface.
- Risks found in the repo that need a decision before any build: see section 2. The most important are the painted-art wording in PLAY-P01, the spend-cap wording in PLAY-P01, and the fact that the current fal adapter does not send the no-storage header.

## 2. Conflicts with existing documents and code (decide first)

1. Art style. PLAY-P01 says "a painted hero portrait and stage cut-out in the game's art style". The art direction is photographic (`FILM_LOOK` in `crates/df-tools/src/gameplay/imagery/prompt.rs`: "Documentary color photograph ... No ... painting or illustration"), and the image route is FLUX only. This report assumes a photographic portrait. PLAY-P01 should be corrected.
2. Spend cap. PLAY-P01 acceptance names "an exhausted spend cap". `crates/df-tools/src/gameplay/resources.rs` says generation has no spend cap of any kind, and the ledger never refuses. The likeness path follows the ledger rule and records spend without a cap.
3. Rating wording. PLAY-P01 asks for "family-suitable" results. The game is an adult game with a `mature` default (planning/gameplay-tuning.md section 11). This report reads that phrase as "no nudity, no gore and no sexual content in the portrait", which holds under both ratings.
4. Reference rule. `FILM_LOOK`'s `NO_REAL_PEOPLE` line ("Fictional identities; no recognizable real person") contradicts a likeness prompt. Likeness prompts must replace that line with a consented-likeness line (section 7.3). Do not weaken the rule for other prompts.
5. Reference store. `crates/df-tools/src/gameplay/imagery/references.rs` serves only reviewed photographs from the art contract, and returns `NotPhotographic` for anything else. A player photo needs its own source. The art contract should not be loosened.
6. Storage header. `crates/df-providers/src/fal_image.rs` sends references as inline data URIs with `sync_mode`, and sends no `X-Fal-Store-IO` header. Per fal's data-retention page, request inputs and outputs are kept 90 days by default, so every person-photo reference sent today would be kept. The likeness call must set the header, and the adapter should set it for all person references.
7. Phone size limit. The brief mentions an 8 KiB limit on some phone clients. The only current constants are `FRAME_BYTES` 16 KiB, `MESSAGE_BYTES` 256 KiB and `RECEIVE_BYTES` 1 MiB in `crates/df-rpc-bridge/src/lib.rs`, plus `RPC_MESSAGE_BYTES` 64 KiB. The 8 KiB figure appears only in a comment in `crates/df-tools/src/gameplay_browser.rs` (around line 988), which says the owner's view exceeded 8 KiB in a three-enemy fight and was raised to `RPC_MESSAGE_BYTES`. I found no 8 KiB limit left in code. It should be confirmed on a real phone before the chunk size is fixed.
8. Identity weight range. The fal schema for PuLID gives `id_weight` a range of 0 to 1. The upstream PuLID documentation describes 0.0 to 3.0. Plan tuning inside 0 to 1 on fal.

## 3. Capture in the browser (Rust/wasm, web-sys)

### 3.1 Secure context and LAN play

- `navigator.mediaDevices` exists only in a secure context. MDN says `getUserMedia()` works only from HTTPS, `file:` or `localhost`. A phone opening `http://192.168.x.x:port` gets no camera API. [MDN getUserMedia](https://developer.mozilla.org/en-US/docs/Web/API/navigator/mediaDevices.getUserMedia)
- Desktop development on `http://localhost` works without a certificate. A laptop on the LAN has the same problem as a phone.
- Options for LAN play: (a) serve the game over HTTPS with a certificate the phone trusts (a locally trusted CA such as mkcert, installed on the phone); (b) use the deployed HTTPS origin, which the transport already requires (WSS in deployment, `planning/rpc-transport.md`); (c) fall back to the file chooser, which does not need a secure context. Option (b) is the normal path. Option (c) keeps LAN demos working without a certificate.
- The browser asks for permission each time unless it has stored a grant. Only the top-level document may request it unless the page delegates the permission to an iframe through Permissions Policy. The game does not use iframes for this.

### 3.2 Camera path (what the prototype does)

- Constraints: `video: { facingMode: { ideal: "user" } }` and `audio: false`. `facingMode` is a preference, so a laptop without a front camera still answers. MDN: `"user"` is the front camera on a phone; a non-exact value is a preference, and only `{ exact: ... }` fails when the camera is missing. [MDN facingMode](https://developer.mozilla.org/en-US/docs/Web/API/MediaTrackConstraints/facingMode)
- Playback: a `<video>` element with `muted`, `autoplay` and `playsinline`. WebKit's policy for iOS video says muted video may autoplay without a gesture and that unmuting without a gesture pauses it. [WebKit video policies](https://webkit.org/blog/6784/) The search summaries also report that a stream on iOS 15 froze before playing, and that the fix is the attribute set above. I could not confirm current iOS behaviour from 2025 or 2026 sources, so this must be tested on a device.
- Capture: draw the video element onto a canvas sized by `fit_within(width, height, 768)`, then `toBlob("image/jpeg", 0.85)`, then read the bytes. The stream is stopped in every case (`stop()` on each track), so the camera light goes off on success, refusal and error.
- Preview: the prototype does not show a preview. The product needs a mirrored preview, a shutter button, and a retake control. The shutter must be a user gesture.

### 3.3 File fallback

- `<input type="file" accept="image/*" capture="user">`. The MDN page says `capture="user"` asks for the user-facing camera and/or microphone, and that the user agent may fall back to its default if that mode is unavailable. It is not described for desktop browsers. [MDN input file](https://developer.mozilla.org/en-US/docs/Web/HTML/Element/input/file) The `accept` section says many mobile devices offer the camera with `image/*`.
- The input must be opened synchronously inside a click handler, or iOS Safari will not show the picker. The prototype creates the input, calls `click()` in the same turn, and waits on `change` or `cancel`. Chrome and Safari fire `cancel` on dismissal; the prototype handles it so the promise cannot hang.
- Decoding: `createImageBitmap(blob)` decodes JPEG, PNG, WebP and, in Safari, HEIC. Chrome on Android may not decode HEIC; the UI should then ask for a JPEG or PNG. EXIF orientation: `createImageBitmap` applies the image's orientation by default in current browsers (`imageOrientation: "from-image"`). This must be checked on a rotated iPhone photo before release.

### 3.4 Platform quirks to test

- iPhone Safari: camera permission sticks per site; muted and playsinline are required; the first `play()` must follow a gesture; the picker must open synchronously; HEIC may arrive as HEIC.
- Android Chrome: `facingMode` works; the permission prompt is per origin; the back camera is picked if the front one is missing.
- Laptop: `facingMode` is ignored; the default webcam is used; the camera LED stays on until the tracks are stopped.
- Tablet: same as the phone for the picker; check the aspect ratio on landscape.

### 3.5 Size and compression

- Long edge 768 px, JPEG quality 0.85. For a head-and-shoulders crop this keeps skin texture. Size estimate (not measured): 80 to 200 KB, for 2 to 5 chunks at 48 KiB. This is a planning number to confirm in the browser test.
- The server refuses inputs above 15 MB before decoding (the prototype's check) and rejects files that do not decode.
- The prototype does not crop to the face. A face-centred crop needs a face detector; the cheaper option is a guide frame on the preview so the player frames their head and shoulders.

## 4. Transport over the existing bridge

### 4.1 Limits found in the code

- `crates/df-rpc-bridge/src/lib.rs`: `FRAME_BYTES` 16 KiB, `MESSAGE_BYTES` 256 KiB (decoded tunnel message), `RECEIVE_BYTES` 1 MiB, `RPC_MESSAGE_BYTES` 64 KiB, `CONCURRENT_STREAMS` 8.
- `planning/rpc-transport.md` sets the window and buffer rules: connection window 1 MiB, stream window 256 KiB, at most 8 MiB outstanding per connection, and bulk work at lower priority than actions and audio.
- A photo upload must stay within the 64 KiB RPC message ceiling. The gRPC framing takes a few bytes, so 48 KiB chunks are safe.

### 4.2 Shape (sketch only; field numbers are a shared-owner prerequisite)

Follow the existing client-streaming pattern in `planning/rpc-api.md` (TalkInput: Start, ordered Chunk, End or Cancel, receipt). Sketch, not a contract:

```text
service Likeness {
  rpc UploadPhoto(stream PhotoPart) returns (PhotoReceipt);
}
PhotoPart  { oneof part { PhotoStart start; bytes chunk (<= 48 KiB); PhotoEnd end; } }
PhotoStart { operation_id; hero_id; consent_id; mime (image/jpeg); total_bytes (<= 1 MiB); chunk_count; width; height }
PhotoEnd   { sha256 of the whole JPEG }
PhotoReceipt { status: accepted | rejected(reason) | duplicate; likeness_state }
```

- Server validation before any decode: consent record exists and is current for this seat and hero, MIME is JPEG, total size within the limit, chunk offsets contiguous, SHA-256 matches, and the seat is allowed to upload now (rate limit: one pending likeness per seat).
- The server holds the bytes in memory only, keyed by `operation_id`. It never writes them to a file, a database row, a log or a telemetry span.
- Retry: a lost receipt is retried with the same `operation_id`; the server returns `duplicate` with the existing state. Cancelling half-way drops the buffer.
- Concurrency: one upload per seat at a time. The client sets `max_decoding_message_size` and `max_encoding_message_size` to `RPC_MESSAGE_BYTES`, as `gameplay_browser.rs` does for the session client.
- Never use `/gameplay/imagery/...` or any other HTTP route for the photo. HTTP is for boot only (`planning/rpc-transport.md`).

### 4.3 Measurements still to take

Upload time for 2 to 5 chunks on a phone over Wi-Fi and 4G, peak WASM memory during encode, and whether the preview stays smooth while the upload runs. These need a browser run. None was performed here.

## 5. Likeness models

### 5.1 Comparison

Prices are as stated on the fal model pages fetched on 2026-10-17 unless marked otherwise. Latency and likeness quality are not published by fal and are not measured here.

| Option | Base and kind | fal endpoint and inputs | Price (source) | Latency | Likeness evidence | Race and species | Licence and policy notes | Verdict |
|---|---|---|---|---|---|---|---|---|
| PuLID Flux | FLUX.1 identity embedding; text to image with a face reference | `fal-ai/flux-pulid`; `prompt`, `reference_image_url` (required); `image_size`, `num_inference_steps` (default 20), `guidance_scale` (4), `true_cfg` (1), `start_step` (0), `id_weight` (1, range 0 to 1), `negative_prompt`, `enable_safety_checker` (true), `sync_mode`, `seed`. [fal page](https://fal.ai/models/fal-ai/flux-pulid), [fal schema](https://fal.ai/api/openapi/queue/openapi.json?endpoint_id=fal-ai/flux-pulid) | $0.0333 per megapixel, rounded up to the next megapixel (fal page). About $0.033 at 768 by 1024; $0.067 at 1024 by 1024 | fal sample reports `timings.inference` 4.25 (unit not stated; probably seconds) | Designed for identity preservation. Not measured here | Strong facial geometry is kept, so non-human features (pointed ears, dwarf brow) will fight the likeness. See 5.2 | Fal page tags "Inference" and "Commercial use". Upstream licence not checked | Primary |
| FLUX Kontext Max multi | FLUX Kontext edit with several images | `fal-ai/flux-pro/kontext/max/multi`; `prompt`, `image_urls` (required, no maximum stated); `aspect_ratio`, `guidance_scale` (3.5), `output_format` (jpeg), `safety_tolerance` ("2"), `num_images`, `sync_mode`. [fal page](https://fal.ai/models/fal-ai/flux-pro/kontext/max/multi), [fal schema](https://fal.ai/api/openapi/queue/openapi.json?endpoint_id=fal-ai/flux-pro/kontext/max/multi) | $0.08 per image (fal page). Whether input images are billed is not stated | Not published | Edits the photo; no face embedding, so it can copy the photo's pose, clothes and background. Needs a style reference to restage | Edit model, so it can change costume; can drift on face | Already wired (`FLUX_KONTEXT_MULTI` in `crates/df-providers/src/fal_image.rs`). Also used by plates | Fallback, and the second pass for plates |
| InstantCharacter | FLUX-based character model (third-party description) | Not verified against fal's schema | Third-party tracker: $0.105 per run (secondary) | Not published | Not evaluated | Not evaluated | Not checked | Test only if PuLID fails the likeness check |
| IP-Adapter Face ID | Stable Diffusion family adapter | `fal-ai/ip-adapter-face-id` | Fal page shows "Inference, research only" and $0 per second as a placeholder | Not published | Not evaluated | Not evaluated | Research-only on the fal page; not FLUX | Excluded |
| InstantID, PhotoMaker | Stable Diffusion XL family face models | Not verified | Not found | Not found | Not evaluated | Not evaluated | Not FLUX | Excluded by the owner's FLUX-only rule |
| Per-player LoRA or DreamBooth training | FLUX training per person | Not evaluated | Training and serving cost, not priced | Minutes to hours | Usually strong | Good with training data | Biometric model kept per person; retention burden | Excluded for v1: slow, costly, and creates a stored face model |
| OpenAI image edit | Non-FLUX edit model already in `imagery.rs` | Not evaluated | Not evaluated | Not evaluated | Not evaluated | Not evaluated | Not FLUX | Not evaluated; the owner's rule points away from it for photographs |

The owner's rule is recorded in the module comment of `crates/df-providers/src/fal_image.rs`: "owner's choice, 2026-10-17: only FLUX read as a real photograph". The table follows that rule.

### 5.2 Race and species

- An identity model keeps the facial geometry of the photo. A dwarf or an elf with the player's face is not reachable through identity weight alone.
- v1 proposal: the likeness is offered only for human heroes. Other species get the standard art, or a costume-only portrait that says it uses no likeness. Species and prosthetics ("pointed prosthetic ears") can be tried in the prompt, but this must be judged on real outputs before it is promised.
- Use `id_weight` between 0.6 and 1.0 for the human case; a lower weight keeps more of the house style but gives less likeness. The exact value is a tuning result from the test in 5.3.

### 5.3 Likeness test (not run)

- Subjects: consenting team members only, three to five people, both sexes, two skin tones at minimum, with glasses and without.
- Measure: a face-similarity score between the photo and each output (for example an ArcFace-style embedding cosine), a human ranking of likeness against the concept art, and the rate of outputs with extra faces or text.
- The similarity model is itself biometric processing. Run it offline on the consenting team's photos, store only the scores, and delete the photos after the test.

### 5.4 Cost and latency (planning numbers)

- PuLID at 768 by 1024: about $0.033 per portrait. Allowing two retakes per hero, three calls worst case: about $0.10 per hero. Four heroes: about $0.40 worst case.
- Kontext fallback: $0.08 per image, so $0.24 worst case per hero.
- Latency: use the deadline pattern in `fal_image.rs` (queue, status reads, one result read, no retry). The planning deadline for a portrait is 90 seconds, a proposal to confirm from measurements.

## 6. Privacy, consent and safety

### 6.1 Legal classification

- Under the GDPR, biometric data processed to identify a person is special-category data (Article 9(1)). Processing needs an exception; the one that fits is explicit consent (Article 9(2)(a)). The person can have it erased (Article 17). [GDPR Art. 9](https://gdpr-info.eu/art-9-gdpr/) The consent must be specific to this purpose and withdrawable.
- Illinois BIPA covers scans of face geometry. Federal decisions have held that BIPA reaches face geometry derived from photographs, not only in-person scans (Rivera v. Google, 2017; Sosa v. Onfido, 2022). [Mondaq summary](https://webiis10.mondaq.com/unitedstates/privacy-protection/1188272/breaking-federal-judge-in-illinois-affirms-that-bipa-extends-to-information-derived-from-photographs) A report of an October 2026 ruling in an Apple Photos case says cropped faces without enough geometry fall outside BIPA. [MLex summary](https://www.mlex.com/mlex/articles/2535955) I did not open the full opinion. The practical rule is the same either way: written consent, a published retention rule, and no use beyond the stated purpose. These are the BIPA statute's requirements as I know them; I did not re-check the text here.
- Players in other places (the game is not limited to Illinois) should get the same consent screen.

### 6.2 Consent screen (before the camera opens)

Show, in plain words, all of the following, and require an affirmative choice:

- What is made: a painted-or-photographic portrait of your hero from your photo; it is shown to the table on the TV and on phones.
- What is sent and where: one photo, sent once, to the image provider (fal.ai, a US company), for that one picture.
- What is kept: the approved portrait image; the photo is not kept; a consent record (time, version, seat, hero) is kept.
- How long: the provider's copy of the request is set to expire; the portrait stays until you remove it.
- How to stop: "Remove my likeness" on the hero card, which restores standard art and deletes the portrait and its derived images. Declining or not answering leaves character creation fully usable.
- The attestation: "This is my face. I am 18 or older." A checkbox, with the consent text version stored.
- A link to the full privacy notice, which must exist before launch (open question 3).

### 6.3 Minors

- The game is for adults (owner decision 2026-10-11, planning/gameplay-tuning.md section 11). The hero is an adult (`age_group` is `adult` in the backstory record).
- Do not estimate age from the face: that is another biometric analysis and it is not needed. Use the attestation and the account's age confirmation.
- A photo that appears to show a child is refused without analysis. The client can offer a "retake" but the server records no image.
- PLAY-P01 says the photo is "possibly of a minor". That is the reason for the attestation and the server-side refusal; it is not a reason to run age analysis.

### 6.4 Retention

| Data | Where | How long | Notes |
|---|---|---|---|
| Photo (JPEG) | Server memory only | One generation call; dropped on approval, rejection, timeout, or disconnect | Never written to disk, logs, database, telemetry or the dev corpus |
| Photo sent to fal | fal request payload | Set `X-Fal-Store-IO: 0` so request history is not stored; set `X-Fal-Object-Lifecycle-Preference` with `expiration_duration_seconds` 3600 for any CDN copy | Default without the header: 90 days of request history (fal data-retention page). Confirm with fal that inline data URIs do not create a CDN copy |
| Generated portrait | Hub resource, PostgreSQL row or file store | Until removed or the account is deleted | Deleted on withdrawal with its cut-out and any plate that used it |
| Consent record | PostgreSQL | Kept for the audit trail, minimum needed; the photo itself is not part of it | Stores consent version, seat, hero, decision, times |
| Telemetry | OTEL spans and logs | Outcome codes only | No image bytes, no face-derived values, no provider request bodies |

- Provider terms: fal's privacy policy (last updated 22 July 2026) says uploaded images are "input content", does not say whether inputs train models, does not mention biometric data, and says that enterprise data processed on a customer's behalf is outside the policy. [fal privacy policy](https://fal.ai/privacy) The game therefore needs a signed enterprise data agreement with fal before any live likeness call, and a written answer on training and on CDN copies. [fal data retention](https://fal.ai/docs/model-apis/data-retention) says input CDN files can be deleted individually, and that request payload deletion does not remove input files.
- The data-retention page says request payloads are kept 90 days by default. A search summary said 30 days; the fetched page is the source used here.

### 6.5 Deletion and withdrawal

- Withdrawal flow: consent record marked withdrawn; the likeness resource and its outputs are invalidated by subject (`character:<hero>`) through the hub's invalidation; the hero's portrait reverts to standard art; the TV and phones update through the normal view revision; plates that used the likeness are regenerated or marked stale.
- Provider copies: delete any fal CDN files that were created (the input-file deletion endpoint), and rely on `X-Fal-Store-IO: 0` for request history.
- Each withdrawal writes an audit line with the consent identifier and no image data.

### 6.6 Abuse: a photo of someone else

- One likeness per hero, bound to the seat that gave the attestation. The likeness cannot be given to another hero or NPC, or shown to other players beyond the portrait the table already sees (PLAY-P01 acceptance).
- Single face: the server refuses a photo with no face or more than one face. The face check needs a detector; the cheapest route is the client detector, checked again on the server. Nothing from the check is stored.
- No sexual content with a likeness hero (existing rule: `based_on_real_person` sets `sexual_content_allowed = false`, character-backstory-creation.md 7.6). Graphic violence is allowed under the mature rating for fiction, but a likeness hero must not be shown in sexual, hateful or humiliating scenes. This needs a rule in the plate prompt, not only in the backstory.
- Provider safety: `enable_safety_checker` true; reject results with `has_nsfw_concepts` true; a provider refusal is a typed failure that falls back to standard art.
- Rate limit: one pending likeness per seat; a bounded number of regenerations per hero, recorded in the ledger (no cap on spend, per the ledger rule).
- Logs: no face text, no hero backstory, no name in the prompt sent to the provider.

### 6.7 Local-only option

- Running an identity model on the player's device would keep the photo off any provider, but no suitable on-device FLUX identity model is in the project, and the X2 NPU work targets other models. Not recommended for v1.
- The honest local option is "no likeness" (standard art). Say so on the consent screen as the alternative.

### 6.8 Gaps before launch

- A signed data processing agreement with fal (or a different provider) covering biometric input.
- Confirmation from fal that generating an image of the requesting person from their own photo is within its acceptable use policy. Not checked here.
- A published retention notice and a withdrawal route that a player can use without the dev CLI.

## 7. Integration design

### 7.1 Where it fits in hero creation

- `planning/character-backstory-creation.md` already has a portrait choice group in creation (the accepted quickstart portraits, `portrait-hero-1`, `portrait-hero-2`, in `crates/df-tools/src/gameplay/dev_control/actions.rs`). Add one option, "Use my face", to that group. It appears only when the consent record is current and a likeness is approved.
- Flow: attestation and consent, then capture (camera or file), then the client downsizes and uploads (section 4), then the server queues generation, then the player sees the result with Approve, Retake and Reject, then Approve sets the hero's portrait to the likeness. The photo is dropped at each end of the flow.
- Creation is not blocked. Refusal, no camera, a bad photo, a provider failure or a timeout keep the standard portrait.
- The TV seat card shows "working out who they were" while generation runs, as the doc already specifies. It never shows the photo.

### 7.2 Resource hub spec and key

- New `ResourceKind::Likeness` (label `likeness`), `CONTRACT_VERSION` 1, in `crates/df-tools/src/gameplay/resources.rs`. A new kind keeps removal and retention separate from depictions.
- Spec (sketch):

```text
LikenessPortrait {
  hero:           "hero:<id>"          // subject is character:<id>
  consent_id:     random per capture   // never a hash of the photo
  species:        server vocabulary    // must be human in v1
  prompt_version: u32
  size:           768x1024
  seed:           recorded in the row, not in the key
}
canonical() = compact JSON of the fields above. The photo bytes and any face-derived value are excluded.
subject()   = "character:<hero id>"
chrome()    = the standard portrait chrome (neutral) until ready
```

- Because the key uses a random `consent_id`, two captures of the same face produce two keys, and a withdrawn consent cannot be served from cache.
- The photo is passed to the provider call as the reference input, not as part of the spec. The spend ledger records the call as usual.

### 7.3 Plate pipeline

- Plates already take an identity sheet and a speaker portrait as numbered references (`crates/df-tools/src/gameplay/imagery/cast_sheet.rs`, `plate_prompt.rs`, `living_scenes/plates.rs`). The cast sheet's legend includes source digests, so changing a likeness invalidates the plate request.
- The likeness enters through `identity_photo` (asset-engine.md section on identity keys): the hero's identity key maps to the approved likeness output when present. The `ReferenceStore` stays unchanged. A small `LikenessSource` loads the approved image only when the consent record is current.
- Prompt: for a likeness figure, `NO_REAL_PEOPLE` is replaced with "The face is the player's own, consented; the character is fictional." Everything else in `FILM_LOOK` stays.
- Plate rules: a likeness figure is never placed in sexual content; the existing `based_on_real_person` rule applies.

### 7.4 Storage

- PostgreSQL: a `likeness_consent` row (id, seat, hero, consent text version, decision, times, withdrawn_at) and the hub row for the portrait. No photo, no photo hash, no embedding.
- The approved portrait image is an ordinary hub resource file, served through the manifest and the asset rules in `planning/rpc-api.md` ("Runtime portraits ... use the same manifest/access system"). Access is limited to the seat and the table's public view.

### 7.5 Dev CLI

Every system must be reachable from the dev CLI (memory rule and `dev_cli.rs` pattern). Proposed group, mirroring `dev art`:

- `df-transport-fixture dev likeness show <hero>`: state (none, pending, ready, failed), consent id and version, resource key, provider label, spend in the ledger. No image bytes.
- `dev likeness set <hero> --photo <file> [--consent-version v1]`: test-only. Requires `DF_GAMEPLAY_DEV_CONTROL=on` and the loopback token. The CLI reads the file the developer names, streams it through the same `UploadPhoto` path, and does not copy it. Use only consenting staff photos or licensed fixtures.
- `dev likeness approve <hero>` and `dev likeness reject <hero>`: stand in for the player's approval.
- `dev likeness clear <hero>`: the withdrawal path (section 6.5).
- `dev likeness regenerate <hero>`: a new call; spend is recorded in the ledger, with no cap.
- Routes follow the existing pattern: `GET /dev/likeness?hero=`, `POST /dev/likeness/edit` (`op`: set, approve, reject, clear, regenerate), with `/dev/likeness` for the upload path on loopback only.

### 7.6 Observability

- Spans and logs carry the hero id, the consent id, the state and the duration. Never the bytes, the prompt, or the provider request body.
- The request-duration metric is the planning input for the 90-second deadline.

## 8. Prototype notes

Location: `artifacts/tmp/face-portrait-prototype/` (standalone Cargo project, `[workspace]` empty so it is not a workspace member; `Cargo.lock` copied from the workspace so the web-sys 0.3.106 and wasm-bindgen 0.2.129 versions match the game).

- `src/lib.rs`: `open_camera` (getUserMedia with `facingMode` ideal user), `play_stream` (muted, autoplay, playsinline), `jpeg_from_camera` (video to canvas to JPEG, stop tracks always), `jpeg_from_file_picker` (synchronous input click, change and cancel handling, then ImageBitmap to canvas to JPEG), `jpeg_from_blob`, `fit_within`, `chunk_plan`, and two exports `capture_likeness_from_camera` and `capture_likeness_from_file` returning `Uint8Array`.
- Checks run: `cargo check` and `cargo build --target wasm32-unknown-unknown --offline` pass with the toolchain 1.98.1 already in `artifacts/cache/rustup`. Host unit tests: 2 pass (`fit_within` aspect and bounds, `chunk_plan` coverage). rustfmt applied with the root `rustfmt.toml`. No CRLF in the source.
- Build command used (Git Bash, low priority):

```text
export RUSTUP_HOME=.../artifacts/cache/rustup CARGO_HOME=.../artifacts/cache/cargo CARGO_TARGET_DIR=.../artifacts/build/haiku-c
nice -n 19 cargo build --target wasm32-unknown-unknown --offline
```

- Not done: no browser run, no real camera, no iOS or Android test, no preview UI, no EXIF check, no measurement of encode time or memory. Blob `arrayBuffer()` is used; its support on the target iOS version is to be confirmed.
- `fal-pulid-request.example.json`: the request sketch for `fal-ai/flux-pulid`, with the headers (`X-Fal-Store-IO: 0`, `X-Fal-Object-Lifecycle-Preference`), the billing note, the likeness rules, and a Kontext fallback body. The photo is a placeholder and no request was sent.
- The prompt keeps the FILM_LOOK wording (photographic, no painting, no CGI, adult) and replaces the "no real person" line with the consented-likeness line.

## 9. Open questions

1. PLAY-P01 art style: photographic (recommended here) or painted? The text and the direction disagree.
2. Confirm that spend has no cap and remove "exhausted spend cap" from PLAY-P01.
3. Privacy notice and consent text: who approves the wording, and what is the retention period for the portrait after the player leaves the campaign (O3 in character-backstory-creation.md says heroes carry across campaigns)?
4. fal: does an inline data URI create a CDN copy during processing? What does the enterprise agreement say about training and biometric inputs? Is self-likeness generation within the acceptable use policy?
5. Does `X-Fal-Store-IO: 0` apply to every call on the account, and should the adapter set it for all person references?
6. PuLID: does `id_weight` above 1 work on fal even though the schema caps it at 1? What is the real `start_step` range?
7. Species: is the likeness human-only in v1? What happens to a player who wants an elf with their face?
8. Does the TV show the likeness only in the hero's card and figure, or also in the plates? (Plates reuse the identity; the plate prompt needs the rule in 7.3.)
9. iOS: does `createImageBitmap` apply EXIF orientation on the target iOS version? Does a HEIC photo from the picker decode?
10. Latency: is 90 seconds the right deadline? Measure against the phone waiting screen.
11. Should the consent check also run at the server for the file route, before the upload starts?
12. Do we need a non-biometric fallback that makes a "stylised avatar" from a selfie without face geometry? Not evaluated.

## 10. Sources

Web (fetched or searched on 2026-10-17; "search summary" means the result snippet was not opened):

- fal PuLID Flux model page, fetched: https://fal.ai/models/fal-ai/flux-pulid
- fal PuLID Flux request schema, fetched: https://fal.ai/api/openapi/queue/openapi.json?endpoint_id=fal-ai/flux-pulid
- fal FLUX Kontext Max multi model page, fetched: https://fal.ai/models/fal-ai/flux-pro/kontext/max/multi
- fal FLUX Kontext Max multi request schema, fetched: https://fal.ai/api/openapi/queue/openapi.json?endpoint_id=fal-ai/flux-pro/kontext/max/multi
- fal data retention, fetched: https://fal.ai/docs/model-apis/data-retention
- fal privacy policy (last updated 22 July 2026), fetched: https://fal.ai/privacy
- fal pricing aggregator (secondary, search summary): https://pricepertoken.com/fal-ai-pricing
- Instant Character and IP-Adapter Face ID pricing and status (secondary, search summary): https://pricepertoken.com/fal-ai-pricing and https://fal.ai/models/fal-ai/ip-adapter-face-id/playground
- PuLID upstream parameter notes (secondary, search summary): https://replicate.com/bytedance/flux-pulid/examples
- MDN getUserMedia, fetched: https://developer.mozilla.org/en-US/docs/Web/API/navigator/mediaDevices.getUserMedia
- MDN facingMode, fetched: https://developer.mozilla.org/en-US/docs/Web/API/MediaTrackConstraints/facingMode
- MDN input type file (capture attribute), fetched: https://developer.mozilla.org/en-US/docs/Web/HTML/Element/input/file
- WebKit, new video policies for iOS (search summary): https://webkit.org/blog/6784/
- GDPR Article 9, fetched: https://gdpr-info.eu/art-9-gdpr/
- Sosa v. Onfido and Rivera v. Google BIPA summary (search summary): https://webiis10.mondaq.com/unitedstates/privacy-protection/1188272/breaking-federal-judge-in-illinois-affirms-that-bipa-extends-to-information-derived-from-photographs
- Apple Photos BIPA ruling report, October 2026 (search summary, not opened): https://www.mlex.com/mlex/articles/2535955

Repository (read on 2026-10-17):

- `AGENTS.md`; `planning/client-architecture.md`; `planning/rpc-transport.md`; `planning/rpc-api.md` (Voice and asset streams; Upload telemetry); `planning/asset-engine.md`; `planning/character-backstory-creation.md` (sections 6.1, 7.6, 9, O3, O5); `planning/gameplay-tuning.md` (section 11); `planning/adventure-generator.md` (section 21).
- `development/operational-tasks.json`: PLAY-P01 (and PLAY-L01 referenced by it).
- `crates/df-rpc-bridge/src/lib.rs` (frame, message, receive and RPC limits); `crates/df-rpc-bridge/src/resources.rs` (window and buffer rules).
- `crates/df-tools/src/gameplay_browser.rs` (around line 988: the 8 KiB comment and `RPC_MESSAGE_BYTES` use).
- `crates/df-tools/src/gameplay/imagery/prompt.rs` (`FILM_LOOK`, `NO_REAL_PEOPLE`, `ADULTS_ONLY`); `crates/df-tools/src/gameplay/imagery/references.rs` (`ReferenceStore`, `NotPhotographic`); `crates/df-tools/src/gameplay/imagery/cast_sheet.rs`; `crates/df-tools/src/gameplay/imagery/photo_edit.rs`; `crates/df-tools/src/gameplay/imagery.rs` (image routes, `ImageEditModel::OpenAi`).
- `crates/df-tools/src/gameplay/resources.rs` (`ResourceKind`, `ResourceSpec`, `ResourceKey`, `SpendLedger` no-cap rule).
- `crates/df-providers/src/fal_image.rs` (`FLUX_ULTRA`, `FLUX_KONTEXT_MULTI`, `kontext_body`, owner's FLUX-only comment dated 2026-10-17).
- `crates/df-tools/src/dev_cli.rs` and `crates/df-tools/src/gameplay/dev_control/mod.rs`, `art.rs` (dev command and route pattern).
- `crates/df-tools/src/gameplay/hero_identity.rs`, `crates/df-tools/src/gameplay/dev_control/actions.rs` (portrait group in creation).
- `artifacts/tmp/face-portrait-prototype/` (prototype and request sketch).
