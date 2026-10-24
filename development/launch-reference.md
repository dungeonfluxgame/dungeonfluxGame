# Launch and configuration reference

Verified against the current Rust CLI and development scripts, 2026-10-23.
The native executable is `df-transport-fixture.exe` on Windows. The planned
account-sign-in desktop launcher is not implemented. The scripts below are
current development launch tools, not that future launcher.

## Native executable

Run from the repository root so relative content paths resolve consistently.
The executable reads process environment settings; it does **not** automatically
read `.env` or a general launcher JSON/TOML configuration file.

| Invocation | Behavior |
| --- | --- |
| `df-transport-fixture.exe --gameplay-demo <web_dir> <asset_dir>` | Run the interactive game server. `web_dir` contains generated `df_tools.js` and `df_tools_bg.wasm`; `asset_dir` normally points to `assets/ui`. PostgreSQL must already be running. |
| `--gameplay-demo <web_dir> <asset_dir> --qualification` | Fixed timed qualification run; not normal interactive launch. |
| `--roll-adventure <tables_dir> [seed]` | Inspect an adventure roll, including DM-only information. |
| `--roll-campaign <tables_dir> [seed]` | Inspect a generated campaign frame. |
| `--simulate-campaign <tables_dir> <seed>` | Inspect the campaign simulation. |
| `--campaign-run <id> <tables_dir> [seed]` | Select/create the campaign record and print its database selection. Does not start the HTTP game server itself. |
| `--campaign-status <id>` | Inspect a stored campaign record. |
| `dev --help` | List actual authenticated developer commands and flags. |
| `dev --server <url> --token-file <path> [--pretty] [--timeout <seconds>] <command> ...` | Call a running development server. Timeout is 1–600 seconds, default 60. No token contents are printed. |

Seeds use the native adventure parser: unsigned 64-bit decimal or `0x` hexadecimal.
Other native build/qualification-fixture modes are outside this gameplay reference.

## Normal shell launcher

`development/run-gameplay.sh` prepares PostgreSQL, optionally builds native/WASM
outputs, launches the server, and stops the PostgreSQL instance it started when
the owned game exits. It does not automatically open a browser.

| Flag | Meaning/default |
| --- | --- |
| `--help`, `-h` | Print usage and exit. |
| `--no-build` | Use existing native executable and generated web outputs. Default builds both. |
| `--print-config` | Nonsecret preflight without building, credential reads or database/provider activity. Supplied database names, campaign option conflicts, port aliases and dotenv guard are validated. Network discovery/native content validation have not run; output labels this explicitly. |
| `--lan` | Discover a LAN address; use wildcard listening only when a public URL/address is available. Default loopback. This is current tooling behavior; automatic LAN play is a future launcher decision. |
| `--new-game` | Allocate a fresh database and refuse an existing explicitly named database. Without a database argument, ordinary launch also generates a new timestamped name. |
| `--database <name>` | Resume/use the named database. Prefix `df_gameplay_demo_`, at most 63 bytes, ASCII letters/digits/underscores/hyphens. The existing native bare-prefix allowance is retained. |
| `--harbor` | Select the fixed harbor adventure. Otherwise the script defaults to random adventure generation. |
| `--seed <u64>` | Seed a random chapter. Existing databases resume their stored chapter. |
| `--campaign <id>` | Use a persistent campaign's selected database. Conflicts with `--new-game`, `--database`, `--harbor`, and `--seed`. |
| `--campaign-seed <u64>` | Campaign seed; requires `--campaign`. |
| `--status` | Show campaign status and exit; requires `--campaign`. |
| `--test-fight <1-4>` | Enable the existing test encounter setting. |
| `--test-foe <creature-id>` | Choose a compiled rules-table creature; requires the test-fight setting. |

Explicit mode flags update the corresponding environment settings; normal unset
settings receive the defaults below. Conflicting campaign flags refuse startup.
The preflight reports requested listen/LAN settings; it does not claim reachability
or complete server validation.

## Core environment and files

