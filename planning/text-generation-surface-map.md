# Text-generation surface map

Date: 2026-10-28
Status: Read-only working-tree audit and migration inventory; no implementation claim
Task: TEXT-SURFACE-PLAN-20261028-A1-SOL
Companion decision: [Unified architecture](text-generation-architecture.md).

## Source, method and denominator

Audited actual `C:/Users/mreca/Desktop/dungeonfluxGame`, not the chat's OneDrive copy.
HEAD: `6d0ce07e1d775d8bff9f569b23826a0503e13035`. Working source already contained
concurrent image/finance/provider/hosted changes; no clean-tree assumption. Initial git status
and source hashes were retained before writing these documents; the expanded final census refreshes
source hashes while retaining the initial snapshot separately. No git/workflow DB
mutation, application edit, provider/CLI call, runtime or test execution occurred.

The reproducible `audit_text_surfaces.py` in
`artifacts/tmp/text-generation-20261028/` enumerates git-tracked working copies plus nonignored
untracked source, excludes artifacts/archive/vendor/private, sensitive environment/secret config,
known binary formats, files over 16 MiB and non-UTF-8, then records exact file/line/nearest function and hashes.
It scans all remaining UTF-8 files, including Rust, Python, shell, PowerShell, TOML, Markdown,
JSON, YAML, SQL, protobuf, JavaScript/MJS evaluator scripts, Swift helpers, HTML/CSS and lockfiles.
The two output plans are excluded to avoid counting their own quoted adapter names. It does
not read credentials. Search patterns and exclusions are in the retained script; evidence does
not become an application dependency.

Snapshot: **3,213 tracked paths + 90 nonignored untracked paths; 2,851 UTF-8 files scanned
(2,764 tracked); 452 excluded paths; 5,875 overlapping lexical candidate rows**.
The expanded final census supersedes an initial extension-limited census; that initial evidence
is preserved under `initial-census/`. The source HEAD is unchanged; working-tree drift is recorded.

| Search family | Rows | Files | Interpretation |
| --- | ---: | ---: | --- |
| Text wrapper symbols | 172 | 35 | Includes implementation, imports, tests and documentation references |
| Expression/semantic consumer methods | 269 | 33 | Includes tests, wrappers and occasional non-model names; not live request count |
| Direct `.choose` | 23 | 5 | SystemOne call sites plus review-required matches |
| Raw text endpoint/SDK names | 21 | 12 | Includes loopback tests/docs; actual dispatch owners below |
| HTTP/client/send/curl/fetch candidates | 2,178 | 527 | Includes unrelated HTTP; not text-provider count |
| Process/spawn/child_process candidates | 1,403 | 358 | Includes async tasks and unrelated tools; not CLI-provider count |
| Text/model configuration | 454 | 93 | Includes tests/docs and actual profile defaults |
| Prompt/purpose declarations | 103 | 32 | Purpose fields, fixed instructions and versions |
| Fixture/live-test candidates | 1,103 | 202 | Broad discovery, not number of text suites |
| Planned/admission contracts | 149 | 41 | Pure contracts distinguished from reachable execution |

Rows overlap intentionally. nearest_function is a lexical locator, not an AST call graph. The
`surface_hint` after-first-cfg-test field is deliberately only a review hint: a cfg(test) import at
file top does not make later production code test-only. Reachability/status decisions below come
from source reading and callers, not that hint. Counts are snapshot data, not quality scores.

Retained artifacts: script, inventory-summary.json, candidate-inventory.csv, ten category .txt
files, tracked-files.txt, untracked-files.txt, excluded-files.json, source-hashes.json,
source-head.txt and source-status-before.txt. A final handoff records drift separately.

## Actual roots and authority

The original consumer-method pattern enumerated named methods and omitted bare
`.classify` aliases. A separate reproducible `audit_classify_aliases.py` now queries the same
2,851-file manifest and retains **18 additional candidates across 8 files** in
classify-alias-inventory.csv/classify-alias-summary.json. Those 18 supplement the 5,875-row
census and 292-row expression/choice appendix; they do not retroactively change its denominator.
The receiver-reviewed alias index below distinguishes generation calls from unrelated parsing.

All gameplay paths in this document are relative to `crates/df-tools/src/gameplay/` unless a full
crate path is given. References name audited file lines/functions; concurrent edits can move them.

| Root / contract | Observed source | Status and migration consequence |
| --- | --- | --- |
| Playable native tools runtime | `crates/df-tools/src/main.rs:46`; `gameplay.rs:1178`; Storyteller bootstrap `gameplay.rs:1431`; Jev `1437,2441,2510,2720` | Actual application composition, not merely smoke utilities. Replace each configured wrapper root. |
| Hosted support worker | `crates/df-hosted/src/runtime.rs:207`; `support_ingest/providers.rs:56,94` | Actually conditionally started from hosted runtime with explicit import/model approval gates. Include non-game text work. |
| df-server campaign-runtime / egress | `crates/df-server/src/serving.rs:274,297`; `composition_registry.rs:438` | Current recovery/health startup and registry string do not prove gameplay TextService registration. Target integration remains required. |
| df-ai contracts | `crates/df-ai/src/lib.rs`; admission/expression/lookup | Real pure validation/recording/read contracts; no runtime AiService implementation. |
| Authored courier answer | `courier_ai.rs:401 execute`, `actor.rs:855`, `journey.rs:4889` | Real prepared-only execution using df-ai read/admission; no model fallback. Preserve independent source/audience commit. |
| Pure narrative/world/rules/directors | `crates/df-narrative/src/lib.rs`; `df-interaction`; runtime-directors plan | Pure proposals/source policies, not hidden provider roots. Real runtime generation currently sits in gameplay wrappers. |

## Backend and raw-dispatch inventory

Five execution adapter families plus hosted duplicate routes are observed. Every row must retire
its public/direct dispatch in favor of private df-ai backends. Shared transport remains shared.

| Adapter / executable entry | Wire / settings / current behavior | Migration |
| --- | --- | --- |
| `df-providers/src/text_response.rs:402 NativeTextProvider::submit`; registry `registry.rs:29 registered_text_provider` | OpenAI `/v1/responses`, configured model/schema/reasoning/max output, `stream:false`, `store:false` (`:443`); complete body/schema/usage checks. Gameplay `storyteller.rs:1619`. | T2 private Responses backend; remove public constructor/factory after all provider/test consumers migrate. Keep neutral schema types where required. |
| `df-providers/src/systemone.rs:96 SystemOneChoiceProvider::choose` | TypeSafe `/v1/systemone` (`:87`), hardcoded `jev-latest` (`:17`), one q1/2..255 options, four-second deadline, no retry; required meter. | T3 private SystemOne backend supporting both single and actual support batched questions. |
| `df-providers/src/plate_judge.rs:211 judge`, `:225 judge_photo_quality`, `:278 judge_with_detail` | OpenAI Chat Completions `/v1/chat/completions`, gpt-5.5 (`:24-25`), pictures+strict schema; output used for photo/identity/staging judgments; no retry. | T4 typed multimodal/JSON API; feature retains verdict semantics and current compatibility degradation. |
| `df-providers/src/head_locate.rs:145 NativeHeadLocator::locate` | Same Chat endpoint/gpt-5.5 (`:23-24`); image, optional reference, targets -> validated boxes; no retry. | T4 typed image parts + head-box schema; preserve target/region/picture identity. |
| `codex_cli.rs:443 text`; `codex_cli/windows_process.rs` native process runner | ChatGPT-authenticated CLI via stdin/JSONL; fixed 0.150.1 preflight (`:239`), gpt-5.6-sol (`:324`), dedicated empty home/hash/scratch checks, Windows-only real implementation; image tools disabled (`:373`). | T2 private native CLI text adapter; capability-based configured versions/models/accounts, owned process tree. Keep no side effects; no image tools through text. |
| `df-hosted/src/support_ingest/providers.rs:199 ReqwestModelTransport::judge` | Raw reqwest `/v1/systemone` (`:204`), supplied DF_SUPPORT_JEV model/base/key, four-second request timeout. | T3 shared API batched-choice request; remove direct HTTP transport. |
| Same `:219 ReqwestModelTransport::draft` | Raw reqwest `/v1/responses` (`:221`), separate DF_SUPPORT_DRAFT model/base/key, store false, strict support schema, 4096 output, bounded response. | T3 after T2 Responses backend; shared structured draft request; preserve classification consistency and human review. |

`df-providers/src/native_http.rs:205,302` is reusable bounded reqwest transport, not another text
provider. `http_contract`/media-fetch modules also serve unrelated media; do not ban those by name.
Raw HTTP review found OAuth (df-google/df-workos), billing/mail (df-hosted), campaign runtime HTTP,
WorldLabs/fal/BFL/ElevenLabs/media retrieval and dependency-download curl. They are not extra text
backends. Process review found launcher/runner/PostgreSQL/build/browser/ffmpeg/render tools and
async tasks; only the Codex text path above is the observed text-model subprocess adapter.
The concurrently added df-imagery Codex process is an image workflow, coordinated separately.
No actual Anthropic/Gemini/Ollama/llama text SDK or raw endpoint was found in scanned source.

## Feature-by-feature migration matrix

S = Storyteller -> Responses or selected Codex text; J = ApproachModel -> SystemOne; V = multimodal
Chat judgment. Each group moves to the same API under its registered purposes. Existing domain
parsing, rules/source authority and deterministic fallbacks remain feature-owned. The appendix
preserves exact expression/semantic method candidates, including inline test calls.

