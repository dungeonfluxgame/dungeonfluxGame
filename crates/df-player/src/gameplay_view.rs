//! Adapter from the generated, player-audience wire projection to the existing exploration UI.
use std::collections::BTreeSet;

use df_protocol::common::{
    GameplayActionKind, PlayerGameplayView, ViewMessage, view_message::Audience,
};
use df_types::{ClientBindingId, RecoveryEpoch, SessionRevision};
use df_ui::{
    ExplorationChoice, ExplorationInput, ExplorationLimits, ExplorationNpc,
    ExplorationValidationError, ExplorationView,
};

/// A generated player view that passed audience and revision checks for one caller binding.
pub struct PlayerGameplayProjection<'a> {
    player: &'a PlayerGameplayView,
    binding: ClientBindingId,
    revision: SessionRevision,
    choices: Vec<ExplorationChoice<'a>>,
    limits: ExplorationLimits,
}

/// Rejected generated views and inputs at the presentation boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GameplayViewError {
    MissingRevision,
    InvalidRevision,
    StaleRevision,
    MissingAudience,
    DisplayAudience,
    BindingMismatch,
    UnofferedInput,
    UnsupportedInput,
    UnsupportedOffer,
    InvalidOffer,
    OfferLimit,
    InvalidPresentation(ExplorationValidationError),
}

impl std::fmt::Display for GameplayViewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "gameplay view was rejected: {self:?}")
    }
}
impl std::error::Error for GameplayViewError {}

impl<'wire> PlayerGameplayProjection<'wire> {
    /// Admits only the player projection at the caller's current revision and owner limits.
    /// The binding is supplied separately and is correlation data, never authority.
    pub fn project(
        message: &'wire ViewMessage,
        binding: ClientBindingId,
        current_revision: SessionRevision,
        limits: ExplorationLimits,
    ) -> Result<Self, GameplayViewError> {
        let revision = message
            .revision
            .as_ref()
            .ok_or(GameplayViewError::MissingRevision)
            .and_then(parse_revision)?;
        if revision != current_revision {
            return Err(GameplayViewError::StaleRevision);
        }
        let player = match message.audience.as_ref() {
            Some(Audience::Player(player)) => player,
            Some(Audience::Display(_)) => return Err(GameplayViewError::DisplayAudience),
            None => return Err(GameplayViewError::MissingAudience),
        };
        validate_offers(player, limits)?;
        let choices = player
            .offers
            .iter()
            .map(|offer| ExplorationChoice {
                id: &offer.offer_id,
                label: &offer.label,
                detail: &offer.label,
                disabled_reason: None,
            })
            .collect();
        Ok(Self {
            player,
            binding,
            revision,
            choices,
            limits,
        })
    }

    /// Retains only same-binding, same-revision caller context and replaces it with current
    /// wire-derived narration, private notice, offers, and scope.
    pub fn exploration_view<'view>(
        &'view self,
        template: &ExplorationView<'view>,
    ) -> Result<ExplorationView<'view>, GameplayViewError>
    where
        'wire: 'view,
    {
        if template.binding != self.binding {
            return Err(GameplayViewError::BindingMismatch);
        }
        if template.revision != self.revision {
            return Err(GameplayViewError::StaleRevision);
        }
        let campaign = df_ui::CampaignView {
            scene: template.campaign.scene,
            chapter: template.campaign.chapter,
            title: template.campaign.title,
            description: template.campaign.description,
            location: template.campaign.location,
            scene_label: template.campaign.scene_label,
            narration: &self.player.narration,
            connection: template.campaign.connection,
            notice: if self.player.private_clue.is_empty() {
                "No private notice."
            } else {
                &self.player.private_clue
            },
            members: template.campaign.members,
            objectives: template.campaign.objectives,
        };
        let npc = self.player.journey.as_ref().and_then(|journey| {
            (!journey.speaker.is_empty()).then(|| ExplorationNpc {
                name: &journey.speaker,
                context: if self.player.private_clue.is_empty() {
                    &self.player.narration
                } else {
                    &self.player.private_clue
                },
                dialogue: if journey.dialogue.is_empty() {
                    &self.player.narration
                } else {
                    &journey.dialogue
                },
                portrait: None,
            })
        });
        let view = ExplorationView {
            binding: self.binding,
            revision: self.revision,
            campaign,
            npc,
            heading: template.heading,
            choices: &self.choices,
            draft_label: template.draft_label,
            submit_label: template.submit_label,
            draft_offer_id: None,
            pending: template.pending,
            rejection: template.rejection,
        };
        view.validate(self.limits)
            .map_err(GameplayViewError::InvalidPresentation)?;
        Ok(view)
    }

    /// Keeps only choices advertised in this projection at its exact scope and revision.
    /// Text drafts have no wire offer contract and are therefore refused.
    pub fn accept_input(
        &self,
        binding: ClientBindingId,
        input: ExplorationInput,
    ) -> Result<ExplorationInput, GameplayViewError> {
        if binding != self.binding {
            return Err(GameplayViewError::BindingMismatch);
        }
        let (id, revision) = match &input {
            ExplorationInput::Choice { id, revision } => (id, revision),
            ExplorationInput::Draft { .. } => return Err(GameplayViewError::UnsupportedInput),
        };
        if *revision != self.revision {
            return Err(GameplayViewError::StaleRevision);
        }
        if !self.choices.iter().any(|choice| choice.id == id) {
            return Err(GameplayViewError::UnofferedInput);
        }
        Ok(input)
    }
}

fn validate_offers(
    player: &PlayerGameplayView,
    limits: ExplorationLimits,
) -> Result<(), GameplayViewError> {
    if player.offers.len() > limits.max_choices {
        return Err(GameplayViewError::OfferLimit);
    }
    if player.action_available != !player.offers.is_empty() {
        return Err(GameplayViewError::InvalidOffer);
    }
    let mut offer_ids = BTreeSet::new();
    for offer in &player.offers {
        let action = GameplayActionKind::try_from(offer.action_kind)
            .map_err(|_| GameplayViewError::UnsupportedOffer)?;
        if action == GameplayActionKind::Unspecified
            || !offer.destinations.is_empty()
            || !offer.input_groups.is_empty()
        {
            return Err(GameplayViewError::UnsupportedOffer);
        }
        if offer.offer_id.trim().is_empty()
            || offer.offer_id.len() > limits.max_identifier_bytes
            || offer.label.trim().is_empty()
            || offer.label.len() > limits.max_text_bytes
            || !offer_ids.insert(offer.offer_id.as_str())
        {
            return Err(GameplayViewError::InvalidOffer);
        }
    }
    Ok(())
}

fn parse_revision(
    revision: &df_protocol::common::SessionRevision,
) -> Result<SessionRevision, GameplayViewError> {
    let epoch = revision
        .epoch
        .as_ref()
        .and_then(|epoch| epoch.value)
        .ok_or(GameplayViewError::InvalidRevision)
        .and_then(|value| {
            RecoveryEpoch::new(value).map_err(|_| GameplayViewError::InvalidRevision)
        })?;
    let sequence = revision
        .sequence
        .ok_or(GameplayViewError::InvalidRevision)?;
    Ok(SessionRevision::new(epoch, sequence))
}
