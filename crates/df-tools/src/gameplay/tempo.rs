//! Native, in-memory presentation owner for public scene facts and judged public exchanges.
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Mutex, PoisonError},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use crate::music_score::PacingBand;
use df_model::checkpoint::{Basis, Checkpoint, TempoState};
use df_persistence::local_demo_scope::{DISPLAY, LocalDemoRole};
use df_protocol::common as rpc;
use df_session::submission::RepositoryError;
use df_tempo::presentation::{PresentationInput, TempoPhase, advance_presentation};

use super::super::{
    adventure::dossier::Effect,
    journey::merchant_bargaining::MerchantMeaning,
    npc_reply::InfluenceVerdict,
    pack, place_music,
    social::{Approach, EmotionLevel, EmotionReaction, ExchangeRecord, PatienceState},
    wire,
};

const SOCIAL_HOLD_MILLIS: u64 = 4_000;
const HISTORY_LIMIT: usize = 128;
const PACING_REVIEW_MILLIS: u64 = 300_000;

struct PacingReview {
    band: PacingBand,
    period_millis: u64,
    next_millis: u64,
    window_start: u64,
    intensity_total: u64,
    intensity_count: u64,
    sequence: u64,
    pending: Option<(PacingBand, u64)>,
}

impl PacingReview {
    fn new(period_millis: u64) -> Self {
        Self {
            band: PacingBand::Steady,
            period_millis,
            next_millis: period_millis,
            window_start: 0,
            intensity_total: 0,
            intensity_count: 0,
            sequence: 0,
            pending: None,
        }
    }

    fn observe(&mut self, intensity: u16) {
        self.intensity_total = self.intensity_total.saturating_add(u64::from(intensity));
        self.intensity_count = self.intensity_count.saturating_add(1);
    }

    fn intensity_band(&self, public_base: u16) -> PacingBand {
        let intensity = self
            .intensity_total
            .checked_div(self.intensity_count)
            .unwrap_or(u64::from(public_base));
        match intensity {
            0..=249 => PacingBand::Calm,
            250..=449 => PacingBand::Steady,
            450..=699 => PacingBand::Driving,
            _ => PacingBand::Urgent,
        }
    }

    fn finish(
        &mut self,
        millis: u64,
        band: PacingBand,
        source: &'static str,
        window: u64,
    ) -> String {
        let old = self.band;
        self.band = band;
        self.next_millis = millis.saturating_add(self.period_millis);
        self.pending = None;
        let reason = if old == band { "hold" } else { source };
        format!(
            "TEMPO pacing review band={}->{} reason={reason} window_s={} sequence={}",
            old.label(),
            band.label(),
            window / 1000,
            self.sequence
        )
    }
}

/// Configuration is process-local presentation policy, not a rules or provider-spend limit.
/// The explicit dev override makes a loop-boundary smoke test practical without a long wait.
fn pacing_review_period() -> u64 {
    for name in [
        "DF_DEV_MUSIC_REVIEW_SECONDS",
        "DF_GAMEPLAY_MUSIC_REVIEW_SECONDS",
    ] {
        match std::env::var(name) {
            Ok(value) => match value.parse::<u64>() {
                Ok(seconds @ 1..=86_400) => return seconds * 1000,
                _ => super::record_audit_line(
                    "gameplay.tempo.configuration",
                    format!("TEMPO pacing configuration invalid setting={name} using=default"),
                ),
            },
            Err(std::env::VarError::NotPresent) => {}
            Err(std::env::VarError::NotUnicode(_)) => super::record_audit_line(
                "gameplay.tempo.configuration",
                format!("TEMPO pacing configuration invalid setting={name}"),
            ),
        }
    }
    PACING_REVIEW_MILLIS
}

/// Only typed judgements from a completed, publicly spoken exchange enter this boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct SocialEvent {
    approach: Approach,
    effect: Effect,
    reaction: Option<EmotionReaction>,
    patience: PatienceState,
    afraid: bool,
    wary: bool,
    influence: Option<InfluenceVerdict>,
    bargain: Option<MerchantMeaning>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mood {
    Neutral,
    Curious,
    Warm,
    Tense,
    Resolved,
}

#[derive(Clone, Copy, Debug)]
struct SocialProfile {
    impulse: i16,
    target: u16,
    mood: Mood,
    cause: &'static str,
}

impl SocialEvent {
    fn profile(self, base: u16) -> SocialProfile {
        let (impulse, target, mood, cause) = if self.approach == Approach::Intimidation {
            if matches!(
                self.influence,
                Some(InfluenceVerdict::Failed | InfluenceVerdict::Refused)
            ) {
                (160, 560, Mood::Tense, "social_intimidation_failed")
            } else {
                (120, 520, Mood::Tense, "social_threat")
            }
        } else if self.bargain == Some(MerchantMeaning::AcceptsHeroPrice) {
            (-140, 160, Mood::Resolved, "social_bargain_resolved")
        } else if self.bargain == Some(MerchantMeaning::Refuses)
            || (matches!(self.approach, Approach::Bargain | Approach::Bribe)
                && matches!(
                    self.influence,
                    Some(InfluenceVerdict::Failed | InfluenceVerdict::Refused)
                ))
        {
            (70, 440, Mood::Tense, "social_bargain_refused")
        } else if matches!(
            self.reaction,
            Some(
                EmotionReaction::Frightened
                    | EmotionReaction::Offended
                    | EmotionReaction::Disgusted
                    | EmotionReaction::NeverForgive
                    | EmotionReaction::Done
                    | EmotionReaction::LosingPatience
            )
        ) || self.patience != PatienceState::Plenty
        {
            (100, 500, Mood::Tense, "social_strained")
        } else if self.reaction == Some(EmotionReaction::Uneasy) || self.afraid || self.wary {
            (80, 460, Mood::Tense, "social_uneasy")
        } else if self.reaction == Some(EmotionReaction::Amused) {
            (-70, 220, Mood::Warm, "social_humour")
        } else if matches!(
            self.reaction,
            Some(EmotionReaction::Warmed | EmotionReaction::Charmed)
        ) || self.effect == Effect::Warm
        {
            let cause = if self.approach == Approach::Flirting {
                "social_flirt"
            } else {
                "social_warm"
            };
            (-90, 180, Mood::Warm, cause)
        } else if matches!(self.approach, Approach::Insult | Approach::Refusal)
            || self.effect == Effect::Cool
        {
            (60, 420, Mood::Tense, "social_cool")
        } else if matches!(
            self.approach,
            Approach::Bargain
                | Approach::Bribe
                | Approach::Deception
                | Approach::Persuasion
                | Approach::Question
        ) {
            (35, 340, Mood::Curious, "social_question")
        } else if self.approach == Approach::Flirting {
            (-40, 240, Mood::Warm, "social_flirt")
        } else {
            (0, base, Mood::Neutral, "social_neutral")
        };
        SocialProfile {
            impulse,
            target,
            mood,
            cause,
        }
    }
}

struct SocialProposal {
    basis: Basis,
    scene_identity: String,
    at: Instant,
    sequence: u64,
    event: SocialEvent,
}

// A bounded presentation mailbox connects the asynchronous reply service to the sole tempo
// owner. No rules or durable gameplay use it. One game/run is hosted per native process.
static SOCIAL_PROPOSALS: Mutex<VecDeque<SocialProposal>> = Mutex::new(VecDeque::new());
static DIAGNOSTICS: Mutex<Option<Value>> = Mutex::new(None);