| Surface / evidence | Actual route and output | Required migration / semantic owner |
| --- | --- | --- |
| Storyteller wrapper `storyteller.rs:614,631,655,674,778` | S; adventure/expression/partial/single-attempt; bounded slot maps | T2 move lifecycle/cache/retry/accounting once; compatibility facade then remove. Preserve independent slot acceptance. |
| SRD critic `storyteller.rs:1239 judge_srd_material`, `:1319 judge_values` | S child `srd_content_judgement`; can run on cache read | Explicit child purpose/attempt, finite review/correction and cost. Never treat critic as rules authority or assume cache means zero calls. |
| Adventure prose `adventure/expression.rs:1272`; deterministic generator `adventure/generate.rs:780` | S `adventure` groups; baseline generator has a synchronous TextSource seam | Same API expression adapter; preserve deterministic generation and accepted/refused group fallback; no network in pure source generator. |
| Free-roam town `adventure/free_roam.rs:323,335 city` | S `free_roam_city`, structured slots and deadline | T5 town context/provenance and validator, same service; no second retry loop. |
| Dialogue dispatch `dialogue.rs:691,724,757,807,829,1001,2822,2893,4258,4395,4588,6294,6982,11043,11707,11811` | J labels/options for request/willingness/help/demand/social/action/target semantics | T5 typed choices/proposals/basis. Dialogue alone never commits a mechanical action. |
| Dialogue prose `dialogue.rs:2916 narrate_scene_examine`, `:2943 narrate_unstaged_address` | S `scene_thing_examine`, `unstaged_addressee` | T5 permitted public facts and existing fallback. |
| Jev semantic wrapper `approach_model.rs:468-1155`; direct choose helpers `1296-1517` | J NPC event/consent, coin handover, hero identity, addressee, target, intent, skill, subject, approach, emotion | T3 wrapper delegates central choice API. Preserve no-decision/typed failure and real current action basis. |
| NPC reply `npc_reply.rs:2196,2257,2307,2377,2424,2500,2717,2819,3544,3606,3675,3783,3813,4178` | J focus/recipient/event/approach/emotion/permission/grounding judgments | T5 registered subpurposes; current NPC knowledge/relationship and owner commit remain authoritative. |
| NPC wording `npc_reply.rs:3922 model_dialogue_inner`, `:4306 model_public_corrected_inner` | S `npc_reply` and corrected wording | T5 listener-safe schema/claims, existing reply validation and matched captions/voice. Central finite correction stages. |
| Influence `npc_reply/person_influence.rs:209` | J same-request/willingness choices | T5 preserve source Influence legality, failed-request restriction and NPC refusal; no AI-created bonus/DC. |
| Emotional profile `npc_dossier.rs:277 generate_emotional_profile` | S `npc_emotional_profile`, source-field-grounded JSON in slots | T5 retain dossier-specific schema/citations/adult restrictions, no invented persistent traits. |
| Fact backfill `fact_backfill.rs:269,421,432,479,508,527,583,675` | J recorded/backfill/reason/consistency + S `npc_fact_questions`, `npc_fact_backfill` | T5 REF-069; narrative owner validates/stably commits new canon before wording; missing answer remains unavailable. |
| Freeform action `freeform_action.rs:793,809,871,877,909,924,940,999,1305,1432,1560` | J act/sense/target/difficulty/recipient/spell/means/consistency choices | T5 semantic proposals only; existing rules/source/intent owner resolves effects. |
| Freeform narration `freeform_action.rs:1545 refine_sense`, `:2614 narrate` | S `freeform_sensory_impression`, `freeform_act` | T5 permitted outcomes only; distinguish input interpretation from output validation. |
| Investigation/item detail `investigation.rs:412,439,464 decide_item_story` | J classification/approval + S `item_story_detail` | T5 REF-070; owner controls ordinary/colored/clue canon and actual checks. |
| Recall/findings/questions/look `investigation.rs:1614,1681,1999,2013,2092,2114,2222,2239,2279,2534,2598,2977` | S `recall`, `observation_finding`, `observation_intention`, `observation_question`, `investigation`, `sightseeing`; J targets/consistency/plain-look | T5 typed source context and output contract; remove direct provider argument at resolve_target(:1663). |
| Local lore `local_lore.rs:632,804` | J role/domain and proposed fact acceptance | T5 lore feature retains canon policy; semantic model cannot authorize hidden disclosures. |
| Place navigation `place_creation.rs:116,153,318,364,397` | J local reference/heading/ride/service | T5 proposal only; place owner checks open exit/destination/source again. |
| Place/person creation `place_creation.rs:134,166,1174,1191,1552,1557,1720` | S `local_spot_reference`, `local_spot_refusal`, `person_persona`, `place_creation`, `travel_beat`; J persona approval | T5 structured place/person schema and durable owner commit; preserve authored fallback and finite semantic corrections. |
| Fight anticipation `fight_foresight.rs:58 forecast`, `:167 judge` | J fight likelihood/arena size via trait adapter | T5 registered judgment purpose; forecast remains speculative, no actual combat/reward creation. |
| Witness reaction/possessions `fight_witness.rs:171 reaction`, `:181 judge`, `:376 identity_possessions` | J witness reaction + S `fight_carried_identity` | T5 witnessed provenance, legal possessions and idempotent social commit; no reroll on replay. |
| Trade/items `journey/merchant_bargaining.rs:62,83,296,1246,2442,2733,3504` | S `merchant_item_details`, `merchant_exchange`, `trade_bulk_pick`; J offer/item/commitment semantics | T5 typed exchange proposals; purse, catalog, inventory, acceptance and durable transfer remain server-owned. |
| Living scene interactions `journey/living_scene.rs:7056,7153`; additional dialogue demand calls | J scene interaction/recipient/demand | T5 explicit operation basis; no coin/item handoff based solely on generated prose. |
| Player-line cleanup `line_cleanup.rs:574 display_line` | S `line_cleanup` | T5 existing speech-preservation and quote/meaning validation; output must not invent player intent. |
| Story-memory summary `memory/dossier.rs:101,110 StorytellerWriter::write` | S `story_dossier`; public/party events and prior derived summary | T5 same API, cited event-set hash and bounded source context; never replaces canonical truth. |
| Memory consistency `memory/consistency.rs:152 check_consistency` | J record-consistency/supersession choice | T5 source-linked domain update; summary/index cannot become another truth writer. |
| Music plan `place_music/spec.rs:717 ask`; ambience `ambience.rs:209,234,252`; pacing `pacing.rs:44` | J musical choice/ambience/pacing labels | T5 text judgment through API; ElevenLabs synthesis/ffmpeg stays media-owned and separately metered. |
| Creator routing `creator_classification.rs:102 classify`, startup `creator_portrait_startup.rs:117`, caller `creator_portraits.rs:537` | Direct SystemOne public appearance -> seven-category assessment -> rating/route | T3/T4 imagery consumer port to TextService, preserve dedicated credentials, rating semantics and no silent cross-provider fallback. |
| Plate/head location `imagery.rs:892,905,987` | V picture/reference+targets -> boxes | T4 typed multimodal; imagery owns reference identity/region checks and downstream presentation. |
| Photo/visible-world review `imagery/photo_check.rs:350,404,488` | V quality/hero-count/custody and visible_features in same judgment | T4 preserve one combined call, supplied public record ids and verdict-specific publication. No second review call merely for extraction. |
| Segmentation semantics `plate_segments/service.rs:649,717 staging_work` | V instance/scene staging under strict schemas | T4 language judgments use API; fal/SAM segmentation and masks remain image processing. |
| Paid diagnostic `dev_control/reads.rs:32 photo_check` | Direct V new judgment of saved bytes, gate+credential+meter | T4 same API/admission under operator authorization; read-style route is still billable. |
| Support triage/draft `df-support/src/classification.rs:145-206`; hosted raw adapters above | Batched J category/urgency/privacy/legal/safety then separate Responses action draft | T3 two purposes/child attempts, mailbox scope, existing model approval and human-review constraints; no send/action tool. |
| Courier prepared answer `courier_ai.rs:401,438,460` | df-ai authorized prepared lookup/complete-record admission/stage completion | T5 API prepared profile, exact source/locale/audience/idempotency; miss never calls live model. |

## Adjacent deterministic and planned surfaces

Do not misclassify every prompt string or every feature plan as an existing generation call:

- `imagery/prompt.rs:529,546,575,602,861,891`, `imagery/mural_prompt.rs:6 party_mural`,
  plate/room/cast/item image prompt modules and `scene_video_prompt.rs:6,66,87` build strings
  deterministically. They remain feature-owned image/video inputs. If a model is later used to
  rewrite them, that new text request must register a TextService purpose first.
- `memory/embedder.rs:68,80,122` wraps NativeEmbeddingProvider; startup `memory/startup.rs:69`.
  Embedding vectors are not generative text. Keep embedding API and its existing cost linkage;
  derived summary and consistency judgments are in scope above.
- `voice.rs`, `speech_input.rs`, `speech_text.rs`, ElevenLabs speech/music, fal video, WorldLabs,
  raster/segmentation and image generation are distinct modalities. Text analysis/critic subcalls
  are in scope, synthesized audio/video/image bytes are not TextService output.
- `adventure/generate.rs`, content tables, locale catalogs, combat/rules narration/templates,
  authored courier text, campaign draft form fields and manually curated share summaries do not
  establish extra live provider calls. Preserve deterministic source behavior.
