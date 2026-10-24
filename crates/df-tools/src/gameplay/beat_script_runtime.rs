//! Durable authored choices for scripts whose scenes need no encounter or new rules.
use super::{
    beat_script::{self, Beat, BeatEffect, BeatKind, BeatOffer, BeatScript, BeatTrigger},
    journey, model, pack, refusal_detail,
};
use df_model::checkpoint::*;
use df_protocol::common as rpc;
use df_session::submission::RepositoryError;
use df_types::MemberId;

/// A social outcome belongs to the beat journal, independently of combat victory.
const OPENING_SOCIAL_POLICY: &str = "opening-social-1";
const OPENING_SOCIAL_EVENT: &str = "opening-social-outcome";
const OPENING_SOCIAL_PREFIX: &str = "opening-social-1:";

#[path = "opening_help.rs"]
pub(super) mod opening_help;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct OpeningPartyChange {
    before: String,
    after: String,
    reason: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct OpeningModeSource {
    advantage: bool,
    label: String,
    helper_operation: [u8; 16],
    helper_session: [u8; 16],
    helper_run: [u8; 16],
}

#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum OpeningSkill {
    Persuasion,
    Deception,
    Intimidation,
}
impl OpeningSkill {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Persuasion => "persuasion",
            Self::Deception => "deception",
            Self::Intimidation => "intimidation",
        }
    }
    fn rules(self) -> df_rules::influence::InfluenceSkill {
        match self {
            Self::Persuasion => df_rules::influence::InfluenceSkill::Persuasion,
            Self::Deception => df_rules::influence::InfluenceSkill::Deception,
            Self::Intimidation => df_rules::influence::InfluenceSkill::Intimidation,
        }
    }
}
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct OpeningSocialRecord {
    member: [u8; 16],
    beat: String,
    skill: OpeningSkill,
    attitude: String,
    willingness: String,
    faces: Vec<u8>,
    succeeded: bool,
    changed_after: Option<[u8; 16]>,
    /// The owner retains the judged words for a changed-offer comparison after recovery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_line: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    party_change: Option<OpeningPartyChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    extra_sources: Vec<OpeningModeSource>,
}

fn opening_social_result(
    state: &GameState,
    record: &OpeningSocialRecord,
) -> Result<df_rules::influence::InfluenceResult, RepositoryError> {
    use df_rules::influence::{Attitude, InfluenceInput, Willingness, WordQuality, WordTilt};
    let member = MemberId::from_bytes(&record.member).map_err(|_| refusal("SocialMember"))?;
    let hero = state
        .members
        .iter()
        .find(|link| link.member == member)
        .and_then(|link| link.character)
        .ok_or_else(|| refusal("SocialHero"))?;
    let (ability_modifier, proficiency_bonus) =
        skill_check_modifiers(state, hero, record.skill.name())?;
    let attitude = match record.attitude.as_str() {
        "friendly" => Attitude::Friendly,
        "indifferent" => Attitude::Indifferent,
        "hostile" => Attitude::Hostile,
        _ => return invalid("SocialAttitude"),
    };
    let willingness = match record.willingness.as_str() {
        "willing" => Willingness::Willing,
        "hesitant" => Willingness::Hesitant,
        "unwilling" => Willingness::Unwilling,
        _ => return invalid("SocialWillingness"),
    };
    df_rules::influence::resolve(InfluenceInput {
        ability_modifier,
        proficiency_bonus,
        skill: record.skill.rules(),
        attitude,
        monster_intelligence: pack::bandit().abilities[3],
        willingness,
        words: WordQuality {
            tilt: WordTilt::Neutral,
            reason: "standard Influence",
        },
        extra_sources: &record
            .extra_sources
            .iter()
            .map(|source| df_rules::influence::ModeSource {
                advantage: source.advantage,
                label: source.label.clone(),
            })
            .collect::<Vec<_>>(),
        faces: &record.faces,
    })
    .map_err(|_| refusal("SocialCheck"))
}
fn social_succeeded(result: &df_rules::influence::InfluenceResult) -> bool {
    matches!(
        result.outcome,
        df_rules::influence::InfluenceOutcome::Success
            | df_rules::influence::InfluenceOutcome::NoRoll(
                df_rules::influence::NoRollCause::Willing
            )
    )
}

thread_local! {
    static OPENING_THREAT: std::cell::RefCell<Option<(GameState, pack::CacheIdentity, bool)>> =
        const { std::cell::RefCell::new(None) };
}
/// Replays the accepted social receipt and its actual dice; loose outcome facts confer nothing.
pub(super) fn opening_threat_resolved(state: &GameState) -> Result<bool, RepositoryError> {
    let pack = pack::cache_identity();
    let remembered = OPENING_THREAT.with(|cache| {
        cache
            .borrow()
            .as_ref()
            .filter(|(input, source, _)| *source == pack && input == state)
            .map(|(_, _, result)| *result)
    });
    if let Some(result) = remembered {
        return Ok(result);
    }
    let result = derived_opening_threat_resolved(state)?;
    OPENING_THREAT.with(|cache| *cache.borrow_mut() = Some((state.clone(), pack, result)));
    Ok(result)
}
fn derived_opening_threat_resolved(state: &GameState) -> Result<bool, RepositoryError> {
    let mut resolved = false;
    let mut prior_attempt = None;
    let mut party_eased = false;
    opening_help::validate_history(state)?;
    for decision in &state.decisions {
        if decision.source_policy.as_str() != OPENING_SOCIAL_POLICY {
            continue;
        }
        let text = decision
            .semantic_output
            .as_deref()
            .and_then(|text| text.strip_prefix(OPENING_SOCIAL_PREFIX))
            .ok_or_else(|| refusal("SocialReceipt"))?;
        let record: OpeningSocialRecord =
            serde_json::from_str(text).map_err(|_| refusal("SocialReceiptJson"))?;
        if resolved
            || !matches!(
                record.beat.as_str(),
                "opening" | "courier-answer-seal" | "courier-answer-escort"
            )
            || !decision.effects.is_empty()
            || record
                .source_line
                .as_ref()
                .is_some_and(|line| line.trim().is_empty() || line.len() > 4096)
        {
            return invalid("SocialHistory");
        }
        if record.changed_after != prior_attempt {
            return invalid("SocialRetryBasis");
        }
        let mut prior_beat = None;
        for prior in state.decisions.iter().filter(|prior| {
            prior.revision < decision.revision
                && prior.source_policy.as_str() == journey::THREAD_POLICY
        }) {
            // Disclosure replacement must not erase the native courier answer's historical beat.
            if state.intents.iter().any(|intent| {
                intent.operation == prior.operation
                    && intent.basis.revision == prior.revision
                    && intent.definition.entry.as_str() == super::courier_ai::DEFINITION
                    && prior.effects.contains(&intent.id)
            }) {
                prior_beat = Some("courier-answer-seal");
            }
            if let Some(fact) = state
                .facts
                .iter()
                .find(|fact| prior.facts.last() == Some(&fact.id))
                && let FactValue::ContentEvent { definition, .. } = &fact.value
            {
                match definition.entry.as_str() {
                    "begin-story" => prior_beat = Some("opening"),
                    "private-courier-note" => prior_beat = Some("courier-answer-seal"),
                    "escort-courier" => prior_beat = Some("courier-answer-escort"),
                    "defend-courier" => return invalid("SocialAfterViolence"),
                    _ => {}
                }
            }
        }
        if prior_beat != Some(record.beat.as_str()) {
            return invalid("SocialBeatBasis");
        }
        if let Some(change) = &record.party_change {
            if party_eased
                || change.before != "hostile"
                || change.after != "indifferent"
                || change.reason != "Jev judged a concrete offer against Dain's wants"
                || record.willingness != "hesitant"
            {
                return invalid("SocialPartyChange");
            }
            party_eased = true;
        }
        if record.extra_sources.len() > 1 {
            return invalid("SocialHelpCount");
        }
        for source in &record.extra_sources {
            opening_help::validate_consumption(state, decision, &record, source)?;
        }
        let result = opening_social_result(state, &record)?;
        if record.succeeded != social_succeeded(&result) || record.faces != result.faces {
            return invalid("SocialOutcome");
        }
        let draws: Vec<_> = state
            .draws
            .iter()
            .filter(|draw| draw.operation == decision.operation)
            .collect();
        if draws.len() != record.faces.len()
            || decision.draws.len() != draws.len()
            || decision.facts.len() != draws.len() + 1
        {
            return invalid("SocialDrawCount");
        }
        for (index, (draw, face)) in draws.iter().zip(&record.faces).enumerate() {
            if draw.ordinal != index as u32
                || decision.draws[index] != index as u32
                || draw.sides != 20
                || draw.value != u32::from(*face)
                || draw.source != journey::rule()?
                || draw.resolution.as_bytes() != decision.operation.as_bytes()
                || draw.window.as_bytes() != decision.operation.as_bytes()
            {
                return invalid("SocialDrawBinding");
            }
        }
        for (index, id) in decision.facts.iter().enumerate() {
            let fact = state
                .facts
                .iter()
                .find(|fact| fact.id == *id)
                .ok_or_else(|| refusal("SocialFactMissing"))?;
            if fact.operation != decision.operation
                || fact.revision != decision.revision
                || fact.ordinal != index as u32
                || fact.audience != AudienceScope::Shared
            {
                return invalid("SocialFactBinding");
            }
            let position = state
                .facts
                .iter()
                .position(|entry| entry.id == fact.id)
                .ok_or_else(|| refusal("SocialFactPosition"))?;
            if fact.cause
                != position
                    .checked_sub(1)
                    .and_then(|prior| state.facts.get(prior))
                    .map(|prior| prior.id)
            {
                return invalid("SocialFactCause");
            }
            let correct = if index < draws.len() {
                matches!(fact.value, FactValue::DrawAccepted { operation, ordinal }
                    if operation == decision.operation && ordinal == index as u32)
            } else {
                matches!(&fact.value, FactValue::ContentEvent { definition, subjects }
                    if *definition == model::content(OPENING_SOCIAL_EVENT)? && subjects.is_empty())
            };
            if !correct {
                return invalid("SocialFactValue");
            }
        }
        resolved = record.succeeded;
        prior_attempt = Some(*decision.operation.as_bytes());
    }
    Ok(resolved)
}

pub(super) struct OpeningSocialOutcome {
    pub(super) checkpoint: Checkpoint,
    pub(super) result: df_rules::influence::InfluenceResult,
    pub(super) line: String,
}
/// Bind the semantic receipt to the actual judged request before the owner commits it.
/// This leaves the staging contract usable by rule-only callers without inventing player words.
pub(super) fn bind_opening_social_source(
    outcome: &mut OpeningSocialOutcome,
    words: &str,
) -> Result<(), RepositoryError> {
    if words.trim().is_empty() || words.len() > 4096 {
        return invalid("SocialSourceBounds");
    }
    let mut state = outcome.checkpoint.state().clone();
    let decision = state
        .decisions
        .last_mut()
        .ok_or_else(|| refusal("SocialSource"))?;
    let encoded = decision
        .semantic_output
        .as_deref()
        .and_then(|text| text.strip_prefix(OPENING_SOCIAL_PREFIX))
        .ok_or_else(|| refusal("SocialSource"))?;
    let mut record: OpeningSocialRecord =
        serde_json::from_str(encoded).map_err(|_| refusal("SocialSource"))?;
    record.source_line = Some(words.to_owned());
    decision.semantic_output = Some(format!(
        "{OPENING_SOCIAL_PREFIX}{}",
        serde_json::to_string(&record).map_err(|_| refusal("SocialSource"))?
    ));
    outcome.checkpoint = model::checkpoint(outcome.checkpoint.basis(), state)?;
    Ok(())
}

