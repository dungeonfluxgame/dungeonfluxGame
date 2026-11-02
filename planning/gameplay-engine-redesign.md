# Gameplay engine redesign

Status: **Collecting ideas; two refinement rounds merged.** Decisions are the owner's; the working
plan is a draft by the assistant until the owner accepts it. The refinement loop (four critics a
round) ran at the owner's request, and its results are in the Analysis section at the end. Round 3
is next. Started 2026-11-02.

## How this document works

1. The owner shares ideas, in any order.
2. Each idea is recorded in the owner's words and restated in plain terms so a misunderstanding
   shows up straight away.
3. Facts from the current code are marked as observations. They are not conclusions.
4. Anything the owner has not decided is marked **draft** or listed as an open question.
5. When the owner says the ideas are complete, the analysis is written: conflicts between ideas,
   risks, a proposed shape and an order of work.

**Precedence.** Sections before the Analysis were written before the refinement loop. Where one of
them disagrees with the Analysis (its resolutions RES1 to RES34, the pipeline in section C, the
machine contract of RES27, the table experience contract in section K), the Analysis governs. The
older text is kept as the record of what was decided and proposed at the time, and the points that
moved are: D4 (RES2, RES20), D7 (RES17), D14 (RES13), D15 (RES1), D16 to D18 (RES5, RES9, RES10,
H.7, K.3), D25 (RES15, pipeline version 2), the single "clock" (now `plate_tick`, RES5), "scene
record layout" (RES1, D.6), "effects" (now `TableEffect`, RES18), "setting up until Ready" (K.2)
and the three-outcome rule (RES16).

## Decisions so far (the owner's)

| # | Decision | Owner's words |
|---|---|---|
| D1 | The gameplay engine is redesigned from scratch, not patched. | "redesign the main gameplay engine from scratch" |
| D2 | The focus is the gameplay display, and the aim is a better-structured engine, because the current one is "clearly too buggy". | "mostly talking about the gameplay display ... to enhance the structure of the gameplay engine" |
| D3 | Each kind of interaction is its own state machine: lobby, character creation, explore, combat, markets and merchandise, and more to come. | "state machines to handle different types of interactions" |
| D4 | Any phase can be entered and left. Each machine's states are distinct, so nothing leaks between them. | "all phases can be transitioned in an out of but the states are distinct so we can avoid spaghetti code and cross contamination" |
| D5 | State is shared per table. | "state is shared per table" |
| D6 | Every resource (text, pictures and the rest) comes from its parent service, in a clean and auditable way, through a single path. | "all resources should be coming from their parent service in a clean auditable way" |
| D7 | Breaking changes are allowed until the engine stabilizes. No saved-game compatibility is owed in the meantime. | "these changes can always be breaking changes until we stabilize the engine" |
| D8 | The engine always knows who is in the scene. | "we need to always know who is in the scene" |
| D9 | Every character in the scene has a character sheet. | "must have character sheets" |
| D10 | Characters carry narrative hooks (if feasible) and a background: thoughts, persona, backstory and so on. | "narrative hooks if feasible and their background (thoughts, personas, backstory etc)" |
| D11 | The set is dressed, and the engine knows where everything in the scene is. | "we need to dress the set as well, need to understand where everything is in scene" |
| D12 | The engine needs a scene builder as a focus: description first, then the background, then a display plate. | "really need to focus on a scene builder, start with the description, generate the bg, then generate a display plate" |
| D13 | The display plate is made from the character references and a description of what is happening on screen. | "with the character references and a description of what is happening on the screen" |
| D14 | The mode where character cutouts are the avatars is removed. Only the display plate is used. | "remove the mode where the character cutouts are the avatars we are using, and only use the display plate" |
| D15 | Every scene gets a SAM 3.1 pass, so the engine knows where the characters are, for gameplay effects. | "make sure the scenes do a SAM 3.1 pass so we can know where the characters are located for gameplay effects" |
| D16 | Display plates are rendered from a queue. As play continues the engine queues a new plate every 15 seconds, even when the image pipeline is running behind. | "the engine should que a new display play for 15 sec intervals even if its relateively delayed due to image gen pipeline" |
| D17 | Rendering is paced and consistent. | "it needs to be paced and consistently rendered" |
| D18 | The interval is configurable, and changing it changes the cost per hour. | "the config changes the gaps length which also changes the cost per hour" |
| D19 | The engine has logging everywhere, and every surface can be reached from the developer CLI. | "make sure it has logging everywhere and has dev cli access for all surfaces" |
| D20 | Shops and inventories each have their own state. | "shops and inventories need their own state" |
| D21 | Each item is visible in the image, so users can explore it. | "each item needs to be visible on the image for users to explore" |
| D22 | There is an explorer item state: the user tells the DM they are viewing an item. | "there needs to be an explorer item state that the user can tell the DM that they are viewing" |
| D23 | In that state the user can inspect the item from different points of view and angles. | "allows the user to inspect from different povs and angles etc" |
| D24 | The design is extensible, so gameplay refinements and new states can be added in the future. | "make sure this is extensible for gameplay refinements or new states in the future" |
| D25 | The player can tell the DM to travel, or to take any viable D&D action, and the DM handles the state transitions or updates. | "the player can tell the dm to travel, or any viable dnd action, and the DM handles state transitions or updates" |

## What I understood, restated

- **The display is the product surface, the engine is what is being fixed.** I read "gameplay
  display" as the thing the players actually see and use: the shared TV and the phones. The aim is
  an engine whose structure makes that display correct by construction. (Confirm: see open
  question 1.)
- **Machines are separate and own their state.** A market machine cannot read or change combat
  state, and the reverse. They meet only through table state and through explicit hand-offs.
- **Any phase can come and go.** A table can enter combat from a market, leave it, and return to
  the market. The machines are not a one-way line of phases. (How resuming works is open: question 4.)
- **One shared table.** There is one authoritative state per table, not one per player. Phones
  show a projection of it for each seat.
- **No side doors for resources.** A line of text or a picture shown to players is traceable to a
  request made through a parent service, with a record of who asked, why and what came back.
- **Break freely, then stabilize.** Old saves and old interfaces can be discarded while the shape is
  found. A later stabilization point ends that freedom.

## Diagrams: the current engine and the revised engine

### The current engine (verified against the code on 2026-11-02, simplified)

```
   PHONE (wasm, one per seat)                        TV DISPLAY (wasm)
   draws the seat's view; the phone also             plates, sprites, captions, voice,
   holds creator and exploration screens             music, motion (client-side pacing)
            |                                                  |
            +------------------------+-------------------------+
                                     |  gRPC over WebSocket (/gameplay/tunnel)
                                     v
  +-----------------------------------------------------------------------------+
  | GAMEPLAY SERVER (df-tools, one process)                                     |
  |                                                                             |
  |   OWNER ACTOR (one thread) - serialises every call, commits to PostgreSQL   |
  |        |                                                                    |
  |        |  ONE PHASE ENUM                                                    |
  |        |  ROOM | CHARACTER_CREATION | OPENING | DIALOGUE | COMBAT | COMPLETE|
  |        |  (no explore phase, no market phase)                               |
  |        |                                                                    |
  |        |  Logic spread over three huge files:                               |
  |        |     actor.rs 7k lines | dialogue.rs 12k lines | journey.rs 9k lines |
  |        |                                                                    |
  |        +-- inside DIALOGUE: talk, rolls, travel, shops and offers, item      |
  |        |   inspection, NPC replies - all one tangle                         |
  |        |                                                                    |
  |        +-- each feature builds its OWN requests, deadlines and wait rules:  |
  |            storyteller | Jev chooser | hosted support | plates | portraits   |
  |            | music | item close-ups                                         |
  |                                                                             |
  |   plate cadence = event driven (state / narrative focus / no change)        |
  |   scene art = room picture + plate + cut-out figure per character           |
  +-----------------------------------------------------------------------------+
              | text requests                      | image requests
              v                                    v
     DURABLE TEXT SERVICE                  IMAGE SERVICE (df-imagery)
     (Codex storyteller, Jev)              Codex images, fal cut-outs
              |                            SAM 3.1 supported, never called in play
              +----------------- PostgreSQL ---------+

   DEV CLI (df-transport-fixture dev ...) reaches many features, not all.
   Logging exists per feature; some failures left no trace (found on 2026-11-02).
```

What this shape costs, as seen in play: a slow job inside a request holds the table, one
transition can blank a screen, the previous scene's picture can stay up, and new kinds of
interaction need edits across the protocol and all three large files.

### The revised engine (draft, from decisions D1 to D24)

```
   PHONES (one per seat)                                  TV DISPLAY
   thin: draw exactly what their projection says          thin: plate playout, captions
            ^                                                      ^
            | projection(seat)                                     | projection(display)
  +---------+------------------------------------------------------+----------------+
  | DISPLAY PROJECTOR (pure function)                                                |
  |   table state + which resources are ready  ->  views that are never empty        |
  |   visible states: setting up | behind by N s | unlocated: <name> | explorer      |
  +-------------------------------------^--------------------------------------------+
                                        | reads
  +-------------------------------------+--------------------------------------------+
  | TABLE STATE (one per table, shared)                                              |
  |   scene record: CAST | SET (items, hotspots) | LAYOUT (areas, optional grid)      |
  |   characters: sheet, persona, thoughts, backstory, hooks (visibility-tagged)     |
  |   inventories | purses | facts | config                                          |
  |   active machine id + one PRIVATE slot per machine                               |
  +-------------^--------------------------------------------------+-----------------+
                | effects (typed, applied by the engine)           | inputs
  +-------------+--------------------------------------------------v-----------------+
  | MACHINE REGISTRY - closed inside a machine, open across machines                 |
  |                                                                                  |
  |  lobby | creation | explore | combat | shop | inventory | item explorer | ...    |
  |                                                                                  |
  |  each machine: enter / step / exit, its own states, saved-state schema,          |
  |  hand-off targets, projection, log events, CLI family                            |
  |  a new machine = one module + one registration line                              |
  +-------------+--------------------------------------------------^-----------------+
                | resource requests (typed, never waiting)         | results arrive
                v                                                  | as inputs
  +----------------------------------------------------------------+-----------------+
  | RESOURCE BROKER - the only caller of the services; audits every request           |
  | deadlines, retries, size rules and prices live here, one place per resource kind   |
  +----------------------+------------------------------------------+------------------+
                         v                                          v
              TEXT SERVICE (single path)                 IMAGE SERVICE (single path)
              storyteller, Jev, staging text             background, plate, item views,
                                                         SAM 3.1 locate (characters + items)

   SCENE BUILDER (a pipeline on the broker)
     describe -> background -> plate -> locate -> ready      (every stage stored + audited)

   PLATE SCHEDULER
     clock (every N s, default 15) -> queue -> builder -> playout buffer -> display
     paced at the configured gap; lateness measured, never hidden; N sets cost per hour

   LOGGING and DEV CLI at every box: list | show | history | drive | config
   (one event shape; failures are logged where they cross a boundary)
```

Not drawn above, to keep the picture readable: the DM sits in front of the machine registry.
Every player input goes through interpret, check, decide, apply and narrate (see "The DM as the
one interpreter"), and only the DM requests moves between machines.

### What changes, side by side

| Area | Current | Revised (draft) |
|---|---|---|
| Interaction kinds | One phase enum; most behaviour inside `DIALOGUE` | A registry of machines, one per kind, each with its own states |
| New kinds | Edits to the protocol and three large files | One module and one registration |
| State | Spread across features | One shared table state, one private slot per machine |
| Who is here and where | Free-text lines per person and item | Scene record: cast, set, layout, checked |
| Characters | Hero sheets; authored dossiers; thin generated people | One character record for everyone, visibility-tagged |
| Waiting on images or text | Inside the player's request | Never; requests are queued and results arrive as inputs |
| Picture of a scene | Room picture, plate, cut-out figures | Background and plate only; SAM locates people and items |
| Plate timing | Event driven, 15 to 19 plates in a short session | Clock driven, paced, configurable, cost shown |
| Resources | Each feature builds its own requests | One broker, one audit, one path per service |
| Display | Draws what it is given; can go blank on empty text | Pure projection that is never empty, with explicit waiting states |
| Logging and CLI | Per feature; gaps found in play | Same event shape and a CLI family for every surface |
| Player input | Handled by whichever phase it arrives in; free actions are fragile | One DM pipeline (admit, interpret, check, decide, apply, narrate, project); every input ends in applied, needs_clarification, declined with a reason, or pending (RES16) |

## The DM as the one interpreter (idea D25), restated

- **Players speak to the DM, not to the machines.** A player says what they want to do: travel,
  attack, search, persuade, buy, climb, rest, or any other action that D&D allows and the scene
  makes possible. They do not pick a machine or a screen.
- **Viable actions only.** "Viable" means the rules and the scene allow it right now: the
  character can reach the target, has the item, the target exists, the action fits the situation.
  An action that is not viable is declined with a reason, not ignored and not guessed at.
- **The DM does the bookkeeping.** The DM turns the action into the right state transitions or
  updates: enter combat, start travel, open a shop, change an inventory, move a character, start an
  item exploration. The player never has to know which machine is involved.

I read "the DM" as the engine's game-master function: the storyteller and Jev together with the
rules, not a separate human. (Confirm: question 49.)

Observations from the current code and from play (verified 2026-11-02):

- **The pieces exist but are not one thing.** A typed line is classified by Jev against a closed
  list of labels (one session made 224 such judgements), checks are rolled by the rules with
  advantage and disadvantage applied, and a free-roam and a free-form action module exist. Which
  of them handles a given line depends on the phase it arrives in.
- **Travel worked, but only as one special case.** A typed "take the road to the market" led to
  travel after the party's way was open, and the destination took about seven minutes to build.
- **Free actions are fragile.** "I draw my greatsword and attack Dain Rusk" was read as an
  Intimidation attempt and no fight started. "I swing my greatsword at Dain Rusk and cut him down"
  answered "Nothing like that is here to act on." Neither the reading nor the refusal told the
  player why, or offered "did you mean to attack?".
- **Some actions need a formal second step.** A shop offer had to be accepted in separate words
  before the coin and the roll changed hands.
- **Declined actions are not explained.** The player gets a narrator line, not a list of what
  would have been possible.

## Scene model (ideas D8 to D11), restated

- **Who is here (D8).** At every moment the engine can answer "who is in this scene", including
  the party, named people, and anyone present but unseen. This is a fact the engine keeps, not
  something the display or the text model infers from the last few lines.
- **Sheets (D9).** Each of those characters has a character sheet. I read "must have" as covering
  non-player characters too, not only the heroes. (Confirm: question 9.)
- **Inner life (D10).** Each character also carries what they think and why: persona (how they
  speak and act), thoughts (what is on their mind now), backstory, and narrative hooks, meaning
  things in their story a player can pull on. You said hooks are "if feasible", so they are a goal
  and not a requirement.
- **A dressed set with a layout (D11).** A scene is more than a title and a picture. It has set
  pieces (props, furniture, goods, weather and light), and the engine knows where each one is,
  and where each character stands relative to them and to each other.

Observations from the current code (verified 2026-11-02):

- **People in a scene.** The scene state lists each person with a name, role and a free-text
  "look and doing" line; the party is listed apart. There is a separate "cast" list used for
  sprites. There is no single record that is the answer to "who is here".
- **Sheets.** Heroes have a character sheet on the wire. Authored non-player characters have a
  stored dossier (origin, past trade, formative event, secrets and more). Whether the people the
  game writes during play, such as the market's three vendors, get anything beyond a name, a role
  and a one-line look was not verified; the state I read showed only those.
- **Persona and backstory.** Persona text for generated people is written by the text service
  (purpose `person_persona`). Where that text is kept and how it is used on later turns was not
  verified.
- **Hooks.** The word "hook" appears in the adventure and campaign code, so some notion exists.
  Its shape and whether players can see or pull on a hook was not verified.
- **Set pieces.** Each place has a list of things (a sealed packet, crates, a moored boat, a
  lantern post), each with a name, what can be done with it, and a position.
- **Where things are.** Positions in a scene are free text ("beside the lantern post", "on the
  wharf", "foreground"). Combat has its own tactical grid. Exploration and conversation have no
  shared spatial model, so nothing can be asked "who is next to what".
- **Why it matters for the display.** The scene pictures are generated from prompts. A prompt
  built from free-text positions cannot guarantee who is in frame or where they stand. Today's
  playthrough showed the effect: the market kept the wharf picture (the courier and the old
  quay), and the speech bubble for the potter was drawn above the courier's head.

## Scene builder (ideas D12 to D15), restated

The scene builder is the pipeline that turns "a place and what is happening in it" into what the
table sees and what the engine can act on. In order:

1. **Description.** A written description of the scene: the place, the set, who is present and
   what is going on. Everything after this starts from it.
2. **Background.** A generated picture of the place with no characters in it.
3. **Display plate.** A generated picture of the scene itself, made from the background, the
   character references, and a description of what is happening on the screen.
4. **Locate.** A SAM 3.1 pass over the plate finds each character in the picture, so the engine
   knows where they stand and can use that for gameplay effects (who is where, who is next to
   whom, who is in an area).

Two consequences you stated:

- **Cutout avatars go away (D14).** Characters are no longer shown as cut-out figures placed over
  a background. What players see of a character is the character as drawn in the plate.
- **Every scene is located (D15).** The SAM pass is part of building every scene, not an extra.

Observations from the current code (verified 2026-11-02):

- **The stages exist in some form.** The image service has separate purposes for a canonical room
  (the background) and a scene plate, plus plate edits. A plate request carries up to three
  character references.
- **SAM 3.1 is already supported.** The providers crate has the segmentation model
  `fal-ai/sam-3-1/image-rle`, marked "the owner's choice", and a `figure_masks.rs` module that
  turns SAM results into plate figure geometry (heads and masks).
- **It did not run in play.** In the two recorded playthroughs the cost log shows no SAM call at
  all. The plate state carried an empty mask list, and the courier's head anchor fell back
  (`missing-actor-head`). The only fal calls were background-removal cutouts (`birefnet`).
- **Cutouts are the current avatar.** Each character is generated as a cut-out figure (extra
  background-removal step), and the scene state carries a sprite for each cast member. The phone
  shows the speaker as a cutout portrait.
- **Plates are rebuilt very often.** One short playthrough made 19 scene-plate generations and
  another 15. Through Codex a plate takes about 1.5 to 4 minutes. When travel happened, the new
  place's plate never arrived and the old picture stayed up.
- **The description is not a first-class thing.** Prompts are assembled from several sources at
  request time, with positions as free text, so there is no stored description that the
  background, the plate and the engine all share.

## Plate pacing (ideas D16 to D18), restated

I read "display play" as "display plate". (Confirm: question 23.)

- **A queue, driven by a clock (D16).** While play continues, the engine puts a new plate request
  on a queue every 15 seconds. The clock, not the pipeline, decides when a plate is requested. If
  the image pipeline is slow, requests wait their turn; the engine does not stop asking.
- **Paced and consistent (D17).** Plates reach the screen at an even rhythm, not in bursts when
  the pipeline catches up and not as an accident of when each job happened to finish.
- **A knob with a price (D18).** The interval is configuration. A shorter gap means more plates
  per hour and a higher cost per hour; a longer gap means fewer and a lower cost. The setting and
  its cost move together, so the cost should be visible where the setting is changed.

Plates per hour at some intervals: 5 s is 720, 10 s is 360, 15 s is 240, 30 s is 120, 60 s is 60.

Observations from the current code and data (verified 2026-11-02):

- **The cadence today follows events, not a clock.** Plate refreshes are triggered by reasons such
  as `state`, `narrative_focus` and `no_visible_change` (log lines `PLATE refresh pipeline=...`),
  and the code has a `cadence_tests` module for that policy. This is what produced 15 and 19
  plates in two short sessions.
- **The image service already has a queue.** Jobs are claimed by workers (up to four foreground
  workers) and have deadlines. A new clock-driven producer would feed this queue; it would not
  replace it.
- **A 15 second interval cannot be met on the current backend.** A Codex plate takes about 1.5 to
  4 minutes. With three workers that is roughly one finished plate every 30 to 85 seconds, so at a
  15 second interval the queue grows. D16 anticipates this ("even if relatively delayed"), which
  makes the behaviour when behind a core design point, not an edge case.
- **Prices known to the project** (`content/pricing.json`):
  - SAM 3.1 through fal: $0.01 per request. At a 15 second interval that is $2.40 per hour for
    the locate pass alone.
  - OpenAI image output: $30.00 per million tokens, plus input rates. The tokens used by one
    plate are not recorded in the pricing file, so a per-plate price is not stated here.
  - Plates made through the Codex subscription are recorded with no dollar price ("unpriced"),
    so their cost per hour is not a dollar figure; it is the subscription's limits and time.
- **A standing rule applies.** The project records spend and never blocks on it, so nothing here
  proposes a spend cap. Limits proposed below are about latency and freshness, not money.

## Logging and developer access (idea D19), restated

- **Logging everywhere.** Every part of the new engine reports what it is doing and why:
  every machine transition, every resource request and result, every scheduler tick, every
  scene-builder stage, every display projection, and above all every failure. Nothing fails
  without a record that names the cause.
- **A developer CLI for every surface.** "Surface" here means anything a person or a test might
  need to look at or drive: each state machine, the scene builder, the plate queue and playout
  buffer, the resource broker, the TV and each phone's projection, and the configuration. For each
  there is a command to see its state and history, and one to drive it.

Observations from the current code (verified 2026-11-02):

- **There is a developer CLI already.** About fifty command families (state, view, transcript,
  timeline, audit, text, imagery, art, direct plate, gates, latency, memory, music, trade-terms,
  place, fight, and more), served by a development control endpoint behind a token. It is the
  tool that made today's playthroughs readable.
- **There is a logging plan.** `planning/observability.md` and the project rules require shared
  instrumentation, no competing ad hoc logging, and a separate read-only telemetry store.
- **Failures that left no trace today** (each one cost time to find):
  - a picture refused because its size differed from the request stalled a job silently;
  - an image request that failed before the image service was called left no log line, so the
    market's picture failing "instantly" had no recorded reason until a line was added;
  - a stage that disagreed with its stored state ended the whole image runtime with no
    explanation of which stage;
  - the phone's fallback when the server sent empty text was visible only in the browser console.
- **Surfaces the CLI could not reach today:**
  - what the phone is actually drawing, or why it is stuck (the console was the only source);
  - the state of the plate queue per scene, and why a given plate is late or never came;
  - the developer screen link itself was broken for a while;
  - driving a phone's actions had to be done by hand in a browser.
- **The CLI was added to per system, not by rule.** Each family was written when its system was
  built, so coverage depends on who built what.

## Shops, inventories and item exploration (ideas D20 to D23), restated

- **Shop state and inventory state are separate (D20).** A shop is its own machine: its stock,
  its prices, its purse, the haggle in progress and any open offer. An inventory is its own
  machine too: what each character or the party holds, what is worn, what is in a container,
  and moves between them. Each follows D4: it owns its states and changes shared facts only
  through effects.
- **Items are in the picture (D21).** Every item that matters in a scene, such as a shop's wares
  or something lying on a table, can be seen in the display plate. A player can pick it out of
  the picture and explore it. The picture and the item list must agree, so an item that is
  listed but cannot be seen counts as a fault.
- **Telling the DM what you are looking at (D22).** A player can say "I'm looking at this item",
  by tapping it in the picture or in words. The table enters an item-explorer state for that
  item, and the DM (the storyteller) is told, so what it says next is about that item.
- **Different views of the item (D23).** In that state a player can look at the item from other
  points of view and angles (front, back, from above, underneath, a close-up of a detail, held in
  the hand), and the table sees each view.

Observations from the current code (verified 2026-11-02):

- **A close-up image exists.** The image service has an `item_close_up` purpose, and the scene
  carries an item card and a list of "hand items", each with an image. One playthrough made seven
  close-up generations. I did not check how they looked or how players reach them.
- **Items can be inspected, but not turned.** Each scene thing has actions such as look, take,
  steal and use, and the phone lists "Inspect the ..." choices. There is a single view of an
  item, with no angles.
- **Shop state is tangled in conversation.** Trade terms carry a price range, stock and purse
  revisions, and an offer is a formal step the player must accept in words ("I accept your offer"),
  but the shop logic sits inside the dialogue code, as there is no market phase.
- **Items are not placed in the picture.** An item has a free-text position ("on the wharf"). The
  plate prompt is not guaranteed to show it, and nothing checks that it appears. In the market
  playthrough the plate never arrived, so the vendors' wares were never seen at all.
- **Inventory is minimal.** Today's state listed items held by Pip (armour and the golden bread
  roll after purchase). I did not examine containers, equipping or transfers.

## Extensibility (idea D24), restated

Two kinds of future change are named:

- **Refinements.** Adjusting how an existing interaction plays: a new rule inside the market, a
  different pace for combat, a new view of an item, a new kind of hook.
- **New states.** Adding a whole new kind of interaction beside lobby, creation, explore, combat,
  market and the rest (for example a puzzle room, a ritual, a chase, a downtime scene).

The aim is that either one is a contained change: new code in a new place, no edits spread through
the rest of the engine, and nothing in an existing machine able to break because a new one arrived.

Observations from the current code (verified 2026-11-02):

- **The phase list is closed.** The set of phases is a fixed enum in the wire protocol, and the
  logic that moves between them is spread across three very large files. That is why there is
  still no explore phase and no market phase: adding one means touching the protocol, the actor,
  the dialogue code and the journey code together.
- **The codebase already uses a registry pattern elsewhere.** Text and image providers are
  registered (`df-providers/src/registry.rs`), and boundary checks enforce who may call what. The
  same idea applies to machines.
- **Refinements are often code edits today.** Tunables such as the plate interval, text deadlines
  (the 8 second observation limit) and the number of workers have been fixed in code or set by
  environment variable, each in its own style.

## Observations from the current code (verified 2026-11-02)

- **One phase list.** The wire model has `ROOM`, `CHARACTER_CREATION`, `OPENING`, `DIALOGUE`,
  `COMBAT`, `COMPLETE`. There is no explore phase and no market phase; buying and haggling happen
  inside `DIALOGUE`.
- **Logic is concentrated.** Phase movement lives mostly in `gameplay/actor.rs` (7,223 lines),
  `gameplay/dialogue.rs` (11,897 lines) and `gameplay/journey.rs` (9,031 lines).
- **Existing plans.** `planning/engine-overview.md`, `narrative-engine.md`, `interaction-engine.md`
  and `runtime-directors.md` describe a larger pure engine. This redesign has to say how it
  relates to them (question 2).
- **Single path today.** All text requests are built in three places (storyteller dispatch, the
  Jev chooser, hosted support) and sent to the one durable text service; provider image backends
  live only in `df-imagery`. Both boundary checks pass. Video generation and story-memory
  embeddings use other providers.
- **Bugs met while playing, which explicit states would have named** (an observation about missing
  structure, not proof that states would have prevented each one):
  - a successful talk-down cleared the scene text and the phone drew nothing;
  - the phone's input stayed disabled after a different character answered;
  - travelling to a new place held the table on "Sending..." for about seven minutes, because the
    place and its people were written inside that one request;
  - the market scene kept the previous scene's picture;
  - a typed attack was read as a threat and no fight began;
  - a tap on "Do this" ran an observation the player did not mean to run.
- **Where single-path bugs came from.** At the edges of the path, not outside it: a deadline too
  short for the slow backend, a read race reported as "unknown", an image whose size differed from
  the request, and per-poll re-hashing that starved the runtime. A single path removes duplicated
  clients; it does not remove those.

## Working plan (draft, not yet accepted)

Everything in this section is the assistant's proposal built from D1 to D7.

### Principles

1. **A machine owns its states and nothing else.** No machine reads or writes another machine's
   state. Shared facts live in table state, and a machine may only change them through typed
   effects the engine applies.
2. **Machines request, they never call.** A machine that needs text or a picture emits a typed
   resource request. It does not call a service and does not wait. The answer comes back later as
   an ordinary input event. This is how D6 stays true, and it is why a slow place-writing job can
   no longer freeze a table.
3. **The display is a pure projection.** What the TV and each phone show is computed from table
   state and from which resources are ready. It is never edited by a machine. A resource that is
   not ready is an explicit placeholder state, not the previous scene's leftovers.
4. **Every resource is audited at one place.** One resource broker is the only caller of the text
   and image services. It records each request (table, machine, state, purpose, key) and its
   outcome, so any picture or line on screen can be traced.
5. **Transitions are explicit and checked.** Entering and leaving a machine are named moves with
   their own rules, and an illegal move is refused with a reason, not ignored.

### Shape

Superseded where it conflicts with RES1 to RES34 and sections C, D, F, H, J and K of the Analysis;
the main conflicts are noted inline below.

- **Table state.** Shared facts (party, place, time, inventory, purses), the id of the active
  machine, and one private slot per machine.
- **Scene record (draft, from D8 to D11).** Part of table state, changed only through effects.
  It holds:
  - the **cast**: one entry per character present (party, named people, anyone unseen), each
    pointing to a character record;
  - the **set**: every set piece with an id, a kind, its state and where it is;
  - the **layout**: where each cast member and set piece stands, as structured data (areas and
    which areas touch, with an optional grid when combat needs one) instead of free text.
  The scene record is the one input to exploration, conversation, combat and the picture
  prompts, so they cannot disagree about who is where.
- **Character record (draft).** The sheet (rules facts), the persona (voice and manner), what the
  character thinks now, backstory, what they know and what they hide, and their narrative hooks.
  Player characters and non-player characters use the same shape; a non-player character may
  fill fewer fields. Each field says whether players may see it.
- **Scene builder (draft, from D12 to D15).** A pipeline with five stages: Describe, Background,
  Plate, Locate, Ready. Each stage is one request through the resource broker, its output is
  stored and audited, and the next stage starts only from a stored output.
  - The **description** is stored with the scene. The background, the plate prompt and the scene
    record are all made from it, so they cannot disagree.
  - **Locate** produces, for every cast member, a region in the plate and a head point, and
    checks them against the cast list. A character it cannot find is marked unlocated, with an
    explicit fallback point, and never silently dropped.
  - Locate writes screen anchors (a mask, a head point, a hotspot). Outside combat they only place
    overlays and hotspots, and the layout of D.6 is the rules input. In combat, gameplay effects (targeting,
    reach, areas, cover) read the rules owner's state and never the picture (RES1). SAM takes one
    request per prompt and returns instances, not identities (J.1).
  - The display shows a clear "setting up" state until the scene is Ready. It never shows the
    previous scene's plate under a new scene (K.2 governs: an establishing card after
    `scene_build_budget_secs`, not an open-ended "setting up").
