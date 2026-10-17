# Standalone cloth shop

`DF_GAMEPLAY_ADVENTURE=merchant-shop` loads this authored social slice before random
chapter generation. The pack shares only the core character creation and SRD
mechanical profile. Its scene, cast, objects and NPC dossiers are authored here.

The pinned script uses `mode: freeplay`. Its validated graph has a gathering state,
an entry into the shop, and a repeating observation action. It opens no story thread,
creates no courier, and has no end card. Combat happens only when a hero starts it: a scene
assault (`SCENE_ASSAULT`) against Marek, Corin or Jessa, each fighting with the SRD 5.2.1 stat
block in their cast entry (`stat_block`). The pack has no random or encounter enemies, so
`tuning.json` states the scene-assault-only tactical block (`tactical.scene_assault`, with the
battle scene `merchant-shop`) and no roster. A typed attack, a freeform strike that Jev reads as a
strike, and the scene's "Start a fight" offer all start the same fight on the shop grid
(`content/battle-scenes/merchant-shop.json`).

When a fight starts, everyone else in the scene chooses (`journey::scene_fight::stance`): a
Commoner (Marek) flees and shouts for the watch; the watch (a Guard, Jessa) joins against the
aggressor unless Friendly to them; any other combatant (a Scout, Corin) joins only if Hostile to
them. The attacked person always fights. The target and every person who joins or flees record
the assault in the social ledger toward the aggressor only. The fight is an encounter inside the
freeplay beat: when either side has no conscious combatant it closes and play returns to the same
beat, with no experience and no payment. People who are down stay down (a Commoner at 0 hit
points is dead, a knocked-out person is unconscious), and neither answers nor can be spoken to;
people who fled are gone. Heroes at 0 hit points are Stable; if every hero is down the watch
drags them out or the survivors leave them. Loot is not modelled.
Native scene interactions persist independently in the existing checkpoint journal.

The serialized content contains only the room and store-entry scenes, their two
creation/entry labels and the observation loop. Tuning contains only the schema,
pack identity and core character creation rules. The record owns one
`freeplay_setting`; legacy chapter settings are an internal adapter after reload.
Story-only fields remain required for Story packs and are forbidden when populated
in Freeplay. No unused quest, antagonist, encounter or ending content is authored.

Marek (`merchant`), Corin (`shop-adventurer`) and Jessa (`shop-soldier`) keep the same
IDs in the scene model, public cast and their role-prefixed private dossier IDs.
Public room knowledge excludes the dossiers' trust-gated and forbidden details.

The finite `clothes-travelers` offer costs exactly 200 CP and advertises the concrete
item ID `item-clothes-travelers` before anyone buys it. A committed purchase transfers that
same durable item ID; replay and item cards resolve it without manufacturing a clone.
Catalog entries alone never establish ownership. Sold offers are removed
from public stock and NPC wares, while the authoritative catalog retains the key
to refuse another purchase without charging again.

`scene-mareks-cloth-shop` is the fresh room reference identity. Until its own art
is available, the asset remains pending; startup never substitutes a harbor room.
The three documentary portrait and sprite identities are the approved cast photos.

Boundary coverage is in `adventure::shop::tests`: production pack loading and record
reload, native hero creation and entry, real dossier targeting, a normal rolled
observation, take/return, purchase/reconnect/examine, current card chronology and
finite stock. Runtime, browser, media and independent evaluation remain separate gates.
