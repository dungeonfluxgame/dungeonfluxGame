# Tactical combat — PLAY-T01 attempt 1

Pure library milestone on base `63484e991213266767f04a72350f19bf7cd6274f`.
The running journey is deliberately unchanged. This is a supported SRD subset,
not completion of standard 2024 rules support.

## Governing ownership and source

This implements adventure-generator §17 items 3,4,6,7 under rules-support,
rules-coverage R01/R04–R09/R12/R15, rules-effect-model's staged/rejected-draw
contract, subsystem-architecture's server crate map and subsystem-interfaces'
Rules/Transition and director ownership. Coding-style and observability apply.
`df-rules` owns positions, paths, initiative, resources, conditions, mechanical
legality and outcomes. `df-combat` proposes tactics and reuses `candidates` and
`ranking`. `df-encounter` already owns canonical participant/challenge admission;
it is unchanged because this pure slice does not invent canonical encounter IDs,
source rights, objective definitions or checkpoint mapping. The follow-up must
use its existing participant/challenge owners when preparing real encounters.
The existing generic `MovementOwner` admits canonical supplied paths but does not
calculate movement. It remains the integration wrapper for these rules queries;
its existing movement_staging explicitly notes the missing path-bearing command.
No replacement canonical RPC/path envelope is invented here.

Mechanics were checked against the retained English SRD 5.2.1 PDF at
`development/rules-sources/srd-5.2.1-en.pdf`, with manifest in that directory:
SHA256 `8974902d109d6e63672d7c490bde9ccf052410503d9cfa768237154fbc5e3d87`.
PDF page and printed page agree. Offline PDFKit extraction is retained in attempt
evidence; no network source or recollection substitutes for its text.

This work includes material from the System Reference Document 5.2.1 ("SRD 5.2.1")
by Wizards of the Coast LLC, available at https://www.dndbeyond.com/srd. The SRD
5.2.1 is licensed under the Creative Commons Attribution 4.0 International License,
available at https://creativecommons.org/licenses/by/4.0/legalcode.

## Model and public boundary

`BattleMap` and `Battle` are validated, read-only values. `CombatantSpec` is input
creation data; `begin` rejects profiles other than the two existing Fighter builds,
the new archer and the twelve source stat blocks. No HP inflation or configurable
house-rule modifiers are accepted. `ArcherBuild` has a private validated receipt.
Each combatant has one five-foot cell, source Speed, HP/conditions, equipment,
masteries, ammunition and turn budget. Large is explicitly rejected: its future
model needs a 2×2 footprint, occupancy, path sweeps, cover and all footprint reach
queries. Small and Medium both occupy one square.

Core signatures (public accessors omitted):

```rust
BattleMap::from_json(bytes: &[u8], layout: &str) -> Result<BattleMap, RuleError>
BattleMap::from_data(data: &BattleSceneData, layout: &str) -> Result<BattleMap, RuleError>
BattleMap::cover_between(from: Cell, to: Cell, creatures: &[Cell]) -> Result<Cover, RuleError>
begin(map: BattleMap, specs: Vec<CombatantSpec>, binding: DrawBinding<'_>,
      records: &[ActualDraw]) -> Result<Resolution, RuleError>
resolve(battle: &Battle, actor: u32, action: &Action, binding: DrawBinding<'_>,
        records: &[ActualDraw]) -> Result<Resolution, RuleError>
Battle::validate_path(id: u32, path: &[Cell]) -> Result<u16, RuleError>
Battle::path_cost(id: u32, path: &[Cell]) -> Result<u32, RuleError>
Battle::shortest_path(id: u32, goal: Cell, max_cost: Option<u16>) -> Result<Vec<Cell>, RuleError>
Battle::reachable_cells(id: u32) -> Result<Vec<Cell>, RuleError>
Battle::legal_targets(actor: u32, weapon: AttackUse) -> Result<Vec<u32>, RuleError>
Battle::attack_from(actor: u32, origin: Cell, target: u32, weapon: AttackUse) -> Result<(), RuleError>
Battle::attack_cover(actor: u32, target: u32) -> Result<Cover, RuleError>
Battle::dexterity_save_adjustments(actor: u32, origin: Cell) -> Result<(i16, bool, bool), RuleError>
CombatantSpec::creature(id: u32, srd_id: &str, cell: Cell, ammunition: u16)
    -> Result<CombatantSpec, RuleError>
CombatantSpec::melee_hero(character: &LegalCharacter, id: u32, cell: Cell)
    -> Result<CombatantSpec, RuleError>
validate_archer(name: &str, choices: &[(&str, &str)]) -> Result<ArcherBuild, RuleError>
ArcherBuild::combatant(&self, id: u32, cell: Cell) -> CombatantSpec
df_combat::tactical::propose(battle: &Battle, actor: u32) -> Result<Option<Action>, RuleError>
df_combat::tactical::choose(battle: &Battle, actor: u32, context: TacticContext<'_>)
    -> Result<Option<Action>, TacticError>
```