- **Plate scheduler (draft, from D16 to D18; superseded in part by RES5, RES9, RES10, D.1 to D.3,
  K.2, K.3 and H.7).** Three parts, each separate and testable:
  - A **clock** that ticks every `plate_interval_secs` (default 15) while a scene is active. Each
    tick takes a snapshot of table state and turns it into a plate job (staging description, cast
    references, layout) and puts it on the plate queue. The tick does not wait for anything.
  - The **scene builder** takes jobs from the queue in order and produces plates through the
    resource broker. Every job keeps its tick time, so lateness is measured, not hidden.
  - A **playout buffer** holds finished plates and shows them in order at the configured rhythm.
    If the next plate is not ready at its slot, the display keeps the current one and reports how
    far behind it is. It never shows a plate from a different scene, and it never skips a slot
    with an empty or half-built picture.
  - **Cost view.** The config shows `plates per hour` and an estimated `cost per hour` computed
    from the interval and the prices of the stages used (plate generation plus the locate pass),
    marked as an estimate where a stage is unpriced.
- **The DM pipeline (draft, from D25).** One path for every player input, whatever the screen.
  This is version 1; version 2 (admission, four outcomes, fallbacks) is in the Analysis,
  section C:

```
  player says or taps something (text, voice, hotspot, button)
        |
        v
  1 INTERPRET    turn it into an intent from a closed list of D&D actions
        |         (move/travel, attack, cast, use item, interact, search/investigate,
        |          persuade/deceive/intimidate, insight, help, hide, dash, disengage,
        |          rest, trade, craft, ...), with its target and how
        |         unclear? -> ask one short question ("attack Dain Rusk, or threaten him?")
        v
  2 CHECK        are the rules and the scene willing? (reach, items, targets, hooks, state)
        |         not viable -> decline with the reason and what would be possible
        v
  3 DECIDE       rules set the mechanics (rolls, advantage, DC, damage)
        |         the DM chooses the transition or update that follows
        v
  4 APPLY        send it to the machine registry as typed transitions and effects
        |         (enter combat, start travel, open a shop, change an inventory, ...)
        v
  5 NARRATE      the text service tells what happened, from the already-decided outcome
```

  - **One interpreter.** Only the DM pipeline requests moves between machines. A machine may ask
    for a hand-off, but it reaches the DM, which checks and applies it. Every transition therefore
    has one audit record: what the player said, the intent read, the rules that allowed it, and the
    effects applied (D19).
  - **Rules first, words after.** The text service never decides an outcome. It tells one that the
    rules and the DM have already settled, so a refusal or a miss cannot be turned into a success
    by the wording.
  - **Viable actions are computed.** The scene record and the active machine give the list of what
    is possible now. The same list feeds the phone's offered choices, the "what would be possible"
    text when something is declined, and a CLI command that prints it.
  - **No dead ends.** Every input ends in one of: applied, needs one clarifying question,
    declined with a reason, or pending a choice, a roll, a reaction or a ruling (RES16). Nothing
    ends in silence or in "Waiting for the game server".
- **Closed inside a machine, open across machines (draft, from D24).** This resolves a tension
  between D3/D4 (strict, checked states) and D24 (new states later).
  - **Inside one machine** the states and moves are a closed set (a Rust enum), so the compiler
    proves every state is handled.
  - **Across machines** the set is open: a registry. A machine is a module that implements the
    machine contract (see below) and registers itself with an id, its entry points, the machines
    it may hand off to, its state schema and its projection.
  - **Adding a machine** is adding that module and one registration line. Nothing else is edited.
    The engine checks the registry at start and in tests: every hand-off target exists, every
    machine has a projection, every machine has a CLI family and log events (D19), and every
    machine's state can be saved and loaded.
- **Extension points (draft, from D24).** Each is a registry or a data table, not a code branch:
  - **Machines** (above).
  - **Resource kinds and purposes**: a new picture or text purpose is registered with its
    deadline, size rule, retry policy and price, and is reached only through the broker.
  - **Effects**: the small language machines use to change shared state. New effects are added
    deliberately and versioned, because every machine depends on them.
  - **Item views and hook types**: new angles and new kinds of narrative hook are data entries
    with a handler.
  - **Configuration**: every refinement-sized knob (intervals, deadlines, counts, thresholds) is a
    named, documented, typed setting with a default, readable and changeable from the CLI, and its
    effect on cost is stated where it has one (D18). A refinement that is only a number never
    needs a code change.
  - **Content**: scenes, shops, hooks and characters are data that machines read, so authors add
    content without touching engine code.
- **Versioning (draft, from D24 and D7).** Breaking changes are free until stabilization (D7), but
  the structure is built for after it: every machine's saved state carries a schema version, a
  machine may add a migration step when its state changes, and the engine refuses to load state
  it has no migration for instead of loading it wrongly.
- **A checklist for a new machine (draft).** The conformance suite is the definition of done: it
  fails until the machine has a registration, a projection that is never empty, log events for
  every transition and failure, a CLI family, a saved-state schema, an entry and exit that return
  the table intact, and a test that runs it alone against a scripted resource broker. It also
  needs: `accepts`; the dice input and `Waiting{pending_id}`; operation-id and `view_revision`
  handling; the typed `carry` list on exit; a declaration for in-flight requests on suspend
  (H.9); `force`, `input`, `dump` and `viable` (H.16); participation in the sentinel-secret and
  paired-state tests (H.11); `schema_version` and `load.refused` (H.12); and an entry in the
  golden sessions (H.17).
- **Shop and inventory machines (draft, from D20).**
  - The **shop machine** states: browsing, haggling, offer open, completed or declined. It holds
    the shop's own stock, prices and purse. A completed deal is one effect that moves the item and
    the coin together or not at all.
  - The **inventory machine** holds who has what and where, including worn and contained items.
    A purchase, a theft, a gift and a drop are all moves in it.
  - Both read the scene record for what is present, and both write only through effects.
- **Items in the picture (draft, from D21).** The scene builder extends to items:
  - the staging description lists every item that must be visible and where it sits in the layout;
  - the Locate pass looks for each listed item as well as each character (SAM 3.1 takes a text
    prompt for what to find), and returns a region per item;
  - an item that is listed but not found is flagged, and the plate is retried or the item is
    shown from its own picture instead, so it is never silently missing;
  - each located item becomes a **hotspot** on the display and on the phone: a tap on it starts
    exploring that item.
- **Item explorer machine (draft, from D22 and D23).**
  - **Enter** by tapping a hotspot or saying "I'm looking at the X". The engine records which item
    and which player, tells the storyteller (so replies are about that item), and shows the table
    that exploration has begun.
  - **Views** are a small fixed set (front, back, top, underside, detail, in hand, in place). Each
    view is one generated picture of the same item, made with the item's first picture as a
    reference so it stays recognisable, stored per item and view, and reused the next time.
  - **Inspect** a view to learn more. What is found can depend on a check or on what the
    character knows (D10), and anything found is written back as a fact about the item, so it
    stays true later.
  - **Leave** returns to the machine the table came from, with its state intact (D4).
