# Predictive asset engine

Date: 2026-09-30
Status: User asset outline adopted; provider selection and measured bounds pending

`df-media` is the separate Asset Engine crate; extend its existing media boundary
with `AssetEngine` orchestration rather than add a competing generation owner.
`df-presentation` requests audience-safe semantic media; `df-ai` realizes approved
text and semantics; existing provider ports/adapters and `df-assets` supply generation
and durable bytes. [Runtime directors](runtime-directors.md) defines authority.
Supplier names/features/rates in the pasted outline are research leads, not frozen
contracts; current provider research and G08 decide routing/actual capabilities.

## Control plane and latency classes

A `NarrativeMoment` is a structured committed event/fact reference with location,
permitted characters, reveal scope, semantic mood/focus/importance and continuity
basis. It drives text/performance/music/visual requests without each generator
independently reinventing the story. A candidate moment remains tentative until
engine validation/commit; generated media cannot canonize dice, world truth or
player commitments. Keep speculative futures server-only, including asset indexes.

| Class | Initial delivery policy |
| --- | --- |
| Live input | streaming STT/VAD with explicit capture lease/format; tentative partial classification, final validated intent before actions |
| Live validated expression | narration/NPC text and streaming TTS; stable approved clauses only after the incremental-publication safety gate, otherwise complete validated text |
| Immediate reused sound | prepared SFX and campaign/NPC/faction music themes/stems, compatible mixer/timebase metadata; ordinary hits do not wait for paid calls |
| Asynchronous identity assets | portraits, items, location stills/backgrounds and reference packs; canonical reuse, prepared placeholder followed by matching ready asset |
| Prefetched/milestone cinema | bounded short shot plans/loops; strict campaign video permission/reservation; no required arbitrary live multi-minute generation |
| Interactive world/tactical rendering | server geometry plus cached/procedural-approved assets; optional 3D only after working flat/still/audio experience |

STT partials can prime bounded cheap/cancellable candidate work; they cannot
execute a predicted action or irreversibly spend an unapproved speculative budget.
The final transcript is validated by `df-intent` and current state. Streaming text
to TTS/captions requires a stable clause identified by sequence/hash, fully checked
against permitted facts, source/outcome/secret policy and a no-later-retraction
contract; unsafe partials remain buffered. Record emitted clauses and actual speech
for replay; model tokens are not direct public output. If the proof fails, use full
validated responses while optimizing latency through other measured paths.

Barge-in uses the authenticated capture/binding lease and a supported explicit
utterance-interruption event routed through the session. The server validates
current conversation/cue identity and issues timed fade/cancel before updating NPC
interaction under approved policy. Client-local mute or stopping audio is never
proof of an NPC interruption or an automatic relationship penalty. Lost/cancelled
unaccepted capture cleans up; accepted run-owned work follows normal job fencing.

## Models, public operations and lifecycle

`df-model` owns `NarrativeMoment`, `AssetDemand`, `AssetRequestKey`, `AssetJobState`,
`AssetDependency`, `CanonicalPack`, `VisualBible`, `EntityIdentityRevision`,
`ShotPlan`, `PerformanceHint`, `DemandPriority` and `PrefetchPolicy` persisted
shapes. `df-media` owns runtime admission/queue/request/errors; `df-assets` owns
complete immutable byte/metadata access. `VisualBible` and canonical NPC/location/
item packs pin art palette/style/appearance, voice and reference hashes/versions.
Future updates create a new identity revision, preserving old referenced assets.
Art is an appearance reference; it never changes item statistics or actual geometry.

Planned Rust-facing operations are:

```text
AssetEngine.admit(JobContext, AssetDemand, BudgetGrant) -> AssetAdmission
AssetEngine.status(JobContext, AssetJobId) -> AssetStatus
AssetEngine.cancel(JobContext, AssetJobId, CancelReason) -> CancelOutcome
AssetEngine.complete(JobContext, GeneratedCandidate) -> ValidatedAssetResult
AssetEngine.reconcile(JobContext, UsageEvidence) -> ReconciliationOutcome
```

