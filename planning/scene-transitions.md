# Scene transitions from player movement

Date: 2026-11-07
Status: Design proposed; no implementation. Open decisions are listed at the end.

How a spoken or typed line such as "we walk to the town square", "we enter the hidden
door" or "we run down the hallway" becomes a state transition and a new display plate.
This is the main way players move through the game, so it is one general path with no
per-example code. It composes pieces that already exist; see "What exists".

Governing documents: [Gameplay engine redesign](gameplay-engine-redesign.md) (INTERPRET,
CHECK, APPLY, PROJECT; Jev), [Narrative engine](narrative-engine.md),
[Runtime directors](runtime-directors.md), [Asset engine](asset-engine.md),
[Tempo engine](tempo-engine.md), [Observability](observability.md) and
[Coding style](coding-style.md).

## The path

```
player line --> Jev (INTERPRET) --> MoveIntent --> rules (CHECK) --> APPLY: state transition
                                                                          |
                                   Narrative engine proposes a SceneDesign <-+
                                          |
                      PlateBrief (prompt + character references) --> image --> locate --> swap
```

1. **Jev classifies.** Jev reads the line against the closed list of action labels and
   returns a `MoveIntent { who, to, kind }`. `kind` is a closed label (walk, enter, run,
   climb, ...). `to` is free text naming the destination. `who` names the characters who
   move; the rest stay. Every example above is the same intent with other arguments.
2. **Rules own the transition.** The intent passes CHECK like any action. APPLY then moves
   the party or the named characters from the current scene to a new or existing one. Jev
   and the narrative engine propose; the rules decide, and clients only render.
3. **The narrative engine designs the scene.** It returns a typed `SceneDesign`: the
   place, the mood, the set elements, and a position for every character including those
   who stayed behind. It is a pure function of the intent and the story state. It does not
   write state; it proposes a staged change through the engine owner.
4. **A prompt method builds the brief.** `PlateBrief` is made from the `SceneDesign` and
   the characters' look sheets by a fixed template. The same design gives the same prompt.
   It forbids writing, captions, speech bubbles and interface in the picture.
5. **Character references keep the characters consistent.** Each character has a
   reference card (a written look sheet, and a library portrait when one exists). If the
   image backend accepts reference images, the brief attaches them; if not, the look
   sheet goes into the prompt text. Which applies is decided by checking the backend
   first (open decision 2).
6. **The plate pipeline delivers.** The existing plate pipeline builds the picture,
   locates the characters in it, and swaps the backdrop. The host, not a script, shows
   the "updating the scene" spinner while the job runs.

## What exists

- The engine's `Interpreter` port (`df-table-host/src/engine/adapters.rs`). The dev
  stand-in `DevInterpreter` only matches an offered choice exactly; the real Jev is a
  model call on the fast lane (redesign question 65).
- The engine pipeline ADMIT, INTERPRET, CHECK, APPLY, PROJECT and the `ActionPlan`.
- `df-narrative` (beat selection, thread progress, convergence, bookends, threat
  relevance). It does not design a set or place characters, and has no dev command line.
- The plate pipeline: `scene build`, `plates`, `buffer show`, `scene lookahead`, `scene
  timing set`, and the broker with its describe, background, plate and locate stages.
- `media generate`, `media locate` and `media locate identify`.
- The spinner glyph on the roleplay title chip (`Glyph::Spinner`).
- `persona converse plate=NAME`, a sequential CLI script that makes one plate from a
  conversation. It is a stopgap. It has no state machine and no deadline of its own, and
  it duplicates the plate pipeline. This design replaces it.

## Built to be debugged

- **One transition state, owned by the host:** `Interpreting`, `Designing`, `Briefing`,
  `Generating`, `Locating`, `Placing`, `Live`. A failed step is a named state with a typed
  reason and the old scene stays up. Each state has a deadline.
- **Every step is a record** sent through the shared telemetry path: the player's line,
  Jev's label and arguments, the `SceneDesign`, the exact prompt, the references used, the
  timings and the result. A player's report can be replayed from it.
- **A dev verb per stage,** in the style of `persona ask` and `persona converse`:
  `move say "<line>"` runs the whole path; `move interpret`, `move design`, `move brief`
  and `move plate` run one stage, and take a hand-written input so a stage can be tried
  with no model call.
- **Fakes for every provider.** The whole path runs locally with no spend. A live run is
  opt-in and bounded by a spend limit.
- **Flexible by data:** the label list, the design template and the look sheets are
  content files, not code.

## Extensible for later gameplay

The path is built for movement first, and later features (combat, social scenes, investigation,
weather, time of day) must plug in without rewriting it. Every stage therefore has a seam:

- **Interpret:** no word table; the stand-in reader is removed. Jev is the model call
  (`POST /dev/move/interpret`), which reads the line against the closed schema generated from the
  move kinds, and `parse_reading` checks its answer and the party.
- **Design:** no table of set kits. The designer is a model call (`POST /dev/move/design`) against
  a closed schema, and `parse_design` checks its answer against the move and the party. The design
  includes residents (non-party characters who belong to the destination, such as a market's fruit
  vendor), placed in the picture; each is given a persona after the plate names them, so
  `persona ask|say|converse` work with them. A new input (time, threat, weather) is more context
  in the facts, not a changed signature.
- **Brief:** the art direction is a `BriefStyle` value with a default (`movement/brief.rs`, the
  single definition of the look; `persona converse plate=` reuses it), so a campaign or scene can
  change it; look sheets are a content file; references are a list, so another kind of reference
  (a prop, a place) is another card type, not another path.
- **Transition:** stages and deadlines are data on the transition, records are plain typed
  values, and the audit line is one formatter, so a new stage or consumer (telemetry, a replay
  tool) reads the same records.
- **Everywhere:** no hard-coded caps that block dev work, no `match` on strings, typed errors
  with stable keys, and nothing in a stage that knows about movement beyond its own file.
  Anything a later feature would need to vary is a field, a table row or a trait impl.

## Build order

1. The typed `MoveIntent`, `SceneDesign` and `PlateBrief`, and the `move` dev verbs on
   fakes, with the transition state and its records.
2. The real Jev call (closed labels with arguments) and the narrative engine's design step.
3. The reference-image check and the hookup to the plate pipeline, replacing `persona
   converse plate=`.

## Open decisions

1. Is the `SceneDesign` a typed structure (place, mood, elements, a position per
   character)? Recommended: yes, over free text, so it can be checked and replayed.
2. References: real reference images if the image backend accepts them, or a fixed look
   sheet in every prompt as the fallback. Recommended: check backend support first.
3. The first demo line: an authored beat, or a line the model says through `persona say`
   that Jev then classifies. The elf and the warrior going out to the balcony is the
   agreed example.
4. Whether the real Jev uses the fast classification lane. The text lane takes 15 to 45
   seconds today (redesign question 65).

## Risks

- Reference-image support on the Codex image backend is unconfirmed; the generator takes
  a prompt only today.
- A new plate takes minutes. The old scene must stay up and playable, and a late plate
  must not replace a scene the party has already left.
- The classifier can read a line as the wrong label ("I draw my greatsword" was read as a
  check in earlier play); the closed list and typed arguments are the guard.
