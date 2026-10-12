# Random adventure generator (PLAY-G0)

Status: design plus authored roll tables; no code. Grounded in commit `c44c818` (line numbers below refer to it). Paths are relative to the repository root; `df-tools/gameplay/` means `crates/df-tools/src/gameplay/`.

Governing documents: AGENTS.md; [Gameplay tuning surface](gameplay-tuning.md) (pack and tuning files, revision pins, departures); [Generated content](generated-content.md) (typed templates, never executable; standard mode never silently changes rules); [Narrative engine](narrative-engine.md); [Runtime directors](runtime-directors.md); [Rules support](rules-support.md). Creature numbers come from the pinned SRD 5.2.1 (`development/rules-sources/manifest.json`, PDF SHA-256 `8974902d...5e3d87`, CC-BY-4.0). All table prose is original; nothing is copied from the Dungeon Master's Guide or any other non-SRD book.

Tables live in `content/adventure-tables/` (index: `tables.json`). The art pool prompts are in `content/adventure-tables/art-manifest.json`. Sections 1-12 and Appendix A design the single adventure (the chapter generator); sections 13-18 and Appendix B add the campaign layer that chains chapters.

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
4. No real-world group, brand or place names (a deny list), and, in the `restrained` content rating only, no gore terms or strong language (section 21); a `mature` rating allows both.
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

## 13. The campaign layer: frame

Everything above generates one chapter. The campaign layer rolls a frame once per new campaign, then asks the chapter generator for one chapter at a time. Same principles: named dice, recorded rolls, original tables, tags for coherence, no model needed for structure.

Frame stream: `Rng(seed XOR 0xCA3DA16B5EED)`, recorded in the same "DM's rolls" format. Order and dice are in `tables.json` under `campaign.frame_roll_order`.