pub(super) fn previous_opening_social(
    current: &Checkpoint,
) -> Result<Option<(df_types::OperationId, Option<String>)>, RepositoryError> {
    let Some(decision) = current
        .state()
        .decisions
        .iter()
        .rev()
        .find(|decision| decision.source_policy.as_str() == OPENING_SOCIAL_POLICY)
    else {
        return Ok(None);
    };
    let encoded = decision
        .semantic_output
        .as_deref()
        .and_then(|text| text.strip_prefix(OPENING_SOCIAL_PREFIX))
        .ok_or_else(|| refusal("SocialSource"))?;
    let record: OpeningSocialRecord =
        serde_json::from_str(encoded).map_err(|_| refusal("SocialSource"))?;
    Ok(Some((decision.operation, record.source_line)))
}

/// A durable party change does not disappear when another hero speaks or an attempt fails.
pub(super) fn opening_party_attitude(
    state: &GameState,
) -> Result<Option<(df_rules::influence::Attitude, Option<String>)>, RepositoryError> {
    opening_threat_resolved(state)?;
    let mut standing = None;
    for decision in &state.decisions {
        if decision.source_policy.as_str() != OPENING_SOCIAL_POLICY {
            continue;
        }
        let encoded = decision
            .semantic_output
            .as_deref()
            .and_then(|text| text.strip_prefix(OPENING_SOCIAL_PREFIX))
            .ok_or_else(|| refusal("SocialReceipt"))?;
        let record: OpeningSocialRecord =
            serde_json::from_str(encoded).map_err(|_| refusal("SocialReceiptJson"))?;
        if record.party_change.is_some() {
            standing = Some((
                df_rules::influence::Attitude::Indifferent,
                record.source_line,
            ));
        }
    }
    Ok(standing)
}

/// Jev compares against every failed urging, so alternating heroes or offers cannot erase it.
pub(super) fn opening_social_history(state: &GameState) -> Result<Vec<String>, RepositoryError> {
    let mut history = Vec::new();
    for decision in &state.decisions {
        if decision.source_policy.as_str() != OPENING_SOCIAL_POLICY {
            continue;
        }
        let encoded = decision
            .semantic_output
            .as_deref()
            .and_then(|text| text.strip_prefix(OPENING_SOCIAL_PREFIX))
            .ok_or_else(|| refusal("SocialReceipt"))?;
        let record: OpeningSocialRecord =
            serde_json::from_str(encoded).map_err(|_| refusal("SocialReceiptJson"))?;
        if !record.succeeded {
            history.push(
                record
                    .source_line
                    .unwrap_or_else(|| "previous words unavailable".to_owned()),
            );
        }
    }
    Ok(history)
}

/// The session owner calls this only for Jev's explicit request to end this obstruction.
/// SRD p184: DC max(15, Intelligence); attitude changes roll mode. Failed attempts never attack.
pub(super) struct OpeningSocialAttempt<'a> {
    pub(super) member: MemberId,
    pub(super) skill: OpeningSkill,
    pub(super) attitude: df_rules::influence::Attitude,
    pub(super) willingness: df_rules::influence::Willingness,
    pub(super) faces: &'a [u8],
    /// Jev must explicitly judge a different offer or approach before another roll is admitted.
    pub(super) changed_after: Option<df_types::OperationId>,
}
pub(super) fn stage_opening_social(
    current: &Checkpoint,
    operation: df_types::OperationId,
    attempt: OpeningSocialAttempt<'_>,
) -> Result<OpeningSocialOutcome, RepositoryError> {
    stage_opening_social_with_sources(current, operation, attempt, &[])
}

/// The owner supplies the currently available typed Help source before drawing the check.
pub(super) fn stage_opening_social_with_sources(
    current: &Checkpoint,
    operation: df_types::OperationId,
    attempt: OpeningSocialAttempt<'_>,
    extra_sources: &[df_rules::influence::ModeSource],
) -> Result<OpeningSocialOutcome, RepositoryError> {
    stage_opening_social_appraised(current, operation, attempt, extra_sources, None)
}

/// Separate party appraisal keeps a personal exception from deciding the group's relationship.
/// `Some(before)` is only supplied after Jev approves a useful offer against that party baseline.
pub(super) fn stage_opening_social_appraised(
    current: &Checkpoint,
    operation: df_types::OperationId,
    attempt: OpeningSocialAttempt<'_>,
    extra_sources: &[df_rules::influence::ModeSource],
    approved_party_easing: Option<df_rules::influence::Attitude>,
) -> Result<OpeningSocialOutcome, RepositoryError> {
    let OpeningSocialAttempt {
        member,
        skill,
        attitude,
        willingness,
        faces,
        changed_after,
    } = attempt;
    use df_rules::influence::Willingness;
    journey::player_entity(member, current)?;
    journey::phase(current)?;
    let state = current.state();
    let beat = beat_script::with_script(|script| script.current(state).cloned())?;
    if !matches!(
        beat.state.durable.as_str(),
        "opening" | "courier-answer-seal" | "courier-answer-escort"
    ) || !state.encounters.is_empty()
        || opening_threat_resolved(state)?
        || state
            .decisions
            .iter()
            .any(|decision| decision.operation == operation)
        || !beat_script::with_script(|script| script.is_canonical())
    {
        return invalid("SocialOpeningUnavailable");
    }
    let previous = state
        .decisions
        .iter()
        .rev()
        .find(|decision| decision.source_policy.as_str() == OPENING_SOCIAL_POLICY)
        .map(|decision| decision.operation);
    if previous != changed_after {
        return invalid("SocialChangedApproachRequired");
    }
    let helper = if willingness == Willingness::Hesitant {
        opening_help::source_for(current, member, skill)?
    } else {
        None
    };
    let stored_sources = match (extra_sources, helper) {
        ([], None) => Vec::new(),
        ([source], Some((helper_operation, expected))) if *source == expected => {
            vec![OpeningModeSource {
                advantage: source.advantage,
                label: source.label.clone(),
                helper_operation: *helper_operation.as_bytes(),
                helper_session: *current.basis().session.as_bytes(),
                helper_run: *current.basis().run.as_bytes(),
            }]
        }
        _ => return invalid("SocialHelpSource"),
    };
    let enforcer = super::dialogue::opening_social_enforcer(current)?;
    let hero = journey::name(state, journey::player_entity(member, current)?)?;
    let base = opening_party_attitude(state)?
        .map(|(attitude, _)| attitude)
        .unwrap_or_else(|| super::dialogue::opening_scene_attitude(enforcer.attitude));
    if approved_party_easing.is_some_and(|before| before != base)
        || approved_party_easing
            .is_some_and(|before| before != df_rules::influence::Attitude::Hostile)
        || (approved_party_easing.is_some() && willingness != Willingness::Hesitant)
    {
        return invalid("SocialPartyAppraisal");
    }
    // Existing actor calls prove a useful offer by changing a non-exception hero's stance.
    // A separate explicit appraisal lets an exception hero improve the party without erasing
    // their own stance or letting a weak personal-Indifferent attempt improve the group.
    let inferred_easing = base == df_rules::influence::Attitude::Hostile
        && attitude == df_rules::influence::Attitude::Indifferent
        && willingness == Willingness::Hesitant
        && !enforcer.attitudes.contains_key(&hero);
    let party_change =
        (inferred_easing || approved_party_easing.is_some()).then(|| OpeningPartyChange {
            before: "hostile".to_owned(),
            after: "indifferent".to_owned(),
            reason: "Jev judged a concrete offer against Dain's wants".to_owned(),
        });
    let mut record = OpeningSocialRecord {
        member: *member.as_bytes(),
        beat: beat.state.durable,
        skill,
        attitude: attitude.word().to_owned(),
        willingness: match willingness {
            Willingness::Willing => "willing",
            Willingness::Hesitant => "hesitant",
            Willingness::Unwilling => "unwilling",
        }
        .to_owned(),
        faces: faces.to_vec(),
        succeeded: false,
        changed_after: changed_after.map(|operation| *operation.as_bytes()),
        source_line: None,
        party_change,
        extra_sources: if willingness == Willingness::Hesitant {
            stored_sources
        } else {
            Vec::new()
        },
    };
    let result = opening_social_result(state, &record)?;
    record.succeeded = social_succeeded(&result);
    record.faces = result.faces.clone();
    let mut state = state.clone();
    let mut basis = current.basis();
    basis.revision = basis
        .revision
        .next_sequence()
        .map_err(|_| refusal("SocialRevision"))?;
    let mut facts = Vec::new();
    for index in 0..=record.faces.len() {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(
            [
                b"opening-social-1:".as_slice(),
                operation.as_bytes(),
                &(index as u32).to_le_bytes(),
            ]
            .concat(),
        );
        let id = FactId::from_bytes(&digest[..16]).map_err(|_| refusal("SocialFactId"))?;
        let value = if let Some(face) = record.faces.get(index) {
            state.draws.push(ActualDraw {
                operation,
                ordinal: index as u32,
                resolution: ResolutionId::from_bytes(operation.as_bytes())
                    .map_err(|_| refusal("SocialResolution"))?,
                window: WindowId::from_bytes(operation.as_bytes())
                    .map_err(|_| refusal("SocialWindow"))?,
                sides: 20,
                value: u32::from(*face),
                source: journey::rule()?,
            });
            FactValue::DrawAccepted {
                operation,
                ordinal: index as u32,
            }
        } else {
            FactValue::ContentEvent {
                definition: model::content(OPENING_SOCIAL_EVENT)?,
                subjects: vec![],
            }
        };
        state.facts.push(GameFact {
            id,
            revision: basis.revision,
            operation,
            ordinal: index as u32,
            cause: state.facts.last().map(|fact| fact.id),
            audience: AudienceScope::Shared,
            value,
        });
        facts.push(id);
    }
    state.decisions.push(AcceptedDecision {
        operation,
        revision: basis.revision,
        facts,
        draws: (0..record.faces.len() as u32).collect(),
        effects: vec![],
        source_policy: model::label(OPENING_SOCIAL_POLICY)?,
        semantic_output: Some(format!(
            "{OPENING_SOCIAL_PREFIX}{}",
            serde_json::to_string(&record).map_err(|_| refusal("SocialEncode"))?
        )),
    });
    let checkpoint = model::checkpoint(basis, state)?;
    opening_threat_resolved(checkpoint.state())?;
    let line = if record.succeeded {
        "Dain steps aside; Finn keeps the packet, and the road to the market opens."
    } else {
        ""
    }
    .to_owned();
    Ok(OpeningSocialOutcome {
        checkpoint,
        result,
        line,
    })
}