Context includes run/generation, source/policy/identity revision, audience and
admitted execution mode/deadline. Admission returns existing identical job/ready
asset, queued reservation, deferred capacity, denied budget or typed unsupported
capability. Runtime queue transitions produce results through the session owner;
only committed demand creates accepted run-owned work. No direct authoritative DB
write from an asset queue callback. Native effect delivery is at least once, so
claim/result/paid-call identities and unknown-outcome reconciliation are required.

Logical lifecycle is Missing -> Queued -> Generating -> Ready, with canonical-pack
selection an explicit committed reference change. Failed/Cancelled/Stale/Superseded
are explicit outcomes. Ready means complete verified bytes and successful metadata
publication; partial files are not cache hits. Superseded ready bytes may remain
valid for old references. Invalidating a style/reference dependency marks affected
future demand stale without deleting historical/referenced assets or restarting a
paid job automatically. Dependency DAG validation rejects cycles/missing roots;
keys include schema, source/moment/identity/style/voice, provider/model/format,
reference hashes, audience and relevant parameter revisions.

## Prediction, fairness and spend

Predictive candidates rank likelihood-of-use heuristic, importance, urgency,
reuse and estimated cost. Sample decimal scores are not calibrated probabilities.
Queues distinguish interaction-critical, soon-likely and optional demand with
fairness/aging and per-campaign/provider concurrency, bytes, time and spend bounds.
Speech/input capacity remains available while optional cinema runs. Forecast
horizon/top-K/branch count/expiry are explicit policy; do not generate every future.
Prepare reusable references/themes once, select cache hits and record hit rate,
lead time, used-versus-abandoned assets and speculative waste by version.

Optional prefetched image/video requires explicit campaign allowance and a budget
reservation through `BudgetStore`; provider dispatch checks the admitted mode and
wallet. Never refund a spent/unknown call merely because the asset expired or was
unused. Reconcile actual usage, cancellation fees and unknown supplier outcomes;
reserve conservatively rather than auto-repeat an ambiguous paid call. Retire
unstarted demand when obsolete; accepted job cancellation does not guarantee a
supplier refund. No predictable cost means no silent spend. Prepared-only/replay
misses are typed unavailable and cannot go live.

## Requested bookends and identity evolution

[Campaign cinematics](campaign-cinematics.md) defines F46 recap/trailer jobs,
committed critical-event eligibility and scene/item identity revisions. Ready
canonical assets and prepared stills/narration/captions are the ordinary path;
optional live video requires a separate quote/cap. Audience/source/access versions
filter fact selection and reference inputs before provider dispatch, manifests or
export. Speculative trailers never establish future facts or expose secret branches.
[Generated content](generated-content.md) admits item/enemy/spell mechanics before
cosmetic generation; failed paid imagery cannot revoke due source rewards. All
these jobs use existing demand/budget/publication/cancellation/replay semantics.

## Nonblocking presentation and acceptance

Presentation chooses the best currently permitted available representation:
ready authorized video; matching still with bounded pan/zoom; prepared illustration/
flat scene; validated narration/captions. Optional usable 3D may improve this path,
but is never prerequisite for a cheaper fallback. Failed/late assets keep game
progress/input available and expose status. A late clip has expiry/resume/replacement
policy and cannot replay a finished milestone, expose an unrealized branch or
change an authoritative result. Semantic generation context and manifests are
filtered before delivery; never send secret future assets for clients to hide.

F15/F16/F17/F25–F28/F44 and G08/G10/G12 require canonical identity continuity,
DAG invalidation, complete publication, stale/duplicate/fenced jobs, fair queue,
cache/prepared/replay misses, actual spend/waste, clause safety, barge-in policy,
late assets and no-block fallback. Benchmark speech first-understood-intent/
first-approved-audio latency, deadline misses, prefetch hit/waste rate, bytes and
cost across actual campaign capacity/device targets. Frontier browser/vision/audio
review checks the running result. No provider feature, quality or profitable
capacity claim is accepted from the pasted vendor examples alone.

## Service spend and income refinement