- **Logging and CLI rules (draft, from D19).** Rules that the engine's structure enforces:
  - **One event shape.** Every event carries the table, the machine and state, a correlation id,
    the resource id if any, a cause code if it is a failure, and timings. Events go through the
    shared instrumentation path and nowhere else.
  - **No silent edges.** An error that crosses a boundary (machine to broker, broker to service,
    scheduler to builder, engine to display) is logged where it crosses, with its cause. A check,
    like the existing boundary checks, fails the build when a boundary drops an error without an
    event.
  - **A command family per surface.** Every machine, the scheduler, the scene builder, the
    broker and each projection registers the same small set of commands: `list`, `show`,
    `history`, `drive` (send an input as a player or a timer would), and `config` (read and
    change settings). A conformance test fails when a registered surface lacks one of them. H.16
  adds a second required set for machines (`force`, `input`, `dump`, `viable` and more), and
  `drive` here is H.16's `input`.
  - **Reading is free, driving is explicit.** The CLI can always read. Driving a surface goes
    through the same entry points as a real input, so a drive command cannot reach state a player
    could not reach.
  - **Projections are inspectable.** The CLI can print exactly what the TV and each seat's phone
    are being sent, and the reason the display is in its current state (for example "behind by
    42 seconds", "setting up", "unlocated: courier").
- **Dressing the set (draft).** Entering a scene runs a "dress" step: the place's set pieces and
  layout are produced (authored, or written through the text service and then checked), stored,
  and audited like any other resource. The display shows a clear "setting up" state until the
  scene record is complete, so a scene is never shown half-dressed (K.2 governs the wait).
- **Machine contract (superseded by RES27).** `enter(table, scene) -> state`; `accepts(seat, input,
  state)`; `step(state, input, clock) -> StepResult{state, table_effects[], resource_requests[],
  outcome}` with the four outcomes and `Waiting{pending_id}`; `exit(state) -> HandOff{target,
  carry[]}`. Inputs include dice draws, operation ids and engine proposals. Read RES27 for the full
  statement.
- **First machines (RES28).** Lobby, creation, advancement, explore, dialogue, travel, rest, combat,
  shop, inventory and item inspector. "Market" is a kind of place and not a machine. Further kinds
  (downtime, puzzle, chase, adventure end) are added the same way.
- **Resource broker.** One service in front of the text service and the image service, with
  deadlines, retries and size rules in one place per resource type. It is the `Hub` of
  `gameplay/resources.rs` extended with text and image kinds, queue classes, leases and epochs, and
  not a second service (J.1).
- **Display projection.** A pure function from table state and resource readiness to the TV view
  and each phone's view, with a non-empty result guaranteed for every state. It extends
  `df-display::project_display_view` and `wire::project_journey` and is fed from an immutable
  snapshot (RES30).
- **Breaking-change policy.** No compatibility layer. A stabilization gate (question 8) ends it.

### Why this shape (draft reasoning)

- It makes each failure above a named state or a refused transition instead of an accident of
  shared code.
- It moves every wait for a slow resource out of the player-facing path.
- It lets each machine be tested alone with a scripted resource broker, with no provider and no
  browser.

## Open questions

Refinement rounds 1 and 2 (Analysis sections B and J) supersede the draft defaults of questions 2,
4, 5, 6, 9, 12, 14, 15, 16, 19, 20, 21, 25, 36, 42, 44, 50, 51 (C.5), 53 and 54. Questions 24 and
28 stay open but are affected by H.7 and D.1. The original text is kept for the record, and
questions 56 onward are new.

1. **Confirm the scope.** Is "gameplay display" the TV and phone presentation, with the engine
   rebuilt so it is correct by construction? Or is it the TV screen alone?
2. **Relation to the existing plans.** Does this redesign supersede the gameplay parts of
   `engine-overview.md`, `narrative-engine.md` and `interaction-engine.md`, or sit inside them?
   Draft default: supersede the presentation and machine structure of the gameplay plans and keep
   the rules, engine, session, persistence and knowledge plans (Analysis section A).
3. **Machines as code or data.** Draft default (revised for D24): a typed Rust enum for the states
   inside each machine, so the compiler checks every state and move, and a registry across
   machines, so new machines are added without editing the others.
4. **Leaving and returning.** When a table leaves a machine and comes back, does it resume where
   it left off or restart? Draft default: resume, with an explicit "discard" move.
5. **One active machine or several.** Can two machines be active at once (a market conversation
   and a timer, or a market during combat)? Draft default: exactly one active machine, others
   suspended.
6. **Who chooses a transition.** Settled in part by D25: the DM does. Draft default for the detail:
   the DM (Jev and the storyteller reading the player's words) proposes the intent, the rules decide
   whether it is allowed and what the mechanics are, and the DM applies the result. The text
   service never decides an outcome.
7. **What counts as a resource.** Text and pictures are clear. Do voice, music, video and story-memory
   embeddings join the same broker now, or later?
8. **The stabilization gate.** What ends the breaking-change period: a version, a feature list, a
   number of clean playthroughs?

9. **Who gets a sheet.** Do non-player characters get a full rules sheet, or a lighter one that
   grows only if they enter combat? Draft default: one shape for everyone, with a light fill for
   people who never fight.
10. **What players may know.** Persona, thoughts, backstory and hooks include secrets. Which parts
    can the table see, and which are hidden from players but visible to the engine? Draft default:
    every field is tagged with who may see it, and the display projection enforces it.
11. **Where hooks come from.** Are hooks authored, written by the text service, or both? And does a
    player see them as choices, or does the storyteller weave them in? Draft default: typed hooks
    with a precondition and an outcome, written by either, and shown as optional leads.
12. **How precise a layout.** Named areas with adjacency, or true coordinates on a grid everywhere?
    Draft default: areas everywhere, a grid only inside combat, and one conversion between them.
13. **Who dresses the set.** Authored places, generated places, or both? A generated set has to be
    checked (no missing exits, nothing placed in an impossible spot) before the table sees it.
    Draft default: both, with the same checks.
14. **Unseen characters.** Does "who is in the scene" include someone hiding, invisible or
    just out of sight? Draft default: yes for the engine, no for the display, with a sight rule.

15. **Which is the source of truth for position.** The structured layout (the engine decides who
    stands where and the plate must match), or the plate (the picture is made first and SAM
    measures where people ended up)? This is the central tension in D11 and D15 together. Draft
    default: the plate is measured and its measurement becomes the layout for that scene, with the
    prompt asking for the intended arrangement.
16. **When a plate is rebuilt.** *Mostly answered by D16:* on a clock, every 15 seconds. What is
    still open is whether a clock tick always makes a new plate, or is skipped when nothing has
    changed since the last one (the current policy has a `no_visible_change` case). Draft default:
    skip a tick when the staging description and the layout are unchanged, so a quiet table does
    not pay for identical pictures.
17. **Background and plate.** Is the plate an edit of the background (same room, characters added),
    or a fresh picture that uses the background as one reference? Draft default: an edit, so the
    place stays the same between plates.
18. **Who writes "what is happening on screen".** The text service from table state, the
    storyteller, or authored beats? And is it the same text players read? Draft default: the text
    service writes a short staging description from table state, stored with the scene.
19. **What removing cutouts takes with it.** Cutouts today also give the phone's speaker portrait,
    the hero library, and possibly combat tokens (not verified). Do those become crops of the
    plate, separate portraits, or go away? Draft default: speaker portraits and tokens are cut
    from the plate by the SAM masks, so there is one source of pictures.
20. **If SAM misses or doubles a character.** Retry the pass, regenerate the plate (bounded), or
    accept an unlocated character with a fallback point? Draft default: retry once, then one
    bounded regeneration, then the explicit fallback.
21. **References per plate.** The current plate takes up to three character references. For a
    party plus several non-player characters, which are chosen and how are they ranked?
22. **Speed target.** A scene build is two generated pictures and a SAM call, one after another,
    and a Codex picture takes about 1.5 to 4 minutes. What is the longest the table may wait, and
    is the next scene built ahead of time?

23. **"Display play" means "display plate".** Confirm.
24. **When the pipeline is behind.** Strict order, where every queued plate is eventually shown and
    the delay grows? Or coalescing, where a queued plate that is already stale is dropped so the
    display catches up? "Even if relatively delayed" reads as strict order. Draft default: strict
    order up to a latency limit, then drop the oldest stale jobs, with every drop logged. (This is
    about freshness, not spending.)
25. **What the table sees while behind.** Hold the current plate and show a small "behind by N
    seconds" mark, or hold it silently? Draft default: show it only in the host and developer
    views, not to players.
26. **What "consistent" means.** A fixed gap between plate changes on screen (a fixed beat), a
    minimum time each plate stays up, or both? Draft default: both. The configured interval is
    the beat, and a plate that arrives early waits for its slot.
27. **The scene boundary.** On a scene change, do queued plates for the old scene get dropped and
    a new one built at once, and does that new plate jump the queue? Draft default: yes, the scene
    change flushes the old scene's queue and starts a fresh one.
28. **How the interval is set.** One global value, per table, or per scene type (a market could
    tick slower than a fight)? Draft default: one value per table, with an optional per-machine
    override.
29. **The cost estimate.** Stages that are unpriced (the Codex subscription) have no dollar figure.
    Should the estimate show "n/a" for them, show time instead, or use a per-stage override the
    owner enters?

30. **What counts as a surface.** The list above (machines, scheduler, builder, broker,
    projections, config). Is anything missing, for example the cost ledger, saved tables, or
    the hosted account surfaces?
31. **Volume.** A plate tick every 15 seconds, per table, plus every transition and every
    resource request, is a lot of events. Is full volume always on, or is there a level setting
    with the failure and transition events never sampled? Draft default: transitions, failures
    and resource requests are never sampled; per-tick detail is a level.
32. **What must be redacted.** Player lines, character backstory and secrets (D10), keys and
    tokens. Draft default: secrets are logged by id, never by content, and keys never.
33. **How long logs are kept**, and who may read them in a hosted game. Draft default: developer
    access only in local and development modes; hosted logs are read through the existing
    read-only telemetry store.
34. **Driving versus playing.** The project's playtests are done by hand through the real screens,
    with the CLI used only to read. Should the new CLI keep that split (read freely, drive only
    for scripted tests), or may it drive live tables? Draft default: keep the split, and mark
    driven inputs in the log so they can be told apart from real play.
35. **Failure policy at a boundary.** When a boundary drops an error today, the table sees nothing.
    In the new engine, does every failure also surface to the display as a state ("this picture
    failed, retrying"), or only to developers? Draft default: to developers always, and to
    players only where it changes what they see.

36. **Whose view is it.** State is shared per table (D5). When one player explores an item, does the
    whole table see it on the TV, with that player's phone choosing the angle, or does each phone
    explore alone while the table stays in the scene? Draft default: the TV shows the active
    explorer's view, other phones can follow or step out, and one explorer at a time per item.
37. **How the DM is told.** Does the storyteller get a note ("the party is examining X, view Y")
    that shapes its next lines, or must the player ask a question first? Draft default: a
    structured note on entering and on each new view or finding.
38. **How many views, and when they are made.** Seven views per item is a lot of pictures, and a
    Codex picture takes 1.5 to 4 minutes. Generate a view when first requested, or make the
    likely ones in advance for items the party has shown interest in? Draft default: on first
    request, with the front view made when the item is created.
39. **Which items are explorable.** Every item, or only items marked as worth exploring (wares,
    clues, loot)? Draft default: marked items only, set when the item is created.
40. **What exploring reveals.** Can a close look turn up a hidden detail, a clue or a secret?
    How is that decided (a skill check, character knowledge) and who may see it? Draft default:
    a check or knowledge decides it, and the result is a visibility-tagged fact (see question 10).
41. **Items in the plate versus on their own.** An item small or far away may not read in a wide
    plate. Does the engine show it in the plate anyway, or give such items a thumbnail hotspot
    that opens their own picture? Draft default: located items get hotspots, and small ones also
    appear in a strip beside the plate.
42. **Shops outside conversation.** Can a shop be entered without talking to its keeper (browse
    the stalls), and does haggling stay a typed conversation or become choices? Draft default:
    browsing is its own state, and haggling stays free text with a rules check.

43. **How open should "new machines" be.** Only code in this repository, or also content packs
    that can bring their own machine (a custom puzzle, say)? Draft default: in-repository machines
    only for now, with the registry shaped so packs could be added later.
44. **The effect language.** Every machine speaks to shared state through effects, so this small
    vocabulary is the hardest part to change. Who decides when an effect is added, and how is an
    old machine kept working? Draft default: effects are added, never changed, and a changed one
    gets a new version beside the old.
45. **Where authored content lives.** Data files in the repository, a database, or both? And are
    they checked when the game starts (every hook points at a real item, every exit at a real
    place)? Draft default: files in the repository, checked at start and in tests.
46. **How a machine is turned on.** Can a new or half-built machine be switched off by a setting
    so a table never enters it, while it is developed beside the working ones? Draft default: yes,
    a per-machine flag, off by default for new machines.
47. **What counts as a refinement.** A knob (a number) versus a rule change (a new check in the
    market) versus a new state. The line decides whether a change needs a migration. Draft
    default: knobs are config, rule changes are code inside one machine, new states are new
    machines, and only the last two can need a migration.
48. **Stability promise to authors.** After the stabilization gate (question 8), which parts are
    promised not to change: the effect language, the machine contract, the content format?
    Draft default: all three, with a version on each.

49. **What "the DM" is.** The engine's own game-master function (Jev, the storyteller and the
    rules together), not a human? Or could a human DM take over the same pipeline? Draft default:
    the engine's own, with the pipeline shaped so a human could use it.
50. **How big the action list is at the start.** The full 5e action set (attack, cast, dash,
    disengage, dodge, help, hide, ready, search, use an object, grapple, shove, and more) or a
    subset added in stages? Draft default: stages, with the list as a registry (D24), so each
    action arrives with its rules and its tests.
51. **When to ask.** An ambiguous line ("I attack") could be a threat or a blow. Ask every time it
    is ambiguous, or only when the two readings lead to different machines? Draft default: ask
    when the readings lead to different transitions, otherwise take the likelier one and say so.
52. **How a decline reads.** Short reason plus what is possible ("Dain is out of reach; you could
    step closer, speak to him, or draw a weapon"). Draft default: always both, drawn from the
    computed viable-action list.
53. **Several players at once.** State is shared per table (D5), so two players can speak at the
    same moment. Draft default: inputs are ordered by arrival, each answered in turn, with a
    short merge window so two players asking for the same thing are not handled twice.
54. **Time and turns outside combat.** Does an action outside combat cost time or a turn (for
    example travel taking an hour, a search taking ten minutes), and who tracks it? Draft default:
    the scene record keeps in-game time, and actions declare their time cost.
55. **Speed of interpretation.** One session made 224 judgements through Jev. Every input costs at
    least one. Is a slower, richer interpretation acceptable, or does a fast path for obvious
    actions (a tap on a hotspot, a button) skip the model? Draft default: taps and buttons are
    already an intent and skip interpretation; only free text is interpreted.

56. **Position authority and optional tactical conventions (RES1).** The rules owner's feet-based
    tactical state is the only truth for reach, range, cover and areas, and the plate illustrates it.
    Is that acceptable, given that the plate then cannot place a creature? A zone or distance-band
    model would be an optional tactical convention (R17) and would have to be chosen explicitly.
    Draft default: rules state authoritative, no band model.
57. **Who rules on the unwritten (RES15).** Where the source leaves a real choice open, the existing
    plans require an authorised adjudicator or a reviewed automatic case. May a table let the engine
    itself adjudicate within stated bounds (a ruling by analogy, reviewed and logged), or must the
    host always decide? Draft default: reviewed automatic cases only, otherwise the host, with the
    engine's suggested answer shown.
58. **Rate limits.** C.1 proposes 3 inputs a second in a burst and 1 sustained per seat. Does
    `rpc-api.md` already set limits? `rpc-api.md` sets no inbound rate numbers, so the numbers of C.1 stand.
59. **A slow or absent player in a window or turn.** The plans forbid a timeout that spends a
    resource, rolls a die or invents a ruling, so a held turn waits. Which approved commands may the
    host use to move on (pause, takeover, an explicit AFK)? Draft default: the commands in
    `remote-play.md`, with a visible notice on the TV and phones.
60. **Split party in the first release (D.1).** Decline `Split` with a reason, or support scene
    groups from the start? Draft default: decline, with the structure ready.
61. **Binding every person players may attack (RES14).** Must the dress step bind every person the
    scene lets players attack, or may some people be unattackable, with a clear reason? Draft
    default: the dress step binds every person it places in a scene.
62. **Hooks.** Are the Q11 hooks the same objects as `CharacterHook` in `narrative-engine.md`?
    Draft default: yes, one type.
63. **Table safety settings.** `character-backstory-creation.md` (O10) proposes optional "never
    include" and "hint only" chips and leaves them open. Required in session zero, or optional?
    Draft default: required step, with "no limits set" as an answer.
64. **A session with no images.** If the image service is unavailable for the whole session, may the
    table keep playing on place cards and text? Draft default: yes, with a visible status.
65. **Which text lane serves INTERPRET.** Codex text calls take 15 to 45 seconds today, and one
    session made 224 closed-label judgements through Jev. Does INTERPRET use the fast classification
    lane (with Codex only for narration and creation), and what is its deadline? Draft default: the
    fast lane for INTERPRET.
66. **A threat the party knows about and cannot see.** How is it shown on the shared TV (not at all,
    a marker, a silhouette)? Draft default: a marker, never the figure.
67. **The plate clock while a window is open.** Does the plate tick keep running while the table
    waits on a reaction or a ruling? Draft default: yes (the picture keeps its rhythm), and game time
    does not advance.
68. **Do rulings bind later play.** A stored ruling is reused when the same question comes up. Does
    the host re-confirm each time, or does it apply automatically, shown as "Ruling (earlier)"?
    Draft default: automatic, with the host able to revoke.
69. **Death and replacement.** Is a replacement character's level a host setting, and is a cheaper
    return than raise dead allowed? Draft default: replacement at party level, host-set, and no
    house-rule return.
70. **Who may read hidden content from the CLI.** On a hosted table, who may see `dm_only` and seat
    secrets through the developer commands? Draft default: nobody except the owner's operator role,
    and never through a player-facing route.

71. **Skipping and dropping plates (RES9, H.7; changes D16 and D17).** May the engine skip a tick
    when nothing visible changed, and drop the oldest queued plate beyond four? Draft default: yes,
    with every skip and drop logged.
72. **Shop and item inspector in combat (RES2, RES20; changes D4).** May a table decline shopping and
    the table-wide inspector while a fight is on? Draft default: yes, with a plain reason, because
    the fight must not freeze for everyone.
73. **Prefetch against the on-demand rule.** The `Hub` generates on demand and nothing ahead of time.
    May the scene builder prefetch adjacent places and likely item views on the lowest queue class?
    Draft default: yes, on the `prefetch` class, never ahead of a player-blocking job.
74. **Shared vision and sharing finds (RES24).** Plate light follows the lowest-vision present member
    unless the host selects shared vision, and a find becomes public when its finder says so. Should
    objects in plain view be shared with the party automatically? Draft default: yes for what any
    present member plainly sees, no for results of a check.
75. **A chase.** Is a chase module required or configured for the first release (R17)? Draft default:
    no, and a pursuit runs in the combat machine as an encounter whose objective is escape or catch.
76. **A theft the victim cannot see (J.3).** The thief's victim keeps seeing the old purse value,
    marked unchecked, until noticed. Is that acceptable to the rules owner? Draft default: yes.
77. **Plan-only notes.** Do plan-only notes appear on the TV or only on the seats' phones? Draft
    default: the phones, with a "show the table" tap.
78. **The first scene.** Is the first scene of a session built during session zero while players pick
    characters? Draft default: yes.
79. **A vote as adjudicator.** May the seats vote to adjudicate low-stakes rulings (an alternative to
    question 57)? Draft default: no, until the owner chooses it.
80. **Callbacks.** At most one callback per scene, or per hour? Draft default: per scene.
81. **Solo beats.** Is a solo beat acceptable as the answer to question 60 in the first release?
    Draft default: yes.
82. **A caller.** Does a table want a standing "caller" role for party choices? Draft default: the
    host calls it.
83. **The recap.** A budgeted video bookend or a text-and-stills card by default? Draft default:
    text and stills, with video where `campaign-cinematics.md` budgets it.
84. **Heroic Inspiration.** Is awarding it adjudicated or discretionary in the rules plan? Draft
    default: discretionary, so the engine suggests and the host decides.
85. **Pranks.** A table setting for cosmetic effects on other players' characters (agency tag
    `cosmetic`), or fold them into `pvp`? Draft default: a separate `pranks` setting, on.

86. **A minimap on the TV.** `implementation-roadmap.md` removes the grid from the TV. Does a
    toggleable minimap of the rules diagram count as that grid? Draft default: it is allowed on
    phones and off the TV until the owner decides.

87. **The new crate edge and the host crate (O.2, O.8).** Approve `df-table` as a new pure crate
    (depending on `df-types` and `df-model`) and `df-table-host` as its async host, with the new
    edges recorded in `subsystem-architecture.md`. Draft default: yes.
88. **The port set (O.4).** Are the eight ports (rules owner, interpreter, narrator, broker, clock,
    dice, knowledge view, event sink) the right minimum for C0? Draft default: yes; split a port if
    it proves too coarse, never remove one.
89. **A stable event contract (O.6).** Which fields of a class A event may never change once
    operators read them? Draft default: table, scene, machine, operation id, `turn_seq`, cause and
    `ms`, additive changes only.
90. **The first machines (O.9, C2).** Which two machines prove the `Machine` trait? Draft default:
    lobby and a minimal explore.

## Ideas to come

Add further ideas below as they are shared, in the owner's words with a restatement.

## Analysis (refinement loop, rounds 1 and 2)

This section was written after the owner asked for a refinement loop with an adversarial agent to
fill the D&D-shaped gaps and to make the engine robust, resilient, fast, logged, performant and
fully reachable from the dev CLI. Everything here is a draft proposal from the assistant, built
from D1 to D25 and from the reports of four independent critics.

### How the loop runs

1. Four critics (combat, exploration and social, characters and magic, quality) read this document
   and the code, and each report gaps with a scenario, a severity and proposed text. They change
   nothing. Their finding ids are C (combat), E (exploration), M (characters and magic) and N
   (quality), and every id is traced to a place in this section in the ledger at the end.
2. The first reports proposed many rules and mechanisms that the project's existing plans already
   own, or forbid (a model-chosen DC, auto-declined reactions, an invented effect language, a
   distance-band model, a rewind command). The critics were sent back to re-read `rules-coverage.md`,
   `rules-support.md`, `rules-effect-model.md`, `interaction-engine.md`, `narrative-engine.md`,
   `scene-combat-binding.md`, `observability.md`, `runtime-reliability.md`, `optimization.md` and
   `storage-architecture.md`, and their second reports (used here) keep only gaps in how this
   document binds to those plans. The first-pass proposals that conflicted with them are withdrawn.
3. The assistant merges the findings here, settles the contradictions, and adds the questions that
   only the owner can answer (questions 56 onward).
4. A fresh critic round reads the revised document and looks for what is still missing and for
   what the new text broke. The loop ends when a round finds no blocker and no major gap that the
   document does not already answer.

| Round | Findings | Blocker | Major | Minor | State |
|---|---|---|---|---|---|
| 1 | 91 (C 24, E 22, M 21, N 24) | 24 | 57 | 10 | merged (sections A to I) |
| 2 | 118 (W 30, K 38, F 23, X 27) | 22 | 67 | 29 | merged (sections J to L) |
| 3 | 130 (P 35, D 32, K 35, F 28) | 16 | 74 | 40 | merged (section M; section L rewritten) |

### A. Who owns what

The redesign is the layer that routes player input, holds the interaction machines, builds and
paces the pictures, projects views, and keeps one audit and CLI shape. It does not own the rules,
the catalogs, time, knowledge, story or telemetry. Those have owners in the existing plans, and the
redesign's job is to make each of them reachable, visible and observable through its pipeline,
machines and projections. This answers question 2: the redesign supersedes the presentation and the
machine structure of the gameplay plans, leaves each concern with the owner in the middle column
below, and provides what the right-hand column lists.

| Concern | Owner today (plan, crate) | What the redesign provides |
|---|---|---|
| Mechanics, legality, dice use, effects and conditions, spells, items, rests | `df-rules`, `df-content`; families R01 to R17 in `rules-coverage.md`; `rules-effect-model.md` | The pipeline steps that call them, a machine home, a projection, log events, a CLI family |
| Pending choices, reactions, rulings | `df-engine` owns `PendingResolution`; `interaction-engine.md` owns `RulingRequest` | One pipeline outcome and one projection state for waiting; never a machine-private copy |
| Durable decisions, explicit dice, exactly-once | `df-session`, `df-persistence`; `storage-architecture.md` | APPLY as that commit; effect intents; recovery by redelivery |
| Logical time, travel, weather, schedules | `df-world` | A read-only view in table state and one `RequestTimeAdvance` effect |
| What each person knows, beliefs, witnesses, secrets | `df-knowledge`; `SecretPolicy` and `ExpressionContext` in `interaction-engine.md` | Per-audience projection for the plate, the TV, each phone and each prompt |
| NPC social state, conversation, intent | `df-interaction`, `df-intent`; EMO-01 | A dialogue machine slot that holds a reference, not a copy |
| Story beats, hooks, awards, progression | `df-narrative`; `narrative-engine.md` | A call after APPLY and a journal projection |
| Encounters and enemy tactics | `df-encounter`, `df-combat`; `scene-combat-binding.md`, `tactical-combat.md` | Combat entry and exit through them |
| Generated people, places, items | `generated-content.md` (candidate, validation, admission) | The dress step, which may only use admitted content |
| Pictures and text calls | `df-imagery`, `df-ai`, `df-providers` | The resource broker in front of them (D6) |
| Telemetry | `df-observe`, `df-telemetry`; `observability.md` | The event classes and fields this document requires of each machine |
| Wire views | `df-api`; `rpc-api.md` | The projection each audience receives |
| Pacing, spotlight, beat dwell | `df-experience`, `df-tempo`; `tempo-engine.md`, `engagement-model.md` | The dwell and focus rules feed them and do not duplicate them |
| Public projection | `wire::project_journey` and `actor.rs::view` (live); `df-display::project_display_view` has no production caller | Slice 1 adopts or replaces it |

"Table state" in this document therefore means the composition the `df-engine` and `df-session`
owner already holds (`runtime-directors.md`), seen through the redesign, and not a second store.

**What the code has today** (an identifier search on 2026-11-02, corrected in round 2).
`PendingResolution`, `ResolutionWindow`, `PendingInput`, `ScopedRuling`, `ActiveEffect`,
`KnowledgeGrant`, `WitnessRecord`, `NpcState` and `CharacterHook` are defined in
`df-model/src/checkpoint.rs`, and `IntentDisposition` in `df-model/src/intent.rs`. The operation id
is `OperationId` in `df-types`, and `OperationContext` in `df-observe` is only a trace parent and
build. `RulesetId` exists only as a contract fixture in `df-types/contracts/boundary.rs`.
`Battle::reachable_cells` is in `df-rules/src/tactical/state.rs`, and `RestInterrupted` is a variant
in `df-rules/src/local_journey.rs`. I found no definition of `legal_choices`, `SpatialQuery` or
`TriggerWindow`; the nearest type to the last is `ResolutionWindow`. Where this section says "the
rules owner returns the legal choices", that is the contract the plans require, and some of it is
still to be built (section L).

### B. Resolutions of contradictions

The critics found places where this document disagreed with itself or with the existing plans. Each
is settled here, and each replaces the draft default of the question named. The owner can overrule
any of them; the ones that change something the owner said are listed again as questions 56 onward.

- **RES1. Who owns position (C1, C2, M9, E7; Q12, Q15; D11, D15).** The rules owner does. From
  combat entry, the tactical state that `df-rules` keeps in five-foot cells (R06,
  `tactical-combat.md`, the Scene-v2 grid) is the only truth for reach, range, cover, light,
  occupancy and areas, and the plate is told that state and illustrates it. SAM Locate still runs
  on every plate (D15), but its product is the **screen anchor** of each character and item: a
  mask, a head point and a hotspot. Anchors drive what happens on the picture (hotspots, hit and
  damage marks, target taps, the turn marker) and never move a creature, change its engagement or
  its range. Outside combat the measured layout may stand as the presentation layout (Q15's
  draft default), and entering combat converts it once into the rules state through one declared
  conversion. If an anchor disagrees with the rules state, the creature does not move: the plate is
  retried within the bound of question 20, then the anchor is reassigned, and the disagreement is
  logged. A distance-band or zone model is not adopted, because `rules-coverage.md` leaves optional
  tactical conventions to an explicit decision (question 56).
- **RES2. One active machine per scene, not per table (E5, M20, C13; Q5, Q36; refined by
  RES20).** A table holds one or more scenes, each the group of seats in one place and each with
  exactly one active group machine (section D). Opening a seat's own sheet, inventory view or
  item picture runs in the seat's lane (RES20) and changes no group machine. Inside combat an inventory act (draw,
  stow, drink, hand over) is a combat intent with a rules cost; shopping and the table-wide item
  inspector are declined with `not_in_combat`.
- **RES3. What survives a hand-off (E13; D4, principle 1).** Conditions, durations, resources,
  hidden status, alert level, positions and the clock are rules or world state in table state, not
  machine state, so they survive a hand-off by construction and D4 holds: a machine owns only its
  own interaction state (an open offer, a puzzle's hint tier, whose turn is on the initiative
  tracker). A hand-off is a request the DM pipeline validates, carrying a typed `carry` list the
  registry check verifies. A suspended machine's timers pause with logical time (H.9), and
  resource results that arrive for it are held and delivered on resume, or stored as `late` if the
  scene epoch has moved on (D.2).
- **RES4. Where waiting is allowed (principle 2).** Principle 2 binds machines. The DM pipeline has
  two model waits that gate a committed outcome, INTERPRET (free text only) and NARRATE, each with a deadline
  and a fallback (sections C.7 and H). Neither can block a rules outcome from being committed.
- **RES5. Time has one owner (E4, M8, C8; Q54).** `df-world` owns causal time. Table state shows
  `world_time` read-only, and the only way to change it is the effect
  `RequestTimeAdvance{seconds, reason, cause}`, which `df-engine` turns into a `df-world` proposal.
  The time a thing costs comes from the rules and world tables (R13, R14), not from the action's own
  declaration. Durations are `ActiveEffect` expiries. The 15-second clock of D16 is renamed
  `plate_tick`, is wall time, and never moves game time. A paused campaign stops the scheduler.
- **RES6. Hidden things and the plate (E1, M3, M13, C15; D21, Q14, Q20).** Each fact has an
  audience from `KnowledgeGrant`. The staging description, the plate prompt and its cast references
  are built from the public projection only, per the paired-state rule in `interaction-engine.md`
  (the public context is byte-identical under different hidden facts). Hotspots exist only for what
  the table knows. A hidden thing is `expected_visible = false` and exempt from the "listed but not
  found" fault, so the fault test becomes "expected visible and known". The viable-action list is
  projected per seat from what that seat knows, so it never offers an undiscovered door.
- **RES7. Dialogue (E9, M10, M16; D25).** An NPC's spoken reply is an outcome, so the path is the
  one in `interaction-engine.md`. Rules and NPC policy decide the stance from `NpcState`,
  `Relationship`, `Belief` and `SecretPolicy`; a `SpeechActPlan` carries the permitted claims; the
  text service voices a `SpeechEnvelope` of grounded and flavour clauses; `qualify_expression`
  checks it before captions or voice. A promise exists only through an explicit approved commitment
  action that creates an `Obligation`. The redesign adds the dialogue machine and its projection
  and invents no social records.
- **RES8. The action list (C23, C7; Q50).** The registry keys every intent to a rules entry id and
  a `RulesetId`, with localisable labels, and uses the 2024 action names (Attack, Dash, Disengage,
  Dodge, Help, Hide, Influence, Magic, Ready, Search, Study, Utilize). An action not built yet is
  declined as `not_supported_yet`, never as "not viable".
- **RES9. A quiet table (Q16; D16 to D18).** The slot always arrives on the beat. If nothing
  changed, the display keeps the current plate for that slot and no plate is paid for. The
  `cost per hour` figure is stated as an upper bound.
- **RES10. A late picture (Q25; C14, N16).** Players see a plain "picture catching up" mark when
  the plate is more than `image_stale_limit_secs` behind (default 90, to be set from the measured
  latency of the lane in use; `runtime-reliability.md` requires an explainable waiting
  condition), and only while the stale plate disagrees with the table (K.3). Every state change
  that matters (a creature down, a turn change, a condition) is drawn at once as an overlay on the
  stale plate, and a downed creature dims immediately. Q25's silent hold is replaced.
- **RES11. Edition, rights and generated content (M7, M15, C11).** The contract is the standard 2024
  rules family under the source gate of `rules-coverage.md`, and table state holds the `RulesetId`.
  A session keeps its `RulesetId`; a change is an explicit validated migration or a new session.
  Every resource request and generated definition carries catalog and source provenance. A
  generated person, set piece or item enters through candidate, validation and admission
  (`generated-content.md`); its rules facts come from an admitted catalog entry, never from model
  text.
- **RES12. Simultaneous input (C5, E20; Q53).** Every input is an `InputEnvelope` with an operation
  id (section C.1). A duplicate operation id returns the stored result and never runs twice. There
  is no merge by meaning. Two seats asking for the same thing are two inputs: outside combat the
  pipeline offers the group check or Help the rules allow, and an input that conflicts with another
  seat's becomes a `pending{choice}` for the affected seat. An act on another player's character
  follows the agency tags of RES23. In combat only the active creature's controller can spend its
  economy, and everyone else may answer a window, speak, or queue a plan-only note.
- **RES13. Cutouts (D14; Q19).** No avatar cutouts and no tokens cut from the plate. A speaker's
  phone portrait is the character's own portrait (the creator already makes one). Combat shows
  overlays (badge, turn marker, condition chips) anchored to the screen anchors of RES1.
- **RES14. Sheets and combat binding (D9; Q9; C11, M15).** Every character record carries a
  `stat_ref` to an admitted catalog entry (R15) from the moment it exists, and a combat-capable
  character carries the `combat_binding` that `scene-combat-binding.md` requires (profile, identity
  revision, objective). The record is a projection of its owners (M6): sheet from `df-rules` and
  `df-content`, persona and profile from the dossier and EMO-01, knowledge from `KnowledgeGrant`
  and `SecretPolicy`, hooks from `CharacterHook`. A character with no binding is declined
  `no_combat_profile` with the alternatives (speak, leave, and so on), and the dress step must
  bind every person the scene lets players attack (question 61).
- **RES15. Who rules on the unwritten (E2, C6, M4, M5; D25).** D25 said the DM handles any viable
  action. The existing plans split that. The DM interprets and routes; the rules decide whatever a
  source covers; where the source leaves a choice open, the answer is a `RulingRequest` to an
  authorised adjudicator, or a reviewed automatic source-grounded adjudication, and never a
  model-chosen DC (`interaction-engine.md`: no LLM sets a die, a DC or an automatic success). The
  DM's part in a ruling is the mapping from the player's words to the source question and a
  suggested answer. Whether a table may let the engine itself act as the adjudicator within bounds
  is the owner's decision (question 57), and product copy must disclose the human judgment that
  remains.
- **RES16. Four outcomes (E3, C6, M4).** Every input ends in one of: `applied`,
  `needs_clarification`, `declined{code, reason, alternatives}` or `pending{choice | roll | reaction |
  ruling}`. The fourth is new. A pending resolution is a `PendingResolution` owned by `df-engine`
  and held in table state, never in a machine's private slot, so it survives machine exit,
  reconnect and restart, and nothing in the table's waiting spends a resource, rolls a die or
  invents a ruling.
- **RES17. D7 and what is durable (C4, M7).** D7 (no saved-game compatibility) applies to machine
  schemas and presentation state. Rules state, the `RulesetId`, pending resolutions, awards and the
  turn commit follow the migration rules of `storage-architecture.md` and `rules-coverage.md`, and a
  load the engine cannot migrate is refused visibly (section H).
- **RES18. Two effect vocabularies (E18, M1; Q44).** The redesign's small language of changes is
  renamed `TableEffect`. Mechanical effects (`ActiveEffect`, durations, concentration, conditions,
  `PendingResolution`) are produced only by `df-rules` and carried inside a `TableEffect` without
  being redefined, so no machine invents a duration.
- **RES19. Replay and recovery (N1; C4).** Recovery is the snapshot plus committed effect intents
  (`storage-architecture.md`), not event sourcing. The turn log is a diagnostic and test record,
  and replay is supported only where reducers are compatible and the state hash is checked.

### C. The DM pipeline, version 2

Version 1 (D25) was five boxes. The critics showed it needs an admission step in front, a fourth
outcome, and honest fallbacks. The five names are kept.

```
  Tap(hotspot or button, view_revision)   SubmitText   PrivateMessage   SafetyCard
        |
        v
  0 ADMIT      InputEnvelope + OperationId; seat from the session, never from the message;
        |      duplicate operation id -> stored result; rate limit; view_revision older than
        |      the seat's last legal-choice change -> CHECK re-runs, it is not dropped
        v
  1 INTERPRET  free text only; taps and offered choices are already an intent and skip this
        |      IntentDisposition: Action | Question | Social | PlanOnly | Meta | Joke |
        |      Clarify | Rejected; an Action is an ActionPlan of at most six PlanSteps
        v
  2 CHECK      the rules owner returns the legal choices for the actor; affordances of set
        |      pieces; what the actor can perceive; combat binding; safety and agency policy
        |      -> ok | declined{code, reason, missing, alternatives} | confirm | needs_ruling
        v
  3 DECIDE     prepare and resolve with the explicit dice df-session supplies; Influence and
        |      other source flows; a stored ruling is reused; an open source question becomes a
        |      RulingRequest; the result is one typed Outcome (draws, effects, windows)
        |      an Outcome that owes someone a reaction is applied in two phases
        v
  4 APPLY      one df-session fenced decision commit: decision, dice counters, resources,
        |      TableEffects, and the committed effect intents (resource requests, the narration
        |      job) together or not at all
        v
  5 NARRATE    a structured outcome line shows at once; a SpeechEnvelope from the text service
        |      is qualified before captions or voice and is appended, or skipped if late
        v
  6 PROJECT    one projection per audience (TV, each seat), never empty, status stamped
```

**C.0 Where `step` runs, and engine input (K2, W2).** APPLY calls the active machine's `step` with
the DECIDE outcome and its draws to compute the slot after-state, then commits (RES27). Timers,
resource results, machine-requested hand-offs and NPC, world and story proposals enter as engine
inputs (RES21): they skip ADMIT's seat checks and INTERPRET and pass CHECK, APPLY and PROJECT. DECIDE supplies rules-produced effects (RES18); `step` adds the
machine's own `TableEffect`s (an offer opened, a hand-off proposed, resource requests); APPLY commits
the union; and engine inputs skip DECIDE unless they need dice.

**C.1 Input (N20, N2).** An `InputEnvelope` (`interaction-engine.md`) carries the actor and
utterance ids. The redesign adds the `view_revision` the input was drawn from (the wire's existing
`observed_revision`) and uses the existing `OperationId` (`df-types`) as the operation id, which
`OperationContext` (`df-observe`) correlates with a trace. The seat comes from the authenticated
session, and phones send intents and never state. Player input uses one serial owner queue served
round-robin by seat, with at most three pending per seat; overflow is declined `table_busy` for
that seat only and logged. Timers and resource results use separate capped lanes whose overflow
is a stall and not a decline (RES22). A seat is limited to 3 inputs a second in a burst and 1
sustained, and to one free-text line in INTERPRET at a time (these limit rate, not money, and are
config); `rpc-api.md` sets no inbound rate numbers, so these stand (question 58).

**C.2 Plans, and what the phone offers (C7).** INTERPRET returns an `ActionPlan`. CHECK validates
each step against the actor's remaining action, bonus action, reaction, movement and free
interaction (R05), applies the longest valid prefix, and asks one question about the first step
that fails, as `interaction-engine.md` already allows. The viable list, the phone's choices and the
decline text all come from the one legal-choice call for that actor, so the phone greys out spent
options and cannot offer what the rules refuse. A plan outlives neither its turn nor its bound.

**C.3 Improvised actions and rulings (C6, E2, M4, M5).** "I swing from the chandelier and kick the
archer", "I tell the guard I am the inspector and hand him a letter I forged" and "I use fire bolt
to light the candle" are valid input. The route is fixed:

1. Map the line to the existing mechanics first: the affordances of the set pieces present, then
   the standard actions (Attack, Shove, Utilize, Jump, Improvised Weapon), then a spell's own
   documented uses (R11 has one fixture per spell entry). A chandelier that is not in the set is
   declined, never invented. A spell effect that is not in the catalog data is not offered.
2. A representable case resolves through the rules owner, with its DC and modifiers from the
   source. A multi-step line is a plan: the forged letter is making it (tool, proficiency and
   materials checked against the inventory) and presenting it, and the guard's reaction is a
   contested or Influence check against the stored result.
3. Where the source leaves a real choice open, DECIDE emits a `RulingRequest`. Reviewed automatic
   adjudication resolves it where the plan allows; otherwise the pending ruling shows
   `AwaitingAdjudicator` (the machine holds only `Waiting{pending_id}`, RES26), and the host's
   phone and the TV show a notice with the permitted
   alternatives (no secrets), and the player may pick a supported alternative or cancel the
   uncommitted part. No timeout spends a resource, rolls a die or invents a ruling.
4. The answer is stored as a `ScopedRuling` (`df-model`) with the normalised key and the question
   held beside it, and disclosed as a ruling, never as a standard rule (question 68 asks whether
   it binds later play automatically). A later input is matched against stored
   rulings by normalised key and retrieval, and a hit is shown as "Ruling (earlier): ...". The only
   settings a host can change are the R17-selected optional modules and a custom `RulesetId`
   (`generated-content.md`); a host cannot inject arbitrary rules.
5. A decline names the missing piece and what is possible ("the forgery needs a kit and ink; you
   have neither"). Requirements are `requires{tool, language, proficiency}` on the action, and the
   rules owner decides whether a missing one is a decline or a disadvantage.

**C.4 Reactions and windows (C3, M2, C4).** When the rules owner reports that someone may
interrupt (an attack roll a Shield could change, a creature leaving reach, a readied trigger, a
spell that can be countered, a legendary action), the Outcome carries a window (`TriggerWindow` in `rules-effect-model.md`, stored as a
`ResolutionWindow`) and APPLY runs in two phases. Phase one commits the `PendingResolution`
(continuation, ordered dice, spent resources, remaining choices) and opens the window. Each eligible
seat's phone gets its offer and nobody else sees the options. Answers come back as inputs addressed
to the pending id, and phase two re-validates against current state and resumes. Nesting is bounded
by the effect-model plan, and a bound hit becomes `needs_ruling`. A step reports
`Waiting{pending_id}`, and the machine contract gains a dice input (`Dice{draw_ids, values}`) so
`step` stays pure. While a window is open the plate tick keeps running and game time does not
advance (question 67). Which pause policy applies when an eligible seat is slow is a campaign policy and is
question 59.

**C.5 Ambiguity, agency and confirmation (C9, M16).** An ambiguous line always returns `Clarify`
(this replaces Q51's "take the likelier"), and in combat a free-text intent with more than one legal
target, or whose reading changes the economy spent, never executes on a guess. A reading that is
irreversible or harms an ally or a neutral (attacking, giving away an item, breaking a quest item)
needs a one-tap confirmation on that player's phone. Taps and offered choices skip INTERPRET and
confirmation. There is no undo: dice are never rerolled, and a wrong act is handled by the ruling
flow. Narration may not state what a player character does, says, feels or decides beyond the
committed outcome, and the qualification step rejects text that does.

**C.6 Declines (E22, C21).** A decline has a stable code, a plain reason, the missing piece if any, and
what would be possible. Decline codes: `not_viable`, `not_supported_yet`, `not_your_turn`, `not_in_combat`,
`no_combat_profile`, `missing_requirement`, `window_closed`, `economy_spent`, `resource_spent`,
`table_busy`, `rate_limited`, `intent_invalid`, `pending_open`, `pvp_off`, `consent_declined`,
`time_locked`, `not_knowable`, `stock_changed`. A clarification, a confirmation and a ruling are
not declines: they are `needs_clarification` and `pending` outcomes (RES16, RES26). A player who
asks "why" gets the rule text.

**C.7 When the text model fails (N5, N12).** A failed model call never produces a guess and never
uses a keyword table. INTERPRET runs on the fast lane (question 65), and its retry uses a shorter prompt and the
legal-choice list only. If that fails, the DM says "I could not read that. Pick one:" and shows the seat's
legal choices as buttons, which need no model, and keeps the player's draft. NARRATE falls back to
a structured readout of the committed outcome's typed fields ("Pip hits Dain Rusk for 7 slashing.
Dain is at 12 of 19."), a rendering of rules output and not an interpretation. Flavour that arrives
within `narration_late_secs` is appended after it and never replaces it. Output that is empty,
over-length, the wrong shape or the wrong language is a failure with cause `text_invalid`. Player
text, names and previously generated text are untrusted data in every prompt, delimited and marked
as carrying no commands; INTERPRET output is parsed against a closed schema, the intent must be
registered, every target must be an id from the viewer's legal choices, and no number is read from
model text. Phones cap free text at 280 characters, and generated persona fields are capped and
stored with `provenance=model_generated`.

**C.8 Questions and non-action input.** `Question`, `Meta`, `Social`, `PlanOnly` and `Joke` are
handled as in section K.1.

### D. The table: groups, time, seats, cast and items

**D.1 Scenes (E5, M20; Q5).** Table state holds `scenes[]`. Each scene has its own cast,
layout, active machine and `scene_epoch`. A seat belongs to one scene. The TV shows the **focus
scene**: the host can lock one, and otherwise the K.10 rule applies, with `focus_min_dwell_secs`, and
the TV cuts only on a beat boundary. A phone always shows its own seat's scene. The plate tick runs per
scene that has a viewer. There is one `world_time`. `Split` and `Regroup` are player intents that the pipeline routes. Because
`narrative-engine.md` states that the party moves together because the scene is shared, the draft
default for the first release is to decline `Split` with a reason, while the structure above keeps
the door open (question 60).

**D.2 Scene epoch (N6).** `scene_epoch` increases in the same commit as a scene change. Every plate
job, resource request and result carries `(scene_id, scene_epoch, snapshot_revision)` and the
`SessionRevision` of RES42. The playout buffer accepts a
plate only when both match, and a result that does not is stored as `late` and logged
`plate.rejected_epoch`. This is how "never show a plate from another scene" holds even when a
provider call cannot be cancelled.

**D.3 Table status and resume (E17).** `table.status` is `Active`, `Idle` or `Paused`. The plate
scheduler stops after `idle_minutes` without input (config) and while Paused, and the world clock
is frozen. Resume restores the active scenes, requests one fresh plate, and shows a recap built
from committed events as two projections, a public one and a private one per seat, under the
private, public and member recap rules of `campaign-cinematics.md` (RES29). Events:
`table.paused`, `table.resumed`.

**D.4 Seats and characters (E15, E16, M21, C21).** A seat has a `PresenceKind` (connected,
sleeping, disconnected or voluntary AFK, as in `remote-play.md`) and a membership of member or
left. A voluntary-AFK character is offstage or follows, by host setting, and never acts alone in
risk; AFK is a deliberate approved command, not an inactivity penalty. A player who joins enters
at a narrated beat, and in combat through the encounter owner's admission at the next round. A seat
maps to `characters[]` with one `active`. Death sends the seat to the creation machine for a
replacement (R01, R02), or to spectating (question 69); import goes only through `validate_build`, and private
import cannot mint rights (`rules-support.md`). A party wipe returns `EncounterOutcome=defeat` and
hands off to `AdventureEnd`, where `narrative-engine.md` allows an authored failure state.
Events: `seat.state_changed`, `character.status_changed`.

**D.5 Cast entries (M11, C19, M10, C22).** A cast entry has `kind` (party, npc, summon, mount, companion,
hireling), `controller` (a seat or the engine), `bound_to` (an `ActiveEffect` or a character),
`stat_ref`, `presented_as` and `visibility`. The last two are derived from active effects and the
rules owner (disguise, invisibility, a form from Wild Shape), never typed by a machine. Each
perception change writes a `WitnessRecord`. Locate expects only entries whose derived visibility is
`Seen`, and keeps an explicit expected-unlocated list. Plate references (Q21) are ranked: the
speaker, then the party, then bound entities, then the nearest threats in combat, then other people
by relevance, and the rest are described in the prompt only.

**D.6 Layout fields the rules owner reads (C2, E7, M9).** Each layout area and set piece carries the
typed fields the spatial query needs: `footprint_ft`, `elevation_ft`, `terrain[]` (difficult,
water, hazard), `light` (bright, dim, dark, magical dark), `cover`, `obscured`, `blocks_sight`,
`blocks_movement` and `occupancy`. The dress step fills them, authored or written through the text
service and then checked (Q13), and the engine checks them at start. Zone effects (Fog Cloud,
Darkness, Spike Growth, Web) are `ActiveEffect` areas that the engine writes into the layout.
The depiction of darkness or fog in the plate derives from these fields, never the reverse. The
rules owner's grid is five-foot cells with obstacles, cover and difficult terrain, so `elevation`,
terrain kinds, `light`, `obscured` and `blocks_sight` are additions to that schema that need a
rules-owner change (J.1). Events:
`spatial.query{kind, inputs_hash, result}`, `layout.changed{cause}`.

**D.7 Items and money (M12, M13, M14, E22).** Item and coin state is one rules-owned record in table
state (R12, `money-arithmetic-contract.md`). The shop machine holds the offer flow and the
inventory machine holds placement, and carrying a purchase across places is an acceptance case. A completed deal is one
effect keyed by an `AwardId` that moves item and coin together or neither. Item pictures are made
from the presented description only; true properties follow `SecretPolicy` until a rules-legal
identify step, and a found detail is a `KnowledgeGrant` to the finder. The "listed but not found"
fault exempts `hidden` items. Haggling is an Influence resolution with a price range from the shop
data (Q42's "free text with a rules check" becomes that flow); loot is an award keyed by `AwardId`
exactly once, and the text service only names and describes. A shop is open when its keeper is
present at `world_time` under the keeper's `df-world` schedule, and a closed shop declines with the
return time. Events: `item.moved{award_id}`, `purse.changed`, `item.revealed{to}`.

### E. What each rules family needs from the redesign

`rules-coverage.md` owns what each family must do. This table is only what the redesign must supply
so that a family is reachable, visible and observable. The suffix after each family id is the
critic finding it closes.

| Family | Machine home | Pipeline and projection | Events and CLI |
|---|---|---|---|
| R01 creation (M21) | creation: list options, submit, `validate_build` | Offers on the phone; import and replacement characters only through the same validation | `build.validated`, `build.rejected{reasons}`; `sheet build`, `sheet validate` |
| R02, R16 advancement (E14, M8) | advancement, entered by an `AwardId` effect | Private level-up on the seat's phone; the TV shows only the public result; `progression: xp or milestone` is a table setting (R16 policy) | `award.applied`, `award.duplicate`, `advance.offered`, `advance.chosen`; `advancement pending`, `character award` |
| R03 species, feats (none) | creation, advancement | As above | As above |
| R04 checks (E12, M17) | any machine, through a `CheckRequest` that opens a `PendingResolution` | Public `RollResultView` on the TV; secret rolls (Insight, Perception, Stealth against the unknown) go to one seat, identical for failure and for no information; passive checks run on scene entry with a private result; a retry of the same task is refused unless the rules owner allows it | `check.requested`, `check.resolved`, `roll{id, audience}`; `check show` |
| R05 initiative, economy, reactions (C3, C7, M2) | combat (section F.1) | The phone shows remaining action, bonus action, reaction, movement and free interaction from the encounter view; windows per C.4 | `window.opened`, `window.answered`, `window.closed`; `pending list`, `pending show`, `pending drive` |
| R06 movement, sight, cover, hiding (C1, C2, E7, C15) | explore, combat | RES1 and D.6; hidden creatures are left out of the public plate and revealed by an effect that queues a new plate; a hidden player's own phone shows its position as a private overlay; a threat the party knows of and cannot see is a marker, never the figure (question 66) | `spatial.query`, `stealth.revealed{by}`; `spatial query`, `combat anchors show` |
| R07 attacks, grapple, shove (C7) | combat | Combat intents from the legal-choice call | `plan.step{id, outcome}`; `combat choices <actor>` |
| R08 damage, death (C21) | combat, rest | Status chips (dying, stable, dead) to the seat; D.4 for replacement | `character.status_changed` |
| R09 conditions, durations (C8) | table state `active_effects`, owned by `df-rules` | Effect chips and a concentration marker from permitted views; expiry at `TurnBoundary{creature, start or end}` in combat and at `df-world` time outside it | `effect.applied`, `effect.expired{origin, boundary}`; `effects list <creature>` |
| R10 spellcasting, concentration (C20, M4, C8) | explore, combat | Spell offers from the legal-choice call; the phone targets by tapping a creature hotspot or a point on the plate, the tap is a proposal that the engine converts to cells with the same declared function, shows for confirmation and lists the creatures caught, and the rules owner validates; costly components and slots are checked by the rules owner | `spell.target_selected{op, origin, caught[]}` |
| R11 features, summons, forms (M11) | cast entry kind and `bound_to` | Form effects replace stat blocks and revert by rule; summons use a light persona and no hooks | `entity.joined`, `entity.ended`, `form.applied`, `form.reverted` |
| R12 equipment (M12, M13) | shop (offer flow), inventory (placement) | D.7 | `item.moved`, `purse.changed` |
| R13 rests (E8, C18, M8) | **rest** (added to the first machines) | Calls the rules owner's rest and `df-world` time; handles `RestInterrupted` and incomplete results; applies rules-produced recovery effects only; watches are per-seat assignments the encounter proposals read; the short rest now inside combat aftermath moves here | `rest.started`, `rest.interrupted`, `rest.completed`; `rest drive` |
| R14 exploration, social, hazards, travel (E6, E9) | explore, travel, dialogue | F.3, F.4 | `travel.leg`, `social.reaction`, `obligation.state` |
| R15 monsters (C11, C12) | combat | `stat_ref`, `combat_binding`, enemy turns through `df-combat` with a perception-limited view | `binding.missing{character}`, `enemy.turn{creature, view_hash, choice, fallback}` |
| R17 DM procedures (E19, M5) | ruling flow; downtime and crafting registered only when R17 marks them required or configured | C.3; an unsupported craft is declined | `ruling.requested`, `ruling.resolved`, `ruling.reused`; `rulings list`, `rulings show` |

### F. Machines added or specified

The machines are those of RES28. Their homes: advancement (row R02 and R16, J.3, K.20); rest (row
R13, K.20); shop and inventory (D.7); item inspector (the Working plan item section, questions 36
to 41); adventure end (K.21 and D.4; specified in release 1 and registered when K.21 is built);
combat, dialogue and travel (F.1 to F.3). Downtime, puzzle and chase are registered later. Each
follows the machine contract and the new-machine checklist.

**F.1 Combat (C1 to C19, C24).**

- **Entry.** `combat.enter` takes an admission from the encounter owner (`scene-combat-binding.md`):
  participants, sides, objective and source, with initiative drawn at admission by the rules owner
  and the layout converted once into the rules state (RES1). A person with no `combat_binding` is
  declined `no_combat_profile`.
- **States.** `Round(n)`, then within it `Turn(creature)` in one of `AwaitingInput`, `Resolving`,
  `AwaitingResolution(pending_id)` (shown while the machine reports `Waiting{pending_id}`) or
  `Held(seat, reason)`. Initiative ties, surprise, delay, ready
  and lair or legendary entries are the rules owner's (R05); the machine only shows the order and
  accepts its turn boundaries.
- **Inputs.** `{seat, actor_id, utterance_id}`. Only the active creature's controller spends its
  economy; others answer windows, speak (table talk, not an action), or queue a plan-only note. A
  non-turn action is declined `not_your_turn` with the permitted choices.
- **Enemy turns.** The machine asks `df-combat` for the tactic with a perception-limited view of
  the table, so a hidden rogue is not known to the goblin (R15). The one versioned model choice at
  the start of an enemy turn that `interaction-engine.md` allows (a legal flee action or a surrender
  offer) is a resource request in DECIDE with its legacy fallback, and a `flee` or `surrender`
  result is an input to the machine's `EncounterOutcome`, not a machine decision.
- **Beats.** The engine resolves and commits a `Beat{events[]}` at once. Clients show the roll and
  result views at a minimum dwell (`beat_min_dwell_ms`, config, so lines stay readable at human
  pace), narration attaches later or is skipped, and the next turn starts after the dwell and not
  after narration.
- **Overlays.** HP, the turn marker, condition chips, badges and damage numbers are drawn by clients
  from state, anchored to the latest screen anchors, and never baked into plates. Mask-to-creature
  assignment is a minimum-cost match against the previous anchors, with the badge shown. A creature
  that goes down dims at once over a stale plate. Events: `display.identity_match{plate,
  assignments, unmatched}`, `beat.shown{dwell_ms}`, `beat.narration_late`.
- **Held turns.** The combat projection shows each seat's status (connected, sleeping, disconnected,
  AFK). A turn or window whose controller is unavailable is held with a notice and the permitted
  host actions (pause, takeover per campaign policy). Waiting rolls nothing and spends nothing.
- **Exit.** `EncounterOutcome{reason: victory, defeat, fled, surrendered or parley, objective}`.
  Rewards, XP and loot are `AwardId` effects, never machine writes, and loot becomes set pieces with
  hotspots (D21). A required reward is never withheld because a provider failed. Events:
  `combat.admitted`, `combat.ended{reason}`, `award.applied{award_id}`.
- **Conformance.** The combat suite runs the rules owner's `RuleFixture`s end to end (action,
  choices, draws, expected windows and resources) and checks the replay state hash, in addition to
  the generic machine checklist.

**F.2 Dialogue (E9).** The private slot holds a reference to the `ConversationState` of
`interaction-engine.md`. Step 5 builds a `SpeechEnvelope` through `qualify_expression` and
per-listener projection. Faction reaction comes from witnessed events and never from a score. The
character record holds `npc_ref` and `dossier_ref` and copies nothing.

**F.3 Travel (E6).** The machine reads route, weather and resource proposals from `df-world` and
hazard proposals from `df-encounter`. A leg is narrated with a reused establishing frame labelled by
place name, never a stale plate. The destination is created in the background (`place_creation`
purpose through the broker), and the table waits at most `scene_build_budget_secs` if it is not ready on arrival (reusing the
SCENE-01 owners `place::plan`, `place::overlay`, `place_creation::judge_exit` and
`place_creation::create`), with the
`place card` fallback (name, one line, cast list) if the build exceeds `scene_build_budget_secs`.
A full scene is built only for an encounter or a stop. Today's seven-minute travel hang is a
failure of this rule. Events: `travel.leg`, `travel.arrival_wait{ms}`.

**F.4 Story, quests and clues (E10).** After APPLY, committed events go to `NarrativeDirector::
evaluate`; its proposals return as machine inputs and never as direct edits. Hooks are
`CharacterHook` references (question 62 asks whether the Q11 hooks are the same objects). A journal
projection lists the active, dormant and failed beats a seat may know. A lost critical NPC or item
selects an authored alternative or a failed state, never a silent loss. Events:
`story.beat_state{why}`; CLI `story beats`, `story why <beat>`.

**F.5 Consequences (E11).** The dress step and every plate job take `df-world` state deltas
(destroyed, opened, occupied) as input, so a burned warehouse stays burned. Created people get a
dossier, shared with F.2. This closes two gaps that `narrative-engine.md` records for SCENE-01
(verified in that plan): items are scoped to the table's party and not
to the place, and every created person has a dossier.
Rumour and recognition reach NPCs only through witness paths. Events: `dress.world_delta_applied`.

**F.6 Puzzle and downtime (E21, E19).** A puzzle has a deterministic key or predicate; the text
service only maps a phrase to candidate answers, the rules decide, and hints are a bounded ladder
(each costing time or a check). Downtime and crafting register only when R17 marks them required
or configured, with one independent `Activity` slot per seat, time through RES5 and results through
the rules owner. Events: `puzzle.attempt`, `puzzle.hint`, `downtime.activity`.

**F.7 Safety, private channels and PvP (M18, M19, E20).** The lobby runs a session-zero step that
stores `lines[]` and `veils[]` in table config. The broker injects them into every text and image
request and rejects a result that conflicts, logged `safety.violation`; a pause control on each
phone interrupts through the pipeline (`safety.pause`, anonymous by default). Whether the chips of
`character-backstory-creation.md` (O10) are required or optional is question 63. A
`PrivateMessage{from, to}` is routed by the projector to the named seats only, kept out of the TV and
out of NPC context, with its content logged by id. A secret role is a seat-scoped `CharacterHook`
secret. `pvp` is `off` or `consent`; with consent, the target's phone is asked before an attack is
legal. Events: `message.private{from, to}`, `pvp.consent`.

### G. Worked traces

These run the pipeline on inputs the critics used. They are the first fixtures for the golden
sessions in section H.

1. **"I swing from the chandelier and kick the archer."** ADMIT stamps the operation. INTERPRET
   returns an `ActionPlan` of one step (an unarmed strike made with a swing). CHECK looks at the set:
   if no chandelier is present the line is declined `not_viable` ("There is no chandelier here. You
   could run at him, climb the crates, or shoot"), with those alternatives from the legal-choice
   call. If the chandelier is present with a "swing" affordance, the legal choices include Unarmed
   Strike, and the source does not say that swinging grants an advantage, so DECIDE emits a
   `RulingRequest` with the suggested answer. Under a reviewed automatic policy it resolves; under
   host adjudication the pending ruling shows `AwaitingAdjudicator` on the host phone and the TV (the
   combat machine reports `Waiting{pending_id}`), and
   the stored ruling is reused next time. The strike then resolves with the table's explicit dice,
   and the commit, the beat and the narration follow.
2. **"I tell the guard I am the inspector and hand him a letter I forged."** The plan has two
   steps. Step one needs a forgery kit or proficiency and materials; without them the line is
   declined `missing_requirement` ("the forgery needs a kit and ink; you have neither"). With them,
   making the letter takes time (a `RequestTimeAdvance`) and its quality is stored as a fact on the
   item. Step two is an Influence or deception resolution against the guard's Insight, read from the
   stored result, and the guard's stance is decided by the rules and voiced as a `SpeechEnvelope`.
   A promise the guard makes becomes an `Obligation` only through an approved commitment action.
3. **"I use fire bolt to light the candle."** `Magic` carries the spell and the purpose. The spell's
   documented uses (R11 fixture) cover igniting an unattended flammable object, so the rules owner
   resolves it with no slot and no check. A use the spell does not document goes to the ruling flow.
4. **"I attack" next to the innkeeper and a goblin.** More than one legal target and a harmful
   reading: INTERPRET returns `Clarify`, the phone shows the two targets as buttons, and choosing the
   innkeeper asks for a one-tap confirmation. This replaces the observed misreading of "I draw my
   greatsword and attack Dain Rusk" as an Intimidation check.
5. **A goblin attacks the wizard, and the rogue has a reaction.** CHECK and DECIDE roll the attack
   with the table's dice; the rules owner reports that Uncanny Dodge applies. APPLY phase one commits
   the pending attack and the window. The rogue's phone gets the offer, the wizard's phone shows
   "waiting for Pip", and the TV shows a plain waiting state. If the rogue's phone drops, the turn is
   `Held`, nothing is spent, and the reconnect reacquires the window by `pending_id`.

### H. Quality contract: robust, resilient, fast, logged, performant, CLI-capable

The existing plans already set the project-level rules (`runtime-reliability.md`,
`observability.md`, `optimization.md`, `storage-architecture.md`). This section states what each
redesign component must do to meet them, with the invariant, the event or the test that shows it.
Budgets are candidates to validate on the first playable slice and are not pinned, as
`optimization.md` requires.

**H.1 One commit per input (N1).** APPLY is one `df-session` fenced decision commit per input. It
holds the decision, the dice counters, the resources, the `TableEffect`s, the machine's slot
after-state and the committed effect intents (resource requests and the narration job). The broker
reads only committed intents, and no transaction spans a model or media call. A turn is `applied`
once committed and `narrated` once its narration row exists, and NARRATE never changes game state. A
narration row may cover a batch of turns (K.6): `turn.narrated{turn_ids}`, one row, one status per turn.
Recovery redelivers committed intents whose outcome row is missing, keyed by intent id, and never
rerolls, re-asks a model to decide, or repeats a dispatch of unknown outcome. Events:
`turn.applied{turn_id, seq, revision}`, `turn.narrated{turn_id, ms}`, `recovery.redelivered{count}`.
Test: kill the process between the two events against a populated data directory and assert one coin
movement and one narration.

**H.2 Operation identity, ordering and stale views (N2).** Inputs carry the operation id and the
`view_revision` (section C.1). A duplicate returns the stored result. An input drawn from a
projection older than the seat's last change to its legal choices is re-checked against current
state and declined with the reason if it is no longer legal. Machines declare
`accepts(seat, input, state)`, so a combat input out of turn is declined and never queued. The owner
serves three capped lanes (player input round-robin by seat; engine inputs, meaning proposals and
hand-offs; timers and resource results), engine lanes before the player lane, and the serve order is
the single total order recorded as `turn.seq`. Lane caps are measured in slice 0. Test: send one operation id 50 times concurrently and assert one effect.

**H.3 Dice (N3).** Dice come from the explicit draws `df-session` supplies, used only in DECIDE and
passed into `step` as input fields, so `step` is a pure function of (state, input, clock). Each draw
is logged `dice.drawn{op, draw_ids, values}`. The CLI has `dice seed`, `dice force` and
`ops show <id>`, and a forced draw is marked `driven=true`. This is an operator tool in local and
test builds and is never reachable in hosted play (`rules-coverage.md`: no operator force-dice).
Replay is a diagnostic and test tool, supported where reducers and migrations are compatible, and it
compares the state hash.

**H.4 What the table sees when a dependency fails (N4).**

| Failure | What happens | What the table sees | Event |
|---|---|---|---|
| Text slow, empty or garbage | C.7: one retry, then buttons or the structured readout | "I could not read that. Pick one" or the structured line | `text.failed{cause}` |
| Plate later than `image_stale_limit_secs` (default 90) | Hold the last plate of the same scene | "Picture catching up" mark (RES10) | `plate.late{lateness_s}` |
| New scene background fails | The authored or last background for the place, else a place card | Playable scene without art | `scene.degraded{stage}` |
| SAM fails | One retry, then one bounded regeneration (Q20), then the rules layout with the character `unlocated` | Hotspots as a list strip | `locate.failed` |
| Image admission refused by the hosted entitlement or spend authority (hosted builds only; local builds never refuse for spend) | The tick stops requesting | A paused-art status, play continues | `plate.admission_refused` |
| Commit to Postgres fails | The input is refused and nothing is mutated in memory; the table goes read-only until a health probe passes | "Could not save, try again" | `persist.failed{cause}` |
| Phone disconnects | The seat is kept and pending operations finish; reconnect resyncs the projection and the last 20 narration lines | Seat shown as disconnected to the table | `seat.state_changed` |
| TV reloads | Fetch the current projection and the plate by `view_revision` | The current state, no replay | `display.resynced` |
| Server restarts mid-pipeline | H.1 recovery and H.9 startup order | A short "server restarting" frame, then resume | `recovery.redelivered` |

Whether a table may play a whole session with no images is question 64.

**H.5 Which steps may call the model, and the budgets (N7).** Invariant, enforced by an
architectural test and the boundary check: the model is called only in INTERPRET, NARRATE, Describe,
Dress and staging text, plus the named exceptions, each behind the broker with a deadline and a
fallback: the enemy flee or surrender choice (F.1), the ruling suggestion (RES15), the ruling
retrieval (C.3), the puzzle phrase mapping (F.6) and those of RES39. DECIDE may emit the broker
requests of these exceptions; the owner never waits on them (RES22) and their results re-enter as
inputs. CHECK, DECIDE, APPLY, the projection and the scheduler tick make no network
call except the Postgres commit, and a tap or offered choice never enters INTERPRET. Every step
emits `step.ms{step, machine}`. Candidate starting budgets, to be validated and not pinned: tap
acknowledgement under 100 ms; the rules-only path under 400 ms at p95; INTERPRET under 4 s at p95;
the structured line immediately after APPLY; qualified narration under 8 s at p95; projection
compute under 5 ms at p95. One observed fact bears on this: the text calls behind today's play take
15 to 45 seconds through Codex (`development/runbook-codex-live-test.md`, trap 30, which measures the observation text call), so INTERPRET needs a fast lane, and which text
lane serves it is question 65.

**H.6 Narration is validated before it is published (N8).** The default is the one in
`interaction-engine.md`: the whole bounded response is buffered and qualified before captions or
voice, and stable-clause streaming stays off until a model and locale are qualified for it. Only a
complete, successful narration is stored for reuse, and a cancelled or failed stream never
overwrites a valid entry. A leak check by fact id (the model returns which secret ids a clause
revealed, with no keyword matching) runs inside the qualification step; a revealed secret is
redacted or re-requested once and logged `leak.blocked{fact_id}`.

**H.7 Queues and backpressure (N9).** Broker queues are per class, in priority order
`player_blocking` (interpret, narrate), `scene_build`, `plate`, `prefetch` (item views,
portraits), with at least one text slot reserved for `player_blocking`. Bounds per table: plate 4
(the oldest is dropped, `plate.dropped_stale`), prefetch 8. A class that cannot drop (scene build)
surfaces as a status and never as a silent loss. Every queue reports depth, oldest age and drops in
`/health` and `broker queues`.

**H.8 Request identity, retries and cost (N19, N24).** A request key is the hash of purpose, canonical
inputs, model and version, and `scene_epoch`. Identical in-flight keys coalesce. A retry after a
timeout is allowed only if the dispatch is known not to have happened, otherwise the unknown-dispatch
rule of the commerce plan applies and unknown spend is never released. A late result for a
superseded request is stored as `late` and is not delivered as an input. Retries are bounded (two,
with backoff and jitter) and never outlive the originating turn's deadline. The cost view (D18) is a
display of recorded usage and never an authority, and for unpriced stages it shows plates per hour,
mean seconds per plate and calls per hour instead of "n/a" (question 29 stays open).

**H.9 Leases, lifetimes and shutdown (N6, N23).** Every broker job row has `lease_owner` and
`lease_expires_at` (60 s, heartbeat-renewed). A reaper requeues an expired lease (three attempts,
then `job.failed{cause}`), subject to the dispatch-unknown rule. Every spawned task has an owner
(table or scene scope) and a cancellation token, and `live_tasks{owner}` returns to zero after
`table.close`. Startup order: validate the registry and content, recover (H.1), re-lease, then
accept connections, with `/health` reporting `starting` until done. Shutdown: refuse inputs, finish
in-flight commits, cancel scene scopes, and send phones a "server restarting" frame. A suspended
machine's timers pause with logical time, and its pending requests either finish and are held or are
cancelled with `request.cancelled{cause=suspend}`, which the machine contract declares. Each
per-table structure has a declared cap and an idle table is evicted to Postgres after 30 minutes.

**H.10 Correlation (N10).** Each input creates one trace. Each pipeline step is a span with
`turn_id`, `step` and `revision`. The trace and operation ids are stored on every committed effect
intent and passed to both services. The broker audit row stores the provider request id, usage
units, price or `unpriced`, `cache_hit` and `attempt`. A plate tick has its own trace, linked to the
`turn_id` of the snapshot it used. `trace <id>` prints the tree across tables, broker and
projections. Test: the integrated-flow correlation check of `observability.md`, from input to phone.

**H.11 Hidden information and injection (N11, N12).** Prompts are built per audience (RES6), with a
`dm_context` block marked never-to-quote for the narration of a secret the NPC must know. Logs hold
secrets by id and player free text as a hash and length, with full text only in the local dev spool.
The paired-state noninterference test of `runtime-directors.md` is part of the machine conformance
suite, and a sentinel-secret test serialises every projection for every seat and the TV and fails if
a sentinel appears. Adversarial fixtures for injection (a player line, a generated persona) run in CI
against a scripted model and nightly against the real one.

**H.12 Versioning (N17).** Every persisted record type (table, scene, character, turn, effect
intent, machine slot) carries `schema_version`. A record the engine cannot migrate fails the table
load with `load.refused{type, version, expected}`, and the host sees "saved with an older engine"
with `table export`. D7 allows delete and recreate until the stabilisation gate, and the refusal
path is built now anyway because it is the same code. `table export` and `table import` round-trip
a bundle (state, turn log, seed), which is also the fixture format.

**H.13 Time is injected (N18).** All time comes from an injected `Clock`. The scheduler, leases,
deadlines and the playout buffer take it, tests and the CLI use a manual clock, and the plate
scheduler computes each tick from the previous nominal tick on a monotonic clock, with missed ticks
coalescing into one.

**H.14 Events (N21).** Class A events (turns, transitions, rolls, broker requests and outcomes,
failures, rejections, stalls) are always on and never sampled. Class B (tick detail, per-seat
projection detail) is a verbose level. Projection events log the revision and a changed-field list,
not whole views. The cap is 40 class-A events per input, and the expected rate at six seats with
one input per seat every ten seconds is stated and measured on the first slice.

**H.15 Projection status (N16).** Every projection carries a `revision` and a `status` (`ok`,
`thinking`, `art_delayed`, `degraded`, `read_only`, `reconnecting`) and a plain waiting reason for
the seat. A status other than `ok` for more than 8 seconds shows a banner on the host phone and the
TV corner, except `art_delayed`, which follows K.3 (shown only when the stale plate disagrees with
the table, and otherwise logged and not shown). Narration is never empty: a blank model output is replaced by the structured readout.
Projection computation does no I/O. Phones get phone-sized plate variants, hotspots also appear as a
text list with accessible names, targets are at least 44 px, reduced motion is honoured, captions are
text, and a `text-only` seat mode renders without images (N22).

**H.16 CLI parity (N13).** Each machine and surface registers, where meaningful: `dump --json`,
`force <machine> <state>`, `input <seat> <text or tap>`, `viable <seat>`, `dice seed or force`,
`clock now, advance or freeze`, `replay <log> [--until seq] [--diff]`, `fixture load <bundle>`,
`sim run --seats N --script <file> --broker scripted or real`, `soak`, `fuzz --seed`,
`chaos <dependency> <fault>`, `health`, `cost`, `trace <id>`, `pending list`, `rulings list` and
`projection show <seat or tv>`. All go through the normal entry points. Fault, fixture and force
commands need operator permission and are disabled in hosted builds, and who may read hidden
content through them is question 70. A conformance test fails any
registered machine missing `force`, `input` or `dump`. A fixture is a table bundle plus a recorded
broker script keyed by request key. Question 34 (read freely, drive only for scripted tests) now
also decides whether `sim` and `soak` on a development table count as playtests.

**H.17 Tests (N14, C24).** Property tests over random input sequences with a scripted broker assert:
exactly one active group machine per scene (the lobby is table-scope before the first scene) and
at most one machine per seat lane; the projection is non-empty and has a status for each
seat and the TV after every step; every input ends in one of the four outcomes; no effect names an
id absent from the scene; coin and item totals are conserved except through named awards; a replay
reproduces the state hash. Golden sessions in `tests/golden/` (seed, scripted broker, inputs, the
section G traces; G.3 and the spell parts of G.1 are fixtures pending R10) diff the turn log, snapshot and per-seat projections. Chaos tests inject each row
of H.4. A load test runs six tables of six seats on a scripted broker and checks that
`live_tasks` returns to zero and memory stays flat. The real-model INTERPRET accuracy run is nightly,
on a labelled line set that includes the three known-bad lines of the observations. Persistence
tests use the PostgreSQL requirement of `storage-architecture.md`, one scratch instance or schema per
run, and include restart against existing data.

**H.18 Progress and preflight (N15).** Per input the checkpoints are `input.received`,
`turn.applied`, `projection.delivered{seat}` and `client.presented`. A per-table watchdog raises
`stall{kind}` when an input shows no progress for 20 seconds, the oldest job passes twice its
deadline, a plate is later than `image_stale_limit_secs`, or a worker heartbeat is missing.
`preflight` runs a scripted two-seat table against the real services at the cheapest purposes and
asserts a non-empty projection for the TV and both seats, and a canary headless table runs against
any deployed build on an interval. This is aimed at the failures that cost hours: empty screens and
stalled image workers.

### I. Ledger: where each finding went

| Finding | Where it is handled |
|---|---|
| C1, C2 | RES1, D.6, row R06 |
| C3, C4 | C.4, H.3, row R05 |
| C5, E20 | RES12 |
| C6, E2, M4, M5 | RES15, RES16, C.3, G.1 to G.3 |
| C7 | C.2, row R07 |
| C8 | RES5, row R09 and R10 |
| C9, M16 | C.5, G.4 |
| C10, C11, C12 | F.1, RES14 |
| C13 | RES2 |
| C14 | RES10, RES13, F.1 overlays |
| C15, E1, M3 | RES6, row R06 |
| C16 | F.1 beats |
| C17, C20 | F.1 held turns, row R10 |
| C18, E8, M8 | row R13, rest machine |
| C19, M11, M10 | D.5, row R11 |
| C21, E15, M21, E16 | D.4 (E15 tagged there) |
| C22 | D.5 plate references |
| C23 | RES8 |
| C24 | F.1 conformance, H.17 |
| E3 | RES16, C.4 |
| E4 | RES5 |
| E5, M20 | D.1, RES2 |
| E6 | F.3, H.4 |
| E7, M9 | D.6, RES1 |
| E9 | RES7, F.2 |
| E10 | F.4 |
| E11 | F.5 |
| E12, M17 | row R04 |
| E13 | RES3 |
| E14 | row R02 and R16 |
| E17 | D.3 |
| E18, M1 | RES18, section A |
| E19, E21 | F.6 |
| E22, M12, M13, M14 | D.7 |
| M2 | C.4 |
| M6 | RES14 |
| M7 | RES11, RES17 |
| M15 | RES11 |
| M18, M19 | F.7 |
| N1, N2, N3 | H.1, H.2, H.3 |
| N4 | H.4 |
| N5, N12 | C.7, H.11 |
| N6, N23 | H.9, D.2 |
| N7 | H.5 |
| N8 | H.6 |
| N9 | H.7 |
| N10 | H.10 |
| N11 | H.11, RES6 |
| N13 | H.16 |
| N14 | H.17 |
| N15 | H.18 |
| N16, N22 | H.15, RES10 |
| N17 | H.12, RES17 |
| N18 | H.13 |
| N19, N24 | H.8 |
| N20 | C.1 |
| N21 | H.14 |

### I.2 Ledger: round 2 findings

| Finding | Where it is handled |
|---|---|
| W1 | RES20 |
| W2 | RES21 |
| W3, W25 | RES22 |
| W4, W29 | RES23 |
| W5, W9 | RES24 |
| W6, W7 | RES25 |
| W8, W10, W11, W16, W17, W21 to W24, W26, W28 | J.3 |
| W12 | K.1 |
| W13, W18 | RES28 |
| W14, W15, W20, W30 | RES29 |
| W19 | RES27 |
| W27 | C.6 |
| K1, K2, K30 | RES27 |
| K3, K8 | RES26, RES16 |
| K4, K5, K11, K13, K14, K23, K24, K25 | J.1 |
| K6 | RES28 |
| K7 | RES2, RES20 |
| K9 | J.1 (broker) |
| K10 | RES1 |
| K12, K16, K17, K18, K19, K20, K22, K28, K37 | corrected in place (the Precedence paragraph, the Working plan notes, the checklist, the open-questions note, the section A sentence, the status line) |
| K15 | RES3 |
| K21 | corrected in place (L.1, L.2, section A) |
| K26 | section A rows |
| K27 | RES34 |
| K29 | J.4 |
| K31 | H.4 (SAM row) |
| K32 | H.5 |
| K33 | D.3, F.5 |
| K34 | F.3 |
| K35 | J.3 |
| K36 | H.8 |
| K38 | D.3, H.9, J.4 |
| F1, F11, F12 | RES32 |
| F2, F13 | RES30 |
| F3 | RES31 |
| F4, F6 | RES22 |
| F5, F15, F18, F19 | RES33 |
| F7, F9, F10, F14 | J.1 |
| F8 | K.3 |
| F16 | C.1 |
| F17 | RES26 |
| F20 | H.13 |
| F21, F22, F23 | L.1, L.2 |
| X1 to X27 | K.1 to K.22 |

### J. Round 2 amendments

Round 2 used four new critics: scenario walkthroughs (W), consistency and integration (K), feasibility
against the code (F) and the table experience (X). The consistency critic checked the Analysis
against the code and the plans and found that some of round 1's statements of fact were wrong; those
are corrected in place (section A, RES1, RES3, RES10, C.1 to C.7, D.1 to D.7, H.4, H.5) and listed
here so the change is visible. Section K is the table-experience contract from the X findings,
and section L is the comparison of the current and the next implementation that the owner asked for.

**J.1 Corrections to round 1.**

- **Code facts (K4).** `PendingResolution`, `ResolutionWindow`, `PendingInput {Choice, Roll,
  Reaction, Ruling}`, `ScopedRuling`, `ActiveEffect`, `KnowledgeGrant`, `WitnessRecord`, `NpcState`
  and `CharacterHook` are defined in `df-model/src/checkpoint.rs`, and `IntentDisposition` in
  `df-model/src/intent.rs`. `df-engine` stages them (`pending_resumption.rs`). `OperationContext`
  (`df-observe`) is only a trace parent and build; the operation id is `OperationId` in `df-types`.
  `RulesetId` exists only as a contract fixture in `df-types/contracts/boundary.rs`, so putting it in
  table state means promoting it. The reach function is `Battle::reachable_cells`
  (`df-rules/src/tactical/state.rs`). `legal_choices`, `SpatialQuery` and `TriggerWindow` have no
  definition in the code; the nearest existing type for the last is `ResolutionWindow`.
- **Five-foot cells (K5).** The rules owner's tactical state is a grid of five-foot cells with
  obstacles, cover and difficult terrain (`tactical-combat.md`, Scene-v2), not a free-feet model.
  The extra layout fields of D.6 (elevation, terrain kinds, light, obscured, blocks sight) are
  additions to that schema and need a rules-owner change.
- **One interface, three names (K3, K8, F14).** A window is a `PendingResolution` with a
  `ResolutionWindow`. The existing `ReactionWindow` and the `ReactionTimer` that declines an offer
  when its window runs out are the first implementation and are migrated, not duplicated.
  `TriggerWindow` is dropped as a name.
- **The broker already exists in part (K9, F9, F10).** The resource broker of D6 is the `Hub` in
  `df-tools/src/gameplay/resources.rs` (typed specs, stable keys, single flight, failure memory, a
  spend ledger that never blocks) extended with text and image kinds as facades over `df-ai` and
  `df-imagery`, queue classes, leases and epochs. It is not a second service. Its header says
  generation is on demand and nothing is made ahead of time, so the `prefetch` class needs that rule
  relaxed (question 73). Durable intents today are single-slot (`run_committed_intents` finds the
  first pending one), and H.1 generalises them with `EffectRegistrationInspector` and `SentUnknown`.
- **Spend refusal is hosted only (K13).** H.4's refused admission applies to hosted builds under the
  entitlement and spend authority of `commerce-service.md`. Local and development builds never
  refuse for spend, and the ledger never blocks.
- **The late-picture mark (K11).** The threshold of RES10 is `image_stale_limit_secs` (default 90),
  to be set from the measured latency of the lane in use. A Codex plate takes 90 to 240 seconds, so a
  fixed 60 would show the mark during normal play. The experience rule that the mark fires only
  when the stale picture disagrees with the table (section K) keeps it from being permanent.
- **Model use (K14).** H.5's list of model-calling steps has four named exceptions, each behind the
  broker with a deadline and a fallback: the enemy flee or surrender choice (F.1), the ruling
  suggestion (RES15), the ruling retrieval (C.3) and the puzzle phrase mapping (F.6).
- **Suspended machines (K15).** A suspended machine's timers pause with logical time (H.9). Resource
  results for it are held and delivered on resume, or stored as `late` if the scene epoch moved.
- **Locate (K25, F7).** SAM 3.1 takes one noun phrase per request, returns instances and not
  identities, and is sensitive to wording (`sam-3-1-research.md`). Locate is one request per
  prompt (people, plus one per listed item), identity is by exclusive assignment (heroes by slot,
  cast by described-person prompts) and never from SAM, and the cost per plate is the real request
  count times the unit price. Screen anchors are immutable attributes of a plate id and travel with
  it through the playout buffer, so overlays and hotspots always use the anchors of the plate on
  screen. Locate results enter the table as job-result inputs, never from a view.
- **Presence and rulings (K23, K24).** Seat presence is the existing `PresenceKind` (connected,
  sleeping, disconnected, voluntary AFK; `remote-play.md`) plus membership (member or left); "Away"
  in D.4 means voluntary AFK. A stored ruling is a `ScopedRuling` plus a normalised key and the
  question held beside it.

**J.2 Resolutions added by round 2.**

- **RES20. Machine scope, group or seat (W1; supersedes part of RES2).** A machine declares
  `scope: group | seat`. Group machines (explore, dialogue, travel, rest, combat, shop) are the one
  active machine of a scene. Seat machines (creation, advancement, a personal item inspector, the
  sheet and inventory views) run in that seat's lane beside it, read and write only that seat's
  character record and their own private slot, and never change the group machine or what other
  seats are shown. An item inspector takes over the TV only when its player taps "show the table",
  and Q36's default stands for that case only. A seat lane is declined with `not_in_combat` only
  where a rule needs it.
- **RES21. Engine-originated input (W2).** `EngineProposal{origin: npc | world | story | encounter |
  timer, proposer_ref, op_id}` enters the same owner queue as player input, with an operation id.
  It skips INTERPRET, runs CHECK, then DECIDE only if it needs dice, then APPLY against current state, and opens a window or a
  private check for the affected seats. An accepted proposal is applied before any player input
  queued after it, and a player input already past INTERPRET is re-checked against the new state.
  A refused proposal is logged `proposal.refused{cause}`, and one that changes the active machine
  goes through the same hand-off validation. The drunk's swing, the alarm, the pickpocket and the
  ambush are engine proposals. Timers, resource results and machine-requested hand-offs also enter
  here: they skip ADMIT's seat checks and INTERPRET, and pass CHECK, APPLY and PROJECT.
- **RES22. INTERPRET runs off the owner (W3, F4, F6).** INTERPRET is a resource request keyed by
  `(seat, utterance_id, state_revision)` that runs beside the owner queue, and its result re-enters
  the queue as an input carrying the revision it was read against. CHECK always re-validates every
  step and target against current legal choices; a result older than the seat's last change is
  re-checked and never trusted. If the machine changed meanwhile, the seat gets `declined{window_closed
  or not_your_turn}` with the current alternatives and keeps its draft. No code on the owner thread
  awaits a model or media call; the pattern is judge off the owner, commit on the owner with a basis
  check, as `submit_line` in `dialogue.rs` already does. The bound of 3 (J.4) counts a seat's inputs admitted and not yet committed, including one in
  INTERPRET, and applies to player input only; the owner serves three capped lanes (H.2), and a
  lane overflow other than the player lane is a stall, not a decline. The queue is served round-robin by seat.
- **RES23. Agency tags replace RES12's rule on other players' characters (W4, W29).** The rules
  owner tags each outcome with `agency_impact: none | beneficial | cosmetic | resource | control |
  harm`. `none` and `beneficial` apply at once with a notice on the target's phone. `cosmetic`
  applies with a notice afterwards. `resource` (theft) and `harm` follow the table's `pvp` setting.
  `control` (Command, carry, forced movement) always asks the target. A target who is unconscious or
  held cannot be asked, so a beneficial effect applies and `control` is declined. A secret act shows
  the room only a generic "the DM is deciding something", and a host who is a party to it sees the
  alternatives and not the target; if no other adjudicator exists the act is declined
  `not_supported_yet`.
- **RES24. Perception and the shared screen (W5, W9, E1).** Public knowledge is what has been granted
  to the party or perceived by every present party member. A fact perceived by one seat is
  `seat_private` and appears only in that seat's private list and overlay, never in the TV plate, TV
  hotspots or plate prompts. It becomes public when the finder taps "tell the party" or says it in
  an action or social line, which writes a party-scope grant in the same commit. Plate light follows
  the lowest-vision present member unless the host selects shared vision (question 74). The plate
  reference, staging description and hotspot label of a cast entry come from `presented_as`, never
  from its true record; a cast entry presented as a set piece (a mimic as a chest) is drawn and
  labelled as that, its affordances are the decoy's, and both carry a shared `same_as` id so an
  action on the decoy resolves through the entry's rules.
- **RES25. What a scene is (W6, W7).** A scene is one place from the site graph with one
  background, and it is the group of seats in that place. Moving between areas inside a place is a
  layout move in the same scene and epoch. Moving to an adjacent place is a scene change. The dress
  step prefetches adjacent places on the `prefetch` class once the party is within one move, reuses
  the stored canonical background, and shows the establishing card (section K) until the new plate
  lands. Characters at different positions in one scene are never a `Split`. Only separate places
  create separate scenes, and until that ships `Split` is declined `not_supported_yet` with "to be on
  your own, move to another area of this place, or ask the host to pause". Where a table has
  several scenes later, the TV focus changes only after `focus_min_dwell_secs` and never mid-beat.
- **RES26. Pending ownership (K3, F14).** The `PendingResolution` is held in table state and staged
  by `df-engine`. The active machine holds only `Waiting{pending_id}`. Answers carrying a pending id
  go to the pending handler and not to `accepts`. A hand-off is declined `pending_open` while any
  pending resolution of that scene exists, unless the hand-off is that resolution's own result.
  `AwaitingAdjudicator` is a state of a pending ruling and not of a machine. A confirmation is
  `pending{choice}` for the same seat, `Rejected` maps to `declined{intent_invalid}`,
  `PartiallyCompleted` maps to `applied` plus a clarification, and `Cancelled` maps to `applied`
  with the uncommitted part dropped. Pending kinds are choice, roll, reaction and ruling. A roll a
  player must make is `pending{roll}`, and the draw is made by the existing single draw point
  (`gameplay/dice.rs`, with `dev dice next` as the operator queue).
- **RES27. The machine contract (K1, K2).** The contract is:
  `enter(table, scene) -> state`;
  `accepts(seat, input, state) -> bool`;
  `step(state, input, clock) -> StepResult{state, table_effects[], resource_requests[], outcome}`,
  where `outcome` is `applied`, `needs_clarification`, `declined{code, reason, alternatives}`,
  `pending{choice | roll | reaction | ruling}` or `Waiting{pending_id}`;
  `exit(state) -> HandOff{target, carry[]}`.
  Inputs are a player `InputEnvelope` (operation id, seat from the session, `view_revision`),
  `Dice{draw_ids, values}`, a resource result, a timer, an engine proposal, and an answer addressed
  to a pending id; the pending handler resolves the answer with the rules owner and then calls
  `step` with the result. `Waiting{pending_id}` is the machine's report of an open pending, and the
  pipeline records the input's outcome as `pending{...}`, so the four outcomes stand. APPLY calls the
  active machine's `step` with the DECIDE outcome and its draws to
  compute the slot after-state, then commits. Resource requests become committed effect intents in
  that commit. A machine owns neither a pending resolution nor the dice, and declares what it does
  with its in-flight requests on suspend. Hand-offs form a stack of depth three; each frame stores
  the machine, its slot and a `resume_check`, and on return `resume(state, table_state)` may reject
  the frame and fall back to explore. A rest interrupted by combat resumes in the rules owner's
  restart mode and never continues silently.
- **RES28. Machine scope in the rest of the list (K6).** The first machines are lobby, creation,
  advancement, explore, dialogue, travel, rest, combat, shop (browse, haggle, offer, done; D.7),
  inventory (who holds what, worn, contained, moves) and item inspector (D22, D23). "Market" is a
  kind of place and not a machine. The shop machine holds a per-seat, per-vendor offer map
  `encounters: map<(seat, vendor), ShopState>`; a vendor serves one open offer at a time and queues
  others, and the stock decrement is part of the deal effect, so two seats cannot buy the same last
  potion (the second is declined `stock_changed`). Scope: lobby, table; creation, advancement, inventory (own
  character) and item inspector (personal), seat; the rest, group. Transfers between characters are
  group-scope `TableEffect`s proposed by the inventory machine and applied by the engine. Downtime,
  puzzle and chase are registered later, and adventure end is specified in release 1 (K.21). A chase runs in the combat machine as an encounter whose objective is escape or
  catch (`scene-combat-binding.md`) with each leg a layout move, if R17 does not mark a chase
  module required (question 75).
- **RES29. Time, safely (W14, W15, W20).** A `RequestTimeAdvance` that would cross a scene with an
  active combat or an open pending resolution is held and declined `time_locked`. Turns of
  creatures that cannot choose (dying, stunned, paralysed) run through the rules owner and are never
  held. A seat can be rebound to a second device only with a host-approved short-lived pairing code,
  never by inference. A held turn counts as activity for the idle clock, and the table cannot go
  idle while any pending resolution or held turn exists. `table.pause` flushes the plate queue and
  rejects in-flight results by the `snapshot_revision` on each job. `table.resume` runs a roll-call
  before the first beat: any seat with a turn, window or pending resolution that is not connected
  gets a visible host choice (takeover, offstage, wait). The recap is two projections, a public one
  from public committed events and a private one per seat that follows the member and observer rules
  of `campaign-cinematics.md`, and a combat resume shows the combat board (initiative order, HP
  chips, whose turn) before narration. A session is one play sitting between `table.resumed` and
  `table.paused`.
- **RES30. Projection is pure and cheap (F2, F13).** `project(snapshot, attachments, audience) ->
  View` is a pure function. Attachments (notebook, table log, tempo, plate slot, layout, voice and
  media URLs) are published by their owners at commit or job-result time into an immutable snapshot,
  and a view call never commits, writes a file or reads provider state. The watch channel carries a
  shared snapshot, memo keys are the basis revision, and wake gating uses `(state_revision,
  attachment_version)`: an attachment publish (plate, table line, tempo cue, layout, mural) produces
  a frame at the same state revision that clients merge by the existing `live_art` rules, because
  the client rejects an equal revision as a duplicate. A keepalive resends the cached last frame at
  least every 5 seconds without recomputing, because clients treat 8 seconds of silence as a dead
  tunnel. This starts from `wire::project_journey` and `actor.rs::view`; `df-display::project_display_view`
  is a tested library function with no production caller, and slice 1 decides whether to adopt or
  replace it. Pacing, spotlight and beat dwell are owned by
  `df-experience`, `df-tempo` and `tempo-engine.md`, which the redesign feeds and does not duplicate.
- **RES31. Release gating by rules coverage (F3).** `LegalChoices` is a trait. Its first
  implementation adapts today's offers and affordance sets, read-only. A family marked `No` in
  `df-rules/src/coverage.rs` (spellcasting and exploration today) declines `not_supported_yet`, and
  release 1 registers a machine over the capabilities whose rules are Yes or Partial there:
  travel and explore register as adapters over authored beats, and `No` capabilities (spells,
  hazards, light, searching) decline `not_supported_yet`.
- **RES32. Storage honesty (F1, F11, F12).** Until the stabilisation gate a machine slot and a scene
  are bounded, versioned, policy-labelled journal records in the checkpoint, and `TableEffect` is a
  Rust enum applied by one `stage_effects` that calls the existing `df-engine` stagers. At the gate
  they are promoted to typed state in one schema bump. Each side store (social ledger, notebook,
  table log, tempo, table settings, dialogue state, plate book, hub registry) is classified as
  derived (rebuilt on load), durable-outside (idempotent apply keyed by operation id) or moved into
  the checkpoint, and a machine slot is never a side store. Plate ticks, playout, hotspot hover,
  projection and scheduler bookkeeping never commit; only an input that changes rules, world or
  machine state does.
- **RES33. Wire and CLI (F5, F18, F19, F15).** One generic message pair is added to
  `gameplay.proto` under the field ledger: `MachineView{machine_id, state_label, offers[], status,
  reason, revision}` and `MachineIntent{machine_id, offer_id, payload}`. New machines use them and
  `JourneyView` stays until combat migrates. The CLI gets one route `/dev/machine/{id}/{verb}` and
  one command `dev machine <id> <verb>`, with `describe` generated from the registry, the H.16
  verbs as a fixed set, and `force` and `drive` as new owner-inbox jobs. Interaction machines are
  called **table machines** and the client display ones **stage machines**. New code may not call
  `record_audit_line`; a shared event builder feeds `df-observe` and the dev timeline, and a boundary
  script like the existing `development/check-*.py` enforces it.
- **RES34. Words (K27).** "DM" is the pipeline. "Storyteller" is the narrating text model.
  "Host" or "adjudicator" is a human or a reviewed automatic case. "Scene" is a place and the group
  of seats in it. `plate_tick` is the picture clock, `world_time` is campaign time and `Clock` is
  the injected time source. "Effect" means a `TableEffect` for the redesign's changes and an
  `ActiveEffect` for rules. "Layout" is authored or generated areas, terrain and set pieces, "position"
  is rules state, and "anchor" is screen-space only. "Seat" is a `MemberId` in a session. `view_revision` is the revision a seat's projection carried when
an input was drawn (the wire's `observed_revision`); `state_revision` is the revision an INTERPRET
result was read against; `snapshot_revision` is the snapshot a job was cut from; all are the table
commit revision at different moments. An establishing card is the TV state while the plate is not
Ready (K.2); a place card is its minimal content (name, one line, cast) and the fallback when a
place or background build fails (F.3, H.4). The identifiers `dm_only` and the rules family name
"DM procedures" (R17) keep their names. `stat_ref`
  is the profile id of `scene-combat-binding.md`. "Phase" is the old word for a machine. Fixed player phrases are those of C.7 and K.5 ("I could not
read that. Pick one:").

**J.3 Walkthrough amendments (W).** The rest of the walkthrough findings, each at its owner.

- **Questions and non-action input (W12, X1).** See C.8 in section K.
- **Mid-scene admission (W10).** `scene.admit{entry}` adds or removes cast during a scene and is an
  engine proposal. A person created in play comes from a content or template entry with a ready
  `combat_binding` only; with none, the person is declined as a combatant until bound. A person
  with no portrait enters the plate prompt as text and the portrait queues on `prefetch`. A
  combatant joins at the next round with the rules owner's initiative draw.
- **Social action in combat (W11).** An Influence action by the active creature is a combat intent
  whose resolution reads and writes the suspended `ConversationState` through `df-interaction`, and
  its speech is rendered under the combat projection. A creature that stands down emits
  `participant.stood_down{creature, reason}`, and the encounter ends with `parley` only when no
  hostile participant remains.
- **Hazards in area-granular exploration (W16).** A hazard set piece carries `area`, `trigger`
  (enter area, cross threshold, touch, pressure point) and `avoid_by` (route choice or check). An
  explore move names a route over area adjacency, and the trigger goes through the rules owner's
  hazard resolution (R14) with the passive result private.
- **Theft and the purse (W17).** `item.moved` and `purse.changed` carry `noticed_by[]`. A seat not in
  it keeps its last-known purse value, labelled unchecked, until a noticing check, a count by the
  player or an effect marks it noticed. Whether that is acceptable to the rules owner is
  question 76.
- **Claims (W22).** A social intent carries `claims[]`, each a reference to a fact the speaker's
  character knows or marked `unbound`. INTERPRET maps the line to existing fact ids only, truth is
  the stored value, and an unbound claim is flavour that updates no belief.
- **Level-up timing (W23).** An award creates `advance.offered`. The old sheet stays authoritative
  until one commit applies the player's validated choices and bumps the identity revision. The
  commit waits while the character is in an encounter and applies at `combat.ended`, and the seat
  shows a persistent "level up ready" chip.
- **Engine responders (W26).** A window may declare `responder: seat | engine`. An engine responder
  resolves through the perception-limited `df-combat` call inside the same commit that opens the
  window.
- **Narration order (W24).** Narration jobs apply in `turn.seq` order per scene and each caption
  holds for `caption_min_dwell_ms` before the next.
- **Polymorph and speech (W28).** `qualify_expression` rejects an envelope whose speaker's active
  form forbids speech, and a non-speaking speaker gets a gesture clause only.
- **Late join (W21).** A `Joining` seat is in the creation lane, receives a private catch-up of
  public facts only, appears as text in plate prompts until a portrait exists, and enters at the
  next beat boundary or the next round.
- **Input kinds without a path (K35).** `PrivateMessage` and `SafetyCard` skip INTERPRET and CHECK,
  commit, and PROJECT, ending `applied`. A whisper to an NPC is an action or social input with
  `audience = [seat, npc]`; its outcome and speech project only to that audience, the NPC gets the
  witness record, and the TV shows a marker with no content.
- **Decline codes added.** `pending_open`, `resource_spent`, `pvp_off`, `consent_declined`,
  `time_locked`, `not_knowable`, `stock_changed`, and the 'why' text of each.

**J.4 Settings introduced by rounds 1 and 2 (K29).**

| Setting | Default | Cost effect |
|---|---|---|
| `plate_interval_secs` | 15 | plates per hour (upper bound) |
| `image_stale_limit_secs` | 90, set from the measured lane | none |
| `scene_build_budget_secs` | 8 (section K) | none |
| `plate_swap_wait_secs` | 6 | none |
| `narration_late_secs` | 8 until measured (H.5 budget, plus `narration_batch_secs`) | none |
| `narration_batch_secs` | 4 | fewer narration calls |
| `beat_min_dwell_ms` and `caption_min_dwell_ms` | formula in section K | none |
| `idle_minutes` | 10; eviction at 30 | stops plate cost when idle |
| `window_timeout` | none: windows hold (RES26) | none |
| `held_nudge_secs` | 30 | none |
| `ruling_nudge_secs` | 20 | none |
| `focus_min_dwell_secs` | 45 | none |
| `reading_pace` | normal | scales dwell |
| per-seat pending | 3 | bounds model calls |
| `solo_beat_max_exchanges` | 6 | none |
| `evening_minutes` | host-set, waits on the owner (RES49) | none |
| first-scene hold | 20 s | none |
| plate dissolve minimum | 1.5 s | none |
| status banner | 8 s | none |
| stall watchdog | 20 s | none |
| job lease | 60 s | none |
| SAM retries | 1 call retry, then one bounded regeneration | per-call price |
| broker retries | 2, never for a refusal | per-call price |
| owner lane caps (engine, timers and results) | measured in slice 0 | none |
| `plate_min_on_screen_ms` | the reading time of the lines shown over it (RES41) | none |
| rate limit per seat | 3 a second burst, 1 sustained | bounds model calls |
| queue bounds | plate 4, prefetch 8 | bounds plates in flight |

### K. The table experience contract

The round 1 critics concentrated on rules and engineering. This section is the experience a table
has with this DM, from the round 2 experience critic's findings (X), and it is binding on the
projection, the pipeline and the machines in the same way as section H. The mechanics stay with the
rules owner throughout, and narration follows committed outcomes.

**K.1 Questions and recall (X1, W12; completes the C pipeline).** The dispositions other than an
action are handled as follows, and none of them commits state except a recall grant.

- A perception question ("is the door locked?") is answered from the scene record and the asker's
  `KnowledgeGrant`s with no check when the answer is plainly visible.
- A rules question (`Meta`) is answered from the rules text and never changes state.
- A recall question ("what do I know about these runes?") is the Study action. Every knowledge fact
  carries a recall skill and DC assigned when the fact is admitted, from authored content or by the
  admission validator from the source's own table, never by the narration model. A fact the
  character's background plainly gives needs no roll. The result is a grant to the asker, shown
  privately.
- A question with no grounded answer returns "You don't know, and nothing here tells you. You could
  search, ask Mara, or study the runes." It never returns an invented fact, and a subject outside the
  asker's knowledge is declined `not_knowable` with alternatives.
- A request about an object that a place of that kind plainly has ("is there a table I can flip?")
  is a question answered from place-type defaults in content data, not a decline. Only an object that
  makes no sense for the place is declined.
- `Social` goes to dialogue, `PlanOnly` goes to a table-notes strip that nothing executes (question
  77), and `Joke` gets a short in-character reaction with no effect.

**K.2 Waiting for art is never a gate after the first scene (X2).** A scene is playable as soon as
its description, cast, set and layout (the dress step) are stored. Until the plate is Ready the TV shows an establishing card:
the place name, the stored description voiced as the first narrator line, the party strip, the
place music and a small "picture on its way" tag. A background that arrives first shows as the
backdrop with a slow dissolve, and a late plate dissolves in only at a line boundary (K.3).
`scene_build_budget_secs` defaults to 8, after which play continues on the establishing card with no
blocking state. Only the first scene of a session or an authored arrival beat may hold up to 20
seconds, with narration filling it (question 78 asks whether the first scene is built during session
zero). Travel narration is timed to cover the build, and the narrator never says "you arrive"
before the destination record exists. This is the answer to the seven-minute travel hang and it
replaces "setting up until Ready" wherever that text remains (the budget is also the trigger for
the place card).

**K.3 The clock requests; the buffer decides when to show (X3).** `plate_tick` decides when a plate is
requested, never when one is shown. The playout buffer swaps a plate only between lines, never
during a dice or result card, never during a reveal beat (K.7), and with a dissolve of at least 1.5
seconds. A ready plate that finds the slot blocked waits at most `plate_swap_wait_secs` (default 6)
for a boundary. Speaker, gesture and mood changes are overlays on the current plate (speaking mark,
portrait inset) and need no new plate, and a tick is skipped when only overlay-level state changed.
The "picture catching up" mark fires only when the stale plate disagrees with the table (the scene
epoch, the cast or a listed item changed), and a quiet stale plate shows nothing. The stage
conductor decides the swap instant within `SCENE_ENTER_CAP_MS` and reports it through
`ReportPresented`; the server decides which plate is next.

**K.4 Rulings do not freeze the table (X4).** A `RulingRequest` shows the requester "Needs a
ruling" with the engine's suggested answer and its source basis. The host's phone gets one tap:
accept the suggestion, change it or decline. The wait never silently ends and the table is not
frozen: outside combat other seats act on their own turns, and in combat the turn stays held for
the ruling. After `ruling_nudge_secs` (default 20) the host sees "Pip is waiting on a ruling" and
the requester sees the supported alternatives as buttons. A stored ruling is reused (C.3). The
default policy is "reviewed automatic cases, otherwise the host", and `ruling.wait_ms` is logged so
the owner can measure how often creative play stalls. Question 79 asks whether a vote of the seats
may adjudicate low-stakes rulings; without an answer to it or to question 57, expressive play depends
on one phone.

**K.5 Acknowledgement and plain words (X5).** The phone echoes a typed line as sent within 0.3
seconds and the TV shows the hero's figure with a composing mark. Named stages follow with fixed
phrases: "Reading your move" (INTERPRET), "Rolling" (DECIDE), "Telling it" (NARRATE), with the
structured line already visible. Every status other than `ok` has one plain sentence naming the
cause and, where a person is the cause, that person ("Waiting for Pip to answer a reaction",
"Picture catching up", "The DM could not read that: pick one"). No codes appear in player text, and
the reason text of `not_supported_yet` reads "I can't run that yet." Dice and result cards of three
seconds or more are staged first so they cover narration latency. This depends on question 65
(INTERPRET on the fast lane), because today's Codex text calls take 15 to 45 seconds.

**K.6 Fairness among seats (X6).** The owner queue is served round-robin by seat, with a bound of
three pending per seat, so `table_busy` applies to the seat over its own bound and never because of
another seat. Outside combat, inputs from different seats arriving within `narration_batch_secs`
(default 4) commit separately in arrival order and then share one NARRATE call over the list of
committed outcomes. The text service only describes committed outcomes, so no meaning is merged.
Structured lines show per seat at once, and a waiting phone shows its place in the queue.

**K.7 Narration brief, callbacks and foreshadowing (X7).** A `NarrationBrief` is built by pure code
per audience and holds the committed outcome, the public scene (set pieces, light, weather, cast
ids), the speaker's voice card (K.9), the table's tone settings (K.22), the last six narrated lines
to avoid repetition, up to two callback candidates retrieved from the committed event log and story
memory (each with a fact id and a one-line summary), and at most one seed action from the
foreshadow ledger. The ledger belongs to `df-narrative`: each planted seed (an omen, a rumour, a
clue, an unfinished threat) has a fact id and a state `planted`, `echoed`, `paid_off` or `retired`,
and the director must pay off or retire a seed within a stated number of scenes. A narrator clause
that references a fact must return its id, and qualification rejects an id outside the brief or the
audience with the leak check of H.6. No callback means no callback. A reveal beat (the payoff of a
seed or a secret) is a flagged moment that uses the tempo stinger, extends the minimum dwell and
holds plate swaps. Question 80 sets the callback cap.

**K.8 The narrator cannot promise what the engine will refuse (X8).** Every physical thing the
narrator introduces is a set piece id or a registered atmosphere item, and qualification rejects
other physical nouns by id mismatch, not by keyword. An atmosphere item that a player refers to is
promoted to a set piece through the generated-content admission path within the budget of one dress
call.

**K.9 Voice (X9).** A character record gains `voice_card` (register, three verbal tics, how they
address strangers, two example lines written and validated at admission) and `voice_id` (a fixed
voice and pitch assigned at admission and never changed). Every prompt for that character carries
the voice card verbatim and their last three spoken lines. `character show` prints both.

**K.10 Solo beats (X10; the draft default for question 60).** A declined `Split` (RES25) offers "Pip
scouts ahead (a short solo scene)". The rest of the party stays in the shared scene with time
frozen (a `df-world` pause with reason `solo_beat`), the scouting seat gets a private narrated
exchange of at most `solo_beat_max_exchanges` (default 6), and the TV gets one public summary line
tagged "Meanwhile". The beat ends on a rules outcome or the player's return. Question 81 asks whether
a solo beat is acceptable for the first release. When several scenes exist, the TV cut rule is: a
scene with a pending resolution wins, then least recently shown, with a minimum on-screen time of 45
seconds, and a strip names the other scenes and who is in them.

**K.11 Combat is legible (X11).** Every seat's phone offers a flat tactical strip drawn from the
rules owner's grid, never from the plate: positions, the active creature's movement, and reach or
range for the highlighted action. The TV shows the turn-order bar, HP chips anchored to creatures
and a toggleable minimap of the same diagram. Legal targets are highlighted, and the choice list
says why a target is out of reach ("12 ft away, reach is 5 ft"). The client already computes
`reachable_from` in `df-render/src/battle/grid.rs`; moving that computation behind the machine view
is listed as debt in section L.

**K.12 Combat pace (X12).** The dwell is `max(1800, 330 * words)` milliseconds for text read across a
room, at least 3000 for a result card, scaled by the table's `reading_pace`. Consecutive turns of
creatures on one side that did not target a party member collapse into one beat with a summary
line ("The goblins close in"), and turns that hit a hero keep their own beat. A seat two turns away
sees "You're up after Mara" with its legal choices for a plan-only note, and the next seat gets a
haptic and a chime. After `held_nudge_secs` (default 30) a held seat's host sees one line with the
approved commands (wait, pause, take over). The engine never acts for the seat by itself.

**K.13 Every hero is always visible (X13).** A party strip runs along the bottom of the TV, always
shown, drawn from each hero's own portrait with HP and condition chips as an overlay. Plate reference
slots after the speaker rotate by least recently referenced, so across a scene each hero is drawn
faithfully in turn. The `df-experience` spotlight offers are inputs to the director, counted per
hero, and never infer disinterest (`engagement-model.md`).

**K.14 The phone's choice list (X14).** At most four suggested choices, ranked by a fixed rule (used
recently, then affects a target present, then cheapest economy; no model), then fixed groups
(Attack, Cast, Move, Use, Talk, Look). Cast expands by spell level with a default filter "only what
works on something here". Illegal options are hidden by default with a "show unavailable" toggle that
gives the reason. Labels are at most four words with a one-line explanation on tap and hold, and a
text field is always visible. Text that matches a legal choice executes that choice's id with no
model, and everything else goes to INTERPRET.

**K.15 Secret checks in a shared room (X15).** A secret check shows the TV the same neutral cue ("The
DM passes Pip a note") for success, failure and no information, with the same shape and duration.
The private result opens as a note card on the phone that auto-hides after 10 seconds, with "share
with the party" (a `PrivateMessage` to chosen seats or all) and "keep it to myself". Nothing about
the note appears on any other seat.

**K.16 Speech and voice (X16).** A `Say` input up to 800 characters is shown verbatim on the TV as the
hero's line in up to three bubbles, and INTERPRET never rewrites spoken words; it only extracts
intent and claims for the rules. (Action text keeps the 280-character cap of C.7.) Voice is
hold-to-talk on the speaker's own phone only, so seat attribution is by device, with the transcript
shown for two seconds with edit and cancel before it enters INTERPRET and the scene's names passed
to the recogniser as hints. Irreversible readings still need the C.5 confirmation, and a speech is
Influence on the audience present with the rules deciding the effect.

**K.17 Terms (X17).** INTERPRET may return `ProposedTerms{give[], ask[]}` using only registered term
types (coin, item, service, information, obligation). The phone shows a terms card for the player to
confirm, the NPC's counter and stance come from `df-interaction` and the rules owner's Influence
outcome, and an accepted deal creates obligations through the approved commitment action. Terms the
registry cannot express are declined with what can be offered.

**K.18 Party choices (X18).** A `PartyChoice` pending resolution addresses all present seats. The TV
shows the options with live tap counts (names only if the table enables them). It resolves when every
connected seat has voted, or when the host taps "Call it" (the plurality, ties to the host's
choice), and abstaining counts as a vote. No timeout resolves it. Question 82 asks whether the table
wants a standing caller.

**K.19 Novices, children and access (X19, X20).** A "What can I do?" button is always visible and lists
the top choices in plain words. Tap and hold explains any choice from the catalog in plain language.
A goal line in the phone header comes from the journal projection. A session-zero "novice" setting per
seat reduces choices (K.14) and adds a guided prompt on first turns, and tone settings include reading
level and narration length, so a child seat defaults to short narration and voiced read-aloud. For
access: every plate has alt text, which is its public staging description; a phone live region
mirrors TV narration in order with combat events collapsed to one summary per round; `reading_pace`
scales dwell; a per-seat "read to me" plays private text through that seat's audio route
(`remote-play.md`); turn markers and condition chips differ by shape and label as well as colour; and
captions are on by default for voiced lines.

**K.20 Rests and rewards have a moment (X21).** The rest machine shows a campfire beat: the party
strip with resources restoring and the watch order, and one optional prompt per seat drawn from its
hero's hooks through spotlight offers that can be ignored with no penalty. Scene builds run during
it. A level-up, a loot reveal or a quest completion is a flagged reward beat on the TV (stinger,
four-second hold, the hero's portrait) while the private choices stay on the phone.

**K.21 Session shape (X22).** The lobby stores `evening_minutes`. At about 85 per cent the director
prefers a chapter end (offered, never forced). The session-end screen on the TV shows what the party
did, from committed events under the recap rules of `campaign-cinematics.md`, with its text-and-stills
fallback; awards; one open thread as a teaser; and the pacing vote of `engagement-model.md`. A new
session shows the recap and guarantees one callback to a prior-session choice in its first scene
(K.7). The adventure-end machine is specified before the first release because it is where a table
decides whether to return. Question 83 asks whether the default recap is a budgeted video bookend or
a text-and-stills card.

**K.22 Smaller table-craft rules (X23 to X27).**

- **Tone.** Session zero stores `tone` (heroic, grim, whimsical), `humour` (0 to 3),
  `violence_description` (implied or plain) and `narration_length`, and the broker injects them with
  lines and veils. Humour is allowed only in a clause tagged flavour, so it cannot carry an outcome.
- **Flaws.** A "play to my flaw" tap on the sheet passes the declaration to NARRATE as stated. The
  engine may suggest Heroic Inspiration to the host (accept or decline); the rules owner owns the
  award and the model never awards it (question 84 asks whether the rules plan treats it as
  adjudicated or discretionary).
- **Table talk.** A hero's said words appear on the TV and actions appear as a short structured line.
  A private message never appears. Talk aloud in the room is not input, and the phone offers "tell
  the DM what we decided" as a plan-only note.
- **Catch-up.** A "story so far" view shows the journal projection and a "what just happened?" button
  that lists the last minute's committed events with no model call.
- **Declined asks.** An opt-in host setting stores declined free-text lines (never accepted ones) in
  the telemetry store, and `declines top` counts them by code, so the owner can see which creative
  asks fail.

**K.23 What not to change.** The experience critic named the strongest parts of the design: the four
outcomes with declines drawn from the same legal-choice call as the phone's buttons; rules first and
words after with a structured readout fallback; per-audience projection with paired-state and
sentinel-secret tests; durable pending resolutions with held turns that never act for a player; and
scene epochs with travel that does not block and plate rules that make a stale-scene plate
impossible.

### L. The current implementation compared with the next one

The owner asked for this comparison once the plan and its gaps were settled. The facts about the
current code come from two independent searches of the code on 2026-11-02: the round 2 feasibility
critic and a round 3 fact-check that corrected the first version of this section. Nothing was run.
Claims the fact-check did not reach are marked unverified, and no effort or time is estimated here:
slice 0 produces the numbers.

**Update after the removal (branch `gameplay-engine-v2`, commit `14a2f715`).** The old gameplay
code described in this section was deleted from the working tree on 2026-11-02, so section L
records the implementation as it stood at commit `2b5bc721`, for reference. The things L.4 lists as
staying inside `df-tools` (the `Hub`, the cost book, the timing ledger, the plate director, the
stage conductor, the dice draw point, the table settings store) went with it and are reread from
history and rebuilt on purpose; the library crates (`df-rules`, `df-model`, `df-session`,
`df-persistence`, `df-imagery` and the rest) stay. The strangler order of L.6 is void and section
O.9 replaces it.

**L.1 The shape of what exists.**

- `df-tools` is 506 Rust files and about 474k lines: the native game server `gameplay/` (about 368k
  lines, tests included) and the wasm client `gameplay_browser/` (about 52k). Server and client are
  one crate, and every TV and phone runs one wasm bundle built from it (`df_tools_bg.wasm`, about 30
  MB), which the server reads into memory at start and cannot hot-swap (runbook trap 26).
- Large files: `gameplay/actor.rs` 7,223 lines, `dialogue.rs` 11,897, `journey.rs` 9,031,
  `resources.rs` 1,982, `wire.rs` 2,336, `journey/tactical.rs` 11,124, `journey/scene_fight.rs` 3,807,
  `merchant_bargaining.rs` 3,511, `adventure/scene_model.rs` 3,448, `pack.rs` 3,660.
- Locally one `df-tools` process serves one run. For hosted play `df-runner` is a launch layer that
  starts one such process per active campaign, so per-table limits are per-process limits.
- The libraries (`df-engine`, `df-world`, `df-knowledge`, `df-interaction`, `df-narrative`,
  `df-encounter`, `df-intent`) are pure staging crates that `df-tools` uses in a few files each
  (engine 8, narrative 5, intent 3, encounter 3, world 2, interaction 2, knowledge 2). Most game flow
  lives in `df-tools`.
- The state is a closed, validated `Checkpoint` (schema 2, codec v3/v4). New data enters as
  policy-labelled journal text, for example `place.rs` with `POLICY = "place-move-1"`.
- **Flow control today.** The wire carries `JourneyPhase` (six values). Inside the server a derived,
  forward-only `SessionState` (Lobby, Story, Conversation, Fight, Aftermath, Ended, Preparing) in
  `journey/session_machine.rs` has an edge table, and the pack's `BeatScript` sequences scenes
  (`beat_script.rs`, `beat_script_runtime.rs`, `ScriptMode::Freeplay` for free roam). Routing is
  `dialogue.rs` and `journey.rs`.
- **Other areas the migration touches**, which the first comparison left out: combat (21k lines under
  `gameplay/journey/`, plus `df-rules/tactical`, `df-combat`, the 50k-line `df-render` battle code and
  `live_battlefield.rs` 4,339 lines); the hero creator (`hero_creator.rs`, `hero_library.rs`,
  `creator_catalog.rs`, creator portraits); the lobby and room (`room.rs`, `df-session/lobby.rs`,
  reclaim and rejoin, display pairing); hosted and commercial paths (`runtime_launch*`,
  `image_hosted_authority.rs`, `df-hosted`, `df-campaign`, `df-server`, `df-runner`); authored content
  (`content/harbor-journey`, adventure tables, free roam, merchant shop, battle scenes); memory
  (pgvector recall), music and voice; and the test assets (`gameplay/fixtures/*beat-script*.json`,
  `harbor-journey-pack-baseline.json`, `df-testkit`, `df-acceptance`).

**L.2 Side by side.**

| Concern | Current | Next | What changes |
|---|---|---|---|
| Flow control | `JourneyPhase` on the wire, a derived `SessionState` with an edge table, scene sequencing from the `BeatScript`, routing in `dialogue.rs` and `journey.rs` | Table machines in a registry (RES27, RES28), each a module with a contract and a checklist | Slice 4 wraps the `SessionState` edges as the first adapter machines; logic moves out of the two big files machine by machine |
| Player input | `DialogueRequest` (9 variants) plus pre-judged proposals committed through owner calls; free text passes classify, addressee and reason stages; confirmation already exists (`confirmation_required`, Confirm and Cancel); the dev audit names 11 decision kinds (line, trade, quote, attempt, action, target, npc-target, approach, freeform, scene, npc-event) | One pipeline: admit, interpret off the owner, check, decide, apply, narrate, project, four outcomes and decline codes (C, RES22) | INTERPRET v1 adapts today's judges in a fixed order; a judge goes only when its machine absorbs it |
| Waiting | One production model wait on the owner thread, `resume_fight_reactions` (`actor.rs:1924`, 8 s). The other `block_on` sites under `gameplay/` are in test modules or process bootstrap (`run_restart.rs`). Every decision commit, operation lookup and load also blocks the owner thread on Postgres (`df-persistence/native_bridge.rs`, `actor_block_on`). Judges already run off the owner in the async handlers | No owner call awaits a model or media call; judge off the owner, commit on the owner with a basis check | The 8-second wait belongs to the combat slice. The Postgres waits are inside the owner's reduce time, so the budget "rules-only under 400 ms at p95" includes commit latency, and slice 0 measures `commit_scoped` separately |
| Rules binding | Offers hand-built in `wire.rs` and `journey/tactical.rs`; 28 wire action kinds (`GameplayActionKind`, values 1 to 24 and 40 to 43; the dev `describe` lists 25 because its range stops at 40, an example of hand-registration drift), harbour- and fighter-specific; `df-rules` covers fighter levels 1 to 5 and an SRD tactical subset; spellcasting is `No/No`, exploration is rules `No` and live `Partial`, resting `Partial` | A `LegalChoices` trait, first implemented by an adapter over today's offers, with `not_supported_yet` for capabilities the rules owner does not cover (RES31) | The design cannot promise spells, searching or hazards until their rules exist |
| Projection | The live view is `wire::journey_view` / `project_journey`, `actor.rs::view` and the plate application in `living_scenes`. Each watcher wakes on five sources (checkpoint, table lines, layout, art, a 5-second sleep) and each wake calls the owner's `Call::View`. That path writes files (notebook, social ledger), publishes into process-global maps (`living_scene::publish`, `place::publish`), advances tempo at view time, requests hand items, clips and the mural, and can commit a picture feature and rebuild the view. A 16-view memo compares deep equality and is bypassed in lobby, creation and combat. `df-display::project_display_view` is a tested library function with no production caller | A pure `project(snapshot, attachments, audience)` over an immutable snapshot (RES30) | Slice 1 decides whether it adopts or replaces `df-display`. Attachments are published at commit or job-result time. About 86 non-test process-global statics in `gameplay/` (including `pack::with_content` and `scene_model::PRESENTATION`) become attachments |
| Hidden information | `df_knowledge::perception::perceive` with shared or member scope (unverified in detail) | Audience on every fact, public projection for plate prompts, hotspots and the TV, sentinel and paired-state tests (RES6, RES24, H.11, RES35) | Extends what exists; the plate and prompt paths are new |
| Scene and plate | Event-driven: one worker, a queue of 16, a book of 32, mood hysteresis, a 15-second throttle that applies only to face edits and state images (other values: 1-second poll, 2-second look-ahead delay, up to 3 look-ahead and 6 successors, a 25-second head deadline), requests largely made by views ("the next view asks again"); plate identity ignores the graph revision (`plates.rs:473`); the display already receives `live_plate_graph_revision`, so a partial stale-plate guard exists; cut-out figure avatars | `plate_tick` feeding the same queue; a playout buffer; `(scene_id, scene_epoch, snapshot_revision)` on every job; anchors tied to a plate id; establishing card; no cutouts (RES13, K.2, K.3, D.2) | The queue stays; the clock, playout, epoch and anchor rules are new; the stage conductor keeps the swap instant |
| Locate | SAM masks for figures (`imagery/figure_masks.rs`); `SegmentationPrompt` is a closed enum (Person, Head); items are not located; no hotspot message anywhere | One request per prompt for people and listed items, exclusive assignment, hotspots from anchors (RES1, J.1) | Item hotspots need new work in the SAM prompt API |
| Resource requests | Three stacks: text through `df-ai` and `df-persistence/text_attempts`, images through `df-imagery` (12 purposes, durable jobs), and the `Hub` for splat worlds, character clips, music sets, ambient layers and home video (voice is a reserved kind); consumers re-ask on the next view. Spend is metered by a separate cost book (`gameplay/cost.rs`, `df_commerce::tariff`, `dev cost`) with a charge-site guard test | The `Hub` extended with text and image kinds as facades; requests committed as durable intents; completions arrive as `GameInput::Job` (J.1, H.8) | No fourth runtime. Slice 8 updates the charge-site guard list in the same change that moves any paid call behind a facade |
| Durable intents | Single-slot lookup of the first pending `RunAi` and one retry; `EffectKind` already has `RunMedia`, `ArmTimer`, `CancelJob`, `PublishPresentation`, and `GameInput` already has `Job`, `Timer`, `Presentation` | Many pending intents with an executor registry and `SentUnknown` (H.1) | A generalisation of the lookup and the executors |
| Commit | Whole-checkpoint document plus fact, operation and intent rows per decision; no pruning found | The same fenced commit; only an input that changes rules, world or machine state commits (RES32) | Ticks, playout and projection stop touching it; slice 0 measures bytes per commit and rows per hour |
| Side stores | Durable outside the checkpoint: table settings (`StoredTableSettings`), hero library, text attempts, imagery jobs, memory index, campaign store, seat tables. Atomic files: social ledger, notebook, cost log. In memory: table log, tempo, plate book, hub registry, dialogue state (not all verified) | Each store classified as derived, durable-outside with idempotent apply, or inside the checkpoint (RES32) | The recovery and `table export` claims of H.1 and H.12 hold only after this classification |
| Wire | A closed `GameplayActionKind`; fixed player and display views carrying `operational_notice` (a free string) and `PlateState`; `DialogueResponse` carries a `DialogueDisposition` (including REJECTED), `confirmation_required` and Confirm and Cancel RPCs; `protocol_service_inventory.json` pins 12 services and 41 methods | `MachineView` and `MachineIntent` added on an existing service (a new rpc needs an inventory and access-class entry), a `status` field, a decline code and alternatives on `DialogueResponse` (RES33, RES40) | A protobuf change under the field ledger; it touches the wasm clients and rebuilds the single bundle |
| Reactions | In play: the tactical snapshot's `PendingReaction` and `ReactionWindow` (`journey/tactical.rs:788`, window bytes inside the snapshot, codec v4) and the actor `ReactionTimer` that declines an offer when its window runs out. `PendingResolution`, `ScopedRuling` and `ResolveRuling` exist in df-model, df-engine and df-rules but no production `df-tools` path creates or consumes them | `PendingResolution` plus `ResolutionWindow`, held and never timed out into a decline (RES26, C.4) | Slice 9 introduces them to play and migrates the tactical snapshot |
| Rulings | `ScopedRuling` and `HostCommand::ResolveRuling` in the libraries; no reuse by key; not used by `df-tools` | A keyed reuse record beside `ScopedRuling`, the ruling flow of C.3, K.4, RES48 | New record and the host phone controls |
| Dice | One draw point (`gameplay/dice.rs`, unverified in detail) with the `dev dice next` queue | The same point, supplied to `step` as an input, logged with a draw id (H.3) | Naming and logging, not a new source |
| Logging | Two functions named `record_audit_line`: `df-observe/src/lib.rs:99` forwarding to an installed sink and `gameplay/actor.rs:1411` which is that sink. In `df-tools` 341 call expressions in 69 files (311 under `gameplay/`, in 66 files); 503 hits in 139 files across the workspace; free text parsed by the dev timeline | Typed records through `df-observe` also rendered as one line from the same record id; new machines may not call either function; the boundary script covers both (RES33, RES41) | Old sites migrate as their code moves; `coding-style.md` §8.3 and §8.7 need the owner's amendment first |
| Dev CLI | About 100 `.route(` calls in `dev_control/mod.rs`; each command is registered in four places (route, match arm in `dev_cli.rs`, `print_help`, `describe_commands` in `reads.rs`); `/dev/describe` exists but is hand-written; family verbs exist under other names (`state`, `view`, `checkpoint`, `act`, `say`, `override`, `dice next`, `fight offers`) | One route and command per machine verb, `describe` generated from the registry (RES33, H.16) | The nine verbs are generated, not hand-written |
| Tests | About 4,200 test functions (3,200 under `gameplay/`). `development/df-tools-known-failures.txt` lists 17 tolerated failures, its header records 27 more unowned baseline failures, and the tactical suite fails 20 to 25 tests in parallel runs because of process-wide state. The `Owner` type is bound to `PostgresRepository`; only `recovery_qualification` and the image suites clearly need scratch Postgres | Machine conformance, golden sessions, property tests, chaos per failure row, `RuleFixture` runs, restart against existing data (H.17) | The conformance and chaos suites need owned scratch Postgres or a new in-memory repository. Baseline failures stay separate |
| Phone logic | `gameplay_browser/hero_creator.rs` (141 KB) holds creation logic; the client computes `reachable_from` (`df-render/src/battle/grid.rs:333`, used by `stage.rs` and `combat_feed.rs`) | Both behind server views before the creation and combat machines are accepted (RES37) | Not optional: client-side rules conflict with `AGENTS.md` |

**L.3 What each known failure does under the next implementation.** These are the failures seen in
play on 2026-11-02 (runbook traps in `development/runbook-codex-live-test.md`). The causes marked
"recorded" are symptoms in the runbook, and the code path is still to be found.

| Failure seen | Cause now | Under the next implementation |
|---|---|---|
| Phone stuck on its loading screen after a talk-down | The client refuses empty text (the client stand-in has landed, `live_screens.rs:531`); the server source of the empty narration is not identified | The projection is never empty and carries a status (RES30, H.15); the structured readout replaces a blank (C.7) |
| Market plate never arrived, old plate under the new title | Recorded as a failed reference request that left no trace; the cause is not identified | The scene epoch rejects the old plate (D.2); the establishing card and place card keep play going (K.2, H.4); the failure has an event (H.14) |
| Observation text always fell back | Hard-coded 8-second `CallProfile` deadlines (`investigation.rs:443,1616,2016,2241`; `freeform_action.rs:1550`) against Codex calls of 15 to 45 seconds | Deadlines are per purpose in the registry; INTERPRET uses the fast lane (question 65); narration is validated and may be late without blocking (C.7, H.6) |
| "I draw my greatsword and attack" read as Intimidation | Recorded in trap 31; the routing path is not identified | `Clarify` for an ambiguous harmful reading, a one-tap confirm, and golden fixtures that include the three known-bad lines (C.5, H.17) |
| Reply labelled with the wrong speaker, composer left disabled | Recorded in trap 32; the cause is not identified | One projection per audience from one snapshot; the composer state is part of the status (RES30) |
| Travel took about seven minutes with Send disabled | Place look-ahead exists (`place_move.rs`, `free_roam.rs`), but it is 0 at start when the storyteller is Codex and only two exits are warmed, so the move waits for place and person text | Travel never blocks on a scene build, with a defined "place pending" state (F.3, K.2) |
| Image runtime stalls and silent worker death | Missing lease and recovery rules at stage boundaries; `finish_or_defer` and survivable outcomes are now in `df-imagery/src/runtime/recovery.rs` | Leases, a reaper, epochs, watchdog and preflight (H.9, H.18); the runtime fixes carry over |

**L.4 What stays.** The fenced decision commit and operation identity (`df-session`,
`df-persistence`, `OperationId`, `DecisionReceipt`), the knowledge and perception scopes, the
imagery runtime and its recovery code, `df-rules` and its tactical state, the stage conductor and its
injected clock, the `Hub`, the cost book, the dice draw point, the table settings store, and the dev
CLI's existing commands (kept as aliases until the machine route replaces them). `PendingResolution`
and `ScopedRuling` stay in the libraries and are introduced to play in slice 9.

**L.5 What goes.** Cut-out avatars (D14), the event-driven plate cadence reasons that RES45 does not
keep, view-time commits and file writes, the 5-second recompute and deep memo equality, the
owner-thread model wait, the single-slot intent lookup, hand-registered CLI routes for machines, and
the free-text audit lines at each migrated site.

**L.6 Delivery order (corrected; void since the removal, see O.9).** Each slice leaves a playable game. Free runs use
`DF_FAKE_PROVIDERS=on`, but that profile switches off the storyteller, imagery, plates, Jev, memory,
voice and music and forces the fixed harbour adventure (`fake_profile.rs`). Free runs therefore cover
slices 0, 1, 4 and 6 and the non-model parts of 3. Slices 2, 5 and the three-bad-lines acceptance of
slice 3 need scripted in-process fakes (the `FakeEndpoint` pattern in the `approach_model` and
`storyteller` tests) or a paid run. Every slice names its roadmap gate and plan-database item and is
accepted under RES44.

0. **Measure.** `actor/timing_ledger.rs` and `dev latency` already give queue and reduce histograms
   per owner call and per hero-line stage. Slice 0 adds only what is missing: bytes per commit and
   rows per hour, the owner-wait inventory including the Postgres calls, async-side plate
   application timing (the ledger measures only the owner's `view()`), and the segmentation-route
   check. Free.
0b. **Wire preparation under the field ledger.** A `status` on both views, a decline code and
   alternatives on `DialogueResponse` (reusing `DialogueDisposition` and the Confirm and Cancel RPCs
   for Clarify), the interval and cut-out fields (`TableSettingChange` carries only
   `choice_hints_hidden` today), an attachment-version field for slice 1, and the
   `protocol_service_inventory.json` update. Touches `gameplay.proto`, `field-ledger.txt`,
   `df-client`, `gameplay_browser` and the wasm bundle. Risk: the single bundle and its ledger tests.
1. **Snapshot projection (RES30).** Pure `project`, attachments published at commit or job-result
   time, never-empty tests, the sentinel-secret test per audience. Wake gating uses
   `(state_revision, attachment_version)`: an attachment publish (plate, table line, tempo cue,
   layout, mural) produces a frame at the same state revision that clients merge by the existing
   `live_art` rules, because `df-client/revisions.rs` rejects an equal revision as a duplicate. A
   keepalive resends the cached last frame at least every 5 seconds without recomputation, because
   clients treat 8 seconds of silence (`VIEW_SILENCE_MS = 8000`) as a dead tunnel. Risks: view-time
   side effects to re-home (notebook and social writes, global scene and place publication, tempo
   advance, plate-application requests, the picture-feature commit); projection reads process-global
   singletons that become attachments.
2. **Plate clock, playout and epoch on the existing queue.** `scene_epoch` is defined now as the
   revision of the commit that last changed the scene key, derivable from the checkpoint with no
   schema change; slice 7 later stores the ids. Anchors tied to plate id, the establishing card
   (using today's scene title and cast until slice 7 stores the description), the interval setting.
   The cost view reads the cost book (`dev cost`), not only the `Hub` ledger. Needs plates on, so
   scripted image fakes. Risk: Codex latency leaves the display behind most of the time.
3. **Pipeline envelope on today's routes.** ADMIT, four outcomes mapped to `DialogueDisposition`,
   `Clarify` via the existing confirmation flow, INTERPRET as an adapter over `dialogue.rs`,
   `approach_model.rs`, `freeform_action.rs`, `investigation.rs` and `merchant_bargaining.rs`. The
   `actor.rs:1924` wait moves here only if the combat slice is not ready; otherwise it stays with
   slice 9. Acceptance on the three bad lines needs scripted fakes plus one live Jev check.
4. **Registry with adapter machines.** The first adapter wraps `session_machine.rs`; a generic
   `MachineView` and `MachineIntent` renderer in the wasm client (`gameplay_browser`, `live_tv.rs`);
   `dev machine`; the conformance suite (needs an in-memory owner or scratch Postgres). This is the
   first playable slice wherever this document says so.
5. **Travel machine.** Prerequisite: define a "place pending" state (name and one line committed,
   persons arriving as jobs), because today a move commits only after the place exists
   (`place_move.rs`, `destination_scene`; `place_creation.rs` 1,928 lines; `place.rs` 1,874). Reuses
   the look-ahead code. Needs the storyteller on.
6. **Shop machine** from `merchant_bargaining.rs` (3,511 lines) and `adventure/shop.rs` (1,487), with
   one award effect.
7. **Scene record, hotspots, item inspector.** Promote the scene model ids. Items cannot be located by
   existing code, so the SAM prompt API (`SegmentationPrompt`, now a closed enum) is new work.
8. **Broker facades and the event shape.** Update the charge-site guard list in the same change;
   the boundary script covers both audit functions; settle the coding-style amendment (RES41).
9. **Combat, rest, advancement.** The largest slice: `journey/tactical.rs`, `scene_fight.rs`,
   `enemy_tactics.rs`, `combat_transition.rs`, the tactical snapshot codec v4 and the migration of
   `PendingReaction` to `PendingResolution`, the `df-render` battle code and `live_battlefield.rs`.
   `resume_fight_reactions` is fixed here if not in slice 3. Spells wait for their rules.
10. **Table craft and the remaining machines.** Section K features (brief and foreshadow, voice cards,
    party strip, acknowledgement ladder), adventure end (K.21), dialogue, explore and inventory
    machines. At the stabilisation gate, slots move to typed state in one schema bump.

**L.7 The riskiest assumption and the cheapest tests.**

- **Riskiest.** That per-audience projection with all waiting moved off the owner fits inside one
  owner thread and one whole-checkpoint commit per input. The design budgets projection under 5 ms
  and the rules-only path under 400 ms at p95, and today's projection runs on the owner with file
  writes, deep clones and a poll, while each commit also waits on Postgres inside the owner's reduce
  time.
- **Cheapest test (free, read-only, not yet run).** Start with `DF_FAKE_PROVIDERS=on` and
  `DF_GAMEPLAY_DEV_CONTROL=on`, create six heroes with `dev create` (not `dev play --heroes 6`, whose
  species default can fail on Dragonborn: "creation group draconic-ancestry has no valid default",
  runbook, `dev_cli/play.rs`), attach a TV and five phones with `df-tools/examples/load_client.rs`
  (and `deploy/scripts/load/run_load.py` for fake-provider tables), then read `dev latency` for the
  `View` queue and reduce p95. Repeat in the opening and dialogue phase (memoised) and in combat (not
  memoised). The fake profile switches plates off, so the number is a lower bound.
- **Second riskiest (needs spend approval and new code).** That items can be located in a generated
  plate. No existing command segments a stored image with free noun phrases, because
  `SegmentationPrompt` is a closed enum, so this needs a small script or code addition that calls the
  fal SAM endpoint over about 20 stored plates, at about $0.01 a request (about $0.20;
  `sam-3-1-research.md`). A found rate under about 80 per cent means item hotspots need their own
  pictures from the start.

**L.8 What the comparison does not settle.** How long the slices take, because the current costs are
not measured yet; whether the design's budgets are reachable on the Codex lane; whether the rules
owner will take the five-foot cell additions the layout fields need (D.6); and the parts the
fact-check did not reach (the dice draw point, `perceive`, the in-memory side stores other than the
three checked, the server side of the hero creator and lobby, and the `df-world` time claims).
Questions 71 to 86 hold the decisions that only the owner can make.

### M. Round 3 amendments

Round 3 used four critics: consistency of the merged text (K), conformance with the project's own
plans (P), a rules-lawyer review of the 2024 claims (D), and a fact-check of the comparison in
section L (F; section L was rewritten from its report). The consistency fixes (K) were applied in
place and are summarised in M.3. The conformance (P) and rules (D) findings changed the design, and
they are recorded as RES35 to RES52 and the register in M.2.

**M.1 Conformance resolutions (P).**

- **RES35. No hidden fact enters a prompt (P1; replaces `dm_context`).** An NPC's stance toward a
  hidden fact reaches the model only as a typed `SpeechActPlan` stance (refuse, deflect,
  deceive-approved) with permitted claim ids. Secret ids and text never enter any prompt. The leak
  check is server-side: every grounded clause carries a permitted claim id, and an id outside the
  listener's projection rejects the envelope. A model critic is supplementary evidence only. The
  `dm_context` block and the model-reported "which secret ids a clause revealed" check of H.6 and
  H.11 are withdrawn.
- **RES36. Private player text is not archived in telemetry (P2).** Routine records carry an opaque
  scoped evidence reference and a length class. A payload specimen exists only in the existing
  authorised diagnostic evidence destination, with a content class, expiry, redaction and deletion
  handling. The declined-lines store of K.22 is withdrawn: `declines top` counts decline codes only.
- **RES37. Reach is server-side (P3, D22).** The tactical strip and the choice-list reasons render
  only the server's `Preview` and `TacticalView` reach sets. `reachable_from` in `df-render` is
  deleted or reduced to drawing a supplied path before the combat machine is accepted, and creation
  logic leaves `hero_creator.rs` before the creation machine is accepted. Spell and area targeting
  uses the flat tactical strip and never a plate pixel; the grid's area geometry (cones, lines,
  spheres on five-foot cells) is disclosed in the rules plan's departures list or selected as an R17
  tactical convention (question 56), and "attack a space" is offered against unseen targets.
- **RES38. Hosted admission and failure (P4, P5, P26, P27, P28).** In hosted builds every resource
  intent, including a plate tick job, is created in a durable admission transaction that checks the
  entitlement revision and the hierarchical spend reservation, is journal-confirmed before
  `claim_dispatch`, and is sent through the egress dispatcher. A refusal yields the prepared fallback
  and never a refused player input. RES32's "ticks never commit" means they never commit the table
  checkpoint; the job row is durable. H.4 gains hosted rows: owner-fence or lease loss (stop publishing
  and dispatching, close streams, resync on a new fence); entitlement Grace, ReadOnly and Restricted
  (finish an admitted pending resolution, then a read-only safe checkpoint, with prepared fallbacks for
  text and speech); supplier circuit open (prepared or silent fallback, liability retained); journal
  ambiguity (pending or unknown, paid admissions closed). A request key adds tenant and campaign
  scope and the `SessionRevision`; a provider refusal is a distinct outcome and is never retried;
  retry only after a known not-sent or known-failed state; an ambiguous dispatch ends `Unknown` with
  liability held, not `failed`. A plate is dropped only before dispatch. Machine slots obey the
  4,096-byte checkpoint text limit, and the service ceilings of `service-operations.md` (text 20 a
  minute per principal, snapshot 1 MiB, message 256 KiB, inbox 256 items and 2 MiB, commit p95 over
  150 ms is an alert) replace the C.1 claim that the rate numbers are unset; the C.1 rates are abuse
  control and are not described as cost control. Every new persisted record names its retention class
  and returns `Redacted` or `Unavailable` after suppression, recaps and replays never rebuild removed
  private text, and non-SRD catalog items and shops need a current reviewed `RightsGrant` at
  admission and dispatch.
- **RES39. More named model calls (P6).** H.5's exceptions also include NPC appraisal (the EMO-01
  Jev call that decides Influence willingness), NPC profile creation, the fact backfill decision, the
  item significance decision, exit and place judgement (`judge_exit`, `place_creation`) and
  appearance classification. Each runs as a request before CHECK; its schema-validated result enters
  as an input, and CHECK and DECIDE only read it.
- **RES40. Wire contract (P7, P8).** Public decline codes map to the closed `RejectionCode` set of
  `rpc-api.md`: `not_your_turn` to `WrongTurn`; `window_closed` and `stock_changed` to `StaleOffer`;
  `intent_invalid` to `InvalidSelection`; `resource_spent` and `economy_spent` to `ResourceMissing`;
  `time_locked`, `pending_open`, `pvp_off` and `consent_declined` to `Paused` or
  `CapabilityUnavailable` as applicable. `table_busy` and `rate_limited` are `RESOURCE_EXHAUSTED`
  failures. The remaining codes stay internal reason keys until the contract wave admits them.
  `MachineIntent.payload` is a oneof of the `InputSpec` value kinds (choice ids, entity ids,
  destination, bounded text, validated number), `MachineView.offers[]` is `ActionOffer`, and the view
  carries `SnapshotStamp` and `SessionRevision`. Field numbers come from the field ledger.
- **RES41. Events and logging (P9, P10, P14).** The 40-event figure is a measured budget that raises
  an alert and never drops an event; defined high-frequency aggregation uses a named window with
  counts. New events are typed records through `df-observe`, each also rendered as one `AREA verb
  key=value` line from the same record id. `coding-style.md` §8.3 and §8.7 require `record_audit_line`,
  so the rule in RES33 needs the owner's amendment of that style document (a devlog entry before the
  event-builder slice), and the boundary script blocks new free-text audit lines and any parsing of
  audit text. Timing is measured from the logs (`AGENTS.md`): add `line.opened` and `line.closed`,
  `plate.swapped{from, to, reason, prior_on_screen_ms}`, `scene.changed{reason}`, `figure.entered` and
  `figure.exited`, and `clip.started` and `clip.ended`, each with a `why`, and the setting
  `plate_min_on_screen_ms` (at least the reading time of the lines shown over it).
- **RES42. Typed revisions (P11).** Every job, result, request key and view stamp carries the
  `SessionRevision` (`RecoveryEpoch`, sequence) as well as `(scene_id, scene_epoch)`; a result from an
  older `RecoveryEpoch` is stored as `late`. In code `scene_epoch` is named `scene_generation`, to
  keep it apart from `RecoveryEpoch` and the presentation epoch.
- **RES43. Hosted CLI and export (P12).** In hosted builds every dev verb that can read hidden or
  private content, and `table export` and `table import`, needs operator permission with a reason, an
  expiry and an audit record. Defaults are redacted. Hosted export is an `ExportGrant`. `table import`
  is local-only, runs `validate_build` and admission, and cannot mint awards or rights. Question 70's
  draft default is fixed.
- **RES44. Acceptance (P13, P33).** Each slice is accepted only by an ADR 0005 evaluator operating TV
  and phone in a real browser at phone and TV sizes (reconnect, asset failure, audio observation),
  with the build and commit identity and a devlog link. Scripted `sim` and `soak` never count as
  playtests or approval. The default for `preflight` and canaries is fakes or prepared-only; a
  real-provider run is a scoped, owner-approved task and not a recurring hidden canary. One
  implementing worker owns each scoped item.
- **RES45. Plate rules keep earlier owner decisions (P15, P16).** D16 amends only the REF-068 cadence.
  A visible change (a weapon drawn, an arrival) still requests a plate at once and resets the tick,
  and swaps follow the minimum on-screen rule. Every plate depicts every seated hero from that hero's
  sheet, and the photo check counts hero figures and regenerates on a wrong count (REF-065). Rotation
  (K.13) applies only to reference slots, never to who appears. The server commits ahead and never
  waits on a display: dwell values are timeline anchors the conductor applies, hero input opens
  immediately, and only server-run enemy turns may be spaced, by a server timer from anchors and
  never from client acknowledgements. Catch-up may shorten non-outcome lines and never the die or its
  result card.
- **RES46. Words are never matched (P17, P18).** The K.14 sentence "text that matches a legal choice
  executes that choice's id with no model" is withdrawn: typed text always goes to INTERPRET with the
  seat's legal choices as the closed enumeration, and a tap is the only no-model path (`coding-style.md`
  §8.1). K.8 is restated: physical things are introduced only as grounded clause entity slots with
  set-piece ids, flavour clauses are noncanonical and cannot promise affordances, and a model critic
  is supplementary.
- **RES47. Social rules are the 2024 ones (P20, D18).** Influence follows willing, unwilling or
  hesitant. A willing request needs no check, an unwilling one is refused, and a hesitant one is a
  check against the higher of DC 15 or the creature's Intelligence score, with no Insight contest
  (the G.2 and C.3 wording is corrected). Willingness is the EMO-01 reviewed automatic case, logged,
  with host override. A failed urging cannot be repeated for 24 hours unless something changes; the
  decline is `same_urging`, keyed by target and request. Haggling is one hesitant check per request,
  and price movement from a check is an R17 optional module that stays off until the coordinator
  records it. A `done` NPC maps to `applied` with a structured line ("Dain turns away") and not to
  silence.
- **RES48. Rulings are keyed, not retrieved (P21, D25).** A `RulingResponse` is limited to existing
  mechanics: a check (ability, skill, DC tier from the source table), advantage or disadvantage, an
  existing condition, and damage by severity tier where the source pins one. Automatic reuse needs
  equality of the typed source-question key and scope, with the affordance id in the key; a retrieval
  or similarity hit is only a suggestion with a host one-tap; the host can revoke a stored ruling.
  A reuse that grants advantage or automatic success is re-confirmed once per scene, and the accept
  button shows the tier name and the source. Question 68's default becomes "keyed reuse automatic,
  anything else a suggestion".
- **RES49. Consent and preferences (P22, P32, P29).** A spotlight offer drawn from a hero's hooks
  needs that seat's `SpotlightPreference` (off by default), otherwise it is a generic beat with no
  disclosure. The pacing vote is a private phone card, and the TV shows only an enabled aggregate
  (at least two votes). `evening_minutes` waits on the owner's confirmation of the engagement-model
  question. Haptics, chimes and stingers obey `SetPreferences` and the fatigue budgets. Voice
  transcripts are final-only and send on an explicit tap (the two-second window is cancel-only and
  scales with `reading_pace`). Each seat has an input and audio lease, a blocked-audio unlock on the
  TV, and a private read-aloud failure falls back to text. A second device binds by `BindClient`
  takeover approved on the launcher or the host phone, and host choices appear on the host panel and
  never on the TV.
- **RES50. Settings (P23, P28, P35).** Each J.4 setting names its owner: launcher Settings (host) or a
  per-seat preference through `SetPreferences`. Each lives in the central configuration file with a
  commented default, is validated at load with a default fallback and an audit line, and is readable
  through `dungeonflux config`. Every broker request first asks `Gates`. Names use `_s` and `_ms`
  units (`scene_build_budget_secs` is `scene_build_budget_s` in code). "Backdrop" is not used: the
  words are plate, establishing card and place card.
- **RES51. Crates (P24, P25).** The machine contract, registry, `step` and the staged transitions
  belong in `df-engine`; the owner queue, durable intents and timers in `df-session`; resource
  facades through `df-media` and `df-provider-api`; audience projection in `df-api` (or
  `df-engine::read_model`); and the browser in `df-web`, `df-player`, `df-display` and `df-ui`, which
  may not depend on `df-model` or the rules. `df-tools` and the current `df-display` placement are
  transitional. D7 does not apply to the wire, the field ledger or the persistence migrations of
  RES17. Each slice names its roadmap gate and plan-database item.
- **RES52. Small conformance rules (P19, P30, P31, P34).** The G traces are candidate narratives and
  not goldens until reviewed against the pinned source; G.5 uses the wizard's own Shield (an
  ally cannot Uncanny Dodge for a wizard), and G.3 keeps Fire Bolt's attack roll against the object's
  AC with no slot. Dialogue keeps the backfill decision and logs it, and K.1's "never an invented
  fact" is scoped to character and world recall. `voice_card` lives in the dossier and the character
  record holds a reference. Combat bindings come from authored or template entries admitted through
  content admission, and a model never picks a profile. K.21 offers a callback when a valid candidate
  exists and does not guarantee one. Seeds may be retired. Whether a TV minimap counts as the grid
  that `implementation-roadmap.md` removes from the TV is a question for the owner (question 86), and
  question 73 is closed by the look-ahead decisions REF-066 and REF-067.

**M.2 Rules-lawyer resolutions (D), as one register.**

The rules-lawyer's main finding is that the four outcomes and the reaction-window model cover other
seats' reactions, but not the roller's own choices, the DM's judgement points, or time and aftermath.
This register names, for each, what the engine does and which carrier owns it. Details marked (V) were
recalled and not confirmed, and the source gate decides them. Each row is a fixture candidate.

| Point | The engine does | Carrier | Test case |
|---|---|---|---|
| D1 death and revival | A creature at 0 HP is dying; a dead character is held with its record. The creation lane opens on a host "replace" or when no revival is possible. Defeat is all heroes dead or an authored capture state, never "all down" while saves pend | rules owner; encounter outcome | Three heroes at 0 HP with saves pending and the goblins withdraw: no `defeat`. A cleric with a slot and the component can still revive a hero 8 rounds after death (V: one minute) |
| D2 DM decision points | A register of each point and its carrier (below) | affordance data, derived facts, encounter data, EMO-01, `pending{choice}`, `RulingRequest` | A fixture is complete only if every choice it triggers names a carrier |
| D3 stakes and failure | Affordances carry `on_failure` chosen from existing mechanics (a condition, hazard damage, time lost, alert, no retry without change); the roll card shows the stakes first; an affordance with no DC resolves without a roll | authored affordance data | A failed climb costs time or a slip and not only "you fail" |
| D4 the roller's own choices | An Outcome is a sequence of roll stages; after each, the rules owner lists the roller's options as `pending{choice}`; a reroll draws a new `draw_id` and keeps the old one logged; C.5 reads "no undo and no reroll except one a source rule grants" | rules owner | A miss by 1 plus Heroic Inspiration: the new die replaces the old, Inspiration is spent, a second miss offers nothing, one HP movement commits |
| D5 Counterspell and Shield | Phase one reserves the slot and draws nothing for effects not yet reached; phase two draws and spends after the window; a countered spell releases the reservation and keeps the action spent; nesting is bounded by eligible reactors | rules owner; window handler | A countered Fireball: slot count unchanged, action spent, no damage draw consumed |
| D6 ordering | `pending{choice: order}` to the controller of the creature whose turn it is; any engine-chosen order is a logged reviewed case | window handler | Ongoing damage plus a save-ends condition: the player chooses the order |
| D7 Ready | The trigger binds to ids (creature, set piece, or an event kind the rules owner sees); an unbound trigger is `pending{ruling}` with a one-tap fired or not; until built it declines `not_supported_yet` | rules owner; ruling flow | A readied leveled spell whose concentration breaks before the trigger: the slot stays spent and nothing fires |
| D8 initiative | Surprise is derived from `WitnessRecord` and gives disadvantage on the initiative roll, not a skipped turn (V); player ties are `pending{choice}`; joiners roll when they arrive and slot in at their count; Delay is an R17 convention or dropped | rules owner; encounter owner | Reinforcements arrive in round 2 and roll 12: they act this round between counts 15 and 8 |
| D9 legendary and lair | `Interlude{creature, source}` after each turn with an engine responder; a count-20 lair entry; Legendary Resistance is an engine-responder window | combat machine | A dragon with 3 legendary actions takes one after each of three hero turns, none after a fourth, and the pool is full at its own turn (Large creatures wait on footprint support) |
| D10 summons and mounts | A cast entry has `initiative_rule: own or shares{owner, after}` from the effect; an uncommanded default comes from the rules owner | cast entry | An ally acts after its summoner each round and takes Dodge when not commanded |
| D11 forms | Two kinds: `overlay` (Wild Shape keeps real HP and adds temporary HP, V) and `replace` (Polymorph replaces HP and carries excess damage back) | form effect | A polymorphed hero at 0 beast HP takes 9 excess: the original form loses 9 HP |
| D12 concentration | Save is Constitution against max(10, half the damage), capped at 30 (V), one per damage source; it ends on incapacitation or death; replacing it needs a confirm | rules owner | 20 damage taking a hero to 0 ends it with no save; 9 then 30 in one round is two saves at DC 10 and DC 15 |
| D13 aftermath | An `Aftermath` sub-state steps in 6 seconds while any hero is dying; rest and travel are refused during it | combat exit | The last goblin falls with a hero on one failed save: explore does not begin |
| D14 rest | Rest states: planning, resting, interrupted or complete, post-rest choices (hit dice loop, spell preparation, recovery features), done; one long-rest benefit per 24 hours, no decline | rest machine | A second long rest 10 hours after the first gives no benefit and is not declined |
| D15 sleepers | The watch plan passes `awake[]` to combat entry; sleepers enter unconscious and surprised; a noise-wake question is a ruling | rest and combat | Four heroes, two on watch, a night ambush: two unconscious, two active |
| D16 combat time | Each round boundary advances `world_time` by 6 seconds through the rules owner; `time_locked` applies to other advances | rules owner | Bless cast in round 1 of a 3-round fight is expired after a 10-minute search |
| D17 hiding | A hidden hero is drawn with a hidden chip on the party's TV; hiddenness filters only the enemy view; a passive score is a static compare | projection | A rogue hides behind a pillar and the TV still shows the rogue |
| D19 kit and proficiency | A missing tool or language declines; missing proficiency proceeds without the bonus | requirements | No forgery kit: declined; a kit without proficiency: allowed, no bonus |
| D20 recall DC | Authored facts carry a DC; a generated fact is plainly known or has no recall DC; the retry key is `(actor, affordance id, method)` and reopens on any committed change to the scene, the affordance or the equipment | content admission | A failed desk search then a bookshelf search is a new task |
| D21 Help and group checks | Outside combat a Help is consumed by the named task's next check or ends with the scene; a group check is called by the DM or a scene trigger and resolves in one commit with half rounded up | rules owner | Three heroes, two succeed: success; four heroes, two suffice |
| D23 light | `obscured` is derived from `light` and effects; visibility is computed per observer; the plate defaults to shared vision | layout | A dwarf and a human in a dark cave see different things on their own seats |
| D24 falling | The fall rule joins the rules-owner layout change | rules owner | A Shove off a 20-foot ledge deals 2d6 and lands prone |
| D26 action mapping | Intents map by rules entry id (Shove and Grapple are Unarmed Strike options inside Attack; Jump is movement) | action registry | "I grab him" resolves as a grapple; Extra Attack can replace one attack |
| D27 spell procedures | `casting_restricted` decline; ritual time routes through the time effect | rules owner | "Healing Word then Fireball" declines the second step |
| D28 Heroic Inspiration | Suggested to the host for any heroic, dramatic or funny moment (not tied to flaws); held one at a time | rules owner | A hero already holding it earns another: not stacked |
| D29 inventory | The rest machine can attune (three items); encumbrance is an R17 decision | rest machine | A fourth attunement is declined naming the three |
| D30 level-up | Offers queue and apply one at a time; HP roll or average is `pending{choice}` | advancement | Two awards before choices apply as two ordered level-ups |
| D31 plans across machines | A plan's lifetime outside combat is the scene generation; a hand-off inside a plan is allowed when the next step names its machine | pipeline | "Pay for a room and rest" ends with coin spent once and a rest started, or neither |
| D32 time cost | Affordances carry an authored `time_cost`; an unauthored one is a `RulingRequest` | affordance data | "Search the whole cellar" with no cost produces a pending ruling |

The register of D2, by carrier. *Authored affordance data*: whether a roll is needed, the ability and
skill, the DC tier, advantage for circumstances, the consequence of failure, the time cost, and the
cover grade at a corner. *Derived facts*: surprise (from witnesses) and whether a sleeper hears.
*Encounter data*: whether a monster dies at 0 HP. *EMO-01 with host override*: willingness and
attitude. *`pending{choice}` to players*: initiative ties among heroes, effect ordering, and whether
a Help is taken. *`RulingRequest`*: everything else, including a readied trigger that did or did not
fire, a task's duration with no authored cost, and improvised damage.

**M.3 Consistency fixes (K), applied in place.** The audit's blockers were mine: RES22's "16" was the
plate queue's capacity and is now the per-seat bound of 3; a machine waiting on a pending is
`Waiting{pending_id}` everywhere (C.4, F.1 and G.1 corrected); `pending` kinds include `roll` in RES16;
the owner serves three capped lanes (H.2); machine scope is stated for every machine (RES2, RES28,
H.17); section F's opening uses the RES28 list; DECIDE and `step` each add effects and APPLY commits
the union (C.0, RES21); a spell-target tap is a proposal the engine converts and the rules owner
validates (RES1, row R10); a scene is playable only when its description, cast, set and layout are
stored (K.2) and waits at most `scene_build_budget_secs` (F.3); and RES31 now lets explore and travel
register as adapters over authored beats while `No` capabilities decline. The audit's other findings
were applied as listed in the patch log of this round: the banner rule of H.15 follows K.3; the focus
rule of D.1 follows K.10; H.5 and RES4 describe DECIDE's brokered requests; H.1 lets one narration row
cover a batch; the settings table gained its missing entries; revision names are defined in RES34;
section E's rows R09 and R10 follow `rules-coverage.md`; section L's slice references are corrected
and a slice 10 covers section K; the code counts are corrected; and the round-2 ledger is I.2.
Facts I could not find in the code and that this document uses as names only: `AwardId`,
`EncounterOutcome`, `RequestTimeAdvance`, `NarrativeDirector::evaluate`, `RuleFixture`,
`TimeAdvance`, `legal_choices`, `SpatialQuery` and `TriggerWindow`.

**M.4 Ledger for round 3.**

| Finding | Where handled |
|---|---|
| P1, P2 | RES35, RES36 |
| P3 | RES37 |
| P4, P5, P26, P27, P28 | RES38 |
| P6 | RES39 |
| P7, P8 | RES40 |
| P9, P10, P14 | RES41 |
| P11 | RES42 |
| P12 | RES43 |
| P13, P33 | RES44 |
| P15, P16 | RES45 |
| P17, P18 | RES46 |
| P20 with D18 | RES47 |
| P21 with D25 | RES48 |
| P22, P29, P32 | RES49 |
| P23, P35 | RES50 |
| P24, P25 | RES51 |
| P19, P30, P31, P34 | RES52 |
| D1 to D32 | M.2 register |
| K1 to K10 | M.3 and the inline edits it lists |
| K11 to K35 | M.3 and I.2 |
| F1 to F28 | section L rewritten; F7 in RES30 and L.6 slice 1; F9 in L.6 slice 0b; F5 in L.2 and RES26 |

### O. The `df-table` crate plan

This is the plan for the first crate of the from-scratch engine, written after the old gameplay code
was removed from `df-tools` on branch `gameplay-engine-v2` (commit `14a2f715`). It turns the design
above into modules, types, seams and tests. Its two drivers, from the owner, are **flexibility**
(a refinement or a new machine, action, setting or resource should be a data entry or one module,
and the compiler or the conformance suite should say what was forgotten) and **observability**
(every decision is a typed event that explains itself, nothing private can reach a log by accident,
and every piece of state can be inspected). Nothing here is built yet.

**O.1 State of the branch, and what the removal changes in this document.**

- The old server (`df-tools/src/gameplay`), the old wasm client, the `dev` CLI, the old client
  modules and the fake profile are gone from the tree. `df-tools` is the S00 transport fixture
  only. Git history keeps the old code, and the plan reads it as reference, not as a base.
- Section L describes the old implementation as of commit `2b5bc721`. Its "what stays" (L.4) now
  means "what to read from history": the `Hub` (`resources.rs`), the cost book, the timing ledger,
  the plate director (`plates.rs`), the stage conductor, the dice draw point and the table
  settings store were all inside the deleted tree, so each is rebuilt on purpose, informed by the
  old code and the corrected facts of L.2, and none is an inherited dependency. The strangler
  delivery order of L.6 no longer applies; O.9 replaces it.
- The libraries remain: `df-model`, `df-rules`, `df-session`, `df-persistence`, `df-imagery`,
  `df-ai`, `df-media`, `df-knowledge`, `df-interaction`, `df-narrative`, `df-encounter`,
  `df-combat`, `df-world`, `df-engine`, `df-api`, `df-protocol`, `df-observe`, `df-player`,
  `df-display`, `df-web`, `df-ui`, `df-render` and the rest, in the sizes measured on
  2026-11-02 (for example `df-engine` 7.2k lines, `df-session` 4.8k, `df-api` 0.8k, `df-web` 1.4k).
  They are skeletons more often than products, and the plan treats each as an owner to extend.

**O.2 Where the crate sits.**

- **Name and kind.** `df-table`, a native-only, pure, synchronous library. It does no I/O, spawns
  nothing, reads no clock and draws no dice. A table is the shared state of one running game (D5), and
  the crate holds the rules of routing input through it.
- **Why a new crate and not `df-engine`.** `df-engine` stages candidate checkpoints from directors
  and rules (`decide`, `apply`, `read_model`) and is consumed by `df-session`. The table layer is a
  new subsystem boundary (machines, the DM pipeline, the owner queue, projection and declines), and
  keeping it apart lets it be built and tested without touching the staging code. The cost is a new
  edge in `subsystem-architecture.md`, which the owner approves in question 87.
- **Dependencies it may have.** `df-types` (ids, `SessionRevision`), `df-model` (the checkpoint,
  `PendingResolution`, `KnowledgeGrant`, `IntentDisposition`), and `df-observe` types only if the
  event sink needs them. It does **not** depend on `df-protocol` (wire types are mapped at the
  boundary), on `df-persistence`, on any provider crate, or on a browser crate. `df-rules` and the
  other owners reach it through ports (O.4), not imports, so the crate can be tested with fakes and
  the owners can change independently.
- **What depends on it.** A native async host crate (O.8) that runs the owner actor, and `df-api`
  for projection types. The browser crates never depend on it.
- **An architecture test** in the crate reads `Cargo.toml` and the `use` lines and fails on a
  forbidden dependency, a `println!`, a wall-clock read (`Instant::now`, `SystemTime::now`), a
  `tokio::spawn`, a `std::fs` call or an unbounded channel.

**O.3 Module map.** One responsibility per module, files aimed under 800 lines of non-test code,
tests in `<module>_tests.rs`, and each module has a `//!` doc that says what it owns, what it never
does and how it fails.

| Module | Owns | Implements |
|---|---|---|
| `outcome` | `Outcome` (applied, needs clarification, declined, pending), `Decline`, `DeclineCode`, `PendingKind`, the mapping from a decline to a public class | RES16, RES26, C.6, RES40 |
| `envelope` | `InputEnvelope`, `InputKind` (tap, submit text, say, private message, safety card, engine proposal, timer, resource result, pending answer), seat and utterance ids, `ViewStamp` | C.1, RES21 |
| `owner_queue` | the three capped lanes, per-seat round-robin, per-seat bounds, a token bucket, duplicate detection, the serve order and `turn_seq` | RES22, H.2, C.1 |
| `machine` | `MachineId`, `Scope`, `MachineSpec`, `StepResult`, `HandOff`, the `Machine` trait | RES27, RES28, RES20 |
| `registry` | registration, startup validation (unique ids, known hand-off targets, scope rules, required capabilities), enable flags | D24, RES28, Q46 |
| `pipeline` | the seven stages as pure functions over ports; the four outcomes; the failure fallbacks | C, RES15, C.7 |
| `pending` | pending resolutions held in table state, two-phase apply, windows, answers | RES26, C.4, D5 to D7 of M.2 |
| `effects` | `TableEffect` (a versioned closed enum), the applier over table state, the `carry` list | RES18, RES32, RES3 |
| `table_state` | the composition view of the owners' state (checkpoint, world time, scenes, seats, cast), `scenes[]`, `scene_epoch` | D.1 to D.7, RES25 |
| `projection` | audience projections, `ProjectionStatus`, the never-empty types, the structured readout, public-versus-private filtering | RES30, H.15, K.5 |
| `ports` | the seams: rules owner, interpreter, narrator, broker, clock, dice, knowledge view, event sink | O.4 |
| `event` | `TableEvent`, event classes, the private-data types, the audit-line rendering | O.6 |
| `inspect` | `Inspect` reports and the verb set behind the CLI | H.16, O.6 |
| `config` | typed settings with defaults, ranges and owners | RES50, J.4 |
| `testkit` (feature) | `ManualClock`, `ScriptedBroker`, `RecordingSink`, scripted interpreter and rules owner, the golden-session runner, the conformance suite | H.17 |

**O.4 The seams.** Every outward dependency is a port owned by this crate and has two real
implementations, a production adapter and a scripted fake (which satisfies the style rule on
traits). The pipeline and the machines see only the ports.

| Port | Question it answers | Production adapter (outside the crate) | Fake in `testkit` |
|---|---|---|---|
| `RulesOwner` | legal choices for an actor, `prepare` and `resolve` with explicit draws, spatial query, rest and time costs | an adapter over `df-rules`, `df-world`, `df-combat` | scripted table of answers |
| `Interpreter` | free text to an `IntentDisposition` and an `ActionPlan` (closed schema) | a Jev call behind the broker | scripted lines |
| `Narrator` | a committed outcome to a speech envelope (qualified) | the storyteller behind the broker | scripted, and a failing one |
| `Broker` | request a resource, learn its readiness, never wait | the rebuilt `Hub` | scripted readiness and failures |
| `Clock` | monotonic now, in milliseconds | system clock | `ManualClock` |
| `Dice` | the explicit draws for a decision | `df-session` supplies them | queued draws |
| `KnowledgeView` | what an audience may know | `df-knowledge` | fixed sets with sentinel secrets |
| `EventSink` | record a `TableEvent` | the audit line plus OTEL sink in the host | `RecordingSink` |

**O.5 Flexibility: how each kind of change is made.** The rule is closed inside, open across
(`Principles`, D24): a closed enum inside one machine so the compiler proves every case, and a
registry across machines so adding one edits nothing else.

| To add or change | You edit | What tells you what you forgot |
|---|---|---|
| a machine | one module implementing `Machine` and one `registry` line | startup validation and the conformance suite: hand-off targets, a projection that is never empty, events for every transition and failure, an inspect report, a saved-state schema, an entry and an exit that return the table intact |
| an action the player can take | one entry in the action registry, keyed to a rules entry id, with its requirements and decline text | the registry test: every action has a rules id, a label key and a decline path; an action without rules is `not_supported_yet` automatically |
| a decline code | one `DeclineCode` variant | an exhaustive `match` in the public-class mapping, the key list and the reason-text table, so it cannot compile half-added |
| a table effect | a new versioned `TableEffect` variant and its applier arm | the applier is an exhaustive match, and a golden session fails until the new effect has a fixture; effects are added and never changed (Q44) |
| a setting | one `SettingSpec` (name, owner, default, range, unit, cost effect) | load-time validation falls back to the default and records an event; the settings table in J.4 is generated from the specs so it cannot drift |
| a resource kind or purpose | one `PurposeSpec` (deadline, size rule, retry policy, priority class, price) | the broker conformance test: every purpose has a deadline, a class and a fallback |
| a new owner (a better rules engine, a different narrator) | a new adapter for the port | the same contract tests run against the adapter and the fake |
| per-table behaviour | a `MachineFlags` entry and the settings | the registry reports which machines are enabled and why |
| saved state | a schema version bump plus one migration step | refusal to load an unmigratable record with `load.refused`, never a silent best effort |

Flexibility rules that the tests enforce: no machine imports another machine; shared facts go
through `TableEffect`s; a machine never reads the clock, dice or the network; a feature that is
half built declines `not_supported_yet` rather than failing; and every tunable number is a named
setting with its default stated once.

**O.6 Observability: how the crate explains itself.**

- **Typed events, one shape.** `TableEvent` is a closed enum, and each variant carries only typed
  fields: ids, enums, counts, millisecond durations and hashes. A `TableEvent` has a class
  (A: always on and never sampled; B: verbose detail), the table and scene, the machine and state, the
  `OperationId`, the `turn_seq`, the `SessionRevision`, a `cause` for a failure and `ms` timings.
  Adding an event is adding a variant, and the exhaustive rendering functions force its audit-line
  form and its OTEL attributes at compile time.
- **Two renderings from one record.** Each event renders as one `AREA verb key=value` line (the
  coding-style form) and as OTEL span or log attributes through the host's sink, from the same
  record id, so the two never disagree. The amendment of the coding-style document that RES41
  asks for is a prerequisite for the first host slice.
- **Privacy by type.** Player text, narration text, secrets and prompts have their own types
  (`PlayerText`, `Narration`, `Secret<T>`) that do not implement `Display`, whose `Debug` prints
  only a length class, and that cannot be put into a `TableEvent` field. Events hold an opaque scoped
  evidence reference instead (RES36). A test tries to format each such type into an event and fails
  if it compiles.
- **Every stage is measured.** Each pipeline stage emits `step.finished{step, machine, ms}` and its
  outcome, and a stage that fails emits a failure once, at the boundary that handles it, with a
  closed cause. Progress checkpoints (`input.received`, `turn.applied`, `projection.delivered`,
  `client.presented`) are events too, so the watchdog and the preflight (H.18) read the same record.
- **Correlation by construction.** The `OperationId`, `turn_seq` and `SessionRevision` travel in a
  `Correlation` value that every port call receives, so a broker request, a narration job and a
  projection can be joined back to the input that caused them (H.10).
- **Inspect reports.** Every module with state implements `Inspect`, returning a typed report with
  an audience (operator, host, seat), so hidden content is redacted by the report itself and not by
  the caller. The reports are the data behind the nine CLI verbs (RES33): `list`, `show`, `history`,
  `config`, `dump`, `force`, `input`, `viable`, `describe`. The host serves them; the crate defines
  them, so a machine without a report fails the conformance suite.
- **Replay and golden sessions.** A `TurnRecord` (input, draws, outcome, effects, state hash) is
  produced for every committed input. The golden-session runner replays a script of inputs against
  scripted ports and compares records and state hashes, which is how the section G traces and the M.2
  register become tests. Replay is a diagnostic and test tool, not the recovery path (RES19).
- **Metrics.** Counters and histograms are declared in one table (`step.ms`, queue depth and oldest
  age per lane, declines by code, window answer latency, ruling wait, plate lateness) and exposed
  through the `EventSink`, so the first playable slice can measure what H.5 only proposes.

**O.7 Typing and invariants worth stating up front.**

- `Outcome` has exactly four shapes, and `Pending` carries a `PendingKind` of choice, roll, reaction or
  ruling. `Decline` carries a code, a reason, the missing piece if any, and alternatives, and a
  decline can only be built with a reason (the constructor requires non-empty text).
- `NonEmptyText` is the only type a projection's narration slot accepts, and
  `Narration::or_readout(model, readout)` returns the readout when the model text is blank, so the
  never-empty rule of H.15 is unrepresentable to break.
- Counts that grow with play are bounded and name their eviction order: remembered operation ids
  (`MAXIMUM_REMEMBERED_OPERATIONS`), seats, lane lengths, hand-off stack depth (3), scenes. An unbounded
  collection is a review failure (`coding-style.md` §7).
- Time is an input everywhere (`now_ms` or a `Clock`), and nothing in the crate calls the system
  clock. Durations are named in `_ms` or `_s`.
- Every `unwrap`, `expect` and index on runtime data is absent from production paths. Errors are one typed
  enum per module with variants a caller can branch on.

**O.8 What lives outside the crate.**

- **The async host** (a new crate, `df-table-host`, question 87): the owner actor that drives
  `owner_queue` and `pipeline`, the broker (the rebuilt `Hub` with text and image kinds, queue
  classes, leases and epochs), the plate scheduler and playout buffer, durable intents through
  `df-session`, the dev-control server that exposes the Inspect verbs, and the event sink adapter to
  `df-observe`. It owns every `.await`; `df-table` owns none.
- **Wire and projection types:** `df-api` and `df-protocol` get `MachineView` and `MachineIntent`
  under the field ledger (RES33, RES40), mapped from `df-table` projections at the boundary.
- **Client:** `df-web`, `df-player`, `df-display`, `df-ui` render `MachineView`; they never import
  `df-table`.
- **Composition:** `df-server` wires the host, the stores and the providers. A local game start is
  a `df-server` mode, and the old launch scripts under `development/` are replaced when it exists.

**O.9 Build order for the crate, then the host.** Each step is small enough to review, ends with
tests that fail before and pass after, and is accepted under RES44 when it reaches a user-visible
surface. The old L.6 slices are void because the old code is gone.

1. **C0 Foundations.** `outcome` (with the exhaustive decline mapping), `event` (variants, classes,
   the private types, audit-line rendering), `ports` (traits only, with the `testkit` fakes),
   `config` (the setting specs), and the architecture test. Acceptance: the privacy-by-type test,
   the exhaustive-mapping test, and an event round-trip test.
2. **C1 Owner queue.** `envelope` and `owner_queue`: lanes, round-robin, bounds, token bucket,
   duplicates, serve order. Acceptance: property tests for fairness (no seat starves another), bounds
   (nothing exceeds a cap), duplicates (one operation id runs once) and determinism (same inputs,
   same serve order).
3. **C2 Machines and registry.** `machine`, `registry`, `effects` skeleton and the conformance suite
   with two real machines (lobby and a minimal explore) so the trait has two implementations.
   Acceptance: a deliberately broken machine fails each conformance check.
4. **C3 Pipeline.** The seven stages over scripted ports, four outcomes, the model-failure fallbacks,
   the structured readout, clarification and confirmation, engine inputs. Acceptance: the section G
   traces 1, 2 and 4 as golden sessions, plus the three known-bad lines.
5. **C4 Pending and windows.** `pending` with two-phase apply, held turns, answers by pending id,
   rulings with keyed reuse. Acceptance: the M.2 register rows D4 to D7 as fixtures, and a restart
   test that replays a committed pending without re-drawing.
6. **C5 Projection.** Audience projections, status, never-empty, sentinel-secret and paired-state
   tests. Acceptance: a seat never sees another seat's private fact in any projection.
7. **C6 Table state and scenes.** `table_state`, scene epochs, seats and presence, the hand-off stack.
   Acceptance: the split-party decline, the stale-result rejection, and idle and pause rules.
8. **C7 Inspect and replay.** Reports for every module, the verb set, `TurnRecord` and the replay
   runner. Acceptance: every machine and port has a report, and a recorded session replays to the same
   state hash.
9. **Host H0 to H3** (outside this crate): the owner actor with the queue and pipeline over real
   adapters; the broker; the event sink and CLI; the first playable path (lobby, one story scene, one
   conversation). Then the wire, the client, and the remaining machines (travel, shop, rest, combat,
   advancement) in the order the owner chooses.

**O.10 Risks the plan accepts, and what would change it.**

- A pure crate with ports may feel heavy for the first slice. The cost is paid back by testing every
  machine against scripted ports with no provider, no browser and no database, which the old
  engine could not do. If the ports turn out too coarse, they are split, not removed.
- `df-rules` does not yet offer legal choices, spells or most exploration. The `RulesOwner` fake lets
  the crate be built and tested first, and the adapter declines `not_supported_yet` for what is
  missing (RES31).
- The host is where timing and the owner thread live, so the riskiest assumption of L.7 (projection
  and commit within one owner thread) is tested in the host steps and not in this crate. The crate's
  contribution is that projection and the stages are cheap, bounded and synchronous.

**O.11 Questions for the owner.** 87 (new crate edge and host crate), 88 (which port set is the
minimum for C0), 89 (event class A fields that must never change), 90 (first machines for C2).

**O.12 Blank canvas mode (the first host slice, built).** The owner asked to start with a mode where the
table is empty and the dev CLI spawns elements at run time, so the engine can be iterated on without
recompiling. It is built in `crates/df-table` (the pure model) and `crates/df-table-host` (the host and
the `df-table` binary), and it is the workbench the machines and the pipeline will plug into.

- **What it is.** A table that starts with nothing on it. Elements are placed on a 100 by 56.25 canvas
  and drawn by a live viewer. Every change is an effect that is checked, applied and recorded as a typed
  event, and every refusal is recorded too.
- **Kinds are data.** A kind of element (character, item, set piece, marker) is a JSON file in
  `assets/canvas/kinds/`: its shape and size, its label and colour fields, and its typed fields
  (text, number with limits, flag, colour, choice) with defaults and required flags. Adding a kind or
  changing a field is a file edit and `df-table dev reload`, never a build. A reload that has problems
  reports every one of them at once and leaves the loaded kinds untouched. An element that no longer
  fits a reloaded kind stays on the canvas and is flagged, so fixing a file never loses the scene.
- **How to run it.** `df-table serve` starts the viewer and the developer API on `127.0.0.1:7878`
  (`--port`, `--kinds DIR`, `--moves`, `--quiet`). `df-table dev <command>` talks to it:
  `spawn <kind> <id> [x=N] [y=N] [field=value ...]`, `set <id> field=value ...`, `move`, `despawn`,
  `clear`, `canvas`, `kinds`, `events [after]`, `reload`, `script <file>` (a file of commands, one per
  line, checked in full before anything runs) and `help`. A command that sends several effects applies
  them in order and reports every failure. The debugging viewer is server-rendered SVG that refreshes
  itself with a meta tag and no JavaScript, as the project rules require; it reloads the page each
  second, so the live view for design work is the browser playground (`crates/df-playground`).
- **Observability and safety.** Each effect produces one audit line (`CANVAS spawn element=pip
  kind=character revision=3`) in a bounded log that `events` and the viewer read; moves are recorded
  only with `--moves`, and a refusal, including a malformed or forbidden request, is an event too. The
  API listens on `127.0.0.1` only, checks the Host header on every request, accepts changes only as
  JSON from a same-origin caller (so a web page cannot trigger a reload), caps a request at 64 KiB, and
  every piece of text that reaches the page is escaped.
- **Playground (the display workbench).** `crates/df-playground` is a browser page, Rust compiled to
  WebAssembly, that draws the shared screen and one phone from fixture files in `assets/playground/`,
  served by the same host at `/playground` (`development/playground.sh build`, then `serve`). The page,
  theme and fixtures are plain files that redraw on save, and a rebuilt package reloads the page. The
  screen can show the running canvas live. The table-experience contract is code here, so a fixture that
  breaks it shows the break: the status copy, the caption dwell, and the choice-list rule (at most four
  suggested choices, fixed groups, labels of at most four words).
- **The dev CLI has full access (local builds only).** Hosted builds expose only the operator-safe
  verbs of H.3 and H.16, and driven changes will carry `driven=true` once sessions exist. The host, not
  the page, owns what the playground shows, so the CLI and
  the browser change the same state and see each other's changes within a poll. `df-table dev playground`
  prints it and `playground set fixture=NAME device=both|tv|phone pace=slow|normal|fast status=KEY
  unavailable=BOOL grid=BOOL` changes it; `fixtures`, `fixture show NAME`, `fixture write NAME FILE`
  (the text is checked as a fixture first, with the line and column of a mistake) and `fixture remove
  NAME` manage the files. `scene caption TEXT [speaker=NAME]`, `scene caption-clear`, `scene party
  add|set|remove|clear`, `scene choices suggested|group|unavailable|clear` and `scene reset` change the
  caption, the party strip and the phone's choices live, as an overlay on the chosen fixture (the file is
  not edited; choosing another fixture drops the overlay). A change that breaks the choice-list rule is
  accepted and shown as a warning, only size limits are refused. A `script` may mix canvas, playground
  and scene commands. Every change is an audit
  line (`PLAYGROUND set device=tv revision=5`) in the same log. Rule for what comes next: anything a
  person can do in a browser surface must also be a CLI verb, and a surface never keeps state the CLI
  cannot read.
- **Elements are entities in an ECS.** `df-table` keeps the canvas in a small dependency-free
  entity-component world: `ecs` (generational handles that go stale on removal, dense component
  stores), `world` (the components: name, kind, position, fields, and a motion while travelling, plus
  the name index), and `motion` (the system that moves travellers each tick). Systems are plain
  functions over the stores, `Canvas` turns an effect into a change of the world and one typed event,
  and a read leaves the world as an `Element` snapshot, so no reader holds a borrow into the stores.
  The machines of later steps add components and systems to the same world instead of new maps.
- **Motion is the host's clock, not a page animation.** `travel ID X Y [speed=N]` sets a motion
  (speed 0.1 to 400 canvas units per second) and the host ticks the canvas about 30 times a second;
  `move` is the instant version and `stop ID` ends a journey. A tick is one revision, only an arrival
  is an audit line (`CANVAS arrive element=pip`), and a tick is cut to 0.25 s so a stalled host cannot
  teleport things. The canvas never reads time: the clock lives in the host and passes seconds in.
- **The page is pushed, not polled.** `/playground/frame?after=N` is a long poll: the host holds it
  until something changed (a canvas effect, a tick that moved something, a control, a scene change, a
  fixture write) and answers with the whole canvas and controls, so a missed frame never matters. The
  page keeps each figure's element and only changes its style, with a 90 ms linear transition, so
  steady travel and fast moves both look continuous. `dev repl` reads commands from standard input in
  one process, about a millisecond each. Measured in the browser (debug build): a command reaches the
  screen in about 50 ms including the CLI's process start; a 70-unit travel produced about 32 updates
  a second and about 120 rendered frames a second with 2.4 to 2.7 px steps; 200 travelling elements
  cost about 3 ms per frame on the host and the page kept up with 6,000 style updates a second.
- **Where it goes next.** The same pattern carries the real engine: a `TableEffect` applied by one
  function and recorded as one event; machines and the DM pipeline added behind the same CLI verbs
  (`spawn machine`, `input <seat> <text>`, `viable <seat>`, `projection show <audience>`); the scene
  record, hotspots and the plate scheduler drawn on this canvas first; and the dev CLI verbs of H.16
  generated from the registry instead of written by hand.

### P. Round log

- **Round 1.** 91 findings merged, 19 contradictions settled (RES1 to RES19), the machine list
  extended, and questions 56 to 70 added. The first-pass proposals withdrawn as conflicting with the
  existing plans: a model-chosen DC tier, auto-declined reactions and an auto-Dodge on timeout, a
  distance-band model, a combat rewind, an invented social record set, an invented effect language,
  and event sourcing as the recovery model.
- **Round 2.** 118 findings (walkthroughs W 30, consistency K 38, feasibility F 23, experience X 27;
  22 blockers, 67 major, 29 minor). The consistency critic found that round 1's code facts were wrong
  in four places and that the machine contract, the machine list, the outcome names and the
  superseded-questions list disagreed with the new text; section J corrects them. New resolutions
  RES20 to RES34, the engine-input path, the walkthrough amendments, the experience contract (K), the
  current-versus-next comparison (L) and questions 71 to 85 were added. The feasibility critic's main
  warning is that projection, the owner thread and the commit path are not yet shaped for the design,
  and that the rules owner does not cover spells, rests or travel hazards, so the first release is
  gated by rules coverage (RES31).
- **Round 3.** 130 findings (conformance P 35, rules D 32, consistency K 35, fact-check F 28; 16
  blockers, 74 major, 40 minor). The conformance critic found that several of my rules conflicted with the
  project's own plans (a closed list of model-calling steps, matching typed text to choices, a
  free-text store in telemetry, a hidden-fact prompt block, client-computed reach). The rules critic
  found that the four outcomes did not cover the roller's own choices, the DM's judgement points,
  or time and aftermath. RES35 to RES52 and the register in M.2 answer them. The fact-check found that
  section L misstated the owner-thread waits, which reaction objects are live and which projection
  is in production, and section L was rewritten.
- **Branch work, 2026-11-02.** Branch `gameplay-engine-v2` was cut from `main`; the plan was committed
  there (`b5b1b6ea`, `2b5bc721`); the old gameplay server, client, `dev` CLI and their client
  modules were removed from `df-tools` (`14a2f715`, 562 files, about 475k lines) with
  `df-tools` native and wasm checks, the workspace check and `df-tools` lib tests passing, and the
  `df-portal` test compile error and two `df-server` environment-read tests failing as before. The
  crate plan of section O followed, at the owner's request.
- **Round 4 (next).** A small round: re-test the M.2 register against the pinned source and check
  the new text for contradictions.

