# Table engagement model: gauging fit and interest, and adapting the story (PLAY-X02)

Date: 2026-10-11
Status: Research and design only. No code, schema or content changed. Nothing here is approved
until the owner answers section 13 and the coordinator dispatches the tasks in section 11.

Governing documents: AGENTS.md; [Tempo engine](tempo-engine.md) ("Audience, preferences and
observations"); [G12-D01 consented observation](g12-d01-consented-observation-policy.md);
[G12-D02 tempo and accessibility](g12-d02-tempo-accessibility-policy.md);
[Runtime directors](runtime-directors.md) ("Authority and composition", "Pure public boundaries",
"Time, recovery and bounded work"); [Narrative engine](narrative-engine.md);
[Interaction engine](interaction-engine.md); [Adventure generator](adventure-generator.md)
(sections 13-20); [Gameplay tuning](gameplay-tuning.md); [Observability](observability.md);
[Rules support](rules-support.md). Code read for grounding: `crates/df-protocol/proto/gameplay.proto`,
`crates/df-tools/src/gameplay/{actor,dialogue,table_log,tempo,npc_reply}.rs`,
`.../adventure/{campaign,campaign_completion}.rs`, `crates/df-experience/src/{window,pacing,spotlight}.rs`.

Source labels used below: **[S]** = confirmed this session by opening or searching the source;
**[S-abs]** = only an abstract or search summary was read, not the full text; **[B]** = from my
background knowledge, not re-checked here. Statements that are my own inference say so.

---

## 1. The short version

1. **Enjoyment cannot be measured from the inputs this game has, and the repository forbids trying.**
   What can be measured is (a) how the party fared in the game (a fact about the encounter, not
   about a person), (b) what the table says it wants (explicit, voluntary), and, if the owner
   amends policy, (c) which optional things the table chose to do. The design is built on (a) and
   (b), with (c) held in reserve and shadow-tested. The honest name for the output is **fit**, not fun.
2. **Explicit beats inferred, always.** A one-tap "too easy / just right / too tough" and a
   "more of / less of" card at the end of each chapter, plus two setup questions, give more
   reliable information per second of attention than any behavioural proxy. Inference only
   chooses *how much* to move inside what the table asked for, and it has to calibrate against
   those labels.
3. **Tiny, noisy data means: strong priors, slow movement, and "do nothing until confident".**
   Each estimate is a small Bayesian model (a Kalman "local level" filter for challenge, Beta
   counts for appetite) with an informative prior, decay for drift, and a minimum-evidence gate.
   In the worked example a hard fight changes nothing; three in a row move the next encounter by
   one rung.
4. **Steering is a bounded tie-break among options that are already valid.** Challenge moves one
   rung per chapter inside the standard encounter guidelines, decided at planning time, never
   inside a fight. Chapter type is a softly weighted draw among the types the campaign planner
   already allows (no candidate shifts by more than 1.5 times). Nothing touches dice, DCs after a
   roll, hidden modifiers, rewards or rules.
5. **Every adjustment is a recorded decision** (inputs, random draw, policy version, reason code)
   so it replays exactly and shows in a host-only "DM's notes" view, and the host can switch the
   whole thing off.

Main risk: **Goodhart and over-reading.** A strain score or an uptake rate will be mistaken for
"enjoyment", tuned until it looks good, and then quietly steer tables toward a bland median.
Mitigations are the explicit labels as ground truth, the small step size, the asymmetry
(easing is automatic on strong evidence; making things tougher needs the table's own say-so),
the no-op default, and an evaluation plan that admits it cannot prove fun with a few dozen tables.

Recommended first step: **record the signals and show them in DM's notes (E1, E2) and add the
explicit feedback cards (E3) before anything steers.** That is useful on day one, risks
nothing, and produces the labelled data every later step needs. See section 11.

---

## 2. What the repository already decides

### 2.1 Binding rules and what they mean here

| Source | Rule (paraphrased) | Consequence |
| --- | --- | --- |
| G12-D01, "Decision and preserved acceptance" | "No ambient emotion." Do not infer or act on emotion, boredom, attention, engagement, fatigue, enthusiasm, discomfort or social willingness from audio, prosody, camera, biometrics, typing or pointer cadence, silence, inactivity, presence, response latency, action volume "or other behavioral proxies". A model's label from such signals is equally invalid. No inferred affect may alter pacing, NPC behaviour, spotlight, difficulty, rewards or access. | Latency, typing cadence, message volume, text sentiment and prosody are out as engagement signals. No "engagement score" may exist. |
| G12-D01, "Permitted observations and consent" | Permitted: committed in-game interaction counts and elapsed **game** time in a versioned `ActivityWindow` ("a workload fact, not a person rating"); explicit voluntary feedback for a named purpose; a current `SpotlightPreference`. Consent is affirmative, purpose-specific, off by default, withdrawable, never covers another participant; stale or ambiguous consent fails closed. | The consent surface must be per purpose and per participant. Withdrawal must fence new reads. Ordinary play must work after opt-out. |
| G12-D01, "Spotlight behavior" | Opportunities are offered, declinable and ignorable with no penalty, no nagging, no inferred disinterest. Fairness audits the distribution of *offers*, not equal speaking time or acceptance rates. | Spotlight balance is a rule about what we offer, not a judgement of players. |
| Tempo engine, "Audience, preferences and observations" | Inactivity is not proof of boredom, nor laughter proof of low tension. No covert microphone monitoring or emotion inference; no social penalty from local muting. | Same boundary for spoken input. |
| G12-D02 | Inertia, deadbands and hysteresis for cosmetic tempo; legal timing unchanged; accessibility profiles fail closed. | Reuse the inertia idea for story-level adaptation; adaptation never changes timers or legal offers. |
| Runtime directors, "Pure public boundaries" | Directors are pure: immutable basis in, bounded candidate out, explicit logical time, supplied dice, policy revision, causal IDs; `df-experience::recommend` returns a bounded pacing or spotlight request. | The estimators are pure functions in `df-experience`; they propose, `df-engine` and the campaign planner decide. |
| Runtime directors, "Time, recovery" | Replay applies recorded decisions, "ordered draws, source/policy/model versions", without a model call; telemetry is not a recovery log. | An adaptation decision stores its inputs and random draw. |
| Rules support | Standard 2024 rules; no custom modifiers or fudged results without a disclosed custom ruleset. | Difficulty moves only by choosing among standard options. |
| Gameplay tuning, section 1 | Changed numbers/stat blocks are a content revision or a disclosed custom ruleset. | Never edit a creature's HP or AC to adapt; pick a different standard creature or count. |
| Adventure generator, sections 15-16 | Chapter n is generated from the recorded outcomes of earlier chapters; the model writes only typed slots; "anything mechanical: never". | Adaptation inputs become part of the recorded chapter inputs; the model receives typed directives, never engagement labels. |
| Observability | Telemetry is not game truth. | Estimates live in the campaign record, not in telemetry; telemetry only mirrors them. |

### 2.2 The tension with the owner's request

The request is to "gauge enjoyment or interest ... via the inputs the game has". Read literally,
several of the most obvious inputs (turn latency, line length, how much someone talks) are
exactly the ones G12-D01 names as invalid proxies. This document does not quietly route around
that. It sorts every candidate into four evidence classes and says which need an owner decision.

| Class | What it is | Examples | Status under G12-D01 (my reading) |
| --- | --- | --- | --- |
| **A. Game facts** | Measurements of the *encounter*: what happened to the heroes under the rules. About the game state, not the person. | Rounds, HP margins, knockouts, resources spent, defeat, chapter result, rest taken | Not observation of a person and not an affect inference. I read this as permitted. Owner to confirm (question 1). |
| **B. Explicit statements** | What the participant chose to tell us for a named purpose. | Setup preference, creation chips, chapter-end card, favourite-NPC tap, host switches | Permitted ("explicit, voluntary feedback"). |
| **C. Revealed choices** | Which *optional, offered* things the table took up, counted per offer. | Optional NPC question taken or not, side investigation taken or not, approach chosen | Ambiguous. "Committed interaction counts" are permitted, but "action volume ... or other behavioral proxies" are banned as proxies for mood or willingness. I read per-offer take-up as content-preference counting, not mood inference, but this needs an explicit amendment (question 1). Until ratified: record and show in DM's notes only if consented; do not steer. |
| **D. Forbidden proxies** | Anything that would be read as affect, attention or willingness. | Response latency, typing cadence, idle gaps, line length as "richness", sentiment/affect from text, prosody, laughter, crosstalk, camera, biometrics, presence | Excluded by policy. Section 4.3 explains why they are also weak evidence, so nothing of value is lost. |

---

## 3. Research summary

Each item says what it found, what I take from it, and what I refuse. Links are in section 14.
Evidence quality is flagged because much of this literature rests on small studies, and the
popular summaries are often stronger than the evidence.

### 3.1 Dynamic difficulty adjustment (DDA) and drama management

* **Hamlet** (Hunicke and Chapman 2004; Hunicke 2005) [S-abs]: treats difficulty as supply and
  demand of in-game inventory (health, ammo) using inventory theory, adjusting reactively
  (accuracy, damage) and proactively (before spawning). The authors state the cost: DDA moves
  control from designer to code. *Take:* adjust what is *supplied before it is needed*, from a
  performance measure that is a game fact. *Refuse:* any adjustment during an event.
* **Left 4 Dead's AI Director** (Booth 2009) [S for the deck's existence and the "dramatic pacing"
  goal; S-abs for Booth's 2007 interview; the intensity details are [B] because I could not read
  the slides' pacing pages]: it models each survivor's recent *in-game* stress from events such as
  damage taken and nearby enemies, decays it with time, and cycles build-up, peak and relax
  (engine settings gate relax and peak transitions on intensity thresholds [S-abs]). *Take:* the
  intensity is computed from **game events, not from the person**, which is exactly the
  policy-compatible pattern and what our tempo engine already does. Cycle structure (do not stack
  peaks) is a good pacing rule. *Refuse:* copying thresholds; Valve's numbers are for a shooter.
* **Resident Evil 4's hidden rank** [S-abs, community-reverse-engineered]: a hidden 1-10 value moved
  by performance changes damage, spawns and behaviour; Capcom never documented it
  ([Engadget](https://www.engadget.com/2015-06-03-resident-evil-4-difficulty.html) says the
  feature is unconfirmed by Capcom). Speedrunners exploit it by failing on purpose. *Take:* it
  shows a small clamped scalar can work in a shipped game; it also shows exploitation (a table
  that sandbags gets an easier game). Treat the details as folklore with a plausible core.
* **Rubber banding** (Mario Kart): catch-up mechanics help the trailing player but are widely
  criticised as negating good play and as unfair, mostly in opinion pieces and forums [S, low
  evidence quality]. Defenders argue it only closes gaps. *Take:* "never punish success" is not
  optional. *Refuse:* any mechanism in which doing well is visibly taxed.
* **Do players want it?** Andrade et al. (2006) found adaptive balancing more effective at
  satisfying players than static approaches, with caveats about measuring satisfaction [S-abs].
  Smeddinck et al. (CHI 2016) found most players preferred *manual* difficulty choice, and that
  the type of adjustment changes perceived autonomy [S-abs]. Studies on overt adjustment suggest
  it can hurt the experience, motivating objective changes that do not alter perceived difficulty
  (Sakaue et al.) [S-abs]. A small study found players unaware of an adjustment felt more immersed,
  and a thesis compares hidden and disclosed DDA [S-abs]. **Overall: mixed, small samples,
  mostly single-player action or exergames.** None of it is about a living-room table of friends.
* **Flow and challenge** (Csikszentmihalyi 1990 [B]; Chen 2006/2007 [S-abs for the framing]):
  Chen's flOw let the *player* steer difficulty by choice rather than hiding the control. The
  naive "challenge equals skill" channel is too simple: in chess and experience-sampling data,
  stronger opponents and **close games** were enjoyed more than weaker opponents and blowouts
  (Abuhamdeh and Csikszentmihalyi 2012) [S-abs]. *Take:* aim for fights that are close and hard-won,
  not easy; the player-chosen challenge is a first-class input. The widely quoted "85 percent
  rule" (Wilson et al. 2019) is about optimal *learning* error rate in training algorithms and
  does not transfer to fun [S-abs]. *Inference of mine:* I found no primary study giving a
  defensible target win rate for enjoyment in cooperative RPG fights, so the target band here is
  a tunable default, set by the table's stated preference and checked by feedback, not a
  finding.

### 3.2 Player modelling and experience-driven content

* **Experience-driven procedural content generation** (Yannakakis and Togelius 2011) [S-abs]: a
  loop of a player-experience model, a content generator and an evaluation function. The
  experience model should be learned from player data and, per Martinez, Yannakakis and Hallam
  (2014) [S-abs], from **ordinal** (ranked or comparative) labels rather than treating ratings as
  absolute classes. *Take:* ask comparatively and coarsely ("tougher or gentler than before"),
  and treat answers as ordinal.
* **Behavioural clustering** (Drachen, Canossa and Yannakakis 2009, Tomb Raider: Underworld) [S-abs]:
  four play-style clusters from six features across 1,365 completed games. *Take:* clusters are
  a *design-time analysis tool across many players*. *Refuse:* clustering one table's evening;
  the sample is hopeless and per-player labels would also be a profile of a person.
* **PaSSAGE** (Thue et al. 2007) [S-abs]: models a player's preferred play style from in-game
  choices, using style dimensions derived from tabletop player types [B], and picks story events
  to match; a user study showed gains for some player types, not all. *Take:* tag every beat and
  offered option with the pillar it serves and count which tagged options are taken. *Refuse:*
  hidden per-person typing; its evidence is a small study in a single-player RPG.
* **Façade's drama manager** (Mateas and Stern 2005) [S-abs]: sequences hand-authored story beats
  from a large pool to shape tension. *Take:* the pattern of authored beats chosen by a manager;
  this is what `adventure/campaign.rs` already is for chapters.

### 3.3 Tabletop craft

* **Robin D. Laws, *Robin's Laws of Good Game Mastering*** [S for the seven types via secondary
  summaries; the book is [B]]: power gamer, butt-kicker, tactician, specialist, method actor,
  storyteller, casual gamer; players are mixes who shift between series. Laws' practical advice is
  to read what each player writes on the sheet and asks for. *Take:* ask, and read the
  character sheet. Collapse to four pillar preferences rather than seven types.
* **Dungeon Master's Guide** [B]: know your players; the three pillars (combat, exploration,
  social interaction); encounter building by budget; the DC ladder (5, 10, 15, 20, 25, 30, to be
  verified against the pinned 2024 sources). *Take:* levers are budget, composition, terrain,
  objectives and DC rung. Exact 2024 difficulty labels (I understand Low, Moderate, High) and
  budget tables are a rules-support implementation gate, not something this document pins.
* **Bartle** and successors [S]: Yee's factor analyses found components of Bartle's types weakly
  correlated and types overlapping, and a 2024 comparison of typologies recommends continuous,
  situational motivations over fixed type labels. *Take:* no fixed player types; per-pillar
  continuous appetite, per table and per chapter.
* **MDA** (Hunicke, LeBlanc, Zubek 2004) [S-abs]: designers control mechanics; what players feel
  (aesthetics, for example challenge, fellowship, narrative, discovery, expression) is an
  *emergent* outcome. *Take:* we steer mechanics and content; "fun" stays an outcome we ask about.

### 3.4 Implicit signals, validity limits

| Signal | Why it is ambiguous (some of it my inference) |
| --- | --- |
| Response latency | A pause can be thinking, reading, rule-checking, laughing with friends, a phone call, a slow keyboard, a motor or reading difference, or boredom. Group play adds social talk on top. No source I found validates latency as engagement in a living-room tabletop setting. Banned by D01 and by G12-D02's accessibility intent. |
| Line length or richness | Depends on typing effort (phones), age, language, voice vs text, and joke lines. A short reply can be the best line of the night. Counting it rewards verbosity (Goodhart). |
| Sentiment or affect from text | Facial-expression research shows how weakly outward signals map to felt emotion (Barrett et al. 2019) [S-abs]; text is no better and the sample here is a handful of short lines. A child's or a joker's lines are ironic by default. Banned. |
| Take-up of optional interactions | The most informative implicit signal, but confounded by exposure (the first NPC gets more), the rate limit (`npc_reply.rs` allows 6 replies per 60 s), interface salience, group dynamics (one dominant talker), and rules pressure. Needs per-offer denominators. |
| Retry or persistence after failure | Means investment *or* stubbornness *or* nothing else to do; in 2024 rules a retry needs a changed approach, so it is gated. Too sparse to use. |
| Time on scene, skips | Wall-clock; ambiguous like latency. Allowed only as a host-declared scheduling fact (a planned session length), never as mood. |
| Session length, return rate | The strongest long-run interest indicator in the industry, but confounded by schedules and holidays. A product metric (section 10), not a steering input. |
| Disconnects and idle | Operational events. Used only as an *exclusion mask* (a disconnected player did not decline anything). |
| Spoken input | If a microphone is ever consented to, only the **final validated transcript words** may be treated like typed text. No prosody, volume, laughter, crosstalk, or room audio. |

### 3.5 Explicit feedback

* Experience-sampling reviews are mixed on whether more prompts per day lower compliance;
  length of each prompt and total burden matter more than frequency [S-abs; I did not read the
  full reviews]. *Take:* one short card per chapter (about 15 seconds) is within what tabletop
  groups tolerate, but I have no direct evidence for game groups, so measure the skip rate and
  stop asking if it climbs.
* Known response biases (mostly [B]): acquiescence and "just right" ceiling effects; social
  desirability when answering in front of friends or a parent; peak-end effects (the last
  and most intense moment colours a retrospective rating, Kahneman) so ask close to the
  climax *and* label what is being rated; scale-use differences between people.
  *Take:* private taps on each phone, coarse three-way scales, ordinal treatment, "skip" always
  available, never show who voted what.

### 3.6 Statistics for tiny, noisy, drifting data

* **Bayesian updating with informative priors** [B: Gelman et al., *Bayesian Data Analysis*]:
  with n of 2-6 fights, the prior is most of the answer. Beta-Bernoulli for rates; a normal
  model for a continuous residual.
* **Hierarchical pooling** (partial pooling, Efron and Morris shrinkage) [B]: a table's estimate
  is pulled to the pool; strength of pull falls as the table's own data grows. Here the pool is
  a versioned global prior (from designers at first, from consenting tables later).
* **Exponential weighting and local-level models** [B]: a Kalman local-level filter is
  closed-form, adds a drift term for non-stationarity, and is explainable.
* **Bandits and Thompson sampling** (Chapelle and Li 2011) [S-abs]: Thompson sampling is simple
  and competitive. **Constraint of mine:** a per-table bandit over 16 chapter types cannot learn
  from 8 pulls. Learning must happen over 3-4 pillar features, with strong priors, and across
  tables offline. The draw is used for variety and bounded exploration, not as an optimiser.
* **Change detection** (Adams and MacKay 2007) [S-abs]: full Bayesian online change-point
  detection is overkill for at most 8 data points; the Kalman drift term plus a "two surprises
  in a row, widen the variance" rule is the proportionate version.
* **Regression to the mean, multiple comparisons, Goodhart's law** [B]: an extreme fight
  (very lucky dice) tends to be followed by a less extreme one even if we do nothing, so
  "we eased it and it got better" proves little. Testing many features finds false patterns.
  A measure that becomes the target stops measuring (Goodhart; Manheim and Garrabrant's
  taxonomy). These drive: minimum evidence gates, a handful of pre-specified quantities,
  explicit labels as the audit, and the control arms in section 10.

---

## 4. Signal inventory

Columns: **Class** (A game fact, B explicit, C revealed choice, D forbidden proxy; section 2.2);
**Norm.** is how to normalise it. "Default" says whether the signal may be recorded and used
by default under G12-D01 as I read it.

### 4.1 Inputs the game has today

| Signal | Where it lives today | What it may indicate | Noise / ambiguity | Normalise per player / table | Class and default |
| --- | --- | --- | --- | --- | --- |
| Rounds per fight | `CombatView.round` (proto); engine has no round cap | Fight length vs the encounter's design length | Dice variance is large; one bandit vs two heroes at level 1 | Divide by the encounter's expected rounds from the simulator; table-level only | A. Default. |
| HP margin (lowest hero fraction, party attrition) | `PartyMemberView.hit_points/maximum_hit_points`; `CombatOutcome.target_hit_points` | How close the fight was | Class/AC differences; max HP 13 for fighters; one hero tanks | Fractions of max HP; use the party minimum and the party sum | A. Default. |
| Knockouts and defeat | `CombatOutcome.knocked_out`; `ChapterResult::{Victory,Defeat,Abandoned}` | Fight went badly; chapter outcome | Knockout leaves 1 HP, nonlethal, in the demo; "Abandoned" may be a life event | Counts per hero, per party size | A. Default. `Abandoned` recorded but never read as dislike. |
| Resources spent | `SECOND_WIND` action, Short Rest taken (`ChapterOutcome.rested`) | Attrition of limited resources | Using Second Wind early is a style choice | Fraction of the party's total uses | A. Default. |
| Attack outcomes | `CombatOutcome` (die, total, AC, hit, critical, damage) | Luck vs build | Pure dice noise | Not used directly; the reference model absorbs it | A. Recorded for simulation only. |
| Approach chosen | `ASK_COURIER` vs `ESCORT_COURIER`; `ChapterOutcome.approach`; `antagonist_spared` | A coarse social-vs-martial choice at a fork | Two buttons; position and wording effects; who pressed it | One observation per fork, per offer shown | C. Not steering until ratified; recorded in DM's notes if consented. |
| Typed line disposition | `DialogueDisposition` (Question, Social, Action, PlanOnly, Meta, Joke, Clarify, Rejected) from `df-intent` | Whether the table talked to an NPC at all | Classifier error; "Meta" and "Joke" are often play, not intent; rate limit 6 replies / 60 s caps visible counts | Count of committed Question/Social lines **per offered NPC opportunity** | C. Counts only. Same as class C above. |
| Typed line text and length | Text is **not retained** in `dialogue.rs`; `TableLine` ring keeps 24 lines of 160 characters, presentation only | Richness of roleplay | See 3.4 | n/a | D. Not stored. |
| Table log lines (`hero_index`, `kind`) | `table_log.rs` in-memory ring | Who spoke or acted | Message volume | n/a | D as a proxy. May support spotlight *offer* balance only through offer counts, not speech counts. |
| Time between prompt and action | Actor/operation timing (not currently stored as a signal) | Thinking, distraction, access needs | Hopelessly ambiguous | n/a | D. Not collected. |
| Reconnects, idle | Session/transport events | Connection problems, breaks | Operational | Used as an exclusion mask | Operational only. |
| Tempo cue | `TempoCue` (intensity permille, phase) from the public display projection | What *we* showed | It is our output | May be a covariate in analysis, never a response | Not an input to the estimators (circular). |
| Character creation choices | `CharacterSelection`, ties, background | Declared tastes ("a sheet is an order", Laws) | Defaults and ease of choice dominate in a fixed demo build | Treat as weak prior only if a taste question is added (E3) | B only if explicitly asked. |
| Examine action (`EXAMINE_HARBOR_SEAL`) | In the proto; a test-only check, unreachable from the room today | An optional exploration take-up | Not reachable yet | Per offer | C when reachable. |
| Chapter-level facts | `ChapterOutcome`, clock, destination reached | Campaign progress | Chapter count is small (about 8) | Per chapter | A. Default. |
| Spotlight offer outcomes (accepted, declined, expired) | `df-experience::spotlight` | Audit of *our* offers | Low response is not disinterest (D01) | Offer-level audit only | Audit. Never read as interest. |

### 4.2 Inputs arriving soon

| Signal | Source when it arrives | Use | Class and default |
| --- | --- | --- | --- |
| Ability-check results and DCs | Roadmap item 3; `AbilityCheckResult` in the proto | Challenge fit for non-combat scenes (success rate against reference); persuasion outcome | A. Default. |
| N-creature encounters, rounds per fight, defeats | Roadmap item 4 | Richer challenge fit (kills per round, creatures left) | A. Default. |
| Optional NPC topic chips (the nine dossier topics, section 19.7) | Dossier adapter, PLAY-N02 | The cleanest take-up signal: topic chips offered vs tapped | C. |
| Beat-script branch choices (investigate, confront, sneak, talk) | Roadmap item 2 | Pillar take-up at a fork with a denominator | C. |
| Exploration and search actions | Roadmap 6 | Optional exploration take-up | C. |
| Spoken input | Voice policy | Final transcript words classified like typed lines | C (counts). No prosody. Separate consent. |
| One-tap reactions, favourite NPC, chapter-end card, setup preference | Section 7 | Primary steering inputs | B. |
| Reading the sheet, opening panels | Client UI events (thin client; none recorded) | Would be UI cadence | D. Do not add. |

### 4.3 Signals deliberately not used

Response latency, typing or pointer cadence, line length, text sentiment, laughter or crosstalk,
prosody, camera or biometrics, presence, speaking-time shares, and wall-clock time on scene as
mood. Reasons: policy (G12-D01), weak validity (3.4), Goodhart incentives (a game that rewards
verbosity teaches verbosity), and fairness (slow input, a quiet child, a non-native speaker, or
someone using assistive input must never look "less interested"; G12-D02). Dropping them costs
little because the cleaner signals above carry nearly all the usable information.

---

## 5. Estimated quantities

### 5.1 Conventions shared by every estimator

* **Unit of analysis is the table, per chapter.** A per-hero value exists only where it is a game
  fact (a hero's HP) or for spotlight offer counts. There is no per-person enjoyment model and
  nothing persists about a person between campaigns by default.
* **Per-offer denominators.** Any take-up rate counts opportunities *offered* (denominator) and
  *taken by at least one consenting contributor* (numerator). This neutralises exposure and
  volume effects.
* **Contributors.** Only participants whose consent covers the purpose are counted in class C
  counts. A non-consenting participant's actions still happen and still affect game facts, but
  add nothing to take-up counts.
* **Informative priors.** Every estimator starts from a versioned designer prior (later a pooled
  prior from consenting tables) with a stated strength in pseudo-observations.
* **Decay.** Challenge uses a drift term; Beta counts decay by multiplying both parameters by
  `lambda = 0.8` per chapter *toward the prior* (an exponential forgetting that keeps prior mass).
* **Minimum evidence gate.** Nothing influences anything until its gate passes. The default
  output is `NoChange`.
* **Explicit overrides.** A class B statement sets direction and target; inference adjusts
  magnitude inside the bounds the statement allows (section 7).
* **Uncertainty is carried and shown.** Every estimate ships with its posterior spread and
  evidence count to DM's notes.

### 5.2 Summary of quantities

| Quantity | Statistic | Prior | Update | Decay | Uncertainty | Minimum evidence before it may steer |
| --- | --- | --- | --- | --- | --- | --- |
| Challenge fit (`delta`) | Table's offset in logit-strain from a reference table, per standard encounter rung | `N(0, 0.5^2)` | Kalman local-level update on each fight's residual | Drift variance `q^2 = 0.05` per fight | Posterior sd | At least 2 fights and `P(abs(delta - delta*) > 0.25) >= 0.70`; tougher also needs table consent (5.3) |
| Pillar appetite (`a_k`, k = combat, social, exploration, puzzle) | Probability the table takes an offered optional opportunity of pillar k | Beta(3,3) per pillar (strength 6), later pooled | Beta-Bernoulli per offer, plus explicit vote multiplier | `lambda = 0.8` per chapter toward prior | Beta credible interval | At least 4 offers in the pillar across at least 2 chapters; explicit votes can substitute (needs a majority of voters) |
| Spotlight balance | Offers made per hero this chapter vs the table mean | Uniform | Pure bookkeeping of what we offered | Per chapter reset, carried deficit capped at 2 | n/a (a count) | None; it is a scheduling rule, but personal hooks need `SpotlightPreference` |
| Pacing fit | `{slower, same, faster}` plus a host-declared session length | `same` | Explicit votes only | Chapter-level, decays after 2 chapters | n/a | One explicit vote moves one notch |
| NPC affinity (per NPC, per table) | Probability the table returns to an NPC when able, plus explicit favourite taps | Beta(2,4) (low prior; most NPCs are not returned to) | Beta-Bernoulli per "could return" opportunity; favourites add 2 pseudo-successes | `lambda = 0.8` | Credible interval | At least 3 return opportunities; or any explicit favourite |

All numbers in this table are illustrative starting values. They are to be calibrated against the
simulator and early labelled sessions (section 10) and versioned as a policy.

### 5.3 Challenge fit

**What it measures.** How hard the last encounter was for *this* table relative to a reference
table facing the same standard encounter rung. It is a measure of the encounter's outcome, not of
anyone's feelings. Whether that is the right place to be is decided by the table's stated
preference (the target below).

**Step 1: per-fight strain `S` in [0, 1].** A weighted sum of game facts, each already in [0, 1]:

```
S = 0.35 * (1 - lowest_hero_hp_fraction)      // closest call
  + 0.25 * party_hp_attrition                 // 1 - end_party_hp / start_party_hp
  + 0.20 * heroes_dropped / party_size        // knocked out at least once
  + 0.10 * limited_resources_spent_fraction   // Second Wind uses etc.
  + 0.10 * clamp((rounds - expected) / expected, 0, 1)
S = 1.0 if the party was defeated or the chapter was abandoned mid-fight (flagged, see below)
```

The weights are a starting guess. The simulator (section 10) checks that `S` is monotone in what
a human would call "harder" and the early labelled sessions regress the weights toward the
table's "too tough / just right / too easy" taps, which is the audit against Goodhart.

**Step 2: residual from a reference.** The offline simulator plays the *same encounter rung* with
reference tables and records the typical strain `S_ref(rung)`. Illustrative: Low 0.30, Moderate
0.45, High 0.62. The observation is

```
r = logit(S) - logit(S_ref(rung))        // positive = harder than the reference table found it
```

This is the key trick: it separates "we dealt a hard encounter" from "this table is struggling".

**Step 3: filter.** Model `delta_t = delta_(t-1) + eta`, `eta ~ N(0, q^2)`, `r_t = delta_t + e`,
`e ~ N(0, sigma^2)`, prior `delta_0 ~ N(0, 0.5^2)`, `sigma^2 = 0.81` (to be set from simulated
dice variance), `q^2 = 0.05`. Closed form per fight:

```
v' = v + q^2                       // drift (skip on the first fight)
v_new = 1 / (1/v' + 1/sigma^2)
m_new = v_new * (m / v' + r / sigma^2)
```

**Step 4: target and decision.** The table's target strain `S*` comes from its preference:
gentler 0.35, as written 0.45, tougher 0.55 (a deadband of 0.10 on each side). Compute the
target offset `delta* = logit(S*) - logit(S_ref(Moderate))`: 0 for "as written", about -0.42
for gentler and +0.40 for tougher (a table that wants it tougher should find a Moderate fight
harder than the reference table does). The gate:
`P(delta - delta* > 0.25) >= 0.70` allows **easing by one rung**;
`P(delta - delta* < -0.25) >= 0.70` allows **one rung tougher, and only if the table's preference
is "as written" or "tougher" (never on inference alone against a "gentler" vote)**. Otherwise
`NoChange`. The rung chosen is the one whose predicted strain
`S_hat = logistic(logit(S_ref(rung)) + m)` is closest to `S*`, moved at most one step.

**Worked example (the demo party: 2 fighters, 13 HP each, one bandit; table preference "as written").**

Fight 1 (Moderate rung, `S_ref = 0.45`). Facts from the committed record:

| Fact | Value | Term |
| --- | --- | --- |
| Lowest hero HP fraction | 1/13 = 0.08 (knocked out once, wakes at 1 HP) | 0.35 * 0.92 = 0.322 |
| Party attrition | start 26, end 8 -> 0.69 | 0.25 * 0.69 = 0.173 |
| Heroes dropped | 1 of 2 = 0.5 | 0.20 * 0.5 = 0.100 |
| Resources spent | Second Wind 1 of 4 uses = 0.25 | 0.10 * 0.25 = 0.025 |
| Rounds | 6 vs expected 3 -> capped 1.0 | 0.10 * 1.0 = 0.100 |
| Result | party won | S = 0.72 |
| Recorded but **not used**: median turn latency 38 s, one 4-minute gap | | forbidden proxy |

Residual `r = logit(0.72) - logit(0.45) = 0.944 + 0.201 = 1.145`.
Update from the prior (`m = 0, v = 0.25`): `v_new = 1/(4 + 1.235) = 0.191` (sd 0.44),
`m_new = 0.191 * (1.145/0.81) = 0.27`. Gate: `P(delta > 0.25) = 0.52`. **No change.** One brutal
fight is dice plus build; the posterior moved from 0 to 0.27 with wide spread.

Fight 2 (Moderate again, `S = 0.65`, `r = 0.82`). With drift, `v' = 0.241`; `m = 0.40`,
`sd = 0.43`. `P(delta > 0.25) = 0.63`. **No change.** Two fights, still under the gate.

Fight 3 (Moderate again, `S = 0.70`, `r = 1.05`). `v' = 0.236`; `m = 0.54`, `sd = 0.43`.
`P(delta > 0.25) = 0.75 >= 0.70` and the minimum of 2 fights is met. **Gate passes:** ease.
Predicted strain on the next rungs: Moderate `logistic(-0.20 + 0.54) = 0.58` (outside
`0.45 +/- 0.10`), Low `logistic(-0.85 + 0.54) = 0.42` (inside). **Step down one rung to Low for the
next chapter.** The same step is taken next chapter only if evidence again clears the gate.

A party defeat sets `S = 1.0`, flagged `defeat`. It is a legitimate observation, but because the
residual would be large and the dice variance is high, a defeat can *never alone* open the gate:
the "at least 2 fights" rule applies, and a defeat additionally triggers the existing
fail-forward rules (narrative engine) and the villain clock (adventure generator section 13) already
change the story after a defeat.

*Check on the opposite case:* a table that crushes every fight with `S = 0.15` has
`r = -1.53`. After three such fights the filter says `delta` is about `-0.7`. If their preference
is "as written", one rung tougher is allowed; if it is "gentler", nothing changes; the table's own
word outranks the inference.

*Interpretation limit (mine):* this measures fights. Until skill checks and non-combat scenes
exist, challenge fit says nothing about investigation or social difficulty.

### 5.4 Pillar appetite

**What it measures.** For each pillar, how often the table took up an *optional* opportunity that
was offered, among all such opportunities offered, plus what it said it wants more or less of.
Pillars: combat, social, exploration or investigation, puzzle. (Today only combat and social
exist in the playable engine; the others wait on the beat-script interpreter.)

**Estimator.** Per pillar k, over the last chapters, Beta counts
`Beta(3 + taken_k, 3 + offered_k - taken_k)` (prior strength 6, about two chapters of evidence),
with the decay of 5.1. Take-up is binary per offer: at least one consenting contributor took it.
Explicit votes (7.2) multiply the used value: net "more" from a majority of voters is `x 1.3`,
net "less" `x 0.7`, never overriding the bounds below.

**Worked example: social appetite, a parley chapter with three heroes and the patron NPC.**

| Evidence | Value | Used? |
| --- | --- | --- |
| Optional topic chips offered by the NPC (who are you, why do you care, what are you hiding, ...) | 5 | yes (denominator) |
| Taken by at least one hero | 3 | yes (numerator) |
| Typed lines classified Question or Social | 4 lines | only as the same 3-of-5 take-up; count of lines is **not** used |
| Characters per line (22, 61, 14, 90) | | forbidden as engagement |
| Seconds spent in the scene (about 9 min) | | forbidden as mood |
| End-of-chapter card, "Talking to people": more 2, same 1, less 0 | net more from 2 of 3 voters | yes, multiplier 1.3 |

Posterior from the Beta(3,3) prior: `Beta(6, 5)`, mean `0.545`, sd `0.144`, roughly a 95 percent
interval of 0.27 to 0.81. Gate (4 offers across at least 2 chapters): this is one chapter, so on
inference alone **nothing happens yet**; the explicit "more" from a majority satisfies the
substitute rule, so the used value is `0.545 x 1.3 = 0.71`.

How it steers (illustrative): with combat `0.50` and exploration `0.43` the planner scores
candidate types by the dot product of each type's pillar mix (`ChapterSlot.pillars`, shares of
combat, social, exploration) with the appetite vector:

| Candidate type | Pillar mix (illustrative) | Score | Probability (`exp(3 x score)`, normalised) | Designer base |
| --- | --- | --- | --- | --- |
| parley | 15 / 70 / 15 | 0.635 | 0.32 | 0.25 |
| heist | 25 / 35 / 40 | 0.545 | 0.24 | 0.25 |
| mystery | 10 / 30 / 60 | 0.521 | 0.23 | 0.25 |
| hunt | 50 / 10 / 40 | 0.493 | 0.21 | 0.25 |

The most favoured option moves from 25 to 32 percent, the least from 25 to 21: a 1.28x and 0.84x
change, inside the 1.5x cap. Variety is preserved, and the draw is a recorded roll (5.8),
so a replay picks the same chapter. The planner still enforces its existing repairs: no two
adjacent identical types, every pillar mean at least 25 percent across the outline
(`balanced()`), the lieutenant before the finale. If the draw would violate them, the designer's
choice stands.

### 5.5 Spotlight balance

* **Definition.** For each hero, the number of *hooks the story offered that hero this chapter*
  (an NPC addressing them, a personal clue, a decision that depends on their skill), compared to
  the table mean. It counts our offers, not their responses.
* **Rule.** If a hero's cumulative offers trail the mean by 2 or more over the last two
  chapters, give the next optional hook to that hero (a scheduling rule that needs no behavioural
  data, so it is fair to a child or a quiet player by construction).
* **Personal hooks** (backstory ties) require that hero's `SpotlightPreference` (D01); otherwise
  the hook is a generic "the NPC turns to you" beat with no disclosure.
* **Success** is measured by the distribution of offers (Gini of offers at most 0.2), never by
  acceptance or speaking share. Ignoring an offer is a complete outcome.

### 5.6 Pacing fit

* **Inputs.** Explicit only: a "slower, same, faster" vote on the chapter-end card, and the
  host-declared evening length at setup (30, 60 or 90 minutes). Elapsed wall-clock time versus
  the declared length is treated as a scheduling fact (am I about to overrun the evening?), not
  a mood signal. This is an interpretation I ask the owner to confirm (question 6).
* **Use.** One notch on the next chapter: recap length (full or short), number of optional beats
  (0 to 3 hooks), and scene count within the chapter type's range.
* **Decay.** The preference lapses after two chapters unless repeated.

### 5.7 NPC affinity

* **Definition.** Per NPC and per table, the probability the table returns to that NPC when it
  can (an optional follow-up chip or a chance to revisit), plus explicit favourite taps. It is
  separate from the NPC's in-world attitude (Friendly, Indifferent, Hostile), which is game
  truth owned by the interaction engine and is never altered by affinity.
* **Use.** Prefer the NPC when picking recurring contacts, lieutenants' returns and callbacks.
  It does not change an NPC's knowledge, attitude, secrets or stats.
* **Caveat.** The first NPC met has the most exposure; the per-opportunity denominator and
  counting only voluntary returns reduce but do not remove that bias. The explicit favourite tap
  outranks the inferred rate.

### 5.8 Random draws and determinism

Every sampled choice (the chapter-type draw, any tie-break) uses a named stream in the campaign
generator's existing recorded-roll style: `Rng(seed XOR constant)`, entries recorded as
`adapt.chapter_type`, with the candidate list, weights and the roll. The adaptation inputs
(the quantities above, as computed) are stored in the chapter's recorded inputs; the chapter seed
already folds in `finalize(outcome_digest)`, so a replay needs only the stored record and policy
version (adventure generator, section 15).

---

## 6. How each quantity may steer the story

### 6.1 Where decisions happen

Only at **chapter boundaries**, in the campaign planner, and in the beat selection of the next
chapter's plan. `df-experience` (pure) computes the quantities and returns an `AdaptationProposal`
from an immutable basis; the planner and `df-engine` accept or reject it like any proposal
(runtime directors, "Authority and composition"). The result is a typed `AdaptationBrief`
recorded in the chapter plan.

```
AdaptationBrief {                 // typed, bounded, listener-safe; illustrative
  policy_version, chapter_number, basis_digest,
  encounter_rung:      Low | Moderate | High,       // planner input; the model never sees numbers
  chapter_type_draw:   { candidates, weights, roll },
  optional_npc_hooks:  0..=3,
  recap_length:        Short | Normal,
  spotlight_offers:    [ { hero, hook_ref } ],      // only with SpotlightPreference
  reasons:             [ReasonCode],                // for DM's notes only
}
```

### 6.2 Levers, all inside standard rules

| Quantity | Lever | Range per chapter | Never |
| --- | --- | --- | --- |
| Challenge fit | **Encounter rung** from the pinned 2024 encounter-building guidance: number and choice of creatures within budget, creature CR (today: which SRD antagonist the generator picks, since the engine runs one creature), terrain and cover, objectives other than kill-all, planned reinforcements committed in the encounter plan *before* the fight, whether a rest opportunity precedes the finale | One rung | Edit a stat block, add or remove hit points mid-fight, trigger reinforcements because the party is struggling, fudge or reroll dice, secret modifiers |
| Challenge fit (non-combat, later) | **DC rung** on the standard ladder, chosen *when the check is set up* and recorded before the roll; **clue redundancy** under the three-clue rule | One rung; one extra clue vehicle | Change a DC after a roll or hide a modifier |
| Pillar appetite | Weight among the planner's already-valid chapter types (5.4); weight among optional beats within a chapter | Candidate weight within 0.67x to 1.5x of the designer base | Remove a pillar (the 25 percent floor), skip required beats (three-clue rule, act twists, lieutenant before finale) |
| Pillar appetite (social) | Number of optional NPC hooks and topic chips offered | -1 to +1 around the default | Gate a required clue behind an optional chat |
| Spotlight balance | Which hero an NPC or hook addresses | Next optional hook | Force speech, spend a turn, expose a secret |
| Pacing fit | Recap length, number of optional beats, scene count within the type's range | One notch | Shorten or remove a legal input window; invent a countdown (G12-D02) |
| NPC affinity | Which NPC returns, callbacks | Prefer among valid candidates | Change attitude, knowledge, secrets, mechanics |

### 6.3 Hard limits

1. **Bounded step.** One difficulty rung, one pacing notch, a 1.5x candidate-weight cap per
   chapter. No chain of steps inside one chapter.
2. **Chapter boundaries only.** No adaptation inside a fight or a check sequence. The encounter
   plan is committed before it starts; df-combat reinforcement triggers are authored in that plan.
3. **Never punish success.** A table that wins easily is never ambushed with a spiteful surprise,
   and toughening stays within standard budgets and standard rewards. Rewards, XP and milestones
   are never modified by adaptation, and a provider failure never withholds them (AGENTS.md).
4. **Asymmetry.** Easing may follow inference once the gate passes; making things tougher
   requires that the table's preference is "as written" or "tougher".
5. **Floor and ceiling.** The rung stays within Low to High. A family default may cap at Moderate.
6. **Explicit outranks inferred.** Inference never moves a quantity against an unexpired explicit
   statement.
7. **Standard rules only.** Any lever that would need a custom rule is unavailable (rules-support,
   generated-content).
8. **No fiction tells.** The storyteller never says or implies "you seem to be struggling". The
   model gets no engagement labels (6.4).
9. **Explainable.** Every proposal carries reason codes shown in DM's notes (6.5).
10. **Switchable.** A host switch turns adaptation off for the whole campaign; turning it off
    leaves designer defaults, deletes no past game state, and takes effect from the next
    planning step.
11. **Deterministic replay.** Section 5.8; the policy version is pinned per campaign so a policy
    update never retcons a live run.
12. **No-op default.** If consent is missing or stale, the basis is incomplete, or evidence is
    below the gate, the result is `NoChange` with a reason, never an error and never a guess.

### 6.4 How the narration engine receives it

The model never sees `delta`, take-up rates or any engagement-flavoured label. The structure (chapter
type, creatures, DC) is decided by dice and tables as today. The storyteller's chapter sheet
(adventure generator, section 16) gains only existing-style typed directives: recap length, how
many optional topic prompts the speaker may offer, which hero the speaker turns to, and which
recurring NPC to bring back. Those are expressed by the model inside the existing validated slots
and fall back to table text on failure. Fail-closed: if the brief is missing, the chapter plays
as designed.

### 6.5 What DM's notes show (host-only)

A short, plain panel after each chapter and before the next is generated, for example:

```
Next chapter plan  (adaptation ON, policy engage-v1, evidence: 3 fights, 2 cards)
Challenge   Eased one step to Low.  Why: last 3 Moderate fights were harder than a typical
            table finds them (estimate +0.54, uncertainty +/-0.43, 75% confident).
            You said: "as written".  Undo: set Challenge to "Moderate".
Story       Parley chosen (32% chance vs 25% default).  Why: you tapped "more talking"
            (2 of 3), and 3 of 5 optional questions were asked.
Spotlight   Next NPC hook goes to Rurik (2 fewer hooks than the others).
Not used    Reply speed, line length and mood are never read.
```

The host can switch any line back to the designer default, and can switch all adaptation off.
Players do not see the panel; a one-line "the story adapts to your table" disclosure appears at
setup (question 5).

---

## 7. Explicit feedback, designed for a phone at a table

### 7.1 Principles

* At most about two taps per chapter, one card, all private to the phone, always skippable.
* Coarse and ordinal (three options), with plain words and icons so a child can answer.
* Anchored to a named thing ("the fight", "talking to people") to limit peak-end drift.
* No individual vote is ever shown. The TV may show a combined "most tapped" only when at least
  two people voted, and only if the host enabled it.
* A skip is not a negative. Skip rate is tracked as a burden metric and prompts get shorter or
  rarer if it rises.

### 7.2 The asks

| When | Ask | Taps | Feeds |
| --- | --- | --- | --- |
| Setup (host, once per campaign) | "How tough?  Gentler / As written / Tougher"; "Evening length  30 / 60 / 90"; adaptation switch (on, off) | 3 | Target strain `S*`; pacing length; consent for adaptation |
| Character creation (each player) | "What do you most want tonight?  Fighting / Talking / Exploring / Puzzles" (pick up to two, or skip) | 1-2 | Pillar prior only; not a type label |
| End of chapter (each player, after the climax, before the recap) | "That fight was:  Too easy / Just right / Too tough" and "More of:  Fighting / Talking / Exploring / Puzzles   (tap any, or skip)" and "Pace:  Slower / Fine / Faster" is **optional every third chapter** | 1-2 (the pace tap rotates in) | Challenge calibration; pillar votes; pacing |
| Any time (player) | A "Keep this character" tap on an NPC portrait | 1, optional | NPC affinity (favourite) |
| End of campaign | "Would you play another chapter like this?  Yes / Not sure / No" | 1, optional | Evaluation only; does not steer |

### 7.3 How explicit outranks inference

1. The setup toughness sets the target `S*`; inference may only move *within* the deadband
   around it. A "gentler" table never gets an inferred "tougher" step.
2. A majority vote on the chapter card (at least 2 voters, or the only voter at a single-player
   table) overrides take-up for the next chapter, but is bounded by section 6.3. "Too tough" from
   a majority passes the gate by itself for one easing step; "too easy" from a majority permits one
   tougher step if the table's setting is not "gentler".
3. A vote decays after two chapters, so one grumpy evening does not lock in a style.
4. Vote counts, never identities, are stored.
5. Because the card is ordinal and anchored to a fight, it also **labels** the strain value for
   the offline regression that keeps `S` honest. This is the main reason to build the card
   before the steering.

### 7.4 Response-bias handling

Treat "Just right" as weakly informative (the common default). Use majority and net counts rather
than any one person's tap. Do not average across a child and an adult as if equally independent;
in family mode (section 9.3) the host's setup choice, not inference, drives challenge. Keep
the chips on-screen long enough to read and never auto-submit.

---

## 8. Group fairness and accessibility

* **A table, not a person.** The challenge and pillar quantities are table-level on purpose.
  The group is the unit that experiences the game; an average that hides a bored child is a
  known weakness, which is why spotlight balance (offers per hero) and an explicit per-person
  "more of" tap exist.
* **The quiet player.** Never inferred and never labelled. A balanced offer schedule (5.5) gives
  everyone the same chances; their opt-in preference can add more; ignoring is free.
* **The dominant talker.** Per-offer "taken by at least one" counting means a single talker
  cannot inflate a pillar's rate by volume; but it can still decide *what* is taken, which is
  why majority tap counts matter for steering and why take-up alone does not steer in the first
  version.
* **A child at the table.** No individual profile, no persistent preference store, no
  inference-driven toughening, simplified icon cards, and a cap at Moderate by default
  (section 9.3).
* **Accessibility.** Slow input is never evidence of anything: latency is not collected. No
  adaptation shortens or removes a legal input window or changes a timer (G12-D02). Cards are
  readable with large type, high contrast, no timeout, screen-reader labels and no reliance on
  audio. A player using assistive input or a different language is neither flagged nor
  measured differently.
* **Disconnects.** A player who is disconnected is excluded from take-up denominators for that
  window (they declined nothing).

---

## 9. Privacy, consent and storage

### 9.1 What is stored, at what granularity

| Data | Granularity | Where | Retention |
| --- | --- | --- | --- |
| `ChapterSignals`: strain components, result, rounds, offered/taken counts per pillar, vote counts, contributor count, consent generation, policy version | **Table, per chapter** | Campaign record in PostgreSQL (continuity authority, adventure generator section 16) | With the campaign; deleted when the campaign is deleted (G05 erasure policy applies) |
| Estimates (`delta`, Beta parameters, affinity), the `AdaptationBrief` and reason codes | Table, per chapter | Campaign record, alongside the decision record used for replay | With the campaign |
| Per-hero spotlight offer counts | Hero within the campaign | Campaign record | With the campaign |
| Raw typed lines, transcript text, latency, cadence | **Not stored.** `dialogue.rs` keeps raw text only transiently; the public table log is a 24-line in-memory ring | n/a | n/a |
| Individual votes | **Counts only; no identities** | Campaign record | With the campaign |
| Any per-person or cross-campaign profile | **None by default** | n/a | n/a |
| Pooled priors across tables | De-identified, aggregated, only from tables that gave a separate consent | A separate analysis store, not the game database | Per G05; deletable by opt-out for future versions |
| Telemetry mirror | Decision events and aggregate counts for dashboards | OTEL SQLite | Per observability; never read back as truth |

### 9.2 Consent, following G12-D01

* **Purposes** are separate switches, off or on as listed, each shown with what it uses and does:
  (1) *Tune challenge from how fights go* (class A only; recommended default **on** with
  disclosure, because it reads only game facts; owner to decide, question 2);
  (2) *Learn which parts of the story we like* (class C, per-offer take-up; default **off**);
  (3) *Share anonymous summaries to improve the game* (pooling; default **off**);
  (4) *Spotlight my character* (existing `SpotlightPreference`; per player; default **off**).
  Explicit cards (class B) are always optional and need no consent beyond answering them.
* **Per participant.** A participant who has not opted into (2) simply does not contribute to
  take-up counts. A co-player's consent never covers them.
* **Withdrawal** bumps the consent generation; pending proposals and new reads under the old
  generation are fenced; committed game outcomes stay. Adaptation falls back to designer defaults.
* **Fail closed.** Missing, stale or ambiguous consent yields `NoChange`.
* **Voice.** If a microphone is ever enabled, only the final validated transcript words may be
  classified (the same counts as typed lines); no prosody, volume, laughter, silence or room audio
  is analysed, stored or used. Voice consent is separate and does not enable (2) by itself.

### 9.3 Family default

A family table (set by the host at setup, or any child account when accounts exist): no class C
inference at all, no pooling, no per-person data; adaptation uses class A and explicit cards
only; the challenge ceiling is Moderate unless the host sets "Tougher"; cards use icons and read
aloud; the host is the only person who sees DM's notes. Whether a stricter age-based rule or a
legal review is needed (children's-data law varies by place) is an owner and legal question I
cannot answer here (question 9).

### 9.4 Telemetry is not truth

Adaptation reads only committed game state and recorded explicit answers. It never queries the
telemetry store, which is lossy and sampled for storage, and exists for review (observability.md).
The campaign record, not telemetry, is the recovery source.

---

## 10. How to tell whether it works

### 10.1 Offline: synthetic player archetypes

Extend the existing `--simulate-campaign` harness (it already plays a campaign through chapters
with synthetic outcomes) with archetypes that generate fight outcomes, optional-offer take-up and
card answers from known true preferences:

| Archetype | True behaviour | What it tests |
| --- | --- | --- |
| Cruiser | Strong, lucky; wins with little damage; says "as written" | Does tougher step happen only when allowed, and not oscillate |
| Struggler | Weak, unlucky; says "just right" despite near-deaths | Does explicit "just right" stop inferred easing? |
| Talker | Takes 80% of social offers, 20% of others | Does the weighting reach the cap and stop; does variety survive |
| Brawler | Mirror of Talker | Same |
| Quiet-one table | One hero never acts on offers | Offer parity holds; no penalty |
| Switcher | Prefers combat, then social from chapter 5 | Drift handling and recovery time |
| Noisy answerer | Random taps | Gate prevents motion; no-op rate stays high |
| Sandbagger | Throws a fight to get an easier next chapter | Exploitation gain stays small and bounded |
| Dominant talker | One player takes every offer for the table | Majority tap counts vs take-up |
| Family | Mixed ages, host sets "gentler" | Ceiling and no-inference rules hold |

Measure: steady-state challenge error (distance of the served `S` to the archetype's preferred
band), time to converge, oscillation (direction reversals; target at most 1 in 8 chapters),
`NoChange` rate when the table is already in band (target above 90 percent), pillar-mix distance
to the preferred mix, offer Gini, exploit gain, **constraint violations (must be 0: any in-fight
change, rule departure, cap breach, step larger than one rung, adaptation while off, use of a
forbidden signal)**, and replay equality (byte-identical for 100 percent of runs). Also run a
**null policy** (always `NoChange`) as the baseline, because regression to the mean will make
almost any policy look good against "no data".

This proves the mechanics are bounded and behave as designed. It cannot show that people have more
fun, and an LLM critic's score must not stand in for that (AGENTS.md).

### 10.2 Live: with real tables

* **Within-table interleaving.** For tables that opt into the study, randomise which chapters in a
  campaign use adaptation versus designer defaults (recorded in the decision log, order
  randomised), compare the chapter-end cards. Each table is its own control, which matters more
  than any other design choice when tables vary so much.
* **Pre-specified outcomes** (few, to avoid the multiple-comparisons trap): "That fight was just
  right" rate; "would play another chapter" rate; skip rate on the card; chapters completed
  versus abandoned; campaign return within two weeks.
* **Analysis.** Paired, ordinal, Bayesian with credible intervals, not a p-value hunt.
* **Honest power.** Between-table A/B to detect "just right" moving from 60 to 75 percent needs
  roughly 150 tables per arm (80 percent power, two-sided 5 percent; my calculation). That is not
  feasible early. A paired design with, say, 20 tables and 4 chapter pairs each (80 pairs) can
  detect only a large effect: if about 40 percent of pairs disagree and adaptation wins 24 of
  those 32, a sign test gives about p = 0.007; a 60/40 split among them would be invisible.
  Expect to learn "no obvious harm and plausible benefit", not a precise effect size.
* **Other limits.** Novelty and observation effects (friends know they are in a study), carry-over
  between chapters, cards that are mostly "just right", and the fact that group chemistry swamps
  the system's influence.

### 10.3 What counts as success and as harm

| Success | Harm (any one pauses rollout) |
| --- | --- |
| No constraint violations; null-policy baseline beaten in simulation on in-band rate | Any rule, timing or reward departure, or a step beyond the caps |
| Adapted chapters rated "just right" at least as often as default chapters | Adapted chapters rated worse than default, with credible interval below zero |
| Host can explain each change from DM's notes; undo works | Adaptation acts against an explicit vote, or after being switched off |
| Skip rate on cards stays low and stable | Skip rate rising, or complaints that cards are intrusive |
| Offer parity across heroes holds | A hero systematically excluded; any inferred-affect field in storage |
| Players do not report "the game cheated me" | Any telemetry-fed or latency-fed input discovered in review |

---

## 11. Phased implementation plan

IDs are mine for reference; the coordinator assigns real task IDs and briefs (ADR 0004).
Smallest useful step first. Everything before E5 is safe on today's engine.

| ID | Goal | Owning area | Needs first |
| --- | --- | --- | --- |
| E0 | Owner decisions in section 13, and a G12-D01 amendment (or ruling) on class C and class A | Owner, coordinator | Nothing |
| E1 | **Record signals only**: a `ChapterSignals` record per chapter (class A facts, offered/taken counts, dispositions, vote counts), written with the existing campaign outcome extraction; no steering | `df-tools` gameplay (`campaign_completion`, `campaign_store`) with pure types in `df-experience`; `df-model` only if checkpointed | E0 ruling on class C; consent flag design; a frozen type shape from shared-contract owners |
| E2 | **DM's notes**: host-only panel showing signals, uncertainty and "what I would do next and why" (shadow recommendation, nothing applied) | `df-tools` gameplay wire plus the host view in the client crates | E1 |
| E3 | **Explicit feedback**: setup preferences, creation chips, chapter-end card, favourite tap, host adaptation switch; stored as counts | Protocol (new messages in `gameplay.proto`; field ledger), `df-ui`/`df-player`, server handler | E0; a shared owner to assign field numbers (RPC API rules) |
| E4 | **Archetype simulator**: extend `--simulate-campaign` with the archetypes and metrics of 10.1, a null policy, replay-equality check | `df-tools` adventure CLI and tests | E1 types (can run in parallel) |
| E5 | **Challenge-fit estimator, shadow mode**: pure filter in `df-experience`, computes proposals and shows them in DM's notes, steers nothing; regress `S` weights against card labels | `df-experience` | E1, E3 labels, E4 |
| E6 | **Challenge-fit steering**: one-rung encounter choice at chapter planning | Campaign planner, `df-encounter` | E5 evidence from real sessions; the pinned encounter-building catalog (rules-support gate); N-creature encounters (roadmap 4) or, interim, SRD creature choice only |
| E7 | **Pillar appetite steering**: weighted chapter-type draw, optional hook counts, recap/pacing notch | Campaign planner (`ChapterSlot`, `balanced()`), generator | E3; beat-script interpreter (roadmap 2) so non-combat chapters are real; ratified class C if take-up is used |
| E8 | **NPC affinity and spotlight offers** | `df-interaction` adapter (dossier 19.9), `df-experience::spotlight` | E3; `SpotlightPreference`; dossier adapter |
| E9 | **Pooled priors across tables** | Offline analysis, `df-experience` prior loader | At least about 30 consenting tables; privacy and erasure review (G05); separate consent |

Smallest useful step: E1 + E2 + E3. E4 can start in parallel after E1's types are fixed. Do not
start E6 or E7 before E5 has shown, on real tables, that the shadow recommendations agree with the
cards more often than a coin flip.

---

## 12. Risks and what I would watch

* **Misreading the output.** Dashboards will say "engagement". Name fields `strain`, `uptake`,
  `votes`; never `engagement`, `interest` or `mood` in code or telemetry.
* **Flattening.** Systems that always pull toward the middle can erase mastery and spikiness.
  The no-op default, one-rung steps and the "tougher" setting guard against it; track the spread
  of served rungs per campaign.
* **Players noticing.** The research is mixed on whether hidden or disclosed adaptation is better;
  the repository's explainability rule favours disclosure to the host and a one-line disclosure
  to the table. Small steps and the "never punish success" rule reduce the risk of anyone feeling
  cheated.
* **Exploitation.** A table can sandbag. The cost is real (hero HP, risk of defeat and the
  clock advancing), easing never reduces rewards, and the Low floor bounds the gain.
* **Reference drift.** `S_ref` comes from a simulator of reference tables, not from real tables;
  if it is off, `delta` is biased. Recalibrate from labelled sessions and version the policy.
* **Interplay with tempo.** Adaptation changes the content that feeds tempo, not the tempo
  engine. Tempo cues are never an input (circularity).
* **Sparse content.** With one creature and two approaches, there is little to steer. Section 11
  orders work so steering follows the engine capabilities it needs, not the reverse.
* **Speculative claims.** Some research above is thin or contested (DDA enjoyment effects, the
  validity of any implicit signal, target win rates). Where it is, the design leans on explicit
  statements and on small reversible steps.

---

## 13. Open questions for the owner

1. **Policy reading (blocking for E1 and E7).** Is class A (game facts such as rounds and HP
   margins) outside "inferred affect" so it may alter difficulty, and is class C (per-offer
   take-up counts) a permitted "committed interaction count" or a banned "behavioral proxy"?
   Recommend a short G12-D01 amendment that says yes to A, yes to C only as table-level
   per-offer counts with purpose consent, and keeps D banned.
2. **Defaults.** Should "tune challenge from how fights go" (class A) be on by default with
   disclosure, or off until the host turns it on? I recommend on, disclosed, because it reads no
   person-level data.
3. **Direction of change.** Is automatic easing acceptable on strong evidence while toughening
   needs the table's say-so (my recommendation), or should both be symmetric, or both manual?
4. **Target band.** What should "as written" aim for: for example, the party wins about 85 to 90
   percent of Moderate fights, usually with someone below half HP at least once? I could not find
   a defensible published number; this is a design taste the owner should set, then calibrate.
5. **Disclosure.** Show only the host DM's notes, or also a table-visible line each chapter ("the
   story is adapting to your table")? Hidden adaptation conflicts with autonomy research and the
   explainability rule; a visible switch is cheap.
6. **Wall-clock use.** May planned versus elapsed wall-clock time be used as a scheduling fact
   against the host's declared evening length, even though G12-D01 bars wall-clock as a mood
   proxy? I treat it as scheduling, not mood.
7. **Declared tastes across campaigns.** May a player keep a self-declared preference (the creation
   chips) in an account for later campaigns, or must every campaign ask again? Default here: ask
   again, store nothing per person.
8. **Pooling.** Is cross-table pooled learning wanted, and under what consent, retention and
   anonymisation, given tables of friends and family? Default here: not before E9, separately
   consented.
9. **Children.** What age or family-mode rules apply, and does any local children's-data
   requirement need legal review before E3 ships? I only specify a conservative family mode.
10. **Crate ownership.** I recommend the estimators live in `df-experience` (its stated authority
    is macro pacing and fair voluntary participation) with persistence in the campaign record.
    Confirm, or name a new crate.
11. **Feedback burden.** One card per chapter, or one per session? The skip-rate metric will tell
    us, but the first version needs a default.
12. **Rules sources.** The 2024 encounter-building difficulty labels and the DC ladder here are
    my understanding; E6 needs the pinned catalog revision (rules-support gate).

---

## 14. References

Labels: **[S]** confirmed this session; **[S-abs]** abstract or search summary only; **[B]**
background knowledge, link not re-checked. Where I could not read the primary text, I say what
the label rests on.

**Dynamic difficulty, drama management**
* Hunicke and Chapman, "AI for Dynamic Difficulty Adjustment in Games", 2004 [S-abs]:
  <https://users.cs.northwestern.edu/~hunicke/pubs/Hamlet.pdf>. Hamlet: inventory-theory DDA in
  Half-Life; names the designer-control tradeoff.
* Hunicke, "The Case for Dynamic Difficulty Adjustment in Games", ACE 2005 [S for the citation
  only]: <https://doi.acm.org/10.1145/1178477.1178573>. Argues for DDA; I did not read the text.
* Booth, "The AI Systems of Left 4 Dead", Valve, 2009 [S for existence and goals]:
  <https://cdn.fastly.steamstatic.com/apps/valve/2009/ai_systems_of_l4d_mike_booth.pdf>; and
  "Replayable Cooperative Game Design: Left 4 Dead" (GDC 2009):
  <https://cdn.fastly.steamstatic.com/apps/valve/2009/GDC2009_ReplayableCooperativeGameDesign_Left4Dead.pdf>.
  Pacing as a design goal. Valve's `info_director` entity page confirms relax and peak states
  gated on intensity thresholds: <https://developer.valvesoftware.com/wiki/Info_director> [S-abs].
  The intensity model details are [B].
* Resident Evil 4 hidden rank: <https://www.engadget.com/2015-06-03-resident-evil-4-difficulty.html>
  (unconfirmed by Capcom) and the community analysis
  <https://www.speedrun.com/re4pc07/guides/qorc9> [S-abs].
* Overview of DDA examples and criticisms: <https://en.wikipedia.org/wiki/Dynamic_game_difficulty_balancing> [S].
* Rubber banding criticism and defence: <https://lawofgamedesign.com/2014/08/25/theory-rubber-bands/>
  and <https://broadly.vice.com/en/article/epngxp/the-blue-shell-and-its-discontents-2> [S-abs,
  opinion pieces].
* Andrade, Ramalho, Gomes, Corruble, "Dynamic Game Balancing: an Evaluation of User Satisfaction",
  AIIDE 2006 [S-abs]: <https://ojs.aaai.org/index.php/AIIDE/article/view/18739>. Adaptive balancing
  evaluated with human players.
* Smeddinck et al., "How to present game difficulty choices", CHI 2016 [S-abs]:
  <https://www.cs.mcgill.ca/~jeromew/data/COMP766/CHI2016/p5595-smeddinck.pdf>. Players mostly
  prefer manual difficulty choices; adjustment type changes perceived autonomy.
* Sakaue et al., "Reducing Objective Difficulty Without Influencing Subjective Difficulty in a
  Video Game" [S-abs]: <https://pure.lib.cgu.edu.tw/en/publications/reducing-objective-difficulty-without-influencing-subjective-diff/>.
  Overt adjustment can hurt experience.
* Hidden versus upfront DDA, Aalto thesis [S-abs]:
  <https://aaltodoc.aalto.fi/items/940fd4e0-96c6-491a-8613-e0983008f1cf>.
* Csikszentmihalyi, *Flow*, 1990 [B]. Chen, "Flow in Games" (MFA thesis, USC, 2006) and "Flow in
  games (and everything else)", CACM 2007 [B]; background on flOw: <https://en.wikipedia.org/wiki/Jenova_Chen> [S-abs].
* Abuhamdeh and Csikszentmihalyi, "The Importance of Challenge for the Enjoyment of Intrinsically
  Motivated, Goal-Directed Activities", PSPB 2012 [S-abs]:
  <https://scholar.usuhs.edu/en/publications/the-importance-of-challenge-for-the-enjoyment-of-intrinsically-mo/>.
  Stronger opponents and close games were enjoyed more than weak opponents and blowouts.
* Wilson et al., "The Eighty Five Percent Rule for optimal learning", Nature Communications 2019
  [S-abs]: <https://pmc.ncbi.nlm.nih.gov/articles/PMC6831579>. About learning efficiency, not fun.
  Cited as a caution against misapplying "85 percent".

**Player modelling and experience-driven content**
* Yannakakis and Togelius, "Experience-Driven Procedural Content Generation", IEEE TAC 2011
  [S-abs]: <https://pure.itu.dk/en/publications/experience-driven-procedural-content-generation/>.
  Player experience model plus content generator plus evaluation.
* Martinez, Yannakakis, Hallam, "Don't Classify Ratings of Affect; Rank Them!", IEEE TAC 2014 [S-abs]:
  <https://www.um.edu.mt/library/oar/bitstream/123456789/29597/1/Dont_classify_ratings_of_affect_rank_them.pdf>.
  Treat ratings as ordinal.
* Yannakakis and Togelius, *Artificial Intelligence and Games*, Springer 2018 (player modelling
  chapters) [B].
* Drachen, Canossa, Yannakakis, "Player Modeling using Self-Organization in Tomb Raider:
  Underworld", CIG 2009 [S-abs]: <https://pure.itu.dk/en/publications/player-modeling-using-self-organization-in-emtomb-raider-underwor/>.
  Four behaviour clusters from 1,365 players.
* Thue, Bulitko, Spetch, Wasylishen, "Interactive Storytelling: A Player Modelling Approach",
  AIIDE 2007 [S-abs]: <https://ojs.aaai.org/index.php/AIIDE/article/view/18780>. PaSSAGE.
* Mateas and Stern, "Structuring Content in the Façade Interactive Drama Architecture", AIIDE 2005
  [S-abs]: <https://ojs.aaai.org/index.php/AIIDE/article/view/18722>. Beats sequenced by a drama manager.

**Tabletop craft and taxonomies**
* Laws, *Robin's Laws of Good Game Mastering*, Steve Jackson Games, 2002 [B for the book];
  secondary summaries [S]: <https://fistfulofwits.com/2013/06/03/gamer-typology-a-la-robin-laws/>,
  <https://slyflourish.com/four_tips_from_robins_laws.html>. Seven player types; read the sheet.
* Wizards of the Coast, *Dungeon Master's Guide* (2014 and 2024) [B]. Know your players, three
  pillars, encounter building, DC ladder.
* Bartle, "Hearts, Clubs, Diamonds, Spades: Players Who Suit MUDs", 1996 [B]; critique and factor
  analysis by Yee [S-abs]: <https://nickyee.com/daedalus/archives/001298.php?page=2>.
* Hunicke, LeBlanc, Zubek, "MDA: A Formal Approach to Game Design and Game Research", 2004 [S-abs]:
  <https://users.cs.northwestern.edu/%7Ehunicke/MDA.pdf>. Mechanics, dynamics, aesthetics.

**Signals, affect and feedback**
* Barrett et al., "Emotional Expressions Reconsidered", Psychological Science in the Public
  Interest 2019 [S-abs]: <https://pmc.ncbi.nlm.nih.gov/articles/PMC6640856/>. Outward expressions
  do not map reliably to felt emotion; caution for any affect inference.
* Experience-sampling and EMA compliance literature [S-abs; mixed findings, I read only summaries],
  for example <https://jmir.org/2019/12/e14475> and <https://pmc.ncbi.nlm.nih.gov/articles/PMC6925392>.
  I could not confirm the specific reviews' authors; treat the "length and total burden matter
  more than frequency" statement as tentative.
* Kahneman et al. on peak-end effects; Ryan, Rigby and Przybylski, PENS, 2006 [B].

**Statistics**
* Chapelle and Li, "An Empirical Evaluation of Thompson Sampling", NeurIPS 2011 [S-abs]:
  <https://papers.nips.cc/paper/2011/hash/e53a0a2978c28872a4505bdb51db06dc-Abstract.html>.
* Adams and MacKay, "Bayesian Online Changepoint Detection", 2007 [S-abs]: <https://arxiv.org/abs/0710.3742>.
* Russo et al., "A Tutorial on Thompson Sampling", 2018; Gelman et al., *Bayesian Data Analysis*;
  Efron and Morris, "Stein's Paradox in Statistics", 1977; Manheim and Garrabrant, "Categorizing
  Variants of Goodhart's Law", 2018; Zohaib, "Dynamic Difficulty Adjustment (DDA) in Computer
  Games: A Review", 2018 (citation seen in secondary sources, not opened) [B].
