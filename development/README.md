# Development plans and durable operational data

`workflow.sqlite3` contains the source-backed whole-project planning corpus using
five tables: features, tasks, dependencies, attempts and append-only devlog.
These are plans and review evidence; no Rust game or automated coordinator exists.
`quality.sqlite3` is the separate installed quality skill's hash/review/finding
cache, not an extra workflow table or a runtime telemetry database.

Preserve all databases, sidecars, backups and retained `evidence/`. They are outside
`artifacts/` and never eligible for stale-build cleanup. Live SQLite files are
local-only; scoped runners/query interfaces are future work. Back up a live WAL
DB with SQLite backup or a consistent snapshot, never just copy the main file
while writes are active.

Source files to track once Git exists:

- `schema.sql`: constraints, indexes, views and schema version.
- `backlog-catalog.json`: explicit per-case owners and expectations across all140 families.
- `expand-backlog.py`: deterministic final backlog refinement; candidate output only.
- `validate-plan-seed.py`: source/classification/payload/DAG preflight before queue or schema mutation.
- `plan-manifest.json`: deterministic plan IDs, complete briefs/models/source
  sections and fingerprints, dependencies and planned RPC service inventory.
- `refine-gap-plans.py`: coordinator-only idempotent source/brief refinement over
  the existing manifest; requires an explicit output path and never writes queue state.
- `operational-tasks.json`: separately authorized operational TODOs merged into
  fresh provisioning, including the blocked GoDaddy domain task.
- `queries.sql`: bounded read-only status/brief/review queries.
- `verify-plans.py`: read-only plan/source/coverage/DAG verification, with negative
  SQL probes on an isolated in-memory backup and retained verification evidence.
- `provision.py`: one-shot stdlib SQLite data provisioning/inspection tool.
  This is development tooling, not the application or Rust agent runner.
- `devlog.py`: parameterized append-only entry intake using a coordinator-supplied
  execution context. It validates task/attempt and immutable retry identity, with
  no queue state mutation. The context file is a local role contract; trusted
  runner identity/access binding remains future work.
- `evidence/planning-refinement-audit.json`: preserved actual critic attempts and
  append-only refinement observations, separate from reproducible planned rows.

The final `workflow.snapshot.sqlite3` was created using SQLite's backup API.
Refresh consistent backups as operational history grows; seeded plans alone do
not restore later attempt/devlog history.

From the project directory, run:

```sh
python3 development/provision.py --check-only
python3 development/verify-plans.py
sqlite3 -readonly development/workflow.sqlite3 < development/queries.sql
```

To restore/seed a database from its manifest, the trusted coordinator runs
`python3 development/provision.py`. IDs are upserted; existing queue status,
created timestamps, attempts and devlog are retained. Active plan ownership blocks
unsafe seed changes. New source assumptions require regenerated reviewed manifest
inputs, not a blind rerun of stale data. The bootstrap v1-to-v2 migration preserves
all original rows plus a durable backup and refuses active/history-bearing queues;
future schema migrations need explicit review. This tool never dispatches work.

A dependency-ready blueprint is not dispatch-ready. Freeze the next bounded task's
scope, paths, actual checks, contracts, limits and required tool capabilities before
claiming it. Feature/slice expansions share one canonical implementation child;
slice composition owns wiring only. See `../planning/plan-database.md`.

## Minute screenshot journal

See [Screenshot journey](screenshot-journey.md) for the working public-preview
recorder, active start/renew/stop lifecycle and heartbeat recovery. Actual images,
source/time/task metadata and devlog payloads are protected in `evidence/journey/`.
The first baseline captures the static project site, not a playable game. The
user-facing timeline lives at `../../outputs/dungeonflux-visual-journey.html`.
Keep browser caches/profiles in `../artifacts/tmp/journey/`, preserve retained
history, and stop the recorder at the end of an active development run.

## Detailed backlog seed pipeline

