# Runbook: the dev playground, personas, move transitions and plates

Verified live on 2026-11-07 against commit `f2a0160f` plus the market and residents work (Windows ARM64, Git Bash). Every
recipe below was run end to end. The flowing quest ("Play the quest end to end") was verified on
2026-11-08 at `f0bd97da` (run 111: market, inn, graveyard, the trap sprung, the end). This is
a development runbook for the dev host, not the
shipped launcher. Plan: [scene-transitions.md](../planning/scene-transitions.md).

## What works

| Capability | Verb | Backend |
| --- | --- | --- |
| Persona coverage, ask, backfill, say, generate | `persona ...` | Codex text (Responses has failure handling only: the OpenAI key had no credit) |
| Characters talk to each other, paced | `persona converse` | Codex text |
| A character's line is read as a MOVE by Jev (a model call) | `move interpret\|design\|brief\|say\|act` | Codex text |
| Scene design and residents from the move | `move design` | Codex text |
| A new character is invented for the place (a fruit vendor at a market), painted into the plate, found by the scan and given a persona you can talk to | `move act ... plate=NAME` then `persona ask\|converse` | Codex text + Codex images + fal |
| Image prompt and character cards from the design | `move brief` | pure code |
| New plate generated, characters found, backdrop swapped, spinner shown | `move act <who> plate=NAME ...` | Codex images + fal SAM scan + Codex text |
| One art style for every plate | `BriefStyle` in `movement/brief.rs` | none |

## One-time setup

1. **Dedicated Codex image home.** The image service refuses your own Codex home (it has a
   `config.toml`: error key `home_has_config`). Make a separate one and sign in:
   `CODEX_HOME=<repo>/artifacts/codex-image-home codex login` (a browser opens; sign in
   yourself). Never copy `auth.json`. `artifacts/` is gitignored.
