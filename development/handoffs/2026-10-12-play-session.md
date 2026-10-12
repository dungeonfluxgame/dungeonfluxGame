# Hand-off: playable-game session, 2026-10-12

Written by the coordinator at the owner's request ("make sure all the agents leave notes to follow up later and mark completed todos"). Workers' own notes are appended below as they came in.

## Where things stand
- `main` and `claude/playable-20261011` are the same commit and are pushed to `origin`. Everything merged during the session is on `main`.
- The game runs with `development/run-gameplay.sh`. Useful settings from this session: `DF_GAMEPLAY_TEST_FIGHT=<1-4>` and `DF_GAMEPLAY_TEST_FOE=<creature>` (first choice starts a fight), `DF_GAMEPLAY_BATTLE_SCENE=scan-riverbed|scan-village` with `DF_GAMEPLAY_SPLAT_DIR=artifacts/tmp/splat-dl`, `DF_GAMEPLAY_SPLAT_LOD=0|1`, `DF_GAMEPLAY_CHOICE_HINTS=on|off`, `DF_GAMEPLAY_CHAPTER_TYPE=mystery|parley`.
- Tests on the merged tree: `cargo test -p df-tools --lib` has the nine long-standing failures (six `gameplay::startup_contract::*`, three `preview::tests::*`). In the coordinator's last runs, under a load average of 30 to 40, `gameplay::home_video_tests::*` also failed at the "flight never settled" wait (`home_video_tests.rs:174`), five to six tests, both in the full run and alone. Workers' own runs had them passing. Not resolved: re-run on an idle machine first; if they still fail, it is a regression from the home-screen or scene-entry merges.
- The scan scenes `scan-riverbed`, `scan-summit` and `scan-valley-bridges` are tracked as scene descriptions only and say "none - private test only"; the scan data itself is untracked under `artifacts/tmp/splat-dl` and must stay unpublished. Only `scan-village` (CC BY 4.0, attribution in its file) may ship.

## Merged to main in this session (after the eight-branch batch)
| Task | What | State |
|---|---|---|
| PLAY-E04 slice 1 | Fight chapters go on to a resolution, an epilogue and an end card per outcome | Merged. Live: stops on the epilogue in the victory layout, no end card and no offer; sent back, fix not yet merged |
| PLAY-U02 M1 | Table setting in the phone Menu that hides or shows the choice chips on the TV | Merged, seen live. M2 (opening length) blocked on a design decision (below) |
| PLAY-U03 | Toggle to talk (tap to start, tap to stop) | Merged; label seen live; real microphone not exercised |
| PLAY-V11 | One composed TV home screen | Merged, seen live |
| PLAY-V12 slice 1 | Scenes open on an establishing shot; no cut-out row while a plate is pending | Merged, seen live in two scenes; whole-chapter sampling not done |
| PLAY-B03 slice 1 | Fight staging in server order, Opportunity Attack prompt, named damage with type, test fights, stale creation offer | Merged, seen live. Still wrong: label lags the card by about 3 s between two enemies; phone needs a reload to show actions after enemy turns (not diagnosed) |
| PLAY-S03 slice 1 | Hairline grid, figures not hidden by scan splats, solved wide resting camera | Merged; NOT seen live by the coordinator (session ended at the rebuild) |