pub(in crate::gameplay) fn record_social(
    basis: Basis,
    scene_identity: &str,
    exchange: &ExchangeRecord,
    influence: Option<InfluenceVerdict>,
    bargain: Option<MerchantMeaning>,
) {
    let mut proposals = SOCIAL_PROPOSALS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if proposals.len() == 32 {
        proposals.pop_front();
    }
    proposals.push_back(SocialProposal {
        basis,
        scene_identity: scene_identity.to_owned(),
        at: Instant::now(),
        sequence: exchange.sequence,
        event: SocialEvent {
            approach: exchange.approach,
            effect: exchange.effect,
            reaction: exchange.reaction,
            patience: exchange.emotional_state.patience,
            afraid: exchange.emotional_state.afraid != EmotionLevel::None,
            wary: exchange.emotional_state.wary != EmotionLevel::None,
            influence,
            bargain,
        },
    });
}

pub(in crate::gameplay) fn diagnostic_snapshot(last: usize) -> Value {
    let mut snapshot = DIAGNOSTICS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .unwrap_or_else(|| json!({"current": null, "history": []}));
    if let Some(history) = snapshot["history"].as_array_mut() {
        let excess = history.len().saturating_sub(last.min(HISTORY_LIMIT));
        history.drain(..excess);
    }
    let intensity = snapshot["current"]["intensity_permille"]
        .as_u64()
        .unwrap_or(0)
        .min(1000) as u16;
    snapshot["ambient_layers"] = place_music::installed().map_or_else(
        || json!({"status":"unavailable","layers":[]}),
        |music| music.current_ambient_report(intensity),
    );
    snapshot
}

#[derive(Clone, Copy)]
struct SocialEnvelope {
    profile: SocialProfile,
    hold_until: u64,
    release_until: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum TempoError {
    Projection(RepositoryError),
    Policy(df_tempo::presentation::PresentationError),
    InvalidProjection,
    Clock,
    Sequence,
    Binding,
}

/// Contains only already public facts. No private clue, canonical state, AC or ability numbers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicEvent {
    StoryBegun,
    SceneChanged,
    AntagonistRevealed,
    CombatBegun,
    Attack {
        kind: AttackKind,
        by_hero: bool,
        target_hero: bool,
        target_hp_permille: u16,
    },
    Dropped,
    Victory,
    Defeat,
    Rest,
    Arrival,
    Social(SocialEvent),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AttackKind {
    Miss,
    Hit,
    Critical,
    Graze,
}

pub(in crate::gameplay) struct TempoOwner {
    state: TempoState,
    phase: TempoPhase,
    target: u16,
    release: bool,
    basis: Basis,
    previous: rpc::DisplayGameplayView,
    scene: pack::SceneKey,
    rested: bool,
    tuning: pack::TempoTuning,
    origin: Instant,
    published_millis: u64,
    cue: rpc::TempoCue,
    social: Option<SocialEnvelope>,
    pending_social: Option<(SocialEvent, u64)>,
    social_sequence: u64,
    next_social_millis: u64,
    cause: &'static str,
    last_impulse: i16,
    history: VecDeque<Value>,
    unix_origin_millis: u64,
    pacing: PacingReview,
}

fn display(
    current: &Checkpoint,
) -> Result<(rpc::DisplayGameplayView, pack::SceneKey, bool), TempoError> {
    let principal =
        df_types::MemberId::from_bytes(&DISPLAY).map_err(|_| TempoError::InvalidProjection)?;
    let projected = wire::project_journey(current, LocalDemoRole::Display, principal)
        .map_err(TempoError::Projection)?;
    match projected.view.audience {
        Some(rpc::view_message::Audience::Display(view)) => {
            Ok((view, projected.scene, projected.rested))
        }
        _ => Err(TempoError::InvalidProjection),
    }
}

fn target(scene: pack::SceneKey, rested: bool, tuning: pack::TempoTuning) -> (u16, bool) {
    let bases = tuning.base_intensity_permille;
    match scene {
        pack::SceneKey::Room => (bases.room, false),
        pack::SceneKey::Opening => (bases.opening, false),
        pack::SceneKey::DialogueSeal | pack::SceneKey::DialogueEscort => (bases.dialogue, false),
        pack::SceneKey::Combat => (bases.combat, false),
        pack::SceneKey::Victory | pack::SceneKey::Defeat if rested => (bases.rest, true),
        pack::SceneKey::Victory => (bases.victory, true),
        pack::SceneKey::Defeat => (bases.defeat, true),
        pack::SceneKey::Inn => (bases.arrival, true),
    }
}

impl TempoOwner {
    pub(in crate::gameplay) fn start(
        current: &Checkpoint,
        tuning: pack::TempoTuning,
        origin: Instant,
    ) -> Result<Self, TempoError> {
        tuning.policy().validate().map_err(TempoError::Policy)?;
        let (previous, scene, rested) = display(current)?;
        let (target, release) = target(scene, rested, tuning);
        let mut state = current.state().tempo.clone();
        // Resume seeds from current public state, never replays old impulses or downtime.
        state.presentation_ticks = 0;
        state.intensity = i64::from(target);
        state.inertia = 0;
        state.fatigue = vec![(state.policy.clone(), 0)];
        let frame = advance_presentation(
            &state,
            TempoPhase::Calm,
            target,
            0,
            PresentationInput {
                target_permille: target,
                impulse_permille: 0,
                release,
            },
            tuning.policy(),
        )
        .map_err(TempoError::Policy)?;
        let cue = cue(&frame.state, frame.phase, 1, tuning);
        Ok(Self {
            state: frame.state,
            phase: frame.phase,
            target,
            release,
            basis: current.basis(),
            previous,
            scene,
            rested,
            tuning,
            origin,
            published_millis: 0,
            cue,
            social: None,
            pending_social: None,
            social_sequence: 0,
            next_social_millis: 0,
            cause: "start",
            last_impulse: 0,
            history: VecDeque::new(),
            pacing: PacingReview::new(pacing_review_period()),
            unix_origin_millis: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| TempoError::Clock)?
                .as_millis()
                .try_into()
                .map_err(|_| TempoError::Clock)?,
        })
    }

