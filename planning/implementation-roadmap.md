# Integrated implementation roadmap

Date: 2026-09-29
Status: Planning SQLite provisioned; game runtime/workspace implementation pending

## Dispatch and ownership

Deliver complete, observable slices through the crate boundaries in
[Subsystem architecture](subsystem-architecture.md),
[Subsystem interfaces](subsystem-interfaces.md) and [RPC API](rpc-api.md).
The 47 families in [Feature inventory](feature-inventory.md) remain a broad
backlog; this roadmap does not imply 47 dispatched tasks or 40 empty crates.
Required standard rules exceed the legacy demo; [Rules coverage](rules-coverage.md)
is the source gate for that work.

The coordinator delegates each user work message or scoped atomic work-item
message to one implementing subagent owning the whole result. It does not split
that single item into parallel implementation agents. Separate independent work
messages may run concurrently when dependencies/edit areas/resources allow.
Independent frontier evaluation is a separate completion gate, not a split of
implementation. The coordinator resolves shared gates and integrates sequentially.

The user authorized the complete detailed backlog across every system. Preserve
that source-backed plan now; freeze roughly two to three dependency-ready atomic
tasks per available worker for execution. A task brief names governing sections, fixed input revision,
owning crate(s), permitted edits, hook owners, normal/failure criteria and evidence.
Unresolved prerequisites block affected dispatch rather than invite invented APIs.
Maintain one active owner for protobuf/types, migrations, manifests/composition,
shared UI foundations, source catalogs and build output registration. A slice may
have sequential atomic messages; consumers wait for shared-owner acceptance.

## Prerequisite decisions

| Gate | Owner / output required before dependent implementation |
| --- | --- |
| G01 Toolchain/targets | coordinator + build owner: stable toolchain/language edition/lockfile, native and WASM features, Rust framework candidates and concrete fmt/Clippy/compile commands |
| G02 Transport/runtime | df-rpc-bridge/df-protocol owner: real browser HTTP/2/gRPC driver and executor compatibility, binary WebSocket framing/bounds, all four modes/status/deadline/cancel/half-close; no unsafe browser Send workaround |
| G03 Shared contracts | df-types/df-model/df-protocol owner: concrete typed enums/IDs/errors/offer/view/stream structures, protobuf numbering/version fixtures, effect registration coverage and pure audience scope |
| G04 Identity/devices | df-auth + client owner: credential/bootstrap storage/admission/origin policy, guest/allocation/credential/binding dedupe, bounded stream revocation, host/operator capability rules, supported browsers/phones/tablets/display targets and codecs/capture constraints |
| G05 Durable state/assets | df-persistence/df-assets owner: PostgreSQL physical schema/access-pattern choice, fenced commit/allocation/idempotency retention, migrations/recovery/backups; initial durable byte root and publication/hash/access policy |
| G06 Telemetry | df-observe/df-telemetry owner: supported native/WASM SDK/export path, bounded upload/receipt and TelemetryIngress ports, OTLP/SQLite mapping, authenticated read-only queries, spool/retention/capture boundary and recovery envelope |
| G07 Rules/content | df-content/df-rules owner: source/catalog/errata manifest under rules-coverage.md, chosen launch campaign, legal early subset labeled as such, rules/content revision preservation |
| G08 Providers/presentation budgets | df-provider-api/df-media + client owner: vendors/models and bounded cost/deadline/concurrency policy, execution modes, recording identities, fallback assets, audio/device formats, initial capacity/latency/frame/memory targets |
| G09 Agent runner/evidence | coordinator/df-workflow owner: actual model/tool availability and routing preferences, independent frontier computer-use/vision evaluator and audio observation, tracked phase leases/recovery/edit/resource limits, durable evidence access |
| G10 Campaign authoring and replay | df-model/df-content/df-engine + storage owner: versioned director state/authoring schemas, beat/asset DAG and references/alternative reachability, logical-time/pause rules, ordered decision/semantic/draw provenance, compatible deterministic reducer/migration and replay/hash contract; unsupported versions explicit |
| G11 Causal interaction and bounded directors | df-world/df-knowledge/df-intent/df-interaction/df-narrative/df-encounter/df-combat owners: source-grounded social/physical rulings, epistemic privacy/witness/rumor/memory policy, actor decision/event/candidate/plan bounds, NPC perception/tactics, agency/refusal and compound-step cancellation/partial outcomes |
| G12 Experience, tempo and predictive assets | df-experience/df-tempo/df-presentation/df-media + device/cost owner: permitted pacing observations/consent, versioned intensity/cue/asset-demand APIs, accessibility/audio/UI budgets, hidden-state noninterference, inertia/fatigue/stem timing, fair prefetch/expiry/reservations/waste and measured session/device/cost calibration; stable-clause/barge-in safety |

Select concrete bounds from representative devices/load and verify them; proposed
frameworks/codecs/vendor names are not frozen just because they were researched.
G01/G03/G04/G08 also resolve the persistent Rust/WASM component implementation,
typed presentation variants/cue policies, actual device matrix and frame/input/
memory/audio-drift budgets in [Client presentation](client-presentation.md).
Do not select an unrestricted UI interpreter or a new RPC service to satisfy
ordinary reusable components/in-place updates. The newly requested server
`df-presentation` crate is the semantic director boundary in
[Runtime directors](runtime-directors.md); existing Rust UI crates retain mounted
component/render ownership. Ten runtime additions bring the plan to 40 crates.
G10/G11/G12 block only their affected work, not initial transport/identity delivery.
Initial hosting/auth provider, exact locales, renderer fidelity and advanced optional
rules remain decisions, with dependency gates limited to affected work. The legacy
Go bridge's compatibility is optional unless explicitly selected; preserve its MIT
attribution when translating/reusing code. No dependency installation is authorized
by writing this plan.

## Delivery slices

| Slice | Capability and prerequisites | Implementation/composition owners | Meaningful completion checks |
| --- | --- | --- | --- |
| S00 Contract and execution foundation | G01–G03; start F22/F23/F31/F32 | df-types, df-protocol, df-observe, df-rpc-bridge, minimal df-testkit/df-tools; df-server composition owner | real Rust/WASM browser demonstrates unary/server/client/bidi streaming with errors/trailers, half-close, deadlines and cancellation; bounded flow during simultaneous traffic; target/style gates and dependency checks |
| S01 First durable two-role slice | S00 + applicable G04–G07/G09; F01–F03/F07/F22–F25/F30/F31 | df-auth, df-model/df-engine, df-session, df-persistence, df-api, df-client/ui/player/display/web, df-telemetry; shared wiring by df-server | create/join/ready in display + player; initial idle and same-phase views; rejected action/draft; private projection/asset denial; duplicate/lost-response operation lookup; reconnect/rebind without another member; PostgreSQL restart/fence conflict; end-to-end OTEL query in SQLite |
| S02 Creation and first source-faithful rules | S01 + G07/G08 where portraits/flavor needed; F04/F05/F07/F10 | df-content, df-rules, df-engine; df-api projection, player/display, prepared df-assets/media | legal source-linked build/choices and complete sheet, deterministic check/save, explicit unsupported options, late/failing portrait fallback; accepted results/resources survive restart; rules-source and view correspondence |
| S03 Story, dialogue, voice and audio | S02 + G08/G10 and applicable G11/G12; F06/F08/F09/F15–F17/F25–F28/F35–F39/F42–F44 | df-engine/content, df-world/knowledge/intent/interaction/narrative/experience/tempo/presentation, df-provider-api/df-providers, df-ai, df-media, df-assets, df-api, df-audio/render/player/display; df-server registers every effect | complete scene/NPC flow, typed rejection and microphone fallback; validate before captions/TTS; Talk half-close/cancel; actual audible timed playback and old-buffer cancellation; matching fallback speech; prepared-only/replay make zero live calls on misses; exact usage/complete recording checks |
| S04 Combat, spells and tactical rendering | S02 rules foundations plus S03 for integrated narration, applicable G10/G11/G12; F07/F10–F13/F17/F40/F41 | df-rules/df-encounter/df-combat/df-engine/content, df-session/API, df-render/player/display/audio | initiative, movement/path/visibility, weapon/spell actions, reaction/concentration/condition/health flow, enemy legal inputs, encounter finish; relevant rules interactions and private info; real both-role flow without debug skip; narration cannot decide outcomes |
| S05 Campaign continuity and controls | S03/S04 + G10; F05/F18–F21/F24/F27–F29/F35/F36/F38/F39 | df-rules/engine/session/persistence, df-locale/content, df-ui/API/player/display, df-tools | advancement/rest/resources/rewards and durable journal; public/own knowledge filtering; host-authorized pause/resume/new run/timers/execution mode; reset cancels old owned jobs, request loss does not; locale captions/audio fallback; authorized checkpoints/debug commands/capture |
| S06 Complete required rules/catalog | S02–S05 + complete G07 manifest; expands F04/F05/F10–F12/F20 | df-rules/df-content, coordinated engine/protocol projections and source owner | every required R01–R17 family/catalog entry reaches source-linked implementation/interaction/integration gate; catalog denominator and remaining gaps visible; unsupported options cannot count as support; integrated frontier rules-flow review |
| S07 Conditional fidelity | after reliable flat/audio flow; selected F14/F15 scope + device/assets budgets | df-render/media/assets/tools/display/player | optional splat/3D/LOD/camera/loops/clips share committed scene/timeline; pre-first-frame flat/still fallback, identity consistency, late assets/deduping/cleanup, device frame/byte budgets; no authored JS renderer |
| S08 Delivery and developer operations | grows from S00 onward; final F29–F34 acceptance | df-tools/df-testkit/df-workflow/df-telemetry, df-server, docs/ and about panel | atomic build registration and stable previews; typed operator CLI/fault/capture/telemetry evidence; PostgreSQL migration/backup restore; five-table queue/lease/devlog/escalation/evaluator gates; resource-aware scheduling/cleanup; six-hour all-commit changelog; public docs/attribution |

