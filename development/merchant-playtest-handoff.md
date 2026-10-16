# Merchant playtest handoff

## Follow-up: Sigrun haggle, 2026-10-16

The game was restarted at the user's request after this handoff. Current live
room is `ZAVAFO`, database `df_gameplay_demo_play_20261016220527_1826`,
build `commerce-sigrun-v6`, tested source `30f249a9`.
The server remains running. Browser tab 3 controls Sigrun; its reclaim revoked
her CLI seat credential. Tab 2 still points at the preserved older room.

Sigrun asked the price, offered 150 CP, then 180 CP. Marek refused both discounts
and kept the listed 200 CP. Her explicit 2-GP purchase committed at revision
1:8. Her phone visibly shows 16 GP and permanent `item-clothes-travelers`.
The initial 200-CP quote was durably closed as hero_declined during haggling.
Display screenshots were inspected after every interaction. This proves a
haggle/refusal followed by a full-price purchase, not an accepted discounted sale.

**Resume correction:** launching saved ZOGAMU with the newer build label rejected
`Checkpoint(BuildMismatch)`. Its database was preserved; the fresh room above
was used. Checkpoint admission pins source/native/WASM/configuration/content.
The earlier resume recipe is conditional on those exact pins matching; changing
`DF_FIXTURE_BUILD` and rebuilding is not a migration. Do not falsify build pins.
To resume the new database with its existing compiled artifacts, use the launch
flags below with `DF_FIXTURE_BUILD=commerce-sigrun-v6` and
`--no-build --database df_gameplay_demo_play_20261016220527_1826`.
Do not start a second server while the current one occupies port 63319.

Evidence: `outputs/merchant-shop-scene/sigrun-v6-*.json`,
`commerce-sigrun-v6-build-startup.txt` (resume rejection) and
`commerce-sigrun-v6-fresh-startup.txt` (current server log), under the same
projectless evidence directory listed below.

Stopped at the user's request on 2026-10-16, about 18:01 EDT.
Repository: `C:\Users\mreca\Desktop\dungeonfluxGame`, branch `main`.

## Exact state at handoff

- Last running build: `commerce-memory-v5`, source `563f848d8b096dfcfe6327dcccf16f5e9d8b1bfe`.
- Room: `ZOGAMU`.
- Durable PostgreSQL database: `df_gameplay_demo_play_20261016214353_1992`.
- Run directory: `development/runs/df_gameplay_demo_play_20261016214353_1992`.
- Display: http://127.0.0.1:63319/gameplay/display
- Player: http://127.0.0.1:63319/gameplay/player?room=ZOGAMU
- Server shut down gracefully via `dev shutdown`; owner lease released; launcher exited 0 and stopped its PostgreSQL server. Preserve the database, run directory, caches and evidence.
- Brakka and Sigrun are ready, 13 HP each, in the standalone Freeplay shop (Opening, no combat/story quest).
- Last gameplay revision: epoch 1, sequence 6. Brakka paid 200 CP for `item-clothes-travelers`; current purse is 16 GP. He permanently owns/holds that item. Sigrun has 18 GP.
- Browser tab 2 claimed Brakka; the old CLI Brakka credential is revoked. Do not kick an active phone. `dev state` remains readable and marks stale bindings `rejoin_required`; private player views require their current credential.
- Tabs were retained for handoff; a connection-lost panel is expected while the server is stopped.

The source now also contains `ec521a83` (shared RPC/CLI NPC completion and merchant fallback repair), `4c4717b2` (lobby spacing), `ac27b8b7` (pending caption race), and `7d534faa` (redundant clone cleanup). These are **newer than the last running build**. Rebuild before judging them. `87d7b1b5` commits the previously present formatting-only splat-stream change at the user's request to commit all changes.

## Launch configuration actually used

Run from the repository in PowerShell. This enables the paid paths already authorized by the user:

```powershell
Set-Location -LiteralPath 'C:\Users\mreca\Desktop\dungeonfluxGame'
$env:DF_FIXTURE_BUILD='merchant-resume-next'
$env:DF_GAMEPLAY_LISTEN='127.0.0.1:63319'
$env:DF_GAMEPLAY_PUBLIC_URL='http://127.0.0.1:63319'
$env:DF_GAMEPLAY_DEV_CONTROL='on'
$env:DF_GAMEPLAY_STORYTELLER='on'
$env:DF_GAMEPLAY_VOICE='on'
$env:DF_GAMEPLAY_IMAGERY='on'
$env:DF_GAMEPLAY_PLATES='on'
$env:DF_GAMEPLAY_HOME_VIDEO='on'
$env:DF_GAMEPLAY_SCENE_VIDEO='on'
$env:DF_GAMEPLAY_STT='on'
$env:DF_GAMEPLAY_ADVENTURE='merchant-shop'

# Rebuild native + WASM, generate bindgen files, then resume this exact purchase.
& 'C:\Program Files\Git\bin\bash.exe' ./development/run-gameplay.sh --database df_gameplay_demo_play_20261016214353_1992
```