The full-project catalogue is durable and source-hashed. The standard
`refine-gap-plans.py` automatically applies `expand-backlog.py` last when the
catalogue exists; older refinement cannot silently reduce the reviewed queue to
coarse plans. Use explicit candidate outputs and independently review them before
the coordinator replaces seed files and runs provisioning.

```sh
python3 development/refine-gap-plans.py --output artifacts/tmp/plan-candidate.json --operational-output artifacts/tmp/operational-candidate.json
```

For a staging directory containing the reviewed manifest, operational manifest,
schema and a SQLite-backup copy of the baseline, use
`python3 development/provision.py --directory <stage> --root <project>` and
`python3 development/verify-plans.py --directory <stage> --root <project> --output <retained-proof.json>`.
These commands do not imply approval to mutate the authoritative queue. Preserve
all baseline execution fields and history, plus intake dependencies outside the
seed. No planning seed enables dispatch. Only frozen `atomic`/`operational` roles
can enter the worker selection view; aggregates use a separate coordinator-owned
evidence-review attempt under the same integration triggers.

## Play the game locally

Run `development/run-gameplay.sh` to build and start the local game server and
PostgreSQL. Open the `display` URL from the server's `READY` line, then open the
`player` URL in two browser tabs to play both seats. Press Ctrl-C to stop the
server; the launcher also stops PostgreSQL when it started it. To resume a saved
game, use `development/run-gameplay.sh --no-build --database <name>` with the
database name printed by the launcher.

Each run has its own room code, derived from its database name so a restart of the same
game keeps it. The `READY` line prints it (`player=...?room=<code>`), the display's join
panel shows it, and the join form and the `?room=` link accept it.
`DF_GAMEPLAY_ROOM_CODE=<1-32 letters or digits>` pins a code, for tests. Slow machines:
`DF_GAMEPLAY_COMMIT_SECONDS` (default 12) bounds a database commit and
`DF_GAMEPLAY_REPLY_SECONDS` (default 20, must exceed it) bounds a request's wait for the game
owner; the browser's request timeout is served from the latter. If a commit's outcome is
unknown the server prints `SESSION fenced ...`, retries by itself and prints
`SESSION recovered`; `SESSION lost` means the saved state needs a server restart.
Browser checks for these live in `development/browser-checks/`.

Edit `content/harbor-journey/tuning.json` to tune hero AC, proficiency, base HP,
ability arrays/background bonuses, Second Wind and weapon dice; bandit stats,
attack/damage and weapon dice; critical threshold, round ticks, combat log depth,
Short Rest seconds and narrative intervention budget. `pack.json` supplies scene
text, offer labels, creation options, equipment, room code and the private courier clue.
Restart to apply changes; re-run without `--database` for a fresh game.
`DF_GAMEPLAY_PACK_DIR` selects another directory containing both required files.
`DF_GAMEPLAY_CHAPTER_TYPE=<chapter type id>` (for example `mystery` or `parley`) forces the
chapter type of a random adventure, to reach a scene flow for testing. It needs
`DF_GAMEPLAY_ADVENTURE=random`, refuses other modes and unknown ids, and is unset by default;
campaign chapters choose their own types.
The startup line prints both file revisions; departures from SRD values show
`CUSTOM RULES` there and a `+custom-<revision>` suffix on the in-game rules label.
Saved games refuse changed revisions; start a new database after editing either file.

### Phones on the network (`--lan`, join QR code)

`development/run-gameplay.sh --lan` listens on every interface (`DF_GAMEPLAY_LISTEN=0.0.0.0:63309`
unless you set it) and works out this machine's LAN address (`ipconfig getifaddr en0`, then
`en1`). Phones then join through `http://<that address>:63309`. If no LAN address is found the
server stays on loopback and says so. Without `--lan` nothing changes: the join link is on this
machine only. PostgreSQL stays bound to loopback either way.

The home screen's "Players join here" panel shows a QR code beside the room code. Scanning it
opens the join page with the room already filled in; the room code stays on screen for typing.
During play the shared display shows no join chrome, so late joiners use the room code.
The server's startup prints `JOIN url=<join link> qr=/gameplay/join-qr.svg`; the QR code itself
is served at `/gameplay/join-qr.svg`.

