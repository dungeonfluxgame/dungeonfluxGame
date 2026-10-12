# Hero tables (PLAY-C08)

Data for the backstory-driven hero creator designed in
[planning/character-backstory-creation.md](../../planning/character-backstory-creation.md)
(sections 4, 5.2, 6, 7 and the owner decisions in 11). Nothing here is code, and nothing
here is a rule: the files are content that a later task (picker, validator, loader) reads.
Authored 2026-10-12, revision 1. All JSON is UTF-8, two-space indented, stable key order,
trailing newline.

The story proposes, the rules decide, the player confirms. Every option card can only spend
a free choice of the standard 2024 build: background, class, ability order, skills,
languages, alignment, kit, fighting style (and masteries, expertise, divine or primal
order). No card grants a feat, feature, item, coin, score, ally or title. Claims stay
claims.

## Files

| File | What it is | Source tag |
|---|---|---|
| `life-path.json` | The eight stages: 96 option cards, roll-for-me tables, follow-up questions, the closed tag list and hero-only dossier-shaped rows | `custom-v1` |
| `jobs.json` | 6 job families, 34 trades (16 shared with the NPC dossier, 18 hero-only), 12 level-3 callings | `custom-v1` (callings point at `srd-5.2.1` subclasses) |
| `backgrounds-custom-v1.json` | 12 original backgrounds on the 2024 background template | `custom-v1` |
| `catalog-srd.json` | SRD 5.2.1 lists the other files must resolve against (not a rules implementation) | `srd-5.2.1` |
| `sources.json` | The two declared sources, with the CC BY 4.0 licence line and attribution text | n/a |

`catalog-srd.json` was transcribed from `development/rules-sources/srd-5.2.1-en.pdf`
(printed page equals PDF page): skills p.9; creation steps, standard languages and the
standard array pp.19-22 (languages p.20); classes pp.28-82 with subclasses at 30, 35, 40,
46, 49, 52, 56, 61, 64, 69, 76, 82; the four backgrounds p.83; species pp.84-86; feats
p.87-88; Divine Order p.37; Primal Order p.42; weapons and armor pp.91-92; tools pp.93-95;
adventuring gear, ammunition and packs pp.95-98. Costs are in copper pieces.

## Schemas (short)

### life-path.json

- Top level: `schema` (`df-hero-tables/1`), `file`, `revision`, `source`, `description`,
  `signal_notation` (legend), `story_tags[{id, kind: location|theme, meaning}]`,
  `hero_rows`, `stages[]`, `follow_ups[]`.
- `hero_rows`: `origins`, `formative_events`, `wants`, `bonds`. Rows in the NPC dossier shape
  (`id`, optional `speakers`, `text`, optional `text_restrained`) for hero-only ids. Ids never
  collide with a dossier id. Not listed in `tables.json`, so adventure revisions do not move.
- `stage`: `id`, `order` (1-8), `title`, `question`, `fills{field, tier}` (`tier` is
  `per-slot` for stages 7 and 8), `fast_path` (stages 3, 5, 6 are the three-tap path),
  `skippable` (skipping rolls), `pick_slots{min, max}`, `slots[]`; stage 5 adds `shown_first`
  (6) and `show_all_label`; stage 7 adds `slot_roll` (a d4 chooses the kind; on a 5 or 6 of a
  d6 roll again and keep a different kind).
- `slot`: `id`, `prompt`, optional `kind`, `tier`, `required`, `roll{die, map[{faces, option}]}`,
  `options[]`. Stages 1-6 have one slot (`main`); stage 7 has `person`, `place`, `debt`,
  `enemy`; stage 8 has `flaw`, `secret`, `fear` (fear optional).
- `roll`: the dossier face rule. `die` is the smallest of 4, 6, 8, 10, 12, 20 with
  n <= die; faces are contiguous, spread evenly, the first (die mod n) entries taking one
  extra; every option appears once. Coherence rejection and rerolls are code, not data.
- `card` (an option): `id` (kebab-case, unique across the file), `title` (<= 40 chars),
  `text` (second person, < 220 chars, mature wording and the default), optional
  `text_restrained` (shown instead when the table rating is restrained), optional
  `dossier{file, table, ids[], hero_only[]}` (dossier ids the card can roll; `hero_only` ids
  live in `hero_rows`), `signals`, `fills{field, tier}`, `source`, plus the optional
  stage-specific fields below.
