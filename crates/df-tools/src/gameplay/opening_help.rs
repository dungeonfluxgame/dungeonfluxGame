//! Actor-owned Help declarations for the opening Influence check (SRD 5.2.1 pp. 182–183).
use super::*;
use df_rules::influence::ModeSource;
use df_types::OperationId;

const POLICY: &str = "opening-help-1";
const PREFIX: &str = "opening-help-1:";
pub(super) const EVENT: &str = "opening-help-declared";

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct HelpRecord {
    helper: [u8; 16],
    beneficiary: [u8; 16],
    session: [u8; 16],
    run: [u8; 16],
    beat: String,
    target: String,
    request: String,
    skill: OpeningSkill,
    source_line: String,
    reason: String,
}

/// A server Jev judgement of the helper's own words. Actor authentication supplies `helper`.
pub(in crate::gameplay) struct Declaration<'a> {
    pub(in crate::gameplay) helper: MemberId,
    pub(in crate::gameplay) beneficiary: MemberId,
    pub(in crate::gameplay) skill: OpeningSkill,
    pub(in crate::gameplay) source_line: &'a str,
    pub(in crate::gameplay) reason: &'a str,
}

fn read(decision: &AcceptedDecision) -> Result<Option<HelpRecord>, RepositoryError> {
    if decision.source_policy.as_str() != POLICY {
        return Ok(None);
    }
    let text = decision
        .semantic_output
        .as_deref()
        .and_then(|text| text.strip_prefix(PREFIX))
        .ok_or_else(|| refusal("HelpReceipt"))?;
    serde_json::from_str(text)
        .map(Some)
        .map_err(|_| refusal("HelpReceiptJson"))
}

fn actor(state: &GameState, member: &[u8; 16]) -> Result<EntityId, RepositoryError> {
    state
        .members
        .iter()
        .find(|link| link.member.as_bytes() == member)
        .and_then(|link| link.character)
        .ok_or_else(|| refusal("HelpHero"))
}

fn validate_record(state: &GameState, record: &HelpRecord) -> Result<(), RepositoryError> {
    if record.helper == record.beneficiary
        || record.target != "bandit"
        || record.request != "stop-obstructing-courier"
        || !matches!(
            record.beat.as_str(),
            "opening" | "courier-answer-seal" | "courier-answer-escort"
        )
        || record.source_line.trim().is_empty()
        || record.source_line.len() > 4096
        || record.reason.trim().is_empty()
        || record.reason.chars().count() > 300
    {
        return invalid("HelpDeclaration");
    }
    let helper = actor(state, &record.helper)?;
    actor(state, &record.beneficiary)?;
    if skill_check_modifiers(state, helper, record.skill.name())?.1 == 0 {
        return invalid("HelpProficiency");
    }
    Ok(())
}

pub(super) fn validate_history(state: &GameState) -> Result<(), RepositoryError> {
    for decision in &state.decisions {
        let Some(record) = read(decision)? else {
            continue;
        };
        validate_record(state, &record)?;
        if !decision.draws.is_empty()
            || !decision.effects.is_empty()
            || decision.facts.len() != 1
            || state
                .draws
                .iter()
                .any(|draw| draw.operation == decision.operation)
        {
            return invalid("HelpDecision");
        }
        let fact = state
            .facts
            .iter()
            .find(|fact| decision.facts.contains(&fact.id))
            .ok_or_else(|| refusal("HelpFact"))?;
        let position = state
            .facts
            .iter()
            .position(|entry| entry.id == fact.id)
            .ok_or_else(|| refusal("HelpFact"))?;
        let subjects = [
            actor(state, &record.helper)?,
            actor(state, &record.beneficiary)?,
        ];
        if fact.operation != decision.operation
            || fact.revision != decision.revision
            || fact.ordinal != 0
            || fact.audience != AudienceScope::Shared
            || fact.cause
                != position
                    .checked_sub(1)
                    .and_then(|index| state.facts.get(index))
                    .map(|prior| prior.id)
            || !matches!(&fact.value, FactValue::ContentEvent { definition, subjects: actual }
                if *definition == model::content(EVENT)? && actual == &subjects)
        {
            return invalid("HelpFactBinding");
        }
    }
    Ok(())
}

