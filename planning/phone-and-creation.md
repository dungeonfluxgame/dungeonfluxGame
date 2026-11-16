# Phone controls and character creation (dev host)

Status: plan, 2026-11-15. No code yet. Owner decisions are marked **(owner, 2026-11-15)**.

Governing documents: [Client architecture](client-architecture.md) (roles, thin client, start
flow); [Gameplay engine redesign](gameplay-engine-redesign.md) (D3/D4 machines, D9 sheets, section
C.1 seat lanes, section D.4 seats, the M row: creation is the next machine after H3);
[Backstory-driven character creation](character-backstory-creation.md) (sections 4, 5, 9: the
picker, the free-text path, the phone flow); [Game refinement](game-refinement.md) REF-042,
REF-046 to REF-049, REF-058, REF-059, REF-061, REF-062, REF-063; [Rules support](rules-support.md)
and [Rules coverage](rules-coverage.md) R01 to R03; [Remote play](remote-play.md) (seat presence);
[Coding style](coding-style.md); `development/runbook-dev-playground.md` "Standing rules".

## What exists today (2026-11-15)

- **No phone page.** The playground (`df-playground`) draws a mock phone from fixture files
  (`view/phone.rs`: hero, narration, private note, choices, composer). Its composer already records
  the microphone (`voice_capture.rs`, `app/voice.rs` -> `/dev/speech/transcribe`). Nothing serves a
  phone page or joins a seat from a browser.
- **The engine has seats but no phone.** `df-table` has the lobby, explore and dialogue machines
  and a per-seat projection (`PhoneScene`: hero, status, narration, suggested, note);
  `df-table-host/src/machine_wire.rs` maps it to the wire's `MachineView`. Seats are driven from the
  dev CLI only.
- **Play runs on dev routes.** Free roam and the dice gauntlet are driven by runner scripts playing
  a persona (Danny, the warrior) through `persona declare|ask`, `move ...`, the check judge and the
  inventory API. The warrior's sheet is `development/market-threads/gauntlet_warrior.sheet.json`,
  read by `persona/check/sheet.rs`.
- **Creation rules.** `df-rules/src/creation_build.rs` (`creation_options`, `validate_build`):
  Fighter 1 and Soldier only, two ability arrays, the 9 SRD species with their sub-choices
  (`species.rs`). The richest sheet shape is the gauntlet's (Fighter 3, point buy, origin feat).
- **The old creator is gone.** The old engine's phone creator (portrait at lock-in, 2024 names,
  hero library) was removed in `14a2f715` (2026-11-02). Read it in git for ideas; do not restore it.
- **The host is loopback only.** `df-table-host/src/guard.rs` refuses any non-loopback host: the
  dev API has no password and must never face the network.
- `df-player` and `df-ui` hold older, unwired phone screens built for the old protocol. They are
  not extended here (see open item 3).

## Owner decisions (2026-11-15)

1. **Hybrid.** Join, lobby and creation go through the `df-table` engine (creation is its planned
   next machine). In play, the phone's controls first drive the routes free roam and the gauntlet
   use today, with the player's hero taking the scripted persona's place.
2. **Design any 2024 build.** A player may design any of the selected 2024 core species, classes
   and backgrounds (REF-042). What the rules cannot play is marked "Not in play yet" on the sheet
   and is never silently played as a Fighter.
3. **Optional depth.** One flow: a quick hero by default (name, a few story cards, Surprise me),
   and the player may open any part to go deeper (choose every option, the life-path stages, write
   or speak the story). Every path ends on the same review sheet.
4. **Portrait at lock-in.** Locking in makes the portrait and the reference sheet
   (`ref-<hero>.png`) so plates paint the player's own hero (REF-058, REF-059). A failed or unknown
   result still lets the hero join; the hero is painted from the sheet's look until it lands.

## State machines (2026-11-15)

Two levels. The table's machines (server, `df-table`) own the game; the phone's machines own only
presentation and connection, and the phone's mode always follows what the server's projection
says (client-architecture "Agreed division of responsibilities").

