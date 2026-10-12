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
use std::io::Read;
use std::sync::OnceLock;

pub(super) const SOURCE: &[u8] = b"SRD5.2.1 page6 D20 Tests Ability Checks; https://media.dndbeyond.com/compendium-images/srd/5.2/SRD_CC_v5.2.1.pdf; Wizards of the Coast LLC; CC BY 4.0; normal single-d20 check only";
const CONTENT: &[u8] = b"DungeonFlux authored local journey profile6: Share the Lantern room. Two independently joined members create source-selected Dwarf Fighter/Soldier heroes; their selections and starting equipment persist. Begin at Lantern Wharf only after both builds. The physically present unnamed courier is canonical lantern-wharf-courier, with cautious-courier personality, deliver-dispatch motivation, and directional unfamiliar relationships to each hero. Asking reveals a member-private delivery clue; it grants no courier knowledge or witness. Accepted escort creates a shared courier-escort-contact fact with the acting hero as subject and an actual same-scene courier witness/known fact. The terminal escort-courier event remains shared with empty subjects for current phase admission. On a subsequent Defend command, source-bound courier-escort-reaction-policy may change only that witnessed hero's relationship to escort-supported through df-interaction::react on the prior committed contact. The change is categorical story policy, without a D&D bonus, action success, resource grant, or packet delivery. Defending begins the normal adjacent nonlethal Bandit encounter with actual initiative, player-selected attacks/SecondWind/endturn, source-qualified damage, unconscious one-HP knockout and no wall-time rest completion. Source knockout starts persist due schedules. After combat a joined member may propose an uninterrupted campaign Short Rest: one hour, no Hit Point Dice or HP healing, Fighter one-use SecondWind recharge capped2. Knockout waking retains Prone and dropped held gear and preserves the battle outcome. Versioned local-journey-rpc-3-threads-1-rest-1-inn-1-npc-1 maps begin-story/private-courier-note/escort-courier events to continued sealed-packet-delivery-thread; defend-courier/greatsword-attack/second-wind/end-turn events continue dockside-threat-thread, resolving only on the current operation accepted bandit unconscious transition. A conscious joined hero may select Harbor Inn from the current source-bound victory offer, before or after Short Rest. The admitted harbor-inn ContentEvent continues the unresolved packet thread and retains party, knowledge, resource, encounter and schedule records at unchanged logical time; this is scene continuation, not timed physical travel or delivery. Current phase admission requires exact accepted source receipts, prerequisite beats and historical native encounter outcome. Native Bandit tactics enumerate at most two source-legal authored adjacent targets in joined party order, select the first legal target without utility weights and preserve attack/draw authority. Delivery remains unresolved after combat; unmapped events consume no narrative source. Preconfigured Investigation/next-scene draft remains an earlier source lineage, not the active browser room.";

