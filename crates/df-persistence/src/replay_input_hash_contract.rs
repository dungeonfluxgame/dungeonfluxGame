use df_model::checkpoint::*;
use df_rules::local_journey::{self, NormalMeleeAttack};
use df_types::{
    BuildIdentity, MemberId, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId,
    SessionRevision,
};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CanonicalHash {
    codec_version: u16,
    model_schema: u16,
    sha256: [u8; 32],
}

fn canonical_hash(
    checkpoint: &Checkpoint,
) -> Result<CanonicalHash, crate::checkpoint_codec::CodecError> {
    let bytes = crate::checkpoint_codec::encode_checkpoint(checkpoint, codec_limits())?;
    Ok(CanonicalHash {
        codec_version: u16::try_from(crate::checkpoint_codec::STORAGE_CODEC_VERSION)
            .map_err(|_| crate::checkpoint_codec::CodecError::UnsupportedCodec)?,
        model_schema: checkpoint.schema(),
        sha256: Sha256::digest(bytes).into(),
    })
}

fn label(value: &str) -> RevisionLabel {
    RevisionLabel::new(Some(value)).expect("static label")
}
fn member(byte: u8) -> MemberId {
    MemberId::from_bytes(&[byte; 16]).expect("nonzero member")
}
fn entity(byte: u8) -> EntityId {
    EntityId::from_bytes(&[byte; 16]).expect("nonzero entity")
}
fn operation() -> OperationId {
    OperationId::from_bytes(&[6; 16]).expect("nonzero operation")
}
fn basis() -> Basis {
    Basis {
        session: SessionId::from_bytes(&[1; 16]).expect("session"),
        run: RunId::from_bytes(&[2; 16]).expect("run"),
        revision: SessionRevision::new(RecoveryEpoch::new(2).expect("epoch"), 8),
    }
}
fn content() -> ContentReference {
    ContentReference {
        package: label("local-journey-content-1"),
        entry: label("encounter-1"),
    }
}
fn source() -> RuleReference {
    RuleReference {
        catalog: label("local-journey-catalog-1"),
        source: label(local_journey::SOURCE_REVISION),
        entry: label("normal-melee-attack"),
        clause: label("attack-damage-knockout"),
    }
}
fn pins() -> CheckpointPins {
    let rules_bytes = include_bytes!("../../df-rules/src/local_journey.rs");
    let codec_bytes = include_bytes!("checkpoint_codec.rs");
    CheckpointPins {
        rules: RulesPins {
            mode: RulesMode::Standard2024,
            ruleset: label("srd-5.2.1-selected-local-journey"),
            catalog: label("local-journey-catalog-1"),
            catalog_digest: ContentDigest(Sha256::digest(local_journey::SOURCE.as_bytes()).into()),
            source_manifest: label(local_journey::SOURCE_REVISION),
            source_manifest_digest: ContentDigest(Sha256::digest(rules_bytes).into()),
            handler: label("df-rules-local-journey-resolve-melee"),
            handler_digest: ContentDigest(Sha256::digest(rules_bytes).into()),
        },
        content: ContentPins {
            content: label("local-journey-content-1"),
            content_digest: ContentDigest(Sha256::digest(b"encounter-1").into()),
            package: label("local-journey-content-1"),
            package_digest: ContentDigest(Sha256::digest(b"local-journey-content-1").into()),
        },
        build: BuildIdentity::new(
            Some("d6648ba329750c0826b369b36879962cc7d9d128"),
            Some(&format!("{:x}", Sha256::digest(codec_bytes))),
            Some("wasm-not-claimed"),
            Some("replay-input-hash-contract-v1"),
            Some("local-journey-content-1"),
        )
        .expect("fixture build identity"),
    }
}
fn limits() -> CheckpointLimits {
    CheckpointLimits {
        maximum_records: 64,
        maximum_text_bytes: 512,
        maximum_total_text_bytes: 2048,
        maximum_retained_bytes: 1024 * 1024,
    }
}
fn codec_limits() -> crate::checkpoint_codec::CodecLimits {
    crate::checkpoint_codec::CodecLimits {
        maximum_document_bytes: 1024 * 1024,
        maximum_allocated_bytes: 8 * 1024 * 1024,
        maximum_collection_items: 4096,
        maximum_text_bytes: 4096,
    }
}
fn content_ref() -> ContentReference {
    content()
}

