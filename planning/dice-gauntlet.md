# Dice gauntlet

Date: 2026-11-10
Status: Owner goal set 2026-11-10; this is the working brief for every agent on it. Decisions
below are the coordinator's defaults where the owner gave none; change them in this file first.

## The ask (owner, verbatim intent)

1. Generate the dice assets first, and design how a roll is displayed on screen.
2. Run a refinement loop on that display until a person can read a roll and understand it at a glance.
3. Then build a gauntlet of checks: one warrior character with his own reference sheet, and a
   simple setting suited to each check.
4. The check is a random mechanic: it must be random, quick, stay on screen long enough to read,
   be easy to read and beautiful to look at.
5. A judge attached to the narrative engine decides whether a player action needs a check, or
   whether the narrator wants to start one.
6. The narrator (DM) must say aloud that a check is called, and read the result in plain words
   with role-play flair.

## Decisions taken (change here first)

- **Rules source:** SRD 5.2.1 (the planning candidate in `rules-coverage.md`; `df-rules` already
  cites it by page). No PHB-only content. Every mechanic cites its clause.
- **Who rolls:** the server only (`rules-support.md`, `gameplay-engine-redesign.md` H.3). Dice come
  from the host's `Dice` port (`adapters/dice.rs`: unbiased draws, every draw recorded, replayable).
  Real system entropy in the gauntlet. A seeded source exists for tests only; no operator
  force-dice in a hosted or gauntlet run. The page never decides or re-rolls a value; its tumble
  is cosmetic and lands on the server's faces.
- **Player character:** a persona (the warrior) states what he does; the judge decides whether a
  check follows. No phone tap-to-roll in this phase.
- **Judge and flow (2026-11-10):** the narrator-initiated moment is the judge's `moment=narrator`
  (reasons `narrator_hazard`, `narrator_obstacle`, `narrator_reaction`); it is not a variant of the
  plotted-story narrator's `Moment`, which needs a story's places and evidence a gauntlet station
  does not have. A mode of advantage or disadvantage needs a closed ground and the sheet must hold
  what the ground names. The SRD has no opposed check, so `contested` is its two plain tests
  (Athletics or Acrobatics against an escape difficulty, Perception against a Stealth total); a
  passive score is read, never rolled, and has no difficulty. The dice are drawn and resolved
  before the call is spoken (nothing shows them) so the result line can be worded while the call is
  heard; the card appears when the call has been heard out (3 s plus 70 ms a character), the
  result is read when the sum is on screen at the latest (4.5 s after the card appears) and the card
  is cleared 0.4 s after max(6 s, result line heard plus 2 s) past that. Heroic Inspiration is
  spent by the operator on a named die (`inspiration=<place>`), as the SRD has the player choose
  it after seeing the roll. A station the narrator initiates passes its designed test as
  `intent=` (legal only at a narrator moment); the judge calls it with its own band, mode and
  grounds or refuses it as `intent_unsuitable`. A line of the dungeon master has a 14 s budget and
  is its composed numbers alone when its words do not finish in it; a flourish names a closed tone
  that code checks against the outcome, so no judge is on the critical path.
- **Stakes:** checks and damage/healing dice only. A failure is narrated and shown, with no
  persistent hit points, conditions or item loss in this phase.
- **Character:** a new Fighter (human, level 3, Champion: Improved Critical, crits on 19 to 20;
  proficiency +2; Heroic Inspiration from Resourceful) with id `gauntlet_warrior`, his own
  persona file, voice and reference sheet (`ref-gauntlet_warrior.png`, front close-up, three-
  quarter, full-length on mid-grey, same pipeline as `ref-danny.png`). Do not reuse the quest
  `warrior`.
- **No string or word matching for gameplay decisions.** Whether an action needs a check, which
  ability or skill, the roll mode and the DC band are the judge's answer: a model call against a
  closed schema, validated in code against enums and tables. DCs come from the SRD's standard
  difficulty ladder (5, 10, 15, 20, 25, 30) or a typed ruling; never invented.
- **No reduced-motion handling anywhere in the page** (owner rule). Animations always run.
- **No limits in dev** on model answers; correctness checks only.
- Thin client: the page renders a server-provided `RollView`; it holds no rules.

## Denominator: what "every kind of check, every die size" means

Each row is at least one gauntlet station (simple setting, one clear beat) and one golden rules
fixture with recorded draws.

| Group | Kinds |
| --- | --- |
| Ability checks | the 6 raw abilities; all 18 skills (Athletics; Acrobatics, Sleight of Hand, Stealth; Arcana, History, Investigation, Nature, Religion; Animal Handling, Insight, Medicine, Perception, Survival; Deception, Intimidation, Performance, Persuasion), proficient and not proficient (Expertise is not a Fighter feature: show it through a hired expert NPC if the source needs it, else note the gap) |
| Passive | Passive Perception (a score shown, no roll) |
| Saving throws | all 6 abilities, proficiency in Strength and Constitution |
| Attack rolls | melee and ranged, normal, advantage, disadvantage, natural 20 and 19 (Champion crit), natural 1 |
| Contested | Athletics versus Athletics or Acrobatics (grapple/shove style) |
| Initiative | Dexterity check as initiative |
| Death saving throw | d20 with the SRD's natural 1 and natural 20 rules |
| Modes | normal, advantage, disadvantage (both dice shown), Heroic Inspiration reroll |
| Dice sizes | d4, d6, d8, d10, d12, d20, d100 (percentile as two d10) as damage, healing, hit die, bonus die or table roll; expressions NdS+M |

Anything the source does not support (or that needs a ruling) is a visible gap in the report, never
an approximation.

## Contracts (freeze these names; add fields, do not rename)