pub(super) const STEP: &str = "beat-script-step";
/// The submitted action entry of an authored investigation (`observe`) choice.
pub(super) const CHOICE_ACTION: &str = "script-choice";
/// The submitted action entry of a resolution choice.
pub(super) const RESOLVE_ACTION: &str = "resolve-errand";
const OFFER_PREFIX: &str = "beat-script-offer-";
fn refusal(cause: &'static str) -> RepositoryError {
    refusal_detail::cause("beat-script-state", cause.to_owned());
    RepositoryError::InvalidCandidate
}
fn invalid<T>(cause: &'static str) -> Result<T, RepositoryError> {
    Err(refusal(cause))
}
pub(super) fn marker(offer: &str) -> Result<ContentReference, RepositoryError> {
    model::content(&format!("{OFFER_PREFIX}{offer}"))
}
pub(super) fn choice<'a>(
    state: &'a GameState,
    decision: &'a AcceptedDecision,
) -> Result<Option<(&'a GameFact, &'a str, MemberId)>, RepositoryError> {
    let mut chosen = None;
    for fact in &state.facts {
        let FactValue::ContentEvent {
            definition,
            subjects,
        } = &fact.value
        else {
            continue;
        };
        let Some(offer) = definition.entry.as_str().strip_prefix(OFFER_PREFIX) else {
            continue;
        };
        if !decision.facts.contains(&fact.id) {
            continue;
        }
        if chosen.is_some()
            || fact.operation != decision.operation
            || fact.revision != decision.revision
            || decision.facts.get(fact.ordinal as usize) != Some(&fact.id)
            || fact.audience != AudienceScope::Shared
            || decision.source_policy.as_str() != journey::THREAD_POLICY
        {
            return invalid("ChoiceFactBinding");
        }
        let [actor] = subjects.as_slice() else {
            return invalid("ChoiceSubjectCount");
        };
        if *definition != model::content(definition.entry.as_str())? {
            return invalid("ChoiceDefinition");
        }
        let index = state
            .facts
            .iter()
            .position(|f| f.id == fact.id)
            .ok_or_else(|| refusal("ChoiceFactMissing"))?;
        let preceding = index.checked_sub(1).and_then(|i| state.facts.get(i));
        if fact.cause != preceding.map(|f| f.id)
            || (fact.ordinal > 0
                && preceding.is_none_or(|f| {
                    f.operation != fact.operation
                        || f.revision != fact.revision
                        || f.ordinal.checked_add(1) != Some(fact.ordinal)
                        || decision.facts.get(f.ordinal as usize) != Some(&f.id)
                }))
        {
            return invalid("ChoiceCausalHistory");
        }
        let bootstrap = journey::bootstrap_member()?;
        let member = state
            .members
            .iter()
            .find(|link| link.character == Some(*actor) && link.member != bootstrap)
            .map(|link| link.member)
            .ok_or_else(|| refusal("ChoiceMemberMissing"))?;
        chosen = Some((fact, offer, member));
    }
    Ok(chosen)
}
thread_local! {
    static LAST_CHOICE: std::cell::RefCell<Option<(GameState, pack::CacheIdentity, Option<String>)>> =
        const { std::cell::RefCell::new(None) };
}
pub(super) fn last_choice(state: &GameState) -> Result<Option<String>, RepositoryError> {
    let pack = pack::cache_identity();
    let remembered = LAST_CHOICE.with(|cache| {
        cache
            .borrow()
            .as_ref()
            .filter(|(input, source, _)| *source == pack && input == state)
            .map(|(_, _, result)| result.clone())
    });
    if let Some(result) = remembered {
        return Ok(result);
    }
    let result = derived_last_choice(state)?;
    LAST_CHOICE.with(|cache| *cache.borrow_mut() = Some((state.clone(), pack, result.clone())));
    Ok(result)
}
fn derived_last_choice(state: &GameState) -> Result<Option<String>, RepositoryError> {
    let mut latest = None;
    let mut revision = None;
    let mut admitted = 0usize;
    for decision in &state.decisions {
        if revision.is_some_and(|previous| decision.revision <= previous) {
            return invalid("DecisionRevisionOrder");
        }
        revision = Some(decision.revision);
        if let Some((_, offer, _)) = choice(state, decision)? {
            latest = Some(offer.to_owned());
            admitted += 1;
        }
    }
    // A marker without its accepted decision is not an authored choice.
    let actual=state.facts.iter().filter(|fact|matches!(&fact.value,FactValue::ContentEvent{definition,..} if definition.entry.as_str().starts_with(OFFER_PREFIX))).count();
    if actual != admitted {
        return invalid("UnassociatedChoiceFact");
    }
    Ok(latest)
}
pub(super) fn scripted_id(current: &Checkpoint, action: rpc::GameplayActionKind) -> Option<String> {
    beat_script::with_script(|script| {
        if script.is_canonical() {
            return None;
        }
        let offer = script
            .current(current.state())
            .ok()?
            .offers
            .iter()
            .find(|o| o.effect.action() == action)?;
        Some(format!(
            "beat-{}:{}:{}",
            offer.id,
            current.basis().revision.epoch().get(),
            current.basis().revision.sequence()
        ))
    })
}
pub(super) fn phase_of(beat: &Beat) -> rpc::JourneyPhase {
    if beat.state.durable == "room" {
        return rpc::JourneyPhase::Room;
    }
    if beat.state.durable == "opening" {
        return rpc::JourneyPhase::Opening;
    }
    match beat.kind {
        BeatKind::Gather => rpc::JourneyPhase::Room,
        BeatKind::Converse | BeatKind::Investigate | BeatKind::Narrate => {
            rpc::JourneyPhase::Dialogue
        }
        BeatKind::Fight => rpc::JourneyPhase::Combat,
        BeatKind::Rest | BeatKind::Travel | BeatKind::Conclude | BeatKind::Ending => {
            rpc::JourneyPhase::Complete
        }
    }
}

/// SRD 5.2.1 p. 6: "Add your Proficiency Bonus to an ability check when the GM determines that a
/// skill or tool proficiency is relevant to the check and you have that proficiency." Search
/// (p. 187) says "you make a Wisdom check"; Study (p. 189) says "you make an Intelligence check";
/// Hide (p. 183) requires a "DC 15 Dexterity (Stealth) check."
/// Ability and proficiency components for the local SRD Search, Study and Hide skills.
pub(super) fn skill_check_modifiers(
    state: &GameState,
    actor: EntityId,
    skill: &str,
) -> Result<(i8, u8), RepositoryError> {
    let ability_name = match skill {
        "perception" | "insight" | "survival" | "medicine" | "animal-handling" => "wisdom",
        "investigation" | "arcana" | "history" | "nature" | "religion" => "intelligence",
        "stealth" | "sleight-of-hand" | "acrobatics" => "dexterity",
        "athletics" => "strength",
        "intimidation" | "persuasion" | "deception" | "performance" => "charisma",
        _ => {
            refusal_detail::cause("search-skill", "UnsupportedSkill".to_owned());
            return Err(RepositoryError::InvalidCandidate);
        }
    };
    let character = state
        .characters
        .iter()
        .find(|character| character.entity == actor)
        .ok_or_else(|| {
            refusal_detail::cause("search-character", "CharacterMissing".to_owned());
            RepositoryError::InvalidCandidate
        })?;
    let score = u8::try_from(journey::value(state, actor, ability_name)?).map_err(|error| {
        refusal_detail::cause("search-ability-score", format!("{error:?}"));
        RepositoryError::InvalidCandidate
    })?;
    let background_proficiency = character.build.entry.as_str() == "dwarf-fighter-soldier"
        && matches!(skill, "athletics" | "intimidation");
    let chosen_proficiency = character.choices.iter().any(|choice| {
        matches!(
            choice.offer.as_str(),
            "skill-1" | "skill-2" | "human-skill" | "keen-senses"
        ) && choice.selected.as_str() == skill
    });
    let proficiency = if background_proficiency || chosen_proficiency {
        pack::with_creation(|creation| creation.proficiency_bonus)
    } else {
        0
    };
    let ability_modifier = df_rules::local_journey::ability_modifier(score);
    let ability_modifier = i8::try_from(ability_modifier).map_err(|error| {
        refusal_detail::cause("search-ability-modifier", format!("{error:?}"));
        RepositoryError::InvalidCandidate
    })?;
    Ok((ability_modifier, proficiency))
}

/// Passive Perception for this local normal-check profile.
pub(super) fn passive_perception(
    state: &GameState,
    actor: EntityId,
) -> Result<i16, RepositoryError> {
    let (ability_modifier, proficiency) = skill_check_modifiers(state, actor, "perception")?;
    Ok(10 + i16::from(ability_modifier) + i16::from(proficiency))
}

pub(super) fn resolve_skill_check(
    state: &GameState,
    actor: EntityId,
    skill: &str,
    die: u8,
    difficulty_class: u8,
) -> Result<df_rules::ability_check::AbilityCheckResult, RepositoryError> {
    let (ability_modifier, proficiency_bonus) = skill_check_modifiers(state, actor, skill)?;
    df_rules::ability_check::resolve(df_rules::ability_check::AbilityCheckInput {
        die,
        ability_modifier,
        proficiency_bonus,
        difficulty_class,
    })
    .map_err(|error| {
        refusal_detail::cause("search-check", format!("{error:?}"));
        RepositoryError::InvalidCandidate
    })
}

