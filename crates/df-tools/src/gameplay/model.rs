use df_content::catalog::{CatalogEntry, CatalogLimits, CatalogSnapshot};
use df_engine::command_entry::{
    CommandEntryContext, CommandEntryLimits, decide_registered_command,
};
use df_model::checkpoint::*;
use df_model::commands::{CommandLimits, validate_client_command};
use df_persistence::local_demo_scope::{LocalDemoAuthority, PLAYER};
use df_persistence::{NativeRecoverySource, NativeScope};
use df_rules::ability_check::{AbilityCheckInput, resolve};
use df_rules::{DispatchRegistry, HandlerRegistration, RulesCommandHandler, RulesCommandInput};
use df_session::submission::{RepositoryError, SessionEngine};
use df_types::{
    BuildIdentity, MemberId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision,
};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

pub(super) const SOURCE: &[u8] = b"SRD5.2.1 page6 D20 Tests Ability Checks; https://media.dndbeyond.com/compendium-images/srd/5.2/SRD_CC_v5.2.1.pdf; Wizards of the Coast LLC; CC BY 4.0; normal single-d20 check only";
const CONTENT: &[u8] = b"DungeonFlux authored local journey profile6: Share the Lantern room. Two independently joined members create source-selected Dwarf Fighter/Soldier heroes; their selections and starting equipment persist. Begin at Lantern Wharf only after both builds. The physically present unnamed courier is canonical lantern-wharf-courier, with cautious-courier personality, deliver-dispatch motivation, and directional unfamiliar relationships to each hero. Asking reveals a member-private delivery clue; it grants no courier knowledge or witness. Accepted escort creates a shared courier-escort-contact fact with the acting hero as subject and an actual same-scene courier witness/known fact. The terminal escort-courier event remains shared with empty subjects for current phase admission. On a subsequent Defend command, source-bound courier-escort-reaction-policy may change only that witnessed hero's relationship to escort-supported through df-interaction::react on the prior committed contact. The change is categorical story policy, without a D&D bonus, action success, resource grant, or packet delivery. Defending begins the normal adjacent nonlethal Bandit encounter with actual initiative, player-selected attacks/SecondWind/endturn, source-qualified damage, unconscious one-HP knockout and no wall-time rest completion. Source knockout starts persist due schedules. After combat a joined member may propose an uninterrupted campaign Short Rest: one hour, no Hit Point Dice or HP healing, Fighter one-use SecondWind recharge capped2. Knockout waking retains Prone and dropped held gear and preserves the battle outcome. Versioned local-journey-rpc-3-threads-1-rest-1-inn-1-npc-1 maps begin-story/private-courier-note/escort-courier events to continued sealed-packet-delivery-thread; defend-courier/greatsword-attack/second-wind/end-turn events continue dockside-threat-thread, resolving only on the current operation accepted bandit unconscious transition. A conscious joined hero may select Harbor Inn from the current source-bound victory offer, before or after Short Rest. The admitted harbor-inn ContentEvent continues the unresolved packet thread and retains party, knowledge, resource, encounter and schedule records at unchanged logical time; this is scene continuation, not timed physical travel or delivery. Current phase admission requires exact accepted source receipts, prerequisite beats and historical native encounter outcome. Native Bandit tactics enumerate at most two source-legal authored adjacent targets in joined party order, select the first legal target without utility weights and preserve attack/draw authority. Delivery remains unresolved after combat; unmapped events consume no narrative source. Preconfigured Investigation/next-scene draft remains an earlier source lineage, not the active browser room.";

fn invalid<T: std::fmt::Debug>(error: T) -> RepositoryError {
    super::refusal_detail::cause(
        "model-value",
        format!("{}::{error:?}", std::any::type_name::<T>()),
    );
    RepositoryError::InvalidCandidate
}
pub(super) fn label(value: &str) -> Result<RevisionLabel, RepositoryError> {
    RevisionLabel::new(Some(value)).map_err(|error| {
        super::refusal_detail::cause("model-revision-label", format!("RevisionLabel::{error:?}"));
        RepositoryError::InvalidCandidate
    })
}
pub(super) fn rule() -> Result<RuleReference, RepositoryError> {
    Ok(RuleReference {
        catalog: label("srd521-journey-subset-2")?,
        source: label("srd521-page6")?,
        entry: label("d20-ability-check")?,
        clause: label("normal-proficient-total-versus-dc")?,
    })
}
pub(super) fn content(entry: &str) -> Result<ContentReference, RepositoryError> {
    Ok(ContentReference {
        package: label("harbor-investigation-demo-profile6-npc-1")?,
        entry: label(entry)?,
    })
}
pub(super) fn contents() -> Result<Vec<ContentReference>, RepositoryError> {
    let mut contents = [
        "public-appearance-v1",
        "narrative-intervention-policy",
        "harbor",
        "mara-preconfigured",
        "inspect-seal",
        "clue-loading-pier",
        "seal-obscured",
        "loading-pier",
        "lantern-wharf",
    ]
    .into_iter()
    .map(content)
    .chain(super::journey::CONTENT_ENTRIES.iter().copied().map(content))
    .chain(
        super::journey::courier_reaction::CONTENT_ENTRIES
            .iter()
            .copied()
            .map(content),
    )
    .chain(
        [super::courier_ai::DEFINITION, super::courier_ai::POLICY]
            .into_iter()
            .map(content),
    )
    .chain(
        super::beat_script_runtime::ENDING_ENTRIES
            .iter()
            .copied()
            .map(content),
    )
    .collect::<Result<Vec<_>, _>>()?;
    for entry in super::beat_script_runtime::contents() {
        let reference = content(&entry)?;
        if !contents.contains(&reference) {
            contents.push(reference);
        }
    }
    Ok(contents)
}
pub(super) fn source_manifest() -> Vec<u8> {
    [SOURCE, df_rules::local_journey::SOURCE.as_bytes()].concat()
}

struct SourceDigests {
    manifest: ContentDigest,
    handler: ContentDigest,
    content: ContentDigest,
}

fn source_digests() -> &'static SourceDigests {
    static DIGESTS: OnceLock<SourceDigests> = OnceLock::new();
    DIGESTS.get_or_init(|| {
        // All inputs are literals or include_bytes! data fixed in this compiled artifact.
        let mut manifest = Sha256::new();
        manifest.update(SOURCE);
        manifest.update(df_rules::local_journey::SOURCE.as_bytes());

        let mut handler = Sha256::new();
        for source in [
            include_bytes!("../../../df-rules/src/ability_check.rs").as_slice(),
            include_bytes!("../../../df-rules/src/influence.rs").as_slice(),
            include_bytes!("opening_help.rs").as_slice(),
            include_bytes!("../../../df-rules/src/local_journey.rs").as_slice(),
            include_bytes!("journey.rs").as_slice(),
            include_bytes!("journey/tactical.rs").as_slice(),
            include_bytes!("../../../df-rules/src/tactical/mod.rs").as_slice(),
            include_bytes!("../../../df-rules/src/srd_creatures.rs").as_slice(),
            include_bytes!("../../../df-rules/src/tactical/state.rs").as_slice(),
            include_bytes!("../../../df-rules/src/tactical/snapshot.rs").as_slice(),
            include_bytes!("../../../df-rules/src/tactical/resolve.rs").as_slice(),
            include_bytes!("../../../df-rules/src/tactical/map.rs").as_slice(),
            include_bytes!("../../../df-rules/src/tactical/weapons.rs").as_slice(),
            include_bytes!("../../../df-rules/src/tactical/build.rs").as_slice(),
            include_bytes!("../../../df-combat/src/tactical.rs").as_slice(),
            include_bytes!("beat_script.rs").as_slice(),
            include_bytes!("beat_script_runtime.rs").as_slice(),
            include_bytes!("pack.rs").as_slice(),
            include_bytes!("journey/narrative_phase.rs").as_slice(),
            include_bytes!("journey/narrative_transition.rs").as_slice(),
            include_bytes!("../../../df-narrative/src/beat_selection.rs").as_slice(),
            include_bytes!("../../../df-narrative/src/convergence.rs").as_slice(),
            include_bytes!("../../../df-content/src/narrative.rs").as_slice(),
            include_bytes!("journey/enemy_tactics.rs").as_slice(),
            include_bytes!("journey/combat_transition.rs").as_slice(),
            include_bytes!("journey/courier_reaction.rs").as_slice(),
            include_bytes!("journey/encounter_outcome.rs").as_slice(),
            include_bytes!("journey/objective_outcome.rs").as_slice(),
            include_bytes!("journey/scene_fight.rs").as_slice(),
        ] {
            handler.update(source);
        }

        let mut content = Sha256::new();
        for source in [
            CONTENT,
            include_bytes!("../../../../content/battle-scenes/harbor-journey.json").as_slice(),
            // Shop scene assaults fight on this grid, so its layout is part of the replayed content.
            include_bytes!("../../../../content/battle-scenes/merchant-shop.json").as_slice(),
            include_bytes!("../../../../content/battle-scenes/salt-quay.json").as_slice(),
            include_bytes!("../../../../content/battle-scenes/pinewood-trail.json").as_slice(),
            include_bytes!("../../../../content/battle-scenes/windward-pass.json").as_slice(),
            include_bytes!("../../../../content/battle-scenes/lamplit-market.json").as_slice(),
            include_bytes!("../../../../content/battle-scenes/rookhill-gate.json").as_slice(),
            include_bytes!("../../../../content/battle-scenes/reedmoor-boardwalk.json").as_slice(),
            include_bytes!("../../../../content/battle-scenes/thistledown-farm.json").as_slice(),
            include_bytes!("../../../../content/battle-scenes/orrery-library.json").as_slice(),
            include_bytes!("../../../../content/hero-builds/dwarf-archer.json").as_slice(),
            include_bytes!("fixtures/harbor-journey-pack-baseline.json").as_slice(),
            include_bytes!("courier_ai.rs").as_slice(),
            include_bytes!("journey/narrative_phase.rs").as_slice(),
            include_bytes!("journey/narrative_transition.rs").as_slice(),
            include_bytes!("../../../df-narrative/src/beat_selection.rs").as_slice(),
            include_bytes!("../../../df-narrative/src/convergence.rs").as_slice(),
            include_bytes!("../../../df-content/src/narrative.rs").as_slice(),
            include_bytes!("journey/enemy_tactics.rs").as_slice(),
            include_bytes!("journey/combat_transition.rs").as_slice(),
            include_bytes!("journey/courier_reaction.rs").as_slice(),
            include_bytes!("journey/encounter_outcome.rs").as_slice(),
            include_bytes!("journey/objective_outcome.rs").as_slice(),
        ] {
            content.update(source);
        }

        SourceDigests {
            manifest: ContentDigest(manifest.finalize().into()),
            handler: ContentDigest(handler.finalize().into()),
            content: ContentDigest(content.finalize().into()),
        }
    })
}

