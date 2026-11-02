# Gameplay engine redesign

Status: **Collecting ideas; four refinement rounds have been run, and the findings of rounds 1 to 4
are merged or being applied.** Decisions are the owner's; the working plan is a draft by the assistant
until the owner accepts it. The refinement loop (four critics a round) ran at the owner's request,
and its results are in the Analysis section at the end. Round 4's findings were filed as GitHub
issues #1490 to #1525 and are being applied to this document. The resolutions run from RES1 to
RES52. Started 2026-11-02.

## How this document works

1. The owner shares ideas, in any order.
2. Each idea is recorded in the owner's words and restated in plain terms so a misunderstanding
   shows up straight away.
3. Facts from the current code are marked as observations. They are not conclusions.
4. Anything the owner has not decided is marked **draft** or listed as an open question.
5. When the owner says the ideas are complete, the analysis is written: conflicts between ideas,
   risks, a proposed shape and an order of work.

**Precedence.** Sections before the Analysis were written before the refinement loop. Where one of
them disagrees with the Analysis (its resolutions RES1 to RES52, the pipeline in section C, the
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

   LOGGING and DEV CLI at every box: the H.16 verb set (dump | force | input | viable | ...)
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

Superseded where it conflicts with RES1 to RES52 and sections C, D, F, H, J and K of the Analysis;
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
    `scene_build_budget_s`, not an open-ended "setting up").
- **Plate scheduler (draft, from D16 to D18; superseded in part by RES5, RES9, RES10, D.1 to D.3,
  K.2, K.3 and H.7).** Three parts, each separate and testable:
  - A **clock** that ticks every `plate_interval_s` (default 15) while a scene is active. Each
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
    broker and each projection registers the same small set of commands. The set is the verb list
    of H.16 (RES33 fixes "the H.16 verbs as a fixed set"); this draft's earlier list (`list`,
    `show`, `history`, `drive`, `config`) is superseded, and `drive` here is H.16's `input`. A
    conformance test fails when a registered surface lacks a verb it should have; H.16 names the
    machine-required ones (`force`, `input`, `dump`).
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
4, 5, 6, 9, 12, 14, 15, 16, 19, 20, 21, 25, 36, 42, 44, 50, 51 (C.5), 53 and 54, and RES48 supersedes the draft default of question 68. Questions 24 and
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
    `rpc-api.md` already set limits? `rpc-api.md` sets no inbound rate numbers; the numbers of C.1 stand
    as the per-seat abuse control, inside the `service-operations.md` ceilings (text 20 a minute per
    principal and the rest) that RES38 says replace the claim that rate numbers are unset.
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
    Draft default (superseded by RES48): reuse is automatic only for a keyed match (equality of the
    typed source-question key and scope, with the affordance id in the key); anything else is a
    suggestion with a host one-tap; the host can always revoke.
69. **Death and replacement.** Is a replacement character's level a host setting, and is a cheaper
    return than raise dead allowed? Draft default: replacement at party level, host-set, and no
    house-rule return.
70. **Who may read hidden content from the CLI.** *Closed (RES43):* the draft default is fixed. On a
    hosted table, who may see `dm_only` and seat secrets through the developer commands? Nobody
    except the owner's operator role (with a reason, an expiry and an audit record), and never
    through a player-facing route.

71. **Skipping and dropping plates (RES9, H.7; changes D16 and D17).** May the engine skip a tick
    when nothing visible changed, and drop the oldest queued plate beyond four? Draft default: yes,
    with every skip and drop logged.
72. **Shop and item inspector in combat (RES2, RES20; changes D4).** May a table decline shopping and
    the table-wide inspector while a fight is on? Draft default: yes, with a plain reason, because
    the fight must not freeze for everyone.
73. **Prefetch against the on-demand rule.** *Closed for places (RES52) by REF-067; open for item
    views.* The `Hub` generates on demand and nothing ahead of time. May the scene builder prefetch
    adjacent places and likely item views on the lowest queue class? REF-067 (Free roam, in
    `game-refinement.md`) generates the next places ahead so the plate, music and ambience are ready
    on arrival, so for places the answer is yes. (REF-066 is place-music pacing and says nothing about
    prefetch.) Nothing decides item views: the earlier draft default stands, which is yes, on the
    `prefetch` class, never ahead of a player-blocking job.
74. **Shared vision and sharing finds (RES24).** The plate defaults to shared vision (M2-D23), defined
    in RES24 as the union of the present members' senses; the host may select the lowest-vision present
    member instead (the intersection), and a find becomes public when its finder says so. Should
    objects in plain view be shared with the party automatically? Draft default: yes for what any
    present member plainly sees (shared vision, the union), no for results of a check.
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

87. **The new crate edge and the host crate (O.2, O.8).** Still open. Approve `df-table` as a new pure
    crate (planned to depend on `df-types` and `df-model`; today it depends on `serde` and
    `serde_json` only) and `df-table-host` as its async host, with the new edges recorded in
    `subsystem-architecture.md`. The crates are already workspace members as a workbench. Draft
    default: yes.
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

## Analysis (refinement loop, rounds 1 to 4)

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
| 4 | 36 issues, GitHub #1490 to #1525 | not tallied here | not tallied here | not tallied here | being applied as in-place corrections (no new RES numbers); see the P round log |

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
| Public projection | `wire::project_journey` and `actor.rs::view` (live); `df-display::project_display_view` has no production caller | O.9 C5 adopts or replaces it |

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
  the plate is more than `image_stale_limit_s` behind (default 90, to be set from the measured
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
config). `rpc-api.md` sets no inbound rate numbers, but `service-operations.md` sets default service
ceilings (text 20 a minute per principal, among others; RES38) that replace the earlier claim that the
rate numbers are unset. The seat rates above stay as the per-seat abuse control inside those ceilings,
and the text ceiling is the tighter bound on free-text lines that call a model (question 58).

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
   the 2024 actions of RES8 (Attack, Dash, Disengage, Dodge, Help, Hide, Influence, Magic, Ready,
   Search, Study, Utilize; Shove and Grapple are Unarmed Strike options inside Attack, Jump is
   movement, M2-D26; an improvised weapon is not an action of its own), then a spell's own
   documented uses (R11 has one fixture per spell entry). A chandelier that is not in the set is
   declined, never invented. A spell effect that is not in the catalog data is not offered.
2. A representable case resolves through the rules owner, with its DC and modifiers from the
   source. A multi-step line is a plan: the forged letter is making it (tool, proficiency and
   materials checked against the inventory) and presenting it, and the guard's reaction is an
   Influence resolution under RES47 (no Insight contest), with the stored result as an input.
3. Where the source leaves a real choice open, DECIDE emits a `RulingRequest`. Reviewed automatic
   adjudication resolves it where the plan allows; otherwise the pending ruling shows
   `AwaitingAdjudicator` (the machine holds only `Waiting{pending_id}`, RES26), and the host's
   phone and the TV show a notice with the permitted
   alternatives (no secrets), and the player may pick a supported alternative or cancel the
   uncommitted part. No timeout spends a resource, rolls a die or invents a ruling.
4. The answer is stored as a `ScopedRuling` (`df-model`) with the normalised key and the question
   held beside it, and disclosed as a ruling, never as a standard rule (question 68 asks whether
   it binds later play automatically). A later input is matched against stored
   rulings by the typed source-question key and scope alone (RES48): equality of the key reuses it, a
   retrieval or similarity hit is only a suggestion with a host one-tap, and a reuse is shown as
   "Ruling (earlier): ...". The only
   settings a host can change are the R17-selected optional modules and a custom `RulesetId`
   (`generated-content.md`); a host cannot inject arbitrary rules.
5. A decline names the missing piece and what is possible ("the forgery needs a kit and ink; you
   have neither"). Requirements are `requires{tool, language, proficiency}` on the action, and the
   rules owner declines a missing tool or language, and a missing proficiency proceeds without the
   bonus (M2-D19); the SRD gives no disadvantage for it.

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
confirmation. There is no undo and no reroll except one a source rule grants (Heroic Inspiration
or Lucky, M2-D4): otherwise dice are never rerolled, and a wrong act is handled by the ruling
flow. Narration may not state what a player character does, says, feels or decides beyond the
committed outcome, and the qualification step rejects text that does.

**C.6 Declines (E22, C21).** A decline has a stable code, a plain reason, the missing piece if any, and
what would be possible. Decline codes: `not_viable`, `not_supported_yet`, `not_your_turn`, `not_in_combat`,
`no_combat_profile`, `missing_requirement`, `window_closed`, `economy_spent`, `resource_spent`,
`table_busy`, `rate_limited`, `intent_invalid`, `pending_open`, `pvp_off`, `consent_declined`,
`time_locked`, `not_knowable`, `stock_changed`, `same_urging` (RES47) and `casting_restricted`
(M2-D27, reused by M2-D38 for silence, a gag, no hand free and untrained armor). M2-D33 to M2-D52
add no code: the missing piece goes in the reason text of an existing code. This is the one list: J.3 "Decline codes added" is a subset of it, and RES40 maps some of
these to the closed wire `RejectionCode` set while the rest stay internal reason keys. A
clarification, a confirmation and a ruling are not declines: they are `needs_clarification` and
`pending` outcomes (RES16, RES26). A player who asks "why" gets the rule text.

**C.7 When the text model fails (N5, N12).** A failed model call never produces a guess and never
uses a keyword table. INTERPRET runs on the fast lane (question 65), and its retry uses a shorter prompt and the
legal-choice list only. If that fails, the DM says "I could not read that. Pick one:" and shows the seat's
legal choices as buttons, which need no model, and keeps the player's draft. NARRATE falls back to
a structured readout of the committed outcome's typed fields ("Pip hits Dain Rusk for 7 slashing.
Dain is at 12 of 19."), a rendering of rules output and not an interpretation. Flavour that arrives
within `narration_late_s` is appended after it and never replaces it. Output that is empty,
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
scene**: the host can lock one, and otherwise the K.10 rule applies, with `focus_min_dwell_s`, and
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
maps to `characters[]` with one `active`. A dead character is held with its record (M2-D1), and the
seat goes to the creation machine for a replacement (R01, R02) only on a host "replace" or when no
revival is possible; until then it waits or spectates (question 69); import goes only through `validate_build`, and private
import cannot mint rights (`rules-support.md`). A party wipe returns `EncounterOutcome=defeat` and
hands off to `AdventureEnd`, where `narrative-engine.md` allows an authored failure state.
Events: `seat.state_changed`, `character.status_changed`.

**D.5 Cast entries (M11, C19, M10, C22).** A cast entry has `kind` (party, npc, summon, mount, companion,
hireling), `controller` (a seat or the engine), `bound_to` (an `ActiveEffect` or a character),
`stat_ref`, `presented_as` and `visibility`. The last two are derived from active effects and the
rules owner (disguise, invisibility, a form from Wild Shape), never typed by a machine. Each
perception change writes a `WitnessRecord`. Locate expects only entries whose derived visibility is
`Seen`, with one exemption: every hero (a party cast entry) is always expected, drawn and counted
even when Hide or Invisible makes its derived visibility something other than `Seen`, because
hiddenness filters only the enemy view and a hidden hero is drawn with a hidden chip (R06, RES45,
K.13, M2-D17). Locate also keeps an explicit expected-unlocated list. Plate references (Q21) are ranked: the
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

**D.6a Fields the M.2 rows D33 to D52 add to the layout.** A set piece gains `material` (cloth, paper,
rope, crystal, glass, ice, wood, stone, iron, steel, mithral, adamantine: the Object Armor Class table,
SRD p.178), `size` (Tiny to Large; a Huge or Gargantuan object is split into Large sections, SRD p.178),
`fragile` (picks the Fragile or the Resilient Hit Points column, SRD p.178), `auto_break` (authored: a
fragile nonmagical object such as glass or paper breaks as an action with no roll, SRD p.12, p.178),
optional `damage_threshold` (SRD p.180) and `lock{dc}`, and an item or effect may carry
`light_source{bright_ft, dim_ft, burns_h}` where `dim_ft` is the extra Dim Light ring beyond the
Bright radius, as the SRD words it (a Torch is 20 and 20 for 1 hour, SRD p.100); a lit source is
written as an `ActiveEffect` area that follows its bearer. A cell's `cover` holds one of none, half,
three_quarters or total, and an authored `rough` or `slippery` flag marks water or a surface that
calls for the GM-option DC 15 Strength (Athletics) check (SRD p.179, p.189). Owners: M2-D35, D36, D41
and D45.

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
| R05 initiative, economy, reactions (C3, C7, M2) | combat (section F.1) | The phone shows remaining action, bonus action, reaction, movement and free interaction from the encounter view; windows per C.4 | `window.opened`, `window.answered`, `window.closed`; `pending list`, `pending show`, `input <seat> <tap>` (H.16) |
| R06 movement, sight, cover, hiding (C1, C2, E7, C15) | explore, combat | RES1 and D.6; a creature hidden from the party (an enemy) is left out of the public plate and revealed by an effect that queues a new plate, but a hidden hero is still drawn, with a hidden chip, because every hero is always visible to the party (M2-D17, K.13, RES45) and hiddenness filters only the enemy view; a hidden player's own phone shows its position as a private overlay; a threat the party knows of and cannot see is a marker, never the figure (question 66) | `spatial.query`, `stealth.revealed{by}`; `spatial query`, `combat anchors show` |
| R07 attacks, grapple, shove (C7) | combat | Combat intents from the legal-choice call | `plan.step{id, outcome}`; `combat choices <actor>` |
| R08 damage, death (C21) | combat, rest | Status chips (dying, stable, dead) to the seat; D.4 for replacement | `character.status_changed` |
| R09 conditions, durations (C8) | table state `active_effects`, owned by `df-rules` | Effect chips and a concentration marker from permitted views; expiry at `TurnBoundary{creature, start or end}` in combat and at `df-world` time outside it | `effect.applied`, `effect.expired{origin, boundary}`; `effects list <creature>` |
| R10 spellcasting, concentration (C20, M4, C8) | explore, combat | Spell offers from the legal-choice call; the phone targets by tapping a creature hotspot or a point on the plate, the tap is a proposal that the engine converts to cells with the same declared function, shows for confirmation and lists the creatures caught, and the rules owner validates; costly components and slots are checked by the rules owner | `spell.target_selected{op, origin, caught[]}` |
| R11 features, summons, forms (M11) | cast entry kind and `bound_to` | Form effects replace stat blocks and revert by rule; summons use a light persona and no hooks | `entity.joined`, `entity.ended`, `form.applied`, `form.reverted` |
| R12 equipment (M12, M13) | shop (offer flow), inventory (placement) | D.7 | `item.moved`, `purse.changed` |
| R13 rests (E8, C18, M8) | **rest** (added to the first machines) | Calls the rules owner's rest and `df-world` time; handles `RestInterrupted` and incomplete results; applies rules-produced recovery effects only; watches are per-seat assignments the encounter proposals read; the short rest now inside combat aftermath moves here | `rest.started`, `rest.interrupted`, `rest.completed`; `force rest <state>` (H.16) |
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
  pace), narration attaches later or is skipped, and the next server-run enemy turn starts after the
  dwell and not after narration (RES45: hero input opens immediately, and only server-run enemy
  turns are spaced, by a server timer from anchors and never from client acknowledgements).
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
purpose through the broker), and the table waits at most `scene_build_budget_s` if it is not ready on arrival (reusing the
SCENE-01 owners `place::plan`, `place::overlay`, `place_creation::judge_exit` and
`place_creation::create`), with the
`place card` fallback (name, one line, cast list) if the build exceeds `scene_build_budget_s`.
A full scene is built only for an encounter or a stop. Today's seven-minute travel hang is a
failure of this rule. Events: `travel.leg`, `travel.arrival_wait{ms}`.

**F.3a Leg data (M2-D43).** A leg carries `terrain` (a row of the Travel Terrain table, SRD p.192),
`pace` (fast, normal or slow, capped by the terrain's maximum pace and raised one step by a good road,
SRD p.192), `road`, `hours`, `mounted`, `vessel{speed_mph, hours_per_day}` (a vessel sets the speed and
the table chooses no pace, SRD p.12, p.192), `marching_order[]` (SRD p.12) and the terrain's
`forage_dc`, `navigation_dc` and `search_dc` (SRD p.192). `dump --json` on the travel machine prints
them, and the `travel.leg` event carries the same fields.

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
   steps. Step one needs a forgery kit (the tool) and materials; without them the line is
   declined `missing_requirement` ("the forgery needs a kit and ink; you have neither"). Proficiency
   with the kit is not required: a hero with the kit and no proficiency proceeds without the
   proficiency bonus (C.3.5, M2-D19). With the kit and materials,
   making the letter takes time (a `RequestTimeAdvance`) and its quality is stored as a fact on the
   item. Step two is an Influence resolution under RES47 (SRD p.184), with the letter's stored quality
   as an input and no Insight contest: a willing guard needs no check, an unwilling one refuses, and a
   hesitant one is a check against the higher of DC 15 or the guard's Intelligence score. The guard's
   stance is decided by the rules and voiced as a `SpeechEnvelope`.
   A promise the guard makes becomes an `Obligation` only through an approved commitment action.
3. **"I use fire bolt to light the candle."** `Magic` carries the spell and the purpose. The spell's
   documented uses (R11 fixture) cover igniting an unworn, uncarried flammable object, so the rules
   owner resolves it as Fire Bolt's ranged spell attack against the object's AC (SRD p.132): no
   slot, because it is a cantrip, but an attack roll, and a hit starts the object burning: it takes
   the Burning hazard's 1d4 Fire damage at the start of each of its turns until it is doused, submerged
   or destroyed (SRD p.178; M2-D41 carries the burning object's per-round step and the light it gives). A use
   the spell does not document goes to the ruling flow (RES52).
4. **"I attack" next to the innkeeper and a goblin.** More than one legal target and a harmful
   reading: INTERPRET returns `Clarify`, the phone shows the two targets as buttons, and choosing the
   innkeeper asks for a one-tap confirmation. This replaces the observed misreading of "I draw my
   greatsword and attack Dain Rusk" as an Intimidation check.
5. **A goblin attacks the wizard, and the wizard has a reaction.** CHECK and DECIDE roll the attack
   with the table's dice; the rules owner reports that the wizard's own Shield can apply (RES52:
   Uncanny Dodge is self-only, SRD p.63, so an ally rogue cannot use it for the wizard). APPLY phase
   one commits the pending attack and the window. The wizard's phone gets the offer, the other
   phones show "waiting for the wizard's player", and the TV shows a plain waiting state. If the
   wizard's phone drops, the turn is `Held`, nothing is spent, and the reconnect reacquires the
   window by `pending_id`.

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
the single total order recorded as `turn.seq`. Lane caps are measured when the owner queue first runs over real adapters (O.9 step 9, host H0 to H3); no O.9 step owns that measurement yet, so the caps stay candidates until one does. Test: send one operation id 50 times concurrently and assert one effect.

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
| Text slow, empty or garbage | C.7: one retry, then buttons or the structured readout | "I could not read that. Pick one:" or the structured line | `text.failed{cause}` |
| Plate later than `image_stale_limit_s` (default 90) | Hold the last plate of the same scene | "Picture catching up" mark (RES10) | `plate.late{lateness_s}` |
| New scene background fails | The authored or last background for the place, else a place card | Playable scene without art | `scene.degraded{stage}` |
| SAM fails | One retry, then one bounded regeneration (Q20), then the rules layout with the character `unlocated` | Hotspots as a list strip | `locate.failed` |
| Image admission refused by the hosted entitlement or spend authority (hosted builds only; local builds never refuse for spend) | The tick stops requesting | The `art_delayed` status (art is not requested while admission is refused), play continues | `plate.admission_refused` |
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
overwrites a valid entry. The leak check is server-side and by id (RES35): every grounded clause
carries a permitted claim id, and an id outside the listener's projection rejects the envelope, with
no keyword matching and no model-reported "which secret ids a clause revealed" (withdrawn by RES35).
The qualification step redacts or re-requests once and logs `leak.blocked{fact_id}`.

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

**H.11 Hidden information and injection (N11, N12).** Prompts are built per audience (RES6). No
secret id or text enters any prompt: an NPC's stance toward a hidden fact reaches the model only as a
typed `SpeechActPlan` stance with permitted claim ids (RES35 withdraws the earlier `dm_context`
block). Logs hold
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
one input per seat every ten seconds is stated here and measured on the first playable path (O.9 step 9, host H0 to H3; no O.9 step owns the measurement yet).

**H.15 Projection status (N16).** Every projection carries a `revision` and a `status` (`ok`,
`reading`, `rolling`, `telling`, `art_delayed`, `degraded`, `read_only`, `reconnecting` and
`waiting_for{who, what}`: the nine keys of `STATUS_KEYS` in `crates/df-playground/src/copy.rs`; the
earlier single `thinking` is replaced by the three named stages of K.5, INTERPRET, DECIDE and
NARRATE) and a plain waiting reason for the seat. A status other than `ok` for more than 8 seconds shows a banner on the host phone and the
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
`projection show <seat or tv>`. This is the minimum set: section E and M2-D33 to M2-D52 also rely on
the read verbs `combat choices <actor>`, `combat anchors show`, `spatial query`, `effects list <creature>`,
`check show`, `pending show <id>` and `rulings show`, and on the family verbs `sheet build`,
`sheet validate`, `advancement pending` and `character award`, and on the stage-direction verbs `camera
focus|wide|frame|follow`, `director show|pause|resume|auto` and `scene state` (O.14), which are additions to it. The travel
leg is read with `dump --json` and the `travel.leg` event, and there is no `travel leg show` (F.3a).
All go through the normal entry points. Fault, fixture and force
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
section G traces once each is reviewed against the pinned source (RES52); G.3 and the spell part of
G.5, the Shield reaction, are fixtures pending R10) diff the turn log, snapshot and per-seat projections. Chaos tests inject each row
of H.4. A load test runs six tables of six seats on a scripted broker and checks that
`live_tasks` returns to zero and memory stays flat. The real-model INTERPRET accuracy run is nightly,
on a labelled line set that includes the three known-bad lines of the observations. Persistence
tests use the PostgreSQL requirement of `storage-architecture.md`, one scratch instance or schema per
run, and include restart against existing data.