struct SceneMemo {
    state: GameState,
    pack: pack::CacheIdentity,
    scene: pack::Scene,
}
thread_local! {
    static SCENE: std::cell::RefCell<Option<SceneMemo>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn scene(state: &GameState) -> Result<pack::Scene, RepositoryError> {
    let source = pack::cache_identity();
    let remembered = SCENE.with(|cache| {
        cache
            .borrow()
            .as_ref()
            .filter(|memo| memo.pack == source && memo.state == *state)
            .map(|memo| memo.scene.clone())
    });
    if let Some(scene) = remembered {
        return Ok(scene);
    }
    let scene = derived_scene(state)?;
    SCENE.with(|cache| {
        *cache.borrow_mut() = Some(SceneMemo {
            state: state.clone(),
            pack: source,
            scene: scene.clone(),
        })
    });
    Ok(scene)
}

fn derived_scene(state: &GameState) -> Result<pack::Scene, RepositoryError> {
    beat_script::with_script(|script| {
        let beat = script.current(state)?;
        let variant = shown_variant(script, beat, state)?;
        let mut scene = match &variant {
            Some(variant) => {
                let mut authored = script
                    .scenes
                    .get(variant)
                    .cloned()
                    .ok_or_else(|| refusal("AuthoredSceneVariantMissing"))?;
                authored.source_variant_id = Some(variant.clone());
                // Who stands on stage is the adventure's own cast, never the static base's.
                authored.cast = pack::scene(beat.scene).cast;
                if beat.kind == BeatKind::Ending {
                    // The closing scenes keep the patron on stage beside the contact.
                    for actor in pack::scene(pack::SceneKey::Opening).cast {
                        if !authored.cast.iter().any(|present| present.id == actor.id) {
                            authored.cast.push(actor);
                        }
                    }
                }
                authored
            }
            None => pack::scene(beat.scene),
        };
        if let Some(variant) = &variant
            && let Some(asset) =
                pack::active().and_then(|loaded| loaded.script_scene_assets.get(variant).cloned())
        {
            scene.asset = asset;
        }
        if script.is_canonical()
            && opening_threat_resolved(state)?
            && matches!(
                beat.state.durable.as_str(),
                "opening" | "courier-answer-seal" | "courier-answer-escort"
            )
        {
            // The committed social exchange tells this outcome after its roll. Re-projecting
            // the same beat must not speak it a second time (or race ahead of the roll).
            scene.narration.clear();
            scene.dialogue.clear();
        }
        // Where the party has gone from the beat's own place (see `place`).
        let mut scene = super::place::overlay(state, &beat.id, scene)?;
        if let Some(snapshot) = journey::living_scene::snapshot(state, &scene)?
            && let Some(setting) = scene.setting.as_mut()
        {
            setting.model = Some(snapshot.model);
        }

        // Inspection facts follow their authored people within the same place, including
        // the enforcer after his entrance; a scene key change cannot erase their source rows.
        if let Some(setting) = scene.setting.as_ref()
            && let Some(model) = setting.model.as_ref()
        {
            let fact_ids = model
                .people
                .iter()
                .flat_map(|person| &person.knowledge)
                .map(|knowledge| knowledge.fact_id.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            let inherited = pack::with_content(|content| {
                content
                    .scenes
                    .iter()
                    .filter(|source| {
                        source
                            .setting
                            .as_ref()
                            .is_some_and(|source| source.place_id == setting.place_id)
                    })
                    .flat_map(|source| &source.findings)
                    .filter(|finding| fact_ids.contains(finding.id.as_str()))
                    .cloned()
                    .collect::<Vec<_>>()
            });
            for finding in inherited {
                if !scene
                    .findings
                    .iter()
                    .any(|existing| existing.id == finding.id)
                {
                    scene.findings.push(finding);
                }
            }
        }
        let observation_operation = state
            .decisions
            .last()
            .map(|decision| {
                journey::living_scene::observation_record(decision).map(|record| {
                    record.map_or_else(
                        || journey::hex(decision.operation.as_bytes()),
                        |record| record.operation,
                    )
                })
            })
            .transpose()?;
        for record in journey::living_scene::observation_journal(state)? {
            if record.scene == scene.key
                && observation_operation.as_deref() == Some(record.operation.as_str())
            {
                scene.findings.push(pack::Finding {
                    id: record.id,
                    sentence: record.text,
                    skill: pack::FindingSkill::Perception,
                    dc: 0,
                    private: false,
                    consequence: Some(format!("observation-check:{}", record.check)),
                });
            }
        }
        // People who fled a fight are gone from the scene, and people it left down neither answer
        // nor can be spoken to: only those still standing are on stage. The narration says where
        // the others lie (`scene_fight::aftermath_narration`).
        let mut standing = Vec::with_capacity(scene.cast.len());
        for member in std::mem::take(&mut scene.cast) {
            if journey::scene_fight::presence(state, &member.id)?
                == journey::scene_fight::Presence::Standing
            {
                standing.push(member);
            }
        }
        scene.cast = standing;
        Ok(scene)
    })
}

pub(super) fn scene_scope(state: &GameState) -> Result<String, RepositoryError> {
    beat_script::with_script(|script| {
        let beat = script.current(state)?;
        Ok(shown_variant(script, beat, state)?
            .unwrap_or_else(|| format!("{:?}", beat.scene).to_lowercase()))
    })
}
/// How the heroes' fight ended, from the tactical snapshot (or, in a legacy knock-out fight, from
/// which heroes are down).
pub(super) fn fight_outcome(
    state: &GameState,
) -> Result<journey::tactical::FightOutcome, RepositoryError> {
    use journey::tactical::FightOutcome as O;
    if journey::tactical::present(state) {
        return journey::tactical::outcome(state);
    }
    let won = journey::combat_victory(state)?;
    let mut down = false;
    for character in &state.characters {
        down |= journey::value(state, character.entity, "unconscious")? != 0;
    }
    Ok(match (won, down) {
        (true, false) => O::Won,
        (true, true) => O::WonCostly,
        (false, _) => O::Lost,
    })
}
/// The scene variant a beat shows. A fight's closing steps name their victory variant; the one
/// shown is the variant of the way the fight actually ended.
fn shown_variant(
    script: &BeatScript,
    beat: &Beat,
    state: &GameState,
) -> Result<Option<String>, RepositoryError> {
    let Some(variant) = &beat.scene_variant else {
        return Ok(None);
    };
    if !(script.uses_encounter() && beat.kind == BeatKind::Ending) {
        return Ok(Some(variant.clone()));
    }
    let stem = variant
        .strip_suffix(journey::tactical::FightOutcome::Won.slug())
        .ok_or_else(|| refusal("EndingVariantOutcomeSuffixMissing"))?;
    Ok(Some(format!("{stem}{}", fight_outcome(state)?.slug())))
}
pub(super) fn native_inn(state: &GameState) -> Result<bool, RepositoryError> {
    beat_script::with_script(|script| {
        let beat = script.current(state)?;
        Ok(script.uses_encounter()
            && beat.scene == pack::SceneKey::Inn
            && beat.state.durable == "harbor-inn"
            && beat.scene_variant.is_none())
    })
}
pub(super) fn destination_scene(beat: &Beat) -> Result<rpc::GameplayScene, RepositoryError> {
    let scene = beat_script::with_script(|script| match &beat.scene_variant {
        Some(id) => {
            let mut scene = script
                .scenes
                .get(id)
                .cloned()
                .ok_or_else(|| refusal("ResolvedSceneVariantMissing"))?;
            scene.source_variant_id = Some(id.clone());
            Ok(scene)
        }
        None => {
            let mut scene = pack::scene(beat.scene);
            scene.asset = pack::source_scene_asset(beat.scene)?;
            Ok(scene)
        }
    })?;
    Ok(rpc::GameplayScene {
        destination: rpc::HarborDestination::HarborInn as i32,
        title: scene.title,
        scene_asset: scene.asset,
        source_scene_key: scene.key.source_label().to_owned(),
        source_scene_variant: scene.source_variant_id.unwrap_or_default(),
        speaker_portrait: scene.speaker_portrait,
        cast: Vec::new(),
        rules_note: String::new(),
        place: String::new(),
        location_id: String::new(),
        live_backdrop: String::new(),
        live_plate: String::new(),
        plate_layout: None,
        plate_state: rpc::PlateState::Unspecified as i32,
        ..Default::default()
    })
}
pub(super) fn pure_target(
    current: &Checkpoint,
    selected: &BeatOffer,
) -> Result<Option<Beat>, RepositoryError> {
    beat_script::with_script(|script| {
        // A fight's native handlers own everything up to its ending; only the ending's own
        // steps are pure.
        let pure = if script.uses_encounter() {
            matches!(selected.effect, BeatEffect::Observe | BeatEffect::Resolve)
        } else {
            matches!(
                selected.effect,
                BeatEffect::Continue
                    | BeatEffect::Observe
                    | BeatEffect::Resolve
                    | BeatEffect::ChooseHarborScene
            )
        };
        if !pure {
            return Ok(None);
        }
        let beat = script.current(current.state())?;
        let trigger = BeatTrigger::Offer {
            offer: selected.id.clone(),
        };
        let next = beat
            .transitions
            .iter()
            .find(|t| t.trigger == trigger)
            .and_then(|t| script.find(&t.target))
            .ok_or_else(|| refusal("OfferTransitionTargetMissing"))?;
        Ok(Some(next.clone()))
    })
}
/// Content entries the ending steps of every script (compiled or authored) use.
pub(super) const ENDING_ENTRIES: &[&str] = &[STEP, RESOLVE_ACTION, "epilogue", "the-end"];
pub(super) fn contents() -> Vec<String> {
    beat_script::with_script(|script| {
        if script.is_canonical() {
            let mut entries = vec![
                CHOICE_ACTION.into(),
                OPENING_SOCIAL_EVENT.into(),
                opening_help::EVENT.into(),
            ];
            for offer in script.beats.iter().flat_map(|beat| &beat.offers) {
                if offer.effect == BeatEffect::Observe {
                    let entry = format!("{OFFER_PREFIX}{}", offer.id);
                    if !entries.contains(&entry) {
                        entries.push(entry);
                    }
                }
            }
            return entries;
        }
        let mut entries = vec![STEP.into(), CHOICE_ACTION.into(), RESOLVE_ACTION.into()];
        for beat in &script.beats {
            entries.push(beat.state.durable.clone());
            for offer in &beat.offers {
                entries.push(format!("{OFFER_PREFIX}{}", offer.id));
            }
        }
        entries
    })
}
fn terminal<'a>(
    state: &'a GameState,
    decision: &AcceptedDecision,
) -> Result<&'a GameFact, RepositoryError> {
    let fact = decision
        .facts
        .last()
        .and_then(|id| state.facts.iter().find(|f| f.id == *id))
        .ok_or_else(|| refusal("TerminalFactMissing"))?;
    if fact.operation != decision.operation
        || fact.revision != decision.revision
        || decision.facts.get(fact.ordinal as usize) != Some(&fact.id)
    {
        return invalid("TerminalFactBinding");
    }
    let index = state
        .facts
        .iter()
        .position(|f| f.id == fact.id)
        .ok_or_else(|| refusal("TerminalFactIndexMissing"))?;
    let previous = index.checked_sub(1).and_then(|i| state.facts.get(i));
    if fact.cause != previous.map(|f| f.id)
        || (fact.ordinal > 0
            && previous.is_none_or(|f| {
                f.operation != fact.operation
                    || f.revision != fact.revision
                    || f.ordinal.checked_add(1) != Some(fact.ordinal)
                    || decision.facts.get(f.ordinal as usize) != Some(&f.id)
            }))
    {
        return invalid("TerminalCausalHistory");
    }
    Ok(fact)
}
/// Reconstruct script progress from current accepted choices. A phase marker alone never
/// establishes a path. Repeated observations record separate operation decisions while
/// same-durable-beat transitions leave the existing completed-beat ledger unchanged.
pub(super) fn validate(
    current: &Checkpoint,
    script: &BeatScript,
) -> Result<rpc::JourneyPhase, RepositoryError> {
    current
        .validate_resume(current.basis(), &model::pins()?)
        .map_err(|error| {
            refusal_detail::cause("beat-script-checkpoint-resume", format!("{error:?}"));
            RepositoryError::InvalidCandidate
        })?;
    let state = current.state();
    // A freeplay state may hold one encounter: the open fight of a hero's scene assault.
    if !state.encounters.is_empty()
        && !(script.mode == beat_script::ScriptMode::Freeplay
            && journey::scene_fight::encounters_admissible(current)?)
    {
        refusal_detail::note("scene-fight-encounter");
        return invalid("EncounterNotAdmitted");
    }
    if script.uses_encounter()
        || state.narrative.definition != model::content("room")?
        || !state.schedules.is_empty()
        || state.narrative.remaining_budget != pack::narrative_budget()
    {
        return invalid("NarrativeInitialState");
    }
    let mut beat = script
        .find(&script.entry)
        .ok_or_else(|| refusal("ScriptEntryMissing"))?;
    let mut completed = Vec::new();
    let mut accepted = Vec::new();
    let mut started = false;
    let mut native_durable = "room";
    let mut creations = 0usize;
    let mut creation_base = None;
    let mut restart_seen = false;
    let mut observation_draws = 0usize;
    // Freeplay scene fights: whether one is open in the replay so far, and what they drew.
    let mut fight_open = false;
    let mut fight_draws = 0usize;
    let mut fought = false;
    for decision in &state.decisions {
        if decision.source_policy.as_str() != journey::THREAD_POLICY {
            continue;
        }
        let terminal = terminal(state, decision)?;
        let receipt = journey::accepted(decision)?;
        if matches!(&terminal.value, FactValue::ContentEvent { definition, subjects }
            if definition.entry.as_str() == "start-new-adventure" && subjects.is_empty())
        {
            if restart_seen
                || beat.kind != BeatKind::Ending
                || !beat.offers.is_empty()
                || terminal.audience != AudienceScope::Shared
                || decision.facts.len() != 1
                || !decision.draws.is_empty()
                || !decision.effects.is_empty()
                || receipt
                    != (rpc::AcceptedAction {
                        phase: rpc::JourneyPhase::Complete as i32,
                        ..Default::default()
                    })
            {
                return invalid("RestartReceiptMismatch");
            }
            restart_seen = true;
            continue;
        }
        if script.mode == beat_script::ScriptMode::Freeplay
            && let FactValue::ContentEvent {
                definition,
                subjects,
            } = &terminal.value
            && let Some(step) = journey::scene_fight::replay_step(
                definition.entry.as_str(),
                receipt.phase,
                fight_open,
            )?
        {
            // A scene fight is an encounter inside the freeplay beat: its decisions record rules
            // outcomes, never a script choice, so they leave the beat and its ledgers as they were.
            if !subjects.is_empty()
                || terminal.audience != AudienceScope::Shared
                || choice(state, decision)?.is_some()
                || receipt.check.is_some()
                || receipt.scene.is_some()
                || receipt.character.is_some()
            {
                refusal_detail::note("scene-fight-replay");
                return invalid("FightReplayMismatch");
            }
            fought = true;
            fight_draws += decision.draws.len();
            fight_open = step != journey::scene_fight::Step::Closes;
            continue;
        }
        let Some((choice_fact, id, member)) = choice(state, decision)? else {
            // A fight changes a hero's combat resources (and a fight in progress adds weapon
            // rows to the sheet), so a fought scene reads each sheet as creation left it.
            let creation_state = creation_base.get_or_insert_with(|| state.clone());
            let character = state
                .characters
                .get(creations)
                .ok_or_else(|| refusal("CreationCharacterMissing"))?;
            creations += 1;
            if started
                || receipt.phase != rpc::JourneyPhase::Room as i32
                || receipt.character
                    != Some(journey::receipt_character_sheet(
                        creation_state,
                        character.entity,
                    )?)
                || receipt.check.is_some()
                || receipt.scene.is_some()
                || !receipt.combat.is_empty()
                || !matches!(&terminal.value,FactValue::ContentEvent{definition,subjects} if definition.entry.as_str()=="create-character" && subjects.is_empty())
                || terminal.audience != AudienceScope::Shared
            {
                return invalid("CreationReceiptMismatch");
            }
            continue;
        };
        let selected = beat
            .offers
            .iter()
            .find(|o| o.id == id)
            .ok_or_else(|| refusal("ReplayOfferMissing"))?;
        let trigger = BeatTrigger::Offer { offer: id.into() };
        let next = beat
            .transitions
            .iter()
            .find(|t| t.trigger == trigger)
            .and_then(|t| script.find(&t.target))
            .ok_or_else(|| refusal("ReplayTransitionTargetMissing"))?;
        if receipt.phase != phase_of(next) as i32
            || !receipt.combat.is_empty()
            || receipt.character.is_some()
        {
            return invalid("OfferReceiptPhaseMismatch");
        }
        if selected.effect == BeatEffect::Observe {
            let source_scene = beat
                .scene_variant
                .as_ref()
                .and_then(|id| script.scenes.get(id))
                .cloned()
                .unwrap_or_else(|| pack::scene(beat.scene));
            let source_scope = beat
                .scene_variant
                .clone()
                .unwrap_or_else(|| format!("{:?}", beat.scene).to_lowercase());
            if let Some(check) = receipt.check.as_ref() {
                // A check made in a created place replays against that place as it stood then,
                // with the item story findings the narrative engine committed there.
                let mut source_scene =
                    super::place::scene_at(state, &beat.id, source_scene, decision.revision)?;
                super::investigation::project_item_story_findings(state, &mut source_scene)?;
                validate_observation_check(
                    state,
                    decision,
                    member,
                    &source_scene,
                    &source_scope,
                    Some(check),
                )?;
                observation_draws += 1;
            }
        } else if receipt.check.is_some() {
            return invalid("UnexpectedOfferCheck");
        }
        let native = match selected.effect {
            BeatEffect::BeginStory => {
                started = true;
                Some("begin-story")
            }
            BeatEffect::AskCourier => Some("private-courier-note"),
            BeatEffect::EscortCourier | BeatEffect::ThirdApproach => Some("escort-courier"),
            BeatEffect::Continue
            | BeatEffect::Observe
            | BeatEffect::Resolve
            | BeatEffect::ChooseHarborScene => None,
            _ => return invalid("UnsupportedOfferEffect"),
        };
        let expected = native.unwrap_or(STEP);
        let private = selected.effect == BeatEffect::AskCourier;
        let scope = if private {
            AudienceScope::Members(vec![member])
        } else {
            AudienceScope::Shared
        };
        let actual_event = matches!(&terminal.value,FactValue::ContentEvent{definition,subjects} if definition.entry.as_str()==expected && subjects.is_empty());
        // A revoked/replaced private payload retains its recorded disclosure's historical
        // progress, exactly as the native courier source owner does. Rendering rechecks scope.
        let retained_private = private
            && state.intents.iter().any(|intent| {
                intent.operation == decision.operation
                    && intent.kind == EffectKind::RunAi
                    && intent.definition.entry.as_str() == super::courier_ai::DEFINITION
                    && decision.effects == [intent.id]
            });
        if private
            && matches!(&terminal.audience, AudienceScope::Members(_))
            && terminal.audience != scope
        {
            return invalid("PrivateAudienceMismatch");
        }
        if (!actual_event || terminal.audience != scope) && !retained_private {
            return invalid("TerminalEventOrAudienceMismatch");
        }
        if choice_fact.ordinal >= terminal.ordinal {
            return invalid("ChoiceAfterTerminalFact");
        }
        if native.is_some() {
            accepted.push(terminal.id);
            native_durable = &next.state.durable;
        }
        if selected.effect == BeatEffect::ChooseHarborScene {
            let expected = destination_scene(next)?;
            if receipt.scene != Some(expected) {
                return invalid("DestinationReceiptMismatch");
            }
        } else if receipt.scene.is_some() {
            return invalid("UnexpectedSceneReceipt");
        }
        if beat.state.durable != next.state.durable {
            let previous = model::content(&beat.state.durable)?;
            if !completed.contains(&previous) {
                completed.push(previous);
            }
        }
        beat = next;
    }
    // Every die is an observation check or a scene-interaction theft check; the scene replay
    // re-validates each theft draw against its receipt.
    // Time passes in a fight by whole rounds and in no other freeplay step.
    if state.draws.len()
        != observation_draws + fight_draws + journey::living_scene::interaction_draws(state)
        || fight_open != !state.encounters.is_empty()
        || (state.logical_time.ticks != 0
            && !(fought
                && state.logical_time.ticks_per_second == 1
                && state.logical_time.ticks % 6 == 0))
    {
        refusal_detail::note("scene-fight-accounting");
        return invalid("FightAccountingMismatch");
    }
    if creations != state.characters.len()
        || script.current(state)?.id != beat.id
        || state.narrative.completed_beats != completed
        || state.narrative.accepted_facts != accepted
        || state.narrative.open_threads
            != if started && script.mode != beat_script::ScriptMode::Freeplay {
                vec![model::content("sealed-packet-delivery-thread")?]
            } else {
                Vec::new()
            }
    {
        return invalid("NarrativeHistoryMismatch");
    }
    if started && !journey::party_ready(current) {
        return invalid("PartyCreationCountMismatch");
    }
    if script.mode != beat_script::ScriptMode::Freeplay {
        journey::validate_script_native_prefix(current, native_durable)?;
    }
    journey::validate_script_characters(state)?;
    if state
        .entities
        .iter()
        .any(|entity| entity.definition.entry.as_str() == "bandit")
    {
        return invalid("UnexpectedBanditEntity");
    }
    Ok(journey::scene_fight::phase_after(state, phase_of(beat)))
}