**Table machines** (redesign D3, RES28, section F; one active group machine per scene):

| Machine | Status | States |
| --- | --- | --- |
| lobby | built (H3) | `gathering`, `started` |
| explore | built (H3) | `presenting`, `awaiting_roll`, `resolved` |
| dialogue | built (H3) | `listening`, `awaiting_roll`, `concluded` |
| creation | next (P2) | per seat: `drafting`, `reviewing`, `ready` |
| combat | after creation (P5) | `Round(n)` > `Turn(creature)`: `AwaitingInput`, `Resolving`, `AwaitingResolution`, `Held` |
| rest, advancement | after combat | not yet specified |
| inventory, shop | after `Coins` | not yet specified |
| item inspector | after S2 and SB | not yet specified |
| travel | last (needs rules) | not yet specified |
| downtime, puzzle, chase, adventure end | later | not yet specified |

Under the hybrid decision, play first runs on the dev routes, whose own state is the story's
`phase` (`place`, or `way` with its stage) and the check judge's pending roll.

**Phone machines** (proposed; presentation only):

1. **Connection**: `loading` > `live` > `quiet` (reconnecting) > `live`; a new host `boot` resyncs
   the whole view. Built in P1a.
2. **Identity and seat**: `room` > `who_are_you` > `guest` | `friend` > `awaiting_approval` |
   `signing_in` > `choose_hero` > `seated`; `refused` and `room_ended` end it. A phone with a stored
   key goes straight to `seated`.
3. **Mode**: `lobby`, `creation`, `play`, `battle`, `rest`, `advancement`, `ended`, taken from the
   seat's projection; the phone never picks it.
4. **Panels in play**: Act is home; Talk, Roll, Inventory, Clues, Look closer, Map, My hero and
   Notebook are reached from the tab bar. A roll asked for, a reaction offered or a pending
   question from the server opens over the current panel.
5. **Small machines**: composer (`idle` > `typing` | `recording` > `transcribing` > `editing` >
   `sending` > `sent` | `declined`); look studio refine (`editing` > `refining` > `comparing` >
   `kept` | `discarded`, or `failed` keeping the old picture); roll (`asked` > `rolling` >
   `result` > `done`); reaction prompt (`offered` > `answered` | `timed_out`); pop-up (`shown` >
   `kept` | `expired`, kept in Recent); map tap (`idle` > `proposed` > `moving` | `refused`).

## Slices

Each slice is one bounded task with one implementer; the coordinator checks it live on the one dev
host (port 7879). Owed tests are marked `TODO(DEV-12)`.

### P1. Phone page and join

- A **player listener** beside the dev API: its own port, bound to the LAN, serving only the phone
  page and seat-scoped routes (join, view, input). It never forwards to `/dev/` routes. The dev API
  stays loopback only.
- **HTTPS** on the player listener with a local development certificate, because a phone browser
  allows the microphone only in a secure context. Plain HTTP is allowed when the mic is not needed.
- The **phone page** is a second entry of `df-playground` (shared look, `view/phone.rs`,
  `composer`), mounted at `/play`. It polls its seat's view the way the TV page polls today.
- The **TV lobby** shows the room code and a QR to the player listener's address.
- **Join**: the phone sends a name; the lobby machine seats it (`SeatJoined`); the phone keeps a
  random seat key in its own storage (REF-046 key, hashed on the host). A refresh or a dropped
  connection returns to the same seat. Up to six seats (REF-062).
- **Logs**: join, rejoin, leave, every tap and line, and each view the phone drew (revision and
  time), through the shared event log.
- Done when: two phones join from the room code, one refreshes and comes back to its seat, the TV
  lobby shows both, and the log shows each step.

#### From the camera to a hero (owner, 2026-11-15)

1. **Scan.** The TV lobby shows a QR. The phone's own camera app reads it and offers the link,
   `https://<player address>/play?room=<CODE>`. Typing the room code on `/play` does the same.
