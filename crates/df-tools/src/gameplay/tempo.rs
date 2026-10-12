//! Native, in-memory presentation owner. Public display projection is the sole event source.
use std::time::Instant;

use df_model::checkpoint::{Basis, Checkpoint, TempoState};
use df_persistence::local_demo_scope::{DISPLAY, LocalDemoRole};
use df_protocol::common as rpc;
use df_session::submission::RepositoryError;
use df_tempo::presentation::{PresentationInput, TempoPhase, advance_presentation};

use super::super::{pack, wire};

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
        let frame = advance_presentation(
            &self.state,
            self.phase,
            self.target,
            millis,
            PresentationInput {
                target_permille: self.target,
                impulse_permille: 0,
                release: self.release,
            },
            self.tuning.policy(),
        )
        .map_err(TempoError::Policy)?;
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
        if scene != self.scene || rested != self.rested {
            let millis = self.millis(now)?;
            let restored = Self::start(current, self.tuning, now)?;
            self.state = restored.state;
            self.state.presentation_ticks = millis;
            self.phase = restored.phase;
            self.target = restored.target;
            self.release = restored.release;
            self.scene = scene;
            self.rested = rested;
            // Keep this live owner's origin, rate anchor, and sequence. The next qualifying
            // emitted change advances that sequence; it never resets in an existing watch.
        }
        self.previous = previous;
        self.basis = current.basis();
        Ok(())
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
        let impulse = event.map_or(0, |event| impulse(event, self.tuning));
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
        let phase = wire_phase(current_phase);
        if millis.saturating_sub(self.published_millis) >= CUE_MINIMUM_GAP_MILLIS
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
        }
        Ok(())
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

/// The fewest milliseconds between two published cues. It is the server's own rate limit and
/// is not sent to clients.
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
}
