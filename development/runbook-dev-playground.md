# Runbook: the dev playground, personas, move transitions and plates

Verified live on 2026-11-07 against commit `f2a0160f` plus the market and residents work (Windows ARM64, Git Bash). Every
recipe below was run end to end. The flowing quest ("Play the quest end to end") was verified on
2026-11-08 at `f0bd97da` (run 111: market, inn, graveyard, the trap sprung, the end). The free
roam ("Free roam: the current run procedure") was run ten times: runs 1 to 5 on 2026-11-09 on the
Forge image backend at 512x512 and 20 steps, runs 6 to 10 on 2026-11-10 on the Codex image
backend; runs 9 and 10 (stamps 11100339 and 11100557) are the ones this procedure was last
checked against. What those runs found and what is still open are in "Known
gaps". This is
a development runbook for the dev host, not the
shipped launcher. Plan: [scene-transitions.md](../planning/scene-transitions.md).

Where to start: the roam is the current work, so read "Free roam: the current run procedure",
"Standing rules" and "Systems and where they live" first. The older recipes (the tavern, the
market, the quest) still work and are kept below; a section's date says when it was last
confirmed.

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

## Free roam: the current run procedure (verified 2026-11-10)

One roam is Danny, a lone painter, walking 20 moves (`market-threads/free-roam.beats`) and then
showing his paintings. "Free roam" says what each part does; this section is how to run it. The
steps are the ones that worked for runs 3 to 5 on 2026-11-09 (Forge pictures) and runs 6 to 10 on
2026-11-10 (Codex pictures). Text comes from Codex, the character scan from fal and voices from
ElevenLabs (both spend). Pictures come from either backend, and the steps say where they differ:
**Codex** (runs 6 to 10, the current default: a move's plate and walk sights take 1.5 to 3
minutes) or the **Forge** studio (Qwen Image at 512x512, 20 steps, no spend). The beats file asks
for at most three reference pictures a plate (`@refs max=3`) on either. Run everything from the repository root in Git Bash, on the
one dev host, port 7879 (see "Standing rules"). Each block assumes the shell of the one before.

Before step 1 these must hold:

- `.env` has `DF_FAL_KEY` and `DF_ELEVENLABS_API_KEY` (never print them), and the Codex image home
  exists (see "One-time setup").
- The scratch playground `artifacts/tmp/persona-live/playground` (a copy of `assets/playground`)
  holds `roam-00.png` with its scan and `danny` named in it, `ref-danny.png` and the
  `item-danny-*.png` sheets. They are gitignored. If the scratch is rebuilt, make them again with
  "Making the starting pictures" under "Free roam".

**1. Look at the Forge studio** (Forge pictures only; with Codex pictures go on to step 2). Its
queue is one line: a head job that waits for memory, was
interrupted or needs a person holds every render behind it. List the open jobs:

```bash
curl -s http://127.0.0.1:5173/api/runtime/jobs | head -c 800
```

If a job at the head reads `waiting_memory`, `interrupted` or `needs_attention`, read "Forge studio
operations" before going on. If nothing answers, start the studio:

```bash
powershell -NoProfile -File "C:\Users\mreca\Desktop\Game Development Studio\start-server.ps1"
```

**2. Stop the host you own, and only that one.** Find its process by the command line, never by
name: other agents run hosts of their own. The filter needs both the command line and the process
name: the PowerShell that runs the query carries the text `serve --port 7879` in its own command
line, so a command-line match alone stops your own shell too.

```bash
P=$(powershell -NoProfile -Command "(Get-CimInstance Win32_Process | Where-Object {
  \$_.CommandLine -like '*serve --port 7879*' -and \$_.Name -like 'df-table*' }).ProcessId")
[ -n "$P" ] && powershell -NoProfile -Command "Stop-Process -Id $P -Force"
```

**3. Build the host** in the warm roam lane. The exe cannot be rebuilt while the host runs from
it, so step 2 comes first. A build that prints nothing may have failed: look for `Finished`.

```bash
CARGO_TARGET_DIR=$PWD/artifacts/build/roam RUST_MIN_STACK=67108864 cargo --offline build -p df-table-host --bin df-table --locked
```

**4. Build and deploy the page** (only when `df-playground` or the page's CSS changed). The page's
wasm is built in the bubbles lane:

```bash
CARGO_TARGET_DIR=$PWD/artifacts/build/bubbles cargo --offline build -p df-playground --lib --target wasm32-unknown-unknown --locked
```

Then wasm-bindgen writes the package the host serves (`artifacts/playground/pkg`, the host's
`--package` default). The generator must be the Cargo.lock version, 0.2.129; this is what
`development/playground.sh build` runs:

```bash
wasm-bindgen --target web --no-typescript --out-dir artifacts/playground/pkg artifacts/build/bubbles/wasm32-unknown-unknown/debug/df_playground.wasm
```

The page's HTML and CSS are served from the scratch playground, not from `assets/`, so copy the
styles after a change (the scratch `kit.css` was found stale against `assets/` on 2026-11-09):

```bash
cp assets/playground/theme.css assets/playground/kit.css artifacts/tmp/persona-live/playground/
```

**5. Archive the last run's files** out of the scratch playground. The memory folder and the
numbered paintings and galleries are named by owner, not by run, so a new run would collide with
them. Use the last run's number for `RUN`. A glob with no match prints an error; ignore it:

```bash
RUN=10; S=artifacts/tmp/persona-live/playground; A=artifacts/tmp/roam-live/run$RUN-archive
mkdir -p $A/media && mv $S/world/memory $A/world-memory
mv $S/media/item-danny-painting_* $S/media/gallery-danny-* $S/media/*-show* $A/media/
```

The `*-show*` glob also takes the show's music (`tempo-show-<hash>.mp3`, the archives of runs 8 and 9
hold one), so the next run composes it again (the start of a run prints `1 composed (75 s of music
asked)`). Copy the finished run's `host.log` and `play<N>.log` into the archive too (runs 8 and 9
did); the next host overwrites its log.

**6. Start the host** with the scratch playground, the Forge image backend and Codex text. The
host does not read `.env`: export the keys into its environment. Leave out `--quiet`: the audit
lines are how a run is read. For **Codex pictures** (runs 6 to 10) the block is the same with
`DF_IMAGERY_BACKEND=codex-cli`; the scratch script `artifacts/tmp/danny-sheets/start-host-codex.sh
roam-00.png` is exactly that (gitignored, so it does not survive a cleanup: rebuild it from this
block). It sends the host log to `artifacts/tmp/market/host.log`, goes to the `weapons_stall`
fixture and then shows the plate named in its argument, so step 7 is done by it.

```bash
CODEX=$(ls -d "$LOCALAPPDATA"/OpenAI/Codex/bin/*/codex.exe | tail -1)
export DF_TEXT_BACKEND=codex DF_TEXT_CODEX_EXECUTABLE="$CODEX" DF_IMAGERY_CODEX_EXECUTABLE="$CODEX"
export DF_IMAGERY_MODE=live DF_IMAGERY_BACKEND=forge CODEX_HOME=$PWD/artifacts/codex-image-home
export DF_FORGE_PROJECT=41fbb3c9-ac91-4897-b890-7a056b4ae371
export DF_FAL_KEY=$(grep '^DF_FAL_KEY=' .env | cut -d= -f2-)
export DF_ELEVENLABS_API_KEY=$(grep '^DF_ELEVENLABS_API_KEY=' .env | cut -d= -f2-)
export RUST_MIN_STACK=67108864
D=$PWD/artifacts/build/roam/debug/df-table.exe
nohup "$D" serve --port 7879 --playground artifacts/tmp/persona-live/playground \
  > artifacts/tmp/roam-live/host.log 2>&1 < /dev/null & disown
```

The Forge project id is "DungeonFlux dev" (`GET http://127.0.0.1:5173/api/projects` lists them).
Check the services after a few seconds (text `configured=true`, imagegen live on `forge`):

```bash
sleep 6; $D dev --port 7879 services
```

**7. Show the starting plate**, Danny alone on a road, located and named `danny`:

```bash
$D dev --port 7879 scene image tv-backdrop roam-00.png
```

```bash
$D dev --port 7879 scene cast
```

`scene cast` must list `danny`. If it does not, the scan or the name is missing: see "Making the
starting pictures".

**8. No market items at the start:**

```bash
$D dev --port 7879 world place -
```

**9. Seed what he carries.** The runner seeds the same file again from its `@inventory` line;
doing it here shows what he holds before the run. He starts with boards 12, linseed oil 12, rags 12,
sketchbook pages 18, charcoal 6, ochre, umber and lead white 6 each, verdigris, madder and lamp
black 5 each, brushes 5, one easel, one satchel and 20 silver pennies (the file's counts; runs 9
and 10 started so):

```bash
$D dev --port 7879 world inventory set danny character development/market-threads/danny.inventory.json
```

**10. Set the image backend.** For Codex pictures:

```bash
$D dev --port 7879 media backend codex-cli
```

For Forge pictures set the size and steps. A `media backend` call clears what it does not repeat,
so say all of them. The studio's own default is 40 steps; 20 at 512x512 is what the Forge timings
below were measured with:

```bash
$D dev --port 7879 media backend forge project=41fbb3c9-ac91-4897-b890-7a056b4ae371 size=512x512 steps=20
```

**11. Reset Danny** (his state, not his persona file; `persona write danny
development/market-threads/danny.persona.json` first when that file changed):

```bash
$D dev --port 7879 persona state danny reset
```

**12. Page settings:** the TV layout and a 1.5 s fade between plates.

```bash
$D dev --port 7879 playground set device=tv transition=fade ms=1500
```

Leave the paid switches (TTS, STT, Music) alone. The host remembers every toolbar switch across
restarts ("Toolbar settings"), so what the owner left in the toolbar stands, and a first run has
all three off. The procedure never turns one on (owner, 2026-11-10): a voice or a composed track
is paid for. The owner turns TTS on in the toolbar, or a run that wants it says so in step 15 with
`DF_RUN_TTS=on` (`DF_RUN_MUSIC=on` for the show's music).

**13. Clear the clue board and the narration box** of the last run:

```bash
$D dev --port 7879 scene clues clear
```

```bash
$D dev --port 7879 scene narration clear
```

**14. Copy the runner**, never run the script itself (bash reads a script as it goes, so an edit
mid-run garbles it), and point its `cd` at the checkout:

```bash
mkdir -p artifacts/tmp/roam-live && cp development/market-thread.sh artifacts/tmp/roam-live/runner.sh
```

```bash
sed -i 's#^cd .*#cd /c/Users/mreca/Desktop/dungeonfluxGame#' artifacts/tmp/roam-live/runner.sh
```

**15. Run it** in the background, with the next free run number for `N`. `FRAMES_LOG` names the
frame transcripts (`frames<N>-<plate>.log` beside it). The runner never turns a paid switch on by
itself. Add `DF_RUN_TTS=on` (voices) and `DF_RUN_MUSIC=on` (the score is composed at the start)
only for a run that should spend; `off` turns one off for the run. Unset, the stored switch
stands, and the first line of `play<N>.log` says which it was: `[settings] tts=<on|off> (from
<stored|flag>)`, then the same for `music`. With Music off `tempo prepare` composes nothing (the
log reads `6 failed`, each `failed (disabled)`) and the roam plays on silent unless the library
already holds the tracks:

```bash
N=11; DF_TABLE_EXE=$D DF_TABLE_PORT=7879 FRAMES_LOG=artifacts/tmp/roam-live/frames$N.log bash artifacts/tmp/roam-live/runner.sh development/market-threads/free-roam.beats > artifacts/tmp/roam-live/play$N.log 2>&1 &
```

**16. Follow it** with a monitor. `grep --line-buffered` keeps the pipe flowing but `cut` buffers
its own output, so `stdbuf -oL` goes in front of it:

```bash
tail -n +1 -f artifacts/tmp/roam-live/play$N.log | grep --line-buffered -E '^\[(move|refs|walk|town|memory|see|thought|thing|dwell|look|insert|declare|painted|trade|inventory|reveal|pace|time|show)' | stdbuf -oL cut -c1-260
```

**17. Reload the page and click Enable sound.** Open `http://localhost:7879/playground/`, reload
it so it runs the package built in step 4, and click Enable sound in the toolbar: the click is
what unlocks the page's audio. `$D dev --port 7879 playground` shows `audio unlocked` when it
took, and `tts:` is what the owner left it: a restart keeps it, it never turns it on. Do this in the first
minute: the first move's plate is up after about 65 s on Forge and 1.5 to 2.5 minutes on Codex
(run 10: 145 s). Do not reload the page again mid-run.

A whole run on Codex takes about 6 minutes a move (5 to 6.5 minutes, longest 8.4): run 9 took 1 h
51 min for its 20 moves and then the show, and run 10's 15 moves took 1 h 28 min. Plan on about 2
hours for 20 moves plus a few minutes for the show.

The run ends with the `[show]` lines. Leave the host up while the inspector reads the run (see
"Standing rules"); stop it with step 2 when done. Stop the runner and the monitor only if you
started them.

### What each log line means

One roam writes these lines to `play<N>.log`; indented `~` lines are the runner echoing host
replies. A move that did not go live says why and he tries the next one from where he is.

| Line | Meaning |
| --- | --- |
| `[move NN] <way> \| said \| read \| place \| plate \| jev_ms design_ms plate_ms live_at_ms` | The move's trace (the host's `TRANSITION` records). `said` is his line, `read` Jev's reading of it (`kind`, `who`, `to`), `place` the designed place, `plate` the picture's name `roam-<MMDDHHMM>-NN`. `jev_ms` covers both readings when Jev read it twice, `design_ms` the design call, `plate_ms` the picture made (studio job plus scan), `live_at_ms` when the plate was behind the scene, from the move's start. |
| `[refs NN] shown base attached count maximum left_out` | What the picture was given: the design's items in sight, its base, the sheets attached (`ref-danny.png`, `item-danny-*.png`), the backend's budget (Forge 8; the roam asks for 3, `@refs max=3`) and what did not fit and was told in words. |
| `[town NN] <look>` | The look of the town that every sight and place of its moves is painted after (a town or market move gives it; the road move out ends it). |
| `[walk NN] stage k/N <sight> ..`, `[walk NN] <picture> made in .. ms`, `[walk NN] stage k/N .. shown_at_ms up_ms`, `[walk NN] arrived stages=S/N walk_ms` | The sights on the way to a place (`@walk`, "Walking about town"): each is made with the place, shown in order for at least 6 s, and spread over the arrival plate's wait. `stage k/N .. skipped reason=late\|failed` is a sight passed over; `[walk NN] memory ..` keeps each as a memory of kind `walk`. |
| `place <plate>: the active place, N thing(s) worth a close look` | The designed place became the world's active place with N things the camera can show. |
| `[memory NN] captured scene ..` | The plate was kept as a memory of kind `scene` (standard memory API). A last `[memory] ..` line names the owners and what they keep. |
| `[see NN] vision=true <what he sees> \| attending: <thing>` | Perception: what the model saw in the picture on screen, and what he attends to. |
| `[thought NN] <text>` | His thought about it (kept as a memory of kind `thought`). A new thing he noticed that the camera can go to is added to the place (`WORLD thing_added .. via=perceive` in the host log). |
| `[thing NN] <id> (<name>): <look>` | A thing the perception found in the plate and added to the place, so the camera can go to it (named; `(?)` is the unnamed form of runs 7 to 9). |
| `[dwell NN] the view was up <s> s before he spoke of it (dwell <s> s)` | How long the new plate was on screen before his line, against the beats' `@roam_dwell` (target 6 s). The wait is the perception, one Codex call of about 11 to 15 s (`PERSONA perceived .. duration_ms=`), so it is 10 to 14 s on Codex (runs 9 and 10), not the 6 s asked for. The show's `[dwell]` line is the town square's wait for the show plate. |
| `[look NN] <line>` | He steps up to a thing and says what he sees of it up close. |
| `[insert NN] item=<id> (<name>) frame shown_at_ms made_ms ended_at_ms up_ms` | The close-up insert: the frame judge named the item, the insert was painted from its look and shown (`shown_at_ms`), how long it took (`made_ms`) and how long it stayed (`up_ms`). `none: the judge saw no new close look` when it named none. From the host's `PERSONA frame_judged` and `frame_timing` lines (`events area=PERSONA`). |
| `[declare NN] <words>` then `[declare NN] plate shown_at_ms made_ms queue_wait_ms superseded ended_at_ms up_ms` | He declares what his hands are doing and the emergency plate of it: made, shown at once (`queue_wait_ms` is the wait for the screen after it was made), and its hold. |
| `[painted NN] <line>` then `[inventory] <item> a -> b (<why>)` | The finishing line, judged against what he really holds: the board and pigments are used up, brushes and easel are kept. |
| `[memory NN] painted <id> seq picture after plate` or `not painted: ..` or `study blank, not stored` | The study painting was kept as a memory of kind `artwork` and as a kept item, or why it was not made (no board used up) or removed (near blank). |
| `[reveal NN] painting <picture> \| <title> \| 10 s` then `[reveal NN] view <picture> \| The view \| 6 s` | The finished painting is shown full frame with its title, then the view it was painted from (`@roam_reveal 10 6`). |
| `[pace NN] painting <picture> up <ms> of <held ms>, view <picture> up <ms> of <held ms>` | What the host measured: runs 9 and 10 held 9992 to 10054 ms of 10000 and 6071 to 6277 ms of 6000 (within about 300 ms). |
| `[time NN] move took <s> s (plate .., of which walk .., look .., declare .., study .., reveal ..)` | Where the move's seconds went. `study` is only the wait left after the declaration (the study is made in the background). |
| `[trade NN] <line>` then `[trade NN] changes: ..; gained: ..` | A market stop (`@way_arrival market`): the sale of a painting and the purchases, applied to what he holds (`gained: no gain recorded` when nothing stood). |
| `[inventory] refused (move NN): <error>` | The host refused a change (`insufficient`, `kept_item`, `unsourced_gain`, ..). |
| `[inventory NN] a->b, ...` | What he held before and after the move, whatever its lines said. |
| `[paint NN] <line>` | His line toward `@roam_paint`; only in a beats file that has no `@roam_declare`. |
| `[show] composed gallery of N pieces -> gallery-danny-1.png`, `[show] plate .. after ..`, `[show] piece <id> <picture> \| <title> \| 5 s`, `[pace show] piece <picture> up <ms> of 5000 ms`, `[show] <line>`, then `[memory] owners: ..` | The day's end: the paintings composed into one sheet, the show plate edited after it and his character sheet, the last 20 paintings (`@roam_show_each 20 5`: count, seconds) shown for 5 s each, then the show plate stays up (run 9, with a count of 6, measured 5005 to 5236 ms; a piece that was not shown reads `piece - up - of 5000 ms`), what he says, and the owners' memory counts. |
| `[tempo NN] ..`, `[music NN] cue=.. track=.. \| decision=..` | The tempo engine and the music director's cue for the move ("Tempo and the music director"). |
| `[settings] tts=<on\|off> (from <stored\|flag>)`, `[settings] music=..` | The first lines of a run: whether each paid switch is on and whether the run set it (`DF_RUN_TTS`, `DF_RUN_MUSIC`) or found it as the host remembered it ("Toolbar settings"). |

### Measured timings (runs 4 and 5, 2026-11-09)

Forge at 512x512, 20 steps, an idle studio queue; from the `[move]`, `[insert]` and `[declare]`
lines and the host's `PERSONA frame_timing` and `PERSONA declared` lines.

| Stage | Measured |
| --- | --- |
| Jev reads the line (`jev_ms`) | about 4 to 6 s |
| Design call (`design_ms`) | about 6 to 8 s |
| Move plate (`plate_ms`) | about 46 to 65 s; live 58 to 70 s after the move began |
| Insert made (`made_ms`) | about 19 to 27 s; up 20.4 to 20.8 s |
| Declared plate made (`made_ms`) | about 55 to 80 s (127 s once, run 5 move 2; cause not found) |
| Declared plate queue wait (`queue_wait_ms`) | under 50 ms (15 to 35 ms); shown at once |
| Declared plate hold (`up_ms`) | about 20 s (20.0 to 21.0 s) |
| A whole move (plate, look, insert, declaration, painting) | about 4 minutes; 20 moves take about 75 to 85 minutes before the show |

These are observed ranges, not limits: read the real values from the run's own log. Before the
512x512 setting a 1664x928 plate at 40 steps took 103 to 226 s (see "Image vendor limits").

**Codex pictures (runs 9 and 10, 2026-11-10)**, from the `[move]`, `[time]`, `[insert]`, `[declare]`,
`[reveal]`, `[pace]` and `[dwell]` lines; run 9 has 18 timed moves, run 10 has 15.

| Stage | Measured |
| --- | --- |
| Jev reads the line (`jev_ms`) | 3.4 to 6.7 s, median 4.2 s |
| Design call (`design_ms`) | 6.5 to 26 s, median 8.6 s (the long ones asked again) |
| Move plate with its walk sights (`[time]` plate) | 97 to 173 s (the walk part 5 to 75 s) |
| Insert made (`made_ms`) | 38 to 56 s; up 20.0 to 20.3 s |
| Declared plate made (`made_ms`) | 59 to 138 s; shown at once (`queue_wait_ms` 21 to 69 ms); up 20.0 to 20.3 s |
| Painting reveal, then its view | 10 s and 6 s, measured 9992 to 10054 ms and 6071 to 6277 ms |
| Walk sight (`up_ms`) | at least 5.99 s (the beats ask for 6 s); the arrival plate follows it |
| New plate up before he speaks (`[dwell]`) | 10 to 14 s in run 10, 10 to 20 s in run 9 (target 6 s) |
| A whole move (`[time]`) | run 9: 232 to 503 s, mean 357 s; run 10: 291 to 391 s, mean 349 s |

### Run history

| Run | Pictures | How it went |
| --- | --- | --- |
| 1 to 5 (2026-11-09) | Forge | Fixes found one run at a time; run 3 reached the show, runs 4 and 5 were stopped at moves 9 and 12. Details in "Known gaps". |
| 6, 7 (2026-11-10) | Codex | Run 6 stopped on purpose at move 5; run 7 stopped after move 10 (the runner went quiet; why is not recorded, see H-1). |
| 8 | Codex | Moves 1 to 8; the boards ran out and it was stopped. |
| 9 (11100339) | Codex | All 20 moves and the show: 12 paintings, 0 duplicate Dannys in 17 declared plates, the show stepped 4 of its 6 pieces (`queue_full`, since fixed); two market moves lost, no board bought. |
| 10 (11100557) | Codex | Stopped by the owner at move 15 of 20: 15 moves timed, 0 refusals, the first board restock (move 7: boards 7 to 12). The show was not reached. |