fn empty_state() -> GameState {
    GameState {
        mode: ExecutionMode::Replay,
        logical_time: LogicalTime {
            ticks: 120,
            ticks_per_second: 10,
        },
        members: vec![MembershipLink {
            member: member(3),
            character: Some(entity(4)),
        }],
        entities: vec![WorldEntity {
            id: entity(4),
            definition: content_ref(),
            location: None,
            position: None,
            identity_revision: label("local-journey-entity-1"),
        }],
        characters: vec![CharacterState {
            entity: entity(4),
            build: content_ref(),
            owner: member(3),
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
            definition: content_ref(),
            active_beats: vec![],
            completed_beats: vec![],
            open_threads: vec![],
            accepted_facts: vec![],
            remaining_budget: 0,
        },
        encounters: vec![],
        activity: vec![],
        tempo: TempoState {
            policy: content_ref(),
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
    }
}

fn checkpoint(
    graze_choice: &str,
    values: &[u32],
    policy: &str,
    mut admitted: CheckpointPins,
) -> Checkpoint {
    let (attack_die, damage_dice) = values.split_first().expect("fixture attack die");
    let prestate = selected_prestate();
    let outcome = local_journey::resolve_melee(NormalMeleeAttack {
        critical_threshold: local_journey::CombatRules::srd_521().critical_threshold,
        damage_sides: local_journey::SRD_521_BASELINE.hero_weapon.damage_sides,
        die: *attack_die,
        modifier: prestate.attack_modifier,
        armor_class: prestate.armor_class,
        damage_dice,
        weapon_dice: prestate.weapon_dice,
        damage_modifier: prestate.damage_modifier,
        savage_attacker_take_higher: false,
        graze: graze_choice == "graze",
        target_hit_points: prestate.target_hit_points,
    })
    .expect("fixture is a valid supported attack");
    let mut state = empty_state();
    let selected = AcceptedChoice {
        participant: member(3),
        offer: label("greatsword-mastery-offer"),
        selected: label(graze_choice),
        source: source(),
    };
    state.characters[0].choices.push(selected);
    state.draws = values
        .iter()
        .enumerate()
        .map(|(i, value)| ActualDraw {
            operation: operation(),
            ordinal: u32::try_from(i).expect("small ordinal"),
            resolution: ResolutionId::from_bytes(&[7; 16]).expect("resolution"),
            window: WindowId::from_bytes(&[8; 16]).expect("window"),
            sides: if i == 0 { 20 } else { 6 },
            value: *value,
            source: source(),
        })
        .collect();
    state.decisions.push(AcceptedDecision {
        operation: operation(),
        revision: basis().revision,
        facts: vec![],
        draws: (0..u32::try_from(values.len()).expect("small draw count")).collect(),
        effects: vec![],
        source_policy: label(policy),
        semantic_output: Some(semantic_output(&outcome)),
    });
    admitted.rules.catalog = source().catalog;
    let rules = [source()];
    let contents = [content()];
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        basis(),
        admitted,
        state,
        ReferenceInventory {
            rules: &rules,
            content: &contents,
            resources: &[],
            assets: &[],
        },
        limits(),
    )
    .expect("canonical fixture checkpoint")
}

const POLICY_REVISION: &str = "policy-rev-1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReplayPrestate {
    attack_modifier: i32,
    armor_class: u32,
    damage_modifier: i32,
    target_hit_points: u32,
    weapon_dice: usize,
}
fn selected_prestate() -> ReplayPrestate {
    ReplayPrestate {
        attack_modifier: 5,
        armor_class: 13,
        damage_modifier: 3,
        target_hit_points: 7,
        weapon_dice: 2,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReplayInputs {
    choice: Option<AcceptedChoice>,
    decision: Option<AcceptedDecision>,
    draws: Vec<ActualDraw>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReplayRefusal {
    WrongBasis,
    MissingChoice,
    MissingDecision,
    MissingDraws,
    UnsupportedSource,
    UnsupportedHandler,
    UnsupportedPolicy,
    UnsupportedPins,
    UnsupportedChoice,
    InvalidDrawOrder,
    InvalidDrawType,
    UnavailableHistory,
    InvalidCheckpoint,
}

fn recorded_inputs(checkpoint: &Checkpoint) -> ReplayInputs {
    ReplayInputs {
        choice: checkpoint
            .state()
            .characters
            .first()
            .and_then(|character| character.choices.first())
            .cloned(),
        decision: checkpoint.state().decisions.first().cloned(),
        draws: checkpoint.state().draws.clone(),
    }
}

fn semantic_output(outcome: &local_journey::AttackOutcome) -> String {
    format!(
        "hit={};critical={};damage={};target_hp={};knocked_out={}",
        outcome.hit,
        outcome.critical,
        outcome.damage,
        outcome.target_hit_points,
        outcome.knocked_out
    )
}

fn replay_selected(
    prestate: ReplayPrestate,
    original: &Checkpoint,
    inputs: &ReplayInputs,
) -> Result<Checkpoint, ReplayRefusal> {
    if original.basis() != basis() {
        return Err(ReplayRefusal::WrongBasis);
    }
    if !original
        .state()
        .continuity
        .recovery
        .redacted_records
        .is_empty()
        || !original
            .state()
            .continuity
            .recovery
            .unavailable_sources
            .is_empty()
    {
        return Err(ReplayRefusal::UnavailableHistory);
    }

    let admitted = pins();
    let actual_pins = original.pins();
    if actual_pins.rules.handler != admitted.rules.handler
        || actual_pins.rules.handler_digest != admitted.rules.handler_digest
    {
        return Err(ReplayRefusal::UnsupportedHandler);
    }
    if actual_pins.rules.source_manifest != admitted.rules.source_manifest
        || actual_pins.rules.source_manifest_digest != admitted.rules.source_manifest_digest
        || actual_pins.rules.catalog != admitted.rules.catalog
        || actual_pins.rules.catalog_digest != admitted.rules.catalog_digest
        || actual_pins.rules.ruleset != admitted.rules.ruleset
        || actual_pins.rules.mode != admitted.rules.mode
    {
        return Err(ReplayRefusal::UnsupportedSource);
    }
    if actual_pins.content != admitted.content || actual_pins.build != admitted.build {
        return Err(ReplayRefusal::UnsupportedPins);
    }
    if prestate != selected_prestate() {
        return Err(ReplayRefusal::UnsupportedSource);
    }
    let mut stripped_state = original.state().clone();
    let character = stripped_state
        .characters
        .first_mut()
        .ok_or(ReplayRefusal::InvalidCheckpoint)?;
    character.choices.clear();
    stripped_state.draws.clear();
    stripped_state.decisions.clear();
    if stripped_state != empty_state() {
        return Err(ReplayRefusal::InvalidCheckpoint);
    }

    let choice = inputs.choice.as_ref().ok_or(ReplayRefusal::MissingChoice)?;
    if choice.source != source() {
        return Err(ReplayRefusal::UnsupportedSource);
    }
    let graze = match choice.selected.as_str() {
        "graze" => true,
        "no-graze" => false,
        _ => return Err(ReplayRefusal::UnsupportedChoice),
    };
    let recorded_choices = original
        .state()
        .characters
        .first()
        .ok_or(ReplayRefusal::InvalidCheckpoint)?
        .choices
        .as_slice();
    if recorded_choices.len() != 1 || recorded_choices.first() != Some(choice) {
        return Err(ReplayRefusal::MissingChoice);
    }
    let decision = inputs
        .decision
        .as_ref()
        .ok_or(ReplayRefusal::MissingDecision)?;
    if decision.source_policy.as_str() != POLICY_REVISION {
        return Err(ReplayRefusal::UnsupportedPolicy);
    }
    if original.state().decisions.len() != 1 || original.state().decisions.first() != Some(decision)
    {
        return Err(ReplayRefusal::MissingDecision);
    }
    if inputs.draws.is_empty() {
        return Err(ReplayRefusal::MissingDraws);
    }
    if original.state().draws != inputs.draws {
        return Err(ReplayRefusal::InvalidDrawOrder);
    }
    if decision.operation != operation()
        || decision.revision != original.basis().revision
        || !decision.facts.is_empty()
        || !decision.effects.is_empty()
    {
        return Err(ReplayRefusal::InvalidDrawOrder);
    }
    if decision.draws.len() != inputs.draws.len() {
        return Err(ReplayRefusal::InvalidDrawOrder);
    }
    for (index, draw) in inputs.draws.iter().enumerate() {
        let ordinal = u32::try_from(index).map_err(|_| ReplayRefusal::InvalidDrawOrder)?;
        if draw.ordinal != ordinal
            || decision.draws.get(index).copied() != Some(ordinal)
            || draw.operation != decision.operation
            || draw.resolution != inputs.draws[0].resolution
            || draw.window != inputs.draws[0].window
        {
            return Err(ReplayRefusal::InvalidDrawOrder);
        }
        if draw.source != source() {
            return Err(ReplayRefusal::UnsupportedSource);
        }
        let expected_sides = if index == 0 { 20 } else { 6 };
        if draw.sides != expected_sides || draw.value == 0 || draw.value > expected_sides {
            return Err(ReplayRefusal::InvalidDrawType);
        }
    }
    let attack_die = inputs.draws[0].value;
    let damage_dice = inputs.draws[1..]
        .iter()
        .map(|draw| draw.value)
        .collect::<Vec<_>>();
    let outcome = local_journey::resolve_melee(NormalMeleeAttack {
        critical_threshold: local_journey::CombatRules::srd_521().critical_threshold,
        damage_sides: local_journey::SRD_521_BASELINE.hero_weapon.damage_sides,
        die: attack_die,
        modifier: prestate.attack_modifier,
        armor_class: prestate.armor_class,
        damage_dice: &damage_dice,
        weapon_dice: prestate.weapon_dice,
        damage_modifier: prestate.damage_modifier,
        savage_attacker_take_higher: false,
        graze,
        target_hit_points: prestate.target_hit_points,
    })
    .map_err(|_| ReplayRefusal::InvalidDrawOrder)?;

    let mut state = empty_state();
    state.characters[0].choices.push(choice.clone());
    state.draws.clone_from(&inputs.draws);
    state.decisions.push(AcceptedDecision {
        operation: decision.operation,
        revision: decision.revision,
        facts: decision.facts.clone(),
        draws: decision.draws.clone(),
        effects: decision.effects.clone(),
        source_policy: decision.source_policy.clone(),
        semantic_output: Some(semantic_output(&outcome)),
    });
    let rules = [source()];
    let contents = [content()];
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        original.basis(),
        admitted,
        state,
        ReferenceInventory {
            rules: &rules,
            content: &contents,
            resources: &[],
            assets: &[],
        },
        limits(),
    )
    .map_err(|_| ReplayRefusal::InvalidCheckpoint)
}

fn decode(
    bytes: &[u8],
    expected: Basis,
    admitted: &CheckpointPins,
) -> Result<Checkpoint, crate::checkpoint_codec::CodecError> {
    let rules = [source()];
    let contents = [content()];
    crate::checkpoint_codec::decode_checkpoint(
        bytes,
        expected,
        admitted,
        ReferenceInventory {
            rules: &rules,
            content: &contents,
            resources: &[],
            assets: &[],
        },
        limits(),
        codec_limits(),
    )
}

fn rebuild_checkpoint(basis: Basis, pins: CheckpointPins, state: GameState) -> Checkpoint {
    let rules = [source()];
    let contents = [content()];
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        basis,
        pins,
        state,
        ReferenceInventory {
            rules: &rules,
            content: &contents,
            resources: &[],
            assets: &[],
        },
        limits(),
    )
    .expect("valid reconstructed checkpoint fixture")
}

fn assert_refusal(
    prestate: ReplayPrestate,
    original: &Checkpoint,
    inputs: &ReplayInputs,
    expected: ReplayRefusal,
) {
    let checkpoint_before = original.clone();
    let inputs_before = inputs.clone();
    assert_eq!(replay_selected(prestate, original, inputs), Err(expected));
    assert_eq!(original, &checkpoint_before);
    assert_eq!(inputs, &inputs_before);
}

#[test]
fn missing_or_unsupported_replay_inputs_refuse_before_reducer_without_mutation() {
    let original = checkpoint("graze", &[18, 4, 2], POLICY_REVISION, pins());
    let prestate = selected_prestate();

    let mut inputs = recorded_inputs(&original);
    inputs.choice = None;
    assert_refusal(prestate, &original, &inputs, ReplayRefusal::MissingChoice);

    let mut inputs = recorded_inputs(&original);
    inputs.decision = None;
    assert_refusal(prestate, &original, &inputs, ReplayRefusal::MissingDecision);

    let mut inputs = recorded_inputs(&original);
    inputs.choice.as_mut().unwrap().selected = label("unsupported-mastery");
    assert_refusal(
        prestate,
        &original,
        &inputs,
        ReplayRefusal::UnsupportedChoice,
    );

    let mut inputs = recorded_inputs(&original);
    inputs.draws.clear();
    assert_refusal(prestate, &original, &inputs, ReplayRefusal::MissingDraws);

    let mut inputs = recorded_inputs(&original);
    inputs.choice.as_mut().unwrap().source.source = label("unavailable-srd-revision");
    assert_refusal(
        prestate,
        &original,
        &inputs,
        ReplayRefusal::UnsupportedSource,
    );

    let mut inputs = recorded_inputs(&original);
    inputs.decision.as_mut().unwrap().source_policy = label("unsupported-policy-revision");
    assert_refusal(
        prestate,
        &original,
        &inputs,
        ReplayRefusal::UnsupportedPolicy,
    );

    let mut inputs = recorded_inputs(&original);
    inputs.draws.swap(1, 2);
    assert_refusal(
        prestate,
        &original,
        &inputs,
        ReplayRefusal::InvalidDrawOrder,
    );

    let mut inputs = recorded_inputs(&original);
    inputs.draws[1].sides = 8;
    assert_refusal(
        prestate,
        &original,
        &inputs,
        ReplayRefusal::InvalidDrawOrder,
    );

    let mut state = original.state().clone();
    state.draws[1].sides = 8;
    let wrong_draw_type = rebuild_checkpoint(original.basis(), original.pins().clone(), state);
    let inputs = recorded_inputs(&wrong_draw_type);
    assert_refusal(
        prestate,
        &wrong_draw_type,
        &inputs,
        ReplayRefusal::InvalidDrawType,
    );

    let mut inputs = recorded_inputs(&original);
    inputs.draws[1].source.source = label("unavailable-srd-revision");
    assert_refusal(
        prestate,
        &original,
        &inputs,
        ReplayRefusal::InvalidDrawOrder,
    );

    let mut state = original.state().clone();
    state.draws[1].source.source = label("unavailable-srd-revision");
    let alternate_source = state.draws[1].source.clone();
    let rules = [source(), alternate_source];
    let contents = [content()];
    let wrong_draw_source = Checkpoint::new(
        CHECKPOINT_SCHEMA,
        original.basis(),
        original.pins().clone(),
        state,
        ReferenceInventory {
            rules: &rules,
            content: &contents,
            resources: &[],
            assets: &[],
        },
        limits(),
    )
    .expect("valid source-qualified foreign draw fixture");
    let inputs = recorded_inputs(&wrong_draw_source);
    assert_refusal(
        prestate,
        &wrong_draw_source,
        &inputs,
        ReplayRefusal::UnsupportedSource,
    );

    let mut pins = original.pins().clone();
    pins.rules.source_manifest = label("unavailable-source-manifest");
    let unavailable_source = rebuild_checkpoint(original.basis(), pins, original.state().clone());
    let inputs = recorded_inputs(&unavailable_source);
    assert_refusal(
        prestate,
        &unavailable_source,
        &inputs,
        ReplayRefusal::UnsupportedSource,
    );

    let mut pins = original.pins().clone();
    pins.rules.handler_digest = ContentDigest([88; 32]);
    let unavailable_handler = rebuild_checkpoint(original.basis(), pins, original.state().clone());
    let inputs = recorded_inputs(&unavailable_handler);
    assert_refusal(
        prestate,
        &unavailable_handler,
        &inputs,
        ReplayRefusal::UnsupportedHandler,
    );

    let mut pins = original.pins().clone();
    pins.content.package_digest = ContentDigest([77; 32]);
    let foreign_content = rebuild_checkpoint(original.basis(), pins, original.state().clone());
    let inputs = recorded_inputs(&foreign_content);
    assert_refusal(
        prestate,
        &foreign_content,
        &inputs,
        ReplayRefusal::UnsupportedPins,
    );

    let mut state = original.state().clone();
    state.characters[0].choices.clear();
    let missing_choice = rebuild_checkpoint(original.basis(), original.pins().clone(), state);
    let inputs = recorded_inputs(&missing_choice);
    assert_refusal(
        prestate,
        &missing_choice,
        &inputs,
        ReplayRefusal::MissingChoice,
    );

    let mut state = original.state().clone();
    state.decisions.clear();
    let missing_decision = rebuild_checkpoint(original.basis(), original.pins().clone(), state);
    let inputs = recorded_inputs(&missing_decision);
    assert_refusal(
        prestate,
        &missing_decision,
        &inputs,
        ReplayRefusal::MissingDecision,
    );

    let mut state = original.state().clone();
    state.draws.clear();
    state.decisions[0].draws.clear();
    let missing_draws = rebuild_checkpoint(original.basis(), original.pins().clone(), state);
    let inputs = recorded_inputs(&missing_draws);
    assert_refusal(
        prestate,
        &missing_draws,
        &inputs,
        ReplayRefusal::MissingDraws,
    );

    let mut state = original.state().clone();
    state.draws.pop();
    state.decisions[0].draws.pop();
    let short_draws = rebuild_checkpoint(original.basis(), original.pins().clone(), state);
    let inputs = recorded_inputs(&short_draws);
    assert_refusal(
        prestate,
        &short_draws,
        &inputs,
        ReplayRefusal::InvalidDrawOrder,
    );
}

#[test]
fn redacted_and_unavailable_history_refuse_without_reconstruction() {
    let original = checkpoint("graze", &[18, 4, 2], POLICY_REVISION, pins());
    let prestate = selected_prestate();

    let mut state = original.state().clone();
    state
        .continuity
        .recovery
        .redacted_records
        .push(RecordId::from_bytes(&[9; 16]).expect("fixture record"));
    let redacted = rebuild_checkpoint(original.basis(), original.pins().clone(), state);
    let inputs = recorded_inputs(&redacted);
    assert_refusal(
        prestate,
        &redacted,
        &inputs,
        ReplayRefusal::UnavailableHistory,
    );

    let mut state = original.state().clone();
    state
        .continuity
        .recovery
        .unavailable_sources
        .push(label("retired-local-journey-source"));
    let unavailable = rebuild_checkpoint(original.basis(), original.pins().clone(), state);
    let inputs = recorded_inputs(&unavailable);
    assert_refusal(
        prestate,
        &unavailable,
        &inputs,
        ReplayRefusal::UnavailableHistory,
    );
}

#[test]
fn recorded_mastery_choice_changes_the_selected_reducer_result() {
    let graze = checkpoint("graze", &[7], POLICY_REVISION, pins());
    let no_graze = checkpoint("no-graze", &[7], POLICY_REVISION, pins());
    let graze_replayed =
        replay_selected(selected_prestate(), &graze, &recorded_inputs(&graze)).unwrap();
    let no_graze_replayed =
        replay_selected(selected_prestate(), &no_graze, &recorded_inputs(&no_graze)).unwrap();
    assert_ne!(
        graze_replayed.state().decisions[0].semantic_output,
        no_graze_replayed.state().decisions[0].semantic_output
    );
}

#[test]
fn semantic_byte_change_is_hashed_and_replay_reconstructs_the_accepted_result() {
    let original = checkpoint("graze", &[18, 4, 2], POLICY_REVISION, pins());
    let mut changed_state = original.state().clone();
    changed_state.decisions[0].semantic_output =
        Some("hit=true;critical=false;damage=10;target_hp=1;knocked_out=true".to_owned());
    let changed = rebuild_checkpoint(original.basis(), original.pins().clone(), changed_state);
    assert_ne!(
        crate::checkpoint_codec::encode_checkpoint(&original, codec_limits()).unwrap(),
        crate::checkpoint_codec::encode_checkpoint(&changed, codec_limits()).unwrap()
    );
    assert_ne!(
        canonical_hash(&original).unwrap(),
        canonical_hash(&changed).unwrap()
    );

    let inputs = recorded_inputs(&changed);
    let reconstructed = replay_selected(selected_prestate(), &changed, &inputs).unwrap();
    assert_eq!(
        reconstructed.state().decisions[0].semantic_output,
        original.state().decisions[0].semantic_output
    );
    assert_eq!(
        crate::checkpoint_codec::encode_checkpoint(&reconstructed, codec_limits()).unwrap(),
        crate::checkpoint_codec::encode_checkpoint(&original, codec_limits()).unwrap()
    );
}

#[test]
fn recorded_choice_ordered_draws_source_pins_and_reducer_replay_round_trip() {
    let original = checkpoint("graze", &[18, 4, 2], "policy-rev-1", pins());
    let before = original.clone();
    let bytes = crate::checkpoint_codec::encode_checkpoint(&original, codec_limits()).unwrap();
    assert_eq!(&bytes[..4], b"DFCP");
    assert_eq!(u16::from_be_bytes([bytes[4], bytes[5]]), 3);
    assert_eq!(original.schema(), 2);
    let hash = canonical_hash(&original).unwrap();
    assert_eq!(hash.codec_version, u16::from_be_bytes([bytes[4], bytes[5]]));
    assert_eq!(hash.model_schema, u16::from_be_bytes([bytes[6], bytes[7]]));
    let restored = decode(&bytes, original.basis(), original.pins()).unwrap();
    let encoded_again =
        crate::checkpoint_codec::encode_checkpoint(&restored, codec_limits()).unwrap();
    assert_eq!(encoded_again, bytes);
    assert_eq!(canonical_hash(&restored).unwrap(), hash);
    let inputs = recorded_inputs(&restored);
    let reconstructed = replay_selected(selected_prestate(), &restored, &inputs).unwrap();
    assert_eq!(
        reconstructed.state().characters[0].choices,
        restored.state().characters[0].choices
    );
    assert_eq!(reconstructed.state().draws, restored.state().draws);
    assert_eq!(reconstructed.state().decisions, restored.state().decisions);
    let reconstructed_bytes =
        crate::checkpoint_codec::encode_checkpoint(&reconstructed, codec_limits()).unwrap();
    assert_eq!(reconstructed_bytes, bytes);
    assert_eq!(canonical_hash(&reconstructed).unwrap(), hash);
    let digest_hex = hash.sha256.map(|byte| format!("{byte:02x}")).join("");
    assert_eq!(digest_hex.len(), 64);
    println!(
        "D04 canonical checkpoint hash: codec={} schema={} sha256={} source={} handler={} policy={}",
        hash.codec_version,
        hash.model_schema,
        digest_hex,
        restored.pins().rules.source_manifest.as_str(),
        restored.pins().rules.handler.as_str(),
        restored.state().decisions[0].source_policy.as_str()
    );
    assert_eq!(restored, before);
}

#[test]
fn meaningful_inputs_policy_and_every_pin_family_change_hash() {
    let baseline = checkpoint("graze", &[18, 4, 2], "policy-rev-1", pins());
    let baseline_bytes =
        crate::checkpoint_codec::encode_checkpoint(&baseline, codec_limits()).unwrap();
    let baseline_hash = canonical_hash(&baseline).unwrap();
    for candidate in [
        checkpoint("no-graze", &[18, 4, 2], "policy-rev-1", pins()),
        checkpoint("graze", &[18, 2, 4], "policy-rev-1", pins()),
        checkpoint("graze", &[18, 5, 2], "policy-rev-1", pins()),
        checkpoint("no-graze", &[7], "policy-rev-1", pins()),
        checkpoint("graze", &[7], "policy-rev-1", pins()),
        checkpoint("graze", &[18, 4, 2], "policy-rev-2", pins()),
    ] {
        assert_ne!(
            crate::checkpoint_codec::encode_checkpoint(&candidate, codec_limits()).unwrap(),
            baseline_bytes
        );
        assert_ne!(canonical_hash(&candidate).unwrap(), baseline_hash);
    }
    let mut variants = Vec::new();
    let mut value = pins();
    value.rules.handler_digest = ContentDigest([99; 32]);
    variants.push(value);
    let mut value = pins();
    value.rules.catalog_digest = ContentDigest([98; 32]);
    variants.push(value);
    let mut value = pins();
    value.rules.source_manifest_digest = ContentDigest([97; 32]);
    variants.push(value);
    let mut value = pins();
    value.content.content_digest = ContentDigest([96; 32]);
    variants.push(value);
    let mut value = pins();
    value.build = BuildIdentity::new(
        Some("other-source"),
        Some("other-native"),
        Some("wasm-not-claimed"),
        Some("replay-input-hash-contract-v1"),
        Some("local-journey-content-1"),
    )
    .unwrap();
    variants.push(value);
    for variant in variants {
        assert_ne!(
            canonical_hash(&checkpoint("graze", &[18, 4, 2], "policy-rev-1", variant)).unwrap(),
            baseline_hash
        );
    }
}

#[test]
fn existing_typed_admission_refusals_do_not_change_checkpoint_or_draws() {
    let original = checkpoint("graze", &[18, 4, 2], "policy-rev-1", pins());
    let before = original.clone();
    let bytes = crate::checkpoint_codec::encode_checkpoint(&original, codec_limits()).unwrap();
    let hash = canonical_hash(&original).unwrap();
    assert_eq!(
        decode(&bytes[..5], original.basis(), original.pins()),
        Err(crate::checkpoint_codec::CodecError::Truncated)
    );
    let mut malformed = bytes.clone();
    malformed[..4].copy_from_slice(b"NOPE");
    assert_eq!(
        decode(&malformed, original.basis(), original.pins()),
        Err(crate::checkpoint_codec::CodecError::UnsupportedCodec)
    );
    for version in [1_u16, 2_u16, u16::MAX] {
        let mut wrong_codec = bytes.clone();
        wrong_codec[4..6].copy_from_slice(&version.to_be_bytes());
        assert_eq!(
            decode(&wrong_codec, original.basis(), original.pins()),
            Err(crate::checkpoint_codec::CodecError::UnsupportedCodec)
        );
    }
    for schema in [1_u16, u16::MAX] {
        let mut wrong_schema = bytes.clone();
        wrong_schema[6..8].copy_from_slice(&schema.to_be_bytes());
        assert_eq!(
            decode(&wrong_schema, original.basis(), original.pins()),
            Err(crate::checkpoint_codec::CodecError::Checkpoint(
                CheckpointError::UnsupportedSchema
            ))
        );
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(
        decode(&trailing, original.basis(), original.pins()),
        Err(crate::checkpoint_codec::CodecError::TrailingBytes)
    );
    let stale = Basis {
        revision: SessionRevision::new(RecoveryEpoch::new(2).unwrap(), 9),
        ..original.basis()
    };
    assert_eq!(
        decode(&bytes, stale, original.pins()),
        Err(crate::checkpoint_codec::CodecError::Checkpoint(
            CheckpointError::StaleBasis
        ))
    );
    let mut wrong_pins = original.pins().clone();
    wrong_pins.content.package_digest = ContentDigest([77; 32]);
    assert_eq!(
        decode(&bytes, original.basis(), &wrong_pins),
        Err(crate::checkpoint_codec::CodecError::Checkpoint(
            CheckpointError::ContentMismatch
        ))
    );
    let mut short = codec_limits();
    short.maximum_document_bytes = bytes.len() - 1;
    let rules = [source()];
    let contents = [content()];
    assert_eq!(
        crate::checkpoint_codec::decode_checkpoint(
            &bytes,
            original.basis(),
            original.pins(),
            ReferenceInventory {
                rules: &rules,
                content: &contents,
                resources: &[],
                assets: &[],
            },
            limits(),
            short
        ),
        Err(crate::checkpoint_codec::CodecError::Capacity)
    );
    assert_eq!(original, before);
    assert_eq!(canonical_hash(&original).unwrap(), hash);
    let inputs = recorded_inputs(&original);
    assert_eq!(
        replay_selected(selected_prestate(), &original, &inputs).unwrap(),
        original
    );
}

#[test]
fn sha256_known_vector_is_the_standard_digest() {
    assert_eq!(
        format!("{:x}", Sha256::digest(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
