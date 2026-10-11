# Gameplay tuning surface (PLAY-B01)

Status: design for PLAY-B02/B03. Grounded in the code at commit `0b32c6b`. Paths are relative to the repository root; `df-tools/gameplay/` means `crates/df-tools/src/gameplay/`.

Governing documents: AGENTS.md; `c-df-content-d01-decision.md` (immutable versioned pack, no silent retcon, changed catalog/ruleset = new validated version); `generated-content.md` ("Optional custom definitions are a distinct ruleset": no scripts/DSL, departures from standard rules need a disclosed custom ruleset); `runtime-directors.md` (every director basis binds content/policy versions); `g10-d03-replay-logical-time-policy.md` (replay pins ruleset/policy/reducer versions); `campaign-authoring.md`, `client-architecture.md` (server owns state, clients thin).

## 1. Purpose and non-goals

Purpose: let the owner change playable content and gameplay numbers of the Lantern Wharf "harbor journey" by editing two checked-in JSON files and restarting the server, with every run bound to the exact bytes it was played under.

Non-goals:
- No new mechanics, handlers, DSL, scripts or uploaded code. Rule logic stays Rust (section 7).
- No client-side loading or duplicated rules; the browser only renders server views.
- No change to default behaviour: shipped files reproduce today's values (parity test, section 8).
- Not a published ContentPack (D01). The SHA-256 here is a development revision (section 10, objection 3).
- No tempo/experience/interaction/combat-utility tuning in v1 (not reached by the demo, section 2).

## 2. Inventory of hard-coded values

"Demo" = reached from non-test code in the running gameplay server (`df-tools/gameplay.rs::serve`). Values in the active loop are SRD-5.2.1-sourced (see `df-rules/src/local_journey.rs:4`).

