//! Owned, public-only presentation values projected from the generated shared-display view.
//!
//! This adapter consumes only `DisplayGameplayView`. It is a final client-side
//! allowlist, not authorization: the caller must receive a view already filtered
//! for the shared display by the server. The source view is never retained.

use std::fmt;

use df_protocol::common::{
    CombatOutcome, CombatView, DisplayGameplayView, GameplayScene, HarborDestination, JourneyPhase,
    JourneyView, PartyMemberView,
};

/// A known public scene destination. Unspecified is represented separately by `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplaySceneKind {
    LoadingPier,
    LanternWharf,
    HarborInn,
}

/// Public scene title and any known canonical destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayScene {
    pub kind: Option<DisplaySceneKind>,
    pub title: String,
}

/// A supported journey phase from the generated public contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayPhase {
    Room,
    CharacterCreation,
    Opening,
    Dialogue,
    Combat,
    Complete,
}

/// Public party facts. Participant identifiers are deliberately absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayPartyMember {
    pub name: String,
    pub character_ready: bool,
    pub hit_points: u32,
    pub maximum_hit_points: u32,
    pub active_turn: bool,
    pub unconscious: bool,
}

/// Plain supplied caption text. No timing, audio cue, or inferred speaker data exists here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayCaption {
    pub speaker: Option<String>,
    pub text: String,
}

/// Explicit committed combat facts copied from the public wire, without identity or revision IDs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayCombatOutcome {
    pub actor_name: String,
    pub target_name: String,
    pub attack_die: u32,
    pub attack_modifier: i32,
    pub attack_total: i32,
    pub target_armor_class: u32,
    pub hit: bool,
    pub critical: bool,
    pub damage_dice: Vec<u32>,
    pub damage: u32,
    pub target_hit_points: u32,
    pub knocked_out: bool,
    pub grazed: bool,
}

/// Public current combat state. `participants` preserves wire order and is not initiative order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicCombatProjection {
    pub participants: Vec<DisplayPartyMember>,
    pub active_actor: Option<String>,
    pub round: u32,
    pub outcomes: Vec<DisplayCombatOutcome>,
    pub finished: bool,
}

/// Complete owned presentation input for the shared display.
///
/// It contains only the selected public scene, phase, party, combat, narration,
/// and plain dialogue-caption values. It has no room/join data, player-specific
/// views, participant IDs, private creation/sheet data, source revisions, or
/// audio/caption timing. Fields copied from the wire are not independently
/// authorized by this type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicDisplayProjection {
    pub scene: DisplayScene,
    pub phase: DisplayPhase,
    pub narration: String,
    pub party: Vec<DisplayPartyMember>,
    pub combat: Option<PublicCombatProjection>,
    pub dialogue_caption: Option<DisplayCaption>,
}

/// Refuses incomplete or unsupported public views without echoing their payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayProjectionError {
    MissingScene,
    MissingJourney,
    UnsupportedScene,
    UnsupportedPhase,
    InvalidSceneTitle,
    InvalidPartyName,
    InvalidActiveActor,
    InvalidSpeaker,
    InvalidOutcomeName,
    InvalidHitPointRange,
}

impl fmt::Display for DisplayProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingScene => "display view is missing its required scene",
            Self::MissingJourney => "display view is missing its required journey",
            Self::UnsupportedScene => "display view contains an unsupported scene",
            Self::UnsupportedPhase => "display view contains an unsupported journey phase",
            Self::InvalidSceneTitle => "display scene title is malformed",
            Self::InvalidPartyName => "display party name is malformed",
            Self::InvalidActiveActor => "display active actor is malformed",
            Self::InvalidSpeaker => "display caption speaker is malformed",
            Self::InvalidOutcomeName => "display combat outcome name is malformed",
            Self::InvalidHitPointRange => "display party hit point range is invalid",
        })
    }
}

impl std::error::Error for DisplayProjectionError {}

/// Projects a generated shared-display snapshot into an owned explicit allowlist.
///
/// Unknown discriminants and missing required messages return a typed error. The
/// source view is never stored or cloned into the projection; the caller remains
/// responsible for its own source-message lifetime.
pub fn project_display_view(
    view: &DisplayGameplayView,
) -> Result<PublicDisplayProjection, DisplayProjectionError> {
    let scene = view
        .scene
        .as_ref()
        .ok_or(DisplayProjectionError::MissingScene)?;
    let journey = view
        .journey
        .as_ref()
        .ok_or(DisplayProjectionError::MissingJourney)?;
    Ok(PublicDisplayProjection {
        scene: project_scene(scene)?,
        phase: project_phase(journey.phase)?,
        narration: view.narration.clone(),
        party: journey
            .party
            .iter()
            .map(project_party_member)
            .collect::<Result<_, _>>()?,
        combat: journey.combat.as_ref().map(project_combat).transpose()?,
        dialogue_caption: project_caption(journey)?,
    })
}

