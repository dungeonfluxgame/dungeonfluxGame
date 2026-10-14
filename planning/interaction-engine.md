# NPC and world interaction

Date: 2026-09-30
Status: Outline adopted; source-based social/physical policy and bounds pending

`df-interaction` chooses coherent NPC/social/object reactions; `df-knowledge`
provides observation/belief/memory policy, `df-world` provides causal entity/time
state, `df-intent` provides input proposals, and `df-rules` owns legality/outcomes.
[Runtime directors](runtime-directors.md) fixes single-owner staged composition.

## Persistent social and epistemic model

`NpcState` includes identity/role, versioned personality traits, goals/needs/fears,
emotional state, faction/resources, short-term intent and schedule reference.
`Relationship` has directional trust, affection, respect, fear, suspicion, debt
and familiarity; it is not a single reputation score. These bounded policy units
are not automatic D&D ability/check bonuses. Faction membership/influence connects
individual reactions with collective attitudes through explicit witnessed events.

`CanonicalFact` is objective approved world truth. `KnowledgeGrant` records who
knows a disclosed fact. `Belief` records a claim, confidence, evidence/source,
observed game time, directness, reliability and possible falsehood. Confidence is
a policy score until calibrated. An NPC can lie, misunderstand or conceal without
mutating truth. `WitnessRecord` identifies actual perception/visibility and
participants; a world fact is never a global NPC prompt merely because it exists.

`Memory` stores event/participant references, salience/emotional effect, confidence,
decay class and tags. Retain identity-defining memories under policy; minor detail
can decay or summarize with provenance. Use bounded relevant retrieval rather
than infinite transcripts. Rumor hops create attributed claim transmissions,
possibly distorted under deterministic policy with supplied choices; never
rewrite the original fact. Cap hop count/fanout/TTL and deduplicate causal event +
recipient. Unknown witnesses produce no fabricated social reputation change.
Relationships, beliefs and memories have typed updates and committed provenance.

`ConversationState` has participants, active topic graph, declared goals/mood,
revealed claims, commitments, exit conditions and pending resolutions. Topic
jumps are allowed when valid; no fixed dialogue tree is mandatory. Hidden facts
are server-only references. `SecretPolicy` specifies owner, permitted knowledge,
sensitivity/reveal conditions/exposure risk; a numeric trust threshold alone
cannot authorize a mechanically or socially impossible disclosure. `Obligation`
records debtor/creditor, typed terms, agreement/provenance, due conditions and
active/fulfilled/broken status. A player does not promise something because an
LLM paraphrase said they did; require the approved explicit commitment action.

## Reaction and schedules

`InteractionEngine::react` processes a permitted perception, retrieved beliefs and
memories, emotion/goals, then chooses a bounded motivated intent/action proposal.
The LLM expresses accepted behavior as dialogue/gesture, never invents persistent
personality, knowledge or a successful social check. Emotional state can modify
policy within limits without replacing identity. The story director can request a
credible witness opportunity; interaction returns valid delivery or a reasoned
refusal. NPCs may decline, bargain, redirect, leave or call help as rules/world
conditions permit. Reveals, hostility and promises use source-linked changes.

Important NPC schedules and event overrides are `df-world` due events. Offscreen
activity uses bounded summaries/coarse simulation; it does not run one LLM loop
per NPC. Faction movement, travel/weather/resources and threat clocks remain world
causality. Interaction owns social response proposals, not competing location or
clock state. A witnessed threat can propagate to a guard through a real contact
path and attributed rumor; avoid teleporting omniscient faction awareness.

## Intent, affordances and compound plans

All supported input channels become `InputEnvelope` with actor/run, input or
utterance ID, declared context, content/relevant state basis and channel. Typed
buttons already express an action; natural language is interpreted into candidate
`IntentDisposition`: Action, Question, Social, PlanOnly, Meta, Joke, Clarify or
Rejected. Questions/hypotheticals/jokes never silently execute. Tentative ASR
partials can help low-cost prediction but cannot commit actions; final validated
input and explicit confirmation where ambiguous are required. A supplied gesture
means a supported typed choice, not a new speculative motion-recognition feature.

`WorldEntity` covers NPC, object, location, creature, faction and environmental
system. Authored/source-based affordances identify open/lock/break/listen, carry/
extinguish/ignite, talk/distract/inspect and other permitted actions with relevant
conditions. Server options show high-confidence permitted choices; voice can
propose unsupported combinations, which resolve through available mechanics or
an explicit `NeedsRuling`, not fake general physics.

A bounded `ActionPlan` has ordered stable `PlanStep` IDs, actor/targets, required
choices/resources/time, dependency and cancellation/partial-outcome policy.
Revalidate each step after intervening world/rules changes; consume only resolved
costs. Commit explicit progress and each accepted outcome, then pause for a
reaction/choice/ruling or return `Completed`, `PartiallyCompleted`, `Rejected`,
`Cancelled` or `NeedsClarification`. A multi-step ale/ignite plan may spill liquid
before failing ignition only if those steps are valid and accepted under the
confirmed policy. Do not treat a sentence as one atomic miracle or promise rollback
of already committed steps. Bound step count and continuation lifetime.