The setting behind it, for another address (a router name, or a machine you reach differently):

- `DF_GAMEPLAY_PUBLIC_URL=http://<host>[:port]` is the base address phones use. It must be
  `http` or `https` with a host and an optional port, and nothing after the port (no path, query
  or fragment). Anything else refuses startup. Unset, the base is the listener's own address,
  and a wildcard listener (`0.0.0.0`) is reached through loopback. The public origin is an
  allowed origin, so a phone on that address is accepted exactly as the loopback origin is.
  `DF_GAMEPLAY_ALLOWED_ORIGINS` still applies for other origins.

```sh
DF_GAMEPLAY_PUBLIC_URL=http://192.168.0.24:63309 DF_GAMEPLAY_LISTEN=0.0.0.0:63309 development/run-gameplay.sh
```

Who may join: the local preview has no accounts. Anyone on the same network who has the room
code can join and take a seat. This is a local demonstration, so keep it to a network you trust
and treat the room code as the only password.

Speech input needs HTTPS or `localhost`: browsers grant the microphone only there. On a plain
`http://` LAN address a phone can type its lines but cannot speak them (see Speech input below).

### Table settings (choice chips on the TV)

Any seated player can change the room's table settings from the Menu sheet on their phone, in the
lobby or during play. The server owns them: the choice is stored with the game's database, so it
survives a reconnect and a server restart, and every screen receives it in its view.

- `DF_GAMEPLAY_CHOICE_HINTS=on|off`: the startup value of "Show choices on the TV". Default `on`.
  Any other value refuses startup. A choice a player has already stored wins over this default;
  the startup line reads `TABLE choice_hints=on` or `TABLE choice_hints=off`.

With the chips off the shared display hides the numbered chip row and gives its space to the
narrator box. The setting changes presentation only; no rule, roll or outcome depends on it.

### Content rating (mature by default)

This is an adult game, so the written and spoken content is `mature` unless you switch it:
`DF_GAMEPLAY_CONTENT_RATING=mature` (the default when unset) or `restrained`. Any other value
refuses startup. The startup line reads `CONTENT rating=mature` or `CONTENT rating=restrained`.
`run-gameplay.sh` passes the variable through when you set it; it sets nothing itself.

```sh
DF_GAMEPLAY_CONTENT_RATING=restrained development/run-gameplay.sh
```

- `mature`: the Dungeon Master runs a table of adults. Violence can be bloody and its
  consequences described, horror can be horrifying, villains can be cruel, interrogation and
  torture may appear as dark fiction, characters may swear when it fits them, and romance and
  seduction are played as adult scenes (an NPC still answers by its own temperament: a devout or
  uninterested one declines, an amused one plays along, an interested one reciprocates). The
  fight, victory and defeat prose, the speaking antagonists' lines, and the NPC flirting, threat
  and curt reactions come from the mature tables (`content/adventure-tables/mature-overlay.json`).
- `restrained`: the earlier behaviour, unchanged. Tense heroic fantasy, no gore, no strong
  language, nothing explicit; the shipped tables are used as written and a seed produces the same
  bytes it always did.