fn finding_skill_name(skill: pack::FindingSkill) -> &'static str {
    match skill {
        pack::FindingSkill::Perception => "perception",
        pack::FindingSkill::Investigation => "investigation",
        pack::FindingSkill::Insight => "insight",
        pack::FindingSkill::Survival => "survival",
        pack::FindingSkill::Medicine => "medicine",
        pack::FindingSkill::Arcana => "arcana",
        pack::FindingSkill::History => "history",
        pack::FindingSkill::Nature => "nature",
        pack::FindingSkill::Religion => "religion",
    }
}

pub(super) fn observation_finding<'a>(
    scene: &'a pack::Scene,
    check: &rpc::AbilityCheckResult,
) -> Option<&'a pack::Finding> {
    use prost::Message;
    let binding = format!("observation-check:{}", journey::hex(&check.encode_to_vec()));
    if let Some(stored) = scene
        .findings
        .iter()
        .rev()
        .find(|finding| finding.consequence.as_deref() == Some(binding.as_str()))
    {
        return Some(stored);
    }
    if !check.succeeded {
        return None;
    }
    let parts = check.source_revision.splitn(4, ':').collect::<Vec<_>>();
    let [_, _, skill, detail] = parts.as_slice() else {
        return None;
    };
    if detail.starts_with("target-") || detail.starts_with("repeat-target-") {
        if detail.starts_with("repeat-") {
            return None;
        }
        let target = super::investigation::check_target(scene, check)?;
        return super::investigation::finding_on(scene, &target, skill);
    }
    if let Some(id) = detail.strip_prefix("finding-") {
        return scene.findings.iter().find(|finding| finding.id == id);
    }
    if detail.starts_with("watch-") {
        let target = super::investigation::check_target(scene, check)?;
        return super::investigation::finding_on(scene, &target, skill);
    }
    let skill = match *skill {
        "perception" => Some(pack::FindingSkill::Perception),
        "investigation" => Some(pack::FindingSkill::Investigation),
        "insight" => Some(pack::FindingSkill::Insight),
        "survival" => Some(pack::FindingSkill::Survival),
        "medicine" => Some(pack::FindingSkill::Medicine),
        "arcana" => Some(pack::FindingSkill::Arcana),
        "history" => Some(pack::FindingSkill::History),
        "nature" => Some(pack::FindingSkill::Nature),
        "religion" => Some(pack::FindingSkill::Religion),
        "stealth" => None,
        _ => return None,
    }?;
    scene.findings.iter().find(|finding| finding.skill == skill)
}