fn matches_check(record: &HelpRecord, social: &OpeningSocialRecord) -> bool {
    social.member == record.beneficiary
        && social.skill.name() == record.skill.name()
        && social.beat == record.beat
        && social.willingness == "hesitant"
}

fn social_record(
    decision: &AcceptedDecision,
) -> Result<Option<OpeningSocialRecord>, RepositoryError> {
    if decision.source_policy.as_str() != OPENING_SOCIAL_POLICY {
        return Ok(None);
    }
    let text = decision
        .semantic_output
        .as_deref()
        .and_then(|text| text.strip_prefix(OPENING_SOCIAL_PREFIX))
        .ok_or_else(|| refusal("SocialReceipt"))?;
    serde_json::from_str(text)
        .map(Some)
        .map_err(|_| refusal("SocialReceiptJson"))
}

fn spent_before(
    state: &GameState,
    help: &AcceptedDecision,
    record: &HelpRecord,
    before: Option<df_types::SessionRevision>,
) -> Result<bool, RepositoryError> {
    for decision in state.decisions.iter().filter(|decision| {
        decision.revision > help.revision && before.is_none_or(|before| decision.revision < before)
    }) {
        if let Some(social) = social_record(decision)?
            && (matches_check(record, &social) || social.member == record.helper)
        {
            return Ok(true);
        }
        if let Some(next) = read(decision)?
            && next.helper == record.helper
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn source(state: &GameState, record: &HelpRecord) -> Result<ModeSource, RepositoryError> {
    let helper = journey::name(state, actor(state, &record.helper)?)?;
    Ok(ModeSource {
        advantage: true,
        label: format!("Help from {helper}: {}", record.reason),
    })
}

/// The next matching check receives at most one Help, revalidated against current presence.
pub(in crate::gameplay) fn source_for(
    current: &Checkpoint,
    beneficiary: MemberId,
    skill: OpeningSkill,
) -> Result<Option<(OperationId, ModeSource)>, RepositoryError> {
    validate_history(current.state())?;
    let beat = beat_script::with_script(|script| script.current(current.state()).cloned())?;
    for decision in &current.state().decisions {
        let Some(record) = read(decision)? else {
            continue;
        };
        if record.session != *current.basis().session.as_bytes()
            || record.run != *current.basis().run.as_bytes()
        {
            return invalid("HelpScope");
        }
        if record.beneficiary != *beneficiary.as_bytes()
            || record.skill.name() != skill.name()
            || record.beat != beat.state.durable
            || spent_before(current.state(), decision, &record, None)?
        {
            continue;
        }
        if !present(current.state(), &record.helper)?
            || !present(current.state(), &record.beneficiary)?
        {
            continue;
        }
        return Ok(Some((
            decision.operation,
            source(current.state(), &record)?,
        )));
    }
    Ok(None)
}

fn present(state: &GameState, member: &[u8; 16]) -> Result<bool, RepositoryError> {
    let entity = actor(state, member)?;
    if state.continuity.presence.iter().any(|presence| {
        presence.member.as_bytes() == member && presence.state != PresenceKind::Connected
    }) {
        return Ok(false);
    }
    // Legacy local checkpoints have no remote-presence rows. The canonical scene projection
    // still proves the hero is in the foreground, rather than inventing a connection state.
    let scene = scene(state)?;
    let name = journey::name(state, entity)?;
    Ok(
        journey::living_scene::snapshot(state, &scene)?.is_some_and(|room| {
            room.model
                .people
                .iter()
                .any(|person| person.role == "hero" && person.name == name && !person.is_background)
        }),
    )
}

pub(super) fn validate_consumption(
    state: &GameState,
    decision: &AcceptedDecision,
    social: &OpeningSocialRecord,
    supplied: &OpeningModeSource,
) -> Result<(), RepositoryError> {
    let help = state
        .decisions
        .iter()
        .find(|entry| entry.operation.as_bytes() == &supplied.helper_operation)
        .ok_or_else(|| refusal("HelpConsumption"))?;
    let record = read(help)?.ok_or_else(|| refusal("HelpConsumption"))?;
    if help.revision >= decision.revision
        || record.session != supplied.helper_session
        || record.run != supplied.helper_run
        || !matches_check(&record, social)
        || spent_before(state, help, &record, Some(decision.revision))?
        || !supplied.advantage
        || supplied.label != source(state, &record)?.label
    {
        return invalid("HelpConsumption");
    }
    Ok(())
}

/// Stage a genuine, specific declaration after Jev appraisal; commit through the actor owner.
/// The helper and ally must be present, and the helper must have the chosen proficiency.
pub(in crate::gameplay) fn stage(
    current: &Checkpoint,
    operation: OperationId,
    declaration: Declaration<'_>,
) -> Result<Checkpoint, RepositoryError> {
    let beat = beat_script::with_script(|script| script.current(current.state()).cloned())?;
    if !beat_script::with_script(|script| script.is_canonical())
        || !current.state().encounters.is_empty()
        || opening_threat_resolved(current.state())?
        || current
            .state()
            .decisions
            .iter()
            .any(|decision| decision.operation == operation)
    {
        return invalid("HelpOpeningUnavailable");
    }
    let record = HelpRecord {
        helper: *declaration.helper.as_bytes(),
        beneficiary: *declaration.beneficiary.as_bytes(),
        session: *current.basis().session.as_bytes(),
        run: *current.basis().run.as_bytes(),
        beat: beat.state.durable,
        target: "bandit".to_owned(),
        request: "stop-obstructing-courier".to_owned(),
        skill: declaration.skill,
        source_line: declaration.source_line.to_owned(),
        reason: declaration.reason.to_owned(),
    };
    validate_record(current.state(), &record)?;
    if !present(current.state(), &record.helper)?
        || !present(current.state(), &record.beneficiary)?
        || source_for(current, declaration.beneficiary, declaration.skill)?.is_some()
    {
        return invalid("HelpAlreadyDeclaredOrAbsent");
    }
    let helper = actor(current.state(), &record.helper)?;
    let beneficiary = actor(current.state(), &record.beneficiary)?;
    let helper_location = current
        .state()
        .entities
        .iter()
        .find(|entity| entity.id == helper)
        .map(|entity| entity.location);
    let ally_location = current
        .state()
        .entities
        .iter()
        .find(|entity| entity.id == beneficiary)
        .map(|entity| entity.location);
    if helper_location != ally_location {
        return invalid("HelpNotNear");
    }
    let mut state = current.state().clone();
    let mut basis = current.basis();
    basis.revision = basis
        .revision
        .next_sequence()
        .map_err(|_| refusal("HelpRevision"))?;
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest([b"opening-help-1:".as_slice(), operation.as_bytes()].concat());
    let id = FactId::from_bytes(&digest[..16]).map_err(|_| refusal("HelpFactId"))?;
    state.facts.push(GameFact {
        id,
        revision: basis.revision,
        operation,
        ordinal: 0,
        cause: state.facts.last().map(|fact| fact.id),
        audience: AudienceScope::Shared,
        value: FactValue::ContentEvent {
            definition: model::content(EVENT)?,
            subjects: vec![helper, beneficiary],
        },
    });
    state.decisions.push(AcceptedDecision {
        operation,
        revision: basis.revision,
        facts: vec![id],
        draws: vec![],
        effects: vec![],
        source_policy: model::label(POLICY)?,
        semantic_output: Some(format!(
            "{PREFIX}{}",
            serde_json::to_string(&record).map_err(|_| refusal("HelpEncode"))?
        )),
    });
    validate_history(&state)?;
    model::checkpoint(basis, state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use df_rules::influence::{Attitude, RollMode, Willingness};

    fn with_opening(run: impl FnOnce(Checkpoint, MemberId, MemberId)) {
        let loaded = pack::load_from_bytes(
            include_bytes!("../../../../content/harbor-journey/pack.json"),
            include_bytes!("fixtures/harbor-journey-tuning-baseline.json"),
        )
        .unwrap();
        pack::with_test_pack(loaded, || {
            let current = opening_social_tests::opening(2);
            let members = journey::participants(&current);
            run(current.clone(), members[0].member, members[1].member);
        });
    }

    fn operation(value: u8) -> OperationId {
        OperationId::from_bytes(&[value; 16]).unwrap()
    }

    fn declare(current: &Checkpoint, helper: MemberId, beneficiary: MemberId) -> Checkpoint {
        stage(current, operation(141), Declaration {
            helper, beneficiary, skill: OpeningSkill::Intimidation,
            source_line: "Vale, I will stand at your shoulder and block Dain's retreat while you warn him.",
            reason: "the helper backs the named ally's warning with a credible presence",
        }).unwrap()
    }

    #[test]
    fn opening_help_is_a_single_recorded_advantage_and_hostile_cancels_it() {
        with_opening(|current, helper, beneficiary| {
            for attitude in [Attitude::Hostile, Attitude::Indifferent] {
                let declared = declare(&current, helper, beneficiary);
                let (_, source) = source_for(&declared, beneficiary, OpeningSkill::Intimidation)
                    .unwrap()
                    .unwrap();
                let mode = df_rules::influence::roll_mode(attitude, std::slice::from_ref(&source));
                assert_eq!(
                    mode,
                    if attitude == Attitude::Hostile {
                        RollMode::Normal
                    } else {
                        RollMode::Advantage
                    }
                );
                let faces: &[u8] = if mode == RollMode::Normal {
                    &[1]
                } else {
                    &[1, 1]
                };
                let attempt = || OpeningSocialAttempt {
                    member: beneficiary,
                    skill: OpeningSkill::Intimidation,
                    attitude,
                    willingness: Willingness::Hesitant,
                    faces,
                    changed_after: None,
                };
                assert!(
                    stage_opening_social(&declared, operation(142), attempt()).is_err(),
                    "pending Help cannot be silently omitted"
                );
                let failed = stage_opening_social_with_sources(
                    &declared,
                    operation(142),
                    attempt(),
                    &[source],
                )
                .unwrap();
                assert_eq!(failed.result.mode, mode);
                assert!(
                    source_for(&failed.checkpoint, beneficiary, OpeningSkill::Intimidation)
                        .unwrap()
                        .is_none()
                );
                assert!(!opening_threat_resolved(failed.checkpoint.state()).unwrap());
                let encoded = failed
                    .checkpoint
                    .state()
                    .decisions
                    .last()
                    .unwrap()
                    .semantic_output
                    .as_ref()
                    .unwrap();
                assert!(encoded.contains("helper_operation"));
                for tamper in 0..3 {
                    let mut state = failed.checkpoint.state().clone();
                    let decision = state.decisions.last_mut().unwrap();
                    let encoded = decision
                        .semantic_output
                        .as_deref()
                        .unwrap()
                        .strip_prefix(OPENING_SOCIAL_PREFIX)
                        .unwrap();
                    let mut record: OpeningSocialRecord = serde_json::from_str(encoded).unwrap();
                    match tamper {
                        0 => record.extra_sources[0].helper_operation = [200; 16],
                        1 => record.extra_sources[0].advantage = false,
                        _ => record.extra_sources.push(record.extra_sources[0].clone()),
                    }
                    decision.semantic_output = Some(format!(
                        "{OPENING_SOCIAL_PREFIX}{}",
                        serde_json::to_string(&record).unwrap()
                    ));
                    assert!(
                        opening_threat_resolved(&state).is_err(),
                        "consumption tamper {tamper}"
                    );
                }
                for session_scope in [true, false] {
                    let mut state = failed.checkpoint.state().clone();
                    let decision = state
                        .decisions
                        .iter_mut()
                        .find(|decision| decision.source_policy.as_str() == POLICY)
                        .unwrap();
                    let mut record = read(decision).unwrap().unwrap();
                    if session_scope {
                        record.session = [201; 16];
                    } else {
                        record.run = [202; 16];
                    }
                    decision.semantic_output = Some(format!(
                        "{PREFIX}{}",
                        serde_json::to_string(&record).unwrap()
                    ));
                    assert!(
                        opening_threat_resolved(&state).is_err(),
                        "consumed declaration scope remains bound during state replay"
                    );
                }
                let replay =
                    model::checkpoint(failed.checkpoint.basis(), failed.checkpoint.state().clone())
                        .unwrap();
                assert!(
                    source_for(&replay, beneficiary, OpeningSkill::Intimidation)
                        .unwrap()
                        .is_none()
                );
                let second = stage_opening_social(
                    &replay,
                    operation(143),
                    OpeningSocialAttempt {
                        member: beneficiary,
                        skill: OpeningSkill::Intimidation,
                        attitude,
                        willingness: Willingness::Hesitant,
                        faces: if attitude == Attitude::Hostile {
                            &[1, 1]
                        } else {
                            &[1]
                        },
                        changed_after: Some(operation(142)),
                    },
                )
                .unwrap();
                assert_eq!(
                    second.result.mode,
                    if attitude == Attitude::Hostile {
                        RollMode::Disadvantage
                    } else {
                        RollMode::Normal
                    }
                );
            }
        });
    }

    #[test]
    fn opening_help_rejects_self_unskilled_absent_stacking_and_tampered_receipts() {
        with_opening(|current, helper, beneficiary| {
            let declaration = |helper, beneficiary, skill| Declaration {
                helper,
                beneficiary,
                skill,
                source_line: "I support Vale's warning by standing with him.",
                reason: "credible assistance",
            };
            assert!(
                stage(
                    &current,
                    operation(141),
                    declaration(helper, helper, OpeningSkill::Intimidation)
                )
                .is_err()
            );
            assert!(
                stage(
                    &current,
                    operation(141),
                    declaration(helper, beneficiary, OpeningSkill::Persuasion)
                )
                .is_err()
            );
            let declared = declare(&current, helper, beneficiary);
            assert!(
                stage(
                    &declared,
                    operation(144),
                    declaration(helper, beneficiary, OpeningSkill::Intimidation)
                )
                .is_err()
            );
            let mut absent = current.state().clone();
            absent.continuity.presence.push(ParticipantPresence {
                member: helper,
                bindings: vec![],
                state: PresenceKind::Disconnected,
                policy: model::content("lantern-wharf").unwrap(),
            });
            let absent = model::checkpoint(current.basis(), absent).unwrap();
            assert!(
                stage(
                    &absent,
                    operation(145),
                    declaration(helper, beneficiary, OpeningSkill::Intimidation)
                )
                .is_err()
            );
            for field in ["helper", "target", "source_line", "run"] {
                let mut state = declared.state().clone();
                let decision = state.decisions.last_mut().unwrap();
                let mut record = read(decision).unwrap().unwrap();
                match field {
                    "helper" => record.helper = record.beneficiary,
                    "target" => record.target = "other-person".to_owned(),
                    "source_line" => record.source_line.clear(),
                    "run" => record.run = [199; 16],
                    _ => unreachable!(),
                }
                decision.semantic_output = Some(format!(
                    "{PREFIX}{}",
                    serde_json::to_string(&record).unwrap()
                ));
                let changed = model::checkpoint(declared.basis(), state).unwrap();
                assert!(
                    source_for(&changed, beneficiary, OpeningSkill::Intimidation).is_err(),
                    "tamper {field}"
                );
            }
        });
    }

    #[test]
    fn opening_help_no_roll_and_failed_retry_admission_leave_help_available() {
        with_opening(|current, helper, beneficiary| {
            let declared = declare(&current, helper, beneficiary);
            let refused = stage_opening_social(
                &declared,
                operation(146),
                OpeningSocialAttempt {
                    member: beneficiary,
                    skill: OpeningSkill::Intimidation,
                    attitude: Attitude::Hostile,
                    willingness: Willingness::Unwilling,
                    faces: &[],
                    changed_after: None,
                },
            )
            .unwrap();
            assert!(
                source_for(&refused.checkpoint, beneficiary, OpeningSkill::Intimidation)
                    .unwrap()
                    .is_some()
            );
            let (_, source) =
                source_for(&refused.checkpoint, beneficiary, OpeningSkill::Intimidation)
                    .unwrap()
                    .unwrap();
            assert!(
                stage_opening_social_with_sources(
                    &refused.checkpoint,
                    operation(147),
                    OpeningSocialAttempt {
                        member: beneficiary,
                        skill: OpeningSkill::Intimidation,
                        attitude: Attitude::Hostile,
                        willingness: Willingness::Hesitant,
                        faces: &[1],
                        changed_after: None,
                    },
                    &[source]
                )
                .is_err()
            );
            assert!(
                source_for(&refused.checkpoint, beneficiary, OpeningSkill::Intimidation)
                    .unwrap()
                    .is_some()
            );
        });
    }
}