    /// One call after a confirmed transition; canonical gameplay is borrowed and never changed.
    pub(in crate::gameplay) fn accepted(
        &mut self,
        current: &Checkpoint,
        accepted: &rpc::AcceptedAction,
        kind: i32,
        now: Instant,
    ) -> Result<(), TempoError> {
        if current.basis() == self.basis {
            return Ok(());
        }
        if current.basis().session != self.basis.session
            || current.basis().run != self.basis.run
            || current.basis().revision < self.basis.revision
        {
            return Err(TempoError::Binding);
        }
        let (next, scene, rested) = display(current)?;
        let mut events = Vec::new();
        let previous = self
            .previous
            .journey
            .as_ref()
            .ok_or(TempoError::InvalidProjection)?;
        let journey = next.journey.as_ref().ok_or(TempoError::InvalidProjection)?;
        if previous.phase != journey.phase {
            events.push(match rpc::JourneyPhase::try_from(journey.phase) {
                Ok(rpc::JourneyPhase::Opening) => PublicEvent::StoryBegun,
                Ok(rpc::JourneyPhase::Dialogue) => PublicEvent::AntagonistRevealed,
                Ok(rpc::JourneyPhase::Combat) => PublicEvent::CombatBegun,
                _ => PublicEvent::SceneChanged,
            });
        } else if self.previous.scene != next.scene {
            events.push(PublicEvent::SceneChanged);
        }
        for outcome in &accepted.combat {
            let target_hero = journey
                .party
                .iter()
                .any(|hero| hero.participant_id == outcome.target_id);
            let by_hero = journey
                .party
                .iter()
                .any(|hero| hero.participant_id == outcome.actor_id);
            let participant = journey
                .combat
                .as_ref()
                .and_then(|combat| {
                    combat
                        .participants
                        .iter()
                        .find(|participant| participant.participant_id == outcome.target_id)
                })
                .ok_or(TempoError::InvalidProjection)?;
            if participant.maximum_hit_points == 0 {
                return Err(TempoError::InvalidProjection);
            }
            let hp = (u64::from(outcome.target_hit_points) * 1000
                / u64::from(participant.maximum_hit_points))
            .min(1000) as u16;
            events.push(PublicEvent::Attack {
                kind: if outcome.critical {
                    AttackKind::Critical
                } else if outcome.hit {
                    AttackKind::Hit
                } else if outcome.grazed {
                    AttackKind::Graze
                } else {
                    AttackKind::Miss
                },
                by_hero,
                target_hero,
                target_hp_permille: hp,
            });
            if outcome.knocked_out {
                events.push(PublicEvent::Dropped);
            }
        }
        let (mut next_target, next_release) = target(scene, rested, self.tuning);
        if journey.phase == rpc::JourneyPhase::Complete as i32 && previous.phase != journey.phase {
            events.push(if scene == pack::SceneKey::Victory {
                PublicEvent::Victory
            } else {
                PublicEvent::Defeat
            });
        }
        if kind == rpc::GameplayActionKind::ShortRest as i32 {
            events.push(PublicEvent::Rest);
            next_target = self.tuning.base_intensity_permille.rest;
        }
        if kind == rpc::GameplayActionKind::ChooseHarborScene as i32 {
            events.push(PublicEvent::Arrival);
        }
        let millis = self.millis(now)?;
        self.advance(millis, self.target, self.release, None)?;
        if scene != self.scene || rested != self.rested || place_changed(&self.previous, &next) {
            self.social = None;
            self.pending_social = None;
        }
        if let Some(social) = self.social.filter(|social| millis < social.hold_until) {
            next_target = social.profile.target;
        }
        self.target = next_target;
        self.release = next_release;
        for event in events {
            self.advance(millis, next_target, next_release, Some(event))?;
        }
        self.previous = next;
        self.scene = scene;
        self.rested = rested;
        self.basis = current.basis();
        self.publish(millis)?;
        Ok(())
    }

    pub(in crate::gameplay) fn decorate(
        &mut self,
        view: &mut rpc::ViewMessage,
        current: &Checkpoint,
        now: Instant,
    ) -> Result<(), TempoError> {
        self.synchronize(current, now)?;
        let millis = self.millis(now)?;
        // Read sampling never changes the event anchor: viewer count and watch traffic cannot
        // change the deterministic curve through repeated integer rounding/fatigue recovery.
        self.consume_social(millis)?;
        self.review_pacing(millis)?;
        let frame = self.sample(millis)?;
        self.publish_frame(millis, frame.phase, frame.state.intensity)?;
        let journey = match view.audience.as_mut() {
            Some(rpc::view_message::Audience::Display(view)) => view.journey.as_mut(),
            Some(rpc::view_message::Audience::Player(view)) => view.journey.as_mut(),
            None => None,
        }
        .ok_or(TempoError::InvalidProjection)?;
        journey.tempo = Some(self.cue);
        Ok(())
    }
    /// Recovery/reload can deliver a newer checkpoint without a new accepted action. Restore
    /// only changed public scene/rest semantics without replaying impulses or process downtime.
    /// Same-scene revisions (joins/private jobs/receipt recovery) retain the ordinary curve.
    fn synchronize(&mut self, current: &Checkpoint, now: Instant) -> Result<(), TempoError> {
        if current.basis() == self.basis {
            return Ok(());
        }
        if current.basis().session != self.basis.session
            || current.basis().run != self.basis.run
            || current.basis().revision < self.basis.revision
        {
            return Err(TempoError::Binding);
        }
        let (previous, scene, rested) = display(current)?;
        if scene != self.scene || rested != self.rested || place_changed(&self.previous, &previous)
        {
            let millis = self.millis(now)?;
            let restored = Self::start(current, self.tuning, now)?;
            self.state = restored.state;
            self.state.presentation_ticks = millis;
            self.phase = restored.phase;
            self.target = restored.target;
            self.release = restored.release;
            self.scene = scene;
            self.rested = rested;
            self.social = None;
            self.pending_social = None;
            self.cause = "resynchronize";
            self.last_impulse = 0;
            // Keep this live owner's origin, rate anchor, and sequence. The next qualifying
            // emitted change advances that sequence; it never resets in an existing watch.
        }
        self.previous = previous;
        self.basis = current.basis();
        Ok(())
    }

    fn consume_social(&mut self, millis: u64) -> Result<(), TempoError> {
        let proposals: Vec<_> = SOCIAL_PROPOSALS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect();
        for proposal in proposals {
            self.accept_social_proposal(proposal)?;
        }
        // Combat and scene outcomes retain their own authority over presentation.
        if matches!(
            self.scene,
            pack::SceneKey::Combat | pack::SceneKey::Victory | pack::SceneKey::Defeat
        ) {
            self.pending_social = None;
        }
        if millis >= self.next_social_millis
            && let Some((event, submitted_millis)) = self.pending_social.take()
        {
            let lifetime = SOCIAL_HOLD_MILLIS + u64::from(self.tuning.release_hold_millis);
            if millis.saturating_sub(submitted_millis) < lifetime {
                // A late watch samples the already-running envelope, never starts an old line
                // again. Coalesced lines wait for the refractory boundary, independent of reads.
                let at = submitted_millis
                    .max(self.next_social_millis)
                    .max(self.state.presentation_ticks);
                self.social_event(event, at.min(millis))?;
            }
        }
        Ok(())
    }

    fn accept_social_proposal(&mut self, proposal: SocialProposal) -> Result<(), TempoError> {
        let scene_matches = self.previous.scene.as_ref().is_some_and(|scene| {
            proposal.scene_identity == format!("{}|{}", scene.title, scene.scene_asset)
        });
        if proposal.basis.session == self.basis.session
            && proposal.basis.run == self.basis.run
            && proposal.at >= self.origin
            && proposal.sequence > self.social_sequence
            && scene_matches
        {
            self.social_sequence = proposal.sequence;
            self.pending_social = Some((proposal.event, self.millis(proposal.at)?));
        }
        Ok(())
    }