fn validate_observation_check(
    state: &GameState,
    decision: &AcceptedDecision,
    member: MemberId,
    scene: &pack::Scene,
    scene_scope: &str,
    check: Option<&rpc::AbilityCheckResult>,
) -> Result<(), RepositoryError> {
    let mut source_scene = scene.clone();
    if let Some(snapshot) = journey::living_scene::snapshot(state, &source_scene)?
        && let Some(setting) = source_scene.setting.as_mut()
    {
        setting.model = Some(snapshot.model);
    }
    let scene = &source_scene;
    let invalid = |cause: &'static str| {
        refusal_detail::cause("search-replay-check", cause.to_owned());
        RepositoryError::InvalidCandidate
    };
    let check = check.ok_or_else(|| invalid("SearchCheckMissing"))?;
    let parts = check.source_revision.splitn(4, ':').collect::<Vec<_>>();
    if parts.len() != 4 || parts[0] != "srd521-p6" || parts[1] != scene_scope {
        return Err(invalid("SearchSourceRevisionMismatch"));
    }
    let skill = parts[2];
    let detail = parts[3];
    if !matches!(
        skill,
        "perception"
            | "investigation"
            | "insight"
            | "survival"
            | "medicine"
            | "arcana"
            | "history"
            | "nature"
            | "religion"
            | "stealth"
            | "sleight-of-hand"
    ) {
        return Err(invalid("SearchSkillUnsupported"));
    }
    let selected_id = detail.strip_prefix("finding-");
    let selected =
        selected_id.and_then(|id| scene.findings.iter().find(|finding| finding.id == id));
    if selected_id.is_some() && selected.is_none() {
        return Err(invalid("SearchFindingUnknown"));
    }
    if let Some(finding) = selected
        && finding_skill_name(finding.skill) != skill
    {
        return Err(invalid("SearchFindingSkillMismatch"));
    }
    if detail.starts_with("watch-")
        && (skill != "insight"
            || !scene
                .cast
                .iter()
                .any(|person| format!("watch-{}", person.id) == detail))
    {
        return Err(invalid("SearchWatchTargetMismatch"));
    }
    let matching = selected.or_else(|| {
        if detail.starts_with("watch-") {
            return None;
        }
        scene
            .findings
            .iter()
            .find(|finding| finding_skill_name(finding.skill) == skill)
    });
    if decision.draws.len() != 1 {
        return Err(invalid("SearchDrawCount"));
    }
    let ordinal = decision.draws[0];
    let die = state
        .draws
        .iter()
        .find(|draw| draw.operation == decision.operation && draw.ordinal == ordinal)
        .filter(|draw| draw.sides == 20 && draw.value == check.die)
        .ok_or_else(|| invalid("SearchDrawMissing"))?;
    let _ = die;
    let actor = state
        .members
        .iter()
        .find(|link| link.member == member)
        .and_then(|link| link.character)
        .ok_or_else(|| invalid("SearchActorMissing"))?;
    let (ability_modifier, proficiency_bonus) = skill_check_modifiers(state, actor, skill)
        .map_err(|_| invalid("SearchAbilityModifiers"))?;
    let total = i32::try_from(check.die).map_err(|_| invalid("SearchDieRange"))?
        + i32::from(ability_modifier)
        + i32::from(proficiency_bonus);
    // SRD 5.2.1 p. 183: "you must succeed on a DC 15 Dexterity (Stealth) check" to Hide.
    let difficulty_class = if detail.starts_with("target-") || detail.starts_with("repeat-target-")
    {
        let target = super::investigation::check_target(scene, check)
            .ok_or_else(|| invalid("SearchTargetMismatch"))?;
        super::investigation::difficulty_class(state, scene, &target, skill)?
    } else if skill == "stealth" {
        15
    } else {
        matching.map_or(10, |finding| finding.dc)
    };
    if !(1..=20).contains(&check.die)
        || check.ability_modifier != i32::from(ability_modifier)
        || check.proficiency_bonus != u32::from(proficiency_bonus)
        || check.total != total
        || check.difficulty_class != u32::from(difficulty_class)
        || check.succeeded != (total >= i32::from(difficulty_class))
    {
        return Err(invalid("SearchReceiptMismatch"));
    }
    Ok(())
}
pub(super) const CHOICE_GROUP: &str = "beat-script-offer";
const RESOLVE_ID_PREFIX: &str = "journey-resolve-";
/// The opaque id of a fight ending's step, which names its authored offer and the revision it was
/// offered at. Authored (marker) scripts use their own ids.
pub(super) fn resolve_id(current: &Checkpoint) -> Option<String> {
    beat_script::with_script(|script| {
        if !script.is_canonical() {
            return None;
        }
        let offer = script
            .current(current.state())
            .ok()?
            .offers
            .iter()
            .find(|offer| offer.effect == BeatEffect::Resolve)?;
        Some(format!(
            "{RESOLVE_ID_PREFIX}{}-{}-{}",
            offer.id,
            current.basis().revision.epoch().get(),
            current.basis().revision.sequence()
        ))
    })
}
fn resolve_offer_id(id: &str) -> Option<&str> {
    let rest = id.strip_prefix(RESOLVE_ID_PREFIX)?;
    let mut parts = rest.rsplitn(3, '-');
    let (_sequence, _epoch, offer) = (parts.next()?, parts.next()?, parts.next()?);
    Some(offer)
}
pub(super) fn submitted_id(text: &str) -> Option<&str> {
    text.strip_prefix("beat-")?
        .split_once(':')
        .map(|(id, _)| id)
}
fn id_for(current: &Checkpoint, offer: &BeatOffer) -> String {
    format!(
        "beat-{}:{}:{}",
        offer.id,
        current.basis().revision.epoch().get(),
        current.basis().revision.sequence()
    )
}
pub(super) fn select(
    current: &Checkpoint,
    action: rpc::GameplayActionKind,
    choices: &[(df_types::RevisionLabel, df_types::RevisionLabel)],
) -> Result<BeatOffer, RepositoryError> {
    if action == rpc::GameplayActionKind::DefendCourier && opening_threat_resolved(current.state())?
    {
        return invalid("OpeningThreatAlreadyResolved");
    }
    beat_script::with_script(|script| {
        let beat = script.current(current.state())?;
        // A freeplay scene fight has no fight beat to select from: its steps are the engine's own
        // tactical actions, which carry the effect of the attack, Second Wind or End Turn offer.
        if journey::scene_fight::open(current.state()) && journey::tactical::combat_action(action) {
            return journey::scene_fight::step_offer(action);
        }
        if journey::tactical::present(current.state()) && journey::tactical::combat_action(action) {
            let source = if beat.kind == BeatKind::Fight {
                beat
            } else {
                if action != rpc::GameplayActionKind::EndTurn
                    || !journey::tactical::dying(current.state())?
                {
                    return invalid("FightActionOutsideDyingTurn");
                }
                script
                    .beats
                    .iter()
                    .find(|beat| beat.kind == BeatKind::Fight)
                    .ok_or_else(|| refusal("DyingFightBeatMissing"))?
            };
            let source_kind = if matches!(
                action,
                rpc::GameplayActionKind::SecondWind | rpc::GameplayActionKind::EndTurn
            ) {
                action
            } else {
                rpc::GameplayActionKind::GreatswordAttack
            };
            return source
                .offers
                .iter()
                .find(|offer| offer.effect.action() == source_kind)
                .cloned()
                .ok_or_else(|| refusal("FightSourceOfferMissing"));
        }
        let mut submitted = choices
            .iter()
            .filter(|(group, _)| group.as_str() == CHOICE_GROUP);
        let id = submitted.next().map(|(_, value)| value.as_str());
        if submitted.next().is_some()
            || (!script.is_canonical() && id.is_none())
            || (script.is_canonical() && id.is_some())
        {
            return invalid("SubmittedChoiceCountOrMode");
        }
        beat.offers
            .iter()
            .find(|offer| offer.effect.action() == action && id.is_none_or(|id| offer.id == id))
            .cloned()
            .ok_or_else(|| refusal("SubmittedOfferMissing"))
    })
}
pub(super) fn matches_id(
    current: &Checkpoint,
    action: rpc::GameplayActionKind,
    id: &str,
) -> Result<bool, RepositoryError> {
    if action == rpc::GameplayActionKind::StartNewAdventure
        || action == rpc::GameplayActionKind::SceneAssault
    {
        return Ok(id == journey::offer_id(current, action));
    }
    beat_script::with_script(|script| {
        if script.is_canonical()
            || (journey::tactical::present(current.state())
                && journey::tactical::combat_action(action))
        {
            return Ok(id == journey::offer_id(current, action));
        }
        Ok(script
            .current(current.state())?
            .offers
            .iter()
            .any(|offer| offer.effect.action() == action && id_for(current, offer) == id))
    })
}
type ViewOffer = (rpc::GameplayActionKind, String, String);
struct OfferMemo {
    current: Checkpoint,
    pack: pack::CacheIdentity,
    members: Vec<(MemberId, Vec<ViewOffer>)>,
}
thread_local! {
    static VIEW_OFFERS: std::cell::RefCell<Option<OfferMemo>> = const { std::cell::RefCell::new(None) };
}
pub(super) fn view_offers(
    current: &Checkpoint,
    member: MemberId,
) -> Result<Vec<ViewOffer>, RepositoryError> {
    let source = pack::cache_identity();
    let remembered = VIEW_OFFERS.with(|cache| {
        cache
            .borrow()
            .as_ref()
            .filter(|memo| memo.pack == source && memo.current == *current)
            .and_then(|memo| memo.members.iter().find(|(who, _)| *who == member))
            .map(|(_, offers)| offers.clone())
    });
    if let Some(offers) = remembered {
        return Ok(offers);
    }
    let offers = derived_view_offers(current, member)?;
    VIEW_OFFERS.with(|cache| {
        let mut slot = cache.borrow_mut();
        if slot
            .as_ref()
            .is_none_or(|memo| memo.pack != source || memo.current != *current)
        {
            *slot = Some(OfferMemo {
                current: current.clone(),
                pack: source,
                members: Vec::new(),
            });
        }
        if let Some(memo) = slot.as_mut() {
            if memo.members.len() == 16 {
                memo.members.remove(0);
            }
            memo.members.push((member, offers.clone()));
        }
    });
    Ok(offers)
}
fn derived_view_offers(
    current: &Checkpoint,
    member: MemberId,
) -> Result<Vec<ViewOffer>, RepositoryError> {
    let available = journey::offered(current, member)?;
    if ended(current.state())? {
        return Ok(available
            .into_iter()
            .map(|(kind, label)| (kind, label, journey::offer_id(current, kind)))
            .collect());
    }
    beat_script::with_script(|script| {
        let dying = journey::tactical::present(current.state())
            && journey::tactical::dying(current.state())?;
        if script.is_canonical()
            || (journey::tactical::present(current.state())
                && (journey::phase(current)? == rpc::JourneyPhase::Combat || dying))
        {
            let mut result = available
                .into_iter()
                .map(|(kind, label)| (kind, label, journey::offer_id(current, kind)))
                .collect::<Vec<_>>();
            // The ending steps of a fight are the script's own, offered once nobody is dying.
            if script.is_canonical() && !dying {
                for offer in &script.current(current.state())?.offers {
                    if offer.effect == BeatEffect::Resolve {
                        result.push((
                            offer.effect.action(),
                            script.label(offer)?,
                            journey::offer_id(current, offer.effect.action()),
                        ));
                    }
                }
            }
            if opening_threat_resolved(current.state())? {
                result.retain(|(kind, _, _)| *kind != rpc::GameplayActionKind::DefendCourier);
            }
            if script.is_canonical() {
                for (kind, label, _) in &mut result {
                    if *kind == rpc::GameplayActionKind::DefendCourier
                        && label == "Stand between the courier and the bandit"
                    {
                        *label = "Draw weapons and fight the enforcer".to_owned();
                    }
                }
            }
            return Ok(result);
        }
        let mut result = Vec::new();
        // A hero who died in a freeplay fight has nothing left to choose.
        if journey::player_entity(member, current)
            .is_ok_and(|who| journey::scene_fight::hero_dead(current.state(), who))
        {
            return Ok(result);
        }
        for offer in &script.current(current.state())?.offers {
            // Authored pure steps are the script's own; no handler affordance backs them.
            let pure = matches!(
                offer.effect,
                BeatEffect::Observe | BeatEffect::Resolve | BeatEffect::Continue
            );
            if pure
                || available
                    .iter()
                    .any(|(kind, _)| *kind == offer.effect.action())
            {
                result.push((
                    offer.effect.action(),
                    if offer.effect == BeatEffect::Observe {
                        "Look around".to_owned()
                    } else {
                        script.label(offer)?
                    },
                    id_for(current, offer),
                ));
            }
        }
        // A freeplay scene may offer to start a fight with any of its people who can be fought.
        if !journey::scene_fight::offered_targets(current, member)?.is_empty() {
            result.push((
                rpc::GameplayActionKind::SceneAssault,
                journey::scene_fight::OFFER_LABEL.to_owned(),
                journey::scene_fight::offer_id(current),
            ));
        }
        Ok(result)
    })
}
pub(super) fn is_observation_offer(
    current: &Checkpoint,
    id: &str,
) -> Result<bool, RepositoryError> {
    beat_script::with_script(|script| {
        if script.is_canonical() {
            return Ok(id
                == journey::offer_id(current, rpc::GameplayActionKind::ExamineHarborSeal)
                && script
                    .current(current.state())?
                    .offers
                    .iter()
                    .any(|offer| offer.effect == BeatEffect::Observe));
        }
        let Some(submitted) = submitted_id(id) else {
            return Ok(false);
        };
        Ok(script
            .current(current.state())?
            .offers
            .iter()
            .any(|offer| offer.id == submitted && offer.effect == BeatEffect::Observe))
    })
}
/// The shared display's labels for a conversation: the current beat's own authored offers, which
/// never depend on a private note's audience. `None` for the canonical script, which keeps its
/// handler label.
pub(super) fn dialogue_labels(
    current: &Checkpoint,
) -> Result<Option<Vec<String>>, RepositoryError> {
    beat_script::with_script(|script| {
        if script.is_canonical() {
            return Ok(None);
        }
        let beat = script.current(current.state())?;
        let mut labels = Vec::new();
        for offer in &beat.offers {
            if offer.effect.action() != rpc::GameplayActionKind::CreateCharacter {
                labels.push(script.label(offer)?);
            }
        }
        Ok(Some(labels))
    })
}
/// The adventure's title for its end card: the opening scene's title, which names the errand.
pub(super) fn adventure_title(state: &GameState) -> Result<String, RepositoryError> {
    if !ended(state)? {
        return Ok(String::new());
    }
    Ok(pack::scene(pack::SceneKey::Opening).title)
}
/// The adventure's end card: an authored ending beat that offers nothing more.
pub(super) fn ended(state: &GameState) -> Result<bool, RepositoryError> {
    beat_script::with_script(|script| {
        // A script without an ending beat in its current state is not at an end card.
        let Ok(beat) = script.current(state) else {
            return Ok(false);
        };
        Ok(beat.kind == BeatKind::Ending && beat.offers.is_empty())
    })
}
/// Whether the story is in its closing steps: the resolution's epilogue or the end card.
pub(super) fn closing(state: &GameState) -> Result<bool, RepositoryError> {
    beat_script::with_script(|script| {
        let Ok(beat) = script.current(state) else {
            return Ok(false);
        };
        Ok(beat.kind == BeatKind::Ending)
    })
}
/// Whether a lost fight has settled: nobody is still making death saves, so the party's next
/// choice (a rest, or closing the chapter) is a story choice and no longer part of the fight.
pub(super) fn settled_defeat(state: &GameState) -> Result<bool, RepositoryError> {
    beat_script::with_script(|script| {
        let Ok(beat) = script.current(state) else {
            return Ok(false);
        };
        Ok(beat.state.victory == Some(false)
            && journey::tactical::present(state)
            && !journey::tactical::dying(state)?)
    })
}
pub(super) fn action_label(
    id: &str,
    kind: rpc::GameplayActionKind,
) -> Result<String, RepositoryError> {
    if kind == rpc::GameplayActionKind::StartNewAdventure {
        return Ok("Start a new adventure".to_owned());
    }
    if kind == rpc::GameplayActionKind::SceneAssault {
        return Ok(journey::scene_fight::OFFER_LABEL.to_owned());
    }
    if journey::tactical::enabled() && journey::tactical::combat_action(kind) {
        return Ok(match kind {
            rpc::GameplayActionKind::GreatswordAttack => "Attack",
            rpc::GameplayActionKind::Move => "Move",
            rpc::GameplayActionKind::Dash => "Dash",
            rpc::GameplayActionKind::Dodge => "Dodge",
            rpc::GameplayActionKind::Disengage => "Disengage",
            rpc::GameplayActionKind::SecondWind => "Second Wind",
            rpc::GameplayActionKind::ActionSurge => "Action Surge",
            rpc::GameplayActionKind::StabiliseWithHealersKit => "Stabilize with Healer's Kit",
            rpc::GameplayActionKind::StabiliseMedicine => "Stabilize with Wisdom (Medicine)",
            rpc::GameplayActionKind::EndTurn => "End turn",
            rpc::GameplayActionKind::OpportunityAttack => "Opportunity attack",
            rpc::GameplayActionKind::DeclineReaction => "Decline opportunity attack",
            rpc::GameplayActionKind::Stand => "Stand up",
            rpc::GameplayActionKind::PickUpWeapon => "Pick up weapon",
            rpc::GameplayActionKind::HelpAttack => "Help an ally attack",
            _ => return Err(refusal("UnsupportedFightActionLabel")),
        }
        .to_owned());
    }
    beat_script::with_script(|script| {
        if script.is_canonical() {
            if kind == rpc::GameplayActionKind::DefendCourier
                && pack::offer_label(kind) == "Stand between the courier and the bandit"
            {
                return Ok("Draw weapons and fight the enforcer".to_owned());
            }
            if kind == rpc::GameplayActionKind::ResolveErrand {
                let offer = resolve_offer_id(id)
                    .and_then(|offer_id| {
                        script
                            .beats
                            .iter()
                            .flat_map(|beat| &beat.offers)
                            .find(|offer| {
                                offer.id == offer_id && offer.effect == BeatEffect::Resolve
                            })
                    })
                    .ok_or_else(|| refusal("ResolutionLabelOfferMissing"))?;
                return script.label(offer);
            }
            if kind == rpc::GameplayActionKind::ExamineHarborSeal {
                return Ok("Look around".to_owned());
            }
            return Ok(pack::offer_label(kind));
        }
        let submitted = submitted_id(id).ok_or_else(|| refusal("SubmittedOfferIdMalformed"))?;
        let offer = script
            .beats
            .iter()
            .flat_map(|beat| &beat.offers)
            .find(|o| o.id == submitted && o.effect.action() == kind)
            .ok_or_else(|| refusal("SubmittedLabelOfferMissing"))?;
        script.label(offer)
    })
}