- **Dice assets (static, shipped):** `assets/playground/dice/<name>.png`, names `d4`, `d6`, `d8`,
  `d10`, `d12`, `d20`; percentile uses the `d10` art twice. Transparent background, one
  consistent beautiful style, **no numerals baked in**: the page draws every number in its own
  font so a face is always exact and legible. Plus a `tray.png` (the surface they land on) if the
  design wants one. Sizes and the style are the art agent's call, recorded in the brief's report.
- **`RollView` (shared host/page type, in `df-playground/src/fixture.rs` beside `PlateKind`):**
  `id`, `title` (what is tested, e.g. "Athletics (Strength)"), `kind` (closed enum of the
  denominator), `mode` (`normal|advantage|disadvantage`), `dice[]` (`{sides, face, role, kept}`,
  `role` in `test|bonus|damage|healing|hit_die|table`), `modifiers[]` (`{label, value}`), `total`,
  `target` (`{label, value}` or none), `outcome` (closed enum), `natural` (`one|twenty|none`),
  `inspiration_reroll` (optional, the die replaced). Carried by a `SceneChange` variant.
- **Check request (judge output, validated):** `{requires_check, initiator (player|narrator),
  kind, ability, skill (or none), dc_band, mode, reason}` with `reason` a closed key. The rules
  owner (`df-rules`, host `adapters`) turns an accepted request into a resolved result using the
  `Dice` port; the DM line and the `RollView` are both built from that one result. Built
  2026-11-10 in `persona/check/` (runbook "The check judge and the DM's checks"), with two added
  fields: `grounds` (the closed reason a mode other than normal follows from, or `none`) and
  `uses` (the sheet's weapon or healing id an attack, damage or healing roll takes its ability,
  dice and critical range from). `kind` is `ability_check|saving_throw|attack_roll|initiative|
  death_save|passive|contested|damage|healing|table`; `dc_band` is 5, 10, 15, 20, 25 or 30 (for an
  attack roll the target's Armor Class on the same ladder; none for initiative, death_save,
  passive, damage, healing and table).
- **DM lines:** two moments, `check_called` (before the roll: it names the test and the DC in
  plain words and tells the player to roll) and `check_result` (after the dice settle: it reads the
  numbers and the outcome, then adds one flourish). The numbers and outcome in the spoken line come
  from the resolved result and are verified against it in code (the line is refused and asked
  again with a typed reason when they do not match); the flavor is the storyteller's. Voiced with
  the `narrator` voice key. Spoken lines have no semicolons (existing narrator rule).

## Display rules (the rubric the refinement loop scores)

A roll takes the TV for one card, staged so the person watching never misses the cause:
1. **Call (about 2 s before the dice appear):** title of the test and the target ("Athletics,
   Strength. DC 15") while the DM says it.
2. **Tumble (1.2 to 1.8 s):** the dice roll in and settle; fast enough not to drag, slow enough to
   follow.
3. **Settle and breakdown (hold at least 6 s, never less than the DM's result line plus 2 s):**
   the faces, then the sum written out once: `14 + 3 Strength + 2 Proficiency = 19`, against the
   target, then the outcome word. Natural 1 and natural 20 get a mark only where the rules give them
   an effect for that test type.
4. **Advantage and disadvantage:** both d20 shown, the unused one dimmed and struck, the kept one
   lit, with the words "Advantage" or "Disadvantage".
5. **Damage and tables:** each die and its face, the modifier, the total.
6. **Legibility (measurable):** numerals at least 6 percent of the screen height for the total and
   at least 3.5 percent for the breakdown; contrast at least 7:1 for numerals, 4.5:1 for labels;
   no text over a busy picture without a plate behind it; nothing moves while it is being read
   (the tumble ends before the breakdown appears). Beautiful: one coherent palette with the
   existing kit (`assets/playground/kit.css`), dice with depth and light, no flat clip art.
7. **Randomness is visible:** two rolls of the same test never look the same (different landing
   spots and spins), and every face value comes from the server.

The refinement loop: build, capture frames at fixed moments for every kind and die size from a
gallery fixture, have the inspector score each against this rubric, fix, repeat until two
consecutive reviews have no finding above polish.

## Phases and owners (one agent owns each whole item)

1. **Assets and display (parallel):** dice art agent; display agent (page card, `RollView`,
   gallery fixture, timing); rules agent (all check kinds and dice expressions in `df-rules`, host
   adapter and a dev verb to roll any kind with recorded draws); warrior agent (persona, voice,
   reference sheet, and the setting plates).
2. **Judge and DM:** the check judge on the narrative path, the two DM moments, the voiced flow.
3. **Gauntlet:** the beats file, station scenes, the runner, the live runs.
4. **Refinement loop on the display** runs through phases 1 and 3 with a visual inspector.

## Acceptance

Evidence from a live run, not unit tests alone: every kind in the denominator rolled at least once
with real entropy; a statistical test per die size on the `Dice` port (uniform within bounds over
at least 10,000 draws); a replay of the recorded draws reproduces the run; every roll card held at
least 6 s and the DM line finished before it left; the DM said the call before the dice and read the
result after; frames inspected one by one against the rubric; timings measured from the logs
(call start, tumble start and end, breakdown shown, card off, DM line start and end).

## References

`rules-coverage.md` (R04, R08, R14), `rules-effect-model.md`, `rules-support.md`,
`gameplay-engine-redesign.md` (H.3 dice, K.5 dice and result cards), `interaction-engine.md`
(NeedsRuling), `narrative-engine.md`, `client-architecture.md`, `coding-style.md`,
`development/runbook-dev-playground.md` ("The dungeon master (narrator)", "Character reference
sheets", "Declared actions", "The UI kit on the live TV").