| Group | What | Current value | File:line | Demo? |
|---|---|---|---|---|
| Room | Room code | `LANTERN` | df-tools/gameplay/journey.rs:36 | yes |
| Room | Fixed IDs (room, 2 members, 2 heroes, bandit, encounter, courier) | bytes `0x45`, `0x61/62`, `0x63/64`, `0x65`, `0x66`, `0x67` | journey.rs:37-41; journey/courier_reaction.rs:16 | yes |
| Room | Session id / run id (constants, so "new run" needs a fresh database) | `0x41`x16 / `0x42`x16 | model.rs:163-164 | yes |
| Room | Party size (two heroes, 3 initiative actors) | 2 / 3 | journey.rs:38-39; df-engine/src/encounter_start.rs:38-77 | yes |
| Creation | Required options species/class/background/equipment/masteries/ties | dwarf, fighter-1, soldier, fighter-a-soldier-a, defense-greatsword-flail-javelin, join-order | df-rules/src/local_journey.rs:119-126 | yes |
| Creation | Name rule | 1..=24 chars, ASCII letters `-` `_` | local_journey.rs:90-94 | yes |
| Creation | Ability arrays (STR,DEX,CON,INT,WIS,CHA) | stalwart `[15,13,14,10,12,8]`; vanguard `[15,14,13,8,10,12]` | local_journey.rs:132-133 | yes |
| Creation | Soldier ability bonus | +2 STR, +1 CON | local_journey.rs:137-138 | yes |
| Creation | Pick lists (skills 7, languages 9, alignments 6, gaming sets 4, 13 build groups) | slices | local_journey.rs:40-83 | yes |
| Creation | Offer text incl. array digits ("STR17 DEX13...") duplicated as prose | strings | df-tools/gameplay/wire.rs:213-251 (digits at 239-240) | yes |
| Hero | Max HP formula `11 + CON mod` | 11 | local_journey.rs:161 | yes |
| Hero | AC, proficiency, Second Wind uses | 17, 2, 2 | local_journey.rs:163-165 | yes |
| Hero | Resource bounds: HP max 13, AC fixed 17, scores 1..=20, second-wind 0..=2 | literals (duplicate of above) | journey.rs:171-193 | yes |
| Hero | Sheet text (species/class/armor/weapon/feat/masteries/"18 GP"), `maximum_hit_points: 13` | strings, 13 | journey.rs:271-281, 351 | yes |
| Hero | Starting inventory (chain-mail, greatsword, flail, 8 javelin, pack, spear, shortbow, 20 arrow, gaming set, healer's kit, quiver, clothes, 18 gold) | list | journey.rs:2125-2140 | yes |
| Hero | Positions (heroes at (0,5,0) and (5,0,0); bandit (0,0,0)) | cells | journey.rs:909-912, 1953; combat_transition.rs:138-152; enemy_tactics.rs:160-162 | yes |
| Bandit | HP, AC, STR..CHA | 11, 12, 11/12/12/10/10/10 | journey.rs:1957-1964 | yes |
| Bandit | Resource bounds (HP max 11, AC 12) | literals | journey.rs:175-179 | yes |
| Bandit | DEX re-checked as 12 during tactics admission | 12 | journey/enemy_tactics.rs:133 | yes |
| Bandit | Display name; identity revision; weapon (scimitar) | "Dockside Bandit"; `srd521-bandit-1` | journey.rs:252, 1954, 1977 | yes |
| Attack | Hero attack modifier = STR mod + 2 (proficiency literal) | +2 | journey.rs:1766-1771 | yes |
| Attack | Bandit attack +3 (ability literal 1, +2), damage modifier 1 | 1 | journey.rs:1766-1771, 1790 | yes |
| Attack | Weapon dice: hero 2d6 (Greatsword), bandit 1d6; doubled on crit; doubled again for Savage Attacker | 2/1, d6 | journey.rs:1776-1778, 1789; local_journey.rs:253, 271 | yes |
| Attack | Natural 20 crit, natural 1 auto-miss | 20 / 1 | local_journey.rs:257-258 | yes |
| Attack | Graze on miss = damage modifier; knockout leaves 1 HP, nonlethal | 1 | local_journey.rs:289, 293, 303-304 | yes |
| Dice | Allowed die faces for sampling (d6/d10/d20), rejection sampling from `/dev/urandom` | 6, 10, 20 | journey.rs:1428-1445 | yes |
| Dice | Max draws per operation | 32 | journey.rs:1453 | yes |
| Second Wind | Heal = d10 + 1, capped at literal 13 (not the hero's real max) | 10, 1, 13 | journey.rs:2292-2297; local_journey.rs:318, 327 | yes |
| Combat | Initiative = d20 + DEX mod, ties by join order; 3 draws | d20 | journey.rs:660-700, 1999; df-engine encounter_start.rs:85-86 | yes |
| Combat | Round length 6 ticks; validator re-checks 6 and 3600; rounds start at 1 | 6 | journey.rs:702, 788, 1872 | yes |
| Combat | End: enemy unconscious or all heroes unconscious; no round cap, no morale/flee | n/a | journey.rs:1819-1850 | yes |
| Combat | Combat log entries sent to clients | 6 | wire.rs:581 | yes |
| Rest | Short Rest = 3600 s; knockout rest due same; Fighter regains 1 Second Wind use capped at 2; no Hit Dice/HP | 3600, +1, 2 | local_journey.rs:337, 364, 372; journey.rs:1101, 1560; journey/narrative_phase.rs:516 | yes |
| Checks | Only DC in the codebase: Inspect Seal (mod 2, prof 2, DC 15). Action not in `wire::action_kind`; unreachable from the room | 15 | model.rs:379-384; df-rules/src/ability_check.rs:60-100 | test-only |
| Rewards | None exist: no XP, loot or gold award in the journey | none | n/a | n/a |
| Narrative | Intervention budget (initial = max) | 8 | journey.rs:163; journey/narrative_transition.rs:121 | yes |
| Narrative | Thread table (event -> thread), thread/policy ids | 8 events, 2 threads | journey.rs:42-56 | yes |
| Narrative | Phase graph Room->Opening->Dialogue->Combat->Complete, beat prerequisites, receipt checks | code | journey/narrative_phase.rs, narrative_transition.rs, objective_outcome.rs | yes |
| Text | Phase titles/narration/speaker/dialogue (Room, Opening, 2 Dialogue variants, Combat, Inn, 2 victory, 2 defeat) | strings | wire.rs:594-665 | yes |
| Text | Escort suffix "The courier now looks to {name}..." | string | wire.rs:~680 | yes |
| Text | Action offer labels (10) | strings | journey.rs:419-489; wire.rs:515 | yes |
| Text | Attack option text (Savage Attacker, Graze) | strings | wire.rs:196-210, 523-531 | yes |
| Text | Courier private clue (prepared recording, bound into job digests) | string | courier_ai.rs:20-23 | yes |
| Text | Authored provenance blurbs `SOURCE`, `CONTENT` (hashed into pins) | strings | model.rs:19-20 | yes |
| Assets | Scene asset paths (mara-harbor-v4.png; inn webp), allowlisted served assets and size caps | paths | wire.rs:697; journey.rs:565-570; public_assets.rs:23-26; startup_configuration.rs:15-40 | yes |
| Policy | Narrative budget policy (`NarrativeBudgetPolicy`: max 8, no tariffs) | struct literal | narrative_transition.rs:114-125 | yes |
| Policy | Combat utility policy: no weights (`criteria: &[]`, `contributions: &[]`); first legal target | empty | enemy_tactics.rs:339-360 | yes |
| Policy | Reaction policy (categorical relationship change; `ReactionPolicy::new`) | literal entries | journey/courier_reaction.rs:466-476 | yes |
| Policy | `PacingPolicy` (df-experience), `ImpulsePolicy`/`TempoElapsedPolicy` (df-tempo), `ConversationLimits.maximum_window_ticks` (df-interaction) | caller-supplied, no `Default` | df-experience/src/pacing.rs:13; df-tempo/src/impulse.rs:29, elapsed.rs:13 | test-only (`tempo_clock` is `#[cfg(test)]`, gameplay.rs:23-24) |
| Limits | Capacity bounds only: `CandidateLimits`, `UtilityLimits`, `ParticipantLimits {3,3}`, `ObjectiveProposalLimits`, `ProgressLimits {512,1,1 MiB}`, `BeatSelectionLimits`, `ReactionLimits`, `DirectorLimits`, `CommandEntryLimits {32,128,8192}/1 MiB`, checkpoint limits `{512,4096,65536,1 MiB}` | inline struct literals, repeated per call site | journey.rs:1373-1389, 2020-2027; model.rs:174-181; narrative_transition.rs:245-256 | yes |
| Ops | Retry 8 attempts/30 s/250 ms backoff; 5 s RPC waits; 2/5 s DB deadlines; message/pending caps | consts | actor.rs:227-228, 263; room.rs:298; dialogue.rs:17-20, 88; gameplay.rs:348-355 | yes |

How policies are constructed today: there is no `Default` impl in any of the eight df-* crates. Every `*Limits`/`*Policy` is an inline struct literal at the call site (often copied 2-3 times); only the `df-tools` literals above and the `df-rules` literals carry gameplay numbers. `df-combat`, `df-encounter`, `df-narrative`, `df-engine` expose capacity bounds only, not balance knobs. `df-experience` and `df-tempo` have real tuning structs (`PacingPolicy.rest_after/lower_intensity_at/lower_after_fatigue`; `ImpulsePolicy` rates) but nothing in the running demo constructs them.

`df-content` today: `catalog.rs` (`CatalogSnapshot::from_published`, caller-supplied version, pins, opaque bytes, `CatalogLimits`), `import.rs` (`validate_import`, bounded draft validation, no decoding), `graph.rs`, `narrative.rs` (`NarrativeBudgetPolicy<Reference>`). It has no `serde`, no decoder, no data files and no loader: it validates structure of bytes someone else decoded. It cannot load this pack; the loader therefore lives in `df-tools` (D01: df-tools owns the author/package workflow).

## 3. The tuning surface

Apply points: **new run** = value is materialized into checkpoint state or pins at run start (changing it requires a new run); **boundary** = adoptable at a safe boundary (section 6) when not yet materialized; **live** = none in v1. "Std" = departs from SRD 5.2.1 when changed (flag -> custom ruleset). Defaults equal today's values.

| Group | Owner (reads value) | File | JSON key path | Type and bounds | Default | Std | Apply |
|---|---|---|---|---|---|---|---|
| Hero AC | `rules::validate_character`, `journey::resources` | tuning | `hero.armor_class` | u8 10..=30 | 17 | yes | new run (before 1st hero) |
| Hero proficiency | `validate_character`, `journey::attack` | tuning | `hero.proficiency_bonus` | u8 2..=6 | 2 | yes | new run |
| Hero base HP | `validate_character`, `resources`, `character_sheet` | tuning | `hero.base_hit_points` | u8 1..=60 | 11 | yes | new run |
| Ability arrays | `validate_character`, `wire::creation_offer` | tuning | `hero.ability_arrays[].scores` | 6 x u8 3..=18; post-bonus <= 20; ids unique; ids must exist in pack `creation.arrays` | stalwart, vanguard | yes | new run |
| Background bonus | `validate_character` | tuning | `hero.background_bonus.{strength,constitution}` | u8 0..=4 | 2, 1 | yes | new run |
| Second Wind | `rules::heal_second_wind`, `complete_no_hit_dice_short_rest`, `resources` | tuning | `hero.second_wind.{uses_max,die_sides,flat_bonus,recharge_per_short_rest}` | u8 0..=5; die in {4,6,8,10,12}; 0..=10; 0..=uses_max | 2, 10, 1, 1 | yes | uses_max new run; rest boundary |
| Hero weapon | `rules::resolve_melee`, `journey::attack` | tuning | `hero.weapon.{damage_dice,damage_sides}` | 1..=2; {4,6,8,10,12} | 2, 6 | yes | boundary (not materialized) |
| Bandit stats | `start_combat`, `resources`, `enemy_tactics` | tuning | `bandit.{hit_points,armor_class,abilities.*}` | HP 1..=200; AC 1..=30; scores 1..=20 | 11, 12, 11/12/12/10/10/10 | yes | boundary before Defend; else new run |
| Bandit offence | `journey::attack` | tuning | `bandit.{attack_bonus,damage_modifier,weapon.*}` | -5..=20; 0..=10; as hero weapon | 3, 1, 1d6 | yes | boundary |
| Critical threshold | `resolve_melee` | tuning | `combat.critical_threshold` | u8 15..=20 | 20 | yes | boundary |
| Round length | `journey::advance`, `combat_round` | tuning | `combat.round_ticks` | u32 1..=600 | 6 | yes | boundary (not in Combat) |
| Log depth | `wire::view` | tuning | `combat.combat_log_entries` | u8 1..=32 | 6 | no | boundary |
| Short Rest | `rules::short_rest_due`, `journey` rest paths, `narrative_phase.rs:516` | tuning | `rest.short_rest_seconds` | u32 60..=86400 | 3600 | yes | boundary (before first rest) |
| Narrative budget | `journey::initial`, `native_budget` | tuning | `narrative.intervention_budget` | u8 1..=64 | 8 | no | new run |
| Room code | `journey::ROOM_CODE` users (`wire`) | pack | `room_code` | 1..=32 ASCII alnum | LANTERN | no | new run |
| Creation options | `wire::creation_offer`, `validate_character` | pack | `creation.groups[]`, `creation.pick_lists.*` | ids kebab-case <= 64; each option id must be accepted by compiled validator (v1: subset of today's) | today's | pick lists: yes | new run |
| Equipment | `accept_character` | pack | `equipment[].{item,quantity}` | item in compiled inventory set; qty 1..=99 | today's 13 rows | yes (qty) | new run |
| Bandit display | `journey::name` | pack | `bandit.name` | 1..=48 chars | Dockside Bandit | no | boundary |
| Scene text | `wire::view` | pack | `scenes[].{title,narration,speaker,dialogue,asset}` | <= 1024 chars; asset must be in served allowlist | wire.rs strings | no | boundary |
| Offer labels | `journey::authored_offered` | pack | `offer_labels.<action>` | <= 96 chars; all 10 actions present | journey.rs strings | no | boundary |
| Courier clue | `courier_ai` | pack | `courier_clue` | <= 512 bytes | courier_ai.rs:23 | no | new run |

Standard-sensitivity is computed, not trusted: the loader holds a compiled `SRD_521_BASELINE` (in `df-rules`) and reports every "Std = yes" key whose value differs (`departures`). JSON cannot declare itself standard.

## 4. JSON schema examples

`content/harbor-journey/tuning.json` (every key required; unknown keys, duplicate keys, floats, strings-for-numbers refuse). Ability score order is STR, DEX, CON, INT, WIS, CHA.

```json
{
  "schema": "df-gameplay-tuning/1",
  "pack": "harbor-journey",
  "hero": {
    "armor_class": 17,
    "proficiency_bonus": 2,
    "base_hit_points": 11,
    "ability_arrays": [
      { "id": "stalwart", "scores": [15, 13, 14, 10, 12, 8] },
      { "id": "vanguard", "scores": [15, 14, 13, 8, 10, 12] }
    ],
    "background_bonus": { "strength": 2, "constitution": 1 },
    "second_wind": { "uses_max": 2, "die_sides": 10, "flat_bonus": 1, "recharge_per_short_rest": 1 },
    "weapon": { "item": "greatsword", "damage_dice": 2, "damage_sides": 6 }
  },
  "bandit": {
    "hit_points": 11,
    "armor_class": 12,
    "abilities": { "strength": 11, "dexterity": 12, "constitution": 12, "intelligence": 10, "wisdom": 10, "charisma": 10 },
    "attack_bonus": 3,
    "damage_modifier": 1,
    "weapon": { "item": "scimitar", "damage_dice": 1, "damage_sides": 6 }
  },
  "combat": { "round_ticks": 6, "critical_threshold": 20, "combat_log_entries": 6 },
  "rest": { "short_rest_seconds": 3600 },
  "narrative": { "intervention_budget": 8 }
}
```

`content/harbor-journey/pack.json` (scenario content; same strictness). Pick lists copied in full from `local_journey.rs:40-68`.

```json
{
  "schema": "df-gameplay-pack/1",
  "id": "harbor-journey",
  "locale": "en",
  "room_code": "LANTERN",
  "creation": {
    "description": "Supported adventure builds: Level 1 Dwarf Fighter / Soldier. Choose one option in each group. Other classes and species remain in development; every submitted build is validated on the server.",
    "groups": [
      { "id": "species", "label": "Ancestry", "options": [ { "id": "dwarf", "label": "Dwarf", "description": "Dwarven Toughness, Darkvision, Dwarven Resilience and Stonecunning retained." } ] },
      { "id": "class", "label": "Class", "options": [ { "id": "fighter-1", "label": "Fighter · level 1", "description": "Weapon training, Fighting Style, Second Wind, three masteries." } ] },
      { "id": "background", "label": "Background", "options": [ { "id": "soldier", "label": "Soldier", "description": "Athletics, Intimidation, gaming set and Savage Attacker." } ] },
      { "id": "array", "label": "Ability training", "options": [ { "id": "stalwart", "label": "Stalwart" }, { "id": "vanguard", "label": "Vanguard" } ], "description_template": "STR{0} DEX{1} CON{2} INT{3} WIS{4} CHA{5} · Soldier +{bonus_str}STR/+{bonus_con}CON" },
      { "id": "equipment", "label": "Starting equipment", "options": [ { "id": "fighter-a-soldier-a", "label": "Fighter A + Soldier A", "description": "Chain Mail, Greatsword, Flail, 8 Javelins, Dungeoneer's Pack; Spear, Shortbow, 20 arrows, gaming set, Healer's Kit, Quiver, Traveler's Clothes; 18 GP." } ] },
      { "id": "style-masteries", "label": "Fighting Style and masteries", "options": [ { "id": "defense-greatsword-flail-javelin", "label": "Defense · Greatsword / Flail / Javelin", "description": "Defense +1 armored AC. Graze / Sap / Slow masteries. This encounter offers Greatsword only." } ] },
      { "id": "allied-ties", "label": "Initiative tie agreement", "options": [ { "id": "join-order", "label": "Party agrees: earlier join acts first", "description": "Party ties use join order; the GM chooses allies before the Bandit when tied." } ] }
    ],
    "pick_lists": {
      "alignment": ["lawful-good", "neutral-good", "chaotic-good", "lawful-neutral", "neutral", "chaotic-neutral"],
      "language": ["common-sign", "draconic", "dwarvish", "elvish", "giant", "gnomish", "goblin", "halfling", "orc"],
      "skill": ["acrobatics", "animal-handling", "history", "insight", "persuasion", "perception", "survival"],
      "gaming-set": ["dice", "dragonchess", "playing-cards", "three-dragon-ante"]
    }
  },
  "equipment": [
    { "item": "chain-mail", "quantity": 1 }, { "item": "greatsword", "quantity": 1 }, { "item": "flail", "quantity": 1 },
    { "item": "javelin", "quantity": 8 }, { "item": "dungeoneer-pack", "quantity": 1 }, { "item": "spear", "quantity": 1 },
    { "item": "shortbow", "quantity": 1 }, { "item": "arrow", "quantity": 20 }, { "item": "gaming-set", "quantity": 1 },
    { "item": "healers-kit", "quantity": 1 }, { "item": "quiver", "quantity": 1 }, { "item": "travelers-clothes", "quantity": 1 },
    { "item": "gold-piece", "quantity": 18 }
  ],
  "bandit": { "name": "Dockside Bandit" },
  "offer_labels": {
    "create-character": "Bring your hero to life", "begin-story": "Enter the Lantern Wharf",
    "ask-courier": "Ask about the broken seal", "escort-courier": "Offer to protect the courier",
    "defend-courier": "Stand between the courier and the bandit", "greatsword-attack": "Greatsword strike · nonlethal",
    "second-wind": "Second Wind · bonus action", "end-turn": "End your turn",
    "short-rest": "Short Rest · 1 hour, no Hit Point Dice", "choose-harbor-scene": "Continue with the courier"
  },
  "courier_clue": "The courier quietly tells you: the sealed packet is addressed to Vell at the Harbor Inn. Keep its destination between you.",
  "escort_support_suffix": "The courier now looks to {name} for support on the escort.",
  "scenes": [
    { "key": "room", "title": "Gather at the Lantern Wharf", "narration": "A rain-soaked harbor waits beyond the lanterns. Share the room with your companions, then bring your heroes into the story.", "speaker": "", "dialogue": "", "asset": "assets/ui/scenes/mara-harbor-v4.png" },
    { "key": "opening", "title": "The Broken Seal", "narration": "You reach the Lantern Wharf as a frightened courier slips from the shadows. Behind him, a boot scrapes across the boards.", "speaker": "Courier", "dialogue": "Please. I can't deliver this alone. The seal was broken before the ship reached port.", "asset": "assets/ui/scenes/mara-harbor-v4.png" },
    { "key": "dialogue-seal", "title": "A Whisper in the Rain", "narration": "The courier presses the packet against his coat. A dockside enforcer approaches, one hand on a scimitar.", "speaker": "Courier", "dialogue": "The crest belongs to a ship that never appears on the ledger. I think someone aboard followed me.", "asset": "assets/ui/scenes/mara-harbor-v4.png" },
    { "key": "dialogue-escort", "title": "An Oath at the Wharf", "narration": "The courier steadies himself behind you. The approaching enforcer draws a scimitar and demands the packet.", "speaker": "Courier", "dialogue": "If you help me reach the inn, I'll tell Vell what I saw aboard that ship. But he won't let us leave.", "asset": "assets/ui/scenes/mara-harbor-v4.png" },
    { "key": "combat", "title": "Steel Beneath the Lanterns", "narration": "The Bandit is within 5 feet of both heroes. All weapons are wielded openly: no surprise, cover or movement. This encounter uses nonlethal melee attacks.", "speaker": "Bandit", "dialogue": "Hand over the packet. Last warning.", "asset": "assets/ui/scenes/mara-harbor-v4.png" },
    { "key": "inn", "title": "The Harbor Inn", "narration": "Warm lamplight frames the next chapter with your party and the courier. The sealed packet remains unresolved.", "speaker": "Courier", "dialogue": "We'll keep the packet close. Its journey isn't over.", "asset": "assets/concept-art/scene-tavern-barkeep-talk-rain.webp" },
    { "key": "victory", "title": "The Courier's Road", "narration": "The Bandit falls unconscious with 1 HP. The courier lowers his shaking hands. The packet—and its secret—remain with your party. His Short Rest has begun; campaign time has not completed it.", "narration_rested": "The Bandit was defeated. An uninterrupted hour has passed without Hit Point Dice or healing. Knocked-out creatures are awake with 1 HP, still Prone; their held weapons remain dropped. The original victory and the courier's unresolved journey remain.", "speaker": "Courier", "dialogue": "You stood your ground. Come on. There's a light waiting for us at the inn.", "asset": "assets/ui/scenes/mara-harbor-v4.png" },
    { "key": "defeat", "title": "A Costly Encounter", "narration": "Both heroes are unconscious with 1 HP after nonlethal attacks. Their Short Rests have begun; campaign time has not completed them. The Bandit takes the packet and leaves.", "narration_rested": "Both heroes were defeated by nonlethal attacks. An uninterrupted hour has passed without Hit Point Dice or healing. They awaken with 1 HP, still Prone, and their held weapons remain dropped. That rest does not change the original defeat: the Bandit took the packet and left.", "speaker": "", "dialogue": "", "asset": "assets/ui/scenes/mara-harbor-v4.png" }
  ]
}
```

Cross-file rules enforced at load: every `ability_arrays[].id` has a pack `array` option and vice versa; `weapon.item` values exist in the compiled item set; every `scenes[].asset` is in the served-asset allowlist (`public_assets.rs:23-26`; adding a new asset still needs a Rust change); all 10 offer labels and all 8 scene keys present; description templates reference only defined placeholders. The "Inspect Seal DC 15" is deliberately absent (section 10, objection 4).

## 5. Loading and validation design

Parsing approach (verified offline, not built): the registry at `/Users/earlcameron/Desktop/dungeonflux/artifacts/cache/cargo/registry/src/index.crates.io-1949cf8c6b5b557f/` contains `serde-1.0.229` (with optional `derive` feature), `serde_derive-1.0.229`, `serde_json-1.0.151`, `serde_path_to_error-0.1.20` (already in the lock via axum), plus `proc-macro2-1.0.107`, `quote-1.0.47`, `syn-3.0.6`. `Cargo.lock` already contains a `serde_derive` entry (reached via `serde_core`). `df-providers` already depends on `serde =1.0.229` and `serde_json =1.0.151` (non-wasm). Recommendation: in `df-tools`'s existing `cfg(not(target_arch = "wasm32"))` table add `serde = { version = "=1.0.229", features = ["derive"] }`, `serde_json = "=1.0.151"`, `serde_path_to_error = "=0.1.20"`; derive `Deserialize` with `#[serde(deny_unknown_fields)]` on every struct. Enabling `derive` adds only the edge `serde -> serde_derive` to `Cargo.lock`; all sources are already vendored in the offline registry. PLAY-B02 step 1 must confirm with `cargo build --offline` before anything else (I did not run cargo). Fallback if that fails: hand-written extraction from `serde_json::Value` (no derive), at the cost of more code and no JSON path in errors.

Where it lives: new module `df-tools/gameplay/pack.rs` (native only; `df-tools` is also a wasm cdylib, so keep it out of the wasm graph, as the other server modules are). Plain typed rules structs live in `df-rules` (`CreationRules`, `CombatRules`, `SecondWindRules`, `RestRules`, constructor `srd_521()` equal to today's literals, no serde) so the rules crate gains parameters, not a data format. The pack/text structs stay in `df-tools`.

Typed pieces:
- `GameplayPack { pack: PackFile, tuning: TuningFile, pack_sha256: [u8;32], tuning_sha256: [u8;32], departures: Vec<&'static str> }`, built only by `GameplayPack::load(dir: &Path) -> Result<Self, PackError>`. Read each file once with a size cap (256 KiB), hash the exact bytes, parse those same bytes (no re-read).
- `PackError`: `Missing { file }`, `Io { file, kind }`, `TooLarge { file, bytes }`, `Parse { file, path, line, column, message }` (path from `serde_path_to_error`; covers unknown key, duplicate key, wrong type), `Schema { file, found }`, `Bounds { file, path, actual, min, max }`, `CrossRef { file, path, reason }`. `Display` names file and JSON location, never file contents. `serve()` maps it to `io::Error::other` and refuses startup before opening the database.
- Directory: `DF_GAMEPLAY_PACK_DIR`, default `content/harbor-journey`, resolved to an absolute path at start and logged.

How values replace constants (each becomes a parameter, no literal left; the parity test in step 1 plus a source audit test grep for the old literals):
- `rules::validate_character(&CreationRules, name, choices)` (arrays, bonuses, HP, AC, proficiency, Second Wind uses; option ids come from the pack lists).
- `rules::resolve_melee`: `NormalMeleeAttack` gains `critical_threshold` and `damage_sides` (replaces `die == 20` and `(1..=6)` at local_journey.rs:257, 271); `heal_second_wind(&SecondWindRules, current, maximum, uses)` (replaces `(1..=10)`, `+1`); `short_rest_due(started, tps, short_rest_seconds)`; `complete_no_hit_dice_short_rest` takes `uses_max` and `recharge`.
- `journey::resources(&Tuning)` (HP/AC/ability/second-wind bounds), `start_combat` bandit block, `attack()` (bandit bonus, dice counts/sides, hero proficiency), SecondWind arm (maximum = the hero's real max HP, not the literal 13 at journey.rs:2296), `advance()` and `combat_round()` (6 / 3600), `initial()` (budget), `character_sheet()`, `name()`, `authored_offered()`, `sample_face`/`draw` (allowed sides = {20} union configured dice sides), `enemy_tactics.rs:133` (bandit DEX), `narrative_transition::native_budget`, `narrative_phase.rs:516`, `wire::view`/`creation_offer`, `courier_ai::RESPONSE`, `inn_scene()`.
- Access: `model::pins()` is a zero-argument function called from ~30 non-test sites (journey.rs, room.rs, wire.rs, actor.rs, encounter_admission.rs, objective_outcome.rs...). Step 3 installs the loaded pack in a process `OnceLock<Arc<GameplayPack>>` set in `serve()` before `journey::initial()`; accessors `pack()` / `tuning()` read it. Tests use an embedded copy of the shipped files and a test-only thread-local override so tests can run altered tuning in parallel. Inner functions take `&Tuning` where practical; converting every remaining `model::pins()` caller to an explicit parameter is optional later work.

Revision flow into run/decision records. Existing fields (no new schema):
- `CheckpointPins.rules.catalog_digest` (model.rs:139, today the compiled manifest digest) becomes `SHA256("df-tuning-pin/1" || manifest_digest || tuning_sha256)`.
- `CheckpointPins.content.content_digest` and `package_digest` (model.rs:147-149, today the compiled source digest) become `SHA256("df-pack-pin/1" || compiled_digest || pack_sha256)`.
- `BuildIdentity.configuration` (model.rs:155, `local-gameplay-demo-config-1`) becomes `local-gameplay-demo-config-1:t-<16 hex>:p-<16 hex>` (label charset allows `: -`, max 128).
- `RulesPins.mode`/`ruleset`: no departures -> `Standard2024`, ruleset unchanged (`srd521-journey-subset-2`); any departure -> `RulesMode::DisclosedCustom` (already encodable, checkpoint_codec.rs:1692-1699) and `ruleset = custom-<tuning_sha 12 hex>`. The player-visible `source_revision` strings (`CharacterSheet`, `CombatOutcome`; currently `rules::SOURCE_REVISION`) get `+custom-<12 hex>` appended so no proto change is needed.
- Consequences already present: `validate_resume` (checkpoint.rs:686-725) refuses any pin mismatch; `persistence` restore decodes with `cold.pins()` (local_demo_scope/startup.rs `restore`), so an old database is refused after an edit; `replay_input_hash_contract.rs:387-388` already compares `catalog_digest` and `ruleset`, so replay is automatically bound. Per-decision there is no pin field: `AcceptedDecision` (`source_policy` label, `semantic_output`) is the only per-decision slot and is not used for revisions in v1.
- Telemetry: one structured event `gameplay.pack.loaded` through the shared OTEL path with both SHA-256 values, run id and `departures` (planning/observability.md); no ad hoc logging.

New run: today a run is a database. Session/run ids are constants (model.rs:163-164) and `admit_startup` (startup.rs) treats a database without `df_game`/`df_local_demo` schemas as fresh and otherwise restores or refuses. "Start new run" therefore means point `DF_GAMEPLAY_DEMO_DATABASE` (must match `df_gameplay_demo_[A-Za-z0-9_]*`, startup_configuration.rs) at an empty database. I did not find a script in `development/` that creates these databases; PLAY-B03 must say how the preview provisions one.

## 6. Live adoption design (phase 2)

Prerequisite (objection 1): `df-model/src/transition.rs:61`, `df-session/src/submission.rs:669` and persistence decoding all require `current.pins() == candidate.pins()`. A live change of pins is therefore not an ordinary transition; it needs a new explicit pin-migration transition kind owned by `df-model`/`df-session`/`df-persistence`. That is a shared contract and is gated on the owner/coordinator, not a `df-tools` change.

Safe boundary (all must hold, evaluated by the single serialized actor on the current committed checkpoint):
1. `current.state().pending.is_empty()` (no `PendingResolution`) and `state.intents` has no unfinished job (courier private-answer completion).
2. No uncertain commit or in-flight operation in `DurableOwner` (`actor.rs`); the actor is idle.
3. `journey::phase(current)` is not `Combat` (`encounter.active_turn` is `None`), so no initiative order or HP is mid-round.
4. Materialization rule: values already written into state are never rewritten. The loader computes `TuningDiff` per key; the key groups in section 3 marked "new run" are refused if materialized (any hero exists for hero keys; bandit entity exists for bandit keys; a rest has occurred for rest keys; budget always).
5. Standard -> custom transition refused unless the command carries `disclose_custom = true`.

Command: development-authorised only (server started with `DF_GAMEPLAY_DEV_CONTROLS=1` and host credential; absent -> RPC returns permission_denied). `AdoptTuning { expected_tuning_sha256, expected_pack_sha256, disclose_custom }`. The client never sends bytes: the server re-reads `DF_GAMEPLAY_PACK_DIR`, loads, hashes, diffs against the active revisions and checks the expected values still match the active ones. Commit: one `AcceptedDecision` (`source_policy: gameplay-tuning-adoption-1`, `semantic_output: adopt:<old tuning>:<old pack>:<new tuning>:<new pack>:<departures>`) plus a shared `ContentEvent` fact, idempotent through the existing operation id/receipt, and the migrated pins. Only after commit does the server swap the active pack.

Refused with typed errors (`AdoptionRefused`): `MidResolution`, `CombatActive`, `MaterializedValueChanged { path }`, `RevisionMismatch`, `CustomNotDisclosed`, `LoadFailed(PackError)`, `DevControlsDisabled`. A refusal changes nothing and keeps the old revision playing.

## 7. Deliberately not tunable

- Rule handler logic and any new mechanic (new checks, spells, reactions, conditions): needs reviewed Rust (generated-content.md). Data cannot add a DC to an action, a weapon property (Graze, Sap, Slow) or a monster action.
- Phase graph, beat prerequisites, thread table, receipt/admission validators (`narrative_phase`, `objective_outcome`, `encounter_outcome`): they are the replay/integrity checks; making them data would make the validators self-certifying.
- Party size, member/entity ids, positions/adjacency, initiative tie rule: structural to the admitted profile (three-actor initiative is checked in `df-engine`).
- Knockout semantics (nonlethal, 1 HP, unconscious/prone/dropped weapon), no-Hit-Dice rest, `RulesMode`, `SOURCE`/`CONTENT` provenance strings: define which ruleset the run claims.
- Capacity bounds and ops timeouts/retries (all `*Limits`, caps in dialogue/actor/gameplay.rs): safety limits, not balance. Tuning cannot raise a bound to unsafe values.
- `df-experience`, `df-tempo`, `df-interaction` conversation/speech, `df-combat` utility weights: no live caller yet. Add keys only when a caller exists (unknown keys refuse, so absence is explicit).
- Dice source integrity: tuning never supplies roll results. Scripted/seeded dice are a separate dev control that marks the run (section 9).

## 8. Implementation plan

Ordered; "||" = may run in parallel (disjoint files). Cargo steps use `--offline`; coding-style.md and `rustfmt.toml` apply; each step ends with affected native/WASM lint gates.

1. **Loader + files** (df-tools only): `crates/df-tools/Cargo.toml`, `Cargo.lock`, `crates/df-tools/src/gameplay.rs` (one `mod pack;` line), new `gameplay/pack.rs`, `gameplay/pack_tests.rs`, new `content/harbor-journey/pack.json`, `tuning.json`. Tests: shipped files load; each error variant (unknown key, duplicate key, wrong type, out of bounds, missing file, oversize, bad cross-ref) names file and path; `departures` empty for shipped files; SHA-256 equals independent digest. No behaviour change. First act: `cargo build --offline` proves the serde derive claim.
2. **Parameterize rules** || with 1 (only `crates/df-rules/src/local_journey.rs`): add `CreationRules`, `CombatRules`, `SecondWindRules`, `RestRules`, `srd_521()`, thread through `validate_character`, `resolve_melee`, `heal_second_wind`, `short_rest_due`, `complete_no_hit_dice_short_rest`; add `SRD_521_BASELINE`; update df-rules tests. Callers in df-tools pass `srd_521()` for now so the tree builds.
3. **Wire numbers into the demo** (after 1 and 2): `df-tools/gameplay/model.rs` (pins), `journey.rs`, `journey/enemy_tactics.rs`, `journey/narrative_transition.rs`, `journey/narrative_phase.rs`, `gameplay.rs` (`serve()`: load, install, refuse on `PackError`). Parity test: full journey (create, defend, attack, second wind, rest, inn) is byte-identical to today with shipped files. Mutation test: raise bandit HP, lower hero AC, change greatsword to 2d8 in a test pack and play the journey. **Deliverable: edit bandit HP / AC / weapon dice / Second Wind / rest length in `tuning.json`, restart against a fresh database, see it in the character sheet and combat log.**
4. **Text and options to pack** (after 1): 4a `wire.rs` + `courier_ai.rs` (|| with 3: different files); 4b `journey.rs` labels, `character_sheet`, `inn_scene`, `name` (after 3: same file). Edit scene text/labels/equipment, restart, see it.
5. **Revision binding and custom flag** (after 3): `model.rs` pins as in section 5, `departures` -> `DisclosedCustom` + visible `+custom-` suffix, `gameplay.pack.loaded` OTEL event, clearer refusal when a stored run's pins differ (message names stored vs file revisions and says to use a new database). Tests: pin differs per file edit; departure flips mode; unchanged bytes reproduce the same pins.
6. **Operator ergonomics** (|| with 5; `startup_configuration.rs`, docs): resolve and print the pack directory and both revisions at start.
7. **Live adoption** (blocked on objection 1): pin-migration transition in df-model/df-session/df-persistence, then `AdoptTuning` in `actor.rs`/`wire.rs`/protocol. Separate brief; not in PLAY-B02/B03.
8. **Dev controls** (later task): show revisions/values, new run, scripted/seeded dice via the existing single supplier `journey::stage_with_supplier` (journey.rs:2449), checkpoint jump to a phase.

Steps 1-3 deliver the "edit, restart, see change" loop for combat/rest numbers. Pure text edits need 4.

## 9. Open choices for the owner

1. **Where revisions are bound.** (a) `CheckpointPins` digests (recommended): strongest no-retcon guarantee and automatic replay binding, but live adoption needs the pin-migration contract. (b) A committed fact per adoption: live adoption becomes ordinary, but restart does not guard against a silent file change. Recommend (a) for phase 1 and decide live adoption after the contract exists.
2. **Dependency approach.** Recommend serde derive plus `serde_path_to_error` (error paths are a stated requirement); this adds `serde_derive` and its proc-macro chain to the audited graph, so `development/dependency-audit/` must be regenerated and checked against `dependency-license-policy.md`.
3. **Departures allowed?** Recommend: allowed locally with the custom flag and visible suffix; hosted/remote rooms require explicit host opt-in and participant disclosure (generated-content.md), so refuse `DisclosedCustom` loads unless `DF_GAMEPLAY_ALLOW_CUSTOM=1`.
4. **Missing directory fallback.** Recommend no silent fallback (decision 2); tests embed the shipped files, the server never does.
5. **Scripted/seeded dice.** Recommend a dev-only run marker (fact plus visible banner) so such a run can never be mistaken for fair play.
6. **Second Wind cap bug.** The literal 13 at journey.rs:2296 equals today's max for both arrays only by coincidence of CON modifiers. Recommend deriving it from the hero's real max in step 3 (behaviour-neutral by default).
7. **Added knobs not in code today** (round cap, rewards/XP, morale, DC-bearing checks). These are features needing Rust and rules review, not tuning. Recommend a separate design task.
8. **Locale.** Pack text is `en` only; translation belongs to df-locale and is out of scope.

## 10. Objections and risks

1. **Decision 4(b) is not workable as stated.** Transitions, session commit and persistence restore all require identical pins, so a live revision change cannot be a logged decision inside today's contract. It needs a new pin-migration transition owned by df-model/df-session/df-persistence (section 6). Phase 1 (decision 4a) is unaffected.
2. **"Edit, restart, see change" does not work on an existing database.** Binding the revision into pins (decision 3) makes `admit_startup` refuse the old run, and there is no in-process new-run mechanism (constant session/run ids, database = run). The loop needs `DF_GAMEPLAY_DEMO_DATABASE` pointed at an empty `df_gameplay_demo_*` database (or a drop/recreate helper).
3. **D01 conflict risk.** D01 rejects a draft/file digest as publication identity. Name these "pack revision" and "tuning revision" (development revisions) everywhere; never present them as a published ContentPack version or `RightsGrant`-backed identity. When a real ContentPack lands, `pack.json` becomes a payload and the digest is replaced by the published version handle.
4. **No DC exists in the live loop.** The only DC (15) belongs to the legacy Inspect Seal path, unreachable from the room and outside `wire::action_kind`. "Tune a DC" is not deliverable without adding a check-bearing action, which is new Rust handler work. v1 tunes attack/AC/damage/HP/Second Wind/rest instead.
5. **Duplicated literals fail at runtime, not compile time.** The same number lives in several validators (bandit HP/AC in `resources()` and `start_combat`; DEX 12 in `enemy_tactics.rs:133`; 6/3600 in `advance`, `combat_round`, `narrative_phase.rs:516`, `short_rest_due`; 13 in three places). A missed site shows up as `InvalidCandidate` mid-game. Mitigation: the mutation tests in step 3 and a source audit test forbidding the old literals.
6. **Global access to `model::pins()`.** ~30 callers rely on a zero-argument function; step 3 uses a `OnceLock` plus a test-only override. Tests that mutate tuning must not share process state with default-tuning tests.
7. **Wasm graph.** `df-tools` also builds as a wasm cdylib; keep all new dependencies and `pack.rs` under `cfg(not(target_arch = "wasm32"))` and add a wasm compile check.
8. **Cargo.lock change.** Enabling serde `derive` edits `Cargo.lock` (offline-resolvable); confirm no unrelated lock churn.
9. **Hash coupling.** Any tuning byte (including whitespace) changes `catalog_digest` and strands stored runs, even for non-rules keys like log depth. Accepted; it is exactly the "edited file is a new revision" rule, but editors should expect it.