S00 uses an isolated transport fixture; its success does not claim a production
game server is ready without PostgreSQL, authorized content and required executors.
Freeze only the shared definitions needed by the next slice; introduce `df-model`
when those domain definitions become prerequisites rather than empty scaffolding.

S08 is an incremental track, not a reason to delay observability, testability or
output review until the end. A manual coordinator follows the ADRs before the Rust
workflow runner exists. The public project site remains `docs/`, outside the game
crate graph. S07's selection is independent of complete standard rules support;
unselected fidelity ideas remain conditional and cannot conceal required gaps.

## Gap-refinement integration

[Gap analysis](gap-analysis.md) maps all 76 research backlog rows and corrects the
historical priority labels. X10 resolves private campaign package/template/import
production through df-content/df-tools; the bounded authoring path and real
activation/resume acceptance follow G07/G10, without blocking initial transport.
Hosted remote/mixed-room play is now required F45 core under the later feature
outline. X11 retains conditional async/community/extra-import/broader-homebrew
scope, while F47 plans explicitly disclosed custom-content opt-in. These plans
do not claim runtime implementation. Required progression/inventory
remains S05/S06. Existing F35/F36/F09/R09 families gain WORLD-TIME-ACCEPT,
MEMORY-LONGHORIZON-ACCEPT, SPEECH-ROOM-ACCEPT and RULE-EFFECT-CONTRACT refinement
plans; they exercise the same canonical implementations, not duplicate owners.
See [Campaign authoring](campaign-authoring.md), [Long-horizon state](long-horizon-state.md),
[Rules effects](rules-effect-model.md) and [Expansion boundaries](expansion-boundaries.md).

## New feature priority overlay

[Feature refinement](feature-refinement.md) maps every numbered idea, priority,
required table stake and product phase onto this single roadmap. F45 adds hosted
remote/mixed-room core: S01 remote join/snapshot/reconnect, S03 real private/public
capture/audio topology, S05 campaign resume and source-valid concurrent input.
G04/G08 freeze relevant admission/network/AFK/device/usage bounds in slice order;
full remote voice acceptance does not become a prerequisite cycle before S01.

F46 adds committed observer-safe recaps and clearly speculative spoiler-safe
trailers through journal/presentation/assets/audio after S05 continuity. F47 adds
source-compatible personalized templates and explicitly disclosed custom ruleset/
handler admission; standard 2024 mechanics/catalog completion remains mandatory.
Four targeted acceptance plans extend private phone cues, critical-event cosmetic
escalation, consented spotlight and committed world/item/media continuity.

P0 cinematic eligibility/identity/fallback contracts start in S03/S04. Paid live
video fidelity remains optional, budgeted and tested in S07 after a reliable legal
campaign and measured cost/latency. Cosmetic pauses never change mechanical
reaction/resource/timer authority. Async/public community/marketplace remain
conditional. No second phase roadmap or duplicate implementation owners are added.

## Runtime director delivery within existing slices

S03 first integrates a bounded authored arc with alternative disclosures, one
credible NPC/topic/secret/obligation, source-valid affordance/intent/compound plan,
world-time/threat/schedule, observer knowledge/provenance, macro pacing and
accessible anchored tempo/presentation. Preserve declined opportunities and
player agency. Expand catalogs/scenarios only after the full causal commit/view/
audio path passes; F35–F44 are broad plans, not a request for ten empty skeletons.

S04 adds source-legal encounter objectives/escape states and perception-limited
NPC tactics, reinforcement/hazard requests resolved by the rules owner. Test
reactions/resources and inaccessible hidden knowledge. S05 adds campaign-scale
bounded memory/rumor/thread continuity and snapshot/decision replay with source/
policy versions; replay makes zero provider calls and reports unsupported gaps.
S07 introduces optional higher-fidelity assets/cinematics only after cheap matching
fallbacks, canonical identity, fair bounded prefetch, reserved allowances and
measured waste. Music/SFX/stems and speech safety are core S03 work, not deferred
until 3D exists. S08 includes model/privacy/replay/performance/output acceptance
for all 41 crates. No new subsystem owns a parallel database/timer/provider loop.

## First-slice failure envelope

S01 proves the entire path before broad parallel delivery: RPC -> authorization
-> session owner -> PostgreSQL committed decision -> permitted projection -> real
player/display -> correlated OTEL -> separate SQLite query. Include actual
TelemetryIngress wiring, ingest watermarks/loss visibility and read-only operator
access. Inject temporary telemetry sink failure without freezing gameplay.

Test RPC cancellation/lost receipt after commit while an identified owned effect
continues and completes once. Also test unaccepted input/capture cancellation,
stale run/timer/job callbacks, owner fencing, duplicate allocation/action/credential/
binding writes, revocation during active subscriptions/transfers,
strict destructive preconditions and still-legal offers after unrelated updates.
No fake gameplay storage or no-op registered executor proves this slice. A small
faithful prepared effect is sufficient; paid providers are unnecessary here.
Retired allocation/operation keys must reject rather than create fresh work.
As tabletop adjudication enters scope, test authorized pending-resolution offers,
scoped ruling persistence and visible failure when no authorized ruling is available.

Inspect exact source/native/WASM/config/content/asset identity, screenshots and
interaction outcomes. A desktop viewport does not establish physical phone sleep,
microphone or audible audio compatibility; run the chosen target checks as their
features enter scope. Missing capabilities keep acceptance pending under
[ADR 0005](../ADR/0005-frontier-output-evaluation.md).

S01's browser evidence must also show a persistent mounted shell in both roles:
same-phase/phase updates, tabs/overlays and reconnect require no full-page reload,
valid focus/drafts survive reconciliation, and repeated updates release replaced
resources without duplicate listeners/layers. S03 adds measured and actually
observed audio/caption timing, late assets and interrupted cues. S04/S07 exercise
overlapping motion, reduced motion, visibility recovery and renderer fallback on
the selected device matrix. Carry these requirements into atomic task briefs;
visual smoothness claims need observed interaction and frame-time evidence.

## Optimization and release completion

Once S01 is accepted, create bounded measured optimization work for every integrated
crate under [Optimization](optimization.md). As additional crates/features integrate,
add their passes; retain one 41-crate checklist so tooling/shared libraries and both
clients are not omitted. Profile representative whole-game load before rewriting;
an evidence-backed within-budget result can close a pass without speculative changes.
Repeat affected source/behavior/privacy/telemetry checks after optimization.

Final acceptance maps F01–F47 plus R01–R17 to integrated evidence, explicit selected
scope and unresolved gaps. Preserve meaningful failure checks, measured p95/p99
action-to-view under concurrent audio/assets, provider cost and browser/server
memory/load evidence. Restore from existing PostgreSQL data, verify migration and
telemetry retention/recovery, and operate the complete intended game through both
roles. Code review, catalog counts or native tests alone cannot close the feature.
The coordinator alone records completion after independent evaluation and checked
integration; two unsuccessful economical attempts trigger the existing escalation.

## Service release stages and expensive prerequisites

The full target is now **41 crates, F01–F47, R01–R17, twelve services/41 RPCs**;
commercial authority does not shrink any game/director/remote/phone/cinema requirement.
`DesignReady` requires independent source-bound repair review and corpus consistency,
not production proof. `ExecutableReady` requires G01–G12 applicable library/source/
rights/type/transport choices and native/WASM fixtures before dispatch. `PrivatePilot`
requires source-valid integrated output, isolation, honest ruling/fallback, recovery
and scoped customer consent; supported subsets are labeled and cannot close full game.
`PaidRelease` additionally requires full required rights+coverage, X12 account/payment/
entitlement/ledger, ASR security/effect/deletion/restore/transport/speech/ruling evidence,
measured service bounds and ownership/on-call, funded worst-case supplier exposure,
price/cohort support/CAC/retention evidence. `IncomeValidated` requires actual recognized
revenue, invoices/COGS/support/acquisition/churn and operating cash against the selected
target, independent evidence and ongoing monitoring. Document scores cannot close it.

Next high-risk proof order: full-catalog commercial rights feasibility and browser
native-gRPC/WASM bound proof precede irreversible broad implementation; then shared
types/source fixtures and initial pure integration; accounts/admission and bounded
native effect execution integrate before live paid assets; all47 families/rules and
crate optimization follow existing slices, then full release evidence. Candidate
planning budgets: rights/source feasibility5person-days plus capped external counsel
quote; transport proof5person-days/$0paid calls; initial integration10person-days;
commerce/dispatch/isolation10person-days; recovery/real-device private pilot5person-days.
These are uncertainty estimates (roughly0.5–3x), not a delivery commitment or authority
to purchase counsel, call models or contact customers. At each failure reassess runway
and resolve mechanism/rights before next expensive tranche; don't redefine finalscope.

The twelve ASR review defects are mapped to concrete contracts/tasks in the service
refinement evidence. Independent review records weighted design separately from
observed evidence, source identity and finite remaining blockers. Full service design
can score highly while feasibility, rights, users and revenue remain unproven.

## Open items after the 2026-10-12 play session

Working list of what is broken, unverified, not started, undecided or unmerged at commit
`de32f94`, gathered from `development/handoffs/2026-10-12-known-bugs-on-main.md` (the newest
note, which wins where notes disagree), `development/handoffs/2026-10-12-play-session.md`
(coordinator sections and every worker note) and `development/session-handoff-20261011.json`.
Those files remain the detailed record; each item points back to its source. The formal task
queue is `development/operational-tasks.json` (76 tasks: 57 pending, 16 in review, 3 blocked);
PLAY-B08, PLAY-P06 and PLAY-U03 are named in the notes but are not queued. Items are not
dispatchable briefs: queue a task before work starts. The business items come from research
done on 2026-10-12 and are proposals awaiting the owner.