- Campaign authoring, recaps/trailers/cinematics, long-horizon memory, generated item/reward flavor,
  optional dynamic encounter/tactic text and future narrative/world hooks are required/planned
  consumers under their governing planning docs when implemented. No separate current model
  backend was found for them by this census. Register their purposes through TextService; retain
  source/rules/consent authority and required deterministic rewards during provider failure.

## Test, fixture, smoke and evaluation migration

All ordinary tests use the same TextService consumer interface with supplied fakes. Backend wire
contract tests can use exact reviewed private-backend/loopback seams. No blanket test bypass.

| Existing surface | Evidence and destination |
| --- | --- |
| Storyteller/fake HTTP | `storyteller_tests.rs` FakeEndpoint, slots/cache/retry/usage/withheld/SRD/dedupe tests; preserve and run through facade then API. |
| Adapter fixtures | `crates/df-providers/tests/text_response.rs`, `native_http.rs`, `metering.rs`; inline systemone/plate_judge/head_locate/text_schema tests; move execution tests with private backend or narrowly reviewed fixture seam. |
| Jev semantics | `approach_model_tests.rs`, `social_tests.rs`, creator_classification inline tests, creator_portrait_route_tests, investigation_tests; keep faithful SystemOne shape and current labels. |
| Feature test trees | dialogue variants, freeform_action_tests, investigation_tests, place_creation_tests/place_tests, npc_reply inline tests, npc_dossier_tests, fact_backfill inline tests, merchant_bargaining tests, living_scene tests, fight_foresight/fight_witness inline tests, memory/tests and music tests. Replace underlying backend, not assertions/source rules. |
| Multimodal fixtures | imagery/photo_check_tests, imagery_tests, living_scenes/plates_tests, imagery/mural_prompt photo_gate_tests, plate_segments tests. Same API with exact image/schema/usage fixture evidence. |
| Pure df-ai contracts | df-ai/tests admission/expression/lookup; gameplay/cache_replay_locale_contract, courier_ai_tests, courier_process_qualification, recovery_qualification. Preserve zero-live replay, current basis and publication gates. |
| Generic provider fakes | df-testkit/src/provider_fake.rs + tests/provider_fake.rs; df-acceptance provider_fixtures/provider_outcome. Reuse scripted events; no production egress escape. |
| Support fixtures | df-support/src/classification_tests.rs:69,91,103,148,167,183; fakes.rs FakeJevJudge; df-hosted/support_ingest/tests.rs:182,226,310,345,463,550,637. Retain two child calls, meter-before-dispatch, crash/manual-review and no reclassification. |
| Paid/ignored conversations | npc_talk_live_tests.rs:450; npc_dossier_live_tests.rs:201; content_rating_live_tests.rs:134; adventure/expression_tests.rs:1316,1380. Keep ignored/live-only policy; migrate setup to same API; not run here. |
| Offline prompt/evaluation utilities | Same live-test files contain seed/prompt-size/disk-comparison utilities; adventure/expression_tests.rs:1454,1479; prompt builder tests and existing dev commands. Offline operations remain zero-egress; explicit live operations still use API and separate scoped authorization. |

Browser/evaluation launch surfaces also reach text through the normal runtime:
`development/browser-checks/stage-clean.mjs:11`, `stage-replies.mjs:20` and
`stage-real-voice.mjs:10` request WITH_STORYTELLER; `stage.mjs:142,144` launches serve.sh,
whose `:34,47,48` selects text credentials/cache and starts the gameplay binary. `serve-local.mjs:42,55`
is a platform launch variant; `talk.mjs:2` and `converse.mjs:53` submit conversations through the UI.
`stage-all.sh:9` includes the real-text checks. These are provider-using acceptance tools, not
raw provider clients. Their ordinary offline versions must select a fixture through TextService;
retain separately scoped live checks. Never run the aggregate script assuming every test is free.
`development/dev-gameplay.py:95-102`, its test, `development/run-gameplay.sh:38` and launcher
`settings.rs:398` preserve credential/feature/environment mapping through centralized configuration.

The broad fixture census also finds unrelated transport/audio/image tests. They are not all text
suites. The exact candidate appendix below and retained CSV permit review without claiming every
match is executed production code. Implementation acceptance must add named stream/disconnect/
malformed-schema/tool/cancel/timeout/Unknown/replay/stale-source/cost-correction cases specified
in the architecture plan and run meaningful affected feature tests.

R3 development migration preserves the measured inventory above and adds the architecture's
Development flexibility contract. Current fixed model/CLI/path and byte/slot/deadline constants
must map to documented configurable profiles or explicit actual capability/protocol/OS limits.
Local, loopback, fixtures, replay and browser tools use that same API without hosted balance;
real hosted authorization, consent, source checks and paid uncertainty remain enforced. The
source census is unchanged; this amendment is a planning requirement, not new runtime support.

## Coverage limits

This is a complete enumerated search of the declared tracked/nonignored text corpus plus manual
tracing of observed adapter roots and feature groups, not a proof of all possible runtime paths.
Generated glue, binary assets, private/ignored source, external dynamically selected executables,
deployed configuration and live backend capabilities were not executed. Current shared edits may
move source lines; retain hashes and refresh before implementation. Future groups listed as planned
must not be reported as working features. No new tests or application code were written.

## Exact lexical expression and semantic-call appendix

Every candidate is retained, including wrapper delegation and tests. The function locator is
lexical; status comes from the curated matrix above. These are source references, not a claimed
number of paid calls. Identical line/category overlaps are kept only once in this appendix.

### `crates/df-providers/tests/metering.rs`

- `crates/df-providers/tests/metering.rs:152` - `the_choice_client_tells_its_meter_what_the_supplier_answered` / `choose`.
- `crates/df-providers/tests/metering.rs:188` - `a_required_meter_does_not_change_the_choice_request` / `choose`.

### `crates/df-tools/src/gameplay/adventure/expression.rs`

- `crates/df-tools/src/gameplay/adventure/expression.rs:1272` - `ask` / `express_adventure_partial`.

### `crates/df-tools/src/gameplay/adventure/expression_tests.rs`

- `crates/df-tools/src/gameplay/adventure/expression_tests.rs:55` - `accepted_text_does_not_refuse_forbidden_terms_inside_other_words` / `express`.
- `crates/df-tools/src/gameplay/adventure/expression_tests.rs:237` - `completed_provider_groups_survive_the_overall_deadline_while_a_group_is_slow` / `single_attempt`.
- `crates/df-tools/src/gameplay/adventure/expression_tests.rs:1416` - `live_one_raw_scenes_attempt` / `single_attempt`.

### `crates/df-tools/src/gameplay/adventure/free_roam.rs`

- `crates/df-tools/src/gameplay/adventure/free_roam.rs:335` - `city` / `express`.

### `crates/df-tools/src/gameplay/adventure/generate.rs`

- `crates/df-tools/src/gameplay/adventure/generate.rs:780` - `generate_with_prose` / `express`.

### `crates/df-tools/src/gameplay/approach_model.rs`

- `crates/df-tools/src/gameplay/approach_model.rs:478` - `classify_npc_reply_event` / `classify_npc_reply_event_with_consent`.
- `crates/df-tools/src/gameplay/approach_model.rs:527` - `classify_npc_reply_event_with_consent` / `choose`.
- `crates/df-tools/src/gameplay/approach_model.rs:577` - `classify_npc_reply_event_with_consent` / `judge_coin_handover`.
- `crates/df-tools/src/gameplay/approach_model.rs:591` - `judge_coin_handover` / `choose_label`.
- `crates/df-tools/src/gameplay/approach_model.rs:614` - `judge_coin_handover` / `choose_label`.
- `crates/df-tools/src/gameplay/approach_model.rs:621` - `judge_coin_handover` / `choose_label`.
- `crates/df-tools/src/gameplay/approach_model.rs:655` - `classify_hero_identity` / `choose`.
- `crates/df-tools/src/gameplay/approach_model.rs:675` - `resolve_addressee` / `resolve_addressee_with_party`.
- `crates/df-tools/src/gameplay/approach_model.rs:689` - `resolve_addressee_with_party` / `resolve_addressee_in_scene`.
- `crates/df-tools/src/gameplay/approach_model.rs:759` - `resolve_addressee_in_scene` / `choose`.
- `crates/df-tools/src/gameplay/approach_model.rs:869` - `choose_label` / `choose`.
- `crates/df-tools/src/gameplay/approach_model.rs:893` - `choose_option` / `choose`.
- `crates/df-tools/src/gameplay/approach_model.rs:994` - `appraise` / `choose`.
- `crates/df-tools/src/gameplay/approach_model.rs:1040` - `classify_input` / `classify_input_listed`.
- `crates/df-tools/src/gameplay/approach_model.rs:1362` - `choose_listed_thing` / `choose`.
- `crates/df-tools/src/gameplay/approach_model.rs:1458` - `choose_line_intent` / `choose`.
- `crates/df-tools/src/gameplay/approach_model.rs:1479` - `choose_attempt_skill` / `choose`.
- `crates/df-tools/src/gameplay/approach_model.rs:1498` - `choose_approach` / `choose`.
- `crates/df-tools/src/gameplay/approach_model.rs:1517` - `choose_subject` / `choose`.

### `crates/df-tools/src/gameplay/approach_model_tests.rs`