    fn social_event(&mut self, event: SocialEvent, millis: u64) -> Result<(), TempoError> {
        self.advance(millis, self.target, self.release, None)?;
        let intensity_before = self.state.intensity;
        let base = target(self.scene, self.rested, self.tuning).0;
        let mut profile = event.profile(base);
        self.pacing.observe(profile.target);
        if profile.impulse < 0 {
            // A resolving floor must never lift an already quiet place.
            profile.target = profile.target.min(intensity_before as u16);
        }
        self.social = Some(SocialEnvelope {
            profile,
            hold_until: millis.saturating_add(SOCIAL_HOLD_MILLIS),
            release_until: millis
                .saturating_add(SOCIAL_HOLD_MILLIS + u64::from(self.tuning.release_hold_millis)),
        });
        self.target = profile.target;
        self.release = profile.mood == Mood::Resolved;
        self.advance(
            millis,
            profile.target,
            self.release,
            Some(PublicEvent::Social(event)),
        )?;
        // Conversation may become urgent, but repeated lines cannot manufacture a musical peak.
        let ceiling = intensity_before.max(650);
        self.state.intensity = self.state.intensity.min(ceiling);
        self.state.inertia = self
            .state
            .inertia
            .clamp(-160, (ceiling - i64::from(profile.target)).clamp(0, 160));
        self.last_impulse = (self.state.intensity - intensity_before) as i16;
        self.next_social_millis = millis.saturating_add(SOCIAL_HOLD_MILLIS);
        Ok(())
    }

    fn review_pacing(&mut self, millis: u64) -> Result<(), TempoError> {
        if self.scene == pack::SceneKey::Room {
            self.pacing.next_millis = millis.saturating_add(self.pacing.period_millis);
            self.pacing.window_start = millis;
            return Ok(());
        }
        let music = place_music::installed();
        if self.pacing.pending.is_none() && millis >= self.pacing.next_millis {
            self.pacing.sequence = self
                .pacing
                .sequence
                .checked_add(1)
                .ok_or(TempoError::Sequence)?;
            let base = target(self.scene, self.rested, self.tuning).0;
            let fallback = self.pacing.intensity_band(base);
            let window = millis.saturating_sub(self.pacing.window_start);
            self.pacing.pending = Some((fallback, window));
            if let Some(music) = music {
                let scene = self.previous.scene.as_ref();
                // Only the public projection reaches Jev. Canonical arcs may contain secrets
                // and currently expose no audience-filtered numerical tension contract.
                music.review_pacing(self.unix_origin_millis, self.pacing.sequence, json!({
                    "current_band": self.pacing.band.label(),
                    "public_narrative": {
                        "title": scene.map(|scene| &scene.title),
                        "place": scene.map(|scene| &scene.place),
                        "beat": scene.map(|scene| &scene.source_scene_key),
                        "phase": format!("{:?}", self.scene),
                        "narration": self.previous.narration.chars().take(4000).collect::<String>(),
                    },
                    "window_s": window / 1000,
                    "judged_exchanges": self.pacing.intensity_count,
                    "intensity_mean_permille": self.pacing.intensity_total.checked_div(self.pacing.intensity_count),
                    "public_beat_intensity_permille": base,
                }));
            }
            self.pacing.window_start = millis;
            self.pacing.intensity_count = 0;
            self.pacing.intensity_total = 0;
        }
        let Some((fallback, window)) = self.pacing.pending else {
            return Ok(());
        };
        let decision = match music {
            Some(music) => music.pacing_result(self.unix_origin_millis, self.pacing.sequence),
            None => Some(Err("music_director_unavailable")),
        };
        if let Some(decision) = decision {
            let (band, source) = match decision {
                Ok(band) => (band, "arc"),
                Err(reason) => {
                    super::record_audit_line(
                        "gameplay.tempo.pacing_unavailable",
                        format!(
                            "TEMPO pacing judgement unavailable reason={reason} sequence={}",
                            self.pacing.sequence
                        ),
                    );
                    (fallback, "intensity")
                }
            };
            let line = self.pacing.finish(millis, band, source, window);
            if let Some(music) = music {
                music.set_band(band);
            }
            super::record_audit_line("gameplay.tempo.pacing_review", line);
        }
        Ok(())
    }

    fn target_at(&self, millis: u64) -> (u16, bool) {
        if self
            .social
            .is_some_and(|social| millis >= social.hold_until)
        {
            target(self.scene, self.rested, self.tuning)
        } else {
            (self.target, self.release)
        }
    }

    fn sample(&self, millis: u64) -> Result<df_tempo::presentation::TempoFrame, TempoError> {
        let mut state = self.state.clone();
        let mut phase = self.phase;
        let mut previous_target = self.target;
        if let Some(social) = self.social
            && state.presentation_ticks < social.hold_until
            && millis > social.hold_until
        {
            let frame = advance_presentation(
                &state,
                phase,
                previous_target,
                social.hold_until,
                PresentationInput {
                    target_permille: previous_target,
                    impulse_permille: 0,
                    release: self.release,
                },
                self.tuning.policy(),
            )
            .map_err(TempoError::Policy)?;
            state = frame.state;
            phase = frame.phase;
            previous_target = target(self.scene, self.rested, self.tuning).0;
        }
        let (target, release) = self.target_at(millis);
        advance_presentation(
            &state,
            phase,
            previous_target,
            millis,
            PresentationInput {
                target_permille: target,
                impulse_permille: 0,
                release,
            },
            self.tuning.policy(),
        )
        .map_err(TempoError::Policy)
    }