// Compatibility profile captured from bc773d4 before the sequencing-only change.
// Legacy checkpoints retain exact pins. Explicit scripts bind current handler source plus
// the script's exact pack bytes; no stored pin comparison is relaxed.
fn legacy_source_digests() -> SourceDigests {
    SourceDigests {
        // The manifest hashes the SRD citation strings, which later changed (short rest hit point
        // dice). The pre-change saves carry the digest of their own strings, so it is frozen too.
        manifest: ContentDigest([
            213, 64, 202, 171, 66, 24, 24, 103, 103, 233, 104, 178, 34, 235, 148, 140, 197, 244,
            215, 31, 12, 230, 231, 119, 7, 184, 120, 45, 43, 184, 137, 131,
        ]),
        handler: ContentDigest([
            37, 222, 214, 38, 60, 123, 108, 79, 176, 101, 224, 124, 70, 12, 56, 33, 163, 113, 121,
            181, 40, 122, 48, 126, 245, 151, 197, 6, 169, 179, 37, 149,
        ]),
        content: ContentDigest([
            207, 33, 135, 10, 158, 140, 164, 157, 62, 47, 175, 66, 240, 194, 255, 162, 105, 244,
            159, 146, 120, 123, 153, 68, 8, 215, 104, 155, 100, 246, 76, 224,
        ]),
    }
}

fn revision_digest(
    domain: &[u8],
    compiled: ContentDigest,
    revision: super::pack::Revision,
) -> ContentDigest {
    let mut digest = Sha256::new();
    digest.update(domain);
    digest.update(compiled.0);
    digest.update(revision.as_bytes());
    ContentDigest(digest.finalize().into())
}

/// The journey rules revision before the current one. Saves made under the legacy pins carry it in
/// their combat receipts and still resume.
const LEGACY_SOURCE_REVISION: &str = "SRD5.2.1-2024-local-journey-2";

/// Whether `pins` are the legacy profile's: the handler and source manifest frozen from the saves
/// made before the short rest change (see `legacy_source_digests`).
pub(super) fn is_legacy_profile(pins: &CheckpointPins) -> bool {
    let legacy = legacy_source_digests();
    pins.rules.handler_digest == legacy.handler
        && pins.rules.source_manifest_digest == legacy.manifest
}

/// Whether the pins in force are the legacy profile's. Every reader of stored history runs after
/// `validate_resume` has matched the checkpoint's stored pins to these, so this is the stored
/// pins' profile.
pub(super) fn active_profile_is_legacy() -> bool {
    pins().is_ok_and(|pins| is_legacy_profile(&pins))
}

/// Whether a stored receipt names the current rules revision, or the legacy one under legacy pins.
pub(super) fn is_recorded_source_revision(recorded: &str) -> bool {
    recorded == source_revision()
        || (recorded == LEGACY_SOURCE_REVISION && active_profile_is_legacy())
}

pub(super) fn source_revision() -> String {
    match super::pack::active() {
        Some(loaded) if !loaded.departures.is_empty() => format!(
            "{}+custom-{}{}",
            df_rules::local_journey::SOURCE_REVISION,
            &loaded.tuning_revision.to_hex()[..12],
            if loaded.tuning.tactical.is_some() {
                "; tactical.cover-centre-line: cover is adjudicated along cell centres"
            } else {
                ""
            }
        ),
        _ => df_rules::local_journey::SOURCE_REVISION.to_owned(),
    }
}

/// Configuration labels contain bounded revision prefixes selected at startup, never credentials.
pub(super) fn validate_file_revisions(
    stored: &df_types::RevisionLabel,
    current: &df_types::RevisionLabel,
) -> Result<(), std::io::Error> {
    if stored != current {
        return Err(std::io::Error::other(format!(
            "gameplay revision mismatch: stored {}; current {}; start a new database (re-run development/run-gameplay.sh without --database)",
            stored.as_str(),
            current.as_str()
        )));
    }
    Ok(())
}

pub(super) fn pins() -> Result<CheckpointPins, RepositoryError> {
    let loaded = super::pack::active();
    let legacy = legacy_source_digests();
    let digests = if loaded
        .as_ref()
        .is_some_and(|loaded| loaded.pack.beat_script.is_some() || loaded.tuning.tactical.is_some())
    {
        source_digests()
    } else {
        &legacy
    };
    // Tests without a pack stand for the current profile, so the explanation tests can check the
    // manifest digest. Production without a pack (qualification) keeps the frozen profile, which is
    // untested: `cfg!(test)` makes that case unreachable from a test. Passing the choice in
    // would touch every `pins()` caller, so it is left implicit.
    let manifest = if cfg!(test) && loaded.is_none() {
        source_digests().manifest
    } else {
        digests.manifest
    };
    let custom = loaded
        .as_ref()
        .is_some_and(|loaded| !loaded.departures.is_empty());
    let mut configuration = loaded.as_ref().map_or_else(
        || "local-gameplay-demo-config-1".to_owned(),
        |loaded| {
            format!(
                "local-gameplay-demo-config-1:t-{}:p-{}",
                &loaded.tuning_revision.to_hex()[..16],
                &loaded.pack_revision.to_hex()[..16]
            )
        },
    );
    if let Some(campaign) = loaded
        .as_ref()
        .and_then(|loaded| loaded.pack.campaign.as_ref())
    {
        configuration.push_str(&format!(
            ":c-{}",
            campaign
                .frame_sha256
                .get(..16)
                .ok_or(RepositoryError::InvalidCandidate)?
        ));
    }
    let ruleset = if custom {
        format!(
            "custom-{}",
            &loaded
                .as_ref()
                .ok_or(RepositoryError::InvalidCandidate)?
                .tuning_revision
                .to_hex()[..12]
        )
    } else {
        "srd521-journey-subset-2".to_owned()
    };
    let catalog_digest = loaded.as_ref().map_or(manifest, |loaded| {
        revision_digest(b"df-tuning-pin/1", manifest, loaded.tuning_revision)
    });
    let content_digest = loaded.as_ref().map_or(digests.content, |loaded| {
        revision_digest(b"df-pack-pin/1", digests.content, loaded.pack_revision)
    });
    Ok(CheckpointPins {
        rules: RulesPins {
            mode: if custom {
                RulesMode::DisclosedCustom
            } else {
                RulesMode::Standard2024
            },
            ruleset: label(&ruleset)?,
            catalog: label("srd521-journey-subset-2")?,
            catalog_digest,
            source_manifest: label("srd521-creation-combat-rest-reviewed-subset")?,
            source_manifest_digest: manifest,
            handler: label("source-qualified-journey-compiled-3")?,
            handler_digest: digests.handler,
        },
        content: ContentPins {
            content: label("harbor-investigation-demo-5-threads-1-rest-1-inn-1")?,
            content_digest,
            package: label("harbor-investigation-demo-profile6-npc-1")?,
            package_digest: content_digest,
        },
        build: BuildIdentity::new(
            Some(crate::BUILD_ID),
            Some(crate::BUILD_ID),
            Some(crate::BUILD_ID),
            Some(&configuration),
            Some("harbor-investigation-demo-profile6-npc-1"),
        )
        .map_err(invalid)?,
    })
}
static RUN: std::sync::RwLock<Option<RunId>> = std::sync::RwLock::new(None);

fn run_identity_error<T>(_: T) -> RepositoryError {
    super::refusal_detail::note("run-identity");
    RepositoryError::InvalidCandidate
}

pub(super) fn install_database_run(database: &str) -> Result<(), RepositoryError> {
    let bytes = if database.starts_with("df_gameplay_demo_next_") {
        let digest = Sha256::digest(database.as_bytes());
        digest[..16].try_into().map_err(run_identity_error)?
    } else {
        [0x42; 16]
    };
    *RUN.write().map_err(run_identity_error)? =
        Some(RunId::from_bytes(&bytes).map_err(run_identity_error)?);
    Ok(())
}