### Story and dialogue

- [ ] **Bug** — The first character brings up "the thief's song" unprompted and then cannot say who the thief is or what was taken, because characters still do not know the plot (story-spine work unmerged). (known-bugs-on-main.md, Bugs seen in the last playthrough, bug 1, lines 21-23; play-session.md, PLAY-N07 worker note)
- [ ] **Bug** — The character repeats stock lines across answers ("it has to arrive before dawn", "I took it on trust"). (known-bugs-on-main.md, bug 2, line 24; play-session.md, PLAY-N07, room_narration stock phrase "Something is about to happen")
- [ ] **Bug** — The character's face in the plate changes between answers and does not match the line (smiling while saying she is hunted). (known-bugs-on-main.md, bug 3, lines 25-26)
- [ ] **Bug** — A hero's typed line is drawn as a bubble pinned to the other character's figure instead of the hero's. (known-bugs-on-main.md, bug 4, line 27)
- [ ] **Bug** — On a scene change the previous scene's title ghosts through the middle of the frame during the crossfade. (known-bugs-on-main.md, bug 5, lines 28-29)
- [ ] **Bug** — NPC replies answer only one of several sub-questions asked together instead of answering each in order. (play-session.md, PLAY-N07 worker note, coordinator message 3)
- [ ] **Bug** — A private-clue reply can leak onto the TV because privacy is decided from a term list rather than from what the spine has revealed to the table. (play-session.md, PLAY-N07 worker note, coordinator message 2)
- [ ] **Follow-up** — Fix the escort line repeating the contact name, the odd `inn_dialogue` `secret.confirm` when the table never heard the private clue, and the stock `room_narration` phrase. (play-session.md, PLAY-N07 worker note, Known defects)
- [ ] **Follow-up** — Keep spine causes intact in campaign chapters, where `campaign_render.rs` still overwrites opening, followup, victory narration, the inn line and labels. (play-session.md, PLAY-N07 worker note, Not started)
- [ ] **Not started** — Build M3 plot-aware dossiers and reply paths plus M4 menacing rumours and antagonist attitudes, with the live "hard questions" before/after transcripts; no provider call has been made for this. (play-session.md, PLAY-N07 worker note, Not started)
- [ ] **Follow-up** — Finish the mystery-evidence half of the owner feedback: looks change nothing and "the clue" is never explained, with `Spine::inquiry` written but nothing reading it yet. (play-session.md, PLAY-N07 worker note, coordinator message 1)
- [ ] **Owner direction, 2026-10-12** — The story branches off what the players learn. When a character tells a player something, or an investigation turns something up, that knowledge opens a lead the player can review on the phone and act on. Leads read as notes and hints, not as quest markers or instructions; the players can follow them or go their own way, and the patron's errand stays open throughout. (owner message, 16:50)
- [ ] **Not started** — Give every fact a character can voice and every investigation result the leads it opens (a person, a place, an object or an action), grant the knowledge to the player who actually received it, and persist it with the run; start from the campaign layer's `ClueDelivery` leads and the unread `Spine::inquiry`. (owner direction above)
- [ ] **Not started** — Derive a scene's offered choices from open leads as well as from the beat script, resolve typed intents against open leads, and give a followed lead a real consequence (a side scene, a shortcut, an edge in the coming fight or talk, or another way to end the chapter). In the 2026-10-12 interview the patron's two offers never changed across 30 lines. (dialogue-a1/transcript-round2.md)
- [ ] **Not started** — Let an ignored lead resurface through another character or scene detail within the narrative engine's intervention budget, never by forcing the party. (planning/narrative-engine.md, Director pass)
- [ ] **Bug** — Nothing a player says or does moves a character: after propositions, a drawn sword and an assault the patron stayed friendly, no roll was asked for and the scene did not react. Moods, attitude movement and the forced Influence roll for intense flattery, charm, seduction and threats are not wired. (dialogue-a1/transcript-round2.md, lines 19 to 30)
- [ ] **Bug** — Characters cannot answer ordinary personal questions (children, home, service, how they know the cast) because the dossier has no such facts; replies pad with unrelated stock lines and repeat a term of address in every reply. (dialogue-a1/transcript-round2.md)

### Fight

- [ ] **Not started** — Replace the high wide overhead resting camera, which leaves figures very small, with the actor-centred camera the owner asked for (designed only, no code). (known-bugs-on-main.md, bug 6, lines 32-33; play-session.md, PLAY-S03 worker note, Not started)
- [ ] **Bug** — The TV runs one to two turns behind the server (turn label showed "Bandit 3's turn" after the next hero had shot), and the fight label also lags the card by about 3 s between enemies. (known-bugs-on-main.md, bug 7, lines 34-36; play-session.md, PLAY-B03 slice 1 "still wrong")
- [ ] **Bug** — The table log says "swings the greatsword - miss" for a Graze that dealt 3 damage. (known-bugs-on-main.md, bug 8, line 37)
- [ ] **Bug** — The fight title bar shows the story place ("The Orrery Library") over whichever battle scene is loaded, since story and battle scene are not tied together. (known-bugs-on-main.md, bug 9, lines 38-40)
- [ ] **Not started** — There is no kill clip and no dice-roll video at the end of fights (both unmerged, see Unmerged work). (known-bugs-on-main.md, bug 10, line 41)
- [ ] **Bug** — The phone combat screen is still developer drop-downs with dozens of cell and weapon options, and the "Ask" button re-renders while a voice line plays so a tap can miss. (known-bugs-on-main.md, bug 11, lines 42-43; play-session.md, Owner decisions "phone tap-to-move map, no brief yet")
- [ ] **Bug** — In the n=4 acceptance victory run the celebration measured 107 s from finishing banner to aftermath card with two victory-fanfare entries (n=1 gave 6.4 s and one), not investigated. (play-session.md, PLAY-B03 worker note, Known defects)
- [ ] **Not started** — Build the reaction attack as a full beat with an "Opportunity Attack" label plus a "declined" beat, which needs a server flag on the outcome/view that does not exist. (play-session.md, PLAY-B03 worker note, Not started)
- [ ] **Follow-up** — Fix enemy-turn staging limits: enemies that only moved walk after the view's attacks instead of in initiative order, and entering-combat views for an enemy that won initiative have no walk. (play-session.md, PLAY-B03 worker note, Known defects, item 0)
- [ ] **Follow-up** — Add distance to attack cards (needs PLAY-R01 fields) and use the server path of a move instead of only the renderer path. (play-session.md, PLAY-B03 worker note, Not started)
- [ ] **Follow-up** — Tune the scan occupancy thresholds (authored from level 6/level 3 with count threshold 6, untuned) and handle rings for the creature a reaction is about. (play-session.md, PLAY-S03 worker note, Known defects 5-6)

### Fight ending, victory and end card

- [ ] **Bug** — The victory card draws heroes and the character as cut-out figures over a dark plate, cuts the story text off with "....", hangs a "..." speech bubble in mid-air, and hides the "party decides: rest or move on" line behind a hero's card. (known-bugs-on-main.md, bug 12, lines 46-48; play-session.md, PLAY-V14 worker note, baseline observations)
- [ ] **Bug** — The end card keeps the scene's opening title behind its own title so the adventure name shows twice, overlapping. (known-bugs-on-main.md, bug 13, lines 49-50)
- [ ] **Not verified** — The settled-defeat variant of the fight-ending fix (both heroes stable at 0 HP) is native-test only and was never re-run in a browser, and the epilogue frame was never captured. (play-session.md, PLAY-E04 worker note)
- [ ] **Not verified** — The post-fight dead end with a hero still dying (the server refused "Advance 6 seconds - death saves") is fixed on main by merge `22ba05f` and tested natively, but has not been seen in a browser; re-run the next-step browser check (`next-step.mjs`) it had blocked. (play-session.md, Final merges at wrap-up; PLAY-E04 and PLAY-R01 worker notes)
- [ ] **Not started** — Build the campaign hand-over (next chapter via next_chapter, TV transition card, per-run resets, recurring cast) since `chapter_flow::set_label` is empty in campaign mode and offers nothing there. (play-session.md, PLAY-E04 worker note, Not started)
- [ ] **Follow-up** — Carry table settings (`df_local_demo.table_settings`) to the successor run instead of dropping them. (play-session.md, PLAY-E04 worker note, Not started)
- [ ] **Not started** — Build the recomposed victory/defeat screen, the end card, the speaker lower third and the death-save drama (PLAY-V14 M2-M3), including the wire gap that carries no death-save data. (play-session.md, PLAY-V14 worker note, Not started and wire gap)
- [ ] **Bug** — The end card shows at once when `adventure_ended` is set without waiting for the last line, while the thinking mark, scene title and party strip stay visible. (play-session.md, PLAY-V14 worker note, coordinator message 1)
- [ ] **Bug** — With choice chips off, the narrator box stays empty while the table waits because `TvStage::narrator_line` clears the text on `LineState::Done`. (play-session.md, PLAY-V14 worker note, coordinator message 3)

### Pictures, video and audio