2. **Load.** The web app (the WASM phone page) loads and shows the room: the adventure's name and
   picture, who has joined so far. A room that ended or a wrong code says so plainly.
3. **Who are you?** One screen with the three kinds of player below. A phone that has been here
   before offers "Continue as <name>" first.
4. **Your hero.** The phone lists the heroes this player already has (portrait, name, species and
   class, last played) with "Play this hero", and "Make a new hero". Choosing one restores it
   ("Getting heroes back"); a new one goes to the creator (P3).
5. **Seat.** The player is seated; the TV lobby card shows them; the creator or the restored hero's
   review sheet opens on the phone.

For the QR to open without a warning, the phone must trust the player address's certificate, so
the certificate choice (open item 5) decides whether this works on the first scan.

#### Who is playing: three kinds of player (owner, 2026-11-15)

| Kind | Who | Signs in? | Where their heroes are kept | How they come back |
| --- | --- | --- | --- | --- |
| **Guest** | anyone at the table, no account | no | the host's hero library, under a random key kept in that phone's browser (REF-046) | the same phone and browser; clearing the browser loses them (said plainly on the phone); an account can claim them later with the key (REF-021) |
| **Friend on an account** | a regular player who has no account of their own; the account holder (usually the host) keeps a profile for them | no; the account holder approves them | the account holder's account, under that friend's profile | from any phone: pick the profile on "Who are you?", and the host approves on the TV or launcher (the start flow's "the host allows it") |
| **Own account** | a player with their own account | yes, on the phone | their own account | from any phone and at any host's table, after signing in |

- The phone never decides which kind a player is: the host checks the key, the approval or the
  sign-in, and the seat carries the kind.
- A guest can become a friend on an account (the holder adds a profile and takes in their heroes
  with the phone's key) or make their own account (REF-021 claim, needing the key and the
  account).
- A friend can later move to their own account; their heroes move with them, with the holder's
  approval.
- The TV shows only the name and portrait, never the kind or the account.

#### Getting heroes back (owner, 2026-11-15)

- The phone lists the heroes the player may see: a guest's under their key, a friend's under their
  profile, an account's under the account. Nobody sees another player's heroes.
- **Into a new campaign**: the hero brings its identity (name, look, portrait, reference sheet,
  story, design choices). The campaign sets level, gear and campaign facts (REF-047).
- **Resuming a campaign**: the hero comes back exactly as the campaign's save left it.
- Every save is a new revision; a campaign records which revision it started from (REF-048).
- A hero is used at one seat at a time; a second phone asking for a hero already seated gets "in
  play at another seat" and the host may hand it over.
- An unfinished creator draft is kept on that phone only (REF-049).

### P2. The creation machine (`df-table`)

- The lobby hands each seat to **creation**; creation hands the table to play when every seat is
  ready. Seat states: `drafting`, `reviewing`, `ready`. A ready seat may reopen until the start.
- **Offers** come from `df-rules`. The catalog lists every 2024 core species, class and background
  with a status: playable, or design only ("Not in play yet") with the reason.
- **The server derives the sheet**: ability scores (standard array, point buy, or the arrays the
  rules have), modifiers, proficiency, skills, saves, hit points, armour class, languages,
  equipment. The phone never sends a number.
- **Output**: one sheet per hero in the shape the check judge reads (the gauntlet sheet's fields),
  with look, public story and the design-only marks.
- Dev CLI verbs for every step (D19): list offers, set a choice, submit, reopen, show a seat's
  sheet.
- Done when: the conformance suite and a golden session pass; a hero made from the CLI rolls a
  check in the gauntlet.

### P3. Creation screens on the phone

- **Quick hero**: name (REF-061 rule, checked on the phone and the server), three story cards
  (job, how you fight, what you want), Surprise me. The server proposes the build.
- **Deeper, when the player wants it**: species and sub-choices, class, background, ability scores,
  skills, languages, equipment, look (the appearance controls and a free description), each with a
  one-line "because".
- **Review sheet**: story and build, each line tappable to change, design-only marks shown plainly.

#### The look studio (owner, 2026-11-15)

The hero's look is refined step by step on one fixed pose, so every version is comparable and the
final picture is a clean reference for the plates.

- **The base pose.** Every hero starts from a base image with a default pose: one full-body figure,
  head to boots, facing the camera, standing relaxed with arms a little away from the body, plain
  studio background, 2:3. It is data in the media library, not code: one base per body frame
  (small for Halfling and Gnome, medium, tall for Goliath), chosen by the species. A three-view
  version of it (front, side, back) is the template for the reference sheet at lock-in.
- **Base controls.** A few server-defined controls, drawn by the phone: sliders for height within
  the species' range, build, age, and hair length; swatches for skin, hair and eye colour; chips
  for face shape, hair style and facial hair; and the species' own (horns, ear shape, scale
  colour, tail) where it has them. The server sends each control's id, kind and range; the phone
  never invents one.
- **Fine details, in the player's words.** A free text box for anything the controls do not
  cover: scars, tattoos, earrings, piercings, freckles, a missing finger, war paint, a cloak pin.
  Jev reads the words into a list of details against a closed schema (kind: scar, tattoo,
  piercing, jewellery, marking, body, clothing, other; where on the body; what it looks like in the
  player's words). The phone shows each detail as a chip the player can remove or change, so a
  scar added once stays on every later version until it is removed. No word list decides any of
  this.
- **Refine.** Changing controls or details costs nothing; the Refine button makes one picture.
  Each refine edits the version the player kept, with the base pose as a reference: keep the pose,
  framing, face and everything not changed, and apply only what changed since the kept version.
  The server works out that change list from the draft (for example "add a scar across the left
  cheek; hair from short to shoulder length"), so the prompt asks for the change, not the whole
  hero again. The first refine renders from the base pose and the whole description.
- **Keep or go back.** A new version shows beside the kept one: Keep it, or keep the old one. A
  strip of earlier versions lets the player go back to any of them; Start again renders anew from
  the base pose. A refine never replaces the kept version until the player keeps it, and a result
  that arrives for an older draft is shown as old, never applied (REF-032).
- **What reaches the image model**: the base pose, the kept version, the species, class and gear
  as public visual context, the controls and the details. Never the name or the private story
  ("Names never reach image prompts"). Jev's maturity routing (REF-039) runs on each refine.
- **Lock in**: the kept version becomes the approved portrait, and the reference sheet
  `ref-<hero>.png` is made from it on the three-view pose template. A design with no kept version
  is rendered once at lock-in (REF-059).
- **Logs**: each refine's change list, the time it took, kept or not, and the version it came
  from.
- **Lock in** makes the portrait and `ref-<hero>.png` (the existing image service and reference
  sheet path); the phone shows the portrait arriving.
- The **draft** lives in the phone's storage (REF-049) and survives a refresh.
- **TV**: each seat card shows the name once chosen, "making their hero...", then the portrait and
  "Ready". Nothing private reaches the TV.
- Done when: two phones make two different heroes, both portraits land, the first plate paints both
  heroes from their sheets, and a refresh mid-draft keeps the draft.

### P4. Play controls: the phone runs the game for its hero (owner, 2026-11-15)

In play the phone is the player's whole control surface: acting, talking with characters,
inventory, puzzles, rolls and the hero. It is a set of panels over one seat view. Each panel only
draws what the host sends and sends the player's input. What a line or tap means is Jev's and the
check judge's to decide, never the phone's (no string matching). The seat's hero takes the
scripted persona's place: the free roam and gauntlet beats take the hero from the seat instead of
a fixed persona id.

| Panel | The player can | Backed by (today) |
| --- | --- | --- |
| **Act** (home) | tap a suggested action, or say/do anything typed or by hold-to-talk (an editable transcript before sending); see the narration and what changed | `choices.rs` rules; `persona declare`, then Jev (`move interpret`); the composer and voice capture |
| **Talk** | see who is in the scene (by learned name and portrait), pick whom to address or speak to the room, follow the conversation thread, leave it | `persona ask` / `converse`; the scene's cast; names are learned, never given |
| **Roll** | when a check is called: see the check, the stakes and any advantage, press Roll; see the result while the TV shows the roll card | the check judge and roll card (`persona/check`) |
| **Inventory** | see what the hero carries with pictures; use, give, drop, inspect; buy, sell and trade with a keeper (price shown, server settles coin and goods) | the inventory API (`/dev/world/inventory`, `transfer`, `history`); item sheets `item-<owner>-<item>.png`; market trades |
| **Clues** (puzzles) | see every piece found (picture, what a close look showed, worked-out marks); look closer at one; put forward an idea about how pieces fit, for the DM to judge; hints only nudge (never name which pieces go together) | the clue route (`persona story clues`) and the clue board; the story's `findings` |
| **Look closer** (item explorer, D22, D23) | tap a thing in the picture or the inventory to inspect it; the TV shows the insert | the frame route's `inspected_item` and inserts |
| **Map** (Go) | tap a place on the map to move there, or say where to go as before (see "Moving by map") | the story's places and ways; the move pipeline (`move design`, `move act`, the walk stages) |
| **My hero** | the sheet (abilities, skills, HP, AC, features, design-only marks), conditions, memories | the seat's sheet (P2); the memory API |
| **Notebook** | write a note, snap the main screen, keep a note that pops up, read and share them (see "The notebook") | a new notebook store on the host, kept with the campaign |

#### Moving by map (owner, 2026-11-15)

Normally a player tells the DM where they go and Jev reads the line as a MOVE. The phone also
offers a map: tapping a place there is the same move without the reading.

- **What the map shows**: the places the party knows: visited, seen on the way or named to them.
  Each place is a node with its plate as a thumbnail; lines are the ways between them; "you are
  here" marks the party's place, or the stage of the way it is on. A quest shows its plotted places
  and ways once the party has learned of them; free roam shows the town as it grows, since its
  places are made as the party goes. A place nobody has told the party of stays off the map.
- **Who draws it**: the host sends the map as data (places, ways, positions, thumbnails,
  where the party is); the phone lays it out. A painted map picture is a later option, not part
  of this slice.
- **A tap is a typed move**: the phone sends `move to <place id>`. It skips Jev's reading (a tap
  is not words to read) and enters the move pipeline where Jev's MOVE does: the same check (is
  the place reachable from here, is the party free to leave), then the same design, walk stages,
  plate, music and arrival. One path for both, so a tapped move and a spoken one look the same on
  the TV and in the log (`MOVE source=tap|line`).
- **Refused taps**: a place that cannot be reached now (a fight, a locked way, mid-conversation)
  shows the reason on the phone and offers what can be done instead; nothing moves.
- **Party**: a move takes the party together (the engine's scene rules); with several phones, a
  tap proposes the move and the others see it, as a spoken move does. The confirmation rule for a
  party move follows the engine's seat rules (open item 6).
- **Within a place**: tapping a person or thing in the scene picture on the phone means go up to
  it or look at it (the Look closer and Talk panels), not a move to another place.
- **Fights** keep their own grid; tapping a square to move in combat belongs to the combat
  machine (P5).

#### The notebook (owner, 2026-11-15)

Each player keeps a notebook on the phone: what they want to remember, in their own words and
pictures.

- **Write a note**: a quick box always one tap away, typed or by hold-to-talk (an editable
  transcript). A note may be pinned to a person, place, item or clue the hero knows, by tapping it.
- **Snap the main screen**: one tap saves what the TV shows now: the plate, the caption or
  narration line, who is in the picture (by learned name) and when. The host makes the snap from
  what it sent the TV, so it is exact, costs no picture and holds nothing that seat may not see.
  It is not a photo of the TV or a browser screenshot.
- **Keep what pops up**: when the game turns up something worth remembering (a name learned, a new
  place, a task, a clue, an item, a line someone said), the phone shows it as a short pop-up with
  Keep. Every line in the narration and the conversation thread can also be kept by a tap. A
  pop-up not kept waits in "Recent" for the rest of the scene.
- **Reading it**: newest first, filtered by People, Places, Items and clues, Snaps and My notes;
  each entry shows its picture, where and when it was made, and what it is pinned to.
- **Private by default**: a note is the player's own; Share sends a copy to the party's
  notebooks. A shared snap or note may be shown on the TV for a moment when the player asks.
- **Lessons from the old notebook** (NTE-03 to NTE-06): one entry per fact, never a copy per
  scene; no raw ids, log lines or scene keys shown; a note is announced in words or not at all; a
  task's state comes from the story, not a guess.
- **Kept with the campaign**: notes and snaps belong to the player in that campaign and come back
  on Resume; they do not travel with the hero into another campaign (REF-047).
- **Logs**: note written, snapped, kept, shared (by kind and entry id, never the note's text).

#### All panels

- **One scroll per page; tabs separate concerns** (owner, 2026-11-15). Every phone page scrolls as
  one region: no nested scroll containers, no capped inner lists, no sideways-scrolling strips.
  Only the header (top) and the composer and tab bar (bottom) are pinned. A page that would hold
  two concerns is split into tabs instead: in play, **Story** (scene picture, narration, private
  note) and **Act** (suggested actions, grouped choices, composer) are separate tabs.
- **The scene behind every page** (owner, 2026-11-15). The phone follows the concept art: the
  current scene plate fills the background, dimmed and vignetted with dark fades top and bottom,
  and the panels are translucent glass over it. Pages without a scene (join, lobby) use the room's
  or adventure's picture the same way.
- Every panel shows the server's current state after a refresh or reconnect; a draft line or open
  panel is kept on the phone.
- Private things (a private note, a secret clue, a whisper) reach only that seat's phone.
- Order of build: Act and Roll first (enough to play a gauntlet station and a free roam move), then
  Talk, My hero and Notebook, then Inventory and Look closer, then Clues and Map.
- Logs: each panel opened, each tap and line sent, and when its answer was drawn, so a turn's pace
  can be measured from the logs.
- Done when: a person on a phone plays a gauntlet station, a free roam walk with a conversation and
  a trade, and a puzzle scene of the quest to its end, by tap and by voice, with the timings
  reported from the logs.

### P5. Battle mode on the phone (owner, 2026-11-15)

When a fight starts the phone switches to battle mode; the other panels (Notebook, My hero,
Inventory) stay reachable. It needs the combat machine, which is the engine's next machine after
creation (redesign M row: combat for the Fighter subset; what the rules cannot do yet is declined
`not_supported_yet` and said in words). The TV keeps no grid (owner decision: a thin ring under the
acting figure and a target); the grid lives on the phone.

- **The grid**: the fight's map from the combat machine: squares, walls, cover, difficult
  terrain, and a token with a portrait for each hero and each creature the hero can see. Pinch to
  zoom, drag to pan, "centre on me".
- **Moving**: on the hero's turn the squares they can reach with the movement left are lit; tapping
  one shows the path and its cost, and warns when it leaves a creature's reach (an opportunity
  attack); Move confirms. Movement left is counted down in feet.
- **Aiming**: tapping a creature makes it the target and shows range, cover and anything that gives
  advantage or disadvantage, as the rules say it.
- **Turn order**: a strip of portraits in initiative order, whose turn it is, and the round.
- **What I can do**: Action, Bonus Action, Reaction and Movement shown as spent or not. The actions
  as big cards: Attack (each weapon as a card, not a list), Dash, Dodge, Disengage, Help, Hide,
  Ready, Use an item, the class's features (Second Wind, Action Surge), spells where supported, and
  End turn. What cannot be done now is greyed out with the reason.
- **Say anything**: the composer stays, for what no card covers ("I kick the brazier into them"),
  read by Jev against the fight's legal choices.
- **Rolls**: attack, damage, saves and death saves go through the Roll panel; the TV shows the
  card.
- **Reactions off turn**: a prompt when the hero can react (an opportunity attack), with Yes and
  No, held for the time the engine's rules give.
- **My state**: hit points, temporary hit points, armour class, conditions as chips with their
  meaning, concentration, and death saves as a tally ("2 successes, 1 failure", DTH-01).
- **What the hero knows of the others**: allies' hit points; creatures by learned name and a
  wound word (unhurt, bloodied, near death), never their numbers.
- **Lessons from the old phone fight** (PHN-01 to PHN-08): no developer notes or raw rule labels
  shown; weapons as cards, not a long list; walls and cover drawn; controls updated in place so a
  tap is never lost.
- **Logs**: each tap, the move or action sent, the engine's answer, and when the phone drew it.
- Done when: two phones fight a short battle to its end (a win, and a loss with death saves),
  every action taken from the grid and the cards, with the timings from the logs.

### P6. Later

The life-path picker with card art, the free-text path read by Jev, party ties, the hero library
across nights (REF-046 to REF-048), and playable classes beyond the Fighter, each with its rules
work.

## Open items

1. **Design-only heroes in play.** A design-only hero (a Wizard, say) can roll ability checks and
   saves from its sheet, but has no class features or spells. Proposed: allow it in free roam and
   the gauntlet with those marked "Not in play yet"; refuse it where a machine needs the missing
   rules (combat).
2. **Backgrounds outside the SRD.** Only 4 of the 16 backgrounds are in SRD 5.2.1, the only source
   acquired. The other 12 can be names and story only until a source is admitted.
3. **`df-player` and `df-ui`.** These older phone screens are not reused. Decide whether to remove
   them once P3 lands.
4. **Transport.** The phone polls the dev host like the TV page. The gRPC-over-WebSocket bridge
   ([RPC transport](rpc-transport.md)) stays the product path and is not part of these slices.
5. **Certificate.** How the development certificate is made and trusted on a phone (a local CA the
   owner installs once, or a self-signed certificate accepted per phone).
6. **A party move from one phone.** Whether one player's tap (or line) moves everyone at once, or
   asks the other seats to agree first. Proposed: it moves the party, and the other phones get a
   short notice they can answer with "wait".
7. **Places the party only heard of.** Whether a place named in talk but never seen goes on the map
   (as an unvisited node) and can be tapped. Proposed: yes; the move then designs it on arrival, as
   a spoken move to a new place does.
8. **Pose drift.** Whether each refine is checked for keeping the pose (a judged look at the new
   version against the base) and asked again when it strays. Proposed: measure first over a few
   dozen refines, then decide.
9. **The base pose images.** They are made once and kept in the media library. Proposed: generate
   three (small, medium, tall) and have the owner pick each before the look studio is built.
10. **Friends on an account versus hosted play.** Launch decision 9
    (`account-service-research.md` 2.1) says every hosted player signs in and guests are local
    only. Friends on an account would let an unsigned player join a hosted table under the
    holder's account. Owner to decide whether friends (and guests) may play hosted tables, and
    whose balance they play on.
11. **Friend profiles and age.** The age gate and voice consent (roadmap "Business and service",
    decision 5) apply to friend profiles too; who confirms a friend's age, and whether a friend's
    voice can be recorded, needs the owner and counsel.
12. **Where the hero library lives.** On the dev host, files in the playground directory; in the
    product, PostgreSQL ([Storage architecture](storage-architecture.md)). Accounts are not wired to
    the dev host: guests and friends can be built first on the dev host; signing in needs the
    account service (`df-account-app`, WorkOS per launch decision 28) and comes after.