`AttackUse::use_mastery` explicitly opts in to an available unlocked property for this attack; false declines it, without granting any unchosen property. Graze miss damage and Slow speed reduction are therefore optional. Vex/Sap apply their source effects when selected, and Nick spend requires explicit opt-in.

`Resolution` returns the successor battle, exact consumed `ActualDraw` records,
and attack summaries including faces, reasons, effective roll mode, distance,
cover/AC, modifiers, damage types, mastery and knockout/death consequence.
`DrawBinding` uses existing operation/resolution/window/source/ordinal types.
The session supplies recorded outcomes; no RNG exists in Rules. Missing required
dice returns `MissingDraw { sides }`; the caller resumes with the same recorded
prefix. Exact metadata, die size, face, ordinals and absence of unused draws are
validated. All failures preserve the original battle and immutable supplied dice.
Caller source qualification is still required: matching metadata is not catalog
admission, source rights or an authorization grant. Fixture pins are explicitly
synthetic, not purported production source identities.

Movement can pause with ordered optional Opportunity Attack offers before the
provoking departure. Only the first offered reactor may take/decline. A decline
spends nothing; a taken attack spends one reaction, never the ordinary action.
An incapacitated mover cancels the remaining path/offers. Accepted reactions can
reduce movement through Slow, after which the path stops when movement runs out.
The reaction refreshes at the start of the reactor's next turn, not at a universal
round reset. Turn order includes downed actors so their saves/timed effects occur.
EndTurn remains available after defeat for the ongoing death-save aftermath;
DeathSave is offered only once at the actual start of a dying hero's turn.

## Scene-v2 subset and geometry adjudication

The serde subset is exactly:

```json
{"schema":2,"grid":{"cols":16,"rows":9,"blocked":[[0,0]]},
 "layouts":[{"id":"yard","obstacles":[{"id":"crate","kind":"crate",
 "cells":[[7,3]],"blocks_movement":true,"cover":"half","height_m":1.0}],
 "difficult":[[6,3]],"spawns":{"heroes":[[1,2]],"enemies":[[14,2]]}}]}
```

Unknown rendering fields at each level are deliberately consumed with flattened
`IgnoredAny`; they cannot alter mechanics. Renderer metres do not change five-foot
rules squares. IDs, unique cells where meaningful, nonempty finite nonnegative
heights, dimensions, coordinates and all layouts are checked. Spawns cannot
coincide/occupy blocked cells, and all must be mutually reachable through geometry.
Obstacle IDs are unique within each layout; layout IDs are unique across a scene.
Overlapping different obstacles use the strongest authored grade. Bounds: 1MiB
JSON,128×128 cells,32 layouts,64 combatants,1024 requested path steps,128 draws per
resolution. Dijkstra search covers the admitted finite map with cell-order ties.
Blocked prevents entry; it does not imply opaque cover. Authors must provide cover
grades for walls/trees; difficult terrain has no intrinsic cover grade.

**Disclosed custom departure `tactical.cover-centre-line`:** SRD "Cover" (p15)
says the attack originates on the opposite side and describes fraction-of-body
coverage, but supplies no corner-ray or grid-line algorithm. This slice uses an
exact rational centre-to-centre segment intersecting the open interior of an
obstacle cell. Pure corner tangency does not intersect. The firing source's own
cell is excluded; an obstacle intersecting the target cell applies its authored
grade. Intervening living creatures (including unconscious creatures) provide
Half Cover. Obstacle grades and creature coverage use the strongest degree,
without stacking. Height is validated authoring metadata, not a 3D occlusion
calculation. This is a deterministic tabletop geometry adjudication, not a claimed
SRD line formula; expose `DEPARTURES` exactly as the loader exposes custom departures.
Do not present this geometry as undisclosed Standard2024 policy. Integration must
admit its scoped ruling/custom policy explicitly or supply a reviewed source-safe
cover owner. There is no alternate diagonal cost departure: every adjacent step,
including diagonals, costs five feet per the SRD p13 default grid rule.

