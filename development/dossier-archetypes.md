# NPC Archetype Library

The NPC archetype library is a reusable source of old-world occupations, social roles, role-specific habits, and light personality flavor. It gives a generated dossier useful characterization without replacing its name, relationships, history, knowledge tiers, or story state.

## Content and coverage

The versioned source is [`content/adventure-tables/npc-archetypes.json`](../content/adventure-tables/npc-archetypes.json). Version 1 contains 203 distinct role profiles across 61 categories and five composable flavor axes. Every role has a stable ID, a readable label, optional aliases, and cues for instincts, priorities, boundaries, knowledge domains, speech habits, and typical actions. The entries cover rural and household work, crafts, market and river trades, civic and sacred service, guilds and court roles, travel, adventuring, and military work.

The role cues are deliberately specific to the work. For example, a mapmaker separates a surveyed route from a tale; an herb gatherer checks season and soil; a watch sergeant asks what witnesses actually saw. The library does not give every person the same generic curiosity or helpfulness, and it avoids modern office and corporate roles.

The five d10 axes are:

- **Experience:** from new to the work and apprenticed through practiced, veteran, returned to the role, and nearing retirement.
- **Competence:** reliable, skilled, exceptional at the craft, careful but slow, quick but uneven, gifted beginner, knowledgeable with gaps, struggling, overconfident, or out of practice.
- **Temperament:** patient, wary, warm, brisk, reserved, curious, wry, proud, even-tempered, or earnest.
- **Motive:** livelihood, kin, duty, debt, craft pride, faith, curiosity, peace, reputation, or independence.
- **Style:** plain-spoken, measured, anecdotal, question-led, formal, concrete, gentle, wry, story-rich, or spare.

These are narrative cues, not mechanics. They add no skill, bonus, authority, progression, or world action. A broad-practice question that directly matches a listed knowledge domain may receive cautious, general occupational advice in the NPC's own words. A domain does not establish facts about a specific item, person, event, current condition, or guaranteed result. A typical action is not an action that has happened in the current scene.

## Resolution and persistence

`dossier_tables.rs` loads and validates this file as part of the source revision. `dossier.rs::roll_one` asks `dossier_archetypes.rs` to resolve the cast member's stable `archetype` ID or alias first, then the normalized role label; if no direct label exists, a role-label alias is the final fallback. A matching stable ID takes precedence over a conflicting label. Unknown roles produce no profile. IDs and explicit aliases share a uniqueness check; normalized labels are also unique. Resolution validates the complete file before any dossier is rolled.

Flavor choices use independent, deterministic streams keyed by adventure seed, NPC ID, and axis name. They do not consume or shift existing dossier rolls. Five flavor rolls are recorded as `dossier.archetype.<axis>`. The resolved base cues, selected labels, guidance, library version, and source ID are copied into the dossier snapshot, so saved records retain the exact characterization after a later source update. Old saved dossiers default to no archetype snapshot and continue to load.

The current narrator path is:

1. `DossierTables::load` reads and versions the source.
2. `roll_dossiers` resolves the cast role and persists the composed profile.
3. `dossier_render::render_dossier` shows the source ID/version and resolved flavor for author inspection.
4. `npc_facts::public_role_archetype` projects safe cues into `NpcReplyContext.facts.role_archetype`.
5. The NPC reply rules use instincts, priorities, boundaries, and voice for characterization. A direct broad-practice question may be answered from a matching knowledge domain as cautious general advice; domains never establish specific facts. Every serialized string is checked against this listener's withheld secrets; a profile containing a withheld term is omitted from that listener's prompt.

The fixed freeplay store bypasses `roll_dossiers`. Its authored snapshots in `content/merchant-shop/dossiers.json` bind Marek to `cloth-merchant`, Corin to `adventurer`, and Jessa to `soldier`. `shop::generate` checks those snapshots against the current library and includes the archetype source in the store revision. Corin's six years guiding mule trains are paired with the authored `seasoned` and `skilled` flavors; his road-wear domain is available in his actual NPC reply facts.