- [ ] **Bug** — Plates of one place drift between shots (interior night, then outdoor day), and the phone lobby picture does not match the TV's. (known-bugs-on-main.md, Still open, lines 53-54)
- [ ] **Bug** — The home video loop did not appear in the lobby in the time it took the party to join. (known-bugs-on-main.md, Still open, line 55)
- [ ] **Blocked** — The home loop still jitters at the wrap and the two-clip hand-off is blocked on a server-side video decoder (no codec crate, no ffmpeg, new dependency forbidden; options in the PLAY-V10 worker note). (known-bugs-on-main.md, Still open, lines 56-57; play-session.md, PLAY-V10 worker note, Known defects)
- [ ] **Follow-up** — Compare consecutive real plates of one place for hero sides, weapons and interior/exterior, and feed the sheet's weapon into the plate director via `PartyMemberView.attack_style` since art and sheet can disagree (e.g. longbow on the sheet, greatsword fighter in art). (play-session.md, PLAY-V12 worker note, Not started and open questions)
- [x] **Follow-up** — Stop remaking the opening plate for each party size (up to 3 requests) while an earlier party's plate keeps running as wasted spend. (play-session.md, PLAY-V12 worker note, Open) **FIXED in `dce149c`**.
- [ ] **Not started** — Finish the PLAY-V05 carry-overs: party sizes 2 and 6 and the six-location recapture. (play-session.md, PLAY-V10 worker note, carry-overs)
- [ ] **Follow-up** — Raise the pair video budget default from 4 to 8 since pairs, kill clip and dice template all draw on video. (play-session.md, PLAY-V10 worker note, Budget recommendation)

### Phone and TV interface

- [ ] **Not started** — Replace the phone combat drop-downs with a tap-to-move map; there is no brief yet. (play-session.md, Owner decisions made late in the session)
- [ ] **Not started** — Dispatch the six-player work: the audit at `artifacts/tmp/play-audit/six-players.md` and the brief at `artifacts/tmp/play-p06/brief.md` were never dispatched. (play-session.md, Owner decisions made late in the session)
- [ ] **Not verified** — Confirm the fixed clip logic in `layout-check.mjs` against the real display, since the previous version read every real-page text box as zero-area and reported 0 overlaps while a hero card plainly covered the prompt. (play-session.md, PLAY-V14 worker note, Done but not verified)
- [ ] **Not started** — Add the missing TV-layout scenarios: WebGPU battlefield fights, look result, resolution/epilogue, end card over plate/establishing/imagery-off, and the no-text-on-plate-face rule. (play-session.md, PLAY-V14 worker note, Not started)
- [ ] **Not verified** — Capture phone frames of the establishing picture and run an imagery-off pass plus a fight-scene run with the new client. (play-session.md, PLAY-V12 worker note, Not started)
- [ ] **Not verified** — Check that the phone shows reaction controls without a reload when a reaction prompt fires, which never happened in the worker's runs. (play-session.md, PLAY-B03 worker note, Known defects)
- [ ] **Bug** — The speaker's bubble is drawn over the Rules note/title. (play-session.md, PLAY-V14 worker note, baseline observations)

### Rules

- [ ] **Not verified** — Play end to end with anything beyond the Dwarf Fighter builds (greatsword, longbow): spells, conditions, levelling, inventory, more than two players and every other class are untested. (known-bugs-on-main.md, Still open, lines 61-62)
- [ ] **Not verified** — Run the 4,800-fight release property test on the final rules code, since only earlier code ran at that size. (play-session.md, PLAY-R01 worker note, Done but not verified and Exact next steps)
- [ ] **Not started** — Offer the Help action to stabilise a dying hero (DC 10 Wisdom (Medicine)) to the conscious hero in the aftermath. (play-session.md, PLAY-R01 worker note, Not started)
- [ ] **Not started** — Implement Surprise (Disadvantage on Initiative) and Long Rest effects on a stable hero; Slow/Sap/Vex expiry is covered by df-rules tests only, not by the auditor, and the enemy planner declines its Opportunity Attacks by policy. (play-session.md, PLAY-R01 worker note, Not started)
- [ ] **Follow-up** — Fix the downed hero's save line being spoken as an NPC-role table line. (play-session.md, PLAY-R01 worker note, Known defects)
- [ ] **Blocked** — Report Topple, Cleave and Push in callouts once the rules add the missing masteries, since the engine has only Graze, Sap, Slow, Vex and Nick. (play-session.md, PLAY-B08 worker note, Known defects)
- [ ] **Not started** — Write the PLAY-B08 M2 battle cries and story lines, M3 narration with table setting, and M4 voices including the antagonist's answer; the M1 commit never compiled. (play-session.md, PLAY-B08 worker note, Not started)
- [ ] **Not started** — Play the browser check where one hero drops, the other wins and the fate resolves, extending `development/browser-checks/play-t03-fight.mjs` once `DF_GAMEPLAY_TEST_FIGHT` is deterministic. (play-session.md, PLAY-R01 worker note, Not started)

### Server, saves and start-up

- [ ] **Blocked** — Do not build or bisect inside commits `236b057` through `cfa28f7`, which do not compile on their own; `22ba05f` puts the two misplaced hunks right. (known-bugs-on-main.md, Things to treat with care, lines 65-66; play-session.md, Final merges)
- [x] **Bug** — A failed startup leaves `df_game` and `df_local_demo` behind, so the next start refuses with `retained_access_unavailable` (`crates/df-persistence/src/local_demo_scope/startup.rs`, lines 226-293). (play-session.md, PLAY-V10 worker note, Known defects) **FIXED in `7d0f493`**.
- [ ] **Follow-up** — Rebuild the fixture binary after every content merge since a stale binary fails with `beat-scripts.json ... UnknownHandler`. (play-session.md, PLAY-V10 worker note, Known defects)
- [ ] **Follow-up** — Treat stored databases as pinned to their build: any edit to `journey.rs`, `beat_script*.rs`, `narrative_phase.rs` or `tactical.rs` changes the handler digest and older databases cannot resume. (play-session.md, PLAY-E04 worker note)
- [ ] **Follow-up** — Make the next-step agreement durable and seamless: it currently counts every seated player rather than connected ones, is lost on restart, drops and re-binds the listener between runs, and falls back to the previous database with only a log line on successor failure. (play-session.md, PLAY-E04 worker note, Open)
- [ ] **Not started** — Prove durability across a server stop at the end card and just after the step, plus browser reload checks there. (play-session.md, PLAY-E04 worker note, Exact next steps)
- [ ] **Blocked** — Re-run the end-to-end gameplay qualification with the prepared numeric preadmission diagnostic, since Runtime17 failed before fixture READY with `ADMISSION_CONTROLLER_AND_OBSERVER_CAP` and no numeric term was retained. (session-handoff-20261011.json, end_to_end)
- [ ] **Blocked** — Point `dungeonfluxdnd.com` and `www` at GitHub Pages through GoDaddy (task SITE-GODADDY-DNS), which is blocked and needs verified Pages target, DNS authority and rollback evidence first. (operational-tasks.json, SITE-GODADDY-DNS, stage implementation, status blocked)

### Tests and checks