## Standing rules

These hold for every run and every change to the dev host. Most are owner decisions; the date is
the decision's.

- **No string matching for gameplay decisions** (owner, 2026-11-07). What a character meant, who a
  line addresses, which action or branch applies, whether text reveals a secret: Jev's to judge (a
  model call against a closed schema, its answer checked in code against ids and enums). String
  matching stays for protocol, ids, file formats and command-line grammar. No word lists, no
  whole-word guards. See `AGENTS.md`.
- **Names never reach image prompts.** A character is painted from their look, and the display name
  and the id stay out of the picture prompt, the design call's facts and the design's texts (the
  image model paints any word it is given: a signboard read `Danny traveling`). See "Free roam",
  "Reference sheets".
- **No caps on dev model answers.** No length, count or `maxItems` limit on what a dev tool's
  model answers; only correctness checks (ids plain and unique, closed enums, references that
  exist). Frame prompts are the one place a length is a backend's own limit (Forge 4000 characters),
  and a too-long answer is asked again, never cut.
- **No reduced-motion handling in the page** (owner, 2026-11-09). No style sheet of the page names
  `prefers-reduced-motion`; animations always run. A test fails if one does
  (`crates/df-playground/src/motion_style_tests/theme.rs`). Do not add one to the page; the
  account app's reduced-motion rule is a different surface.
- **A run never turns a paid switch on** (owner, 2026-11-10). TTS, STT and Music are off on a first
  run and the host remembers each across restarts; no procedure, runner, start script or page load
  sets one. The owner turns it on in the toolbar, or a run asks for it with `DF_RUN_TTS=on` or
  `DF_RUN_MUSIC=on`, and its first log lines say which. With a switch off the host sends nothing
  to the supplier ("Toolbar settings"). See "Toolbar settings".
- **One dev host.** Only the host on port 7879 runs. You stop only the host you started, found by
  its command line (`*serve --port 7879*`), never by name. **Subagents start no servers**: they
  build and run unit checks, and the coordinator checks live.
- **The coordinator runs the live checks, and watches the screen.** While a run plays, the
  coordinator looks at the TV (plates, transitions, positions, occlusions, clues), hands every fix
  to an implementing subagent with a precise brief, and does not reload the page mid-scene.
- **An inspector agent looks at every picture and event of every run.** A long-lived inspector
  reads the run's log and opens every plate, insert, declared plate and painting. It writes to
  `artifacts/tmp/inspector/` (gitignored, so copy anything durable into this runbook): one
  `run-roam<N>-report.md` per run (metrics, findings numbered `R<run>-<n>` and graded S1 to S3, a
  per-picture verdict table), `run<N>-notes.md` while the run plays, `state.json` (runs seen, open
  findings, the checks for the next run) and `sheet.sh`, which makes a contact sheet of media
  pictures. What it checks each run: every move live, Jev and design refusals, the stage times,
  inserts against moves, declared plates for a second or third Danny (a duplicate is S1),
  perception lines and refusals, thoughts for repetition, inventory against what his finishing line
  said, market trades, paintings real and not blank, the gallery and the show plate, a page
  screenshot (whole picture visible, the thinking cue, readable bubbles).
