# Proposed scene memory representation

Date: 2026-10-16. Design only, based on source `fc916aea8335ef7c3e58d7bdbfcef51a5df5c21f` and retained FUTERA evidence. No implementation, build or provider calls.

## Decision

Keep an immutable observation for each distinct plate, plus an immutable record of how it was reconciled with the current scene. Canonical history remains the existing checkpoint and decision journal. A derived current view joins that history with a qualified plate observation; it never replaces world state with pixels.

There are three different questions: **what happened**, **what this image depicts**, and **what this NPC actually witnessed or believes**. They have linked provenance, not one mutable description string.

## Proposed private Rust-like records

This is a contract sketch, not existing compiled API. Existing domain types are reused; all collections need admission bounds. New records would be versioned through the current model/persistence/asset owners after contract review.

```rust
struct SceneBasisRef {
    checkpoint: Basis,                 // existing session/run/epoch/sequence
    checkpoint_digest: ContentDigest, // server-only retained state reference
    scene: ContentReference,
    scene_entry: OperationId,          // accepted visit/entry cause, not a timestamp
    owner_generation: u64,             // scene-owner lifetime; new contract gate
    public_snapshot: ContentDigest,    // exact permitted render projection
    world_at: LogicalTime,             // existing canonical game clock
}
struct PlateRecord {
    id: RecordId,
    render_basis: SceneBasisRef,
    base_plate: Option<RecordId>,      // exact parent photograph used for an edit
    request_key: RevisionLabel,        // request identity; NOT image-byte identity
    image: AssetReference,             // digest = SHA256 of complete delivered bytes
    camera_digest: ContentDigest,
    bytes_ready_unix_ms: u64,          // diagnostic wall time, not game time
}
struct PlateObservation {
    id: RecordId,
    plate: RecordId,
    vision_input_digest: ContentDigest,// exact downscaled input, if transformed
    job: JobId,
    schema: RevisionLabel,
    model: RevisionLabel,
    policy: ContentReference,
    audience: AudienceScope,
    measured_unix_ms: u64,
    entities: Vec<EntityAppearance>,
}
struct EntityAppearance {
    entity: EntityId,                  // trusted ID binding, never a guessed name
    identity_revision: RevisionLabel,
    visibility: Visibility,
    screen_box_permille: Option<[u16; 4]>, // left/top/right/bottom in THIS image
    fields: Vec<AppearanceField>,      // allowlisted pose/color/material/placement
    confidence: Confidence,            // Unknown/Low/Medium/High; not a probability
}
struct AppearanceField {
    kind: VisualField,                 // closed field vocabulary, no lore/motive
    value: String,                     // bounded, qualified visual description
    evidence: RecordId,                // exact observation, retained attribution
    confidence: Confidence,
}
enum Visibility { Visible, Occluded, OutOfFrame, Unresolved }
struct SceneMergeRecord {
    id: RecordId,
    previous_merge: Option<RecordId>,  // chronological scene-view parent
    observation: RecordId,
    checked_against: SceneBasisRef,
    policy: ContentReference,
    decisions: Vec<FieldMergeDecision>,// accepted/unknown/conflict/historical/stale
}
```

`base_plate` expresses image ancestry; `previous_merge` expresses accepted view history. Neither implies a world event. Display occurrences reuse existing presentation cues/reports to reference the selected plate/merge and their actual start/finish times. Rendering a fresh composition can have no base image while still following a previous merge. Repeatedly displaying the same image does not create a new observation or paid call.

Every accepted field retains its source observation, image digest, known entity binding, confidence and policy. Missing fields remain unknown. A current appearance cache may retain a separate **last seen** reference, but must not carry an old box into a new image as if measured there. Unknown confidence is not permission to infer a fact.

## IDs, revisions and time