The currently checked-in `content/battle-scenes` files are schema1; another task
owns their schema2 authoring. This parser refuses schema1 instead of inventing
layouts. Three crate fixture files cover open court, broken yard and wolf trail.
The open court also declares a distant spawn used for the skeleton simulation;
DM-selected initial positions are allowed by "Establish Positions" (p13).

## Source clauses and executable evidence

Tests below are in `df-rules::tactical::tests`, except names prefixed simulator/
policy, which are public-boundary tests in df-combat/tests/tactical_simulator.rs.

| Source section (SRD page) | Implemented behaviour | Proving test |
| --- | --- | --- |
| Playing on a Grid; Movement and Position; Moving around Other Creatures (13–14) | Five-foot diagonals, filled corners prohibited, difficult entry10ft, ally passage normal, incapacitated hostile passage difficult, occupied end refused, split movement, validated shortest paths/reachable cells | movement_diagonals_difficult_allies_hostiles_split_move_and_shortest_path; geometry_corners_cover_adjacent_shots_and_intervening_creatures |
| Crawling (179); Prone (186); Dash/Disengage/Dodge (180–181) | Crawl+terrain additive15ft, half-Speed standing rounded down, Dash extra current Speed, action and movement budgets, Dodge sight/speed/incapacitation and Dexterity-save benefit | standing_crawling_dash_and_disengage_use_source_budgets; cover_and_dodge_change_recorded_attack_and_dexterity_save_adjustments |
| Opportunity Attacks (15); Reaction (186) | Optional before-exit melee/unarmed attack, sight and reaction gating, Disengage suppression, own-turn refresh, cancellation on knockout | opportunity_window_is_optional_before_exit_and_reaction_refreshes_at_own_turn; opportunity_knockout_cancels_remaining_path_and_multiple_reactors_are_ordered; ranged_holder_opportunity_uses_unarmed_or_an_actually_held_melee_weapon |
| Attack Rolls; Advantage/Disadvantage (7–8); Ranged Attacks; Reach (15) | Natural1miss/20hit, 5ft diagonal melee, normal/long ranges, close hostile who sees/not incapacitated, all signs cancel by presence | ranged_boundaries_ammunition_and_immutable_draw_rejections; advantage_disadvantage_presence_cancels_and_close_hostile_must_see_and_be_capable |
| Cover (15) | Half+2/three-quarters+5 AC and Dex saves; total direct target refusal; strongest only; disclosed geometry | geometry_corners_cover_adjacent_shots_and_intervening_creatures; cover_and_dodge_change_recorded_attack_and_dexterity_save_adjustments |
| Properties and Weapons (89–91); Unarmed Strike (190–191); Shield (92) | Heavy STR13 melee/DEX13 ranged, Two-Handed, Finesse same chosen ability, Versatile only player melee, thrown inventory, ammunition/bolts, shield hand and action to doff; unarmed fixed1+STR | weapon_properties_hands_heavy_finesse_versatile_and_thrown_inventory; ranged_holder_opportunity_uses_unarmed_or_an_actually_held_melee_weapon |
| Light (89); Nick/Vex/Slow/Graze/Sap (90) | Different Light weapon extra once, positive ability damage omitted; Nick keeps bonus action; Vex next attack per target before end next turn; Slow max10 until origin next turn; Sap next attack; Graze on miss | vex_nick_light_and_savage_attacker_once_per_turn; vex_marks_multiple_targets_and_expires_after_next_turn_not_at_start; slow_does_not_stack_and_expires_at_source_next_turn; graze_sap_critical_and_second_wind_preserve_action_budget; policy optional_masteries_can_be_declined_without_changing_source_damage |
| Savage Attacker (87); Critical Hits (16) | Optional weapon dice twice/higher once per turn including ranged; critical dice doubled (including goblin rider) | vex_nick_light_and_savage_attacker_once_per_turn; full_creature_catalog_preserves_actions_and_required_riders |
| Initiative; Your Turn; Ending Combat (13–14) | N actors, one recorded initiative per identical monster group, caller-agreed tie order, actions/bonus/reaction/movement, either-side defeat and rounds | initiative_group_draws_ties_and_invalid_size_or_stats_are_explicit; simulator_four_complete_source_fights_with_recorded_dice |
| Knocking Out; Dropping to0; Death Saving Throws (17–18); Unconscious (191) | Melee optional1HP knockout, dropped held items/Prone, ranged monster death, hero dying, massive damage, at0critical2failures, natural1two failures/20heal1, stable resets, timed aftermath | melee_knockout_ranged_death_massive_damage_and_timed_death_saves; death_at_zero_critical_failures_stable_damage_and_recovery_are_source_correct; postfight_death_save_requires_turn_boundary_and_stable_resets_counters |
| Ammunition (89) | One per ranged shot, living capable collector spends afterfight minute to recover floor(used/2), one recovery only | ranged_boundaries_ammunition_and_immutable_draw_rejections; death_at_zero_critical_failures_stable_damage_and_recovery_are_source_correct; simulator_four_complete_source_fights_with_recorded_dice |
| Fighter; Second Wind; Weapon Mastery (47–48); Soldier/Dwarf (83–84); Archery (87); Armor (92) | Validated source build, three masteries, two SecondWind uses1d10+1, derived AC/HP/attacks | archer_standard_array_background_creation_data_and_derived_sheet_match; graze_sap_critical_and_second_wind_preserve_action_budget |
| Bandit261, Cultist278, Guard296, Goblin290, Kobold302, Skeleton325–326, Zombie343–344, Wolf364, GiantRat353, GiantWeasel355, Mastiff357, Scout322 | Source Speed/actions; PackTactics; goblin Advantage1d4/NimbleDisengage; wolf/mastiff automaticProne; skeleton vulnerability; cultist separate Necrotic1; zombie FortitudeDC5+damage exceptcritical; Scouttwoattacks | full_creature_catalog_preserves_actions_and_required_riders; simulator_four_complete_source_fights_with_recorded_dice; policy skeleton_uses_actual_ranged_stat_block_and_cover_policy_is_legal |