**H.18 Progress and preflight (N15).** Per input the checkpoints are `input.received`,
`turn.applied`, `projection.delivered{seat}` and `client.presented`. A per-table watchdog raises
`stall{kind}` when an input shows no progress for 20 seconds, the oldest job passes twice its
deadline, a plate is later than `image_stale_limit_s`, or a worker heartbeat is missing.
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
  relaxed (question 73: REF-067 relaxes it for next places; item views stay an open default). Durable intents today are single-slot (`run_committed_intents` finds the
  first pending one), and H.1 generalises them with `EffectRegistrationInspector` and `SentUnknown`.
- **Spend refusal is hosted only (K13).** H.4's refused admission applies to hosted builds under the
  entitlement and spend authority of `commerce-service.md`. Local and development builds never
  refuse for spend, and the ledger never blocks.
- **The late-picture mark (K11).** The threshold of RES10 is `image_stale_limit_s` (default 90),
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
  to the party or in shared vision. **Shared vision** is defined here once: it is the union of the
  present party members' senses, so a thing in plain view (no check needed) of at least one present
  member is in the shared view; this is the default (M2-D23, question 74). The host may instead
  select the lowest-vision present member, which is the intersection (only what every present member
  can see). A fact that one seat gets from a check, or from a sense the others lack and that is not
  in plain view, is `seat_private` and appears only in that seat's private list and overlay, never in
  the TV plate, TV hotspots or plate prompts. It becomes public when the finder taps "tell the party" or says it in
  an action or social line, which writes a party-scope grant in the same commit. The plate
  defaults to shared vision (M2-D23, which supersedes the earlier "lowest-vision member" default);
  the host may instead select the lowest-vision present member (question 74). The plate
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
  several scenes later, the TV focus changes only after `focus_min_dwell_s` and never mid-beat.
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
  where `outcome` is `applied`, `needs_clarification`, `declined{code, reason, alternatives}` or
  `pending{choice | roll | reaction | ruling}`; a step may instead return `Waiting{pending_id}`, which
  is a `StepResult` value and not an `Outcome` variant (O.7);
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
  the frame and fall back to explore. A Long Rest interrupted by combat (rolling Initiative, damage, a spell other
  than a cantrip, or an hour of exertion) is offered a resume by the rest machine, never continued
  silently: if at least 1 hour was rested before the interruption the rested hours give the benefits
  of a Short Rest, and the Long Rest can be resumed immediately and then needs 1 additional hour per
  interruption (SRD p.185). A Short Rest is interrupted only by rolling Initiative, casting a spell other than a cantrip or taking any damage, never by an hour of walking or other exertion (which interrupts a Long Rest only), and an interrupted Short Rest confers no benefits (SRD p.187; its restart from zero is M2-D46's convention).
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
  is a tested library function with no production caller, and O.9 C5 (Projection) decides whether to adopt or
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
  verbs as a fixed set (the minimum: H.16 also names the read verbs that section E and M2-D43 add), and `force` and `input` (H.16; the older name `drive` is retired) as new owner-inbox jobs. Interaction machines are
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
- **Trap data (M2-D42).** A trap set piece's `trigger` set (W16) gains `detect[]` (each `{action: search or
  study, ability, skill, dc, reveals}`), `disarm[]` (`{action or tool, ability, skill, dc, on_fail}`,
  where `on_fail: triggers` means a failed attempt springs the trap), `resets{when, limit}` (absent means
  inert after one activation, SRD p.199) and the severity and level range of SRD p.199. A passive result
  stays private (SRD p.186).
- **Other fields added by M2-D44, D48, D50 and D51.** A creature has `exhaustion` (0 to 6, one cumulative
  `ActiveEffect`, SRD p.181); an item has `attuned_to`, `cursed` and `charges{max, regain}` with regain
  rules such as 1d6 at dawn (SRD p.206, p.216); a species feature may carry `rest_override{long_rest_h,
  conscious}` (Elf Trance, SRD p.85) or a rest-end award (Human Resourceful, SRD p.86); a stat-block
  ability has `usage{recharge{min, max}, per_day{n} or after_rest}` (SRD p.257).
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
| `plate_interval_s` | 15 | plates per hour (upper bound) |
| `image_stale_limit_s` | 90, set from the measured lane | none |
| `scene_build_budget_s` | 8 (section K) | none |
| `plate_swap_wait_s` | 6 | none |
| `narration_late_s` | 8 until measured (H.5 budget, plus `narration_batch_s`) | none |
| `narration_batch_s` | 4 | fewer narration calls |
| `beat_min_dwell_ms` and `caption_min_dwell_ms` | formula in section K | none |
| `idle_minutes` | 10; eviction at 30 | stops plate cost when idle |
| `window_timeout` | none: windows hold (RES26) | none |
| `held_nudge_s` | 30 | none |
| `ruling_nudge_s` | 20 | none |
| `focus_min_dwell_s` | 45 | none |
| `reading_pace` | normal | scales dwell |
| `reduced_effects` | off; a per-seat preference through `SetPreferences`, and the shared screen has its own value in the host's launcher Settings (RES50, O.13, O.14) | none |
| `focus_hold_s` | 8, host launcher Settings (O.14) | none |
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
| owner lane caps (engine, timers and results) | measured when the owner queue first runs over real adapters (O.9 step 9); no O.9 step owns the measurement yet | none |
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
- A recall question ("what do I know about these runes?") is the Study action. An authored knowledge
  fact carries a recall skill and DC assigned when the fact is admitted, from authored content or by
  the admission validator from the source's own table, never by the narration model. A generated
  fact is plainly known or has no recall DC (M2-D20, the later rule). A fact the
  character's background plainly gives needs no roll. The result is a grant to the asker, shown
  privately.
- A question with no grounded answer returns "You don't know, and nothing here tells you. You could
  search, ask Mara, or study the runes." It never returns an invented fact about a character or the
  world (RES52 scopes the rule to character and world recall, and dialogue keeps its logged backfill
  decision), and a subject outside the asker's knowledge is declined `not_knowable` with alternatives.
- A request about an object that a place of that kind plainly has ("is there a table I can flip?")
  is a question answered from place-type defaults in content data, not a decline. Only an object that
  makes no sense for the place is declined.
- `Social` goes to dialogue, `PlanOnly` goes to a table-notes strip that nothing executes (question
  77), and `Joke` gets a short in-character reaction with no effect.

**K.2 Waiting for art is never a gate after the first scene (X2).** A scene is playable as soon as
its description, cast, set and layout (the dress step) are stored. Until the plate is Ready the TV shows an establishing card:
the place name, the stored description voiced as the first narrator line, the party strip, the
place music and a small "picture on its way" tag. A background that arrives first shows as the
background with a slow dissolve, and a late plate dissolves in only at a line boundary (K.3).
`scene_build_budget_s` defaults to 8, after which play continues on the establishing card with no
blocking state. Only the first scene of a session or an authored arrival beat may hold up to 20
seconds, with narration filling it (question 78 asks whether the first scene is built during session
zero). Travel narration is timed to cover the build, and the narrator never says "you arrive"
before the destination record exists. This is the answer to the seven-minute travel hang and it
replaces "setting up until Ready" wherever that text remains (the budget is also the trigger for
the place card).

**K.3 The clock requests; the buffer decides when to show (X3).** `plate_tick` decides when a plate is
requested, never when one is shown. The playout buffer swaps a plate only between lines, never
during a dice or result card, never during a reveal beat (K.7), and with a dissolve of at least 1.5
seconds. A ready plate that finds the slot blocked waits at most `plate_swap_wait_s` (default 6)
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
the ruling. After `ruling_nudge_s` (default 20) the host sees "Pip is waiting on a ruling" and
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
"Picture catching up", "I could not read that. Pick one:"). No codes appear in player text, and
the reason text of `not_supported_yet` reads "I can't run that yet." Dice and result cards of three
seconds or more are staged first so they cover narration latency. This depends on question 65
(INTERPRET on the fast lane), because today's Codex text calls take 15 to 45 seconds.

**K.6 Fairness among seats (X6).** The owner queue is served round-robin by seat, with a bound of
three pending per seat, so `table_busy` applies to the seat over its own bound and never because of
another seat. Outside combat, inputs from different seats arriving within `narration_batch_s`
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

**K.9 Voice (X9).** A character's dossier holds `voice_card` (register, three verbal tics, how they
address strangers, two example lines written and validated at admission) and the character record
holds a reference to it (RES52), plus `voice_id` (a fixed voice and pitch assigned at admission and
never changed). Every prompt for that character carries
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
and, by default, no minimap: the tactical strip is on the phones only, and whether a toggleable TV
minimap counts as the grid that `implementation-roadmap.md` removes from the TV is open (question 86,
RES52). Legal targets are highlighted, and the choice list
says why a target is out of reach ("12 ft away, reach is 5 ft"). The client already computes
`reachable_from` in `df-render/src/battle/grid.rs`; moving that computation behind the machine view
is listed as debt in section L.

**K.12 Combat pace (X12).** The dwell is `max(1800, 330 * words)` milliseconds for text read across a
room, at least 3000 for a result card, scaled by the table's `reading_pace`. Consecutive turns of
creatures on one side that did not target a party member collapse into one beat with a summary
line ("The goblins close in"), and turns that hit a hero keep their own beat. A seat two turns away
sees "You're up after Mara" with its legal choices for a plan-only note, and the next seat gets a
haptic and a chime. After `held_nudge_s` (default 30) a held seat's host sees one line with the
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
text field is always visible. Typed text always goes to INTERPRET with the seat's legal choices as the
closed enumeration; a tap on a choice is the only no-model path (RES46 withdraws matching typed text
to a choice id).

**K.15 Secret checks in a shared room (X15).** A secret check shows the TV the same neutral cue ("The
DM passes Pip a note") for success, failure and no information, with the same shape and duration.
The private result opens as a note card on the phone that auto-hides after 10 seconds, with "share
with the party" (a `PrivateMessage` to chosen seats or all) and "keep it to myself". Nothing about
the note appears on any other seat.

**K.16 Speech and voice (X16).** A `Say` input up to 800 characters is shown verbatim on the TV as the
hero's line in up to three bubbles, and INTERPRET never rewrites spoken words; it only extracts
intent and claims for the rules. (Action text keeps the 280-character cap of C.7.) Voice is
hold-to-talk on the speaker's own phone only, so seat attribution is by device, with the transcript
shown as final text only, sent on an explicit tap (RES49), with a cancel-only window of two seconds
that scales with `reading_pace`, and the scene's names passed to the recogniser as hints. Irreversible readings still need the C.5 confirmation, and a speech is
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
session shows the recap and offers one callback to a prior-session choice in its first scene when a
valid candidate exists (K.7; RES52: no callback is guaranteed). The adventure-end machine is specified before the first release because it is where a table
decides whether to return. Question 83 asks whether the default recap is a budgeted video bookend or
a text-and-stills card.

**K.22 Smaller table-craft rules (X23 to X27).**

- **Tone.** Session zero stores `tone` (heroic, grim, whimsical), `humour` (0 to 3),
  `violence_description` (implied or plain) and `narration_length`, and the broker injects them with
  lines and veils. Humour is allowed only in a clause tagged flavour, so it cannot carry an outcome.
- **Flaws.** A "play to my flaw" tap on the sheet passes the declaration to NARRATE as stated. The
  engine may suggest Heroic Inspiration to the host (accept or decline) for any heroic, in-character
  or entertaining moment (SRD p.8), not only a flaw tap (M2-D28); the rules owner owns the
  award and the model never awards it (question 84 asks whether the rules plan treats it as
  adjudicated or discretionary).
- **Table talk.** A hero's said words appear on the TV and actions appear as a short structured line.
  A private message never appears. Talk aloud in the room is not input, and the phone offers "tell
  the DM what we decided" as a plan-only note.
- **Catch-up.** A "story so far" view shows the journal projection and a "what just happened?" button
  that lists the last minute's committed events with no model call.
- **Declined asks.** `declines top` counts declines by decline code only, so the owner can see which
  creative asks fail. No free-text store of declined lines exists: RES36 withdraws it, and private
  player text is not archived in telemetry.

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
the first measured run of the host (O.9 step 9; no O.9 step owns that measurement yet) produces the numbers.

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
| Dev CLI | About 100 `.route(` calls in `dev_control/mod.rs`; each command is registered in four places (route, match arm in `dev_cli.rs`, `print_help`, `describe_commands` in `reads.rs`); `/dev/describe` exists but is hand-written; family verbs exist under other names (`state`, `view`, `checkpoint`, `act`, `say`, `override`, `dice next`, `fight offers`) | One route and command per machine verb, `describe` generated from the registry (RES33, H.16) | The H.16 verb set (RES33) is generated, not hand-written |
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
- **RES37. Reach is server-side (P3, M2-D22).** The tactical strip and the choice-list reasons render
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
- **RES47. Social rules are the 2024 ones (P20, M2-D18).** Influence follows willing, unwilling or
  hesitant. A willing request needs no check, an unwilling one is refused, and a hesitant one is a
  check against the higher of DC 15 or the creature's Intelligence score, with no Insight contest
  (the G.2 and C.3 wording is corrected). Willingness is the EMO-01 reviewed automatic case, logged,
  with host override. After a failed check the same urging cannot be repeated for 24 hours or a duration
  the GM (host) sets (SRD p.184); the decline is `same_urging`, keyed by target and request, and it
  reopens when something changes (custom: the SRD says only "in the same way", so reading a changed
  request or circumstance as a different urging is this design's convention). Haggling is one hesitant check per request,
  and price movement from a check is an R17 optional module that stays off until the coordinator
  records it. A `done` NPC maps to `applied` with a structured line ("Dain turns away") and not to
  silence.
- **RES48. Rulings are keyed, not retrieved (P21, M2-D25).** A `RulingResponse` is limited to existing
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
  units, with two stated exceptions that are set in whole minutes because a host thinks of them that
  way: `idle_minutes` (J.4, D.3) and `evening_minutes` (J.4, K.21) (RES50 renamed every `_secs` name to `_s` throughout this document; the old `scene_build_budget_secs` is `scene_build_budget_s`). "Backdrop" is not used in game text: the
  words are plate, establishing card and place card (the built workbench keeps its slot name `tv-backdrop` and O.13 its camera layer, neither of which is game text).
- **RES51. Crates (P24, P25).** The machine contract, registry, `step` and the staged transitions
  belong in `df-engine`; the owner queue, durable intents and timers in `df-session`; resource
  facades through `df-media` and `df-provider-api`; audience projection in `df-api` (or
  `df-engine::read_model`); and the browser in `df-web`, `df-player`, `df-display` and `df-ui`, which
  may not depend on `df-model` or the rules. `df-tools` and the current `df-display` placement are
  transitional. D7 does not apply to the wire, the field ledger or the persistence migrations of
  RES17. Each slice names its roadmap gate and plan-database item.
- **RES52. Small conformance rules (P19, P30, P31, P34).** The G traces are candidate narratives and
  not goldens until reviewed against the pinned source; each becomes a golden session (H.17, O.9 C3)
  when that review is done, and not before. G.5 uses the wizard's own Shield (an
  ally cannot Uncanny Dodge for a wizard), and G.3 keeps Fire Bolt's attack roll against the object's
  AC with no slot. Dialogue keeps the backfill decision and logs it, and K.1's "never an invented
  fact" is scoped to character and world recall. `voice_card` lives in the dossier and the character
  record holds a reference. Combat bindings come from authored or template entries admitted through
  content admission, and a model never picks a profile. K.21 offers a callback when a valid candidate
  exists and does not guarantee one. Seeds may be retired. Whether a TV minimap counts as the grid
  that `implementation-roadmap.md` removes from the TV is a question for the owner (question 86), and
  question 73 is closed for places by the look-ahead decision REF-067 (REF-066 is place-music
  pacing, not look-ahead); item-view prefetch is not decided and stays an open default.

**M.2 Rules-lawyer resolutions (D), as one register.**

The rules-lawyer's main finding is that the four outcomes and the reaction-window model cover other
seats' reactions, but not the roller's own choices, the DM's judgement points, or time and aftermath.
This register names, for each, what the engine does and which carrier owns it. Details marked (V) were
recalled and not confirmed, and the source gate decides them. Each row is a fixture candidate. A detail
checked against the pinned source (SRD 5.2.1, `development/rules-sources/srd-5.2.1-en.pdf`) carries its
page as (SRD p.N). A rule that SRD 5.2.1 does not contain is marked (custom) with the reason: it is a
table rule of this design, stays so until the owner accepts it, and is never presented as a 2024 rule.
Outside this table the register's ids are written M2-Dn (M2-D5, and so on), so they cannot be read as
the owner's decisions D1 to D25; the table itself keeps the bare numbers. M2-D18, M2-D22 and M2-D25
have no row: their findings (social rules, reach, rulings) were resolved directly as RES47, RES37 and
RES48, and those entries are their record.

Rows D33 to D52 were added for the twenty rules gaps of GitHub issues #1547 to #1566 (gaps C1 to C20 in
issue order, so C1 is D33 and C20 is D52): each fills a rules area that had no owner, each cite was
checked against the pinned PDF, and each has at least two test cases with a boundary. They add no
decline code (the missing piece is the reason text of an existing code, C.6). They use the dev verbs of
H.16, the read verbs section E already names (`combat choices`, `spatial query`, `effects list`,
`check show`, `pending show`, `rulings show`) and the audit-line shape of RES41. The data they need is
in D.6a, F.3a and J.3, and M.5 lists which rules areas have an owner and which are still thin.

| Point | The engine does | Carrier | Test case |
|---|---|---|---|
| D1 death and revival | A character at 0 HP is Unconscious and dying (death saves), and dies outright when the damage left over after reaching 0 equals or exceeds its HP maximum (SRD p.17); whether a monster dies at 0 HP or is treated as a character is encounter data (D2), and the default is that it dies (SRD p.17); a dead character is held with its record. The creation lane opens on a host "replace" or when no revival is possible (D.4 agrees). Defeat is all heroes dead or an authored capture state, never "all down" while saves pend | rules owner; encounter outcome | Three heroes at 0 HP with saves pending and the goblins withdraw: no `defeat`. A cleric with a slot and the component can still revive a hero 8 rounds (48 seconds) after death (Revivify, SRD p.159: the creature died within the last minute). A hero with 12 HP maximum and 6 HP takes 18: dies at once with no saves. A goblin at 0 HP is dead unless its encounter data says otherwise |
| D2 DM decision points | A register of each point and its carrier (below) | affordance data, derived facts, encounter data, EMO-01, `pending{choice}`, `RulingRequest` | A fixture is complete only if every choice it triggers names a carrier |
| D3 stakes and failure | Affordances carry `on_failure` chosen from existing mechanics (a condition, hazard damage, time lost, alert, no retry without change); the roll card shows the stakes first; an affordance with no DC resolves without a roll | authored affordance data | A failed climb costs time or a slip and not only "you fail" |
| D4 the roller's own choices | An Outcome is a sequence of roll stages; after each, the rules owner lists the roller's options as `pending{choice}`; a reroll draws a new `draw_id` and keeps the old one logged; C.5 reads "no undo and no reroll except one a source rule grants" | rules owner | A miss by 1 plus Heroic Inspiration: the new die replaces the old, Inspiration is spent, a second miss offers nothing, one HP movement commits |
| D5 Counterspell and Shield | Phase one reserves the slot and draws nothing for effects not yet reached; phase two draws and spends after the window; a countered spell releases the reservation and keeps the action spent (Counterspell, SRD p.120: on a failed save the spell has no effect, its action is wasted, and a slot used to cast it is not expended); nesting is bounded by eligible reactors | rules owner; window handler | A countered Fireball: slot count unchanged, action spent, no damage draw consumed |
| D6 ordering | `pending{choice: order}` to the controller of the creature whose turn it is; any engine-chosen order is a logged reviewed case | window handler | Ongoing damage plus a save-ends condition: the player chooses the order |
| D7 Ready | The trigger binds to ids (creature, set piece, or an event kind the rules owner sees); an unbound trigger is `pending{ruling}` with a one-tap fired or not; until built it declines `not_supported_yet` | rules owner; ruling flow | A readied leveled spell whose concentration breaks before the trigger: the slot stays spent and nothing fires |
| D8 initiative | Surprise is derived from `WitnessRecord` and gives disadvantage on the initiative roll, not a skipped turn (SRD p.13); player ties are `pending{choice}`; joiners roll when they arrive and slot in at their count (custom: SRD 5.2.1 says nothing on joiners mid-combat); Delay is an R17 convention or dropped (custom: the SRD has no Delay action) | rules owner; encounter owner | Reinforcements arrive in round 2 and roll 12: they act this round between counts 15 and 8 |
| D9 legendary and lair | `Interlude{creature, source}` after each turn with an engine responder for Legendary Actions (one at a time, only after another creature's turn, all uses regained at the start of the creature's own turn, SRD p.257); a count-20 lair entry (custom: SRD 5.2.1 has no lair-action section and no "Initiative count 20" text, only stat-block values that change "in Lair"); Legendary Resistance is an engine-responder window and is an X/Day use that returns after a Long Rest, never at the creature's turn (SRD p.257-258) | combat machine | A dragon with 3 legendary actions takes one after each of three hero turns, none after a fourth, and the pool is full at its own turn (Large creatures wait on footprint support). The same dragon spends one of 3 Legendary Resistance uses: 2 remain at its next turn and 3 after a Long Rest |
| D10 summons and mounts | A cast entry has `initiative_rule: own or shares{owner, after}` from the effect; an uncommanded default comes from the rules owner | cast entry | An ally acts after its summoner each round and takes Dodge when not commanded |
| D11 forms | One kind: a form replaces game statistics but never Hit Points. The creature keeps its own HP and gains Temporary Hit Points, equal to the Beast form's HP for Polymorph (SRD p.153) and to the Druid level for Wild Shape (SRD p.43); the ordinary temporary-HP rule applies (lost first, leftover damage carries to real HP, SRD p.18). Polymorph ends early on the target when it has no Temporary Hit Points left (SRD p.153); Wild Shape has no such early end and lasts for its duration, or until the druid is Incapacitated, dies or leaves it (SRD p.42) | form effect | A polymorphed hero in a 20 HP beast form takes 25: the 20 Temporary Hit Points are gone, the form ends, and the hero's own HP loses 5. A Wild Shaped level 4 druid takes 6: 4 temporary points are lost, the druid's own HP loses 2, and the form stays |
| D12 concentration | Save is Constitution against DC 10 or half the damage taken (rounded down), whichever is higher, up to DC 30 (SRD p.179), one per damage source (custom: the SRD says only "if you take damage", this design reads each source as its own damage); it ends when the creature is Incapacitated or dies (SRD p.179); replacing it needs a confirm | rules owner | 20 damage taking a hero to 0 ends it with no save; 9 then 30 in one round is two saves at DC 10 and DC 15 |
| D13 aftermath | An `Aftermath` sub-state steps in 6 seconds while any hero is dying (custom: no SRD 5.2.1 source for this cadence; the SRD has death saves only at the start of the dying creature's turn, p.17); rest and travel are refused during it | combat exit | The last goblin falls with a hero on one failed save: explore does not begin |
| D14 rest | Rest states: planning, resting, interrupted or complete, post-rest choices (hit dice loop, spell preparation, recovery features), done; after finishing a Long Rest a character must wait at least 16 hours before starting another (SRD p.185), so a Long Rest started sooner gives no benefit and is not declined (the no-decline handling is this design's convention) | rest machine | A second long rest 10 hours after the first finished gives no benefit and is not declined; one that starts about 20 hours after the first finished gives its full benefits |
| D15 sleepers | The watch plan passes `awake[]` to combat entry; sleepers enter unconscious and surprised; a noise-wake question is a ruling | rest and combat | Four heroes, two on watch, a night ambush: two unconscious, two active |
| D16 combat time | Each round boundary advances `world_time` by 6 seconds (a round is about 6 seconds, SRD p.13) through the rules owner; `time_locked` applies to other advances | rules owner | Bless (Concentration, up to 1 minute, SRD p.113) cast in round 1 of a 3-round fight is expired after a 10-minute search |
| D17 hiding | A hidden hero is drawn with a hidden chip on the party's TV; hiddenness filters only the enemy view. Hide needs a DC 15 Dexterity (Stealth) check made while Heavily Obscured or behind Three-Quarters or Total Cover and out of every enemy's line of sight; on a success the hider has the Invisible condition while hidden, and the check total is the DC for a creature to find the hider with an active Wisdom (Perception) check (the Search action), not a compare against a static passive score (SRD p.183); a hider who is Invisible when Initiative is rolled has Advantage on the roll (SRD p.184) | projection | A rogue hides behind a pillar and the TV still shows the rogue. A hider's total of 17 is the DC for an enemy's Search: a Perception total of 16 does not find the rogue, 17 does. A hidden rogue at the start of combat rolls Initiative with Advantage |
| D19 kit and proficiency | A missing tool or language declines; missing proficiency proceeds without the bonus | requirements | No forgery kit: declined; a kit without proficiency: allowed, no bonus |
| D20 recall DC | Authored facts carry a DC; a generated fact is plainly known or has no recall DC; the retry key is `(actor, affordance id, method)` and reopens on any committed change to the scene, the affordance or the equipment | content admission | A failed desk search then a bookshelf search is a new task |
| D21 Help and group checks | Outside combat a Help (an ability check) needs the helper to have the named skill or tool proficiency, is consumed by the named task's next check, and lapses at the start of the helper's next turn if unused, with the GM deciding whether the assistance is possible (SRD p.182-183); a group check is called by the DM or a scene trigger and resolves in one commit with half rounded up (custom: SRD 5.2.1 has no group-check rule) | rules owner | Three heroes, two succeed: success; four heroes, two suffice. A helper without the proficiency cannot Help that check; an unused Help lapses at the helper's next turn |
| D23 light | `obscured` is derived from `light` and effects; visibility is computed per observer; the plate defaults to shared vision | layout | A dwarf and a human in a dark cave see different things on their own seats. Mixed vision for the shared view (RES24, question 74): a human carrying a lit Torch and a dwarf with Darkvision 120 feet (SRD p.84, p.180) stand in an unlit cave, and a statue stands 45 feet away, which is Darkness for the human and Dim Light for the dwarf (M2-D41). By default (union) the statue is in the shared view, so the plate is drawn with it lit as the dwarf sees it, the TV shows its hotspot and the plate prompt names it, and the human's phone lists it as public. With the host's option set to the lowest-vision member (intersection) the plate is drawn as the human sees it, the statue is not on the TV or in the plate prompt, and only the dwarf's phone shows it, as a seat-private overlay with "tell the party"; tapping that writes a party-scope grant in the same commit and the next plate shows it. In both settings no seat-private fact reaches the TV, and switching the option mid-scene changes only what the next plate and the TV hotspots show |
| D24 falling | The fall rule joins the rules-owner layout change | rules owner | A Shove off a 20-foot ledge deals 2d6 and lands prone |
| D26 action mapping | Intents map by rules entry id (Shove and Grapple are Unarmed Strike options inside Attack; Jump is movement) | action registry | "I grab him" resolves as a grapple; Extra Attack can replace one attack |
| D27 spell procedures | `casting_restricted` decline: on a turn a creature can expend only one spell slot to cast a spell (SRD p.105), so a cantrip never triggers it; ritual time routes through the time effect | rules owner | "Healing Word then Fireball" declines the second step. A bonus-action cantrip then a slotted action spell is allowed |
| D28 Heroic Inspiration | Suggested to the host for any heroic, dramatic or funny moment (not tied to flaws); held one at a time (SRD p.8); a second one earned while holding it is lost unless it is given to a player character in the group who lacks it (SRD p.8, p.183), which the holder chooses as a `pending{choice}` | rules owner | A hero already holding it earns another: not stacked, and it passes to a seatmate who lacks it when the holder gives it, or is lost when everyone holds one or the holder declines |
| D29 inventory | The rest machine can attune (no more than three items, SRD p.102); encumbrance is an R17 decision | rest machine | A fourth attunement is declined naming the three |
| D30 level-up | Offers queue and apply one at a time; HP roll or average is `pending{choice}` | advancement | Two awards before choices apply as two ordered level-ups |
| D31 plans across machines | A plan's lifetime outside combat is the scene generation; a hand-off inside a plan is allowed when the next step names its machine | pipeline | "Pay for a room and rest" ends with coin spent once and a rest started, or neither |
| D32 time cost | Affordances carry an authored `time_cost`; an unauthored one is a `RulingRequest` | affordance data | "Search the whole cellar" with no cost produces a pending ruling |
| D33 standard actions and opportunity attacks (C1) | Leaving an enemy's reach is the one trigger: when a creature the reactor can see leaves its reach using its action, Bonus Action, Reaction or one of its speeds, the reactor may take a Reaction for one melee attack with a weapon or an Unarmed Strike, and the attack occurs right before the creature leaves (SRD p.15, p.185); a Reach weapon adds 5 feet (SRD p.90). The rules owner holds the mover at the last cell inside the reach and opens a window (C.4): a seat gets a private `pending{choice}` (attack or let it pass), an engine reactor resolves in the same commit (W26), and phase two re-validates the mover and resumes the move only if it is still able. No window opens for the Disengage action (movement does not provoke for the rest of that turn, SRD p.181), for a Teleport (never provokes, SRD p.190), or for movement that does not use the mover's own movement, action, Bonus Action or Reaction, such as a push or a fall (SRD p.15); none opens when the reactor cannot see the mover, has no Reaction (it returns at the start of the reactor's own turn, SRD p.186) or is Incapacitated (SRD p.184). The other actions get one fixture each and are listed in `combat choices <actor>` with the economy they cost (a spent economy greys the entry with `economy_spent`): Dash (extra movement equal to the Speed after modifiers, SRD p.180), Disengage, Dodge (until the start of the actor's next turn attack rolls against it have Disadvantage when it can see the attacker and its Dexterity saves have Advantage, lost when Incapacitated or at Speed 0, SRD p.181), combat Help (an enemy within 5 feet of the helper; the next attack roll by one ally against it has Advantage; it lapses at the start of the helper's next turn, SRD p.183; ability-check Help is M2-D21), Search (a Wisdom check by what is sought: Insight, Medicine, Perception or Survival, SRD p.187), Study (an Intelligence check: Arcana, History, Investigation, Nature or Religion, SRD p.189) and Utilize (a nonmagical object use beyond the one free interaction per turn, SRD p.10, p.13, p.191). Free text: "I back off" is a move (a window if it leaves reach), "I slip away" is Disengage, and anything the registry cannot map goes through C.3. Shown by `pending show <id>` (the open window), `combat choices <actor>`, `spatial query` (kind reach: the reach cells and which path steps provoke) and the audit line `RULES opportunity_attack reactor=goblin_2 mover=pip trigger=leave_reach outcome=hit`. | rules owner; combat machine; window handler | Pip (Speed 30) starts next to a goblin whose Reaction is free and moves 10 feet away: the move halts at the exit cell, the goblin's attack resolves first, and Pip's move resumes only if Pip still stands. A move to another cell still inside the reach opens no window. Boundaries: the same move after Disengage opens none, and a move on the next turn opens one again; a Misty Step teleport and a Shove that pushes Pip 5 feet open none; a goblin that already took a Reaction this round, or cannot see Pip (Pip hidden, M2-D17), gets no window. Dash at Speed 30 allows 60 feet, and at Speed 15 (Exhaustion 3, M2-D44) 30. After Dodge a goblin's attack on Pip has Disadvantage, and the benefit is gone once Grappled sets Pip's Speed to 0. Sam's Help gives Pip's next attack against a goblin within 5 feet of Sam Advantage and not a second attack, and it lapses at the start of Sam's next turn if unused. A Study of a clue is an Intelligence (Investigation) check that spends only the action |
| D34 knocking out and stabilizing (C2) | When a melee attack (a weapon, an Unarmed Strike or a melee spell attack) would reduce a creature to 0 Hit Points, the attacker may instead reduce it to 1 Hit Point and give it the Unconscious condition; it then starts a Short Rest at the end of which the condition ends, and it ends early when the creature regains any Hit Points or someone takes an action to give first aid with a successful DC 10 Wisdom (Medicine) check (SRD p.17, p.184). The choice comes after the damage is rolled and before Hit Points are written, so APPLY holds the attack as a `pending{choice: knock_out}` to the attacker's seat (two-phase APPLY, C.4) with `knock_out` and `drop_to_zero` as the answers; a ranged attack, a spell that is not an attack and any other damage never offer it, and an engine attacker offers it only when encounter data marks the fight `nonlethal` (the carrier of M2-D1; the default is no). The creature is at 1 Hit Point, so it makes no Death Saving Throws and is not dying, and Unconscious applies in full (Incapacitated and Prone, Speed 0, attacks against it have Advantage, a hit from within 5 feet is a Critical Hit, SRD p.191); it stays Prone when the condition ends. Massive Damage is moot because the creature never reaches 0 (custom: SRD p.17 says "instead" and does not discuss the interaction). The Short Rest is one hour of `df-world` time held as an `ActiveEffect` expiry (RES5, SRD p.187); the SRD does not say whether the interruptions of a Short Rest (damage, Initiative, a spell above a cantrip) cancel that timer or whether the creature gains Short Rest benefits at its end, so the condition ends at the one-hour mark regardless and no Hit Dice are offered (custom: owner to confirm). Stabilizing a creature at 0 Hit Points is the Help action with a DC 10 Wisdom (Medicine) check (SRD p.18, p.10): on a success it is Stable (no Death Saving Throws, still Unconscious, the death save counts reset, SRD p.17-18); any damage ends Stable and is itself one failure; an unhealed Stable creature regains 1 Hit Point after 1d4 hours, drawn once with a `dice.drawn`; any healing ends Unconscious (SRD p.17). The SRD gives no distance for first aid, so the helper must be within 5 feet (custom); a monster at 0 Hit Points is dead unless encounter data treats it as a character (M2-D1). Free text: "I knock him out" or "I hit him with the flat" is an Attack with a `nonlethal` intent and the question is still asked when a hit would drop the target, so the words never set the outcome; "I bandage her" is the stabilize Help when the patient is at 0 Hit Points and `not_viable` otherwise. Shown by `pending show <id>`, `effects list <creature>` (Unconscious with `source=knock_out` and `expires_at`), `clock advance`, the event `character.status_changed{reason: knocked_out}` and the audit line `RULES knock_out attacker=pip target=goblin_1 hp_after=1`. | rules owner; combat machine; rest timer via `df-world` | Pip's longsword hit of 8 would take a goblin at 3 Hit Points to 0: Pip's seat gets `pending{choice: knock_out}`; chosen, the goblin is at 1 Hit Point and Unconscious with no death saves; `clock advance` of 59 minutes 59 seconds leaves it Unconscious and 1 hour ends it, Prone. Healing of 1 Hit Point ends it at once; first aid with a Medicine total of 10 ends it and 9 does not. A shortbow hit or a Fireball that would take it to 0 offers nothing, and an engine goblin hitting a hero in a fight with no `nonlethal` flag offers nothing (the hero goes to 0 and dying, M2-D1). Stabilize: a hero at 0 Hit Points with one failed death save, an ally's Help with a Medicine total of 9 changes nothing and spends the action, and 10 makes the hero Stable with the counts reset; 3 damage then ends Stable and records one failure; an unhealed Stable hero with a forced 1d4 of 3 regains 1 Hit Point after 3 hours |
| D35 cover (C3) | Cover is graded by what lies between the origin of an attack or effect and the target: Half Cover gives +2 to AC and Dexterity saving throws, Three-Quarters Cover +5 to both, and Total Cover means the target "can't be targeted directly"; a target gains cover only when the attack or effect comes from the opposite side, only the most protective degree applies and degrees never add, and a creature or an object can give Half Cover while Three-Quarters and Total come only from objects (SRD p.15, p.179). The grade is read from the layout `cover` field of the cells and set pieces on the line (D.6a, authored per M2-D2). The SRD gives no line algorithm, so the rules owner takes every cell the straight line from the origin cell to the target cell touches, takes the highest grade among them and counts a creature on the line as Half Cover (custom: cell tracing is this design's convention, RES37). A target behind Total Cover is dropped from direct attack and targeted-spell choices with `not_viable{reason: total_cover}` (a spell needs a clear path, SRD p.106); the cover object itself stays a legal target (M2-D36) and "attack a space" (RES37) is offered only into a space the attacker cannot see into. The Disadvantage on a ranged attack roll made within 5 feet of an enemy that can see the attacker and is not Incapacitated (SRD p.15) is not cover: it is Disadvantage on the roll and never changes AC. Free text: "I duck behind the table and shoot" is a move and an attack, and the cover is computed from the layout, never granted by the words. Shown by `spatial query` (kind cover: `origin`, `target`, `grade`, `sources[]`), `combat choices <actor>` (each target with its cover and total AC) and the event `spatial.query{kind: cover, result}`. | rules owner; layout | A goblin (AC 13) behind an overturned table (Half Cover) has AC 15, so a roll total of 14 misses and 15 hits, and its Dexterity save gains +2; behind a stone pillar (Three-Quarters) AC 18, total 17 misses and 18 hits, save +5; behind a closed door (Total) it is not listed as a target. A creature (Half) and a tree trunk (Three-Quarters) in line give +5 and not +7. An attacker beside the wall on the same side as the target gets no cover for the target. An archer next to a goblin that can see it shoots a second goblin 30 feet away behind a table: Disadvantage (adjacent enemy) and AC 15 both apply; if the adjacent goblin is Paralyzed, or cannot see the archer, the Disadvantage goes and the cover stays |
| D36 breaking objects, doors and locks (C4) | A set piece carries `material`, `size`, `fragile`, `auto_break` and optional `damage_threshold` and `lock{dc}` (D.6a). Armor Class comes from `material` (cloth, paper or rope 11; crystal, glass or ice 13; wood 15; stone 17; iron or steel 19; mithral 21; adamantine 23) and Hit Points from `size` and `fragile` (Tiny 2 or 5, Small 3 or 10, Medium 4 or 18, Large 5 or 27); an object is Immune to Poison and Psychic damage, has no ability scores and so fails every saving throw, and a damage threshold gives Immunity to any single hit below it (SRD p.178, p.180). An action breaks a fragile nonmagical object such as a glass container or paper automatically with no roll, through the Attack or Utilize action (SRD p.12, p.178); `auto_break` is the authored flag that says so, and a magical or unflagged object takes damage instead. Picking a Lock is a Utilize action with Thieves' Tools against DC 15, a Dexterity (Sleight of Hand) check (SRD p.94, p.98); no Thieves' Tools in the inventory is `missing_requirement`, and tools without proficiency proceed without the bonus (M2-D19). Forcing a door with a Strength check has no DC in the SRD (a Portable Ram gives +4, SRD p.99), so "I kick the door in" is `pending{ruling}` with the typed source-question key `force_object{object_id, method}` and a suggested DC tier, never a model-chosen number, and a stored ruling is reused only on key equality (RES48; custom: no door DC exists in the SRD). The path with a number is an attack roll against the object's AC with its Hit Points as above. Free text maps "I smash the bottle" to the auto-break, "I pick the lock" to the Utilize, "I kick the door in" to the ruling and "I burn through the door" to the fire damage path (M2-D41). Shown by `check show`, `dump --json` (object `hp`, `material`, `state`), `rulings show`, and the events `object.damaged{id, hp_after}` and `object.broken{id, how}`. | rules owner; authored set-piece data; ruling flow | A cellar door (wood, Large, resilient) has AC 15 and 27 Hit Points: a hit for 6 leaves 21, 26 leaves 1 and 27 destroys it, and Poison damage deals 0. A glass vial (Tiny, `auto_break`) breaks on a Utilize with no draw; the same vial with `auto_break` false (warded) takes damage instead. Picking the lock: no Thieves' Tools gives `missing_requirement` naming them; with the tools a Dexterity (Sleight of Hand) total of 14 fails and 15 opens it, and without proficiency the bonus is left out. "I kick the door in" opens `pending{ruling}`; a second kick at the same door by the same method reuses the stored ruling as "Ruling (earlier)" |
| D37 post-hit riders and weapon mastery (C5) | After a hit the rules owner lists the riders the attacker may add as one `pending{choice}` to the attacker's seat before the damage is written (D4, two-phase APPLY), each answer showing its cost and source: a weapon mastery property (Cleave, Graze, Nick, Push, Sap, Slow, Topple, Vex) is offered only when the character has a feature that unlocks it for that kind of weapon, such as Weapon Mastery (SRD p.90, p.48); Sneak Attack, once per turn, only with a Finesse or Ranged weapon and either Advantage on the roll or an ally within 5 feet of the target who is not Incapacitated while the attacker has no Disadvantage (SRD p.61-62); and Divine Smite, a level 1 spell cast as a Bonus Action right after a hit with a Melee weapon or an Unarmed Strike, for 2d8 Radiant damage (1d8 more against a Fiend or Undead, and 1d8 per slot level above 1), which the Paladin's Smite feature lets a Paladin cast once per Long Rest without a slot (SRD p.125, p.54). Smite is a spell, so it obeys M2-D27 and M2-D38: it needs the Bonus Action (`economy_spent` when spent), a slotted cast is `casting_restricted` when a slot was already expended this turn (SRD p.105), and the free cast expends no slot and is still offered. Mastery details: Topple forces a Constitution save at DC 8 plus the attack's ability modifier plus Proficiency Bonus; Push moves a Large or smaller target 10 feet straight away; Graze acts on a miss and deals the ability modifier; Slow cuts Speed by 10 feet and does not stack (SRD p.90). Riders that act on the same hit are ordered by the attacker (SRD p.187). Sneak Attack is counted per creature turn, so an Opportunity Attack on an enemy's turn has its own use (the SRD says only "once per turn", p.61; counting each turn separately is this design's reading). The list comes from the legal-choice call and not from the words: "I sneak attack" is an Attack, and the offer appears only if the rules allow it. Shown by `combat choices <actor>` (riders under `after_hit`), `pending show <id>` and the audit line `RULES rider_offered attacker=pip rider=sneak_attack eligible=true reason=ally_within_5ft`. | rules owner; class and weapon data | A Rogue with a Shortsword (Finesse) and Sam, a conscious ally within 5 feet of the goblin, hits: the 1d6 is offered and taken, and a second hit that turn (a Nick or Bonus Action attack) offers none; with Sam Paralyzed and no Advantage nothing is offered; Advantage and Disadvantage cancel, so no Advantage. A Fighter 2 and Paladin 2 casts Bless with the Magic action (a level 1 slot), uses Action Surge for another Attack action and hits: a slotted Divine Smite is `casting_restricted`, while the free Paladin's Smite is offered and allowed. A Fighter with Longsword mastery is offered Sap on a hit, and one who chose other weapon kinds is not; Topple at Strength +3 and Proficiency +2 is DC 13; Push is not offered against a Huge target |
| D38 casting procedure (C6) | The casting steps are checked in a fixed order by the rules owner: the spell is prepared or reached from a magic item or Scroll (SRD p.104); the components V, S and M are met (SRD p.105-106); a slot is chosen; the target has a clear path and is not behind Total Cover (SRD p.106); the slot is spent even when the target proves invalid (SRD p.106). Ritual: a prepared spell with the Ritual tag may be cast as a Ritual (a Wizard's Ritual Adept instead allows any Ritual-tagged spell in its spellbook, SRD p.78; the rest of that feature's text did not extract, so any further condition on it is unverified), which takes 10 minutes longer than the normal casting time, expends no slot and so cannot be cast at a higher level (SRD p.104, p.187; Alarm is 1 minute or Ritual, p.107); it runs as a timed activity through `RequestTimeAdvance` (RES5), is declined `time_locked` while combat or a pending resolution is open (RES29), and a spell of 1 minute or more needs the Magic action on each of the caster's turns and Concentration, failing without a slot spent if Concentration breaks (SRD p.105). Upcasting: when more than one slot level of at least the spell's level is available the caster answers `pending{choice: slot_level}`, and the spell takes the higher level for that casting with its own higher-level text applied (SRD p.104-105); a cantrip, or a caster with one eligible level, has nothing to choose. Components: a Verbal component is impossible when gagged or in magical silence; a Somatic component needs at least one hand; a Material component needs a free hand (it may be the Somatic hand), and a Component Pouch (a hand free) or a held Spellcasting Focus replaces unconsumed materials that have no listed cost, never costly or consumed ones (SRD p.105-106); casting in armor without training is impossible (SRD p.104, p.177). Refusals are `casting_restricted` with `reason` of `verbal`, `somatic`, `no_free_hand` or `armor_untrained`; a missing costly material is `missing_requirement` (C.3.5) and an unprepared spell is `not_viable{reason: not_prepared}`. The SRD does not say whether a hand that holds a weapon can make a Somatic gesture, so this design lets a hand that is free or holds the focus do it (custom: owner to confirm). Free text: "I cast Alarm on the door" is the Magic action with the spell and target, "as a ritual" sets the Ritual flag, and words are never matched against a table (RES46). Shown by `viable <seat>` (each spell with its `components` and the failing one), `combat choices <actor>`, `pending show <id>` (the `slot_level` choice), `clock advance` (ritual time) and the audit line `RULES cast spell=alarm slot=none ritual=true minutes=11`. | rules owner; spell data | A Ranger with Alarm prepared and level 1 slots at 2 of 2 casts it as a Ritual: 11 minutes of world time pass, the slots stay 2 of 2 and no `slot_level` choice is asked; with Alarm known but not prepared the Ritual entry is `not_viable{reason: not_prepared}`; started in combat it is `time_locked`; a Wizard with Alarm only in the spellbook may still cast it as a Ritual (Ritual Adept). Damage of 14 at minute 5 asks a Constitution save at DC 10 or half the damage, whichever is higher (DC 10 here, SRD p.179); a failure ends the cast with no slot lost. A wizard with a level 3 and a level 4 slot casts Fireball: `pending{choice: slot_level}` offers 3 and 4 and 4 deals 9d6 (SRD p.131); with only the level 3 slot left nothing is asked. A gagged wizard casting Fireball (V, S, M): `casting_restricted{reason: verbal}`, and the same in a Silence area. A wizard holding a Shield and a Longsword casting Alarm (M, a bell and silver wire): `casting_restricted{reason: no_free_hand}`, legal once the sword is stowed. A wizard in Plate Armor without training: `casting_restricted{reason: armor_untrained}` |
| D39 spell areas (C7) | An area is a shape from a point of origin whose size the spell fixes; the engine writes it onto the five-foot cells of the rules grid and lists what it catches in `caught[]` (R10, `spell.target_selected{op, origin, caught[]}`). Origins: a Sphere and a Cylinder include the origin point; a Cone, a Line, a Cube and an Emanation exclude it unless the creator decides otherwise, a Cube's origin lies on one of its faces, and an Emanation's origin is the creature or object that carries it and moves with it (SRD p.179-181, p.184, p.188). A location is in the area only if at least one straight line from the origin to it is not blocked, and only Total Cover blocks a line, so a creature behind Total Cover leaves `caught[]` while one behind Half or Three-Quarters Cover stays caught and takes the bonus on its Dexterity save (SRD p.177, p.15). If the caster places the origin at a point it cannot see and an obstruction such as a wall lies between, the origin comes into being on the near side of the obstruction and `origin_adjusted` is logged (SRD p.177). Fireball is the reference case (SRD p.131): range 150 feet, a 20-foot-radius Sphere centered on the chosen point, Dexterity save for 8d6 Fire damage and half on a success, and unworn, uncarried flammable objects in it start burning (M2-D41); the caster and allies are caught like anyone else, and the phone's confirm card lists them before the cast. The SRD defines shapes in feet and gives no grid rule for them, so the cell conversion is this design's declared function: a cell is in a Sphere when its center lies within the radius of the origin point, used identically for the phone's tap proposal and the rules owner's validation (custom, RES37). Shown by `spatial query` (kind area: shape, origin, `caught[]`, `excluded[{creature, reason}]`), `projection show <seat>` (the confirm card) and the event `spell.target_selected`. | rules owner; layout | Fireball centered on a cell: a goblin 20 feet away in a straight line (4 cells) is caught, one 25 feet away (5 cells) is not, and one at 3 cells by 3 cells (about 21.2 feet) is not. A goblin behind a 10-foot stone wall (Total Cover) 15 feet from the origin is in `excluded` with `total_cover`; one behind a table (Half Cover) is caught and its save gains +2. The caster aims at an unseen point 60 feet away behind a closed door: the origin lands on the near side of the door, `origin_adjusted{reason: unseen_point}` is logged and `caught[]` is computed from there. A 15-foot Cone does not catch its caster, and a Sphere whose origin is the caster's own cell does. A hero in the 20-foot radius shows in the confirm card as an ally caught, and the cast is not refused |
| D40 condition-driven legality (C8) | The legal-choice call (C.2) reads each active condition and changes `viable <seat>` and the target lists; a changed set counts as a change of legal choices, so a tap drawn from the older view is re-checked (H.2). One fixture per family. *Incapacitated* (SRD p.184): no action, Bonus Action or Reaction, so only free choices remain; Concentration is broken; it cannot speak, so Verbal spells and speech are not offered; Incapacitated at Initiative is Disadvantage. *Grappled* (SRD p.182): Speed 0 so no move; Disadvantage on attacks against anyone but the grappler; the escape action is listed (M2-D47). *Prone* (SRD p.186): the only movement is crawling or standing, which costs half the Speed rounded down (at Speed 0 it cannot stand); Disadvantage on its own attacks; attacks against it have Advantage from within 5 feet and Disadvantage from farther; dropping Prone is free (SRD p.14). *Blinded* (SRD p.177): it cannot see and automatically fails an ability check that needs sight, its attacks have Disadvantage and attacks against it Advantage; a target it cannot see is not a creature tap and "attack a space" is offered (RES37, SRD p.14), and a spell that needs a creature "you can see" is not offered. *Deafened* (SRD p.181): it automatically fails an ability check that requires hearing, shown as `auto_fail` with no roll; casting is not restricted by it (SRD p.105 names only a gag or silence). *Charmed* (SRD p.178): the charmer is not offered as a target of an attack or a damaging ability, and the charmer has Advantage on social checks against it. *Frightened* (SRD p.182): Disadvantage on ability checks and attack rolls while the source is in line of sight, and move choices that end closer to the source are greyed. An area effect that would only catch the charmer is a `pending{ruling}` (the SRD says "target", p.178). A refusal is `not_viable` with the condition named in the reason and never a new code. Shown by `viable <seat>`, `combat choices <actor>`, `effects list <creature>` and the audit line `RULES legality seat=2 condition=frightened removed=move:cell_7`. | rules owner; condition system | An Incapacitated hero's `viable` list holds only free choices, and a Bless it concentrates on ends when the condition applies. A Prone hero at Speed 25 stands for 12 feet of movement (half, rounded down), and at Speed 0 stand is not offered; a ranged attack on a Prone target from 5 feet has Advantage and from 10 feet Disadvantage. A Frightened hero 30 feet from an ogre at Speed 30 may move to cells at that distance or farther, a cell 5 feet closer is greyed, and when the ogre leaves line of sight the Disadvantage ends and the movement bar stays (the SRD ties only Disadvantage to sight). A Blinded hero's attack on the goblin it cannot see is an "attack a space" with Disadvantage that misses if the goblin is not there. A Deafened hero's Perception check to listen is `auto_fail`. A Charmed hero's attack targets omit the charmer, a goblin shaman, and keep the others |
| D41 light sources and burning (C9) | `light_source{bright_ft, dim_ft, burns_h}` is an item or effect field whose `dim_ft` is the extra Dim Light ring beyond the Bright radius, as the SRD words it: a Torch 20 and 20 for 1 hour (it is also a Simple Melee weapon that deals 1 Fire damage on a hit), a Candle 5 and 5 for 1 hour, a Lamp 15 and 30 and a Hooded Lantern 30 and 30 on Oil that burns 6 hours in all, not necessarily in a row, with the hood lowered by a Bonus Action to Dim Light in 5 feet (SRD p.96, p.98-100). Lighting one with a Tinderbox is a Bonus Action and lighting other fires takes 1 minute (SRD p.100). A lit source writes an `ActiveEffect` area into the layout that follows its bearer (D.6a); each cell's `light` is the brightest of the sources and the base light, where Bright Light is normal illumination, Dim Light makes a cell Lightly Obscured and Darkness makes it Heavily Obscured (SRD p.11, p.178, p.180-181), computed per observer with Darkvision (SRD p.180, M2-D23). `burns_h` runs on `df-world` time (RES5) and ends the effect with `layout.changed{cause: light_expired}`. Heavy rain and strong wind extinguish open flames (SRD p.195), so a lit Torch or Candle goes out when such weather starts (custom: the SRD does not say whether a Lamp or Lantern is an open flame, so this design treats a Torch and a Candle as open and a Lamp and a Lantern as shielded, owner to confirm). The Burning hazard: a burning creature or object takes 1d4 Fire damage at the start of each of its turns; a creature puts itself out with an action by dropping Prone and rolling, and the fire also ends when doused, submerged or suffocated (SRD p.178). A flammable object that is not worn or carried starts burning when hit by Fireball (SRD p.131) or Fire Bolt (G.3); an object has no turn in the SRD, so a burning object takes the 1d4 at the start of each combat round and each 6-second step of `df-world` time (custom). Free text: "I light the torch" is the Tinderbox's Bonus Action; "I wave the torch at the mummy" is an Attack with a Simple Melee weapon for 1 Fire damage, and whether that sets a target alight is a `pending{ruling}`. Shown by `spatial query` (kind light: the cell's grade and its sources), `effects list <creature>` (a `burning` effect), `clock advance`, `viable <seat>` and the audit lines `RULES light source=torch_1 bright=20 dim=20 burns_h=1` and `RULES burning target=barrel_2 roll=3 hp_after=15`. | rules owner; layout; weather via `df-world` | A Torch: the cell 20 feet away is Bright, 25 feet and 40 feet are Dim, 45 feet is Darkness; at 59 minutes it is lit and at 60 `layout.changed{cause: light_expired}` fires; a dwarf with Darkvision sees the 45-foot cell as Dim. Heavy rain starting puts a lit Torch out at once and `light.extinguished{cause: heavy_rain}` is logged. A wooden barrel (Medium, resilient, 18 Hit Points) lit by a Fireball loses 3 at the first round start (1d4 drawn as 3) and burns until destroyed or doused; a burning hero drops Prone and spends an action, and the fire ends at once on entering water |
| D42 traps: detect, disarm, reset (C10) | A trap is a set piece with the parts of SRD p.199: a severity and level range (nuisance or deadly), a `trigger` (J.3: pressure plate, trip wire, doorknob, wrong key), a duration and whether it resets, and with no reset it is inert after activating. It also carries `detect[]` and `disarm[]` (J.3): each is an action (Search for a Wisdom (Perception) check, Study for an Intelligence check, or a tool use), an ability and skill, a DC and what it reveals, as in the Poisoned Needle (detect: Search, DC 15 Wisdom (Perception); disarm: an action, DC 15 Dexterity (Sleight of Hand), and a failed attempt triggers the trap, SRD p.201) and the Fire-Casting Statue (detect DC 10 Perception for the glyph and DC 15 Perception for the plate, then a Study DC 15 Intelligence (Arcana), the glyph defaced as an action to disarm; it resets at the start of the next turn, SRD p.200). Thieves' Tools disarm a trap at DC 15 as a Utilize (SRD p.94); the tool is `requires` data on a trap only where its entry names it. Detection is an affordance of the trap's own set piece, so "I check the lock for traps" maps to the Search action with that piece as subject (RES6, C.3), and a trap that is not in the set is never invented. The first creature in the marching order to trigger it takes the effect (SRD p.12, M2-D43). A passive result is private: a creature whose Passive Perception (10 plus its Perception bonus, SRD p.186) meets a detect DC learns it privately on scene entry; the SRD says only that the GM uses the score to decide whether a creature notices something, so reading "meets the DC" is this design's convention (custom); a failure and "nothing to find" look the same to the seat (R04, K.15) and the TV shows only the neutral cue. Shown by `check show` (the private roll and its audience), `dump --json` (explore machine: trap `state` armed, inert or `resets_at`), the events `trap.detected{id, by, via}`, `trap.triggered{id, by}` and `trap.disarmed{id, by}`, and the audit line `RULES trap id=needle_1 trigger=lock_opened_improperly actor=pip`. | rules owner; authored trap data; explore machine | Poisoned Needle, detect DC 15: a Search total of 14 finds nothing and tells no other seat anything, 15 detects it; a hero with Passive Perception 15 is told privately on entry and one with 14 is not and sees no hint. Disarm total 14 triggers it: a DC 11 Constitution save, 5 (1d10) Poison damage and Poisoned for 1 hour on a failure, half damage on a success; 15 disarms it. A Knock on the lock leaves it untriggered. The Fire-Casting Statue fires again at the start of the next turn on a hero still on the plate; Poisoned Darts (resets while activated fewer than three times) fires three times and is then inert; Hidden Pit: after the first fall the lid stays open and a second creature crossing triggers nothing, and a detected pit with a spike wedged is safe to cross |
| D43 travel legs (C11) | A leg carries `terrain`, `pace`, `road`, `hours`, `mounted`, `vessel` and `marching_order[]` (F.3a). Pace is Fast, Normal or Slow at 4, 3 or 2 miles an hour (400, 300 or 200 feet a minute) and 30, 24 or 18 miles a day taken from the table, not by multiplying the hour figure by 8; Fast imposes Disadvantage on Wisdom (Perception or Survival) and Dexterity (Stealth) checks, Normal imposes Disadvantage on Dexterity (Stealth) only, Slow gives Advantage on Wisdom (Perception or Survival) checks; mounts can double the distance for 1 hour and then need a Short or Long Rest to repeat it (SRD p.12). The terrain's maximum pace caps the choice (Arctic Fast with equipment such as skis, Grassland Fast, Mountain and Swamp Slow, and Coastal, Desert, Forest, Hill, Underdark and Urban Normal), a good road raises the cap one step, a member whose Speed is reduced to half or less forces Slow, and flying or ignoring Difficult Terrain allows Fast anywhere (SRD p.192). Each terrain also has a Foraging, a Navigation and a Search DC and an encounter distance (Forest 10, 15 and 15, 2d8 times 10 feet; Mountain 20, 15 and 20, 4d10 times 10 feet; SRD p.192); the SRD does not name the ability for those three, so each is a `pending{ruling}` suggestion proposing Wisdom (Survival) for foraging and navigation and Wisdom (Perception) for searching with the table DC as the tier (custom, RES48). Extended Travel: beyond 8 hours, at the end of each extra hour each character makes a Constitution save at DC 10 plus 1 per hour past 8 (DC 11 for the 9th hour) or gains 1 Exhaustion level (SRD p.192, M2-D44), and an hour at 10,000 feet or higher counts as 2 for that purpose (SRD p.195). A vessel sets the speed in miles an hour and the table chooses no pace (SRD p.12, p.192): a Sailing Ship 2 mph and a Galley 4 mph (SRD p.101), halved against a strong wind, not moving under sail in a dead calm, and a Keelboat or Rowboat adds the current's 3 mph downstream (SRD p.100); a ship may travel up to 24 hours a day (SRD p.12). The marching order is an ordered seat list, changeable outside combat, read for who meets a trap or an ambush first (SRD p.12, M2-D42). A pace above the cap is `not_viable{reason: terrain_max_pace}` and a pace for a vessel is `not_viable{reason: vessel_sets_speed}`. Shown by `dump --json` on the travel machine (the leg and its DCs; H.16 has no `travel leg show` verb, and the `travel.leg` event carries the same fields), `clock advance`, `force travel <state>`, the event `travel.leg{terrain, pace, hours, dcs}` and the audit line `RULES travel terrain=forest pace=normal hours=8 miles=24`. | rules owner; travel machine; `df-world` | Forest with Fast chosen is `not_viable{reason: terrain_max_pace}` (cap Normal) and allowed on a good road; Mountain with Normal chosen is declined, and allowed on a road. A Normal leg through Forest gives Disadvantage on Dexterity (Stealth) and a Slow leg gives Advantage on Perception and Survival. A member at Speed 15 (Exhaustion 3, half of 30) forces Slow, and Speed 20 (Exhaustion 2) does not. A 10-hour day asks no save at hour 8, a Constitution save at DC 11 at the end of hour 9 and DC 12 at hour 10: a total of 10 at hour 9 gains Exhaustion 1 and 11 passes. A Sailing Ship covers 48 miles in 24 hours at 2 mph, moves 0 in a dead calm, 1 mph against a strong wind, and a pace choice is declined |
| D44 exhaustion (C12) | One cumulative condition `exhaustion{level}` on the creature's `ActiveEffect` list (R09): each time it is received the level rises by 1; the roll of every D20 Test (ability check, attack roll, saving throw) is reduced by 2 times the level; Speed is reduced by 5 feet times the level, to a floor of 0; the creature dies at level 6; finishing a Long Rest removes 1 level, and the condition ends at 0 (SRD p.181, p.185). A creature that died with levels returns from a revival with 1 fewer (SRD p.180). A natural 20 on an attack roll still hits and is still a Critical Hit, because the reduction applies to the total (SRD p.179). The SRD does not say whether a Death Saving Throw is a D20 Test (it is a saving throw not tied to an ability, SRD p.17), so the penalty applies to it as a saving throw unless the owner rules otherwise (custom). Sources, each an effect from the rules owner and never from narration: Dehydration (less than half the day's water: 1 level at day's end, not removable until the full amount is drunk, SRD p.181), Malnutrition (eating less than half the food: a DC 10 Constitution save or 1 level at day's end; nothing for 5 days: 1 level and one more each later day, SRD p.185), Suffocation (1 level at the end of each of its turns once the breath runs out, all removed when it can breathe, SRD p.189), Extended Travel (M2-D43), Deep Water, Extreme Cold, Extreme Heat and Frigid Water (SRD p.195) and the contagions of SRD p.194 (M2-D52). `df-world` day boundaries deliver the day-end sources as engine inputs (RES5). A member whose Speed falls to half or less forces a Slow pace (M2-D43). Shown by `effects list <creature>` (level, Speed after, penalty), `check show` (a modifier line `exhaustion -2n`), `clock advance` and the audit line `RULES exhaustion creature=pip level=3 source=extended_travel`. | rules owner; `df-world` day boundary | At level 2 an attack with a d20 of 15 and a +5 bonus totals 16 and misses AC 17, while a natural 20 still hits; Speed 30 is 20 at level 2 and 5 at level 5, Speed 25 at level 5 is 0, and level 6 is dead at once. A Long Rest at level 3 leaves level 2, and at level 1 ends the condition. A Medium hero who drinks exactly half a gallon gains no level; just under half gains 1 at the day's end, and a Long Rest does not remove it until a full gallon is drunk. A hero revived after dying at level 3 returns at level 2 |
| D45 movement modes, water and falling into it (C13) | Movement costs 1 foot per foot, plus 1 extra for each of Difficult Terrain (including liquid between shin and waist deep, SRD p.181), climbing, crawling and swimming (SRD p.178-179, p.189); climbing, crawling or swimming in Difficult Terrain costs 2 extra feet per foot (3 in all), and a Climb Speed or Swim Speed used for its mode removes that mode's extra foot (SRD p.178, p.189). Jumping spends movement foot for foot: a Long Jump covers up to the Strength score in feet and a High Jump 3 plus the Strength modifier (minimum 0), each only after at least 10 feet of movement on foot immediately before and half as much from a standing start; landing in Difficult Terrain after a Long Jump is a DC 10 Dexterity (Acrobatics) check or Prone (SRD p.183-185). A creature can hold its breath for 1 plus its Constitution modifier in minutes (minimum 30 seconds), then gains 1 Exhaustion level at the end of each of its turns until it can breathe, and loses every level gained that way when it can (SRD p.189, M2-D44); deep water over 100 feet needs a DC 10 Constitution save each hour for a creature with no Swim Speed (SRD p.195). A creature that falls into water can use its Reaction for a DC 15 Strength (Athletics) or Dexterity (Acrobatics) check, its choice, to halve the fall damage, and it lands Prone unless it took none (SRD p.182, M2-D24). Rough water or a slippery surface calling for a DC 15 Strength (Athletics) check is the GM's option in the SRD (p.179, p.189), so it applies only where the set piece's authored `rough` or `slippery` flag says so (D.6a). Free text: "I dive off the quay and swim for the boat" is a move in swim mode over `terrain: water` cells and "I leap the gap" is a jump the rules owner prices from the Strength score; neither invents a check. Shown by `spatial query` (kind move_cost: feet per cell, mode and total), `viable <seat>` (the farthest legal cell), `effects list <creature>` (a `holding_breath{ends_at}` effect) and the audit lines `RULES move mode=swim ft=15 cost=30` and `RULES suffocation creature=pip level=1`. | rules owner; layout | Speed 30 swimming in calm water covers 15 feet; in a Difficult Terrain cell (waist-deep water) each foot costs 3 so 10 feet; with a Swim Speed of 30 it covers 30. Climbing a wall covers 15 feet and crawling 15. Strength 16 (+3): a Long Jump of 16 feet after a 10-foot run and 8 standing, a 9-foot run counts as standing, a High Jump of 6 feet and 3 standing; Strength 4 (-3) jumps 0 feet high. Constitution +2 gives 3 minutes (30 rounds) before suffocation begins, and Constitution -3 gives the 30-second minimum (5 rounds); at the end of the first turn after that Exhaustion 1 applies and surfacing removes it. A hero pushed off the 40-foot quay into the harbour takes 4d6 Bludgeoning; a Reaction check total of 14 leaves it whole and 15 halves it, no check is offered with the Reaction already spent, and a hero who takes no damage lands upright |
| D46 rest start, interruption and the Short Rest loop (C14) | Aligned with RES27 and M2-D14 and not repeating them: both rests need at least 1 Hit Point to start (SRD p.185, p.187). A Short Rest is a 1-hour period in which the creature does nothing more strenuous than reading, talking, eating or standing watch; Initiative rolled, a spell above a cantrip cast or any damage taken interrupts it, and an interrupted Short Rest gives no benefits; the hour of walking or other exertion interrupts a Long Rest only (SRD p.187), so RES27's "interrupted the same way" is read with that difference. The SRD does not say how a Short Rest resumes, so an interrupted one is restarted from zero (custom). A Long Rest interrupted after at least 1 hour gives Short Rest benefits and may resume at once, needing 1 additional hour for each interruption (SRD p.185, RES27), read here as 8 hours plus 1 per interruption in all; before 1 hour it gives nothing. Short Rest benefits come from the Hit Dice loop: repeated `pending{choice: spend_hit_die}` to the character's seat, each answer rolling the chosen die with a `dice.drawn` and adding the Constitution modifier for at least 1 Hit Point, the player stopping after any roll or going on while dice remain, and features that recharge on a Short Rest do so (SRD p.187). A finished Long Rest regains all lost Hit Points and all spent Hit Dice, never half (SRD p.185). Free text: "we take a short rest" proposes the rest through the pipeline with the watch plan (M2-D15), and it never starts while a pending resolution exists (`pending_open`) or a hero is dying (M2-D13). Shown by `force rest <state>`, `dump --json` (rest machine: state, `hours_rested`, `interruptions`, `dice_left`), `clock advance`, the events `rest.started`, `rest.interrupted{cause, hours}` and `rest.completed`, and `pending list` for the loop. | rest machine; rules owner | A hero at 12 of 20 Hit Points with 2 d10 Hit Dice and Constitution +2 draws 4 on the first die (6, so 18 of 20) and may stop; a second draw of 1 gives 3 and the total is capped at 20; with Constitution -1 a draw of 1 totals 0 and is raised to 1. A Short Rest started at 0 Hit Points is `not_viable{reason: needs_1_hp}`. An arrow at minute 40 ends the rest with no Hit Dice offered; a Light cantrip at minute 40 does not interrupt it and a level 1 Cure Wounds does. A Long Rest interrupted by Initiative at 59 minutes gives nothing, and at 60 minutes gives Short Rest benefits and, if resumed, finishes after 9 hours in all. A Long Rest finished with 1 of 3 Hit Dice left restores all 3 |
| D47 grapple and shove (C15) | Both are options of the Unarmed Strike, a melee attack against a target within 5 feet, and replace one attack of the Attack action (M2-D26, SRD p.190). The target chooses Strength or Dexterity (`pending{choice: save_ability}` to its seat; the rules owner picks the higher modifier for an engine creature, custom) and must meet DC 8 plus the attacker's Strength modifier plus Proficiency Bonus, the same DC as escape (SRD p.190). The target must be no more than one size larger than the attacker for both, and a grapple needs a free hand (or the body part a monster grapples with), one creature per hand (SRD p.182, p.190). A failed save makes the target Grappled: Speed 0, Disadvantage on attacks against anyone but the grappler, and the grappler may drag or carry it, each foot costing the grappler 1 extra foot unless the target is Tiny or two or more sizes smaller (SRD p.182). The Grappled creature escapes with an action and a Strength (Athletics) or Dexterity (Acrobatics) check against the escape DC; the grapple also ends when the grappler is Incapacitated, when the distance exceeds the grapple's range, or when the grappler releases it at no cost (SRD p.182). A failed shove save lets the attacker choose a push of 5 feet away or the Prone condition (`pending{choice: shove_effect}` to the attacker's seat, SRD p.190); a push is movement the target does not make with its own action, so it provokes no Opportunity Attack (M2-D33), and a push off a ledge falls under M2-D24. Declines are `not_viable` with the reason `size`, `no_free_hand` or `out_of_reach`. Free text: "I grab him" and "I shove him into the canal" map to Unarmed Strike options (M2-D26) and still reach the save; the words never set the effect. Shown by `combat choices <actor>` (`unarmed_strike` with `damage`, `grapple` and `shove`, each target and any reason), `check show`, `effects list <creature>` (Grappled with source and escape DC), the event `effect.applied{origin}` and the audit line `RULES grapple attacker=brakka target=goblin_1 dc=14 save=dex total=13 result=grappled`. | rules owner; combat machine | Brakka (Strength +4, Proficiency +2, DC 14) grapples a goblin (Dexterity +2, Strength -1): the goblin picks Dexterity, a save total of 13 fails (Grappled) and 14 succeeds. A Medium attacker may grapple a Large ogre and not a Huge giant (`not_viable{reason: size}`). A hero holding a two-handed Greatsword has no free hand and the grapple is declined, and it is offered once the weapon is unequipped as part of the Attack action (SRD p.177). The goblin's escape check of 13 fails and 14 frees it; an ogre that becomes Incapacitated frees its captive at once. A Medium grappler with Speed 30 drags a Small goblin 15 feet, and a Tiny captive costs no extra. A shove that fails and sets Prone gives the next melee attack within 5 feet Advantage; a 5-foot push out of a third creature's reach provokes nothing |
| D48 magic items, potions and dawn charges (C16) | Attunement is a Short Rest spent focused on only that item while touching it, and not the same Short Rest used to learn its properties; an interrupted rest fails the attempt (SRD p.102), so the rest machine (M2-D29, M2-D46) offers `attune{item}` as the rest's focus and completes it at the end. At most three items at a time and never two copies of one item; it ends when the prerequisites are lost, when the item has been more than 100 feet away for at least 24 hours, when the creature dies (a revived creature is not attuned, SRD p.180) or when another creature attunes to it, and the creature can end it with another Short Rest focused on the item unless the item is cursed, a cursed item's attunement not ending until the curse is broken, for example by Remove Curse (SRD p.102-103, p.206). The Identify spell or one Short Rest focused on an item reveals its properties but not a curse (SRD p.102, D.7 SecretPolicy), so a cursed item shows no `end_attunement` choice and its curse stays private until it shows itself. A potion is drunk or given to another creature as a Bonus Action, takes effect at once and is used up (SRD p.204), and `economy_spent` is the decline when the Bonus Action is gone. "I mix the potions" goes to the Potion Miscibility table and is not invented: the rules owner rolls 1d100 for the first mix (`dice.drawn`) and again for each extra potion, and the result applies (01 an explosion, 4d10 Force in a 5-foot-radius Sphere centered on the mixer; 02 to 08 both become an ingested poison of the adjudicator's choice; 09 to 15 both lose their effects; 16 to 25 one loses its effect; 26 to 35 both halved; 36 to 90 normal; 91 to 99 one doubled; 00 one works permanently, the simplest or most fun effect), and only the two adjudicator choices (the poison from the catalog of M2-D52, the permanent effect) are `pending{ruling}` keyed `mix_potions{item_a, item_b}`; effects that are not obvious show only when they become evident (SRD p.204). Daily charges: an item with `charges{max, regain}` regains expended charges at the next dawn (1d6 for a Cube of Force, which starts with 10, SRD p.206, p.216) when `df-world` delivers `Dawn{day}` as an engine input; each regain is a `dice.drawn`, never above the maximum, and a plane with no dawn leaves the time to the host (SRD p.206). Shown by `dump --json` (item `attuned_to`, `charges`), `effects list <creature>`, `pending list`, `clock advance`, `viable <seat>` and the audit lines `RULES attune item=ring_of_protection seat=2 hours=1` and `RULES charges item=cube_of_force before=4 roll=3 after=7 cause=dawn`. | rest machine; rules owner; ruling flow; `df-world` dawn | A Short Rest broken by damage at minute 59 fails the attunement, and one Short Rest cannot both identify and attune. A second Ring of Protection is `not_viable{reason: second_copy}`. An item left 23 hours 59 minutes away is still attuned and at 24 hours the attunement ends, and a ring in a companion's pack within 100 feet counts as near. A cursed item shows no end option, and the Short Rest option appears after Remove Curse. A Potion of Healing drunk as a Bonus Action followed by a second potion in the same turn is `economy_spent`. Mixing two potions: a roll of 36 works both, 35 halves both, 91 doubles one, 1 explodes (4d10 to everyone in the 5-foot sphere, the mixer included), 5 opens `pending{ruling}` for the poison with the sample poisons as the permitted alternatives. A Cube of Force at 4 charges with a draw of 3 has 7; at 8 charges with a draw of 6 it has 10 |
| D49 armor, weapon properties and ammunition (C17) | Armor takes time to put on: Light 1 minute to don and doff, Medium 5 minutes to don and 1 to doff, Heavy 10 to don and 5 to doff, and a Shield is a Utilize action to don or doff (SRD p.92); the time goes through `RequestTimeAdvance` (RES5), so donning in combat is declined `time_locked` (RES29). Without training in the armor a creature has Disadvantage on every D20 Test that uses Strength or Dexterity and cannot cast spells (`casting_restricted{reason: armor_untrained}`, M2-D38), and an untrained Shield gives no AC bonus (SRD p.92, p.177); armor with a Strength entry slows the wearer by 10 feet unless the Strength score meets it (Chain Mail 13, Splint Armor and Plate Armor 15) and the Stealth column gives Disadvantage where it says so (SRD p.92). A missing weapon proficiency is not a penalty: the Proficiency Bonus is simply not added (SRD p.89, M2-D19). Weapon properties show in the legal choices: Ammunition needs ammunition, each attack expends one piece, drawing it is part of the attack and loading a one-handed weapon needs a free hand, and after a fight 1 minute recovers half the pieces used, rounded down, the rest being lost (SRD p.89); Loading allows one piece fired per action, Bonus Action or Reaction whatever the number of attacks (SRD p.90); Heavy gives Disadvantage below 13 in the ability used; Two-Handed needs both hands, Versatile one or two, and Thrown draws as part of the attack (SRD p.89-90). Out of ammunition is `resource_spent{reason: no_ammunition}` and a second Loading shot in one action is `economy_spent{reason: loading}`. Free text: "I put on my armor" is the don time cost and "I fire again" meets the Loading limit. Shown by `combat choices <actor>` (`ammo`, `loading`), `viable <seat>`, `effects list <creature>` (an `armor_untrained` line), `check show`, `clock advance` and the audit line `RULES ammo weapon=longbow used=1 left=13`. | rules owner; equipment data | A hero with Strength 12 in Chain Mail has Speed 20 and with Strength 13 has 30. Donning Plate Armor takes 10 minutes of world time outside combat and is `time_locked` in combat; doffing Chain Mail takes 5. A hero untrained in Plate Armor has Disadvantage on a Strength save and none on a Wisdom save, and a Fireball cast is `casting_restricted`. A Longbow with 20 arrows, 7 used in a fight: after 1 minute 3 are recovered (7 halved is 3.5, rounded down), the quiver holds 16 and 4 are lost, and 1 used recovers 0. A Fighter with Extra Attack and a Heavy Crossbow (Loading): the second attack of the Attack action is not offered with the crossbow (`economy_spent{reason: loading}`), and Action Surge's extra Attack action fires one more bolt. A Strength 12 hero with a Greatsword (Heavy) has Disadvantage and one with Strength 13 does not |
| D50 species traits in play (C18) | A species trait is data the rules owner reads, never narration. Elf Trance (SRD p.85): the elf does not need to sleep and finishes a Long Rest in 4 hours spent in trancelike meditation while keeping consciousness, so the rest machine (M2-D14, M2-D46) reads `rest_override{long_rest_h: 4, conscious: true}` and completes that character at hour 4 while others in the same rest complete at their own hour (a Human needs 8 hours with at least 6 asleep, SRD p.185); the trancing elf is never Unconscious in the rest (Unconscious comes with sleep, SRD p.185), so it is not a sleeper for M2-D15, and the 16-hour wait, the interruptions and the additional hour per interruption of SRD p.185 still apply to it. Human Resourceful (SRD p.86): the character gains Heroic Inspiration whenever it finishes a Long Rest, produced as one rules-owned award keyed `(character, rest_id)` so a replay or restart grants it once (R16 `AwardId`), and it obeys the Only One at a Time rule (SRD p.8, p.183): a holder who is granted another gets `pending{choice: give_inspiration}` to give it to a player character in the group who lacks it, and it is lost when everyone holds one or the holder declines (M2-D28). The sidebar of SRD p.8 says Humans "start each day" with it, a gloss; the trait on p.86 says "whenever you finish a Long Rest" and governs, so two Long Rests in one day are two grants. Whether the elf's "magic can't put you to sleep" blocks the Unconscious from a sleep spell is not stated at condition level, so it is a `pending{ruling}` keyed to the spell entry (custom). Shown by `dump --json` (rest machine: per character `ends_at_h`), `clock advance`, `pending list`, and the events `rest.completed{seat, hours}` and `award.applied{award_id, kind: heroic_inspiration, source: human.resourceful}`. | rest machine; rules owner; award effect | An Elf and a Human rest together: after `clock advance` of 4 hours the elf has all Hit Points, spent Hit Dice and slots back and Exhaustion down by 1 and the human has not; at 7 hours 59 minutes the human has not and at 8 hours has. An elf that stops at 3 hours 59 minutes gains nothing; an elf damaged at 3 hours gains Short Rest benefits and, resuming, finishes after 5 hours in all. A Human with no inspiration finishing a Long Rest holds it; a Human already holding it gets `pending{choice: give_inspiration}` listing seatmates without it, and when all hold it the award is lost and logged; a restart after the commit grants nothing twice |
| D51 monster procedures (C19) | Recharge X to Y: the part can be used once; at the start of each of the monster's turns, while it is spent, the engine draws 1d6 (`dice.drawn{op, draw_ids, values}` naming creature and ability) and the part is regained when the roll is within X to Y; it also recharges when the monster finishes a Short or Long Rest (SRD p.257). X per Day: that many uses, regained after a Long Rest; "recharge after a Short or Long Rest" is one use per rest (SRD p.257). A stat-block ability carries `usage{recharge{min, max}, per_day{n} or after_rest}` (J.3), and monster state holds `available` or `uses_left`. The roll is a start-of-turn effect ordered with the others by M2-D6 and is made only while the ability is spent, never "to see". Initiative: a group of identical creatures (one `stat_ref`) makes a single roll and each member has the same Initiative count, the order among tied monsters being the GM's choice, which is the engine (SRD p.13, M2-D8); when members of the group differ in surprise (SRD p.13) the group splits into one roll per surprise state (custom: the SRD gives one roll for a group and does not say how surprise applies to it). Shown by `dice force` in tests, `dump --json` (combat machine: per ability `available`, per group `initiative`), `combat choices <actor>` (a spent ability absent with `resource_spent`) and the audit lines `RULES recharge creature=blink_dog_1 ability=teleport roll=3 range=4-6 regained=false` and `RULES initiative group=goblin roll=14 members=3`. | combat machine; rules owner | A Blink Dog uses Teleport (Recharge 4 to 6, a Bonus Action, SRD p.266): at its next turn start the draw is 3, so it is not regained and `combat choices` omits Teleport; the next draw is 4 and it is regained (4 is in, 3 is out). A Short Rest between the turns recharges it with no draw. A dragon's Fire Breath (Recharge 5 to 6, SRD p.268) regains on 5 and not on 4. A creature with Legendary Resistance (3 per Day) that uses one has 2 at its next turn and 3 after a Long Rest (M2-D9). Three goblins of one stat block roll once, 14, and all have count 14 in an engine-chosen order; a goblin boss of another stat block rolls separately; four wolves in an ambush with two surprised roll twice, the surprised pair with Disadvantage (custom) |
| D52 poison, curses and contagion (C20) | Poison is catalog data of four types (SRD p.197): contact (potent until touched or washed off, exposing a creature that touches it with exposed skin), ingested (a whole dose swallowed; the host may allow a partial dose Advantage on the save or half damage), inhaled (a 5-foot Cube, holding the breath does not help) and injury (applied to a weapon or ammunition as a Bonus Action, potent until delivered through a wound or washed off, and delivered when the coated object deals Piercing or Slashing damage). A dose is an item with a `poison_ref`; applying it spends the Bonus Action and a delivering hit consumes it, while a miss or a Bludgeoning hit does not. Each catalog poison resolves with its own save, damage and Poisoned duration (Serpent Venom, injury: DC 11 Constitution, 10 (3d6) Poison damage and half on a success, SRD p.198), and the Poisoned condition gives Disadvantage on attack rolls and ability checks (SRD p.186). Harvesting: a dead or Incapacitated venomous creature, 1d6 minutes, then a DC 20 Intelligence (Nature) check with a Poisoner's Kit; a success gives one dose and no more from that creature, a failure gives none, and a failure by 5 or more (a total of 15 or less) exposes the harvester to the creature's poison (SRD p.197). Contagion: each is a rules effect with an infection save, an onset (1d4 days for Sewer Plague and Cackle Fever), a daily or per-Long-Rest save and effects that hook other rules; Sewer Plague is DC 11 Constitution, Exhaustion at onset, half Hit Dice healing and no Long Rest recovery while any Exhaustion remains, and a daily-at-dawn DC 11 save that adds a level on a failure, removes one on a success and ends the contagion at 0 (SRD p.194), delivered by the `Dawn{day}` input of M2-D48; 3 days of rest earn one DC 15 Constitution save for Advantage against the contagion for 24 hours (SRD p.194). Curses are effects whose own entry defines them, removed only by Remove Curse, Greater Restoration or magic that explicitly ends curses (SRD p.179, p.193; items in M2-D48). A poison, curse or disease that is not catalogued is not invented: the line becomes `pending{ruling}` keyed `substance{description}` with the nearest catalog entries as the permitted alternatives, and a model never gives it a DC (RES48). Shown by `effects list <creature>` (Poisoned, and a contagion with its next save time), `check show`, `dump --json`, `clock advance`, `pending list`, the event `effect.applied{origin}` and the audit line `RULES poison dose=serpent_venom target=goblin_1 save=con dc=11 total=10 result=failed`. | rules owner; catalog data; ruling flow; `df-world` dawn | A Rogue coats a Dagger with Serpent Venom as a Bonus Action: a miss keeps it, a Piercing hit delivers it and the goblin's Constitution save of 10 fails (3d6 Poison drawn) while 11 passes for half, the dose is gone after the first damaging hit, and a Club hit never delivers it. Harvest from a venomous corpse with a Poisoner's Kit: 1d6 drawn as 4 gives 4 minutes; a Nature total of 20 gives a dose, 16 gives nothing and 15 exposes the harvester to a Constitution save per the creature's entry; a second harvest from the same corpse is `not_viable{reason: already_harvested}` and no kit is `missing_requirement`. Sewer Plague at dawn at Exhaustion 1: a DC 11 save total of 11 succeeds, level 0 and the contagion ends; 10 fails to level 2; a Long Rest between neither restores Hit Points nor lowers Exhaustion while infected. "I dip my arrows in nightshade" opens `pending{ruling}` offering Serpent Venom or Spider's Sting and invents no poison |

The register of M2-D2, by carrier. *Authored affordance data*: whether a roll is needed, the ability and
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
stored (K.2) and waits at most `scene_build_budget_s` (F.3); and RES31 now lets explore and travel
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
| P20 with M2-D18 | RES47 (no M.2 row) |
| P21 with M2-D25 | RES48 (no M.2 row) |
| P22, P29, P32 | RES49 |
| P23, P35 | RES50 |
| P24, P25 | RES51 |
| P19, P30, P31, P34 | RES52 |
| M2-D1 to M2-D52, except M2-D18, M2-D22 and M2-D25 | M.2 register rows; M2-D22 (with P3) is in RES37, M2-D18 in RES47 and M2-D25 in RES48, which have no row |
| Rules gaps C1 to C20 (GitHub issues #1547 to #1566) | M2-D33 to M2-D52 in issue order (C1 is M2-D33, C20 is M2-D52); data fields in D.6a, F.3a and J.3; the coverage matrix is M.5 |
| K1 to K10 | M.3 and the inline edits it lists |
| K11 to K35 | M.3 and I.2 |
| F1 to F28 | section L rewritten; F7 in RES30 and L.6 slice 1; F9 in L.6 slice 0b; F5 in L.2 and RES26 |

**M.5 Rules-area coverage matrix.** Which row or section owns each rules area after M2-D33 to M2-D52.
The third column is honest: "owned" means a row with a test case exists, "thin" says what is missing,
and `none yet` means nothing owns it. A row is a fixture candidate and not a finished implementation,
and `rules-coverage.md` still decides completeness against the pinned catalog.

| Area | Owner | Status |
|---|---|---|
| Initiative | M2-D8 (order, ties, surprise, joiners), M2-D51 (one roll for a group), F.1, R05 | owned |
| Actions | RES8, C.3, M2-D26, M2-D33 | owned |
| Bonus actions | C.2 (CHECK validates what remains), R05, and exercised by M2-D27, M2-D37, M2-D48 | thin: no row tests "one Bonus Action per turn" (SRD p.177) on its own |
| Reactions | C.4, M2-D5, M2-D7, M2-D33 | owned |
| Movement and terrain | RES1, D.6, D.6a, M2-D24, M2-D33, M2-D45 | thin: mounting and dismounting cost and controlling a mount (SRD p.15), and moving through other creatures' spaces (SRD p.14), are `none yet` |
| Cover | M2-D35 | owned |
| Vision and light | M2-D17, M2-D23, M2-D41, D.6 | thin: Blindsight, Tremorsense and Truesight (SRD p.177, p.190) are `none yet` |
| Conditions | M2-D40 (Incapacitated, Grappled, Prone, Blinded, Deafened, Charmed, Frightened), M2-D12, M2-D17 (Invisible), M2-D34 (Unconscious), M2-D44 (Exhaustion), M2-D47 (Grappled escape), M2-D52 (Poisoned) | thin: Paralyzed, Petrified, Restrained and Stunned have no fixture (SRD p.186-189) |
| Exhaustion | M2-D44 | owned |
| Concentration | M2-D12, M2-D38 | owned |
| Death and dying | M2-D1, M2-D13, M2-D34 | owned |
| Rests | M2-D14, RES27, M2-D46, M2-D50 | owned |
| Damage types | R08, M2-D11 (temporary Hit Points), M2-D36 (object Immunities) | none yet: Resistance, Vulnerability and Immunity and their order of application (SRD p.17) have no row |
| Grappling and shoving | M2-D26, M2-D47 | owned |
| Spellcasting | M2-D5, M2-D12, M2-D27, M2-D38, M2-D39, R10, R11 | owned; one fixture per spell entry stays R11 |
| Class features | M2-D37 (post-hit riders), M2-D50, R02 and R11 in `rules-coverage.md` | thin: only the rider families have rows; per-class features are R02 and R11 fixtures |
| Species | M2-D50 (Trance, Resourceful), M2-D28, R03 | thin: other species traits have no row |
| Equipment | M2-D19, M2-D29, M2-D49, D.7 | owned; encumbrance is an R17 decision |
| Magic items | M2-D29, M2-D48 | owned; item-specific behavior is R11 |
| Monsters | M2-D1, M2-D9, M2-D51, R15 | thin: Multiattack and the monster rule "cast at the lowest level" (SRD p.257) are `none yet` |
| Legendary and lair | M2-D9 | owned; the lair entry is custom |
| Traps and hazards | M2-D42 (traps), M2-D41 (Burning), M2-D24 (Falling), M2-D45 (Suffocation), M2-D44 (Dehydration, Malnutrition) | thin: the Environmental Effects of SRD p.195 other than heavy rain, strong wind and deep water (extreme cold and heat, frigid water, high altitude, slippery ice, thin ice) are `none yet` |
| Social | RES47, M2-D21, C.3, F.2 | thin: fear effects and mental stress (SRD p.196) are `none yet` |
| Exploration and travel | M2-D17, M2-D20, M2-D23, M2-D32, M2-D43, F.3, F.3a | owned |
| Downtime | F.6, R17 | none yet: crafting, brewing and scribing (SRD p.103) register only when R17 marks them required |
| XP | R16, F.1 exit awards, M2-D30 (level-up) | thin: the XP thresholds (SRD p.23) and the encounter XP budget (SRD p.202) have no row |

### O. The `df-table` crate plan

*Note on lettering: the sections run A to M and then O, and there is no section N. The ids N1 to N24
are not a section: they are the finding ids of the round 1 quality critic (see "How the loop runs"),
and the ledger in section I traces each to its place (for example N1 to H.1). Section O was lettered
after M and skips N so that a finding id such as N16 cannot be read as a section reference. Section
names are not renumbered.*

This is the plan for the first crate of the from-scratch engine, written after the old gameplay code
was removed from `df-tools` on branch `gameplay-engine-v2` (commit `14a2f715`). It turns the design
above into modules, types, seams and tests. Its two drivers, from the owner, are **flexibility**
(a refinement or a new machine, action, setting or resource should be a data entry or one module,
and the compiler or the conformance suite should say what was forgotten) and **observability**
(every decision is a typed event that explains itself, nothing private can reach a log by accident,
and every piece of state can be inspected). Only the first host slice, a blank-canvas workbench, is
built (O.12); the machines, pipeline, owner queue, projection and the rest of this plan are not built
yet, and O.12 lists where the built slice departs from the plan below.

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
- **Dependencies today.** The built `crates/df-table` depends on `serde` and `serde_json` only (its
  `Cargo.toml`; a `testkit` feature adds nothing). It has no edge to `df-types` or `df-model`, and the
  three new crates (`df-table`, `df-table-host`, `df-playground`) are already workspace members while
  question 87, the approval of the new crate edges, is still open.
- **Dependencies it may have (planned, pending question 87).** `df-types` (ids, `SessionRevision`),
  `df-model` (the checkpoint,
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
| `pending` | pending resolutions held in table state, two-phase apply, windows, answers | RES26, C.4, M2-D5 to M2-D7 of M.2 |
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
  the caller. The reports are the data behind the CLI verb set, which is the H.16 list (RES33: "the
  H.16 verbs as a fixed set", with `describe` generated from the registry; the earlier "nine verbs"
  were a subset of it, and the dev CLI implements only some verbs today, see O.12). The host serves
  them; the crate defines them, so a machine without a report fails the conformance suite.
- **Replay and golden sessions.** A `TurnRecord` (input, draws, outcome, effects, state hash) is
  produced for every committed input. The golden-session runner replays a script of inputs against
  scripted ports and compares records and state hashes, which is how the section G traces and the M.2
  register become tests. Replay is a diagnostic and test tool, not the recovery path (RES19).
- **Metrics.** Counters and histograms are declared in one table (`step.ms`, queue depth and oldest
  age per lane, declines by code, window answer latency, ruling wait, plate lateness) and exposed
  through the `EventSink`, so the first playable slice can measure what H.5 only proposes.

**O.7 Typing and invariants worth stating up front.**

- `Outcome` has exactly four shapes (applied, needs clarification, declined, pending), and `Pending`
  carries a `PendingKind` of choice, roll, reaction or ruling. RES27's step result has a fifth value,
  `Waiting{pending_id}`, which is the machine's report that a pending is open and not a fifth outcome:
  the pipeline records the input's outcome as `pending{...}`, so the four outcomes stand. `Waiting`
  belongs to the machine's `StepResult` and is not a variant of `Outcome`. `Decline` carries a code, a reason, the missing piece if any, and alternatives, and a
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
surface. The old L.6 slices are void because the old code is gone. O.16 amends this order (two
spikes, the wire, the fixture contract, C5 before C4, H0 right after C3) and merges it with the
presentation tracks.

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
   traces 1, 2 and 4 as golden sessions (each recorded as a golden only after its review against the
   pinned source, RES52), plus the three known-bad lines.
5. **C4 Pending and windows.** `pending` with two-phase apply, held turns, answers by pending id,
   rulings with keyed reuse. Acceptance: the M.2 register rows M2-D4 to M2-D7 as fixtures, and a restart
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

- **Departures from O.1 to O.9 (the `df-table` crate docs in `crates/df-table/src/lib.rs` list the
  same).** The built slice is a workbench that exists ahead of O.9's order, which puts the host last,
  after C0 to C7: `crates/df-table`, `crates/df-table-host` and `crates/df-playground` are already
  workspace members. It is not C0 yet: there is no `outcome`, `envelope` or `owner_queue`. Only one of
  the eight ports of O.4 exists (`EventSink`). The revision is a bare counter, not `SessionRevision`
  (no recovery epoch exists yet). `EventClass` has two values (always, verbose), not the classes A and
  B of O.6, and an event has no `Correlation` or `OperationId`. `CanvasEffect` stands in for the first
  `TableEffect`, and the vocabulary says "canvas" where the plan says "table". Elements are entities in
  a small entity-component world (`ecs`, `world`, `motion`), which O does not describe (see the ECS
  bullet below). O.2's dependency list is not yet true either (see O.2).
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
  line; every line is parsed before anything runs, so a typo on a late line runs nothing, and then
  running stops at the first line that fails, with the lines before it staying applied) and `help`. A command that sends several effects applies
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
  verbs of H.3 and H.16: `force`, `fixture load` (H.16) and every verb that can read hidden or private content are
  operator-only and absent from hosted builds (RES43, H.3, H.16), and driven changes will carry
  `driven=true` once sessions exist. Question 34 (read freely, drive only for scripted tests) still
  limits driving live tables. Verbs implemented today (the usage text of
  `crates/df-table-host/src/command.rs`): `spawn`, `set`, `move`, `travel`, `stop`, `despawn`,
  `clear`, `canvas`, `kinds`, `events`, `reload`, `playground`, `scene`, `media`, `clients`, `fixtures`,
  `fixture`, `script`, `repl` and `help` (`media` and `clients` are described below; `clients` only
  reads). The workbench `fixtures` and `fixture show|write|remove NAME` verbs manage **playground
  fixture files**, the screen and phone drawings under `assets/playground/`. They are a different
  thing from H.16's `fixture load <bundle>`, which loads a table bundle (state, turn log, seed) into a
  machine for a golden session; the two are distinct and the H.16 one does not exist yet. These are
  workbench verbs; the H.16 set (`dump`, `force`, `input`,
  `viable`, `describe` and the rest) arrives with the machines. The host, not
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
- **Pictures and sound on both screens, and a way for an agent to check.** A media library
  (`assets/playground/media`; pictures png, jpg, webp and gif, sounds mp3, ogg, wav, m4a and opus; no SVG
  because opened on its own it can run script from the host's origin) is filled with `media add FILE`,
  listed with `media` and emptied with `media remove NAME`, and served by name with a content type, `nosniff`
  and an entity tag, so an unchanged file costs a 304. `scene image tv-backdrop|tv-card|phone NAME`
  puts a picture behind the figures, in the middle of the shared screen, or at the top of a phone's story;
  `scene music NAME [on=both|tv|phone] [volume=N] [loop=BOOL]` plays until stopped or replaced;
  `scene sound NAME` plays once. A fixture may name pictures too. A change that names a file the library
  does not hold, or the wrong kind, is refused with the names that exist. A sound is a numbered cue: the
  host keeps the newest sixteen, a page plays each id once, the first frame of a page only learns the
  count (no replay of old sounds), and a reset or a new fixture keeps the count rising so a cue is never
  mistaken for an old one.
  An agent cannot see or hear a screen, so each page reports what it shows and plays
  (`/dev/playground/report`, sent when it changes and every ten seconds) and `clients` prints it: whether
  sound is unlocked, the newest cue taken in, the music playing, the pictures that loaded, and the errors.
  A browser plays sound only after someone clicks the page; a refusal is reported as such, not hidden, and a
  sound that started counts as proof that sound is allowed. A page not heard from for 45 seconds is
  forgotten. A report is a reading, not a change: it is not an audit line and wakes no other page.
  Measured: pictures loaded and were reported on both screens; sound cues were taken in and played with
  no error; a missing or mistyped name was refused with the list of what exists.
- **Assets made by the project's own services.** The playground's newest pictures and lines were not
  made by hand: the game was started as the Codex live-test runbook describes (Codex for text and
  images, accounting record-only), two heroes were created, and the storyteller was asked in-scene
  questions. Its answers are the captions of `harbour_courier`, `bandit_at_the_quay` and `market_arcade`;
  its image service (Codex image backend, 117 to 196 s each) made the portraits of Finn Alder, Dain Rusk,
  Brakka and Sigrun and a rainy night refresh of the quay (123 s). The pictures are read back from the
  game's own `/gameplay/imagery/<key>` addresses and resized into `assets/playground/media`; the
  authored room picture and any local transform (about 1,955,472 bytes) are not counted as generations.
  Observed limits, for whoever builds the real provider port: `dev direct plate` is cancelled
  (`interrupted`) while the scene's own plate jobs run and refused (`invalid_request`) until its base
  plate exists; the line judge withholds a question that is not about the scene; leaving a scene needs
  its threat settled; and the new engine has no host for these services yet, so the playground can only
  be filled from a running game. A native provider port that the playground host calls (so `media
  generate` is a verb) is the next step and needs the services' durable store.
- **Scenes and transitions.** A playground scene (a fixture scenario, not the table scene of RES34 and D.1) is a scenario with its own backdrop, card, party layout, phone
  choices and, new, its own music (`music` in the fixture, replaced or stopped by `scene music` and
  `scene music-stop`). `playground go NAME [transition=cut|fade|wipe] [ms=0-5000]` moves to a scene (the
  name may leave off `.json`; a transition that is too long is refused before anything changes, so a
  `go` is all or nothing), `playground set transition=.. ms=..` sets how the next change looks, `wait MS`
  pauses the command line (for a `script` or a `repl` session that stages scenes over time), and
  `playground cycle A B [times=N] [hold=MS]` goes back and forth, stopping at the first scene it cannot
  reach. The transition lives in the host's controls like every other setting, so the page and the CLI
  agree. The page lays a copy of the old scene over the new one and lets a CSS animation take it away
  (a fade, or a wipe from the left), so the new scene is already in place underneath, the pictures of
  the copy stop counting as shown, and the audio follows the new scene at once. A reduced-motion
  setting skips the animation. Measured with two scenes (quay in the rain, the Flagon): each change
  produced one leaving copy that lasted the set 900 ms and was then removed, and the soundtrack
  reported by the page followed the scene. These `go` transitions are the workbench's: a real table
  starts a scene or plate change only in a K.3 swap window and never shorter than the 1.5 s dissolve
  (O.13 rule 11).
- **Turning and sliding a picture.** `scene image tv-card NAME rotate=0-359` draws a picture turned
  clockwise (180 is upside down; 360 or more is refused with the range), and `scene slide tv-card NAME
  [from=15] [to=85] [over=5000] [rotate=..]` moves the card across the shared screen at an even pace
  (0 to 100 percent of the width, 1 to 60000 ms; only the card on the shared screen can slide). The
  host numbers each slide, so a page that redraws part-way carries on from where the card was, and a
  new command restarts it. Measured with the upside-down portrait: 15% at the start and 84.9% after
  5 s, and a redraw forced 2.2 s in made no jump. Transforms in general are catalogued in O.13.
- **Camera, avatars and the tour.** `scene camera ELEMENT [zoom=100-400] [over=0-30000]` moves the
  shared screen's camera in on an element and keeps it in the middle (defaults 200% over 1200 ms);
  `scene camera-reset [over=MS]` pulls back out to the whole stage. The camera moves one world layer
  (backdrop, grid and figures) with a single transform, so the card, caption, status and party strip
  stay where they are; a target near an edge is held so the picture never shows empty space. Each move
  is numbered by the host and starts from where the camera had got to, so two moves back to back pan
  between faces without pulling out, and a redraw part-way carries on. The host refuses a target that
  is not on the stage (naming the ones that are) and a zoom or time out of range. A new `avatar` kind
  draws a portrait (`portrait` names a picture in the media library) in its figure; it is a workbench
  kind, and in play there are no cut-out avatars (RES13, D14; O.14); the
  `camera_demo` scene and `development/playground-scripts/camera-tour.txt` spawn Finn and Sigrun and
  zoom into each, then out. Measured: Finn centred exactly at 2.5 times, a 1.5 s pan to Sigrun at an
  even 6.6 points per 150 ms, and a redraw forced 3.2 s into an 8 s move made no jump (largest step
  1.4 points against an even 1.25). How these become gameplay controls is O.14.
- **Where it goes next.** The same pattern carries the real engine: a `TableEffect` applied by one
  function and recorded as one event; machines and the DM pipeline added behind the same CLI verbs
  (`spawn machine`, `input <seat> <text>`, `viable <seat>`, `projection show <audience>`); the scene
  record, hotspots and the plate scheduler drawn on this canvas first; and the dev CLI verbs of H.16
  generated from the registry instead of written by hand.

**O.13 Transforms (back-filled from rotate, slide and travel).** Three things on the canvas have
been built and measured: rotating a scene picture, sliding a card across the shared screen, and
travelling an element. They show what a transform needs, and this section generalises it so the next
transforms are added the same way and none of them weakens the rules the table relies on.

- **What the three built ones teach (each is a rule below).**
  - *Rotate* (`rotation_degrees`, 0 to 359 clockwise, any scene picture; `scene image ... rotate=N`)
    is one checked number written as a CSS transform, never as text from a scene file. A turn of 360 or
    more is refused with the range in the message.
  - *Slide* (`scene slide tv-card NAME [from] [to] [over]`: 0 to 100 percent of the screen width, 1 to
    60000 ms, even pace; only the shared screen's card) carries a host-assigned **serial**. A page notes
    when it first saw each serial (16 are remembered), so a redraw part-way carries on from where the
    card was (a negative animation delay), a new command restarts it, and a scene change forgets it.
    Measured: 15% at the start and 84.9% after 5 s at 14 points a second; a redraw forced 2.2 s in made
    no jump. Known limits of what is built: a page that opens mid-slide starts it from the beginning
    (the host does not yet say how long ago it began), and it animates `left`, a layout property.
  - *Travel* (`travel ID X Y [speed=N]`) is the one transform that changes **truth** in the workbench:
    the canvas position is authoritative there, so the host's clock moves it (30 Hz, a tick cut to 0.25 s)
    and a page only glides between samples (a 90 ms linear transition). The canvas ECS is the
    playground's host-side world. In the engine the rules owner's committed position is the only truth
    that feeds Travel (O.14 G2, RES1), and Travel only moves the picture between committed positions. Measured: 32 updates a second, 2.4 to 2.7 px
    steps at about 120 rendered frames a second, 200 travellers at about 3 ms a frame on the host.
  - *Scene transitions* (`playground go ... transition=cut|fade|wipe ms=0-5000`) lay a copy of the old
    scene over the new one and let a CSS animation take it away; the new scene is already in place.
- **Two kinds, and they never mix.** *Placement* (position, size in canvas units, which audience sees
  an element, any state a rule reads) is truth in the workbench: it lives in the ECS world, which is the
  playground's host-side world in canvas units (a 100 by 56.25 canvas, not feet or cells), and in the
  projection, is changed by an effect, and is the only thing the workbench's hit tests, distances, areas
  and `spatial query` read. In the engine the rules owner's five-foot-cell state is the only truth for
  position, reach, range, cover and areas (RES1, RES34), and `spatial query` is a `RulesOwner` port
  answer (O.4, M2-D33): the ECS placement components are then a presentation mirror written from the
  committed state through one declared cells-to-canvas conversion (declared in G2, O.14), and no rule,
  hit test or `spatial query` reads the mirror.
  *Appearance* (rotation, scale, flip, opacity, tint, filters, shake, pulse, camera) is presentation: it
  is drawn by the page, never read by a rule, and never changes what is legal. An appearance transform
  that **conveys a rule state** (Prone as a quarter turn, Unconscious as grey and dimmed, dead as faded,
  Invisible as 40% for its owner and, for the rest of the party, drawn with the hidden chip because every hero stays visible to the party (M2-D17, K.13, RES45), while a creature hidden from the party is left out of the public view (R06)) is **derived by the
  projection from the condition and the audience** (M2-D17, K.13), so a client, a script or the DM's
  text can neither set nor clear it. **What it is applied to.** In the workbench it turns the canvas
  element or figure. In play there are no cut-out figures (RES13, D14) and a creature is drawn inside
  one generated plate that a transform cannot turn, grey or fade, so a derived transform is applied
  only to what the plan draws separately: the hero's portrait in the party strip (K.13) and on its own
  phone, and the badge, turn marker and condition chips anchored to the creature (RES13, RES1, F.1
  overlays). The plate figure is never transformed. The dev CLI may override it in a local build, marked `driven=true`
  (H.3). Cinematic effects (a flash, a shake, a fade) are decoration and may come from the DM, but only
  as **named effects from a closed list** that the host expands into checked numbers; the model never
  supplies a number or a style.
- **The catalogue.** Status is `built` (measured above), or the build step of the order below.

  | Transform | Parameters and limits | Applies to | Drawn with | Status |
  |---|---|---|---|---|
  | Rotate | degrees 0 to 359 clockwise; pivot centre or `pivot=x,y` 0 to 100% | scene picture; then canvas element, figure, card | CSS `rotate` | built for pictures; T0 for elements |
  | Slide | from and to 0 to 100% of the width, 1 to 60000 ms, easing | TV card; then backdrop pan, phone picture | CSS animation, serial | built for the card; T2 for the rest |
  | Travel | target in canvas units, speed 0.1 to 400 units/s | canvas element | ECS `Motion`, host clock | built (truth) |
  | Glide | 90 ms linear between host samples | figures | CSS transition | built |
  | Scene change | cut, fade, wipe, 0 to 5000 ms | whole scene | leaving copy plus animation | built; T3 adds dissolve, iris, flash-to-white, fade-to-black |
  | Scale or zoom | factor 0.1 to 4.0 about the pivot | element, card, backdrop | CSS `scale` | T1 |
  | Flip | horizontal, vertical | element, picture | `scaleX(-1)`, `scaleY(-1)` | T1 |
  | Opacity or fade | 0 to 100%; fade in or out over ms | any | `opacity` | T1 |
  | Layer | bring to front, send back; layers 0 to 9 | element, picture | `z-index` | T1 |
  | Tween, generic | any numeric property above, `from` a value or `current`, `to`, ms, easing, repeat | any | CSS animation or Web Animations, serial | T2 |
  | Easing | linear, ease-in, ease-out, ease-in-out, spring (a fixed `linear()` curve), steps(n) | every tween | `animation-timing-function` | T2 |
  | Repeat | count 1 to 100, yoyo; infinite only for decoration | tweens | iteration count, direction | T2 |
  | Shake, bounce, pulse | amplitude 0 to 10 units, 0.5 to 12 Hz, decaying over ms | element, card, camera | keyframes | T2 |
  | Spin | -120 to 120 revolutions a minute | element, picture (a die, a coin, a wheel) | rotate keyframes | T2 |
  | Tint and colour | hex to hex over ms | figure fill, picture overlay | colour tween, overlay | T3 |
  | Filters | blur 0 to 20 px, brightness 0 to 200%, contrast, grayscale, sepia, hue 0 to 359 | picture, figure, scene | CSS `filter` | T3 |
  | Mask or frame | circle, rounded, arch | pictures (portraits) | `clip-path` | T3 |
  | Skew | -45 to 45 degrees | element | `skew` | T3, low use |
  | Camera, focus on an element | zoom 100 to 400%, 0 to 30000 ms, even pace, from wherever the camera was | the stage world layer (backdrop, grid, figures); the card, caption, status and party strip stay on the screen | one transform on the world layer, host-numbered move | built (O.14) |
  | Camera, pan to a point, frame several, drift | pan x and y 0 to 100%, frame 2 to 8 targets, slow drift 5 to 120 s (a Ken Burns move) | the same world layer | the same transform | T4 |
  | Path | up to 8 waypoints, line, arc, orbit | element; a projectile, a circling familiar | host `Motion` with a `Path`, or decoration | T4 |
  | Attach | offset in canvas units from another element | element on element (a rider, a banner, a held light) | ECS `Attached` relation | T4 |
  | Timeline | steps `{at_ms, target, tween}`; sequence, parallel, stagger | many elements | one serial, one client clock | T5 |

- **Data and wire.**
  - *ECS.* New components next to `Position` and `Motion`: `Rotation`, `Scale`, `Flip`, `Opacity`,
    `Filters`, `Pivot` and `Tweens` (at most 8 per element). Each value is a checked newtype like
    `Speed` (`Degrees`, `Factor`, `Percent`) whose refusal names its range in plain words. A snapshot
    (`Element`) carries them; the host view (`ElementView`) gains `transform` and `tweens`, each tween
    with its `serial` and, from T2, `elapsed_ms` taken when the frame is built, so a page that opens
    mid-tween continues it.
  - *Effects and events.* `CanvasEffect::Transform { id, set }` makes one change of several fields at
    once, all or nothing, and records one `ElementTransformed` event; `CanvasEffect::Tween { ... }`
    records one `TweenStarted`. Audit lines read `CANVAS transform element=pip kind=rotate revision=7`.
    A tween has no per-frame lines; a finite client-side tween has no end event (the page reports how
    many are running).
  - *Pictures* keep their fields on the picture (`rotation_degrees`, `slide`) until T2 folds them into
    the same tween model; the scene overlay remains the owner and keeps its own counter.
- **Rules.**
  1. *Continuity key.* Every tween has a host-assigned serial. Same serial means the same tween, so a
     redraw continues it; a new command gets a new serial and restarts it; a scene change forgets all.
  2. *Replace, not stack.* A new tween on the same element and property replaces the running one and
     starts from its current value (`from=current`), so nothing jumps.
  3. *All or nothing, checked at the edge.* Every number has a range and a refusal that says the
     range; a command with several parts applies none of them if one is refused (as `go` does).
  4. *Limits.* At most 8 tweens on an element, 64 running on a page, 60 s for a finite tween, and
     infinite decoration on at most 8 elements. Beyond that the effect is refused with the limit named.
  5. *Compose order.* Position, then rotate, then scale, then flip, about the pivot; fixed, so two pages
     draw the same picture.
  6. *Text stays upright and readable.* Transforms apply to pictures, figures, the backdrop and the
     camera, never to a caption, a choice button or a status line.
  7. *Never gates input or a machine.* Input opens at once (RES45); a tween never delays a beat, and the
     dwell rules of K (not tweens) decide how long a line stays up.
  8. *Reduced motion and safety.* A reduced-motion setting, or the per-seat `reduced_effects` setting
     (J.4), shows the final state with no shake, spin, flash or camera drift. A flash is at most 3 a
     second, and a repeated flash (lightning, a damage flash) never covers more than a quarter of the
     screen (the WCAG 2.3.1 general-flash limit). The T3 scene-change flash-to-white covers the whole
     screen, so it is allowed only as one single transition: it rises and falls over at least 300 ms, is
     never repeated, and has no other flash within the same second. Under `reduced_effects` it is a plain
     fade, and both limits are checked at the edge like every other number.
  9. *Audience safety.* A tween travels in the projection for one audience; a seat-private effect (a
     hero's private note trembling) reaches only that seat, never the shared screen.
  10. *Compositor first.* Prefer `transform`, `opacity` and `filter` to `left`, `top`, `width`; the slide
      moves to a translate in T2. Blur is expensive: at most 3 elements above 8 px.
  11. *Swap windows and the reveal hold.* On a real table a scene change, a plate dissolve, a
      flash-to-white and a fade-to-black are requested through the playout buffer and start only in a K.3
      swap window: between lines, never during a dice or result card, never during a reveal beat (K.7),
      waiting at most `plate_swap_wait_s`, and lasting at least the 1.5 s plate dissolve minimum (J.4), so
      a `cut` or a shorter fade is refused for a plate or scene change. A tween or camera move that is not
      part of the swap (a shake, a pulse, a card slide, a camera focus) never starts during a reveal beat
      unless it belongs to that reveal, and during a dice or result card only if it belongs to that card
      (O.14 camera rule 5). The workbench's free `go`, `playground set transition` and tweens are dev
      tools that ignore these windows and are absent from a hosted build's table (O.12, O.14).
- **Where a table uses them.** In play each item below that names a figure or a set piece (a fade-out
  on death, a door that swings, a drawbridge, a familiar that orbits, a banner or held light attached to
  its bearer, a damage shake on a creature) is drawn only as an overlay, badge or inset the plan draws
  separately, or in the workbench; a creature or set piece inside the plate is never turned, faded or
  moved by a transform (RES13, D14). Prone, Unconscious, dead, Invisible and Hidden (derived, above, on
  the party-strip portrait, the owner's phone portrait and the anchored badge); Frightened as a slow tremble and Concentrating as a faint pulse ring on the caster (derived);
  a damage flash and shake on a hit; a fade-out on death; a die that spins and slows and a coin that
  flips (spin and flip); a door that swings open about its hinge (rotate with a pivot); a drawbridge;
  a projectile or a ray along a path; an initiative marker that slides along the strip to the next
  creature; a familiar that orbits; a banner or a held light attached to its bearer; the camera that
  pushes in on the speaker or pans to the fight; lightning as a flash and a shake; a slow Ken Burns
  drift on a plate while a line is read; a portrait that slides in from the left as a speaker
  arrives; and a darkvision or dim-light scene as a brightness and tint filter.
- **Dev CLI.** Each transform is a verb of the same shape as the built ones, with defaults, checked
  numbers and one numbered answer: `rotate ID DEG [pivot=x,y]`, `scale ID FACTOR`, `flip ID x|y`,
  `fade ID PERCENT [over=MS]`, `tint ID #rrggbb [over=MS]`, `blur ID PX`, `spin ID RPM`, `shake ID
  [amp=] [hz=] [over=]`, `pulse ID`, `tween ID PROPERTY to=N [from=N|current] over=MS [ease=] [repeat=]`,
  `camera pan|zoom|drift ...`, `attach ID TO dx dy`, `timeline FILE` (steps with `at`) and the scene
  forms `scene slide tv-card ...` (built) and `scene transform tv-backdrop|tv-card|phone ...`.
  `canvas` prints each element's transform and its running tweens; `clients` reports `tweens_running`
  and `reduced_motion`.
- **Acceptance cases.** Rotate: 0, 359 accepted, 360 and -1 refused, a picture and an element both turn.
  Slide: start, end, a redraw mid-slide with no jump, a new command restarts, a scene change forgets,
  a late joiner continues from `elapsed_ms` (T2). Replace: a second tween on the same property starts
  from the current value. Limits: the 9th tween on an element and the 65th on a page are refused.
  Reduced motion: every transform shows its final state. Derived: Prone shows a quarter turn on the hero's
  party-strip portrait on the TV and on the owner's phone portrait, and the plate figure is not touched; an Invisible hero's phone portrait is 40% on its owner's phone and the TV's party strip carries the hidden chip, while an Invisible enemy is left out of the TV view; a client cannot
  clear either. Projection: a seat-private tween never appears in another audience's frame.
  Windows: a scene change asked for during a result card or a reveal beat starts only after it, within
  `plate_swap_wait_s`, and a `cut` or a 100 ms fade for a plate or scene change is refused on a real
  table. Flash: a scene-change flash-to-white is one transition of at least 300 ms with no other flash in
  the same second, and a plain fade under `reduced_effects`.
  Performance: 200 elements each with one running tween keep the 95th percentile frame gap under
  16.7 ms on the reference machine, measured as the travel test was.
- **Build order.** *T0* rotation for canvas elements (ECS component, view, page, `rotate`).
  *T1* scale, flip, opacity, layer. *T2* the generic tween (easing, repeat, replace, `elapsed_ms`,
  compositor-only slide, shake, pulse, spin). *T3* filters, tint, masks, skew and the extra scene
  changes. *T4* the rest of the camera (pan to a point, frame several, drift; focus on an element is
  built, O.14), path and attach. *T5* timelines and stagger. *T6* the derived condition
  transforms in the projection, behind the audience rules, applied to the party-strip and phone
  portraits and the anchored badges. Each step ends with its acceptance cases
  above and a browser measurement.
- **Decisions taken (change on request).** Appearance tweens are drawn by the page and only position
  is host-driven; revisit if recording or replay needs the host to sample frames. Condition-derived
  transforms are on by default, and `reduced_effects` removes only their motion (the tremble and the pulse) and keeps their static pose, tint and chip, so a seat can never hide a rule state. The model chooses a named
  effect from a closed list, never a number. The flash limit is 3 a second, with the quarter-screen
  limit applying to repeated flashes and the single scene-change flash-to-white bounded by rule 8.

**O.14 Table controls for later gameplay (back-filled from the camera, slide, rotate, travel, scene
and sound controls).** The playground built a set of controls that act on what the table sees and
hears. Each has a checked number or a closed choice, one dev verb, one audit line and a measured
result. This section says how the same controls become part of play, who may drive each, and what
stops them from hiding or changing anything a rule depends on. Nothing here changes a rule: every
control is presentation (O.13), so the table's truth (positions, who may act, what is visible to whom)
stays where D.6, K.13, M2-D17 and R06 put it.

- **The controls and where they are now.**

  | Control | Dev verb (built) | What it does | Gameplay driver | Status |
  |---|---|---|---|---|
  | Camera focus | `scene camera ELEMENT [zoom] [over]`, `scene camera-reset` | keeps one element in the middle of the shared screen at 100 to 400%, then pulls back | the Director, on a named reason (below) | built for the playground; engine wiring G1 |
  | Picture turn | `scene image SLOT NAME rotate=0-359` | draws a picture turned | a handout or sign shown the wrong way up; a prone portrait | built for pictures |
  | Card slide | `scene slide tv-card NAME [from] [to] [over]` | slides a card across the screen at an even pace | a speaker's portrait arriving or leaving; the next creature in the turn bar | built |
  | Travel | `travel ID X Y [speed]` | the host moves an element a little each tick | movement in combat and exploration (truth: the rules owner's committed position) | built as a canvas feature; engine wiring G2 |
  | Scene change | `playground go NAME transition=cut\|fade\|wipe ms=` | puts a new scene in place under a leaving copy of the old | a new location, a flashback, a cut to the next day, only in a K.3 swap window and never shorter than the 1.5 s dissolve (O.13 rule 11) | built (the dev verb ignores the windows) |
  | Music and sound cues | `scene music`, `scene sound` | a track per scene, one-shot cues by id | scene mood; a hit, a door, a bell | built |
  | Looping video | none yet (O.15) | a muted video drawn in a picture slot or a figure, looping | a burning hearth, rain on the quay, a portal, a moving map | planned (O.15, V0 to V1) |
  | Per-item sound | none yet (O.15) | a sound that comes from one element, placed and levelled by where it stands | a campfire's crackle, a footstep while a creature travels, a door | planned (O.15, V2 to V4) |
  | Caption | `scene caption`, `caption-clear` | the line on the shared screen | the narrator and NPC speech (K.12 dwell applies) | built |
  | Party strip, choices | `scene party`, `scene choices` | the party bar and one phone's choice list | the engine's view of the party and of what a seat may do | built as fixtures; the engine's projections are the real source |
  | Pace, grid, transition style | `playground set pace= grid= transition= ms=` | table settings | the table's own settings, never the model; a transition style on a real table is bounded by O.13 rule 11 | built |

- **Who may drive a control.** There are four drivers and they are ranked. The **dev CLI** may drive
  any control with any legal value in a local build and marks the result `driven=true` in the audit
  line; it is absent from a hosted build's table. The **Director** (a pure stage of `df-presentation`,
  whose `PresentationDirector` already owns the audience-safe audiovisual plans in
  `subsystem-architecture.md`; `df-engine` composes it and `df-session` supplies the injected clock and
  the timers; never the model) drives the camera, slides, transitions and cues from engine events,
  using the reasons below. The **DM text model** may only *suggest* one of a closed list of named
  directions (`focus speaker`, `wide`, `reveal object`) in its structured narration output, a closed
  value on a clause that the qualification of K.7 and RES46 checks (the narration brief is pure-code
  input and carries none); the Director accepts, delays or drops it, and the model never supplies a
  number, a style or a path, and names an element only by a set-piece or cast id from the brief's
  public scene, which the Director then checks against the audience-safe view (rule 1).
  A **seat** may set its own phone's settings (reading pace, reduced effects, auto-focus off for its own
  screen, never for the shared one) and may press "look at me" on its phone to ask the Director to focus
  its hero, which the Director treats as the `requested` reason, the lowest rank below. A client can
  never drive the shared camera directly.
- **What the camera acts on in play.** The workbench camera moves a world layer of avatar and figure
  canvas elements (O.12). In play there are no cut-out avatars or figures (RES13, D14): the world layer
  is the one generated plate, and a focus target is a **screen anchor** of RES1 (a character's head
  point or an item's point, from SAM Locate on that plate id), never a rules position. The reasons below
  name a creature, and the Director resolves it to its anchor on the plate now showing. The overlays
  anchored to those anchors (badge, turn marker, hit marks, damage numbers and condition chips on the
  plate; RES13) belong to the world layer: they move and scale with it, stay on their creature and keep
  one constant on-screen size. While the establishing card or a place card shows (K.2) there is no
  plate and no anchor, so every focus is declined `no_target` and the camera stays wide. A swap inside a
  scene brings a plate with its own anchors (K.3): the Director re-aims at the same creature's anchor on
  the new plate within the same dissolve, or goes wide if it has none, and never stays zoomed on the old
  plate's position. A `reveal` whose object appears only in a plate that is still queued starts after
  that plate's swap, because a reveal beat holds plate swaps (K.3, K.7).
- **Camera rules, from what was built and measured.**
  1. *Targets come from the audience's projection.* The Director resolves a focus target from the
     audience-safe view of the shared screen, so the camera can only zoom to something the whole table
     may see. An element hidden from the party (R06) or a hidden creature is not targetable, and a
     target that stops being visible makes the camera pull back at once (no zoom onto an empty place
     that would give the hiding spot away); that pull-back is a `wide` that outranks every reason,
     ignores the 800 ms spacing and the result-card rule of rule 5, and starts within one host tick.
     Every hero is always visible (K.13), so a hero may always be
     focused.
  2. *Presentation only.* The camera moves one layer (backdrop, grid, figures). The turn bar, the party
     strip with its HP and condition chips, the caption, status and the pending-choice banner are
     screen-fixed and never move or scale (the overlays anchored on the plate ride with the layer, as
     above), so nothing a person must read or act on can be pushed off the screen by a zoom.
  3. *Framing keeps the picture full.* The slide is held so the world always covers the screen; a target
     near an edge is as near the middle as the edge allows. A zoom is at most 4 times and the default for
     play is 2 times (the playground's default); a faster or deeper move is refused, not clamped silently.
  4. *Moves are numbered and continuous.* Each move has a host-assigned serial and starts from where the
     camera had got to, so a move interrupted by another pans on to the new target without pulling out;
     a redraw part-way carries on (measured: no jump); a scene change forgets the camera and starts wide.
  5. *Pace a person can follow.* One move at a time, at an even pace, 600 to 2500 ms for a focus and at
     most one new move in any 800 ms, except that a higher-priority reason (the order below) may start at
     once and the pull-back of rule 1 always does. The Director coalesces a burst inside 800 ms into the
     highest-priority direction and, among equals, the latest; a burst for one result that names several
     heroes (an area) leaves the camera wide until T4 can frame them. A line stays up for its K.12 dwell
     whatever the camera does, and the camera never moves during a result card's minimum 3000 ms unless
     the card is about the target (the pull-back of rule 1 excepted). Every move also obeys the swap
     windows and the reveal hold (O.13 rule 11), never gates input (RES45: a hero's input opens at once,
     and the Director's holds run on the server's timers from the conductor's anchors, never on a client
     acknowledgement), and stays wide through a collapsed beat of K.12 (consecutive turns of creatures on
     one side that did not target a party member), where `active_creature` does not fire.
  6. *Always returns.* A focus holds for at most `focus_hold_s` (default 8) after its reason ends, then
     the camera pulls back to the whole stage; combat starts and every new scene start wide; a hold longer
     than that needs a fresh reason. A stuck zoom is a defect.
  7. *Follow.* A focus on a travelling element is re-aimed each frame without restarting the move
     (built in the page; not yet measured), and the move ends if the element leaves the audience's view.
  8. *Reduced motion.* A reduced-motion setting, or the shared screen's `reduced_effects`, shows the
     final framing with no animation (a cut), and the camera does not drift.
  9. *Logged.* Each move writes `CAMERA focus element=ID zoom=250 over=1500 reason=speaker serial=7`
     and each return `CAMERA wide reason=timeout serial=8` through the shared instrumentation path, with
     the Director's reason, so a playtest can measure when the camera moved and why.
- **Reasons the Director focuses the camera.** Each is a named reason with its own default zoom, time
  and hold, all in data, none typed by the model.

  | Reason | When | Target | Default | Returns |
  |---|---|---|---|---|
  | `speaker` | an NPC speaks and has an anchor on the plate now showing (a narrator line has none and leaves the camera as it is) | that anchor (in the workbench, the avatar) | 200%, 1200 ms | when the line's dwell ends |
  | `active_creature` | a turn starts in a scene with more than 4 figures | the creature taking its turn | 160%, 900 ms | when the turn ends |
  | `hit` | an attack or spell lands on a hero | the target, then wide | 220%, 700 ms | after the result card |
  | `reveal` | a hidden object, trap or clue is found | the object (now visible to the party) | 250%, 1500 ms | `focus_hold_s` |
  | `roll` | a death save or a contested check (the list of roll kinds is data) | the roller | 200%, 900 ms | when the result card ends |
  | `arrival` | a figure enters or leaves | that figure | 180%, 1200 ms | `focus_hold_s` |
  | `spotlight` | the K.13 spotlight offer picks a quiet hero | that hero | 180%, 1500 ms | `focus_hold_s` |
  | `requested` | a seat taps "look at me" | that seat's hero | 180%, 1200 ms | `focus_hold_s` |
  | `wide` | combat starts, a scene starts, a target becomes hidden, a hold ends | none | 100%, 1200 ms | n/a |

  A lower-priority reason never interrupts a higher one inside its hold; the order is `hit`, `roll`,
  `reveal`, `active_creature`, `speaker`, `arrival`, `spotlight`, `requested`. `wide` is not a focus
  and has no place in that order: a `wide` for a target that became hidden outranks every reason (rule
  1), and the others (a start, a hold ending) never interrupt a hold.
- **Data model and wire.**
  - `TvScene` carries `camera: Option<Camera>` (target element or none, zoom percent, duration ms,
    host serial); the engine's shared-screen projection gains the same field, produced only by the
    Director, so every audience's TV frame (one shared screen, so one value) matches and a phone never
    receives one.
  - The Director's output is a `StageDirection` (`Focus{element, reason}`, `Wide{reason}`,
    `Slide{...}`, `Cue{...}`), pure data from `df-presentation`; the host applies it with the same checked
    numbers the dev verbs use (`SceneChange` in the playground is the model for it), so a dev verb and
    the engine take one code path and there is nothing separate to keep in step.
  - Portraits ride on the element (the `avatar` kind's `portrait` field); a figure without one draws as
    before. A portrait is a media name from the library, never a URL.
  - Replay and recordings: a direction has a serial and a reason, so a recording replays the camera from
    the log rather than sampling frames (O.13 decision).
- **Dev CLI for play.** The built verbs stay and gain gameplay forms, all with the same numbered
  answer and audit line: `camera focus ID [reason=] [zoom=] [over=] [hold=]` (a Director direction with
  a reason, so the rules above apply), `camera wide`, `camera frame ID ID ...` (T4), `camera follow ID`,
  `director show` (the directions queued, applied and dropped, with reasons), `director pause|resume`
  (hold every automatic direction while a dev drives by hand), `director auto camera|slide|cue on|off`
  (each class on its own), and `scene state` (everything on the shared screen as the Director sees it).
  Tour scripts (`development/playground-scripts/`) stay the way to rehearse a sequence, and a tour can
  assert state between steps (`expect camera element=sigrun`). In H.16's terms these are family verbs,
  additions to its minimum set like `sheet build` (H.16 lists them): `camera focus|wide|frame|follow`,
  `director pause|resume|auto` and the workbench `scene camera` drive the table, so they are
  operator-only, absent from hosted builds and marked `driven=true`. `director show` and `scene state`
  read the audience-safe view only: a direction dropped because its target is hidden is listed as
  `dropped reason=target_hidden` with no element id, and so is its audit line (H.3, RES43); only an
  operator build may show the id.
- **Acceptance cases.** Focus: Finn then Sigrun back to back pans without pulling out and ends centred
  (built, measured in the playground). Hidden: a hidden creature is refused as a focus target and a focus
  on a creature that becomes hidden starts pulling back within one host tick, even inside a result
  card's minimum time or the 800 ms spacing; a hero is always focusable. Priority: a
  `hit` interrupts a `speaker` focus and not the reverse, and a burst of three directions inside 800 ms
  leaves one move, to the highest-priority one and among equals the latest; three heroes hit by one
  area leave the camera wide. Requested: "look at me" during a `speaker` focus is dropped and logged as
  lower priority, and on a quiet stage it focuses the seat's hero. Plate: while the establishing card
  shows every focus is declined `no_target`; a swap inside a scene re-aims at the same creature's anchor
  on the new plate or goes wide; anchored overlays stay on their creature at every zoom. Collapsed
  beat: a K.12 collapsed beat keeps the camera wide, and an `active_creature` focus never delays a
  hero's input. Return: every focus ends wide within `focus_hold_s` after its
  reason; a new scene and combat start wide. Legibility: with the camera at 400% the turn bar, party
  strip, chips and caption keep their on-screen size and place. Dwell: a caption stays up for K.12 and the
  camera never moves during the minimum result-card time except onto its subject. Reduced motion: a cut.
  Projection: no phone frame carries a camera, and a TV frame for the table carries only a visible
  target. Log: every move and return has a line with a reason, `director show` matches the log, and in a hosted build a direction dropped for a hidden target has no element id in the log, in `director show` or in `scene state`.
  Follow: a travelling element stays centred within one tick (to measure).
- **Build order.** *G0* (built) the playground controls above. *G1* the Director's camera stage in
  `df-presentation` (reasons, priority, hold, coalescing, with an injected clock) driving the same `SceneChange` numbers, with
  `director show` and the audit lines, tested with a controlled clock. *G2* engine wiring: positions
  and speakers from the machines, the projection field, and `travel` fed by committed moves. *G3*
  `reveal`, `hit`, `roll` and `spotlight` reasons, with the hidden and visibility cases. *G4* pan to a
  point, frame several and drift (O.13 T4), and per-seat auto-focus settings. *G5* a browser play-test
  at human pace judging that the camera never fights a line, flickers or sticks zoomed, with the timings
  taken from the logs (AGENTS timing rule).
- **Decisions taken (change on request).** The camera is one shared value for the table's screen, not
  per seat; per-seat views are the phone's own picture, which does not move. Default focus zoom 200%
  and `focus_hold_s` 8. The DM model suggests named directions only. The camera starts wide in combat so
  the whole fight is legible, and focuses on the active creature only when more than four figures are
  on the stage. Pace is linear in the playground; easing is a T2 option and is not needed for gameplay.

**O.15 Looping video and per-item sound (planned; extends O.12 pictures and sound).** Pictures and
scene-wide sound are built. Two things are not: a video that plays in a loop in a picture slot or a
figure, and sound that comes from one element. Both are presentation (O.13), driven by the same four
drivers as the other controls (O.14), and neither may carry anything a rule depends on.

- **What exists and what is missing.** The media library takes png, jpg, jpeg, webp and gif pictures
  and mp3, ogg, wav, m4a and opus sounds, at most 16 MiB a file and 128 files, served with an ETag and
  `304`. An animated GIF already loops as a picture. Sound is scene-wide: `scene sound` plays one cue
  by id for `both`, `tv` or `phone`, `scene music` sets one track per scene, and the page's audio
  engine reports whether sound is locked until someone clicks. Missing: a video type, any placement of
  one, range requests, any link between a sound and an element, and any position in a sound.
- **Video: the file.** The library accepts `mp4` (H.264 with AAC) and `webm` (VP9 with Opus). The type
  is read from the first bytes (the `ftyp` box or the EBML header), never trusted from the extension,
  so a renamed text file or an SVG is refused with the reason. A video is at most 64 MiB and 60 s, and
  a library may hold 16 of them (they count against the 128 files). The host serves video with `Range`
  requests and `206` answers, because browsers need them to start and to seek, and keeps the ETag and
  `304`. A video may name a `poster`, a picture from the library shown while it loads, if it fails,
  and under reduced effects.
- **Video: placement and play.** A picture slot may hold a video wherever it holds a picture: the
  shared screen's backdrop and card, a phone's image, and an element (an `avatar` or a `set_piece`
  kind's `video` field, drawn in its figure the way `portrait` is). `scene video SLOT NAME [loop=true]
  [rate=1] [poster=NAME] [sound=false]` places one and `scene video-clear SLOT` takes it away. It plays
  `muted`, `playsinline` and `loop` with autoplay, which browsers allow without a click; its own audio
  track is ignored unless `sound=true`, and then it plays only after the audio engine's unlock and at
  that scene's music volume. `rate` is 0.25 to 2.
- **Video: continuity.** A video placement has a host-assigned serial like a slide (O.13 rule 1). A page
  notes when it first saw the serial and, after a redraw, seeks to `elapsed mod duration` so the loop
  carries on from where it was instead of restarting, to within 250 ms; a new placement restarts it; a
  scene change forgets it. The camera zooms a video backdrop like any picture, a leaving copy of the old
  scene pauses its video so two are never playing at once, a hidden tab pauses it, and at most 2 videos
  play on the shared screen and 1 on a phone. The next scene's video is preloaded (metadata and the
  first segment) while the current scene is shown, so a transition never reveals a black frame.
- **Video rules.**
  1. *Never the only carrier.* A video shows mood and place; every fact it suggests is also in the
     caption, a chip or a figure, as with a plate (K.2: art is never a gate, and the scene is playable
     without it).
  2. *Failure is a still.* A video that is missing, slow or refused shows its poster or, without one, the
     still plate; a bad name shows a plain note as a bad picture does. A provider-generated video (the
     asset engine) is cosmetic: it never withholds a reward or changes timing, and a late one swaps in
     only at a K.3 swap window.
  3. *Reduced effects and safety.* Reduced motion, a seat's `reduced_effects` or a data-saving setting
     shows the poster and does not play. A video must declare `flashing: false` in its library record,
     set when it enters the library by the asset pipeline's flash check (3 a second and a quarter of the
     screen, as O.13 rule 8), and one that does not declare it is shown as its poster on a shared screen.
  4. *Audience.* A video is served only to the audience whose projection names it; a seat-private video
     goes to that phone alone, and a video that would show a hidden thing is never placed.
  5. *Budget.* A page records bytes fetched and frames dropped; a scene that cannot hold 24 frames a
     second on the reference phone falls back to its poster and says so in `clients`.
- **Per-item sound: sources.** A sound is attached to an element in four ways, all data, all checked
  against the library when a kind loads (a missing file is a warning on `kinds`, never a crash):
  1. *Kind sounds.* A kind file may carry `sounds`, a map from an event to a library sound: `loop` (while
     it exists), `spawn`, `despawn`, `move` (while it travels, as footsteps), `stop`, `hit`, `use`.
  2. *An element override.* An element's `sound_loop` field replaces the kind's loop for that element (a
     lit lantern against an unlit one).
  3. *A one-shot from an element.* `scene sound NAME from=ELEMENT [volume=] [pan=auto|-1..1]`.
  4. *A Director cue.* The Director's `StageDirection::Cue{element, sound, reason}` (O.14) for a hit, a
     door, a spell, from the committed event, never typed by the model.
- **Per-item sound: place and level.** A voice is panned and levelled from where its element stands
  and from where the camera looks, by pure functions with their own tests. Pan is `(x - 50) / 50 * 0.8`
  of the stage, so the far edge is at most 0.8 to a side. Gain is 1 at the camera's focus (or the middle
  with no focus) falling evenly to a floor of 0.35 at the far edge, and the element the camera focuses
  gets up to 1.25 times, never past full scale after the master limiter. A phone has no stage, so its
  voices are mono at the element's gain. `on=tv|phone|both` still picks the screens, and an element's
  default is `tv`. The master volume and the music volume stay separate.
- **Per-item sound: voices.** The page keeps at most 12 voices and 6 loops, one loop per element, each
  with a 150 ms fade in and out so nothing clicks. When full, a one-shot ranks above a loop and a newer
  one above an older one; the lowest-ranked loop ducks to silent before any one-shot is dropped. A
  despawned element's loop fades out; a travelling element's `move` loop starts with its motion and
  stops when it stops. One-shots that arrive before the browser's unlock are dropped after 2 s, as scene
  cues are now; loops start when the unlock arrives. The cue id and count rule is kept so a reset never
  replays an old cue.
- **Per-item sound: audience, access and loudness.**
  - *Audience.* A sound from an element the audience may not see is omitted from that audience's
    projection, never muted in the client (R06, M2-D17); a hero is always visible (K.13), so a hero's
    sound is always allowed; a private sound targets one seat's phone.
  - *Never the only signal.* Every sound that carries information (a door, an alarm) has a visual
    equivalent, and a seat's `sound_captions` setting shows a short label for it (K.19); a seat can turn
    sound from items off without turning off music or voice. Reduced effects does not mute.
  - *Loudness.* Library sounds are normalised at ingest to a common loudness with a true-peak ceiling
    so no item is louder than the narration; the engine never raises a sound above full scale.
- **Data and wire.**
  - `MediaKind` gains `Video`; an `ImageRef` becomes a `Visual` (a picture or a video, with `poster`,
    `loop`, `rate`, `sound` and the serial).
  - A kind file gains `sounds`; an element gains `video` and `sound_loop` fields; the scene overlay's
    cues and voices gain `element: Option<String>`; the page's audio engine gains `play_from` taking the
    element's stage position and returning a voice it can fade.
  - The client report adds videos playing, failed and on poster, and active voices by element, and the
    host's `clients` verb shows them.
- **Dev CLI and logging.** `media add` accepts video (checked as above); `scene video SLOT NAME ...` and
  `scene video-clear SLOT`; `scene sound NAME from=ELEMENT`; `sound loop ELEMENT NAME|off [volume=]`;
  `kinds` lists a kind's `sounds`; `voices` lists active voices with element, gain, pan and loop;
  `clients` shows videos and voices. Audit lines: `MEDIA video slot=tv-backdrop name=hearth.webm
  serial=3`, `AUDIO voice element=fire loop=true gain=0.82 pan=-0.40`, `AUDIO voice_dropped
  element=goblin_3 reason=voice_cap`, each with the driver's reason.
- **Acceptance cases.**
  - *Ingest.* Accepts a real mp4 and a real webm; refuses a text file renamed `.mp4`, an SVG, a video
    over 64 MiB or 60 s, and a 17th video; a `Range` request returns `206` with the right bytes and a
    request past the end returns `416`.
  - *Loop.* A video loops with no `ended` event and no black frame at the seam in a browser check; a
    redraw 2.2 s into a 3 s loop resumes within 250 ms of where it was; a new placement restarts it; a
    scene change forgets it; a leaving copy does not play a second video.
  - *Fallback.* A missing video, reduced effects and a data-saving setting each show the poster; a
    video without `flashing: false` shows its poster on the shared screen.
  - *Per-item sound.* The pan and gain functions are monotone and bounded (pan within -0.8 to 0.8, gain
    within 0.35 to 1.25 before the limiter); the 13th voice and the 7th loop are handled in the stated
    order; a despawn fades the loop out; a travel starts and stops the footstep loop with the motion
    events; a one-shot before unlock is dropped after 2 s and a loop starts at unlock.
  - *Audience.* A hidden creature's sound is absent from the table's projection and a hero's is present;
    a private sound reaches only its seat; no phone frame carries a camera-based gain.
  - *Access.* With `sound_captions` on, each informative sound has its label; with item sound off, music
    and voice still play.
  - *Measured.* Video start under 1 s on the reference machine from a warm scene, voice start under 100
    ms from the command, and no more than 1 dropped frame in a minute of a 1080p loop, taken as the
    travel and camera numbers were.
- **Build order.** *V0* video in the library (type check by bytes, size and duration limits, `Range`
  answers) and the dev `media` verbs. *V1* the page: a video in the backdrop, card, phone image and a
  figure, loop continuity by serial, poster, reduced effects, pause on a hidden tab and the two-video
  limit. *V2* `scene sound ... from=ELEMENT` with the pan and gain functions (pure, tested natively)
  and the voice cap. *V3* kind `sounds`, element `sound_loop`, spawn, despawn and travel footsteps.
  *V4* Director cues and the projection rules (O.14 G1 and G3). *V5* provider video from the asset engine
  with its deadline and still fallback, preload at a K.3 swap window, and the browser measurements.
- **Decisions taken (change on request).** Video is muted by default, and sound from a video is opt-in
  and follows the unlock. Both mp4 and webm are accepted so every browser has one. Per-item sound is
  spatial only on the shared screen; a phone gets mono. The pan limit is 0.8, the gain floor 0.35 and
  the voice limits 12 and 6. Reduced effects shows a poster but does not mute sound; a separate
  per-seat switch turns item sound off.

**O.16 Master build order (amends O.9; merges O.9, O.13, O.14 and O.15).** The plan has four orders
that were written at different times: O.9 (the pure crate C0 to C7, then the host H0 to H3), O.13 (T0
to T6, transforms), O.14 (G0 to G5, table controls) and O.15 (V0 to V5, video and item sound). The
code does not follow O.9: the built part is the canvas workbench (O.12), and none of C0 to C7 exists
yet. This section is the one order to build from, with what each step needs, what can run beside it
and where it is accepted. It was written from a review of the plan against the code on 2026-11-02
(issues #1591 to #1595).

- **Facts it rests on.** `df-table` holds the canvas model only (ECS, canvas, motion, kinds); none of
  outcome, config, envelope, owner queue, machine, registry, pipeline, pending, projection, table state
  or inspect exists. `df-table-host` and `df-playground` are the workbench. `df-protocol` has about 26
  lines of source and `df-api` about 760, so no view or intent can reach a client. `df-rules` has no
  legal-choice listing and few exploration rules (O.10, M.5). The old server and its `dev latency`
  test were deleted (O.1), so the riskiest assumption of L.7 has no test. The SAM item-location test
  (second riskiest) has never run.
- **Amendments to O.9.**
  1. Two spikes are steps with owners: S1 (the owner-thread latency harness) and S2 (the SAM
     item-location test). Both come before the work that depends on their answer.
  2. A wire step W0 and W1 comes before H3, because the first playable path must reach a browser and a
     phone.
  3. A fixture contract K0 makes the playground the viewer for the real projections.
  4. C5 (projection) is built before C4 (pending and windows): a pending choice is shown through a
     projection, and K0 and W0 need one.
  5. H0 (the owner actor with the queue and pipeline over fake adapters and the dev CLI) starts right
     after C3, so C4 to C7 are exercised end to end and not only by tests.
- **The steps.** *Needs* lists what must be done first. *Beside* lists what can run at the same time
  without sharing edit areas. *Accepted by* says how the step is accepted (RES44 for anything a person
  sees; tests and the CLI for the rest).

  | Step | What | Needs | Beside | Accepted by |
  |---|---|---|---|---|
  | B0 | Close questions 87 to 90 (the crate edge, the minimum port set for C0, event class A fields that never change, the first machines for C2) and #1589, #1590 | none | T0 to T2, V0, V1 | the owner's answers recorded in the plan |
  | C0, C1 | O.9 C0 foundations and C1 owner queue | B0 | T0 to T2, V0, V1, RR | O.9 acceptance (privacy by type, fairness, bounds, determinism) |
  | RR | A readiness table for `df-rules`: for each machine, which rules calls exist, which are `not_supported_yet` (RES31) and which M.5 area owns the rest | none | anything | the table in the plan; no code |
  | S1 | Owner-thread latency harness in `df-table-host` over fake ports: six seats and a TV, queue, a representative projection and a stub commit; report p50 and p95 of the view queue and the reduce | C1 | C2 | numbers recorded against H.5 (projection under 5 ms, rules-only path under 400 ms p95); a miss changes the design before C3 |
  | S2 | The SAM item-location test: free noun-phrase prompts over about 20 stored plates (about $0.20) | none | anything | found rate recorded; under about 80 percent, item hotspots use their own pictures (L.7) |
  | C2 | Machines, registry, conformance suite, lobby and a minimal explore | C0, C1 | S1, S2 | O.9 acceptance (a broken machine fails each check) |
  | C3 | The pipeline over scripted ports, the golden sessions G.1, 2, 4 and the three known-bad lines | C2 | S2 | O.9 acceptance; goldens reviewed against the pinned source (RES52) |
  | H0 | The owner actor and the dev CLI over fake adapters | C3, S1 | C5 | an input to a projection through the real queue, CLI-driven; S1 rerun on it |
  | C5 | Projection: audiences, never-empty, sentinel-secret and paired-state tests | C3 | H0 | O.9 acceptance |
  | K0 | The fixture contract: the playground's `Fixture` is a serialisation of the real shared-screen and seat projections; a host test projects a table state for the TV and a seat and parses it with `df_playground::fixture::Fixture` | C5 | W0 | the contract test, then the playground shows a real projection |
  | W0, W1 | The first `MachineView` and `MachineIntent` fields under the field ledger (RES33, RES40), the generated contract, the WASM feasibility gate of `rpc-transport.md`, and one view and one intent through the gRPC-over-WebSocket bridge | C5 | K0 | the bridge round trip in a browser; field ledger entry |
  | C4 | Pending and windows, two-phase apply, held turns, rulings | C5 | C6 | O.9 acceptance (M2-D4 to D7, restart without redraw) |
  | C6 | Table state and scenes (epochs, seats and presence, the hand-off stack) | C3 | C4 | O.9 acceptance |
  | C7 | Inspect and replay | C4, C6 | H1 | a recorded session replays to the same state hash |
  | H1 | The broker (the rebuilt `Hub` with text and image kinds, queue classes, leases and epochs) | H0 | C4 to C7 | the broker's own tests; every service call audited at one place |
  | H2 | The event sink to `df-observe` and the full CLI verb set (H.16) | H0 | H1 | the audit lines and `dump`, `viable`, `projection show` on a real run |
  | H3 | The first playable path: lobby, one story scene, one conversation, on a TV and a phone | C4 to C7, W1, K0, H1, H2 | T and V steps | RES44: an evaluator operates the TV and a phone; S1 rerun with the real projection |
  | SB | The scene builder (Describe, Background, Plate, Locate, Ready), the plate scheduler and the playout buffer | H1, S2 | G1 | RES44 plus the K.2 and K.3 timings measured from the logs |
  | G1 | The Director's camera stage (O.14) driving the same checked numbers as the dev verbs, with `director show` | C6, H0 | SB | a controlled clock; the O.14 acceptance cases that need no engine events |
  | G2, G3 | Engine wiring and the reasons (positions and speakers, then `hit`, `reveal`, `roll`, `spotlight`) | G1, H3 | machines | O.14 acceptance, including the hidden and visibility cases |
  | T0 to T2 | Rotation for elements, scale, flip, opacity, layer, the generic tween (O.13) | none | everything | O.13 acceptance and a browser measurement |
  | V0, V1 | Video in the library and on the page (O.15) | none | everything | O.15 acceptance and a browser measurement |
  | V2, V3 | Item sound from an element, kind sounds, loops, footsteps (O.15) | none for the playground; V4 needs G1 | everything | O.15 acceptance |
  | V4, V5 | Director cues and the projection rules; provider video, preload at a K.3 swap window | G1, SB | machines | O.15 acceptance and the measured start times |
  | T3 to T6 | Filters, camera pan and drift, path, timelines, derived condition transforms | T2; T6 needs C5 and G1 | machines | O.13 acceptance; T6 behind the audience rules |
  | M | The machines after H3, each behind a rules-readiness gate: travel, rest, shop and inventory, item inspector (needs S2 and SB), advancement, and combat last | H3, RR | G and V steps | the conformance suite, a golden session and RES44 per machine |

- **Gates.** *Gate A (spine ready)* is B0, C0 to C3 and S1 within budget: the engine accepts an input
  through the queue and pipeline over fakes and the owner thread is shown to fit its budget. *Gate B (first
  playable)* is H3 accepted under RES44. No machine beyond lobby and the minimal explore starts before
  Gate A, and no step that shows engine state to a person starts before K0 and W1. A machine starts only
  when RR shows its rules calls exist or are declined `not_supported_yet` by the adapter (RES31).
- **Order of the machines after H3, and why.** Dialogue (in H3) and explore need the fewest rules.
  Travel and rest need exploration and time rules that exist in part (M2-D43, M2-D46). Shop and
  inventory need the purse and item model (D20) and no SAM. The item inspector needs S2 and SB. Advancement
  needs the creation rules (R02, R16). Combat needs legal choices, spells and the tactical layer in
  `df-rules`, the largest gap, so it comes last and may need its own rules work first.
- **What runs in parallel and who works on it.** The spine (C0 to C7) is one sequence with one owner;
  the host steps follow it. The playground tracks (T, V and the part of G that needs no engine) are
  independent of the spine and can run at once in `df-playground` and `df-table-host`, one implementer per
  work item and no two in the same files (ADR 0001, ADR 0002). Rules work (RR, then the `df-rules`
  additions each machine needs) is a third track that touches `df-rules` only.
- **Freeze rule.** From the adoption of this order until Gate A, changes to this document are bug
  fixes, answers to open questions and corrections found by a review. A new design section (like O.13 to
  O.15) needs an owner request, because each one added since round 5 produced a round of its own review
  findings (round 6: 24 issues, about 19 from O.13 and O.14).
- **Review.** The order is reviewed like any other part of the plan: a critic checks that every
  dependency above exists in the section it cites, that no step shows engine state before K0 and W1, and
  that the acceptance column names a real surface. A step added later must name its row here.

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
- **Round 4.** The planned small round (re-test the M.2 register against the pinned source and check
  the new text for contradictions). Its findings were filed as GitHub issues #1490 to #1525 and are
  being applied in place: older prose is edited to agree with RES35 to RES52 and the M.2 register,
  the status and count lines are corrected, and section O gains an explicit list of departures from
  its own plan (O.12). Four review rounds have been run; the RES list ends at RES52.