The library does not replace the separate private/public knowledge tiers. It contains reusable public role guidance, not a character's secret biography. It does not change identity, ownership, party knowledge, or server-authoritative state.

## Worked example

A cast member authored with `archetype = "merchant"` and `role_label = "cloth merchant"` resolves to `cloth-merchant` through the stable alias. The shared base profile says what a cloth seller notices about weave and dye, values about fair measures, will not promise about cloth quality, knows from market work, tends to say, and may do at a counter. The adventure seed then selects that NPC's experience, competence, temperament, motive, and style independently. A second merchant can reuse the same role cues while receiving different seeded flavor. Those choices are saved on the dossier; they do not generate new family, secrets, skills, or facts about the current adventure.

The library also has explicit `adventurer` and `soldier` profiles. Their concerns differ: the adventurer weighs reward, route, provisions, and the companions' consent; the soldier considers assigned duty, order, and the company. The adventurer's knowledge domain includes ordinary travel wear: soles, pack straps, garment seams, damp cloth, and friction beneath a loaded pack. Asked what tends to wear out first on a long road, an experienced traveler can give a short rule of thumb about those stress points; that does not assert anything about the clothes currently on sale. Both archetypes can still be inexperienced, skilled, wary, or wry through the independent flavor axes.

## Authoring and validation

Add a new role as a distinct profile with a stable kebab-case ID, a short category and label, and one to three useful cues in each of the six role-specific fields. Reuse an alias only when it is an actual stable cast ID or common role label; IDs and aliases share one uniqueness check, while normalized labels are unique separately. Write cues that say what the person notices, values, will not promise, has encountered in work, sounds like, or might do while working. Keep private facts, guaranteed outcomes, mechanical benefits, and commands out of this file.

When changing the JSON shape, change `df-npc-archetypes/1` and the parser together. Content revisions change the dossier-table digest automatically. The resolved dossier snapshot is the replay contract; do not regenerate old NPCs from the newest library. The parser checks unique IDs and aliases, required fields, bounded plain text, recognized dice, unique flavor IDs, and full die coverage.

Focused tests cover the library breadth and role distinctions in `dossier_archetypes.rs`, generated snapshot/provenance and rendered inspection in `adventure/dossier_tests.rs`, and generated dossier-to-NPC-context persistence plus listener-specific secret filtering in `npc_dossier_tests.rs`.

## Role index

The IDs and readable labels are grouped by category below. The JSON source remains the authoritative text and alias index.