| Setting | Default and precedence |
| --- | --- |
| `DF_GAMEPLAY_DATABASE_PORT` | Canonical PostgreSQL port, default 55517. Decimal digits, 1024–65535. |
| `DF_GAMEPLAY_DEMO_DATABASE_PORT` | Legacy alias only; valid when canonical absent. If both exist, both must be valid and numerically equal, otherwise startup refuses. |
| `DF_GAMEPLAY_DEMO_DATABASE` | Native server database name. The shell launcher supplies its selected `--database`/campaign/generated name; direct native execution defaults to the historical local demo name. |
| `DF_GAMEPLAY_LISTEN` | `127.0.0.1:63309`; explicit environment takes precedence over LAN's proposed wildcard default. |
| `DF_GAMEPLAY_PUBLIC_URL` | Optional exact public browser origin for joining; no path/query/userinfo. |
| `DF_GAMEPLAY_ALLOWED_ORIGINS` | Optional comma-separated exact origins. Native validation requires explicit origin policy for non-loopback listening. |
| `DF_GAMEPLAY_LOAD_DOTENV` | Shell-only `on` by default; `off` skips every `.env` credential read. Other values refuse. Native executable does not read this file. |
| `DF_GAMEPLAY_DEV_CONTROL` | `off` unless exactly `on`; enables authenticated development endpoints and writes a private token file. |
| `DF_GAMEPLAY_DEV_CONTROL_PUBLIC` | Separate explicit `on` required to expose general dev control on non-loopback. Seat-preview ticket routes remain loopback-only. |
| `DF_GAMEPLAY_DEV_OVERRIDES` | `off`; separate opt-in for battle/NPC state edits. Preview driver uses ordinary validated actions. |
| `DF_GAMEPLAY_SESSION_SECONDS` | Unset means run until stopped; explicit native session bound must be valid. |
| `DF_GAMEPLAY_ADVENTURE` | Native default harbor; shell default random. Supports harbor/random/campaign/merchant-shop in the native resolver. |
| `DF_GAMEPLAY_PACK_DIR` | Shell defaults to absolute `content/harbor-journey`; native relative default is the same directory. |
| `DF_GAMEPLAY_ADVENTURE_TABLES` | Shell defaults to absolute `content/adventure-tables`. |
| `DUNGEONFLUX_ARTIFACT_ROOT` | Shell default `<repo>/artifacts`; contains caches/build/runtime output. |
| `DUNGEONFLUX_BUILD_ROOT` | `<artifact_root>/build/play`; native executable under `debug/`, generated browser package under `play-web/`. |
| `DUNGEONFLUX_TMP_ROOT` | `<artifact_root>/tmp/play`; temporary build staging. |
| `DUNGEONFLUX_PLAY_ROOT` | `<artifact_root>/tmp/play`; owned PG `pgdata`, log and per-launch stop directory. A different PG port requires a distinct play root to isolate clusters. |
| `DUNGEONFLUX_POSTGRES_BIN` | `<artifact_root>/build/postgres-g05/install/bin`. |

The shell sets `RUSTUP_HOME=<artifact_root>/cache/rustup`,
`CARGO_HOME=<artifact_root>/cache/cargo`, and the selected Cargo target directory.
`rust-toolchain.toml`, `.cargo/config.toml`, `Cargo.toml`, `Cargo.lock`, and
`rustfmt.toml` configure builds/formatting, not campaign startup. The current pinned
toolchain is 1.98.1; this change does not install or upgrade it.

The native content loaders read `content/harbor-journey/pack.json` and `tuning.json`,
adventure-table files, and the voice-casting content JSON when applicable.
`startup_contract.json` records qualification evidence; it is not a user-editable
launcher configuration loaded by the executable. Specialized voice, imagery,
video, speech, battle and music environment settings are documented in
[Development README](README.md); no provider is selected by this reference.

The normal shell may read only these credential names from ignored `.env` when
missing in the environment: `OPENAI_API_KEY`, `DF_JEV_API_KEY`, `FAL_KEY` (fallback
`DF_FAL_KEY`), and `ELEVENLABS_API_KEY`. Explicit nonempty environment values win.
Never paste values into commands, documentation or logs. A normal run with keys
can invoke configured providers. The developer preview below removes credentials
and disables these features deliberately.

## Direct developer screen entry

```powershell
python development/dev-gameplay.py --list-screens
python development/dev-gameplay.py --print-config
python development/dev-gameplay.py --screen creation --role player1
python development/dev-gameplay.py --screen opening --role display --no-build
```

On Windows the wrapper finds `C:/Program Files/Git/bin/bash.exe`; set `DF_DEV_BASH`
to another Git Bash executable when needed. For the normal shell launcher:

```powershell
& 'C:/Program Files/Git/bin/bash.exe' development/run-gameplay.sh --print-config
& 'C:/Program Files/Git/bin/bash.exe' development/run-gameplay.sh --harbor --no-build
```