To start a separate clean shop instead, replace the last command with:

```powershell
& 'C:\Program Files\Git\bin\bash.exe' ./development/run-gameplay.sh --new-game --seed 671003
```

Do not use `--new-game` when resuming the saved transaction. `--no-build` is valid only when the compiled native binary and generated browser bundle already match the intended source. Keep one fixed build throughout an assessment. Read the new startup output for the actual room, database, URLs and token-file path.

The launcher sets:
- `RUSTUP_HOME=artifacts/cache/rustup`
- `CARGO_HOME=artifacts/cache/cargo`
- `CARGO_TARGET_DIR=artifacts/build/play` (override: `DUNGEONFLUX_BUILD_ROOT`)
- `TMPDIR=artifacts/tmp/play`
- PostgreSQL tools: `artifacts/build/postgres-g05/install/bin`
- Text cache: `artifacts/tmp/play/text-cache`; voice/image caches are also under `artifacts/tmp/play`.

Credentials are loaded from the ignored local `.env` by the launcher. Never print or copy their values into logs/docs. Do not commit `.env`, dev-control tokens, caches, generated builds, or runtime databases.

Observed startup defaults, not extra overrides: text model `gpt-6-luna`, approach `jev-latest`, image `gpt-image-2.5-flare`, video `fal-ai/kling-video/o1/image-to-video`, STT `scribe_v2`, mature rating. Timing: commit 12s, reply 20s, lease 300s, browser 25s. Image interval 20s. Startup budgets reported text 100000000, voice 1000000, imagery 10000 images, video 1000, STT 86400s. Enabled video flags do not prove successful video generation; prior video price availability remained unresolved.

Loopback serves the in-app browser on this PC. Real phones on another device need `--lan` and the actual advertised LAN URL; do not assume 127.0.0.1 is reachable from another phone.

## Agent CLI

After startup, use its reported token-file path. For the saved run this was:

```powershell
$env:DF_DEV_CONTROL_URL='http://127.0.0.1:63319'
$env:DF_DEV_CONTROL_TOKEN_FILE='C:\Users\mreca\Desktop\dungeonfluxGame\development\runs\df_gameplay_demo_play_20261016214353_1992\dev-control-token'
$fixture='.\artifacts\build\play\debug\df-transport-fixture.exe'
& $fixture dev state
& $fixture dev view --as tv
& $fixture dev describe
& $fixture dev merchant-quotes --last 20
& $fixture dev timeline --help
```

Use the current help for timeline/layout/visibility arguments. The visual-resource manual documents `--type image`, `--type plate`, `--type browser_media`, `--type video --summary` and component timing/readbacks. Inventory metadata now includes owned items, owner/holder, permanent versus observation-only state, revision and image state.

For a fresh room only, create the two players quickly:

```powershell
& $fixture dev --timeout 600 play --to opening --max-steps 1
```

Repeat one step at a time and visually inspect after **each** mutation. Five steps normally join two seats, create Brakka/Sigrun from defaults, and enter Opening. An exit code 1 with `play exhausted --max-steps` can mean one successful bounded step; read the JSON. Freeplay stays in Opening: do not loop toward the Story-only dialogue target.

Roleplay examples (use a currently active CLI seat, or type on its phone):

```powershell
& $fixture dev say Brakka 'Marek, how much for the travelers clothes?' --context social
& $fixture dev say Brakka 'I buy the travelers clothes from Marek for 2 GP.' --context social
& $fixture dev say Brakka 'Marek, will you buy these travelers clothes back from me? What would you pay?' --context social
& $fixture dev merchant-quotes --last 20
& $fixture dev view --as Brakka
```

`dev act Brakka interact --target clothes-travelers --option buy` is an available stock action, but success there does not prove the natural-language route. Do not use it to conceal a dialogue classification failure.

To stop cleanly:

```powershell
& $fixture dev shutdown
```

## What actually passed and failed