    fn millis(&self, now: Instant) -> Result<u64, TempoError> {
        u64::try_from(
            now.checked_duration_since(self.origin)
                .ok_or(TempoError::Clock)?
                .as_millis(),
        )
        .map_err(|_| TempoError::Clock)
    }
    fn advance(
        &mut self,
        millis: u64,
        target: u16,
        release: bool,
        event: Option<PublicEvent>,
    ) -> Result<(), TempoError> {
        if event.is_none() {
            let frame = self.sample(millis)?;
            (self.target, self.release) = self.target_at(millis);
            self.state = frame.state;
            self.phase = frame.phase;
            return Ok(());
        }
        let impulse = event.map_or(0, |event| impulse(event, self.tuning));
        if let Some(event) = event {
            self.cause = event.cause();
            self.last_impulse = impulse;
        }
        let frame = advance_presentation(
            &self.state,
            self.phase,
            self.target,
            millis,
            PresentationInput {
                target_permille: target,
                impulse_permille: impulse,
                release,
            },
            self.tuning.policy(),
        )
        .map_err(TempoError::Policy)?;
        self.state = frame.state;
        self.phase = frame.phase;
        Ok(())
    }
    fn publish(&mut self, millis: u64) -> Result<(), TempoError> {
        self.publish_frame(millis, self.phase, self.state.intensity)
    }
    fn publish_frame(
        &mut self,
        millis: u64,
        current_phase: TempoPhase,
        intensity: i64,
    ) -> Result<(), TempoError> {
        // The existing phase contract carries coarse mood: warmer resolution, curiosity,
        // or tense anticipation. It never selects a new place set or exposes private words.
        let current_phase = match self
            .social
            .filter(|social| millis < social.release_until)
            .map(|social| social.profile.mood)
        {
            Some(Mood::Warm) => TempoPhase::Calm,
            Some(Mood::Resolved) => TempoPhase::Release,
            Some(Mood::Tense) => TempoPhase::Anticipation,
            Some(Mood::Curious) => TempoPhase::Build,
            _ => current_phase,
        };
        let phase = wire_phase(current_phase);
        let minimum_gap = if self.social.is_some() {
            SOCIAL_HOLD_MILLIS
        } else {
            CUE_MINIMUM_GAP_MILLIS
        };
        if millis.saturating_sub(self.published_millis) >= minimum_gap
            && (phase != self.cue.phase
                || intensity.abs_diff(i64::from(self.cue.intensity_permille)) >= 50)
        {
            self.cue = rpc::TempoCue {
                intensity_permille: intensity as u32,
                phase,
                sequence: self
                    .cue
                    .sequence
                    .checked_add(1)
                    .ok_or(TempoError::Sequence)?,
                hold_millis: hold_millis(current_phase, self.tuning),
            };
            self.published_millis = millis;
            let entry = json!({
                "unix_ms": self.unix_origin_millis.saturating_add(millis),
                "sequence": self.cue.sequence, "phase": phase_name(phase),
                "intensity": self.cue.intensity_permille,
                "cause": self.cause, "impulse": self.last_impulse,
            });
            if self.history.len() == HISTORY_LIMIT {
                self.history.pop_front();
            }
            self.history.push_back(entry);
            super::record_audit_line(
                "gameplay.tempo.cue",
                cue_log(&self.cue, self.target_at(millis).0, self.cause),
            );
            self.last_impulse = 0;
        }
        let (target, release) = self.target_at(millis);
        *DIAGNOSTICS.lock().unwrap_or_else(PoisonError::into_inner) = Some(json!({
            "current": {"phase": phase_name(self.cue.phase), "intensity_permille": self.cue.intensity_permille,
                "target": target, "release": release, "sequence": self.cue.sequence,
                "pacing_band": self.pacing.band.label(),
                "pacing_review_period_s": self.pacing.period_millis / 1000,
                "pacing_review_sequence": self.pacing.sequence,
                "pacing_review_pending": self.pacing.pending.is_some(),
                "next_pacing_review_unix_ms": self.unix_origin_millis.saturating_add(self.pacing.next_millis),
                "mood": self.social.filter(|social| millis < social.release_until).map(|social| format!("{:?}", social.profile.mood))},
            "history": self.history,
        }));
        Ok(())
    }
}
fn place_changed(previous: &rpc::DisplayGameplayView, next: &rpc::DisplayGameplayView) -> bool {
    match (previous.scene.as_ref(), next.scene.as_ref()) {
        (Some(previous), Some(next)) => {
            previous.title != next.title || previous.scene_asset != next.scene_asset
        }
        (None, None) => false,
        _ => true,
    }
}
fn phase_name(phase: i32) -> &'static str {
    match rpc::TempoPhase::try_from(phase) {
        Ok(rpc::TempoPhase::Calm) => "calm",
        Ok(rpc::TempoPhase::Build) => "build",
        Ok(rpc::TempoPhase::Anticipation) => "anticipation",
        Ok(rpc::TempoPhase::Peak) => "peak",
        Ok(rpc::TempoPhase::Release) => "release",
        _ => "unspecified",
    }
}
fn cue_log(cue: &rpc::TempoCue, target: u16, cause: &str) -> String {
    format!(
        "TEMPO cue phase={} intensity_permille={} target={} cause={} sequence={}",
        phase_name(cue.phase),
        cue.intensity_permille,
        target,
        cause,
        cue.sequence
    )
}
impl PublicEvent {
    fn cause(self) -> &'static str {
        match self {
            Self::StoryBegun => "story_begun",
            Self::SceneChanged => "scene_changed",
            Self::AntagonistRevealed => "antagonist_revealed",
            Self::CombatBegun => "combat_begun",
            Self::Attack { .. } => "attack",
            Self::Dropped => "dropped",
            Self::Victory => "victory",
            Self::Defeat => "defeat",
            Self::Rest => "rest",
            Self::Arrival => "arrival",
            Self::Social(event) => event.profile(0).cause,
        }
    }
}
fn impulse(event: PublicEvent, tuning: pack::TempoTuning) -> i16 {
    let values = tuning.impulses_permille;
    let base = match event {
        PublicEvent::StoryBegun => values.story_begun,
        PublicEvent::SceneChanged => values.scene_changed,
        PublicEvent::AntagonistRevealed => values.antagonist_revealed,
        PublicEvent::CombatBegun => values.combat_begun,
        PublicEvent::Attack { kind, .. } => match kind {
            AttackKind::Miss => values.miss,
            AttackKind::Hit => values.hit,
            AttackKind::Critical => values.critical,
            AttackKind::Graze => values.graze,
        },
        PublicEvent::Dropped => values.combatant_dropped,
        PublicEvent::Victory => values.victory,
        PublicEvent::Defeat => values.defeat,
        PublicEvent::Rest => values.short_rest,
        PublicEvent::Arrival => values.arrival,
        PublicEvent::Social(event) => event.profile(0).impulse,
    };
    match event {
        PublicEvent::Attack {
            target_hero: true,
            target_hp_permille: 1..=250,
            ..
        } => base.saturating_add(values.hero_low).clamp(-1000, 1000),
        _ => base,
    }
}
fn wire_phase(phase: TempoPhase) -> i32 {
    (match phase {
        TempoPhase::Calm => rpc::TempoPhase::Calm,
        TempoPhase::Build => rpc::TempoPhase::Build,
        TempoPhase::Anticipation => rpc::TempoPhase::Anticipation,
        TempoPhase::Peak => rpc::TempoPhase::Peak,
        TempoPhase::Release => rpc::TempoPhase::Release,
    }) as i32
}
fn cue(
    state: &TempoState,
    phase: TempoPhase,
    sequence: u64,
    tuning: pack::TempoTuning,
) -> rpc::TempoCue {
    rpc::TempoCue {
        intensity_permille: state.intensity as u32,
        phase: wire_phase(phase),
        sequence,
        hold_millis: hold_millis(phase, tuning),
    }
}

/// Existing story/combat cue rate limit. Roleplay uses its longer audible hold so a complete
/// crossfade settles before another target. Neither is the release hold sent to clients.
const CUE_MINIMUM_GAP_MILLIS: u64 = 250;

/// What a cue's `hold_millis` means: for a Release cue, how long the music holds the release
/// before settling to the calm bed (from the tuning); for every other phase, 0 (nothing to
/// hold). It is never the rate limit between cues.
fn hold_millis(phase: TempoPhase, tuning: pack::TempoTuning) -> u32 {
    if phase == TempoPhase::Release {
        tuning.release_hold_millis
    } else {
        0
    }
}

