# Runbook: the dev playground, personas, move transitions and plates

Verified live on 2026-11-07 against commit `19dfd7b4` (Windows ARM64, Git Bash). Every
recipe below was run end to end. This is a development runbook for the dev host, not the
shipped launcher. Plan: [scene-transitions.md](../planning/scene-transitions.md).

## What works

| Capability | Verb | Backend |
| --- | --- | --- |
| Persona coverage, ask, backfill, say, generate | `persona ...` | Codex text (Responses has failure handling only: the OpenAI key had no credit) |
| Characters talk to each other, paced | `persona converse` | Codex text |
| A character's line is read as a MOVE by Jev (a model call) | `move interpret\|design\|brief\|say\|act` | Codex text |
| Scene design, image prompt and character cards from the move | `move design\|brief` | pure code |
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

## Verify (browser pane)

The browser pane repaints only on navigation or resize, so read the page, then reload:
`document.querySelector('.rp-title-spin')` exists while a plate generates (the "Updating the
scene..." chip), the backdrop `src` becomes `/playground/media/<name>.png`, and `scene cast`
lists the characters found in it.

## The look (one place)

`BriefStyle::default()` in `crates/df-table-host/src/movement/brief.rs`: `direction` (the
look) and `writing_ban`. Every plate, including `persona converse plate=`, uses it. Today:
live-action TV drama still, photorealistic, 35mm, soap-opera lighting, Hollywood teal and
orange grade, real actors, not an illustration.

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
| `cargo build` fails to link | the host holds `df-table.exe` | stop your host first |
| whole party moves on "we" | Jev had no context | `move act` passes the situation as context; real play passes recent lines |
| OpenAI Responses `credit_balance_exhausted` | key has no credit | add credit or use `DF_TEXT_BACKEND=codex` |

## Known gaps

Reference portraits for character consistency are not built (cards carry job lines only).
The design step is a table of set kits, not the narrative engine. Owed tests are marked
`TODO(DEV-12)`. The transition's Generating, Locating and Placing timings are written after
the plate call returns. `persona converse` has no topic shifts. Each plate run spends an
image generation and a fal scan.
