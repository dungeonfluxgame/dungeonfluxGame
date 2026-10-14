//! Pack-owned scene sequencing. Scripts select existing handlers, never mechanics.
use super::{journey, model, pack, refusal_detail};
use df_model::checkpoint::{Checkpoint, GameState};
use df_protocol::common as rpc;
use df_session::submission::RepositoryError;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

/// The label of an ending step in a script that authors none (the compiled demo pack).
const DEFAULT_RESOLVE_LABEL: &str = "Settle the errand";
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct BeatScript {
    pub(super) entry: String,
    pub(super) terminal: Vec<String>,
    pub(super) beats: Vec<Beat>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) labels: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) scenes: BTreeMap<String, pack::Scene>,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Beat {
    pub(super) id: String,
    pub(super) kind: BeatKind,
    pub(super) scene: pack::SceneKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) scene_variant: Option<String>,
    /// Existing checkpoints retain their durable markers. Outcome variants are derived,
    /// so installing the canonical script does not rewrite any stored state.
    pub(super) state: BeatState,
    pub(super) offers: Vec<BeatOffer>,
    pub(super) transitions: Vec<BeatTransition>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum BeatKind {
    Gather,
    Narrate,
    Converse,
    Fight,
    Rest,
    Travel,
    Conclude,
    Investigate,
    /// A resolution, epilogue or end card: authored pure steps that follow a terminal beat.
    Ending,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct BeatState {
    pub(super) durable: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) victory: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) rested: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) chosen_offer: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct BeatOffer {
    pub(super) id: String,
    pub(super) label_slot: String,
    pub(super) effect: BeatEffect,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum BeatEffect {
    CreateCharacter,
    BeginStory,
    AskCourier,
    EscortCourier,
    DefendCourier,
    GreatswordAttack,
    SecondWind,
    EndTurn,
    ShortRest,
    ChooseHarborScene,
    ThirdApproach,
    /// Public observation only. A source-qualified ability-check handler will attach
    /// here in roadmap item 3; this effect contains no check, DC or roll fields.
    Observe,
    /// Hands over the errand's object or person. Offered once, on a resolution beat.
    Resolve,
    Continue,
}
impl BeatEffect {
    pub(super) fn action(self) -> rpc::GameplayActionKind {
        use rpc::GameplayActionKind as A;
        match self {
            Self::CreateCharacter => A::CreateCharacter,
            Self::BeginStory => A::BeginStory,
            Self::AskCourier => A::AskCourier,
            Self::EscortCourier => A::EscortCourier,
            Self::DefendCourier => A::DefendCourier,
            Self::GreatswordAttack => A::GreatswordAttack,
            Self::SecondWind => A::SecondWind,
            Self::EndTurn => A::EndTurn,
            Self::ShortRest => A::ShortRest,
            Self::ChooseHarborScene => A::ChooseHarborScene,
            Self::ThirdApproach | Self::Observe => A::ExamineHarborSeal,
            Self::Resolve => A::ResolveErrand,
            Self::Continue => A::BeginStory,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct BeatTransition {
    pub(super) trigger: BeatTrigger,
    pub(super) target: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(tag = "on", rename_all = "kebab-case", deny_unknown_fields)]
pub(super) enum BeatTrigger {
    Offer { offer: String },
    FightWon,
    FightLost,
    RestComplete,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ScriptError {
    pub(super) beat: String,
    pub(super) reason: ScriptErrorReason,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ScriptErrorReason {
    Bounds,
    UnknownHandler,
    UnknownScene,
    DuplicateBeat,
    AmbiguousState,
    HandlerWiring,
    TerminalWiring,
    MissingEntry,
    UnknownTerminal,
    DuplicateOffer,
    DuplicateEffect,
    MissingLabel,
    UnsupportedEffect,
    UnknownOffer,
    MissingTarget,
    DuplicateTransition,
    NoExit,
    Unreachable,
    AutomaticCycle,
    FightWiring,
    RestWiring,
}
impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "beat script at beat {}: {:?}", self.beat, self.reason)
    }
}
impl std::error::Error for ScriptError {}
fn refuse(beat: &str, reason: ScriptErrorReason) -> ScriptError {
    ScriptError {
        beat: beat.into(),
        reason,
    }
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
impl BeatScript {
    pub(super) fn uses_encounter(&self) -> bool {
        self.beats.iter().any(|b| b.kind == BeatKind::Fight)
    }
    /// The compiled sequencing graph. Labels and scene variants are presentation: a generated
    /// chapter dresses the graph in its own text without changing how it sequences.
    pub(super) fn is_canonical(&self) -> bool {
        let canonical = canonical();
        self.entry == canonical.entry
            && self.terminal == canonical.terminal
            && self.beats.len() == canonical.beats.len()
            && self.beats.iter().zip(&canonical.beats).all(|(a, b)| {
                a.id == b.id
                    && a.kind == b.kind
                    && a.scene == b.scene
                    && a.state == b.state
                    && a.transitions == b.transitions
                    && a.offers == b.offers
            })
    }
    pub(super) fn label(&self, offer: &BeatOffer) -> Result<String, RepositoryError> {
        if let Some(label) = self.labels.get(&offer.label_slot) {
            return Ok(label.clone());
        }
        if offer.effect == BeatEffect::Resolve {
            return Ok(DEFAULT_RESOLVE_LABEL.into());
        }
        let kind = super::wire::action_kind(&offer.label_slot)
            .filter(|kind| *kind != rpc::GameplayActionKind::ExamineHarborSeal)
            .ok_or(RepositoryError::InvalidCandidate)?;
        Ok(pack::offer_label(kind))
    }
    pub(super) fn find(&self, id: &str) -> Option<&Beat> {
        self.beats.iter().find(|beat| beat.id == id)
    }
    pub(super) fn validate(&self, content: &pack::PackFile) -> Result<(), ScriptError> {
        use ScriptErrorReason as E;
        if self.beats.is_empty()
            || self.beats.len() > 64
            || self.terminal.is_empty()
            || self.terminal.len() > 64
        {
            return Err(refuse(&self.entry, E::Bounds));
        }
        if self.find(&self.entry).is_none() {
            return Err(refuse(&self.entry, E::MissingEntry));
        }
        for id in &self.terminal {
            if self.find(id).is_none() {
                return Err(refuse(id, E::UnknownTerminal));
            }
        }
        if self.find(&self.entry).is_none_or(|beat| {
            beat.kind != BeatKind::Gather
                || beat.state.durable != "room"
                || beat.state.chosen_offer.is_some()
        }) {
            return Err(refuse(&self.entry, E::HandlerWiring));
        }
        for id in &self.terminal {
            if self
                .find(id)
                .is_none_or(|beat| !matches!(beat.kind, BeatKind::Conclude | BeatKind::Ending))
            {
                return Err(refuse(id, E::TerminalWiring));
            }
        }
        for (index, beat) in self.beats.iter().enumerate() {
            if self.beats.iter().skip(index + 1).any(|other| {
                beat.state.durable == other.state.durable
                    && (beat.state.victory.is_none()
                        || other.state.victory.is_none()
                        || beat.state.victory == other.state.victory)
                    && (beat.state.rested.is_none()
                        || other.state.rested.is_none()
                        || beat.state.rested == other.state.rested)
                    && (beat.state.chosen_offer.is_none()
                        || other.state.chosen_offer.is_none()
                        || beat.state.chosen_offer == other.state.chosen_offer)
            }) {
                return Err(refuse(&beat.id, E::AmbiguousState));
            }
        }
        let uses_encounter = self.uses_encounter();
        if self.labels.len() > 64 || self.scenes.len() > 64 {
            return Err(refuse(&self.entry, E::Bounds));
        }
        for (slot, text) in &self.labels {
            if !identifier(slot)
                || text.is_empty()
                || text.chars().count() > 96
                || text.chars().any(char::is_control)
            {
                return Err(refuse(slot, E::Bounds));
            }
        }
        for (id, scene) in &self.scenes {
            if !identifier(id) || !content.scenes.iter().any(|s| s.key == scene.key) {
                return Err(refuse(id, E::UnknownScene));
            }
            pack::validate_script_scene(content, scene).map_err(|_| refuse(id, E::UnknownScene))?;
        }
        let mut offer_definitions = BTreeMap::new();
        for beat in &self.beats {
            for offer in &beat.offers {
                if offer_definitions
                    .insert(&offer.id, offer)
                    .is_some_and(|previous| previous != offer)
                {
                    return Err(refuse(&beat.id, E::DuplicateOffer));
                }
            }
        }
        let mut ids = BTreeSet::new();
        for beat in &self.beats {
            // The native encounter destination receipt/proof still names its Inn.
            // Generic no-encounter destinations own their authored scene receipt.
            if uses_encounter
                && beat.state.durable == "harbor-inn"
                && (beat.scene != pack::SceneKey::Inn || beat.scene_variant.is_some())
            {
                return Err(refuse(&beat.id, E::HandlerWiring));
            }
            if !uses_encounter && beat.kind == BeatKind::Rest {
                return Err(refuse(&beat.id, E::RestWiring));
            }
            // A fight chapter's concluding scenes lead on to its ending steps.
            if beat.kind == BeatKind::Conclude
                && !self.terminal.contains(&beat.id)
                && !(uses_encounter
                    && beat
                        .offers
                        .iter()
                        .any(|offer| offer.effect == BeatEffect::Resolve))
            {
                return Err(refuse(&beat.id, E::TerminalWiring));
            }
            if !identifier(&beat.id)
                || !identifier(&beat.state.durable)
                || beat.offers.iter().any(|o| o.id.len() > 40)
                || beat.offers.len() > 16
                || beat.transitions.len() > 32
            {
                return Err(refuse(&beat.id, E::Bounds));
            }
            if !ids.insert(&beat.id) {
                return Err(refuse(&beat.id, E::DuplicateBeat));
            }
            if !content.scenes.iter().any(|scene| scene.key == beat.scene) {
                return Err(refuse(&beat.id, E::UnknownScene));
            }
            if beat.transitions.is_empty() && !self.terminal.contains(&beat.id) {
                return Err(refuse(&beat.id, E::NoExit));
            }
            if (beat.state.victory.is_some() || beat.state.rested == Some(true))
                && !matches!(beat.state.durable.as_str(), "complete" | "harbor-inn")
            {
                return Err(refuse(&beat.id, E::HandlerWiring));
            }
            if beat
                .transitions
                .iter()
                .any(|t| t.trigger == BeatTrigger::RestComplete)
                && !beat
                    .offers
                    .iter()
                    .any(|o| o.effect == BeatEffect::ShortRest)
            {
                return Err(refuse(&beat.id, E::RestWiring));
            }
            if let Some(variant) = &beat.scene_variant
                && self
                    .scenes
                    .get(variant)
                    .is_none_or(|scene| scene.key != beat.scene)
            {
                return Err(refuse(&beat.id, E::UnknownScene));
            }
            // A fight's closing scenes tell how it ended: the beat names the victory variant and
            // the other outcomes' variants sit beside it, chosen when the scene is shown.
            if uses_encounter
                && beat.kind == BeatKind::Ending
                && let Some(variant) = &beat.scene_variant
            {
                let stem = variant
                    .strip_suffix(journey::tactical::FightOutcome::Won.slug())
                    .ok_or_else(|| refuse(&beat.id, E::UnknownScene))?;
                for outcome in journey::tactical::FightOutcome::ALL {
                    if self
                        .scenes
                        .get(&format!("{stem}{}", outcome.slug()))
                        .is_none_or(|scene| scene.key != beat.scene)
                    {
                        return Err(refuse(&beat.id, E::UnknownScene));
                    }
                }
            }
            if !uses_encounter && (beat.state.victory.is_some() || beat.state.rested.is_some()) {
                return Err(refuse(&beat.id, E::HandlerWiring));
            }
            let handler_state = match beat.kind {
                BeatKind::Gather => beat.state.durable == "room",
                BeatKind::Converse => beat.state.durable == "opening",
                BeatKind::Narrate if !uses_encounter => true,
                BeatKind::Investigate if !uses_encounter => true,
                BeatKind::Travel if !uses_encounter => true,
                BeatKind::Conclude if !uses_encounter => true,
                BeatKind::Ending => true,
                BeatKind::Narrate => matches!(
                    beat.state.durable.as_str(),
                    "courier-answer-seal" | "courier-answer-escort"
                ),
                BeatKind::Fight => beat.state.durable == "combat",
                BeatKind::Rest | BeatKind::Travel => beat.state.durable == "complete",
                BeatKind::Conclude => {
                    matches!(beat.state.durable.as_str(), "complete" | "harbor-inn")
                }
                BeatKind::Investigate => false,
            };
            if !handler_state {
                return Err(refuse(&beat.id, E::HandlerWiring));
            }
            let required: &[BeatEffect] = match beat.kind {
                BeatKind::Narrate
                | BeatKind::Investigate
                | BeatKind::Travel
                | BeatKind::Conclude
                    if !uses_encounter =>
                {
                    &[]
                }
                BeatKind::Gather => &[BeatEffect::CreateCharacter, BeatEffect::BeginStory],
                BeatKind::Narrate => &[BeatEffect::DefendCourier],
                BeatKind::Fight => &[
                    BeatEffect::GreatswordAttack,
                    BeatEffect::SecondWind,
                    BeatEffect::EndTurn,
                ],
                BeatKind::Rest | BeatKind::Travel if beat.state.victory == Some(true) => {
                    &[BeatEffect::ShortRest, BeatEffect::ChooseHarborScene]
                }
                BeatKind::Rest | BeatKind::Travel => &[BeatEffect::ShortRest],
                BeatKind::Converse => &[],
                BeatKind::Conclude => &[],
                BeatKind::Investigate => &[],
                BeatKind::Ending => &[],
            };
            if required
                .iter()
                .any(|effect| !beat.offers.iter().any(|o| &o.effect == effect))
                || (beat.kind == BeatKind::Converse && beat.offers.is_empty())
            {
                return Err(refuse(
                    &beat.id,
                    if beat.kind == BeatKind::Fight {
                        E::FightWiring
                    } else {
                        E::HandlerWiring
                    },
                ));
            }
            let mut offers = BTreeSet::new();
            let mut effects = Vec::new();
            let mut actions = Vec::new();
            for offer in &beat.offers {
                if uses_encounter
                    && (effects.contains(&offer.effect) || actions.contains(&offer.effect.action()))
                {
                    return Err(refuse(&beat.id, E::DuplicateEffect));
                }
                effects.push(offer.effect);
                actions.push(offer.effect.action());
                if !identifier(&offer.id) || !offers.insert(&offer.id) {
                    return Err(refuse(&beat.id, E::DuplicateOffer));
                }
                if !self.labels.contains_key(&offer.label_slot)
                    && offer.effect != BeatEffect::Resolve
                    && super::wire::action_kind(&offer.label_slot)
                        .is_none_or(|kind| kind == rpc::GameplayActionKind::ExamineHarborSeal)
                {
                    return Err(refuse(&beat.id, E::MissingLabel));
                }
                let allowed = match beat.kind {
                    BeatKind::Gather => matches!(
                        offer.effect,
                        BeatEffect::CreateCharacter | BeatEffect::BeginStory
                    ),
                    BeatKind::Narrate if !uses_encounter => offer.effect == BeatEffect::Continue,
                    BeatKind::Investigate if !uses_encounter => matches!(
                        offer.effect,
                        BeatEffect::Observe
                            | BeatEffect::AskCourier
                            | BeatEffect::EscortCourier
                            | BeatEffect::Continue
                    ),
                    BeatKind::Travel if !uses_encounter => {
                        offer.effect == BeatEffect::ChooseHarborScene
                    }
                    BeatKind::Conclude if !uses_encounter => false,
                    BeatKind::Ending => {
                        matches!(offer.effect, BeatEffect::Resolve | BeatEffect::Continue)
                    }
                    BeatKind::Narrate => offer.effect == BeatEffect::DefendCourier,
                    BeatKind::Converse => matches!(
                        offer.effect,
                        BeatEffect::AskCourier
                            | BeatEffect::EscortCourier
                            | BeatEffect::ThirdApproach
                    ),
                    BeatKind::Fight => matches!(
                        offer.effect,
                        BeatEffect::GreatswordAttack | BeatEffect::SecondWind | BeatEffect::EndTurn
                    ),
                    BeatKind::Rest | BeatKind::Travel => {
                        matches!(
                            offer.effect,
                            BeatEffect::ShortRest | BeatEffect::ChooseHarborScene
                        ) || (offer.effect == BeatEffect::Resolve
                            && beat.state.victory == Some(false))
                    }
                    BeatKind::Conclude => {
                        matches!(offer.effect, BeatEffect::ShortRest | BeatEffect::Resolve)
                    }
                    BeatKind::Investigate => false,
                };
                if !allowed {
                    return Err(refuse(&beat.id, E::UnsupportedEffect));
                }
                if !beat
                    .transitions
                    .iter()
                    .any(|transition| match &transition.trigger {
                        BeatTrigger::Offer { offer: id } => id == &offer.id,
                        BeatTrigger::RestComplete => offer.effect == BeatEffect::ShortRest,
                        _ => false,
                    })
                {
                    return Err(refuse(&beat.id, E::MissingTarget));
                }
            }
            for transition in &beat.transitions {
                let Some(target) = self.find(&transition.target) else {
                    continue;
                };
                let permitted = match &transition.trigger {
                    BeatTrigger::Offer { offer } => {
                        let Some(effect) = beat
                            .offers
                            .iter()
                            .find(|o| &o.id == offer)
                            .map(|o| o.effect)
                        else {
                            continue;
                        };
                        if target
                            .state
                            .chosen_offer
                            .as_ref()
                            .is_some_and(|chosen| chosen != offer)
                            || (effect == BeatEffect::CreateCharacter
                                && target.state.chosen_offer.is_some())
                        {
                            return Err(refuse(&beat.id, E::HandlerWiring));
                        }
                        if matches!(
                            effect,
                            BeatEffect::AskCourier
                                | BeatEffect::EscortCourier
                                | BeatEffect::ThirdApproach
                        ) && beat.state.durable != "opening"
                        {
                            return Err(refuse(&beat.id, E::HandlerWiring));
                        }
                        if !uses_encounter
                            && effect == BeatEffect::ChooseHarborScene
                            && !matches!(target.kind, BeatKind::Conclude | BeatKind::Ending)
                        {
                            return Err(refuse(&beat.id, E::HandlerWiring));
                        }
                        let durable = match effect {
                            BeatEffect::CreateCharacter => "room",
                            BeatEffect::BeginStory => "opening",
                            BeatEffect::AskCourier => "courier-answer-seal",
                            BeatEffect::EscortCourier | BeatEffect::ThirdApproach => {
                                "courier-answer-escort"
                            }
                            BeatEffect::DefendCourier => "combat",
                            BeatEffect::GreatswordAttack
                            | BeatEffect::SecondWind
                            | BeatEffect::EndTurn => "combat",
                            BeatEffect::ChooseHarborScene if !uses_encounter => {
                                target.state.durable.as_str()
                            }
                            BeatEffect::Observe | BeatEffect::Continue | BeatEffect::Resolve
                                if !uses_encounter =>
                            {
                                target.state.durable.as_str()
                            }
                            BeatEffect::ChooseHarborScene => "harbor-inn",
                            BeatEffect::Resolve if target.kind == BeatKind::Ending => {
                                target.state.durable.as_str()
                            }
                            BeatEffect::Observe | BeatEffect::Continue | BeatEffect::Resolve => {
                                return Err(refuse(&beat.id, E::HandlerWiring));
                            }
                            BeatEffect::ShortRest => return Err(refuse(&beat.id, E::RestWiring)),
                        };
                        target.state.durable == durable
                            && (!uses_encounter
                                || effect != BeatEffect::ChooseHarborScene
                                || (beat.state.victory == Some(true)
                                    && target.state.victory.is_none_or(|v| v)
                                    && target
                                        .state
                                        .rested
                                        .is_none_or(|rested| beat.state.rested == Some(rested))))
                    }
                    BeatTrigger::FightWon => {
                        target.state.chosen_offer.is_none()
                            && beat.kind == BeatKind::Fight
                            && target.state.durable == "complete"
                            && target.state.victory == Some(true)
                            && target.state.rested == Some(false)
                    }
                    BeatTrigger::FightLost => {
                        target.state.chosen_offer.is_none()
                            && beat.kind == BeatKind::Fight
                            && target.state.durable == "complete"
                            && target.state.victory == Some(false)
                            && target.state.rested == Some(false)
                    }
                    BeatTrigger::RestComplete => {
                        if target.state.chosen_offer.as_ref().is_some_and(|chosen| {
                            beat.offers.iter().any(|offer| {
                                offer.effect == BeatEffect::ShortRest && &offer.id != chosen
                            })
                        }) {
                            return Err(refuse(&beat.id, E::RestWiring));
                        }
                        target.state.durable == beat.state.durable
                            && target.state.victory == beat.state.victory
                            && target.state.rested != Some(false)
                    }
                };
                if !permitted {
                    return Err(refuse(
                        &beat.id,
                        if matches!(transition.trigger, BeatTrigger::RestComplete) {
                            E::RestWiring
                        } else if beat.kind == BeatKind::Fight {
                            E::FightWiring
                        } else {
                            E::HandlerWiring
                        },
                    ));
                }
            }
            let mut triggers = Vec::new();
            for transition in &beat.transitions {
                if self.find(&transition.target).is_none() {
                    return Err(refuse(&beat.id, E::MissingTarget));
                }
                if triggers.contains(&&transition.trigger) {
                    return Err(refuse(&beat.id, E::DuplicateTransition));
                }
                triggers.push(&transition.trigger);
                if let BeatTrigger::Offer { offer } = &transition.trigger
                    && !beat.offers.iter().any(|o| &o.id == offer)
                {
                    return Err(refuse(&beat.id, E::UnknownOffer));
                }
            }
            if beat.transitions.is_empty() && !self.terminal.contains(&beat.id) {
                return Err(refuse(&beat.id, E::NoExit));
            }
            if beat.kind == BeatKind::Fight
                && (beat.offers.is_empty()
                    || beat.state.durable != "combat"
                    || ![BeatTrigger::FightWon, BeatTrigger::FightLost]
                        .iter()
                        .all(|trigger| {
                            beat.transitions.iter().any(|t| {
                                &t.trigger == trigger
                                    && self
                                        .find(&t.target)
                                        .is_some_and(|b| b.state.durable == "complete")
                            })
                        }))
            {
                return Err(refuse(&beat.id, E::FightWiring));
            }
            if beat
                .offers
                .iter()
                .any(|o| o.effect == BeatEffect::ShortRest)
                && (!matches!(beat.state.durable.as_str(), "complete" | "harbor-inn")
                    || !beat
                        .transitions
                        .iter()
                        .any(|t| t.trigger == BeatTrigger::RestComplete))
            {
                return Err(refuse(&beat.id, E::RestWiring));
            }
        }
        let mut reached = BTreeSet::new();
        let mut pending = vec![self.entry.as_str()];
        while let Some(id) = pending.pop() {
            if !reached.insert(id) {
                continue;
            }
            if let Some(beat) = self.find(id) {
                pending.extend(beat.transitions.iter().map(|t| t.target.as_str()));
            }
        }
        let mut checked = BTreeSet::new();
        for beat in &self.beats {
            if !reached.contains(beat.id.as_str()) {
                return Err(refuse(&beat.id, E::Unreachable));
            }
            let mut path = BTreeSet::new();
            if self.automatic_cycle(&beat.id, &mut path, &mut checked) {
                return Err(refuse(&beat.id, E::AutomaticCycle));
            }
        }
        if !uses_encounter {
            self.validate_native_prefix()?;
        }
        Ok(())
    }
    /// The inherited courier handlers consume one creation/opening/conversation
    /// prefix. Public observation/continuation cycles can revisit generic beats;
    /// they cannot offer an irreversible native source a second time.
    fn validate_native_prefix(&self) -> Result<(), ScriptError> {
        let mut visited = BTreeSet::new();
        let mut pending = vec![(self.entry.as_str(), false, false)];
        while let Some((id, started, answered)) = pending.pop() {
            if !visited.insert((id, started, answered)) {
                continue;
            }
            let beat = self
                .find(id)
                .ok_or_else(|| refuse(id, ScriptErrorReason::MissingTarget))?;
            for offer in &beat.offers {
                let trigger = BeatTrigger::Offer {
                    offer: offer.id.clone(),
                };
                let target = beat
                    .transitions
                    .iter()
                    .find(|transition| transition.trigger == trigger)
                    .and_then(|transition| self.find(&transition.target))
                    .ok_or_else(|| refuse(id, ScriptErrorReason::MissingTarget))?;
                let next = match offer.effect {
                    BeatEffect::CreateCharacter if !started && target.id == self.entry => {
                        (false, false)
                    }
                    BeatEffect::BeginStory if !started => (true, false),
                    BeatEffect::AskCourier
                    | BeatEffect::EscortCourier
                    | BeatEffect::ThirdApproach
                        if started && !answered =>
                    {
                        (true, true)
                    }
                    BeatEffect::Continue
                    | BeatEffect::Observe
                    | BeatEffect::Resolve
                    | BeatEffect::ChooseHarborScene
                        if started && target.state.durable != "room" =>
                    {
                        (started, answered)
                    }
                    _ => return Err(refuse(id, ScriptErrorReason::HandlerWiring)),
                };
                pending.push((&target.id, next.0, next.1));
            }
        }
        Ok(())
    }
    fn automatic_cycle<'a>(
        &'a self,
        id: &'a str,
        path: &mut BTreeSet<&'a str>,
        checked: &mut BTreeSet<&'a str>,
    ) -> bool {
        if checked.contains(id) {
            return false;
        }
        if !path.insert(id) {
            return true;
        }
        let cyclic = self.find(id).is_some_and(|beat| {
            beat.transitions
                .iter()
                .filter(|t| {
                    !matches!(
                        t.trigger,
                        BeatTrigger::Offer { .. } | BeatTrigger::RestComplete
                    )
                })
                .any(|t| self.automatic_cycle(&t.target, path, checked))
        });
        path.remove(id);
        checked.insert(id);
        cyclic
    }
    pub(super) fn current(&self, state: &GameState) -> Result<&Beat, RepositoryError> {
        let [durable] = state.narrative.active_beats.as_slice() else {
            refusal_detail::note("beat-not-single-active");
            return Err(RepositoryError::InvalidCandidate);
        };
        let rested = state
            .narrative
            .completed_beats
            .iter()
            .any(|b| b.entry.as_str() == "short-rest-complete");
        let chosen = super::beat_script_runtime::last_choice(state)?;
        let mut found = None;
        for beat in &self.beats {
            if beat.state.durable != durable.entry.as_str()
                || durable != &model::content(&beat.state.durable)?
                || beat.state.rested.is_some_and(|expected| expected != rested)
            {
                continue;
            }
            if beat
                .state
                .chosen_offer
                .as_ref()
                .is_some_and(|expected| chosen.as_ref().is_none_or(|actual| actual != expected))
            {
                continue;
            }
            if let Some(expected) = beat.state.victory
                && journey::combat_victory(state)? != expected
            {
                continue;
            }
            if found.replace(beat).is_some() {
                refusal_detail::note("beat-ambiguous");
                return Err(RepositoryError::InvalidCandidate);
            }
        }
        found.ok_or_else(|| {
            refusal_detail::note("beat-not-found");
            RepositoryError::InvalidCandidate
        })
    }
}
pub(super) fn with_script<T>(run: impl FnOnce(&BeatScript) -> T) -> T {
    pack::with_content(|content| match &content.beat_script {
        Some(script) => run(script),
        None => run(canonical()),
    })
}
pub(super) fn canonical() -> &'static BeatScript {
    static SCRIPT: std::sync::OnceLock<BeatScript> = std::sync::OnceLock::new();
    SCRIPT.get_or_init(|| {
        serde_json::from_slice(include_bytes!("fixtures/canonical-beat-script.json"))
            .expect("compiled canonical beat script is validated by tests")
    })
}
/// Handler eligibility supplies mechanics/source constraints. The script is the sole
/// sequencing decision: an eligible native action absent here cannot be offered or admitted.
pub(super) fn admit_offers(
    current: &Checkpoint,
    eligible: Vec<(rpc::GameplayActionKind, String)>,
) -> Result<Vec<(rpc::GameplayActionKind, String)>, RepositoryError> {
    with_script(|script| {
        let beat = script.current(current.state())?;
        let mut result = Vec::new();
        for offer in &beat.offers {
            if eligible.iter().any(|(kind, _)| {
                *kind == offer.effect.action()
                    || (offer.effect == BeatEffect::ThirdApproach
                        && *kind == rpc::GameplayActionKind::EscortCourier)
            }) {
                result.push((offer.effect.action(), script.label(offer)?));
            }
        }
        Ok(result)
    })
}
pub(super) fn validate_step(
    before: &Checkpoint,
    after: &Checkpoint,
    action: rpc::GameplayActionKind,
    selected_id: &str,
) -> Result<(), RepositoryError> {
    with_script(|script| {
        let beat = script.current(before.state())?;
        let next = script.current(after.state())?;
        if journey::tactical::present(before.state())
            && beat.kind != BeatKind::Fight
            && action == rpc::GameplayActionKind::EndTurn
            && journey::tactical::dying(before.state())?
            && beat.id == next.id
        {
            return Ok(());
        }
        let action = if journey::tactical::present(before.state())
            && journey::tactical::combat_action(action)
            && !matches!(
                action,
                rpc::GameplayActionKind::SecondWind | rpc::GameplayActionKind::EndTurn
            ) {
            rpc::GameplayActionKind::GreatswordAttack
        } else {
            action
        };
        let offer = beat
            .offers
            .iter()
            .find(|o| o.effect.action() == action && o.id == selected_id)
            .ok_or_else(|| {
                refusal_detail::note("step-offer-missing");
                RepositoryError::InvalidCandidate
            })?;
        let trigger = if beat.kind == BeatKind::Fight && next.kind != BeatKind::Fight {
            if journey::combat_victory(after.state())? {
                BeatTrigger::FightWon
            } else {
                BeatTrigger::FightLost
            }
        } else if offer.effect == BeatEffect::ShortRest
            && journey::short_rest_started(after.state())
            && !journey::short_rest_completed(after.state())
        {
            return Ok(());
        } else if offer.effect == BeatEffect::ShortRest {
            BeatTrigger::RestComplete
        } else {
            BeatTrigger::Offer {
                offer: offer.id.clone(),
            }
        };
        if beat
            .transitions
            .iter()
            .any(|transition| transition.trigger == trigger && transition.target == next.id)
        {
            Ok(())
        } else {
            refusal_detail::note("step-transition-missing");
            Err(RepositoryError::InvalidCandidate)
        }
    })
}

/// Inspect only enum discriminants before typed parsing so refusal identifies the owning
/// authored beat even when its handler/scene spelling cannot deserialize. Typed parsing
/// still rejects unknown and duplicate keys; a malformed document follows its normal path.
pub(super) fn preflight(bytes: &[u8]) -> Result<(), ScriptError> {
    if bytes.len() as u64 > pack::MAX_FILE_BYTES {
        return Ok(());
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Ok(());
    };
    let Some(beats) = value
        .get("beat_script")
        .and_then(|s| s.get("beats"))
        .and_then(|v| v.as_array())
    else {
        return Ok(());
    };
    for beat in beats {
        let name = beat
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("<missing-id>");
        if let Some(kind) = beat.get("kind")
            && serde_json::from_value::<BeatKind>(kind.clone()).is_err()
        {
            return Err(refuse(name, ScriptErrorReason::UnknownHandler));
        }
        if let Some(scene) = beat.get("scene")
            && serde_json::from_value::<pack::SceneKey>(scene.clone()).is_err()
        {
            return Err(refuse(name, ScriptErrorReason::UnknownScene));
        }
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ScriptResumeError {
    RevisionMismatch,
    InvalidState,
}
pub(super) fn validate_resume(current: &Checkpoint) -> Result<(), ScriptResumeError> {
    let pins = model::pins().map_err(|_| ScriptResumeError::InvalidState)?;
    current
        .validate_resume(current.basis(), &pins)
        .map_err(|_| ScriptResumeError::RevisionMismatch)?;
    journey::phase(current)
        .map(|_| ())
        .map_err(|_| ScriptResumeError::InvalidState)
}
