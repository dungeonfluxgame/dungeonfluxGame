# Custom character creation: current contracts and next implementation

Date: 2026-10-23. Read-only source audit followed by the separately bounded media-default policy change. This is not evidence that a full custom-character or complete 2024 catalog feature is implemented.

Governing documents: [Game refinement](game-refinement.md) REF-019–021 and REF-026–029; [Rules support](rules-support.md); [Rules coverage](rules-coverage.md) R01–R03/R10–R12; [Asset engine](asset-engine.md) identity revisions, admission, references, caching and nonblocking failure; [Client architecture](client-architecture.md); [Generated content](generated-content.md); [Coding style](coding-style.md); ADR 0004/0005. User decisions supersede historical provider preferences without changing photographic continuity or server authority.

> Historical audit: the source findings below predate the later input-driven portrait, Jev/BFL routing and creator-catalog refinements. Read [Game refinement](game-refinement.md), especially REF-032–042 and their qualification notes, for current implementation status. The original observations remain as dated evidence rather than a current support claim.

## Catalog and mechanical status

`development/rules-sources/manifest.json` pins English SRD 5.2.1, acquired 2026-09-30, SHA256 `8974902d109d6e63672d7c490bde9ccf052410503d9cfa768237154fbc5e3d87`, CC-BY-4.0. It explicitly lists the full Player's Handbook, Dungeon Master's Guide and Monster Manual as unacquired, with full denominator/rights and production RulesetId unresolved. Public name indexes do not supply that missing source/rights admission.