[Commerce service](commerce-service.md) supplies the single exact hierarchical
platform/supplier/payer/campaign/job spend and concurrency authority, including
current entitlements, reservations and unresolved liabilities. Every hedged branch
requires its own admitted reservation under the same global cap; cache/prepared
modes reserve no new supplier spend. [Commercial validation](commercial-validation.md)
and the reproducible service-economics model separate net earned service revenue,
credit cash/liability, support, rights, acquisition/churn and fixed operating costs.
The earlier small-server/$2 support contribution illustration is not company profit
or proven capacity. [Service operations](service-operations.md) supplies finite
proposed workload and recovery targets. Paid launch requires the observed evidence,
not just source-backed planning parity.

## Runtime scene imagery provider (PLAY-L01, researched and measured 2026-10-11)

Living scenes (`gameplay/imagery.rs`, `gameplay/living_scenes.rs`) re-render the shared
display's backdrop and the character in focus from server story state. They are
cosmetic and never change an outcome or a timing ([Campaign cinematics](campaign-cinematics.md)).
Generated runtime images are written only to the run's cache directory
(`DF_GAMEPLAY_IMAGERY_CACHE`, default `artifacts/tmp/play/imagery-cache`) and a run-local
provenance manifest (`imagery.jsonl`: key, kind, model, prompt, reference hashes, time,
bytes, measured cost); never under `assets/`, never committed.

What the candidates offer, from the vendors' own documentation read on 2026-10-11:

| Candidate | Multi-reference edit | Transparent output | Published price | Moderation controls |
| --- | --- | --- | --- | --- |
| OpenAI `gpt-image-2.5-flare` / `-sunburst` (`/v1/images/edits`) | `image[]`, several files ([guide](https://developers.openai.com/api/docs/guides/image-generation)) | `background: transparent`, PNG or WebP | tokens: 5 USD text in, 8 image in, 30 out per million ([pricing](https://developers.openai.com/api/docs/pricing), [model](https://developers.openai.com/api/docs/models/gpt-image-2.5-flare)) | `moderation: auto` or `low`; blocked calls return `moderation_blocked` and must not be retried unchanged |
| fal `fal-ai/nano-banana/edit` | `image_urls` list; data-URI input accepted on the newer model page ([schema](https://fal.ai/models/fal-ai/nano-banana/edit/llms.txt)) | none (needs a background-removal call, about 0.018 USD) | 0.039 USD per image | `safety_tolerance` 1 to 6 is only fal's layer; Google's model-level blocks on sexually explicit and violent content stay ([Google](https://docs.cloud.google.com/vertex-ai/generative-ai/docs/multimodal/gemini-image-responsible-ai)) |
| fal `flux-kontext/dev`, `qwen-image-edit-plus`, `bytedance/seedream/v4/edit` | single image (Kontext), several (Qwen, up to 10 for Seedream) ([Kontext](https://fal.ai/models/fal-ai/flux-kontext/dev/llms.txt), [Qwen](https://fal.ai/models/fal-ai/qwen-image-edit-plus/llms.txt), [Seedream](https://fal.ai/models/fal-ai/bytedance/seedream/v4/edit/llms.txt)) | none | 0.025 to 0.03 USD per megapixel or image | `enable_safety_checker` can be switched off only with account authorization; unauthorized requests are always checked and flagged output comes back black |

Decision: **OpenAI `gpt-image-2.5-flare`, medium quality, 1024x1536 transparent PNG for a
cut-out and 1536x864 JPEG for a backdrop** (`gpt-image-2.5-sunburst`, the "editing precision"
model, is the one other validated model id). Reasons: `FAL_KEY` is empty in this machine's
`.env`, so no fal model could be called at all; of the OpenAI models it is the fast one; it
returns real alpha in one call (no background-removal step, no second charge); and the same
person, costume and pose came back in every measured edit. fal remains a documented option
only (a second adapter would reuse `HttpRequest` and `NativeHttp` and add a removal step); it is
not wired, and no refusal is ever rerouted to another model or reworded.

Measured through the shipped adapter, 29 real calls (27 results kept): cut-out edit 10 to 15 s
and 0.018 USD (about 340 output tokens), backdrop or close-up 10 to 16 s and 0.022 USD, with one
54 s outlier (a mature interrogation backdrop; it would have missed the default 45 s deadline
and kept the authored painting). Total spend 0.55 USD.

What mature mode can and cannot show with this provider (`DF_GAMEPLAY_CONTENT_RATING=mature`,
the default, asks for `moderation: low` and removes the softening clauses; `restrained` asks for
`auto` and adds no gore, no nudity and no sexual content):

- Renders: a bloodied, bleeding, furious antagonist; an interrogation room dressed for cruelty
  (restraints, tools, blood on the floor); anger, fear, contempt, pain, a sultry look.
- Not tried and not promised: explicit nudity or sex acts. OpenAI's usage policies still apply
  to every call whatever the `moderation` setting ([usage policies](https://openai.com/policies/usage-policies/));
  a refusal is a typed failure and the screen keeps what it shows. Sexual content involving
  anyone depicted as under 18, and explicit content from a real person's likeness, are
  excluded by the server in both ratings (every character is an adult; a character whose art
  derives from a real person never gets the seductive variant; the request flag is false today).
- fal's nano-banana would block explicit and graphic content at the model level whatever the
  tolerance; its open-weight alternatives need account authorization to disable their checker.
  So no candidate makes "no censorship" unconditional; the provider's policy is the real floor.

Settings (all validated at startup; the startup line is
`IMAGERY on model=<id> budget=<n> images interval=<s>s ...` or `IMAGERY off (<reason>)`):
`DF_GAMEPLAY_IMAGERY` on/off, `_MODEL`, `_QUALITY` low/medium/high, `_BUDGET` (calls per server
run, default 40, charged before each call and never refunded), `_INTERVAL_SECONDS` (default 30,
minimum 20), `_DEADLINE_SECONDS` (default 45), `_CACHE`, plus the shared
`DF_GAMEPLAY_CONTENT_RATING`. Expected cost of a 30-minute session at the defaults is at most
40 calls, about 0.85 USD, and typically 15 to 25 calls (a repeated character and expression
pair, and a repeated backdrop request, come from the cache for nothing), about 0.3 to 0.5 USD.

## PLAY-H01: health-tier character art (2026-10-12)

One tier definition (`df-tools/src/health_tier.rs`: Healthy over 75%, Hurt over 50%, Bloodied over
25%, Critical above 0, Down at 0 or unconscious) feeds the server's art pipeline and both
clients. The art pipeline makes one cut-out per character and tier below Healthy, from the
character's full-body figure through the same image service, with closed-vocabulary phrases
(`health_face_phrase`, `health_look_phrase`: mature adds blood, bruising and torn gear, restrained
dirt, sweat and bruising). Health outranks mood at Bloodied or worse; above it the mood face wins.

- Warm-up is background work in tier order Bloodied, Critical, Hurt, Down, one request at a time,
  heroes, then the stage, then the adventure's fight cast, never during a fight, and never when
  only the 10-call reserve is left; the tier a character is in right now is requested first. A
  fight reads the cache only ("peek") and shows the nearest cached tier or the authored art.
  A failed request is retried once after everything else.
- Measured through the shipped adapter (16 variants for two heroes and two cast members): 13 to
  15 s and 0.017 USD each, 339 KB average as WebP with alpha (a PNG of the same cut-out was 2.7
  MB), so a two-hero, three-NPC adventure is 20 variants, about 0.34 USD and 5 minutes of
  background time. Three of the first sixteen requests (all Critical) failed once and succeeded
  when retried; the cause is not visible from outside the service (no failure is logged outside
  telemetry), which is why one retry exists.
- Portraits are crops of the full-body variant (head and shoulders by CSS background size and
  position), not separate generations: one request per tier instead of two, identity cannot drift
  between the portrait and the figure, and the crop read clearly at HUD size.
- Cache keys include the hash of the reference art and the prompt, so replacing the base art
  (for example a photographic re-shoot under the same file names) makes the old variants
  unreachable and the warm-up makes new ones by itself. The prompts still say "Painterly cinematic
  realism" (`HOUSE_STYLE`) and the backdrop prompts say "painting"; those two phrases need editing
  for a live-action look, and editing them also changes the keys, so everything regenerates.
