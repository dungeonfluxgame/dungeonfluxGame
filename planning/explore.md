# Explore: dynamic single-player free roam

Owner, 2026-11-17: "let me build a character and let me manually use the phone client to explore
the world" and "do not hard code anything, Jev all the things, make it dynamic". This governs
`development/market-threads/explore.beats` and the runner's explore loop. It applies
`planning/interaction-engine.md` (Intent, affordances and compound plans) to the playground roam.

## What was wrong

The first explore run reused the painter's day roam: a fixed cycle (ask a move, then ask a look,
trade only on a `market` step) driven by a fixed `@move` list of way kinds and fixed cards. Any
line that was not a move to a named place was dropped ("I turn around and take a view at the
vista", "I take a closer look at the sign"), speech at the look question reached no one, and a
market chosen by the player got no trade because the list said `town`.

## The loop

- **One open question per turn.** The phone asks "What do you do?". The player types or taps
  anything. There is no fixed order of questions and no move list; the run goes on until the
  player says they are done (no turn cap in dev).
- **Jev reads every line** into one intent from a closed schema, validated in code against ids
  and enums the host supplies for the current scene:
  - `move`: back to a place the hero has been (`revisit`, one of the places the runner offers:
    "I go back to the market", "back into town" from outside a town place been to), shown again
    as it was left with no new design; to a destination named in words (the way is one of the
    scenario's `@way` ids), toward
    something on the plate (its id), toward a direction (ahead, left, right, behind) with what
    lies there, or toward something named that the scene has no id for yet. A line that sets off
    somewhere is always a move, never a look.
  - `look`: at a thing or person on the plate (id), a way (a direction: the hero turns to face
    it), something named that the scene has no id for yet, or the view as a whole.
  - `talk`: to a person on the plate (id), or to whoever is there (the addressee judge's rules).
  - `trade`: with a person who sells (id), buying, selling or asking for wares.
  - `act`: anything else done in place. The hero says the deed first; then the existing check
    judge calls a check when the rules say one is needed (call, roll, result); then what the deed
    got or used is judged as finished work told how the roll came out, and the DM tells what
    changed hands. A required check that does not resolve resolves nothing: the player is told
    and offered the line again.
  - `ask`: a question to the DM (what can I do here, what is that building); the DM answers it
    from the scene and the plate and nothing is acted on (`planning/interaction-engine.md`:
    questions never silently execute).
  - `done`: the player ends the roam.
  A line that does more than one thing ("I thank her, wave goodbye and walk off toward the tower")
  is a bounded plan: its first intent and up to two follow-on steps, each checked as an intent of
  its own and carried out in order, the line said once. A move by a way goes where the judge
  read that step to go: the move is not read again. A step the line does not commit to now
  ("and maybe later I'll climb down") is left out; talk and trade are speech and always said. A
  field the chosen kind does not use is left out (`unused_field`), never a refusal, and a line no
  reading stands for is played as an act
  (`INTENT fallback`): no line is ever dropped.
  During an approach, "keep walking" (or the "Keep walking toward …" card, offered first) goes
  on with the same approach and takes the step made ahead; only a clearly different target starts
  a new one. The judge names what the line names in its own words, and asserts that a target id
  IS that thing; when nothing in the scene is, the plate is searched for it. A line with no
  readable meaning is only ever a clarify, never a move, talk, trade, act or done.
  The judge takes the most plausible reading. A `clarify` intent asks one short question back
  only when two or more concrete readings conflict: it lists them (ids, ways or directions) and
  the question names them. Nothing is silently dropped and no keyword or string matching decides
  any of it.
- **Every plate has a scene.** The opening plate (no design) is the active place, and what the
  hero sees on the picture is added to it as its things before the first ask. Something a line
  names that the scene lacks is searched for on the picture itself: first a grounded query (the
  locate path's SAM, prompted with Jev's short noun of it, nothing stored), then the hero's
  perception with vision. What either finds is added and the line read again; only what neither
  finds may the DM say is not there. For a talk or a trade, the target is the person addressed and
  what the line names is its subject.
- **The DM tells looks, acts and answers without a story**, from the place, its things and people,
  the plate and what the hero sees on it. A look or an answer tells only what is: nothing moves or
  changes in it.
- **Dispatch to the existing handlers**: move to the place design and plate (with walking
  stages), look to the frame judge's insert, talk to the persona answer, trade to the inventory
  deal path (buy, haggle, pay with change), act to DM narration and checks. One handler per
  intent; the roam and the journey share the seat helpers.
- **Suggested cards are dynamic**, built each turn from the scene the host knows: the people on
  the plate (talk), the things worth a close look (look), the ways on (move), a seller (trade).
  The player can always type instead.

## Movement toward what is on screen

Owner, 2026-11-17: "if I see something on screen I need to be able to walk towards it and see it
up close or to walk forever towards it".

- **Anything visible is a target.** The host knows what each plate shows: the place design's
  things and people, and the located regions of the picture (the existing scan). Jev's `move`
  intent can name one of those ids as the target ("I walk toward the tower", "I go up to the
  well") or a direction in the view, validated against the plate's ids; a destination named in
  words alone ("the market") stays a place move as before.
- **An approach is a sequence of steps along one line of sight.** Each step makes a new plate
  that continues the previous view: the same scene from closer in, the target larger and more
  detailed, the hero walking toward it, the ground between passed by. It reuses the walk stages
  and the place design (no second image path), following the previous plate as its reference.
- **Distance decides how far one step goes.** The design gives each target a distance band
  (within reach, across the square, down the road, on the horizon). A near target is reached in a
  step or two: the step that goes up to it is a plate of it up close, made ahead like any step,
  and the DM tells what the hero sees there (and what is newly in view after every step;
  the card reads "Go up to …" once it is within reach). A far one
  takes as many steps as the player keeps walking: the land changes along the way (new places
  are designed between), and a target on the horizon draws closer for as long as they walk.
  A walk along a way (a direction: "along the town wall to the left") has no destination and is
  never reached: it goes on, its card "Keep walking …", until the player names where to.
- **Every step is a turn.** After each step the phone asks "What do you do?" again, with a card
  to keep walking toward the target, so the player can stop, look, talk or turn at any point.
  The step logs `MOVE approach target= step= distance= plate=` with its timing.
- **The next step is made ahead.** A plate takes 70 to 150 s today (run explore-1). As soon as an
  approach step lands, the host starts the next step toward the same target in the background,
  so "keep walking" shows it at once. A step not taken is kept in the library, not shown. Ahead
  work counts toward the plate queue's limits like any other plate.

## What is said, and to whom

- **Only spoken words are said aloud.** Owner, 2026-11-17: "actions shouldnt be said out alout
  unless the prompt says to say it out loud". The judge gives `spoken`, the exact words of the
  line said aloud (a substring of it, checked in code; a talk or a trade may render reported
  speech). A deed alone is logged `[said NN] .. spoken=none | does: ..` and never voiced.
- **A line to several people is answered by each.** For a talk or a trade said to more than one
  person present ("good day to you both"), the judge lists `addressees` (ids of the people there,
  in the order they answer, the target first). Each answers in turn and is told what was just
  answered before them. With no addressee named the addressee judge decides; a line plainly
  spoken to someone present is never said to no one.
- **What the hero carries is the hero's.** The judge is given the hero's bag (`carries`): a line
  about a carried thing ("lift the spear above my head") is an act on it, never a look or a move
  toward something in the world, and a word that makes no sense is read as the word most likely
  meant.

## Things, doors and going inside

- **A search adds what the plate shows.** The search prompt is Jev's plain noun; the region the
  segmentation is surest of is used. When the eyes take that region for a thing the place already
  has, it is only a candidate: it is that thing when the line read again names it, else the region
  is added as the new thing. Where a region sits in the picture is for the log only; the DM tells
  where a thing is in the place.
- **Doors and gates keep their state.** An act on a thing of the scene may set `sets` (open,
  closed, locked, broken), decided by the act's outcome: a failed opening leaves it closed. The
  state is a note on the thing (`world thing note`: one note per kind, told after the thing's own
  look, which is never overwritten), so the judges and the DM read it with the thing, and the
  thing is renamed when its name told the old state (`renamed`: the open timber gate,
  shut, is the timber gate). Going into something closed or locked is refused with that reason;
  going into something open needs no second check. A way on (a thing whose id is a direction) has
  no state.
- **Going inside and peeking in.** A move into a thing of the scene (`enter`) designs its inside
  as a place of its own; stepping back out revisits the place outside. A look into or through
  something (`into`: "I peek through the window of the nearest shop") tells what is inside, never
  the outside. What is looked into is the building, never its window: the thing that is or holds
  it (`enter`), else the picture is searched for the building, and when it is not found the inside
  is still told from the line's words. What was told is kept as a note on the thing, so walking
  in matches. The DM is given what is looked into, an act the rules forbid, a stance held and
  words nobody answered as typed fields of its own, never as orders in the outcome.
- **What is left lies on the ground.** A thrown or dropped thing (a line's change, or the Bag
  page's Drop) goes onto the ground of the place it was left at (`ground-<place>`), shown in the
  place's things with how many lie there, and can be walked to and picked back up there. The DM is
  told where it now lies. A pose plate takes the whole ground along, and a step toward a lying
  thing takes that thing, since each plate is a place of its own.

## Poses

- **Only a held stance is painted.** The judge sets `pose` for a stance the hero holds after the
  deed (crouching, sitting, a pose struck and held, a weapon raised and held up); a bow, a wave or
  a nod has none. The DM tells the stance first; then a plate of the same spot is made from the
  plate on screen with the stance as its first instruction. The place keeps its things, their ids
  and states, and its ground.
- **The stance is checked.** A closed vision check asks whether the painted figure takes the
  stance; when it does not and the first painting was quick, it is painted once more. The plate is
  bounded like an insert (`EXPLORE_POSE_WAIT`): past it the turn goes on from the plate on screen.
  Logs `POSE plate= pose= ms= checked= shows= repainted=`.

## The people of a place

- **A resident is the person as painted.** When a place is arrived at, the hero's eyes read how
  each person the scan named looks on the plate, and their persona is made to fit that figure; the
  picture wins over the design's words. Logs `PERSONA painted plate= people=` and
  `PERSONA matched_look id= from=plate|design`.
- **Names are their own.** A new persona's name is never one already given (the stored personas'
  and a file kept across runs, `DF_NAMES_GIVEN`), and a closed check reads whether the name fits
  how the person presents as painted; a reused or misfitting name is written again, at most three
  times. Logs `PERSONA name_checked` and `PERSONA name_given`.

## Done when

From the phone alone, a player can in one run: look at the view and at a named thing without
moving, talk to a person on the plate and get an answer, ask a seller for wares and buy or haggle
with coins and weight changing, walk to a place they name (market, road, a landmark) with the
right kind of arrival, walk toward something seen on the plate and reach it up close in steps,
keep walking toward something on the horizon over several steps, have an in-place action narrated
(and checked when the rules call for it), and end the roam. Every turn logs its intent
(`INTENT kind= target= way=`) and its handler.