- `crates/df-tools/src/gameplay/approach_model_tests.rs:16` - `jev_physical_attack_names_the_listed_target_or_none` / `classify_input`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:32` - `jev_physical_attack_names_the_listed_target_or_none` / `classify_input`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:53` - `jev_appraisal_receives_the_full_character_and_state_and_returns_the_selected_reaction` / `appraise`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:105` - `jev_intent_options_and_attempt_skills_are_reachable_under_srd_5_2_1_pages_187_189` / `classify_input`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:168` - `jev_can_select_every_search_study_and_stealth_skill_under_srd_5_2_1_pages_187_189` / `classify_input`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:207` - `jev_skill_deadline_is_neutral_speech_with_no_skill_or_thing` / `classify_input`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:225` - `jev_off_classify_input_is_neutral_and_makes_no_request` / `classify_input`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:244` - `jev_timeout_on_an_observation_is_neutral_speech` / `classify_input`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:278` - `jev_coin_handover_judges_two_gold_coins_on_the_counter_and_no_handover` / `judge_coin_handover`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:305` - `jev_coin_handover_judges_two_gold_coins_on_the_counter_and_no_handover` / `judge_coin_handover`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:313` - `jev_coin_handover_judges_two_gold_coins_on_the_counter_and_no_handover` / `judge_coin_handover`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:406` - `startup_jev_uses_the_run_ledger_before_the_cost_book_opens` / `choose_option`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:635` - `adventure_subject_options_include_its_object_and_reject_an_unlisted_person` / `classify_for_dossier`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:644` - `adventure_subject_options_include_its_object_and_reject_an_unlisted_person` / `classify_for_dossier`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:725` - `a_jev_answer_slower_than_the_old_800_ms_deadline_is_still_accepted` / `choose_label`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:771` - `a_jev_line_reading_inside_a_timing_line_records_one_classify_stage` / `classify_input`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:792` - `a_line_judgement_that_times_out_is_logged_unavailable_with_its_deadline_flag` / `classify_input`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:823` - `a_router_timeout_is_recorded_once_as_an_addressee_line` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:841` - `a_router_error_is_recorded_once_with_the_deadline_flag_clear` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:857` - `the_addressee_judgement_records_an_addressee_stage_in_its_line` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:951` - `jev_routes_public_background_description_to_the_present_bandit` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:971` - `without_jev_a_description_routes_to_the_focus_and_is_never_matched` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:978` - `without_jev_a_description_routes_to_the_focus_and_is_never_matched` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:989` - `without_a_provider_a_present_name_does_not_route_to_that_actor` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1010` - `jev_distinguishes_discourse_prefixes_from_an_absent_named_addressee` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1019` - `jev_distinguishes_discourse_prefixes_from_an_absent_named_addressee` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1030` - `jev_distinguishes_discourse_prefixes_from_an_absent_named_addressee` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1043` - `jev_no_explicit_addressee_selects_focus_even_when_a_description_was_parsed` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1057` - `jev_addressee_decides_over_exact_ambiguous_and_named_matches` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1064` - `jev_addressee_decides_over_exact_ambiguous_and_named_matches` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1091` - `no_jev_answer_routes_to_the_focus_and_never_to_an_exact_match` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1098` - `no_jev_answer_routes_to_the_focus_and_never_to_an_exact_match` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1109` - `no_jev_answer_routes_to_the_focus_and_never_to_an_exact_match` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1143` - `jev_picking_a_party_hero_option_yields_that_hero_as_an_open_addressee` / `resolve_addressee_with_party`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1170` - `jev_private_aside_label_sets_the_flag_while_the_addressee_stays_the_hero` / `resolve_addressee_with_party`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1178` - `jev_private_aside_label_sets_the_flag_while_the_addressee_stays_the_hero` / `resolve_addressee_with_party`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1199` - `npc_and_absent_choices_are_unchanged_when_party_heroes_are_present` / `resolve_addressee_with_party`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1207` - `npc_and_absent_choices_are_unchanged_when_party_heroes_are_present` / `resolve_addressee_with_party`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1215` - `npc_and_absent_choices_are_unchanged_when_party_heroes_are_present` / `resolve_addressee_with_party`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1237` - `without_party_heroes_jev_gets_no_hero_options_and_absent_stays_absent` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1256` - `without_jev_a_party_hero_line_routes_to_the_focus_with_no_hero` / `resolve_addressee_with_party`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1264` - `without_jev_a_party_hero_line_routes_to_the_focus_with_no_hero` / `resolve_addressee_with_party`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1272` - `without_jev_a_party_hero_line_routes_to_the_focus_with_no_hero` / `resolve_addressee_with_party`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1294` - `jev_failure_or_unknown_option_routes_to_the_focus_with_no_hero` / `resolve_addressee_with_party`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1297` - `jev_failure_or_unknown_option_routes_to_the_focus_with_no_hero` / `resolve_addressee_with_party`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1312` - `jev_physical_intent_stands_without_action_words` / `classify_input`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1328` - `jev_physical_intent_chooses_the_listed_target` / `classify_input`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1344` - `jev_non_action_intent_stands_over_action_words` / `classify_input`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1360` - `physical_words_without_jev_stay_neutral_speech` / `classify_input`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1372` - `physical_words_without_jev_stay_neutral_speech` / `classify_input`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1382` - `physical_words_without_jev_stay_neutral_speech` / `classify_input`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1401` - `jev_subject_decides_for_a_dossier` / `classify_for_dossier`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1422` - `the_classifier_request_carries_the_speakers_public_identity_bounded_to_600_characters` / `classify_for_dossier`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1445` - `the_classifier_request_carries_the_speakers_public_identity_bounded_to_600_characters` / `classify_for_dossier`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1560` - `possessive_wording_is_jevs_choice_of_the_listed_thing` / `classify_input`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1592` - `choose_label_returns_the_jev_label_under_the_data_guard` / `choose_label`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1615` - `choose_label_reports_each_typed_reason_without_guessing` / `choose_label`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1624` - `choose_label_reports_each_typed_reason_without_guessing` / `choose_label`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1634` - `choose_label_reports_each_typed_reason_without_guessing` / `choose_label`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1643` - `choose_label_reports_each_typed_reason_without_guessing` / `choose_label`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1679` - `live_addressee_probe_absent_discourse_and_background` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1795` - `hs25_flirt_is_not_a_literal_bargain_and_scimitar_address_uses_narration` / `resolve_addressee_in_scene`.
- `crates/df-tools/src/gameplay/approach_model_tests.rs:1831` - `approach_model_travel_intent_uses_jev_without_matching_player_words` / `classify_input`.

### `crates/df-tools/src/gameplay/creator_classification.rs`

- `crates/df-tools/src/gameplay/creator_classification.rs:112` - `classify` / `choose`.

### `crates/df-tools/src/gameplay/dialogue.rs`

- `crates/df-tools/src/gameplay/dialogue.rs:691` - `opening_willingness` / `choose_label`.
- `crates/df-tools/src/gameplay/dialogue.rs:724` - `opening_heroes_depart` / `choose_label`.
- `crates/df-tools/src/gameplay/dialogue.rs:757` - `judge_opening_party_easing` / `choose_label`.
- `crates/df-tools/src/gameplay/dialogue.rs:807` - `judge_opening_help` / `choose_label`.
- `crates/df-tools/src/gameplay/dialogue.rs:829` - `judge_opening_help` / `choose_label`.
- `crates/df-tools/src/gameplay/dialogue.rs:1001` - `opening_social_by_line` / `choose_label`.
- `crates/df-tools/src/gameplay/dialogue.rs:2005` - `hs50_dialogue_trade_followup_keeps_bram_until_jev_addresses_tessa` / `resolve_addressee_in_scene`.
- `crates/df-tools/src/gameplay/dialogue.rs:2017` - `hs50_dialogue_trade_followup_keeps_bram_until_jev_addresses_tessa` / `resolve_addressee_in_scene`.
- `crates/df-tools/src/gameplay/dialogue.rs:2822` - `judge_examine_target` / `choose_option`.
- `crates/df-tools/src/gameplay/dialogue.rs:2893` - `judge_examine_approach` / `choose_label`.
- `crates/df-tools/src/gameplay/dialogue.rs:2916` - `narrate_scene_examine` / `express`.
- `crates/df-tools/src/gameplay/dialogue.rs:2943` - `narrate_unstaged_address` / `express`.
- `crates/df-tools/src/gameplay/dialogue.rs:3487` - `jev_routes_bluff_reading_to_insight_and_recall_out_of_investigation` / `classify_input`.
- `crates/df-tools/src/gameplay/dialogue.rs:3498` - `jev_routes_bluff_reading_to_insight_and_recall_out_of_investigation` / `classify_input`.
- `crates/df-tools/src/gameplay/dialogue.rs:3521` - `jev_can_select_a_present_addressee_for_a_typed_attempt` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/dialogue.rs:3540` - `jev_short_item_alias_resolves_to_the_present_scene_item` / `classify_input`.
- `crates/df-tools/src/gameplay/dialogue.rs:4185` - `judge_listed_service_request` / `resolve_addressee_with_party`.
- `crates/df-tools/src/gameplay/dialogue.rs:4258` - `judge_listed_service_request` / `choose_label`.
- `crates/df-tools/src/gameplay/dialogue.rs:4588` - `judge_demand` / `choose_label`.
- `crates/df-tools/src/gameplay/dialogue.rs:4628` - `judge_hero_trade` / `choose_label`.
- `crates/df-tools/src/gameplay/dialogue.rs:4656` - `judge_merchant_reply` / `choose_label`.
- `crates/df-tools/src/gameplay/dialogue.rs:4763` - `judge_scene_interaction` / `choose_label`.
- `crates/df-tools/src/gameplay/dialogue.rs:4857` - `hero_line_form` / `choose_label`.
- `crates/df-tools/src/gameplay/dialogue.rs:5482` - `submit_line_inner` / `classify_input_listed`.
- `crates/df-tools/src/gameplay/dialogue.rs:5577` - `submit_line_inner` / `resolve_addressee_in_scene`.
- `crates/df-tools/src/gameplay/dialogue.rs:5623` - `submit_line_inner` / `resolve_addressee_with_party`.
- `crates/df-tools/src/gameplay/dialogue.rs:5834` - `submit_line_inner` / `resolve_addressee_with_party`.
- `crates/df-tools/src/gameplay/dialogue.rs:5863` - `submit_line_inner` / `judge_coin_handover`.
- `crates/df-tools/src/gameplay/dialogue.rs:6023` - `submit_line_inner` / `resolve_addressee_with_party`.
- `crates/df-tools/src/gameplay/dialogue.rs:6294` - `judged_fight_addressee` / `choose_option`.
- `crates/df-tools/src/gameplay/dialogue.rs:6385` - `observation_call` / `resolve_investigation_target`.
- `crates/df-tools/src/gameplay/dialogue.rs:6982` - `examine_scene_thing_call` / `choose_label`.
- `crates/df-tools/src/gameplay/dialogue.rs:11043` - `hs39_dialogue_trade_flirt_gets_a_personal_answer_without_service_quote_facts` / `choose_label`.
- `crates/df-tools/src/gameplay/dialogue.rs:11707` - `opening_social_weak_repeat_and_claimed_other_heros_help_gain_nothing` / `choose_label`.
- `crates/df-tools/src/gameplay/dialogue.rs:11811` - `opening_social_persuasion_deception_and_intimidation_reach_hesitant_checks` / `choose_label`.