Dwarf Speed is30ft, wolf/mastiff/giant-weasel40ft, zombie20ft, other supported
creatures30ft. Natural bite/slam actions do not require weapon hands, never enter held/dropped inventory, and do not gain player masteries. Stat-block riders and dice are bound to their named actions: Bite for wolf/mastiff Prone and rat/weasel d4, Scimitar/Shortbow for goblin Advantage d4, Ritual Sickle for cultist Necrotic1, Shortsword/Longbow only for Scout Multiattack. Unarmed Strike keeps its ordinary STR+PB attack and fixed1+STR damage, without those weapon-action riders. Ranged
alternatives are BanditLightCrossbow80/320, Goblin/SkeletonShortbow80/320,
GuardSpear20/60, KoboldDagger20/60, ScoutLongbow150/600. Monster equipment quantities
not fixed in the stat block are admitted encounter data; zero ammo removes that
action. Guards/goblins must doff their shield before a two-handed bow/crossbow
attack; doffing takes an action and removes the Shield+2AC. Guard's stat-block
Spear always remains1d6+1, even after doffing: player Versatile is not silently
substituted into a monster action. Kobolds have three Daggers as their Gear states.

Independent boundary regressions additionally prove `tactical_monster_unarmed_attacks_do_not_inherit_named_weapon_actions`, `tactical_scout_multiattack_admits_only_its_named_actions`, `tactical_approach_uses_rules_attackable_endpoints_instead_of_total_cover_target`, `tactical_optional_masteries_can_be_declined_without_changing_source_damage` and `tactical_adjacent_ranged_creature_with_safe_retreat_keeps_distance_and_fires`. The last case reaches an adjacent skeleton turn with20ft movement and a hostile's reaction already spent, then verifies legal withdrawal and its actual +5Shortbow action.

## The ranged hero and species options

`content/hero-builds/dwarf-archer.json` uses the existing creation.groups/options,
pick_lists, hero.ability_arrays, equipment and departures shapes. The bonus adds a
Dexterity field and `{bonus_dex}` template placeholder, which the follow-up must
admit: existing fixed groups and bonus structure cannot plug it in unchanged.
The existing melee options remain unchanged.

SRD Fighter starting option B, p47 (typographic line-break hyphenation normalized):
"(B) Studded Leather Armor, Scimitar, Shortsword, Longbow, 20 Arrows, Quiver,
Dungeoneer's Pack, and 11 GP". There is no shield in this option. Soldier B (p83)
is50GP; this build selects Fighter B + Soldier B, hence20 arrows and61GP, with the
Soldier gaming-set proficiency but no granted physical gaming set. Fighter B +
Soldier A is a different legal package:40 arrows, two Quivers,25GP, plus Spear,
Shortbow, Gaming Set, Healer's Kit and Traveler's Clothes. Do not discard the
background's20 arrows when integrating that alternative.