At running source `563f848d`:
- Default CLI quickstart: passed; screenshots inspected after each step.
- Native bargaining tests: 16 passed. Native shop tests: 8 passed. Scene-script tests: 4 passed at `d5b679c2`. Goods refresh boundary: 1 passed at `d5b679c2`. Native/WASM builds and WASM lib/tests check passed.
- Natural explicit 2-GP purchase: committed 1:6, coin 18→16 GP, permanent canonical clothes entry on the phone, photographic item review image loaded.
- Unified photographic shop plates updated with turns; Marek now matches the short, round, middle-aged, bearded velvet-cap dossier.
- CLI price and bargaining prose produced no durable quotes. Found `/dev/say` duplicated completion and skipped the RPC quote path. Integrated shared completion in `ec521a83`; **runtime retest pending**.
- Both CLI and phone buyback still answered as if Marek were selling. Phone logs showed `no_merchant_commitment`. Found downstream dossier fallback still used the original Reward subject after trade correction. Integrated repair in `ec521a83`; **not proven fixed live**.
- Failed buybacks did not transfer the clothes or change coins.
- The pending phone request briefly resurfaced old purchase narration. Integrated `ac27b8b7`; new test/runtime checks pending.
- Lobby welcome caption overlapped ready status at 1346×1051. Follow-up `4c4717b2` moves it up; new visual verification pending.
- Native check passed at `4c4717b2` with one redundant-clone warning later fixed. WASM lib/tests check passed there. No Cargo or browser checks were run after the final pending-caption and clone commits because the user asked to finish and stop.
- Strict Clippy is **not clean**. Logs retain native dependency errors, 103 native scoped findings, and the WASM findings. One affected hand-card map-entry finding was fixed in `fc356d89`; do not describe the overall lint gate as passed.
- Audio was enabled, but this agent could not hear playback; metadata is not audible verification.

Independent source review found the exact prior item/merchant/hero sale memory, lower buyback ceiling, and durable restart-safe quote acceptance correct through `d5b679c2`. Review of the final shared-completion refactor was interrupted at stop; it remains pending.

## First work for the next agent

1. Read `AGENTS.md`, `planning/coding-style.md`, relevant interaction/narrative/runtime/asset plans and `development/visual-resource-manual.md`.
2. Check current Git head/status and read the final refactor. Run focused native tests and native/WASM compile checks for the final source, especially shared completion, trade subjects, pending captions and quote lifetime.
3. Rebuild and resume ZOGAMU with the exact flags above. Retest the saved same item: merchant should know the committed 200-CP sale and treat the request as buying it back.
4. Confirm a buyback offer below the actual sale, with this item's canonical buyback maximum 100 CP (also bounded by merchant purse). A legitimate refusal is allowed; unrelated reward/selling answers are a failure. No prose may invent a transfer.
5. Accept one valid quote and verify exact-once coin movement, unchanged item ID, removal from Brakka's permanent phone inventory, merchant ownership and restored stock. Test counteroffer/decline and no old-quote acceptance after closure.
6. Compare CLI and phone behavior. Visually inspect the TV and affected phone after **every interaction**, and wait for the actual new reply before judging the first asynchronous screenshot. Preserve readable pacing, photographic medieval style, continuous plate updates and crossfades.
7. Update the acceptance record honestly; helper/reducer success does not establish live behavior.

Other retained work is committed on isolated branches but **not integrated** into main:
- `7b050ec1` → `468c964a` → `95367e14`: special-location stock, containers, cross-place custody and NPC theft callee.
- `ba6b88014045f8fe91b436602253f2ba78d8d42c`: NPC theft classifier/caller, based on `fe2a9042`.
Review dependencies and conflicts before cherry-picking. Do not claim these features shipped. SAM placement is deferred in `planning/issue-triage.md`; do not restart it automatically.

## Evidence and references

Retained local evidence directory:
`C:\Users\mreca\Documents\Codex\2026-10-15\find-repo-and-read-the-readme\outputs\merchant-shop-scene`

Key files:
- `commerce-memory-v5-build-startup.txt`: full last build/run log.
- `commerce-v5-assessment.json`, `commerce-v4-assessment.json`: actual runtime outcomes.
- `commerce-v5-start-01.json` through `05.json`: quickstart.
- `commerce-v5-price.json`, `commerce-v5-haggle.json`, `commerce-v5-buy.json`, `commerce-v5-buyback.json`, `commerce-v5-buyback-reminder.json`.
- `commerce-v5-after-buy-view.json`, `commerce-v5-after-phone-state.json`.
- `commerce-memory-bargaining-tests-04.txt`, `commerce-memory-shop-tests-03.txt`, `commerce-memory-scene-script-tests-02.txt`, `commerce-memory-goods-tests-02.txt`.
- `commerce-shared-native-check.txt`, `commerce-shared-wasm-check.txt`.
- `commerce-memory-native-clippy.txt`, `commerce-memory-native-scoped-clippy.txt`, `commerce-memory-wasm-clippy.txt`.

Screenshots were visually inspected inline through CUA; no screenshot file paths are claimed. Preserve all run evidence and the workflow/telemetry databases. No authorized workflow execution context was supplied, so no workflow completion/devlog IDs were fabricated.