pub(super) fn basis() -> Result<Basis, RepositoryError> {
    Ok(Basis {
        session: SessionId::from_bytes(&[0x41; 16]).map_err(run_identity_error)?,
        run: RUN
            .read()
            .map_err(invalid)?
            .unwrap_or(RunId::from_bytes(&[0x42; 16]).map_err(invalid)?),
        revision: SessionRevision::new(RecoveryEpoch::new(1).map_err(invalid)?, 0),
    })
}
pub(super) fn actor() -> Result<EntityId, RepositoryError> {
    EntityId::from_bytes(&[0x44; 16]).map_err(invalid)
}
pub(super) fn member() -> Result<MemberId, RepositoryError> {
    MemberId::from_bytes(&PLAYER).map_err(invalid)
}
/// The largest single text a checkpoint may hold is one decision's recorded
/// receipt (`AcceptedDecision::semantic_output`: the hex of an
/// `AcceptedAction`). Sized from the fight's own caps, not from observed
/// traffic: at most 10 combatants (6 heroes, 4 enemies) act in one decision
/// (`enemy_turns` stops at the first able hero, and a battle finishes when a
/// side is wholly incapacitated), each turn records at most 12 outcomes
/// (move segments, its attack, opportunity attacks it provokes, a death
/// save), and one outcome encodes in at most 5,120 bytes (32-character UTF-8
/// names of at most 128 bytes, at most 4 UTF-8 fight lines, a move path, ids, and the source
/// revision). Hex doubles the bytes. The per-outcome and per-decision
/// budgets are proven by
/// `journey::tactical::tests::a_worst_case_receipt_fits_the_checkpoint_text_limit`
/// and the every-creature sweep.
const MAXIMUM_RECEIPT_TEXT_BYTES: usize =
    (df_rules::local_journey::MAXIMUM_PARTY + 4) * 12 * 5120 * 2;
pub(super) fn limits() -> CheckpointLimits {
    CheckpointLimits {
        maximum_records: if super::journey::tactical::enabled() {
            8192
        } else {
            // Six accepted heroes retain bounded public-look chunks in both character
            // and creation records, in addition to their source-bound build choices.
            3072
        },
        maximum_text_bytes: MAXIMUM_RECEIPT_TEXT_BYTES,
        // Generated places, their people and recorded facts accumulate over a session.
        maximum_total_text_bytes: 4 * 1024 * 1024,
        maximum_retained_bytes: 8 * 1024 * 1024,
    }
}
pub(super) fn recovery() -> Result<NativeRecoverySource, RepositoryError> {
    Ok(NativeRecoverySource {
        rules: vec![rule()?, super::journey::rule()?],
        content: contents()?,
        resources: super::journey::resources()?,
        resource_resolver: Some(super::journey::resources_for),
        assets: vec![],
        asset_resolver: Some(creator_portrait_assets),
        limits: limits(),
    })
}
pub(super) fn checkpoint(basis: Basis, state: GameState) -> Result<Checkpoint, RepositoryError> {
    let resources = super::journey::resources_for(&state)?;
    let assets = creator_portrait_assets(&state).inspect_err(|_| {
        super::refusal_detail::note("checkpoint-creator-portrait-assets");
    })?;
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        basis,
        pins()?,
        state,
        ReferenceInventory {
            rules: &[rule()?, super::journey::rule()?],
            content: &contents()?,
            resources: &resources,
            assets: &assets,
        },
        limits(),
    )
    .map_err(|error| {
        super::refusal_detail::cause("checkpoint-validation", format!("{error:?}"));
        super::actor::record_audit_line(
            "gameplay.checkpoint_invalid",
            format!("CHECKPOINT invalid error={error:?}"),
        );
        invalid(error)
    })
}

/// Only server-attached creator metadata can register an appearance during recovery.
/// The wire rejects these choice fields from clients; approved bytes are checked separately
/// when attached/served. This pure resolver never consults caches or regenerates missing media.
pub(super) fn creator_portrait_assets(
    state: &GameState,
) -> Result<Vec<AssetReference>, RepositoryError> {
    let maximum = limits().maximum_records;
    if state.characters.len() > maximum || state.continuity.canonical_packs.len() > maximum {
        super::refusal_detail::cause(
            "creator-portrait-assets",
            format!("Capacity budget=records limit={maximum}"),
        );
        return Err(RepositoryError::Capacity);
    }
    let mut records = state.characters.len();
    for character in &state.characters {
        records = records
            .checked_add(character.choices.len())
            .ok_or(RepositoryError::Capacity)?;
        if records > maximum {
            super::refusal_detail::cause(
                "creator-portrait-assets",
                format!("Capacity budget=records limit={maximum}"),
            );
            return Err(RepositoryError::Capacity);
        }
    }
    for pack in &state.continuity.canonical_packs {
        records = records
            .checked_add(pack.identities.len())
            .ok_or(RepositoryError::Capacity)?;
        if records > maximum {
            super::refusal_detail::cause(
                "creator-portrait-assets",
                format!("Capacity budget=records limit={maximum}"),
            );
            return Err(RepositoryError::Capacity);
        }
        for identity in &pack.identities {
            records = records
                .checked_add(identity.appearances.len())
                .ok_or(RepositoryError::Capacity)?;
            if records > maximum {
                super::refusal_detail::cause(
                    "creator-portrait-assets",
                    format!("Capacity budget=records limit={maximum}"),
                );
                return Err(RepositoryError::Capacity);
            }
        }
    }
    let mut assets = Vec::<AssetReference>::new();
    assets
        .try_reserve_exact(state.characters.len())
        .map_err(|_| RepositoryError::Capacity)?;
    for character in &state.characters {
        let field = |name: &'static str| -> Result<Option<&str>, RepositoryError> {
            let mut choices = character
                .choices
                .iter()
                .filter(|choice| choice.offer.as_str() == name);
            let selected = choices.next().map(|choice| choice.selected.as_str());
            if choices.next().is_some() {
                super::refusal_detail::cause(
                    "creator-portrait-assets",
                    format!("DuplicateField field={name}"),
                );
                return Err(RepositoryError::InvalidCandidate);
            }
            Ok(selected)
        };
        let metadata = [
            field("identity-portrait:key")?,
            field("identity-portrait:digest")?,
            field("identity-portrait:bytes")?,
            field("identity-portrait:revision")?,
        ];
        if metadata.iter().all(Option::is_none) {
            continue;
        }
        let [Some(key), Some(digest), Some(bytes), Some(revision)] = metadata else {
            let fields = ["key", "digest", "bytes", "revision"];
            let field = metadata
                .iter()
                .position(Option::is_none)
                .map_or("metadata", |index| fields[index]);
            super::refusal_detail::cause(
                "creator-portrait-assets",
                format!("MissingField field={field}"),
            );
            return Err(RepositoryError::InvalidCandidate);
        };
        let lowercase_hex = |text: &str| {
            text.len() == 64
                && text
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        };
        let reject = |field: &'static str| {
            super::refusal_detail::cause(
                "creator-portrait-assets",
                format!("InvalidField field={field}"),
            );
            RepositoryError::InvalidCandidate
        };
        let bytes = bytes.parse::<u64>().map_err(|_| reject("bytes"))?;
        if !lowercase_hex(key) {
            return Err(reject("key"));
        }
        if !lowercase_hex(digest) {
            return Err(reject("digest"));
        }
        if bytes == 0 || bytes > 6 * 1024 * 1024 {
            return Err(reject("bytes"));
        }
        if revision.parse::<u64>().map_err(|_| reject("revision"))? == 0 {
            return Err(reject("revision"));
        }
        let asset = super::hero_identity::approved_portrait_asset(state, character.entity)
            .ok_or_else(|| reject("approved-appearance"))?;
        if assets
            .iter()
            .any(|registered| registered.key == asset.key && registered != asset)
        {
            return Err(reject("conflicting-asset"));
        }
        if !assets.contains(asset) {
            assets.push(asset.clone());
        }
    }
    for pack in &state.continuity.canonical_packs {
        for identity in &pack.identities {
            for asset in &identity.appearances {
                if super::hero_identity::approved_portrait_asset(state, identity.entity)
                    != Some(asset)
                {
                    let field =
                        super::hero_identity::approved_portrait_asset(state, identity.entity)
                            .map_or("appearance-owner", |approved| {
                                if approved.kind != asset.kind {
                                    "appearance-kind"
                                } else if approved.digest != asset.digest {
                                    "appearance-digest"
                                } else if approved.byte_length != asset.byte_length {
                                    "appearance-bytes"
                                } else {
                                    "appearance-key"
                                }
                            });
                    super::refusal_detail::cause(
                        "creator-portrait-assets",
                        format!("Mismatch field={field}"),
                    );
                    return Err(RepositoryError::InvalidCandidate);
                }
            }
        }
    }
    Ok(assets)
}