Standard array assignment STR13 DEX15 CON14 INT10 WIS12 CHA8 is a permutation of
15,14,13,12,10,8 (Character Creation p21). Soldier permits STR/DEX/CON increases:
DEX15+2=17, CON14+1=15, others unchanged. Modifiers are +1,+3,+2,+0,+1,−1.
Fighter1 proficiency+2; Studded Leather12+DEX3=AC15. HP10+CON2+Dwarf1=13.
Archery gives+2 to attacks with Ranged weapons (not thrown melee weapons).

| Weapon | Attack | Damage | Mastery | Properties |
| --- | --- | --- | --- | --- |
| Longbow | DEX3+PB2+Archery2=+7 |1d8+3Piercing;150/600ft |Slow |HeavyDEX≥13,Two-Handed,Ammunition |
| Shortsword (DEX chosen) |DEX3+PB2=+5 |1d6+3Piercing |Vex |Finesse,Light |
| Scimitar (DEX chosen) |DEX3+PB2=+5 |1d6+3Slashing; Light extra attack1d6 |Nick |Finesse,Light |

Soldier's Savage Attacker is once per turn on any weapon hit, including Longbow.
Second Wind is unchanged: two uses, Bonus Action,1d10+1 at level1, one recovered
per Short Rest through existing rest APIs. The hero may be presented as female;
gender grants no rule modifier and does not require a new stat contract.

Human (p86) requires Medium/Small choice, Speed30, extra skill choice, extra Origin
feat and all that feat's supported mechanics, Resourceful granting Heroic
Inspiration after Long Rest, Inspiration reroll/spend/transfer state and recovery.
Remove DwarvenToughness (HP12 for this array), dwarfDarkvision/poisonresilience/
Stonecunning; creation, rest and shared view/persistence must reflect those traits.
This slice cannot honour arbitrary extra Origin feats or Heroic Inspiration yet.

Elf (pp84–85) requires Medium, Speed30 (WoodElf35), Darkvision60 (Drow120), lineage
choice and casting-ability choice INT/WIS/CHA. Drow getsDancingLights; HighElf gets
Prestidigitation plus long-rest Wizard-cantrip replacement; WoodElf getsDruidcraft.
At3/5 lineage grants FaerieFire/Darkness, DetectMagic/MistyStep or
Longstrider/PassWithoutTrace respectively, always prepared, once per Long Rest
free casts and slot-casting options. Also FeyAncestry advantage versusCharmed,
KeenSenses choice Insight/Perception/Survival, Trance4h conscious Long Rest and
immunity to magical sleep. Remove dwarf traits/HPbonus. Cantrips/spellcasting,
Charmed/sleep/rest exceptions and lineage progression are not implemented here;
offering Elf now would silently lose source traits, so it remains unavailable.

## Policy, simulations and limits

The custom table tactic policy chooses reachable opponents deterministically,
first seeks a legal ranged position for ranged builds, falls back to adjacent melee when retreat cannot yield a firing position, verifies each proposed approach endpoint with Rules attack_from (including total cover), finds a shortest legal approach, Dashes then moves when
needed, and ranges from cover beyond adjacent hostiles when possible. It selects
melee equipment for the existing melee hero build, even though its background
includes a Shortbow. Candidate admission rechecks the pure Rules action. Ranking
uses empty contributions to retain this explicit policy order, never fake evidence
IDs, invented numerical rule bonuses or production pins. `TacticContext` requires
the caller's canonical Basis/CheckpointPins, policy and permitted knowledge.
This milestone's synthetic fixtures have full public battlefield sight; native
integration must supply a perception-filtered tactical snapshot. Policy declines
optional reactions; players/other policies can explicitly take them. Monster
Hide, climbing and sunlight are outside this normal-sight/night subset, not
silently approximated. Kobold sunlight scenarios must not be admitted to it.

The simulator's seed stream exists only in tests and turns outcomes into existing
ActualDraw records before any Rules call. It retries MissingDraw with the retained
prefix and checks every accepted action, no input mutation, exact records, turn
budgets, ammunition, cover, range reasons, and completion. Fixed source stats are
never increased to lengthen encounters. The selected one-goblin seed is a retained
miss/cover regression, not a claim that every goblin fight will take three rounds.
Fixed seeds after the independently reviewed policy repairs are31 (one goblin),13 (three goblins),23 (wolves) and0 (skeleton). Four scenarios are required to last at least three rounds: one goblin, three
goblins in cover, two wolves, and a distant ranged skeleton. Logs and exact results
are retained in the attempt evidence; two compact logs are included in the handoff.