/// Optional presentation failure cannot withhold an accepted outcome or legal view.
pub(in crate::gameplay) fn observe(result: Result<(), TempoError>) {
    if let Err(error) = result {
        let context = df_observe::OperationContext {
            trace_parent: String::new(),
            build: crate::BUILD_ID.to_owned(),
        };
        let status = match error {
            TempoError::Projection(_) => "tempo_projection_refused",
            TempoError::Policy(_) => "tempo_policy_refused",
            TempoError::InvalidProjection => "tempo_public_event_invalid",
            TempoError::Clock => "tempo_clock_refused",
            TempoError::Sequence => "tempo_sequence_exhausted",
            TempoError::Binding => "tempo_owner_binding_refused",
        };
        df_observe::begin(&context, "gameplay.tempo").finish_unmeasured(status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn pacing_reviews_choose_calm_and_tense_windows_and_hold_for_a_full_period() {
        let mut owner = owner(pack::TempoTuning::default());
        owner.scene = pack::SceneKey::Opening;
        owner.pacing = PacingReview::new(300_000);
        for _ in 0..20 {
            owner.pacing.observe(180);
        }
        owner.review_pacing(299_999).unwrap();
        assert_eq!(owner.pacing.band, PacingBand::Steady);
        owner.review_pacing(300_000).unwrap();
        assert_eq!(owner.pacing.band, PacingBand::Calm);
        assert_eq!(owner.pacing.next_millis, 600_000);
        for _ in 0..20 {
            owner.pacing.observe(560);
        }
        owner.review_pacing(599_999).unwrap();
        assert_eq!(owner.pacing.band, PacingBand::Calm);
        owner.review_pacing(600_000).unwrap();
        assert_eq!(owner.pacing.band, PacingBand::Driving);
        assert_eq!(owner.pacing.sequence, 2);
        assert_eq!(owner.pacing.next_millis, 900_000);
        let line = owner
            .pacing
            .finish(900_000, PacingBand::Driving, "arc", 300_000);
        assert!(line.contains("band=driving->driving reason=hold window_s=300 sequence=2"));
    }

    #[test]
    fn pacing_window_uses_judged_social_intensity_without_changing_hs18_impulses() {
        let mut owner = owner(pack::TempoTuning::default());
        owner
            .social_event(social(Approach::Intimidation), 1000)
            .unwrap();
        assert_eq!(owner.pacing.intensity_count, 1);
        assert_eq!(owner.pacing.intensity_band(0), PacingBand::Driving);
        let mut window = PacingReview::new(60_000);
        for intensity in [850, 900, 950] {
            window.observe(intensity);
        }
        assert_eq!(window.intensity_band(0), PacingBand::Urgent);
        assert_eq!(window.next_millis, 60_000);
        assert_eq!(
            window.finish(60_000, PacingBand::Calm, "arc", 60_000),
            "TEMPO pacing review band=steady->calm reason=arc window_s=60 sequence=0"
        );
        assert_eq!(window.next_millis, 120_000);
    }
    fn owner(tuning: pack::TempoTuning) -> TempoOwner {
        TempoOwner::start(
            &super::super::super::journey::initial().unwrap(),
            tuning,
            Instant::now(),
        )
        .unwrap()
    }
    fn step(
        owner: &mut TempoOwner,
        at: u64,
        target: u16,
        release: bool,
        event: Option<PublicEvent>,
    ) -> (TempoPhase, i64) {
        owner
            .advance(at, owner.target, owner.release, None)
            .unwrap();
        owner.target = target;
        owner.release = release;
        owner.advance(at, target, release, event).unwrap();
        (owner.phase, owner.state.intensity)
    }
    fn script(tuning: pack::TempoTuning) -> Vec<(TempoPhase, i64)> {
        let mut owner = owner(tuning);
        let bases = tuning.base_intensity_permille;
        let mut frames = vec![(owner.phase, owner.state.intensity)];
        for (at, target, release, event) in [
            (5000, bases.opening, false, Some(PublicEvent::StoryBegun)),
            (
                12000,
                bases.dialogue,
                false,
                Some(PublicEvent::AntagonistRevealed),
            ),
            (18000, bases.combat, false, Some(PublicEvent::CombatBegun)),
            (
                24000,
                bases.combat,
                false,
                Some(PublicEvent::Attack {
                    kind: AttackKind::Miss,
                    by_hero: true,
                    target_hero: false,
                    target_hp_permille: 1000,
                }),
            ),
            (
                30000,
                bases.combat,
                false,
                Some(PublicEvent::Attack {
                    kind: AttackKind::Hit,
                    by_hero: false,
                    target_hero: true,
                    target_hp_permille: 600,
                }),
            ),
            (
                36000,
                bases.combat,
                false,
                Some(PublicEvent::Attack {
                    kind: AttackKind::Graze,
                    by_hero: true,
                    target_hero: false,
                    target_hp_permille: 450,
                }),
            ),
            (
                42000,
                bases.combat,
                false,
                Some(PublicEvent::Attack {
                    kind: AttackKind::Critical,
                    by_hero: true,
                    target_hero: false,
                    target_hp_permille: 150,
                }),
            ),
            (
                48000,
                bases.combat,
                false,
                Some(PublicEvent::Attack {
                    kind: AttackKind::Hit,
                    by_hero: false,
                    target_hero: true,
                    target_hp_permille: 100,
                }),
            ),
            (54000, bases.combat, false, Some(PublicEvent::Dropped)),
            (54000, bases.victory, true, Some(PublicEvent::Victory)),
            (70000, bases.rest, true, Some(PublicEvent::Rest)),
            (90000, bases.arrival, true, Some(PublicEvent::Arrival)),
            (120000, bases.arrival, true, None),
        ] {
            frames.push(step(&mut owner, at, target, release, event));
        }
        frames
    }
    #[test]
    fn scripted_public_journey_builds_spikes_and_releases_deterministically() {
        let tuning = pack::TempoTuning::default();
        let frames = script(tuning);
        assert_eq!(frames, script(tuning));
        println!("SCRIPTED TEMPO {frames:?}");
        assert_eq!(frames[0].0, TempoPhase::Calm);
        assert_eq!(frames[1].0, TempoPhase::Build);
        assert_eq!(frames[2].0, TempoPhase::Anticipation);
        assert_eq!(frames[3].0, TempoPhase::Peak);
        assert!(frames[7].1 > frames[6].1, "critical spikes");
        assert!(frames[8].1 > frames[7].1, "visible low hero spikes");
        assert_eq!(frames[10].0, TempoPhase::Release);
        assert!(frames[11].1 < frames[10].1);
        assert_eq!(frames.last().unwrap().0, TempoPhase::Calm);
        let mut changed = tuning;
        changed.impulses_permille.critical = 20;
        assert_ne!(frames, script(changed));
    }
    #[test]
    fn victory_and_nonlethal_defeat_have_different_release_shapes() {
        let tuning = pack::TempoTuning::default();
        let mut victory = owner(tuning);
        let mut defeat = owner(tuning);
        victory.state.intensity = 900;
        defeat.state.intensity = 900;
        victory.phase = TempoPhase::Peak;
        defeat.phase = TempoPhase::Peak;
        let won = step(
            &mut victory,
            0,
            tuning.base_intensity_permille.victory,
            true,
            Some(PublicEvent::Victory),
        );
        let lost = step(
            &mut defeat,
            0,
            tuning.base_intensity_permille.defeat,
            true,
            Some(PublicEvent::Defeat),
        );
        assert_eq!(won.0, TempoPhase::Release);
        assert_eq!(lost.0, TempoPhase::Release);
        assert!(lost.1 > won.1);
        assert!(
            step(
                &mut defeat,
                20000,
                tuning.base_intensity_permille.defeat,
                true,
                None
            )
            .1 > step(
                &mut victory,
                20000,
                tuning.base_intensity_permille.victory,
                true,
                None
            )
            .1
        );
        assert_eq!(target(pack::SceneKey::Defeat, false, tuning), (300, true));
        assert_eq!(target(pack::SceneKey::Defeat, true, tuning), (80, true));
    }
    #[test]
    fn only_a_release_cue_carries_a_hold_and_it_is_the_tuned_release_duration() {
        let tuning = pack::TempoTuning::default();
        assert_eq!(tuning.release_hold_millis, 14_000);
        assert_eq!(hold_millis(TempoPhase::Release, tuning), 14_000);
        for phase in [
            TempoPhase::Calm,
            TempoPhase::Build,
            TempoPhase::Anticipation,
            TempoPhase::Peak,
        ] {
            assert_eq!(hold_millis(phase, tuning), 0, "{phase:?}");
        }
        let mut tuned = tuning;
        tuned.release_hold_millis = 6_000;
        let mut victory = owner(tuned);
        victory.state.intensity = 900;
        victory.phase = TempoPhase::Peak;
        step(
            &mut victory,
            0,
            tuned.base_intensity_permille.victory,
            true,
            Some(PublicEvent::Victory),
        );
        victory.publish(1_000).unwrap();
        assert_eq!(victory.cue.phase, rpc::TempoPhase::Release as i32);
        assert_eq!(victory.cue.hold_millis, 6_000);
        assert_ne!(victory.cue.hold_millis, 250);
    }
    #[test]
    fn cues_are_rate_bounded_and_repeated_views_do_not_replay_impulses() {
        let mut owner = owner(pack::TempoTuning::default());
        let origin = owner.origin;
        step(&mut owner, 100, 260, false, Some(PublicEvent::StoryBegun));
        owner.publish(100).unwrap();
        assert_eq!(owner.cue.sequence, 1);
        owner.publish(250).unwrap();
        assert_eq!(owner.cue.sequence, 2);
        let cue = owner.cue;
        owner.publish(250).unwrap();
        assert_eq!(owner.cue, cue);
        let current = super::super::super::journey::initial().unwrap();
        let mut view = wire::journey_view(
            &current,
            LocalDemoRole::Display,
            df_types::MemberId::from_bytes(&DISPLAY).unwrap(),
        )
        .unwrap();
        owner
            .decorate(&mut view, &current, origin + Duration::from_millis(250))
            .unwrap();
        let state = owner.state.clone();
        owner
            .decorate(&mut view, &current, origin + Duration::from_millis(250))
            .unwrap();
        assert_eq!(owner.state, state);
        owner
            .accepted(
                &current,
                &rpc::AcceptedAction::default(),
                rpc::GameplayActionKind::BeginStory as i32,
                origin + Duration::from_millis(250),
            )
            .unwrap();
        assert_eq!(
            owner.state, state,
            "same checkpoint receipt cannot repeat event"
        );
        let next_basis = Basis {
            revision: current.basis().revision.next_sequence().unwrap(),
            ..current.basis()
        };
        let unrelated =
            super::super::super::model::checkpoint(next_basis, current.state().clone()).unwrap();
        owner
            .decorate(&mut view, &unrelated, origin + Duration::from_millis(300))
            .unwrap();
        assert_eq!(
            owner.state, state,
            "same public scene on a newer basis retains the curve"
        );
        assert_eq!(owner.basis, unrelated.basis());
    }
    fn social(approach: Approach) -> SocialEvent {
        SocialEvent {
            approach,
            effect: Effect::Steady,
            reaction: None,
            patience: PatienceState::Plenty,
            afraid: false,
            wary: false,
            influence: None,
            bargain: None,
        }
    }

    fn dialogue_owner() -> TempoOwner {
        let mut owner = owner(pack::TempoTuning::default());
        owner.scene = pack::SceneKey::DialogueSeal;
        owner.target = 480;
        owner.state.intensity = 480;
        owner.phase = TempoPhase::Anticipation;
        owner
    }

    #[test]
    fn social_outcomes_set_bounded_impulses_moods_and_targets_from_judgements() {
        let mut cases = vec![
            (social(Approach::Intimidation), 120, Mood::Tense),
            (
                SocialEvent {
                    influence: Some(InfluenceVerdict::Failed),
                    ..social(Approach::Intimidation)
                },
                160,
                Mood::Tense,
            ),
            (
                SocialEvent {
                    influence: Some(InfluenceVerdict::Refused),
                    ..social(Approach::Intimidation)
                },
                160,
                Mood::Tense,
            ),
            (social(Approach::Flirting), -40, Mood::Warm),
            (
                SocialEvent {
                    reaction: Some(EmotionReaction::Charmed),
                    ..social(Approach::Flirting)
                },
                -90,
                Mood::Warm,
            ),
            (
                SocialEvent {
                    reaction: Some(EmotionReaction::Amused),
                    ..social(Approach::Other)
                },
                -70,
                Mood::Warm,
            ),
            (
                SocialEvent {
                    bargain: Some(MerchantMeaning::AcceptsHeroPrice),
                    ..social(Approach::Bargain)
                },
                -140,
                Mood::Resolved,
            ),
            (
                SocialEvent {
                    bargain: Some(MerchantMeaning::Refuses),
                    ..social(Approach::Bargain)
                },
                70,
                Mood::Tense,
            ),
            (
                SocialEvent {
                    effect: Effect::Warm,
                    ..social(Approach::Kindness)
                },
                -90,
                Mood::Warm,
            ),
            (
                SocialEvent {
                    effect: Effect::Cool,
                    ..social(Approach::Other)
                },
                60,
                Mood::Tense,
            ),
            (social(Approach::Question), 35, Mood::Curious),
            (social(Approach::Other), 0, Mood::Neutral),
        ];
        for reaction in [
            EmotionReaction::Frightened,
            EmotionReaction::Offended,
            EmotionReaction::Disgusted,
            EmotionReaction::NeverForgive,
            EmotionReaction::LosingPatience,
            EmotionReaction::Done,
        ] {
            cases.push((
                SocialEvent {
                    reaction: Some(reaction),
                    ..social(Approach::Question)
                },
                100,
                Mood::Tense,
            ));
        }
        for patience in [PatienceState::Short, PatienceState::Done] {
            cases.push((
                SocialEvent {
                    patience,
                    ..social(Approach::Question)
                },
                100,
                Mood::Tense,
            ));
        }
        cases.push((
            SocialEvent {
                reaction: Some(EmotionReaction::Uneasy),
                ..social(Approach::Question)
            },
            80,
            Mood::Tense,
        ));
        for (event, impulse, mood) in cases {
            let profile = event.profile(480);
            assert_eq!(profile.impulse, impulse);
            assert_eq!(profile.mood, mood);
            assert!(profile.target <= 650);
            let mut owner = dialogue_owner();
            owner.social_event(event, 4_000).unwrap();
            let changed = owner.state.intensity;
            assert!((0..=650).contains(&changed));
            assert_eq!(changed.cmp(&480), i64::from(impulse).cmp(&0));
            owner.publish(4_000).unwrap();
            let expected = match mood {
                Mood::Warm => rpc::TempoPhase::Calm,
                Mood::Tense => rpc::TempoPhase::Anticipation,
                Mood::Curious => rpc::TempoPhase::Build,
                Mood::Resolved => rpc::TempoPhase::Release,
                Mood::Neutral => rpc::TempoPhase::Anticipation,
            };
            assert_eq!(owner.cue.phase, expected as i32);
            let settled = owner.sample(124_000).unwrap();
            assert!((settled.state.intensity - 480).abs() <= 1);
        }
    }

    #[test]
    fn social_hold_release_and_read_sampling_preserve_the_same_decay_curve() {
        let mut owner = dialogue_owner();
        owner
            .social_event(social(Approach::Intimidation), 4_000)
            .unwrap();
        owner.publish(4_000).unwrap();
        let cue = owner.cue;
        owner
            .social_event(social(Approach::Flirting), 5_000)
            .unwrap();
        owner.publish(7_999).unwrap();
        assert_eq!(
            owner.cue, cue,
            "a new mood cannot interrupt the minimum audible hold"
        );
        let anchor = owner.state.clone();
        let later = owner.sample(45_000).unwrap();
        for at in 5_000..45_000 {
            owner.sample(at).unwrap();
        }
        assert_eq!(owner.sample(45_000).unwrap(), later);
        assert_eq!(owner.state, anchor, "readers cannot move the curve anchor");
        assert_eq!(owner.target_at(8_999).0, 240);
        assert_eq!(owner.target_at(9_000).0, 480);
        let dip = owner.sample(9_000).unwrap().state.intensity;
        assert!(dip < 480, "the held warm target eases below the place base");
        assert!(
            later.state.intensity > dip,
            "the warm dip releases toward the place base"
        );
        assert!((later.state.intensity - 480).abs() < 5);
    }

    #[test]
    fn social_bursts_coalesce_and_sustained_talk_never_saturates_or_replays() {
        let mut owner = dialogue_owner();
        owner
            .social_event(social(Approach::Intimidation), 4_000)
            .unwrap();
        let first = owner.state.intensity;
        for _ in 0..100 {
            owner.pending_social = Some((social(Approach::Intimidation), 4_000));
        }
        owner.next_social_millis = 8_000;
        owner.consume_social(7_999).unwrap();
        assert_eq!(owner.state.intensity, first);
        owner.consume_social(8_000).unwrap();
        assert!(owner.state.intensity <= 650);
        let once = owner.state.clone();
        owner.consume_social(8_000).unwrap();
        assert_eq!(owner.state, once);
        for at in (12_000..120_000).step_by(4_000) {
            owner.pending_social = Some((social(Approach::Intimidation), at));
            owner.consume_social(at).unwrap();
            assert!(owner.state.intensity <= 650);
            assert!(owner.state.inertia <= 160);
            for offset in (0..4_000).step_by(100) {
                assert!(owner.sample(at + offset).unwrap().state.intensity <= 650);
            }
        }
    }

    #[test]
    fn tempo_log_is_once_per_cue_and_history_is_bounded_in_sequence_order() {
        let mut owner = dialogue_owner();
        for index in 1..=140 {
            owner.state.intensity = if index % 2 == 0 { 300 } else { 600 };
            owner.cause = "social_threat";
            owner.last_impulse = 120;
            owner.publish(index * 4_000).unwrap();
            let count = owner.history.len();
            owner.publish(index * 4_000).unwrap();
            assert_eq!(owner.history.len(), count);
        }
        assert_eq!(owner.history.len(), HISTORY_LIMIT);
        assert_eq!(
            owner.history.back().unwrap()["sequence"],
            owner.cue.sequence
        );
        assert_eq!(owner.history.back().unwrap()["cause"], "social_threat");
        assert!(
            owner.history.front().unwrap()["unix_ms"].as_u64().unwrap()
                < owner.history.back().unwrap()["unix_ms"].as_u64().unwrap()
        );
        assert_eq!(
            cue_log(
                &rpc::TempoCue {
                    phase: rpc::TempoPhase::Anticipation as i32,
                    intensity_permille: 600,
                    sequence: 42,
                    hold_millis: 0
                },
                520,
                "social_threat"
            ),
            "TEMPO cue phase=anticipation intensity_permille=600 target=520 cause=social_threat sequence=42"
        );
    }

    #[test]
    fn live_music_social_cues_use_place_layers_and_keep_combat_silent() {
        use crate::music_score::{Phase, mix_for, scene_layer_gains};
        let warm = mix_for(Phase::Calm, 180, false);
        let tense = mix_for(Phase::Anticipation, 600, false);
        let released = mix_for(Phase::Release, 160, false);
        assert!(tense.layers.in_track_order()[2] > warm.layers.in_track_order()[2]);
        assert!(released.layers.in_track_order()[1] > 0.0);
        assert_eq!(
            scene_layer_gains(tense.layers.in_track_order(), false)[3],
            0.0
        );
    }
    #[test]
    fn social_mailbox_rejects_old_duplicate_and_replaced_scene_proposals() {
        let mut owner = dialogue_owner();
        let identity = owner
            .previous
            .scene
            .as_ref()
            .map(|scene| format!("{}|{}", scene.title, scene.scene_asset))
            .unwrap();
        let proposal = |sequence, scene_identity| SocialProposal {
            basis: owner.basis,
            scene_identity,
            at: owner.origin,
            sequence,
            event: social(Approach::Flirting),
        };
        let wrong_scene = proposal(1, "elsewhere".to_owned());
        let first = proposal(1, identity.clone());
        let duplicate = proposal(1, identity.clone());
        let older = proposal(0, identity);
        owner.accept_social_proposal(wrong_scene).unwrap();
        assert!(owner.pending_social.is_none());
        owner.accept_social_proposal(first).unwrap();
        assert!(owner.pending_social.take().is_some());
        owner.accept_social_proposal(duplicate).unwrap();
        owner.accept_social_proposal(older).unwrap();
        assert!(owner.pending_social.is_none());
    }
    #[test]
    fn social_late_join_samples_the_existing_envelope_and_skips_expired_lines() {
        let mut immediate = dialogue_owner();
        immediate.pending_social = Some((social(Approach::Intimidation), 4_000));
        immediate.consume_social(4_000).unwrap();
        let mut delayed = dialogue_owner();
        delayed.pending_social = Some((social(Approach::Intimidation), 4_000));
        delayed.consume_social(10_000).unwrap();
        assert_eq!(
            immediate.sample(12_000).unwrap(),
            delayed.sample(12_000).unwrap()
        );
        let mut expired = dialogue_owner();
        expired.pending_social = Some((social(Approach::Intimidation), 4_000));
        expired.consume_social(24_000).unwrap();
        assert!(expired.social.is_none());
        assert_eq!(expired.state.intensity, 480);
    }
    #[test]
    fn social_impulse_direction_is_preserved_at_quiet_and_already_intense_boundaries() {
        for before in [0, 60, 160, 480, 650, 800, 1000] {
            for event in [
                social(Approach::Intimidation),
                social(Approach::Flirting),
                SocialEvent {
                    bargain: Some(MerchantMeaning::AcceptsHeroPrice),
                    ..social(Approach::Bargain)
                },
            ] {
                let mut owner = dialogue_owner();
                owner.state.intensity = before;
                owner.state.presentation_ticks = 4_000;
                let impulse = event.profile(owner.target).impulse;
                owner.social_event(event, 4_000).unwrap();
                if impulse > 0 {
                    assert!(
                        owner.state.intensity >= before,
                        "a threat cannot lower {before}"
                    );
                } else {
                    assert!(
                        owner.state.intensity <= before,
                        "a resolution cannot raise {before}"
                    );
                }
                assert!((0..=1000).contains(&owner.state.intensity));
            }
        }
    }
}