pub(super) fn initial() -> Result<Checkpoint, RepositoryError> {
    let harbor = content("harbor")?;
    let state = GameState {
        mode: ExecutionMode::PreparedOnly,
        logical_time: LogicalTime {
            ticks: 0,
            ticks_per_second: 1,
        },
        members: vec![MembershipLink {
            member: member()?,
            character: Some(actor()?),
        }],
        entities: vec![WorldEntity {
            id: actor()?,
            definition: content("mara-preconfigured")?,
            location: None,
            position: None,
            identity_revision: label("mara-preconfigured-1")?,
        }],
        characters: vec![CharacterState {
            entity: actor()?,
            build: content("mara-preconfigured")?,
            owner: member()?,
            choices: vec![],
        }],
        resources: vec![],
        inventory: vec![],
        facts: vec![],
        draws: vec![],
        decisions: vec![],
        pending: vec![],
        intents: vec![],
        timers: vec![],
        active_effects: vec![],
        knowledge: vec![],
        beliefs: vec![],
        memories: vec![],
        schedules: vec![],
        threats: vec![],
        relationships: vec![],
        conversations: vec![],
        obligations: vec![],
        narrative: NarrativeState {
            definition: harbor.clone(),
            active_beats: vec![harbor.clone()],
            completed_beats: vec![],
            open_threads: vec![],
            accepted_facts: vec![],
            remaining_budget: 1,
        },
        encounters: vec![],
        activity: vec![],
        tempo: TempoState {
            policy: harbor,
            presentation_ticks: 0,
            intensity: 0,
            inertia: 0,
            fatigue: vec![],
        },
        presentation: vec![],
        continuity: ContinuityState {
            creation: vec![],
            simulation: vec![],
            catch_up: None,
            environment: vec![],
            travel: vec![],
            witnesses: vec![],
            rumors: vec![],
            journal: vec![],
            summaries: vec![],
            retrieval: vec![],
            retrieved: vec![],
            consolidation: vec![],
            npcs: vec![],
            hooks: vec![],
            arcs: vec![],
            remote: None,
            presence: vec![],
            audio: None,
            private_offers: vec![],
            knowledge_cues: vec![],
            moments: vec![],
            demands: vec![],
            asset_jobs: vec![],
            asset_dependencies: vec![],
            canonical_packs: vec![],
            shots: vec![],
            prefetch: None,
            scenes: vec![],
            item_origins: vec![],
            bookends: vec![],
            exports: vec![],
            critical_cues: vec![],
            content_candidates: vec![],
            content_admissions: vec![],
            recovery: RecoveryState {
                origin: None,
                retired_epochs: vec![],
                lost_ranges: vec![],
                suppression_generation: 0,
                redacted_records: vec![],
                unavailable_sources: vec![],
            },
        },
    };
    checkpoint(basis()?, state)
}

pub(super) fn random<const N: usize>() -> Result<[u8; N], RepositoryError> {
    let mut bytes = [0; N];
    fill_os_entropy(&mut bytes).map_err(|_| RepositoryError::Unavailable)?;
    if N > 1 && bytes.iter().all(|byte| *byte == 0) {
        return Err(RepositoryError::Unavailable);
    }
    Ok(bytes)
}

/// Fills `bytes` from the operating system's cryptographic random source.
#[cfg(unix)]
pub(super) fn fill_os_entropy(bytes: &mut [u8]) -> std::io::Result<()> {
    use std::io::Read;
    std::fs::File::open("/dev/urandom").and_then(|mut file| file.read_exact(bytes))
}

// Windows has no `/dev/urandom`, so entropy comes from the operating system's
// cryptographic random source through the `getrandom` crate instead.
#[cfg(windows)]
pub(super) fn fill_os_entropy(bytes: &mut [u8]) -> std::io::Result<()> {
    getrandom::getrandom(bytes).map_err(|error| std::io::Error::other(format!("{error:?}")))
}
fn draw_d20() -> Result<u8, RepositoryError> {
    u8::try_from(super::dice::draw(20)?).map_err(|_| RepositoryError::Unavailable)
}
pub(super) struct HarborHandler {
    pins: CheckpointPins,
}
impl RulesCommandHandler for HarborHandler {
    type Rejection = RepositoryError;
    fn pins(&self) -> &CheckpointPins {
        &self.pins
    }
    fn stage(
        &self,
        input: RulesCommandInput<'_>,
        current: &Checkpoint,
    ) -> Result<Checkpoint, RepositoryError> {
        let GameInput::Game(command) = input.command else {
            super::refusal_detail::cause("harbor-stage", "ExpectedGameInput".to_owned());
            return Err(RepositoryError::InvalidCandidate);
        };
        let GameCommand::ProposeAction {
            actor: who,
            action,
            targets,
            choices,
        } = &command.command
        else {
            super::refusal_detail::cause("harbor-stage", "ExpectedProposeAction".to_owned());
            return Err(RepositoryError::InvalidCandidate);
        };
        if *who != actor()?
            || command.member != member()?
            || *action != content("inspect-seal")?
            || !targets.is_empty()
            || !choices.is_empty()
            || !current.state().decisions.is_empty()
        {
            super::refusal_detail::cause("harbor-stage", "ActorOfferBinding".to_owned());
            return Err(RepositoryError::InvalidCandidate);
        }
        let [draw] = input.supplied_draws else {
            super::refusal_detail::cause("harbor-stage", "SuppliedDrawCount".to_owned());
            return Err(RepositoryError::InvalidCandidate);
        };
        if draw.sides != 20 || draw.source != rule()? {
            super::refusal_detail::cause("harbor-stage", "SuppliedDrawBinding".to_owned());
            return Err(RepositoryError::InvalidCandidate);
        }
        let result = resolve(AbilityCheckInput {
            die: u8::try_from(draw.value).map_err(invalid)?,
            ability_modifier: 2,
            proficiency_bonus: 2,
            difficulty_class: 15,
        })
        .map_err(invalid)?;
        let mut next = current.basis();
        next.revision = next.revision.next_sequence().map_err(invalid)?;
        let mut state = current.state().clone();
        let draw_fact = FactId::from_bytes(&[0x51; 16]).map_err(invalid)?;
        let result_fact = FactId::from_bytes(&[0x52; 16]).map_err(invalid)?;
        state.draws.push(draw.clone());
        state.facts.push(GameFact {
            id: draw_fact,
            revision: next.revision,
            operation: command.operation,
            ordinal: 0,
            cause: None,
            audience: AudienceScope::Members(vec![member()?]),
            value: FactValue::DrawAccepted {
                operation: command.operation,
                ordinal: 0,
            },
        });
        state.facts.push(GameFact {
            id: result_fact,
            revision: next.revision,
            operation: command.operation,
            ordinal: 1,
            cause: Some(draw_fact),
            audience: AudienceScope::Members(vec![member()?]),
            value: FactValue::ContentEvent {
                definition: content(if result.succeeded {
                    "clue-loading-pier"
                } else {
                    "seal-obscured"
                })?,
                subjects: vec![actor()?],
            },
        });
        state.decisions.push(AcceptedDecision {
            operation: command.operation,
            revision: next.revision,
            facts: vec![draw_fact, result_fact],
            draws: vec![0],
            effects: vec![],
            source_policy: label("srd521-journey-subset-2")?,
            semantic_output: None,
        });
        checkpoint(next, state)
    }
}

pub(super) struct HarborEngine;

pub(super) fn stage_registered_journey(
    current: &Checkpoint,
    input: &GameInput,
) -> Result<Checkpoint, RepositoryError> {
    let GameInput::Game(command) = input else {
        super::refusal_detail::cause("harbor-stage", "ExpectedGameInput".to_owned());
        return Err(RepositoryError::InvalidCandidate);
    };
    let staged = super::journey::stage(current, input)?;
    super::courier_ai::inspect_candidate(current, &staged, command).map_err(invalid)?;
    Ok(staged)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HarborScene {
    Harbor,
    LoadingPier,
    LanternWharf,
    HarborInn,
}
impl HarborScene {
    pub fn entry(self) -> &'static str {
        match self {
            Self::Harbor => "harbor",
            Self::LoadingPier => "loading-pier",
            Self::LanternWharf => "lantern-wharf",
            Self::HarborInn => "harbor-inn",
        }
    }
    pub fn from_entry(entry: &str) -> Result<Self, RepositoryError> {
        match entry {
            "harbor" => Ok(Self::Harbor),
            "loading-pier" => Ok(Self::LoadingPier),
            "lantern-wharf" => Ok(Self::LanternWharf),
            "harbor-inn" => Ok(Self::HarborInn),
            _ => Err(RepositoryError::InvalidCandidate),
        }
    }
}

pub(super) fn scene(current: &Checkpoint) -> Result<HarborScene, RepositoryError> {
    let [active] = current.state().narrative.active_beats.as_slice() else {
        super::refusal_detail::cause("harbor-stage", "ActiveBeatCount".to_owned());
        return Err(RepositoryError::InvalidCandidate);
    };
    HarborScene::from_entry(active.entry.as_str())
}

pub(super) fn investigation_succeeded(
    current: &Checkpoint,
) -> Result<Option<bool>, RepositoryError> {
    if current.state().decisions.is_empty() {
        return Ok(None);
    }
    let result = current
        .state()
        .facts
        .iter()
        .find(|fact| {
            matches!(&fact.value, FactValue::ContentEvent { definition, .. }
            if definition.entry.as_str() == "clue-loading-pier"
                || definition.entry.as_str() == "seal-obscured")
        })
        .ok_or(RepositoryError::InvalidCandidate)?;
    match &result.value {
        FactValue::ContentEvent { definition, .. } => {
            Ok(Some(*definition == content("clue-loading-pier")?))
        }
        _ => Err(RepositoryError::InvalidCandidate),
    }
}

pub(super) fn destinations(current: &Checkpoint) -> Result<Vec<HarborScene>, RepositoryError> {
    if scene(current)? != HarborScene::Harbor {
        return Ok(Vec::new());
    }
    match investigation_succeeded(current)? {
        None => Ok(Vec::new()),
        Some(true) => Ok(vec![HarborScene::LoadingPier, HarborScene::LanternWharf]),
        Some(false) => Ok(vec![HarborScene::LanternWharf, HarborScene::HarborInn]),
    }
}

