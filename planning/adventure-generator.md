# Random adventure generator (PLAY-G0)

Status: design plus authored roll tables; no code. Grounded in commit `c44c818` (line numbers below refer to it). Paths are relative to the repository root; `df-tools/gameplay/` means `crates/df-tools/src/gameplay/`.

Governing documents: AGENTS.md; [Gameplay tuning surface](gameplay-tuning.md) (pack and tuning files, revision pins, departures); [Generated content](generated-content.md) (typed templates, never executable; standard mode never silently changes rules); [Narrative engine](narrative-engine.md); [Runtime directors](runtime-directors.md); [Rules support](rules-support.md). Creature numbers come from the pinned SRD 5.2.1 (`development/rules-sources/manifest.json`, PDF SHA-256 `8974902d...5e3d87`, CC-BY-4.0). All table prose is original; nothing is copied from the Dungeon Master's Guide or any other non-SRD book.

Tables live in `content/adventure-tables/` (index: `tables.json`). The art pool prompts are in `content/adventure-tables/art-manifest.json`.

## 1. Purpose and non-goals

Purpose: every new game plays a different short adventure, produced the way a Dungeon Master improvises one. The server rolls named dice on tables (location, hook, patron, stake, antagonist, complication, secret, destination, contact, reward, names, traits, wording), records every roll, and stitches the results into the pack and tuning files the loader already understands. The whole adventure is a pure function of one 64-bit seed, the tables revision and the rules mode. No model call is needed to get a playable, readable adventure.

Non-goals (v1):
- No hand-written story, no fixed paragraph per slot: every spoken or shown sentence is a rolled pattern with several variants.
- No new rules. Mechanics come from the existing single-enemy melee fight. Anything the engine cannot express is excluded or flagged (section 9).
- No branching beyond the two approaches, no multiple enemies, no skill checks, no loot or XP (section 11).
- No model-written prose is required. The language-model expression step (section 7) is optional and can only re-word facts that dice already decided.
- No client logic. Clients render what the server sends; the generator runs on the server before a run starts.
- This is a CHAPTER generator. The campaign layer (section 13 onward) chains chapters; nothing in the chapter design depends on it.

## 2. Beat structure and the fields each beat fills

The five-beat classic short-adventure arc stays exactly as the engine plays it today: (1) Gathering, (2) Hook, (3) Roleplay with a private secret, ending in the antagonist's arrival, (4) Confrontation, (5) Aftermath, rest and travel to the resolution scene. Pack field names are those of `content/harbor-journey/pack.json` (the in-flight pack work is expected to keep them; if it renames one, map 1:1). Scene keys (`room`, `opening`, `dialogue-seal`, `dialogue-escort`, `combat`, `inn`, `victory`, `defeat`) stay as internal ids because the journey phases and `SceneKey` (`pack.rs:545`) are compiled against them; only their text and art are generated.

| Beat | Field path | Filled from |
|---|---|---|
| all | `id`, tuning `pack` | `adv-` + 16 hex digits of the seed |
| 1 Gathering | `scenes[room].title` | `prose.room_title` x `locations` |
| 1 | `scenes[room].narration` | `prose.room_narration` + `locations.atmosphere` x2 |
| 1 | `scenes[*].asset` (room, opening, dialogues, combat, victory, defeat) | `locations.art.arrival` |
| 1 | `offer_labels.begin-story` | `prose.begin_label` x `locations` |
| 2 Hook | `scenes[opening].title` | `hooks.title` |
| 2 | `scenes[opening].narration` | `prose.opening_narration` + `hooks.entrance` + `patrons.desc` + `prose.foreshadow_*` (by antagonist group) + `locations.atmosphere` |
| 2 | `scenes[opening].speaker`, `.dialogue` | patron first name; `hooks.pitch` x `macguffins` x `contacts` |
| 3 Roleplay | `offer_labels.ask-courier` | `prose.ask_label` x patron first name |
| 3 | `offer_labels.escort-courier` | `prose.help_label` |
| 3 | `scenes[dialogue-seal]` (question branch) | `prose.ask_title`, `prose.ask_narration` + `npc-traits.mannerisms` + `antagonists.arrivals`; dialogue `hooks.answer` |
| 3 | `scenes[dialogue-escort]` (help branch) | `prose.help_title`, `prose.help_narration` + `antagonists.arrivals`; dialogue `hooks.accept` (names destination and contact) |
| 3 | `courier_clue` (private) | `prose.clue` + `prose.route_<kind>` + `secrets.fact` |
| 3 | `escort_support_suffix` | `prose.escort_support_suffix` x `patrons.role` (keeps its single `{name}`) |
| 3 | `offer_labels.defend-courier` | `prose.stand_label` x `antagonists.the` |
| 4 Confrontation | `scenes[combat]` title, narration | `prose.combat_title`, `prose.combat_open_*` + fixed `combat_mech` |
| 4 | `scenes[combat].speaker`, `.dialogue` | antagonist name and `antagonists.warnings` (speaking creatures only; empty otherwise) |
| 4 | `bandit.name` (to be renamed `antagonist.name`) | `names` x `antagonists.name_form` |
| 4 | tuning `bandit.*` (hit_points, armor_class, abilities, attack_bonus, damage_modifier, weapon dice) | `antagonists.stats`, copied from the SRD |
| 5 Aftermath | `scenes[victory]` title, narration, `narration_rested` | `prose.victory_title`, `victory_lead` + `complications` + fixed `victory_mech`, `victory_rested` |
| 5 | `scenes[victory].speaker`, `.dialogue` | patron first name; `prose.victory_dialogue` |
| 5 | `scenes[defeat]` | `prose.defeat_title`, `defeat_<kind>` + fixed `defeat_mech`, `defeat_rested` |
| 5 Resolution | `offer_labels.choose-harbor-scene` and the destination option label | `prose.continue_label`, `destinations.name` |
| 5 | `scenes[inn]` title, narration | `prose.inn_title`, `destinations.arrival` + `prose.inn_intro` x `contacts` |
| 5 | `scenes[inn].speaker`, `.dialogue` | contact first name; `prose.inn_dialogue` + `secrets.confirm` + `rewards` |
| 5 | `scenes[inn].asset` | `locations.art.interior` |