- `signals`: `background{acolyte|criminal|sage|soldier: 1-3}`, `class{class id: 1-3}`,
  `species{species id: 1-2}`, `ability{str..cha: 1-2}`, `skills{skill id: 1-2}`,
  `languages{standard language id: 1-2}`, `tools{gaming-set variant item id: 1-2}`,
  `alignment{law|chaos|good: 1-2}` (evil is never produced), `tags[]` (from `story_tags`).
- Optional card fields: `tie_handle` (stage 2: family, community, mentor, comrades, troupe,
  wild, guild, foster; stage 7: `tie.person|place|debt|enemy`), `bond_lean`, `ideal_lean`,
  `flaw_lean`, `fear_lean`, `secret_lean` (dossier or card ids), `weak` and `follow_up`
  (stage 3), `featured` and `level1{fighting_style, weapon_masteries, expertise,
  divine_order, primal_order}` and `ability_alt` and `kit_hint` (stage 5), `tie_kind`,
  `direction`, `creditor`, `adults_only`, `hook{cast, kinds, stake_tags, location_tags,
  binding}` (stage 7, matching hook kinds, stake tags and location tags of the adventure
  tables), `secret_kind`, `secret_terms` (words to withhold from every public slot),
  `known_by` (`none`), `creates_claim{kind, stance, tier}` (the birth-belief card).
- `follow_up`: `id`, `question`, `when`, `applies_to`, `skippable`,
  `counts_toward_free_text_limit` (design 5.3: three per free-text session), optional
  `default`, and `options[{id, label, effect}]` or `options_from{stage, filter}`. The six:
  `work-taught-you` (discipline, cunning, faith, lore map to the four carriers),
  `which-fighter`, `reach` (the four SRD fighting styles), `order`, `claim-stance`,
  `spotlight-consent` (default `not-yet`).

### jobs.json

- `families[]`: `id` (equals the stage-3 card id), `label`, `card`, `carrier_weights`,
  `weak`, optional `follow_up`, `summary`.
- `trades[]`: `id`, `label`, `family`, `dossier_past_trade` (id equal to the trade id, for the
  16 person trades of the dossier) or `phrase` (hero-only), `srd_background` (the carrier
  that supplies mechanics today), `custom_background` (id in `backgrounds-custom-v1.json` or
  `null`; used instead of the carrier when the table enables the `custom-v1` pack),
  `skill_leans` (1-3 skills, weight 1, colour for the class's own picks, never a
  proficiency), optional `tool_leans` (Soldier gaming-set variants only), `source`.
  Craft-and-field trades carry a provisional `srd_background`; the `work-taught-you`
  answer overrides it.
- `callings[]`: `class`, `subclass_id`, `subclass_name` (the one SRD subclass), `tag`,
  `line` (design 4.9 wording), `sheet_line`, `leaning` (a leaning, not a choice),
  `playable_choice` (false), `subclass_source`, `source`. `reserved_calling_tags[]` are
  inactive tags for licensed subclasses (`subclass_id` null, `shown` false).

### backgrounds-custom-v1.json

`backgrounds[]`: `id`, `name`, `family`, `description`, `ability_scores` (three distinct),
`origin_feat{id[, list]}` (an SRD Origin feat: Alert, Magic Initiate with a list, Savage
Attacker or Skilled), `skill_proficiencies` (two distinct), `tool_proficiency{kind:
specific, id[, variant]}` or `{kind: choice, category}`, `equipment{a{items[{item, qty}],
gp}, b_gp}` (A is worth exactly 50 GP at SRD prices, B is 50 GP), `carries_trades[]`,
`source: "custom-v1"`, `disclosure` (the line a table sees). The key list is closed: any
extra key (a feature, a perk) is a defect.

### catalog-srd.json and sources.json

Catalog keys: `abilities`, `alignments`, `skills`, `classes` (with `skill_choose`,
`skill_list`, `subclass`), `subclasses` (one per class, level 3), `species`, `backgrounds`
(the four SRD backgrounds in the same shape as the custom ones), `origin_feats`,
`fighting_style_feats`, `class_choices`, `tools`, `tool_variants`, `languages`
(standard with d12 faces, rare), `free_choices`, `items`. Sources: `id`, `kind`, `name`,
`version`, `licence{id, name, url, line}`, `attribution`, `disclosure`, `used_by`; the
custom source also has `date` (2026-10-12), `mechanics_added` (false) and
`may_be_switched_off_per_table`.

## How a later validator should check it