fn stage_scene(current: &Checkpoint, input: &GameInput) -> Result<Checkpoint, RepositoryError> {
    let GameInput::Game(command) = input else {
        super::refusal_detail::cause("harbor-stage", "ExpectedGameInput".to_owned());
        return Err(RepositoryError::InvalidCandidate);
    };
    let GameCommand::ProposeAction {
        actor: who,
        action,
        targets,
        choices,
    } = &command.command
    else {
        super::refusal_detail::cause("harbor-stage", "ExpectedProposeAction".to_owned());
        return Err(RepositoryError::InvalidCandidate);
    };
    let [(input_id, selected)] = choices.as_slice() else {
        super::refusal_detail::cause("harbor-stage", "DestinationSelection".to_owned());
        return Err(RepositoryError::InvalidCandidate);
    };
    let destination = HarborScene::from_entry(selected.as_str())?;
    if *who != actor()?
        || command.member != member()?
        || *action != content("choose-harbor-scene")?
        || !targets.is_empty()
        || *input_id != label("harbor-destination")?
        || current.state().decisions.len() != 1
        || !destinations(current)?.contains(&destination)
    {
        super::refusal_detail::cause("harbor-stage", "DestinationOfferBinding".to_owned());
        return Err(RepositoryError::InvalidCandidate);
    }
    validate_client_command(
        input,
        current,
        ReferenceInventory {
            rules: &[rule()?],
            content: &contents()?,
            resources: &super::journey::resources_for(current.state())?,
            assets: &creator_portrait_assets(current.state())?,
        },
        CommandLimits {
            maximum_records: 16,
            maximum_text_bytes: 128,
            maximum_retained_bytes: 8192,
        },
    )
    .map_err(invalid)?;
    let mut next = current.basis();
    next.revision = next.revision.next_sequence().map_err(invalid)?;
    let mut state = current.state().clone();
    let fact = FactId::from_bytes(&[0x53; 16]).map_err(invalid)?;
    state.facts.push(GameFact {
        id: fact,
        revision: next.revision,
        operation: command.operation,
        ordinal: 0,
        cause: Some(FactId::from_bytes(&[0x52; 16]).map_err(invalid)?),
        audience: AudienceScope::Shared,
        value: FactValue::ContentEvent {
            definition: content(destination.entry())?,
            subjects: vec![actor()?],
        },
    });
    state.decisions.push(AcceptedDecision {
        operation: command.operation,
        revision: next.revision,
        facts: vec![fact],
        draws: vec![],
        effects: vec![],
        source_policy: label("harbor-authored-scene-choice-1")?,
        semantic_output: Some(destination.entry().to_owned()),
    });
    state.narrative.completed_beats.push(content("harbor")?);
    state.narrative.active_beats = vec![content(destination.entry())?];
    state.narrative.accepted_facts.push(fact);
    checkpoint(next, state)
}

impl HarborEngine {
    /// The admitted native gameplay body. Session validates its scope before entering here.
    pub(super) fn decide_admitted(
        &mut self,
        current: &Checkpoint,
        input: &GameInput,
        operation: df_types::OperationId,
    ) -> Result<Checkpoint, RepositoryError> {
        if let GameInput::Job(completion) = input {
            return super::courier_ai::stage_completion(current, completion, operation)
                .map_err(invalid);
        }
        if let GameInput::Game(command) = input
            && let GameCommand::ProposeAction { action, .. } = &command.command
        {
            if action.entry.as_str() == "join-room" {
                return super::journey::stage_join(current, input);
            }
            if super::journey::CONTENT_ENTRIES.contains(&action.entry.as_str())
                || action.entry.as_str() == super::beat_script_runtime::CHOICE_ACTION
                || action.entry.as_str() == super::beat_script_runtime::RESOLVE_ACTION
            {
                return stage_registered_journey(current, input);
            }
        }

        if matches!(input, GameInput::Game(CommandInput {
            command: GameCommand::ProposeAction { action, .. }, ..
        }) if *action == content("choose-harbor-scene")?)
        {
            return stage_scene(current, input);
        }
        // Lookup runs in DurableOwner first, so a retry never reaches this draw owner.
        if !current.state().decisions.is_empty() {
            super::refusal_detail::cause("harbor-stage", "PriorDecision".to_owned());
            return Err(RepositoryError::InvalidCandidate);
        }
        let GameInput::Game(command) = input else {
            super::refusal_detail::cause("harbor-stage", "ExpectedGameInput".to_owned());
            return Err(RepositoryError::InvalidCandidate);
        };
        let source = rule()?;
        let pins = pins()?;
        let selector = label("harbor-normal-ability-check-1")?;
        let handler = HarborHandler { pins: pins.clone() };
        let manifest = source_manifest();
        let entries = [CatalogEntry::new(&source, &manifest)];
        let catalog = CatalogSnapshot::from_published(
            &pins.rules.catalog,
            &pins,
            &manifest,
            &entries,
            CatalogLimits {
                max_complete_bytes: 4096,
                max_entries: 1,
                max_item_bytes: 4096,
                max_total_item_bytes: 4096,
            },
        )
        .map_err(invalid)?;
        let registrations = [HandlerRegistration::new(&selector, &source, &handler)];
        let registry =
            DispatchRegistry::from_catalog(catalog, &registrations, 1).map_err(invalid)?;
        let draw = ActualDraw {
            operation: command.operation,
            ordinal: 0,
            resolution: ResolutionId::from_bytes(&[0x61; 16]).map_err(invalid)?,
            window: WindowId::from_bytes(&[0x62; 16]).map_err(invalid)?,
            sides: 20,
            value: u32::from(draw_d20()?),
            source: source.clone(),
        };
        decide_registered_command(
            RulesCommandInput {
                command: input,
                supplied_draws: &[draw],
            },
            current,
            CommandEntryContext {
                current_basis: current.basis(),
                admitted_pins: &pins,
                inventory: ReferenceInventory {
                    rules: std::slice::from_ref(&source),
                    content: &contents()?,
                    resources: &super::journey::resources_for(current.state())?,
                    assets: &creator_portrait_assets(current.state())?,
                },
                limits: CommandEntryLimits {
                    command: CommandLimits {
                        maximum_records: 16,
                        maximum_text_bytes: 128,
                        maximum_retained_bytes: 8192,
                    },
                    maximum_staged_bytes: 1024 * 1024,
                },
            },
            &registry,
            &selector,
            &source,
        )
        .map_err(invalid)
    }
}

impl SessionEngine<NativeScope<LocalDemoAuthority>> for HarborEngine {
    fn decide(
        &mut self,
        current: &Checkpoint,
        scope: &NativeScope<LocalDemoAuthority>,
        input: &GameInput,
    ) -> Result<Checkpoint, RepositoryError> {
        use df_session::submission::OperationScope;
        scope.validate_input(input)?;
        self.decide_admitted(current, input, scope.operation())
    }
    fn decide_transition(
        &mut self,
        current: &Checkpoint,
        scope: &NativeScope<LocalDemoAuthority>,
        input: &GameInput,
    ) -> df_model::transition::TransitionResult<RepositoryError> {
        use df_model::transition::TransitionResult;
        use df_session::submission::OperationScope;
        if let Err(error) = scope.validate_input(input) {
            super::refusal_detail::cause("engine-input-validation", format!("{error:?}"));
            return TransitionResult::Rejected(error);
        }
        match self.decide_admitted(current, input, scope.operation()) {
            Ok(candidate) => staged_transition(current, candidate, scope.operation()),
            Err(error) => TransitionResult::Rejected(error),
        }
    }
    fn validate_recovery(&mut self, current: &Checkpoint) -> Result<(), RepositoryError> {
        current
            .validate_resume(current.basis(), &pins()?)
            .map(|_| ())
            .map_err(invalid)
    }
}