- Bind existing authored `cast.id` / `SceneThing.id` to canonical `EntityId` through the content/session owner. Keep labels and aliases separate. Current `living_scene::snapshot` appends heroes using hero-name IDs: rename-safe binding/migration is an explicit prerequisite, not an already solved property. No new competing entity registry.
- Current renderer/cache identities also mix `npc:*`, `hero-{art}`, `hero:{art}` and names. Freeze one trusted binding at the boundary; image recognition must not choose or merge identities from those strings.
- `Basis` already contains session, run and `SessionRevision { epoch, sequence }`. Order canonical events by accepted revision and fact ordinal/cause. Verify ancestry and owner generation; a larger sequence alone does not prove the same branch after undo.
- `SceneGraph.revision` is a content hash, not chronological time. Use it only as a composition fingerprint. Retain the exact public render projection or reconstruct it from pinned checkpoint/content/reducer versions; missing evidence is a replay gap.
- That graph fingerprint is only eight bytes, and current `plate_id` does not include graph revision. Neither is the complete public-snapshot digest proposed here. A dedicated scene/session owner must define `owner_generation`; display, media, provider, access and consent generations cannot be substituted for it.
- `world_at` says which canonical moment the image depicts. Byte-ready, vision-completion and display timestamps say when processing/presentation occurred. Wall time and camera changes never advance `GameState.logical_time` or a threat clock. Several plates may depict the same world instant.

## Three-way merge: render basis B, observation O, current canonical scene C

First validate run/scene/entry, branch/generation, source/access/identity versions and actual image bytes. Then compare **relevant canonical fields** in B and C; unrelated private changes must not alter public output. The merge produces appearance metadata only, never a `SceneChange`, grant or new fact.

| B / O / C condition | Decision |
| --- | --- |
| Entity and relevant state unchanged; O locates it | Accept qualified appearance for this exact plate. |
| Entity moved, changed holder, left or was removed in C | Keep O as historical evidence of B; reject its old placement/state for the current scene. Never resurrect it. |
| O appears to show a different holder/open/broken state | Record a rendition conflict. C wins; no transfer/open/break event is created. |
| Entity omitted, occluded, outside crop or unresolved in O | Visibility unknown in that image. Do not delete the entity or claim absent-world. |
| Screen position changes under a different camera | Image placement only; no canonical movement or changed witness geometry. |
| O names a new person/object/identity | Reject as an authoritative entity; optional decoration proposal needs separate approval. |
| New checkpoint, same relevant public state | Requalify unchanged fields against C; no automatic call or universal invalidation. |
| Wrong branch/generation/image/access basis | Stale/rejected for current use, even if it completed last. Preserve permitted historical provenance only. |

A stale whole photograph containing contradictory actors/props must not become the current scene merely because some background fields remain reusable. Reuse boxes only with the same image bytes. Keep a valid current representation or canonical fallback; no automatic render-observe-render repair loop.

## Concrete harbor example and hypothetical handoff

Actual retained FUTERA plate request key: `67ff5cded9ad6c885ef152d575dc30f7969f0f8b0812bbea5c9de22413058fde`. Read-only hashing found its 616,832-byte JPEG has SHA256 `cf79b10ce6819c309af50edf15a843831e87613beb034ea99148fff5ec730dff`. Its saved layout names Courier/Bandit and head boxes; it has no packet observations or exact render-basis record. Import must mark that missing provenance **LegacyUnbound**, not invent a capture revision from a later displayed view. The fixed harbor packet also still needs an authored entity binding.

The following is a **proposed test fixture**, not an action observed in FUTERA. `E_packet`, `E_courier`, `E_sigrun` denote stable IDs resolved by the future approved binding, not display names.

| Order | Canonical / image event | Memory result |
| --- | --- | --- |
| Epoch 7, seq 100; world tick 600 | Packet held by Courier. Start render A at B100, based on prior plate P0. | A pins B100, packet identity/state and base P0. |
| Seq 101; world tick supplied by accepted rules | An authorized packet handoff to Sigrun commits as event F101. | Existing journal changes holder to Sigrun; record actual witnesses only. No time cost is invented here. |
| A completes after seq 101 | A's image still depicts packet near Courier. | Store immutable A/OA; merge against C101 = holder conflict, historical-only. Never restore Courier as holder. |
| Start and complete render B from C101 | B depicts Sigrun with packet; Courier is cropped out. | B links its actual base image and source C101. Accepted appearance has Sigrun's box; Courier is OutOfFrame, not gone. |
| Later seq 102 moves Sigrun away | An older B result arrives again. | Deduplicate job/observation; reject obsolete placement. History still answers who held the packet at F101. |

If Bandit did not witness F101, his last supported belief may remain “Courier had it”; a later authorized observation can revise that belief. He is not updated because the public picture or world database changed. The packet's private destination remains private throughout.

## NPC memory and privacy

