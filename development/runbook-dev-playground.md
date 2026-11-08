# Runbook: the dev playground, personas, move transitions and plates

Verified live on 2026-11-07 against commit `f2a0160f` plus the market and residents work (Windows ARM64, Git Bash). Every
recipe below was run end to end. This is a development runbook for the dev host, not the
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
   export it into the host's environment, never print it).
3. **Find the current Codex executable.** The desktop app moves it when it updates, and the
   text service then reads as unconfigured (`missing=DF_TEXT_CODEX_EXECUTABLE`):
   `ls -d "$LOCALAPPDATA/OpenAI/Codex/bin/"*/codex.exe`.

## Build (warm lane)

Use one shared target directory, never `--target-dir` per attempt. Stop the host first: it
locks `df-table.exe`.

```bash
export CARGO_TARGET_DIR=$PWD/artifacts/build/bubbles RUST_MIN_STACK=67108864
powershell -Command "Get-Process -Name df-table -ErrorAction SilentlyContinue | Stop-Process -Force"
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

# Any edit by hand
$D dev --port 7879 media generate "Lena now laughs" as=bar-edit base=bar.png wait=true
```

Settings, re-read on every call (no rebuild): `persona/frames.json`: `gate_percent`, `kinds`
(the closed list the judge may name), `scale` (what each percentage means; the model scores by
it), `edit_rule` (how an edited frame keeps the picture) and `shots` (the camera shots a frame
may take, each an id and how it looks: two-shot, close-up, extreme close-up, over-the-shoulder,
telephoto, wide, Dutch angle, low angle, high angle). The judge picks the shot that tells the
moment and is told the shot on screen, so it varies; the edit is told to frame it that way.
`with=warrior,tavern_server` on `persona converse` chooses who talks (otherwise everyone in the
scene with a persona talks, the band and dancers included). Command options override them for
one run. Frames are stored as `<frames>-1`, `<frames>-2`...; a name already in the library is
kept, not made again, so give each run a new `frames=` name. `scene=` matters: it is what the
judge compares the lines against. Without it the judge starts from the cast list alone. The
transcript says when each frame was judged (and its changes), started and shown, and how long it
took; the host logs `PERSONA frame_judged changed_percent=.. gate=.. triggered=.. duration_ms=..`.

## The market thread (re-runnable persona and backfill exercise)

`bash development/market-thread.sh` runs 20 messages at the fruit stall: her name, where the
apples come from, the price, a discount on one apple (should be refused), a discount on five
(should be granted). The elf and the warrior get one goal per beat and word their own lines
(`persona say`); Mara is fully live (`persona ask`: coverage, then backfill of what her persona
lacks, then the answer). The script first adds her two discount rules to her persona (a firm
price on one piece, a bulk discount at five; triggers are statements the model judges). Needs
the market scene up (the walk-through above). Output: `[NN] speaker: line`, then coverage and
backfill result lines for Mara; `persona backfills fruit_vendor` lists what was added.

Any scene can use the same runner: a beats file sets who answers and where with `@keeper <id>`,
`@keeper_situation <text>` and `@speaker_situation <text>`. Example, the bar at night:
`bash development/market-thread.sh development/market-threads/flirt.beats` (Kael flirts with
Lena the server for 20 rounds). `@state <emotion> <level>` lines start the keeper mid-story
(the jealousy and farewell scenes start with attraction and affection already there). Never
edit the runner or a beats file while a run uses it: bash reads a script as it goes, so an edit
garbles the run. Run a copy for long batches. Reset both first: `persona state <id> reset`. Each run's full
record goes in `development/persona-transcripts/` (`py -3 development/persona-transcript.py
<log> <out.md> <title> <header> [section=<beats>] warrior=Kael tavern_server=Lena`), its summary
in `development/persona-refinement-log.md`. The romance edge cases are
`market-threads/romance-{pushy,shy,jealous,farewell,rejected,dancer}.beats` (`@rapport <toward>
<value>` presets a standing); a speaker whose own persona refuses
a beat (Kael will not be pushy) needs a temporary stance in the scratch copy for that test
(`persona behavior warrior add drunk-pushy ...` plus a trigger), dropped afterwards.

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
`document.querySelector('.rp-title-spin')` exists while a plate generates (the "Updating the
scene..." chip), the backdrop `src` becomes `/playground/media/<name>.png`, and `scene cast`
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
ban what is modern separately from what is high-fantasy. The bar plate in this look is
`bar-period.png` (made with `media generate`, then `media locate`, `media locate identify` and
`media locate label` for the names).

## Keep the scene clean

`scene reset` clears live edits; the page chrome comes from the fixture. For a bare scene
set in `ui_roleplay_thought.json`: `roleplay.title: null`, `roleplay.caption: null`,
`pills: { "logo": "inline" }`. After `scene reset`, re-apply a backdrop with
`scene image tv-backdrop <name>.png`.

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
| a SAM tag or bubble shows an id (`tavern_server`) | the plate was named before shown names, or the persona has no name | `persona set <id> display_name <Name>`, then `media locate label` (Names, above) |

## Known gaps

Reference portraits for character consistency are not built (cards carry job lines only).
The design step is a model call (`POST /dev/move/design`) that may invent residents; their
ids are not checked against the persona routes' reserved words (`list`, `all`, ...). Picture
framing is not controlled (a face can be cut off by the frame). Owed tests are marked
`TODO(DEV-12)`. The transition's Generating, Locating and Placing timings are written after
the plate call returns. `persona converse` has no topic shifts. Each plate run spends an
image generation and a fal scan.