Out of scope: attacks against allies/objects/locations (this slice only admits opposing creature targets), spells, Large/Tiny/flying/swimming/climbing/grappling, hiding/stealth,
light/senses beyond authored line of sight, surprise, Ready/nested unrelated
reactions, source stat-block senses/skill use, magic resistances, sunlight,
full equipment encumbrance/drop/retrieval at distant positions, rest catalog,
medical first aid/stabilization, automatic1d4-hour stable recovery, surrender/flee
or both-sides-agree endings, advancement and every other R01–R17 catalog option.
Knockout rest/awakening continues through the existing local_journey rest contract;
this battle API retains its knocked_out/Prone/dropped state for integration to apply
those benefits. These unavailable actions must not be offered as supported rules.
No custom rule shortcuts are used for the supported modes beyond the disclosed
cover geometry; unsupported scenarios require a typed refusal or integration gate.

## Follow-up integration and observations

1. Freeze a versioned df-model durable tactical snapshot/codec and actor-ID mapping.
   Store layout id and immutable scene/pack/source revisions; grid/terrain/cover;
   all combatants' cells, source stats and traits, HP/Prone/Unconscious/dead/knockout,
   held/dropped items, counts, ammo used/recovered, SecondWind; initiative order,
   active index/round/turn counters, all budgets, Slow/Sap/Vex expiries, optional
   pending movement path/next step/ordered reactor choices and death-save timing.
   Preserve this pure detached value; session alone commits state/draws exactly once.
2. Reuse existing df-encounter canonical participant/challenge admission. Source
   owner pins actual clauses/catalog/rights; engine provides valid source-qualified
   TacticContext and perception-safe snapshot, and binds Rules movement queries to
   the existing MovementOwner/staging adapter. Do not invent canonical offers.
3. Add generated Rust game actions Move(path), Dash, Disengage, Dodge, Stand,
   attack(weapon/mode/target, optional melee knockout/Savage), Light/Nick extra,
   SecondWind, EndTurn, optional Opportunity take/decline, DeathSave and minute
   recovery. Add needed shield/drop pickup equipment operations. Server prepares
   exact draws and persists pending windows across reconnect before publication.
4. Admit the new creation option atomically: new equipment/items, array id,
   Dexterity background bonus/template, Archery and three masteries, source-derived
   weapon rows and inventory counts. Explicitly disclose centre-line departures.
   Keep Standard2024 catalog admission separate from custom geometry policy.
5. Wire new N-enemy/variable-layout story preparation, source creature actions and
   the selected female archer presentation. Existing fixed three-actor journey,
   nonlethal narration and unconditional1HP ShortRest assumption cannot be reused
   for ranged lethality. Monster0HP is dead by default; hero0HP is dying unless
   massive damage killed them. Continue timed death saves even after fight ends;
   do not describe dead ranged targets as knocked out or recovering on ShortRest.
6. Audience view must carry layout_id and source revision; each visible combatant's
   id/side/cell/HP/status, initiative/acting actor/round; acting hero's action/bonus/
   reaction/movement remaining and Speed, ammunition/weapon modes, reachable cells,
   legal targets by weapon/mode and cover per target, target range and roll reasons;
   pending reaction owner/options/path preview; explicit dying/dead/knockout/end
   result. These are supplied server decisions. Client sends input and renders;
   it never recomputes legality or hides private facts received from the server.
7. Emit bounded decision diagnostics through shared df-observe at engine/session
   admission/commit, correlated with operation/run/revision/pins. Record typed
   rejections, action/tactic/pending timing, path cost, effect reasons, exact recorded
   draws under scoped evidence policy, cover departure and lethal consequence.
   Pure Rules returns facts and performs no ad hoc logging/OTEL I/O. No provider
   failure may change source outcomes. Then test durable retry/reconnect/optional
   reaction continuation, same-phase view updates and real WASM/browser output in
   the follow-up; none is claimed tested by this pure milestone.

Workflow SQLite is absent in this worktree; sanitized worker observations are
retained at `artifacts/tmp/play-t1/devlog-worker.jsonl` for later scoped intake.
Model routing substitution: inherited worker model for hard mechanics/interfaces;
unavailable Terra is not assumed. No commits, network, servers, databases or
screenshot recorder were started. No running preview exists for this pure task;
ADR0005's executable internal-boundary evaluation applies. Independent evaluation
and coordinator integration remain distinct from passing these worker checks.