### `crates/df-tools/src/gameplay/dialogue_navigation_tests.rs`

- `crates/df-tools/src/gameplay/dialogue_navigation_tests.rs:479` - `n1_fled_and_dead_people_are_excluded_from_the_jev_addressee_choices` / `resolve_addressee`.

### `crates/df-tools/src/gameplay/dialogue_tests.rs`

- `crates/df-tools/src/gameplay/dialogue_tests.rs:2393` - `a_missing_router_provider_is_not_reported_as_a_model_call_or_deadline` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/dialogue_tests.rs:2411` - `a_router_timeout_returns_its_typed_failure_and_deadline_fact` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/dialogue_tests.rs:2425` - `a_router_error_returns_its_typed_failure_without_a_deadline` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/dialogue_tests.rs:2741` - `read` / `classify_input_listed`.
- `crates/df-tools/src/gameplay/dialogue_tests.rs:2756` - `addressee` / `resolve_addressee`.
- `crates/df-tools/src/gameplay/dialogue_tests.rs:2772` - `interaction` / `choose_label`.
- `crates/df-tools/src/gameplay/dialogue_tests.rs:3257` - `fight_addressee` / `choose_option`.

### `crates/df-tools/src/gameplay/fact_backfill.rs`

- `crates/df-tools/src/gameplay/fact_backfill.rs:269` - `guard_fact_reply_inner` / `choose_label`.
- `crates/df-tools/src/gameplay/fact_backfill.rs:421` - `prepare_fact_reply_inner` / `choose_label`.
- `crates/df-tools/src/gameplay/fact_backfill.rs:432` - `prepare_fact_reply_inner` / `choose_option`.
- `crates/df-tools/src/gameplay/fact_backfill.rs:479` - `prepare_fact_reply_inner` / `choose_label`.
- `crates/df-tools/src/gameplay/fact_backfill.rs:508` - `prepare_fact_reply_inner` / `choose_label`.
- `crates/df-tools/src/gameplay/fact_backfill.rs:527` - `prepare_fact_reply_inner` / `express`.
- `crates/df-tools/src/gameplay/fact_backfill.rs:583` - `prepare_fact_reply_inner` / `express`.
- `crates/df-tools/src/gameplay/fact_backfill.rs:675` - `prepare_fact_reply_inner` / `choose_label`.

### `crates/df-tools/src/gameplay/fight_foresight.rs`

- `crates/df-tools/src/gameplay/fight_foresight.rs:58` - `forecast` / `choose_option`.

### `crates/df-tools/src/gameplay/fight_witness.rs`

- `crates/df-tools/src/gameplay/fight_witness.rs:171` - `reaction` / `choose_option`.
- `crates/df-tools/src/gameplay/fight_witness.rs:376` - `identity_possessions` / `express`.

### `crates/df-tools/src/gameplay/freeform_action.rs`

- `crates/df-tools/src/gameplay/freeform_action.rs:793` - `plan` / `choose_option`.
- `crates/df-tools/src/gameplay/freeform_action.rs:809` - `plan` / `choose_option`.
- `crates/df-tools/src/gameplay/freeform_action.rs:871` - `plan` / `choose_option`.
- `crates/df-tools/src/gameplay/freeform_action.rs:877` - `plan` / `choose_option`.
- `crates/df-tools/src/gameplay/freeform_action.rs:909` - `plan` / `choose_option`.
- `crates/df-tools/src/gameplay/freeform_action.rs:924` - `plan` / `choose_option`.
- `crates/df-tools/src/gameplay/freeform_action.rs:940` - `plan` / `choose_option`.
- `crates/df-tools/src/gameplay/freeform_action.rs:999` - `chosen_spell` / `choose_option`.
- `crates/df-tools/src/gameplay/freeform_action.rs:1305` - `refine_sense` / `choose_label`.
- `crates/df-tools/src/gameplay/freeform_action.rs:1432` - `refine_sense` / `choose_label`.
- `crates/df-tools/src/gameplay/freeform_action.rs:1545` - `refine_sense` / `express`.
- `crates/df-tools/src/gameplay/freeform_action.rs:1560` - `refine_sense` / `choose_label`.
- `crates/df-tools/src/gameplay/freeform_action.rs:2614` - `narrate` / `express`.

### `crates/df-tools/src/gameplay/investigation.rs`

- `crates/df-tools/src/gameplay/investigation.rs:412` - `decide_item_story` / `choose_label`.
- `crates/df-tools/src/gameplay/investigation.rs:439` - `decide_item_story` / `express`.
- `crates/df-tools/src/gameplay/investigation.rs:464` - `decide_item_story` / `choose_label`.
- `crates/df-tools/src/gameplay/investigation.rs:1614` - `describe_recall` / `express`.
- `crates/df-tools/src/gameplay/investigation.rs:1681` - `resolve_target` / `choose`.
- `crates/df-tools/src/gameplay/investigation.rs:1999` - `complete_finding` / `choose_option`.
- `crates/df-tools/src/gameplay/investigation.rs:2013` - `complete_finding` / `express`.
- `crates/df-tools/src/gameplay/investigation.rs:2092` - `check_intention` / `express`.
- `crates/df-tools/src/gameplay/investigation.rs:2114` - `check_intention` / `choose_option`.
- `crates/df-tools/src/gameplay/investigation.rs:2222` - `complete_finding_for_question` / `choose_option`.
- `crates/df-tools/src/gameplay/investigation.rs:2239` - `complete_finding_for_question` / `express`.
- `crates/df-tools/src/gameplay/investigation.rs:2279` - `complete_finding_for_question` / `choose_option`.
- `crates/df-tools/src/gameplay/investigation.rs:2534` - `describe_for` / `express`.
- `crates/df-tools/src/gameplay/investigation.rs:2598` - `judge_plain_look` / `choose_option`.
- `crates/df-tools/src/gameplay/investigation.rs:2977` - `describe_plain_for_question` / `express`.

### `crates/df-tools/src/gameplay/investigation_tests.rs`

- `crates/df-tools/src/gameplay/investigation_tests.rs:676` - `played_investigation_and_item_lines_resolve_to_the_target_jev_chooses` / `resolve_investigation_target`.
- `crates/df-tools/src/gameplay/investigation_tests.rs:816` - `target_choice_timeout_is_absent_and_named_by_the_players_words` / `resolve_investigation_target`.
- `crates/df-tools/src/gameplay/investigation_tests.rs:1063` - `registered_search_accepts_repeated_and_different_targets_without_unrelated_findings` / `block_on`.
- `crates/df-tools/src/gameplay/investigation_tests.rs:1132` - `unseen_search_of_companion_pack_uses_companions_passive_perception` / `resolve_investigation_target`.
- `crates/df-tools/src/gameplay/investigation_tests.rs:1185` - `absent_target_key_preserves_apostrophes_and_unicode_without_an_input_panic` / `block_on`.
- `crates/df-tools/src/gameplay/investigation_tests.rs:1195` - `absent_target_key_preserves_apostrophes_and_unicode_without_an_input_panic` / `resolve_investigation_target`.
- `crates/df-tools/src/gameplay/investigation_tests.rs:1218` - `an_absent_notice_is_not_resolved_to_the_door_it_would_be_pinned_to` / `resolve_investigation_target`.
- `crates/df-tools/src/gameplay/investigation_tests.rs:1246` - `looking_around_the_taproom_does_not_select_the_lodging_offer_named_room` / `resolve_investigation_target`.
- `crates/df-tools/src/gameplay/investigation_tests.rs:1271` - `narration_sentence_is_not_promoted_to_an_investigation_target` / `block_on`.
- `crates/df-tools/src/gameplay/investigation_tests.rs:1299` - `searching_an_absent_satchel_does_not_search_its_owners_face` / `block_on`.
- `crates/df-tools/src/gameplay/investigation_tests.rs:1402` - `scene_names_accept_ordinary_synonyms_without_matching_nearby_objects` / `block_on`.
- `crates/df-tools/src/gameplay/investigation_tests.rs:1408` - `scene_names_accept_ordinary_synonyms_without_matching_nearby_objects` / `block_on`.
- `crates/df-tools/src/gameplay/investigation_tests.rs:1445` - `jev_absent_answer_gives_the_absent_target_named_by_the_words` / `resolve_investigation_target`.
- `crates/df-tools/src/gameplay/investigation_tests.rs:1516` - `without_jev_the_target_is_absent_even_when_the_words_name_a_present_thing` / `resolve_investigation_target`.

### `crates/df-tools/src/gameplay/journey/living_scene.rs`

- `crates/df-tools/src/gameplay/journey/living_scene.rs:7056` - `judge_take_target` / `choose_option`.
- `crates/df-tools/src/gameplay/journey/living_scene.rs:7153` - `judge_recipient` / `choose_option`.

### `crates/df-tools/src/gameplay/journey/living_scene_tests.rs`

- `crates/df-tools/src/gameplay/journey/living_scene_tests.rs:1219` - `an_absent_scene_target_is_described_without_drawing_a_die` / `resolve_investigation_target`.
- `crates/df-tools/src/gameplay/journey/living_scene_tests.rs:1258` - `an_ordinary_counter_without_a_finding_is_described_without_drawing_a_die` / `resolve_investigation_target`.

### `crates/df-tools/src/gameplay/journey/merchant_bargaining.rs`

- `crates/df-tools/src/gameplay/journey/merchant_bargaining.rs:62` - `generate_offer_details` / `express`.
- `crates/df-tools/src/gameplay/journey/merchant_bargaining.rs:83` - `generate_offer_details` / `choose_label`.
- `crates/df-tools/src/gameplay/journey/merchant_bargaining.rs:296` - `judge_exchange` / `express`.
- `crates/df-tools/src/gameplay/journey/merchant_bargaining.rs:1246` - `judge_purchase_quantity` / `choose_option`.
- `crates/df-tools/src/gameplay/journey/merchant_bargaining.rs:2442` - `judge_trade_item` / `choose_option`.
- `crates/df-tools/src/gameplay/journey/merchant_bargaining.rs:2733` - `judged_bulk_picks` / `express`.
- `crates/df-tools/src/gameplay/journey/merchant_bargaining.rs:3504` - `hs50_counted_buy_at_summed_list_prices_is_not_a_pending_unit_counteroffer` / `choose_label`.

### `crates/df-tools/src/gameplay/line_cleanup.rs`

- `crates/df-tools/src/gameplay/line_cleanup.rs:574` - `display_line` / `express`.

### `crates/df-tools/src/gameplay/local_lore.rs`

- `crates/df-tools/src/gameplay/local_lore.rs:632` - `decide_role_knowledge` / `choose_label`.
- `crates/df-tools/src/gameplay/local_lore.rs:804` - `judge_fact` / `choose_label`.

### `crates/df-tools/src/gameplay/memory/consistency.rs`

- `crates/df-tools/src/gameplay/memory/consistency.rs:152` - `check_consistency` / `choose_option`.

### `crates/df-tools/src/gameplay/memory/dossier.rs`

- `crates/df-tools/src/gameplay/memory/dossier.rs:110` - `write` / `express`.

### `crates/df-tools/src/gameplay/npc_dossier.rs`

- `crates/df-tools/src/gameplay/npc_dossier.rs:277` - `generate_emotional_profile` / `express`.

### `crates/df-tools/src/gameplay/npc_reply.rs`

- `crates/df-tools/src/gameplay/npc_reply.rs:2196` - `resolve_scene_listener` / `resolve_addressee_in_scene`.
- `crates/df-tools/src/gameplay/npc_reply.rs:2257` - `resolve_scene_listener` / `choose_option`.
- `crates/df-tools/src/gameplay/npc_reply.rs:2307` - `resolve_investigation_target` / `resolve_investigation_target`.
- `crates/df-tools/src/gameplay/npc_reply.rs:2377` - `read_focus` / `choose_label`.
- `crates/df-tools/src/gameplay/npc_reply.rs:2424` - `public_reply_event` / `classify_npc_reply_event`.
- `crates/df-tools/src/gameplay/npc_reply.rs:2717` - `converse_in_character` / `classify_for_dossier`.
- `crates/df-tools/src/gameplay/npc_reply.rs:2819` - `converse_in_character` / `appraise`.
- `crates/df-tools/src/gameplay/npc_reply.rs:3544` - `guard_merchant_reply` / `choose_label`.
- `crates/df-tools/src/gameplay/npc_reply.rs:3606` - `guard_merchant_reply` / `choose_label`.
- `crates/df-tools/src/gameplay/npc_reply.rs:3675` - `guard_item_detail_reply` / `choose_label`.
- `crates/df-tools/src/gameplay/npc_reply.rs:3783` - `guard_service_reply` / `choose_option`.
- `crates/df-tools/src/gameplay/npc_reply.rs:3813` - `guard_service_reply` / `choose_label`.
- `crates/df-tools/src/gameplay/npc_reply.rs:3922` - `model_dialogue_inner` / `express`.
- `crates/df-tools/src/gameplay/npc_reply.rs:4178` - `hear_public_names` / `choose_label`.
- `crates/df-tools/src/gameplay/npc_reply.rs:4306` - `model_public_corrected_inner` / `express`.

### `crates/df-tools/src/gameplay/npc_reply/person_influence.rs`

- `crates/df-tools/src/gameplay/npc_reply/person_influence.rs:209` - `judge_person_influence` / `choose_label`.

### `crates/df-tools/src/gameplay/npc_room_tests.rs`

- `crates/df-tools/src/gameplay/npc_room_tests.rs:818` - `unavailable_identity_models_do_not_invent_a_classification` / `classify_hero_identity`.

### `crates/df-tools/src/gameplay/place_creation.rs`

- `crates/df-tools/src/gameplay/place_creation.rs:116` - `judge_local_reference` / `choose_option`.
- `crates/df-tools/src/gameplay/place_creation.rs:134` - `judge_local_reference` / `express`.
- `crates/df-tools/src/gameplay/place_creation.rs:153` - `judge_local_reference` / `choose_option`.
- `crates/df-tools/src/gameplay/place_creation.rs:166` - `judge_local_reference` / `express`.
- `crates/df-tools/src/gameplay/place_creation.rs:318` - `judge_heading` / `choose_option`.
- `crates/df-tools/src/gameplay/place_creation.rs:364` - `judge_ride` / `choose_option`.
- `crates/df-tools/src/gameplay/place_creation.rs:397` - `judge_ride_service` / `choose_option`.
- `crates/df-tools/src/gameplay/place_creation.rs:1174` - `express_persona` / `express`.
- `crates/df-tools/src/gameplay/place_creation.rs:1191` - `express_persona` / `choose_label`.
- `crates/df-tools/src/gameplay/place_creation.rs:1552` - `express_place` / `express`.
- `crates/df-tools/src/gameplay/place_creation.rs:1557` - `express_place` / `express`.
- `crates/df-tools/src/gameplay/place_creation.rs:1720` - `narrate_travel` / `express`.

### `crates/df-tools/src/gameplay/place_music/ambience.rs`

- `crates/df-tools/src/gameplay/place_music/ambience.rs:209` - `choose` / `choose_label`.
- `crates/df-tools/src/gameplay/place_music/ambience.rs:234` - `choose` / `choose_label`.
- `crates/df-tools/src/gameplay/place_music/ambience.rs:252` - `choose` / `choose_label`.

### `crates/df-tools/src/gameplay/place_music/pacing.rs`

- `crates/df-tools/src/gameplay/place_music/pacing.rs:44` - `review_pacing` / `choose_label`.

### `crates/df-tools/src/gameplay/place_music/spec.rs`

- `crates/df-tools/src/gameplay/place_music/spec.rs:717` - `ask` / `choose_label`.

### `crates/df-tools/src/gameplay/social_tests.rs`

- `crates/df-tools/src/gameplay/social_tests.rs:330` - `jev_picks_a_subject_only_among_the_facts_in_this_adventure` / `classify_for_dossier`.
- `crates/df-tools/src/gameplay/social_tests.rs:348` - `jev_picks_a_subject_only_among_the_facts_in_this_adventure` / `classify_for_dossier`.
- `crates/df-tools/src/gameplay/social_tests.rs:451` - `jev_readings_of_the_played_evidence_set_approach_subject_and_roll_eligibility` / `classify_for_dossier`.

### `crates/df-tools/src/gameplay/storyteller.rs`

- `crates/df-tools/src/gameplay/storyteller.rs:619` - `express_adventure` / `express`.
- `crates/df-tools/src/gameplay/storyteller.rs:638` - `express_adventure_partial` / `express_partial`.

### `crates/df-tools/src/gameplay/storyteller_tests.rs`

- `crates/df-tools/src/gameplay/storyteller_tests.rs:330` - `a_valid_answer_is_returned_cached_and_charged_at_the_reported_usage` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:379` - `an_srd_creature_passes_the_content_judgement` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:416` - `a_non_srd_creature_judgement_rewrites_once_and_degrades_with_an_audit_line` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:462` - `adventure_slots_rewrite_a_judged_non_srd_creature` / `express_partial`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:498` - `a_partial_rewrite_includes_generation_and_judgement_usage` / `express_partial`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:537` - `an_unknown_structured_srd_entity_id_is_refused` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:563` - `a_response_much_larger_than_the_schema_is_still_accepted` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:582` - `an_unparseable_response_is_not_retried` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:601` - `output_missing_the_requested_field_is_retried_then_refused_as_the_wrong_shape` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:634` - `a_normalised_scene_sentence_is_rejected_and_named_for_the_retry_srd_5_2_1_page_184` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:664` - `an_over_length_answer_is_rejected_whole_not_truncated` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:687` - `a_missing_must_mention_term_is_rejected` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:711` - `a_forbidden_term_is_rejected_even_in_a_different_case` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:733` - `a_rejected_first_answer_is_retried_once_with_the_complaint_and_can_succeed` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:764` - `two_rejected_answers_give_a_typed_error_after_exactly_two_calls` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:779` - `a_repeated_identical_request_is_served_from_the_cache_without_a_call` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:786` - `a_repeated_identical_request_is_served_from_the_cache_without_a_call` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:791` - `a_repeated_identical_request_is_served_from_the_cache_without_a_call` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:820` - `a_cached_text_that_no_longer_validates_is_not_served` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:837` - `a_silent_endpoint_hits_the_deadline_and_the_sent_call_stays_charged` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:860` - `a_supplier_refusal_is_typed_and_refunded` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:879` - `an_unknown_supplier_failure_is_typed_and_not_refunded` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:946` - `a_batched_answer_keeps_the_good_slots_and_asks_again_only_for_the_bad_ones` / `express_partial`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:982` - `a_slot_refused_twice_is_reported_while_the_accepted_slot_is_still_returned` / `express_partial`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:1011` - `a_batched_retry_includes_each_refused_slots_validation_reason` / `express_partial`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:1035` - `a_partial_cache_retries_missing_slots_and_merges_the_newly_accepted_slot` / `express_partial`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:1037` - `a_partial_cache_retries_missing_slots_and_merges_the_newly_accepted_slot` / `express_partial`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:1057` - `a_partial_cache_retries_missing_slots_and_merges_the_newly_accepted_slot` / `express_partial`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:1075` - `an_off_storyteller_refuses_without_any_call` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:1092` - `the_adventure_wrapper_uses_the_adventure_purpose` / `express_adventure`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:1127` - `malformed_expressions_are_refused_before_any_call` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:1294` - `scene_narration_grounds_presence_and_injuries_in_scene_facts` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:1382` - `two_identical_concurrent_text_requests_make_one_provider_call` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:1425` - `two_identical_concurrent_batched_requests_make_one_provider_call` / `express_partial`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:1426` - `two_identical_concurrent_batched_requests_make_one_provider_call` / `express_partial`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:1548` - `a_cached_text_with_a_withheld_term_is_not_served` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:1589` - `hs54_caller_rejection_evicts_the_cached_answer` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:1593` - `hs54_caller_rejection_evicts_the_cached_answer` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:1599` - `hs54_caller_rejection_evicts_the_cached_answer` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:1620` - `hs54_valid_multibyte_answers_larger_than_64k_round_trip_the_cache` / `express`.
- `crates/df-tools/src/gameplay/storyteller_tests.rs:1621` - `hs54_valid_multibyte_answers_larger_than_64k_round_trip_the_cache` / `express`.