What neither mode changes: no sexual content involving minors or any character presented as
underage (the tables' `young` characters are young adults, and the model is told so); no sexual
content about real people (a character flagged as based on a real person's likeness is excluded);
what players say is untrusted input in both modes; all secret-safety rules (a model reply that
names a secret term is refused in both); the game rules and mechanics; and the text model's own
policy. A refusal or empty reply falls back to the prepared in-character line and is never
retried with reworded instructions to get around it. The setting is read once at startup. A saved
adventure keeps the text it was generated with.

### Speech input (tap to talk)

On the phone, a "Tap to talk" button sits under the dialogue text box. The player taps it
(pointer, touch, or Space/Enter) to start; the button reads "Listening… tap to stop" while it
listens, for up to 15 seconds. A second tap stops it, and the server transcribes the clip and the
words appear in the same text box. Listening also ends without a tap when the page is hidden, the
connection drops, the text box goes away, or the player starts typing. A press that is cancelled
or dragged off the button does nothing, and Escape cancels. The player edits the words and presses
Send (the same path as a typed line) or Clear. Typing works throughout, and a tap is the only thing
that ever asks the browser for the microphone.

Settings (process environment; invalid values refuse startup without echoing them):

- `ELEVENLABS_API_KEY`: the same key as spoken narration; `run-gameplay.sh` exports it from
  `.env`. Without it speech input is off and the button is not shown.
- `DF_GAMEPLAY_STT=off`: switch speech input off. The startup line reads `STT on model=<id>
  budget=<seconds>s` or `STT off (<reason>)`.
- `DF_GAMEPLAY_STT_SECONDS_BUDGET`: audio seconds the server may transcribe in one run
  (default 600). Each clip is charged, rounded up to whole seconds, before the provider call
  and never refunded; when it is spent the phone says so and typing continues.
- `DF_GAMEPLAY_STT_MODEL`: default and only accepted value today is `scribe_v2`, the model the
  registered ElevenLabs speech-to-text route sends.

Browsers only grant the microphone on `https` or `localhost`. A phone that joins over plain
`http://<this-machine-ip>:63309` cannot use it; the button then explains that instead of
failing silently. To use speech input from a phone on the network, put an HTTPS front or a
tunnel in front of the server and set `DF_GAMEPLAY_ALLOWED_ORIGINS` to that `https://` origin.

Privacy: the clip is decoded to 16 kHz mono in the page and sent in one request to
`POST /gameplay/speech`, authenticated with the player's room binding. The server holds the
audio and the transcript in memory for the length of that request only, and returns the
words to the player who sent them. Neither is written to disk, cached, logged or put in a
telemetry span (which records sizes, durations and an outcome label). Only the words are
used: nothing is derived from the voice, and nothing is kept. Microphone tracks are stopped
as soon as the recording ends.

### Home video (the lobby's loop)

The shared display's home screen starts with its still and, once a loop is ready, fades in a
five-second ambient loop made from that still by fal (`fal-ai/kling-video/o1/image-to-video`).
Settings, read by `DF_GAMEPLAY_*` and `FAL_KEY`:

- `FAL_KEY`: the fal credential. `run-gameplay.sh` exports it from the ignored `.env` the way it
  exports the other keys, and never prints it. Without it the feature is off.
- `DF_GAMEPLAY_HOME_VIDEO=on|off`: default on when a key is present.
- `DF_GAMEPLAY_HOME_VIDEO_MODEL`: the endpoint. Default `fal-ai/kling-video/o1/image-to-video`;
  any other value is refused at startup, since the adapter has no request shape for it.
- `DF_GAMEPLAY_VIDEO_BUDGET`: clips the run may request, default 4, at most 1000. Each clip is
  charged before its submit and never refunded; a cached loop costs nothing.
- `DF_GAMEPLAY_HOME_VIDEO_CACHE`: where loops are cached, by content hash. Default
  `artifacts/tmp/play/home-video-cache`.

The startup line says what was decided, for example `HOME VIDEO on model=... budget=4` or
`HOME VIDEO off (no FAL_KEY)`. Each paid call is logged with its time, model and price. Nothing
waits on the video: a pending, failed, or refused loop leaves the still on screen.

## Real Gaussian-splat scans as battle scenes

Real photogrammetry scans (streamed level-of-detail SOG scenes from superspl.at: `lod-meta.json`
plus `<level>_<chunk>/` folders of WebP planes) can stage a fight. Scan data is large and some
scans carry no licence, so none of it is ever committed: keep it under the gitignored
`artifacts/` folder (for example `artifacts/tmp/splat-dl/<scene-id>/v1/...`, exactly as
downloaded) and point the server at it.

- `DF_GAMEPLAY_SPLAT_DIR=<directory>`: where the server serves `/assets/splat/...` from. Default
  `assets/splat`. Relative paths are taken from the current directory; an empty value or one
  with `..` steps refuses startup. Only regular files (no symlinks) up to three directories
  deep, lowercase `[a-z0-9._-]` names with a `.json`, `.webp` or `.splat` extension, at most
  128 MiB each and 4096 files in all, are served.
- `DF_GAMEPLAY_BATTLE_SCENE=<scene id>`: stage every live fight on that scene instead of the
  per-location scenes. Unset keeps today's behaviour. The id must be a built-in scene (see
  `content/battle-scenes/`); scenes marked `"distribution": "private-test-only"` can only be
  reached this way (or by the `#battle-preview` address below), never from the default
  rotation. If the scan cannot be drawn (files missing, decode failure, a frame budget that
  cannot be met) the location's own painted ground and backdrop are shown instead.
- `DF_GAMEPLAY_BATTLE_FPS=<24|30>`: the battlefield's fixed presentation rate; default 30, never
  higher. Any other value refuses startup. The page asks for a frame at the display's refresh
  rate and the battlefield presents only on the ticks an even-interval accumulator selects, so a
  skipped tick does no update and no GPU work. Simulation time stays real seconds (walks, camera
  moves, dice, projectiles and sound timing are unchanged); only the rate of presentation is
  capped. On a 60 Hz display 30 fps is every second tick exactly and 24 fps alternates two and
  three ticks (the 3:2 pattern a 60 Hz display forces); on 120 Hz both are exact; on 144 Hz 24
  fps is exact and 30 fps mixes four and five ticks. If the page cannot keep the cadence it drops
  to the next even divisor (15, 10, 7.5 fps for the 30 setting) and tries the faster rate again
  later. `data-battlefield-stats` reports `fps_target`, `fps_presented`, `fps_divisor`,
  `display_hz` and `cost_ms`. The `#battle-preview` address takes `&fps=24` or `&fps=30`.
- `DF_GAMEPLAY_SPLAT_LOD=<0-3>`: which tier of a streamed scan the battlefield draws as its finest;
  default 1. Every scan is stored in tiers (level 0 the most splats, each next level about half as
  many); the scene file authors the largest tier worth drawing for it (`levels`), which is level 0
  here, and level 1 is the next one down (about half the splats, a lighter and faster load). A
  tier never goes past the scene's coarsest one. Any other value
  refuses startup. The `#battle-preview` address takes `&lod=<0-3>`.

### Test fights against 1 to n enemies

A development setting, off by default (a normal game is unchanged): the adventure's first choice
leads straight to a tactical fight against `n` enemies.

- `DF_GAMEPLAY_TEST_FIGHT=<n>` (or `development/run-gameplay.sh --test-fight <n>`): `n` from 1 to
  4. Any other value refuses startup. After the opening's first choice (ask or escort) the server
  submits the courier defence for the same player at once, so the fight begins without the
  dialogue step; every rule, check and revision is the ordinary one.
- `DF_GAMEPLAY_TEST_FOE=<creature id>` (or `--test-foe <id>`): the creature, from the compiled
  rules table (`bandit`, `cultist`, `guard`, `goblin-warrior`, `kobold-warrior`, `skeleton`,
  `zombie`, `wolf`, `giant-rat`, `giant-weasel`, `mastiff`, `scout`). Default: the adventure's
  own creature. Needs `DF_GAMEPLAY_TEST_FIGHT`; an unknown id refuses startup. A creature that is
  not the adventure's own carries 20 arrows or bolts when it has a bow or crossbow.
- The startup line `TEST FIGHT enemies=<n> foe=<id>` confirms it. It applies to databases created
  while it is set; a stored game keeps the enemies it started with.
- The limit of 4 is the number of enemy entities the server holds and of enemy spawn cells on
  every layout of every battle scene (13 scenes, including the four scans); a native test starts
  a fight against 1, 2, 3 and 4 on every scene. At six heroes against six enemies several things
  break and none is changed here: the server numbers heroes 1 and 2 and enemies from 3, there are
  four enemy entities, four spawn cells per side on every layout, the participant admission allows
  six combatants (and 15 identity comparisons), the pack and adventure tables allow at most four
  enemies, and the flat display fallback holds two heroes and three non-player figures. A larger
  fight needs all of these raised together, and new spawn cells authored on all 36 layouts.

Run a real fight on a scan (the scans listed in `artifacts/tmp/splat-dl/README.txt`):

```sh
DF_GAMEPLAY_SPLAT_DIR=artifacts/tmp/splat-dl DF_GAMEPLAY_BATTLE_SCENE=scan-village \
  development/run-gameplay.sh
```

The grid on every scene (scans and painted grounds alike) is drawn as straight hairlines (about one pixel at
1080p, low contrast, no glow) on one flat play plane: warm gold when neutral, cool blue on hero cells and red on
enemy cells, fading with distance and toward the edge of the field. A hero's reachable region is one faint tint
over its cells with a single thin outline around its outer edge (the lines inside it keep the base strength);
legal targets and the acting figure's cell are thin rings, the only strong marks on the field. Where scan
geometry stands well in front of a line the line dims instead of breaking. The look is tuned in one place,
`crates/df-render/src/battle/grid_lines.rs`. A scan's plane sits at the median ground height under the
open cells and figures stand on it; cells whose ground is more than 0.45 m from the plane are
blocked at fitting time (`--deviation`), so nobody floats or sinks further than that.

The camera rests on a solved wide shot (`crates/df-render/src/battle/rest_shot.rs`): from the combatants, the
cells the server offers this turn and the scene's authored `tactical` shot as a hint for direction and lens, it
finds a raised three-quarter view that holds every living figure and the reachable region between the HUD
cards (the party and enemy columns, the round strip and the dice card are keep-out rectangles) with the place
showing beyond the grid. A turn begins with a short lean toward its figure and back out within 1.5 s; strikes,
projectiles, speakers and the finishing shot keep their own shots. `data-battlefield-camera` reports the shot
held (`wide` or `action`) and its pose.

Scan splats that are beside or behind a figure never hide it (`figure_clear.rs`, applied per splat in the
projection pass): only a splat centred in front of the figure over its screen box hides it, and where scan
geometry truly stands between the camera and a figure the figure is drawn faintly over it so it still reads.

The scenes are `scan-riverbed`, `scan-valley-bridges` and `scan-summit` (private test only, no
licence: do not publish, record or ship them) and `scan-village` (CC BY 4.0, attribution in
`THIRD_PARTY_NOTICES.md`). To look at one without playing, open
`/gameplay/display#battle-preview=scan-village`; useful additions are `&hold` (stage the figures
and stop), `&shot=<tactical|combat_est|turn_focus|impact|ko|victory>`, `&orbit`, a free camera
`&cam=<target x,y,z>,<distance>,<yaw>,<pitch>,<fov>` and, for benchmarks, `&fixed` with
`&budget=<splats>`, `&minc=<min contribution>`, `&sscale=<0.25-1>` and `&rscale=<0.25-1>`; `&fps=<24|30>` sets the
presentation rate, and `&dbg=<bits>` (with `&fixed`) turns on debugging views: 1 draws the scan without writing
its depth (shows what occlusion hides), 2 skips the scan, 4 skips the post pass, 8 shows the scan's depth as grey, 16 turns off the figures' ground margin.
Battle feel (camera shake, hit-stop, slow motion, handheld sway, blur, particles; all constants in
`crates/df-render/src/battle/feel.rs` and `particles.rs`, the attack framing rules in `framing.rs`) can be
reviewed with `&beat=graze|heavy|critical|miss|finish|selfheal|enemyhit` (stages one blow after the camera has
idled), `&slow=0.4` (the stage clock at that share of real time, for frame strips), `&tempo=0..1000` (the tempo
intensity the sway follows) and `&feel=off` (everything off, for measurements).

### The `#battle-preview` address

Open `/gameplay/display#battle-preview=<scene>[:<layout>][&option...]` (on the gameplay server, or
any page that carries the display) to look at the battlefield without playing. Options are joined
with `&` and may come in any order:

| Option | Meaning |
| --- | --- |
| `<scene>` or `<scene>:<layout>` | A built-in scene id (`scan-village`, `salt-quay`, ...) and optionally one of its layouts; with no layout the preview cycles through them. |
| `heroes=<n>` | How many heroes stand on the field (default 3: Tobin the fighter, Sylvie the archer, Nimrael; more repeat those figures), 1 up to what the layout seats. |
| `enemies=<n>` | How many goblins stand opposite (default 1), 1 up to what the layout seats. |
| `hold` | Stage the figures and stop (no scripted fight). With `orbit` the camera turns slowly. |
| `shot=<name>` | Cut to a named camera: `tactical`, `combat_est`, `turn_focus`, `impact`, `ko`, `victory`. |
| `cam=<x,y,z>,<distance>,<yaw>,<pitch>,<fov>` | A free camera. |
| `place=<token>:<col>:<row>` | Stand a token on a cell (repeatable). |
| `reach` | Show the first hero's full reach (every walkable cell within six, 30 ft) as reachable cells and every foe as a target, as on a hero's turn. |
| `beat=<kind>` | Stage one thing after the camera has idled, then hold: `rest` (a hero's turn begins: the lean toward it, then the solved resting shot), `graze`, `heavy`, `critical`, `miss`, `finish` (one blow of that kind), `selfheal`, `enemyhit`, `shoot` (the archer across the field), `speak` (an enemy's line, then a hero's, each with its bubble and the camera easing in and back), `named`, `namedmiss`, `namedcrit` (a blow whose floating result names the attack), `chain` (every enemy strikes in a row: stacked results), `victory` (every foe brought down: the held victory beat), `defeat` (every hero brought down). |
| `lod=<0-3>` | The scan tier drawn as the finest (default 1; 0 is the largest). |
| `splat` | Replace the scene's scan with the built-in procedural ground. |
| `sog=<name>`, `splatfile=<name>` | A splat scene served from `/assets/splat/`. |
| `fixed`, `budget=<n>`, `minc=<x>`, `sscale=<s>`, `rscale=<s>`, `dbg=<bits>` | Hold the frame-quality ladder for benchmarks and switch debugging views. |
| `fps=24` or `fps=30` | The presentation rate. |
| `slow=<share>` | Run the stage clock at that share of real time, for frame strips. |
| `tempo=<0-1000>` | The tempo intensity the camera sway follows. |
| `feel=off` | Battle feel (shake, hit-stop, blur, particles) off, for measurements. |
| `restrained` | The restrained content rating. |
| `statuses`, `sheet`, `sheet=<page>`, `still`, `close` | Review modes: the status cycle, contact sheets of figures with statuses, the layout still (`close` frames it nearer). |

The battlefield data attributes (`data-battlefield-splash`, `-stats`, `-load`, `-figures`, `-demo`, `-line`,
`-impact`, ...) on the host element tell a harness what is on screen.

The loading splash holds over the battlefield (place, mark and a bar driven by the real chunk count) until the
scan at the chosen tier is fully resolved, its art has arrived and the figures are placed; the stage clock waits
behind it, so nothing of the fight plays early; then it fades out over the finished scene. The display's battle
machine reads `live_battlefield::scene_resolved()`; a lost scan never holds longer than 45 seconds.

Fitting a new scan or area is done with the native authoring tool, which reads the finest
on-disk level, estimates the ground height field from the splats and marks cells a figure cannot
stand in (it decodes WebP with macOS `sips`):

```sh
cargo run --release -p df-render --example splat_fit -- <scan>/v1 out.json \
  --orient 180,0,0 --scale 1 --offset-y 0 --translate 0,0,0 --grid -7,-4.9,1.4,10,7 \
  --finest 1 [--deviation 0.45] [--search -45,45,-45,45,3]
```

`--search` slides the grid over a region and ranks open, flat windows; the printed `terrain` and
`blocked` data go into the scene file by hand. The real-data decode tests run when
`DF_SPLAT_SCAN_DIR` (or `artifacts/tmp/splat-dl` beside the crate) holds the scans and skip
otherwise.