pub(super) fn advance_state(state: &mut GameState, next: &str) -> Result<(), RepositoryError> {
    for active in std::mem::take(&mut state.narrative.active_beats) {
        if !state.narrative.completed_beats.contains(&active) {
            state.narrative.completed_beats.push(active);
        }
    }
    state.narrative.active_beats = vec![model::content(next)?];
    Ok(())
}

/// A party with no living heroes cannot choose the authored steps that close its defeat.
pub(super) fn end_for_total_party_death(state: &mut GameState) -> Result<(), RepositoryError> {
    beat_script::with_script(|script| {
        let mut beat = script.current(state)?;
        for _ in 0..script.beats.len() {
            if script.terminal.contains(&beat.id) && beat.offers.is_empty() {
                return Ok(());
            }
            let mut resolutions = beat
                .offers
                .iter()
                .filter(|offer| offer.effect == BeatEffect::Resolve);
            let Some(resolve) = resolutions.next() else {
                return invalid("ResolutionOfferMissing");
            };
            if resolutions.next().is_some() {
                return invalid("ResolutionOfferAmbiguous");
            }
            let Some(transition) = beat.transitions.iter().find(|transition| {
                transition.trigger
                    == (BeatTrigger::Offer {
                        offer: resolve.id.clone(),
                    })
            }) else {
                return invalid("ResolutionTransitionMissing");
            };
            let Some(next) = script.find(&transition.target) else {
                return invalid("ResolutionTargetMissing");
            };
            let durable = next.state.durable.clone();
            advance_state(state, &durable)?;
            beat = script.current(state)?;
        }
        invalid("ResolutionChainExhausted")
    })
}

pub(super) fn first_action(
    current: &Checkpoint,
) -> Result<Option<(BeatEffect, MemberId)>, RepositoryError> {
    beat_script::with_script(|script| {
        for decision in &current.state().decisions {
            if let Some((_, id, member)) = choice(current.state(), decision)? {
                let effect = script
                    .beats
                    .iter()
                    .flat_map(|beat| &beat.offers)
                    .find(|o| o.id == id)
                    .map(|o| o.effect)
                    .ok_or_else(|| refusal("LastEffectOfferMissing"))?;
                if effect != BeatEffect::BeginStory {
                    return Ok(Some((effect, member)));
                }
            }
        }
        Ok(None)
    })
}

pub(super) fn private_action(
    current: &Checkpoint,
) -> Result<Option<(BeatEffect, MemberId)>, RepositoryError> {
    beat_script::with_script(|script| {
        for decision in &current.state().decisions {
            if let Some((_, id, member)) = choice(current.state(), decision)? {
                let offer = script
                    .beats
                    .iter()
                    .flat_map(|beat| &beat.offers)
                    .find(|offer| offer.id == id)
                    .ok_or_else(|| refusal("LastCourierOfferMissing"))?;
                if offer.effect == BeatEffect::AskCourier {
                    return Ok(Some((offer.effect, member)));
                }
            }
        }
        Ok(None)
    })
}

/// Native handlers qualify mechanical history; this independently qualifies every
/// authored choice and transition, including logical aliases of the same durable beat.
pub(super) fn validate_encounter_choices(
    current: &Checkpoint,
    script: &BeatScript,
) -> Result<(), RepositoryError> {
    if script.is_canonical() {
        return Ok(());
    }
    let mut beat = script
        .find(&script.entry)
        .ok_or_else(|| refusal("ReplayScriptEntryMissing"))?;
    for decision in &current.state().decisions {
        let Some((_, id, member)) = choice(current.state(), decision)? else {
            continue;
        };
        let offer = beat
            .offers
            .iter()
            .find(|offer| offer.id == id)
            .ok_or_else(|| refusal("ReplayBeatOfferMissing"))?;
        if offer.effect == BeatEffect::ShortRest
            && !decision.facts.iter().any(|fact_id| {
                current.state().facts.iter().any(|fact| {
                    fact.id == *fact_id && matches!(fact.value, FactValue::TimeAdvanced { .. })
                })
            })
        {
            continue;
        }
        let receipt = journey::accepted(decision)?;
        if offer.effect == BeatEffect::Observe && receipt.check.is_some() {
            let source_scene = beat
                .scene_variant
                .as_ref()
                .and_then(|id| script.scenes.get(id))
                .cloned()
                .unwrap_or_else(|| pack::scene(beat.scene));
            let source_scope = beat
                .scene_variant
                .clone()
                .unwrap_or_else(|| format!("{:?}", beat.scene).to_lowercase());
            // The check replays against the place the party stood in at that revision.
            let mut source_scene = super::place::scene_at(
                current.state(),
                &beat.id,
                source_scene,
                decision.revision,
            )?;
            super::investigation::project_item_story_findings(current.state(), &mut source_scene)?;
            validate_observation_check(
                current.state(),
                decision,
                member,
                &source_scene,
                &source_scope,
                receipt.check.as_ref(),
            )?;
        } else if receipt.check.is_some() {
            refusal_detail::cause("search-unexpected-check", "UnexpectedCheck".to_owned());
            return Err(RepositoryError::InvalidCandidate);
        }
        let trigger = if offer.effect == BeatEffect::ShortRest {
            BeatTrigger::RestComplete
        } else if beat.kind == BeatKind::Fight {
            let outcome = current.state().facts.iter().find_map(|fact| {
                if !decision.facts.contains(&fact.id) {
                    return None;
                }
                match &fact.value {
                    FactValue::ContentEvent { definition, .. }
                        if definition.entry.as_str() == "combat-victory" =>
                    {
                        Some(BeatTrigger::FightWon)
                    }
                    FactValue::ContentEvent { definition, .. }
                        if definition.entry.as_str() == "combat-defeat" =>
                    {
                        Some(BeatTrigger::FightLost)
                    }
                    _ => None,
                }
            });
            outcome.unwrap_or_else(|| BeatTrigger::Offer { offer: id.into() })
        } else {
            BeatTrigger::Offer { offer: id.into() }
        };
        beat = beat
            .transitions
            .iter()
            .find(|transition| transition.trigger == trigger)
            .and_then(|transition| script.find(&transition.target))
            .ok_or_else(|| refusal("ReplayBeatTransitionMissing"))?;
        if journey::accepted(decision)?.phase != phase_of(beat) as i32 {
            return invalid("ReplayTransitionPhaseMismatch");
        }
    }
    if beat.id != script.current(current.state())?.id {
        return invalid("ReplayActiveBeatMismatch");
    }
    Ok(())
}

