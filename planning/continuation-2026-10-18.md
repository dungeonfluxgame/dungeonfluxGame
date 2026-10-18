# Continuation — 2026-10-18 battle play-test session

Where the battle work stands at the end of the 2026-10-18 session, and what to pick up next.

## State of `main`

The game builds, starts, and reaches a live battle on the TV: two level-5 Fighters (Brakka, Sigrun) against four
Skeletons on the `scan-village` SuperSplat, with the initiative strip, HP cards, roll cards and hit feedback.

Commits from this session, newest first:

| Commit | What |
| --- | --- |
| `2ed94f3d` | R2+R3: CR 2–5 SRD foes (Ogre, Owlbear, Wight, Hill Giant, Troll) with 2x2/3x3 footprints, a dedicated `ACTION_SURGE` action end to end (`dev act HERO action-surge`), the group-initiative policy in the `COMBAT initiative` line, `CreatureSize::HUGE`. Lane reports: `artifacts/tmp/lanes/R2/report.md`, `R3/report.md`. |
| `43a53447` | Server deadlock at fight start fixed (the old K1-HANG: TV stuck on its splash, `dev state` / `dev fight` timing out). `LivingScenes::describe_depiction` locked the living-scenes state twice in one tuple expression; the character-clip resolver hits that path for every fighter. The view path also held a checkpoint-watch borrow across that lock. Regression test `describing_a_character_depiction_returns_instead_of_locking_the_state_twice`. |
| `1868ae81` | Startup refused by the character-clip providers fixed: image-edit timeout 300 s → 180 s (the native HTTP ceiling) and one fal CDN pin per host (reuses `home_video::resolve_pins`; the media fetcher allows at most 16). |
| `f7eb2782` | Q13: tests that touch the forced-dice queue or `DF_GAMEPLAY_DEV_OVERRIDES` share one test scope, so they no longer leak into parallel tests. |
| `cfe072f0` | Battle presentation (dice, roll card, thrown-weapon flight), camera home pose per scene + `dev camera`, full dev battle control (K1/K2, gated by `DF_GAMEPLAY_DEV_OVERRIDES=on`), home-video flicker fix, character clips (P3). |
| `12a32d3a` | `planning/character-clip-research.md`: video-model clips at the battle camera's home angle. |
| `3a232ff3` | R1: level-5 Fighters (Extra Attack, Action Surge, Champion) from SRD 5.2.1. |

## Uncommitted work in the tree

- **Not from R2/R3, provenance unknown, left alone:** `crates/df-tools/src/gameplay/dev_control/economy.rs` (economy edit
  surface through the staged override path), `crates/df-providers/src/fal_image.rs` (+121 lines),
  `planning/segment-anything-feasibility.md`. Review before committing or discarding.

## How to run a battle

1. Build and start (HEAD plus an optional overlay folder of changed files):
   `OVERLAY=<dir> SCAN=scan-village FOES=4 FOE=skeleton LEVEL=5 bash launch-scan-overlay.sh startupNN`
   (session scratchpad script; it archives HEAD into `artifacts/tmp/play-src`, builds into `artifacts/build/play`, sets
   `DF_GAMEPLAY_SPLAT_DIR=artifacts/tmp/splat-dl`, `DF_GAMEPLAY_SPLAT_LOD=0`, `DF_GAMEPLAY_DEV_OVERRIDES=on`, and reads keys
   from `.env`). Server: `http://127.0.0.1:63319`, TV at `/gameplay/display`.
2. `df-transport-fixture dev play --to opening` (seats the two heroes), then
   `dev fight start --foe skeleton --count 4 --level 5 --scene scan-village`.
3. Drive heroes: `dev fight offers Brakka`, `dev act Brakka attack --target "Skeleton 4" --weapon javelin`, etc.
   Enemy turns run on their policy (`dev npc hold all on` hands them to the CLI).
4. Stop with `dev shutdown`. A hard kill blocks a restart on the same database for about two minutes.