2. **fal key** for the character scan: `DF_FAL_KEY` in `.env` (the host does not read `.env`;
   export it into the host's environment, never print it). The same goes for the voices:
   `DF_ELEVENLABS_API_KEY`. Without it, `speech status` reads `configured=false` and no line is
   voiced, even with TTS on in the playground (run 97).
3. **Find the current Codex executable.** The desktop app moves it when it updates, and the
   text service then reads as unconfigured (`missing=DF_TEXT_CODEX_EXECUTABLE`):
   `ls -d "$LOCALAPPDATA/OpenAI/Codex/bin/"*/codex.exe`.

## Build (warm lane)

Use one shared target directory, never `--target-dir` per attempt. Stop the host first: it
locks `df-table.exe`. Stop only your own host, by the port it serves. Other agents run hosts of
their own on other ports and lanes, and a kill by name stops theirs too.

```bash
export CARGO_TARGET_DIR=$PWD/artifacts/build/bubbles RUST_MIN_STACK=67108864
P=$(powershell -NoProfile -Command "(Get-CimInstance Win32_Process | Where-Object {
  \$_.CommandLine -like '*serve --port 7879*' -and \$_.Name -like 'df-table*' }).ProcessId")
[ -n "$P" ] && powershell -NoProfile -Command "Stop-Process -Id $P -Force"
cargo --offline build -p df-table-host --bin df-table
```

`cargo --offline check -p df-table-host --lib --tests` is the fast compile check. A build
that prints nothing may have failed: look for `Finished`.

## Launch the host

Use a scratch copy of the playground so persona and library writes do not touch the repo.

```bash
mkdir -p artifacts/tmp/persona-live && cp -r assets/playground artifacts/tmp/persona-live/playground
X='C:\Users\<you>\AppData\Local\OpenAI\Codex\bin\<hash>\codex.exe'
export DF_FAL_KEY=$(grep -E '^DF_FAL_KEY=' .env | cut -d= -f1 --complement | tr -d '"\r')
export DF_ELEVENLABS_API_KEY=$(grep -E '^DF_ELEVENLABS_API_KEY=' .env | cut -d= -f1 --complement |
  tr -d '"\r')
export DF_TEXT_BACKEND=codex DF_TEXT_CODEX_EXECUTABLE="$X"
export DF_IMAGERY_MODE=live DF_IMAGERY_BACKEND=codex-cli DF_IMAGERY_CODEX_EXECUTABLE="$X" \
  CODEX_HOME="$PWD/artifacts/codex-image-home"
D=./artifacts/build/bubbles/debug/df-table.exe
nohup $D serve --port 7879 --playground artifacts/tmp/persona-live/playground --quiet \
  > artifacts/tmp/persona-live/host.log 2>&1 &
$D dev --port 7879 playground go ui_roleplay_thought   # the tavern scene, five characters
$D dev --port 7879 services                             # text configured=true, imagegen live
```

Page: `http://localhost:7879/playground/`. Use your own port and stop only your own host.
Leave out `--quiet` when you want the audit lines (`PERSONA story_next served=..`, `SPEECH
synthesized`, `TEXT rejected`) in the host log: they are how a run's pace, voices and retries
are read.

## Recipes

Run from the repo root with `D=./artifacts/build/bubbles/debug/df-table.exe`.

```bash
# Persona mechanisms (text only, no image spend)
$D dev --port 7879 persona ask innkeeper "Where do you keep your ledger?"
$D dev --port 7879 persona say elf "A stranger asks about the road north."
$D dev --port 7879 persona coverage elf "Why did you leave your home city?"
$D dev --port 7879 persona backfill elf gap=personality:how_she_acts_when_lost
$D dev --port 7879 persona generate wizard overwrite=true

# Characters talk to each other (paced to reading time; pace=false to skip waits)
$D dev --port 7879 persona converse turns=8 opener=warrior "which road to take north"

# Jev and the transition, stage by stage (no image spend)
$D dev --port 7879 move interpret "We walk to the town square"
$D dev --port 7879 move design "The elf and the warrior go out to the balcony"
$D dev --port 7879 move act elf "You and the warrior decide to take the balcony. Tell the table."

# The full transition: the elf speaks, Jev reads a MOVE, a new plate replaces the backdrop
$D dev --port 7879 move act elf plate=balcony "You and the warrior decide to take the \
balcony for some air. Tell the table in your own words."
```

A `plate=` run takes about 60 to 75 s (one Codex image, one fal scan, three text calls) and
costs real money. Options such as `plate=NAME` go before the text.

Expected trace: `interpreting, designing, briefing, generating, locating, placing, live`,
no error. Only the characters Jev says move are in the design and the prompt; the rest are
`left behind` and are absent from the picture.

## Walk-through: balcony to a market and a vendor you can talk to

This is the run that came out well (2026-11-07). The point: one spoken line moves the party,
the model invents who lives at the destination, and that person becomes a real character.

1. **Start on the balcony.** After a host restart the scene is the tavern. Put the balcony
   plate back (its layout is stored, so the cast is the elf and the warrior):
   `$D dev --port 7879 scene image tv-backdrop balcony-real.png`, then `scene cast`.
2. **Check the design first, text only (no image spend).** The elf speaks, Jev reads the
   MOVE, the model designs the place and any residents:
   `$D dev --port 7879 move act elf "You and the warrior have had enough of the balcony. You decide to take the warrior down into the town market to buy fruit for the road. Tell the table where you are going, in your own words."`
   Expect: `reading: move ... who=elf, warrior to=the market`, a `design:` with
   `place`, `mood`, `elements`, one `resident fruit_vendor (...)` with a visible look, and
   the prompt naming all three. It stops with "give plate=NAME" and no image is made.
3. **Run it for real** with the same line and `plate=market` (about 75 s; one image, one
   scan, text calls). Expect the trace to end `live` and these lines:
   `plate market: market.png is behind the scene (N regions found, 3 of 3 characters named)`
   and `resident fruit_vendor: persona made; persona ask fruit_vendor "..." and persona
   converse work with them now`.
4. **Look at it.** Reload `http://localhost:7879/playground/`. `scene cast` lists the elf,
   the warrior and `fruit_vendor`; `persona show fruit_vendor` shows the generated persona
   (name, job, personality, speech style, backstory taken from the look the design gave).
5. **Talk to the vendor.**
   `$D dev --port 7879 persona converse turns=8 opener=fruit_vendor "the elf and the warrior are at the stall buying fruit for the road north"`
   or put your own question: `$D dev --port 7879 persona ask fruit_vendor "What is ripe today?"`.
   The speakers of `converse` are the cast members that have a persona, so the new vendor
   joins automatically.

How a resident becomes a character: the design (`POST /dev/move/design`) returns
`residents` (id, name, job, visible look) and places them like the party; the brief paints
them from that look; the plate is scanned and `identify` names the vendor's region from the
same look; then `persona generate <id>` runs with the look as its hint (it will not overwrite
an existing persona). A resident the scan cannot name gets no persona yet: the command says
so, and `persona generate <id>` works once a region is named.

## Names (players and NPCs)

Every character is shown by name, never by id. The name is the persona's `display_name`; the id
(`warrior`, `tavern_server`) stays the key for routes, anchors and layouts.

- Player characters: the warrior is Kael (the scene's hero card already uses that name) and the
  elf is Sylvi, in `assets/playground/personas/*.persona.json` and in the host's scratch copy.
- NPCs from a transition keep the name the design gave them (`resident tavern_server (Lena)`):
  the resident's persona is generated with that name in its hint.
- A transition passes each character's name to `identify`, so the same call that names a
  region in the SAM layout stores its shown name (`display_name` on the region).
- The page shows that name on the SAM tag (`10 Lena · 0.93`) and on the anchored speech bubble
  (`Lena`), both from the layout region, so they always agree. `scene cast` and
  `media locate show` print it next to the id.

Who knows whose name (owner rule, 2026-11-07: a name comes only from context, narrative or
backstory, never from thin air). Each character's state keeps `known_names`, the ids whose name
they know; a fresh state knows nobody. An id can carry a name (`mara_adventurer`), so a
character never sees the id of someone whose name it does not know: everywhere the id would be
(`present`, `asked_by`, the labels of `recent` lines, the briefs in `others`, its own standings
and attention) it sees a handle instead, `person-1`, `person-2` in the order the others are met,
with their trade, manner and speech and no name, backstory, voice samples or expertise
(`persona/handles.rs`). What the model answers with a handle (toward, focus, noticed,
`names_learned`) is read back to the id in code. Each line's model call reports `names_learned`:
whom the character now knows by name from what was said aloud ("I'm Kael"), from the narrative
(the situation or an events line says so, "Kael has come to see Lena every night") or from their
own backstory; code keeps only those present. A backfill is told the others the same way (it
writes into the persona, so a name given there would be kept for good). `persona converse`
labels recent lines by id, sends them as `recent` (never inside the situation text) and names no
place in the situation. The shown scene's description is never given to a character or a
backfill (only its title): it is the fixture's author note, and after a transition it still
said "Mara Vell at her fruit stall" (run 59: Haldor said "Mara" from it). Check and preset it:

```bash
$D dev --port 7879 persona state tavern_server            # "knows the names of: warrior"
$D dev --port 7879 persona state tavern_server knows warrior        # they have met
$D dev --port 7879 persona state tavern_server knows warrior false  # forget it
```

In an events file: `0 | ! persona state tavern_server knows warrior`. Write scene text with care:
an events line or topic that names someone gives that name to everyone who hears it (the first
strangers run leaked "Lena" through "sits down at Lena's counter").

Runtime API, to name or rename someone in a live scene (no restart):

```bash
$D dev --port 7879 persona set warrior display_name Kael            # the persona's name
$D dev --port 7879 media locate label bar.png warrior warrior Kael    # the SAM tag and bubble
$D dev --port 7879 scene image tv-backdrop bar.png                   # push the scene again
```

`media locate label <picture> <region> <id|-> [<shown name...>]` is
`POST /dev/media/locate/label` with `{"picture", "region", "label", "display_name"}`; a name
stays with its label and goes when the label changes. A plate named before this existed has no
shown names: set them once with the commands above. Reload the page to see them.

## Frames that follow the talk

The picture updates during a conversation when enough has visibly changed. After each line, a
judge (one model call, closed schema: `POST /dev/persona/frame`) reads what the picture shows
and the lines said since it was made. It answers how much would visibly change (0 to 100), each
change (who, a kind from the closed list, how it looks now) and the picture as it should look
now. Above the gate a new frame is made in the background while the talk goes on: by default an
edit of the scene's first picture, so faces and the room stay the same. One frame at a time;
lines said meanwhile wait for the next judgement.

```bash
# The judge alone: what would change, and does it pass the gate?
$D dev --port 7879 persona frame "line=Kael: Dance with me?" "line=Lena: Fine, hold my tray." \
  "A packed bar at night; Kael stands front left; Lena walks through the crowd with a tray."

# A conversation with frames (every option but frames= is optional)
$D dev --port 7879 persona converse turns=12 opener=warrior frames=bar-live1 gate=25 \
  "scene=<what the picture shows now>" edit=true edit_from=first hold=30 max_frames=6 \
  judge_model=<name> "<topic>"

# Frames for lines said elsewhere (the market runner's `@frames` lines use this)
printf 'warrior | Which blade suits narrow streets?\n' | \
  $D dev --port 7879 persona watch frames=<new-name> hold=30 "scene=<what the picture shows>"
# ...with a frame of the room every 30 s whatever is said, and a found clue shown up close
printf '@focus A wooden wheel bears monkish signs and shallow grooves.\n' | \
  $D dev --port 7879 persona watch frames=<new-name> every=30 "scene=<what the picture shows>"

# Any edit by hand
$D dev --port 7879 media generate "Lena now laughs" as=bar-edit base=bar.png wait=true
```

Settings, re-read on every call (no rebuild): `persona/frames.json`: `gate_percent`, `kinds`
(the closed list the judge may name), `scale` (what each percentage means; the model scores by
it), `edit_rule` (how an edited frame keeps the picture) and `shots` (the camera shots a frame
may take, each an id and how it looks: two-shot, close-up, extreme close-up, over-the-shoulder,
telephoto, wide, Dutch angle, low angle, high angle). The judge picks the shot that tells the
moment and is told the shot on screen, so it varies; the edit is told to frame it that way.
**The camera on an item.** The judge also names the item someone is looking at closely
(`inspected_item`, an id from the items offered at the active place in the world bible, checked
in code; null when nobody is) and whether that close look begins now (`new_look`, the judge's
call, told the item looked at before). A new inspection cuts at once to `inspect_shot` (`insert` in `frames.json`: the item fills the screen edge to edge),
whatever the gate and inside the hold, so the players at the table can see the item's quality.
The insert is code's to take for a new inspection only: the judge is not offered it among the
shots it picks from (run 58 took it for a gated frame, and the room later came back to a
close-up).
An insert is not scanned (nobody's face is in it) and is shown with the layout of the room it
cuts away from, so the cast stays (who may speak, where their bubbles sit); run 55 showed why:
with the insert's own empty scan every line said during it was refused `unknown_character`. The
room comes back after `insert_hold=<seconds>` (20 by default, a `persona converse`/`watch`
option) unless a new frame replaced the insert first, and the next room frame edits the room,
never the insert. The same holds for any frame: the watch keeps the newest picture whose own
scan names the whole cast, and a frame that leaves someone out (a close-up, an extreme close-up)
is shown with that cast, so the camera never takes a character out of the scene (run 61: an
extreme close-up named only the keeper and three lines were refused). An insert has a place of
its own beside the `parallel` room frames, waits for no room frame, and holds the screen; a room
frame that finishes meanwhile waits until the hold ends, and every picture stays up at least 8 s
(run 60 lost two close looks while both room slots were busy). The judge writes the close-up
from the item's bible tags and what was said about it, so a keeper who describes the guard, grip and runes puts them on
screen. Before an insert is made, the coverage judge (`POST /dev/persona/coverage`) is asked
whether the keeper of the item's stall (named in the bible) already knows the making of it and
how it looks up close (guard, grip, pommel, runes, finish). Only when not is the backfill engine
(`POST /dev/persona/backfill`) asked for it: what it writes goes into the keeper's public
backstory, the memory of his own work (expertise stays authored and is never backfilled), so
his later lines agree with it, and into the insert's picture. A second look at the same item
adds nothing ("already knew ... nothing backfilled" in the frames transcript). The frame judge
is given each keeper's backstory (`keepers` in its facts), so every close-up of an item shows it
as its maker described it. `@backfill sync` in a beats file has the keeper's line gaps filled
before he answers instead of after (`persona ask backfill=sync`). The judge is
asked after every line now (not only after the hold), since an inspection may cut in at any time;
the host logs `inspected=<id|-> inspection=true|false` on each `frame_judged` line.
**Plates on a schedule, clues on screen.** `persona watch` also starts a frame of the room as the
talk has left it every `every=<seconds>` (`frames.json` `show.every_seconds`, 30 by default; 0 for none; a quest's beats file sets its own with `@frames every=60`), on
slots counted from its start (the hold and slow frames never move them), on a timer beside its
input, so the plate moves while the talk is slow. It makes `parallel=` frames at once (8 by
default in the watch; one takes 40 to 200 s). Making and showing are apart: a finished room plate
goes up on the next showing slot, `every` after the last room plate went up (never while an
insert holds the screen), and only when no newer one is up or ready, so the newest picture of
the talk goes up and an older one is dropped (`plate <name>: made, not shown`; dungeon run 3:
plates shown in their start order came 8 s and 100 s apart, run 5: 30.0 s apart). A slot that
comes while all are busy is missed (`frame_schedule ... missed=N`). Without `max_frames=N` (the cap, if one is ever wanted) it makes
frames for as long as it runs. The toolbar's Plate timer counts down to the next slot. An input line `@focus <text>` (what a close look at a found clue shows) cuts to an
insert of it (the `insert` shot, no model asked) with `<text>` on the scene's caption bar for the
insert hold; the caption before it comes back with the room. Each frame's start and showing, and
each insert's end, is one `PERSONA frame_timing moment=started|shown|ended frame=.. reason=
line|schedule|inspection|focus at_ms=.. took_ms=..` line on the pages' `TIMING` clock.
`with=warrior,tavern_server` on `persona converse` chooses who talks (otherwise everyone in the
scene with a persona talks, the band and dancers included).

A dramatic scene for shot variety (the recipe of 2026-11-07, on `bar-period.png`):

```bash
$D dev --port 7879 scene image tv-backdrop bar-period.png
$D dev --port 7879 persona state warrior reset; $D dev --port 7879 persona state tavern_server reset
$D dev --port 7879 persona converse turns=14 opener=tavern_server with=warrior,tavern_server \
  events=development/market-threads/drama.events frames=<new-name> hold=20 max_frames=5 \
  "scene=<what bar-period.png shows>" "a busy night at the tavern"
```

An events file line may also be `<turn> | ! <dev command>`: the command runs before that turn's
line, to switch a stance on or off mid-scene. `market-threads/whiplash.events` uses it: Kael
flirts Lena to the top, turns cold for a few lines (a temporary `cold-contempt` stance and a
trigger, added and dropped by the file itself), then is ashamed. Preset Lena first:

```bash
for e in "attraction 70" "trust 75" "affection 40" "joy 60"; do \
  $D dev --port 7879 persona state tavern_server set $e; done
$D dev --port 7879 persona state tavern_server rapport warrior 50
$D dev --port 7879 persona converse turns=12 opener=tavern_server with=warrior,tavern_server \
  events=development/market-threads/whiplash.events frames=<new-name> hold=15 parallel=2 \
  max_frames=5 "scene=<what bar-period.png shows>" "a late shift at the tavern"
```

What happens goes in an events file, one `<turn> | <what happens>` a line (`#` comments). On
that turn and the next, the speaker is told it is happening right now and reacts to it; the frame
judge gets it as a seen line `(Happening now: ...)`, because dialogue alone never shows a spilled
tray. Do not script events in the topic: a topic is what is on the table for every line, and one
that listed events ("a drunk shoves Kael, then they make up", even "this happens now, in order")
was played as already over ("You kept your sword sheathed. Good."). `hold=20` lets frames come
closer together than the 30 s default, `parallel=2` lets two frames be made at once (shown in
order, one spinner) so fast scenes lose fewer beats, and `max_frames` caps the spend (each frame
is one image edit and one scan). Command options override them for
one run. Frames are stored as `<frames>-1`, `<frames>-2`...; a name already in the library is
kept, not made again, so give each run a new `frames=` name. `scene=` matters: it is what the
judge compares the lines against. Without it the judge starts from the cast list alone. The
transcript says when each frame was judged (and its changes), started and shown, and how long it
took; the host logs `PERSONA frame_judged changed_percent=.. gate=.. triggered=.. duration_ms=..`.

## The story behind a scene (`persona story`)

The narrative engine's authoring step for the playground (`planning/narrative-engine.md`:
an arc with phases and beats, the facts it rests on and a hook for each character). The full
`df-narrative` director runs over session checkpoints and is not wired into this host; this is
the storyteller's plot, handed to the persona engine through the guidance it already takes.

```bash
# Plot it: one storyteller call, closed schema, every id checked (the cast are persona ids)
$D dev --port 7879 persona story plot cast=warrior,mara_adventurer,weapon_seller "<premise>"
$D dev --port 7879 persona story show      # the plot, the truth included (operator view)
# Clear a run's play (what was found, the narrator's lines, where the danger struck, where play
# is, the stuck-party count) and keep the plot, its ways and its reference sheets
$D dev --port 7879 persona story reset
# Plot the ways between the places of a stored story (a plot made before ways has none)
$D dev --port 7879 persona story ways
# Hand it out: each character of the plot in the shown scene gets its own guidance only
$D dev --port 7879 persona story apply
# Plot (or plot again) one scene's moves for the stored story, then print them as runner beats
$D dev --port 7879 persona story moves judgement
$D dev --port 7879 persona story beats judgement > artifacts/tmp/<id>/judgement-moves.beats
```

Each beat names who leads it and its moves: who acts (`speaker`), whom it is put to (`to`, an
id or `room`, never the speaker; code refuses a move put to oneself) and the goal, an order to
the speaker naming the listener by look and pronouns (from each persona's public side), never
by name, id or hidden part, never their words and never anyone's answer; the verdict is played,
not plotted. Code also refuses a judgement where the accused has fewer than three moves of his
own. The moves question sees every character under a `person-N` handle, read back to ids in
code: with the real ids it echoed them as descriptions ("as a question to the bought guard").
It is asked with the scene's old moves cleared, or it copies them back word for word. Only the
people in the shown picture are present (`scene cast`): show the scene's picture before plotting
its moves. A refused answer is asked once more with why (`previous_refusal`). Each beat also
names its exhibits, the evidence on show, with what anyone sees of it up close
(`up_close`, so the false dagger shows its loose handle and painted marks to whoever examines
it); `story beats` prints them as `@exhibit` lines and the runner adds them to every situation.
Names never reach them: the moves question is asked with every name and old exhibit text
blanked, and a names judge (a model call against a closed schema, `PERSONA story_names
scene=.. texts=.. named=..`) refuses an answer whose goals or exhibits call someone by name,
which is then asked once more with why (`PERSONA story_moves_retried`). The runner lets a
beat by the one a line was put to stand as their answer (no answering twice), and a judgement
starts with every participant reset (`persona state <id> reset`, then `knows`): run 70's Kael
"remembered" run 69's acquittal mid-hearing. Every character in the shown picture makes at
least one move of their own (code refuses a plot that leaves one silent: the first arrest plot
had five moves and no questions for Kael). `persona story beats <scene>` prints `@keeper <who
leads>` and one `speaker | goal` a move; put the scene's `@keeper_situation`,
`@speaker_situation`, `@frames` and `!` presets above them. The market runner has whoever a line
was put to answer it (`persona addressed`, told the speaker's goal), the keeper when it was put
to the room, and an answer that itself puts a question to someone gets one reply from them.

**What works: a scene from the plot to the screen** (runs 71 and 72: acquittals earned on the
flaws found in play; photographic frames and a camera insert on the dagger):

1. Plate. Make each scene's plate fresh, never by editing an illustrated one: `media generate
   "<the scene, every person by look> Medium-wide shot at eye level." as=<plate>
   purpose=plate aspect=16:9 wait=true` (the house look is added). Describe each cast member
   with the same words on every plate (one shell variable per person, used in each prompt).
2. Names. `media locate <plate>.png masks=32`, then `media locate label <plate>.png <region>
   <id> <Display Name>` for each character, and `persona set <id> job <look>` where a persona's
   look differs from the picture. Show it: `scene image tv-backdrop <plate>.png`.
3. Plot. `persona story moves <scene>` with that plate shown (who is there is who it holds), then
   `persona story beats <scene>`.
4. Beats file. The scene's header (situations, `@frames frames=<new name>` and a `scene=` that
   describes the plate, presets), the exhibits, the plotted moves, and for the judgement a last
   beat asking for the verdict. The first scene of a story resets everyone and sets who knows
   whom; a scene replayed alone resets everyone it holds; a later scene of a sequence resets only
   newcomers, so what was learned carries. To play a story, show each scene's plate (`scene
   image tv-backdrop`) and run that scene's beats with a copy of the runner, one after another.
5. Run a copy of the runner (a running script must not be edited) and grade the transcript.

**Flowing play: the story is prepared, never scripted** (owner, 2026-11-08: "the narrative
engine should just flow and not predetermine scenes"; quest "The Lantern Bait", runs 77 to 86).

- **Prepared ahead, by `persona story plot cast=<ids> scenes=<ids> <premise>`:** the hook, the
  truth and who is behind it, the places (each with its dressing, things a camera can show, and
  its danger, for the storyteller only), the beats (what each place offers and turns on, no
  moves), clues (`evidence`, with what a close look shows) and every character's guidance and
  secrets (text, sensitivity, reveal conditions, who else knows). Existing personas may be cast
  under their own ids. A new id names a public place, never a hidden part (code refuses
  `harbour_framer`; an id is shown to whoever learns that name). `persona story apply`, with each
  place's plate shown in turn, writes the new personas and puts each character's secrets into
  their persona.
- **Played live, by `persona story next [opening=true] [after=<id>] [arrived=<n>]
  [recent=<line>]... <place>`:** the storyteller as game master gives one move at a time from
  what has been said: who acts, whom it is put to, a goal, and whether the end is reached.
  Code decides who may move (`turn_speakers`):
  - someone already there opens a place;
  - after someone who is no player, a player moves;
  - nobody moves twice running;
  - play goes in rounds, the players who have spoken least first.

  Judges, each a model call against a closed schema, refuse a goal; it is asked again with
  every reason, until a move stands:
  - the names judge, for a goal that names someone;
  - a goal carrying a storyteller handle (`person-N`) has the handle put as that person's look
    (dungeon run 6: refusing it cost a dozen asks a place); only a handle with no look to put
    is refused, which is the handle of someone who is not here;
  - the knowledge judge, for a goal that holds what its speaker could not know from their
    guidance, their secrets, what the party found (`party_found`), what was said, what is in
    sight and who is there. A player knows everything the party saw or worked out, at any
    place (`party_found` in `story_next.rs`, from the story's `findings`; someone who is no
    player is given none): before, a goal using an earlier place's clue was refused as unknown
    (vault run, eight refusals). The same judge refuses a goal that sends the speaker to
    question themself, that repeats what was already asked or done, or that keeps the players
    at a place it has said they leave.
  - **The storyteller sees only people present** (`tell_only_who_is_here`): someone of the story
    who is elsewhere keeps their part for the plot, shown as "not here" with no handle, is left
    out of the cast, and the lines they said are labelled "not here" too. A goal calling an
    absent culprit by handle was refused 4 times in 5 moves of one run. The look lookup
    for a handle compared ids against handles; it now compares the handle with the cast's
    handles (`handles.shown(id)`).
- **Moving on (runs 87 to 94):** left to choose, the game master never sent the players on,
  even long after a place's clue was out. Now:
  - before each move the pace judge says whether the place has given what it offers, from its
    beat, its danger, what its people keep and the lines said there (`arrived=` tells where
    those begin);
  - code first waits until every player there has spoken at the place;
  - once the pace judge says yes, the move is the set-off, and code takes the players to the
    story's next place in its order;
  - at the last place that is the story's end, which comes only once the truth behind the hook
    is out (talk going round never ends it).
- **Guidance holds only what a character already knows:** run 94's players were told they
  "do not know who set the trap", and so knew of a trap and asked after a reward before anyone
  offered it. The plot is told that a guidance never mentions a person, offer, place, plan or
  danger its character has not met yet, not even to deny it; check each player's must_avoid
  after plotting, as a plot can still break it. Before re-plotting with personas that played an
  earlier story, check their private backstories: `story apply` appends each story to them, and
  the next plot builds on what it finds there.
- **Replay from clean personas (run 101):** every run saves what it learned into the personas:
  each backfill appends to a backstory (recorded in `<id>.backfills.json`), and `story apply`
  leaves the earlier story in place. A test replay of the same story then inherits the last run,
  half-told secrets included, and a cast reused from an earlier story carries it (Nessa kept
  telling of Kael's arrest). Before testing, take the earlier story and the backfills out,
  check the cast, save the clean personas, and restore them with `persona write <id> <file>`
  before every run. In real play the saving is right: the world remembers.
- **Pace (runs 108 to 111):** a middle place is left once every player has spoken about three
  lines there, or sooner when the pace judge reads it as done. At the last place, once every
  player has had those turns, the next move is the danger striking (someone who is no player
  acts, `strike_now`). Only after that is the end judged, so a story never ends before its trap
  springs.
- **A close look is said aloud (run 101):** the game master tells a player who looks closely to
  say what they see. A line that only says "I'll check" finds nothing, and the last place never
  ends.
- **Answers to follow-ups:** after a move's answer, its speaker may follow up once, and that
  follow-up is answered too, so the next player never asks it again.
- **Clues are planned by the plot (run 96):** each beat names its exhibits, the evidence on show
  at its place, with what a close look shows. The runner adds them to everyone's situation there,
  and the game master steers players toward them with hints. Without them a close look found
  nothing. A plot made before this rule gets its exhibits from `persona story moves <scene>`,
  with the place's plate shown.
- **Clues found in play, and research (2026-11-08):** a piece of evidence may name the pieces it
  `needs`: its flaw can be worked out only with what they show (a seal read against a record
  found two chambers earlier). Code refuses needs that name no piece or go round in a circle.
  After every line the runner asks `persona story clues <place> <id>: <line>`: the clue judge
  (a model call, closed schema) says which pieces the line looked at closely, saying what a close
  look shows, and which it worked out. Code keeps a piece seen only where it is on show, and
  worked out only from a player's line once everything it needs was seen. The host owns what was
  found (REF-070): each new piece is committed to the stored story (`findings`: `seen` and
  `worked_out`, each `{piece, place, line}`, oldest first, once each) under the story's write
  lock, and `story next`, `story narrate` and the clue board read it there; no route takes
  `seen=`/`worked_out=` any more, and the runner holds no clue state of its own: the route
  takes the line and `place`, the story file holds `findings` (written by the clue route under
  `story_write`), and the runner keeps only the `workable=` text of the last reply. The reply
  lists `seen=`, `worked_out=` and `workable=` lines, and `stalled=true` when the party is stuck
  (`PERSONA story_clues place=.. seen=.. worked_out=.. workable=.. stalled=..`). The runner prints `? seen <id>: ..` and `? worked out <id>: ..`,
  shows each piece seen up close on screen (`@focus <what it shows>` to the frame watch), adds
  what careful study can now find to the situation of whoever makes the next move, and prints
  what was found at the story's end (`persona story show` lists it). In `story next` the game
  master has the players examine, compare and work things out wherever
  things are on show (the players do the working out; the narrator never does, see "The dungeon
  master"), and the pace judge counts a piece not yet seen or worked out as still
  coming.
- **Music per place:** the plot gives each place its `music` (mood, pace, instruments, no
  words), printed by `story beats` as `@music`. The runner plays it on arriving there with
  `scene music generate as=music-<plate> <score>`: a 120 s looping track composed once
  (ElevenLabs music) and reused from the library on every later visit or replay.
- **Reference sheets (2026-11-08):** whatever is looked at gets one sheet in the story
  (`items`): its name and look (material, size, colours, marks, wear, what a close look shows),
  designed once by the storyteller (`persona story item <place> <what is looked at>`; a sheet
  judge, closed schema, first says which known thing it is). A sheet for a piece of evidence
  agrees with it; any other is a world detail that proves nothing. The runner designs the
  sheets of a place's exhibits on arriving (in the background), every room plate is told how
  each looks (`persona watch place=<place>`), and a clue's close-up is drawn from its sheet and
  keeps its picture (`persona story item_picture`), so the next close-up of it is the same.
  Audit lines: `PERSONA story_item place=.. item=.. kind=known|evidence|new` and
  `PERSONA story_item_picture item=..`.
- **Plates for a research quest (dungeon runs 2 to 5):** `@frames every=60` (a new plate every
  60 s; owner, 2026-11-09, relaxed from 30 s because Codex hit `usage_limited` after about 3 h of
  30 s plates), `@frames edit=false` (each painted anew in the house look: edits of a plate drifted from
  photographic to illustration) and no `parallel=` (eight at once: one takes 40 to 200 s). Each
  run's frames get the runner's own stamp (`<name>-<MMDDHHMM>-<place>-N`): a name already in the
  library is shown again, not made. The watch starts room frames ahead of their slots so one is
  ready at each (as many in the making as one takes slots to make, plus two: `a room frame
  starts ahead of its slot`), shows no spinner while a schedule runs, holds each clue's close-up
  its whole hold, and when play leaves a place drops the frames still being made there (`frame
  N dropped: the talk moved on`), so the next place starts at once.
- **Research is the players' (dungeon runs 1 to 6):** only the one who makes a move is told what
  can now be worked out, only a player's line works a piece out (anyone may say what a close
  look shows), a close look tells of the one thing its move names, and the game master is told
  what nobody has looked at yet (`unseen_here`) so a player's move looks at it. A refused move's
  reason is logged (`PERSONA story_next_retried place=.. why=".."`, the engine's own words); a
  goal that calls someone by a handle has the handle put as their look.
- **Who hears what:** whoever answers hears only the lines said at their place. Whoever makes a
  move remembers all of them (runs 94 and 96: people at the inn spoke of market talk).
- **The camera in talk:** faces change more than bodies in conversation, so a talk-heavy quest
  lowers its frame gate (`@frames gate=20`): a few changed faces cut to a new shot.
- **Secrets:** every line that lets a secret out is weighed by the reveal judge against that
  secret's conditions, on what the speaker was shown. Out too early, the line is worded again;
  a second early wording is refused. Run 77's opening lines let out two secrets unasked; since
  then each reveal has come only when a condition held (run 85: the fruit seller told what she
  saw once she was asked gently and promised no blame, exactly her conditions).
- **Running it:** a beats file with `@flow <first place>` and, for each place, `@place <id>
  <plate>`, `@place_keeper`, `@place_keeper_situation`, `@place_situation` and `@place_scene`
  lines, plus the presets (`development/market-threads/lantern-bait.beats`). The runner prints
  each move (`> <speaker> to <listener>: <goal>`) before its line, so a line can be checked
  against what it was asked; the one a move is put to answers it. After every move it asks
  `persona story phase` what comes next and shows it: `=== on the way: <stage>` for a stage of
  the way, `=== <place>` on arriving (see "The way between places").
- **Players' styles:** each player has a default stance whose speech style is their kind of
  question (`persona behavior <id> add <stance> default=true style=<how they ask> ...`), and a
  goal leaves the manner to the speaker unless the approach is what will move someone.
- **The runner holds no story:** every scene, keeper, situation and rule is in its beats file,
  and a scripted file must name its `@keeper` and situations.

The plot holds the public charge, the truth (kept in `persona/story.json` for tooling, never in
a character's prompt), the culprit and motive, the evidence with what gives each piece away, the
beats of each scene (`arrest`, `march`, `judgement`) and, for every character, a
`NarrativeGuidance` (phase, beat, hooks, threads, `must_establish`, `must_avoid`, tone, note) as
it stands before the first scene: nobody remembers their own future (the first apply wrote the
whole plot into Kael's private backstory as if it had happened; the instruction now forbids it).
New characters get new ids that carry no name and no hidden part, and are no stored persona's
id: an id is shown to whoever learns that person's name, so the first plot's
`market_guard_bought` told everyone who knew Tavin that he was bought (renamed `town_guard`,
and the lawful sergeant `town_sergeant`).
`apply` generates a new character's persona from its guidance once it is in the shown scene (in
the picture's scan, named: `media locate identify`), its name established by the guidance; a
character with a persona, a new one's kept from before included, gets it through a guided
backfill of its private backstory (`give_guidance`), unless the coverage judge says it already
holds it, so a second apply adds nothing. A character not in the
scene yet is left for a later apply. Generation does not see the picture: check that a persona
fits it (the first sergeant was written "he" for the woman on the plate).

**The arrest story** (2026-11-08, story "The Nightglass Charge"), three scenes on photographic
plates made fresh with one cast description (`stall-photo-fresh.png`, `march-photo-fresh.png`,
`hall-photo-fresh.png`; the first illustrated ones, `arrest-*-real.png`, are retired), scanned and
named, then `persona story apply`: `market-threads/arrest.beats` (the guards and the accuser
question Kael; he answers), `market-threads/arrest-march.beats` (dragged up the street; Kael and
his friends press the guards, a fruit seller calls out) and `market-threads/arrest-judgement.beats`
(Kael on his knees before the mayor; evidence, witnesses and the defence). Their moves come from
the plot (`persona story moves`); the hand-written moves in those files are the fallback.

## Play the quest end to end (flowing, voiced, in the browser)

The quest "The Lantern Bait" (`market-threads/lantern-bait.beats`), played from a stored plot on
clean personas, with every line voiced. A run takes about an hour (around 60 lines, each with
its judges and its voice).

1. **The host** as in "Launch the host", with both keys exported, and the market plate shown:
   `$D dev --port 7879 scene image tv-backdrop quest-market.png`.
2. **The story and the cast as they were,** in this order:
   - Restore the story file: copy the saved plot back to
     `artifacts/tmp/persona-live/playground/persona/story.json`, or keep the stored plot and run
     `persona story reset`, which clears the play state (what was found, the narrator's lines,
     where the danger struck, where play is, the stuck-party count) and keeps the plot, its ways
     and its reference sheets. A plot made before ways has none: `persona story ways` plots them.
   - Restore the clean personas: `persona write <id> <file>` for every cast persona, before every
     run. A run saves its backfills into the personas, and the next run would inherit them.
   - Run `persona story apply`, with each place's plate shown in turn, if the clean copies were
     saved before it. It hands every character of the plot their part, and it now also fills an
     existing persona of a story character (`give_guidance` in `persona/story.rs`: a guided
     backfill of its private backstory, skipped when the coverage judge says it already holds it).
     Before that, a persona kept plain got nothing: dungeon runs 7 to 23 had a rival with no
     purpose, so the danger never struck. A character not in the shown picture is left for a later
     apply.
   - The same steps play `drowned-archive.beats` (the quest of dungeon runs 1 to 23, scenes
     scriptorium, reliquary, vault), which sets `@frames every=60` (see "Plates on the schedule").
3. **Voices.** Open `http://localhost:7879/playground/` in the browser pane and click TTS in the
   toolbar. The click is what unlocks the page's audio. Check with `$D dev --port 7879
   playground`: it should show `tts: true` and `audio unlocked`. A host restart turns TTS off
   again. The runner asks for every line with `speak=true`, and the toolbar decides.
4. **Run a copy of the runner,** never the script itself while it runs, with its `cd` line
   pointing at the checkout:
   ```bash
   cp development/market-thread.sh artifacts/tmp/quest/runner.sh
   sed -i 's#^cd .*#cd /c/Users/<you>/Desktop/dungeonfluxGame#' artifacts/tmp/quest/runner.sh
   FRAMES_LOG=artifacts/tmp/quest/frames.log bash artifacts/tmp/quest/runner.sh \
     development/market-threads/lantern-bait.beats > artifacts/tmp/quest/play.log 2>&1
   ```
   Every dev call in the runner waits up to `HUNG=1800` s (30 minutes), which replaced the
   120 to 280 s client timeouts: a slow judge is waited for, and only a call that is really hung
   ends.
5. **Follow it** in `play.log`: `=== <place>` on each move to a place, `> <speaker> to
   <listener>: <goal>` for each move (`(place served)` when it leaves the place), `[NN] id:
   line` for each line, and `revealed secrets:` after a line that lets one out. It ends with `===
   the story ends`. Under each move a `! refused: <why>` line is printed for every storyteller
   answer that was refused before a move stood (`NextMove.refused`, the judges' reasons), so a
   move that took long shows why.

What a good run shows (run 111):
- **Market:** the fruit seller tells what she saw only when asked gently and promised no
  blame, and the guard gives his report only when a traveller asks.
- **Inn:** the stranger makes his offer to the room, and the players question him and read
  the clay on his boots and the black wax.
- **Graveyard:**
  - the players read each staged sign as it is;
  - once each has had their turns, someone who is no player acts and the trap springs;
  - then the story ends.

To restart mid-run: stop the runner, save `play.log`, stop the host by its port, rebuild if
code changed, start the host, and restore the plot and the cast before the next run.

## The market thread (re-runnable persona and backfill exercise)

`bash development/market-thread.sh development/market-threads/haggle.beats` (the runner takes
no default: the beats file holds the scene, its keeper and its rules) runs 20 messages at the
fruit stall: her name, where the apples come from, the price, a discount on one apple (should be refused), a discount on five
(should be granted). The elf and the warrior get one goal per beat and word their own lines
(`persona say`); Mara is fully live (`persona ask`: coverage, then backfill of what her persona
lacks, then the answer). Its beats file first adds her two discount rules to her persona (`!`
lines: a firm price on one piece, a bulk discount at five; triggers are statements the model
judges). Needs
the market scene up (the walk-through above). Output: `[NN] speaker: line`, then coverage and
backfill result lines for Mara; `persona backfills fruit_vendor` lists what was added.

Any scene can use the same runner: a beats file sets who answers and where with `@keeper <id>`,
`@keeper_situation <text>` and `@speaker_situation <text>`. After every line the addressee judge
(`persona addressed`) says whom it is put to and whether an answer is owed (`answer_owed=`: a fact,
a decision, an action, or the answer to a question or demand); when only an acknowledgement or an
echo would answer it, nobody answers and the runner prints `- no answer owed` (dungeon runs: a
torchbearer said "Yes, sir" to nearly every player line). Example, the bar at night:
`bash development/market-thread.sh development/market-threads/flirt.beats` (Kael flirts with
Lena the server for 20 rounds). `! <dev command>` lines run a dev command between beats
(switch a stance on and off mid-scene, as `romance-whiplash.beats` does).
`@state <emotion> <level>` lines start the keeper mid-story
(the jealousy and farewell scenes start with attraction and affection already there). Never
edit the runner or a beats file while a run uses it: bash reads a script as it goes, so an edit
garbles the run. Run a copy for long batches (the copy's `cd "$(dirname "$0")/.."` must be
changed to the repository root, or it cannot find its beats file). Reset both first: `persona state <id> reset`. Each run's full
record goes in `development/persona-transcripts/` (`py -3 development/persona-transcript.py
<log> <out.md> <title> <header> [section=<beats>] warrior=Kael tavern_server=Lena`), its summary
in `development/persona-refinement-log.md`. The romance edge cases are
`market-threads/romance-{pushy,shy,jealous,farewell,rejected,dancer}.beats` (`@rapport <toward>
<value>` presets a standing); a speaker whose own persona refuses
a beat (Kael will not be pushy) needs a temporary stance in the scratch copy for that test
(`persona behavior warrior add drunk-pushy ...` plus a trigger), dropped afterwards.

**The blade gauntlet** (`market-threads/blade-gauntlet.beats`, on `weapons-stall-blades.png`
with `world place market`; start it with `playground go weapons_stall`, a scratch fixture copied
from `market_stall.json` with the stall's own name, a nameless description and that backdrop,
because in run 59 the scene's words still said "Mara Vell at her fruit stall" and Haldor called
Mara by a name nobody had said; no character or backfill is given a fixture's words now, but the
picture and its scan come from the fixture): Kael and four
adventurers test Haldor Brask, the bladesmith, with
expert, ignorant, suspicious, haggling and ready-to-buy questions. The beats file resets all six,
presets the band's names (`! persona state <id> knows <other>`: they travel together; Haldor
knows nobody) and has the picture follow the talk with `@frames <option>` lines (one
`persona watch` option each: `frames=<new name>`, `hold=30`, `max_frames=8`, `parallel=2`,
`scene=<what the picture shows, who stands where>`). The runner hands every line said to
`persona watch`, which judges and frames it as `persona converse frames=` does; its transcript
(each judgement, frame started and shown, with seconds) is printed after the thread and kept in
`$FRAMES_LOG` (default `artifacts/tmp/market/frames.log`). Give each run a new `frames=` name.
Check, per Haldor line: the craft answer is right and his own (he forged them), the ignorant one
is corrected kindly, proof is shown not claimed, every price is the bible's in gold, a concession
is small at first and never below the floor, a bundle's spoken total equals its quoted lots, and
he asks for the coin.

Prices live in the world bible (`world/bible.json`): `items` carry `value_cp`, a `stalls` entry
names its keeper, what it sells, `bulk` tiers and `haggle_floor_permille`, and `coins` (gp 100,
sp 10, cp 1; `name` is what follows a count: "gold") say an amount in coins. A line never writes
an amount: it writes `{price:N}` (lot N of its quotes) and `{order_total}` (its lots together),
and code puts in the total held (clamped to the stall's bounds), in the largest coins that fit
("608 gold and 5 silver"; `N cp` without coins). The model added and converted badly (it said
"722 gold and 5 silver" for 361 + 247, and "36 gold and 100 silver" for 361), so the number said
is now always the one charged. A line that quotes without slots, names a lot it did not quote,
or writes braces that are no slot is worded again once; a total the stall would not charge is
worded again too.

The Codex text backend hands its prompt over standard input (`codex exec -`): a keeper with four
price tables made a prompt longer than Windows' 32,767-character command line, and every line
was refused as `configuration`.

The bar scene came from one line at the market (one plate, about 90 s):
`$D dev --port 7879 move act warrior plate=bar "Evening has fallen. You leave the elf at the
market and head off alone to the liveliest bar in town, a packed place with a live band and
women in skimpy, revealing evening dresses dancing and flirting with the crowd. Tell the table
where you are going, in your own words."` Jev read a walk for the warrior alone, the design
invented a server, a fiddler and two dancers, and each got a persona.

First run (2026-11-07): name consistent (messages 2 and 20); the discount on one refused,
on five granted; two backfills added facts (orchard origin, shelf life); one backfill refused
a malformed model answer; two price backfills were blocked by the private-detail guard, so
Mara never states an amount. That last point is why prices belong in a world bible, not in a
persona (planned).

## Verify (browser pane)

The browser pane repaints only on navigation or resize, so read the page, then reload:
`document.querySelector('.kit-turning')` exists while a plate generates (the turning icon of
the kit's scene-title banner; it was `.rp-title-spin` on the old chip), the backdrop `src` becomes `/playground/media/<name>.png`, and `scene cast`
lists the characters found in it.

## The look (one place)

`BriefStyle::default()` in `crates/df-table-host/src/movement/brief.rs`: `direction` (the
look) and `writing_ban`. Every plate, including `persona converse plate=`, uses it, and every
frame edit restates it. Today (2026-11-07): a still from a glossy TV period soap opera set in
a medieval Dungeons and Dragons world, photorealistic, 35mm, an intimate medium shot on the main
characters' faces and reactions, soft flattering candlelit key light, creamy shallow depth of
field, Hollywood teal and orange grade, real actors in lived-in medieval linen, wool and leather
in a real set of timber, stone and candlelight; nothing modern and no high-fantasy art. History:
the first bar plate drifted into an epic fantasy-film tavern (wide crowd shot); "not a fantasy
film" then drifted into a modern bar (drum kit, glass mugs). Name the world and the period, and
ban what is modern separately from what is high-fantasy. 2026-11-08 (the arrest story): plates
written by hand as "a medieval fantasy world" drifted into fantasy finery (gold-embroidered
velvet, heraldic banners, cathedral light shafts, glowing runes), and every frame edited from
them kept it. The direction now asks for costumes and light like a grounded historical drama and
bans glowing runes, gilded finery, god-rays and cathedral grandeur. A plate made by hand must
use the same words, and an edit that only says "change the look" keeps everything: say what to
tone down (plainer wool and leather, timber and plaster, overcast daylight, no banners). The
arrest plates in this look are `arrest-stall-real.png`, `arrest-march-real.png` and
`arrest-hall-real.png` (edits of the first ones, faces kept). The bar plate in this look is
`bar-period.png` (made with `media generate`, then `media locate`, `media locate identify` and
`media locate label` for the names).

2026-11-09: the direction was shortened (the Forge studio takes 4000 characters in all) and
reworded again, because "soft, low-contrast" drifted waxy and painterly. It now asks for an
unretouched photograph of a live-action medieval TV drama set: a candid mid-scene moment, people
turned to each other or their task (nobody looking at the camera or posing in a row); real skin
with pores, fine hairs and blemishes; natural lens sharpness and contrast, no HDR, a neutral
grade. Its bans: painting, illustration, airbrushed, waxy, plastic or glowing skin, and an
amber, golden or teal-orange grade. Only water and what it wets look wet; metal, stone, skin,
cloth and wood are matte.

The look is a photograph (owner, 2026-11-08: "not photographic, it's like an illustration"). The
house direction (`BriefStyle`) is written as an unretouched photograph from the set of a TV
drama: a named camera and lens, natural light, real skin and worn cloth, a neutral grade, and a
ban on painted, illustrated, waxy or golden looks. The old one asked for "candlelit key light
with a warm glow on skin, creamy depth of field, teal and orange", and frames came out golden,
waxy and painted. `media generate` now sends its prompt inside that direction (`look=house`,
the default); `look=none` sends it as written. An edit keeps the finish of its base: an
illustrated plate edited "into a photograph" stays an illustration, so make a fresh plate
(`hall-photo-fresh.png`, scanned with `media locate <picture> masks=32` and named with `media
locate label`) and fit each persona's look to it (`persona set <id> job <look>`).

## Character reference sheets

Plates painted from text drift: a face or a costume changes between plates. Each character of
the drowned-archive quest has a photographic reference sheet in the media library,
`ref-<persona id>.png` (`ref-warrior.png`, `ref-mara_adventurer.png`, `ref-jory_adventurer.png`,
`ref-sella_adventurer.png`, `ref-village_torchbearer.png`, `ref-rival_scholar.png`): the same
actor as a front close-up, a three-quarter view and a full-length view on mid-grey. Each was made
(2026-11-09) as an edit of `look-<persona id>.png`, the character's crops from
`archive-scriptorium-2.png`, `archive-reliquary.png` and `archive-vault.png` (the boxes of their
`media locate` regions, padded 1.5%, scaled to 900 px high and set side by side with ffmpeg, then
`media add`), with `purpose=portrait aspect=16:9 look=none` and a prompt that names the
character's look from the beats and asks for the sheet.

`media generate ... refs=ref-warrior.png,ref-sella_adventurer.png` attaches sheets as reference
pictures (after `base=`, at most 8 pictures in all; the Codex image tool takes at most five, and
at six the job ends `InputMismatch`: every vault plate failed so in run 18); the host adds a line telling the model to
keep each person as the sheet shows them. Both host backends take them: the Codex CLI as
`localImage` inputs of the turn, Forge as the render's references (it takes up to 10). A sheet
that is not in the library ends the job as `reference_unreadable`; a host built before `refs=`
refuses a body that has it.

## Keep the scene clean

`scene reset` clears live edits; the page chrome comes from the fixture. For a bare scene
set in `ui_roleplay_thought.json`: `roleplay.title: null`, `roleplay.caption: null`,
`pills: { "logo": "inline" }`. After `scene reset`, re-apply a backdrop with
`scene image tv-backdrop <name>.png`.

## The UI kit on the live TV

The kit (`crates/df-playground/src/kit/**`; looks in `assets/playground/kit.css` and `theme.css`)
replaced the old views for: the off-picture speaker card, the close-up title card, the italic
caption bar, the clue board, the status chip, the speech bubbles and thinking cue, the
scene-title banner, the brand mark and the Dungeon Master narration box. Their mounts are
`view/{clues,roleplay,overlays,brand,tv}.rs`. The narration box is the kit's (`narration_box` in
`view/tv.rs`; the centre box has the tab header). The kit's speech bubbles are live too: the page's
layout places the frame (`view::bubble_layout`) and the kit draws the bubble, and an anchored
bubble is first drawn 31 % of the screen wide (`ANCHORED_WIDTH_PERCENT` in `image_layout.rs`,
31cqw), then measured, so a short line hugs its words. The party strip (`view/seats.rs`) is still
the old view, and the kit's choices and quest journal (`kit/draw/{choices,journal}.rs`) are
drawn only in the gallery, not mounted on the live screen. There is one logo surface: the kit
brand at the top of `.tv-heading` (`view/brand.rs`), and the old top-bar logo was removed from
`view/pills.rs`; a scene's `brand` or `pills.logo` asks for the one mark. A development
build shows the kit's sample boards as the whole stage at `?kit=N` (`kit/gallery.rs`, boards in
`kit/gallery.json`), over the `ui_kit_gallery` scene.

The top-left column `.tv-heading` stacks, top to bottom, the brand mark, the place, the scene
banner and the clue board. The thinking cue is a capsule with a dot trail to the speaker's head.

Occlusion rules (owner, 2026-11-09):
- The clue board stays at its home under the logo and never moves.
- Open or folded, it fades out (opacity over 400 ms, class `clue-hidden`) while it would cover
  a face in the shown plate's layout, a speech bubble, the narration box or the caption bar
  (`board_covers` in `clues.rs`, `settle_clue_board` in `view/bubble_layout.rs`), and fades back
  when clear.
- The bubbles are placed again each time the board opens and folds; their keep-out is
  `.tv-clues .kit-clues`.
- The lower-third boxes (narration, caption bar, speaker card, close-up title card) lie over the
  outline and bubble layer (`z-index: 2`, `theme.css`).
- On a close-up with an off-picture speaker, the title card sits at the left and the speaker card
  at the right of the lower third, so nothing covers the item (the card covered the ledger's
  binding fold, the clue on show).
- The close-up title capitalizes its first letter (`.kit-tc-title::first-letter`): a sheet may
  keep an item name in lower case.

## Clue board

The shared screen can show every clue found so far as a slim column of cards down the left (under
the logo and the place name, in `.tv-heading`; the right is the off-screen speaker's card). It is the scene change
`SceneChange::Clues { notes }` (oldest first, the newest last; `ClearClues {}` takes it away), so it
shows on every client and `scene reset` drops it. The newest note sits on top with its picture,
name and what was found, and glows warmly for 2 s; older ones follow, smaller once there are more
than 5. A seen clue has an open eye; a worked-out one a check and a gold edge. Try it:

```
scene clues set "ring|Gold ring|It bears a crest.|seen|ring.png" "note|Torn note|Half a date.|worked"
scene clues clear
clients timeline <client> kind=clues
```

In play the clue route sets it (`persona story clues`, after each line that finds something):
every piece seen so far, by its reference sheet's name and picture, what a close look showed or,
once worked out, what it means, the piece the line found last. A sheet that keeps its close-up
puts that picture on the board. The board opens for 10 s on a change, then folds into a slim
`Clues N` strip of thumbnails under the logo, so the plate stays clean; the strip shows a
thumbnail only for a clue whose close-up picture exists (`.kit-note:not(:has(.kit-note-picture))`
is hidden in `theme.css`). Open, it shows the
newest three notes; both forms fade out while they would cover a face, a bubble, the narration or
the caption bar (see the kit section above).

## The dungeon master (narrator)

The narrator guides the party through the story as a game master does at the table: on arrival
it sets the scene, after a clue is seen or worked out it recalls what the party has found and
earned, and it turns their eyes to what is still to look at, never to what that shows. It never
solves: it never names, links or compares found pieces, and never suggests setting them side by
side (the owner caught it telling the players which clues to combine in dungeon run 23; how the
pieces fit is the players' to work out). Spoken lines have no semicolons, the narrator's and the
characters' alike (`story_narrate.rs`, `line.rs`): the speaker ends the sentence and starts
another. It speaks to
the party ("you"), names nobody and never speaks for a character. The storyteller decides whether
a word would help and words it; the names judge and a hidden judge (the truth, who is behind it,
secrets, the danger's cause, what an unseen thing or a not-yet-worked-out one would show, and
which pieces work one another out, `links_not_worked_out`) refuse a line, a pairing of found
pieces and a suggested comparison included (a recall, one thing in sight, or a nudge that names
no pieces together passes), which is asked again with a fixed reason (never what the judge read in it). Each line it
says is kept in the stored story (`narration`, place and line), and it is told its earlier lines
at the place, so each one adds something new rather than repeating them (dungeon run 7 restated
the same finds three times). What the party has found is read from the story's `findings`.
`persona story reset`, or restoring the story file, starts a run with none.

```
persona story narrate [recent=<line>]... [model=<name>] \
  <place> arrival|found|worked_out|stalled|approach   # prints the line, or nothing when it keeps silent
persona narrate [speak=true|false] <text...>    # shows it in the "Dungeon Master" box, voiced
```

The box stays up 12 s plus 70 ms a character after the line is cued; a newer line keeps it up
and the newest one's rest takes it away (`SceneChange::ClearNarration`). A plate with a caption
refuses the box (the two share one place). The voice is the `narrator` voice key: set it with
`speech voice narrator <voice-id>`, a warm older storyteller voice (on ElevenLabs, a voice from
`GET /v1/voices` whose labels read an old age, a narration or story use and a warm tone; on
Forge, `Uncle_Fu`). Until it is set, the speech service picks one as it does for any character
without a voice; a line it cannot voice is shown unvoiced and `persona narrate` says why.

`development/market-thread.sh` asks it in flowing play at each arrival (once the plate is up,
before the first move) and after a line that newly sees or works out a piece. Its word is composed
in the background while the next move is planned, and spoken just before the next line is voiced
(or before the party leaves, or at the end), once the last line is heard out (3 s plus 70 ms a
character); the run waits the same for its own line. The runner prints `DM: <line>`.

Hints when stuck: the clue route (`persona story clues`) counts the player lines at a place since
the last piece newly seen or worked out; at `STALLED_PLAYER_LINES` (6, `story_clues.rs`) it prints
`stalled=true` once a stretch (the reply's `stalled=` line), and progress or a new place starts a
new stretch. The runner then
asks the `stalled` moment and prints `DM (hint): <line>`: only one of a recall of the scene, one
thing on show not yet looked at (`unseen_here`), or a contentless nudge when none is left, never
what a look shows. The pairing rule above holds at every moment. The stretch lives in the
host's memory, so a host restart or `persona story reset` clears it. Audit lines: `PERSONA story_narrate place=.. moment=..
speak=.. duration_ms=..`, `PERSONA story_narrate_retried place=.. why=names|hidden|unreadable`
and `PERSONA narrated chars=.. voiced=.. rest_ms=.. revision=..` (no story text).

## The way between places

Owner, 2026-11-09: the party draws near the next place slowly instead of being put there. The
plot holds a `way` between each place and the next (`ways`: `from`, `to`, two or three `stages`,
each `{id, sight, worth_a_look}`; `worth_a_look` is one sign, mark or sound that adds mood or
warns of the danger ahead, never a clue, or null). `persona story plot` plots them with the story;
`persona story ways` plots them for a stored one. Code refuses a way that skips or repeats a
place, a stage count outside 2 to 3 and a stage id that is no new plain id.

A way carries no clue (dungeon run 24: the `inscribed_turn` stage showed the carved inscriptions,
a reliquary exhibit, where clue checks are off, so players who examined them got no credit). The
ways question is told the story's evidence (`evidence`: each piece's id, `what`, `up_close` and
`shown_at`, the places it is on show at), and every stage answers `touches`, the ids of the pieces
its sight or worth_a_look shows (a closed list of the evidence ids). Code refuses a stage that
touches any piece, or names an id that is no piece, and the question is asked once more with why
(`previous_refusal`). A plot made with its ways is checked the same way against its own evidence.

Each stage can have a plate made before play: a fresh plate takes 90 to 400 s (Forge is the slow
end), and in run 24 most stages were over before theirs came, so the screen kept the last place's
picture. `persona story way-plates party=<id>,... <how the party looks>` makes one plate per stage
of every way: the stage's sight and thing worth a look, the party going on together as the words
say (take them from the beats' `@place_scene` lines), a wide shot in the house look, drawn after
the party's reference sheets, then scanned and named for the `party` ids (`media locate`,
`media locate identify`). It is named `way-<stage id>.png` and kept on the stage (`plate`, shown
by `persona story show`; audit line `PERSONA story_way_plate stage=.. plate=..`). A plate already
in the library with its scan is kept, not rendered again, so a second run only fills what failed.
A stage id that ways plotted again reuse keeps its old plate: `media remove way-<stage id>.png`
first to have it painted anew. Check the `n of m characters named` of each made plate: whoever
the scan misses is out of the stage's cast while its plate is up.

For the drowned-archive quest (scenes scriptorium, reliquary, vault), with the host up:

```bash
$D dev --port 7879 persona story ways      # plotted again, with no clue on the way
$D dev --port 7879 persona story show      # read the ways before painting them
$D dev --port 7879 persona story way-plates \
  party=warrior,mara_adventurer,jory_adventurer,sella_adventurer,village_torchbearer \
  "the tall blond bearded man of about thirty-five with shoulder-length wavy hair, in a shaggy grey-brown fur mantle over a worn wool tunic and mail, a sword at his hip; the dark-haired woman of about forty with a long face and black hair under a dark green wool hood and cloak, a leather belt and satchel; the broad red-bearded man of about thirty with curly red hair, in a heavy brown wool travelling cloak over a leather jerkin; the young woman of about twenty with a single long brown braid and freckles, in a scuffed brown leather jerkin over a cream linen shirt; the thin village youth of about sixteen with short tousled light-brown hair and a narrow freckled face, in a patched brown wool tunic with a rope belt, holding up the torch"
$D dev --port 7879 persona story reset     # before the run, as always
```

Where play is belongs to the host (`phase` in the story: `place`, or `way` with `from`, `to`,
`stage`, `moves`; cleared by `persona story reset`, shown by `persona story show`). The set-off
move puts play on the way. On the way `persona story next <place left>` gives a way move (players
talk, look at the thing worth a look, react; never a set-off). `persona story phase` then asks the
stage judge (typed `served`) once the stage has had a move, and moves on without asking after
two, so a way never stalls; the last stage gives way to the next place's plate, music and arrival.
On entering a stage the runner puts up its plate made before play at once (`plate=` in `persona
story phase`, shown with `scene image tv-backdrop` as a place's `@place` plate is, and printed as
`=== on the way: <stage> (<plate>)`); a stage without one keeps the picture that was up. Then it
plays the stage with its own frame watch for later shots (`scene=` from the host: sight, the thing
worth a look, the party by their looks; on the place's `@frames` timer, so every 60 s in a quest),
the stage's situation for everyone, and the narrator's `approach` word (`persona story narrate
<place left> approach`, kept under the stage id). Clue checks are skipped on the way, which is why
a way may carry no clue. Audit lines: `PERSONA story_ways`, `PERSONA story_ways_retried`,
`PERSONA story_way_plate stage=.. plate=..`, `PERSONA story_phase at=.. place=.. to=.. stage=..
moves=..`, `PERSONA story_stage_judged stage=.. served=..` and `PERSONA story_next .. stage=..`.

## The strike at the last place

At the story's last place, once every player has had their turns, the danger strikes: the next
move goes to one it holds, the story's culprit when they are there (by id), else whoever is there
and no player (a trap's keeper). Whether it has struck is the pace judge's typed `struck` (someone
it holds acted so its purpose shows; its signs alone and others watching are no strike), never who
spoke last: in dungeon run 7 the torchbearer's "I'll keep the light steady" counted as the strike,
so the rival was never sent in. The end is judged only once it has struck. The storyteller is
told the strike move is an act (step out, seize what they came for, bar the way), never a
question to anyone, a look into something or keeping watch, written to the speaker as "you" and
never naming their own part as someone else; the knowledge judge is given the speaker's
`own_part` (part, role, and the place's danger for the culprit), so `to_self` refuses "before the
rival can take it" given to the rival (run 24). That move comes back with a typed `strike=true`
(only when `strike_now` and its speaker is one the danger holds); the runner passes it to `persona
say strike=true`, and the line route lifts the speaker's concealment (a must_avoid hiding who they
are or what they came for) for that line only: they act in the first person, every other rule
kept. In run 24 the culprit's guidance kept his strike hedged, so it was never judged struck.

After the strike the story ends only when its truth comes out. In run 25 the culprit struck ("I
take the genuine charter"), the players only held him back, he only repeated his defiance, and the
pace judge never said served, though every clue was worked out and his secret's reveal conditions
("confronted with proof of the forged charter and planted traces") already held. Now, once the
danger has struck at the last place, the revealable judge (`story_revealable`, a closed verdict
list per secret id of those there and no player, checked against the ids) weighs each secret's
reveal conditions against what the party found and what was said. Its answer is kept in the stored
story (`revealable`: the place, the findings count it was judged on, and holder and secret) and
asked again only when the findings change; a secret once revealable stays so, and `persona story
reset` clears it. The game master is given `revealable` (holders by handle; null before the
strike): a player's move confronts a holder with the matching proof, and the holder, once
confronted, may confess. The line route gives the speaker `now.revealable` (their own revealable
secret ids): such a secret is let out in character (a confession or a defiant admission) without
asking the line's reveal judge again, while any other secret a line lets out is still weighed and
refused when its conditions do not hold. The pace judge then sees the truth out in `said_here`,
`served` follows, and a player's move ends the story (`end=true`). Audit line: `PERSONA
story_revealable place=.. secrets=.. revealable=.. duration_ms=..`.

## Plates on the schedule

`persona watch every=<seconds>` asks every frame judge (a line's, a focus's, a scheduled room frame's)
as an owned task beside the frames being made, so a judge's minute holds back no tick or line
(dungeon run 8: awaited in the tick, a frame started only once a minute; run 13: awaited after
each line, the ticks stalled 340 s). One line judge is out at a time; lines said meanwhile wait
for the next (run 14: a judge a line slowed every model call to about a minute). A judge out counts as a
frame on its way; frames start ahead of their slot to cover the averaged judge time plus making
time, up to `MAXIMUM_PARALLEL_FRAMES` (12) at once. A schedule judgement overtaken by a frame cued
by a line is dropped, and lines said while a judge works stay for the next judgement.

Finished plates queue for slots (`claim_room`): one goes up every `every` seconds (60 in a quest), the oldest first, and a
plate with two newer ones waiting gives way, so none is thrown away while the screen keeps close
to the talk (runs 10 and 12 dropped finished plates and showed nothing for 171 s and 89 s). A plate
older than a line-cued one already up never goes up, and the newest picture (an edit's base, the
room an insert returns to) only moves forward. The strike is latched in the stored story
(`struck`, by place) once the pace judge sees it.

## The speaker's outline

While a character's line is up, the page traces a thin warm rim and glow round their figure from
their SAM mask (`.rp-outline`, drawn with the anchored bubbles; the SAM layer stays off). It has no
message of its own: it follows the anchored bubble that shows words (a thinking indicator, a
character without a mask or a bubble without an anchor draws none). A scene with masks needs no
model: `playground go ui_roleplay_thought`, then
`scene roleplay set bubble=1 speaker=Sylvi tone=ally anchor=elf line="..."`. Timing: `clients
timeline <client> kind=outline` (`TIMING outline` in the host log) gives `start_ms`/`end_ms` per
character; the start equals the bubble's `CLIENT bubble` draw and the end is the next speaker's
start (`reason=replaced`) or the line leaving (`removed`). Use `--package DIR` and your own
`wasm-bindgen` output when you must not overwrite `artifacts/playground/pkg`.

The outline keeps only the figure: `figure_of` (`df-imagery/src/oneshot_segment.rs`) cuts a scan's
mask down to its connected piece that holds the head point (or the nearest piece), so stray
pieces elsewhere in the picture no longer join the rim; the locate scan uses it (`layout_from`,
`df-table-host/src/locate/scan.rs`). A layout scanned before keeps its old masks until it is
scanned again.

A line whose speaker is not in the picture on screen (a clue close-up is up, so the room's scan does
not apply, or the layout has no region of that name) is not a pointing bubble: it is a lower-third
card (`.rp-card`: face cut from the room picture, shown name, line; one at a time, above the clue
caption) that fades in and, as a kept copy, out. Cue it on a close-up with the same
`scene roleplay set ... anchor=<character> line="..."` after `scene backdrop` shows the item
picture. While the backdrop changes plate, the stage is `tv-fading`: the bubbles and outlines fade
out, a hidden twin of the visible speaker's line (`.rp-card-twin`) fades in, and both swap back when
the new plate has finished fading in (`view::backdrop_fade`; the fade follows the controls'
transition and starts once the image has loaded). `clients timeline <client>` has the kinds `card` (`card-<character>-<line>`),
`fade` (`fade-load-<media>`, `fade-<media>`) and `third` (`third-<character>`).

The camera follows the speaker (`CAMERA speaker character=.. move=push|hold|wide|rest ..` in the
host log): a spoken line (`persona say`/`ask`, `scene cue <c> speak`) eases the camera over 2 s to
the `speaker` shot of that character (112 %, `region:<name>:speaker`), the same speaker again
holds, a character with no region on the plate sends it wide, and a thinking cue never moves it.
Only the speaker keeps a bubble and outline; 15 s plus 70 ms per character after the last line the
line leaves and the camera eases back out. `clients timeline <client> kind=camera` gives each move.

## Forge as the model provider (optional, no spend)

The owner's local Forge studio (`Game Development Studio`, `http://127.0.0.1:5173`) can stand in
for the paid suppliers, each concern on its own switch. Unset keeps the paid default.

| Concern | Switch | Forge model | Paid default |
| --- | --- | --- | --- |
| voices and speech recognition | `DF_SPEECH_BACKEND=forge` | Qwen3-TTS 1.7B CustomVoice, Nemotron ASR | `elevenlabs` |
| SAM scan (`media locate`) | `DF_SCAN_BACKEND=forge` | SAM 3.1 | `fal` |
| picture generation | `DF_IMAGERY_MODE=live DF_IMAGERY_BACKEND=forge` | Qwen Image 2.1 | `codex-cli` |

All three also need `DF_FORGE_PROJECT=<forge project id>` (the renders land in that project;
`GET http://127.0.0.1:5173/api/projects` lists them; "DungeonFlux dev" is
`41fbb3c9-ac91-4897-b890-7a056b4ae371`). `DF_FORGE_URL` overrides the origin (loopback
`http://` only). An unknown switch value refuses (`missing=DF_SPEECH_BACKEND`,
`unknown_scan_backend`), never falls back to a paid supplier. `services` and `speech status`
show the models. The host starts only the one engine a call needs (never start-all) and never
offloads: check `GET /api/runtime/services` and offload by hand
(`POST /api/runtime/models/<id>/offload`) when you are done. A job Forge marks `interrupted`
is never resubmitted; resolve it in Forge's Recovery panel.

When the Codex image subscription is spent (`usage_limited`, after about 3 hours of continuous
plates; text keeps working), switch picture generation to Forge on the running host, without a
restart: `df-table dev --port 7879 media backend forge` (with the `DF_FORGE_URL` and
`DF_FORGE_PROJECT` the host was started with, or `url=` and `project=`), and back with
`media backend codex-cli` (the host's own `CODEX_HOME` and Codex settings). `media backend` alone
shows the backend and its limits; `services` shows it too. Jobs already running finish on the
backend they started on; a backend whose setup does not hold is refused
(`IMAGEGEN refused verb=backend to=..`) and nothing changes; a switch is one
`IMAGEGEN switched from=.. to=.. running=..` line. Start the studio first with
`C:\Users\mreca\Desktop\Game Development Studio\start-server.ps1`. Forge's refusal reasons show
as `detail=` on the `IMAGEGEN` failed lines of the host log. The voice supplier
(`DF_SPEECH_BACKEND`) still needs a restart to change.

How each maps: a voice id that is a Forge speaker (`Ryan`, `Aiden`, `Vivian`, `Serena`,
`Uncle_Fu`, `Dylan`, `Eric`, `Ono_Anna`, `Sohee`) is used as is (`speech voice elf Aiden`);
an ElevenLabs id maps to one speaker by a stable hash. Clips are `voice-<n>.wav`. Nemotron is
English only and has no key-term boosting. The scan runs one Forge job per instance (Forge
saves only the top-k matches as one mask). `FORGE` lines in the host log give queue and render
times (`FORGE job kind=voices outcome=completed queued_ms= render_ms=`).

Measured through the host on 2026-11-08 (RTX Spark, `FORGE` and `SPEECH` lines): TTS cold
57.6 s for a 6.5 s line (engine start 2.8 s, model load 32.5 s, render 22.3 s), warm 8.5 s for
a 2 s line, so Qwen3-TTS renders about four times slower than the speech lasts and a voiced line
arrives seconds after its text; Nemotron cold 4.4 s (load 4 s), warm 0.3 s for 6.5 s of audio;
SAM cold 21.5 s for two people, warm about 2.4 s per instance (16 people in 40 s); Qwen Image
cold 227 s for a 1664x928 plate (about 95 s of it the model load).

## Image vendor limits

Compiled 2026-11-09. Tags: `c` code (file:line), `d` vendor docs, `o` observed on 2026-11-09. The
host (`/dev/media/generate`) selects only `codex-cli` and `forge` (`imagegen/backend.rs`
`backend_for`); OpenAI, fal and BFL run only in `df-imagery`'s durable dispatch. The text sent is
prompt + `\n\n` + references line (237) + `\n\n` + plate direction (123), so 364 characters go on top
of a plate prompt that has references (`imagegen/model.rs:405`).

| Vendor | Prompt | Pictures in | Output | Edit base | Parallel | Per picture | Limit, our error | Cost |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Host caps | 32,000 chars (c `request.rs:19`); 16,000 on the one-shot path (c `oneshot.rs:41`) | 8 with the base (c `request.rs:21`, `oneshot.rs:44`) | asks a 1672 long side (c `imagegen/limits.rs:16`); shapes up to 4:1 | yes | no host cap (c `imagegen/service.rs:10`); frame watch up to 12 (c `persona/frame_watch.rs:87`) | deadline 15 min + 60 s grace (c `imagegen/limits.rs:8`) | over a cap: `unaccepted` (503) | none |
| Codex CLI (`codex-cli`) | 16,000 chars (c `oneshot.rs:41`); tool's own limit unknown | 8 admitted (c); 6 fail `refused` `Protocol(InputMismatch)`, 5 work (o): the tool echoes a different input list (c `codex_cli/protocol.rs:824`) | size only asked in the text (c `protocol.rs:546`); tool picks about 1672x941 for wide (c `imagegen/limits.rs:14`); result at most 10 MiB (c `df-assets/src/validation.rs:6`) | yes, staged as the first picture | 12 ran at once (o) | 90–220 s (o) | `usageLimitExceeded` -> `usage_limited` 503 (c `protocol.rs:757`, `imagegen/backend.rs:117`) after about 3 h of up to 12 parallel (o); 5-hour/weekly windows, images use them 3–5x faster (d, third-party) | ChatGPT subscription, then credits (d, third-party) |
| Forge, Qwen Image 2.1 | 4000 UTF-16 units incl. directions (c Studio `runtime/jobs.js:683`); over it `forge_failed` `rejected` in under 0.4 s (o); token limit unknown (d) | 10 staged (c `jobs.js:721`, Qwen `app.py:169`; d model card); PNG/JPEG/WebP, 10 MiB each, 40 MiB total (c `runtime/references.js:6`) | sides 32–2752, multiples of 32 (c `jobs.js:697`, `forge.rs:62`); published 1:1 2048, 16:9 2752x1536, 4:3, 3:2 and portraits (d) | yes, as first reference (c `imagegen/backend.rs:540`) | one render at a time: engine lock (c Qwen `app.py:25`, `:162`); the queue takes more, no cap found | 100–185 s warm (o); 227 s cold (runbook above) | any 4xx -> `rejected` (c `forge.rs:893`); no usage limit | local GPU |
| OpenAI API (`gpt-image-2.5-*`) | 32,000 (c `df-providers/src/lib.rs:8`; d) | code 4, 4 MiB each (c `api/openai.rs:33`); docs 16, 20,971,520-char data URL (d) | x16 sides, max 3840, 1:3–3:1 (c `openai.rs:219`; d); pixels code 655,360–4,194,304 (c `openai.rs:35`), docs to 8,294,400, above 2560x1440 experimental (d) | yes (edits; mask) | by tier: 20 / 150 / 250 images per min (d) | up to 2 min (d) | 429 -> `RateLimited`; 429 `insufficient_quota` -> `QuotaExhausted` (c `openai.rs:175`) | per token; gpt-image-2 1024x1536 $0.005/$0.041/$0.165 low/med/high (d) |
| fal (`flux-pro/v1.1-ultra`, `kontext/max/multi`) | 32,000 in code (c `api/fal.rs:800`); docs unknown | code 4, 4 MiB each (c `fal.rs:785`); docs unknown | 9 aspects 21:9–9:21 (c `fal.rs:43`; d); ultra up to 4 MP (d) | Kontext multi only | account cap, 2 for a new account, IN_QUEUE not counted (d) | ultra about 10 s (d); Kontext unknown | 429 -> `RateLimited` (c `fal.rs:764`) | $0.06 ultra, $0.08 Kontext multi per image (d) |
| BFL (`flux-3-image`) | 32,000 in code (c `api/bfl.rs:684`); docs unknown | none: portrait 1024x1536 only (c `bfl.rs:692`); docs (third-party) say 10 | 2:3 "1.5k" only (c `bfl.rs:702`) | no (code) | 24 active tasks (d) | unknown | 429 -> `RateLimited` (c `bfl.rs:644`); 402 out of credits is a generic provider error (c; d) | per image, 1 credit = $0.01 (d) |

What the host does about them (2026-11-09): each backend states its limits (`media backend`,
`services` `imagegen.limits`), and a request over them is refused before any job. Codex takes 5
pictures, no text bound below 16,000 and no negative prompt; Forge takes 8 pictures, 4000
characters of text and a 4000-character negative prompt. A room plate attaches the people's sheets
first, then each thing's close-up (`Reference N: <thing> as it looks.`); a thing without one, or
whose close-up finds no room, is told in words, and the transcript names what was dropped. The
house look's bans (`BriefStyle::avoid`) go to Forge as `negative_prompt` with `true_cfg_scale` 4
(about twice the render time) and to Codex as a `Never show:` line. On Forge the frame judge gets
a target length for its scene (`picture_target_characters`): what is left of 4000 after the rest
of the plate, less 200; a longer answer is asked again, never cut. The scriptorium plate (five
people, three close-ups): before, 4278 characters and 5 pictures on both; after, Forge 3715
characters (scene target 2486), 8 pictures and a 497-character negative prompt, Codex 4329
characters (a 1500-character scene) and 5 pictures.

What plates must respect:

- Prompt budget: Forge takes 3636 prompt characters with references (3875 without), and tonight's
  4100–4450-character room prompts were over it. `maximum_prompt_chars` checks against Codex's
  16,000, not Forge's 4000, so a long prompt reaches Forge and is refused there.
- Pictures: send at most 5 with the base to Codex (6 fail, though the code admits 8), and up to 8
  to Forge (10 allowed, but the host caps 8).
- Concurrency: Forge renders one at a time, so in-flight requests beyond one only queue (about
  150 s each). Codex ran 12 at once but hit `usage_limited` within about 3 h.
- Failures: `usage_limited` (503) means the subscription window is spent; stop and wait for the
  reset. `refused` with `Protocol(InputMismatch)` means too many pictures. `forge_failed` `rejected` is a
  Forge 4xx (long prompt, bad size or reference), and `interrupted` stays for Forge's Recovery panel.

## Troubleshooting

| Symptom | Cause | Fix |
| --- | --- | --- |
| `backend: home_has_config` | image home is your own Codex home | dedicated home, `codex login` (above) |
| `configured=false missing=DF_TEXT_CODEX_EXECUTABLE` | Codex app updated, old path gone | re-run `ls` for the new `codex.exe`, restart the host |
| `move act ...` prints `usage: move <id> <x> <y>` | stale binary or verb not routed | rebuild, confirm `Finished`; stage verbs are routed in `command.rs` |
| build "succeeds" but behaviour is old | compile error hidden by `\| head` | run `cargo check` and read the errors |
| `cargo build` fails to link | the host holds `df-table.exe` | stop your host first; Windows can hold the file a few seconds after `Stop-Process`, so build again and look for `Finished` before restarting (a failed link leaves the old binary running) |
| a resident has no persona after the plate | the scan did not name their region | `scene cast` to see who was found; rerun, or `persona generate <id>` after a region is named |
| a character's head is cut off at the top of the picture | the image model's framing; the prompt does not control it | rerun the plate; a framing line in `BriefStyle` is the planned fix |
| whole party moves on "we" | Jev had no context | `move act` passes the situation as context; real play passes recent lines |
| OpenAI Responses `credit_balance_exhausted` | key has no credit | add credit or use `DF_TEXT_BACKEND=codex` |
| an `emotions.json` or persona edit has no effect | the host reads its scratch copy (`artifacts/tmp/persona-live/playground`), not `assets/` | edit both, or copy the file into the scratch copy; no restart needed |
| no line is voiced | the host was started without `DF_ELEVENLABS_API_KEY`, TTS is off, or the page's audio is locked | export the key and restart; click TTS in the toolbar (it also unlocks audio); `playground` shows `tts:` and `audio` |
| a place goes round and round, nobody leaves | the pace judge keeps reading it as not served | a middle place is left once every player has said about three lines there; read `PERSONA story_next served=` in the host log |
| an earlier story or a past run leaks into lines (Kael's arrest at the market) | personas keep what earlier stories and runs wrote into them | restore the cast's clean personas before the run (Play the quest, step 2) |
| a close look finds nothing, or names go unchecked | an old `story moves` saved its nameless view, blanking names and other scenes' exhibits | fixed at `f0bd97da`; restore the plot from a copy made before, or plot again |
| a SAM tag or bubble shows an id (`tavern_server`) | the plate was named before shown names, or the persona has no name | `persona set <id> display_name <Name>`, then `media locate label` (Names, above) |

## Coordinator practice during play

Owner, 2026-11-09: while a quest plays, the coordinator watches the TV, hands every fix to a
subagent rather than making it, and asks before reloading the page mid-scene.

## Known gaps

Reference portraits for character consistency are not built (cards carry job lines only).
The design step is a model call (`POST /dev/move/design`) that may invent residents; their
ids are not checked against the persona routes' reserved words (`list`, `all`, ...). Picture
framing is not controlled (a face can be cut off by the frame). Owed tests are marked
`TODO(DEV-12)`. The transition's Generating, Locating and Placing timings are written after
the plate call returns. `persona converse` has no topic shifts. Each plate run spends an
image generation and a fal scan.

Flowing play (2026-11-08):
- **Flirting:** the flirting at the inn never comes, because the plot's inn beat has no room for
  it.
- **Speaking another's part:** a speaker now and then plays the other's part (a player telling a
  witness's sighting as his own), and an answerer can invent a sighting it never had.
- **The trap:** it springs mildly, with the schemer calling his watchers out rather than an
  ambush.
- **Players' plans:** players who plan to go somewhere else are taken to the story's next place
  anyway.
- **Forge voices** trail their line by several seconds (about 8 s for a 2-second line warm, about
  58 s cold), too slow for live play.