### `crates/df-ui/src/character_phase.rs`

- `crates/df-ui/src/character_phase.rs:1453` - `update_groups` / `borrow_mut`.
- `crates/df-ui/src/character_phase.rs:1815` - `rejected_and_repeated_updates_preserve_valid_local_selections` / `choose`.
- `crates/df-ui/src/character_phase.rs:1896` - `removed_disabled_and_unknown_offers_cannot_be_selected_or_submitted` / `choose`.
- `crates/df-ui/src/character_phase.rs:1899` - `removed_disabled_and_unknown_offers_cannot_be_selected_or_submitted` / `choose`.
- `crates/df-ui/src/character_phase.rs:1908` - `removed_disabled_and_unknown_offers_cannot_be_selected_or_submitted` / `choose`.
- `crates/df-ui/src/character_phase.rs:1923` - `readiness_is_reported_and_generation_or_owner_changes_reset_ownership` / `choose`.
- `crates/df-ui/src/character_phase.rs:1943` - `readiness_is_reported_and_generation_or_owner_changes_reset_ownership` / `choose`.
- `crates/df-ui/src/character_phase.rs:2091` - `identical_duplicate_is_idempotent_but_equal_revision_conflicts_are_rejected` / `choose`.


## Exact test-module reference index

This index lists test files matching actual generation/fake/qualification symbols in the scanned
source. Inline tests in production modules remain indexed by the feature matrix and call appendix.
No test was executed; references show migration reach, not pass status.