- **Fixes go to implementing agents.** The inspector and the coordinator report; one subagent owns
  each fix and its files (`AGENTS.md`: no worktrees, one shared checkout, exact edits, never
  reset or rewrite another's files). An independent evaluator approves; the inspector does not.
- **Hints never solve** (owner, 2026-11-09). The Dungeon Master may recall the scene or point at
  one unexamined thing in sight; it never names which found pieces belong together or suggests
  comparing them. See "The dungeon master (narrator)".
- **Speed over test depth in gameplay refinement.** Small direct edits, one warm build, one browser
  check; mark each test still owed `TODO(DEV-12): <test>`.
- **Every interactive character has a persona and shows its name** (the display name on the SAM tag
  and the bubble, never an id); no speech bubbles are added to fixtures (the thinking cue is the
  indicator).
- **A quest shows a new plate every 60 s** (owner, 2026-11-09, relaxed from 30 s because Codex hit
  its image limit): its beats carry `@frames every=60` and the frame watch stays on. The
  `frames.json` default is still 30 s. The roam makes no scheduled plates (`every=0`): its move
  plates, inserts and declared plates are the pictures.

## Systems and where they live

All of these are standard and story-neutral: the roam's beats file is data, and none of the code
knows a painter. Code paths are under `crates/`; the sections named are the details.

| System | Route and verb | Code | Details |
| --- | --- | --- | --- |
| Inventory API | `/dev/world/inventory`, `.../owner`, `.../set`, `.../add`, `.../remove`, `.../consume`, `.../batch`, `.../transfer`, `.../history`; `world inventory ..` (also `sheets`); judged lines answer `inventory_changes`, a gain needs a `source` | `df-table-host/src/world/inventory*` (`inventory.rs`, `inventory/{cli,operations,request,routes,store}.rs`) | "Free roam", "What he holds" |
| Memory API and gallery composite | `/dev/world/memory`, `.../list`, `.../get`, `.../put`, `.../capture`, `.../delete`, `.../references`, `.../compose`, `.../inspect`, `.../history`; `world memory ..` | `df-table-host/src/world/memory*` (`memory/{cli,host,limits,operations,request,routes,store}.rs`); the sheet is `df_imagery::gallery` (`df-imagery/src/gallery.rs`) | "Memories" |
| Perception API | `POST /dev/persona/perceive`, `.../perceive/scene`, `GET /dev/persona/thoughts`; `persona perceive`, `persona thoughts` | `df-table-host/src/persona/perceive*` (`perceive/{answer,attending,cli,facts,request,routes,screen,thoughts}.rs`) | "Perception" |
| Declare action and emergency plates | `POST /dev/persona/declare`; `persona declare`; the frame watch's `@declare` input line; `media generate .. priority=emergency` | `persona/declare.rs`; `persona/plate.rs` (`make_declared`); `persona/frame_watch.rs` (`FrameWatch::declare`) | "Declared actions and emergency plates" |
| People check on a made plate | none (inside a declared plate: one count check, a second attempt, then a refusal) | `persona/people_check.rs` | "Declared actions and emergency plates", "Verified after it is made" |
| Jev: interpret, design and retry | `POST /dev/move/interpret`, `.../design`; `move interpret\|design\|brief\|act`; the second reading and the design retry carry `refused_before` | `df-table-host/src/movement/{interpret,design,routes,transition,brief}.rs` | "Free roam" (How Jev reads a roam line), Troubleshooting |
| Insert pictures | the frame route `POST /dev/persona/frame` names `inspected_item`; `persona watch .. insert_hold=` | `persona/frame.rs` (the judge), `persona/plate.rs` (`insert_request`, the item alone, painted anew), the playground's `persona/frames.json` (`insert` shot) | "Frames that follow the talk" |
| Reference sheets | character `ref-<id>.png`, item `item-<owner>-<item>.png`; `world inventory sheets <owner>`; an edit is given the single front view | `world/inventory*` (the missing-sheet ask); `df_imagery::sheet_view` (`df-imagery/src/sheet_view.rs`), applied in `df-table-host/src/imagegen/service.rs` | "Reference sheets", "Character reference sheets", "One person, once" |
| Page aspect-ratio mapping | none (the page decides, not a control): a plate within 5 % of 16:9 fills the frame (cover), any other lies whole over a blurred copy of itself (contain); every point tied to the picture takes its screen place from the same fit | `df-playground/src/plate_fit.rs` (`PlateFit`, `data-fit` read by `theme.css`), `image_layout.rs`; tests `plate_fit_tests.rs` | this table |
| Forge backend runtime settings | `media backend [codex-cli \| forge [url=] [project=] [size=WxH] [steps=N]]`; size and steps apply to every Forge picture until the next `media backend` call | `imagegen/{cli,backend,limits}.rs`; the studio client `df-providers/src/forge.rs` | "Forge as the model provider", "Image vendor limits" |

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

This is the general recipe, in the bubbles lane; the roam builds in the `roam` lane (see "Free
roam: the current run procedure"). Use one shared target directory, never `--target-dir` per
attempt. Stop the host first: it
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
(`inspected_item`, an id from the items offered at the active place in the world bible and the
things in sight there, a designed place's included (see "Free roam"), checked in code; null when
nobody is) and whether that close look begins now (`new_look`, the judge's
call, told the item looked at before). A new inspection cuts at once to `inspect_shot` (`insert` in `frames.json`: the item fills the screen edge to edge),
whatever the gate and inside the hold, so the players at the table can see the item's quality.
The insert is code's to take for a new inspection only: the judge is not offered it among the
shots it picks from (run 58 took it for a gated frame, and the room later came back to a
close-up).
An insert is painted anew from the item's look, never an edit of the room plate, and with no
character sheet (`insert_request` in `persona/plate.rs`; no `base`, `attached=-`): roam run 2 made
every insert an edit of the room, whose edit rule keeps the people's faces, so three of six were
the wide shot with the person still standing in it. Its prompt asks for the item alone filling the
picture edge to edge, with no person, border, bar or plain strip (`INSERT_FRAMING`); it used to ask
for the lower third left plain for the caption card, and the painter drew that as a flat beige band
across the bottom quarter of every insert's picture. It is asked in the shape the backend makes (the
`media backend ... size=` override: `aspect=1:1` for 512x512, not 16:9), and a picture the backend
delivers in another shape is said so in the audit log (`IMAGEGEN reshaped job=.. asked=WxH
delivered=WxH`, before its `stored` line); the `requested` line now carries `size=WxH`.
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
the host logs `offered=<n> inspected=<id|-> inspection=true|false` on each `frame_judged` line
(`offered` is how many items the judge was given at that moment: 0 means the active place has
none, not that the judge declined). A look at an item other than the one before (`last_inspected`)
always begins a look, whatever `new_look` the judge says (a first live roam saw it call a first
look one that "goes on"); `new_look` only matters for the same item again.
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
line|schedule|inspection|focus|declared at_ms=.. took_ms=..` line on the pages' `TIMING` clock. An
input line `@declare <id> | <json>` is a character declaring what they are doing right now (see
"Declared actions and emergency plates").
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
   toolbar (or start the runner with `DF_RUN_TTS=on`). The click is what unlocks the page's audio.
   Check with `$D dev --port 7879 playground`: it should show `tts: true` and `audio unlocked`.
   The host remembers the switch across restarts and a first run has it off; nothing but the
   owner or that flag turns it on. The runner asks for every line with `speak=true`, and the
   switch decides: with TTS off a line is shown as text only (`SPEECH line_unvoiced
   error=disabled`) and nothing is sent to the supplier.
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

## Free roam

How to run it is "Free roam: the current run procedure", near the top; this section is what the
parts do.

One lone traveller, Danny, a painter (`market-threads/danny.persona.json`), wanders through 20
places (`market-threads/free-roam.beats`): towns reached by the road and spots within each town.
Nobody answers him. Each move is his own line toward a loose goal, read by Jev as a move
(`move act`; see "How Jev reads a roam line" below). The destination is designed and painted, and its plate replaces the backdrop. The
designed place becomes the world's active place, with the things in sight there (`things` in
the design, each with an id, a name and a look). There he picks out one thing he would like to
paint (`persona say`). The frame judge names it as `inspected_item`, the insert cuts in, the room
comes back after `insert_hold`, he sets up and paints it with what he carries (`persona say`
toward `@roam_paint`, or declares what his hands are doing with `@roam_declare`, which shows him
mid-painting in an emergency plate; see "Declared actions and emergency plates" and the inventory
below), and he moves on. What counts as looking closely at
a thing in sight is in the judge's instruction: whoever steps up to it and says what they see of
its material, colour and wear, even alone, as opposed to passing, naming or mentioning it. The
first live roam (2026-11-09) cut to no insert on 19 lines of descriptive monologue
(`inspected=-`) while the place offered its things each time (`WORLD arrived ... things=1`): the
instruction knew only looks done to an item with someone to show it to, so `@roam_look` now has
him say that he steps up to the thing and what he sees up close. A thing in sight has no keeper: its look is in the
judge's facts, and its first close-up is kept as its reference sheet (`POST /dev/world/picture`),
so a second look shows the same picture. The roam makes no scheduled room plates (`every=0`) and
no line-gated ones (`gate=100`): the move plates and the inserts are the pictures. Nothing in the
engine knows of roaming or painting; Danny, his goal and the cadence are data.

**How Jev reads a roam line.** `move act` has the character say a line toward a goal of going
somewhere, so it tells Jev the line was meant as a move (`declared: move` on `POST
/dev/move/interpret`; every other caller leaves it out and it is `unknown`). A destination is
anything to go to or toward: a town, a road, a room, a thing or opening in sight, a direction, with
or without a verb of motion, so a bare place phrase ("the next town along the road") is a move to
that place. For a declared move Jev picks the best destination the line and the situation support
and answers `needs_clarification` only for a line that names nothing to go to or toward. The answer
says what was unclear (`unclear`: `destination` or `other`) and why (`reason`, Jev's words; data,
never read by code). If the first answer finds no destination in a declared move, the walk reads
the line once more with that answer as `refused_before` and no more. The runner passes nothing:
`move act` declares. In the output:

- the trace has a second `TRANSITION id=1 stage=interpreting at_ms=<end of the first reading>
  detail=retry reason=the movement names no destination: <reason>` record, and the `designing`
  record's `at_ms` (the runner's `jev_ms`) covers both readings;
- the host log has one `MOVE interpreted reading=.. unclear=.. declared=move second_reading=false|true
  duration_ms=..` line for each reading;
- a line that still names nothing prints `not moved: the movement names no destination: <reason>`
  (the runner's `read:`), so the reason is there to read.

**What he holds: the standard inventory API.** Danny's art supplies are one owner of the dev
host's inventory (`market-threads/danny.inventory.json`: easel, satchel, boards, sketchbook pages,
charcoal, brushes, pigments, linseed oil, rags and silver pennies). The inventory is not his:
it is the host's standard for any number of owners (`crates/df-table-host/src/world/inventory.rs`).

- **An owner** is any character (player or NPC), a party's stash, a container or a shop. Only its
  `kind` (`character`, `party`, `container`, `shop`) tells them apart. An owner id and an item id are
  lowercase letters, digits, `_` or `-` (the world bible's id grammar; an item id may be a bible
  item's). An item is `{id, name, look, use, count}`: a stable id, its name, how it looks (its sheet
  in words), how it behaves when used and a count, a whole number from zero. An item at zero stays
  listed. `use` is `consumed` (the default; using it uses it up: a pigment, a coin spent) or `kept`
  (a tool or possession; using it does not use it up). Code enforces it: a `consume` of a kept item
  is refused (`kept_item`), and a kept item's count falls only through a `remove` (it wore out,
  broke, was lost or was given away).
- **Storage** is `world/inventory/<owner id>.json` for each owner (`{"schema": 1, "owner", "kind",
  "items"}`) and `world/inventory/index.json` (every owner with its kind, item count and unit
  count). Files are read again on every call and written whole or not at all; one owner's change
  rewrites only its file and the index, so the cost does not grow with the number of owners. A change
  of two owners (a transfer) writes both, and puts the first back when the second cannot be written.
  There is no reader for the older `world/carried.json`: seed the owners again and delete it.
- **Routes** (dev routes, one reply envelope, `outcome` `done` or `rejected` with a typed `error`):

  | Route | Body or query | Answers |
  | --- | --- | --- |
  | `GET /dev/world/inventory` | | every owner: `owner`, `kind`, `items`, `units` |
  | `GET /dev/world/inventory/owner` | `?owner=` | `inventory` and `ledger` |
  | `POST /dev/world/inventory/set` | `{owner, kind, items}` | seeds or replaces a whole inventory |
  | `POST /dev/world/inventory/add` | `{owner, item, count, name?, look?, use?, why?}` | `name`, `look` and `use` only for an item not held |
  | `POST /dev/world/inventory/remove` | `{owner, item, count, why?}` | the item leaves the owner |
  | `POST /dev/world/inventory/consume` | `{owner, item, count, why?}` | the item is used up |
  | `POST /dev/world/inventory/batch` | `{owner, changes: [{op: add\|remove\|consume, item, count, ..}]}` | all changes or none |
  | `POST /dev/world/inventory/transfer` | `{from, to, item, count, why?}` | both owners change or neither |
  | `GET /dev/world/inventory/history` | `?owner=&limit=` | the host's change lines, oldest first |

  A change answers `{applied: [{owner, item, kind, before, after, why}], owners: [{inventory,
  ledger}]}` (a transfer lists the giver first). A transfer gives the receiver the item with the
  giver's words when it did not hold it.
- **Errors** are typed, with a stable `error` key, and never change anything: `invalid_body`,
  `invalid_query`, `invalid_owner`, `invalid_item`, `invalid_schema`, `unknown_owner`,
  `unknown_item`, `duplicate_item` (an item twice in a list or a batch), `missing_words` (a new
  item needs a name and a look), `empty_batch`, `zero_count`, `insufficient` (the count would go
  below zero), `kept_item` (a kept item cannot be used up on the explicit route), `invalid_reason`,
  `unsourced_gain`, `unreasoned_loss`
  (judged lines only, below), `overflow`, `same_owner` (a transfer to the owner it is from), `store_unreadable`,
  `store_invalid`, `store_unwritable`, `stopped`. Each refusal is one `WORLD refused
  verb=inventory_<verb> error=<key>` line.
- **Audit lines** are ids and counts only (a reason is words): `WORLD inventory_changed owner=..
  item=.. kind=add|remove|consume|transfer_out|transfer_in before=.. after=..` for each item that
  moved, and `WORLD inventory_set owner=.. kind=.. items=..`. The history route reads these back from
  the host's log (the log is bounded, so it is the recent history).
- **Verbs** (`$D dev --port P world inventory ...`; a name, look or reason is one quoted word):

  ```bash
  $D dev --port 7879 world inventory                              # list the owners
  $D dev --port 7879 world inventory set danny character development/market-threads/danny.inventory.json
  echo '[{"id":"bread","name":"loaves","look":"round brown loaves","count":6}]' > bread.json
  $D dev --port 7879 world inventory set baker shop bread.json    # a second owner
  $D dev --port 7879 world inventory get danny
  $D dev --port 7879 world inventory consume danny ochre 1 why="the sky"
  $D dev --port 7879 world inventory add danny brush 2 name="brush" look="splayed bristles" use=kept why=bought
  $D dev --port 7879 world inventory transfer baker danny bread 2 why="bought bread"
  $D dev --port 7879 world inventory batch danny changes.json     # [{"op":"consume","item":"ochre","count":1}, ..]
  $D dev --port 7879 world inventory history danny limit=10
  $D dev --port 7879 world inventory sheets danny                 # ask for the missing item sheets
  ```

  The transfer leaves `danny` with 2 bread, the item named and looking as the baker's, and
  `baker` with 4. `world inventory remove ...` takes the same words as `consume`.
- **Who changes it** is the judged line, the one path for every speaking character. The per-line
  call of any character (`persona say` and `persona ask`) is told what that character alone holds
  (`now.inventory`, each item with its count and its `use`, so it knows what is running low and
  which things are used up by use) and answers `inventory_changes`. Each entry is `{item, change,
  reason, why, name, look, use, source, paid_with, where, from, made_from, what_happened, to}`:
  `change` is below zero for what leaves and above for what is gained, and `reason` is one of a
  closed list (null for a loss is a loss with no reason):

  | `reason` | Direction | What the host does |
  | --- | --- | --- |
  | `gained` | gain | an `add` (bought, found, given, received) |
  | `used_up` | loss | a `consume`: refused for a kept item (`unreasoned_loss`) |
  | `sold` | loss | a `remove`, only with its payment: a gain by `exchange` in the same line names the item in `paid_with`, of at least that count |
  | `given_away` | loss | a `remove`, only with `to`: an id of a character present, not the owner |
  | `lost`, `broken`, `worn_out` | loss | a `remove`, only with a `what_happened` that is not blank (null when the line does not show it) |

  A reason that does not fit the direction is refused (`invalid_reason`). **A loss needs a reason
  that stands, as a gain needs a source.** Nothing leaves an inventory by itself: a loss with no
  reason, a kept item `used_up`, a sale with no payment in the line, a `lost`, `broken` or
  `worn_out` with nothing said of what happened, or a `given_away` to nobody present, is refused as
  `unreasoned_loss` and logged. The instruction says in neutral words that using a kept item (a
  tool, a possession) does not lower its count, so a kept item drops only for `sold`,
  `given_away`, `lost`, `broken` or `worn_out`, and only when the line shows it. A sale and its
  payment depend on each other: a sale whose payment is refused is refused, and so is the payment
  that leaned on it, so both apply or neither. Code checks that `what_happened` is not blank; it
  does not read it (no word matching), so the instruction asks for null when the line shows nothing.
  A gain of something the character does not hold yet proposes it: a new id (the id grammar), a
  `name`, a `look` and a `use` (`name`, `look` and `use` are null for an item already held); a new
  item with no name or look is refused (`missing_words`) and an id that is not an id
  `invalid_item`; no count of it is too large (only zero and overflow are refused). A new item the
  line brought in has its reference sheet asked for at once, by the same missing-sheet path pictures use (`persona say` and
  `persona ask` print `sheet item-<owner>-<item>.png: none yet, asked for`).

  **A gain needs a source.** Goods do not appear from nowhere. A gain says where it came from in
  a closed `source`, with the field that source needs, and code checks it:

  | `source` | Field | Code accepts it when |
  | --- | --- | --- |
  | `exchange` | `paid_with: [{item, count}]` | the same line has a loss entry for each item, of at least that much, applied in the same batch (selling a thing is that thing going down: list it) |
  | `found` | `where: <thing id>` | the id is a thing offered at the active place (`world/state.json` things, or a stall item) |
  | `given` | `from: <owner id>` | the giver is a character present in the scene and is not the owner |
  | `made` | `made_from: [{item, count}]` | the same line uses each input up (`used_up`), of at least that much |

  A gain with no source, or whose field does not hold up (an empty list, a count of zero, a payment
  not in the line, one coin claimed by two gains, a thing not offered, a giver not present), is
  refused as `unsourced_gain`, and so are the loss entries named by its `paid_with` or `made_from`:
  an exchange applies both sides or neither (a coin that was to pay for a refused gain does not go
  down). A loss entry that no gain names stands on its own. Operators using the explicit routes
  (`add`, `batch`) are not asked for a source. A `from` the character was shown as a handle is read
  back to its id in code, one that is not a person shown to them reads as none. The refusal is one
  `WORLD refused verb=inventory_judged error=unsourced_gain` line, which the roam runner already
  prints as `[inventory] refused`.

  **One line, one change per item.** The entries of one item in a line are netted into one signed
  change before the counts are checked: a coin that rises by a sale and falls by a purchase is the
  difference (equal amounts cancel and the coin is not changed), entries of one item in the same
  direction merge, and an item that is sold and listed in `paid_with` is one decrease. Each entry
  keeps its own reason, source and `paid_with` for the checks (a decrease paying for a gain counts
  gross, not net). The net must be possible: a coin that would end below zero is refused as
  `insufficient`, and so is everything that leaned on it (the purchase it paid for is
  `unsourced_gain`, the sale that its proceeds fed is `unreasoned_loss`), so a sale and a purchase
  apply together or not at all. Only the judged path nets: the explicit `batch` route is operator
  input and still refuses an item twice (`duplicate_item`).

  **Deals.** The judge instruction gives, in generic words, the two common shapes. A sale plus a
  purchase in one line is four entries: the sold item down (`sold`, `to` the buyer's id when
  named), the coins up (`gained`, `source: exchange`, `paid_with` the sold item and its count), the
  same coins down (`used_up`), and the goods up (`gained`, `source: exchange`, `paid_with` the
  coins and their count). A barter has no coins: the sold item down, and the goods up with
  `paid_with` the sold item. Every entry of a deal names the others, and `item` is always an id
  from `now.inventory`, matched to what the line says by meaning, never made up (except for a new
  item gained). When the model leaves the proceeds without their payment (run 7: `unsourced_gain`
  and `unreasoned_loss`, so only the purchase applied), code repairs it before refusing: a `sold`
  item that no gain by exchange names is paired with the one gain by exchange whose item pays for
  another gain by exchange of the same line (the coins that buy the goods), and the pairing is
  validated like any other. Two such gains, or none, repair nothing and the sale is refused as it
  stands. A `paid_with` that names a sold item with a larger count than the line sells of it (run 8:
  the price, 5, where the one painting's count, 1, belongs) is set to what the line sells, since the
  sale's own entry is what the counts are checked on; the item is listed as repaired. **A line is
  atomic.** When any entry of a judged line is refused, for any reason, none of its changes apply:
  a sale and its purchase never apply by halves (run 8 applied the purchase and kept the painting),
  nor do uses (run 9 move 16 spent the ochre and lamp black of a painting whose board, oil and rags
  were short). The refusals that caused it are the `WORLD refused` lines.

  **A refused answer is asked again.** The line call judges the answer's `inventory_changes`
  before it settles the line (`world::judged_miss`, the same judging as the application, changing
  nothing). When any entry is refused, the call asks the judge again with `now.retry_reason`
  holding every refusal so far as typed ids and counts only (`unsourced_gain boards`,
  `insufficient coins (holds 2, asked 3)`, and for each item the gains name as paid, the count
  they name together against the count that goes down in the line), until the entries stand or
  the text deadline (`TEXT_DEADLINE`) since the line began has passed, with no count of askings
  capped, as the design call's. Each asking is one `WORLD retry verb=inventory_judged
  attempt=<n> reason=<keys>` line, and the refused answer one `WORLD judged_entries` line. If the
  deadline passes first, the last answer is judged as always: the deal is refused whole and
  nothing of it applies. The other reasons to word a line again (secret, intent, repeat, price)
  still ask once. The instruction states that what is paid in a deal is conserved: the counts
  named for one item across the gains add up to no more than what goes down of it, so one payment
  that buys several goods is split between their gains (or each gain names its own payment). Code
  does not accept the same payment named in full by two gains: it cannot tell one payment buying
  both from the same coin claimed twice, so the asking again settles it. Whenever a judged line has a refused entry, or a repair, the host logs the model's
  structured answer: `WORLD judged_entries owner=<id> entries=<item:change:reason:source[:pay=..]
  [:make=..][:to=..][:from=..][:where=..]; ...> refused=<keys|-> repaired=<items|->`. Ids and
  numbers only (a word that is not an id is `?`, no reason or other free text), at most the first
  forty entries.

  Each entry is judged on its own, in order, against the character's list as it stands. One code
  refuses (an item the character does not hold, a kept item used up, a reason that does not fit, a
  net below zero) is one `WORLD refused verb=inventory_judged error=<key>` line and refuses the
  whole line, which is asked again (below); a line with no refused entry is taken as one atomic
  batch.
  `move act` prints each change under the line (`inventory: <item> <before> -> <after> (<why>)`). A
  character with no inventory has nothing to change.
- **The judge's facts stay bounded as owners grow.** A speaker's line sees only its own list. The
  frame judge sees only the people in the picture: for each, the items it holds at least one of, as
  `id`, `name` and `count` (no look). The design call sees only the movers' lists. No call is given
  every owner's list, so its size follows the scene, not the number of owners.

`GET /dev/world/inventory/owner?owner=danny` also answers `ledger`: the kit's list panel
(`df_playground::kit::ledger::Ledger`) for that owner, `kind: loot`, `framed: true`, no title, and
one row `{id, name, quantity: "x4"}` for each item the owner holds at least one of, in the order
held. The page is not wired to show it yet. The roam runner tells every move what Danny holds
now, so he can turn toward a town to buy supplies when he runs low (a purchase is judged the same
way: pennies down, pigment up). The roam's `@roam_paint` line (in the beats file, never in the
engine) is the beat that uses supplies: after the close look he says one line toward setting up and
painting what he studied with what he carries, or toward buying or trading for what is low, and the
per-line judge answers `inventory_changes` only for what that line uses up, spends or gains. A
change code refuses is `WORLD refused verb=inventory_judged error=<key>` and the runner prints it
as `[inventory] refused`. When a supply runs out is data: `free-roam.beats` has a `@way market` stop
(four of the twenty moves: sell a small painting for pennies, then buy or trade for what is low or
gone, judged like any other change), and `@roam_paint` says to buy instead of paint when a supply is
low. A beats file that wants more or fewer such stops changes its `@move` lines; a run that should
never run out changes the counts in its inventory file.

**Reference sheets.** Every picture attaches its sheets within the backend's picture budget
(Codex 5, Forge 8): first the people's sheets (`ref-<id>.png`), then the things the picture
shows. A move plate shows what the design says the movers hold in sight (`shown_items`, most
noticeable first, checked against what they hold). The design is told to list only what a camera
at that framing would plainly see (worn, in hand, strapped on, or set down beside them; never what
is packed in a bag, pouch or box) and to prefer few. Code then keeps the first of them that the
picture budget leaves after the people's sheets (and an edit's base), in the design's order, and
leaves the rest out: they are not described and no sheet is asked for them
(`items left out for want of room: 3 of 11 fit beside 1 people`). A room frame shows what the
frame judge says is in view (`items_in_view`, checked the same way and capped the same way). What
finds no room among the places' things, or has no sheet yet, is told in words. An inventory item's sheet is `item-<owner id>-<item id>.png` in the media library.
**The prompt never names anyone.** A character is painted from their look (a look sheet, else their
job text), as `A person (<look>) stands in the <depth> <side>, <doing>`, with ` with their <item
names> in view` for what the design shows them with. The display name and the id stay out of the
picture prompt, the design call's facts and the design's instruction forbids them in its texts:
the image model paints any word it is given (the first roam's plate 1 had a signboard reading
`Danny traveling`).
**What a job followed.** Every image job that has a base picture, references or left-out pictures
writes `IMAGEGEN references job=<n> name=<picture> base=<name|-> attached=<names> count=<n>
maximum=<budget> left_out=<names|->` after its `requested` line; `left_out` is what the caller
could not fit within the backend's budget (it sends them as `left_out` in the request, to be
written, never sent). The roam runner prints it per move as `[refs NN]`, with the design's
`items in sight`.
A picture that shows an item without one goes ahead, told in words, and asks the image service
for the sheet in the background (`sheet item-danny-easel.png: none yet, asked for` in the move's
report). An insert of a thing in sight is drawn from its kept close-up once it has one.

### Making the starting pictures

Only when the scratch playground lacks `roam-00.png`, `ref-danny.png` or the `item-danny-*.png`
sheets (they are gitignored). The run itself is "Free roam: the current run procedure", near the
top. With the host up and the Forge backend set (steps 6 and 10 there), Danny's persona, when its
file changed:

```bash
$D dev --port 7879 persona write danny development/market-threads/danny.persona.json
```

His reference sheet (front close-up, three-quarter and full-length views side by side) and the
sheets of what he carries (each an image job; the run does not wait for them):

```bash
$D dev --port 7879 media generate "A lean man of about thirty with a short sandy-brown beard \
and a sun-browned, freckled face, in a wide-brimmed straw hat, a paint-flecked oatmeal linen \
smock over brown wool trousers and scuffed knee boots, a worn leather satchel at his hip and a \
folding wooden easel strapped across his back. The same man as a front close-up, a three-quarter \
view and a full-length view side by side on a plain mid-grey ground." \
  as=ref-danny purpose=portrait aspect=16:9 wait=true
```

```bash
$D dev --port 7879 world inventory sheets danny
```

`media generate status` shows the `item-danny-*` jobs until they are stored. The starting plate,
Danny alone on a road outside a town, located and named:

```bash
$D dev --port 7879 media generate "A lean painter in a straw hat with a folding easel on his \
back walks alone along a dirt road between fields toward a small town, morning." \
  as=roam-00 purpose=plate refs=ref-danny.png wait=true
```

```bash
$D dev --port 7879 media locate roam-00.png masks=1
```

```bash
$D dev --port 7879 media locate show roam-00.png
```

`show` prints his region number (usually 1); name it:

```bash
$D dev --port 7879 media locate label roam-00.png 1 danny Danny
```

Each place's frame watch transcript is kept in `artifacts/tmp/roam-live/frames<N>-<plate>.log`. The
log lines of a run are explained under "What each log line means" in the run procedure.

## Memories

The dev host's standard memory API keeps what any owner remembers as records with pictures, for any
number of owners and records (`crates/df-table-host/src/world/memory.rs`). It knows no story: a scene
visited, a picture painted and a note with a sketch are all a memory, told apart only by their
`kind`, a string in the id grammar chosen by the story or the runner (`scene`, `artwork`, `note`,
..). A memory is a record that something happened and how it looked: it is never a rule and never
the canonical truth of the world, and a summary or an index of it does not replace it
(`planning/long-horizon-state.md`).

- **A memory** is `{id, owner, kind, title, description, place, seq, pictures, links, tags,
  created_by}`. `owner` is an inventory owner's id (the same id grammar: lowercase letters, digits,
  `_` or `-`); the owner needs no inventory. `id` is unique within its owner. `description` is the
  look in words, so the picture can be redrawn or told without its file. `place` is an optional place
  id. `seq` is assigned by code, per owner, only ever up (the order things happened; a delete never
  gives its number again; a replace keeps its own). `pictures` are media-library names, the first is
  the primary, and every one must be in the library when the memory is put. `links` are typed
  `{what: item|memory|place, id}`; a `memory` link must name a memory the same owner keeps. `tags` are
  ids. `created_by` is optional free text for the judge or runner that made it. Deleting a memory
  never deletes a picture.
- **Storage** is `world/memory/owners/<owner id>.json` for each owner (`{"schema": 1, "owner",
  "next_seq", "galleries", "memories"}`), `world/memory/index.json` (every owner with its memory
  count, the count of each kind and the next `seq`) and `world/memory/galleries/<name>.json` (the
  members of each gallery sheet). Files are read again on every call and written whole or not at
  all; a change rewrites only its owner's file and the index, so the cost of a call does not grow
  with the number of owners.
- **Routes** (dev routes, the inventory's envelope: `outcome` `done` or `rejected` with a typed
  `error`):

  | Route | Body or query | Answers |
  | --- | --- | --- |
  | `GET /dev/world/memory` | | every owner: `owner`, `memories`, `next_seq`, `kinds` |
  | `GET /dev/world/memory/list` | `?owner=&kind=&place=&tag=&limit=&after=` | `memories` (oldest first, by `seq`), `next` (opaque cursor, when there are more) and `ledger` (the kit's list panel: a row per memory with `name` the title and `picture` the primary) |
  | `GET /dev/world/memory/get` | `?owner=&id=` | the memory |
  | `POST /dev/world/memory/put` | a memory without `seq` | `{memory, created}`: creates, or replaces the owner's memory of that id |
  | `POST /dev/world/memory/capture` | `{owner, kind, picture, title, id?, description?, place?, links?, tags?, created_by?}` | `{memory, created}`: a memory of one existing picture; the id is `<kind>-<seq>` and the description is the title when not given; an id that is kept is `duplicate_id` |
  | `POST /dev/world/memory/delete` | `{owner, id}` | `{memory}` that left |
  | `POST /dev/world/memory/references` | `{owner, filter?, ids?, budget}` | `pictures` (primary pictures that fit `budget`), `memories` (their ids) and `left_out`; newest first for a filter, in the order given for `ids` |
  | `POST /dev/world/memory/compose` | `{owner, filter?, ids?, name?, columns?, tile_width_px?, tile_height_px?, gap_px?, background?}` | the gallery sheet: `{picture, members, pictures, columns, rows}` |
  | `GET /dev/world/memory/inspect` | `?picture=` | `{picture, blank, luminance_deviation, edge_energy}`: whether the middle of a library picture is near blank (`df_imagery::picture_check`: a flat board with a pale edge) |
  | `GET /dev/world/memory/history` | `?owner=&limit=` | the host's memory lines, oldest first |

  `filter` is `{kind?, place?, tag?}` (all that are given must match; none matches every memory).
  A selection is `ids` or a `filter`, not both.
- **The gallery sheet** (`compose`) is how a collection too large to attach one by one becomes one
  reference: the image budget is 8 pictures on Forge and 5 on Codex. The primary pictures of the
  selected memories, in `seq` order, are tiled into one PNG (`df_imagery::gallery`): every picture is
  scaled to fit its tile whole and centred, tiles fill left to right then down, the columns derive
  from the count so the sheet is roughly square. A tile has the shape of the median member
  picture (square paintings in square tiles, wide ones in 16:9 tiles) with its long side 448 px, a
  12 px gap, mid-grey, unless the request gives both `tile_width_px` and `tile_height_px` (one
  alone is `gallery`); nothing is drawn on it, no label and no text. The sheet is
  stored in the media library as a normal picture named `name`, or `gallery-<owner>-<n>` with `n`
  the owner's next gallery number; a memory with no picture is left out of it. Attach the returned
  `picture` as one reference.
- **Errors** are typed, with a stable `error` key, and never change anything: `invalid_body`,
  `invalid_query`, `invalid_owner`, `invalid_id`, `invalid_kind`, `invalid_place`, `invalid_tag`,
  `invalid_link`, `invalid_picture`, `invalid_name`, `invalid_cursor`, `invalid_schema`,
  `invalid_sequence`, `unknown_owner`, `unknown_memory`, `unknown_picture` (not in the media
  library), `owner_mismatch`, `duplicate_id`, `duplicate_picture`, `duplicate_tag`, `duplicate_link`,
  `duplicate_selected`, `conflicting_selection`, `empty_title`, `empty_description`, `too_long`,
  `too_many`, `nothing_to_compose`, `gallery` (the sheet cannot be laid out; its `cause` says why),
  `picture_unreadable`, `picture_undecodable`, `picture_not_stored`, `store_unreadable`, `store_invalid`,
  `store_unwritable`, `stopped`. Each refusal is one `WORLD refused verb=memory_<verb> error=<key>`
  line.
- **Audit lines** are ids and counts only (a title and a description are words): `WORLD memory_put
  owner=.. id=.. kind=.. place=.. seq=.. pictures=<n> created=true|false` (a put and a capture),
  `WORLD memory_deleted owner=.. id=.. kind=..` and `WORLD memory_composed owner=.. picture=..
  members=<n> columns=.. rows=..`. The history route reads these back from the host's log (the log is
  bounded, so it is the recent history).
- **Verbs** (`$D dev --port P world memory ...`; a title or description is one quoted word, `tag=`,
  `id=` and `link=` may be repeated):

  ```bash
  $D dev --port 7879 world memory                                   # the owners and what they keep
  $D dev --port 7879 world memory capture danny scene roam-start.png "the road out of town" place=road tag=morning
  $D dev --port 7879 world memory capture mira scene roam-start.png "the same road"   # a second owner
  $D dev --port 7879 world memory put memory.json                    # a whole memory (no seq)
  $D dev --port 7879 world memory get danny scene-1
  $D dev --port 7879 world memory list danny kind=scene limit=20     # then after=<next> for the next page
  $D dev --port 7879 world memory references danny 5 kind=scene      # up to 5 primary pictures, newest first
  $D dev --port 7879 world memory compose danny kind=artwork         # -> gallery-danny-1.png
  $D dev --port 7879 world memory delete danny scene-1               # its picture stays in the library
  $D dev --port 7879 world memory inspect item-danny-painting_01.png   # blank=true for a blank board
  $D dev --port 7879 world memory history danny limit=10
  ```

  Two owners keep separate ids and separate sequences: both captures above are `scene-1` with `seq`
  1, and `world memory` lists `danny  1 memories (scene 1), next seq 2` and `mira  1 memories (scene
  1), next seq 2`. A `list` answers 100 memories unless `limit=` says (1 to 10000).

**In the roam** the memories are data: `free-roam.beats` names the kinds. Every plate that goes live
is captured as a memory of kind `scene` (`@roam_scene_kind`): its title is the design's place name,
its `place` the designed place's id (the plate's name) and its description what the plate shows, so
`world memory list danny kind=scene` is every place he visited. Each painting is made when the line
toward `@roam_paint` (or the declaration) is said, and only if the board became the painting: first a
line toward `@roam_painted` is said (`[painted NN]`) and judged like any line against what he really
holds, so the board he painted on, the pigments, the oil and the rags are used up (the brushes and
easel are kept) and whatever he lacks is not consumed. If that line used up no board
(`@roam_art_board`) the study is not made and the log says `[memory NN] not painted: no boards was
used up (had .., have ..)`. The move's plate is then edited into a study of the place, no figures
painted in (`media generate .. base=<plate>`, the plate its only reference and the board told in
words: given the board's own sheet as a reference the model often painted just the board; the
look statement `@roam_art_look`, no house look because the house look avoids paintings). The
study is then inspected: if it is near blank it is made once more with `@roam_art_retry` added,
and if it is still blank it is removed and not kept (`[memory NN] study blank, not stored`; the
board is used up either way). A study that stands is stored as `item-danny-painting_NN.png` (so it is the
item's reference sheet too), kept as a memory of kind `artwork` (id `painting_NN`, linked to the
item `painting_NN` and the place) and added to what he holds as a kept item with the same id. That
is the inventory's add operation with name and look from the memory, not the judged gain of a new
item. The day ends with `@roam_show`: a move to the centre of town, then `world memory compose
danny kind=artwork` makes one gallery sheet, the show plate is the move's plate edited after that
sheet and his character sheet (`@roam_show_look`), located (the scan's first region, numbered from 0,
is named for him with `media locate label`) and put behind the scene, and he says a
line toward `@roam_show_say`. The log has one `[memory NN]` line for each scene captured and each
painting kept, and `[show]` lines for the sheet (`composed gallery of N pieces -> <name>`), the show
plate and his line. A kind of move with a `@way_arrival <way> <situation>` line (the market stops of
`free-roam.beats`) has its own arrival step instead of the declaration and the study: after the look he
says one line toward the situation (`[trade NN] <line>`), the per-line judge applies the sale of a
painting (`given_away`, pennies up) and the purchases to what he really holds, and the log adds
`[trade NN] changes: ..; gained: ..` or `gained: no gain recorded`. The declaration's situation says
that setting a thing up does not use it up, so only the finishing line uses the board.
Pacing is data too (`@roam_dwell`, `@roam_reveal`, `@roam_show_dwell`, `@roam_show_each`, `@refs max=3`): a
new plate stays up for the dwell while he perceives it (`[dwell NN]`), each painting made is shown full
frame with its title and then the view it was painted from (`[reveal NN]`, `@show <picture> | <caption>
| <seconds>` to the frame watch, `[pace NN] painting <picture> up 8024 of 10000 ms, view <picture> up 7988 of 6000 ms`: the picture
from the host's `PERSONA revealed picture=..` line, the time up from the `took_ms` of the
`frame_timing moment=ended .. reason=reveal` line (matched by order in the move), and the hold asked
for, so a short or stretched hold shows), and the show steps through the last paintings after the show plate and ends
on it (`[show] piece ..`, `[pace show] ..`). A study follows its plate alone and the show plate its
base, the gallery sheet and his sheet: no more than `@refs max=`.
The study is made in the background once the finishing line has shown a board used up, while the
declared plate is on screen; its lines (`[painted NN]`, `[memory NN] ..`) are held in files and printed
in the usual order when the declared plate has ended, and the memory, the item and the reveal follow
(the `~ item` line now comes before `[reveal NN]`). A study that does not end within three hung-bounds
is stopped, its file removed and `[memory NN] study abandoned after N s` printed. The runner polls the
host's events once a second. `[time NN] move took <s> s (plate .., look .., declare .., study ..,
reveal ..)` closes each move: `study` is only what remained to wait for after the declaration.
The page does not show the memories yet: `list` answers the rows of the kit's
list panel (`ledger`, a row per memory with its title and primary picture), the same shape as the
inventory's.

## Perception

The players and the characters of a scene can all see what is on screen. The perception API
(`crates/df-table-host/src/persona/perceive*`, standard and story-neutral) feeds the picture that
is shown, with what the scan knows of it, and one character's own state through the language
model, which tells what that character sees and what is on their mind. Nothing in it knows a
story, a place or a painter; who looks, at what and how often is the caller's data (the roam's
`@roam_perceive on`).

**The picture really goes to the model.** The text request carries pictures
(`df_ai::text::TextRequest.images`) and both text backends send them: Responses as an
`input_image` (`detail: low`, set in `text/responses_text.rs`), Codex as an `--image` argument. So
`vision: true` is the default and the model is given the picture on screen itself. `vision: false`
(asked for with `vision=false`, or the picture is not a readable library picture) asks from the
scan's layout and the scene's words alone and the answer says `vision: false`; it is never
presented as sight. (A raise of the Responses `detail` is a one-word change in `responses_text.rs`
and an owner's call on cost.)

| Route | Verb | Body or query | Answers |
| --- | --- | --- | --- |
| `POST /dev/persona/perceive` | `persona perceive <who> [mode=glance\|study] [picture=<name>] [focus=<id>] [recent=<text>]... [scene=<words>] [vision=true\|false] [model=<name>]` | `{who, mode?, picture?, focus?, recent?, scene?, vision?, model?}` | `Perceived` |
| `POST /dev/persona/perceive/scene` | `persona perceive scene [who=<id>,<id>..] [mode=] [picture=] [recent=]... [scene=] [vision=] [parallel=<1-8>] [model=]` | `{who?: [ids], mode?, picture?, recent?, scene?, vision?, parallel?, model?}` | `{picture, vision, results: [{who, outcome: perceived\|refused}]}` |
| `GET /dev/persona/thoughts` | `persona thoughts <who> [limit=<n>]` | `?who=&limit=` | `{who, thoughts: [{id, seq, place, picture, text}]}`, oldest first |

- **Input.** `who` is a persona id. `picture` defaults to the picture on screen now (the shown
  backdrop; a close-up insert or a room shown with the previous room's cast has no scan that fits
  it, so it is offered with no regions); a name is a media library picture. `focus` is a region id
  of the scan or an item id of the place or of what the character carries that they already
  attend to. `recent` is the last lines of the scene, `<speaker id>: <line>`. `mode` is how
  closely they look: `glance` (default, the one to four things that catch the eye) or `study`
  (everything they would pick out). `scene` is the words the picture was made from, used only
  when the picture itself is not sent. The model is `model=` or the persona model.
- **What the model is given** (`facts.rs`): the picture; the scan's regions (id, number from the
  left, box in thousandths of the picture) each with who it is *to this character*: `you`, the id
  of someone whose name they have learned, or `person-N` with how they look for a stranger, and
  `someone` for a figure the scan did not name; the persona (own brief, private side included, since
  it is their own mind) and the others' public briefs under the same handles; their state (mood,
  emotions above zero, attention, standings); what they carry (id, name, count, use); the place
  and the things in sight there (with their looks; a stall item's keeper by what the character
  calls them); their last six thoughts (so the shape of any repetition shows); and the last lines. Names are learned, never given: a name
  the character has not learned (nor its id or display name) is not in the facts. A handle that comes
  back in an answer is put as how the person looks (`persona/handles.rs`), or the answer is refused
  (`handle_in_text`).
- **Bounds** (the facts only; never the answer): 48 regions (the character's own, then the surest;
  the rest as `regions_left_out`), 48 items in sight, 48 kinds carried, 30 lines of the scene
  (`recent_lines_left_out`), 6 earlier thoughts, 64 characters and at most 8 asked at once in a
  scene call (4 when it does not say).
- **The answer, checked.** `Perceived`: `who`, `mode`, `picture`, `vision`, `viewpoint`
  (`{from: pictured, region}` or `{from: viewer}` for a character who is not in the picture and sees
  what the screen shows a viewer; a character in the picture is told in the facts, as `now.you`,
  which region is them, and the instruction says the figure there is their own body, told in the
  first person; a sight at that region is `self` whatever subject the model gave),
  `regions` (how many the model was told of), `sees` (`what` in
  the character's terms, `region` a scan id, `item` an offered item or carried one, `salience` 0
  to 100), `thinking_about` (first person, their own voice, as long as it needs), `attention`
  (`{on: region, id}`, `{on: item, id}` or `{on: none}`), `feeling` (an emotion of
  `persona/emotions.json` and a change in points, which is taken into the state only through the
  line pipeline's state path, `PERSONA state_applied`), `wants`, `leak_suspect` (the model's report
  that the thought states the private side: such a thought is never to be shown), `thought` (the
  memory it was kept as) and `duration_ms`. Code checks every id and number; a refused answer
  changes nothing, is asked once more with the reason (`now.retry_reason`) and is refused for good
  if it fails again.
- **Errors** are typed (`{"error": "bad_request"|"answer_refused"|.., "cause": {"kind": ..}}`, the
  frame judge's envelope). `kind` is one of: `persona` (the character has no usable persona),
  `shown` (the scene shows no picture), `picture`, `unknown_focus`, `no_characters`,
  `too_many_characters`, `invalid_parallel`, `invalid_limit`, `thoughts`, `malformed`,
  `empty_thought`, `unknown_region`, `unknown_item`, `unknown_emotion`, `bad_attention`,
  `bad_salience`, `bad_new_thing`, `new_thing_for_person`, `handle_in_text`. In a scene call one character's refusal is their row's
  (`outcome: refused`) and stops no one; nothing of one character's answer is in another's facts.
- **Audit lines** hold ids and numbers, never a thought or a sight: `PERSONA perceived who=..
  vision=true|false regions=N attention=region:<id>|item:<id>|none sees=N feeling=<emotion:+n>|-
  viewpoint=pictured|viewer picture=<name>|- leak_suspect=.. retried=.. thought=<memory id>|-
  duration_ms=..`, `PERSONA perceived_scene characters=.. perceived=.. refused=.. parallel=..
  vision=.. picture=.. duration_ms=..`, `PERSONA refused verb=perceive|perceive_scene|thoughts
  [id=..] error=<key> cause=<kind>`, `PERSONA thing_unadded who=.. error=<key>`, `WORLD
  thing_added place=.. id=.. via=perceive`, `PERSONA thought_unkept` and `PERSONA
  thought_unreadable` when the memory store failed (the perception still answers).
- **What is seen becomes what a close-up can cut to** (`new_things`). A road or a lane often has
  no designed things, so the frame judge is offered nothing to cut to, though the character sees
  a wall. In the same call the character may propose, as things of the active place, what they
  notice that `items_here` does not list: each is one of their own `sees` entries that has no
  region (`see` is its index; its `item` is null or the new id itself, since the entry names the
  thing it proposes, and so may `attention`), with an `id` (the id grammar, not one the place or
  what they carry already has, nor another proposal's), a `name`, a `look` (what a camera brought
  up close would show, in neutral words) and `attended` (at most one, and only when `attention`
  names nothing else or that thing) and `kind` (`object`, the default, or `vista`). A camera can
  go up close to an object (a wall, a door, a trough, a gate, a stall) and not to a vista (a
  distant town, open fields, the sky, a road, a crowd): a vista is left out as `PERSONA
  thing_unadded error=vista`, with the rest of the answer kept. The model decides what strikes
  the character; there is no
  salience threshold or count in code. Proposals are an extra of the answer and never cost the
  sight and thought: each is checked on its own, and a bad one is left out while the rest is kept,
  with no retry. It is left out when its `sees` entry has a region (a person of the scan, never a
  thing: `new_thing_for_person`), names a different item, has a taken or malformed id, a missing
  `see`, blank words, or is a second attended thing (`bad_new_thing`); each leaves one `PERSONA
  thing_unadded who=.. error=<key>` line and a `new_things` entry with `added: false` and `why_not`
  (what to change), and an item or attention the answer gave that pointed at it is cleared. A
  proposal for an entry that already names that very id when the place has it is the thing that is
  there and is dropped silently. Only a bad core (a region, item or emotion that is not offered, a
  bad attention or salience, an empty thought) is refused and asked once more with the reason. An
  accepted proposal is written to the active place by `world::add_things` (the designed-place path:
  `check_place` as `POST /dev/world/arrive` checks, all or none, only for a place designed in play,
  never a bible place), so the next `place_items` offers it to the frame judge: the `sees` entry
  now names it as its `item` (and the attention when it was attended), `Perceived.new_things` lists
  `{id, name, look, added, why_not?}`, and the log has `WORLD thing_added place=.. id=.. via=perceive`. A
  place that cannot take it (none active, or a bible place) leaves everything as it was, reports
  `added: false` and writes `PERSONA thing_unadded`. The runner calls `perceive` before the look
  line, so a thing the character attends to and then looks at closely cuts to an insert as usual.
- **Thoughts are kept** as memories of the standard memory API (`world memory`): kind `thought`,
  owner the character, title `Looking at <place name>`, description the thought and then
  `Saw: ..` and `Wanted: ..`, picture the picture looked at, place the world's active place, links
  the attended item and the place, tags `perceived`, the mode and `private` for a leak suspect.
  `persona thoughts danny` and `world memory list danny kind=thought` read them back.
- **The words the runner reads** from `persona perceive`: a header line (`danny perceives
  (mode=.. vision=.. picture=.. regions=.. viewpoint=.. attention=.. duration_ms=..)`), then one
  `see: <what> | region=<id|-> item=<id|-> salience=NN` line for each thing seen, one `thing <id>
  (<name>): <look>` line (the shape of the move's design lines) for each new thing that is now a
  thing of the place, `thought: <text>`, `attention: region:<id>|item:<id>|none`, and `feeling:`,
  `wants:`, `leak_suspect:` and `kept:` when there are any. Line breaks inside a thought are folded
  to spaces. The runner adds the `thing` lines to the move's own, so a close look at a thing the
  character noticed has its name and look (run 9 printed `item=wooden_door (?)`: the lines were
  not in the output, and the runner looked the name up among the design's things only).

```bash
$D dev --port 7879 persona perceive danny recent="danny: The road bends here."   # the screen now
$D dev --port 7879 persona perceive danny mode=study focus=well vision=true
$D dev --port 7879 persona perceive scene parallel=4        # everyone in the cast with a persona
$D dev --port 7879 persona perceive scene who=danny,wren recent="wren: Morning."
$D dev --port 7879 persona thoughts danny limit=5
```

**Sights are the place and the others, never the character or what they carry.** Runs 6 and 7 had
Danny list his own dark coat, and a painting in his hand, as sights, so attention went to himself.
The instruction now says the character does not list themselves, what they wear or what they carry
(known from their own facts). Each `sees` entry has a closed `subject` (`place`, `person`, `self`,
`carried`; `Seen.subject`, default `place`), chosen by the model. Code, with no word matching:
attention on the character's own region, on a carried item or on an entry marked `self`/`carried`
is cleared to nothing (`PERSONA attention_cleared who=.. why=self|carried`); such entries are not
kept as what was seen in the thought, never become new things (dropped as `bad_new_thing`), and an
entry marked `person` is never a thing either (`new_thing_for_person`). When the attention is then
nothing, the instruction asks the character to choose what in the place draws them most, and code
backs it up: the `place` sight that names an offered, not-carried item with the highest salience (the
first of equals), else the strongest thing just added, becomes the attention, so it is rarely empty.
The runner's `[see NN]` line leaves out `self` and `carried` entries (the `see:` lines end with
`subject=..`).

**What a speaker attends to reaches the frame judge.** The frame judge used to guess from the
look line alone, and in run 5 an offered wall ("I'll step up to that wall") was not cut to
(`offered=3 inspected=-`). Now `judge` adds `attending: [{speaker, item}]` to its facts
(`perceive::attending`): for each speaker of the call, the item their newest kept thought
attended to, only when that thought was had at the place that is active now and the item is still
one the judge is offered. It expires when the place changes, and a newer perception that attended
to nothing (or to a person) ends it. `FRAME_INSTRUCTION` tells the judge that a line of that
speaker that steps up to, goes closer to or describes the attended thing is a close look at it
(`inspected_item`); the judge still decides, and its id is still checked against the items. A
thought's `world memory` record links the attended item, and `GET /dev/persona/thoughts` returns
it as `attended`.

**In the roam.** With `@roam_perceive on` (data in `free-roam.beats`) the runner calls `persona
perceive danny` the moment a move's plate is live (in the background, before the move's own
bookkeeping: the scene memory, the tempo impulse, the frame watch start), waits out `@roam_dwell`
counted from then and the perception, and speaks the look line after it. It prints `[see NN] vision=true <what he sees>; <..> | attending: <the thing he attends to>` and
`[thought NN] <what he is thinking about>`. His thought and what he attends to are added to the
situation of the look and paint lines (` What is on your mind: .. You have your eye on: ..`), so
what he says is about what is really on screen. Off, the roam is as before.

**Showing a thought** (not built; a proposal). The TV's thinking cue is the three-dot pill
(`scene cue <character> think`). A thought could be shown as a bubble in the kit's speech style,
faded in and out and not moved, anchored with `plate_fit` at the character's head point
(`region.head_x/head_y`), kept off faces and the narration box. It needs the data `{character,
text, hold_ms}` on the scene; the text is the thought's first line, only when `leak_suspect` is
false.

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

The inventory panel (`view/inventory.rs`, scene slot `inventory`, hidden by default) is the
kit's loot list at the lower right above the caption bar, anchored to the stage, not the picture.
`world inventory show <owner>` shows that owner's inventory under their display name (`Satchel`
when they have no persona); `world inventory hide` takes it away. While it is shown, every
application of changes to that owner (the judged line, `world inventory add|remove|consume|batch|
transfer|set`, a trade, a painting added) rebuilds it from the stored inventory (host:
`world/inventory/screen.rs`; log lines `WORLD inventory_shown|inventory_followed|
inventory_hidden`, and the scene line `inventory rows=.. moves=..`). Rows run supplies first, then
kept things; a numbered family (ids ending `_<number>`, such as the finished paintings) of two or
more collapses into one plural row with the total (`Paintings x2`), listed first; past 16 rows the
lowest counts fold into `Other items` (a family or a row at zero never folds). The viewer sees USED
UP happen: a count that fell flashes the row red-amber with a `-N` tag, one that rose flashes green
with `+N`, each for 4 s from when the page first showed it (`inventory::FLASH_MS`, `FlashClock`,
`inv-flash-*` in `theme.css`). The host carries every move of the last 4 s, merged by row, so two
pushes close together (a finishing line's batch, then the painting added a moment later) both
flash; a redraw carries each flash on from where it was (`--inv-age`) and never restarts it. While
a full-frame picture is up (a reveal, an inspection or focus insert, a close-up backdrop, or the
day's show plate, which the runner puts up with `scene image tv-backdrop <name> plate=show`
(`PlateKind::Show`, otherwise a room plate with its own scan); not a walk, declared or room plate)
the panel fades out (`inventory-covered`, `picture_is_up`), moves
that arrive meanwhile start their flash when the panel fades back in (`inv-return`); a supply at 1 has an amber tint; a row at 0 stays listed, struck and dimmed, for as
long as the panel is shown (an item never held there is not listed). Like the story mark it fades
out (class `inventory-hidden`, 400 ms) while it would cover a face, a bubble, the narration or
caption bar, the clue board or the story chip, and never moves. The roam runner shows it at the
start (`play_roam`). Check it live: `dev --port 7879 world inventory show danny`, then
`world inventory consume danny boards 1` and watch the TV; `scene` lines and the page's timeline
(`clients timeline <client>`) show the redraw of the `tv-inventory` part.

## Toolbar settings

The page's toolbar rows and where each switch lives. The host holds all of them in the playground's
controls (`playground` prints them) and keeps them in `settings.json` in the playground directory
(the one `--playground` names; for the roam `artifacts/tmp/persona-live/playground/settings.json`),
written whole after each change of one of them and read once when the host starts. The page keeps
none of its own: it draws what the host holds, so a reload or a second page shows the same. A first
run with no file starts safe: TTS, STT and Music off, Sound on, volume 80, Unavailable, Grid and
SAM off. The start line `PLAYGROUND settings_restored source=defaults|file|defaults_after_error`
names what the host started with (`error=malformed|unreadable` when the file could not be taken,
which leaves every switch at its default), and each save writes `PLAYGROUND settings_saved ..` (or
`settings_unsaved error=..`, with the change answered as a failure).

| Toolbar control | Setting | `playground set` key | Spends or listens |
| --- | --- | --- | --- |
| Unavailable, Grid, SAM | what the page draws over the picture | `unavailable=`, `grid=`, `sam=` | no |
| Sound | every sound the page plays, muted when off | `sound=` | no |
| Music | the background music plays; a missing track is composed only while on | `music=` | composing a track (ElevenLabs) |
| Volume | master volume, 0 to 100 | `volume=` | no |
| TTS | lines are voiced (ElevenLabs text to speech) | `tts=` | yes |
| STT | spoken questions are taken (speech to text, microphone) | `stt=` | yes |
| Plate | the frame watch's schedule, read only | (set by `persona watch`) | no |

"Enable sound" / "Audio unlocked" is the browser's permission to play audio, not a switch: the
first click or key press on the page unlocks it, whatever it was on. The Sound switch mutes.

**The spend gates.** Each is checked at the one place the supplier is called, so no caller can
bypass it: TTS in `Speech::synthesize` (every spoken line, `speech speak`, `persona ... speak=true`),
STT in `Speech::transcribe`, Music in the music engine's compose step (`scene music generate` and
`tempo prepare` both reach it). A refusal is typed (`SpeechError::Disabled`, `MusicError::Disabled`),
nothing is sent, and it is logged: `SPEECH line_unvoiced character=.. error=disabled` (the line is
shown as text only), `SPEECH refused verb=.. error=disabled`, `MUSIC refused name=.. error=disabled`.
A track the library already holds is still found and set on the scene with Music off, and the page
plays it only while Music is on. Casting a character's voice lists the supplier's voices and asks
the text model; it is not synthesis and is not gated.

**Live click check** (the coordinator, in the browser pane, with the preview host; expected state
after each click, from the page and `$D dev --port 7879 playground`):

1. Reload: Overlays Unavailable, Grid, SAM unlit; Speech TTS and STT unlit; Sound lit, Music
   unlit; the Plate dot hollow while it reads `Plate: off`.
2. Click each of Unavailable, Grid, SAM, Music, TTS, STT: it lights at once, `playground` shows
   that key `true`; click it again: unlit, `false`. Click Sound: it goes unlit (`sound: false`) and
   the page is muted; again: lit.
3. Before any click the toolbar reads "Enable sound"; after the first click anywhere it reads
   "Audio unlocked", and that first click still toggled the switch it was on.
4. Drag Volume and let go: the loudness follows the drag, and on release `playground` shows the
   new `volume:`; reload the page: the slider is where it was left.
5. Turn TTS on, Music on and Volume to 35, restart the host, reload: all three are as left. Turn
   TTS off, restart, reload: it stays off, and `settings.json` and the `settings_restored
   source=file` line agree.
6. With TTS off a spoken line appears as text and the log shows `SPEECH line_unvoiced .. error=disabled`
   and no `SPEECH synthesized`; with Music off `tempo prepare` reports every profile `failed (disabled)`.

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

## The check judge and the DM's checks

Owner goal 2026-11-10 (`planning/dice-gauntlet.md`): a judge decides whether a player's action needs
a roll, or whether the narrator wants one called, and the dungeon master says the call aloud and
reads the result in plain words with flair. Code: `crates/df-table-host/src/persona/check/`
(`judge`, `rolling`, `numbers`, `lines`, `view`, `flow`, `routes`, `cli`, `sheet`). It uses the rules
owner's `df_rules::roll` and the host's `RulesCheckKit::roll` over `RecordedDice::from_system_entropy`
(real entropy; never seeded in the host), and the display's `SceneChange::Roll` / `ClearRoll`
(`RollView`, the roll card). Nothing in it reads words: what an action means is the judge's closed
answer, validated in code.

```
persona check judge <character> [sheet=<file>] [stations=<file>] [station=<id>] [action=<line>] \
  [moment=player|narrator] [place=<words>] [sight=<words>]... \
  [checked=<kind>[:<ability>[:<skill>]]]... [intent=<kind>[:<ability>[:<skill>]]] [model=<name>]
persona check run <character> <the same options> [inspiration=<die place>] [speak=true|false] \
  [told=<what the runner just said aloud>]
```

| Verb | Route | Body | Answers |
| --- | --- | --- | --- |
| `persona check judge` | `POST /dev/persona/check/judge` | `{character, sheet, moment, action?, station?, place?, in_sight?, checked?, model?}` | `{verdict: {initiator, reason, call?}, retried, duration_ms, designed?}` |
| `persona check run` | `POST /dev/persona/check/run` | `{judge: <the judge body>, speak?, inspiration?}` | `{judged, rolled?: {called_line, result_line, called_voice, result_voice, view, times}}` |

The command line reads the sheet (default `development/market-threads/<character>.sheet.json`) and,
with `station=`, the station (`stations=`, default `development/market-threads/gauntlet.stations.json`)
and sends them typed. `moment` is `player` when an `action=` is given and `narrator` when not; a
player moment must state its action and a narrator moment must not. `run` needs the character's
persona stored (its display name is kept out of the lines) and a shown scene (the dungeon master's
box). The judge is never told the station's designed test; the reply says whether it matches
(`designed: kind=.. match=true|false`). `checked=` lists tests already rolled at this place so the
judge does not call one twice (the host keeps no list: pass the ones a run printed).

**The narrator's intent.** `intent=<kind>[:<ability>[:<skill>]]` (for example
`intent=ability_check:dexterity:acrobatics`, the station's designed test) is legal only at a narrator
moment (an `action=` or `moment=player` with it is refused `intent_needs_narrator`). It reaches the
judge as the fact `narrator_intent`: the narrator has authored that test. The judge calls exactly
that kind (and the ability and skill when given) and still sets the band, the mode and its grounds;
or, when the test makes no sense for the situation, answers `check_required=false` with reason
`intent_unsuitable` (a typed refusal the runner logs; `already_checked` stays legal). Code refuses a
call of another test (`intent_ignored`), any other no-roll reason while an intent stands, and
`intent_unsuitable` when no intent was given (`no_intent_to_refuse`); each is asked again. A runner
passes the station's designed test as `intent=` at narrator stations; at player stations the judge
stays free and `designed: .. match=` is the check.

What each prints:

```
gauntlet_warrior check: required=true initiator=player reason=uncertain_with_consequence retried=0 duration_ms=4120
  call: kind=ability_check ability=strength skill=athletics dc=15 mode=normal grounds=none uses=-
  designed: kind=ability_check match=true
  called (text_only): The rock is cold under your hands. Athletics, Strength. Difficulty fifteen. Roll it.
  result (text_only): Fourteen on the die, plus three for Strength, plus two for proficiency. Nineteen against fifteen. A success. The ledge holds, and so do you.
  roll: Athletics (Strength) kind=skill_check total=19 target=DC 15 outcome=success natural=none dice=d20=14
  times: drawn_ms=.. called_ms=.. shown_ms=.. result_start_ms=.. result_end_ms=.. cleared_ms=.. hold_ms=.. min_hold_ms=..
```

(`judge` prints the first three lines; a judgement that calls no roll prints `required=false` and
`run` then says `no roll was called`; a die set aside by advantage or disadvantage shows as `d20=5x`.)

**Closed keys** (each an enum with `key()`, serde name equal to the key):

| Field | Keys |
| --- | --- |
| `kind` | `ability_check`, `saving_throw`, `attack_roll`, `initiative`, `death_save`, `passive`, `contested`, `damage`, `healing`, `table` |
| `initiator` | `player`, `narrator` (the moment's; any other is refused) |
| `reason`, a roll is called | `uncertain_with_consequence` (player), `narrator_hazard`, `narrator_obstacle`, `narrator_reaction` (narrator) |
| `reason`, none is called | `trivial_no_check`, `impossible_no_check`, `no_stakes`, `already_checked`, `intent_unsuitable` (the narrator's intended test makes no sense here) |
| `dc_band` | 5, 10, 15, 20, 25, 30 (SRD 5.2.1 page 6); an attack roll's band is the target's Armor Class |
| `mode` and `grounds` | `normal` with `none`; `advantage` with `character_feature`, `ally_help`, `favorable_circumstance`, `unaware_opponent`; `disadvantage` with `unfavorable_circumstance`, `armor_penalty` |
| `tone` of a flourish | `triumph`, `relief`, `wry`, `awe`, `neutral`, `dry_humor`, `setback`, `dread`, `rue`, `anticipation` (allowed ones by outcome in "The lines") |

**What code refuses in the judge's answer** (each a typed `CheckError`, asked again with every
refusal so far in `previous_refusal`, until an answer stands or the text deadline ends it): an
initiator that is not the moment's; a reason that does not fit `check_required` or the initiator; a
field the kind does not use, or one it needs (`ability_check` needs an ability or a skill and a
band; `saving_throw` an ability and a band, no skill; `attack_roll` a weapon of the sheet (`uses`)
and a band, its ability the weapon's; `initiative` only Dexterity, no band; `death_save` nothing;
`passive` only Wisdom (Perception), no band; `contested` athletics, acrobatics or perception and a
band; `damage` a weapon; `healing` a healing of the sheet; `table` nothing); an ability that is not
the skill's (SRD page 9); a band off the ladder; a mode with no ground, a ground that gives the
other mode, a ground the sheet cannot back (`character_feature` needs a feature, `armor_penalty`
equipment) or a mode on a roll with no d20; a test already in `checked`; `already_checked` with
nothing checked. Trivial, impossible and no-stakes actions are answered `check_required=false` and
nothing is rolled. The judge's instruction names an opponent (a living creature or enemy in the
situation or `in_sight` that can act, defend, dodge, take cover or answer back) and says a declared weapon
attack on one is uncertain, so it is an `attack_roll`, never `trivial_no_check` or `no_stakes`; only an
attack on a thing that cannot resist or respond, with no consequence for missing, may be `no_stakes`.
Contested rolls are the SRD's two plain tests (pages 183, 190): Athletics or
Acrobatics against an escape difficulty, Perception against a Stealth total.

**One check** (`persona check run`, `flow.rs`; times in ms from the start, logged as `CHECK ..`):

1. The judge answers. No roll: `CHECK skipped character=.. reason=..` and nothing else happens.
2. The dice are drawn from the recording dice and resolved by `df-rules` (nothing shows them) and
   the card is built. The call is worded first (one model call, about 6 s) and cued as soon as it
   stands; the result is worded after it exists, told the call as `said_before` so it repeats
   neither its image nor its opening, while the call is cued and heard out (each line has a
   budget, below). `CHECK read_len moment=check_called|check_result chars=.. est_s=..` records
   each line's length and its read time at 14 characters a second (target: a call about 10 s, a
   result about 12 s, the game's own words included). `RULES rolled` is the rules adapter's own
   line.
3. `CHECK called at_ms=.. kind=.. skill=.. dc=.. voice=played|failed|text_only`: the call is shown
   in the dungeon master's box (voiced when TTS is on), then the run waits for it to be heard out:
   3 s plus 70 ms a character.
4. `CHECK rolled at_ms=.. drawn_ms=.. dice=.. total=.. outcome=.. natural=..` and `CHECK shown
   at_ms=.. id=.. revision=.. min_hold_ms=..`: the card (`SceneChange::Roll`) goes up with
   `min_hold_ms` = 0.6 s (the page picks its tumble between 1.2 and 1.8 s) + the result line's
   heard-out time + 2 s. The page stages it: call 2 s, tumble, settle 0.7 s, then the sum.
5. At 4.5 s after the card (the latest the sum can be on screen) the result is cued:
   `CHECK result_read start_ms=.. end_ms=.. voice=..`; the run waits for it to be heard out.
6. The card is held until max(6 s, result line heard + 2 s) after the sum, plus its 0.4 s exit, then
   `ClearRoll`: `CHECK cleared at_ms=.. hold_ms=..` (`hold_ms` counts from 4.5 s after the card).
   A failure after the card is up still clears it.

With TTS off (the default) each line is shown as text only and the log says so
(`CHECK called .. voice=text_only`, `PERSONA narrated .. voiced=false`); `speak=false` does the same
with TTS on. Never turn TTS on for a check test without the owner. A voice that fails shows the
line as text (`voice=failed`, `SPEECH line_unvoiced`) and the run goes on.

**The lines** (`lines.rs`, `numbers.rs`): code composes the numeric slot from the resolved roll as
words and the storyteller words ONE short flourish beside it in one call: a call's `before` (at
most one short sentence of about twelve words, first; silent when the player has just acted, since
the numbers are the call; at a narrator station the place was already described aloud, so it is
the single sentence naming only the immediate trigger of the check, never silent) or a result's
`after` (one short sentence following the verdict, about the outcome and what it means for him,
never re-describing the scene or the call). It is told the outcome, whether a natural 1 or 20
happened (only where the rules give it an effect: attack rolls and death saves), how close the roll
was (`narrow`, `clear`, `wide`), no numbers, `room_chars` (about how many characters fit the target
read time beside the game's words; guidance in the instruction, never a cap in code), and its last
sixteen flourishes in full (`said_before`, the host's whole memory, never reset per station) and
their closing sentences (`closers_used`), kept in the host's memory across the run, the call of this
very roll last, so it varies shape and imagery and not only words: the run-1 flourishes ended again
and again on "the X keeps its counsel" and "the X has the last word", so the instruction forbids
those closers. The 17-station run (play3) reused one setting image across a scene (the ford's
cold spray at stations 6 to 11, the dusk wind at the death saves), so the instruction also says the
scene's stock details are used up by the earlier flourishes, forbids echoing any image, noun or
setting detail they used even reworded, and points the model at the outcome's effect on the body, a
sound, the opponent, the sound of the roll settling or an unnamed detail of what is in sight; the
fix is by prompt only, never by comparing the model's text. The flourish is one short sentence and names its one `tone`, a closed key, and code checks the tone
against how things stand, so a flourish cannot cheer a failure or mourn a success and no judge
call is made. The schema offers only the allowed tones; a tone outside them is refused
(`tone_not_allowed`) and asked again.

**Speech rules the flourish keeps** (instruction and code): it speaks to the party as "you", never
of the character as he, she, they or a man, and code refuses a flourish that uses a third-person
pronoun as a whole word (`not_spoken_to_the_party`; the model's own output against a closed list,
like the name check, so a person is said by their look or trade and a thing by its name). A roll
with no verdict (damage, healing, initiative, a table roll, a passive score) is never a win or a
loss and nothing waits to be learned: its flourish is about the effect (the blow's weight, the
warmth, the readiness, the prize), never about a verdict. `told=` passes what the runner has just
said aloud (a station's scene line) as the fact `already_told_aloud`; a call never restates or
paraphrases it and adds only the immediate trigger of the check.

The numeric part says nothing twice: a single die that is the whole total is not said as the die
and then as the total (`Eleven against fifteen. A failure.`, `Twelve against ten. A success.`,
`One hit point restored.`); with a modifier, an advantage or a disadvantage the dice are read once
and the total once.

| How it stands | Allowed tones |
| --- | --- |
| the call (before anyone has seen the dice) | `neutral`, `anticipation`, `awe`, `wry`, `dry_humor` (no `dread`: it telegraphs a failure) |
| natural 20 | `awe`, `triumph` |
| natural 1 | `dread`, `rue`, `wry`, `dry_humor` |
| a win within 2 of the target (success, hit, critical hit, death save success or revived) | `relief`, `wry` |
| a loss within 2 of the target | `rue`, `wry` |
| any other win | `triumph`, `relief`, `wry`, `awe`, `neutral`, `dry_humor` |
| any other loss (failure, miss, death save failure) | `setback`, `dread`, `rue`, `wry`, `neutral`, `dry_humor` |
| no verdict (initiative, damage, a table, a passive score) | `neutral`, `wry`, `dry_humor`, `awe` |

All tones: `triumph`, `relief`, `wry`, `awe`, `neutral`, `dry_humor`, `setback`, `dread`, `rue`,
`anticipation` (`Tone` in `request.rs`).

**The call is worded blind to the dice** (audit, gauntlet runs 1 and 2). The dice are drawn before
the call is spoken, so the result line can be worded while the call is heard; the call must not
lean on them. Code check: the `check_called` request carries the test name, the scene, the
initiator, `allowed_tones`, `said_before`, `closers_used` and `room_chars` only. `outcome`,
`natural`, `how_close` are null; the difficulty band, the mode and its grounds (a disadvantage
leans toward failure, and the game says them itself) are withheld from a call; `said_before` and
`closers_used` hold earlier checks' flourishes, and the same check's result is worded after the
call exists; `previous_refusal` holds only the call's own refusals. A test runs the same call
with dice showing 2 and 19 and requires identical facts. Statistics from the logs (play1 and
play2, 52 rolls, calls whose flourish has a failure-flavoured image such as slip, almost,
nearly, falter, fall, reach, catches): of 8 such calls 4 were failures, 3 wins and 1 had no
verdict, against 31 failures in the 51 rolls with a verdict (61 percent), so no association;
the failures the owner found came from the model's own bias toward dread and from a high base
failure rate (DCs of 15 and above against small modifiers, two disadvantage rolls). Hardening: the call instruction says it is spoken before anyone has seen
the dice, describes only the situation and the stakes, and never hints at how it will go (no
slipping, falling short, almost, nearly, too late, no chance, nor a promise of success either),
and a call's tones are `neutral`, `anticipation`, `awe`, `wry`, `dry_humor` (`dread` removed).

**Repairs and refusals** (`lines.rs`). Code repairs what is formatting and never asks the model
again for it: a semicolon in a flourish becomes a sentence break (`a; b` is `A. B.`), so it is
never refused (run 2 refused three flourishes for semicolons and the line fell back to bare
numbers). Code refuses what is content, with a typed reason, and asks once more with it
(`ASKS_PER_LINE` is 2, since each ask is about 5 s of the budget): a slot that is not the roll's
(`numbers_mismatch`), a tone that does not fit (`tone_not_allowed`), a digit or nothing in a
flourish, a word of the persona's display name (a whole-word check of the model's output,
`names_called`; the model is not told the name) and a third-person pronoun
(`not_spoken_to_the_party`). A call's flourish carries no outcome by its tones alone.

**Budget and fallback.** A line has `LINE_BUDGET_MS` (14 s). When the budget runs out, or both
asks are refused, the line is the LAST answer read with the sentences that hold a digit, a name or
a third-person word dropped, if any sentence is left (the tone of the answer was allowed when it
was read), else its composed numbers alone, no flourish. The log says `CHECK line_unjudged
moment=check_called|check_result budget_ms=14000 why=budget|refused kept=true|false`; `kept=true`
means a flourish was kept. The numbers and the verdict are the roll's either way. In a healthy run
there is no `line_unjudged`. The narrator's rules hold: to the party as "you", no semicolons,
naming nobody, speaking for no character, solving nothing. Spoken text, by kind (the call, then
the result):

| Roll | Call | Result |
| --- | --- | --- |
| Ability check | Athletics, Strength. Difficulty fifteen. Roll it. | Fourteen on the die, plus three for Strength, plus two for proficiency. Nineteen against fifteen. A success. |
| Advantage | .. You have advantage, so roll two dice and keep the higher. | Seventeen and five with advantage, so seventeen counts, plus three for Strength, plus two for proficiency. Twenty-two against fifteen. A success. |
| Saving throw | Constitution saving throw. Difficulty fifteen. Roll it. | Nine on the die, plus two for Constitution, plus two for proficiency. Thirteen against fifteen. A failure. |
| Attack, critical | Melee attack roll. Armor class sixteen. Roll it. | Nineteen on the die, plus three for Strength, plus two for proficiency. Twenty-four against armor class sixteen. A critical hit. |
| Natural 1 / 20 | | One on the die, .. Six against armor class sixteen. A natural one, which always misses. / .. A natural twenty, a critical hit. |
| Heroic Inspiration | | Heroic Inspiration rerolled a six into a fourteen. Fourteen on the die, .. |
| Initiative | Initiative. Roll it. | Twelve and three with advantage, so twelve counts, plus one for Dexterity, plus two for proficiency. Your initiative is fifteen. |
| Death save | Death saving throw. Difficulty ten. Roll it. | Twenty against ten. A natural twenty, and you regain one hit point. |
| Passive | Passive Perception. Nothing is rolled, your score is read. | Ten to start, plus one for Wisdom, plus two for proficiency. Your passive Perception is thirteen. |
| Contest | Athletics, Strength, to break free. Difficulty fifteen. Roll it. | .. Nineteen against an escape difficulty of fifteen. A success. |
| Damage | Damage. Roll one eight-sided die and add three. | Six on the eight-sided die, plus three for Strength. Nine damage. |
| Healing | Healing. Roll one ten-sided die and add three. | Seven on the ten-sided die, plus three. Ten hit points restored. |
| Table (d100) | Table roll. Roll percentile dice, two ten-sided dice, tens first. | The tens die shows sixty and the ones die shows two. Sixty-two on the table. |

The card (`view.rs`): a percentile pair is one die of 100 sides; the death-save verdicts the card
has no word for read success or failure with the natural mark; a contest reads won or lost; a roll
with no target (initiative, damage, a table) has no verdict. The wire type is the display's
`RollView` only.

A modifier is spoken by the source its roll labels ("plus three for Strength" for a damage roll).
A healing of the sheet has only a bare dice expression (`1d10+2`), so its request is built with the
`Flat` label and its modifier is spoken unlabelled ("plus two"), not "for Constitution": naming it
needs a label on the sheet's healing entry (the hit die's Constitution, a Second Wind's Fighter
level, which `df-rules` has no `ModifierLabel` for), an open item.

**Audit lines** (no story text): `PERSONA check_judged character=.. moment=.. required=.. kind=..
reason=.. mode=.. retried=.. duration_ms=..`, `PERSONA check_judge_retried why=<error key>`,
`PERSONA check_line moment=check_called|check_result tone=.. retried=.. duration_ms=..`,
`PERSONA check_line_retried moment=.. why=<error key>`, the `CHECK ..` lines above and
`PERSONA refused verb=check error=<key>`.

**Run one station.** With the host up (one dev host, port 7879) and `gauntlet_warrior` stored
(`persona write gauntlet_warrior development/market-threads/gauntlet_warrior.persona.json`):

```bash
$D dev --port 7879 persona check judge gauntlet_warrior station=athletics            # the narrator's moment
$D dev --port 7879 persona check judge gauntlet_warrior station=athletics "action=I set my hands to the rock and climb."
$D dev --port 7879 persona check run gauntlet_warrior station=athletics "action=I set my hands to the rock and climb."
```

**Live checklist** (coordinator, with TTS off unless the owner says): (1) `judge` gives a typed
answer and `designed: .. match=true` for the station; trivial and impossible actions are not
rolled; (2) `run`: `CHECK called` before `CHECK rolled`, the card appears after the call has been
heard (3 s plus 70 ms a character); (3) the card holds at least 6 s and until `result_read` has
ended plus 2 s (`cleared_ms - result_end_ms >= 2000`, `hold_ms >= 6000`); (4) the numbers on the
card equal the numbers in the result line, word for word; (5) with TTS off the log says
`voice=text_only`; (6) a second run at the same place with `checked=` naming the first test does
not call it twice; (7) real entropy: two runs of one station differ and `RULES rolled` lines show
the faces; (8) no `CHECK` line carries story text.

**Why the 2026-11-10 trials stalled, and what to measure.** First trial, host log `TEXT answered ..
duration_ms=`: the judge 6.9 s; flourish calls 4.6 to 6.2 s; names judge 3.4 to 6.0 s (one 56.9 s);
consistency judge 7.0 s, 47.9 s, 50.8 s, 52.7 s, 53.3 s; three of four lines refused `contradicts`.
Cause one was a code defect (the judge saw the composed numbers, so "Difficulty fifteen" read as a
number said), fixed by judging the flourish alone. Second trial: every line still hit its 14 s
budget, because the consistency judge (a Codex call of about 50 s) was on the critical path. It is
gone, with its `Contradiction` reasons: the flourish names a `tone` and code checks it, the names
judge is a whole-word check of the model's output, and a line is one model call. Scripted tests
cannot show latency; measure live (`grep -E 'TEXT answered|CHECK|check_line' host.log`): the
judge about 7 s; `check_called_line` and `check_result_line` about 5 to 6 s each, started together
and with no `check_line_consistency` or `story_names` after them; `PERSONA check_line .. tone=..
retried=0` per line (a retry costs another call); no `CHECK line_unjudged` in a healthy run;
`CHECK called` within about 8 s of the judge's answer; and the whole run (judge, call heard out,
card, result read, cleared) near 50 s, excluding TTS audio length. Check that consecutive flourishes
open differently. Third trial: the flourish returned, but a call of 40 words and a result of 25 s
read (the card up 32 s) were not quick, and the result re-described the call's image because both
were worded in the same breath. Now the result is worded after the call, told it, and both
flourishes are one short sentence; measure `CHECK read_len`: a call about 10 s and a result about
12 s at 14 characters a second, and a station of about 60 s with the dungeon master's scene line
spoken by the runner first (a narrator station's call must not describe that scene again).

Not verified without a live model run: how often the judge's answers are refused and asked again,
and the flourishes themselves (every scripted test pins the code around them). Gaps: no tool or
Expertise rolls (a Fighter has none), no social contests beyond the two SRD tests (a model that
wants one is told to pick another kind), and the story narrator's `Moment` has no `check_call`
variant (the narrator's moment is the judge's `moment=narrator`).

## The gauntlet run

One run of 48 stations: every station in `market-threads/gauntlet.stations.json` (39: 18 skills, 6 raw
abilities, passive Perception, 6 saving throws, 2 attack rolls, the contest, initiative, a death save,
damage, healing and a percentile table) and nine repeats (below), played by the runner's gauntlet mode from
`market-threads/gauntlet.beats`. Design: `planning/dice-gauntlet.md`; the flow it drives is "The check
judge and the DM's checks" above. Files: `gauntlet_warrior.persona.json` (stored by the runner),
`gauntlet_warrior.sheet.json`, the stations file and the plates `gauntlet-<name>.png` in the scratch
playground's `media/`.

**Prep (before the first station).** The host refuses a spoken line on a plate that has no layout
(`no_layout`) or none with him named (`unknown_character`), so the runner makes every station plate
(33, each once) ready itself. For each it reads `media locate show <plate>`: a layout with
`gauntlet_warrior` named is `already`; none is scanned (`media locate <plate>`, `scanned`); then
`media locate identify <plate> "gauntlet_warrior=<@gauntlet_look>" "portrait.gauntlet_warrior=<@gauntlet_portrait>"`
names him (`named`; the look has no name, the portrait defaults to `ref-<id>.png`). A plate where he
is not found, or where the scan or the ask failed, logs `[prep] <plate> FAILED(<reason>)` and its
stations become narrator-only (`[station NN] narrator-only: ..`: the dungeon master sets the scene
and calls the check, no `persona say`; the `glade` plate has no warrior in it). A closing
`[prep] N plates: already= scanned= named= failed= | narrator-only plates: ..` counts them. A
scan and an ask take about 6 s each, so a cold prep is about 3 minutes; a second run finds them
`already`.

**Per station** the runner does: (a) `scene image tv-backdrop <plate>` (the page's fade is set once
from `@gauntlet_fade 800`), (b) `persona narrate` of the station's scene line (the dungeon master,
voiced when TTS is on, waited out at 3 s plus 70 ms a character), (c) `persona say gauntlet_warrior`
toward the station's goal (his own words; at the 20 narrator runs nothing is said and the
dungeon master calls the check unasked) and (d) `persona check run gauntlet_warrior station=<id>
moment=player action=<his line>` (or `moment=narrator`), which judges, calls, rolls with real
entropy, shows the card, reads the result and clears it. Then a 1.5 s breath. Narrator stations:
acrobatics, animal_handling, survival, the six raw abilities (raw_str, raw_dex, raw_con, raw_int, raw_wis,
raw_cha: the judge picks a skill whenever one fits a player's line, so these are narrator scenes of a pure act of the
ability with the test passed as `intent=`), passive_perception, save_str, save_dex, save_con, save_int, save_wis,
contested, initiative and death_save (three times). Advantage is the sheet's (Remarkable Athlete:
athletics, contested, initiative), disadvantage the armour's (`@station_sight stealth` says his chain
mail clinks); Heroic Inspiration is spent on the d20 of persuasion (`@station_inspire persuasion 0`).

**Directives** (`@` lines of the beats file): `@gauntlet <character>`, `@gauntlet_persona`,
`@gauntlet_sheet`, `@gauntlet_stations` (files), `@gauntlet_look <short look, no name>`,
`@gauntlet_portrait <reference sheet>` (the prep), `@gauntlet_fade <ms>`, `@gauntlet_breath <s>`,
`@gauntlet_limit <s>` (a station past this is logged `[station NN] stuck` and left), `@station <id> |
player|narrator | <the dungeon master's scene line> | <his goal>`, `@station_sight <id> <words>` (a
fact in sight for the judge, repeatable), `@station_inspire <id> <die place>`, and `@station_plate <id>
<plate>` / `@station_place <id> <words>` (a different plate and setting for every run of that id). A
station id listed again is a repeat. The nine repeats: a second and third shot at the range, a second and third round of the
bout, three more damage runs and a second and third death save. The bout is at the ford (the intimidation plate): a
straw dummy has no armor class, so the melee attacks (`attack_melee` three times) and the damage runs
(`damage`: longsword d8, greataxe d12, dagger d4, shortbow d6) are against the bandit, a live armed foe
who strikes back. Every goal is an act in the fiction, never "roll", dice, damage or a table. A damage
roll is called only where the scene says the blow has already landed (a scene that opens a new strike
is judged an attack roll: run 2 got no d6 or d12 that way), so each damage scene says the hit is done
and he follows through. The percentile table is a lot drawn from an offering urn. Healing has a stake
(an open cut across his ribs that must be closed before the road; the judge called no stakes for a man
who only sat and drank), and the performance stage has one too (the tavern crowd decides tonight whether
the company is sheltered, and it hinges on his song).