fn project_scene(scene: &GameplayScene) -> Result<DisplayScene, DisplayProjectionError> {
    let kind = match HarborDestination::try_from(scene.destination)
        .map_err(|_| DisplayProjectionError::UnsupportedScene)?
    {
        HarborDestination::Unspecified => None,
        HarborDestination::LoadingPier => Some(DisplaySceneKind::LoadingPier),
        HarborDestination::LanternWharf => Some(DisplaySceneKind::LanternWharf),
        HarborDestination::HarborInn => Some(DisplaySceneKind::HarborInn),
    };
    if !valid_label(&scene.title) {
        return Err(DisplayProjectionError::InvalidSceneTitle);
    }
    Ok(DisplayScene {
        kind,
        title: scene.title.clone(),
    })
}

fn project_phase(phase: i32) -> Result<DisplayPhase, DisplayProjectionError> {
    match JourneyPhase::try_from(phase).map_err(|_| DisplayProjectionError::UnsupportedPhase)? {
        JourneyPhase::Unspecified => Err(DisplayProjectionError::UnsupportedPhase),
        JourneyPhase::Room => Ok(DisplayPhase::Room),
        JourneyPhase::CharacterCreation => Ok(DisplayPhase::CharacterCreation),
        JourneyPhase::Opening => Ok(DisplayPhase::Opening),
        JourneyPhase::Dialogue => Ok(DisplayPhase::Dialogue),
        JourneyPhase::Combat => Ok(DisplayPhase::Combat),
        JourneyPhase::Complete => Ok(DisplayPhase::Complete),
    }
}

fn project_party_member(
    member: &PartyMemberView,
) -> Result<DisplayPartyMember, DisplayProjectionError> {
    if !valid_label(&member.name) {
        return Err(DisplayProjectionError::InvalidPartyName);
    }
    if member.hit_points > member.maximum_hit_points
        || (member.character_ready && member.maximum_hit_points == 0)
    {
        return Err(DisplayProjectionError::InvalidHitPointRange);
    }
    Ok(DisplayPartyMember {
        name: member.name.clone(),
        character_ready: member.character_ready,
        hit_points: member.hit_points,
        maximum_hit_points: member.maximum_hit_points,
        active_turn: member.active_turn,
        unconscious: member.unconscious,
    })
}

fn project_caption(
    journey: &JourneyView,
) -> Result<Option<DisplayCaption>, DisplayProjectionError> {
    if journey.dialogue.is_empty() {
        return Ok(None);
    }
    let speaker = if journey.speaker.is_empty() {
        None
    } else if valid_label(&journey.speaker) {
        Some(journey.speaker.clone())
    } else {
        return Err(DisplayProjectionError::InvalidSpeaker);
    };
    Ok(Some(DisplayCaption {
        speaker,
        text: journey.dialogue.clone(),
    }))
}

fn project_combat(combat: &CombatView) -> Result<PublicCombatProjection, DisplayProjectionError> {
    let active_actor = if combat.active_actor.is_empty() {
        if combat.finished {
            None
        } else {
            return Err(DisplayProjectionError::InvalidActiveActor);
        }
    } else if valid_label(&combat.active_actor) {
        Some(combat.active_actor.clone())
    } else {
        return Err(DisplayProjectionError::InvalidActiveActor);
    };
    Ok(PublicCombatProjection {
        participants: combat
            .participants
            .iter()
            .map(project_party_member)
            .collect::<Result<_, _>>()?,
        active_actor,
        round: combat.round,
        outcomes: combat
            .outcomes
            .iter()
            .map(project_outcome)
            .collect::<Result<_, _>>()?,
        finished: combat.finished,
    })
}

fn project_outcome(
    outcome: &CombatOutcome,
) -> Result<DisplayCombatOutcome, DisplayProjectionError> {
    if !valid_label(&outcome.actor_name) || !valid_label(&outcome.target_name) {
        return Err(DisplayProjectionError::InvalidOutcomeName);
    }
    Ok(DisplayCombatOutcome {
        actor_name: outcome.actor_name.clone(),
        target_name: outcome.target_name.clone(),
        attack_die: outcome.attack_die,
        attack_modifier: outcome.attack_modifier,
        attack_total: outcome.attack_total,
        target_armor_class: outcome.target_armor_class,
        hit: outcome.hit,
        critical: outcome.critical,
        damage_dice: outcome.damage_dice.clone(),
        damage: outcome.damage,
        target_hit_points: outcome.target_hit_points,
        knocked_out: outcome.knocked_out,
        grazed: outcome.grazed,
    })
}

fn valid_label(label: &str) -> bool {
    !label.trim().is_empty() && !label.chars().any(char::is_control)
}