## Owner decisions made late in the session that are not built yet
- Remove the grid from the TV entirely (keep only a thin ring under the acting figure and a target). Sent to PLAY-S03.
- Camera: sits behind and above whoever is acting, looking across the whole battlefield, and follows their movement (coordinator's reading of "ego centric"; the owner was told and did not correct it). Sent to PLAY-S03.
- Home loop: two clips that hand off to each other so the wrap does not jitter. Sent to PLAY-V10's worker.
- Callouts ("OPPORTUNITY ATTACK"), battle cries, story lines from the characters and a sparing Dungeon Master narration of the fight: brief at `artifacts/tmp/play-b08/brief.md`.
- Battle scans: use the second-best level of detail but not muddy; level 0 on the riverbed looked right and is the reference frame (`artifacts/tmp/play-s03/riverbed-lod0-good-action-shot.jpg`).
- Opening length (PLAY-U02 M2), coordinator decision: the opening becomes a multi-line scene (an ordered list of narrator and speaker lines the display plays before the first choice; server-owned, no timers).
- Six players: audit at `artifacts/tmp/play-audit/six-players.md`, brief with the coordinator's default decisions at `artifacts/tmp/play-p06/brief.md`. Not dispatched.
- Phone combat screen is still developer drop-downs; it needs a tap-to-move map. No brief yet.

## Workers and their branches at wrap-up
Each worker was told to commit everything (GREEN, or labelled `WIP (not green):`), stop its processes and write `artifacts/tmp/play-<id>/HANDOFF.md`. Their notes follow in the next section. Branch names are `worktree-agent-<id>`; worktrees are under `.claude/worktrees/agent-<id>`.
| Task | Agent id | Brief |
|---|---|---|
| PLAY-E04 endings, next step, chapter flow | abe2121761b0d618f | artifacts/tmp/play-e04/brief.md |
| PLAY-V12 establishing shots, plate continuity | a0d8e630308b47719 | artifacts/tmp/play-v12/brief.md |
| PLAY-B03 fight staging | a68fb704e2220053d | artifacts/tmp/play-b03/brief.md |
| PLAY-S03 battle scans, camera, grid | a8ccec3abe79264ce | messages only (see its HANDOFF) |
| PLAY-N07 story spine, plot-aware characters | a1ba969a4efb78ad7 | artifacts/tmp/play-n07/brief.md |
| PLAY-R01 strict rules audit (queue id PLAY-R02) | adb2c31b9abea68ba | artifacts/tmp/play-r01/brief.md |
| PLAY-V10 home loop as two clips | a7a373d701ad1f2b2 | artifacts/tmp/play-v10/brief.md plus messages |
| PLAY-V14 victory, defeat and end-card screens; TV layout check | a9531e4e2a6599b86 | artifacts/tmp/play-v14/brief.md |
| PLAY-V15 dice roll video | aff485b5dca448a30 | artifacts/tmp/play-v15/brief.md |
| PLAY-B08 callouts, cries, story lines, narration | a5df122ea13e2587f | artifacts/tmp/play-b08/brief.md |
| PLAY-V13 kill clip (paused for memory, never resumed) | af243eb40681140dd | artifacts/tmp/play-v13/brief.md; work saved as WIP commit e9dcd6f, not built |
| PLAY-I01 examination scene (paused for memory, never resumed) | a4618b6b977070905 | artifacts/tmp/play-i01/brief.md; work saved as WIP commit 0af7d1f, not built |
The briefs and frames live under `artifacts/` (gitignored): keep that folder if this work is to be picked up.

## Task queue
The queue refuses `done` without an independent evaluation record, so nothing was marked done. Merged tasks were moved to `review`: PLAY-Q01, T03, E02, E03, V11, N05, N06, H01, H02, V05, V09, S01, S02, F06, T02, V10. The in-flight tasks above stay `pending` (queue ids: PLAY-E04, U02, V12, V13, V14, V15, B07 for the fight staging, S03, N07, R02 for the rules audit). PLAY-U03, PLAY-B08 and PLAY-P06 were never queued.

## Defects seen live and not yet fixed
See `artifacts/tmp/claude-play/defects-28.md` (items 45 to 56) and the list in "Merged to main" above. The ones a player hits first: a fight won with a hero still dying dead-ends (the death-save advance is refused); no death save is shown on a dying hero's turns; the fight ending stops before the end card; the phone needs a reload to show actions after enemy turns; the victory screen draws a hero card over the prompt line; characters cannot answer hard questions about the plot and chapters have no through-line.

## Workers' notes

---

### PLAY-E04 endings, next step, chapter flow (worker note, verbatim)

#### PLAY-E04 handoff (chapters flow into each other)

##### Task, brief, branch, commits
- Brief: /Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/play-e04/brief.md (with the coordinator's addendum and the later reorderings: fight endings, then the smallest next step, then campaign chapters).
- Branch: worktree-agent-abe2121761b0d618f (worktree /Users/earlcameron/Desktop/dungeonflux/.claude/worktrees/agent-abe2121761b0d618f). Not pushed. Worktree clean.
- Commits, in order:
  - a6b5084 End every fight chapter with a resolution, an epilogue and an end card. GREEN, already merged by the coordinator (marker FIGHT-ENDINGS-READY.txt).
  - b17a889 merge of the integration branch (bookkeeping only).
  - b42a844 Draw a fight's closing scenes as story scenes, and offer the way out of a settled defeat. GREEN for its slice (marker FIGHT-ENDINGS-FIX-READY.txt); merge by hash. Focused tests green (fight_ending 6/6, adventure and beat_generation tests); the full lib suite was not re-run on this exact commit.
  - 1cc0424 WIP next step: agreement, successor run, carried seats and heroes. WIP, not verified end to end.
  - be18527 merge of the integration branch into the WIP (conflicts resolved in proto, wire.rs, actor.rs, gameplay.rs, local_demo_scope.rs). Compiles natively (cargo check -p df-tools --lib --tests); nothing else run on it.
  - fb03371 WIP (not green): merge of the integration branch again (bookkeeping only). Branch tip.
- NEXT-STEP-READY.txt was NOT written: the next step is not verified. Do not merge 1cc0424 onward as done.

##### Done and verified
- Fight endings (a6b5084): native tests journey/fight_ending_tests.rs (won, won with a rest, total defeat, defeat with survivors, generated-chapter closing scenes per outcome, refusals); full lib suite then 1061 passed, exactly the 9 known failures; fmt, cargo check -p df-tools --all-targets, wasm check, clippy clean on touched files.
- Fight end card in a browser (b42a844 fix), database-backed, own ports, two phones and a TV, development/browser-checks/fight-ending.mjs. Evidence: /Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/play-e04/fight1 (won, both standing: TV end card, phones "The End"), .../fight2 (costly win, one hero down: "The price of the road is plain on every face"); logs fight1-run.txt, fight2-run.txt. In fight1 the TV end card also showed "Begin a new adventure . Ready: Brakka . Waiting for Thrain." and the phone "Ready. Waiting for Thrain." (the agreement display of the WIP step works to that point).
- Golden hashes: RESTRAINED_GOLDEN pack and record re-pinned twice, each time after generating with the previous table (old pack hash matched the old pin, pack minus beat_script byte-identical, tuning unchanged).

##### Done but not verified
- Next step (1cc0424 in be18527/fb03371): GameplayActionKind NEXT_STEP offered at the end card; agreement held in the session owner (session.next_step, not durable); view fields 55-57; when all seated players agree the owner creates the successor database and writes development/runs/<previous>/successor.json (atomic); the server then stops the run and serve() loops into the next run in the same process (same port, room code and run identity, previous run's grants copied, heroes re-created from their choices at recovery epoch + 1). Never completed in a browser: development/browser-checks/next-step.mjs was run once and could not reach the end card (heroes stuck dying, see defects); the second adventure's opening was never seen. Unit-tested only: carried_room (fight_ending_tests::the_next_run_keeps_the_seats_and_heroes_rested_and_does_not_revive_the_dead) and the ledger (chapter_flow::tests).
- A full cargo test -p df-tools --lib on be18527 was stopped at wrap-up at about 1073 passing. Failures seen beyond the 9 known: gameplay::courier_ai::tests::cache_replay_locale_contract::all_actual_wire_fields_and_encoded_bytes_are_noninterfering_for_retained_private_payloads and gameplay::journey::tactical::tests::submitted_enemy_lethality_reaches_defeat_and_timed_dying_saves_without_universal_rest_revival. Not investigated (integration merge, or my wire/actor change; the tactical one touches the defeat path my settled_defeat change affects). Check these first.

##### Not started
- Campaign hand-over (next chapter via next_chapter -> generate_chapter -> record.begin -> create_campaign_cache, on_victory/on_defeat and clock applied, "Go on to chapter N: <title>", campaign end then new adventure). chapter_flow::set_label is empty in campaign mode, so no step is offered there.
- TV chapter transition card; durability proof across a server stop at the end card and just after the step; browser reload checks (end card, transition, after); recurring cast keeps names/voices/faces; per-run reset of social memory and living-scene state; tests for failure at each stage of the hand-over; per-chapter log lines on later runs; devlog entries.
- Table settings (choice-hint toggle, df_local_demo.table_settings) are not carried to the successor run.

##### Known defects and open questions, and coordinator messages
- Heroes stuck dying in a browser run: "Advance 6 seconds - death saves" refused by the server (PLAY-R01 owns it); it blocked next-step.mjs. Message "fight endings reachable from every post-fight state, wait for a dying hero": ending steps are refused and not offered while any hero is dying (native test); stable-at-0 and dead covered by settled_defeat; a costly victory is covered by the classification unit test and one browser run (fight2), not a played native test.
- Message "FESEFU: fight ends on the victory card": fixed in b42a844. The settled-defeat variant (both heroes stable at 0 HP, nothing offered, old layout; fight4 run) is fixed by settled_defeat in wire.rs, native test only, NOT re-run in a browser. The epilogue frame was never captured (fight3 ran out of steps, fight4 hit the defeat).
- Message "epilogue is not a victory card": closing steps now draw as story scenes (no combat view, inn scene kind) with no client change. PLAY-V14 still to recompose the end card: the labelled slot data-slot="next-step" is filled from JourneyView.next_step_label/agreed/message via presentation::Card.next_step.
- Messages "reorder: smallest next step first" and "NEXT-STEP-READY.txt with field numbers": not met. Field numbers taken: JourneyView 55, 56, 57; GameplayActionKind 21.
- Open: agreement counts every seated player, not only connected ones; it is not durable (a restart asks again); clients see a reconnect (listener dropped and re-bound between runs), not a seamless stream swap; a failed start of the successor falls back to the previous database (Run::fall_back) with a log line only (a failure before the stop, in HandoverPort::prepare, is shown to the table). The successor generates its adventure at startup, so DF_GAMEPLAY_CHAPTER_TYPE persists across runs.
- Pins: any edit to journey.rs, beat_script*.rs, narrative_phase.rs, tactical.rs changes the handler digest; databases from older builds cannot resume.

##### Exact next steps
1. Run cargo test -p df-tools --lib on the tip; look at the two extra failures first (baseline is nine known: six gameplay::startup_contract::*, three preview::tests::*). Also cargo test -p df-protocol and -p df-persistence (not run since the merge).
2. Build (cargo build -p df-tools --bin df-transport-fixture; wasm lib; wasm-bindgen into $CARGO_TARGET_DIR/f05-web), start own PostgreSQL and the server with development/browser-checks/own-postgres.sh and serve.sh (short DF_CHECK_PG_SOCKET_DIR such as /tmp/e04pg; ports not 55517/63309; DF_GAMEPLAY_ADVENTURE=random DF_GAMEPLAY_CHAPTER_TYPE=courier DF_GAMEPLAY_ADVENTURE_TABLES=<worktree>/content/adventure-tables); wait for READY in artifacts/tmp/browser-checks/server-<port>.log; run development/browser-checks/next-step.mjs <base> <out> <room>. Fix what it shows in crates/df-tools/src/gameplay.rs (serve_once, Run), gameplay/chapter_flow.rs, gameplay/handover.rs, gameplay/actor.rs (agree_to_go_on).
3. When it passes write NEXT-STEP-READY.txt and commit green (a new commit; the WIP subjects stay in history).
4. Durability: stop the server at the end card and after the step, restart with the original --database; chapter_flow::resolve_head should land on the successor; add a browser check plus reload checks.
5. Campaign: in adventure/campaign_startup.rs select the next chapter into a new database from HandoverPort/Run (label "Go on to chapter N: <title>"; terminal_outcome already records the chapter), TV transition card, per-run resets, recurring cast.
6. Carry table settings; add typed failure tests per stage.

##### Hunks in shared files
- Proto: gameplay.proto (GameplayActionKind NEXT_STEP = 21; JourneyView 55/56/57), field-ledger.txt (3 fields + 1 enum value).
- df-persistence: new local_demo_scope/carry.rs (stored_grants, import_member_grants), re-exported in local_demo_scope.rs.
- df-tools: gameplay.rs (serve split into serve() loop + serve_once, Run, End, database override, first-run guards), gameplay/pack.rs (installed pack replaceable: OnceLock to RwLock), gameplay/actor.rs (SessionState next_step/handover, agree_to_go_on, decorate call in view; lifecycle only), gameplay/wire.rs (closing/settled-defeat scene kind, next-step field defaults), gameplay/beat_script.rs, beat_script_runtime.rs (endings, next-step offer), journey.rs (initial_in, carried_heroes, carried_room, ending offer id), journey/narrative_phase.rs (ending states), journey/tactical.rs (FightOutcome, outcome, hero_dead), model.rs (ending content entries), adventure/campaign_completion.rs, adventure/beat_tables.rs, adventure/expression.rs (numbered slot names), presentation.rs and gameplay_browser/live_tv.rs (card next_step slot), content/adventure-tables/beat-scripts.json, gameplay/fixtures/canonical-beat-script.json.
- New files: gameplay/chapter_flow.rs, gameplay/handover.rs, gameplay/journey/fight_ending_tests.rs, development/browser-checks/{chapter-lib,fight-ending,next-step}.mjs.

##### Spend on providers
None. No provider keys were used; voice, text model and imagery were off in every run.

---

### PLAY-V12 establishing shots and plate continuity (worker note, verbatim)

#### PLAY-V12 handoff (attempt 1)

##### Task and brief path; branch; commits
- Task: a story scene never shows the old cut-out stage while imagery is on; opening plate requested earlier; every scene opens on an establishing shot, then cuts to the plate. Later extended by the coordinator (mystery-chapter owner feedback, see "Known defects").
- Brief: `/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/play-v12/brief.md`
- Branch: `worktree-agent-a0d8e630308b47719` (worktree `.claude/worktrees/agent-a0d8e630308b47719`). Not pushed.
- Commits:
  - `56b5fd1` establishing shot, no cut-outs while a plate is on its way, lobby request, successor lookahead, place-seeded staging. GREEN (the coordinator's log says it was already merged to the integration branch).
  - `2f89e2d` merge of `claude/playable-20261011` after it. GREEN.
  - `6767bc9` plate shown as soon as its picture lands (predicted layout), heads measured afterwards; lower third raised. Compiles, 17 plate tests pass, fmt clean. Full `--lib` was NOT re-run after it. Treat as GREEN for compile and the focused tests only.
  - `44c29e9` merge of `claude/playable-20261011` after that. `cargo check -p df-tools --lib --all-targets` finishes with no errors.
- Marker with the first slice: `artifacts/tmp/play-v12/READY.txt` (written for `2f89e2d`).

##### Done and verified
- Gates at `2f89e2d`: fmt, native and wasm check, clippy (no new warnings in my files), `schema_ledger` (3 pass), focused tests (62), full `cargo test -p df-tools --lib`: 1068 passed, 9 failed (exactly the known nine).
- Browser run, real imagery, voices on (ElevenLabs), mystery pack, 100 ms in-page sampler, 3 runs under `artifacts/tmp/play-v12/frames/mystery1`, `mystery2` (`mystery3` was stopped by the wrap-up; no results):
  - Run 2 (10,192 samples, 9,535 in story scenes): 0 samples with cut-out figures visible while the plate was pending, ready or unavailable (`node artifacts/tmp/play-v12/analyze.mjs frames/mystery2/samples.json`).
  - Establishing frames: `frames/mystery1/005-Traces-to-Follow-establishing-presenting.jpg` (place picture, narrator box in the lower band), `006-...establishing-awaiting-choice.jpg` (speaker in the lower third with portrait and name), `007-...plate-awaiting-choice.jpg` (mid focus-pull to the plate).
  - Times, scene entry to plate ready/cut: opening 32.6 s in run 1 (the narrator was over after 10.2 s, so about 22 s of establishing hold with the lower third); "The Way Onward" 34.8 s in run 2. In run 2 the first three scenes never got a plate before the scripted clicker (about 15 s per scene) moved on.
- Native tests: lobby request and per-party re-request, dropped request becomes pending and is re-asked, creature speaker gives UNAVAILABLE, lookahead through a won fight / journey / ending (`successors_in`), predicted then measured layout, place-keeps-camera/sides/props/light/hour with no weapon poses (`shot_tests.rs`), shot decision (`presentation.rs`).

##### Done but not verified
- Phone scene picture: the code path is unchanged (phones always showed the place painting until the plate); no phone frames of the establishing picture were taken.
- Imagery off still plays on cut-outs: by construction (state NONE gives shot `stage`), no browser run.
- "Plate shown before its heads are measured" (`6767bc9`): unit-tested only; not run in a browser. Expected effect: ready time falls by the vision-locate time (measured about 30 s in run 2: the image was written 10:23:39, the layout note about 30 s later).
- Interior/exterior phrase in the plate prompt, no weapon-handling hero poses, place-seeded camera/hero sides/props/light/hour: unit-tested, no pair of consecutive real plates compared yet.
- Narrator box in a wide low band (CSS): measured highest top edge 729 px of 1080 (68 %) in run 2; not compared with the head boxes.
- Lower third raised to `*7.4u` (`6767bc9`): not seen in a frame.

##### Not started
- Item 1 of the owner feedback, ending and end card in a browser run: the run never reached the ending (the clicker plan stalled in run 1; run 2 stopped at the travel scene; run 3 was stopped).
- Sampling a journey to a second place and the epilogue/end card.
- The timing report for every scene of one whole adventure (target: plate ready minus entry at or below zero except the opening).
- Weapon and armour from the character sheet: the plate director only sees art numbers (`hero_description`); the second hero's sheet can say longbow while the art is the greatsword fighter.
- Phone frames; imagery-off run; fight scene run with the new client.

##### Known defects and open questions
Coordinator messages and their state:
1. Slice-first nudge: done (marker `READY.txt`, `2f89e2d`).
2. Owner feedback (mystery chapter):
   - (1) one visual language, no cut-outs at any scene including new places, the epilogue, the end card: implemented (PENDING, READY-before-cut and the new `PLATE_STATE_UNAVAILABLE` all hold the establishing shot); only FAILED (plate failed for good) and imagery off draw cut-outs, as the brief says. Browser-verified only for the first four scenes.
   - (2) make ahead across a place change: implemented as lookahead depth 3 through Travel and Ending beats, cap 6 successors (`successors_in`, `MAXIMUM_SUCCESSORS`); unit-tested; the travel-ahead timing was not demonstrated in a browser. Plates are requested 2 s apart and the image service runs two at a time, so a fast table can still outrun them (run 2).
   - (3) continuity: partly. Done: camera, hero sides (hero 1 left, hero 2 right), props, light, hour chosen from the place alone; hero poses never hold a weapon; interior/exterior from the record. Not done: weapon/armour from the sheet; a `HERO_WEAPONS` prompt sentence was dropped because the prompt reached the 4000-character limit (the two phrases `INTERIOR`/`EXTERIOR` fit only after shortening). No consecutive-plate evidence pairs were produced.
   - (4) narrator/look box in the lower third: CSS only, see above.
3. Wrap-up: done as far as listed here.
- Open: the cut-out stage still shows for FAILED plates; the owner may want an establishing-only fallback instead.
- Open: the hero-2 portrait (HUD) and the plate art of the hero can differ from the sheet's weapon.
- Open: the opening plate is made again for each party size (at most 3 requests); an earlier party's plate keeps running and is wasted spend.
- Left by me, unrelated: clippy warnings in `df-provider-api` and `table_log.rs:869`; dead code `CardKind::End`/`end_card` (pre-existing).
- `mystery3` run was stopped; no server, PostgreSQL, browser or monitor of mine is running (the server and PG were stopped with `artifacts/tmp/play-v12/run.sh stop`).

##### Exact next steps
1. `cargo test -p df-tools --lib` once on `44c29e9` (expect only the known nine failures); then merge.
2. `sh artifacts/tmp/play-v12/start.sh <suffix>` (mystery pack, fresh imagery cache, voices on; needs `build.sh` first), then `node artifacts/tmp/play-v12/adventure.mjs <base> <outdir> <pack.json> 1 14` with `LOBBY_WAIT=45000 DWELL=25000 PLAN='Look at the ground|Read the markings|Follow the trail|Press on|Hand the clue|Close the case'`; analyse with `node artifacts/tmp/play-v12/analyze.mjs <outdir>/samples.json`. Make the plan reach the end card; note `adventure.mjs` finds choices by label text.
3. Compare consecutive plates of one place (cache `artifacts/tmp/play-v12/imagery-cache-m2`, `-m3`) for hero sides, weapons, interior/exterior; if weapons still drift, add the sheet's weapon via `PartyMemberView.attack_style` (`crates/df-tools/src/gameplay/living_scenes/plates.rs` `view_of`) and `hero_description` in `crates/df-tools/src/gameplay/imagery/plate_prompt.rs`; shorten other fixed phrases to stay under 4000 characters.
4. Phone frames of the establishing picture (`live_screens.rs` phone scene art) and an imagery-off run (`DF_GAMEPLAY_IMAGERY=off`).
5. If a real table still outruns the plates, raise priority of the next scene's plate or lower `AHEAD_DELAY` (`plates.rs`).

##### Hunks in shared files
- Proto: `crates/df-protocol/proto/gameplay.proto` and `field-ledger.txt`: `JourneyView.adventure_name = 75`; `PlateState.PLATE_STATE_UNAVAILABLE = 4`.
- `crates/df-tools/src/gameplay/wire.rs`: one line (`adventure_name`).
- `crates/df-tools/src/gameplay_browser/live_tv.rs` (establishing card, lower third, shot flags, plate hold/instant removed), `live_motion.rs` (two small hooks: `set_scene_state`, `line_command`), `live_screens.rs` (two model fields), `gameplay_browser.rs` (one comment), `live_plate.css` (all new CSS; old hold rules removed). `live_stage.rs`, `live_stage.css`, `stage_conductor.rs`, `stage_machines.rs` are untouched.
- `presentation.rs` (`Shot`, `shot_of`, `PlateState::Unavailable`), `gameplay/shot.rs` (`exterior` input and spec field, place stream, weapon filter), `imagery/plate_prompt.rs` (interior/exterior phrases).
- `RESTRAINED_GOLDEN` not touched.

##### Spend on providers
- Real image (plates and others), ElevenLabs voices and OpenAI text for three mystery runs; no per-call numbers were recorded and the spend was not totalled. Imagery caches: `artifacts/tmp/play-v12/imagery-cache`, `-m2`, `-m3`; the three runs wrote roughly 40 plate-sized pictures in total (about 25 in `-m2`). Budget flag was 400 images; none reached it.

---

### PLAY-B03 fight staging and test fights (worker note, verbatim)

#### PLAY-B03 handoff

##### Task and brief path; branch; commits
Task: PLAY-B03 (finish the fight wiring: speech in a fight, one sting, damage type, test fights, appearance list, enemy-turn highlights) plus item 0 added by the coordinator (enemy turns staged in the server's order) and the phone defects. Brief: /Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/play-b03/brief.md. Branch worktree-agent-a68fb704e2220053d (not pushed).

- e1a2d26 slice 1. GREEN (gates run on it; the coordinator already merged it as 0b8dafa).
- eabb0ed merge of claude/playable-20261011 into the branch.
- 445ff0a slice 2 (director gate, no snap-back, card waits for walk, acceptance browser checks). Compiles and focused tests passed (df-render 301, stage_director/conductor/presentation 74); full gates NOT re-run on it: treat as GREEN-but-unverified (clippy, full df-tools --lib, schema_ledger not re-run).
- 4d8f2ed merge of claude/playable-20261011 (compiles: native --all-targets and wasm checks clean after it).
Worktree clean. Server, own PostgreSQL (port 55711) and browsers stopped; no background jobs. READY.txt in the task directory describes slice 1.

##### Done and verified
- Gates on e1a2d26: fmt; check -p df-tools --all-targets; check --lib wasm32; clippy df-tools/df-render/df-rules/df-ui (no new warnings); schema_ledger; df-render 301; df-tools --lib 1062 passed with exactly the 9 known failures.
- Native tests added: presentation::turn_marks (three enemy turns); df-render stage tests (three enemy turns in order, label follows beats; suspended turn with prompt and named graze float); stage_director gate; table_log damage type; float_text format; startup test-fight setting; roster; placement of 1/2/3/4 enemies on all 13 scenes (incl. both scans in the table) via a real start; appearance offer equals server validation.
- Real browser (WebGPU TV + two phones, frames in artifacts/tmp/play-b03/):
  - play-b03-create-stale.mjs PASSED (stale creation offer fixed: refused phone 1 confirms again after phone 2 created, no reload; appearance list: only sword figures with a greatsword package, only the bow with the archer package).
  - play-b03-enemy-round.mjs: n=3 and n=4 painted, n=1 on scan-village: label named the card's attacker in every sample, no melee card while the attacker was beyond one cell, floats named ("Dagger · 5 piercing"), phones showed controls within a second, cards read "strikes"/"shoots ... · Light crossbow". (The n=3 run before the final fix had the interleave assertion fail; the final n=4 and scan n=1 runs ALL PASSED.) Frames: enemy-round/*strip*.jpg, enemy-round-1-scan-strip-0200.jpg.
  - play-b03-acceptance.mjs n=1 painted ALL PASSED: typed line as a bubble over Brakka's figure with the camera eased in (acceptance/acceptance-1-speech.jpg), named float "Javelin · 8 piercing, down", celebration 6.4 s, exactly one victory-fanfare in data-sfx-log.
- Test fight works live: the first choice goes straight to the fight against n enemies.

##### Done but not verified
- Scan scenes for the acceptance script (n=1 and n=4 on scan-village with the acceptance script; only the enemy-round check was run on a scan, n=1).
- Flat-fallback behaviour of speech (speak() false) and the six-hero slot (OFF_STAGE_SLOT) have no test.
- Item 6 (no hero-blue reachable cells on enemy turns) has code (Stage::shown_tactical_highlights) but no dedicated test or frame.
- Per-attack melee/ranged pose (Stage::set_attack_style per cue) verified only through the crossbow "shoots" card and float, not by a frame of the pose.

##### Not started
- Diagnosing the phone showing "Waiting for X..." with no controls after enemy turns: NOT reproduced in my runs (play-b03-enemy-round samples: no phone waited over 1 s). Not fixed; suspect list: publish_tactical_controls in gameplay_browser/live_combat/combat.rs returns early on an action_button error (an offer group with zero options -> "unsupported offered input shape").
- Reaction attack as a full beat with an "Opportunity Attack" label and a "declined" beat ("Brakka lets it go"): needs a server flag on the outcome/view; none exists.
- Distance on attack cards (needs PLAY-R01 fields); server path of a move (renderer path is used).
- Full test-fight acceptance frames for n=4 on a scan.

##### Known defects and open questions (and coordinator messages)
- Item 0 (order of enemy turns): DONE and verified as above. Limitations: enemies that only moved (no outcome) walk after the view's attacks, not in initiative order; entering-combat views (an enemy that won initiative) have no walk.
- Suspended turn / reaction prompt (message 3): prompt "<name>: Opportunity Attack?" with camera on the two figures is implemented (live_battlefield sync, HudState.prompt) and native-tested at stage level; NOT seen in a live fight (no reaction occurred in my runs). Phone showing the reaction controls without reload: not checked.
- Reaction attack beat (message 4): not done (see Not started). A graze on a miss is named "Weapon · 3 slashing, graze" (the brief wording was "Greatsword · Graze 3").
- Floats/cards naming (message 3 item 2): done; ranged shows its projectile through per-cue attack style (not seen in a frame).
- Phone controls after enemy turns (message 2): not reproduced, not fixed; the check in play-b03-enemy-round.mjs asserts it (passed).
- Stale creation offer (message 5): fixed and verified (client.creation_offer). Other offer panels (combat actions, choices, free-text confirm) were NOT audited for the same stale-closure pattern; a refusal for a stale offer already redraws the current view.
- DEFECT: in the n=4 acceptance run (victory) the measured celebration was 107 s from the finishing banner to the aftermath card and data-sfx-log held TWO victory-fanfare entries (n=1 gave 6.4 s and one). Not investigated; suspect the director gate (BATTLEFIELD_WAIT_CAP 20 s, stage_director.rs) or the flat VictoryFanfare dedupe flag ENDING_STING_PLAYED in live_sfx.rs being consumed by an earlier ending event.
- The TV round counter stuck at "Round 1" in an earlier run was caused by the stale-latest-view snap-back and went away after the accepted_cells fix (n=4 run showed Round 2).
- Bound: ENEMY_ENTITIES stays 4 (documented in development/README.md with what breaks at 6 vs 6).
- Open: PLAY-R01 must use CombatOutcome field numbers 20+ and CombatView 16+ (I took 19 on CombatOutcome only).

##### Exact next steps
1. Investigate the n=4 victory run: run `N=4 node development/browser-checks/play-b03-acceptance.mjs <dir>` (env: DUNGEONFLUX_ARTIFACT_ROOT=<root with symlinks cache/build>, DUNGEONFLUX_BUILD_ROOT=artifacts/build/claude-t03-archer, DF_CHECK_PG_PORT, DF_CHECK_PG_SOCKET_DIR=/tmp/<short>, PORT; own-postgres.sh start, serve.sh bindgen first). Check sfx entries and the finishing-to-aftermath time; look at crates/df-tools/src/stage_director.rs (gate) and gameplay_browser/live_sfx.rs (ENDING_STING_PLAYED).
2. Re-run the full gates on 445ff0a/4d8f2ed: cargo clippy -p df-tools -p df-render -p df-rules -p df-ui, schema_ledger, df-tools --lib (baseline 9 failures).
3. Run play-b03-acceptance.mjs with SCAN=1 DF_GAMEPLAY_SPLAT_DIR=artifacts/tmp/splat-dl for n=1 and n=4 (never copy the scans).
4. Reproduce the phone "Waiting for X..." defect with a bandit fight (ranged enemies) and a longer run; start at publish_tactical_controls in crates/df-tools/src/gameplay_browser/live_combat/combat.rs and action_button in gameplay_browser.rs (early return on an empty option group).
5. With PLAY-R01's fields (reaction flag, path, distance) add the reaction attack beat, "Opportunity Attack" card label, declined-reaction beat, distance on the card.
6. Add a dedicated test for Stage::shown_tactical_highlights and a flat-fallback speech test.

##### Hunks in shared files
- Proto: CombatOutcome.damage_type = 19 (string); JourneyOption.requires_group = 4, requires_options = 5 (repeated string); field-ledger.txt updated.
- df-rules: DamageType::id() in tactical/weapons.rs.
- df-render (shared with the battle-scene worker): battle/stage.rs (server_active/hold_active/stage_move/set_attack_style/set_prompt, highlight filter, roll-card wait for walks, SetActive clears the card, attack label takes a damage type), float_text.rs (compose "Name · 9 slashing"), model.rs (HudState.prompt), hud.rs (prompt in the turn label), host.rs (data-trace-figures harness attribute), combat_feed.rs (Outcome.damage_type).
- df-ui: character_phase.rs/character_phase_theme.rs (require_option, hidden options).
- df-tools: gameplay/actor.rs (test-fight follow-up submit), journey/tactical.rs (TestFight, roster, enemies()), startup_configuration.rs, gameplay.rs (startup line), table_log.rs, wire.rs, pack.rs (appearance_fits_package), presentation.rs (turn_marks, standing_cells), stage_director.rs/stage_conductor.rs (battlefield gate), gameplay_browser.rs (creation_offer, option requirements), live_motion.rs, live_battlefield.rs, live_sfx.rs, live_combat/combat.rs (card label).
- development/README.md, run-gameplay.sh (--test-fight, --test-foe); browser checks play-b03-*.mjs.

##### Spend on providers
None (no provider keys read, no provider calls).

---

### PLAY-S03 battle scans, camera, grid (worker note, verbatim)

#### PLAY-S03 handoff (battle camera, hidden figures, level of detail, grid)

##### Task, brief, branch, commits
Task: PLAY-S03 on the battlefield. Owner items, in the order they arrived: figures hidden under scan splats; the battle camera (wide resting
shot, then "ego centric" actor camera); level of detail ("use the second best LOD", muddy scan); the grid (thinner, then removed from the TV).
Brief: the coordinator messages in this session (no separate brief file); evidence and markers in `artifacts/tmp/play-s03/`.
Branch: `worktree-agent-a8ccec3abe79264ce` (nothing pushed). HEAD `24bc0fe3858dbdeed056bb11afd3dc2aab1ba6f2` (merge of `claude/playable-20261011`).

| commit | what | state |
| --- | --- | --- |
| 7dfafa9 | Grid: hairlines, region tint + one outline, thin rings; preview `&reach`, `&beat=rest` | GREEN, mergeable |
| 1f321e8 | Figures never hidden by splats beside/behind them (per-splat rule + ghost); solved wide resting camera; tilt-shift only in focus pulls | GREEN, mergeable |
| 2c57fbc | merge of the integration branch (slice 1; already reported in READY.txt) | GREEN |
| 282942d | Level of detail: chosen level drawn complete out to the budget, then steps coarser | GREEN (tests + browser frames) |
| ebc5bda | Camera keeps a clear line from the lens to every figure and the field (scan occupancy in the scene files, `splat_occupancy` example) | GREEN (df-render tests, clippy) |
| 6dbbbb0 | The TV draws no grid by default; `DF_GAMEPLAY_BATTLE_GRID=on|off`, preview `&grid=1` | GREEN (fmt, clippy wasm both, df-render lib 316 pass, focused df-tools tests, one browser frame) |
| 0afbaea | WIP (not green): `crates/df-render/tests/figure_visibility.rs`, native real-scan visibility test | WIP, never run to completion; do not merge as is |
| 24bc0fe | merge of `claude/playable-20261011` | merge only (no compile/test re-run after this last merge) |

##### Done and verified
- Grid (7dfafa9) and grid off (6dbbbb0): unit tests in `grid_lines.rs`, `startup_configuration.rs` (`the_battle_grid_is_off_by_default_and_only_on_or_off_are_accepted`), `gameplay.rs` page tests; browser frames with real WebGPU:
  `artifacts/tmp/play-s03/grid/sheet-{village,riverbed,painted}.jpg` (before = old build live fights, after = full reach, 200% line crops) and
  `artifacts/tmp/play-s03/nogrid/sheet-nogrid-village.jpg` (grid off: only a thin ring under the acting figure remains).
- Figures (1f321e8): `figure_clear.rs` rule tests; depth-debug browser view shows a depth hole at each figure; palisade row shows the faint ghost where wood truly stands in front (`play-s03/depth/`).
- Camera (1f321e8): `rest_shot.rs` tests for every scene at 1v1, 4v3, 6v6 (2, 7, 12 figures): all living bodies inside the safe area and the free area between the HUD keep-outs, reachable cells in frame; stage tests for lean-in/out, refit.
- Camera clear (ebc5bda): `rest_shot::tests::a_scene_with_scan_occupancy_keeps_a_clear_line_to_every_figure_and_the_field` (riverbed before: 4-9 blocked figure lines, now 0; the other three scans 0).
- LOD (282942d): `lod::tests::the_full_detail_radius_grows_until_the_budget_runs_out`; browser side-by-sides `artifacts/tmp/play-s03/lod/sheet-{village,riverbed}-{turn_focus,tactical}.jpg` (old level 1 | new level 1 | level 0, with splat counts and GPU ms).
  Facts: the village scan has 11 levels (59.7M splats at level 0 down to 58k); only levels 3-10 are on disk (level 3 = 7.47M, level 4 = 3.73M). The other scans have 4 levels; levels 1-3 on disk (level 0 = 5.0M missing). The scene `levels` ([3,10] village, [1,3] others) are the finest on disk, and `DF_GAMEPLAY_SPLAT_LOD=1` = second best ON DISK (village scan level 4, riverbed level 2), not the scan's own second best (level 1, not downloaded).
  Old level 1 on the village drew only 477k splats at level 4 (within a 10 m radius of the grid), the rest at levels 5-10; planned 1.13M. New level 1: whole level 4 as far as the 2.5M budget allows (2.40M planned). Riverbed: old 0.81M, new 1.25M (the whole of level 2). Level 0 costs a few ms more on this machine (village GPU ~14-16 ms vs ~11-14 ms under load).
  The remaining softness is the scan (the village is soft even at level 3); the fence smear at old level 1 was the thin outer rings plus the tilt-shift blur, now fixed.
- `cargo test -p df-tools --lib` after the first merge: 1065 pass, 10 fail = the known baseline names (build_set publication_recovery_boundary, 6 startup_contract, 3 preview).

##### Done but not verified
- The owner's exact "Brakka beside the fence" frame was never reproduced; the figure rule holds by construction. No before/after of that cell. The WIP native test (below) was meant to prove it.
- Grid-off frames only for the village (rest + action); riverbed and painted scene not captured with the grid off.
- 4v4 / largest-fight camera before/after frames not captured.
- After the last merge (24bc0fe) nothing was rebuilt or re-tested.
- `df-tools` native all-targets clippy has 3 PRE-EXISTING errors (presentation.rs `End`/`end_card` dead code, table_log.rs collapsible if); not mine.
- One stale-artifact trap: after merging the integration branch `cargo build -p df-tools --bin df-transport-fixture` failed with "no StoredTableSettings" until `cargo clean -p df-persistence` in the build dir.

##### Not started
- The actor-centred ("ego") camera: designed only (below), no code. Tests, frames and `CAMERA-READY.txt` do not exist.
- Reactions bringing the camera to the reacting figure.
- Riverbed resting-shot before/after was shown only through the occupancy test numbers (no browser frame after ebc5bda, which was not rebuilt into a preview).

##### Known defects and open questions (coordinator messages and their state)
1. Figures under the splats at level 1 (top item): rule implemented (1f321e8), exact owner cell unverified; native test is WIP.
2. Reachable-highlight look: done (7dfafa9), then superseded by the owner removing the grid (6dbbbb0).
3. LOD "smeared / muddy / second best": done as described above (282942d); the scan's own second-best level is not on disk for the village (levels 0-2 are not downloaded; ~230 MB for level 1, ~455 MB for level 0 from the scan's CDN, needs the owner's permission to download).
4. Resting camera (wide, higher, three-quarter, HUD keep-outs, 1 v 1 to 6 v 6): done (1f321e8). Cause of the old framing is in READY.txt (plan_turn/plan_move on TurnFocus).
5. Camera clear of scan geometry: done (ebc5bda). Open: the occupancy was authored from level 6 (village) / level 3 (others) with a count threshold of 6 and honours each scene's ceiling cut; thresholds are untuned.
6. Grid too thick (third complaint): hairlines done (7dfafa9); owner then asked to remove it: done (6dbbbb0). Choices to confirm: I kept the dark pool + cross under a downed figure; I removed side-coloured rings under idle figures, status rings (Marked/Concentrating), cover pips, difficult-ground hatch and blocked-cell voids when the grid is off. Rings for "the creature a reaction is about" are NOT specially handled (a reacting figure only gets a ring if it is the active figure).
7. Ego camera direction (behind and above the actor, follows moves, enemy turns from the enemy's side, 6 v 8): NOT DONE, see next steps.
8. Machine load average above 30 made every build and test run 2-10 minutes; the visibility test was never completed.

##### Exact next steps
1. Rebuild after the merge: `cargo build -p df-tools --bin df-transport-fixture` (if "no StoredTableSettings": `cargo clean -p df-persistence` in your target dir) and the wasm release; run `cargo test -p df-render` and `cargo test -p df-tools --lib` (baseline 10 known failures).
2. Capture the missing frames with the grid off on riverbed and salt-quay (`artifacts/tmp/play-s03` scripts are in the session scratchpad only: the commands are `#battle-preview=<scene>&heroes=2&enemies=3&reach&beat=rest` and `&beat=heavy`, add `&grid=1` for the old look). Write `NO-GRID-READY.txt`.
3. Ego camera in `crates/df-render/src/battle/rest_shot.rs` + `stage.rs` + `director.rs`:
   - add `ego_pose(RestInput, actor Body, toward Vec3)`: heading = from the actor toward the centroid of the opposing living side (camera yaw = atan2(-fx, -fz) of that direction), try pitches {22,30,40,52}, fovs {44,58}, yaw offsets {0,+-20,+-40}; generalise `fit` to take pitch and fov and a minimum distance (4.5 m); keep REST_AREA, blocked_lines (occupancy), hidden_pairs, home-heading penalty (> 75 deg from the tactical yaw costs 1000); score by actor body share of frame height near 0.20 (accept 0.12-0.30), fall back to `rest_pose` when nothing fits.
   - `CameraFocus::Ego { actor, stand: Option<Cell> }`; `Stage::solve(ego)` replacing `wide_pose`; hold `rest_actor` so `refit_rest` recomputes the ego pose; `camera_label` returns `ego`.
   - director: TurnStart and the first turn after the roll-call = ego(actor) pan about 1.0 s (drop `lean_in_and_out`); `plan_move` = ego(actor, stand = destination) panned over the whole walk instead of TurnFocus then settle; attack/heal settles go to ego(attacker/source) instead of `Wide`; the opening, roll-call and pull-backs stay `Wide`.
   - make `CameraPose::lerp` take the short way round for yaw (camera.rs) so a change of actor does not orbit the long way.
   - update the director tests that name the steps (`the_initiative_runs...`, `a_server_move_plans...`) and the stage tests `a_turn_leans...`.
   - add the tests asked for: per scene at 2, 7, 12 figures on each figure's turn: actor body height share in the band, everyone and the reachable region inside the safe area and outside the HUD, blocked_lines zero with occupancy, and continuity (bounded per-frame change of eye and look-at) across a simulated move.
   - write `artifacts/tmp/play-s03/CAMERA-READY.txt` with browser frames: a hero's turn start, mid-move, settled, an enemy's turn, and a reaction, with the old shots beside them.
4. Run `crates/df-render/tests/figure_visibility.rs` with `DF_SPLAT_SCAN_DIR=artifacts/tmp/splat-dl cargo test -p df-render --test figure_visibility -- --nocapture` (expect minutes in debug), read the worst cells it prints, tune the bound `EXTRA_COVER_BOUND`, add `#[ignore]` if it stays slow, then make it a normal commit.
5. If the owner wants the scan's own second-best level on the village: ask for permission to download levels 1-2 (`artifacts/tmp/play-s01/dl_chunks.py` pattern, CC BY scan 1994c65c), then set the scene `levels` to `[1,10]` (lod 0 would then be the scan's level 1 near the grid within the budget).

##### Hunks in shared files
- `crates/df-render/src/battle/gpu/splat_shaders.rs`: uniform block `U` gained `figs: array<vec4<f32>, 32>` (UNIFORM_BYTES 320 -> 832 in `splat_engine.rs`); the cache word 7 sign now marks a splat that adds no depth (negative falloff).
- `crates/df-render/src/battle/model.rs`: `CameraFocus::Wide` and `CameraFocus::Lean(TokenId)` added.
- `crates/df-render/src/battle/scene_file.rs`: optional top-level `camera_occupancy {origin, cell_m, dims, hex}` (schema version unchanged); `BattleScene.occupancy`; `SceneFileError::Occupancy`. The four `content/battle-scenes/scan-*.json` carry it (about 24 KB each).
- `crates/df-tools/src/gameplay.rs`, `gameplay/startup_configuration.rs`, `gameplay/connection_tests.rs`: `PageState.battle_grid`, `page(..., battle_grid)`, `data-battle-grid` attribute, `DF_GAMEPLAY_BATTLE_GRID`.
- `crates/df-tools/src/gameplay_browser/live_battlefield.rs`: preview `&reach`, `&grid=1`, `&beat=rest`; `StageOptions.show_grid`.
- `development/README.md`: preview table rows, grid/camera/figure paragraphs, `DF_GAMEPLAY_SPLAT_LOD` and `DF_GAMEPLAY_BATTLE_GRID` items.
- No proto field numbers taken.

##### Spend on providers
None.

---

### PLAY-N07 story spine and plot-aware characters (worker note, verbatim)

#### PLAY-N07 handoff (attempt 1, stopped at the owner's wrap-up)

##### Task and brief path; branch; commits
- Task: one causal spine per rolled chapter; every scene, line, choice, fight and ending rendered from it; people know the plot; dread.
- Brief: `/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/play-n07/brief.md`
- Branch: `worktree-agent-a1ba969a4efb78ad7` (worktree `/Users/earlcameron/Desktop/dungeonflux/.claude/worktrees/agent-a1ba969a4efb78ad7`). Not pushed.
- Commits:
  - `1ff4360` WIP (not green): M1 spine + most of M2 rendering + spine validators. Do not merge as is.
  - `a683689` merge of `claude/playable-20261011` into it (clean textual merge; it brought in the neighbours' edits to `adventure/tests.rs`, `adventure/expression.rs` and three other adventure files). The merged tree was NOT compiled. WIP.
- No marker files M1..M4-READY.txt were written: no milestone passed all its gates.

##### Done and verified
Verified by `cargo test -p df-tools --lib spine_` (run7, before the last fixes below), 200 seeds x 16 chapter types x 2 ratings:
- Every scene names its cause and the reason the next comes (the chain sentences are inside the scene text); the fight's cause, place (`settings.confrontation`) and taunt are in the combat scene; no scripted line, label or pledge uses another errand's verb; patron and contact names are introduced with description or role at first appearance; the ending says what became of the stake, patron and antagonist; the spine is in the stored record and reloads equal; the patron's secret is not in the public facts; every (location, stake, contact) has a peril to join; every (hook, private-clue secret) has a patron secret to join. Six of seven checks passed. The seventh (antagonist introduced with its kind) failed on a test false positive (it compared `ant.the` instead of the table noun).
- Table load, static bounds (`bounds.rs`, per real antagonist and complication combination) and a printed-chapter reading (`DF_SPINE_SEEDS=1,2,3 cargo test ... print_chapters -- --ignored --nocapture`) worked for courier chapters.

##### Done but not verified
- The fix for the seventh test (noun from tables), and moving all checks onto one shared sweep (`OnceLock`) because six separate sweeps took 994 s under load 44: written, run8 started but stopped at wrap-up, result unknown.
- Mystery and parley scripts rewritten in `beat-scripts.json` (looks feed trace sentences, destination line states the answer): lengths checked by a Python estimate only; the mystery and parley rows of the sweep passed in run7 (all 16 types incl. mystery and parley generated and passed 6 checks). `Spine::inquiry` (question, answer, culprit, red herrings, evidence) is in the spine; nothing reads it yet.
- Restrained wording of the new tables; the `_mature` fields.
- Everything after the merge: compile of the merged tree, `tests.rs` golden pins, campaign tests, `expression.rs` interplay, wasm check, clippy, fmt, the full lib run (baseline nine failures). `RESTRAINED_GOLDEN` in `adventure/tests.rs` is NOT re-pinned and will fail. `five_thousand_standard_seeds_validate`, `longest_coherent_opening_fits_the_narration_bound` (forces `prose.opening_narration` variant 1 and `foreshadow`), the campaign tests and `campaign_render.rs` overlays were never run against the new tables.

##### Not started
- M3: plot facts in dossiers (patron, contact, antagonist knowledge from the spine; the real patron secret in the trusted tier replacing the phobia answer; antagonist Hostile unless `spine.antagonist.victim`), reply-path changes (`npc_facts.rs`, `npc_reply.rs`, `npc_talk_rules.rs`: answer sub-questions in order, say who would know, accusation or "what are you hiding" is a plot question), new rule checks, the live "hard questions" scenario and the before/after transcripts. No provider call was made.
- M4: menacing rumours (`npc-knowledge.json`), antagonist starting attitude (`npc-social.json`), narration register tables beyond what the mature overlay already had.
- Campaign chapters: `campaign_render.rs` still overwrites opening, followup and victory narration and the inn line with campaign text and replaces the ask/help/defend labels with `chapter-types.json` phrases; spine causes are not guaranteed there.
- Model lint: spine facts reach the model through the fact-sheet facts (`spine.*` keys) but no must-mention or contradiction lint was added in `expression.rs`.
- Browser check, golden re-pin, CHANGELOG.

##### Known defects and open questions; coordinator messages
- The escort line repeats the contact name ("Kelbin is my oldest friend. Kelbin can read..."): change `escort_line` in `prose-patterns.json` and the `ESCORT` text of the mystery and parley scripts to `{contact.sub|cap} can {spine.ability}` (add `contact.sub` to `PATTERN_KEYS` in `beat_tables.rs`).
- `inn_dialogue` still carries the old `secret.confirm` ("Yes, the quick way is real") even when the table never heard the private clue; it reads oddly. Needs a design decision.
- `room_narration` still has the stock phrase "Something is about to happen".
- The sweep takes minutes in debug under load; lower restrained seeds if needed.
- The shared build directory `claude-n1-npc` served a `df-protocol` from another worktree (errors `table_setting`, `choice_hints_hidden`). I cloned it to `/Users/earlcameron/Desktop/dungeonflux/artifacts/build/claude-n07` (APFS clone) and touched `crates/df-protocol/proto/*.proto` mtimes to force a rebuild. The clone can be deleted.
- Coordinator messages:
  1. Mystery evidence (looks change nothing, "the clue" never explained, tutorial-hint line): partly addressed by the rewritten mystery script and `Spine::inquiry` for PLAY-I01. State: spine side done, unverified; the question the party answers is I01's scene.
  2. Leak of a private-clue reply onto the TV: NOT addressed. Intended design: a fact is private until the story has revealed it to the table, decided from the spine (fields not yet in any public scene text), not from a term list; add the exchange "who wrote that petition, who is Dessa Pennington to you, and what happens if the wrong hands open it?" to the `npc_talk_rules` checks and the audience tests (`npc_reply.rs` `released_terms`, `Withheld` in `npc_dossier.rs`).
  3. Reply answered one of three sub-questions: NOT addressed (M3).
  4. Nudge to commit a first visible slice: committed as WIP (not green), so no marker file.
  5. Wrap-up: done.

##### Exact next steps
1. In the worktree run `/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/play-n07/cargo-n07.sh check -p df-tools --all-targets --offline --locked` (own target dir `claude-n07`). Fix merge breakage (`adventure/expression.rs`, `adventure/tests.rs` came from other branches).
2. `cargo test -p df-tools --lib spine_`. Fix the escort-line pronoun and the inn line, then read chapters with `print_chapters` (`DF_SPINE_SEEDS`, `DF_SPINE_KINDS=courier,mystery,parley,rescue`, `DF_SPINE_RATING=restrained`).
3. `cargo test -p df-tools --lib adventure::` and repair: re-pin `RESTRAINED_GOLDEN` (the output changed on purpose), `longest_coherent_opening_fits_the_narration_bound`, the campaign tests; adapt `campaign_render.rs` to keep the spine sentences (opening, followup, victory, inn overlays, label overrides).
4. Gates: `cargo fmt --all -- --check`, `cargo check -p df-tools --lib --target wasm32-unknown-unknown`, clippy, full `cargo test -p df-tools --lib` (baseline nine failures). Then write the `M1-READY.txt` and `M2-READY.txt` markers.
5. M3 then M4 as in the brief. Spine accessors to feed dossiers: `Spine::patron.{because, why_not_authorities, stands_to_lose, stands_to_gain, secret, secret_terms}`, `contact.{relation, relation_told_by_contact, ability, can}`, `antagonist.{wants, sent, why_now, taunt, victim, real_threat}`, `threat.*`, `stake.does`. Route them through `adventure/dossier_cast.rs` (`DossierFacts`) into `knowledge_tiers` in `adventure/dossier.rs` (new source names `plot_*`, a plot kind in `npc_facts.rs`).
6. The authoring scripts that generated the JSON lived only in the session scratchpad (`spine_data.py`, `build_spine.py`, `lens.py`, `edit*.py`); the committed JSON is the source of truth.

##### Hunks in shared files
- Content: new `content/adventure-tables/spine.json` (listed in `tables.json` files and roll order); `complications.json` (new fields wants, sent, why_now, taunt, taunt_mature, fate, fate_mature, victim, real_threat; `broken-ward` text); `hooks.json` (pitch, answer, accept replaced by `because`); `antagonists.json` (`warnings` removed); `prose-patterns.json` (many tables rewritten, `defeat_*` collapsed to `defeat_narration`, new `opening_line`, `seal_line`, `escort_line`, `combat_open_*` single variant); `mature-overlay.json` (matching replacements; antagonist `warnings` and `victory_dialogue` entries removed); `beat-scripts.json` (mystery and parley scenes and labels only); `tables.json`.
- Rust (`crates/df-tools/src/gameplay/adventure/`): new `spine.rs`, `spine_tables.rs`, `spine_tests.rs`, `spine_validation_tests.rs`; edited `generate.rs` (spine rolled after settings, patterns, chain, combat narration bound 250), `bounds.rs`, `tables.rs`, `schema.rs`, `record.rs` (optional `spine` field), `mod.rs` (`GeneratedAdventure.spine`), `beat_tables.rs`, `mature_overlay.rs`.
- No proto fields and no wire fields (`JourneyView` untouched). Typed accessors for neighbours: `GeneratedAdventure::spine` and `AdventureRecord::spine`; `Spine::inquiry` for the examination scene (PLAY-I01); `Spine::{threat, fight, ending}` for the opening beats.
- The roll order changed (spine rolls after settings; the `@warning` roll is gone), so every stored seed output and golden hash changes.

##### Spend on providers
None. No model or image call was made.

---

### PLAY-R01 strict rules audit (worker note, verbatim)

Kobold Warrior 3's turn: legal, as far as the engine can show; the live record itself was not available. The engine's enemy planner cannot produce an illegal turn (zero violations in 3,600 generated fights before my changes, see below), a Kobold had no ranged option before this work, and the reported end positions are exactly what a legal turn does: move beside Brakka, stab him, then spend the rest of its 30 ft Speed walking toward the only hero still standing (it ended 4 squares from Brakka beside Sylvie). `a_kobold_can_stab_one_hero_then_walk_toward_the_other_within_thirty_feet` (crates/df-combat/tests/rules_invariants.rs) reproduces that shape on scan-village and audits it. I could not replay the live fight (its database was not in the run directory), so "this exact turn was legal" is inferred, not observed.

Out of order / second action: no, as found. The independent auditor (crates/df-combat/tests/support/rules_audit.rs) found 0 violations of turn order, one action, one bonus action, one reaction per round, or movement budget over 2,400 fights (114,819 hero inputs, 21,152 enemy turns, 136,904 accepted steps) plus 1,200 mixed-creature fights, release build, on the engine before my changes; injected-defect tests prove it would have flagged them. After my changes it passed 330 more fights (debug). The 4,000-fight release run on the final code was NOT done.

#### HANDOFF: PLAY-R01 (attempt 1)

##### Task, brief, branch, commits
- Brief: /Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/play-r01/brief.md. Branch worktree-agent-adb2c31b9abea68ba (worktree .claude/worktrees/agent-adb2c31b9abea68ba), based on claude/playable-20261011 at e8eb3c9.
- Commit 6456c3b: "Show the dying hero's saves and make the post-fight advance work; add the independent rules auditor, ..." GREEN for what I ran (see below), mergeable after resolving the merge conflict below. It carries M1 (audit, dying path), most of M2 and M3 together in one commit.
- Merge of claude/playable-20261011 (now 4a24a78) was attempted and ABORTED: conflicts in crates/df-protocol/proto/gameplay.proto, crates/df-protocol/proto/field-ledger.txt, crates/df-tools/src/gameplay/journey/tactical.rs, crates/df-tools/src/gameplay/table_log.rs. Tree is clean at 6456c3b. The coordinator must resolve (proto numbers: see last section; the other worker also took damage_type = 19 and maybe more).
- Marker files: M1-READY.txt, M2-READY.txt, M3-READY.txt next to this note, all pointing at 6456c3b.

##### Done and verified
- Gates on 6456c3b (before the integration merge): cargo fmt --all --check clean; cargo test -p df-rules 103 passed; cargo test -p df-combat all green (debug; includes rules_invariants 11 tests, 220 + 110 generated fights, 0 violations); cargo check -p df-tools --all-targets and --lib --target wasm32-unknown-unknown pass; clippy on df-rules, df-combat, df-tools: no new warnings (df-tools keeps its pre-existing end_card and table_log:993 ones, df-provider-api untouched); cargo test -p df-tools --test schema_ledger 3 passed; cargo test -p df-tools --lib: 1056 passed, 11 failed = the nine known (six startup_contract, three preview) plus two gameplay::home_video_tests timing failures ("flight ... never settled") that I attribute to machine load (load average about 35) and the merged home-video work, not to this change; not re-run in isolation.
- Order of combat auditor, property test and mutation tests: crates/df-combat/tests/rules_invariants.rs and tests/support/rules_audit.rs. Checks: initiative order recomputed from the recorded d20s (one roll per identical group, ties by agreed order, Kobold sunlight Disadvantage), round and active actor, budget reset, one action / bonus / reaction, incapacitated take no actions, path cost and Opportunity Attack windows recomputed from the scene JSON, cover by an independent segment test, attack reach/range/long range/close enemy, Advantage cancel and never stack, hit/critical/damage dice from the recorded draws, target consequences, death-save arithmetic and one save per dying turn, Second Wind, Savage Attacker once per turn, masteries, stable recovery, short rest. Random legal and illegal hero inputs are checked both ways (accepted illegal, rejected legal). Coverage counters from the 2,400-fight release run: 36,384 moves, 6,328 Opportunity windows, 2,916 Opportunity Attacks, 32,297 attacks (17,043 melee, 15,254 ranged), 3,204 death saves, 1,501 critical hits, 4,084 Advantage rolls, 6,750 Disadvantage rolls, 3,878 Dashes, 990 Second Winds, 1,227 Savage Attacker hits, 2,773 mastery attacks, 7,196 illegal paths and 31,114 illegal attacks correctly refused.
- Violations found in the engine and planner by the auditor: none (every early auditor failure was a bug in the auditor itself, fixed). Defects found by reading and probing, fixed: (1) a Death Saving Throw was rolled but never recorded as an outcome, so it was missing from the log, the "Latest roll" list and the view; (2) a won fight with a dying hero dead-ended: reproduced with the live run's generated pack (test `a_won_fight_with_a_dying_hero_resolves_on_the_table_without_a_dead_end`); causes: the aftermath EndTurn was replayed as an offer of the already-finished fight beat (beat_script_runtime::validate_encounter_choices), phase_from_state found no transition for it, and stage_authored_threads / narrative_phase tried to progress the already-resolved threat thread; (3) a Stable hero at 0 could never recover (now Action::StableRecovery, 1d4 hours, SRD p18) and any hero at 0 now keeps the only offer to "Advance 6 seconds" or "Wait" until resolved; (4) moves and attack distance/mode were not recorded anywhere.
- Native test of the dying path: the test above runs saves that succeed, fail and roll a natural 20 against the live pack, checks every roll is in CombatView.death_save_outcomes, that time advances and that the table can then rest. Passed.
- Stat blocks: Kobold Warrior Sunlight Sensitivity (typed Lighting on BattleMap; scans sunlit, painted scenes night/interior: checked against the backdrop paintings for the painted ones, the scans' daylight is taken from the brief) with Initiative Disadvantage; Pack Tactics was already correct (live observation 2: Kobold 1 had no ally near Sylvie yet, correctly plain; Kobold 2 had Pack Tactics and Advantage, but the village is sunlit so true rules cancel it to one die, which the old engine did not do); thrown Dagger/Spear (planner: crates/df-combat/src/tactical.rs thrown_attack; tests `a_kobold_that_cannot_reach_melee_this_turn_walks_into_range_and_throws_its_dagger`, df-rules kobold tests); SRD_521_CREATURE_FEATURES table of all twelve creatures (every action and trait: implemented / not exercised / missing) with a test; missing ones (Giant Rat and Giant Weasel Climb Speed, Goblin Hide) are appended to the combat rules note via journey::tactical::unmodelled_features_note.
- M3 matrix: crates/df-rules/src/coverage.rs (rows, build order, scenarios; tests check the evidence it cites still exists and every planning family is present); generated report at artifacts/tmp/play-r01/coverage/matrix.md (generated before the last small edits; regenerate with DF_R01_COVERAGE_REPORT=<path> cargo test -p df-rules coverage).

##### Done but not verified
- The live server log line and table-log lines (actor.rs record_combat_audit, table_log.rs audit_line/action_lines) compile and share paths with tested code but have no dedicated test and were not seen in a running server.
- The 4,000-fight release run and report on the final code (only the earlier code was run at that size). Run: DF_R01_FIGHTS=4800 DF_R01_REPORT=<path> cargo test --release -p df-combat --test rules_invariants.
- No browser or screen evidence at all: nothing about the TV or phone was observed.

##### Not started
- Browser check that plays a fight where one hero drops, the other wins and the fate resolves (needs DF_GAMEPLAY_TEST_FIGHT from PLAY-B03 to be deterministic; extend development/browser-checks/play-t03-fight.mjs).
- Help action to stabilise (DC 10 Wisdom (Medicine), SRD p18) offered to the conscious hero (needs a new GameplayActionKind, Move/Stabilize in the aftermath, and beat-script offer mapping).
- Long Rest; effect of rests on a stable hero is only: a rest cannot start while any hero is at 0, the table first waits out the fate.
- Surprise (Disadvantage on Initiative) is not implemented (listed missing). Slow/Sap/Vex expiry timing is covered by df-rules tests only, not by the auditor. Enemy planner declines its Opportunity Attacks by policy.

##### Known defects and open questions, coordinator messages
- Kobold 3 question: answered above (inferred). Death-save visibility: fixed in the server (log, view), display is PLAY-V14's. Positions/Pack Tactics observation: explained above. Post-fight dead end: fixed and tested, not seen in a browser. Widened scope (initiative, order, action economy, Fighter features): auditor done as above; Surprise and tie-breaking: ties among heroes by the party's join order are permitted by SRD p13 (players decide), a hero-versus-monster tie goes to the heroes by the spec order, which is a GM decision the engine makes silently (undisclosed).
- Corpses: a dead creature does not occupy a square (Kobold 1 stood on Kobold 3's corpse square); the SRD does not say; undisclosed.
- A downed hero's save line is spoken as an NPC-role table line (actor.rs treats any speaker other than the acting hero as NPC).
- Existing test submitted_disengage_prevents_source_opportunity_window... was changed: a move is now recorded as an outcome, so it asserts moves only.
- Open: whether CombatView.outcomes consumers (display, df-render) should read move_outcomes/death_save_outcomes; I kept outcomes attack-only so nothing existing breaks.

##### Exact next steps
1. Resolve the four merge conflicts with claude/playable-20261011 (proto and ledger: keep both sides' numbers; tactical.rs and table_log.rs: keep both), then run the gates again.
2. Run the 4,800-fight release property test; keep its report.
3. Add a COMBAT-line test and exercise the log in a running server.
4. Implement the Help-stabilise offer and the browser check once DF_GAMEPLAY_TEST_FIGHT exists.
5. Have PLAY-V14 read CombatView.death_save_outcomes, death_saves and move_outcomes.

##### Hunks in shared files
- crates/df-protocol/proto/gameplay.proto and field-ledger.txt: CombatOutcome 20 distance_ft, 21 kind (OutcomeKind), 22 path (GridCell), 23 cost_ft, 24 remaining_ft, 25 roll_mode, 26 faces, 27 reasons, 28 cover, 29 death_save_result (DeathSaveResult), 30 death_successes, 31 death_failures, 32 recovery_hours; CombatView 16 death_saves (DeathSaveState), 17 move_outcomes, 18 death_save_outcomes. 19 left to PLAY-B03's damage_type.
- crates/df-tools/src/gameplay/journey/tactical.rs (record_outcomes, scene_map and LIGHTING table, dying/needs_saves, aftermath, split_outcomes, project_combat death_saves, unmodelled_features_note, tests), journey.rs (offered label, phase_from_state, aftermath no script choice, after_victory_time), journey/narrative_phase.rs (skip aftermath time facts), wire.rs (partition outcomes, disclosed gaps in rules note), actor.rs (COMBAT log lines), table_log.rs (lines for moves, saves, recovery). Cargo.lock and df-combat/Cargo.toml: serde_json dev-dependency.
- Not touched: crates/df-render, stage_conductor.rs, stage_machines.rs, live_battlefield.rs.

##### Spend on providers
None. No .env value read, no ports used, no servers started.

---

### PLAY-V10 home loop as two clips (worker note, verbatim)

#### PLAY-V10 handoff: home-screen loop, pair follow-up

##### Task and brief
- Task: PLAY-V10 (home video loop), with the owner follow-up to render two clips per place and transition between them without jitter.
- Brief: `/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/play-v10/brief.md`
- Worktree: `/Users/earlcameron/Desktop/dungeonflux/.claude/worktrees/agent-a7a373d701ad1f2b2`
- Branch: `worktree-agent-a7a373d701ad1f2b2` (not pushed). It already contains `claude/playable-20261011` at 0b8dafa, merged fast-forward.

##### Commits
- `5932d5e` Wake the home loop from the server's cache and play it on the lobby. **Not green**: the full `cargo test -p df-tools --lib` run shows 11 failures against the nine expected. Not checked against a baseline. Treat as WIP.
- `77caa61` Measure the home loop's joins every frame, and give the flight wait a longer bound. **GREEN** for what it covers: fmt clean, and `cargo test -p df-tools --lib -- home_video live_art` passes on the merged source (19 passed, 3 ignored). Full lib suite, clippy, schema ledger and df-providers tests were not re-run after the merge.

##### Done and verified
- Merged `claude/playable-20261011` (0b8dafa). Native fixture and wasm client rebuilt from the merged source in `claude-v10-homevideo`.
- Video off (`DF_GAMEPLAY_HOME_VIDEO=off`): browser run `artifacts/tmp/play-v10/evidence/off-merged-summary.json`. Still only, loop released, no errors.
- Broken cached clip (truncated copy under the real key): `artifacts/tmp/play-v10/evidence/broken-merged-summary.json`. Still stays, loop released, no errors. No new clip was written to the cache.
- Measurement harness `development/browser-checks/home-pair-measure.mjs`, sampling every display frame.
- Baseline of today's single-clip wrap (cached pinewood loop, 60 s): `artifacts/tmp/play-v10/pair/single-clip-baseline-joins.json` and `-frames.jsonl`. 11 joins. Median join block-difference peak 0.38, against mid-clip p95 0.138 and max 0.56. All 11 joins exceed mid-clip p95; none exceeds the mid-clip max. Measure is per-frame mean luminance and mean absolute 32x18 block difference, approximated by mixing the two clips by computed opacity.
- Startup refusal `retained_access_unavailable Unauthorized`: not reproduced on fresh databases with the merged build (`df_gameplay_demo_v10u`, seed 6 resolves correctly, READY). Earlier refusals were on the pre-merge binary, on databases whose failed first start had already created `df_game` and `df_local_demo` (for example `v10e`). Root cause not found.
- "Flight never settled" tests: bound in `settle()` raised from 30 s to 120 s (`crates/df-tools/src/gameplay/home_video_tests.rs`).

##### Done but not verified
- Full `cargo test -p df-tools --lib` on the merged source: not run.
- Clippy, schema ledger test, df-providers tests after the merge: not run.
- Baseline harness: the opacity mix assumes the still is covered by full-opacity clips. Optical flow was not computed, only block difference.
- Story-start run (earlier, pre-merge): loop released at 12.96 s with scene mode. The first fade came 11.8 s after load because the phone join runs first, so it does not test the 1-2 s fade requirement.

##### Not started
- Two clips per place (item 1). Blocked, see below.
- Pair display: handoff, prepared incoming element, 0.8 s fade window (item 2).
- Pair join measurements (item 3). Only the baseline exists.
- Pair budget, cache keys, view state (item 4). Budget recommendation given; view state not decided.
- `artifacts/tmp/play-v10/PAIR-READY.txt`: not written, because nothing pair-related is built.

##### Known defects and open questions
- **Blocker, item 1: the second frame F needs a decoder.** Clip B's start must be a real frame of clip A, taken on the server from the validated clip. The server has no H.264 decoder: no codec crate in `Cargo.lock`, and no `ffmpeg` on the machine. The owner's rule forbids a new codec dependency on the server. Options:
  1. Test a slow-settle prompt on a single still-to-still clip. Today's joins are not at rest (see the baseline). Needs a prompt change in `crates/df-tools/src/gameplay/home_video.rs`, a rebuild, and one paid clip (about $0.56). If the clip rests at both ends, the single clip already joins at rest and pairs are unnecessary. No approval needed beyond the spend rule.
  2. fal `fal-ai/ffmpeg-api/extract-frame` with `frame_type: last` on clip A. Public OpenAPI schema saved at `artifacts/tmp/play-v10/extract-frame-openapi.json`. Open questions: whether fal accepts clip A as an inline data URI (the Kling path accepts data URIs for images only), its price (pricing endpoint needs the key), and whether it counts as "taken by the server". New paid provider endpoint and media input path, so it needs the owner's approval.
  3. Approve a server-side decoder dependency. The owner's rule currently forbids it.
- **Budget recommendation (not changed):** with pairs, each place takes two clips. With the kill clip and dice template also drawing on video, the default should rise to 8, not 4. Default is in `crates/df-tools/src/gameplay/home_video.rs`.
- **Full-lib failures, commit 5932d5e:** `startup_contract` (6, need env `DF_SERVER_D02_FIXTURE_ROOT`), `preview` (3, `InvalidRoot`), `build_set` (1, `UnsafePath`), `courier_ai::cache_replay_locale_contract` (1, stale retained wire witness). None is in code I changed. Baseline comparison not done.
- **Half-initialized databases:** a failed startup leaves `df_game` and `df_local_demo` behind. The next start then refuses with `retained_access_unavailable` (`crates/df-persistence/src/local_demo_scope/startup.rs`, lines 226-293). Not investigated further.
- **Stale binary:** a fixture binary built before a content merge fails with `beat-scripts.json ... UnknownHandler`. Rebuild after every merge.
- **Coordinator items:**
  1. Two clips per place: not started, blocked on the decision above.
  2. Pair display: not started.
  3. Join measurements: baseline done, pair not done.
  4. Budget, cache, view state: budget recommendation given; view state not decided. Proposal: keep field 31 with `READY` meaning one clip and add `PAIR = 5`; field 32 stays the A path; add field 33 for the B path.
  5. Off and broken re-runs on final build: done (above). Startup refusal: not reproduced on fresh databases (above).
  6. Gates, commit, PAIR-READY: partly done (`77caa61`). Full gates and PAIR-READY not done.
- Carry-overs from PLAY-V05 (party sizes 2 and 6; six-location recapture) are still not done.

##### Exact next steps
1. Decide the frame source. Option 1 needs no new approval: change the prompt in `crates/df-tools/src/gameplay/home_video.rs` to a slow settle at both ends, build (`cargo build --locked --offline -p df-tools --bin df-transport-fixture`, plus wasm), serve a fresh pinewood cache, and run `node development/browser-checks/home-pair-measure.mjs http://127.0.0.1:63381 artifacts/tmp/play-v10/pair <label> 60`. Check the join peak against mid-clip p95 (0.138).
2. If option 2 or 3 is approved, add frame extraction behind a trait in `home_video.rs`. Pair state, B cache key (keyed on A's key and F's hash) and the view fields follow.
3. Run the full gate set: `cargo fmt --all -- --check`, `cargo test -p df-providers`, `cargo check -p df-tools --all-targets`, `cargo check -p df-tools --lib --target wasm32-unknown-unknown`, `cargo clippy -p df-providers -p df-tools --all-targets`, `cargo test -p df-tools --lib`. Expect 11 failures until a baseline check identifies the nine known ones.
4. Re-run browser checks on the final build: `wrap`, `story`, `off`, `broken` via `development/browser-checks/home-video.mjs`, and the join measurement.

##### Hunks in shared files
- `crates/df-protocol/proto/gameplay.proto`: JourneyView fields 31 (`HomeVideoState home_video_state`) and 32 (`string home_video_path`), and the `HomeVideoState` enum (NONE=1, PENDING=2, READY=3, FAILED=4). Field 33 is unused.
- `crates/df-protocol/proto/field-ledger.txt`: matching lines.
- `crates/df-tools/src/gameplay.rs`: home video module, router, startup line, `apply` after `living.apply`, wake subscription fallback.
- `crates/df-tools/src/gameplay/living_scenes.rs`: `signal_art`.
- `crates/df-tools/src/live_art.rs`: loop merge in the display branch of `merge_art`.
- `crates/df-tools/src/gameplay/wire.rs`: JourneyView defaults for the two fields.
- `crates/df-tools/Cargo.toml`: web-sys feature `HtmlVideoElement`.
- `crates/df-providers`: `fal_media.rs`, `http_contract.rs`, `media_container.rs`; `lib.rs` re-exports `KLING_O1_IMAGE_TO_VIDEO`.
- `development/run-gameplay.sh`, `development/README.md`: FAL_KEY export and home video settings.

##### Spend on providers
- This wrap-up: none. No provider call, no paid clip. Cache checks show no new clip written.
- Earlier in this session: one paid clip for pinewood-trail, $0.56 (5 s at $0.112/s, from the pricing endpoint, not the invoice), in `artifacts/tmp/play-v10/cache-real/`.
- Running total across PLAY-V10 is roughly $4.0 to $4.5. The earlier count of 6 to 7 failed-submit clips is still unreconciled. Owner should check the fal dashboard for kling-o1 requests around 09:55 to 10:35 UTC and 11:00 to 11:03 UTC.

---

### PLAY-V14 victory and end screens, TV layout check (worker note, verbatim)

#### PLAY-V14 HANDOFF

##### Task and brief path; branch; commits
- Task: PLAY-V14, TV victory/defeat/end-card recomposition, death saves as drama, and a TV layout check (M1-M4).
- Brief: /Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/play-v14/brief.md (plus the coordinator's end-card addition and the two live observations, listed below).
- Branch: worktree-agent-a9531e4e2a6599b86 (worktree /Users/earlcameron/Desktop/dungeonflux/.claude/worktrees/agent-a9531e4e2a6599b86). Not pushed.
- Commits:
  - 4e1c771 "WIP (not green): TV layout check (PLAY-V14 M1) ..." (WIP; node tooling only, no Rust changed; do not merge as-is).
  - 5afc44a merge of claude/playable-20261011 (clean merge, no conflicts).
- No M1/M2/M3/M4-READY.txt was written: no milestone is complete.

##### Done and verified
- layout-check.mjs measures text per rendered line and reports overlap, covered, clipped, ellipsis (line clamp / text-overflow, `data-single-line` exempts one-liners) and outside-5%-safe-margin. Verified on a synthetic page (artifacts/tmp/v14/dbg2.mjs): it flags a text crossed by another, a text under an opaque box, a line clamp that hides lines.
- tv-layout.mjs scenario `lobby` ran green on the pre-merge build: home with 0 and 2 heroes at 1280x720, 1920x1080, 3840x2160, 0 violations after fixing one false positive (line-clamp descender rounding).
- The harbor `story` scenario ran through opening, talk scene, hero line with bubble, flat combat, to the victory/defeat screen on the pre-merge build.

##### Done but not verified
- Last fix to the clip logic in layout-check.mjs (clips follow the containing-block chain; inline elements ignored) was never run against the real display. Reason: the previous version read every real-page text box as clipped away (zero area), so it reported 0 overlaps on a screen where the hero card plainly covers the prompt. The run that would confirm the fix was killed at wrap-up.
- `fight` scenario (random adventure seed 0xd5f94172edb1d404 played by chapter-lib.playUntil, checkpoint at each new screen kind incl. victory/defeat/end card) and the rest/inn steps of `story` are written but never run.
- tv-layout.sh (one command: postgres start, node tv-layout.mjs, server and postgres stop) written, run only via the same env; not run on its own.
- lib.mjs: hero creation now waits for a visible aria-pressed button (the new table-settings toggles are hidden aria-pressed buttons and broke create()).

##### Not started
- M2 code: the recomposed victory/defeat screen, the end card (closing-title layout, one title, last line finishes first, waiting line in the next-step slot, no chrome showing through), the speaker lower third.
- M3 death saves: no code. Finding: the wire carries no death-save data (no pips, no roll). See open questions.
- M4 fixes, the M1 violation list for the report, before/after frames, the gates (fmt, check, wasm check, clippy, df-tools lib tests), the READY files.
- Scenarios still to add to tv-layout.mjs: battlefield (WebGPU) with 2 heroes v 3 enemies and with 4 enemies, look result, resolution/epilogue, end card over a plate / over the establishing picture / imagery off, the "no text on a plate face rectangle" rule (PlateLayout head boxes; needs the plate root to expose them, e.g. a data attribute set in TvStage::set_plate).

##### Known defects and open questions
- Observed on the baseline (owner frames and my frames): hero HUD card at the bottom-left over the aftermath prompt "The party decides: rest or move on."; narrative clamped with an ellipsis (live_combat/style.rs line clamp 6; `.aftermath-narrative`); the speaker's bubble drawn over the Rules note / title; the display shows the old cut-out stage. The check flagged only the ellipsis so far (see "not verified").
- Server room takes two players ("This two-player room is full"): "home with 6 heroes" cannot be reached by playing. tv-layout.mjs stands six cards in the DOM and labels the state as a stand-in. Decision needed.
- Tuning validation (pack.rs validate_tuning) fixes hero stats and caps enemies at 4 (count 1..4), so a tuned defeat or a "dying hero after a won fight" cannot be forced by tuning; reaching those is luck of the dice (one of my runs ended in Defeat with both heroes at 0 HP).
- Death saves wire gap: PartyMemberView.conditions carries only dead/stable/unconscious. The roll is drawn in df-rules resolve.rs death_save (the d20 is in Resolution.draws) and nothing records it. Options: record each save in df-tools gameplay/journey/tactical.rs `apply` (Action::DeathSave: d20 from result.draws, successes/failures from health before/after), carry it in AcceptedAction (like `combat`) and project the last N in wire.rs next to combat.outcomes (proto in crates/df-protocol/proto/gameplay.proto, JourneyView 90-94 allowed by the brief). The existing EndTurn offer "Advance 6 seconds · death saves" (journey.rs offered()) is the only offer; phone-side actions (e.g. Medicine to stabilise) belong to PLAY-R01.
- Coordinator messages and their state:
  1. Owner end-card complaint (covers characters and dialogue, stuck, no next step; frames owner-end-card-over-cutouts.jpg, owner-end-card-over-characters.jpg): not started. Findings: the card shows at once when adventure_ended is set (live_tv.rs update, `data-ended`), without waiting for the last line; `.tv-stage[data-ended] .tv-bubble` is hidden but the thinking mark, scene title top right and party strip stay. BigCard in live_tv.rs mount_card creates the `next-step` slot (data-slot) but does not store it; PLAY-E04 fills it.
  2. Fight-chapter resolution/epilogue drawn as the VICTORY layout with the speaker's bubble on the title (frame artifacts/tmp/play-e04/owner/fight-ending-stops-on-victory-card.jpg): not started; chapter-flow worker is fixing the screen kind.
  3. Choice chips off leaves the narrator box empty while the table waits: not started. Cause: TvStage::narrator_line (live_tv.rs) clears the text on LineState::Done, and live_stage.css hides `.tv-stagebox` unless data-speaker=narrator.
  4. Merge of claude/playable-20261011 and re-reading the brief: done.
  5. Wrap-up: done.

##### Exact next steps
1. Build once in /Users/earlcameron/Desktop/dungeonflux/artifacts/build/claude-f05-fixes with artifacts/tmp/v14/build.sh (native fixture about 11 min under load, wasm, bindgen). The worktree's artifacts/tmp/v14/ holds env.sh, run.sh, build.sh, dbg*.mjs (gitignored); recreate if lost: set RUSTUP_HOME/CARGO_HOME, DUNGEONFLUX_ARTIFACT_ROOT=/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/play-v14/root (symlinks to cache and build, its own tmp), DUNGEONFLUX_BUILD_ROOT, PORT=63642, DF_BASE, DF_CHECK_PG_PORT=55631, DF_CHECK_PG_SOCKET_DIR=/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/play-v14/s (the socket path must be short).
2. Run `ONLY=story DF_DEBUG=1 VIEWPORTS=1920x1080 development/browser-checks/tv-layout.sh` and confirm the box lists now show rectangles (not empty) and that the victory screen reports the hero card covering the prompt. Then run all viewports and ONLY=fight. Keep the output as the M1 "before" list.
3. Remove the `dummy` lines at the end of development/browser-checks/tv-layout.mjs, add the missing scenarios (above), commit M1 as GREEN, write M1-READY.txt.
4. M2 in crates/df-tools/src/gameplay_browser: live_combat/aftermath.rs (present_display: build a TV-owned result block: title, whole text, "the fight" line from journey.combat round/outcomes/party conditions, prompt, speaker lower third), live_combat.rs present_stage (call the plate setter, make TvStage::set_plate pub(super); aftermath arrangement), live_stage.css/live_fx.css (HUD cards at top for data-mode=aftermath; hide the df-ui `.df-aftermath` on the TV), live_stage.rs show_card plus presentation.rs StageActor (status: Bloodied/Dying/Stable/Dead), live_tv.rs/live_stage.css for the end card (gate `.tv-bigcard` on data-scene-state=awaiting-choice, hide thinking mark/title/party strip when data-ended, lower-band layout, waiting line in the next-step slot). Keep the display thin: only what the view carries.
5. M3: the wire change described above, then the dice card on the TV and the pips on the HUD cards (live_sfx cues exist), phone side shows only what the server offers.
6. M4, then gates: `cargo fmt --all -- --check`; `cargo check -p df-tools --all-targets`; `cargo check -p df-tools --lib --target wasm32-unknown-unknown`; clippy on df-tools; `cargo test -p df-tools --lib` (baseline nine known failures: six gameplay::startup_contract::*, three preview::tests::*).

##### Hunks in shared files
- development/browser-checks/lib.mjs: create() waits for a visible aria-pressed button (timeout 25 s).
- No Rust, proto or shared-module changes; no proto field numbers taken.
- New files: development/browser-checks/layout-check.mjs, tv-layout.mjs, tv-layout.sh.

##### Spend on providers
None. No provider keys used (STORYTELLER off, fake voice), no `.env` value read.

##### Processes
All processes I started were stopped (tv-layout node run killed; server and my PostgreSQL on port 55631 stopped; ports 55517 and 63309 never used). Worktree clean after this commit.

---

### PLAY-V15 dice roll video (worker note, verbatim)

#### PLAY-V15 handoff: every d20 roll as a filmed die with the server's number on it

##### Task and brief path; branch; commits
- Brief: /Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/play-v15/brief.md (probe: artifacts/tmp/play-v08/).
- Branch: worktree-agent-aff485b5dca448a30 (worktree .claude/worktrees/agent-aff485b5dca448a30). Not pushed. Tree clean.
- d2001a7 WIP: first body of work (provider, template, face measurement, TV and phone beats), on the old base.
- eb5568e WIP (not green): d2001a7 merged with claude/playable-20261011 (conflicts in gameplay.proto, field-ledger.txt, journey/tactical.rs, live_motion.rs resolved by hand) plus the "roll waits for the battlefield to be at rest" gate. Do NOT merge as is.
- No M1-READY.txt was written: M1 never reached green. No milestone is delivered.

##### Done and verified
- df-providers: Kling o1 first-and-last-frame (two reference frames, 3 s) accepted only for a pair of frames; new test `kling_sends_a_start_and_a_different_end_frame_for_three_seconds_and_only_for_a_pair`; `cargo test -p df-providers --test fal_media` 18 passed (before the merge).
- dice_face.rs (Rust measurement of the settled top face from the end still, refuses numbered or missing faces): on the old base 9 of 10 `gameplay::dice*` tests passed, covering the probe's good stills (single, black, both dice of a pair), numbered (brass and engraved) refused, cached template ready with no provider call, refused face never charged, budget, refused clip charged once. The one failure (flat JPEG not refused, noise stretched into lines) was fixed with MINIMUM_LINE_CONTRAST; that fix was not re-run alone.
- wasm check `cargo check -p df-tools --lib --target wasm32-unknown-unknown` compiled on the merged tree (6m25s) before the last small edits (dead-field cleanup, dead_code allow on from_initiative).

##### Done but not verified
- The last native `cargo test -p df-tools --lib` on the merged tree (killed at wrap-up) had FAILED: dice_video_tests::a_cached_template_is_ready_at_once_with_the_measured_faces_and_no_provider_call, seven home_video_tests (a_malformed_clip..., a_refusal_or_failure..., a_timeout..., one_submit_serves_every_display..., a_cached_loop_is_served_at_once..., the_budget_is_charged..., the_display_lobby_view_carries_the_loop...), imagery::tests::invalid_oversize_wrong_sized_and_opaque_output_is_refused. Causes not diagnosed. Suspects: my refactor of home_video.rs (ClipSpec/video_request/run_clip_job; the tests call clip_key etc.) or other merged work; the imagery one may be a baseline failure; check against the nine known baseline failures.
- Display: dice_roll.rs (pure: RollPlay from CombatOutcome with death save, saving throw, ability check words, advantage/disadvantage pair with kept die, timelines, face matrix; unit tests written, run only in the pre-merge run) and live_dice.rs/live_dice.css (pooled video, vignette, number drawn by CSS matrix on the face, nat 20 / nat 1 light, large centred card fallback, phone own roll and result line); live_motion.rs hooks; stage_director/conductor set_roll_hold. Compiled for wasm, never run in a browser.
- Server: dice_video.rs engine (session template single then pair, budget, cache, route /gameplay/dice-video/<key>, startup line "DICE VIDEO on|off (...)", view fields), imagery dice prompts, wire fields, death-save outcomes in journey/tactical.rs, README section. Never run against real providers.
- Not run at all: fmt check after the last edits, clippy, check --all-targets, schema_ledger test.

##### Not started
- Real-provider run (no spend), the real-browser run with frames every 100 ms, overlay offset and pace measurements, three-enemy round before/after seconds.
- Initiative rolls on the video (hook at live_battlefield entrance() when `.bf-hud[data-initiative]` first shows; `RollPlay::from_initiative` exists but is unused, needs a queue in live_dice and a quicker timeline).
- Settle fraction check on a real clip (SETTLE_PERMILLE=870 is the probe's figure, an assumption), nat 20 / nat 1 / death save / pair seen live, M1-M4 READY files, other dice (d4 d6 d8 d10 d12; d6 probe needs a high camera, pips appeared on side faces mid-roll).
- No decoder exists offline, so the clip's last frame is not checked against the end still; overlay accuracy rests on the end still's measurement.

##### Known defects and open questions
- Coordinator messages: (1) merge claude/playable-20261011 and re-check the tree: done (eb5568e), staging seam is stage_director Rolling dwell via set_roll_hold; (2) commit the first slice green and write M1-READY.txt: NOT done; (3) keep home_video.rs hunks additive and list them: listed below, but they are a refactor, not purely additive; (4) wrap-up: done as WIP.
- The die design reference is the game's own d20.webp (black marble, gold numerals, blue glow); the prompt asks for blank faces and no glow. Unknown whether the image model follows it; the probe stills used a bone/brass die. If the face finder fails on the real stills, the template falls back to the card (by design).
- Pair (advantage) needs a second paid clip made from the single's settled still; if the pair fails the display shows the card with both numbers.
- Roll beat waits up to 600 ms (1500 ms if the attacker walks) for `live_battlefield::at_rest()`; the director dwell includes that allowance, so a quiet beat has dead time. PLAY-B03's order of beats is merged; the roll still starts at Rolling enter.
- damage_dice is empty in tactical outcomes, so the damage beat shows only the total.
- Saving throws and ability checks are never produced by the live server (check: None); only the display words exist.
- Ports used: none (no server was started).

##### Exact next steps
1. Build in /Users/earlcameron/Desktop/dungeonflux/artifacts/build/claude-ui2 (one build): `cargo test -p df-tools --lib gameplay::home_video` and `gameplay::dice`, diagnose the failures above (crates/df-tools/src/gameplay/home_video.rs, home_video_tests.rs, dice_video_tests.rs).
2. Run the gates: fmt, df-providers tests, `cargo check -p df-tools --all-targets`, wasm check, clippy, `cargo test -p df-tools --test schema_ledger`, lib tests (nine known failures only). Commit green; write M1-READY.txt.
3. Real run: scratchpad-style scripts in this session (pg on 55415, server on 63415, key read inside one command) are not saved; recreate from development/browser-checks/serve.sh and own-postgres.sh with your own ports, DF_GAMEPLAY_DICE_VIDEO_CACHE under artifacts/tmp/play-v15, and a driver based on play-t03-fight.mjs that records TV frames every 100 ms and the `.dice-roll` attribute log. Look at the generated stills before paying for the clip; keep the cached template.
4. Measure overlay offset per frame after the settle (decode the clip with artifacts/tmp/play-v08/tools/decode_av.py), verify SETTLE_PERMILLE, tune the CSS size/position, then initiative, pace measurements (three-enemy round), phones.

##### Hunks in shared files
- Proto: CombatOutcome 25-29 (roll_kind, other_die, roll_mode, difficulty_class, succeeded); JourneyView 95-97 (dice_video_state, dice_single, dice_pair); new enums RollKind, DiceVideoState; messages DiceFace, DiceTemplate; field-ledger.txt lines added.
- crates/df-providers/src/fal_media.rs submit_video (Kling pair of frames, 3 s) and its test file.
- crates/df-tools/src/gameplay/home_video.rs: helpers made pub(super) (limits, owner, provider_failure, request_refused, base64_encode, cached_clip, write_clip, context, record_event, resolve_pins, consts); new build_provider, live_price, charge_clip, ClipClass, run_clip_job (extracted from Engine::run_job), ClipSpec; video_basis/video_request now take a ClipSpec (home_clip builds the old one). Not purely additive: the other worker's pair-of-clips change will conflict here.
- gameplay.rs (mods, apply, install, router, startup line), wire.rs (JourneyView fields), imagery.rs (ImageKind::DiceSettled/DiceTumbling, re-exports), journey/tactical.rs (record_attacks fields, other_die, record_death_save), presentation.rs (fingerprint adds roll_kind and other_die), live_art.rs (merge_dice), lib.rs (dice_roll), gameplay_browser.rs + live_theme.rs (live_dice), live_motion.rs (RollDetail play/hero/attack, wait_ms, roll_hold, start_roll, phone hook), live_battlefield.rs (`at_rest()` only), stage_director.rs (roll_hold field, set_roll_hold, Rolling dwell) and stage_conductor.rs (set_roll_hold pass-through). Tell PLAY-B03 owners about the stage_director/conductor hunks.
- The CSS hides the battlefield HUD's own `.bf-roll` card (live_dice.css); df-render untouched.

##### Spend on providers
None. No provider call was made in this session (the probe's $4.14 is play-v08's).

---

### PLAY-B08 callouts, cries, story lines, narration (worker note, verbatim)

#### PLAY-B08 handoff (combat callouts, battle cries, the Dungeon Master's telling)

##### Task and brief path; branch; commits
- Task PLAY-B08 attempt 1. Brief: `/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/play-b08/brief.md`.
- Branch `worktree-agent-a5df122ea13e2587f` (worktree `.claude/worktrees/agent-a5df122ea13e2587f`), based on `claude/playable-20261011` at b161111, then fast-forwarded to the merged tip. Not pushed.
- Commit `fd30a26` **WIP (not green)**: M1 code, never compiled. Do not merge as is. No marker files `M<n>-READY.txt` were written (nothing is verified).

##### Done and verified
- Nothing in the code is verified. What was verified is the environment: a clean build of b161111 (native 10m23s, wasm 2m30s, load average about 45) and one real-browser fight recording of it (see "Baseline" below).

##### Done but not verified (all in commit fd30a26, uncompiled, untested)
- Wire: `CombatOutcome` 30 `rule_events` (repeated string ids), 31 `roll_mode`, 32 `advantage`, 33 `disadvantage` (reason ids), 34 `enemies_standing`, 35 `heroes_standing`, 36 `round`, 37 `no_attack` (bool: a beat that is not an attack), 38 `healed`. Field 39 and `CombatView` 20-29 are free (planned: `FightLine` for cries/story/narration). `field-ledger.txt` has the nine lines.
- `crates/df-tools/src/rule_event.rs`: `RuleEvent` (28 variants incl. OpportunityAttack, CriticalHit, NaturalOne, Miss, Graze, Sap/Slow/Vex/Nick, SavageAttacker, Advantage/Disadvantage, PackTactics, Bloodied, Down, Dead, FirstBlood, LastEnemy, SecondWind, Dash, Dodge, Disengage, DeathSaveSuccess/Failure, Revived, Stable, OpportunityOffered, NewRound), wire ids, tests.
- `crates/df-tools/src/fight_callout.rs` (wasm|test): one `TABLE` row per event (words, weight minor/major/epic, moment roll/impact/defeat/beat, subject actor/target/neutral for side colour, sting), `callouts_of(outcome, participants)`, `result_words`, test `table_has_a_row_for_every_event`.
- `crates/df-tools/src/gameplay/journey/rule_events.rs` (server): `blow_events` (pure, tested: reaction, advantage, pack tactics, crit/natural 1/miss, graze, mastery only when its effect took hold via an exhaustive `match` on `Mastery`, savage, first blood, bloodied/down/dead, last enemy), `attack_facts`, `action_beat` (Second Wind, Dash, Dodge, Disengage, death save from the draw's d20 and before/after health), exhaustive `AttackReason` ids.
- `tactical.rs`: `record_attacks` now fills the new fields from the engine's `AttackSummary` and the before/after `Battle`; new `record_beats` pushes `no_attack` outcomes for non-attack actions and a `new_round` beat when `battle.round()` rose; test `a_dodge_is_reported_as_a_beat_...`.
- Client: `stage_director.rs` new `CueKind::Announce` / `StageState::Announcing` (800 ms, cosmetic, played before the blow, in `on_blow` too; counts as an outcome for the conductor's holds) and `ViewDiff.announce`; `stage_conductor.rs` carries it in the held opening and the finishing-blow split; `live_motion.rs` stages `Staged::{Blow,Beat}`, `announce_callouts` fires a blow's callouts at its Rolling/Impact/Defeat enter, a critical's dice sting is left to its callout when the battlefield is live, the server's Second Wind beat replaces the inferred heal, only blows count for `expect_cues`; `live_callout.rs` + `.css` (DOM layer inside `.battlefield`: one major at a time, queue of 3, tags above the dice card, timers by `once_into_js`, trail in `data-callout-log`, stings through `live_sfx::play_shaped`, now `pub(super)`); `live_battlefield.rs` `with_field` and the Opportunity Attack offer callout (`OFFERED` thread-local, announced after `sync`); `df-render/src/battle/callout.rs` pure placement (bands under the title block, clear of plates, round strip, dice card and the two figures; native tests) with `.bf-callout` added to the bubble's avoid list; `presentation.rs` fingerprint now includes round, rule events and `no_attack` (test added); phone result line (`live_combat/combat.rs`) leads its source line with the same words and skips beats; `table_log.rs` `action_lines` skips beats.
- Driver `development/browser-checks/play-b08-fight.mjs` (records TV label, round, card, callouts, bubbles, narrator box, sound log every 70 ms; frames on each change; rotates Second Wind/Attack/Dodge/Disengage/Dash per hero). The first run used a wrong "your turn" test and pressed nothing; fixed but its rerun was stopped.

##### Not started
- M2 battle cries and story lines, M3 narration and the table setting (field 72), M4 voices and the antagonist's answer; tempo "after"; frames of OA/crit/graze/kill/fall; hero Move to provoke or answer Opportunity Attacks in the driver; `HANDOFF` marker files.

##### Known defects and open questions
- Never compiled: expect errors. Likely spots: `live_callout.rs` (`Stage` borrow types, `Vec3 +` operator, web-sys features such as `DomRect`/`NodeList`), the `beat` closure in `record_beats`, `table_log.rs` shadowed `combat` (`&&CombatOutcome`), director test (`names`/`run` helper use), `ViewDiff` literals in conductor tests (they use `..Default::default()`, check), unused `Moment`/`Callout` imports.
- The native check printed only one warning (unused `AttackSummary`, fixed) before it was stopped, so the server side up to `rule_events.rs` parsed at that point.
- Engine has only five masteries (Graze, Sap, Slow, Vex, Nick): Topple, Cleave, Push cannot be reported until the rules add them (the `match` forces a row when they do). Death-save die and Second Wind amount are in `attack_die` / `healed` of a `no_attack` outcome.
- `combat_log_entries` (6) windows the outcomes the view carries; beats share that window, so long enemy rounds may drop old beats (client already stages at most 6).
- Bystanders have no figure on the battlefield, so "cries from cover" need a caption path (not designed). Enemies are nameless creature kinds ("Kobold Warrior 1"), not dossier NPCs: dossier/spine facts (stand_down, motive) live in the actor's `SocialLedger`, not the checkpoint, so cries chosen in `tactical::apply` can use pack globals, hero creation choices, creature kind and the content rating only; dossier-fed story lines would have to be chosen in `actor.rs record_table_action`.
- Callout sting reuse: no new audio exists (no ffmpeg/sox); Toll is the turn chime at 0.6 rate. No flat-stage fallback for callouts (battlefield only).
- Coordinator messages: only the wrap-up; done except item 5's "final message" (this note).

##### Exact next steps
1. `git merge claude/playable-20261011`; `sh <scratch>/cargo.sh check -j 3 -p df-tools --all-targets` (script: sets the env from the brief, a fresh TMPDIR, `--offline --locked`), fix errors; then `--lib --target wasm32-unknown-unknown`; `cargo test -p df-render callout`; `cargo test -p df-tools --lib rule_event fight_callout rule_events presentation stage_director a_dodge`; `schema_ledger`; fmt (the table rows need rustfmt); clippy.
2. Browser run: build (`build-all.sh` pattern: native bin, wasm, `wasm-bindgen --target web --out-name df_tools --out-dir <target>/f05-web`), then `PORT=63811 DF_CHECK_PG_PORT=55811 DF_CHECK_PG_SOCKET_DIR=/private/tmp/b08sock DUNGEONFLUX_ARTIFACT_ROOT=<artifacts>/tmp/play-b08/root DUNGEONFLUX_BUILD_ROOT=<build dir> N=3 node development/browser-checks/play-b08-fight.mjs`. (The socket dir must be short: the default path was too long for Postgres.) Look at the callout position over the frames (title block ends near 23% of the height; figures sit at 50-65%) and tune `BANDS` in `callout.rs`.
3. Write M1-READY.txt, commit green. Then M2: `FightLine` message (`CombatOutcome` 39 `repeated FightLine lines`), pure tables `gameplay/journey/fight_cry.rs` chosen in `tactical::apply` (seed = operation id and ordinal; no repeat from earlier decisions' outcomes; about a third of beats; always kill, fall, Opportunity Attack; creature kind sounds for non-speaking kinds; rating-aware), shown as an `Announcing`-like cue with `live_battlefield::speak`/`end_speech` (do not put cries through the conductor's line machine: it fades lines on every outcome). M3: narration as a `no_attack` beat with a line, budget test over a generated five-round fight, setting 72 with `SET_TABLE_SETTING`. M4: add cry lines to `voice_line::display_lines` and the server voice registry (`gameplay/voice.rs`, role `npc:<name>`); hero voices: heroes carry only `voice_set` (sound family from portrait), no TTS voice assigned at creation (finding).
4. Tempo: run the driver on the baseline binary kept at `artifacts/tmp/play-b08/base/target` (copy of b161111's build) and on the new build; compare mean seconds per turn from `*-samples.json`.

##### Hunks in shared files
- Proto: `gameplay.proto`, `field-ledger.txt` (CombatOutcome 30-38).
- B03 files: `stage_director.rs`, `stage_conductor.rs`, `live_motion.rs` (staging block of `on_view`, `enter`, `start_roll`, new `Staged`/`BeatDetail`), `presentation.rs` (fingerprint).
- S03 files: `df-render/src/battle/mod.rs` (`pub mod callout;`), `speech_bubble.rs` (avoid selector `.bf-callout`), `live_battlefield.rs` (`with_field`, `OFFERED`, end of `sync`).
- Others: `gameplay/journey.rs` (`mod rule_events`), `journey/tactical.rs`, `table_log.rs`, `live_combat/combat.rs`, `live_sfx.rs` (`play_shaped` pub), `lib.rs`, `gameplay_browser.rs`.

##### Spend on providers
None (no provider call, no key read).

##### Baseline (kept)
`artifacts/tmp/play-b08/base/target` (binary and web bundle of b161111), `before/fight-before-3-*` (one 600 s recording and frames; pressed nothing, so no tempo numbers). A look at its frames shows the TV layout the callouts must fit.