| Test file | Matching source lines |
| --- | --- |
| `crates/df-ai/tests/admission.rs` | 1 |
| `crates/df-ai/tests/expression_request_contract.rs` | 1 |
| `crates/df-ai/tests/lookup.rs` | 1 |
| `crates/df-hosted/src/support_ingest/tests.rs` | 2, 10, 163, 166, 182, 226, 273, 310, 345, 353, 378, 463, 494, 501, 550, 575, 637 |
| `crates/df-provider-api/tests/execution_mode_contract.rs` | 3 |
| `crates/df-provider-api/tests/provider_contract.rs` | 18 |
| `crates/df-provider-api/tests/request.rs` | 3, 7 |
| `crates/df-provider-api/tests/stream.rs` | 7 |
| `crates/df-providers/tests/metering.rs` | 17, 144, 180, 210, 268 |
| `crates/df-providers/tests/native_http.rs` | 4, 10 |
| `crates/df-providers/tests/text_response.rs` | 18, 28, 30, 31, 89, 97, 498, 573, 653, 655, 658 |
| `crates/df-support/src/classification_tests.rs` | 69, 91, 103, 148, 167, 183 |
| `crates/df-testkit/tests/provider_fake.rs` | 3 |
| `crates/df-tools/src/gameplay/adventure/expression_tests.rs` | 1, 141, 204, 259, 317, 357, 422, 470, 479, 488, 493, 768, 1037, 1292, 1294, 1297, 1302, 1305 |
| `crates/df-tools/src/gameplay/approach_model_reply_tests.rs` | 14, 92 |
| `crates/df-tools/src/gameplay/approach_model_tests.rs` | 2, 14, 30, 44, 100, 161, 198, 218, 224, 236, 256, 264, 276, 303, 312, 329, 386, 387, 395, 397, 454, 513, 551, 593, 619, 674, 717, 736, 765, 785, 818, 840, 856, 873, 885, 891, 924, 949, 970, 976, 988, 1001, 1029, 1040, 1054, 1090, 1097, 1104, 1141, 1167, 1196, 1235, 1255, 1263, 1271, 1293, 1296, 1310, 1326, 1342, 1359, 1371, 1377, 1398, 1419, 1443, 1488, 1502, 1531, 1558, 1587, 1614, 1623, 1629, 1639, 1659, 1786, 1829 |
| `crates/df-tools/src/gameplay/arena_opening_tests.rs` | 118, 196, 209 |
| `crates/df-tools/src/gameplay/attack_refusal_tests.rs` | 93 |
| `crates/df-tools/src/gameplay/content_rating_live_tests.rs` | 10, 14, 16, 137, 142, 145, 147 |
| `crates/df-tools/src/gameplay/cost_tests.rs` | 1898, 2497, 2722 |
| `crates/df-tools/src/gameplay/courier_ai_tests.rs` | 3 |
| `crates/df-tools/src/gameplay/creator_portrait_route_tests.rs` | 191 |
| `crates/df-tools/src/gameplay/dev_control/override_tests.rs` | 10, 176 |
| `crates/df-tools/src/gameplay/dialogue_examine_tests.rs` | 13, 14, 117, 321, 446, 447, 454, 565, 587, 589, 648, 673, 681, 692, 800, 803, 810, 860, 861, 862, 886, 896, 897, 960, 972, 973, 1137, 1143, 1207, 1209, 1236, 1237, 1244, 1349, 1351, 1419, 1573, 1794, 1795, 1826, 1827 |
| `crates/df-tools/src/gameplay/dialogue_hs28_tests.rs` | 13, 14, 138, 258, 259, 261, 262, 287, 303, 311, 519, 763 |
| `crates/df-tools/src/gameplay/dialogue_navigation_tests.rs` | 10, 11, 165, 172, 259, 260, 266, 349, 351, 357, 393, 394, 413, 431, 478, 491, 617, 628, 638 |
| `crates/df-tools/src/gameplay/dialogue_opening_social_resume_tests.rs` | 8, 18, 66, 201, 227, 579, 596, 606 |
| `crates/df-tools/src/gameplay/dialogue_tests.rs` | 130, 162, 176, 569, 640, 667, 747, 760, 1144, 2367, 2368, 2369, 2392, 2403, 2405, 2421, 2423, 2678, 2739, 2754, 2770, 2884, 2937, 3253, 3255 |
| `crates/df-tools/src/gameplay/fight_foresight_tests.rs` | 6, 8, 37, 53, 67, 104, 114, 138, 183, 292 |
| `crates/df-tools/src/gameplay/freeform_action_tests.rs` | 3, 73, 74, 77, 86, 115, 116, 220, 221, 287, 288, 329, 332, 400, 401, 452, 463, 599, 653, 679, 1179, 1185, 1221, 1227, 1248, 1254, 1280, 1296, 1322, 1369, 1407, 2073, 2214, 2355 |
| `crates/df-tools/src/gameplay/hero_library_tests.rs` | 384, 385 |
| `crates/df-tools/src/gameplay/imagery/photo_check_tests.rs` | 35, 292, 357 |
| `crates/df-tools/src/gameplay/imagery_tests.rs` | 994, 1258, 1264, 1270, 2619, 2622, 2623, 2765, 2768, 2866, 2887, 2893 |
| `crates/df-tools/src/gameplay/investigation_tests.rs` | 7, 22, 24, 37, 66, 91, 109, 112, 137, 159, 168, 182, 209, 216, 218, 260, 271, 272, 326, 346, 348, 517, 673, 782, 783, 788, 809, 810, 827, 828, 1021, 1038, 1116, 1124, 1175, 1178, 1208, 1211, 1237, 1240, 1264, 1266, 1282, 1294, 1371, 1391, 1416, 1420, 1425, 1440, 1443, 1512, 1515, 1715, 1730, 1732, 1752, 1769, 1770, 1792, 1803, 1805, 2116, 2125, 2127 |
| `crates/df-tools/src/gameplay/journey/living_scene_aftermath_tests.rs` | 9, 318, 319, 398, 549, 652, 655 |
| `crates/df-tools/src/gameplay/journey/living_scene_tests.rs` | 1207, 1212, 1241, 1251, 1762, 1763, 1793, 1800, 1807, 2660, 2685, 2938, 2961 |
| `crates/df-tools/src/gameplay/journey/merchant_bargaining_tests.rs` | 5, 6, 77, 95, 114, 2620, 2666, 2769, 2991, 3010, 3054, 3080, 3094, 3153 |
| `crates/df-tools/src/gameplay/living_scenes/plates_tests.rs` | 4589, 4590, 4591, 4592, 6081, 6088 |
| `crates/df-tools/src/gameplay/memory/tests.rs` | 973, 1014, 1032, 1043, 1051 |
| `crates/df-tools/src/gameplay/npc_dossier_live_tests.rs` | 11, 14, 15, 204, 209, 212, 214 |
| `crates/df-tools/src/gameplay/npc_dossier_tests.rs` | 30, 39, 291, 292, 567, 809, 812, 1004, 1096, 1211, 1239, 1350, 1693, 2582, 2583, 2671, 2672, 2776, 2796, 3064, 3112, 3239, 3308, 3375, 3423, 3483, 3511, 3527, 3660, 3668, 3669, 3706, 3707, 3737, 3738, 3770, 3771, 3796, 3797, 3823, 3824, 3870, 3926, 3927, 3965, 3966, 3995, 3996, 4046, 4047 |
| `crates/df-tools/src/gameplay/npc_room_tests.rs` | 165, 232, 382, 427, 505, 508, 512, 578, 670, 814 |
| `crates/df-tools/src/gameplay/npc_talk_live_tests.rs` | 22, 26, 28, 393, 453, 458, 461, 462 |
| `crates/df-tools/src/gameplay/observation_completion_tests.rs` | 6, 94, 210, 212, 218, 291, 292, 300, 341, 343, 349 |
| `crates/df-tools/src/gameplay/place_creation_tests.rs` | 5, 196, 197, 222, 223, 591, 592, 627, 649, 662, 695, 741, 751, 755, 766, 954, 1006, 1008, 1015, 1023, 1029, 1041, 1197, 1234, 1262, 1263, 1278, 1312, 1313, 1359, 1360, 1417, 1461, 1550, 1551, 1621, 1622, 1674, 1675, 1714, 1720, 1744, 1759, 1800, 1867, 1902, 1934, 1937, 1966, 1989, 1998, 2143, 2412, 2483, 2512 |
| `crates/df-tools/src/gameplay/place_music/pacing_tests.rs` | 3, 5, 11, 15 |
| `crates/df-tools/src/gameplay/place_music/preparation_tests.rs` | 5, 98, 99, 111, 266 |
| `crates/df-tools/src/gameplay/place_music/variant_tests.rs` | 53 |
| `crates/df-tools/src/gameplay/place_music_tests.rs` | 18, 107, 112, 113, 125, 160, 237, 310, 361, 388, 409, 425, 485 |
| `crates/df-tools/src/gameplay/place_tests.rs` | 10, 15, 21, 22, 34, 58, 972, 1038, 1039, 1056, 1057, 1117, 1118, 1138, 1220, 1221, 1280, 1281, 1302, 1329, 1332, 1378, 1405, 1409, 1445, 1446, 1542, 1543, 1544, 1919, 1923, 1992, 2060, 2068, 2222, 2227, 2290, 2534, 2950, 2951, 2994, 2995, 3373, 3386, 3434 |
| `crates/df-tools/src/gameplay/room/rejoin_tests.rs` | 39 |
| `crates/df-tools/src/gameplay/social_tests.rs` | 6, 9, 168, 175, 239, 327, 347, 448 |
| `crates/df-tools/src/gameplay/storyteller_tests.rs` | 37, 78, 87, 194, 282, 283, 324, 362, 388, 420, 437, 480, 513, 542, 557, 573, 586, 592, 607, 620, 640, 658, 670, 680, 693, 702, 717, 727, 758, 768, 774, 807, 831, 841, 851, 866, 873, 883, 937, 973, 995, 1026, 1050, 1062, 1074, 1079, 1084, 1101, 1128, 1275, 1366, 1408, 1534, 1583, 1610 |
| `crates/df-ui/tests/fixtures/phase_assets_fixture.rs` | 200, 207 |