fn invalid<T>(_: T) -> RepositoryError {
    RepositoryError::InvalidCandidate
}
pub(super) fn label(value: &str) -> Result<RevisionLabel, RepositoryError> {
    RevisionLabel::new(Some(value)).map_err(invalid)
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
            include_bytes!("../../../df-rules/src/local_journey.rs").as_slice(),
            include_bytes!("journey.rs").as_slice(),
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
        ] {
            handler.update(source);
        }

        let mut content = Sha256::new();
        for source in [
            CONTENT,
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
        manifest: source_digests().manifest,
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

pub(super) fn source_revision() -> String {
    match super::pack::active() {
        Some(loaded) if !loaded.departures.is_empty() => format!(
            "{}+custom-{}",
            df_rules::local_journey::SOURCE_REVISION,
            &loaded.tuning_revision.to_hex()[..12]
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
        .is_some_and(|loaded| loaded.pack.beat_script.is_some())
    {
        source_digests()
    } else {
        &legacy
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
    let catalog_digest = loaded.as_ref().map_or(digests.manifest, |loaded| {
        revision_digest(b"df-tuning-pin/1", digests.manifest, loaded.tuning_revision)
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
            source_manifest_digest: digests.manifest,
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
pub(super) fn basis() -> Result<Basis, RepositoryError> {
    Ok(Basis {
        session: SessionId::from_bytes(&[0x41; 16]).map_err(invalid)?,
        run: RunId::from_bytes(&[0x42; 16]).map_err(invalid)?,
        revision: SessionRevision::new(RecoveryEpoch::new(1).map_err(invalid)?, 0),
    })
}
pub(super) fn actor() -> Result<EntityId, RepositoryError> {
    EntityId::from_bytes(&[0x44; 16]).map_err(invalid)
}
pub(super) fn member() -> Result<MemberId, RepositoryError> {
    MemberId::from_bytes(&PLAYER).map_err(invalid)
}
pub(super) fn limits() -> CheckpointLimits {
    CheckpointLimits {
        maximum_records: 512,
        maximum_text_bytes: 4096,
        maximum_total_text_bytes: 65536,
        maximum_retained_bytes: 1024 * 1024,
    }
}
pub(super) fn recovery() -> Result<NativeRecoverySource, RepositoryError> {
    Ok(NativeRecoverySource {
        rules: vec![rule()?, super::journey::rule()?],
        content: contents()?,
        resources: super::journey::resources()?,
        assets: vec![],
        limits: limits(),
    })
}
pub(super) fn checkpoint(basis: Basis, state: GameState) -> Result<Checkpoint, RepositoryError> {
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        basis,
        pins()?,
        state,
        ReferenceInventory {
            rules: &[rule()?, super::journey::rule()?],
            content: &contents()?,
            resources: &super::journey::resources()?,
            assets: &[],
        },
        limits(),
    )
    .map_err(invalid)
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
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|_| RepositoryError::Unavailable)?;
    if N > 1 && bytes.iter().all(|byte| *byte == 0) {
        return Err(RepositoryError::Unavailable);
    }
    Ok(bytes)
}
fn draw_d20() -> Result<u8, RepositoryError> {
    // Reject the final incomplete bucket; modulo of all 256 byte values is biased.
    for _ in 0..16 {
        let [byte] = random::<1>()?;
        if let Some(face) = d20_face(byte) {
            return Ok(face);
        }
    }
    Err(RepositoryError::Unavailable)
}
fn d20_face(byte: u8) -> Option<u8> {
    (byte < 240).then_some(byte % 20 + 1)
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
            return Err(RepositoryError::InvalidCandidate);
        };
        let GameCommand::ProposeAction {
            actor: who,
            action,
            targets,
            choices,
        } = &command.command
        else {
            return Err(RepositoryError::InvalidCandidate);
        };
        if *who != actor()?
            || command.member != member()?
            || *action != content("inspect-seal")?
            || !targets.is_empty()
            || !choices.is_empty()
            || !current.state().decisions.is_empty()
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        let [draw] = input.supplied_draws else {
            return Err(RepositoryError::InvalidCandidate);
        };
        if draw.sides != 20 || draw.source != rule()? {
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
        return Err(RepositoryError::InvalidCandidate);
    };
    let GameCommand::ProposeAction {
        actor: who,
        action,
        targets,
        choices,
    } = &command.command
    else {
        return Err(RepositoryError::InvalidCandidate);
    };
    let [(input_id, selected)] = choices.as_slice() else {
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
        return Err(RepositoryError::InvalidCandidate);
    }
    validate_client_command(
        input,
        current,
        ReferenceInventory {
            rules: &[rule()?],
            content: &contents()?,
            resources: &[],
            assets: &[],
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
            if super::journey::CONTENT_ENTRIES.contains(&action.entry.as_str()) {
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
            return Err(RepositoryError::InvalidCandidate);
        }
        let GameInput::Game(command) = input else {
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
                    resources: &[],
                    assets: &[],
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
    TransitionResult::classify(current, candidate, operation).unwrap_or(TransitionResult::Rejected(
        RepositoryError::InvalidCandidate,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use df_types::OperationId;

    #[test]
    fn every_d20_face_has_twelve_equally_likely_byte_inputs() {
        for face in 1..=20 {
            assert_eq!(
                (0..=u8::MAX)
                    .filter(|byte| d20_face(*byte) == Some(face))
                    .count(),
                12
            );
        }
        assert_eq!(d20_face(0), Some(1));
        assert_eq!(
            (0..=u8::MAX)
                .filter(|byte| d20_face(*byte).is_none())
                .count(),
            16
        );
    }

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
    const TUNING: &[u8] = include_bytes!("../../../../content/harbor-journey/tuning.json");
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
                    .source_revision
                    .ends_with(&suffix)
            );
        });
    }
}