| Preview flag | Default/behavior |
| --- | --- |
| `--help`, `-h` | Usage without launch. |
| `--list-screens` | Current target registry and limitations; no launch. |
| `--print-config` | Requested preview configuration; no launch. |
| `--screen <target>` | `creation`; join/room/creation/opening/dialogue/combat/complete/ended. |
| `--role <role>` | `player1`; player1/player2/display. Join supports player1/player2 (unjoined entry); room supports display only (shared lobby). Invalid combinations refuse before launch. |
| `--no-build` | Reuse existing native/WASM outputs; default builds. |
| `--no-open` | Print the URL without opening the default browser. |
| `--port <port>` | Loopback game port 63329. |
| `--database-port <port>` | Private PostgreSQL port 55527. Must differ from game port. |
| `--artifact-root <path>` | `<repo>/artifacts`; cache/tool location and retained development-run output. |
| `--build-root <path>` | `<artifact_root>/build/dev-screens`; separate from normal gameplay builds. |

Each preview creates a fresh database and unique
`<artifact_root>/tmp/dev-screens/<timestamp-pid>/play` cluster directory. Inherited
`DF_GAMEPLAY_*`, `DF_DEV_*`, `DF_JEV_*`, launch-root settings, and supported provider
credentials are removed from the child environment. The wrapper sets loopback-only
networking, `LOAD_DOTENV=off`, and storyteller/voice/imagery/plates/STT/music/home and
scene video off. It uses the fixed harbor content and existing art. It does not
bypass account authentication or generate missing future screens.

After actual `READY` and `DEV CONTROL` lines, the bounded `dev play --to <phase>`
driver joins the existing two development seats and performs real validated actions.
The current playable prototype still has two seats; the six-player refinement
is planned. Later targets can fail or time out; `phase.json` records the real result,
and failure does not pretend the phase was reached. Join/room need no play driver.

A player target receives a one-use link to an already joined development seat,
issued through bearer-authenticated `POST /dev/screens/ticket` and consumed at
`/gameplay/dev/screens/<ticket>`, inside the existing gameplay browser entry path. Maximum 16 pending links,
60-second lifetime. The game page saves the owner-issued scope and replaces its
URL with `/gameplay/player`, so normal reload uses existing seat recovery. Expired,
used or stale-run links refuse access. The long-lived dev control token never
appears in a URL. Preview pages use no-store and no-referrer headers.

## Stop, resume and evidence

Normal shell Ctrl-C requests the exact `<run_identity>.stop` marker named by the
server's `target.receipt` inside that invocation's unique stop directory. It never
truncates the directory or guesses another run's marker. The launcher waits up to
20 seconds before its existing owned-server hard-stop fallback. It only stops
PostgreSQL it started, and verifies a reused cluster's published port matches the
requested one. A hard stop can delay resume until the existing lease expires.

Resume normal play with the database name printed at launch:

```powershell
& 'C:/Program Files/Git/bin/bash.exe' development/run-gameplay.sh --no-build --harbor --database df_gameplay_demo_YOUR_SAVED_NAME
```

For a named campaign use `--no-build --campaign <id>`. The dev wrapper intentionally
starts fresh disposable previews, not saved campaigns. Ctrl-C first requests the
authenticated shutdown and falls back to the exact owned receipt. Its launch log
and phase result remain in that run's directory; private control/seat files live
under `development/runs/<database>/` per the existing server. Do not share those
files. This workflow does not create a GUI account launcher or alter saved games.

Focused checks:

```powershell
python development/test-launch-settings.py
python development/test-dev-gameplay.py
```

## Windows desktop icon and shortcut

See [Windows desktop entry](windows-desktop.md) for the executable icon, Windows SDK build requirements, shortcut installer flags, and the distinction between the current development console and the planned graphical account launcher.


## Current media generation defaults

Home, scene/kill and character-action video generation default **off**, independently
of whether provider credentials exist. Only explicit `on` opts in through
`DF_GAMEPLAY_HOME_VIDEO`, `DF_GAMEPLAY_SCENE_VIDEO`, or `DF_GAMEPLAY_CHARACTER_VIDEO`,
respectively. Unset/empty/`off` remain disabled; invalid flags refuse without echoing
them. Character generation also avoids video-only bookend image jobs while disabled.
Still portraits, canonical character images, visual references and scene plates
remain separate. Existing cached character clips are not deleted by these switches.

`DF_GAMEPLAY_IMAGERY_PROVIDER` unset/empty/`openai` selects OpenAI. Explicit `flux`
requires `FAL_KEY`; unknown values refuse. OpenAI was already the model default;
this changes automatic routing when both keys are present. The existing imagery
configuration still requires a usable OpenAI credential before routing, so this is
not a new promise that a fal-only environment works. No provider call is implied by
reading configuration or listing the planned species/class catalog.