Social tactics (persuade/deceive/intimidate/bargain/appeal/comfort/recruit/etc.) are
semantic policy categories mapped to source-defined resolution or permitted
adjudication. The outline's DC17 and +4 debt/+2 memory arithmetic are illustrative
homebrew, not approved 2024 rules. Likewise flammability/door geometry examples
need supported content/mechanics or a visible host ruling. No LLM sets a die,
automatic success, relationship bonus or free extra action. `df-rules` validates
preconditions, modifiers, resources/timing and outcome, including refusal where
success cannot grant the request.

## Acceptance

F35–F38/G10/G11 cover NPC ignorance/false belief/lying, provenance queries, memory
decay, bounded rumor loops, schedule overrides, conflicting goals, secrets/consent,
promise breach, independent faction reactions, narrative refusal, affordance
privacy, jokes/questions, ambiguous input, compound partial failure and stale
ASR/action basis. Replay retains accepted semantics and source/policy versions;
no provider call is needed to recover. Actual two-role playtests observe these
flows, refusals and clear uncertainty instead of treating prompt quality as proof.
## Grounded creative speech and listener-safe generation

`SpeechActPlan` is a committed/proposed typed interaction result: permitted claim IDs, exact outcomes/names/numbers, intent, attribution, declared uncertainty, approved NPC deceit/false-belief claim and audience basis. NPC policy may consider private knowledge server-side, but `ExpressionContext` is separately projected for each listener; unauthorized facts/text/raw secret IDs never enter its provider prompt, retrieval results, style references or asset cues. The same public expression context is identical under paired irrelevant hidden-secret states. Intentional deceit is an approved attributed speech act, not a new canonical fact: inventing the lie's world truth or exposing its hidden true counterpart is forbidden. Scope guards live before every provider input and after generation, not in a second model's promise.

`SpeechEnvelope` contains a bounded ordered clause list of `GroundedClause{claim_id, outcome_slot, entity/name/number_slots, attribution}` and `FlavorClause{noncanonical, allowed_style, local_text}`. Canonical outcome/quantity/negation clauses use reviewed localization templates or deterministic slot rendering; a model can choose permitted expressive variants but cannot paraphrase away constrained meaning. Rich open NPC flavor remains allowed after listener-safe projection and qualification, with typed Uncertain/Rejected outcome for unvalidated claim-bearing additions. Noncanonical flavor cannot define durable locations/quests/resources/rules or promise player commitments; suspected assertion becomes review/rejection instead of silently canonized lore. This preserves natural dialogue without pretending arbitrary natural-language semantics are mechanically proven.

`df-ai::qualify_expression` validates schema, claim/source/access version, allowlisted slots and lengths, named-entity/quantity/negation consistency, instruction/URL/tool prohibition and locale policy; model-based checking is supplementary evidence, never authority. Commit any approved reveal before releasing its text/audio. Default publication buffers the entire bounded response before validated captions/TTS; stable-clause streaming is disabled until model/locale qualification proves each released clause requires no later retraction and its exact approved scope. Rejected/stale/uncertain output uses source-safe prepared expression and matching caption/audio or visible silent gap; no raw tokens reach public captions/TTS first. Qualification freezes model/prompt/source/locale versions and multilingual adversarial fixtures for names, contradictory rumors, lies, numbers, negation, encoded instructions, extraction/injection and secret pairs; report exact denominator, false accepts/rejects, delay and residual error. A zero observed error is not proof of zero future error.

## Source-guided rulings for an inexperienced host

`RulingRequest` binds pending resolution, actor, rules/source versions, source question, understandable reason, safe explanation, permitted typed responses, cost/timing basis and designated adjudicator capability. For representable supported cases, reviewed source-grounded automatic adjudication resolves without human involvement; source discretionary choices remain explicit rather than a universal AI-created DC. `RulingResponse` is permitted choice/explanation, decline, await or unsupported; no unrestricted rule injection. The player may choose a supported alternative or cancel the uncommitted continuation, preserving already committed compound steps/resources. Host cannot force another player's response or operator-only dice.

An absent/disconnected/unready host produces AwaitingAdjudicator and a clear phone/TV notice with permitted alternatives. Logical-time pause is explicit existing authorized campaign policy, never a model timeout. No timeout spends resources, rolls new dice or invents a ruling. Authorized adjudicator transfer follows campaign host permissions/current binding and pending ID; recovery resumes once via operation lookup and retained source basis. A novice-host acceptance flow observes explanation, decline/wait/cancel/alternative, reacquisition and source-correct resume with actual audio and private contexts. Track discretionary interventions/session and minutes; autonomous-GM product copy must disclose remaining human judgment until qualified pilots show real frequency. Full2024 catalog support is still required and does not by itself eliminate tabletop discretion.

## Character emotional profile and relationship state (EMO-01)