Fixed, not generated: `creation`, `equipment`, `room_code`, the rules-facing labels (`create-character`, `greatsword-attack`, `second-wind`, `end-turn`, `short-rest`), and the mechanical sentences. Mechanical sentences (`combat_mech`, `victory_mech`, `victory_rested`, `defeat_mech`, `defeat_rested`) are fixed templates whose only slots are names; they state outcomes the rules produced and no table or model may alter them. The 220-character narration bound applies to the generated (story) part only.

## 3. The roll procedure

Randomness: SplitMix64 seeded from the 64-bit seed (initialisation, step and unbiased `roll(N)` are specified verbatim in `tables.json` under `rng`). The server draws the seed from its entropy source; `DF_ADVENTURE_SEED` may override it for development, and then the run is flagged as a seeded-dice run (gameplay-tuning section 7). Seeds should be entropy-drawn; the initial finalizer pass makes adjacent seeds produce unrelated streams (checked: consecutive golden-ratio multiples without it produced near-identical adventures).

Order (also in `tables.json` `roll_order`); each step rolls the named die on the named table:

1. `location` d8.
2. `hook` d12, compatible with location tags.
3. `patron` d10, compatible with location tags.
4. `stake` (the MacGuffin) d20, tags match the hook's `stake_tags`.
5. `antagonist` d12, compatible with location tags; in standard mode only `fidelity: exact`.
6. `complication` d20, must suit the antagonist group (speaker, beast, undead).
7. `secret` d12, hook kind must be allowed.
8. `destination` d4, on the sub-table for the rolled location.
9. `contact` d6, compatible with location tags.
10. `reward` d12 (story-only thanks).
11. Names: patron given and surname (d20, d20), contact given and surname, antagonist name (person: d20+d20; goblin or kobold: d12; creature: d12 adjective; undead: d8 adjective). Contact given name and surname must differ from the patron's.
12. Traits: patron and contact mannerism (d12) and speech direction (d12); antagonist menace (d8) if it speaks.
13. Atmosphere a, b, c (d6 each, no repeats).
14. Prose variants in beat order (d4 each; the antagonist's arrival and, if it speaks, warning also d4).

Selection: an entry owns a face range. A list of n entries uses the smallest die in {4,6,8,10,12,20} with n <= die; faces are spread evenly and the first (die mod n) entries take one extra face. If the entry is incompatible with earlier results, the roll is recorded as rejected and the same die is rolled again, exactly as a DM says "that does not fit, roll again". After 64 rejections, the first compatible entry in table order is taken and recorded as `fallback` (natural 0). A load-time lint must prove that every upstream result leaves at least one compatible entry in each downstream table.

Measured on the reference model (100,000 random seeds, standard and custom modes): mean 55.6 recorded rolls and 6.1 rejected per adventure, no fallback, no text limit exceeded. Rejections are many because a d20 complication table is shared by three antagonist groups; this is cheap and keeps the record honest. An implementation may pre-filter the face ranges instead only if the recorded rolls stay reproducible from the same rule.

## 4. Coherence rules

Tags instead of a cross product. Location tags: `water town wild mountain sacred grave farm scholar market`.
- Hooks, patrons, contacts and antagonists list the location tags they fit (any-of). Every location has at least 4 patrons, 8 hooks, 3 contacts and 4 standard-mode antagonists (verified).
- Hook kind (`deliver`, `escort`, `warn`, `retrieve`) constrains the stake (`stake_tags`: carry, message, heirloom, remedy), the secret (`kinds`) and the routing sentence (`route_<kind>`) and defeat sentence (`defeat_<kind>`).
- The antagonist group (person and goblinoid speak; beasts and undead do not) constrains the complication, the foreshadowing, the combat opening and whether a warning line exists.
- The destination belongs to the rolled location (4 per location), so the contact always waits somewhere plausible; contacts fit the location tags.
- Gender is fixed per archetype and drives pronouns, name pool, portrait and voice; creatures use "it".

Distinct adventures (computed by exact enumeration of compatible tables, ignoring wording): 651,806,592 story skeletons in standard mode (location x hook x patron x stake x antagonist x complication x secret x destination x contact x reward), 952,886,592 when simplified antagonists are allowed. Names and traits multiply that by about 10^11 and the prose variants by about 10^14, so a repeat is not a practical concern; the meaningful variety is the skeleton count.

## 5. Seed, "DM's rolls" record and persistence

Record (`adventure.json`, schema `df-adventure-record/1`): `seed` (hex), `tables_revision` (SHA-256 over the bytes of the files listed in `tables.json`, in order), `generator_version`, `rules_mode` (`standard` or `custom_allowed`), `rolls[]` as `{n, table, die, natural, entry_id, status: ok|rejected|fallback}`, `expression` (`patterns` or `model`, with model id and the SHA-256 of the fact sheet and of the accepted output), and the SHA-256 of the produced `pack.json` and `tuning.json`. A player-facing "DM's rolls" panel may show the story rolls (steps 1-10) and hide the rest.

Binding. A generated pack is just a pack whose bytes are produced at new-run time; the existing pin mechanism (gameplay-tuning section 5) is unchanged and does all the work:
- `pack.json`, `tuning.json` and `adventure.json` are written once, before the run's first checkpoint, to a run directory named for the run's database (`df_gameplay_demo_<name>`; v1 stores runs outside `artifacts/` cleanup, for example `development/runs/<database>/`, owner choice). The loader reads exactly those bytes (`DF_GAMEPLAY_PACK_DIR` points at the run directory) and their SHA-256 values flow into `CheckpointPins` as already designed. A restart resumes the same adventure because the same bytes hash to the same pins; a missing or edited file fails the pin check and refuses to resume.
- The target store is PostgreSQL: the three files as one immutable row in the run's schema, written in the transaction that creates the run. The directory is then a cache. This is the only new durable object.
- Regenerating from `(seed, tables_revision, rules_mode)` must reproduce the stored bytes when the model step was not used; a diagnostic compares them but the stored bytes are authoritative (tables may later change).
- Replay needs no generation: it reads the stored record. Telemetry: one `gameplay.adventure.generated` event through the shared OTEL path with the seed, tables revision, rules mode, pack and tuning hashes and the roll count (no prose).

## 6. Prose pattern grammar

`prose-patterns.json` holds lists of variants. A variant is plain text with `{path}` and `{path|cap}` placeholders; `|cap` upper-cases the first character and is the only filter. Paths read the roll context: `loc.*`, `patron.*`, `contact.*`, `ant.*`, `stake.*`, `dest.*`, `secret.*`, `reward.line`, plus derived slots `entrance`, `foreshadow`, `arrival`, `route`, `story`. NPC records carry `first`, `full`, `role`, `desc` (with article), `sub obj pos self`, `mannerism` (its `{pos}` resolves to that NPC). Resolution is two-pass: NPC and entity fields first, then entry templates (hook, secret, complication, antagonist), then patterns. An unknown path is a generation error, caught by the table lint, never an empty string.

Variants are written to be read aloud: no markup, no stage directions in speech, no unresolved braces. Verb forms are present tense in narration, past tense in `complication` recaps ("Afterward, the truth comes out"), and agree with he/she/it because only those pronoun sets exist.

Length limits (checked on the rendered result; the table lint also checks every pattern against the longest value of each slot it uses): scene title <= 32, generated narration <= 220, spoken line <= 160, offer label <= 40, speaker <= 28, private clue <= 280. The loader's own bounds (1024 / 96 / 512) are looser.

## 7. Optional language-model expression

Purpose: re-express the rolled prose in a fresher voice. It never decides anything. The language-model step runs once, at generation time, before the pack bytes are fixed; if it is off, slow or its output fails any check, the pattern text is used and `expression` records `patterns`.

Fact sheet sent per beat (JSON): `beat`, `slot` (title, narration, line, label, clue), `speaker` and its `voice`/`delivery`, `facts` (the rolled names, role, location, stake short and full, destination, contact, antagonist name and group, complication, secret, kind), `must_mention` (names and the nouns the sentence needs), `forbidden`, `bounds` (section 6), `baseline` (the pattern text, as a style anchor) and `tone` ("heroic fantasy for a living-room table"). The mechanical sentences are never sent.

Validators (all must pass, else fall back for that slot):
1. Length within the slot's bound; one paragraph; no markup or stage directions.
2. Every `must_mention` string appears (case-insensitive).
3. No digits, no dice or rules words (HP, hit points, AC, damage, roll, DC, attack, save, level, XP), and no outcome verbs (kills, dies, slain, wins, loses, succeeds, fails).
4. No gore terms and no real-world group, brand or place names (a deny list), consistent with nonlethal heroic tone.
5. No new proper nouns: every capitalised word that is not sentence-initial must occur in the facts or a small allow list.
6. Speaker consistency: lines for a silent creature (beasts, undead) are rejected; spoken lines contain no narration verbs about the speaker.
7. Private clue: must contain the destination name and contact name exactly once and the secret's content words; it must not appear in any display slot.
8. A repair attempt is allowed once; the second failure falls back.

## 8. Audio and voice casting

Each text slot has one reader. Narration and titles are read by the narrator voice. A scene's `dialogue` is read by the voice of its `speaker`: patron (voice tag from `patrons.voice`, for example "anxious young man", "gruff older woman"), antagonist (`antagonists.voice`, speaking creatures only; others get an effect cue, not speech), contact (`contacts.voice`). `npc-traits.speech` (and `menace` for antagonists) is a delivery direction ("speaks in a hurried whisper") passed to the voice service; it is never shown. The private clue is read to the asking player's own device only, in the narrator voice, whispering. Mechanical sentences are read by the narrator at the same level as the rest of the narration. The server, not the client, decides voice and text; clients only play what is sent (AGENTS.md).

## 9. Antagonists: the SRD creature table

Twelve creatures, each copied from the stat block cited (SRD 5.2.1, pages are PDF = printed pages; "A-Z" is Monsters A-Z, pp. 258-343; "Animals" is pp. 344-364). Engine model: one melee attack, 1 die of 4/6/8/10/12 sides plus a flat modifier, AC and HP as written. `exact` means every number and every feature that can matter in this encounter is as written; `simplified` means a feature was dropped or folded, so the loader must report a departure (custom ruleset, disclosed).

| Creature | Page | CR | HP / AC | Attack | Fidelity |
|---|---|---|---|---|---|
| Bandit | A-Z 261 | 1/8 | 11 / 12 | Scimitar +3, 1d6+1 | exact (the shipped baseline) |
| Cultist | A-Z 278 | 1/8 | 9 / 12 | Ritual Sickle +3, 1d4+1 plus 1 necrotic | simplified: the necrotic point is folded into the modifier (+2) |
| Guard | A-Z 296 | 1/8 | 11 / 16 | Spear +3, 1d6+1 (melee) | exact |
| Goblin Warrior | A-Z 290 | 1/4 | 10 / 15 | Scimitar +4, 1d6+2 | exact (Nimble Escape and the Advantage rider cannot occur in this encounter) |
| Kobold Warrior | A-Z 302 | 1/8 | 7 / 14 | Dagger +4, 1d4+2 | exact (Pack Tactics needs an ally; Sunlight Sensitivity cannot apply because every generated scene is night or interior) |
| Skeleton | A-Z 325-326 | 1/4 | 13 / 14 | Shortsword +5, 1d6+3 | exact (the Greatsword is slashing, so the bludgeoning vulnerability is not exercised) |
| Zombie | A-Z 343-344 | 1/4 | 15 / 8 | Slam +3, 1d8+1 | simplified: Undead Fortitude is not modelled (Knocking Out, SRD p.17, arguably never triggers it; owner may reclassify) |
| Wolf | Animals 364 | 1/4 | 11 / 12 | Bite +4, 1d6+2 | simplified: Prone rider dropped |
| Giant Rat | Animals 353 | 1/8 | 7 / 13 | Bite +5, 1d4+3 | exact (Pack Tactics needs an ally) |
| Giant Weasel | Animals 355 | 1/8 | 9 / 13 | Bite +5, 1d4+3 | exact |
| Mastiff | Animals 357 | 1/8 | 5 / 12 | Bite +3, 1d6+1 | simplified: Prone rider dropped |
| Scout | A-Z 322 | 1/2 | 16 / 13 | Shortsword +4, 1d6+2 | simplified: Multiattack (two attacks) dropped, making it weaker than written |

Full ability scores are in `antagonists.json`. Seven are exact, so standard-mode adventures (the default) draw from 7 creatures; `custom_allowed` draws from all 12 and makes the run a disclosed custom ruleset. Jackal (damage `1d4 - 1`) is excluded because the damage modifier cannot be negative.

Balance check (reference Monte Carlo, 20,000 fights per creature, two heroes of AC 17, 13 HP, +5 Greatsword 2d6+3 with Savage Attacker, enemy picking a random hero, random initiative simplification): the heroes win at least 99.9% of fights against every creature, and fights last 1.1 to 1.7 rounds. This is the honest consequence of pairing two fighters with CR 1/8 to 1/2 creatures. It meets "win or survive nonlethally"; it does not make the fight tense. Making fights harder needs more enemies or higher-CR creatures (section 11), not different numbers. This check is design evidence, not verification of the running game.

## 10. Loader and engine changes required (minimal)

Line numbers: commit `c44c818`.
1. Antagonist baseline (`pack.rs:1527-1626` `departures`; `df-rules/src/local_journey.rs:230-310` `SRD_521_BASELINE.bandit`). Today every bandit field that differs from the Bandit is reported as a departure, so every other creature would flag the run as custom. Add a compiled `SRD_521_CREATURES` table in `df-rules` (the twelve stat blocks, each marked exact or simplified) and a pack field `antagonist.srd_id`; the loader compares the numbers to the compiled row, reports no departure for an exact match, and a departure `antagonist.simplified:<id>` for a simplified one. JSON never declares itself standard. A parity test asserts `antagonists.json` equals the compiled table.
2. Weapon item allowlist (`pack.rs:62-69` `COMPILED_WEAPONS`, `pack.rs:1061-1066` `validate_weapon`). The antagonist weapon is a display label, not an inventory item. Replace `bandit.weapon.item` with `weapon.label` (1-32 characters of text); keep the internal identifiers `bandit`/`scimitar` (`journey.rs:76,88`, `enemy_tactics.rs:126,148`, `encounter_admission.rs:149`, `combat_transition.rs:132`) as the opaque "antagonist slot" and "melee attack" identities so no digest-bound content id changes.
3. Fixed antagonist name (`journey.rs:252` "Dockside Bandit"; `pack.rs:523` `BanditPack`, `1113` bound 1-48). Read the name from the pack; allow up to 28 generated characters.
4. Hard-coded antagonist numbers (`journey.rs:175` `resources()` HP 11 and AC 12, `journey.rs:1966` `start_combat`). Already scheduled by gameplay-tuning section 5 ("bandit stats"); generation depends on it. Bound HP at 16 and AC at 8..16 for generated runs.
5. Scene asset allowlist (`pack.rs:71-76` `SERVED_SCENE_ASSETS`; `public_assets.rs:20-23`, `extra_router`). Replace the four fixed paths with the paths named by the run's art pool (16 scene images, loaded and hashed at startup like `ExtraAsset`). Missing art for a rolled key must fall back to the location's arrival image, never fail the run.
6. Client art and portraits (`gameplay_browser/live_screens.rs:111-121` picks art by phase and destination; `125-130` and `313-318` map the speaker NAME "Courier"/"Bandit"/"Vell" to a compiled portrait). They must use `scene_asset` from the server and a new `GameplayScene.speaker_portrait` string (proto field, assets from the portrait pool); until then scenes render without portraits.
7. Destination label and enum (`wire.rs:514-515` label "Harbor Inn", `wire.rs:128-131` enum `HarborDestination`). Keep the single enum value as "the destination", take the label from the pack.
8. Scene text still compiled (`wire.rs:593-662`) and the private clue (`courier_ai.rs:23,108,292` `RESPONSE`, digest-bound prepared recording). Both are being moved to the pack by the in-flight work; the clue recording must be created per run from `courier_clue`.
9. Pack bounds. `courier_clue` is checked in bytes (`pack.rs:1117`, <= 512) while the generator bounds characters (<= 280); fine. Add `antagonist.*` keys to `deny_unknown_fields` structs. No other loader change; creation groups, equipment and arrays stay fixed.
10. New: a pure generator module (no I/O) and a CLI to run it (section 12).

## 11. Out of scope for v1, and the v2 list

Out of scope: more than two approaches; more than one enemy; ranged and spell attacks; skill or ability checks; loot, gold and XP; other classes or levels; encounter terrain or movement; a second chapter in the same run (the campaign layer, section 13+, needs run chaining).

v2, in priority order: (1) data-driven beat script instead of fixed phases (see section 17); (2) a skill-check action with a DC; (3) two or three enemies with the same melee model; (4) loot and a reward ledger; (5) a ranged attack option; (6) a third approach (sneak or bargain); (7) more levels and classes.

## 12. Implementation plan (small ordered steps)

1. `df-rules`: add `SRD_521_CREATURES` and its parity test against `content/adventure-tables/antagonists.json` (new `crates/df-rules/src/srd_creatures.rs`).
2. `df-tools/gameplay/pack.rs`: loader changes 1-3 and 5, tests in `pack_tests.rs` (exact creature has no departure, a simplified one does, a tampered stat does).
3. New `crates/df-tools/src/gameplay/adventure/` (pure): `rng.rs` (SplitMix64 and `roll`), `tables.rs` (load, hash and lint the table files with serde, `deny_unknown_fields`), `generate.rs` (roll order, record), `render.rs` (grammar), `emit.rs` (pack and tuning bytes). Golden test: the three appendix seeds reproduce the appendix texts byte for byte; a property test over 100,000 seeds asserts the section 6 limits and no unresolved braces.
4. CLI `--generate-adventure <seed|random> <out-dir> [--custom-allowed]` in `df-tools`, writing `pack.json`, `tuning.json`, `adventure.json`; `development/run-gameplay.sh --new` calls it before `serve`.
5. Journey literals (change 4) and client art/portrait mapping (changes 6-7), coordinated with the in-flight text/number work.
6. Art: curate the 44-image pool from `art-manifest.json`; add the files to the served allowlist.
7. Persistence row in PostgreSQL (section 5) and the telemetry event.
8. Optional: the model expression step behind a flag (section 7).

Evaluation (ADR 0005): run three generated seeds end to end in a real browser, play both approaches and both outcomes, check the TV and phone text for each beat, the private clue visibility, and a restart mid-adventure.

## Appendix A. Three worked adventures (standard mode)

The texts below are produced by the reference model with the tables as committed; an implementation must reproduce them. Titles, narration, spoken lines, labels, the private clue and the antagonist numbers are complete. Entry ids in the roll tables are table ids.

#### Sample seed `0x3c6ef378dd81de97`

**Facts:** the Lamplit Market / hook `missed-signal` (warn) / patron `bell-ringer` (Ilsa Greaves, voice: calm middle-aged woman; delivery: speaks in a hurried whisper) / stake `debt-ledger` / antagonist `goblin-warrior` (Snag the Goblin) / complication `afraid` / secret `my-fault` / destination `lantern-makers-shop` / contact `watch-sergeant` (Lorcan Nettleby, voice: brisk, no-nonsense man; delivery: formal and precise) / reward line 11.

**Antagonist numbers** (Goblin Warrior, SRD 5.2.1 p.290, exact): HP 10, AC 15, abilities STR 8 DEX 15 CON 10 INT 10 WIS 8 CHA 8, attack +4, scimitar 1d6+2.

**Offer labels:** `begin-story` = "Enter the Lamplit Market"; `ask-courier` = "Ask Ilsa what is wrong"; `escort-courier` = "Vow to see Ilsa through"; `defend-courier` = "Face down the goblin"; `choose-harbor-scene` = "Press on to the shop"; destination option "the Lantern-Maker's Shop".

**Private clue** (asking player only): Ilsa draws you close. The warning is for Lorcan Nettleby at the Lantern-Maker's Shop. This is partly my fault. I left something unguarded, and I am ashamed. Keep it between us.

- **room** | *Arrive at the Lamplit Market* | art `loc-lamplit-market-arrival`
  - Narration: Puddles carry the red and gold of a hundred lamps. Paper lanterns bob above crowded awnings. Share the room with your companions, then bring your heroes into the story.
- **opening** | *The Missed Signal* | art `loc-lamplit-market-arrival`
  - Narration: As you cross the Lamplit Market, a calm, tired bell ringer waves to catch your attention, anxious and apologetic. Out in the dark, a second set of footsteps stops.
  - Ilsa: "A signal meant for Lorcan never arrived. Someone cut it off, and I need to put it right tonight."
- **dialogue-seal** | *Whispers in the Dark* | art `loc-lamplit-market-arrival`
  - Narration: Ilsa lowers her voice and leans close, standing very straight. Snag the Goblin scrambles into view, small and quick, a rusty scimitar waving over his head.
  - Ilsa: "I don't know who, but I know it was deliberate. And the ledger is the only proof I have."
- **dialogue-escort** | *Walk Together* | art `loc-lamplit-market-arrival`
  - Narration: Ilsa presses close behind you, trusting your lead. Snag the Goblin scrambles into view, small and quick, a rusty scimitar waving over his head.
  - Ilsa: "Then we go to the Lantern-Maker's Shop. Lorcan will know how to read the ledger, if we arrive in time."
- **combat** | *Trouble at the market* | art `loc-lamplit-market-arrival`
  - Narration: Snag the Goblin raises his weapon in answer. Both heroes are within 5 feet of the goblin, and everyone fights in the open: no surprise, cover or movement. This encounter uses nonlethal melee attacks.
  - Snag the Goblin: "The ledger is mine now! Mine, mine, mine!"
- **inn** | *Lantern-Maker's Shop* | art `loc-lamplit-market-interior`
  - Narration: In the lantern-maker's shop, a hundred unlit lamps hang like sleeping fruit. Lorcan, a brisk watch sergeant, looks up at once, tugging at his sleeve.
  - Lorcan: "Do not blame Ilsa. What matters is that you came. Now, then. You have friends here now, whether you wanted them or not."
- **victory** | *Breathing Room* | art `loc-lamplit-market-arrival`
  - Narration: Snag the Goblin falls unconscious with 1 HP. As the dust settles, the truth comes out. Snag the Goblin was more frightened than angry, sure that the ledger would bring ruin on everyone. His Short Rest has begun; campaign time has not completed it.
  - Ilsa: "Let's go before anything else finds us. The Lantern-Maker's Shop is close."

- **defeat** | *Out of the Fight* | Narration: Both heroes are unconscious with 1 HP after nonlethal attacks. The warning goes undelivered, and Lorcan will hear the news too late. Their Short Rests have begun; campaign time has not completed them.

**DM's rolls** (55 total, 5 rejected as incompatible). Story rolls:

| # | Table | Die | Natural | Entry | Note |
|---|---|---|---|---|---|
| 1 | location | d8 | 4 | `lamplit-market` |  |
| 2 | hook | d12 | 9 | `missed-signal` |  |
| 3 | patron | d10 | 10 | `bell-ringer` |  |
| 4 | stake | d20 | 17 | `debt-ledger` |  |
| 5 | antagonist | d12 | 4 | `goblin-warrior` |  |
| 6 | complication | d20 | 11 | `guarding-nest` | rejected: incompatible, rerolled |
| 7 | complication | d20 | 10 | `driven-from-den` | rejected: incompatible, rerolled |
| 8 | complication | d20 | 12 | `hard-winter` | rejected: incompatible, rerolled |
| 9 | complication | d20 | 19 | `woken-on-purpose` | rejected: incompatible, rerolled |
| 10 | complication | d20 | 8 | `afraid` |  |
| 11 | secret | d12 | 8 | `my-fault` |  |
| 12 | destination | d4 | 4 | `lantern-makers-shop` |  |
| 13 | contact | d6 | 3 | `watch-sergeant` |  |
| 14 | reward | d12 | 11 | `variant 11` |  |

Remaining rolls (names, traits, atmosphere, prose variants) in order: name.patron.given=d20:9, name.patron.surname=d20:6, name.contact.given=d20:11, name.contact.surname=d20:12, name.antagonist=d12:2, trait.patron.mannerism=d12:4, trait.patron.speech=d12:1, trait.contact.mannerism=d12:3, trait.contact.speech=d12:6, trait.antagonist.menace=d8:3, atmosphere.a=d6:5, atmosphere.b=d6:1, atmosphere.c=d6:5, atmosphere.c=d6:2, prose.begin_label=d4:1, prose.room_title=d4:3, prose.room_narration=d4:1, prose.opening_narration=d4:3, prose.foreshadow_speaker=d4:2, prose.ask_label=d4:1, prose.help_label=d4:3, prose.stand_label=d4:2, prose.continue_label=d4:3, prose.ask_title=d4:3, prose.ask_narration=d4:2, antagonist.arrival=d4:1, prose.help_title=d4:3, prose.help_narration=d4:4, prose.route_warn=d4:2, prose.clue=d4:4, prose.combat_title=d4:2, prose.combat_open_speaker=d4:4, antagonist.warning=d4:2, prose.victory_title=d4:3, prose.victory_lead=d4:2, prose.victory_dialogue=d4:4, prose.defeat_title=d4:3, prose.defeat_warn=d4:2, prose.inn_title=d4:1, prose.inn_intro=d4:3, prose.inn_dialogue=d4:2.


#### Sample seed `0xdaa66d325ccc5aac`

**Facts:** the Pinewood Trail / hook `dying-promise` (deliver) / patron `acolyte` (Brisa Ashdown, voice: earnest young woman; delivery: voice cracks when excited) / stake `cradle-blanket` / antagonist `skeleton` (Ash-Pale Sentinel) / complication `careless-hand` / secret `passphrase` / destination `woodcutters-lodge` / contact `river-healer` (Odalys Ravensby, voice: gentle middle-aged woman; delivery: voice cracks when excited) / reward line 5.

**Antagonist numbers** (Skeleton, SRD 5.2.1 p.325, exact): HP 13, AC 14, abilities STR 10 DEX 16 CON 15 INT 6 WIS 8 CHA 5, attack +5, shortsword 1d6+3.

**Offer labels:** `begin-story` = "Step into the Pinewood Trail"; `ask-courier` = "Draw Brisa aside to talk"; `escort-courier` = "Vow to see Brisa through"; `defend-courier` = "Face down the skeleton"; `choose-harbor-scene` = "Continue to the lodge"; destination option "the Woodcutters' Lodge".

**Private clue** (asking player only): Brisa lowers her voice so only you can hear. Odalys Ravensby waits at the Woodcutters' Lodge for the blanket. At the door, say that the lamps are low. Odalys will know you are friends. Tell no one.

- **room** | *Gather at the Pinewood Trail* | art `loc-pinewood-trail-arrival`
  - Narration: Night has settled over the Pinewood Trail. An owl asks its question from somewhere above. Share the room with your companions, then bring your heroes into the story.
- **opening** | *A Promise to Keep* | art `loc-pinewood-trail-arrival`
  - Narration: You reach the Pinewood Trail as an earnest young temple acolyte stumbles toward you, cradling a crest-stitched cradle-blanket. The lanterns gutter, as though the dark were listening.
  - Brisa: "I swore to an old friend that the blanket would reach Odalys. Tonight I may not keep my word."
- **dialogue-seal** | *Hard Questions* | art `loc-pinewood-trail-arrival`
  - Narration: Before you can press further, Brisa stiffens, holding a lantern high. With a slow clatter, the Ash-Pale Sentinel steps into the light and lifts its shortsword.
  - Brisa: "My friend is gone now. This is all I have left of the promise, and I won't see it lost."
- **dialogue-escort** | *Walk Together* | art `loc-pinewood-trail-arrival`
  - Narration: Brisa nods, holding a lantern high, and stays close at your shoulder. With a slow clatter, the Ash-Pale Sentinel steps into the light and lifts its shortsword.
  - Brisa: "Thank you. Odalys will be at the Woodcutters' Lodge. Get me there, and my promise is kept."
- **combat** | *The Way Is Barred* | art `loc-pinewood-trail-arrival`
  - Narration: The Ash-Pale Sentinel advances without a sound. Both heroes are within 5 feet of the skeleton, and everyone fights in the open: no surprise, cover or movement. This encounter uses nonlethal melee attacks.
- **inn** | *End of the Road* | art `loc-pinewood-trail-interior`
  - Narration: The lodge is all timber beams and firelight, with antlers over a wide stone hearth. There, frowning at the ground, stands Odalys, a gentle river healer.
  - Odalys: "Well done. You knew the words about the lamps. Then Brisa trusts you. I know a sealed cellar nobody has opened in years. Ask me when you are rested."
- **victory** | *Breathing Room* | art `loc-pinewood-trail-arrival`
  - Narration: The Ash-Pale Sentinel falls unconscious with 1 HP. In the quiet that follows, you piece it together. A careless hand had disturbed its long rest, and it did not understand why. Its Short Rest has begun; campaign time has not completed it.
  - Brisa: "I owe you more than I can say. The Woodcutters' Lodge is just ahead."

- **defeat** | *Out of the Fight* | Narration: Both heroes are unconscious with 1 HP after nonlethal attacks. The blanket is lost in the confusion, and Brisa's errand fails tonight. Their Short Rests have begun; campaign time has not completed them.

**DM's rolls** (51 total, 3 rejected as incompatible). Story rolls:

| # | Table | Die | Natural | Entry | Note |
|---|---|---|---|---|---|
| 1 | location | d8 | 2 | `pinewood-trail` |  |
| 2 | hook | d12 | 2 | `dying-promise` |  |
| 3 | patron | d10 | 7 | `acolyte` |  |
| 4 | stake | d20 | 18 | `cradle-blanket` |  |
| 5 | antagonist | d12 | 6 | `skeleton` |  |
| 6 | complication | d20 | 9 | `hunger-and-light` | rejected: incompatible, rerolled |
| 7 | complication | d20 | 14 | `followed-trail` | rejected: incompatible, rerolled |
| 8 | complication | d20 | 18 | `careless-hand` |  |
| 9 | secret | d12 | 4 | `passphrase` |  |
| 10 | destination | d4 | 2 | `woodcutters-lodge` |  |
| 11 | contact | d6 | 1 | `old-mapmaker` | rejected: incompatible, rerolled |
| 12 | contact | d6 | 2 | `river-healer` |  |
| 13 | reward | d12 | 5 | `variant 5` |  |

Remaining rolls (names, traits, atmosphere, prose variants) in order: name.patron.given=d20:2, name.patron.surname=d20:1, name.contact.given=d20:15, name.contact.surname=d20:16, name.antagonist.adjective=d8:8, trait.patron.mannerism=d12:7, trait.patron.speech=d12:3, trait.contact.mannerism=d12:10, trait.contact.speech=d12:3, atmosphere.a=d6:3, atmosphere.b=d6:5, atmosphere.c=d6:6, prose.begin_label=d4:2, prose.room_title=d4:1, prose.room_narration=d4:3, prose.opening_narration=d4:1, prose.foreshadow_undead=d4:3, prose.ask_label=d4:3, prose.help_label=d4:3, prose.stand_label=d4:2, prose.continue_label=d4:1, prose.ask_title=d4:2, prose.ask_narration=d4:3, antagonist.arrival=d4:2, prose.help_title=d4:3, prose.help_narration=d4:3, prose.route_deliver=d4:4, prose.clue=d4:2, prose.combat_title=d4:3, prose.combat_open_undead=d4:1, prose.victory_title=d4:3, prose.victory_lead=d4:1, prose.victory_dialogue=d4:2, prose.defeat_title=d4:3, prose.defeat_deliver=d4:2, prose.inn_title=d4:4, prose.inn_intro=d4:2, prose.inn_dialogue=d4:3.


#### Sample seed `0x11a25cdccc7d808f`

**Facts:** Thistledown Farm / hook `promised-remedy` (retrieve) / patron `innkeeper` (Kestrel Marlow, voice: warm older woman; delivery: warm, rolling cadence) / stake `fever-herb` / antagonist `giant-rat` (Scarred Giant Rat) / complication `hunger-and-light` / secret `leak` / destination `farmhouse-kitchen` / contact `miller` (Odalys Nettleby, voice: hearty woman of middle years; delivery: warm, rolling cadence) / reward line 7.

**Antagonist numbers** (Giant Rat, SRD 5.2.1 p.353, exact): HP 7, AC 13, abilities STR 7 DEX 16 CON 11 INT 2 WIS 10 CHA 4, attack +5, bite 1d4+3.

**Offer labels:** `begin-story` = "Begin at Thistledown Farm"; `ask-courier` = "Press Kestrel for the truth"; `escort-courier` = "Vow to see Kestrel through"; `defend-courier` = "Block the path of the giant rat"; `choose-harbor-scene` = "Continue to the kitchen"; destination option "the Farmhouse Kitchen".

**Private clue** (asking player only): Kestrel whispers to you alone: Odalys Nettleby is keeping the fever-herb for me at the Farmhouse Kitchen. Someone I trust sold word of this errand. I do not know who. Keep it between us.

- **room** | *Arrive at Thistledown Farm* | art `loc-thistledown-farm-arrival`
  - Narration: Sheep bells clink somewhere on the dark hill. Cart tracks run silver with moonlit puddles. Share the room with your companions, then bring your heroes into the story.
- **opening** | *A Remedy Promised* | art `loc-thistledown-farm-arrival`
  - Narration: You reach Thistledown Farm as a warm, worried innkeeper hurries toward you, twisting a worn cloth in her hands. Something unseen moves along behind her.
  - Kestrel: "A child I know is ill, and only a rare white fever-herb will help. Odalys holds it, but I daren't go alone."
- **dialogue-seal** | *A Quiet Word* | art `loc-thistledown-farm-arrival`
  - Narration: Before you can press further, Kestrel stiffens, tapping a nervous rhythm on her belt. With a shrill squeal, the Scarred Giant Rat darts across your path and turns to face you.
  - Kestrel: "I was told Odalys would give it freely, if I can only reach her in one piece."
- **dialogue-escort** | *Walk Together* | art `loc-thistledown-farm-arrival`
  - Narration: Kestrel lets out a shaky breath and steadies herself behind you. With a shrill squeal, the Scarred Giant Rat darts across your path and turns to face you.
  - Kestrel: "The Farmhouse Kitchen. Hurry, if you can. Every hour counts, and I will not forget your kindness."
- **combat** | *Trouble at the farm* | art `loc-thistledown-farm-arrival`
  - Narration: There is no reasoning with it. Both heroes are within 5 feet of the giant rat, and everyone fights in the open: no surprise, cover or movement. This encounter uses nonlethal melee attacks.
- **inn** | *A Welcome Waiting* | art `loc-thistledown-farm-interior`
  - Narration: The farmhouse kitchen is bright with copper pans, a crackling fire and fresh-baked bread. A hearty, flour-dusted miller, Odalys, rises to greet you, standing very straight.
  - Odalys: "So there was a leak. I will find out who talks. Rest easy. I will vouch for you to anyone who matters in this region."
- **victory** | *The Way Ahead* | art `loc-thistledown-farm-arrival`
  - Narration: The Scarred Giant Rat falls unconscious with 1 HP. As the dust settles, the truth comes out. Hunger and lantern light had drawn it down to the path, and it would not be turned aside. Its Short Rest has begun; campaign time has not completed it.
  - Kestrel: "You stood your ground. Come on; the kitchen isn't far now."

- **defeat** | *The Night Turns Against You* | Narration: Both heroes are unconscious with 1 HP after nonlethal attacks. The fever-herb stays out of reach, and Kestrel leaves empty-handed. Their Short Rests have begun; campaign time has not completed them.

**DM's rolls** (63 total, 15 rejected as incompatible). Story rolls:

| # | Table | Die | Natural | Entry | Note |
|---|---|---|---|---|---|
| 1 | location | d8 | 7 | `thistledown-farm` |  |
| 2 | hook | d12 | 10 | `promised-remedy` |  |
| 3 | patron | d10 | 8 | `innkeeper` |  |
| 4 | stake | d20 | 11 | `signet-ring` | rejected: incompatible, rerolled |
| 5 | stake | d20 | 15 | `silent-clock` | rejected: incompatible, rerolled |
| 6 | stake | d20 | 18 | `cradle-blanket` | rejected: incompatible, rerolled |
| 7 | stake | d20 | 5 | `lullaby-book` | rejected: incompatible, rerolled |
| 8 | stake | d20 | 13 | `fever-herb` |  |
| 9 | antagonist | d12 | 9 | `giant-rat` |  |
| 10 | complication | d20 | 1 | `hired-rival` | rejected: incompatible, rerolled |
| 11 | complication | d20 | 19 | `woken-on-purpose` | rejected: incompatible, rerolled |
| 12 | complication | d20 | 1 | `hired-rival` | rejected: incompatible, rerolled |
| 13 | complication | d20 | 3 | `old-debt` | rejected: incompatible, rerolled |
| 14 | complication | d20 | 17 | `dim-duty` | rejected: incompatible, rerolled |
| 15 | complication | d20 | 7 | `old-oath` | rejected: incompatible, rerolled |
| 16 | complication | d20 | 16 | `broken-ward` | rejected: incompatible, rerolled |
| 17 | complication | d20 | 15 | `old-binding` | rejected: incompatible, rerolled |
| 18 | complication | d20 | 20 | `drawn-to-flame` | rejected: incompatible, rerolled |
| 19 | complication | d20 | 17 | `dim-duty` | rejected: incompatible, rerolled |
| 20 | complication | d20 | 9 | `hunger-and-light` |  |
| 21 | secret | d12 | 1 | `leak` |  |
| 22 | destination | d4 | 1 | `farmhouse-kitchen` |  |
| 23 | contact | d6 | 6 | `miller` |  |
| 24 | reward | d12 | 7 | `variant 7` |  |

Remaining rolls (names, traits, atmosphere, prose variants) in order: name.patron.given=d20:11, name.patron.surname=d20:11, name.contact.given=d20:15, name.contact.surname=d20:12, name.antagonist.adjective=d12:2, trait.patron.mannerism=d12:9, trait.patron.speech=d12:12, trait.contact.mannerism=d12:4, trait.contact.speech=d12:12, atmosphere.a=d6:6, atmosphere.b=d6:4, atmosphere.c=d6:4, atmosphere.c=d6:3, prose.begin_label=d4:3, prose.room_title=d4:3, prose.room_narration=d4:1, prose.opening_narration=d4:1, prose.foreshadow_beast=d4:4, prose.ask_label=d4:4, prose.help_label=d4:3, prose.stand_label=d4:3, prose.continue_label=d4:1, prose.ask_title=d4:1, prose.ask_narration=d4:3, antagonist.arrival=d4:4, prose.help_title=d4:3, prose.help_narration=d4:1, prose.route_retrieve=d4:2, prose.clue=d4:1, prose.combat_title=d4:2, prose.combat_open_beast=d4:3, prose.victory_title=d4:1, prose.victory_lead=d4:2, prose.victory_dialogue=d4:1, prose.defeat_title=d4:4, prose.defeat_retrieve=d4:4, prose.inn_title=d4:2, prose.inn_intro=d4:4, prose.inn_dialogue=d4:4.