| Frame element | Source table (file) | Roll |
|---|---|---|
| Premise / storyline (16 classics: ancient evil, cult countdown, relic race, usurper and missing heir, blight, waking dungeon, monster-hunt conspiracy, border war and traitor, planar breach, thieves' war, dragon's tribute, drowned kingdom, false prophet, fey bargain, unfinished spell, saboteur on the trade road) | `premises` (campaign-premises.json) | d20 |
| Title | premise `titles`, filled with region and artifact | d4 |
| Villain type (12) | `villains.types`, restricted to the premise's `villain_types` | d12 |
| Villain name and title ("Aldous, the Winter Regent") | `names.villain_he/she`, `villain_adjective`, type noun | d12, d12 |
| Motive, scheme, method, resources, weakness, signature | `villains.motives/schemes/methods/resources/weaknesses/signatures` (scheme filtered by premise family) | d12, d12, d10, d8, d12, d12 |
| Lieutenants (1, or 2 for 9-10 chapters) | `villains.lieutenants` (each is an SRD creature in `antagonists.json`; standard mode excludes simplified ones) | d8 |
| Factions (2-4) with attitude toward the party and toward the villain | `factions` | d6 count, d12, d6, d6 |
| Region name | `premises.region_names` | d12 |
| Region: home base plus 5-8 sites from the 8 locations, each with a role (home base, first clue, lieutenant's lair, stronghold, plus rumor source, ally haven, old secret, neutral ground) | `locations`, `premises.site_roles` | d4 size, d8 per role (soft role preference, no repeats) |
| Recurring allies: one patron archetype and one contact archetype with fixed names | `patrons`, `contacts`, `names` | d10, d6, d20 x2 each |
| MacGuffin of the campaign | `premises.artifacts` (12 relics) | d12 |
| Themes (2) and tone | `premises.themes`, `tones` | d12 x2, d6 |
| Villain's clock (fronts style) | `clocks` selected by the premise | none |
| Chapter count (6-10), act split and provisional chapter types | `act-structure.chapter_count`, `slots`, `chapter-types` | d6, then one roll per slot |
| Act-break twists (2) | `twists`, restricted to the premise family | d12 x2 |

The villain is built the way a DM builds one rather than rolled as a block: a type (what it is), a motive (why), a scheme (what it does, with the premise's family so a ritual premise never rolls "provoke a war"), a method (how it works on people), resources (what it can spend), a weakness the party can discover through two named clue lines, and a signature (the calling card that foreshadows it from chapter 1). Lieutenants are SRD creatures with a persistent name and epithet.

Fronts and clocks. The clock is a 7-step timeline of what happens if nobody intervenes; step 1 is already true when play begins and step 7 is the doom (`clocks.json`). Position advances +1 at each act break, +1 when a chapter ends in defeat, and +1 when a lieutenant escapes. If the position reaches 6 before the finale, the finale uses the desperate framing (the finale may be replayed once after a defeat with the clock +1). Players never see the number; they see its signs (the `text` of the current step, rendered as an omen).

Distinct frames. Counting independent choices (premise 16 x villain types 4 x schemes by family x name and title 144 x motive 12 x method 10 x resources 8 x weakness 12 x signature 12 x artifact 12 x region name 12 x site assignment x factions and attitudes x allies and names x lieutenants x themes x tone x chapter count and slot types x twists), the tables give on the order of 10^25 structural frames, more than 10^20 even with the premise fixed (an estimated product, not a proof: some fields correlate). In practice any two campaigns differ in storyline, villain, region, cast and clock.

## 14. Campaign structure: named DM craft encoded as rules

| Technique | Where it lives |
|---|---|
| Three-act structure with rising stakes (local, regional, existential) | `act-structure.acts`, `slots` per chapter count: 6 -> 2/2/2, 7 -> 2/3/2, 8 -> 3/3/2, 9 -> 3/3/3, 10 -> 3/4/3 |
| Fronts and clocks | `clocks.json`, section 13 |
| Node-based design: the party can never get stuck | `act-structure.nodes` (omens, villain, scheme, stronghold, weakness), `clues.json` |
| Three-clue rule: every conclusion has at least three clues, by at least two vehicles, before the chapter that needs it | rules in `act-structure.rules`; vehicles are the private clue, the victory recap and the contact's line; rescue rule uses the public "Previously" line |
| Five-room site | the `delve` chapter type (entrance, puzzle, setback, guardian, reward) played as the guardian room (section 17) |
| Pillars of play (combat, social, exploration) | each chapter type has an intended mix; the generator repairs the outline until each pillar's mean is at least 25 percent and no two adjacent chapters share a type |
| Twist or reversal at the act breaks, foreshadowed twice | `twists.json`: two hint lines must be delivered in earlier chapters before the reveal |
| Villain foreshadowed from chapter 1 | the signature appears in chapter 1's recap clue; omens in the clock steps |
| Lieutenant confrontation before the finale | slot template always contains `lieutenant` before `finale`; 9-10 chapters have two |
| Rest and downtime beats | the Short Rest the engine already offers after each win; the `rest-interlude` slot is a recap screen between chapters |
| Milestone rewards | `rewards.milestones` after every act; levels are v2 |
| Callbacks | the campaign record keeps what each hero carried (for example chapter 1's stake); the twist `price-of-cure` reuses it |

Chapter types (16, `chapter-types.json`): courier, escort, warning and retrieval (the four native shapes), mystery (three-clue), delve (five-room), defence/siege, heist/infiltration, hunt, parley/intrigue, race against the clock, rescue, escape, lieutenant confrontation, finale, and the rest interlude. Each lists its intended combat/social/exploration mix, the engine capabilities it really needs, whether it is native, faked or unavailable in v1, a mapping onto the single playable shape (hook kind, stake tags, antagonist role, location role), its own ask/help/stand labels, and `on_victory` and `on_defeat` types that bias what comes next.

## 15. Chapters are generated one at a time

Only the frame and the provisional slot types exist at creation. Chapter n is generated when chapter n-1 completes, from the frame plus the recorded outcomes of earlier chapters. Chapter seed: `finalize(campaign_seed + n * 0x9E3779B97F4A7C15 + finalize(outcome_digest))`, where the outcome digest is the first 8 bytes of SHA-256 of the canonical outcome record. Two tables playing the same campaign seed therefore diverge as soon as an outcome differs, and a replay needs only the stored record.

Inputs (what truly differs between tables in v1): victory or defeat; which approach was chosen (ask or help) and by whom; whether the Short Rest completed; whether the destination was reached; each hero's private clue (who asked); the lieutenant's fate; the clock. Honest limit: mercy versus execution, the party's route and any skill result are not player choices in v1 (section 17); fate is a recorded die roll.

Procedure for chapter n:
1. Pick the type. The slot's allowed types are intersected with the previous chapter's `on_victory` or `on_defeat` list; if the intersection is empty the provisional type stands. A re-pick is a recorded roll.
2. Apply the type's `beat` mapping: hook kind and stake tags; location from the site role (the opening is always the home base); antagonist from the villain's minions (the villain type's `lieutenant_pref` plus bandit) or, for `lieutenant` and `finale`, the named lieutenant or the strongest exact creature; patron and contact from the recurring cast or a faction agent; premise `family_hooks` bias the hook.
3. Choose three leads for the two earliest unresolved nodes (private clue, recap, contact line). On a defeat the recap and contact lines never appear, so the delivery ledger marks a clue delivered only when its vehicle is shown, and the rescue rule moves a missing clue to the next chapter's public "Previously" line.
4. If the previous slot ended an act, put the twist first: its reveal opens this chapter and the act milestone is granted.
5. Run the standard chapter roll with forced entries (recorded as `forced`), then render with the standard patterns plus the campaign overlay: the room scene's title is the campaign title, its narration begins "Chapter n.", and its speaker line carries the "Previously" recap (chapter 1: the premise teaser).
6. After the chapter: extract the outcome, roll the lieutenant's fate if one fell, update the clock, ledger, facts and attitudes (section 16).

How a different earlier outcome changes the next chapter (examples in Appendix B): a defeat raises the clock, replaces the recap clue by the rescue rule and biases toward a rescue or defence chapter; choosing "help" instead of "ask" changes which hero holds the private clue and which attitude ticks up; a lieutenant who escapes returns once; an ally who was not escorted safely becomes a captive to rescue.

## 16. Continuity state and the storyteller fact sheets

Durable campaign record (`df-campaign-record/1`):
- `frame`: the whole rolled frame including names (immutable, hashed), tables revision, rules mode, generator version, the frame's DM's-rolls list.
- `clock`: position and history `[{chapter, delta, reason}]`.
- `chapters[]`: `{n, slot, type, seed, outcome_digest_in, rolls_sha256, pack_sha256, tuning_sha256, run_database, result (victory|defeat|abandoned), approach, asker (member), rested, destination_reached, lieutenant_fate, campaign_time}`.
- `facts[]`: `{id, text, audience (public | private:<hero> | dm), established_in, revealed}`; established facts include what each hero carried and every revealed twist.
- `clues[]`: `{node, vehicle, chapter, audience, delivered}`.
- `npcs[]`: `{id, name, role, attitude (allied | friendly | neutral | wary | hostile), status (active | captured | escaped | defected), last_seen}`; factions are npcs of kind faction.
- `heroes[]`: `{member, name, creation choices, secrets: [fact ids]}`.

Where it lives. Reused unchanged: the per-chapter pack and tuning pins (each chapter is a normal generated adventure with its own run database, `adventure.json` and pins; gameplay-tuning section 5). New: (1) a campaign database (`df_gameplay_campaign_<name>`) holding the immutable frame row, append-only chapter, fact, clue and attitude rows, written transactionally when a chapter ends; (2) a `campaign_id` and chapter number in each chapter's `adventure.json` and the frame hash as a suffix `:c-<16 hex>` on `BuildIdentity.configuration`, so every chapter's checkpoints bind to its campaign; (3) character carry-over: the next chapter's run starts with each member's saved creation choices pre-filled for one-tap confirmation (the engine has no persistent heroes across runs). The telemetry event `gameplay.campaign.chapter` records ids, hashes and the clock only.

Storyteller sheets (both JSON, built by the server, never by a client; audience tags decide inclusion):
- `recap_sheet` (listener-safe, for the TV "Previously..." and for models): premise teaser, region and campaign title, the last 3 chapters as `{title, result, approach in words}`, the public facts (at most 6, newest first), current open leads in plain words, the clock described by its latest sign (never the number), living NPCs with attitude words, and tone. It excludes `dm` and `private:*` facts, the villain's true name until revealed, hidden faction loyalties and unrevealed twists. A per-hero private recap for a phone may add that hero's own private facts.
- `chapter_sheet` (server and model, never displayed): the chapter's rolled facts as in section 7 plus the DM-only facts the prose must foreshadow without revealing (each flagged `hint_only` with the strings that must not appear: the villain's true name, the twist, the secret alliances).

Validators for recap text: at most 2 lines of 160 characters; must mention the last chapter's title or place and its result in plain words; no digits; no forbidden strings; no proper nouns outside the sheet; nothing from `dm` or another hero's private facts.

### Model-written slots

The dice and tables decide structure, antagonists and their numbers, who knows what, clock movement, twists and what is true. The language model (`gpt-6-luna`, already wired as a validated expression service: facts in, slot-bounded prose out, with must-mention and forbidden terms) may write only the following typed slots, each validated by the same pipeline the pattern text must pass, and each falling back to the table or pattern text on failure.

| Slot | Written by | Bounds | Validation |
|---|---|---|---|
| Scene title, narration, spoken lines, offer labels (all chapter beats) | model re-expression of the rolled pattern | section 6 limits | section 7 validators 1-8 |
| Private clue | model | <= 280 characters | validator 7 plus the node's content words; never in a display slot |
| "Previously..." recap | model, from `recap_sheet` | 2 lines x 160 | recap validators above |
| Proposed names: region, villain given name and epithet, faction names, lieutenant epithet, recurring NPC nicknames | model proposes, once, at frame creation | 2-28 characters, at most 3 words, letters, apostrophe, hyphen, space | not in the real-world deny list, not equal or within edit distance 2 of any existing name, vowel ratio 0.25-0.6, capitalised; chosen name is stored immutably in the frame, else the table name is used |
| Colour lines: place sensory line, NPC catchphrase, villain omen line | model | 100 / 60 / 100 characters | no digits, no outcome verbs, no new proper nouns, omen line must keep the signature's keywords, catchphrase must not state facts |
| Anything mechanical (numbers, creature choice, outcomes, checks, clock, who holds a secret) | never | n/a | not sent; mechanical sentences are fixed templates |

## 17. Engine reality, and the roadmap that unlocks variety

Honest summary: today the engine plays exactly one chapter shape (hook, two-approach roleplay with a private secret, one single-enemy melee fight, aftermath and rest, destination scene) with two level-1 fighters, in a run that is one database. A "campaign of linked single-shape chapters" is therefore real (new story, region, villain and clock every chapter, with continuity) but each chapter is mechanically the same fight. Status below is at commit `c44c818`.

Capabilities:

| Capability | Status | Evidence | What v1 can fake narratively | What needs engine work |
|---|---|---|---|---|
| Skill or ability checks with DCs | partial | `AbilityCheckResult` and `PlayerGameplayView.check` exist (`gameplay.proto:11,14`); nothing in `df-tools` or `df-rules/src/local_journey.rs` rolls or emits one | the "ask" approach reads as an insight check that always succeeds | a check action, DC from the beat, result in the view (roadmap 3) |
| Multiple enemies | missing | one `BANDIT` entity (`journey.rs:40`), `initiative_order` takes exactly 3 actors (`local_journey.rs:405`); `df-encounter`/`df-combat` are not wired | "the thief and her hounds" in narration only; one stat block fights | N-actor encounters (roadmap 4) |
| Ranged or spell actions | missing | only `greatsword-attack` and Second Wind (`wire.rs:68-90`) | enemies "shoot" in text | action catalog, ranges, spell handlers (roadmap 7) |
| Branching beyond two approaches | missing | fixed phases (`narrative_phase.rs`), offers `ask-courier`/`escort-courier` | per-type relabelling of two buttons | beat-script interpreter (roadmap 2) |
| Travel or exploration turns | missing | one `choose-harbor-scene` offer; `df-world` crate not wired to the journey | the destination scene and narration cover the trip | movement and route choice (roadmap 6) |
| Loot and rewards | missing | no reward in the journey; challenge consequences are planned types in `df-encounter` | story-only reward lines and milestones | reward ledger and inventory (roadmap 5) |
| Levelling | missing | level 1 only (`FIXED_GROUPS`, `pack.rs:78`) | milestone narration | level-up rules and per-level numbers (roadmap 5) |
| More hero classes | missing | `fighter-1` only | none | creation and rules per class (roadmap 7) |
| Persistent campaign state across sessions | partial | PostgreSQL checkpoints persist one run; a run is a database (gameplay-tuning section 5); no campaign object | none | campaign record and chapter chaining (roadmap 1) |
| Higher-CR creatures, boss features | missing | damage dice 1-2, HP 1-200 allowed but untested above CR 1/2 | a "champion" persona on the strongest creature | legendary-lite features, multi-attack (roadmap 8) |
| A scene shape with no fight | missing | every run goes Opening -> Dialogue -> Combat -> Complete | the rest interlude is a recap screen outside the pack | beat-script interpreter |

Chapter types:

| Type | Needs | v1 | What is faked |
|---|---|---|---|
| Courier, escort, warning, retrieval | the native shape | native | nothing |
| Mystery (three-clue) | checks, branching | faked | clue-heavy warning chapter; the three clues are the three leads |
| Delve (five-room) | multiple enemies, checks, exploration | faked | only the guardian room is played; the site is the destination scene |
| Defence/siege | multiple enemies, ranged | faked | one assailant, the rally point is the destination |
| Heist/infiltration | checks, branching | faked | "case it" is the ask approach, "go in together" is the help approach, the guard is the fight |
| Hunt | tracking, packs | faked | one creature; tracking is the ask approach |
| Parley/intrigue | checks, branching | faked | talks fail into one fight at the table |
| Race against the clock | travel | faked | clock +1 on a defeat |
| Rescue / escape | multiple enemies, travel | faked | the captive is the stake; destination is the rendezvous |
| Lieutenant | multiple enemies, higher CR | faked | the named lieutenant is one CR 1/8-1/2 creature |
| Finale | higher CR, checks | faked | the strongest exact creature as the villain's champion; the villain's fate is told in the epilogue patterns |
| Rest interlude | a non-fight shape | not in the pack | recap screen |

Roadmap (smallest first, most variety per unit of work):
1. Campaign record and chapter chaining: campaign database, chapter run creation from the record, character carry-over by saved creation choices, outcome extraction. Unlocks every generated campaign in this document, with continuity and divergence. Mostly new, no rules changes.
2. Data-driven beat-script interpreter replacing the hard-coded journey phases (`narrative_phase.rs`, `journey.rs` transitions, the `SceneKey` set): beats, offers and transitions become pack data validated against a closed handler set. Unlocks 3-4 approaches, no-fight chapters (mystery, parley, interludes), per-chapter beat counts and real branching without touching rules.
3. Ability-check action using the existing proto message: DC and ability in the beat, result recorded and shown. Unlocks mystery, heist, parley and race chapters and gives skills (already chosen at creation) a purpose.
4. N-enemy encounters (entities, initiative, targeting) over the same melee model. Unlocks delve, defence, hunt packs and lieutenants with minions, and makes fights tense.
5. Reward ledger, inventory deltas and milestone levels. Unlocks retrieval and heist payoffs and act-break advancement.
6. Travel and exploration turns using the world crate. Unlocks race and hunt as written.
7. Ranged and spell actions and additional hero classes. Unlocks party variety and defence.
8. Boss features and higher-CR creatures with disclosed custom rules where needed. Unlocks real lieutenant and finale fights.

## 18. Campaign tables, counts and implementation additions

New files in `content/adventure-tables/`: `campaign-premises.json` (16 premises, 12 relics, 12 themes, 6 tones, 12 region names, 8 site roles), `villains.json` (12 types, 12 motives, 12 schemes, 10 methods, 8 resources, 12 weaknesses with two clue lines each, 12 signatures, 8 lieutenants), `factions.json` (12 factions, 6 attitudes each way), `chapter-types.json` (16 types), `act-structure.json` (3 acts, chapter-count table, slot templates for 6-10 chapters, 5 nodes, 6 lieutenant fates, rules, family hooks), `clocks.json` (9 clocks of 7 steps), `twists.json` (12), `clues.json` (5 nodes x 3 clues x 3 voices), `rewards.json` (the 12 chapter lines plus 12 milestones), `names.json` (extended with villain names and title adjectives), `art-manifest.json` (12 villain portraits added: 56 images in all). Lieutenants and minions reuse the antagonist portraits.

Reference-model checks (in the design session): 3,000 random frames with all slots, clocks, clues, twists and milestones rendered, no unresolved braces, every frame with a lieutenant before the finale and the finale last, no adjacent identical chapter types, every pillar mean at least 25 percent; fallbacks (a compatible entry not found in 64 rolls) occur in roughly 1 frame in 1,500, which is the recorded, deterministic path. This is design evidence, not verification of the running game.

Implementation additions (after section 12): (9) `adventure/campaign.rs`: frame roll, outline, repair, clock, ledger; (10) forced entries in the chapter generator (`force` map, recorded as `forced`); (11) campaign overlay patterns and the room-scene recap line (a proposed `recap` pack field, or reuse of `speaker`/`dialogue` as in the appendix); (12) campaign database and chapter chaining (roadmap 1); (13) recap and chapter fact sheets and the name and colour slot validators; (14) golden tests: the Appendix B frame and chapter 1 reproduce byte for byte; (15) evaluation: play chapters 1-3 of a generated campaign twice with different outcomes and check that chapter 2 and 3 differ and the recap is listener-safe.

Open choices for the owner are listed in the report that accompanies this commit.

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

## Appendix B. A worked campaign

Campaign seed `0x79a53da6463aaeed`, standard rules mode (all lieutenants and minions are exact SRD creatures). Everything in the frame below is a table result; the outline paragraphs are the DM-eye reading of the rolled slots (in the product, only the frame and the slot types exist at creation, and each chapter is written when it is reached).

### B.1 The frame: *A Bargain at Hollow Hill*

- **Premise:** A fey bargain comes due (family `blight`). Generations ago, the region's founders struck a bargain with something in the hills, and the term has run out. Someone is quietly trying to pay with other people's lives.
- **Villain:** Aldous, the Winter Regent: a smiling usurper in borrowed finery (voice: smooth, charming man). Motive: Aldous has lost the thread of reason and calls it clarity. Scheme: to call in a bargain that hands over the whole region. Method: bargains with fine print. Resources: a pack of trained beasts. Weakness the party can discover: Aldous still protects one person they once loved (clues: "A grave is tended every night with fresh flowers." and "A letter in an unfamiliar hand begs someone to stop."). Signature: Aldous turns every nearby lamp flame faintly blue. Lieutenant: the Bone Sentinel (an oathbound Skeleton, SRD p.325).
- **MacGuffin:** the Tidehorn, a spiral horn that calls the water; it commands tide and flood. Whoever sounds it at the hill can call in the debt.
- **Region:** the Tarn Marches. Sites and roles: the Orrery Library (home-base); the Windward Pass (first-clue); the Reedmoor Boardwalk (lieutenant-lair); the Pinewood Trail (stronghold); the Rookhill Gate (ally-haven); Thistledown Farm (old-secret); the Salt Quay (rumor-source); the Lamplit Market (neutral-ground).
- **Factions:** the Gray Mantle (collect secrets and trade them for influence); toward the party neutral, open to a good argument, toward the villain wary; the Ashen Court (restore a fallen noble house to power); toward the party wary and testing, toward the villain foe.
- **Recurring cast:** Dessa Quillon, a brisk young archivist (voice: brisk young woman); Rudd Cobb, a brisk watch sergeant (voice: brisk, no-nonsense man). Rival and minions: the Regent's guards and trained beasts.
- **Themes and tone:** mercy against vengeance, the cost of power; heroic and warm.
- **Clock (blight spreads, 7 steps):**
  1. Crops yellow near the Windward Pass; healers shrug.
  2. The sickness reaches the outskirts of the Orrery Library.
  3. Wells and streams turn bitter, and rumors accuse neighbors.
  4. Aldous offers a cure, and the desperate begin paying.
  5. Half of the Tarn Marches is under quarantine and quarrels break out.
  6. The source of the blight is moved to the Pinewood Trail, beyond reach.
  7. The land fails; Aldous stands as its only hope, and its only owner.
- **Act-break twists:** end of Act I, `price-of-cure`; end of Act II, `villain-right`.

### B.2 Act and chapter outline

Eight chapters: Act I (3), Act II (3), Act III (2). Pillar balance (combat/social/exploration, intended): 36/34/30. Each paragraph names the slot type, the place (a site from the region), the SRD creature that fights, the three leads and how it bends with an earlier outcome.

- **1. A Light Going Out (warning, opening; the Orrery Library).** Dessa, the archivist, runs in with the news that the old wards are failing and Rudd must be told before dawn. A hired guard of the Winter Regent blocks the way (Guard, AC 16, HP 11). Leads: Dessa's private whisper that animals are leaving the Marches; Rudd's remark that something strange is happening at the Windward Pass; the recap that whoever sent the guard 'turns every nearby lamp flame faintly blue' and is called the Winter Regent. The stake is a steady lantern that Dessa will not explain (hint 1 for the Act I twist). If the party loses, the clock moves to step 2, Rudd hears too late, and chapter 2 opens with him hurt.
- **2. Tracks in the Snow (hunt, first site; the Windward Pass).** Shepherds are losing flocks to something that follows lamplight. Hook: if chapter 1 was won, Rudd asks the party to guide a trapper; if lost, the trapper is the one who was never warned. The hunted thing is a raider (Goblin Warrior, AC 15, HP 10) leading the Regent's trained beasts. 'Ask' is reading the tracks (private clue: the Regent's title), 'help' is taking the lead. Leads: omens (public recap), villain (private), a first scrap of the scheme. If the party helped instead of asked, the trapper becomes an ally npc; if they asked, the asker holds a secret the others do not.
- **3. A Table in the Lamplight (parley, ally; the Lamplit Market).** The Ashen Court (wary, an enemy of the Regent) and the Gray Mantle (neutral) each want the party's account. The party escorts an Ashen Court envoy to the Gray Mantle's table; the Regent's agents try to seize her (Bandit, AC 12, HP 11). Leads: the scheme (the Regent means to call in the founders' bargain), a second villain clue, and a physician's glance at the steady lantern (hint 2). If the envoy is not delivered, she is taken, the Ashen Court turns hostile, and she is the hostage of chapter 7.
- **Act I break (twist `price-of-cure`).** The Gray Mantle's physician explains that the blight can only be paid off with a flame that never gutters, carried to Hollow Hill: the lantern the party carried in chapter 1. The party now owns the only cure and the Regent will want it. Milestone: the Gray Mantle owes the party a favor. Clock +1.
- **4. Three Trails to One Door (mystery, midpoint; Thistledown Farm).** A farmer says the blight began in the old orchard. Three trails (a grave tended every night with fresh flowers, an unsigned letter begging someone to stop, the founders' charter hidden in the orchard) all lead to Hollow Hill. Leads: the stronghold (Pinewood Trail), the first weakness clue (the tended grave), the scheme. A Kobold Warrior (AC 14, HP 7) bursts from the cellar to scare them off. Played as a clue-heavy warning chapter; the three trails are the three leads. If an earlier chapter was lost, the missing clue arrives by the rescue rule in the Previously line.
- **5. Under the Founders' Stones (delve, pressure; the Rookhill Gate crypt).** The charter points to a writ kept in the old founders' crypt. The entrance, the puzzle and the setback are told in the narration; the party plays the guardian room against a Giant Rat swollen by the crypt's strange air (AC 13, HP 7). Leads: the letter's author (weakness clue two), the stronghold (second), and the founders' writ itself (reward). If the Bone Sentinel was alerted by a lost chapter 3, it patrols the crypt stair and appears early.
- **6. The Bone Sentinel (lieutenant; the Reedmoor Boardwalk).** The Regent's oathbound dead guards a shrine on stilts where the 'cure' is brewed (Skeleton, AC 14, HP 13, SRD p.325). Leads: the stronghold, the weakness (a loved one), the Regent's method (bargains with fine print). After the fight the lieutenant's fate is rolled: captured and talking (a missing clue), escaped (clock +1, returns once), or informant (a lead and an obligation). Act II ends.
- **Act II break (twist `villain-right`).** The founders' writ shows the founders cheated the hill-folk and took more than they promised. The Winter Regent is, in one respect, right. The party must decide, in the story told by their approach labels, whether to finish the Regent or renegotiate the bargain (themes: mercy against vengeance). Milestone: the party's name is known to the Gray Mantle's council. Clock +1.
- **7. The Lantern-Maker's Shop (rescue, approach; the Lamplit Market).** The Regent's agents hold a hostage in a lantern-maker's shop: Rudd or Dessa if either was lost, otherwise the envoy of chapter 3. A Guard captain-for-hire (Guard, AC 16, HP 11) holds the door. Leads: the final stronghold confirmation and the loved-one clue (a name on the hostage's letter). If the clock reached step 5, the hostage is moved to the stronghold and chapter 7 becomes a pursuit.
- **8. The Hollow Hill (finale; the Pinewood Trail).** The Winter Regent speaks from the hilltop; his champion, an oathsworn Guard, bars the way (AC 16, HP 11; in custom mode the Scout, CR 1/2). 'Ask' is using the weakness (speaking the loved one's name from the letter), 'help' is rallying the allies. The fight is the same single melee; the epilogue differs: mercy (the bargain renegotiated and the debt shared) or vengeance (the bargain broken and the hill sealed, at a cost). Both epilogues are narration patterns keyed by the approach and by the clock step.

### B.3 What the storyteller is given for chapter 2

`recap_sheet` (listener-safe) after chapter 1 won, asked by the first hero:

```json
{"campaign": "A Bargain at Hollow Hill", "region": "the Tarn Marches", "tone": "heroic and warm",
 "teaser": "Generations ago, the region's founders struck a bargain with something in the hills, and the term has run out.",
 "chapters": [{"title": "A Light Going Out", "result": "won", "approach": "asked Dessa privately"}],
 "public_facts": ["Dessa Quillon's warning reached Rudd Cobb at the Archivist's Study.", "A guard in the pay of someone called the Winter Regent tried to stop it.", "Animals are leaving the Marches."],
 "open_leads": ["strange doings near the Windward Pass"], "latest_sign": "Crops yellow near the Windward Pass.",
 "npcs": [{"name": "Dessa Quillon", "attitude": "friendly"}, {"name": "Rudd Cobb", "attitude": "friendly"}]}
```

Pattern fallback for the Previously line: "Previously in A Bargain at Hollow Hill: Dessa's warning reached Rudd, but a guard in the Winter Regent's pay tried to stop it. Animals are leaving the Marches, and the old folk will not speak of the Windward Pass." The `chapter_sheet` additionally carries, flagged `hint_only`, the villain's true name (Aldous) and the Act I twist; the validators forbid both strings in any chapter 2 output.

### B.4 Chapter 1, fully written in the playable shape

Chapter seed `0xa2a2671963a4e9d7` (campaign seed, chapter 1, outcome digest 0). Forced by the campaign: location (home base), hook kind `warn` (type warning) with premise-biased hook `failing-ward`, patron and contact (the recurring allies), antagonist from the villain's minions, the three leads, the type's ask/help/stand labels. Antagonist: Halden Thorne, Guard, SRD 5.2.1 p.296 (exact): HP 11, AC 16, attack +3, spear 1d6+1.

**Offer labels:** `begin-story` = "Walk into the Orrery Library"; `ask-courier` = "Ask how bad it really is"; `escort-courier` = "Offer to carry the word"; `defend-courier` = "Keep the word alive"; `choose-harbor-scene` = "Press on to the study"; destination option "the Archivist's Study".

**Private clue** (asking player only): Dessa whispers to you alone: Rudd Cobb at the Archivist's Study must hear it before dawn. Animals have been leaving the Marches, and nobody can say what scared them. Keep it between us.

- **room** | *A Bargain at Hollow Hill* | art `loc-orrery-library-arrival`
  - Narration: Chapter 1. Rain taps the tall windows in patient rhythm. Dust motes drift through a shaft of moonlight. Share the room with your companions, then bring your heroes into the story.
  - The tale begins: "Generations ago, the region's founders struck a bargain with something in the hills, and the term has run out."
- **opening** | *A Light Going Out* | art `loc-orrery-library-arrival`
  - Narration: A brisk young archivist hurries toward you, shielding a small flame with her hand just as you arrive at the Orrery Library. A shadow detaches itself from the dark behind her.
  - Dessa: "The old wards are failing, and Rudd must be told before they fail completely. Please, help me reach him."
- **dialogue-seal** | *Whispers in the Dark* | art `loc-orrery-library-arrival`
  - Narration: Dessa draws you aside, breathing hard. Halden Thorne steps into the light, spear ready and a stubborn frown fixed in place.
  - Dessa: "Nobody listens to me. But the steady lantern proves it, and Rudd will know what it means."
- **dialogue-escort** | *Walk Together* | art `loc-orrery-library-arrival`
  - Narration: Relief crosses Dessa's face, and she moves to your side. Halden Thorne steps into the light, spear ready and a stubborn frown fixed in place.
  - Dessa: "The Archivist's Study is the place. Walk with me, and if anyone stops us, let them see you're ready."
- **combat** | *Trouble at the library* | art `loc-orrery-library-arrival`
  - Narration: Halden Thorne raises his weapon in answer. Both heroes are within 5 feet of the guard, and everyone fights in the open: no surprise, cover or movement. This encounter uses nonlethal melee attacks.
  - Halden Thorne: "I have my orders. Surrender the steady lantern, or I will make you."
- **inn** | *Safe at Last* | art `loc-orrery-library-interior`
  - Narration: The study is narrow and tall, its ladder-reached shelves lit by one bright reading lamp. Rudd, a brisk watch sergeant, looks up at once, rubbing his tired eyes.
  - Rudd: "Strange doings near the Windward Pass; I have been watching. On your next road, I will send word ahead that you are friends."
- **victory** | *A Moment of Quiet* | art `loc-orrery-library-arrival`
  - Narration: Halden Thorne falls unconscious with 1 HP. When the shaking stops, the pieces fit together. The attackers wore no marks, yet whoever sent them turns every nearby lamp flame faintly blue and is called the Winter Regent. His Short Rest has begun; campaign time has not completed it.
  - Dessa: "Let's go before anything else finds us. The Archivist's Study is close."
- **defeat** | *The Night Turns Against You* | Narration: Both heroes are unconscious with 1 HP after nonlethal attacks. The warning goes undelivered, and Rudd will hear the news too late. Their Short Rests have begun; campaign time has not completed them.

Ledger after chapter 1 if the party wins, asks, reaches the destination: omens 2/3 clues, villain 1/3; clock step 1; facts: the party carried the steady lantern; Dessa and Rudd friendly. If the party loses: omens 1/3 (private only, if asked), villain 0/3 (the recap never appears), clock step 2, Rudd wounded.

## 19. NPC dossiers (PLAY-N01)

Status: implemented as a pure module plus authored tables; PLAY-N02 (19.12) rolls it with every generated adventure and speaks from it. Governing documents: AGENTS.md; [Interaction engine](interaction-engine.md) ("Persistent social and epistemic model", "Grounded creative speech and listener-safe generation"); [Rules support](rules-support.md); [Coding style](coding-style.md). Appended after the appendices; sections 1-18 are unchanged.

### 19.1 Purpose

Until now an NPC in a generated adventure was a name, a role, a mannerism, a speech direction and (for the patron) one secret. That gives a reply writer nothing deeper to draw on when a player asks "who are you?", "why do you care?", flatters, flirts, threatens or offers a bribe. The dossier gives every speaking cast member a Dungeon Master's working sketch: where they come from, what they used to do, what they want and fear, why they are personally tied to THIS adventure, what they will say freely, what only after trust, what never, and how they react to the usual social pressures. Like everything else here it is rolled from named dice on original tables, recorded, and needs no model.

Non-goals: no rules. A dossier changes no number, grants no benefit and decides no outcome. No model call is made (the reply task, 19.9, uses the dossier as context). Silent creatures (beasts, undead) have no dossier.

### 19.2 Files and loading

`crates/df-tools/src/gameplay/adventure/`: `dossier.rs` (types and roll), `dossier_tables.rs` (typed serde loading with `deny_unknown_fields` and validation), `dossier_render.rs` (plain-text DM sheet), `dossier_tests.rs`.

Tables are five files in `content/adventure-tables/`: `npc-background.json`, `npc-character.json`, `npc-ties.json`, `npc-knowledge.json`, `npc-social.json` (schema `df-npc-tables/1`). They are deliberately NOT listed in `tables.json`: that list is hashed into the chapter tables revision, and adding to it would change the recorded revision of every existing adventure. The dossier tables carry their own revision (SHA-256 over their bytes in a fixed order), stored in each dossier as `tables_revision`. If the owner later wants one revision, the index entries and the loader can be merged in a single coordinated change.

Validation at load: standard dice only (d4, d6, d8, d10, d12, d20); entries at most the die; face ranges tile 1..die with no gap or overlap; unique kebab-case ids; template length 1-240 characters; every `{slot}` known and every tie naming the fact it binds; tags known; each tone-tagged table has an untagged entry (an agreement fallback); and a coverage lint that proves, for every role and speaker group and every kind of antagonist, that at least one entry needs only facts a one-chapter adventure always has. A rare combination therefore cannot fail an adventure at run time.

### 19.3 The dossier

`Dossier` (serde, stable field names, persisted with the adventure record): `npc_id`, `name`, `role` (`patron`, `contact`, `antagonist`, `villain`, `lieutenant`), `role_label`, `tables_revision`; `given` (the generator's own mannerism, speech direction, voice tag, the tone tags read from them and a DM note when they already disagree); `origin`, `past_trade`, `formative_event`, `ideal`, `bond`, `flaw`, `want`, `fear`, `talent`; `story_tie` (id, what it binds to, the bound fact, bound cast member id, public or private, DM text, first-person line); `motive` and `stand_down` (antagonist, villain and lieutenant only); `knowledge` (three tiers, 19.5); `temperament`; `starting_attitude`; `speech_style`; `prepared_answers`; `rolls`.

### 19.4 Tables and dice

| File | Table | Die | Entries | Notes |
|---|---|---|---|---|
| background | origins | d20 | 20 | 14 for people, 6 for goblinoids; a participle phrase used as "I was ..." |
| background | past_trades | d20 | 20 | 16 / 4; each has a first-person line; `except` keeps a trade from equalling the job |
| background | formative_events | d20 | 20 | 15 / 5; public-safe first-person line |
| background | talents | d12 | 12 | DM only |
| character | ideals, bonds | d20 | 20 each | classic personality tradition; some role- or speaker-filtered |
| character | flaws | d20 | 20 | each has a never-spoken `shame` line and `shame_terms` |
| character | wants | d20 | 20 | role-filtered: 12 for patron and contact, 6 each for antagonist and lieutenant, 3 for villain |
| character | fears | d12 | 12 | each has a `confide` line for the trusted tier |
| ties | patron, contact, antagonist | d20 | 20 each | bindings: stake, antagonist, location, destination, another cast member, faction, villain |
| ties | villain | d12 | 12 | |
| ties | lieutenant | d8 | 8 | |
| ties | motives | d20 | 20 | human reasons; some goblinoid-only, some villain-only |
| ties | stand_downs | d12 | 12 | what would make them back off or bargain |
| knowledge | place_specific | 8 x d4 | 32 | four local-colour lines per location id |
| knowledge | place_generic | d12 | 12 | uses the location and the destination |
| knowledge | rumours | d12 | 12 | about the antagonist, filtered by creature group |
| knowledge | hints | d12 | 12 | the single useful, mechanically neutral pointer (trusted tier) |
| knowledge | world | d8 | 8 | four about a faction, four about the villain |
| knowledge | secret_keywords | by secret id | 12 | phrases no public line may contain, on top of the rendered secret text |
| social | attitudes | 5 x d20 | weighted ranges | patron 13/6/1 friendly/indifferent/hostile; contact 10/8/2; antagonist 1/5/14; villain 2/6/12; lieutenant 1/7/12 |
| social | styles | d20 | 20 | sentence length, vocabulary, a tic, an opener and a closer |
| social | flattery, flirting, threats, lies, bribes | d8 | 8 each | a DM description and a first-person line |
| social | deflections | d12 | 12 | the "what are you hiding" answers; reveal nothing |
| social | frames | d4 / d6 | 22 | assemble prepared answers (19.7) |
| social | tag_vocabulary, incompatible, trait_tags | | 10 tags, 8 pairs, 51 keyword rules | the agreement rule (19.6) |

Templates use `{name}`, `{role}`, `{role_a}` (with article), `{sub}`, `{obj}`, `{pos}`, `{stake}`, `{stake_full}`, `{location}`, `{destination}`, `{antagonist}`, `{antagonist_the}`, `{villain}`, `{faction}`, `{other}`, `{other_role}` and the `|cap` filter. A template using a slot the adventure does not have (no villain, no faction, no other eligible cast member) is "incompatible" and is rolled again, exactly as in section 3. Slots naming the NPC itself stay unavailable (an antagonist's tie never says "owes the antagonist"). Antagonist ties and motives are human: debts, hunger, pride, a sick relative, an old wrong; each antagonist also has a stand-down so a conversation is worth having.

### 19.5 Three knowledge tiers

- `says_freely`: local colour, rumours, public facts about location, destination and faction, and the NPC's self-description (origin, trade, event, ideal, bond, want, a public tie, the stated motive of an opposing NPC). Each line carries a topic.
- `says_when_trusted`: a private tie, one hint about the antagonist or the road that changes nothing mechanically, the confided fear, and (opposing NPCs) the stand-down.
- `never_says`: the NPC's shame (with its terms) and, by reference, the adventure's secret terms (the rendered secret and confirmation text plus the keywords for that secret id).

Safety rule, enforced in code (`Dossier::leak`) and tests: no `never_says` term and no secret term may appear, as a case-insensitive whole-word (whole-phrase) match, in any `says_freely` line, any prepared answer, or a story tie marked public; the NPC's shame terms appear nowhere that is spoken. During rolling, a candidate whose public text would break the rule is rejected and re-rolled; a tie that would have to mention a secret term is simply private (it lives in the trusted tier, where secret terms are allowed). `roll_dossiers` fails closed with `DossierError::Leak` if a leak survives. The check is a heuristic backstop: the tables themselves avoid asserting anything about ownership, contents or routes, which the secrets own. Summaries never replace canonical truth; the dossier never states a fact the pack contradicts.

### 19.6 Temperament, attitude and agreement

Starting attitude uses the standard 2024 Friendly / Indifferent / Hostile scale, weighted by role (19.4). It is where the NPC starts, nothing more; the live value is owned and saved by the server.

Temperament gives, for flattery, flirting, threats, lies and bribes, a DM description and a first-person line. In the `restrained` content rating every entry is family-suitable warmth, wit and banter (flustered, teasing, spoken for, cool and unimpressed, playing along to learn something), never explicit and never a mechanical benefit; the `mature` rating draws flirting and threats from replacement tables (section 21). Neither is ever a mechanical benefit.

Agreement rule. The chapter generator already gave each NPC a mannerism, a speech direction and a voice tag, and the review found clashes such as "brisk" beside "tugging nervously". Keyword rules (`trait_tags`) read tone tags from those three texts; a speech style and each temperament entry must not carry a tag that the incompatible-pair list (nervous/calm, nervous/brisk, hurried/slow, brisk/slow, calm/hurried, warm/cold, playful/gruff, playful/formal) forbids against the given tags or against anything already chosen. A clashing entry is rejected and rolled again; untagged entries guarantee an answer. If the given traits themselves already clash, the dossier records a note telling the DM to play it as strain showing through. The dossier never edits the given traits.

### 19.7 Speech style and prepared answers

A speech style records sentence length, a vocabulary note, a verbal tic and a short opener and closer. `prepared_answers` holds one in-character answer (one to three sentences, first person) for each fixed topic: who are you, your past, why this matters to you, the antagonist (for opposing NPCs, their stated motive), the place, a compliment or flirt, a threat, a bribe, what are you hiding. The first five are assembled from a rolled frame and lines the NPC may say freely; the other four use that NPC's temperament line or deflection, which reveal nothing. The tic is added as an opener or closer when the answer still fits. Prepared answers are the fallback when no text model is on and the style anchor when one is.

### 19.8 Determinism and the per-NPC stream

A dossier is a pure function of `(adventure seed, NPC id, facts, tables)`. Each NPC rolls on its own SplitMix64 stream, `Dice::new(seed ^ 0x4e504344_4f535331 ^ fnv1a64(npc_id))`, using the existing generator; the adventure's main stream is never read or advanced, so every existing seed still produces byte-identical adventures and a new NPC never shifts another's rolls. Every roll is recorded like the DM's rolls (`n`, `table`, `die`, `natural`, `entry_id`, `status`), with table names prefixed `dossier.`; rejections re-roll the same die and after 64 the first fitting entry is taken and recorded as a fallback. Roll order: attitude, flaw (first, so its shame terms guard every later public line), origin, past trade, formative event, ideal, bond, want, fear, talent, speech style, the five temperament entries, deflection, story tie (and which other cast member), motive and stand-down (opposing roles), place lines, rumour, hint and world line (patron and contact), then the five frames. No two cast members of one adventure share a story-tie template. NPC ids must be unique and stable; the suggested form is `<role>:<archetype id>`.

### 19.9 What PLAY-N02 does with it

Reply context for a player's spoken or typed question = the dossier's public tier (`says_freely`, the public tie, the speech style, the temperament line for the kind of pressure applied, `given` traits) plus the trust-gated tier only when the server-owned attitude allows it (Friendly, or Indifferent after a successful interaction); never `never_says`, and the secret terms are passed as a forbidden list the generated reply is checked against, with the prepared answer as the fallback and the style anchor. The attitude lives on the server, is saved with the run, and starts at `starting_attitude`. A change of attitude that matters mechanically must use the standard Influence action (an ability check) once ability checks exist; until then the server may move attitude by narrative events only and must record that it did. Deceit is attributed: a lie told by a player or an NPC is a recorded statement by that speaker, not a fact of the world. Dossier text is flavour; mechanical outcomes, rewards and secrets are never decided by it.

### 19.10 Standard and custom

Standard: the Friendly / Indifferent / Hostile attitude scale; the Influence action as the future mechanical hook; no change to any rule or number. Custom (and so marked wherever shown): everything else in this section is original table content and a presentation aid, not a rules feature, in particular tone tags, temperament, story ties, prepared answers and the trust gate. Names and lore stay generic heroic fantasy; nothing is copied from non-SRD sources.

### 19.11 Facts the adapter must supply

`DossierFacts`: `cast` (one `CastMember` per speaking NPC: stable `id`, the name the party hears, `role`, article-free `role_label`, `archetype` id, `gender`, `group` person or goblinoid, and the generator's existing `mannerism`, `delivery` and `voice` strings), `stake` (short and full text), `location_id`, `location` (with article), `destination`, `antagonist` (display name, "the ..." form, creature group), optional `villain` and `faction` (names the party may already hear), `secret_id` and `secret_terms` (the rendered secret and confirmation). Silent antagonists are left out of the cast. The campaign layer later adds the villain, faction and lieutenants without changing the contract.

### 19.12 What PLAY-N02 built (dossiers in play)

Status: implemented and wired. Everything below is custom story colour, not a rules feature: it grants no bonus, changes no roll or difficulty and unlocks only conversation. It is NOT the standard Influence action (an ability check, which does not exist yet); when that exists it replaces how attitude moves and this layer only voices the outcome.

**Rolled with the adventure.** `adventure/dossier_cast.rs` is the adapter of 19.11. Cast = patron and contact, plus the antagonist only when it speaks; ids are `<role>:<archetype id>`; the antagonist's group comes from the antagonists table's class. Random adventures roll in `generate`; campaign chapters roll again at the end of `generate_chapter` (after the overlay renames the antagonist and replaces the secret) with only the public villain alias (`the <title>`) and the first faction, and with the private lead and the villain's real name added to the secret terms. The dice are the per-NPC streams of 19.8, so no seed changes what it generates; the dossiers are not in `rolls`. They are stored in the adventure record (`adventure.json`, key `dossiers`, optional on load: an adventure saved before this keeps playing without them, and so does the shipped harbor pack). Loading refuses duplicate ids or a dossier that leaks (`Dossier::leak`). The temperament tables gained an `effect` per entry (`warm`, `cool` or `steady`) and `npc-social.json` a `curt` table (d8); each dossier keeps three curt lines, rolled last so no earlier roll moved.

**Attitude and trust** (`gameplay/social.rs`). Each NPC has an attitude toward each hero (Friendly / Indifferent / Hostile, starting at `starting_attitude`) and a trust counter between -1 and +1. A line is read as one of question, kindness, flattery, flirting, persuasion, deception, intimidation, bribe, insult or other: by the same model call that writes the reply (the `approach` slot), or by a keyword reader when the model is off or gives no usable word. Flattery, flirting, intimidation, deception and bribe take their `effect` from the NPC's own temperament entry; kindness warms, insult cools, question, persuasion and other change nothing. Bounds: a warm or cool exchange moves trust by one; two in the same direction move the attitude one level and reset the counter, so one exchange moves attitude at most one level; per hero and NPC the attitude never ends a scene more than one level from where the scene found it (a blocked step holds trust one point short); an opposing NPC (antagonist, villain, lieutenant) never becomes Friendly from talk alone. The trust-gated tier is open when the attitude is Friendly, or, for an opposing NPC, Indifferent with trust of at least one (it hears you out). The state is server-owned, per run, and saved after each exchange in `social.json` beside the run's `pack.json` (atomic replace; no PostgreSQL schema change). It holds attitude, trust, the scene, and the last six lines of each conversation. On start it is reloaded; a file for another adventure or out of bounds is refused, not ignored. Every change is a public table line under the NPC's name: "warms to Brakka", "cools toward Brakka", "softens a little toward Brakka", "grows wary of Brakka".

**The reply** (`gameplay/npc_dossier.rs`, `npc_reply.rs`). A typed line goes to the on-stage NPC it names, otherwise to the one speaking. The prompt facts hold, by tier: always the identity, given voice, manner and delivery, the speech style (length, vocabulary, tic, opener, closer), the NPC's want, every `says_freely` line (this includes the public story tie), the five temperament notes, the attitude toward this hero, the last six lines of this hero's conversation with this NPC, the scene title, the NPC's current scripted line and the offered choices; only when trusted, also `says_when_trusted` and the confided fear. Never in the prompt: `never_says` text, any `never_says` or secret term (the pack's terms the table has not heard, plus the dossier's own), the private clue, the flaw and the talent. Any line of any tier that mentions a withheld term is dropped, and a term the player types is redacted to an ellipsis. The withheld list is given to the storyteller as a slot-level list that is checked in the answer and never printed in the prompt (`Slot::withholding`), together with phrases that mean the reply left character ("as an AI", "ignore all", and similar); the existing forbidden-terms refusal stays as the second line of defence. The instructions are fixed text; the player's words appear only inside `facts.player.exact_words` and are described as speech to react to, never as instructions. Replies are two to four spoken sentences (limit 420 characters, the table log's line limit was raised to match), first person, in character, answering the actual question; under `restrained`, flirting gets warmth, wit or a graceful deflection, never explicit and never a promise of help; under `mature`, flirting is answered by the NPC's own temperament (section 21). Never a promise of help in either.

**Fallback.** With the text model off, over budget, failing, rate limited or refused for a leak, the reply is the dossier's prepared answer for the detected topic: a Hostile NPC, or any NPC insulted, gives one of its curt lines; a trusted NPC asked something a trusted line answers (what are you hiding, the antagonist, why it matters) confides that line; a lie gets the NPC's own reaction to being lied to; everything else gets the prepared answer, and a prepared answer that would name a withheld term gives the deflection instead. Every line still gets an in-character reply.

**Wire.** `JourneyView.standing` (field 13, `NpcStanding { npc_id, name, attitude }`, 1 hostile, 2 indifferent, 3 friendly, 0 unknown) lists, for the player's own view only, each present NPC with a dossier. The display view carries none: attitude is per hero, so no party-wide figure exists, and every change is already published as a table line.

## 20. Scene settings

PLAY-G05 makes scene geography explicit. Every newly generated scene carries `place_id`, `name`, `short`, a closed `kind`, `exterior`, and the intended `art_key`. Names describe where the narration takes place, including a standalone inn beside a library rather than an innkeeper inside its reading hall. `places.json` owns this information without modifying patron/contact archetypes.

Closed vocabulary: `apothecary`, `archive`, `archivists-study`, `bell-chamber`, `bindery`, `boathouse`, `bothy`, `caretakers-lodge`, `chandlers-room`, `chapel`, `charcoal-hut`, `cider-cellar`, `eel-catchers-hall`, `farm-kitchen`, `farmyard`, `ferry-office`, `ferrymans-hut`, `forest-trail`, `graveyard-gate`, `guardhouse`, `hayloft`, `herb-shed`, `hermits-cabin`, `inn`, `lamp-room`, `lantern-shop`, `library-hall`, `map-room`, `market-street`, `marsh-boardwalk`, `mill-house`, `mountain-pass`, `net-loft`, `pilgrims-hall`, `quay`, `reading-gallery`, `refuge-hall`, `sheepfold`, `shop`, `shrine`, `stilt-house`, `tea-house`, `temple`, `theater`, `undercroft-study`, `vestry`, `waystation-hall`, `woodland-lodge`.

Each chapter has three settings: the patron meeting place (opening and both conversation branches), the location confrontation (gathering lobby, antagonist arrival/combat and defeat), and the destination (victory aftermath and resolution). The lobby retains the location arrival painting. The current SceneKey contract has no separate antagonist-arrival scene: the recorded bridge and antagonist arrival therefore begin Combat, when its background changes. Conversation stays at the meeting place. Meeting sensory detail and confrontation bridge are recorded d4 rolls. Campaign forced cast must have at least one drawable compatible destination at its site; otherwise a compatible faction contact is rolled. The campaign opening and aftermath overlays retain their placement sentences.

Scene manifest entries declare typed `depicts` kinds, judged from their prompt. Table loading checks selected art/setting compatibility, destination coverage and contact compatibility. Pack loading rejects unknown kinds, depiction/path mismatches and partly missing settings. `adventure.json` persists the three chapter settings and binds them to every pack scene; absent settings use the typed `LegacyNoSettings` path, preserving historical stored scenes without regeneration. The three settings are also added to expression facts.

Startup checks the actual served UI art directory (webp, png, then jpg). Missing scene files use the location arrival painting and emit `ADVENTURE art missing <art_key> -> <fallback>`. If that painting is also absent, the already-required harbor general painting is used. This explicitly recorded presentation substitution leaves declared settings and immutable pack/record bytes unchanged; intended-art depiction validation is performed before substitution. Pure generation performs no I/O.

Unresolved source conflict: `loc-orrery-library-arrival` actually depicts an indoor library hall, so it truthfully declares `library-hall` and `exterior: false`. Its lobby and confrontation cannot simultaneously use that painting, depict the setting honestly, and satisfy the all-exterior requirement. An exterior-library painting or coordinator-selected location exclusion is required before that acceptance gate can pass. No exterior description is falsely assigned to the indoor image.

The following eight destinations have matching existing paintings: Net Loft (`net-loft`), Woodcutters' Lodge (`woodland-lodge`), Windward Waystation (`waystation-hall`), Brannock's Apothecary (`apothecary`), Chapel of Quiet Bells (`chapel`), Stilt House (`stilt-house`), Farmhouse Kitchen (`farm-kitchen`), and Map Room (`map-room`). All other destinations have `art: null` and are excluded until dedicated art exists, in standard and custom modes alike. A miller may be met only at a mill or farm kitchen; the latter is presently the only drawable option.

Excluded destination kinds and dedicated painting requests (one painting each; names identify exact subjects):

- `chandlers-back-room` / `chandlers-room` — Behind barrels of lamp oil, a warm back room offers a long table and a banked fire. Existing `salt-quay` interior depicts another kind; paint this named setting.
- `tidewatch-lamp-room` / `lamp-room` — At the top of the Tidewatch Light, the great lamp turns and throws gold across the water. Existing `salt-quay` interior depicts another kind; paint this named setting.
- `old-boathouse` / `boathouse` — Inside the old boathouse, upturned skiffs make a shadowed hall around one hanging lantern. Existing `salt-quay` interior depicts another kind; paint this named setting.
- `charcoal-hut` / `charcoal-hut` — The hut's low door opens on a banked hearth, drying herbs and the scent of woodsmoke. Existing `pinewood-trail` interior depicts another kind; paint this named setting.
- `hermits-cabin` / `hermits-cabin` — A tiny cabin hides among the pines, one thread of candlelight leaking past its shutters. Existing `pinewood-trail` interior depicts another kind; paint this named setting.
- `pilgrims-rest` / `pilgrims-hall` — Pilgrims' Rest is a long, low hall of whitewashed stone, its benches warm from a roaring fire. Existing `pinewood-trail` interior depicts another kind; paint this named setting.
- `shepherds-bothy` / `bothy` — The bothy is little more than stone and turf, but its stove glows and the kettle sings. Existing `windward-pass` interior depicts another kind; paint this named setting.
- `muleteers-inn` / `inn` — The inn rings with low talk, wet boots steaming before a crowded hearth. Existing `windward-pass` interior depicts another kind; paint this named setting.
- `cairn-house` / `refuge-hall` — The Cairn House is a squat stone refuge, its long table lit by a single iron lamp. Existing `windward-pass` interior depicts another kind; paint this named setting.
- `tin-street-tea-house` / `tea-house` — Paper screens glow around the tea house, where steam rises from a dozen small cups. Existing `lamplit-market` interior depicts another kind; paint this named setting.
- `puppet-theater` / `theater` — Behind the little stage curtain, painted faces watch from a lamplit shelf. Existing `lamplit-market` interior depicts another kind; paint this named setting.
- `lantern-makers-shop` / `lantern-shop` — In the lantern-maker's shop, a hundred unlit lamps hang like sleeping fruit. Existing `lamplit-market` interior depicts another kind; paint this named setting.
- `caretakers-lodge` / `caretakers-lodge` — The lodge is small and tidy, with a kettle on the stove and keys hanging in neat rows. Existing `rookhill-gate` interior depicts another kind; paint this named setting.
- `undercroft-study` / `undercroft-study` — Down a short stair, the undercroft study smells of beeswax, old paper and cold stone. Existing `rookhill-gate` interior depicts another kind; paint this named setting.
- `vestry` / `vestry` — The vestry is warm and cluttered with robes and candle stubs, and a lamp glows on the table. Existing `rookhill-gate` interior depicts another kind; paint this named setting.
- `heron-shrine` / `shrine` — Inside the shrine, lanterns shaped like herons light a floor of polished boards. Existing `reedmoor-boardwalk` interior depicts another kind; paint this named setting.
- `eel-catchers-hall` / `eel-catchers-hall` — The hall smells of smoke and river mud, and every beam is hung with lanterns and baskets. Existing `reedmoor-boardwalk` interior depicts another kind; paint this named setting.
- `ferrymans-hut` / `ferrymans-hut` — The ferryman's hut is snug and low, with a stove, a rope bed and a lamp swinging overhead. Existing `reedmoor-boardwalk` interior depicts another kind; paint this named setting.
- `old-hayloft` / `hayloft` — Up in the old hayloft, lanterns hang from the rafters over sweet-smelling bales. Existing `thistledown-farm` interior depicts another kind; paint this named setting.
- `mill-house` / `mill-house` — In the mill house, great wooden wheels creak softly behind a table laid with bread and cheese. Existing `thistledown-farm` interior depicts another kind; paint this named setting.
- `cider-cellar` / `cider-cellar` — Down in the cider cellar, casks line the walls and one warm lamp lights a crooked table. Existing `thistledown-farm` interior depicts another kind; paint this named setting.
- `binding-workshop` / `bindery` — The bindery smells of paste and leather, with unfinished books lined up like sleeping birds. Existing `orrery-library` interior depicts another kind; paint this named setting.
- `archivists-study` / `archivists-study` — The study is narrow and tall, its ladder-reached shelves lit by one bright reading lamp. Existing `orrery-library` interior depicts another kind; paint this named setting.
- `reading-gallery` / `reading-gallery` — The gallery's balcony looks down over sleeping desks, and a single lamp burns on the rail. Existing `orrery-library` interior depicts another kind; paint this named setting.

Meeting paintings still being supplied by the coordinator: `scene-place-inn-taproom`, `scene-place-temple-sanctuary`, `scene-place-archive-stacks`, `scene-place-ferry-office`, `scene-place-cloth-shop`, `scene-place-herb-shed`, `scene-place-lamp-room`, `scene-place-shepherds-fold`, `scene-place-guardhouse`, `scene-place-bell-chamber`. The manifest and served allowlist include them; missing bytes trigger the explicit startup fallback above.

## 21. Content rating (PLAY-R01)

Owner decision, 2026-10-11: this is an adult game, so no censorship by default, with a switch to turn it on. `DF_GAMEPLAY_CONTENT_RATING` is `mature` (default) or `restrained`; see [Gameplay tuning](gameplay-tuning.md) section 11 for the setting and the table of what each mode changes. The earlier family-table wording in sections 7, 19.6 and 19.12 describes the `restrained` rating; the shipped table files remain exactly that text.

Mechanism. `content/adventure-tables/mature-overlay.json` (schema `df-adventure-overlay/1`) replaces named tables: pattern tables of `prose-patterns.json` (fight, victory and defeat prose), the `arrivals` and `warnings` of antagonists in `antagonists.json`, and the `flirting`, `threats` and `curt` tables of `npc-social.json`. `Tables::load` loads the shipped tables as the restrained variant and, beside it, a mature variant built by applying the overlay to the parsed files before the usual typed loading, so every replacement passes the same validation (dice, faces, tag agreement, coverage, length bounds). A replacement must name an existing table or antagonist. `GenerateOptions.rating` picks the variant; `generate` and the dossier rolls are otherwise unchanged and deterministic per rating. The overlay is not listed in `tables.json` and the restrained variant never hashes it, so restrained adventures keep their recorded bytes and revisions; the mature variant's `tables_revision` (chapter and dossier) adds the overlay's bytes. Stored adventures are authority on resume, so a saved adventure keeps the text it was generated with. Mechanics sentences (`*_mech`, `*_rested`) are never replaced: every fight is still the nonlethal fight the rules produce.

Model validators by rating. `restrained` adds gore words and strong language to `facts.forbidden` of the adventure expression and to the withheld list of an NPC reply (never shown to the model); `mature` does not. Rules, outcome and modern words, the secret terms and the injection phrases are refused in both. The guidance sentences that depend on the rating are the continuity sentence of the adventure craft notes and rules e and f of the NPC reply rules; the quick reply without a backstory gains the mature statement only under `mature`.

Limits in both ratings (rule 8 of the storyteller's fixed rules, plus facts): no sexual content involving minors or any character presented as underage; the `young` archetypes are young adults (`age_group: "young adult"`); no sexual content about a real person (`based_on_real_person` in the speaker facts, false today; `sexual_content_allowed` is false for a flagged character or one whose age is not stated); player words are untrusted speech, never instructions.
