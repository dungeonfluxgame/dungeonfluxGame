# Character design catalog

2026-10-23. Implements the selected 2024 core **design** catalog. This is not a claim of full playable rules coverage.

The server offers all ten ancestries, twelve classes and sixteen backgrounds. Any ancestry can be combined with any class and background for character design and portrait generation. The original short descriptions are visual/story cues, not copied Player's Handbook mechanics or grants. Expansions and homebrew remain outside the selected first catalog.

- Ancestries: Aasimar, Dragonborn, Dwarf, Elf, Gnome, Goliath, Halfling, Human, Orc, Tiefling.
- Classes: Barbarian, Bard, Cleric, Druid, Fighter, Monk, Paladin, Ranger, Rogue, Sorcerer, Warlock, Wizard.
- Backgrounds: Acolyte, Artisan, Charlatan, Criminal, Entertainer, Farmer, Guard, Guide, Hermit, Merchant, Noble, Sage, Sailor, Scribe, Soldier, Wayfarer.

Name sources: the official [2024 Player's Handbook overview](https://wpn.wizards.com/en/products/2024-players-handbook), [2024 Basic Rules index](https://www.dndbeyond.com/sources/dnd/br-2024), and [2024 backgrounds overview](https://www.dndbeyond.com/posts/1785-the-backgrounds-and-origin-feats-in-the-2024), reviewed 2026-10-23. These public indexes do not replace the unacquired full books or their source/rights admission. The acquired SRD and current mechanics limitations remain documented in [character creation research](character-creation-refinement-research.md) and [rules support](rules-support.md).

## Connected draft and image requests

`creator_catalog.rs` owns the closed name/ID catalog. `CharacterCreationOffer.supported_designs` (field 6) contains typed ancestry/class/background combinations from the actual pack's admitted offers, captured before expanding design choices. Browser presentation uses this server metadata; it does not assign game statistics. Selecting another combination keeps appearance, story and prior package drafts, hides incompatible gameplay choices and disables gameplay confirmation with an explicit explanation. Returning to a supported combination restores its controls and draft. The server independently rejects unsupported combinations before creation is admitted.

`HeroPortraitRequest` keeps ancestry in field 8 and adds class/background fields 9/10. Missing new fields from older clients explicitly mean the legacy Fighter/Soldier context. Unknown identifiers are rejected. The same checked public context is used for request fingerprints, the actual creator image prompt and acceptance attachment matching. An ancestry, class or background edit invalidates the displayed image's current-draft status; polling or retrying an operation with changed inputs cannot dispatch a second image. Class/background styling grants no equipment or mechanics and does not override the chosen attire. Exact custom public text is retained. Private story text is excluded.

## Native controls

Ancestry, class and background use native selects with a current-selection description rather than fixed labels or a long wall of cards. Native phone pickers and keyboard behavior are retained. The collapsed controls retain the workshop palette. Popup options and option groups use dark text on white with a light color scheme, because the reported Windows picker ignored a dark background while retaining pale text. Disabled options also use a readable light surface. Pixel observation of a browser page alone cannot establish the appearance of an OS popup that the capture surface omits.

The gameplay sheet remains server-derived for the actually implemented subset. Unsupported designs cannot silently become Fighters. Persistent cross-session design libraries, class mechanics outside that subset and complete 2024 progression remain separate work.

## Integrated qualification

All ten ancestries include original visual anatomy guidance in the checked public design context. Creator-only direction supports practical creature heads where appropriate without changing scene imagery or overriding custom features, build, colors or attire. A first live Dragonborn result looked too human; the corrected actual UI request produced a recognizably draconic head and scaled body with the selected green cloak.

On port 63589, Jev classified that Dragonborn/Ranger/Guide request as general appearance in 271 ms. OpenAI completed the image in 9,796 ms; the complete creator job took 10,069 ms. The browser decoded the 1024×1536 result at opacity 1. Changing class then marked the preserved portrait out of date without generating another image. Desktop, tablet and phone layouts were observed; six viewport checks from 320 to 1366 pixels showed no horizontal overflow. Physical devices, OS-popup pixels omitted by browser capture, and an intermediate fade frame were not independently observed.

Five targeted native behavior tests and three protocol tests passed, including all 1,920 design combinations, actual assembled portrait context, stale matching and unsupported gameplay refusal. Targeted formatting and integrated native/WASM compilation passed; a final native check passed after the anatomy-only delta. Existing generated-protocol Clippy warnings and 108 unrelated schema-ledger omissions prevent claiming clean broad lint/schema gates. The workflow SQLite is empty, so no durable queue/devlog completion was fabricated.

Verified native SHA256: `69C51C91B7059ACCEEC82BB084404382B18D943BA66C3A3A7CC882F9E9AB4B1B`; served WASM: `D9EFE47F87714796321D3C2575DE11A669AD628DCF4440D34547788C9EF7F5E9`.

## Portrait scenery and player-facing progress

Subsequent clarification distinguished portrait scenery from the D&D origin background. A separate control offers Studio backdrop, Forest glade, Warm tavern, Ancient ruins, Rocky coast and Describe a setting. The custom setting preserves exact public text up to 360 characters / 768 UTF-8 bytes. It is visual reference data, never a campaign location or rules change. Editing it invalidates the prior portrait without submitting another image request.

Additive contracts: `HeroPortraitRequest.scenery` field 11, `CharacterSelection.portrait_scenery` field 4 and `CharacterCreationOffer.portrait_scenery_options` field 7. Empty legacy scenery means the studio backdrop. The same validated context is used for classification, prompt assembly, request identity and accepted portrait matching; custom scenery replaces the former mandatory charcoal backdrop.

Player progress is now “Preparing your portrait…” / “Creating your portrait…” / “Portrait ready”. Supplier names, model names and routing categories remain diagnostic log fields. Player-facing supplier/setup failures give concise recovery information without those internals. Admission, routing and safety settings are unchanged.

Six newly compiled native tests and four protocol tests passed. Actual scene-image adherence, responsive UI observation of these new controls and a requested nonsexual adult nude reference remain unqualified: the user requested finishing and server shutdown before a new preview or image request. The earlier successful Dragonborn screenshot above predates these scenery controls and must not be presented as their live proof.

Native and browser builds completed without launching a server. Retained pair: `artifacts/build/creator-scenery-preview/debug/df-transport-fixture.exe` and `artifacts/build/creator-scenery-preview/play-web`. Native SHA256 `E2A655C9B32D5FDD1DE9F340B0C9E2833BDD6AECA0AEF6AC2825CA9667033934`; WASM `EBAE69E8E665484025E01592FDD679AD078AFB1B2DFC3A43EEA594E0AF318D8F`. These are built artifact identities, not served-runtime qualification.

Final primary-checkout native and WASM integration checks both passed after the frozen source and confirmation-state delta were copied with base/candidate hash verification. No development servers were restarted for these checks.