/// Resolve the selected fight-start transition before the durable fight beat is installed.
pub(super) fn fight_scene(
    current: &Checkpoint,
    selected: &BeatOffer,
) -> Result<pack::Scene, RepositoryError> {
    beat_script::with_script(|script| {
        let beat = script.current(current.state())?;
        let fight = beat
            .transitions
            .iter()
            .find(|transition| {
                transition.trigger
                    == BeatTrigger::Offer {
                        offer: selected.id.clone(),
                    }
            })
            .and_then(|transition| script.find(&transition.target))
            .filter(|beat| beat.kind == BeatKind::Fight)
            .ok_or_else(|| refusal("SelectedFightTransitionMissing"))?;
        match &fight.scene_variant {
            Some(id) => script
                .scenes
                .get(id)
                .cloned()
                .ok_or_else(|| refusal("FightSceneVariantMissing")),
            None => Ok(pack::scene(fight.scene)),
        }
    })
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    #[test]
    fn unordered_beat_decision_history_records_the_exact_invariant() {
        let current = model::initial().unwrap();
        let mut state = current.state().clone();
        let decision = AcceptedDecision {
            operation: df_types::OperationId::from_bytes(&[9; 16]).unwrap(),
            revision: current.basis().revision,
            facts: vec![],
            draws: vec![],
            effects: vec![],
            source_policy: model::label("synthetic-policy").unwrap(),
            semantic_output: None,
        };
        state.decisions = vec![decision.clone(), decision];
        let _ = df_session::submission::take_refusal_diagnostic();
        assert!(last_choice(&state).is_err());
        let detail = df_session::submission::take_refusal_diagnostic().unwrap();
        assert_eq!(detail.step, "beat-script-state");
        assert_eq!(detail.cause, "DecisionRevisionOrder");
    }
}

#[cfg(test)]
pub(super) mod opening_social_tests {
    use super::*;
    use df_rules::influence::{Attitude, Willingness};
    use df_types::OperationId;

    pub(in crate::gameplay) fn opening(count: usize) -> Checkpoint {
        use crate::gameplay::beat_script_tests::{build, input};
        let mut current = journey::initial().unwrap();
        for index in 0..count {
            current = journey::stage_join(
                &current,
                &input(
                    &current,
                    journey::bootstrap_member().unwrap(),
                    index as u8 + 1,
                    "join-room",
                    vec![],
                ),
            )
            .unwrap();
        }
        let members: Vec<_> = journey::participants(&current)
            .iter()
            .map(|link| link.member)
            .collect();
        for (index, member) in members.iter().enumerate() {
            current = journey::stage_with_supplier(
                &current,
                &input(
                    &current,
                    *member,
                    index as u8 + 20,
                    "create-character",
                    build(["Brynn", "Vale", "Orla", "Pip", "Vesper", "Thorin"][index]),
                ),
                &mut |_| panic!("creation draws no dice"),
            )
            .unwrap();
        }
        journey::stage_with_supplier(
            &current,
            &input(&current, members[0], 40, "begin-story", vec![]),
            &mut |_| panic!("opening draws no dice"),
        )
        .unwrap()
    }
    fn operation(value: u8) -> OperationId {
        OperationId::from_bytes(&[value; 16]).unwrap()
    }
    fn with_harbor(run: impl FnOnce()) {
        let loaded = pack::load_from_bytes(
            include_bytes!("../../../../content/harbor-journey/pack.json"),
            include_bytes!("fixtures/harbor-journey-tuning-baseline.json"),
        )
        .unwrap();
        pack::with_test_pack(loaded, run);
    }
    #[test]
    fn beat_script_runtime_talk_down_resolves_without_fight_for_one_four_and_six_heroes() {
        with_harbor(|| {
            for count in [1, 4, 6] {
                let opening = opening(count);
                let member = journey::participants(&opening)[0].member;
                assert!(
                    super::super::place_move::travel_blocker(&opening)
                        .unwrap()
                        .is_some()
                );
                let settled = stage_opening_social(
                    &opening,
                    operation(80),
                    OpeningSocialAttempt {
                        member,
                        skill: OpeningSkill::Persuasion,
                        attitude: Attitude::Hostile,
                        willingness: Willingness::Hesitant,
                        faces: &[20, 20],
                        changed_after: None,
                    },
                )
                .unwrap();
                assert_eq!(settled.result.difficulty_class, 15);
                assert!(social_succeeded(&settled.result));
                assert!(opening_threat_resolved(settled.checkpoint.state()).unwrap());
                assert!(settled.checkpoint.state().encounters.is_empty());
                assert_eq!(
                    journey::phase(&settled.checkpoint).unwrap(),
                    rpc::JourneyPhase::Opening
                );
                let projected = scene(settled.checkpoint.state()).unwrap();
                assert!(projected.dialogue.is_empty());
                assert!(projected.narration.is_empty());
                assert_eq!(settled.line.matches("steps aside").count(), 1);
                assert!(
                    super::super::place_move::travel_blocker(&settled.checkpoint)
                        .unwrap()
                        .is_none()
                );
                assert!(
                    !super::super::place_move::travel_options(&settled.checkpoint)
                        .unwrap()
                        .is_empty()
                );
                assert!(
                    select(
                        &settled.checkpoint,
                        rpc::GameplayActionKind::DefendCourier,
                        &[]
                    )
                    .is_err()
                );
                let recovered =
                    super::super::beat_script_variety_tests::persisted(&settled.checkpoint);
                assert!(opening_threat_resolved(recovered.state()).unwrap());
                let asked = super::super::beat_script_variety_tests::act(
                    &opening,
                    member,
                    41,
                    "ask-courier",
                );
                assert_eq!(journey::phase(&asked).unwrap(), rpc::JourneyPhase::Dialogue);
                let talked = stage_opening_social(
                    &asked,
                    operation(84),
                    OpeningSocialAttempt {
                        member,
                        skill: OpeningSkill::Persuasion,
                        attitude: Attitude::Hostile,
                        willingness: Willingness::Hesitant,
                        faces: &[20, 20],
                        changed_after: None,
                    },
                )
                .unwrap()
                .checkpoint;
                assert!(opening_threat_resolved(talked.state()).unwrap());
                assert!(talked.state().encounters.is_empty());
                assert!(
                    super::super::place_move::travel_blocker(&talked)
                        .unwrap()
                        .is_none()
                );
                let mut stale = super::super::beat_script_tests::input(
                    &talked,
                    member,
                    85,
                    "defend-courier",
                    vec![],
                );
                assert!(
                    journey::stage_with_supplier(&talked, &stale, &mut |_| panic!(
                        "settled threat cannot draw initiative"
                    ))
                    .is_err()
                );
                // A stale request is rejected without erasing the native private clue source.
                let GameInput::Game(command) = &mut stale else {
                    panic!("native game input")
                };
                command.basis = asked.basis();
                assert!(
                    journey::stage_with_supplier(&talked, &stale, &mut |_| panic!(
                        "stale request cannot draw initiative"
                    ))
                    .is_err()
                );
                super::super::beat_script_variety_tests::persisted(&talked);
            }
        });
    }
    #[test]
    fn beat_script_runtime_failed_talk_keeps_talking_and_requires_a_changed_request() {
        with_harbor(|| {
            let opening = opening(1);
            let member = journey::participants(&opening)[0].member;
            let failed = stage_opening_social(
                &opening,
                operation(81),
                OpeningSocialAttempt {
                    member,
                    skill: OpeningSkill::Intimidation,
                    attitude: Attitude::Hostile,
                    willingness: Willingness::Hesitant,
                    faces: &[1, 1],
                    changed_after: None,
                },
            )
            .unwrap();
            assert!(!opening_threat_resolved(failed.checkpoint.state()).unwrap());
            assert_eq!(failed.line, "");
            assert!(failed.checkpoint.state().encounters.is_empty());
            assert!(
                stage_opening_social(
                    &failed.checkpoint,
                    operation(82),
                    OpeningSocialAttempt {
                        member,
                        skill: OpeningSkill::Persuasion,
                        attitude: Attitude::Hostile,
                        willingness: Willingness::Hesitant,
                        faces: &[20, 20],
                        changed_after: None
                    }
                )
                .is_err()
            );
            let changed = stage_opening_social(
                &failed.checkpoint,
                operation(82),
                OpeningSocialAttempt {
                    member,
                    skill: OpeningSkill::Persuasion,
                    attitude: Attitude::Hostile,
                    willingness: Willingness::Hesitant,
                    faces: &[20, 20],
                    changed_after: Some(operation(81)),
                },
            )
            .unwrap();
            assert!(opening_threat_resolved(changed.checkpoint.state()).unwrap());
        });
    }
    #[test]
    fn beat_script_runtime_social_receipt_rejects_tampered_dice_outcome_and_beat() {
        with_harbor(|| {
            let opening = opening(1);
            let member = journey::participants(&opening)[0].member;
            let settled = stage_opening_social(
                &opening,
                operation(83),
                OpeningSocialAttempt {
                    member,
                    skill: OpeningSkill::Deception,
                    attitude: Attitude::Hostile,
                    willingness: Willingness::Hesitant,
                    faces: &[20, 20],
                    changed_after: None,
                },
            )
            .unwrap()
            .checkpoint;
            for tamper in 0..4 {
                let mut state = settled.state().clone();
                match tamper {
                    0 => state.draws.last_mut().unwrap().value = 1,
                    1 => state.facts.last_mut().unwrap().cause = None,
                    2 => {
                        let text = state
                            .decisions
                            .last_mut()
                            .unwrap()
                            .semantic_output
                            .as_mut()
                            .unwrap();
                        *text = text.replace("\"succeeded\":true", "\"succeeded\":false");
                    }
                    _ => {
                        let text = state
                            .decisions
                            .last_mut()
                            .unwrap()
                            .semantic_output
                            .as_mut()
                            .unwrap();
                        *text = text
                            .replace("\"beat\":\"opening\"", "\"beat\":\"courier-answer-escort\"");
                    }
                }
                assert!(opening_threat_resolved(&state).is_err(), "tamper {tamper}");
            }
        });
    }
}