Reuse `WitnessRecord` (observer EntityId, fact, perceived logical time), `AttributedClaim`, `MemoryEpisode`, `SecretPolicy`, and source-bound summaries. `df-knowledge::validate_belief_update` checks attributed evidence; it does not rewrite canonical truth. Current `KnowledgeGrant` and public perception scope are member-oriented, so NPC witness-to-memory adaptation is an open contract gate, not a reason to invent a member for an NPC.

Authorize retrieval for both the NPC speaker and the intended listener. A retained witness or high-confidence image never grants disclosure. Preserve original attribution and supersession links when beliefs change; do not erase an old belief or silently treat a rumor as truth. Private name/motive/recipient gates remain authoritative.

## Existing owners, bounds and recovery

- `SceneNow` + typed `SceneChange` replay in `journey/living_scene.rs` are the current scene projection. Its `changes` strings are display summaries, not a substitute for decision IDs/time/provenance. No dedicated `SceneSnapshot` type was found; reuse the existing checkpoint/journal spine rather than adding another truth store.
- `df-session` owns fenced commits; `df-persistence` already stores versioned checkpoints/decision facts in PostgreSQL. `df-assets` already has `AssetManifest` byte SHA256/publication and `AssetReference`. Extend metadata through those owners; no memory database or new clock.
- Proposed first-slice bounds: at most 32 known entities and 64 KiB per observation; 64 hot plate/merge entries and 8 MiB total per active scene cache; a query returns at most 16 plate records/64 events/64 KiB. These are candidate limits to test/freeze, not measured guarantees. Overflow returns Capacity/Incomplete, never invented/truncated authority. Page durable history; never recursively scan every parent or query every NPC per frame.
- Deduplicate by admitted job/operation and exact `(run, image-byte digest, public basis, identity/access scope, schema/model/policy)`; preserve existing unknown-call and retired-namespace protections. Same request key with different returned bytes is not the same observation. Confidence never wins a conflict with canon.
- Derive current views and indexes; cache eviction loses speed, not facts. Retain immutable metadata/assets under existing finite storage/rights policy, pin active snapshots/replay evidence, and expose unavailable/redacted history. Do not evict source facts needed for canonical memory merely because an image expires.
- Development undo recommits a saved state at a newer revision; it need not change epoch. Reuse `SavedMoment` checkpoint/social/notebook restoration and fence outstanding appearance jobs. Rebuild the active view on the restored decision lineage; discarded-future observations remain historical and inaccessible to current NPC memory. Redo reuses retained records, never rerolls or recalls a provider. Exact visual replay requires retained image/observation/cue bytes; otherwise report the missing presentation evidence.

## Queries, diagnostics and minimum slice

Separate read questions: “where is packet now?” → canonical state; “what did plate A depict?” → A/OA; “what did Bandit last witness?” → authorized witness/belief history. A source-bound query carries observer, purpose, entity IDs, logical-time/revision window, access generation and hard byte/item limits. Reauthorize before publication; no embedding or summary can widen access.

CLI timeline should join canonical revision/fact ordinal and world time with plate parent/base, exact byte hash, render basis, merge-against revision, display occurrence and terminal reason. Record accepted/unknown/conflict/stale counts and field/source IDs through existing OTEL. Keep wall times diagnostic; omit private text, raw prompts and image bytes from default logs. Distinguish missing model, unbound legacy record, unknown visibility, conflicting holder, stale branch, unavailable asset and deliberate NPC silence.

Minimum first slice: (1) stable authored harbor entity binding and canonical public snapshot, then its no-dossier NPC projection, (2) immutable plate/basis/byte metadata plus known-ID observations, (3) pure conservative three-way merge and bounded read-only timeline. The first NPC consumer uses canonical facts; image appearance requires a separately admitted witness/perception adapter before it can enter NPC memory. Keep automatic new-entity discovery, belief inference, semantic search and image regeneration outside this slice.

Open gates: entity/rename mapping; sparse fixed-scene validation; scene-entry/undo generation binding; new persistence codec and metadata placement; NPC witness adapter; appearance vocabulary/confidence and exact post-image adapter; tested bounds/retention/replay. Acceptance must include packet handoff with a late frame, actor departure, occlusion/crop, rename, undo/redo, hidden-state pairs, provider off/failure, duplicate completion and zero paid replay calls. Source sketches are not runtime acceptance.

Governing plans: long-horizon-state, interaction-engine, narrative-engine, runtime-directors, storage-architecture, runtime-reliability, asset-engine and observability. Workflow SQLite remains zero bytes and was not changed. Companion: [two-pass hydration](scene-observation-hydration.md).
