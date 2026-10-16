<!-- Saved for later at the owner's request; SAM integration is deferred. -->

# Segment Anything feasibility for scene coordinates

**Research checked:** October 15, 2026  
**Repository reviewed:** DungeonFlux at HEAD 942c2018, under <repository>  
**Scope:** Read-only code and official-source research, plus bounded hosted API probes. No repository code, browser, or server was changed. The authorized Meta probe listed SAM 3.1 but its segmentation request failed with a billing configuration error. Two authorized Fal queue submissions failed before returning an HTTP response. Sanitized records are in meta-sam-key-probe.json and fal-sam-queue-probe.json. No billing settings were changed.

## Recommendation

**Meta now offers an official hosted SAM 3.1 Model API.** The earlier draft’s claim that no official Meta inference API existed was wrong and has been removed. The API runs over HTTPS, so the first integration would not need a local model, Python, PyTorch, a GPU, or a sidecar. A native Rust server adapter is feasible. The custom special-token output format and lack of an official Rust parser make parsing the main implementation task. [Meta SAM API overview](https://dev.meta.ai/docs/sam/overview), [Meta SAM parser libraries](https://dev.meta.ai/docs/sam/client-libraries)

The live probe confirmed that the configured key could call the model-list endpoint and that SAM 3.1 was listed. Its one image segmentation request returned HTTP 402 with error code billing_not_configured after 1.268 seconds, without segmentation output. This demonstrates model visibility but not billable inference access for that key. Do not treat a Muse Code subscription as covering general API billing: Meta says its subscription-linked credential is for Muse Code CLI use; any additional API keys are billed pay-as-you-go. The probe used the process MODEL_API_KEY, and it did not establish that this was the same credential as the stored Muse subscription key. [Meta Muse Code subscriptions and key scope](https://dev.meta.ai/docs/muse-code/subscriptions)

If account billing is configured through an independently approved path, I recommend a small cached hosted-API pilot. SAM can find whole people and instances of a chosen object concept in a baked photographic plate. It cannot map a person to a named hero/NPC, judge which object is important to the story, or promise a head coordinate for speech bubbles. Keep the current head locator for head placement; use SAM as optional person/object geometry for overlay avoidance or as the selection boundary in a separate image-editing flow.

## What DungeonFlux already knows

- The head-location provider is in crates/df-providers/src/head_locate.rs. It locates speaker and hero head boxes in a baked plate.
- crates/df-tools/src/gameplay/living_scenes/plates.rs schedules measurement after plate generation and retains the predicted layout if measurement fails.
- The current PlateLayout protocol carries speaker and hero head boxes in permille of the source plate. DisplaySceneGraph also has anchors for layered actors. There is no prop/object geometry in that protocol today.
- crates/df-tools/src/plate_layout.rs maps source-image head boxes through the centered object-fit: cover crop before placing speech bubbles. New image detections must use that same source-to-screen mapping, including camera pan/zoom.
- For actors or elements whose location is already available from the scene graph or measured DOM layout, use that exact geometry. SAM only adds value when an item is baked into the image and its position is unknown.

These points are based on the current source and crates/df-protocol/proto/gameplay.proto. The applicable project plans are planning/client-architecture.md, planning/asset-engine.md, planning/observability.md, and planning/runtime-reliability.md. The full Rust server and thin Rust-WASM client arrangement favors calling the hosted API from the server and never placing its key in browser code.

## Current Meta SAM API

Meta’s current hosted model is SAM 3.1, released March 27, 2026. The Meta Model API documents POST https://api.meta.ai/v1/responses, model sam-3.1, and Bearer authentication using a MODEL_API_KEY. The API accepts an image plus a short text prompt and returns a box and mask for each matching object. [Meta SAM 3.1 announcement and date](https://ai.meta.com/blog/segment-anything-model-3/), [Meta SAM API overview](https://dev.meta.ai/docs/sam/overview)

**Prompt and output limits**

- The Meta Model API accepts text prompts only. Each request should name one concept with a short noun phrase, for example person, sword, or goblet. Multiple object types require separate requests.
- Open-weight SAM research code additionally supports visual box and point prompts. Those are not accepted by the Meta Model API; the API’s boxes are output geometry.
- The API locates the named concept. It does not reason about the scene or select the story’s “major” object. Long relational prompts such as “the second sword from the right, held by the innkeeper” are unsupported or only partially honored; the app can filter results by returned box coordinates.
- The API returns all matches as source-pixel boxes and masks. It does not document a confidence score in the response grammar.
- No match is a normal successful response with empty output.
- The same endpoint also accepts video and documents stable object IDs across frames within a response; it cautions against relying on IDs across entry/exit, full occlusion, or very long clips. [Meta video output semantics](https://dev.meta.ai/docs/sam/reading-segmentation)

[Meta prompt guide](https://dev.meta.ai/docs/sam/segmenting), [Meta response and coordinate format](https://dev.meta.ai/docs/sam/reading-segmentation)

The still-image output is not regular JSON records. Meta returns special-token text in the Responses output-text lane. Each record carries an object ordinal, a box token, and a mask token. Box coordinates and image width/height use source pixels. The mask is a base85 payload with its own height and width; it is smaller than the source frame, so code using mask pixels must map that raster to source-image coordinates. The documented Meta parser packages are Python and TypeScript. Meta explicitly permits a custom parser based on the published grammar, which makes a small bounded Rust parser an option consistent with this repository’s language architecture. A still-image request does not require streaming, so a non-streaming request may simplify the first native adapter. [Meta request and response](https://dev.meta.ai/docs/sam/overview), [Meta parser libraries](https://dev.meta.ai/docs/sam/client-libraries), [Meta parsing cookbook](https://dev.meta.ai/docs/cookbook/sam-api-basics)

The API accepts a public image URL or base64 data URL. Using a data URL avoids making the plate public, but the image still goes to Meta. Meta’s key guidance says not to expose API keys in browser/mobile code; calls and secrets belong on the server. [Meta image inputs](https://dev.meta.ai/docs/sam/segmenting), [Meta key guidance](https://dev.meta.ai/docs/authentication)

## Current access, price, and rate limits

Checked against Meta’s public product and pricing pages on October 15, 2026:

| Item | Current published information |
| --- | --- |
| Availability | Public preview, described as having expanded global access. An API key is created in the Model API dashboard. Those statements do not prove that every region/account is enabled. |
| Billing | Pay-as-you-go; Meta states no minimum or upfront commitment. |
| Still images | $2.50 per 1,000 images, or $0.0025 per image. Streaming and non-streaming have the same price. |
| Video | $0.20 per 1,000 frames. |
| Rate limit | Published SAM 3.1 limit: 3,000 requests per minute per team and no token-per-minute limit. Team keys share the limit. |
| Over limit | The API rejects requests. Meta recommends exponential backoff with jitter and honoring Retry-After. |

The published still-image price implies $0.01 for four separate concept prompts on one image, and about $10 for 1,000 plates with four prompts each, before other charges. This arithmetic assumes one image per request and follows Meta’s one-concept-per-request requirement. Caching by image bytes, model, and prompt set avoids repeat requests. Prices, limits, and account availability can change; check the dashboard and current pricing page before implementation. [Meta product page](https://dev.meta.ai/products/meta-model-api), [Meta pricing and rate limits](https://dev.meta.ai/docs/pricing-rate-limits)

Meta says Standard mode content is not used to train Meta models. That statement is not a guarantee about retention, processing geography, deletion, or contractual confidentiality; those were not verified here. Treat this as an external image processor and review the current API terms for the chosen account and use. Meta API credentials stay server-side. Keep raw images and prompts out of ordinary logs under the project’s observability design. [Meta Standard mode](https://dev.meta.ai/docs/pricing-rate-limits), [Meta key safety](https://dev.meta.ai/docs/authentication), planning/observability.md

## Other integration routes

| Route | Fit | Trade-offs |
| --- | --- | --- |
| **Meta Model API, SAM 3.1** | Official Meta endpoint, one concept per image, source-pixel boxes and masks, inexpensive published image price. Accepts a URL or base64 data URL. | Current probe reached the model-list endpoint, but the segmentation request failed with 402 billing_not_configured. Needs billable API access, external image processing, a server key, and a Rust parser for custom output text. Text prompts only. |
| **fal.ai hosted SAM 3.1 image endpoint** | Third-party hosted endpoint fal-ai/sam-3-1/image. Its schema supports text, point, and box prompts. JSON result includes mask image files; optional fields request scores and normalized center-x/center-y/width/height boxes. The model page showed $0.01 per request on the research date. | A separate provider, account, key, and image processor, not Meta’s API. Four concept queries would be $0.04 per plate at its displayed request price. I found no public RPM limit on the reviewed endpoint page. Confirm account limits and terms. The mask result URLs need bounded fetching and caching. The official queue POST route was attempted twice with the configured key; both failed at TLS transport before an HTTP response, so availability, billing acceptance, and segmentation output remain unverified. |
| **Self-hosted SAM 3.1 weights** | Keeps inference inside infrastructure you control; research code supports visual prompt workflows. | Official setup requires Python 3.12+, PyTorch 2.7+, and CUDA 12.6+ compatible GPU. Checkpoint download requires Hugging Face access approval. The repository uses Meta’s custom SAM License. This is a larger deployment and license review; hosted APIs avoid serving local weights. |

The fal endpoint page labels this model “Commercial use,” but that label is not a full terms review. Meta Model API terms and fal terms govern each hosted service; the open-weight SAM License concerns the self-hosted checkpoint path. Do not treat these licenses or terms as interchangeable. [fal SAM 3.1 endpoint schema and price](https://fal.ai/models/fal-ai/sam-3-1/image/api), [official SAM repository and setup](https://github.com/facebookresearch/sam3), [SAM License](https://github.com/facebookresearch/sam3/blob/main/LICENSE)

## Live Fal queue probe

The project’s existing native Fal adapter in `crates/df-providers/src/fal_media.rs` defaults to `https://queue.fal.run` and confines status/result URLs to the same queue host and request ID. I used the documented SAM endpoint at `https://queue.fal.run/fal-ai/sam-3-1/image`, with the authorized configured Fal key, `image_url` set to a data URI, and one text concept (`person`). Automatic redirects were disabled, no returned mask URL was fetched, and both attempts used the same generated plate. The first submitted the original 2560×1440 JPEG (499,615 bytes); the second submitted an in-memory 1536×864 JPEG (226,682 bytes) to check whether request size caused the transport failure. The second request body was 302,406 bytes.

Both POSTs ended locally with `URLError` and no HTTP status. The second exposed `SSLEOFError` (`EOF occurred in violation of protocol`, errno 8) after 1.368 seconds. A credential-free GET to a nonexistent status path on the same official queue host returned HTTP 405 in 332 ms, confirming the host was reachable but not proving POST or authentication worked. The earlier direct `fal.run` attempt also failed in transport. The sanitized two-attempt record is in [fal-sam-queue-probe.json](fal-sam-queue-probe.json). Fal publishes $0.01 per request; because the client received no response, it is unknown whether either POST reached the provider far enough to count as billable. Do not repeat requests without resolving the transport path and checking provider usage first. The input image hash was `9c042d6e569690cc0b1b4735c13488015304cd5a828a4715606fffc74a0c89f4`; the resized submission hash is recorded in the probe JSON.

For local-runtime estimates, the official repository states SAM 3 has 848 million parameters, Python/PyTorch/CUDA prerequisites, and a basic prompt-to-mask/box/score workflow. The setup page gives no minimum VRAM or CPU latency guarantee. Meta reports 30 ms for a SAM 3 image with more than 100 detections on an H200, and the SAM 3.1 release notes report 32 frames per second on one H100 for video with a medium number of objects. These are published Meta benchmarks, not latency predictions for DungeonFlux stills, a developer workstation, or the hosted API. The hosted path does not require those resources. [Official repository](https://github.com/facebookresearch/sam3), [Meta performance claim](https://ai.meta.com/blog/segment-anything-model-3/), [SAM 3.1 release notes](https://github.com/facebookresearch/sam3/blob/main/RELEASE_SAM3p1.md)

## Integration contract to preserve

1. Run only after the plate is available; make work optional, asynchronous, cached, bounded, and backed by the current layout fallback.
2. Cache with the exact plate hash/revision, provider/model version, and concept set. Associate results with the source asset and current scene/graph revision; discard stale results.
3. Store detections in intrinsic source-image coordinates. Preserve the source-pixel box and, if needed, mask dimensions, encoding, and binary raster. Do not invent a confidence value for Meta output that has no score field. Keep a concept match index separate from hero/NPC identity.
4. Project from source coordinates through the same cover crop, current camera transform, and viewport used by the scene. Evaluate wide and phone aspect ratios.
5. Keep API keys out of Rust-WASM. Bound image bytes, prompt count, response bytes, result count, timeout, and queue depth. Return typed provider, rate-limit, timeout, cancellation, and stale-result outcomes. Segmentation must never block a required game action or outcome.
6. For the first Meta still-image adapter, use non-streaming unless measurements justify streaming. Request one-bit mask encoding when masks are needed. Even when only boxes are consumed, the parser must safely validate or skip mandatory mask records.
7. Instrument safe identifiers, latency, result count, cache hit, and outcome through the shared observability path. Do not log image bytes or raw prompts.

A source-pixel box can support rough overlay placement; a mask supports pixel collision tests or object selection. Repositioning a painted object still needs background reconstruction/inpainting plus lighting, shadow, and occlusion repair. SAM does not create the repaired image. [Meta mask encoding and raster format](https://dev.meta.ai/docs/sam/reading-segmentation)

## Minimal evaluation before implementation

Use a fixed set of existing generated photographic medieval plates and manually annotate visible people and a small explicit list of scene-relevant object concepts. Compare the existing speaker/hero head locator, SAM’s person and selected-object prompts, and direct geometry for scene-graph/DOM elements already measured.

Measure people/item recall and box IoU at 0.5, mask IoU, false detections per image, normalized center error, caption overlap against annotations, and screen-projection error after cover/camera transforms. Record cold first-call and warm per-image latency, response size, cache hit rate, and actual spend. Set acceptance thresholds before reviewing results. Meta’s public benchmark and GPU timings do not establish performance on DungeonFlux art.

A minimal first pilot is one cached “person” query per completed plate; test object concepts on a labeled subset. This isolates whether person masks help caption avoidance before multiple queries are added. Do not retry billing or call the inference endpoint until a separately scoped task has an approved key, spend limit, and test asset set.

## Conclusion and unperformed checks

SAM is technically feasible as an image-space localizer from a native Rust server, with no local GPU deployment required. This session did not verify a working hosted inference route. Meta’s authorized key listed SAM 3.1, but its segmentation request returned HTTP 402 `billing_not_configured` and no output. Fal’s two queue POSTs ended in TLS transport errors without an HTTP response; a GET to the queue host succeeded with 405. The user’s Muse Code subscription should not be assumed to cover Meta’s API; Meta documents that other API keys use pay-as-you-go billing.

After approved billing is available, the integration is a moderate task: enforce one short text concept per call, parse the special-token output, preserve source coordinates, and handle image transfer and provider failures. SAM is not a replacement for the direct DOM/scene-graph coordinates or head locator. Fal.ai is a possible third-party alternative if billing/access or parser effort makes Meta’s endpoint unsuitable, at a higher posted cost and with separate provider terms.

No valid segmentation result, model inference latency benchmark, art-quality evaluation, coordinate-transform test, or local hardware benchmark was produced. The failed Fal POSTs may have reached the provider despite the missing client response, so their billing outcome is unknown; the maximum published-price exposure for the two submissions is $0.02. No weights were downloaded and no license conclusion was made. No devlog entry was added because the development SQLite append tool/table was unavailable for this research task.

