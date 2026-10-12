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
- [ ] **Follow-up** — Stop remaking the opening plate for each party size (up to 3 requests) while an earlier party's plate keeps running as wasted spend. (play-session.md, PLAY-V12 worker note, Open)
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
- [ ] **Bug** — A failed startup leaves `df_game` and `df_local_demo` behind, so the next start refuses with `retained_access_unavailable` (`crates/df-persistence/src/local_demo_scope/startup.rs`, lines 226-293). (play-session.md, PLAY-V10 worker note, Known defects)
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