- **adventuring-life** — `adventurer` (`adventurer`)
- **adventuring-service** — `ruin warden` (`ruin-warden`)
- **adventuring-trade** — `bounty hunter` (`bounty-hunter`), `delver` (`delver`), `monster hunter` (`monster-hunter`)
- **animal-service** — `animal keeper` (`animal-keeper`), `hound keeper` (`guard-dog-handler`), `stable master` (`stable-master`), `stablehand` (`stablehand`)
- **animal-trade** — `mule breeder` (`mule-breeder`)
- **burial-service** — `gravedigger` (`gravedigger`), `undertaker` (`undertaker`)
- **caravan-service** — `caravan cook` (`caravan-cook`), `caravan guard` (`caravan-guard`), `caravan master` (`caravan-master`)
- **civic-guard** — `guard captain` (`guard-captain`), `night watchman` (`watchman`), `river watch guard` (`river-watch`), `veteran watch guard` (`watch-veteran`), `watch guard` (`watch-guard`), `watch scout` (`watch-scout`), `watch sergeant` (`watch-sergeant`)
- **civic-office** — `advocate` (`barrister`), `bailiff` (`bailiff`), `council speaker` (`council-speaker`), `reeve` (`reeve`), `tax collector` (`tax-collector`), `village magistrate` (`village-judge`), `warrant clerk` (`warrant-clerk`)
- **civic-service** — `bell ringer` (`bell-ringer`), `bell tower keeper` (`bell-tower-keeper`), `lamp warden` (`lamp-warden`), `ledger keeper` (`tax-ledger-keeper`), `road steward` (`steward-of-roads`), `road warden` (`road-warden`), `scribe clerk` (`scribe-clerk`), `signal keeper` (`signal-keeper`), `street lamplighter` (`lamp-lighter`), `toll keeper` (`toll-keeper`), `village messenger` (`village-messenger`), `watch clerk` (`watch-clerk`), `woodland warden` (`woodland-warden`)
- **civic-trade** — `land surveyor` (`land-surveyor`)
- **clay-craft** — `potter` (`potter`)
- **community-role** — `folk storyteller` (`folk-storyteller`), `keeper of local stories` (`story-keeper`), `town musician` (`town-musician`), `town or village resident` (`commoner`), `village elder` (`village-elder`), `village warder` (`village-warder`)
- **court-craft** — `heraldic painter` (`heraldic-painter`)
- **court-service** — `court fool` (`fool`), `court messenger` (`court-messenger`), `court reader` (`heralds-reader`), `herald` (`herald`), `herald at arms` (`herald-at-arms`), `herald's page` (`heralds-page`)
- **craft** — `glassworker` (`glassworker`)
- **diplomacy** — `envoy` (`envoy`)
- **earth-trade** — `miner` (`miner`)
- **field-service** — `ranger and guide` (`ranger-guide`), `scout` (`scout`)
- **fine-craft** — `jeweler` (`jeweler`), `silversmith` (`silversmith`)
- **food-trade** — `baker` (`baker`), `brewer` (`brewer`), `butcher` (`butcher`), `vintner` (`vintner`)
- **guild-trade** — `guild apprentice` (`guild-apprentice`), `guild factor` (`guild-factor`), `guild journeyman` (`guild-journeyman`), `guild master` (`guild-master`)
- **healing-service** — `field surgeon` (`field-surgeon`), `healer` (`healer`), `midwife` (`midwife`), `roadside healer` (`roadside-healer`), `village midwife` (`town-midwife`)
- **healing-trade** — `apothecary` (`apothecary`), `barber-surgeon` (`barber-surgeon`), `leech gatherer` (`leech-gatherer`)
- **hospitality** — `barkeep` (`barkeep`), `innkeeper` (`innkeeper`), `pilgrim host` (`pilgrim-host`), `refuge keeper` (`refuge-keeper`), `tavern keeper` (`tavern-keeper`), `tavern server` (`tavern-server`), `wayside innkeeper` (`roadside-innkeeper`), `waystation keeper` (`waystation-keeper`)
- **household-guard** — `castle gatekeeper` (`castle-gatekeeper`)
- **household-military** — `castle armorer` (`castle-armorer`), `knight's squire` (`squire`)
- **household-office** — `court steward` (`court-steward`)
- **household-service** — `cook` (`cook`), `household retainer` (`retainer`), `laundress` (`laundress`), `message runner` (`messenger-child`), `storehouse keeper` (`storehouse-keeper`)
- **household-trade** — `chandler` (`chandler`)
- **itinerant-craft** — `tinker` (`tinker`)
- **journey** — `pilgrim` (`pilgrim`), `pilgrim guide` (`pilgrim-guide`), `pilgrim seeking alms` (`pilgrim-beggar`), `wayfarer` (`wayfarer`)
- **learning-service** — `schoolmaster` (`schoolmaster`)
- **leather-craft** — `cobbler` (`cobbler`), `cobbler's apprentice` (`cobbler-apprentice`), `leatherworker` (`leatherworker`), `saddler` (`saddler`)
- **market-service** — `market crier` (`market-crier`), `porter` (`porter`)
- **market-trade** — `charcoal merchant` (`charcoal-merchant`), `cloth merchant` (`cloth-merchant`), `market factor` (`ledger-merchant`), `peddler` (`peddler`)
- **metal-craft** — `armorer` (`armorer`), `bell founder` (`bell-founder`), `bell foundry hand` (`bell-founder-helper`), `blacksmith` (`blacksmith`), `farrier` (`farrier`), `herd bell maker` (`herd-bell-maker`), `lantern maker` (`lantern-maker`), `locksmith` (`locksmith`), `smith's apprentice` (`smith-apprentice`)
- **military-craft** — `crossbow maker` (`crossbow-maker`), `field carpenter` (`field-carpenter`), `fletcher` (`fletcher`), `sapper` (`siege-sapper`), `siege engineer` (`siege-engineer`)
- **military-service** — `archer` (`archer`), `banner bearer` (`banner-bearer`), `camp cook` (`camp-cook`), `field medic` (`field-medic`), `field scout` (`field-scout`), `garrison cook` (`garrison-cook`), `mercenary` (`mercenary`), `outrider` (`outrider`), `pikeman` (`pikeman`), `quartermaster` (`quartermaster`), `retired soldier` (`retired-soldier`), `shield bearer` (`shield-bearer`), `soldier` (`soldier`)
- **military-training** — `fighting instructor` (`fighting-master`), `swordmaster` (`swordmaster`)
- **militia-service** — `militia captain` (`militia-captain`), `militia volunteer` (`militia-volunteer`)
- **river-craft** — `boatwright` (`boatwright`), `netmaker` (`netmaker`), `sailmaker` (`sailmaker`)
- **river-livelihood** — `fisher` (`fisher`)
- **river-service** — `ferry boatman` (`ferry-boatman`), `ferry clerk` (`ferry-clerk`), `ferry keeper` (`ferry-keeper`), `ferryman` (`ferryman`), `retired sailor` (`retired-sailor`), `river pilot` (`river-pilot`), `sailor` (`sailor`)
- **road-service** — `courier` (`courier`), `road guard` (`old-road-guard`)
- **road-trade** — `carter` (`carter`), `muleteer` (`muleteer`), `provisioner` (`provisioner`)
- **roof-craft** — `thatcher` (`thatcher`)
- **rural-healing** — `herb gatherer` (`herb-gatherer`), `herbalist` (`herbalist`), `woodland herbalist` (`woodland-herbalist`)
- **rural-livelihood** — `farmer` (`farmer`), `goatherd` (`goatherd`), `shepherd` (`shepherd`)
- **rural-trade** — `beekeeper` (`beekeeper`), `fence mender` (`fence-mender`), `miller` (`miller`), `shearer` (`shearer`), `vine dresser` (`vine-dresser`), `windmill keeper` (`windmill-keeper`)
- **sacred-service** — `camp chaplain` (`chaplain`), `grave priest` (`grave-priest`), `pilgrim chaplain` (`pilgrim-chaplain`), `priest` (`priest`), `shrine keeper` (`shrine-keeper`), `shrine lamplighter` (`shrine-lamplighter`), `temple acolyte` (`temple-acolyte`)
- **scholarly-service** — `archivist` (`archivist`), `scribe` (`scribe`), `scribe apprentice` (`scribe-apprentice`)
- **scholarly-trade** — `mapmaker` (`mapmaker`), `star reader` (`astrologer`)
- **shore-trade** — `salt worker` (`salt-worker`), `ship chandler` (`ship-chandler`)
- **solitary-livelihood** — `hermit` (`hermit`)
- **stone-craft** — `mason` (`mason`), `stonecutter` (`stonecutter`)
- **textile-craft** — `dyer` (`dyer`), `dyer's apprentice` (`cloth-dyer-apprentice`), `pilgrim's tailor` (`pilgrim-tailor`), `tailor` (`tailor`), `weaver` (`weaver`)
- **underworld-trade** — `smuggler` (`smuggler`)
- **wood-craft** — `carpenter` (`carpenter`), `cooper` (`cooper`), `wheelwright` (`wheelwright`)
- **woodland-service** — `huntsman` (`huntsman`)
- **woodland-trade** — `charcoal burner` (`charcoal-burner`), `woodcutter` (`woodcutter`)