An NPC's emotional profile is stored inside the adventure dossier, beside the origin, formative
event, bonds, flaws, fears, motives and knowledge that ground it. The storyteller writes one
bounded profile per dossier from the complete dossier in one structured-output call. Each
attraction, fear, delight, aversion and grudge names the dossier field that supports it. The
server rejects malformed or overlong profiles, a missing or unknown source field, an adult-only
failure, or a value that fails the dossier's bounded profile contract. The prompt forbids claims
that contradict or outrun their cited source. The existing five temperament reactions migrate
into the profile as legacy reactions; renderers and reply prompts show only the unified profile.
An adventure written before EMO-01 has no profile field, so load maps its five stored reactions
into the same runtime profile without changing its attitude or its bytes.

The profile fields are short grounded feeling items (`text`, `because`), an adult-only attraction
scope and orientation, forgiveness (`forgiving` or `unforgiving`), patience (`low`, `middling` or
`high`), brief observable tells, and the five legacy approach reactions. Each feeling cites one
of `origin`, `past_trade`, `formative_event`, `ideal`, `bond`, `flaw`, `want`, `fear`, `talent`,
`story_tie`, `motive`, `stand_down` or `knowledge`; optional dossier fields are valid only when
present. The legacy reactions are compatibility data, not a second personality description.

`social.json` keeps one compact relationship per hero and character: the book's attitude, the
feelings `fond`, `afraid`, `drawn_to`, `resentful` and `wary` at `none`, `a_little` or `much`,
patience (`plenty`, `short`, `done`), the current scene's done marker and an optional permanent
never-forgives reason. Missing emotional state fields load as empty state so older saves retain
their exact attitudes. No state is projected raw. The server speaks a short perceived standing
line; a listening hero can read only that character's visible reaction. Insight may use the
profile's observable tells. A new scene resets `done` and may let the storyteller soften a
forgiving resentment by one level; a never-forgives reason survives every scene and restart.

For each perceived spoken moment, Jev receives the full dossier and profile, the current
relationship in words, the classified approach and subject, the hero's visible description, the
current line/action and the count of similar asks in this scene. One fixed-choice call selects a
reaction and strength; an optional willingness result is part of that same choice when the line
urges an NPC to disclose a held fact or act. Rust maps the choice to the compact state. A profile
that is unforgiving may produce `never_forgives`; otherwise that choice is rejected and becomes
`offended`. The first `done` reaction gets one in-character refusal and subsequent lines from
that hero receive no answer until the scene changes. A permanent grudge makes later Influence
requests unwilling, which the 2024 rules resolve without a check.

The SRD 5.2.1 Influence action remains unchanged: willing requests need no check; unwilling
requests are refused without a check; hesitant requests use the applicable Charisma skill or
Wisdom (Animal Handling) against the higher of DC 15 or the creature's Intelligence score, with
Advantage when Friendly and Disadvantage when Hostile. The server owns the roll and records the
result through the existing check presentation. Emotion decides willingness and attitude only;
it never supplies a bonus, changes the target DC, or grants success. A failed check observes the
book's 24-hour same-urging restriction.

The reply prompt receives the character's unified profile, the state toward this hero, the
latest reaction and only the tells this hero can read. Reply validation rejects banter that
contradicts fear, a reply after `done`, or a refusal/forgiveness claim inconsistent with the
state. Story narration and next-beat offers may use these states; at most one added offer is
allowed, and its kind must already be accepted by the beat script. Expression follows the latest
perceived state. Enemy dossiers use the same profile; combat may consult a versioned Jev choice
only for a legal flee action or surrender offer, leaving attack, movement, timing and outcomes to
the existing combat rules.

When a model is disabled, late or invalid, the already-classified approach uses the dossier's
legacy reaction (warm, cool or steady); it does not invent an orientation, grudge or model-only
reading. The dossier fallback is short and derives from existing traits and those five reactions.
Provider failures are logged as fallback sources and never withhold required rules outcomes.

The profile prompt is `content/adventure-tables/npc-emotional-profile.prompt.md`; the appraisal
prompt is `content/adventure-tables/npc-emotional-appraisal.prompt.md`. Each instruction has a
one-line reason comment. The profile costs one storyteller call per character per adventure;
appraisal costs one Jev call per perceived moment; a scene transition costs at most one storyteller
call per character, and only when a forgiving character has resentment to soften. Calls use the
existing storyteller and Jev deadlines. Combat adds at most one Jev choice at an enemy turn's
start, with the legacy legal-policy fallback.

Tests use the existing structured-text and Jev fakes, not live providers. They cover accepted and
rejected profile fields and grounding, prompt contents, every reaction mapping, patience and
one-time refusal, the permanent-grudge gate, the SRD Influence DC/advantage rules, reply and
narrative state context, accepted offer kinds, enemy policy version/replay, old saves/restarts,
and the adventure-record byte limit. A passing unit suite checks contracts; a running server
playtest must still observe the phone standings, dialogue, notebook note and combat surrender or
flight with keyless fallbacks and again with models enabled.