## Bare classify-alias supplemental index

Query: `\.classify\s*\(` across the same 2,851-file source manifest. All 18 candidates
are retained; the following receiver review classifies their actual migration meaning. This
supplement closes the omitted short alias without calling unrelated parser methods model work.

| Exact source | Function / receiver classification |
| --- | --- |
| `crates/df-hosted/src/support_ingest.rs:327` | `admit_mail` - Support workflow wrapper delegates classified/draft work already mapped; same TextService migration. |
| `crates/df-tools/src/gameplay/approach_model_tests.rs:259` | `a_jev_failure_on_classify_returns_the_neutral_reading` - ApproachModel::classify test fixture; preserve semantics through the same API. |
| `crates/df-tools/src/gameplay/approach_model_tests.rs:265` | `a_jev_failure_on_classify_returns_the_neutral_reading` - ApproachModel::classify test fixture; preserve semantics through the same API. |
| `crates/df-tools/src/gameplay/approach_model_tests.rs:332` | `approach_definitions_make_a_fair_favour_bargain_and_a_tip_kindness` - ApproachModel::classify test fixture; preserve semantics through the same API. |
| `crates/df-tools/src/gameplay/approach_model_tests.rs:341` | `approach_definitions_make_a_fair_favour_bargain_and_a_tip_kindness` - ApproachModel::classify test fixture; preserve semantics through the same API. |
| `crates/df-tools/src/gameplay/approach_model_tests.rs:516` | `recorded_errand_bargain_and_false_authority_are_labeled_srd_5_2_1_page_184` - ApproachModel::classify test fixture; preserve semantics through the same API. |
| `crates/df-tools/src/gameplay/approach_model_tests.rs:524` | `recorded_errand_bargain_and_false_authority_are_labeled_srd_5_2_1_page_184` - ApproachModel::classify test fixture; preserve semantics through the same API. |
| `crates/df-tools/src/gameplay/approach_model_tests.rs:559` | `words_are_quoted_data_and_truncated_at_two_thousand_unicode_characters` - ApproachModel::classify test fixture; preserve semantics through the same API. |
| `crates/df-tools/src/gameplay/approach_model_tests.rs:874` | `the_reason_judgement_records_a_reason_stage_in_its_line` - ApproachModel::classify test fixture; preserve semantics through the same API. |
| `crates/df-tools/src/gameplay/approach_model_tests.rs:1490` | `hero_facts_keep_their_facts_whole_when_the_short_fields_are_long_and_reach_the_plain_path` - ApproachModel::classify test fixture; preserve semantics through the same API. |
| `crates/df-tools/src/gameplay/approach_model_tests.rs:1789` | `hs25_flirt_is_not_a_literal_bargain_and_scimitar_address_uses_narration` - ApproachModel::classify test fixture; preserve semantics through the same API. |
| `crates/df-tools/src/gameplay/creator_classification.rs:249` | `actual_jev_adapter_preserves_public_state_and_uses_only_closed_choices` - Inline loopback creator-classifier test, not a new backend. |
| `crates/df-tools/src/gameplay/creator_portraits.rs:537` | `run` - Production creator Classifier -> SystemOne; same central classification purpose. |
| `crates/df-tools/src/gameplay/dialogue.rs:4395` | `unstaged_addressee_reply` - Production ApproachModel::classify in unstaged_addressee_reply -> SystemOne. |
| `crates/df-tools/src/gameplay/npc_reply.rs:2500` | `reply_in_public_report` - Production ApproachModel::classify in reply_in_public_report -> SystemOne. |
| `crates/df-tools/src/gameplay/pack.rs:1240` | `parse_error` - Non-model: serde_json error category in parse_error; no provider migration. |
| `crates/df-tools/src/gameplay/social_tests.rs:180` | `without_jev_every_line_reads_neutral_and_its_words_decide_nothing` - ApproachModel::classify test fixture; preserve semantics through the same API. |
| `crates/df-tools/src/gameplay/social_tests.rs:242` | `jev_decides_the_approach_and_subject_of_a_line_whatever_its_words` - ApproachModel::classify test fixture; preserve semantics through the same API. |