- [ ] **Bug** — `cargo test -p df-tools --lib` keeps nine long-standing failures (six `gameplay::startup_contract::*`, three `preview::tests::*`), and `gameplay::home_video_tests::*` also time out under heavy load. (known-bugs-on-main.md, Still open, lines 59-60; play-session.md, Where things stand)
- [ ] **Follow-up** — Give the two tactical tests the coordinator changed in `22ba05f` a second look (settled defeat's view carries no fight; `fight()` helper initiative dice fall back to 1). (known-bugs-on-main.md, Things to treat with care, lines 67-69; play-session.md, Checks on 22ba05f)
- [ ] **Not verified** — Decide the fate of `crates/df-render/tests/figure_visibility.rs`, which is ignored and fails with private scans present ("scan-village lod 1 Tactical cell 7:0: 0.16 of the figure is hidden, 0.04 at the finest level"); run with `DF_SPLAT_SCAN_DIR` set and `--ignored`. (known-bugs-on-main.md, Things to treat with care, lines 70-73)
- [ ] **Not verified** — None of the six final wrap-up merges, nor today's two tooling merges, has had an independent evaluation, and the queue refuses `done` without one. (known-bugs-on-main.md, Things to treat with care, line 74; play-session.md, Task queue)
- [ ] **Follow-up** — Reconcile the previous session's queue snapshot (3,198 total: 386 done, 2,804 pending, 6 review, 1 running, 1 blocked) and recover the expired B-C-df-client-D03 CustomerService adapter attempt before further implementation. (session-handoff-20261011.json, queue_snapshot and other_retained_gap)
- [ ] **Follow-up** — Regenerate the PLAY-R01 coverage report at `artifacts/tmp/play-r01/coverage/matrix.md` since it predates the last small edits. (play-session.md, PLAY-R01 worker note)
- [ ] **Not verified** — Re-run the full gates on the merged tips that never got them (PLAY-B03 `445ff0a`/`4d8f2ed`, PLAY-V12 `44c29e9`, PLAY-V10 merged source) and diagnose the PLAY-V15 dice/home-video/imagery failures against the nine-known baseline. (play-session.md, PLAY-B03/V12/V10/V15 worker notes)

### Owner decisions made but not built

- [ ] **Decision** — Remove the grid from the TV entirely, keeping only a thin ring under the acting figure plus a target (sent to PLAY-S03). (play-session.md, Owner decisions made late in the session)
- [ ] **Decision** — Put the camera behind and above whoever is acting, looking across the battlefield and following movement, per the coordinator's "ego centric" reading the owner did not correct (sent to PLAY-S03). (play-session.md, Owner decisions made late in the session)
- [ ] **Decision** — Make the home loop two clips that hand off to each other so the wrap does not jitter (sent to PLAY-V10). (play-session.md, Owner decisions made late in the session)
- [ ] **Decision** — Add fight callouts ("OPPORTUNITY ATTACK"), battle cries, character story lines and sparing Dungeon Master narration, per the brief at `artifacts/tmp/play-b08/brief.md`. (play-session.md, Owner decisions made late in the session)
- [ ] **Decision** — Use battle-scan level 0 on the riverbed as the reference frame and take the second-best level of detail without muddiness. (play-session.md, Owner decisions made late in the session)
- [ ] **Decision** — Make the opening a multi-line server-owned scene (ordered narrator and speaker lines played before the first choice, no timers) for PLAY-U02 M2, per the coordinator's decision. (play-session.md, Owner decisions made late in the session)
- [ ] **Decision** — Commission the six-player game from the audit and the brief with the coordinator's default decisions; it was never dispatched. (play-session.md, Owner decisions made late in the session)

### Decisions needed from the owner

- [ ] **Decision** — Pick the two-clip frame source: retune the prompt to a slow settle at both ends (needs only a ~$0.56 clip), extract the last frame via fal `ffmpeg-api` (needs approval as a new paid media path), or approve a server-side decoder dependency the current rule forbids. (play-session.md, PLAY-V10 worker note, Known defects blocker)
- [ ] **Decision** — Allow downloading the village scan's own finest levels (level 1 ~230 MB, level 0 ~455 MB from the scan CDN) and setting scene `levels` to `[1,10]` if the scan's own second-best is wanted. (play-session.md, PLAY-S03 worker note, next step 5)
- [ ] **Decision** — Choose the FAILED-plate fallback: keep drawing the cut-out stage or switch to an establishing-only fallback. (play-session.md, PLAY-V12 worker note, Open)
- [ ] **Decision** — Settle the `inn_dialogue` `secret.confirm` line ("Yes, the quick way is real") for tables that never heard the private clue. (play-session.md, PLAY-N07 worker note, Known defects)
- [ ] **Decision** — Decide how to reach a six-hero home screen when a server room takes only two players (the layout check stands six cards in the DOM as a stand-in). (play-session.md, PLAY-V14 worker note, Known defects)
- [ ] **Decision** — Disclose or change the silent GM calls: hero-vs-monster initiative ties go to the heroes by spec order, and a dead creature's square does not block movement. (play-session.md, PLAY-R01 worker note, Known defects)
- [ ] **Decision** — Fix the pair view state and cache keys (proposal: keep field 31 with `READY` meaning one clip, add `PAIR = 5`, keep field 32 for A, add 33 for B). (play-session.md, PLAY-V10 worker note, coordinator item 4)

### Unmerged work on branches

The branches below stay on their local branches (`worktree-agent-<id>`, not pushed to origin) because each would break main, as the notes do; about thirty older branches from before this session (`codex/*`, `*-d0*`, older `worktree-agent-*`), each one commit ahead of main, were not looked at today.

| Work | Agent id | Why it is not on main |
|---|---|---|
| PLAY-N07 story spine | a1ba969a4efb78ad7 | Merges and compiles, but 11 more tests fail: scene binding across 200 seeds, saved-run dossier, expression slots, rating check, tactical document pins and five spine checks |
| PLAY-V15 dice-roll video | aff485b5dca448a30 | Conflicts in `gameplay.proto`, `journey/tactical.rs`, `stage_director.rs`; its `CombatOutcome` fields 25-29 clash with the merged 20-32; marked not green |
| PLAY-V13 kill clip | af243eb40681140dd | Conflicts in `gameplay.proto` and the field ledger; never built; calls the video provider at end of fight and has never been run |
| PLAY-I01 examination scene | a4618b6b977070905 | Conflicts in `gameplay.proto`, the field ledger, `adventure/beat_tables.rs`, `beat_script.rs`; never built |
| PLAY-B08 callouts and narration | a5df122ea13e2587f | Conflicts in five files; fields 30-38 clash with the merged ones; never compiled |
| PLAY-E04 next step and chapter flow | abe2121761b0d618f | Conflicts in `beat_script_runtime.rs` and `wire.rs` with the fight-ending fix now on main; marked not green |

- [ ] **Unmerged** — Merge the six branches above in an order that resolves the `CombatOutcome`/`CombatView` field clashes (merged 20-32 vs WIP 25-29 and 30-38) and the `gameplay.proto` ledger conflicts, since PLAY-V15 and PLAY-B08 renumbering blocks pickup. (known-bugs-on-main.md, Leftover branches; play-session.md, Final merges and field numbers)
- [ ] **Follow-up** — Keep `development/operational-tasks.json` PLAY-V13, PLAY-V15, PLAY-B07, PLAY-S03, PLAY-N07 and PLAY-R02 pending (not copied here) and queue the never-queued PLAY-U03, PLAY-B08 and PLAY-P06 before dispatching them. (play-session.md, Task queue)

### Not verified (built, never seen working)

- [ ] **Not verified** — The six final wrap-up merges on `main` were checked (`cargo check`, focused runs, 1,083 passed with nine known failures) but never played live by the coordinator. (play-session.md, Final merges, Checks on 22ba05f)
- [ ] **Not verified** — The PLAY-V12 plate-as-soon-as-it-lands change with predicted layout and raised lower third is unit-tested only and was never run in a browser. (play-session.md, PLAY-V12 worker note, Done but not verified)
- [ ] **Not verified** — The PLAY-B03 slice-two staging (director gate, no snap-back, card waits for walk) compiled with focused tests passing but never had clippy, full lib or schema-ledger gates re-run. (play-session.md, PLAY-B03 worker note)
- [ ] **Not verified** — The PLAY-S03 battle-scene slices were never rebuilt or re-tested after the final integration merge. (play-session.md, PLAY-S03 worker note, Done but not verified)
- [ ] **Not verified** — The PLAY-R01 server COMBAT/audit log lines and table-log lines compile and share paths with tested code but have no dedicated test and were never seen in a running server. (play-session.md, PLAY-R01 worker note, Done but not verified)
- [ ] **Not verified** — The PLAY-V15 dice display (`dice_roll.rs`, `live_dice.rs`/CSS, motion hooks) compiled for wasm but never ran in a browser, and its engine never ran against real providers. (play-session.md, PLAY-V15 worker note, Done but not verified)
- [ ] **Not verified** — The previous session's B-C-df-audio-D02 receipt-fenced captions have no established audible playback, B-C-df-player-D03 has no physical-phone qualification, and B-C-df-web-D04 stays an unmerged review candidate on `codex/web-campaign-panel-20261011-a1` (pushed, clean, gates passed) awaiting browser behavior, review and integration. (session-handoff-20261011.json)
- [ ] **Not verified** — The real microphone path of toggle-to-talk (PLAY-U03) merged with only its label seen live. (play-session.md, Merged to main; known-bugs-on-main.md, Still open)

### Business and service

- [ ] **Decision** — Adopt host-pays-per-campaign subscription plus prepaid top-ups (recommended yes; single receipt per table). (report-v2.md, section 8, decision 1)
- [ ] **Decision** — Accept $59 at a 60% target with a 10-minute support budget (recommended yes; 70%-at-20-min is arithmetically impossible at ≈$67 minimum). (report-v2.md, section 8, decision 2)
- [ ] **Decision** — Use a merchant of record (Paddle or Polar, not Lemon Squeezy until clarified) over Stripe if any EU/UK sale is planned in year one. (report-v2.md, section 8, decision 3)
- [ ] **Decision** — Buy identity (Clerk MRU or WorkOS) rather than building it (recommended yes; both start at $0). (report-v2.md, section 8, decision 4)
- [ ] **Decision** — Gate at 13+ (18+ simplest) with voice-consent confirmation and defer under-13 (`LAWYER`). (report-v2.md, section 8, decision 5)
- [ ] **Decision** — Offer dollar top-ups ($10 minimum, 12-month extend-on-use, non-refundable except where law requires, no pooled wallets at launch; `LAWYER` for expiry/forfeiture per state). (report-v2.md, section 8, decision 6)
- [ ] **Decision** — Rename off the D&D marks (domain, product, marketing) and confirm SRD-only commercial scope with exact attribution before any paid user (`LAWYER`; current domain fails Fan Content Policy three ways). (report-v2.md, section 8, decision 7)
- [ ] **Decision** — Cap support at budgeted minutes-per-campaign with a measured overage path. (report-v2.md, section 8, decision 8)
- [ ] **Blocked** — Before any paid user, clear with counsel: domain rename plus product-name clearance, SRD-only scope, compatibility marketing (including whether "5e" is safe), prepaid expiry/forfeiture per state, e-money and VAT treatment, age gate and voice-consent design, and terms/privacy with AI disclaimer. (research-biz-a2 handoff.md, What needs a lawyer or the owner)

### Platform

- [ ] **Not started** — Windows build and launcher port in progress (task WINPORT-001, 2026-10-12): `df-tools` and `df-telemetry` use Unix-only file APIs and the launch scripts assume macOS tools.

### Start-to-end playthrough, 2026-10-12 evening (room BADENE, seed 0x0d8f36bd0984ec85)

One full game played by the coordinator from the lobby to "The End" in about ten minutes, on the tree at c400f95 plus the stopped workers' uncommitted changes: two players, mature rating, text, voice and pictures on, the test fight off, the village scan forced as the battlefield. The party lost the fight, so this run covers the defeat branch. Sound was not heard by anyone during the run except the owner; sound items come from the display's own state and logs unless marked as the owner's. Evidence: `artifacts/tmp/playthrough-3/issues.txt`, `artifacts/tmp/play-session-1/server.log`.

Owner directions given while watching:

- [ ] **Owner direction** — "the camera needs an xray feature in case it swings behind an object, the player characters need to be cut out so they can be seen". Seen: for a finishing blow the camera sat behind the stockade fence, which filled the screen out of focus with the fight barely visible through it, and stayed there for the next turn.
- [ ] **Owner direction** — "the player characters should look beat up in the display plates when they are low hp". Seen: after the defeat both heroes stood upright and unmarked in the scene picture at 1 of 13 hit points.
- [ ] **Owner direction** — "the defeat screen isnt contextual and I dont hear the DM narrating". Seen: the defeat text is two generic sentences ("Elspeth is dragged away, shouting your names. The way to the stilt house remains closed tonight.") with nothing about the weasels, who fell or how; the only narrator line on that screen ("Darkness takes the field.") is hidden under it, and the closing paragraph is not read aloud.
- [ ] **Owner direction** — "the character icon order seems broken". Checked: the turn strip's order matches the server's, but the highlight jumps from the first enemy to the first hero because the other enemies' turns are never shown, dead and downed combatants stay in the strip as unlabelled dim circles, and the phone lists the party first instead of turn order.

Start-up and lobby:

- [ ] **Bug** — 17 of 54 generated text slots (31%) were refused by validation at start-up and replaced by table text.
- [x] **Bug** — The phone's join page is titled "Gather at the Lantern Wharf" with a harbour picture whatever the adventure's place is; the waiting page then shows the right title over the same harbour picture. **FIXED in `301391e`** (neutral title; showing the real place needs a server field; browser check pending).
- [x] **Bug** — The TV lobby prints "Or enter the code at 127.0.0.1:63319", an address no phone can reach, with no hint that the session is local only. **FIXED in `6b5c691`** (seen in a browser).
- [x] **Bug** — The TV's "Enable sound" control is tiny and in a corner; the game is silent until someone finds it. **FIXED in `dc17a41`** (browser check pending).

Character creation (phone):

- [ ] **Bug** — The field "A detail that makes you memorable" is in the page but collapsed to zero size; it cannot be seen or used. **REOPENED**: the fix in `edece58` is not effective; still zero size when played by hand on 2026-10-13.
- [ ] **Bug** — The form is about 7,100 px tall with twelve groups; four of them offer a single option under "Choose one offered option". Only Dwarf, Fighter and Soldier exist, so every party is the same. **PART FIXED in `e8a800f`**: single options are pre-selected; the heading text and the range of classes are not done.
- [ ] **Bug** — Developer wording is shown to players: "every submitted build is validated on the server", "Fighter B + Soldier B", "Other lawful SRD combinations remain unsupported", "Initiative tie agreement".
- [ ] **Bug** — The sticky "Confirm your hero" bar covers the form text beneath it. **REOPENED**: the fix in `edece58` is not effective; it still covered the Class card on 2026-10-13.

Story scenes:

- [ ] **Bug** — The title card's sentence is repeated word for word in the Dungeon Master box straight after it.
- [x] **Bug** — On every scene change the TV shows the adventure's title again instead of the new scene's, and during the crossfade it sits over the new picture and the health cards. **FIXED in `3720b36`**; the scene titles were seen right by hand on 2026-10-13.
- [ ] **Bug** — The heroes in the scene pictures are tall red-haired humans seen from behind; they are dwarves with different portraits. The patron is a different-looking woman in the second picture. Characters are not consistent between pictures, or with their portraits.
- [ ] **Bug** — The adventure title and place stay printed large over the picture for the whole scene. **REOPENED**: the fix in `3720b36` is not effective in story scenes; seen all scene on 2026-10-13.
- [ ] **Bug** — The whole story is five screens: opening, one follow-up, the fight, the aftermath, the epilogue. Three of the four choices had a single option. There is no journey, no investigation and no use for the private clue.
- [ ] **Bug** — The only choice before the fight is "Face the giant weasel", naming an enemy nobody has seen or mentioned.
- [ ] **Bug** — The loading card says one "Wary Giant Weasel" springs out; the fight has three.
- [ ] **Bug** — The private clue runs narration straight into the patron's first-person speech with no quotation ("Elspeth draws you close. I must reach Jorun Harrow..."), and it is repeated in the phone's story log on every later screen to the end. **PART FIXED in `d4246c4`**: the phone shows the clue once; the missing quotation marks are not done.
- [ ] **Bug** — Choices after the fight are chapter titles, not actions: "The Road Takes Its Toll", "The Clock Never Comes". The phone records them as "Brakka thes Road Takes Its Toll". **PART FIXED in `67b9586`**: the log no longer mangles the titles; the choices are still titles.
- [ ] **Bug** — The phone keeps offering "Confirm original strike" from the aftermath to the end; it is never explained. **PART FIXED in `8fa68c8`**: fixed in the phone menu; the fight panel copy is still to do.
- [ ] **Bug** — A rules log line is shown as a hero's speech bubble on the TV: "BRAKKA: has been stable for 1 hours and regains 1 hit point".
- [ ] **Bug** — The ending contradicts itself: the defeat says the patron "is dragged away"; the epilogue has the survivors report a lost clock and never mention her; the end text says "Elspeth and the survivors must begin again". The contact in the epilogue is not in the scene picture until after the end card is up.

Conversation with the patron (three typed lines, all answered by the model):

- [ ] **Bug** — She cannot say what her own errand's object is or why its deadline matters: "It's the old clock I vowed to bring... I vowed to get it there by dawn".
- [ ] **Bug** — Asked whether she has children, she gives family facts (mother, a betrothed) and does not answer the question.
- [ ] **Bug** — A threat with a drawn greatsword gets "Keep your voice low. I don't know what you're asking me to hide" plus a stock sentence; no reaction to the weapon, no change of attitude, no roll, no change to the picture.
- [ ] **Bug** — The offered choices do not change after talking; nothing she says can be followed up.
- [x] **Bug** — TV: the hero's own typed line is a faded bubble over the patron's figure, partly under her reply. **FIXED in `a1b3e25`** (browser check pending).
- [ ] **Bug** — Phone composer: her reply is shown with a different portrait from the one in the scene card; the send button is labelled "Ask for clarification" under two drop-downs reading "Uncertain intention" and "No action selected". **PART FIXED in `a252a22`**: labels fixed; the reply portrait still differs.

Fight (three giant weasels against two heroes):

- [x] **Bug** — The TV did not show most of the enemies' first round: six beats were resolved before the first hero turn, the TV showed the first (a miss), waited about 25 s, then jumped to the sixth. The two bites that hurt the heroes were never seen. **FIXED in `8d5425b`** (browser check pending).
- [ ] **Bug** — The phone offered Brakka his turn at beat 6 of 6 while the TV's own counter was at 2 of 6. The counter itself went 6, 0, 2 during the entrance.
- [ ] **Bug** — The TV's health cards went backwards: hurt (8 and 7 of 13) before any bite was shown, then full (13 of 13) for 26 s while both phones said hurt, then hurt again.
- [ ] **Bug** — The turn label flickered Brakka, Giant Weasel 1, Brakka within five seconds.
- [x] **Bug** — The music moved to the ending suite 3.4 s before the last bite was shown. **FIXED in `4570a60`** (not yet heard).
- [ ] **Bug** — No dice clip was seen in any frame of the fight; the dice element was on screen for under two seconds per roll. Death saves were rolled five times by the server and never shown at all. **PART FIXED in `0d8934f`**: presented rolls now play their clip (browser check pending); death saves are still not shown as rolls.
- [x] **Bug** — After pressing End turn the phone still read "YOUR TURN" with Move enabled; the move was refused with "That offer has ended. Choose from the current view." **FIXED in `0761a5b`** (browser check pending).
- [x] **Bug** — A downed player's phone says nothing about dying or death saves and offers nothing to do. **FIXED in `0761a5b`** (browser check pending).
- [ ] **Bug** — A bite sounds like a sword: each one logs `sword-swing` then `blade-hit`.
- [x] **Bug** — Music stays ducked for as long as any line or paragraph is on screen, spoken or not: 31 s during a wait for a player, and for the whole of the defeat screen and the end card. **FIXED in `e995a87`** (not yet heard).
- [ ] **Bug** — "FINISHING BLOW / ENEMY VANQUISHED" takes the whole screen for the first of three weasels.
- [ ] **Bug** — Unlabelled round icons float above figures and under health cards; a downed hero is drawn as a dark spiky heap.
- [ ] **Bug** — The phone's fight panel prints developer notes ("Disclosed custom rule tactical.cover-centre-line...", "Disclosed gap giant-weasel: Climb Speed 30 ft is not modelled") and an unstyled status line over the background picture.
- [ ] **Bug** — Attack lists offer weapons the builds do not state: Spear and Shortbow for the greatsword fighter, Scimitar and Shortsword for the archer.
- [ ] **Follow-up** — Balance: three enemies act before either hero each round; the archer fell in round 2 and the fighter in round 3. The player did not use Second Wind or Dodge, so judge this on a second run before changing numbers.

Defeat, aftermath and end card:

- [ ] **Bug** — The defeat screen says "The party decides: rest or move on" and offers one choice, "Advance 6 seconds · death saves", then "Wait · a stable hero regains 1 hit point after 1d4 hours". These are rules steps shown as story choices.
- [ ] **Bug** — The end card leaves the health cards and the last speaker's line visible under it, and the scene picture changes behind it while it is up. **PART FIXED in `3720b36`**: cards, bubbles and choices are hidden; the picture changing behind the card is not done.
- [ ] **Bug** — After "The End" nothing on the TV or the phones offers a new game or a way back to the lobby; the phone still shows the composer. **PART FIXED in `301391e`**: the phone now shows a clear end state; a new-game path is still missing.
- [ ] **Bug** — The phone shows internal notes to the player ("Greatsword Dropped; ownership retained", "defeat does not restore hit points or held weapons") and a hero portrait cropped to the top of a head.
- [ ] **Follow-up** — Cost: 20 pictures (half the session's budget of 40) and 18 voice clips for a ten-minute game.

Seen working in this run: the narrator caption on the loading card; music ducking under a voiced line; model replies for every typed line; personal facts in a character's answers; the phone waiting ("On the TV now…") while the TV caught up; a clean, separate defeat screen (first time seen in a browser); the server starting on Windows without the manual stack patch.

### Second start-to-end playthrough, 2026-10-12 evening (room FUREGA, seed 0x26a35f7dabda7c75): the victory branch

Same build as the playthrough above, played critically at the owner's request ("be critical with me for every dimension"). The party won in three actions, so this run covers victory, the journey's end, the epilogue and the end card. Only what the first playthrough did not already record is listed. Evidence: `artifacts/tmp/playthrough-4/notes.txt`, `artifacts/tmp/play-session-1/server.log`.

Owner observations while watching, with what the logs show:

- [ ] **Owner direction** — "that was the wrong order as that was 2 bow strikes in a row". The order shown matched the server (fighter, archer, third kobold, fighter), but the fighter's thrown javelin is played as a bow shot: each javelin logs bow-draw, bow-release, arrow-flight, arrow-impact on the TV. A thrown weapon needs its own animation and sounds.
- [x] **Owner direction** — "the 3rd creature didnt even get a chance to play". The server resolved Kobold Warrior 3's turn (20 ft move, dagger at the fighter, miss); the TV's own beat log marks it "skipped". The TV sat on beat 2 of 4 for 25 s and then jumped past it when the next hero acted. **FIXED in `8d5425b`** (browser check pending).

Story and writing:

- [ ] **Bug** — The adventure's text generation missed its 25 s start-up deadline ("ADVENTURE prose=patterns") and every line of prose in the session was fallback pattern text. Nothing on the TV, the phones or the lobby says so, and the session is not retried.
- [ ] **Bug** — One leftover sentence, "The herb gatherer now looks to Brakka for support on the way", is glued onto the fight's intro, the victory text, the journey's-end narration, the epilogue and the end text.
- [ ] **Bug** — The opening gives no job, stake or reward: "An old friend, Kelbin, holds a ledger of an old debt for me. Collecting it alone has become dangerous."
- [ ] **Bug** — The plot does not hold together. The job is to collect a debt from Kelbin, who holds the ledger; at the end "Kelbin takes the ledger" and the text says "With the ledger delivered". Kelbin says "Do not blame Rowan" with nothing to blame her for. "Someone held a secret over Pebble the Kobold" is stated at the victory and never explained. Pebble, named in the fight's intro with "a dagger in both claws", is not in the fight (three generic Kobold Warriors) and never appears.
- [ ] **Bug** — Narration contradicts the picture and the events: "Rowan lets out a shaky breath and steadies herself behind you" over a picture of her calmly stirring a cauldron; "Gods, that was close" after a fight in which no enemy touched anyone; the party is "at Brannock's Apothecary" in the victory text and is then offered "Continue to the apothecary".
- [ ] **Bug** — The single choice before the fight is "Stand your ground" with no threat seen, heard or described.
- [ ] **Bug** — Kelbin is described as "a dry, careful old mapmaker" and drawn as an apothecary pouring potions; Rowan, whom the party walked there, is not recognisably in the apothecary pictures.
- [ ] **Bug** — "Short Rest · 1 hour, no Hit Point Dice" is offered first, twice, to two heroes at full hit points.
- [ ] **Bug** — The fight's log line ("Brakka strikes with the javelin — hit, 7 piercing damage") is carried into the phone's story log of the next scene.

Conversation (four typed lines; two model replies, two fallbacks logged `reply_rejected`):

- [ ] **Bug** — Half the replies fell back because the model's answer failed validation twice. The fallback for an accusation was the patron's opening line, word for word.
- [ ] **Bug** — "We don't work for free. What's it worth to you?" got three unrelated character-sheet sentences. The game has no notion of a reward to talk about, although the next scripted line says "I'll make sure you're repaid": scripted lines and conversation do not share what the character knows.
- [ ] **Bug** — After the hero swept the herbs off her table and called her a liar, the picture updated to show the patron smiling warmly, the herbs still on the table, attitude still friendly.
- [ ] **Bug** — A rumour that names the antagonist ("Somebody paid a boy to follow Pebble the Kobold") opened nothing to follow.

Fight and its presentation:

- [x] **Bug** — There is no dice roll on screen. The dice element holds a still image with the number, shown for 1.6 s, 0.4 s and 0.6 s on the three rolls; the tumbling clip does not play. **FIXED in `0d8934f`** (browser check pending).
- [ ] **Bug** — The scene transition and victory fanfare fired before the last shot was loosed (sound log: scene-transition, bow-draw, victory-fanfare, bow-release). **PART FIXED in `4570a60`**: the finale music now waits; the fanfare and transition are still to confirm.
- [ ] **Bug** — In the default view a hero is about 40 px tall on an 800 px wide screen and most of the frame is empty ground; the fight cannot be read from across a room.
- [ ] **Bug** — "ROLL FOR INITIATIVE" shows a tag per combatant, but one enemy's tag is drawn rotated with its number upside down, and the banner overlaps the tags.
- [ ] **Bug** — The greatsword fighter is offered "Javelin (throw)" at 5 ft, which is rolled with disadvantage, with nothing marking it as the worse option beside his greatsword.
- [ ] **Follow-up** — Balance swings wildly: the first playthrough's fight killed the party in three rounds; this one ended in three hero actions (about 45 s) before any enemy was seen to act. Enemy count, hit points and who acts first need tuning against a party of two.

Pacing and layout:

- [ ] **Bug** — The whole game is nine minutes and five screens again; the journey promised by the lobby (a market, a walk to the apothecary) does not exist.
- [x] **Bug** — The lobby shows neither the adventure's title nor what happens next or who starts; "THE PARTY IS GATHERED" is small grey text in a corner. **FIXED in `6b5c691`** (seen in a browser).
- [ ] **Bug** — The title card with the Dungeon Master's box stayed up for at least 18 s on the apothecary transition while a long line was read.
- [ ] **Bug** — The phone's label for a choice differs from the TV's ("Continue to the apothecary · Brannock's Apothecary" against "Continue to the apothecary").

Seen working in this run: the initiative roll-call with a tag per combatant; phones and TV in step during hero turns; the clean, separate victory screen; the pictures of the patron and the contact, which are the strongest thing in the game; the first reply, which gave a clear job and a named rumour; music ducking under voiced lines.

### Third start-to-end playthrough, 2026-10-12 evening (room TAMUDA, seed 0xf232f1de53cf08d4)

Same build, played critically with the owner watching and commenting. The adventure's prose came from the model this time (36 of 55 slots), and the difference shows: the opening gives a clear job, the private clue pays off at the end ("The false trail worked. They are watching the wrong door."), and the plot holds together. Victory in four rounds against three bandits. Only new items are listed. Evidence: `artifacts/tmp/playthrough-5/notes.txt`, `artifacts/tmp/play-session-1/server.log`.

Owner directions given while watching:

- [ ] **Owner direction** — "before wystan talks a speech bubble flashes". Measured at 50 ms steps: a small "intent" bubble (about 46 by 22 px) fades in beside the hero and is gone half a second later; an empty 35 px strip sits at the speaker's place for about 3 s while the reply is generated; the speaker's bubble then appears at 10 percent opacity and about 60 percent size and grows over 0.3 s while the hero's bubble fades out on top of it. A stray "..." bubble was also seen floating over a background figure in the final scene. **PART FIXED in `4c7e69b`**: the typed line now leaves first; the empty strip while a reply is generated is not done.
- [x] **Owner direction** — "on the display plate refreshes the characters dont move". The refresh prompt tells the picture model to keep exactly the same pose, framing, camera distance and lighting and change only the expression, so every refresh within a scene shows the same people frozen in place. Poses do change between scenes. **FIXED in `7ecd7f9`** (needs a paid picture to confirm; browser check pending).
- [ ] **Owner direction** — "if the match idles the DM should make some more commentary, he tries to keep intensity up". Nothing is spoken between beats or while the game waits on a player; waits of 20 s and more are silent.
- [x] **Owner direction** — "it said enemy vanquished then he stood back up". Not caught in a frame by the coordinator; consistent with the display applying a death and then playing an older queued beat for the same figure (see "the dead guy moved" above). In this fight one "vanquished" came from a miss: the greatsword's graze dealt 3 to a bandit with 3 hit points, which the screen gives no way to understand. **FIXED in `8d5425b`** (browser check pending).
- [ ] **Owner direction** — "the guys are walking through the fence, there should be a smart way to block off those grids using the voxel strategy from the supersplat or playcanvas repos". Figures cross the scan's stockade freely; the walkable grid is not derived from the scan. Needs a research pass on how those repositories voxelise a splat scene, then blocked cells and cover taken from it.
- [ ] **Owner direction** — "he rolls hit but doesnt roll damage". Only the d20 is shown; damage appears as a finished number ("Longbow · 9 piercing") and the server's combat record holds no damage dice either.
- [ ] **Owner direction** — "and no opportunity attack when they walked away". Checked against the server: each time an enemy left the fighter's reach without one, he had already spent his reaction on an earlier opportunity attack that round, which is the rule. Nothing on the TV or the phone says the reaction is spent, so it looks like a miss by the game.

New defects:

- [ ] **Bug** — The TV's health card did not follow a heal: Second Wind took the fighter from 4 to 10 on his phone, and the TV showed 4/13 "bloodied" for the rest of that round and all of the next.
- [ ] **Bug** — After moving next to an enemy the fighter's greatsword attack was not offered within four seconds on four turns running, while End turn stayed available; he never attacked on his own turn in the fight. Probably the new "wait for the TV" gate hiding attacks until the move has been shown; needs a manual check, and End turn must not be the only live button while an action is still owed.
- [ ] **Bug** — The TV asked "Brakka: Opportunity Attack?" with Brakka off screen.
- [ ] **Bug** — The fight's intro again names one villain ("Tomas Ingram ... his scimitar dark with old blood") and fields three generic Bandits with crossbows; the victory text then blames Tomas Ingram, whom nobody met.
- [ ] **Bug** — Epilogue lines are shared across adventures word for word: "Your names will be spoken kindly in every tavern from here to the coast", "You came through on your own feet. That is worth more than I can say" and "Rest well. The next road can wait for morning" were said by Kelbin in the previous game and by Calla in this one.
- [ ] **Bug** — Choice labels are still titles and the phone still mangles them: "Brakka calla'ses Thanks", "Brakka thes Lantern's Last Light". **PART FIXED in `67b9586`**: the log no longer mangles the titles; the choices are still titles.
- [x] **Bug** — Eleven rolls in this fight, eleven still dice images, each on screen between 0.4 s and 1.8 s; no tumbling clip in any of them. Two of twenty-six beats were marked skipped by the TV. **FIXED in `0d8934f`** (browser check pending).
- [ ] **Follow-up** — Start-up text generation decides the quality of the whole session: with it the story is coherent, without it (the previous playthrough, deadline missed) every line is filler. It needs a retry or a longer deadline, and the lobby should not open until it has finished or failed for good.

### Recorded speed runs, 2026-10-12 evening (rooms BOSAKA and ZODAPO)

At the owner's request ("record a video and play it as fast as possible") a script drove one TV and two phones in a hidden browser, pressing every choice and taking every fight turn the moment the phone offered it, while recording the TV and one phone. Videos (no sound) and logs: `artifacts/tmp/playthrough-6/` (`speedrun-tv.mp4`, `speedrun-phone.mp4`, `broken-run-tv.mp4`, `run.log`, `broken-run/`). The script is coordinator tooling in the session scratchpad, not in the repository.

- [ ] **Bug** — Pressing choices quickly broke a session for good. In room BOSAKA the script chose "Question Osric closely" and under a second later "Face down the kobold", then pressed that again three times at 0.9 s intervals because the phone still offered it. From then on the server answered everything with `UNAVAILABLE site=authority reason=InvalidCandidate` (73 times in two minutes), the TV and phones showed "The connection to the game was lost", the fight never started and the room never recovered. Whether the trigger is the repeated press or choosing before the previous step had settled is not yet known. A player double-tapping could do this. **PART FIXED in `5abd72e`**: the refused fight behind it is fixed (heroes confirmed out of join order); why a refusal left the room dead is still open.
- [ ] **Bug** — The whole story before the fight can be skipped in 1.6 s: three presses, 0.3 s apart, with no line shown or spoken first. Nothing holds a choice until its scene has been presented.
- [ ] **Bug** — A hero sometimes cannot move and attack in the same turn (intermittent; see the correction in the next section). After moving next to an enemy the greatsword attack was not offered within eight seconds, on four turns in the recorded game; it was offered on the following turn. The fighter loses his action every time he moves.
- [x] **Bug** — The phone keeps showing "YOUR TURN" with Attack and End turn enabled for up to ten seconds after the turn has been spent; the script pressed End turn up to seventeen times in a row. The server took only one action per turn, but the phone gives no sign the turn is over. **FIXED in `0761a5b`** (browser check pending).
- [ ] **Bug** — "Tomas and the Salve" had to be pressed twice: the first press showed "Waiting for the game server…" and the same choice was offered again.
- [ ] **Bug** — Seen in the recording's frames: one almost black, blurred frame in the middle of the fight (the camera inside or against something); heavy smear with colour fringing during strikes; the still dice image with the gold square outline round it.
- [ ] **Follow-up** — A full game played at the phones' pace took 3 min 18 s, of which 2 min 56 s was the fight, and most of that was heroes waiting 11 to 15 s at a time for the TV to finish showing enemy turns. Two of the three enemies died to a "graze" on a missed roll (a 3 and a 6), and seven enemy bites in a row missed, so nobody was hurt.
- [x] **Follow-up** — The recording method works on this machine (headless Edge has WebGPU, so the scan battlefield is in the video). Worth turning into a repository check so every fight change comes with a video. **FIXED in `0a9436f`** (development/browser-checks/play-and-record.mjs; browser check pending).

### Other scans as the battlefield, 2026-10-12 night (rooms KENOKA, SUFARO, ZATOZO)

The owner asked for a session with a different scan in the fight. Two were tried with the same recording script as above, holding 12 s on each story scene. Evidence: `artifacts/tmp/playthrough-7/` (`riverbed-tv.mp4`, `riverbed-phone.mp4`, `sheet-riverbed.png`, `run.log`, `run-refused-valley.log`, `server-refused.log`, `server-riverbed.log`).

- [x] **Bug** — The Hockley Valley scan (`DF_GAMEPLAY_BATTLE_SCENE=scan-valley-bridges`) cannot start a fight against bandits. In room KENOKA (seed 0xb2563e1a9bd2a1a0, three bandits, story place orrery-library) every press of "Bar the thief's way" logged `REFUSAL action=DefendCourier reason=InvalidCandidate status=refused`; the TV kept the choice on screen and nothing else happened. The same seed on the village scan (room SUFARO) started the fight at once. The existing every-scene test passes, but it only stages kobolds. The cause is not known: every error on the fight-start path is reduced to `InvalidCandidate`, so the log cannot say which step refused. Needs a test that starts each adventure creature on both layouts of every scan, and a refusal reason that names the step. **FIXED in `5abd72e`** (the cause was the order the heroes were confirmed in, not the scan; verified on a running server).
- [ ] **Bug** — When the fight cannot start the players are told nothing: the choice stays offered and can be pressed for ever. **PART FIXED in `65ff23c`**: the server log now names the refusing step; the phone still shows nothing.
- [ ] **Follow-up** — The riverbed scan (`scan-riverbed`, room ZATOZO, two guards) works as a battlefield at level of detail 1: the fight started, the ground reads clearly under the light pool and the initiative close-up looks good. The fight was played to the second round and then lost to a fault in the coordinator's script, not the game, so the ending on this scan was not seen. The summit scan was not tried.
- [ ] **Bug** — With a forced scan the fight is still titled after the story place: "THE SALT QUAY" over an autumn riverbed, straight after a card that says the party is on the quay. Acceptable for a private test scene; a scan chosen by a story must carry its own name.
- [ ] **Bug** — One frame in the pane showed the whole battlefield smeared by motion blur during a camera move, with the figures reduced to dark streaks; in the wide view a hero is again about 30 px tall.
- [ ] **Bug** — When both phones went away mid-fight the game stayed on "Brakka's turn" with no time limit seen, and a new browser opening the room's player address was offered only a fresh join, with no way to take the abandoned hero back. A player whose phone dies or loses its storage ends the game for the table.
- [x] **Bug** — Development command line: `dev health` and `dev state` answer "no seat has joined yet" while two browser players are in a fight, and `dev join 1` answers `stale-offer`. Health must not need a seat, and the tool cannot inspect or take over a session that was started from browsers. **FIXED in `7526b4d`**.
- [ ] **Bug** — "SESSION lease renewal failing (Unavailable); retrying" was logged once in the first minute of room ZATOZO and recovered.
- [ ] **Follow-up** — Start-up story text missed its 25 s deadline in three of five server starts this evening (rooms RATOVE, TAKOVU, ZATOZO), so three of five sessions would have been filler prose. With the same seed the story text is not the same twice: seed 0xb2563e1a9bd2a1a0 was titled "Halden's Family Key" on one start and "The Ferry Office Key" on the next.
- [ ] **Follow-up** — Correction to "A hero cannot move and attack in the same turn" above: it is intermittent, not a rule. In rooms SUFARO and ZATOZO the server took a move and then a greatsword attack in the fighter's first turn. In room ZODAPO the server's record shows him moving to a cell next to a weasel (which then bit him without moving) and making no attack that turn, twice; that is the case to reproduce.

### Fourth start-to-end playthrough, by hand, 2026-10-13 (room KESEZA, seed 0x982e5d9d6433d25a)

Played by the coordinator in a browser, one TV page and two phones, from the lobby to "The End", at the owner's request to scour the whole process. Tree at `6d4ab2f7`; text, voice and pictures on; story text from the model (50 of 54 slots); village scan forced by the launcher; victory in three rounds against three giant weasels; no refusal in the server log. All 57 findings, with the earlier rows they repeat, are in `planning/issue-triage.md` section 19 (OBS-01 to OBS-57). The ones that matter most:

- [ ] **Bug** — Between enemy beats the TV falls back to an older state of the fight: an enemy that has just walked up and bitten is back on its old square, the bitten hero's health card returns to its old number, and the heading reads the previous hero's turn before the next enemy's. Every enemy phase, both rounds. (OBS-31)
- [ ] **Bug** — Second Wind never reaches the TV: no beat is shown and the health card keeps the old number until the hero is next hit, or until the fight ends. The server log has no line for it. (OBS-32)
- [ ] **Bug** — The last blow of the fight is never shown; the victory card comes up over the fight's heading and the unshown attack's card. (OBS-33)
- [ ] **Bug** — The phone shows enemy results ahead of the TV, and on a hero's own turn his phone also says "Waiting for Brakka…". (OBS-34, OBS-35)
- [ ] **Bug** — The contact does not know the adventure: she says the heroes have come to collect the ledger they are delivering and that she has no word of the patron who has just thanked them at her door. A threat with drawn steel gets no reaction from the patron. (OBS-25, OBS-22)
- [ ] **Bug** — The short rest is a story choice that heals nobody, can be taken without limit to refill Second Wind, and replaces the victory text on the TV with rules prose. (OBS-48)
- [ ] **Bug** — The phone's fight panel is still lists of 33 cells and up to 37 attack entries, and can pre-select the worst attack. (OBS-43)
- [ ] **Bug** — After "The End" the phones say a new game is started from the TV, and the TV has no control for it. (OBS-53)
- [ ] **Bug** — The TV's four music requests return 404, so the session has no score. (OBS-55)
- [ ] **Follow-up** — The logs cannot explain the TV's faults: the server log records attacks and moves only, and the display keeps no log the command line can read. A beat-by-beat display log (received, started, finished, state shown) is needed before the timeline can be fixed with confidence. (OBS-57)