SuperSplat scans live in `artifacts/tmp/splat-dl/<id>/v1` (village 1994c65c, riverbed 9b6bc6bf, valley-bridges 2b94113d,
summit a35a74ba). Re-download with `DungeonFlux/scripts/download-supersplat.ps1`.

## Open defects (full log: `artifacts/tmp/play-r6/battle-findings.txt`)

Seen live this session:

- **TV-STALE-01 (new).** After Brakka's first javelin hit, the TV kept "Skeleton 4's turn" in the header and Skeleton 4 at
  13/13 while the server's TV view said Brakka's turn and 3/13. A reload showed the correct state. The live update after
  the initiative reveal is not applied; look at the initiative intro sequence in `live_tv` / `live_combat` and whether a
  view arriving during it is dropped.
- **INIT-ORDER (new).** "Roll for initiative" appears after the first attack lands, overlapping the damage card.
- **DMG-COLOUR (new).** Damage on a foe shows as a green `+10`, which reads as healing.
- **ART-02.** Skeletons render as robed human women (figure and portrait). The creature art prompt needs the SRD creature.
- **LOG-DUP (new).** Enemy `COMBAT turn` planning lines are logged twice per decision (candidate staging runs twice).

Carried over:

- **CAM-02** action shots occluded by the palisade on `scan-village`.
- **ART-03** watermark in a generated still.
- **CLIP-02** clip prompt still uses the old 15°/38° camera instead of the scene home pose.
- **DEV-BACK-01** `dev back` answers InvalidResource after a level-5 fight.
- **JUDGE-02** room photos still rejected as `modern_setting` (seen again this session for the cloth shop).
- **R3 follow-up:** Large/Huge figures are not yet centred over their 2x2/3x3 space and selection rings use one cell;
  needs `df-render` changes (R3 `request.md` item 8).
- Failing on HEAD before and after R2+R3 (same six): `beat_script_tests::stored_game_without_hit_point_dice...` and
  `real_pre_change_checkpoint_bytes...` (historical), and four legacy-fight tests that now get `InvalidCandidate` on
  EndTurn or unwrap `None` at `journey/tactical.rs:5051`: `campaign_finale_projects_bounded_branch_endings...`,
  `submitted_archer_shot_uses_cover...`, `canonical_live_default_seed_sample...`, `a_won_fight_with_a_dying_hero...`.
  The last four are worth fixing first; they predate this session's last three commits.

## Next steps, in order

1. Fix TV-STALE-01, then play a full level-5 fight to the end and check the timing with the DISPLAY records.
2. Try the new foes: `dev fight start --foe ogre --count 1 --level 5`, then `hill-giant` (Huge), and watch footprints.
3. INIT-ORDER, DMG-COLOUR, ART-02, then the carried-over list.

## Cleanup done at session end

Free space on C: went from 160 GB to 230 GB. Removed: the cargo targets `artifacts/build/{play,integrate,lane-f}` (about
46 GB), build output under `artifacts/tmp/worktrees/{scene-framing-dressing,creation-scene-references}/artifacts/build`
(about 13 GB), 121 copied test executables in `artifacts/tmp/lanes` (9 GB), and the temporary source copies
`artifacts/tmp/{integrate,play-src}`.

Kept on purpose: the Rust toolchain and cargo registry (`artifacts/cache`), the Postgres install
(`artifacts/build/postgres-g05`), the game database and paid-generation caches in `artifacts/tmp/play` (music, home video,
voice), the clip outputs in `artifacts/tmp/char-clips`, the SuperSplat downloads, lane briefs and reports, and the findings
log. The 39 git worktrees under `artifacts/tmp/worktrees` (Oct 16 branches, about 8 GB of source) were not touched because
they may hold uncommitted work; prune them deliberately with `git worktree remove` once checked.

The first build after this is a full rebuild (roughly 20–40 minutes for the play build).