1. Parse each file as UTF-8 JSON; require two-space indentation and a trailing newline.
2. Every `source` value (documents, cards, trades, callings, backgrounds, SRD backgrounds)
   is a declared id in `sources.json`. A hero's record keeps the tags of the sources it used.
3. Ids unique: per list in the catalog, per slot and globally for cards, trades, custom
   backgrounds, follow-ups, story tags.
4. Cards: at least six per slot; die and face ranges follow the face rule; `text` and
   `text_restrained` under 220 characters; second person; `text_restrained` (or `text`
   where there is no variant) free of the restrained-rating refused words; ids kebab-case;
   `fills.field` matches the stage and `fills.tier` the slot; unknown keys refused.
5. Signals resolve: background to the four SRD ids, class, species, skill, language, tool,
   ability and tag ids to the catalog or `story_tags`; weights within range; alignment
   axes only law, chaos, good.
6. Dossier references resolve to ids in `content/adventure-tables/npc-background.json` or
   `npc-character.json` (or to `hero_rows`); the 14 person origins and 15 person formative
   events each appear exactly once in stages 1 and 4; goblinoid rows are never used; all 16
   person past trades appear as trades; every `hero_rows` id is used by a card.
7. `level1` values resolve (fighting-style feats, weapon items, skills, class choices);
   `hook` kinds, stake tags and location tags are those of `hooks.json`, `macguffins.json`
   and `tables.json` `compat_tags`.
8. No dead options: every card declares a use (signals, a tie handle or hook, a dossier id,
   a lean, a secret kind, a follow-up). No dead tags. Text must not read as a grant.
9. Jobs: each family equals a stage-3 card with equal carrier weights; `weak` matches;
   `custom_background` agrees both ways with `carries_trades`; skill leans resolve; one
   calling per class pointing at that class's one SRD subclass.
10. Custom backgrounds: exactly the template keys; three distinct abilities; an SRD Origin
    feat; two distinct skills; a tool that exists in the SRD (with a valid variant); every
    item in the SRD item list; option A worth exactly 5000 copper pieces; B is 50 GP; the
    disclosure string is present; no name, skill pair, feat-plus-skill or feat-plus-tool
    combination equals a published background (the published non-SRD list is not in this
    repository, so that check needs the licensed reference or a reviewer).

## What each stage can and cannot decide

| Stage | Can decide (free choices it may feed) | Cannot decide |
|---|---|---|
| 1 Origin | Origin line; a weak species lean (only for an unchosen species tile); the two free languages; ability emphasis; a campaign location tag | Class; background beyond a 1-point nudge; any score; kit; a species the player has already chosen |
| 2 Upbringing | Background and class nudges; alignment points; the tie handle; a bond lean | Class (only tips a tie); a relationship to a campaign NPC; an item or gold |
| 3 Job | The job label; the carrier background (or the custom background); colour skill leans for the class's picks; a gaming-set lean | Class; any skill, tool or feat beyond what the background gives; equipment beyond the background package |
| 4 Event | The event line; alignment points; ideal, bond, flaw, fear and secret leans; one-point ability, skill and class nudges; story tags | A feat, scar, power, item, title or injury; any penalty |
| 5 Learned | Class (decisive); fighting style, weapon masteries, Expertise, Divine or Primal Order; ability order; ranged-or-melee kit hint; the calling | Subclass (only a leaning for level 3); spells; level; feats other than the Fighting Style; features the class does not grant |
| 6 Want | The public want; alignment points; ideal lean; hook tags | Anything mechanical besides alignment |
| 7 Ties | Hook handles (person, place, debt, enemy) and location tags for the generator; private or public tie lines | Allies, standing, gold, items, canon NPCs, anything the party receives |
| 8 Flaw, secret, fear | A story flaw (a gambler may lean to dice or cards), a sealed secret with withheld terms, a private fear; a claim recorded as the hero's belief | Penalties, bonuses, Disadvantage, the truth of a claim, a revealed secret (only the player's Reveal action opens one) |

## Custom content and licensing

`custom-v1` is disclosed, versioned custom content under
[planning/generated-content.md](../../planning/generated-content.md). It adds no rule
and no mechanic; each background is built on the SRD template, so a table that disables the
pack loses variety, not compatibility. Nothing here is copied from, or re-skinned from, a
Player's Handbook background or any other non-SRD book. A licensed pack, if the owner ever
holds a written licence recorded as a reviewed RightsGrant, would be a new source tag and
new files; it does not change these.