/// The journey source returns a detached checkpoint; this same classifier is used before the
/// serialized session owner commits either an accepted decision or a suspended continuation.
pub(super) fn staged_transition(
    current: &Checkpoint,
    candidate: Checkpoint,
    operation: df_types::OperationId,
) -> df_model::transition::TransitionResult<RepositoryError> {
    use df_model::transition::TransitionResult;
    TransitionResult::classify(current, candidate, operation).unwrap_or_else(|error| {
        super::refusal_detail::cause("staged-transition", format!("{error:?}"));
        TransitionResult::Rejected(RepositoryError::InvalidCandidate)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use df_types::OperationId;

    fn staged(die: u32, sides: u32) -> Result<Checkpoint, RepositoryError> {
        let current = initial()?;
        let operation = OperationId::from_bytes(&[0x81; 16]).map_err(invalid)?;
        let command = GameInput::Game(CommandInput {
            basis: current.basis(),
            operation,
            member: member()?,
            observed_revision: current.basis().revision,
            command: GameCommand::ProposeAction {
                actor: actor()?,
                action: content("inspect-seal")?,
                targets: vec![],
                choices: vec![],
            },
        });
        let draw = ActualDraw {
            operation,
            ordinal: 0,
            resolution: ResolutionId::from_bytes(&[0x61; 16]).map_err(invalid)?,
            window: WindowId::from_bytes(&[0x62; 16]).map_err(invalid)?,
            sides,
            value: die,
            source: rule()?,
        };
        df_rules::stage_handler(
            &HarborHandler { pins: pins()? },
            &pins()?,
            RulesCommandInput {
                command: &command,
                supplied_draws: &[draw],
            },
            &current,
            1024 * 1024,
        )
        .map_err(invalid)
    }

    #[test]
    fn invalid_checkpoint_diagnostic_reaches_the_final_refusal_line() {
        let current = initial().unwrap();
        let mut state = current.state().clone();
        state.logical_time.ticks_per_second = 0;
        let _ = super::super::refusal_detail::take();
        let _ = df_session::submission::take_refusal_diagnostic();
        assert!(checkpoint(current.basis(), state).is_err());
        let detail = df_session::submission::take_refusal_diagnostic().unwrap();
        assert_eq!(detail.step, "checkpoint-validation");
        assert_eq!(detail.cause, "InvalidTime");
        df_session::submission::note_refusal_diagnostic(detail.step, detail.cause);
        super::super::actor::record_refusal(
            df_protocol::common::GameplayActionKind::CreateCharacter as i32,
            "InvalidCandidate",
            "refused",
        );
        assert!(
            super::super::actor::audit_lines_containing("REFUSAL action=CreateCharacter")
                .iter()
                .any(|line| line.contains(
                    "step=checkpoint-validation outer=checkpoint-validation cause=InvalidTime"
                ))
        );
    }

    #[test]
    fn success_and_failure_commit_distinct_information_facts_with_one_actual_draw() {
        let success = staged(11, 20).unwrap();
        let failure = staged(10, 20).unwrap();
        for checkpoint in [&success, &failure] {
            assert_eq!(checkpoint.basis().revision.sequence(), 1);
            assert_eq!(checkpoint.state().draws.len(), 1);
            assert_eq!(checkpoint.state().decisions.len(), 1);
            assert_eq!(checkpoint.state().facts.len(), 2);
        }
        assert!(
            matches!(&success.state().facts[1].value,FactValue::ContentEvent { definition,.. } if *definition==content("clue-loading-pier").unwrap())
        );
        assert!(
            matches!(&failure.state().facts[1].value,FactValue::ContentEvent { definition,.. } if *definition==content("seal-obscured").unwrap())
        );
        assert!(staged(11, 12).is_err());
    }

    fn choose(
        current: &Checkpoint,
        destination: HarborScene,
    ) -> Result<Checkpoint, RepositoryError> {
        stage_scene(
            current,
            &GameInput::Game(CommandInput {
                basis: current.basis(),
                operation: OperationId::from_bytes(&[0x82; 16]).map_err(invalid)?,
                member: member()?,
                observed_revision: current.basis().revision,
                command: GameCommand::ProposeAction {
                    actor: actor()?,
                    action: content("choose-harbor-scene")?,
                    targets: vec![],
                    choices: vec![(label("harbor-destination")?, label(destination.entry())?)],
                },
            }),
        )
    }

    #[test]
    fn each_outcome_has_two_real_scene_choices_and_preserves_original_roll() {
        for (die, allowed) in [
            (
                11,
                vec![HarborScene::LoadingPier, HarborScene::LanternWharf],
            ),
            (10, vec![HarborScene::LanternWharf, HarborScene::HarborInn]),
        ] {
            let checked = staged(die, 20).unwrap();
            assert_eq!(destinations(&checked).unwrap(), allowed);
            for destination in allowed {
                let next = choose(&checked, destination).unwrap();
                assert_eq!(scene(&next).unwrap(), destination);
                assert_eq!(next.basis().revision.sequence(), 2);
                assert_eq!(next.state().draws, checked.state().draws);
                assert_eq!(next.state().logical_time, checked.state().logical_time);
                assert_eq!(next.state().resources, checked.state().resources);
                assert_eq!(next.state().facts.len(), 3);
                assert_eq!(next.state().decisions.len(), 2);
                assert_eq!(next.state().decisions[0], checked.state().decisions[0]);
                assert!(next.state().decisions[1].draws.is_empty());
                assert_eq!(
                    next.state().decisions[1].semantic_output.as_deref(),
                    Some(destination.entry())
                );
                assert_eq!(next.state().facts[2].audience, AudienceScope::Shared);
                assert_eq!(
                    next.state().narrative.completed_beats,
                    vec![content("harbor").unwrap()]
                );
                assert_eq!(
                    next.state().narrative.accepted_facts,
                    vec![next.state().facts[2].id]
                );
                assert!(destinations(&next).unwrap().is_empty());
            }
        }
    }

    #[test]
    fn failed_seal_does_not_unlock_private_pier_lead() {
        let checked = staged(10, 20).unwrap();
        let before = checked.clone();
        assert!(choose(&checked, HarborScene::LoadingPier).is_err());
        assert_eq!(checked, before);
        assert_eq!(investigation_succeeded(&checked).unwrap(), Some(false));
    }

    #[test]
    fn next_scene_requires_completed_check_and_cannot_be_selected_twice() {
        let initial = initial().unwrap();
        assert!(choose(&initial, HarborScene::LanternWharf).is_err());
        let checked = staged(11, 20).unwrap();
        let selected = choose(&checked, HarborScene::LoadingPier).unwrap();
        assert!(choose(&selected, HarborScene::LanternWharf).is_err());
        assert_eq!(selected.state().draws, checked.state().draws);
    }
}

#[cfg(test)]
mod diagnostic_tests;

#[cfg(test)]
mod tuning_pin_tests {
    use super::super::pack;
    use super::*;
    const PACK: &[u8] = include_bytes!("../../../../content/harbor-journey/pack.json");
    const TUNING: &[u8] = include_bytes!("fixtures/harbor-journey-tuning-baseline.json");
    const LEGACY_PACK: &[u8] = include_bytes!("fixtures/legacy-beats/pack.json");
    const SHOP_PACK: &[u8] = include_bytes!("../../../../content/merchant-shop/pack.json");
    const SHOP_TUNING: &[u8] = include_bytes!("../../../../content/merchant-shop/tuning.json");

    #[test]
    fn the_legacy_revision_is_the_one_before_the_current_revision() {
        let number = |revision: &str| {
            let (prefix, number) = revision.rsplit_once('-').unwrap();
            (prefix.to_owned(), number.parse::<u32>().unwrap())
        };
        let (prefix, current) = number(df_rules::local_journey::SOURCE_REVISION);
        let (legacy_prefix, legacy) = number(LEGACY_SOURCE_REVISION);
        assert_eq!(prefix, legacy_prefix);
        assert_eq!(legacy + 1, current);
    }
    #[test]
    fn pins_cite_the_frozen_manifest_only_under_the_legacy_profile() {
        let current = source_digests().manifest;
        let frozen = legacy_source_digests().manifest;
        assert_ne!(current, frozen);
        // A pack without script or tactical tuning is the legacy profile.
        let legacy =
            pack::with_test_pack(pack::load_from_bytes(LEGACY_PACK, TUNING).unwrap(), || {
                pins().unwrap()
            });
        assert_eq!(legacy.rules.source_manifest_digest, frozen);
        assert!(is_legacy_profile(&legacy));
        // A scripted pack is the current profile.
        let scripted = pack::with_test_pack(
            pack::load_from_bytes(SHOP_PACK, SHOP_TUNING).unwrap(),
            || pins().unwrap(),
        );
        assert_eq!(scripted.rules.source_manifest_digest, current);
        assert!(!is_legacy_profile(&scripted));
        // Tests without a pack stand for the current profile.
        let unpacked = pins().unwrap();
        assert_eq!(unpacked.rules.source_manifest_digest, current);
        assert!(!is_legacy_profile(&unpacked));
    }
    #[test]
    fn stored_receipts_name_the_current_revision_and_the_legacy_one_only_under_legacy_pins() {
        let near_misses = [
            "SRD5.2.1-2024-local-journey-1",
            "SRD5.2.1-2024-local-journey-20",
            "SRD5.2.1-2024-local-journey-2+custom-0123456789ab",
            " SRD5.2.1-2024-local-journey-2",
            "SRD5.2.1-2024-local-journey-2 ",
            "srd5.2.1-2024-local-journey-2",
            "SRD5.2.1-2024-local-journey-",
            "",
        ];
        let check = |accepts_legacy: bool| {
            assert!(is_recorded_source_revision(&source_revision()));
            assert_eq!(
                is_recorded_source_revision(LEGACY_SOURCE_REVISION),
                accepts_legacy
            );
            for near_miss in near_misses {
                assert!(!is_recorded_source_revision(near_miss), "{near_miss:?}");
            }
        };
        pack::with_test_pack(pack::load_from_bytes(LEGACY_PACK, TUNING).unwrap(), || {
            check(true);
        });
        pack::with_test_pack(
            pack::load_from_bytes(SHOP_PACK, SHOP_TUNING).unwrap(),
            || check(false),
        );
        check(false);
    }
    #[test]
    fn identical_files_bind_stable_standard_pins_and_each_byte_edit_changes_its_digest() {
        let baseline = pins().unwrap();
        let standard = pack::with_test_pack(pack::load_from_bytes(PACK, TUNING).unwrap(), || {
            pins().unwrap()
        });
        let again = pack::with_test_pack(pack::load_from_bytes(PACK, TUNING).unwrap(), || {
            pins().unwrap()
        });
        assert_eq!(standard, again);
        assert_eq!(standard.rules.mode, RulesMode::Standard2024);
        assert_eq!(standard.rules.ruleset, baseline.rules.ruleset);
        assert_eq!(source_revision(), df_rules::local_journey::SOURCE_REVISION);
        let tuning_edit = [TUNING, b" "].concat();
        let changed =
            pack::with_test_pack(pack::load_from_bytes(PACK, &tuning_edit).unwrap(), || {
                pins().unwrap()
            });
        assert_ne!(changed.rules.catalog_digest, standard.rules.catalog_digest);
        assert_eq!(changed.content, standard.content);
        let pack_edit = [PACK, b" "].concat();
        let changed =
            pack::with_test_pack(pack::load_from_bytes(&pack_edit, TUNING).unwrap(), || {
                pins().unwrap()
            });
        assert_ne!(
            changed.content.content_digest,
            standard.content.content_digest
        );
        assert_ne!(
            changed.content.package_digest,
            standard.content.package_digest
        );
        assert_eq!(changed.rules, standard.rules);
        let stored = standard
            .build
            .revision(df_types::BuildRevision::Configuration);
        let current = changed
            .build
            .revision(df_types::BuildRevision::Configuration);
        let refusal = validate_file_revisions(stored, current)
            .unwrap_err()
            .to_string();
        assert!(refusal.contains(stored.as_str()) && refusal.contains(current.as_str()));
        assert!(refusal.contains("start a new database"));
        assert!(validate_file_revisions(stored, stored).is_ok());
    }
    #[test]
    fn departures_disclose_custom_rules_in_pins_and_existing_view_source_fields() {
        let tuning = String::from_utf8(TUNING.to_vec())
            .unwrap()
            .replace("\"armor_class\": 17", "\"armor_class\": 15");
        let loaded = pack::load_from_bytes(PACK, tuning.as_bytes()).unwrap();
        let suffix = format!("+custom-{}", &loaded.tuning_revision.to_hex()[..12]);
        pack::with_test_pack(loaded, || {
            let checkpoint = super::super::journey::initial().unwrap();
            assert_eq!(checkpoint.pins().rules.mode, RulesMode::DisclosedCustom);
            assert!(
                checkpoint
                    .pins()
                    .rules
                    .ruleset
                    .as_str()
                    .starts_with("custom-")
            );
            assert!(
                super::super::wire::creation_offer()
                    .unwrap()
                    .source_revision
                    .ends_with(&suffix)
            );
        });
    }
}

#[cfg(test)]
mod creator_portrait_recovery_tests {
    use super::*;
    use crate::gameplay::{beat_script_tests, hero_library, journey, pack, wire};
    use df_protocol::common as rpc;
    use df_types::OperationId;

    fn with_heroes(
        count: usize,
        full_identity: bool,
        run: impl FnOnce(Checkpoint, Vec<(AssetReference, MemberId)>),
    ) {
        let mut tuning: serde_json::Value = serde_json::from_slice(include_bytes!(
            "fixtures/harbor-journey-tuning-baseline.json"
        ))
        .unwrap();
        tuning["tactical"] =
            serde_json::json!({"enemies":[{"srd_id":"bandit","count":3,"ammunition":20}]});
        let loaded = pack::load_from_bytes(
            include_bytes!("../../../../content/harbor-journey/pack.json"),
            &serde_json::to_vec(&tuning).unwrap(),
        )
        .unwrap();
        pack::with_test_pack(loaded, || {
            let mut current = journey::initial().unwrap();
            let mut members = Vec::new();
            for index in 0..count {
                current = journey::stage_join(
                    &current,
                    &beat_script_tests::input(
                        &current,
                        journey::bootstrap_member().unwrap(),
                        index as u8 + 1,
                        "join-room",
                        vec![],
                    ),
                )
                .unwrap();
                members.push(current.state().members.last().unwrap().member);
            }
            let mut portraits = Vec::new();
            for (index, member) in members.into_iter().enumerate() {
                let joined = current;
                let mut identity = crate::gameplay::hero_identity::Identity::from_menus(&[])
                    .unwrap()
                    .to_rpc();
                identity.appearance.as_mut().unwrap().public_description =
                    "Silver horns, emerald skin and a violet coat.".to_owned();
                if full_identity {
                    identity.appearance.as_mut().unwrap().public_description =
                        format!("{}\u{96ea}{}", "\u{1f409}".repeat(698), "a".repeat(1301));
                    identity.origin = "\u{1f409}".repeat(160);
                    identity.occupation = "\u{1f409}".repeat(160);
                    identity.motivation = "\u{1f409}".repeat(160);
                    identity.backstory = "\u{1f409}".repeat(480);
                    identity.facts = (0..6)
                        .map(|fact| format!("{fact}{}", "\u{1f409}".repeat(159)))
                        .collect();
                }
                let selection = rpc::CharacterSelection {
                    name: ["Neris", "Sigrun", "Brynn", "Corin", "Elara", "Vix"][index].into(),
                    choices: [
                        ("species", "dwarf"),
                        ("class", "fighter-1"),
                        ("background", "soldier"),
                        ("language-1", "dwarvish"),
                        ("language-2", "elvish"),
                        ("skill-1", "acrobatics"),
                        ("skill-2", "survival"),
                        ("gaming-set", "playing-cards"),
                        ("allied-ties", "join-order"),
                        ("alignment", "chaotic-good"),
                        ("build-package", "melee-stalwart"),
                    ]
                    .into_iter()
                    .map(|(group, option)| rpc::JourneyChoice {
                        group_id: group.into(),
                        option_id: option.into(),
                    })
                    .collect(),
                    identity: Some(identity),
                    portrait_scenery: if full_identity {
                        format!("{}{}", "\u{1f409}".repeat(136), "a".repeat(224))
                    } else {
                        String::new()
                    },
                };
                let request = rpc::SubmitActionRequest {
                    session_id: Some(rpc::SessionId {
                        value: Some(joined.basis().session.as_bytes().to_vec()),
                    }),
                    run_id: Some(rpc::RunId {
                        value: Some(joined.basis().run.as_bytes().to_vec()),
                    }),
                    operation_id: Some(rpc::OperationId {
                        value: Some(vec![index as u8 + 10; 16]),
                    }),
                    observed_revision: Some(wire::revision(joined.basis().revision)),
                    offer_id: journey::offer_id(&joined, rpc::GameplayActionKind::CreateCharacter),
                    action_kind: rpc::GameplayActionKind::CreateCharacter as i32,
                    character: Some(selection),
                    ..Default::default()
                };
                let mut input = wire::journey_input(&request, member, &joined).unwrap();
                let GameInput::Game(command) = &mut input else {
                    panic!("game command")
                };
                let GameCommand::ProposeAction { choices, .. } = &mut command.command else {
                    panic!("creation")
                };
                choices.extend(hero_library::identity_choices(request.character.as_ref()).unwrap());
                // The actor's trusted approved-attachment hook, after wire rejects client metadata.
                let image = AssetReference {
                    key: label(&if count == 1 {
                        "a".repeat(64)
                    } else {
                        format!("{index:064x}")
                    })
                    .unwrap(),
                    digest: ContentDigest(
                        Sha256::digest(b"approved synthetic creator portrait").into(),
                    ),
                    // Media bytes live in AssetStore, independently of checkpoint budgets.
                    byte_length: if full_identity {
                        2 * 1024 * 1024
                    } else {
                        b"approved synthetic creator portrait".len() as u64
                    },
                    kind: AssetKind::Image,
                };
                for (field, value) in [
                    ("key", image.key.as_str().to_owned()),
                    ("digest", journey::hex(&image.digest.0)),
                    ("bytes", image.byte_length.to_string()),
                    ("revision", "7".to_owned()),
                ] {
                    choices.push((
                        label(&format!("identity-portrait:{field}")).unwrap(),
                        label(&value).unwrap(),
                    ));
                }
                choices.shrink_to_fit();
                let accepted = HarborEngine
                    .decide_admitted(
                        &joined,
                        &input,
                        OperationId::from_bytes(&[index as u8 + 10; 16]).unwrap(),
                    )
                    .expect("approved portrait must not prevent accepting the hero");
                portraits.push((image, member));
                current = accepted;
            }
            run(current, portraits);
        });
    }

    fn with_hero(run: impl FnOnce(Checkpoint, AssetReference, MemberId)) {
        with_heroes(1, false, |accepted, mut portraits| {
            let (image, member) = portraits.pop().unwrap();
            run(accepted, image, member);
        });
    }

    fn codec_limits() -> df_persistence::NativeCodecLimits {
        crate::gameplay::live_codec_limits()
    }

    #[test]
    fn obsolete_four_kib_codec_text_limit_refuses_the_creation_receipt_with_exact_budget() {
        with_hero(|accepted, _, _| {
            let largest_receipt = accepted
                .state()
                .decisions
                .iter()
                .filter_map(|decision| decision.semantic_output.as_ref())
                .map(String::len)
                .max()
                .unwrap();
            println!("CREATOR_RECEIPT largest_text_bytes={largest_receipt}");
            super::super::refusal_detail::begin_action();
            let obsolete = df_persistence::NativeCodecLimits {
                maximum_document_bytes: 2 * 1024 * 1024,
                maximum_allocated_bytes: 4 * 1024 * 1024,
                maximum_collection_items: 8192,
                maximum_text_bytes: 4096,
            };
            assert_eq!(
                recovery()
                    .unwrap()
                    .encode_candidate(&accepted, accepted.pins(), obsolete),
                Err(RepositoryError::InvalidCandidate)
            );
            let diagnostic = df_session::submission::take_refusal_diagnostic().unwrap();
            assert_eq!(diagnostic.step, "checkpoint-codec-budget");
            assert_eq!(
                diagnostic.cause,
                format!("Capacity budget=text_bytes actual={largest_receipt} limit=4096")
            );
        });
    }

    #[test]
    fn six_creator_portraits_and_full_library_identities_leave_checkpoint_session_capacity() {
        with_heroes(
            df_rules::local_journey::MAXIMUM_PARTY,
            true,
            |accepted, portraits| {
                assert_eq!(accepted.state().characters.len(), 6);
                assert_eq!(creator_portrait_assets(accepted.state()).unwrap().len(), 6);
                let source = recovery().unwrap();
                let live = codec_limits();
                // Keep half the document/allocation budgets and a quarter of collection items
                // free even after this workload; no production bound is changed.
                let headroom = df_persistence::NativeCodecLimits {
                    maximum_document_bytes: live.maximum_document_bytes / 2,
                    maximum_allocated_bytes: live.maximum_allocated_bytes / 2,
                    maximum_collection_items: live.maximum_collection_items * 3 / 4,
                    maximum_text_bytes: 8192,
                };
                let encode = |checkpoint: &Checkpoint| {
                    super::super::refusal_detail::begin_action();
                    source
                        .encode_candidate(checkpoint, checkpoint.pins(), headroom)
                        .unwrap_or_else(|error| {
                            panic!(
                                "{error:?}; {:?}",
                                df_session::submission::take_refusal_diagnostic()
                            )
                        })
                };
                let initial = encode(&accepted);
                let mut state = accepted.state().clone();
                let mut basis = accepted.basis();
                // A bounded storage workload: 2,048 distinct accepted decisions with 512-byte
                // semantic outputs. This measures checkpoint headroom, not arbitrary game turns.
                for index in 1u128..=2048 {
                    basis.revision = basis.revision.next_sequence().unwrap();
                    state.decisions.push(AcceptedDecision {
                        operation: OperationId::from_bytes(&index.to_be_bytes()).unwrap(),
                        revision: basis.revision,
                        facts: vec![],
                        draws: vec![],
                        effects: vec![],
                        source_policy: rule().unwrap().catalog,
                        semantic_output: Some("s".repeat(512)),
                    });
                }
                let long_session = checkpoint(basis, state).unwrap();
                let encoded = encode(&long_session);
                let restored = source
                    .decode_checkpoint(
                        &encoded,
                        long_session.basis(),
                        long_session.pins(),
                        headroom,
                    )
                    .unwrap();
                assert_eq!(restored, long_session);
                for (image, member) in portraits {
                    let saved = hero_library::snapshot(&restored, member).unwrap();
                    let hero = saved.hero.unwrap();
                    let identity = hero.identity.unwrap();
                    assert_eq!(identity.appearance.unwrap().public_description.len(), 4096);
                    assert_eq!(identity.backstory.chars().count(), 480);
                    assert_eq!(identity.facts.len(), 6);
                    assert_eq!(hero.portrait_scenery.chars().count(), 360);
                    let portrait = saved.portrait.unwrap();
                    assert_eq!(portrait.key, image.key.as_str());
                    assert_eq!(portrait.digest, image.digest.0);
                    assert_eq!(portrait.bytes, 2 * 1024 * 1024);
                    assert_eq!(portrait.revision, 7);
                }
                println!(
                    "SIX_PORTRAIT_CAPACITY initial_bytes={} long_session_bytes={} added_decisions=2048 decision_text_bytes=512 retained_bytes={:?}",
                    initial.len(),
                    encoded.len(),
                    restored.retained_bytes()
                );
            },
        );
    }

    #[test]
    fn accepted_creator_portrait_survives_candidate_codec_recovery_and_library_seat_save() {
        with_hero(|accepted, image, member| {
            assert_eq!(
                creator_portrait_assets(accepted.state()).unwrap(),
                vec![image.clone()]
            );
            let source = recovery().unwrap();
            let encoded = source
                .encode_candidate(&accepted, accepted.pins(), codec_limits())
                .unwrap_or_else(|error| {
                    panic!(
                        "{error:?}; {:?}",
                        df_session::submission::take_refusal_diagnostic()
                    )
                });
            let restored = source
                .decode_checkpoint(&encoded, accepted.basis(), accepted.pins(), codec_limits())
                .unwrap();
            assert_eq!(restored, accepted);
            let saved = hero_library::snapshot(&restored, member).unwrap();
            let hero = saved.hero.unwrap();
            assert_eq!(hero.name, "Neris");
            assert_eq!(
                hero.identity
                    .unwrap()
                    .appearance
                    .unwrap()
                    .public_description,
                "Silver horns, emerald skin and a violet coat."
            );
            let portrait = saved
                .portrait
                .expect("the library save reads the committed portrait");
            assert_eq!(portrait.key, image.key.as_str());
            assert_eq!(portrait.digest, image.digest.0);
            assert_eq!(portrait.bytes, image.byte_length);
            assert_eq!(portrait.revision, 7);
            let mut fixed = source.clone();
            fixed.asset_resolver = None;
            assert_eq!(
                fixed.decode_checkpoint(
                    &encoded,
                    accepted.basis(),
                    accepted.pins(),
                    codec_limits()
                ),
                Err(RepositoryError::InvalidCandidate)
            );
        });
    }

    #[test]
    fn tampered_or_foreign_creator_metadata_is_refused_locally_and_on_decoded_recovery() {
        with_hero(|accepted, image, _| {
            for defect in 0..8 {
                let mut state = accepted.state().clone();
                let appearance = &mut state.continuity.canonical_packs[0].identities[0];
                match defect {
                    0 => appearance.appearances[0].digest.0[0] ^= 1,
                    1 => appearance.appearances[0].byte_length += 1,
                    2 => appearance.appearances[0].kind = AssetKind::Audio,
                    3 => appearance.appearances[0].key = label("foreign").unwrap(),
                    4 => appearance.entity = journey::room_entity().unwrap(),
                    5 => state.characters[0]
                        .choices
                        .retain(|choice| choice.offer.as_str() != "identity-portrait:revision"),
                    6 => {
                        state.characters[0]
                            .choices
                            .iter_mut()
                            .find(|choice| choice.offer.as_str() == "identity-portrait:bytes")
                            .unwrap()
                            .selected = label("0").unwrap()
                    }
                    7 => {
                        state.characters[0]
                            .choices
                            .iter_mut()
                            .find(|choice| choice.offer.as_str() == "identity-portrait:key")
                            .unwrap()
                            .selected = label(&"A".repeat(64)).unwrap()
                    }
                    _ => unreachable!(),
                }
                assert!(
                    checkpoint(accepted.basis(), state.clone()).is_err(),
                    "defect {defect}"
                );
                // Encode an otherwise model-valid candidate with a caller-supplied inventory.
                // Native candidate admission and recovery must still refuse it.
                let candidate_assets = state
                    .continuity
                    .canonical_packs
                    .iter()
                    .flat_map(|pack| &pack.identities)
                    .flat_map(|identity| &identity.appearances)
                    .cloned()
                    .collect::<Vec<_>>();
                let candidate = Checkpoint::new(
                    CHECKPOINT_SCHEMA,
                    accepted.basis(),
                    accepted.pins().clone(),
                    state,
                    ReferenceInventory {
                        rules: &[rule().unwrap(), journey::rule().unwrap()],
                        content: &contents().unwrap(),
                        resources: &journey::resources_for(accepted.state()).unwrap(),
                        assets: &candidate_assets,
                    },
                    limits(),
                )
                .expect(
                    "metadata defect is model-valid only under the caller's supplied inventory",
                );
                {
                    let source = recovery().unwrap();
                    assert!(
                        source
                            .encode_candidate(&candidate, accepted.pins(), codec_limits())
                            .is_err(),
                        "defect {defect}"
                    );
                    let encoded = df_persistence::local_demo_scope::encode_owned_demo_checkpoint(
                        &candidate,
                        codec_limits(),
                    )
                    .unwrap();
                    assert!(
                        source
                            .decode_checkpoint(
                                &encoded,
                                accepted.basis(),
                                accepted.pins(),
                                codec_limits()
                            )
                            .is_err(),
                        "defect {defect}"
                    );
                }
            }
            let mut state = accepted.state().clone();
            let duplicate = state.characters[0]
                .choices
                .iter()
                .find(|choice| choice.offer.as_str() == "identity-portrait:key")
                .unwrap()
                .clone();
            state.characters[0].choices.push(duplicate);
            let _ = df_session::submission::take_refusal_diagnostic();
            assert!(creator_portrait_assets(&state).is_err());
            let detail = df_session::submission::take_refusal_diagnostic().unwrap();
            assert_eq!(detail.step, "creator-portrait-assets");
            assert!(
                detail
                    .cause
                    .starts_with("DuplicateField field=identity-portrait:")
            );
            let mut state = accepted.state().clone();
            state.continuity.canonical_packs[0].identities[0]
                .appearances
                .push(image.clone());
            state.continuity.canonical_packs[0].identities[0]
                .appearances
                .last_mut()
                .unwrap()
                .digest
                .0[0] ^= 1;
            assert!(creator_portrait_assets(&state).is_err());
            let detail = df_session::submission::take_refusal_diagnostic().unwrap();
            assert_eq!(detail.step, "creator-portrait-assets");
            assert_eq!(detail.cause, "Mismatch field=appearance-digest");
        });
    }
}
