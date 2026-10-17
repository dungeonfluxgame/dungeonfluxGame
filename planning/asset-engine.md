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

What the candidates offer, from the vendors' own documentation read on 2026-10-16:

| Candidate | Multi-reference edit | Transparent output | Published price | Moderation controls |
| --- | --- | --- | --- | --- |
| OpenAI `gpt-image-2.5-flare` / `-sunburst` (`/v1/images/edits`) | `image[]`, several files ([guide](https://developers.openai.com/api/docs/guides/image-generation)) | `background: transparent`, PNG or WebP | tokens: 5 USD text in, 8 image in, 30 out per million ([pricing](https://developers.openai.com/api/docs/pricing), [model](https://developers.openai.com/api/docs/models/gpt-image-2.5-flare)) | `moderation: auto` or `low`; blocked calls return `moderation_blocked` and must not be retried unchanged |
| fal `fal-ai/nano-banana/edit` | `image_urls` list; data-URI input accepted on the newer model page ([schema](https://fal.ai/models/fal-ai/nano-banana/edit/llms.txt)) | none (needs a background-removal call, about 0.018 USD) | 0.039 USD per image | `safety_tolerance` 1 to 6 is only fal's layer; Google's model-level blocks on sexually explicit and violent content stay ([Google](https://docs.cloud.google.com/vertex-ai/generative-ai/docs/multimodal/gemini-image-responsible-ai)) |
| fal `flux-kontext/dev`, `qwen-image-edit-plus`, `bytedance/seedream/v4/edit` | single image (Kontext), several (Qwen, up to 10 for Seedream) ([Kontext](https://fal.ai/models/fal-ai/flux-kontext/dev/llms.txt), [Qwen](https://fal.ai/models/fal-ai/qwen-image-edit-plus/llms.txt), [Seedream](https://fal.ai/models/fal-ai/bytedance/seedream/v4/edit/llms.txt)) | none | 0.025 to 0.03 USD per megapixel or image | `enable_safety_checker` can be switched off only with account authorization; unauthorized requests are always checked and flagged output comes back black |

The OpenAI Images Edit reference documents a `prompt` maximum length of 32,000 characters
(retrieved 2026-10-16): <https://developers.openai.com/api/reference/resources/images/methods/edit>.
The adapter and gameplay prompt builders use that documented limit; they do not impose a smaller
development-time scene budget or remove cast and public-state context to fit one.

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
`IMAGERY on model=<id> interval=<s>s ...` or `IMAGERY off (<reason>)`):
`DF_GAMEPLAY_IMAGERY` on/off, `_MODEL`, `_QUALITY` low/medium/high, `_INTERVAL_SECONDS` (default 30,
minimum 20), `_DEADLINE_SECONDS` (default 45), `_CACHE`, plus the shared
`DF_GAMEPLAY_CONTENT_RATING`. There is no spend cap of any kind (owner decision 2026-10-17): the
`DF_GAMEPLAY_IMAGERY_BUDGET` setting is gone and every call is only recorded in the hub spend
ledger. Measured cost of a 30-minute session at the old defaults was typically 15 to 25 calls (a repeated character and expression
pair, and a repeated backdrop request, come from the cache for nothing), about 0.3 to 0.5 USD.

### Scene plates (PLAY-V09)

A scene plate is the whole story scene as ONE composed photographic film still (the camera
stands with the party; heroes small and seen from behind, the speaker the lit focal point,
extras, foreground objects, one light) in place of the painted backdrop with separate cut-outs.
It is generated just in time and cached like every runtime image (`ImageKind::ScenePlate`,
2560x1440 JPEG, medium quality, about 14 to 24 s and 0.06 USD each; a repeated plate is a cache
hit). Combat and the lobby never get one.

- Staging is the rule-based shot director `gameplay/shot.rs`: a closed, validated `ShotSpec`
  (camera, speaker slot and task, hero slots and activities, extras, foreground dressing, light)
  seeded by the scene, from the closed vocabulary `gameplay/shot/place_families.json` (every
  person in it is an adult). No model stages a scene; no player text and no model output reaches
  a plate prompt (`imagery/plate_prompt.rs`, the photographic recipe of `look-v2.md`; 3,978
  characters at most, the adapter's limit is 4,000).
- References: the heroes' standing figures composed into one party sheet
  (`imagery/party_sheet.rs`), the speaker's full-length portrait, the place painting.
- Timing (`gameplay/living_scenes/plates.rs`): the plate of the scene that becomes current and
  the plates of the scenes the beat script can lead to next are requested at once, once each;
  nothing waits for one. A lasting face the mood tracker asks for on the speaker is an EDIT of
  the plate on screen (the first plate is the base), not a fresh render.
- Wire: `GameplayScene.live_plate` (20), `plate_layout` (21: the speaker's head box and the calm
  side for the bubble, in permille of the plate, the measured 9% head-raise bias corrected) and
  `plate_state` (22: unspecified, pending, ready, failed). Display and phones both get them.
- Head placement: the image model does not obey a requested position (asked 70% across, the
  speaker landed at 37 to 62%), so each finished plate is shown once, downscaled, to a vision model
  (`df-providers/src/head_locate.rs`, `gpt-5.5`, about 4 s and a fraction of a cent, never charged to
  the image budget) with the speaker's fixed description, and the measured head box becomes the
  layout; when the call fails the director's prediction stands (asked head 70/30% across and 56% down,
  corrected by the measured drift: 14 to 18% of the frame height higher, 6% toward the middle).
- Heroes and identity (follow-up): the same vision call also finds the heroes' heads (`PlateLayout.heroes`,
  proto `PlateHead`), so a hero's typed line stands beside that hero in the plate (the hero's party card
  is the fallback when the hero is not found). The speaker's reference is both the full-length portrait and
  a close portrait of the face, and the speaker is named by role only (no clothes): a description that
  disagreed with the art (a plain tunic where the art has a kerchief) was the likely reason the same
  person drifted between two scenes' plates. A measured placement is kept in `<key>.layout.json` beside
  the cached plate and read back by a later run. Phones draw the plate as the scene picture.
- Recipe (PLAY-V09 tuning, `artifacts/tmp/play-v09/tune`): a wide shot on a 28mm lens from far back,
  heroes no wider than a quarter of the frame with their heads about 55% down, all heads below the top
  30% (the display's title block sits top right), photographic capture terms rather than a style.
- Display: a ready plate is the scene picture; the cut-outs of the heroes and the cast are not
  drawn and their speech bubbles stand beside the speaker's head (`plate_layout.rs`); until it
  is ready, and if it fails, the scene looks as it always did. The plate has a slow push-in,
  dust and a breathing vignette; an edit swaps in with a focus pull.
- Setting: `DF_GAMEPLAY_PLATES` `on` (default) or `off`; plates also need imagery on (the
  `OPENAI_API_KEY` present). Nothing caps the number of plates.
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

## Scene rendering engine v2: a layered stage with baked frames (decided 2026-10-13)

Why. The owner, watching played scenes on 2026-10-13: "even though the scenes are updating, the
player characters never move and nor does the background, the only thing that moves is the npc
facial expression"; "I want the display plate to be an every 10 second snapshot of the game
state (json prompting) so we can see emotions on all the characters faces and have them
interact with the scene and move around"; "with those hard timings and deps I still want more
motion and dynamism in the scenes"; "make a plan and upgrade the scene rendering engine". Budgets
are out of scope for now ("a later design config decision"): requests respect only the existing
`DF_GAMEPLAY_IMAGERY_BUDGET`.

What limits it (measured on 2026-10-13 games, `imagery.jsonl`): a scene plate takes about 19 s
(23 s at worst), a plate edit 18 s, a transparent cut-out 13 s, an empty-room backdrop 12 s. A
single baked still therefore cannot follow play; anything that must react within a second or two
has to be drawn by the browser from parts made in advance.

The model. A story scene is no longer one picture. The server sends a **scene graph** with the
display view, and the TV draws it in layers; the image models make the layers' parts and, when
the scene has settled, a photographic **baked frame** of the whole graph.

- **Scene graph (server, JSON, audience-safe).** The place (its empty-room plate), time of day,
  weather and light; the camera's framing; every actor present (heroes, the cast, background
  people) with their reference art, where they stand (a named spot in the room with a picture
  anchor and a depth), pose (standing, seated, leaning, holding, gesturing, walking), expression
  and body language (from the emotional model, EMO-01), who they are looking at, what they hold;
  the props that can change (from the scene model, ENV-07) with their state; the effects (fire,
  candles, rain, smoke). One function builds it from the server's state; its JSON is the prompt
  of every bake, so a frame is reproducible and two frames can be compared.
- **Layers on the TV, bottom to top.** (0) the set: the empty-room plate, or its ambient video
  loop when ready (MOT-02); (1) depth: the floor band and spots of the set, known from the set's
  own prompt ("a wide clear floor across the lower third") and refined by the existing vision
  call that measures heads (`PlateLayout`), giving each spot a scale; (2) actors: transparent
  cut-outs per actor, pose and expression, cached, placed at their spot and scaled by depth, with
  a contact shadow, relit to the set's light (colour and brightness from the set's measured
  palette), softened by distance (the battle view's tilt-shift), breathing, turning toward who
  they look at, leaning in when they speak, sliding with a step bob when they walk to a new spot;
  (3) props whose state has changed, as cut-outs (a set edit removes a baked prop for good);
  (4) atmosphere: dust, embers, smoke, rain or snow from the weather, light flicker (done); (5) the
  camera rig: a slow drift, a push toward the speaker, a pull back for the narrator (done:
  `f43d3fa7`), and director shots chosen per beat (wide, two-shot of speaker and listener, close
  on the speaker) with crossfades.
- **Composite while things change, baked frame when settled.** Every visible change to the graph
  (an actor moved or changed pose, a feeling went up a level, a prop changed, someone came or
  left) is drawn at once by the composite. When the graph has been still for a few seconds and
  differs visibly from the last baked frame, the server bakes it (a plate edit of the last baked
  frame with the graph as JSON and the fixed references: party sheet, portraits, the set; a fresh
  plate every few bakes to stop drift) at most once every 10 s; the TV crossfades from the
  composite to the baked frame (and its video loop) and back to the composite at the next change.
  A bake that arrives after the graph changed again is dropped.
- **Heroes act out what players say.** A hero line or action that implies a place or a pose ("I
  lean on the bar", "I sit by the fire", "I pick up the mug") changes that hero's spot and pose in
  the graph (the scene model's interaction resolves it); the composite shows it within a second;
  the next bake makes it photographic.
- **Item close-ups** (ENV-08) use the same machinery: a cached cut-out or card picture per thing
  and state.
- **Phones** keep the latest still (baked frame or set) and do not run the composite.
- **Never in a fight or a roll**, as today; nothing waits on an image; a missing part falls back to
  the layer below (no cut-out: the actor is not drawn; no set: the old painting).

Order of work (one worker at a time in `df-tools`):

| Step | Worker | What | Depends on |
| --- | --- | --- | --- |
| 1 | coordinator | The living camera on the plate | done `f43d3fa7` |
| 2 | R01 (Luna, max effort) | Scene graph v1 from what exists (set plate, cast and hero cut-outs, spots from the layout, light); the layered composite on the TV with depth scale, shadows, relighting, idle motion, speaker focus and director shots; hero cut-outs; the composite/baked switch using today's plates as the baked frame | after the investigation worker |
| 3 | E01 (Sol) | Emotional model: expressions and body language per actor per moment | R01 |
| 4 | M01 (Luna, max effort) | Ambient video loops of the set and of each baked frame | R01 |
| 5 | V06 (Luna, max effort) | Scene model and state: spots, props, interactions that move actors and change props | R01 |
| 6 | V07 (Luna, max effort) | Bake on change from the graph's JSON; set edits for props; item close-ups | V06, E01 |

Acceptance is seen on a running game, not in tests alone: in a conversation scene the heroes and
the people present are visible as figures that breathe and turn toward whoever speaks, the camera
moves to the speaker, a hero who says "I lean on the bar" is at the bar within two seconds and in
the next baked frame, and every face shows the feeling the emotional model holds.

## Art contract and the resource hub (2026-10-17)

The game's look is documentary photography. Painted or illustrated art never reaches a screen (TV
display, phones, battle map) or an image-generation request. Two modules own this, and nothing else
decides what a picture is or how one is made.

**The art contract** (`crates/df-render/src/art_contract.rs`, std-only, shared by server and
WebAssembly clients). A *depiction* is a picture of a person, creature, place, ground or prop
(stems starting `portrait-`, `figure-`, `sprite-`, `scene-`, `ground-`, `prop-`); everything else
(icons, frames, dividers, dice, ribbons, status icons, wordmark, sound) is chrome and allowed as it
is. A depiction has exactly two sources: live generated photographic imagery served by key under
`/gameplay/imagery/<key>`, or a reviewed `-documentary-photo` library asset
(`art_contract::REVIEWED_PHOTOS`, listed in `assets/ui/generated/documentary-photo-v2.assets.json`).
Content (packs, battle scenes, cast entries, the adventure art manifest) may name a reviewed
photograph, a role token (`live-room-photo`, `live-canonical-photo`, `neutral`) or a live path,
nothing else (`check_content_reference`). Identity keys such as `portrait-hero-2` or
`portrait-ant-wolf` name a subject, never a file; `identity_photo` maps one to its reviewed
photograph when it has one. The server's static route (`is_servable_stem`), the generation
reference loader (`ReferenceStore::load`, which resolves through `plan` and reads no other file),
content validation (pack, cast entries, battle scenes, the art manifest) and the browser renderer
(`fetch_target`) all refuse a painted stem. The painted files are deleted.

One resolver, `art_contract::plan(role, request)`, gives every role the same fallback order:

1. A live image already generated and cached for this subject.
2. A reviewed `-documentary-photo` asset.
3. Request the live generation now. Until it is ready, show the neutral chrome fallback
   (silhouette, initial, plain floor or block); never painted art.
4. If generation fails, keep the neutral fallback and record it in the audit log.

| Role | Live source | Neutral |
| --- | --- | --- |
| battle backdrop | room photograph, then scene plate | dark stage |
| battle ground | room photograph | plain floor colour |
| battle prop | room photograph | neutral block |
| creature or NPC token, hero portrait | canonical character photograph | silhouette or initial |
| lobby art | room photograph, then scene plate | none |
| stage place (TV) | scene plate, then room photograph | none |
| generation reference | none | the neutral photographic carrier |

Generation is on demand: the first time a view needs a depiction that has no reviewed photograph,
the *server* requests it through the canonical character path (people, creatures) or the canonical
room and scene plate path (places), once per subject, bounded by the imagery worker's concurrency
(there is no spend cap), cached by key like every live image, and written to the `ART`, `RESOURCE` and
`IMAGERY` audit lines. Nothing is generated ahead of time, no library is pre-generated, and neither
a test nor a client render loop ever requests one. Grounds and props are drawn as a plain floor or
neutral block; the place's room photograph is their live source, so no separate ground or prop image
is generated.

**The resource hub** (`crates/df-tools/src/gameplay/resources.rs`) is the one place the program
asks for a generated resource and the one place that remembers what was asked, what is in flight,
what is ready, what failed and what it cost. A typed `ResourceSpec` per kind (`Depiction`,
`ScenePlate`, `PlateEdit`, `ItemCloseUp`; `MusicSet`, `VoiceLine`, `VideoClip` reserved) derives a
`ResourceKey` from the kind, a per-kind `CONTRACT_VERSION` and the canonical spec, so a changed spec
or version is a new key and a stale value cannot be served. Consumers call `Hub::get_or_request`
(queue a generation) or `Hub::render` (run one now); the hub owns the registry, the one-in-flight
rule, the per-kind timeout and attempt limit, the failure memory (permanent failures stay failed,
with the reason, until invalidated), the freshness check of a ready entry, the shared `SpendLedger`
(reserve, commit, refund-if-not-sent, reconcile, per unit; it records spend and never refuses) and
the dev-tool hooks. The image bytes
stay in the imagery cache, which an entry's handle points at; `Imagery::generate` is the only way
an image is made, and `Imagery::render` is private to the hub's generator.