### Explicit creator portraits

`DF_GAMEPLAY_CREATOR_PORTRAITS=on` enables the creator's explicit **Generate portrait / Regenerate portrait** actions; default `off`. The helper accepts `--creator-portraits` and requires inherited `OPENAI_API_KEY`, projected only as `DF_GAMEPLAY_CREATOR_OPENAI_KEY` in its child. It never reads `.env`; automatic scene/NPC imagery, music, video, speech and embedding providers stay keyless/off.

There is **no local-development portrait quota or session allowance**. Each fresh user-triggered operation uses the current appearance. Exact duplicates/polls read the same job; changed content under the same operation is refused. A network failure or unknown result is not automatically resubmitted. Portraits use1024×1536 medium quality, the existing configured OpenAI image model, a180-second request deadline, no image input, no judge or text rewrite. The spend ledger records usage without enforcing a dollar cap.

`DF_GAMEPLAY_CREATOR_OPENAI_KEY` is the native creator-only credential input and is never printed. Images use the existing imagery cache and a process-specific manifest under `artifacts/tmp/creator-portraits/`. Approved portrait key/digest/length/revision are retained with the canonical character; serving a saved image checks its recorded integrity and does not generate another image. Account billing/entitlements and shared scene-plate reference consumption remain separate work.
# Creator appearance classification and optional BFL route

Creator Generate/Regenerate classifies only the current public appearance using
the existing Jev `SystemOneChoiceProvider` (`jev-latest`, one closed choice), once inside the owned operation. It retains the exact
description. General appearances use OpenAI. Clothed adult nonsexual intimate anatomy customization uses FLUX for provider compatibility while retaining the restrained, nonsexual portrait prompt, including in restrained campaigns. This does not classify body proportions as sexual or unsafe. Jev selects the route before any image request; pending status names the selected provider and category. Supported nonsexual adult nudity and
fantasy violence select direct `flux-3-image` before any image dispatch. Sexualized
content, extreme gore, and mature requests with minors, ambiguous ages or real
people require revision. Anatomy, body shape, species, gender, disabilities and
healed scars alone are ordinary appearance details.

The alternate hosted API still moderates requests. BFL moderation stays at its
default `safety_tolerance=2`; grounding/search is disabled. Refusal, malformed output
or uncertain delivery never triggers another provider or automatic resubmission.
Previous portraits and public drafts are retained. No development usage quota exists.

Enable in the development helper with:

```text
python development/dev-gameplay.py --screen creation --creator-portraits --creator-alternate-bfl
```

The process must inherit `OPENAI_API_KEY`, `DF_JEV_API_KEY` and `BFL_API_KEY`. This helper does not
load `.env`; the normal launch environment or parent shell supplies credentials.
It strips general provider credentials and forwards only the dedicated
`DF_GAMEPLAY_CREATOR_OPENAI_KEY`, `DF_GAMEPLAY_CREATOR_JEV_API_KEY` and `DF_GAMEPLAY_CREATOR_BFL_KEY`.
An inherited `DF_JEV_BASE_URL` forwards only as `DF_GAMEPLAY_CREATOR_JEV_BASE_URL`;
otherwise the existing Jev default `https://api.typesafe.ai` applies. Other Jev
consumers remain disabled in the developer helper. Jev's existing 800 ms single-attempt
deadline remains in force, with no OpenAI text fallback. Unsupported or uncertain
choices dispatch no image. Classification attempts are counted separately; Jev
does not report supplier token/currency usage through this choice interface.
Without alternate
opt-in, mature routing reports unavailable without an image dispatch. Native EXE
startup recognizes `DF_GAMEPLAY_CREATOR_PORTRAITS=on` plus these dedicated keys;
ordinary imagery, story, voice and video settings remain independent.

Source check: 2026-10-23. The [BFL API schema](https://docs.bfl.ai/api-reference/utility/generate-an-image-with-flux-3)
documents the hosted route and organization-dependent moderation ceilings. The
[BFL usage policy](https://bfl.ai/legal/usage-policy) applies; configuring a key does
not prove every mature request is accepted. Appearance classification uses the
repository's existing closed-choice Jev wire contract, not ChatGPT/OpenAI text.
Real mature-content acceptance and actual supplier
charges remain unqualified until a scoped live check; offline fixtures are not
vendor acceptance evidence.