The selected initial 2024 core lists in [refinement](game-refinement.md#selected-initial-2024-core-catalog) contain ten species and twelve classes, verified through the official linked name indexes. The user explicitly selected “2024 core catalog first.” Expansions and homebrew are not selected for this initial scope. Aasimar is in that PHB list but absent from the nine-species SRD enum.

| Current boundary | Observed source |
| --- | --- |
| Species enum recognizes Dragonborn, Dwarf, Elf, Gnome, Goliath, Halfling, Human, Orc, Tiefling | `crates/df-rules/src/local_journey.rs`, `HeroSpecies` and `from_choice` |
| Actual low-level creation validator admits only Dwarf, Halfling, Orc | Same file, `validate_character`; other variants return InvalidChoice |
| Harbor's current selectable species is Dwarf only | `content/harbor-journey/pack.json` creation groups and `crates/df-tools/src/gameplay/pack.rs` fixed-group validation |
| Class/background are Fighter/Soldier | `local_journey.rs` required choices, harbor pack and sheet construction |
| Three build packages are melee-stalwart, melee-vanguard and archer | `pack.rs::build_packages`; archer data from `content/hero-builds/dwarf-archer.json` |
| Fixed Fighter/Champion progression supports 1–5; normal creation level 1, test creation level 5 | `local_journey.rs::FighterProgression`, `journey/tactical.rs::test_fighter_level` |
| Four background IDs appear in an origin-feat helper, but that is not four playable creation backgrounds | `local_journey.rs::background_origin_feat`; validator still requires Soldier |

The source label remains `SRD5.2.1-2024-local-journey-3`. It describes an explicit subset. Full class names, enum variants or a settings toggle do not supply breath weapons, spells, feats, subclass progression, multiclassing or other missing handlers.

## Creation and persistence contracts

`crates/df-protocol/proto/gameplay.proto` defines `CharacterSelection`, `CharacterCreationOffer`, `HeroAppearance`, `HeroIdentity`, `PreviewHeroIdentityRequest/Response`, and `CharacterSheet`. The current appearance contains ten closed fields: presentation, pronouns, adult age category, build, skin, hair, hair color, eyes, mark, attire. The `HeroIdentity` also carries origin, occupation, motivation, backstory and public facts.

`gameplay_browser/hero_creator.rs` keeps one `HeroDraft`/`IdentityDraft`; it is the existing place to preserve selections across simple/advanced mode changes. It sends server choices and identity, not authoritative stats. `gameplay/hero_identity.rs` validates IDs against `content/hero-tables/identity-menu.json` and bounds story fields. Its active preview RPC calls `read_local` (matching known phrases). An async model-reading helper exists, but the active endpoint does not currently call it. Arbitrary detailed body features are not preserved by that closed schema.

`gameplay/wire.rs` validates submitted identity then stores only canonical `identity-appearance:*` choices across the durable creation boundary. It deliberately excludes story data from shared photographic prompts. Consequently, the full submitted free-text identity/backstory is not durably round-tripped by that path. A new accepted identity record/reference is needed before claiming complete persistent customization; changing the UI alone would lose information.

`journey.rs::accept_character_at_level` runs `pack::validate_character`, commits accepted choices and server-derived resources through the existing owner, then produces sheet facts/HP/AC. Several accepted content IDs remain hard-coded as `dwarf-fighter-soldier`; these must change with actual broader mechanics, not merely a label. The cosmetic workflow must not silently replace unsupported builds with this Fighter record.

## Image/reference hooks that already exist

- `gameplay/imagery.rs::ImageRoute` already contains OpenAI and FLUX adapters. `startup_configuration.rs` already defaults its image model to `gpt-image-2.5-flare`. The bounded change makes unspecified provider routing choose OpenAI, rather than FLUX merely because its key exists. Explicit FLUX remains supported under current admitted configuration; this audit does not select a new model or run paid calls.
- `living_scenes/canonical.rs` reconstructs accepted appearance from checkpoint choices, selects canonical hero identity/gear revisions, and calls `render_canonical` with `HeroVisualIdentity`. Custom accepted appearance can reach canonical generation without a quickstart dossier. This is an existing hook, not a missing entire pipeline.
- `imagery/canonical_prompt.rs::canonical_character` and `imagery/prompt.rs::hero_portrait` build public photographic identity prompts. Existing simple/advanced attributes should feed this checked boundary, not raw private text.
- `imagery/references.rs` admits bounded reviewed photographic reference bytes with hashes. `imagery/party_sheet.rs` and `cast_sheet.rs` deterministically compose reference photographs with stable identity order and source digests. These existing cast sheets are not a generated front/profile/back character turnaround sheet.
- `living_scenes/plates.rs` and `imagery/plate_prompt.rs` consume canonical references and speaker portrait/figure images. Binding hashes and existing resource-hub cache keys provide the integration point for an approved character reference revision.
- `resources.rs` supplies Depiction/ScenePlate and related resource specifications, pending/ready/failure status, canonical keys, cache handling and spend records. Keep generation, validation and publication here; do not add a parallel provider queue.

Gaps: no explicit user-approved immutable portrait/reference-pack record is exposed in the creation protocol; no complete generated visual turnaround sheet approval flow exists; raw arbitrary appearance detail is not stored losslessly; free-account character portability remains unimplemented. Optional selfie capture/provider consent is a separate future feature and must not become an implicit ordinary plate reference upload.

## Video-cost policy and provider routing

Home/scene settings previously enabled automatically with a configured fal credential. Scene enablement also gates kill clips (`home_video.rs::request_kill_shot` and `Engine::request_kill_shot`). Character-action clips have a separate provider composition in `character_clips.rs::native_providers`; home/scene switches alone could not stop them.

The bounded runtime change uses the actual shared production `media_policy.rs`: absent/empty/off video flags are false, explicit on true, invalid values refuse without echoing input. Home and scene/kill flags stay independent. New `DF_GAMEPLAY_CHARACTER_VIDEO=on` is required for character clip generation; with it off, no video adapter or clip-only image-frame provider is constructed. The portrait/canonical still/scene-plate pipeline is separate and remains available. Existing character cached assets remain readable; turning off generation is not a deletion operation. New scene/home generation is not requested with defaults off.

Image routing defaults to OpenAI with unset/empty/openai; explicit flux requires a fal credential; other and non-Unicode inputs refuse. The existing imagery admission still requires OpenAI's credential before route construction, so fal-only launch is not newly promised to work. Provider keys alone do not choose a different image route or enable paid video.

## Next bounded implementation slice: accepted hero identity and visual pack

Own one complete creation→approval→durable resume→scene-reference result, within the currently mechanically supported build subset. Scope: creator UI, typed appearance draft/accepted-identity contracts, owning server validation/persistence, canonical image resources, explicit generation/review controls and plate consumption. Do not couple this to accounts, selfie likeness, payments, six-player expansion, full spell systems or complete class mechanics.

Acceptance:

1. Simple and advanced edits preserve the same draft, including custom descriptive detail, without granting rules effects. Stale previews cannot overwrite newer edits; rejected submissions remain editable.
2. The server derives and validates the mechanical sheet. Client-provided/class-sounding prose cannot mint stats, feats, spells or gear.
3. An explicit bounded generation request produces portrait and proposed visual reference views from public accepted appearance/actual gear. No paid generation occurs on each toggle. A faithful local provider fake exercises the ordinary failure paths.
4. Review approves immutable identity/asset revisions. Generation refusal, timeout, reconnect and stale completion preserve existing usable output and expose recovery. No raw selfie/private story/entire private sheet is sent to general plate requests.
5. Durable resume restores the accepted details and matching assets without rereading an LLM or generating again. A public scene plate demonstrably consumes those reference hashes; changing identity invalidates future dependent demands without corrupting historical references.
6. Catalog settings list pinned names and distinct listed/enabled/implemented/source-unavailable states. Unsupported class/species combinations cannot finalize as playable or silently become Fighters. Full-core source/rights and subclass/background/spell dependencies remain explicit follow-up work.
7. Focused native/WASM checks plus real phone-sized and shared-display browser operation validate the submitted build. Camera, audio and actual provider quality remain unverified unless directly observed in separately scoped checks.

## Verification and evidence limits

This audit does not rebuild or replay the running integrated preview. The workflow database is still zero bytes and no current-task devlog context was provided; old unrelated contexts were not reused and the database was not initialized. The worker retained observations in its task report instead. The primary tree's previous launch/icon changes and unknown provider/economy edits are preserved.

The media-default change uses small production-policy tests and an affected native `cargo check`; results and exact source hashes accompany the implementation handoff. Full df-tools test linking is deliberately avoided after the earlier disk exhaustion. No paid requests or new generated images/video were sent for this task.