**`intent=`.** At a narrator station the called check should be the designed one. The runner asks the
host's `persona check` usage text at the start (`[gauntlet] persona check takes intent=: yes|no`) and,
only when it shows `intent=`, adds `intent=<kind[:ability[:skill]]>` from the station's own kind,
ability and skill (the abilities as the roll vocabulary writes them: `intent=saving_throw:dexterity`,
`intent=ability_check:wisdom:survival`, `intent=death_save`) to every station run as the narrator's,
the narrator-only ones included. Nothing changes on a host build without it.

**`told=`.** The runner has just spoken the station's scene line before the check, and a narrator
station's call must not say it again. The same usage-text gate (`[gauntlet] persona check takes
told=: yes|no`) adds `told=<the scene line>` to `persona check run` at every station that has one (all
of them), which the host gives the model as the fact `already_told_aloud`. At player stations his own
line still goes through `action=`. The line is passed as one argument, so apostrophes and commas need
no quoting. `--plan` shows `told=` on every check line, as it is sent once the host takes it.

**A station with no roll.** He is a plain-spoken veteran who may refuse a distasteful act (live trial 4:
at the purse he said he would leave it, the judge rightly called no roll, and the gauntlet had a
station with none). So every player goal in the beats is a decision already taken ("You have decided
to .. and you do it now. Say, in one short line, exactly what you do"), never a question, an option,
a die, a difficulty, a skill or his name, and where an honest man would refuse (theft, the lie, the
threat) the scene or the goal gives him a motive (the coin was stolen from a starving village, the
guard is a smuggler's man, a family cannot cross). When `persona check run` still answers
`required=false` the runner logs `[station NN] NO CHECK (judge: <reason>; he said: <line>)`; at a
player station whose designed kind is a roll it then asks him once more (`persona say` with "You
commit to it: do it, in a sentence." added to the goal, `[said NN] retry: ..`) and runs the check
again, and a second miss logs `NO CHECK after the retry: given up`. Both are logged and the
`[summary]` counts them (`no_check=N` stations that ended with no roll, and how many first asks were
retried). Nothing is hidden: a retry that rolls is still a station whose first answer was a refusal.

**A station that runs out of time** (`@gauntlet_limit`) is logged `stuck at <step>`; when its
`persona check run` had begun, the runner then runs `scene roll-clear`, waits (two minutes at most)
until the host's flow ends it (`CHECK cleared`, `CHECK skipped` or `PERSONA refused verb=check`),
clears again and logs `[station NN] cleared after stuck`, so a stuck station cannot cover the next.

**Check the plan first** (no host, no spend; it lists every station's four steps and the prep, and
fails on a missing plate, portrait, file, station or look, or a scene line with a semicolon, and
prints `PLAN RESULT PASS`; the plan shows `intent=` as it is sent once the host takes it):

```bash
bash development/market-thread.sh development/market-threads/gauntlet.beats --plan
```

**Start** (voiced; leave out `DF_RUN_TTS=on` for a text-only run, and the host's stored switch
stands). Needs the one dev host on 7879 as the roam procedure leaves it (steps 6 and 7 of "Free roam:
the current run procedure": the scratch playground, Codex text, a shown scene) and the page open on
`http://localhost:7879/playground/` with the TV layout and Enable sound clicked, reloaded before the
run. `D` is the host's exe. The runner stores the persona, resets its state, sets the fade and shows
each plate itself:

```bash
D=$PWD/artifacts/build/roam/debug/df-table.exe
$D dev --port 7879 scene clues clear
$D dev --port 7879 scene narration clear
mkdir -p artifacts/tmp/gauntlet-live && cp development/market-thread.sh artifacts/tmp/gauntlet-live/runner.sh
sed -i 's#^cd .*#cd /c/Users/mreca/Desktop/dungeonfluxGame#' artifacts/tmp/gauntlet-live/runner.sh
N=1; DF_RUN_TTS=on DF_TABLE_EXE=$D DF_TABLE_PORT=7879 bash artifacts/tmp/gauntlet-live/runner.sh \
  development/market-threads/gauntlet.beats > artifacts/tmp/gauntlet-live/play$N.log 2>&1 &
```

**Monitor** (the same pipe as the roam's):

```bash
tail -n +1 -f artifacts/tmp/gauntlet-live/play$N.log | grep --line-buffered -E '^\[(settings|gauntlet|prep|station|dm|said|check|roll|hold|time|host|inspire|summary)' | stdbuf -oL cut -c1-260
```

**Length:** about 60 to 90 s a station (run 1: mean 66 to 68 s, 48.7 minutes for 39 and 45 stations), so about 55 minutes for the 48 (a slow model call can add
to it, and a station gets 180 s), after a prep of about 3 minutes on a cold start (seconds when every
plate is `already`). Do not reload the page mid-run.

**Log lines** (all from the command's answer and the host's own `CHECK`, `TIMING roll`, `RULES rolled`
and `PERSONA check_*` lines read after each station; none is a guess):

| Line | Meaning |
| --- | --- |
| `[settings] tts=on (from flag)` | The first lines: whether the paid switches are on and who set them. |
| `[station NN] id \| title \| kind` | A station starts. Then `[dm NN] scene: ..` and `[said NN] ..` (`(nobody: ..)` at a narrator station). |
| `[check NN] required=.. kind=.. ability=.. skill=.. dc=.. mode=.. grounds=.. uses=.. initiator=.. reason=.. retried=.. judge_ms=.. designed=match\|differs(kind)` | The judge's typed answer; `designed` says whether it is the station's designed test. `required=false` is an action the judge called no roll for (`reason=trivial_no_check` and the like). |
| `[roll NN] faces=d20=14,d20=5x total=.. outcome=.. natural=.. card=.. target=.. (title)` | The dice (`x` is the one set aside) and the card's numbers. |
| `[dm NN] call: ..` and `[dm NN] result: ..` | What the dungeon master said before the dice and after them. |
| `[hold NN] card_up_ms dm_result_ms hold_ms min_hold_ms result_to_clear_ms page_card_ms voice held` | `card_up_ms` is `CHECK shown` to `CHECK cleared`, `dm_result_ms` the result line's `result_read` start to end, `result_to_clear_ms` the card's time left after the line ended (at least 2000), `page_card_ms` the page's own `TIMING roll` for the card (`-` when no page reported it), `held=ok` when `hold_ms` is at least 6000 and `result_to_clear_ms` at least 2000, else `SHORT`. |
| `[time NN] station took N s (judge, call, roll, result, hold)` | Seconds: the judge's model call, the draw to the card up (wording, the call heard out), the card up to the sum, the result line, the rest of the hold. |
| `[host NN] judge_retries=.. line_retries=.. refused=..` | The judge asked again, a line asked again, `PERSONA refused verb=check`. |
| `[inspire NN] Heroic Inspiration replaced d20:6:kept (was 6, now 14) .. total_before=.. target=..` | The die replaced by Heroic Inspiration and the total before the reroll against the target, from `RULES rolled replaced=`. |
| `[prep] <plate> already\|scanned\|named\|FAILED(<reason>)` and `[prep] N plates: ..` | The layout prep of each plate, once, before the first station. |
| `[station NN] narrator-only: ..` | The prep found no warrior in the plate: no `persona say`, the dungeon master calls the check. |
| `[station NN] stuck at <step> after N s` / `skipped at <step>: ..` | A station left (the time ran out; or a plate, line or file failed). |
| `[station NN] cleared after stuck (the host was idle N s later)` | After a stuck check: `scene roll-clear`, then the host's flow ended (or `.. did not end its check within N s`). |
| `[summary] ..` | Stations planned, run, no_check (and how many first asks were retried), rolled, skipped, stuck, failed; initiated by; rolls by kind and die size; modes and Heroic Inspiration; natural marks; the judge's matches and retries; cards held too short; times. |

**What to look for** (the coordinator and the inspector): (0) `[prep]` ends with `failed=0` (else the
narrator-only stations are listed); (1) 48 `[station NN]` lines, `stuck=0`,
`skipped=0`, `failed=0`; (2) every `[hold NN]` says `held=ok` and `card_up_ms` is at least 10000
(4.5 s to the sum, at least 6 s of hold, 0.4 s exit); (3) `[dm NN] call` before the card and
`[dm NN] result` after the dice, and the numbers in the result equal `[roll NN]`; (4) 20 narrator runs (plus any the prep made narrator-only)
stations show `initiator=narrator` with a `narrator_*` reason, the 28 others `player` with
`uncertain_with_consequence` (a `NO CHECK` line is a station with no roll, a failure to count: `no_check=0`);
(5) athletics, contested and initiative show `mode=advantage grounds=character_feature` when the
judge accepts the feature, stealth `disadvantage armor_penalty`; (6) one `[inspire NN]` line, at
persuasion; (7) two stations of one kind do not roll the same dice (real entropy); (8) `rolls by die
size` has d20 and, from the damage runs, healing and the table, d4, d6, d8, d10, d12 and d100 (the
damage runs use all four weapons); (9) a `time` per station near 60 to 90 s; (10) with TTS off `voice=text_only`, with it on
`voice=played`; (11) frames: a `TIMING roll` for each card in the host log, then the inspector goes
through the screen captures one by one against `planning/dice-gauntlet.md`'s display rules.

**Gaps and limits.** Every kind of the flow runs (ability_check, saving_throw, attack_roll,
initiative, death_save, passive, contested, damage, healing, table); the judge picks the kind from the
line he said, so `designed=differs(..)` is possible and is logged, not forced. Heroic Inspiration is
committed to a die before the dice are drawn (the flow takes `inspiration=` with the request), so the
run cannot spend it only on a failed roll: it is spent at persuasion, where his Charisma is negative
and a failure is likely, and `[inspire NN]` says what the first total was. A natural 1 or 20 shows on
a card only for attack rolls and death saves, and the run does not force one (the dice are real):
the summary counts them; the run has six attack rolls and three death saves for the chance, and a second run adds more. Advantage by
`ally_help`, `favorable_circumstance` and `unaware_opponent`, and disadvantage by
`unfavorable_circumstance`, are not exercised (no station gives them; a `@station_sight` line could).
Expertise has no Fighter source. Not verified without a live run: the judge's refusals, the lines
themselves, the 60 to 90 s pace and the page's `TIMING roll` records.

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

## Walking about town

Owner, 2026-11-10: "the character needs to walk about town"; players move gradually between places,
they do not teleport. A move used to be a jump cut: he says where he goes and the plate swaps to the
new place. Now a move can show the sights he passes on the way first, then the place. It reuses the
idea of "The way between places" (stages with a sight and a plate) for a move of free roam, and is
story-neutral: the sights, the town and the pace are data.

**The contract.**

- **The design call returns `stages`.** `POST /dev/move/design` takes `walk: {stages, town,
  move_id}` (`move act ... walk=N town=ROLE`); with it the closed schema adds `stages`, 0 to N
  sights `{id, sight, pose}`: `id` a place id no other sight of the move has, `sight` what a camera
  sees of it (a fountain, a row of shopfronts, an archway), `pose` `walking` or `pausing`. Code
  refuses an id that is not a place id or repeats (`invalid_stage`), a blank text, a pose outside the
  two words, and more sights than the move asked for (`too_many_stages`, with the count); a refused
  answer is asked again with every refusal so far, as for every design (see "Troubleshooting"). A move that asked for no walk takes no
  sights. Nothing in the design names anyone or lettering (the instruction says so, as for the rest).
- **`@walk <way> <sights> [town=<seeds|within|leaves>]` lines** in the beats say how many sights a
  kind of move asks for at most and its role in the town (`free-roam.beats`: town moves 2 sights,
  road moves 1, market moves 2). `@walk_hold <seconds>` is how long each sight is up (default 6, never
  under 5: `move act` refuses `hold=` under 5 s). The runner passes them to `move act` as `walk=`,
  `town=`, `hold=` and, from `@tempo walk <kind> <strength>`, `impulse=<kind>:<strength>`.
- **One town record.** `GET /dev/world/town` (`world town`) answers the town the party is in:
  `{id: "town-<move id>", look, created_at: <move id>}`, kept in `world/state.json` beside the active
  place. The design of a `seeds` move gives `town_look` (materials, roofs, doors, street layout,
  light; no names) when there is no town, and it is kept (`WORLD town_seeded id=.. created_at=..`);
  a `seeds` or `within` move after it is told the look (`route.town` in the facts), asks for none and
  carries it; a `leaves` move ends it (`WORLD town_left id=..`) and the next `seeds` move gives a new
  one. A designed place (`arrive`) keeps the town; setting the place to nowhere or to a bible place
  ends it. The `MOVE designed` line says `stages=N town=seeded|reused|left|none`. A move with no
  plate is only looked at and changes no town.
- **Places do not repeat (the design's `feature`).** Every design answers `feature`: a short
  lowercase id (letters, digits, `_`, `-`) for the one thing that sets the place apart (a ford, a
  mill, a crossroads shrine: the instruction states the principle, never a list). `world/state.json`
  keeps `visited`, oldest first, `{name, feature}` of each place the party arrived at (`POST
  /dev/world/arrive` takes the design's `feature`, which `move act ... plate=NAME` sends once the
  plate is up; a text-only `move design` or `move brief` keeps nothing). The design call's facts
  carry them all as `previously_designed`, and the instruction asks for a place distinct from every
  one while it still fits the destination words and the town (`route.town`). An answer whose
  `feature` equals a kept one (exact id comparison, no words matched) is refused as
  `repeated_feature` and asked again with that rule among the `refused_before` list (`MOVE retry
  verb=design attempt=2 reason=repeated_feature ..`); the `MOVE designed` line names `feature=<id>`. The list
  ends when a bible place is made active (`world place <id>` or `world place -`); to start a run
  fresh without that, delete the `visited` key of `state.json` (it is read again on every call).
  Known limit: going back to a place already designed is not a move this supports yet, since a
  design always has to be a new feature.
- **The town's look is in every picture of it**: the place's prompt (and the `picture:` words the
  frame judge reads) say "The town, the same in every picture of it: <look>", and so does each
  sight's.

**How the show runs.** `move act ... plate=NAME walk=N` makes the sights and the place at the same
time, not one after the other: each sight is its own plate (`NAME-walk1`, `NAME-walk2`) painted
anew from a compact prompt (the town's look, the sight, the movers by look as small figures walking
or pausing; no things, no items, no residents) after the movers' reference sheets alone, all at normal priority (a sight never
starves the place) and shown the full-length view of the sheet (`sheet: full_length`: one person, once);
the place is made as always (sheets of what he carries, residents) and is not made later for the
sights. Each plate is scanned and named like an arrival plate. The sequencer (`movement::walk`,
pure, tested on a controlled clock) then shows them:

- a sight goes up as soon as it is ready and the one before it has been up its hold, through the
  existing swap (backdrop with the page's fade, the bubbles of anyone not in it taken away, the
  scene's cast and layout of that plate), in order;
- while the place is not made, the sights are spread over the wait it is expected to take instead
  of going up back to back: the place is expected `PLACE_TO_SIGHT` (2) times as long as the sights
  took to land (live run 9, moves 5 to 7: 2.2, 2.9, 1.7 times), and each sight is up its share of the
  time left to then, never under its hold. A place that comes in sooner ends the spreading: the
  sight up only finishes its plain hold. Run 9 move 6 (sights landed at 72 s, place at 207 s) left
  its last sight up 129 s; more sights share the same wait, so a bigger `@walk` count shortens each
  one (a walk of 4 sights gives about 18 s each from 72 s);
- a sight not ready when its turn comes and the place is ready is passed over
  (`skipped reason=late`), so a slow sight never keeps the place from arriving; a sight whose
  making or swap failed is passed over (`reason=failed`); the move never fails for a sight;
  plates that come in together are all taken in before any is passed over, a sight made before an
  earlier one waits its turn, and a stream that closes with every plate in only waits out the hold;
  a sight that never came is passed over (`no plate came for it`);
- the place never goes up before a sight that is ready, and goes up once the last sight shown has
  been up its hold; with no sight ready, it goes up as soon as it is made, as before; the place made is never dropped
  and the walk always ends on it. Only the place failing ends the walk in failure, and that failure
  says each plate's state (`plates_stopped: .. stage 1 passed, stage 2 pending, place pending`);
- the spinner title is put back when the first picture of the walk goes up.

So a sight is up 5 s at the least (6 s by default) and the viewer never sees a change faster than
that. The sights are made while the old place is still up; they add no time to the move unless
the image backend is one queue (Forge: each sight is one more render ahead of the place; on Codex
they run side by side). The walk cannot overlap the previous move's reveal and dwell: the runner starts a move only
after the last one ended (its frame watch is stopped), so the walk starts at the design.

**Logs.** `move act` prints, and the runner turns into `[walk NN]`:

| Line | Meaning |
| --- | --- |
| `[town NN] <look>` | The look of the town the place was designed in. |
| `[walk NN] stage k/N <sight> prompt: ..` | The compact prompt of each sight. |
| `[walk NN] <frame> made in <ms> ms: <regions found, characters named>` | A plate made and scanned (the sights and the place). |
| `[walk NN] stage k/N <sight> shown_at_ms=.. up_ms=..` | A sight was shown (playground clock) and how long it was up. |
| `[walk NN] stage k/N <sight> skipped reason=late\|failed` | A sight passed over. |
| `[walk NN] memory <id> kind=walk seq=.. place=.. picture=..` | The sight kept as a memory of the traveller, linked to the place (`links` has the place). |
| `[walk NN] arrived stages=S/N walk_ms=..` | The end: sights shown of those asked, and the time from the first sight up to the place up. |
| `[time NN] move took .. (plate P, of which walk W, ..)` | The plate's time includes the walk. |

The host logs `PERSONA frame_timing moment=started|shown|ended|dropped frame=<NAME>-walkK
reason=walk ..` for each plate of the walk (the place is `frame=<NAME>`), `WORLD walk_stage
place=.. stage=.. number=k of=N outcome=shown|late|failed at_ms=.. up_ms=..` for each sight, and
`WORLD town_seeded` / `WORLD town_left`. The transition's `TRANSITION` records are written as the
walk happened: `locating` at the place made, `live` at the place up (the runner's `plate_ms` is
the place's making; `live_at_ms` includes the walk).

**Tempo.** Each sight posts the impulse of `@tempo walk <kind> <strength>` (`walk` in
`free-roam.beats`, `{"kind": "walk"}` in the music table) when it goes up; a table without the kind
only prints `walk impulse .. not posted`.

**Check it without spend** (text only, the host up): `$D dev --port 7879 move act danny walk=2
town=seeds "<situation>"` with no `plate=` runs Jev and the design and prints the `stage k/N ..`
lines and the `town:` line, and changes no town. The live check is a roam run (see "Free roam: the
current run procedure"): watch the `[walk NN]` lines, the TV (each sight up 6 s with a fade, then
the place) and `$D dev --port 7879 world town`.

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

## Declared actions and emergency plates

A character can **declare** the physical action they are doing right now, in the first person and
the present tense ("I set the board on my easel and lay in the door with ochre"). The declaration
is spoken like any line (voice, bubble, the speaker's name), and it is the subject of an
**emergency plate**: a picture of the scene showing the action in progress, made ahead of
everything else and shown at once. Owner, 2026-11-09: "I wanna see him mid painting too with a
DECLARE action he says exactly what he is doing and that emergency queues and reorders the display
plate order".

```bash
# The character words it (the per-line call is asked to declare; its answer is checked, below)
$D dev --port 7879 persona declare danny speak=true "recent=Danny: Right then." \
  "You are at the foot of a hill. Set up to paint the wall in front of you with what you carry."
# Or the words are given, spoken as written (no model reads them: no inventory, no feeling moves)
$D dev --port 7879 persona declare danny "text=I set the board on my easel and lay in the door with ochre." \
  "doing=sets a primed board on a folding easel and lays in a door with ochre" uses=easel,boards,pigments
```

It prints the line as `persona say` does (`danny says: ..`, `inventory: ..`), then
`  declared: doing=.. | pose=.. | uses=<item ids>` and one `watch: @declare danny | {..}` line, the
input that hands the declaration to the frame watch that is running (`persona watch` reads it on
standard input beside its `<id> | <line>` and `@focus` lines; the market runner pipes that line to
it). The watch owns the screen and its queue, so the plate is made there.

**The words (`POST /dev/persona/declare`, `DeclareRequest`).** `character`, then either `text`
(spoken as written; `doing` is what the picture shows, in the plain present tense of a third person
with no name, else the text itself; `uses` the item ids the hands use) or a `situation` the
character turns into the declaration; also `model`, `speak`, `recent`, `asked_by`. The per-line
call is told `now.declare` and answers a closed `declared_action` as well: `doing` (what the hands
do and with what, third person, no name), `pose` (or null) and `items_in_use` (ids from
`now.inventory`). Code checks it against what the character holds now: an item it does not hold
(or holds none of) is refused, and the line is worded again once with why (`retry_reason`: `declared_action
uses spoon, which the character does not hold now; they hold: ladle, flour`); a second miss, or an
answer with no `declared_action`, is a refusal (`PERSONA refused verb=declare .. error=answer_refused`,
`LineError::Declaration`), never a picture of something they do not carry. `text` is checked the
same way for `uses`. Whether the words declare an action at all is the model's judgement (the
line's `follows_intent`, as for any intent); code never reads them. Judged lines change inventory as
always, so a declaration that uses a pigment up lowers it (`inventory: ochre 3 -> 2`). With no
supplies the situation says what to do and the facts decide: they declare what they can do with
what they hold (sketching with charcoal) or that they set off to buy. Audit:
`PERSONA declaration id=.. verbatim=true|false items=<n> retried=..`.

**The emergency plate (`FrameWatch::declare`).** No frame judge decides whether to cut (the frame
route answers `ask: {reason: declare, scene}` with the frame settings and no model call, as a
`@focus` does) and no scheduled slot is waited for. The plate is an edit of the picture on screen
(the place and the faces stay; painted anew when there is none) of the scene "A person (<look>)
<doing>, <pose>, with their <item names> in view. Everything else in the scene is as it was.",
framed as `show.declare_framing` (medium-wide, eye level). The prompt is painted from the look and
the action and **never names anyone** (the id and the display name stay out). It is drawn after the
speaker's reference sheet first, then the sheets of the items in use within the backend's picture
budget (`items_that_fit`; Codex 5, Forge 8), and logs `IMAGEGEN references ..` as any plate does; an
item without a sheet is told in words and its sheet asked for.

**One person, once (roam run 3).** The first two declared plates showed Danny two and three times:
`ref-danny.png` is one picture of three views side by side (front close-up, three-quarter,
full-length) and an edit shown the whole sheet painted each view as a person, and pulled the base's
distant figures forward in his likeness. Three fixes, story-neutral: (1) the scene's words state the
cast in view ("The cast in view is one person, the one described, who appears once and only
once. Nobody is added and no one is copied: anyone else already in the picture, such as small
distant figures, stays as small, as far away and as unlike this person as before"; for a larger
cast, "{n} people; the one described is one of them"), counted from the scene's cast; (2) the
negative prompt adds `the same person twice, clones, extra people` (`NO_DUPLICATES`); (3) the image
service never gives an EDIT (a request with a base) a person's whole multi-view sheet: a reference
picture at least 3:2 wide is cut to its left third, the front view (`df_imagery::sheet_view`, pure
pixels, unit-tested), in memory for that job, and `IMAGEGEN sheet_view job=.. name=.. picture=..
view=front` says so; a picture that is not that wide is a single view and goes as it is, and a
picture painted anew (no base) is given the whole sheet unless the request says otherwise: the
request carries `sheet: whole|front|full_length` (`SheetPick`), and the view follows it, not the
priority. A declared plate painted anew and every sight of a walk (a small figure in a wide view) ask
for `full_length` (the full-length third, `view=full_length`); an edit that says nothing gets the
front crop, for every edit, not only declared plates.

**Extra figures in the base (roam run 4: a market stall).** Even with the cast count and the front
view, an edit of a base that already holds other people carried one more likeness into the action
(1 of 6 plates). So before it edits, the watch counts the figures in the base's stored scan
(`<base>.layout.json` regions) against the cast (`plan_base`): more figures than the cast and it
does not edit; it paints anew from the place's words (the watch's `scene=`, else the newest
picture's words) joined to the declared scene, with the cast count statement, the speaker's sheet
(full-length third) and the items in use. A base holding only the cast is edited, as before; one
with no scan is edited (nothing says otherwise). The transcript says `declared plate base=edit|fresh
reason=cast_only|extra_figures|layout_unread|no_base (N figure(s) .., M in the cast)` and the
`PERSONA declared ..` line ends `.. made_ms=.. first_made_ms=<ms|-> attempt=1|2 base=edit|fresh
reason=.. shown_at_ms=..`.

**Verified after it is made (roam run 5: two identical men at a street well, `base=edit
reason=cast_only`).** No prompt prevents a duplicate every time, so a declared plate is checked
(`persona/people_check.rs`, one count check `more_than_cast` and the two attempts `verified`; only
declared plates use it now, any frame whose cast is known could): the person scan every plate
already gets in `scan_plate` (the located regions, the same pass as `media locate`, no second call)
is counted against the cast. More people than the cast and the plate is made once more as a fresh
paint (`<frame>-redo`: the place's words, the cast count statement, the full-length sheet view,
the negative prompt, still first in the backend's line). The attempt within the cast is the plate,
and its own scan stays as its layout (the first attempt's picture and layout stay in the library
unused). If the redo still shows more, nothing is shown and the room stays as it is: `PERSONA
declared .. attempt=2 .. rejected=extra_figures people=N cast=M failed_at_ms=..` and `frame_timing
moment=failed reason=declared`; the declaration's words were still said. `made_ms` is the shown
(or last) attempt's making time, `first_made_ms` the first's when there was a second, and
`queue_wait_ms` the rest of the plate's time. A redo is a second image job, so a plate that is
refused takes about two plates' time before the failure. Not done: item sheets, which also show
two views side by side.

**A soft cap on the pictures of a plate (smart references, not many).** More reference pictures
dilute a plate and slow it, and Codex takes 5 at most. The host holds the pictures of a plate
request it builds itself (move plates, declared plates, inserts, shows) to a soft cap, 3 by
default, never above the backend's own maximum: `media backend refs=<N>` sets it on the running
host (`POST /dev/media/references`, one `IMAGEGEN reference_cap maximum=.. backend_maximum=..`
line), `persona watch refs_max=<N>` sets it when the watch starts, and the runner's `@refs max=<N>`
does both. It works through the limits every caller already fits its pictures to
(`services` / `media backend` show the capped `pictures`, and `soft cap N`), so no caller changed.
The picture to edit counts as one. Relevance order: the people's sheets first (the speaker first
in a declared plate), then the items in use or in hand in the order given (a declared plate's
`items_in_use`; a move plate's most visible first), then what is only seen; an insert is given
none. What does not fit is left out and told in words where it has a look. Every job's
`IMAGEGEN references .. count=<n> maximum=<soft> backend_maximum=<n> left_out=..` line says the
cap and what a caller left out (a declared plate's items left out are in the watch's transcript).

**Showing a picture that already exists (`@show`, `persona show-picture`).** A finished painting
is cut to as it is: no picture is made and no model is asked. In a watch, `@show <picture> |
<caption> | <seconds>` (seconds optional, 10 by default; the last field is the seconds only when it
is a whole number, so a caption may hold ` | `): an insert of that picture, full frame, with the
room's layout so the cast and the bubbles stay valid, the caption on the scene's caption bar, held
for the seconds, then the room and the caption that was there return. It is shown at once like an
emergency plate (when a generated picture is up, only its least time, 8 s, is waited for) but is no
new picture of the talk: plates waiting for their slots are not dropped, they wait out the hold
and go up after it, and the newest picture (the next edit's base) does not change.
**Reveals are a sequence (run 6: `@show` twice, 10 s then 6 s, were both shown 8 s).** Each is
shown its FULL requested time: one asked for while another is up or queued waits for the earlier
one's whole hold, in order, then is shown at once and replaces it directly (the room does not
flash back between them); the 8 s least time is for a generated picture displaced by something
else, never for a reveal's own hold, so a hold under 8 s is honoured and the room is back at the
hold's end (the watch ticks every 250 ms, so a hold ends within that). The queue has no count cap
(the end-of-day show sends 20 in a row; run 9's cap of 4 refused the last two of six with
`queue_full`): a place is a number and one waiting task, nothing else is held, so each is taken and
shown in order for its whole hold. A room plate, an insert or a declared
plate arriving meanwhile waits for the whole sequence and goes up after it. Log:
`PERSONA frame_timing moment=started|shown|ended frame=<frames>-show-N reason=reveal ..` (`started`
is when it was asked for; for N queued behind others the first reads `PERSONA reveal_queued
picture=<name> behind=<n>`; `ended` of one is the `shown` of the next, `took_ms` its hold, a few
ms over) and, at
`shown`, `PERSONA revealed picture=<name> hold_ms=<ms>`. A picture that is no picture file name
(`invalid_name`) or that the library does not hold (`unknown_picture`) is refused, nothing changes
on screen, and the log has `PERSONA refused verb=reveal picture=<name|-> error=<key>` and a
`frame_timing moment=failed reason=reveal` line. Without a watch:
`persona show-picture <picture> room=<the picture the room goes back to> [caption=<text>]
[hold=<seconds>]` does the same on its own and waits out the hold (it refuses an unknown picture
with an error).

**Priority and reordering (`plate.rs`).**
- A declaration starts at once whatever is being made: it takes a slot even when `parallel` and
  `ROOM_QUEUE` are full. At most one emergency plate is on its way: a newer declaration replaces an
  older one still being made or waiting (`declared plate X replaced by a newer declaration`). The
  `max_frames` spend cap still applies.
- It asks the image backend for the front of its own queue (`priority: emergency` on
  `/dev/media/generate`, `media generate .. priority=emergency`; `IMAGEGEN priority job=.. level=emergency`).
  Codex has no queue of its own, so the host's is the only priority; Forge's is below.
- **Superseded.** Every room plate started before the declaration that is still waiting for its
  slot or being made is superseded: made, never shown (`plate <name>: made, not shown, superseded
  by <declared plate>`). Those waiting give up their place at once, so none holds back a newer one;
  those still being made are dropped when they are made. Plates started after it follow the normal
  rules. (A superseded plate is still drawn and scanned: its Forge jobs cannot be called back.)
- **Shown at once.** The plate goes up the moment it is made: no showing slot, no insert's hold, no
  plates ahead of it; only the least time the picture before it stays up (`show.minimum_shown_seconds`,
  8 s) is waited for, so nothing changes faster than a person can follow. It then holds the screen
  `declare_hold` (`persona watch declare_hold=<seconds>`, else `show.declare_hold_seconds`, 20) like
  an insert, and the next room plate's slot counts from it; then the room's plates go on. It becomes
  the newest picture (the next edit's base) like any room plate.
- **Timing, from the log.** `PERSONA frame_timing moment=started|shown|ended|failed frame=<frames>-N
  reason=declared at_ms=.. took_ms=..` (`took_ms`: for `started` the time since the frame before it
  started; for `shown` the time since it started; for `ended` the time it was up) and, at `shown`,
  `PERSONA declared who=<id> plate=<frame> superseded=<n> queue_wait_ms=<wait for the screen after it was
  made> made_ms=<generated, scanned, named> shown_at_ms=..`. The watch transcript has the
  `[NN] Name (declaring): ..` line and `frame N (name) started for a declaration .. superseding n
  room plate(s) (m waiting)`.
- **Every started frame closes once (the owner's "the timer is broken").** A frame that starts ends
  with exactly one of `moment=shown` (then `ended` for an insert), `moment=failed` and
  `moment=dropped ... why=superseded|overtaken|replaced|abandoned`: a room plate a newer one took
  the slot of (`overtaken`), one a declaration made after it started (`superseded`), a declared
  plate a newer declaration replaced while it was made (`replaced`), and a frame the talk left, or
  whose task stopped, before it was made (`abandoned`). A declared plate made again (the redo) is
  one frame: it started once and closes once, with the second attempt (`attempt=2`). The host
  keeps `plate.in_flight`, the names of the frames started and not yet closed, so frames made
  together count together (`controls.plate.in_flight`; `making` is `in_flight` not empty), and a
  stopped watch clears it. The toolbar adds `· N in flight` from two.

**Priority in the Forge studio (`ForgeClient::render_with`).** The studio's queue takes the job
`POST /api/runtime/jobs/<id>/move` with `{"to": "top"}` (`runtime/queue-order.js`; the same
`Origin` and `X-Forge-Request: runtime` guard as every runtime write; it moves only waiting jobs
and answers 409 `not_movable` for one with an engine job, 404/400/409 `stale` otherwise; the job
records carry `queueRank`, which this host does not need: it moves only the job it has just
submitted, by id). For an emergency image job only, right after the submit the host asks for that
move once, for that job and no other, and only when the job is still waiting (a job Forge already
runs is never asked). One line: `FORGE prioritised job=<first 8 of the id> outcome=moved|refused|
unavailable status=<job status> answer=<http|none> position=<n|-> waiting=<n|->`: `moved` (the
studio put it first; `position` and `waiting` as it answered), `refused` (the studio understood
and said no, or the job was not waiting), `unavailable` (no such route: a studio without it
answers 404 with no `code`, or cannot be reached; the job keeps its place and the host's queue is
the only priority). It never fails the render. The studio runs one render at a time, so the plate
waits for the render under way and goes next; its scan (SAM) job and the reference sheets are
ordinary jobs.

**In the roam.** `@roam_declare <situation>` (and `@roam_declare_hold <seconds>`) in the beats
file: after the look and the perception, the character declares the painting action
(`persona declare` with the situation, `$MIND` joined) instead of a separate `@roam_paint` line:
the declaration is their painting line (the supplies it uses are judged as before and `[inventory]`
shows them), the study painting of `@roam_art_*` is made after it as before, and the runner waits
for the plate to be shown and to end its hold. `[declare NN] <words>`, then
`[declare NN] plate=.. shown_at_ms=.. made_ms=.. queue_wait_ms=.. superseded=N ended_at_ms=..
up_ms=..`. The engine knows nothing of painters: the situation is data.

**Check it live** (the coordinator's host on 7879, rebuilt from this checkout's lane; Forge started
and `media backend forge size=512x512 steps=20` or the Codex backend; the full steps are in "Free
roam: the current run procedure"):
1. Danny and his inventory as in "Free roam", then the roam itself with `@roam_declare` set
   (`bash development/market-thread.sh development/market-threads/free-roam.beats`, the
   `@roam_declare` and `@roam_declare_hold` lines are in the file). By hand: a plate with Danny in it
   shown, `persona watch frames=declare-1 every=0 gate=100 "scene=<what the plate shows>"` reading
   standard input on one terminal, and the `watch:` line of `persona declare danny ..` pasted into it.
2. Log: `PERSONA declaration ..`, `IMAGEGEN priority job=.. level=emergency`, `FORGE prioritised ..
   outcome=moved`, `PERSONA frame_timing moment=started .. reason=declared`, then `moment=shown ..
   reason=declared`, `PERSONA declared who=danny .. superseded=N queue_wait_ms=<small> made_ms=..`,
   then `moment=ended` about `declare_hold` s later. A room plate started before it that finished
   later logs `made, not shown, superseded by`.
3. Screen: Danny at his easel with the board on it and a brush in hand, the place behind him, the
   speech bubble with his words; no other plate replaces it for the hold. At 512x512, 20 steps, with
   the Forge queue idle the plate takes the studio's job time for one image plus the scan (the
   studio's own `queued_ms`, `render_ms` on `FORGE job kind=images` and the scan job lines); measure
   it from `made_ms` of the `PERSONA declared` line. Measured in roam runs 4 and 5 (2026-11-09):
   `made_ms` 55 to 80 s (once 127 s), `queue_wait_ms` under 50 ms, about 20 s on screen.

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

## Forge studio operations

The Forge studio is a separate application, not part of this repository: `C:\Users\mreca\Desktop\Game
Development Studio`, at `http://127.0.0.1:5173`. It has its own `AGENTS.md` (read it, and
`docs/STUDIO_GUIDE.md`, before changing anything there). This repository only talks to its HTTP
API (`crates/df-providers/src/forge.rs`). "Forge as the model provider" below says which concerns
Forge can serve; this section is how to keep it running.

**Start and stop** it with its own scripts, which track the process by PID and refuse to touch
anything that is not a verified studio (`-Port` for another port). Do not kill node by name.

```bash
powershell -NoProfile -File "C:\Users\mreca\Desktop\Game Development Studio\start-server.ps1"
```

```bash
powershell -NoProfile -File "C:\Users\mreca\Desktop\Game Development Studio\stop-server.ps1"
```

Start needs `node_modules` and `dist\index.html` (run `npm install` and `npm run build` in the
studio folder first). Stop asks for an orderly stop (it closes SQLite and checkpoints the queue)
and terminates the process only if it is still up after 8 s.

**One render at a time.** The studio's queue is durable and runs one render from its head (the
image engine holds a lock). Everything else waits behind it, the scan (SAM) jobs and the sheets
included, so a blocked head holds back every plate. List what is open:

```bash
curl -s http://127.0.0.1:5173/api/runtime/jobs
```

**What the host does when the queue is blocked** (`df-providers/src/forge.rs`; unit-tested in
`crates/df-providers/tests/forge.rs`):

- It reads the queue before it submits and again about 1 s later, then every 5 s while its job
  waits. A head that waits for memory (`waiting_memory`), was interrupted (`interrupted`) or needs a
  person (`needs_attention`) ends the render at once with a typed cause, `queue_waiting_memory`,
  `queue_interrupted` or `queue_needs_attention` (`FORGE job .. outcome=queue_*`; Forge's reasons
  show as `detail=` on the `IMAGEGEN` failed lines), instead of waiting out the 15-minute deadline.
  The read is advisory: a failed read never fails a render.
- It withdraws what it stopped waiting for: on any error while its job was last seen waiting (a
  passed deadline, a blocked queue, a failed poll) it cancels the job and reads it back to confirm.
  One line: `FORGE withdrawn outcome=cancelled|taken|refused|unreachable`. A job that started
  meanwhile finishes on its own and the line says so (`taken`).
- It never resubmits a job: one whose outcome is unknown (`interrupted`) is an error the caller
  sees.
- An emergency picture (a declared plate) is moved to the top of the studio's line right after
  the submit, once, when the job is still waiting: `FORGE prioritised .. outcome=moved|refused|
  unavailable`, by `POST /api/runtime/jobs/<id>/move` with `{"to": "top"}`. Seen working live
  (nine `moved` lines on 2026-11-09). See "Declared actions and emergency plates".

**Stalls seen on 2026-11-09, and what cured them**

| What was seen | Cause | Cure |
| --- | --- | --- |
| No plate comes; the head job of the list is `waiting_memory` | The studio's memory check: the render does not fit in the GPU memory that is free (a model another job loaded still sits there) | Free the memory: offload the idle engine (`POST /api/runtime/models/<id>/offload`, see the headers below) or from the studio's Models page. The studio's own automatic release (it stops one idle, studio-owned engine) is one of the fixes loaded only after a rebuild and restart (below) |
| The head job is `interrupted` and nothing behind it moves | The job's outcome is unknown (the studio or the engine stopped while it ran). The studio never retries it and the host never resubmits it | In the studio's queue page use "Mark cancelled after checking" once you have checked that the engine is no longer running that render. The deployed page asks with a native `confirm()`, which embedded browsers (an editor's or an agent's browser pane) suppress, so the click does nothing there: use an ordinary browser at `http://127.0.0.1:5173`. The same action is `POST /api/runtime/jobs/<id>/resolve` with `{"confirmed": true}` (read in `runtime/jobs.js`, not run) |
| A render that is running cannot be cancelled | The studio cancels only a job no engine has taken (`Only queued jobs can be cancelled here`) | Only the engine's Force stop (studio page) ends a running render. Check the job list afterwards |
| A cancel or a move by hand answers 403 `Local studio requests only.` | Every runtime write must come from the studio's own origin | Send `Origin: http://127.0.0.1:5173` (the same as the Host) and `X-Forge-Request: runtime` with it |

A by-hand cancel of a waiting job, as the host does it:

```bash
curl -s -X POST http://127.0.0.1:5173/api/runtime/jobs/<job id>/cancel -H "Origin: http://127.0.0.1:5173" -H "X-Forge-Request: runtime"
```

**The studio's own fixes are not in the running studio until it is rebuilt and restarted.** The
studio folder's working copy (2026-11-09, uncommitted there) has an automatic memory release that
may also stop the idle resident model of the render's own engine when it alone holds the memory
and the studio restarts it by itself, a memory wait that gives up after 30 minutes
(`memoryWaitLimitMs`), and a rewritten queue page (`src/RuntimeQueue.jsx`) without the native
`confirm()`. They load only after `npm run build` in the studio folder and a restart of the
studio with the two scripts above. Built, not verified live: no stall has been cured by them yet.

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
times (`FORGE job kind=voices outcome=completed queued_ms= render_ms=`); an emergency picture's
`FORGE prioritised job=<first 8 of the job id> outcome=moved|refused|unavailable` says whether the
studio put it first in its line (see "Declared actions and emergency plates").

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

Forge size and steps at runtime (2026-11-09): `media backend forge [size=<W>x<H>] [steps=<N>]` sets the
size (each side 1 to 16384, then rounded to a multiple of 32 and clamped to 32–2752 as every Forge
job is) and the steps (1–100, Forge's request bound; the studio's own default is 40) of every Forge
picture, whatever shape was asked, until the next `media backend` call, which clears what it does
not repeat. Without them nothing is sent and nothing changes. `media backend` and the
`IMAGEGEN switched` line show them (`size=512x512 steps=20`); Codex takes neither and refuses them.
The stored picture's own PNG size is what is recorded and read (`width_px`, `height_px`); the
provenance's `requested_*` is the set size. A smaller plate lowers the playground's zoom ceiling,
which is figured from the plate's real width. Observed in the studio's job records (o, 41
completed image jobs to 2026-11-09 21:46): 1664x928 at 40 steps took 103–226 s (mean 135 s) with no
pictures in, 160–259 s with 5 references, and 338–562 s with the negative prompt (guidance 4) and
5 references; 1024x1024 at 20 steps took 57–187 s (mean 88 s, ten jobs). 512x512 at 20 steps has
run since (roam runs 3 to 5, 2026-11-09): the studio's `render_ms` on the host's `FORGE job
kind=images` lines was 12–63 s (mean 36 s, 39 jobs of one host log, plates and 1:1 inserts
together), and a move's plate took 46–65 s all told with its scan (see "Free roam: the current run
procedure").

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
| `the transition failed: answer_refused: the model's design was refused: <rule>` (HTTP 502, host `MOVE refused verb=design error=answer_refused cause=<key> detail=<rule>`) | the design call's answer broke a check; the host asks again after every refusal (`MOVE retry verb=design attempt=<n> reason=<key> detail=<rule>`), sending back all the rules broken so far as the `refused_before` list, with no count of askings capped, and stops only when an answer stands or the design stage's deadline (`Deadlines::designing_ms`, 60 s) has passed; this refusal is the last one then, or the text service itself failed (`text_service`). The `cause` key says which: `bad_thing` (a thing id with capitals or spaces, or repeated), `invalid_shown_item` (the detail says which: an owner who holds nothing in `inventory`, such as a resident invented in the same answer; an item its owner does not hold, named by its name instead of its id; or the same item twice; the facts' `shown_item_choices` lists the pairs that are allowed), `bad_resident` (the detail says: not a persona id, such as a hyphen, space or capital; or already taken by a party member or another resident), `malformed_design` (a missing or blank field, a mover not placed once, or someone who does not move listed in `placements`). Only ids, types and references are checked: no length or count is refused | read the rule in `detail` (never the model's text); if the same key repeats, the instruction or the facts leave the model no way to meet it; add the case to `design_tests.rs` |
| `not moved: the movement names no destination: <reason>` on a line that does name one (a roam run, 2026-11-09) | Jev's first reading refused a bare place phrase or a thing in sight, and the line was not declared a move | `move act` declares its line a move and reads it a second time with the reason (see "Free roam"); `move say`/`move interpret` lines are undeclared and read once |
| OpenAI Responses `credit_balance_exhausted` | key has no credit | add credit or use `DF_TEXT_BACKEND=codex` |
| no plate comes in a roam, or a plate fails `queue_waiting_memory`, `queue_interrupted` or `queue_needs_attention` | the Forge studio's queue head is blocked | "Forge studio operations" |
| the page looks old after a rebuild, or a style change does not show | the page's package is `artifacts/playground/pkg` but its CSS and HTML are served from the scratch playground | rebuild and run wasm-bindgen, copy `theme.css` and `kit.css` into the scratch playground, reload ("Free roam: the current run procedure", step 4) |
| `cargo build` of the host says the exe is in use during a roam | the host is running from the roam lane's `df-table.exe` | stop your host by its command line (step 2), then build |
| `scene cast` does not list `danny` after showing `roam-00.png` | the plate's scan or his name is missing (a scratch rebuilt without the layout) | "Making the starting pictures" under "Free roam" |
| an `emotions.json` or persona edit has no effect | the host reads its scratch copy (`artifacts/tmp/persona-live/playground`), not `assets/` | edit both, or copy the file into the scratch copy; no restart needed |
| no line is voiced | the host was started without `DF_ELEVENLABS_API_KEY`, TTS is off (the first-run default; a restart keeps it as it was), or the page's audio is locked | export the key and restart; click TTS in the toolbar (the click also unlocks audio); `playground` shows `tts:` and `audio` |
| a place goes round and round, nobody leaves | the pace judge keeps reading it as not served | a middle place is left once every player has said about three lines there; read `PERSONA story_next served=` in the host log |
| an earlier story or a past run leaks into lines (Kael's arrest at the market) | personas keep what earlier stories and runs wrote into them | restore the cast's clean personas before the run (Play the quest, step 2) |
| a close look finds nothing, or names go unchecked | an old `story moves` saved its nameless view, blanking names and other scenes' exhibits | fixed at `f0bd97da`; restore the plot from a copy made before, or plot again |
| a SAM tag or bubble shows an id (`tavern_server`) | the plate was named before shown names, or the persona has no name | `persona set <id> display_name <Name>`, then `media locate label` (Names, above) |

## Coordinator practice during play

Owner, 2026-11-09: while a quest plays, the coordinator watches the TV, hands every fix to a
subagent rather than making it, and asks before reloading the page mid-scene. The other standing
rules, and the inspector who reads every run, are under "Standing rules" near the top.

## Known gaps

Reference sheets (`ref-<id>.png`) are made by hand for the characters that need them (the quest
cast, Danny); no code was found that makes one for a resident the design invents, who is painted
from the look alone. The design step is a model call (`POST /dev/move/design`) that may invent
residents; their ids are not checked against the persona routes' reserved words (`list`, `all`,
...). Picture framing is not controlled (a face can be cut off by the frame). Owed tests are
marked `TODO(DEV-12)`. The transition's Generating, Locating and Placing timings are written
after the plate call returns. `persona converse` has no topic shifts. Each plate run spends an
image generation and a fal scan.

**Dice gauntlet, state after run 5 (2026-11-10).** The 48-station beats ran unvoiced end to end
(46 rolled, none stuck or failed, every card held long enough, d6 to d100 and every check kind
rolled; a voiced 17-station subset ran before it). Open: (1) a declared ranged shot at a hostile
creature is still sometimes ruled `no_stakes` on the first ask (stations 33 and 34 of the full
run, the barn raider and the wolf in the re-runs) even after the judge instruction names what an
opponent is; the runner's one retry usually gets a roll, the barn raider did not. (2) A natural
1 or 20 has not yet shown its mark on a card (a natural 1 on an ability check shows none by rule).
(3) The healing line says "plus two" with no ability named, because the sheet's healing entry
and `ModifierLabel::Flat` carry no label. (4) The flourish-repetition fix (fresh image per
flourish, 16 lines of history) was written after the full run and is only seen in the ranged
re-run. (5) The card's runtime animation and real frame rate have not been viewed on the owner's
screen (the pane's clock stops when the window is hidden), and the visual refinement round 4 is
owed.

**H-1, "the host stops logging" (2026-11-10): CLOSED, not a host defect.** On the live host the log was
checked read-only after run 9: `latest` did not move for a `GET` and a `world inventory get`
because a successful read writes no line by design (`server::logs_served`: only a `/dev/` change or a
refusal is an `HTTP served` line). A request the host does log moved it at once (`GET
/dev/host/status` with `Host: probe.invalid` -> 403, `latest` 9081 -> 9083, two lines, and the
same two lines appeared in `host.log` in that request). Nothing was stuck: `EventLog` takes its lock
with `unwrap_or_else(PoisonError::into_inner)` and `push` has no gate, channel or cap that drops a
line; the echo had `echo_pending=0`, `echo_dropped=0`, `echo_writer_gone=false`. The page was
alive (visible, 78 animation frames a second) and had nothing new to report, and run 9's runner had
finished (its `play9.log` ends on the final show and the memory summary). So a log that "stopped"
means the driver stopped, not the log. To tell quiet from stuck, probe with a request the host
always logs: `curl -H 'Host: probe.invalid' localhost:<port>/dev/host/status` (answers 403, adds
`CANVAS reject cause=forbidden_request` and `HTTP refused ...`) and compare `log.latest` in
`/dev/host/status` before and after. A probe that does not move it would be a real H-1; none has
been seen: `host.log` grew to the end of runs 8 and 9 and through run 10's 15 moves, and the freeze
watch was stopped. A terminal that fails a write is also covered now (`sink.rs`): the first failure
is one `HOST log_sink_failed kind=..` line and every line after it is also appended to
`host-terminal.log` in the playground directory (unit-tested, never triggered live). Runs 6 and 7:
only `run6-host.log` survives (it ends on page `CLIENT gone` lines at
00:48); the runners' last lines (`play6.log` 00:43, `play7.log` 02:34) precede the host logs' last
lines by about 4 and 2 minutes, so those runs stalled or were stopped on the runner side; why is
not recorded.

Free roam (2026-11-09 and 2026-11-10, ten runs; the stamps of runs 1 to 5 are 11091737, 11091838,
11092018, 11092200 and 11092245, of runs 9 and 10 11100339 and 11100557; see "Run history"). The
numbers are the inspector's (`artifacts/tmp/inspector/`) and the runner's logs. Runs 1 to 5 ran at
`ce4b6c83` plus the uncommitted work of the day and runs 9 and 10 on later builds of that work
(the code is at `5bc540b7` now), so a later commit may already have changed what is listed as built.

Fixed, and seen working in a live run:
- **Close-ups.** Inserts on 0 of 19 live moves in run 1, 16 of 19 in run 2 (the judge's rule for
  a close look, `@roam_look` saying he steps up to the thing); 8 of 9 in run 4. Beige band and
  reframes gone once an insert was painted anew from the item's look (run 4).
- **Names in pictures.** The `Danny traveling` signboard of run 1 did not come back (run 2 on).
- **Tools used up.** Easel and brushes stay (`use: kept`, run 3).
- **Reference budget.** Over-listed packed items gone: 3 references a move and plates of 46 to
  64 s (run 3); in run 2 the plates with 4 to 8 references took 73 to 118 s.
- **Jev.** A declared move reads a bare place phrase, with a second reading: no Jev refusal in
  runs 3 and 4 (run 2 had one, run 1 one).
- **Design refusals.** Three in run 3, none in run 4 (the typed retry).
- **Blank studies.** Six of 17 paintings of run 3 were blank boards; the two paintings of run 4
  the inspector opened were real oil studies (the blank check and the second try).
- **Declared plate timing.** Shown at once, queue wait under 50 ms, a hold of 20 s, in every run
  from 3; `FORGE prioritised outcome=moved` seen 9 times in the host log.
- **Declared duplicates.** Nine of 17 declared plates in run 3 had two or three Dannys; one of 9
  in run 4, after the cast count, the negative prompt and the single front view; none of 17 in
  run 9 (the people check, `reason=cast_only attempt=1` on every one).
- **The declaration no longer uses the board up** (run 5): the finishing line does.
- **Perception.** Answers with a bad proposal for a new thing were refused whole in run 4 (4 of 9);
  from run 5 the bad proposal is dropped (`PERSONA thing_unadded`) and the sight and thought stay.
- **The page.** A 512x512 plate lies whole over a blurred copy of itself (contain), letterboxed,
  with the thinking cue and bubbles on it (run 4 screenshot). The inventory panel is hidden while a
  painting is revealed and returns matching the API (run 8 move 1).
- **Perceived things are named** (run 10: `[thing NN] stone_post (weathered milestone)`; run 9 still
  had three `(?)` inserts), and the walk, reveal and pace lines read as in the table above.
- **Trades apply whole.** A sale and its purchase net together and coins do not move on a barter:
  run 8 move 7, run 10 moves 3, 7 and 13 (painting 1 to 0, goods up; coins unchanged). The first
  board restock was run 10 move 7 (boards 7 to 12); oil came in on move 13 (2 to 3).
- **The end of the day** (run 9): the gallery sheet of all 12 paintings (3 by 4, square tiles, none
  blank), the show plate with the paintings laid out on a fountain and tables, the show's naming
  (no `unknown_region`) and the closing `[show]` line were all seen; the 5 s piece holds were exact.
- **Reveals and holds.** Painting 10 s and view 6 s exact in every reveal of runs 7 to 10; walk
  sights at least 5.99 s; declared plates shown at once and held 20.0 to 20.3 s.
- **The frame judge** gave no `frame_refused` in run 9 (run 8 move 5 had an `answer_refused` and no
  insert).

Built, not yet seen live (unit tests and the code only; check each in the next full run):
- **A judged line applies whole or not at all, and a refused line is asked again** with the typed
  refusals until it stands or the text deadline passes; a design call is asked again until it
  stands or the design deadline passes. Run 10 had 0 refusals, so no re-ask ran; run 9 move 3 still
  lost its design after one retry.
- **The show steps through every piece**: the reveal queue has no count cap and `@roam_show_each 20
  5` asks for 20 (run 9's cap of 4 showed 4 of 6, `queue_full`). Run 10 stopped before the show.
- **The show plate hides the inventory panel** (`PlateKind::Show`; run 9 left it over Danny, R9-7).
- **The frame judge asked again** once with a typed reason (`PERSONA frame_retried` /
  `frame_refused`): none occurred in run 9.
- **`HOST log_sink_failed` and `host-terminal.log`** (see H-1 above).
- **What a speaker attends to reaches the frame judge** (`perceive::attending`), and the object
  or vista `kind` of a new thing: in progress when this was written.
- **The host's early refusal and withdrawal** when the studio's queue is blocked (the `queue_*`
  causes, `FORGE withdrawn`): no such line is in today's host log.
- **The studio's own fixes** (auto memory release, the queue page): see "Forge studio operations".

Open:
- **Thoughts repeat a template.** The `[thought]` lines of run 5 open "Hm." or speak of a warm
  edge of light on cool stone; his looks open "I'll step up to ..." and "late light" came in
  about 15 of 20 moves of run 2.
- **Markets do not reliably offer the supplies he is out of.** Run 9 bought no board all run (move
  3 lost its design, move 13 refused `unsourced_gain` for a parcel the plate showed, move 19 lost its
  line, "Where shall I go next?", read `needs_clarification`); run 10 restocked boards only at move 7
  and oil at move 13, and used up its rags at move 15. A trade restocks only what the judge accepts
  from what the plate shows.
- **A finished painting can be sold to restock supplies**, which shrinks the show, and the show
  gallery and the memory list still count it (run 9: 12 pieces including the sold painting_05, the
  panel `Paintings x11`; run 10 sold paintings 02, 01 and 05). Owner decision pending: block sales
  of paintings, or show only what he owns.
- **Out-of-stock wording is inconsistent in the old build.** Run 9 moves 16, 18 and 20: the
  finishing line claimed a board and pigments he did not hold (move 16 also spent ochre and lamp
  black for no painting), while move 17 was honest. The atomic-use and typed re-ask fix is built;
  its live behaviour is not observed (run 10 was stopped at move 15, before supplies ran out).
- **The new plate stays up 10 to 14 s before he speaks (dwell)** against the 6 s asked for: the
  perception call is slower than the dwell (`[dwell]` lines of runs 9 and 10; run 7 9 to 23 s).
- **The milestone road opens move 1 in six runs** (runs 5 to 10), and the same market, handcart and
  blue-door lane come back; "the next town" never reads as a new town. The declared set-up shot
  repeats in composition (inspector R9-6).
- **Some moves skip a walk sight as `late`** because the arrival plate finishes first (run 9 moves
  2, 8, 12 and 17; run 10 moves 2 and 6).
- **Missed close-ups.** Run 9 had inserts on 15 of its 18 live moves, none where no thing was
  offered; run 5 move 8 offered three things and the judge named none (`inspected=-`).
- **Self-recognition.** The `[see]` line calls him a bearded man in the third person in run 4 move
  9, run 5 move 8 and run 9 move 7; the fix is built and run 10 was not checked for it.
- **The title chip is stale** (it read "Market: the enchanted-steel stall" through the roam), and
  stone inserts show a faint knitted texture.
- **One declared plate took 127 s** on Forge (run 5 move 2) against 55 to 80 s; the cause was not
  found. On Codex they take 59 to 138 s.
- **A narrator-box timing test is flaky** (the coordinator's report; its name and cause are not
  recorded here).
- **The studio's queue page** with the fix for an `interrupted` job is not deployed.

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
- **Still unverified** (the roam runs do not exercise them): the quest run's ending check (a
  confession, `end=true`), stage plates, and the arrival plate's re-scan from the earlier quest goal.

## Tempo and the music director

The tempo engine (`crates/df-tempo`, `presentation::advance_presentation`) keeps one journey's
intensity with its inertia, fatigue and phase (calm, build, anticipation, peak, release). The
host (`crates/df-table-host/src/tempo`) holds one such run, moves it at the host's clock when an
event is posted, and a music director plays the profile of a music table that the tempo calls for.
The engine has one intensity channel, not separate tension, energy and triumph channels (the plan's
channel list is not what `df-tempo` carries today), so the audit lines and views report
`intensity`, `inertia`, `momentum`, `target`, `fatigue` and the phase. Everything about style is
data: `development/market-threads/free-roam.music.json`. Nothing in the host names a story or a
sound. The runner posts the events it already has; it decides nothing.

**Routes and verbs** (each answers the shared dev envelope with a typed `TempoError`):

| Verb | Route | Does |
| --- | --- | --- |
| `tempo load <table.json>` | `POST /dev/tempo/table` (the file is the body) | checks and installs the table, starts a run at rest; what plays stays |
| `tempo prepare` | `POST /dev/tempo/prepare` | finds or composes every profile's track, one after another; the only place a track is made |
| `tempo reset` | `POST /dev/tempo/reset` | starts the journey again; the director plays the base profile |
| `tempo impulse <kind> [0-1000] [note=<text>]` | `POST /dev/tempo/impulse` `{kind, intensity?, note?}` | one event of the journey, at a strength in permille (1000 when left out) |
| `tempo tick` | `POST /dev/tempo/tick` | advances to now and lets the director decide |
| `tempo [state]` | `GET /dev/tempo` | where the tempo stands now (read only), the recent impulses, what plays |

Errors, with stable keys: `invalid_body`, `no_table` (load first), `unknown_impulse {kind}`,
`invalid_intensity`, `invalid_note`, `invalid_table {cause}` (`no_profiles`, `no_impulses`,
`too_many`, `invalid_policy`, `invalid_director`, `invalid_id`, `duplicate_profile`,
`duplicate_impulse`, `unknown_base`, `invalid_impulse`, `invalid_prompt`, `invalid_length`,
`invalid_volume`, `invalid_crossfade`, `invalid_condition` with `empty_phases`,
`invalid_intensity_range`, `invalid_recent` or `unknown_impulse_kind`), `engine {cause}` and
`stopped`. An unknown field or phase in the table file is `invalid_body` (cause `data`). The CLI
prints two lines for every verb that moves the run:

```
tempo phase=build intensity=300 inertia=261 momentum=rising target=100 fatigue=200 at_ms=63000 policy=tempo/1
music cue=journey track=tempo-journey-1a2b3c4d reused volume=22 | decision=hold reason=dwell wanted=road until_ms=151000
```

**The table file.** One JSON object, every field checked when it is loaded:

- `version`: the policy version the views report.
- `policy`: the numbers `df-tempo` runs on: `inertia_half_life_ms`, `decay_half_life_ms`,
  `fatigue_half_life_ms`, `fatigue_charge_permille`, `fatigue_attenuation_permille`, `thresholds`
  (where calm gives way to build, build to anticipation, anticipation to peak), `hysteresis_permille`,
  plus `rest_permille` (where the journey starts and rests) and `momentum_deadband_permille`.
- `director`: `min_dwell_ms` (how long a cue plays before another replaces it), `tick_ms` (how often
  the director decides when no impulse comes), `hold_margin_permille`, `track_prefix` and `target`
  (`tv`, `phone` or `both`).
- `impulses`: the kinds the journey posts, each `{kind, impulse_permille, target_permille,
  release}`: the push on the intensity at full strength (negative lowers it), the level the tempo
  settles toward after it, and whether it is a release.
- `base` and `profiles`, in priority order. A profile is `{id, when, prompt, length_seconds,
  volume_percent, loop, crossfade_ms}`. `when` is closed and every field in it must hold: `phases`
  (a list of phases), `intensity_min`, `intensity_max`, `momentum` (`rising`, `falling`, `stable`),
  `recent {kinds, within_ms}` (an impulse of one of the kinds that long ago or less). The tempo
  wants the first profile whose `when` holds, else the base; `{}` always holds.

**How a track change happens.** At each boundary (an impulse, `tempo tick`, `tempo reset`, the
director's own tick every `tick_ms`, or the end of a held dwell) the engine is advanced to the host
clock and the director decides. The playing cue changes only when its profile no longer applies
(its intensity bounds widened by `hold_margin_permille`, unless a profile before it in the table
now applies), it has played `min_dwell_ms`, and the wanted profile's track is in the library. A
change that has to wait for the dwell is `hold`, and the director wakes itself at the end of the
dwell; a track not in the library is `skip` (`TEMPO cue_skipped`), never awaited and never composed
during play. The page plays one music track at a time and has no crossfade, so the change is a dip
fade of scene changes: the old track in 3 volume steps down, the new track from a quarter of its
volume in 4 steps up, `crossfade_ms` of the profile in all (a profile with 0 changes in one step).
Each step is a `PLAYGROUND` line. A reset or a new table makes pending changes stale and they are
dropped (`TEMPO cue_dropped reason=stale`).

**Tracks.** A profile's library file is `<track_prefix>-<id>-<hash>.mp3`, the hash of its prompt and
length: the same prompt and length is found again (the music engine's reuse), a changed prompt or
length is a new track and a new composition. Volume and crossfade are not part of the name.

**The free-roam score** (`free-roam.music.json`): chill, inspiring instrumentals with no vocals,
G major, 84 to 92 BPM, fingerpicked acoustic guitar, soft felt piano and light strings, every
prompt saying it is one score (same key, same instruments, same tempo family) with no hard ending.
The roam's `@tempo` lines map events to kinds (arrival by `road`, `town` or `market`, `look`,
`declare`, `painted`, `trade`, `show`). In table order: `show` (a show within 15 min, a build or
higher phase; 75 s, volume 26), `market` (a trade or a market arrival within 150 s, intensity 150 or
more; 75 s, 25), `study` (a look, declaration or finished painting within 150 s, intensity at most
450; sparse piano, 60 s, 18), `road` (a road arrival within 150 s in build or anticipation; open and
rising, 90 s, 24), `town` (a town arrival, same; a little more motion, 75 s, 24) and the base
`journey` (calm or released, easy walking, 90 s, 22). A day: journey, then road or town on
arriving, study for the close look and the painting, back to journey once they have settled,
market on a trade, show last. On the scripted day of `tempo::cue_tests` (10 moves of 7 minutes)
there are 28 changes in 71 minutes, none closer than the dwell of 75 s.

**Costs.** `tempo prepare` on the shipped table composes 6 tracks, 465 s of music in all (75 + 75
+ 60 + 90 + 75 + 90), through the music engine's one ElevenLabs composer
(`DF_ELEVENLABS_API_KEY`, never printed): 6 `/v1/music` calls, composed one after another, each
stored in the media library as `tempo-<id>-<hash>.mp3`. A second `tempo prepare` composes
nothing (`6 reused`). The runner runs it at its start, so a roam that finds the tracks pays nothing.
Run it by hand first to see the timings: `$D dev --port 7879 tempo load
development/market-threads/free-roam.music.json`, then `$D dev --port 7879 tempo prepare`. It prints
a summary and one line per profile (`reused`, or `composed (.. bytes, took .. ms)`, or `failed
(<error key>)`); a failed profile is skipped during play and a second `tempo prepare` composes only it.

**Volume against the voice.** A spoken line plays as a scene sound at `DEFAULT_VOLUME` 80, music
at the profile's volume (18 to 26, about 30% of the voice), and both are scaled by the same master
volume of the viewer. The page has no ducking under speech (`crates/df-playground/src/audio.rs`
plays music and sounds as independent elements), so the music stays at about a third of the voice
instead: a gap to close if lines are lost under the strings.

**Danny's voice.** `voices.json` holds `"danny": "bIHbv24MWmeRgasZH58o"` in the scratch playground
(`artifacts/tmp/persona-live/playground/voices.json`) and now in `assets/playground/voices.json`
(an opaque ElevenLabs voice id, the same one the scratch file gives the village torchbearer).
`speech cast danny` would choose another from the supplier's list by his persona. A line is voiced
when the runner asks with `speak=true`, the playground's `tts` switch is on (the toolbar, or
`DF_RUN_TTS=on` for the run; the host remembers it across restarts and it starts off) and the
page's audio is unlocked by a click.

**Running it.** Rebuild the host in the roam lane (step 3 of the run procedure; the page is
unchanged). Then, with the host up and the keys exported as in step 6:

```bash
$D dev --port 7879 tempo load development/market-threads/free-roam.music.json
$D dev --port 7879 tempo prepare        # 6 compositions, 465 s of music, the first time only
$D dev --port 7879 tempo reset          # the journey starts: music cue=journey ...
```

The roam (step 15) does the same at its start. Its log shows `[music] prepared: 6 profiles: 6
reused, 0 composed ..`, `[tempo 00]` and `[music 00] cue=journey ..`, then per move `[tempo NN]
phase=.. intensity=..` and `[music NN] cue=<profile> track=<name> reused | decision=..` after the
arrival, the close look, the declaration, a finished painting, a trade and the show. The host log
has `TEMPO table`, `TEMPO impulse kind=.. strength=.. phase=.. intensity=.. inertia=.. momentum=..
target=.. fatigue=.. at_ms=..`, `TEMPO decision action=.. reason=.. wanted=.. playing=..`,
`TEMPO cue profile=.. track=.. volume=.. crossfade_ms=.. steps=.. from=..` and `TEMPO prepared
...`. The state is in the host's memory: a host restart needs `tempo load` again (the runner does
it). Owed: a listening check of the cues and their changes by ear, and the page-side crossfade and
ducking noted above.
