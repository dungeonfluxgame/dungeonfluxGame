//! The session's one forward state machine. See
//! `artifacts/tmp/campaign/flow-a2/STATE-MACHINE.md`.
//!
//! The state is derived from the checkpoint (never stored), every command is
//! checked against the table in `next` before any handler runs in
//! `stage_using`, and the table only ever moves forward. The fight is a
//! sub-machine: its turn order and dying/death-save facts live in the
//! encounter, and it is entered (`StartFight`) and left (a combat event whose
//! rules outcome ends it) only through this table.

use df_model::checkpoint::Checkpoint;
use df_protocol::common as rpc;
use df_session::submission::RepositoryError;

use super::phase;
use crate::gameplay::beat_script::BeatEffect;

/// Where a session is, from lobby to the end card.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SessionState {
    Lobby,
    Story,
    Conversation,
    Fight,
    Aftermath,
    Ended,
}

impl SessionState {
    /// The state of a checkpoint: its journey phase, with the end card split
    /// out of `Complete`.
    pub(super) fn of(current: &Checkpoint) -> Result<Self, RepositoryError> {
        Ok(match phase(current)? {
            rpc::JourneyPhase::Room => Self::Lobby,
            rpc::JourneyPhase::Opening => Self::Story,
            rpc::JourneyPhase::Dialogue => Self::Conversation,
            rpc::JourneyPhase::Combat => Self::Fight,
            rpc::JourneyPhase::Complete
                if crate::gameplay::beat_script_runtime::ended(current.state())? =>
            {
                Self::Ended
            }
            rpc::JourneyPhase::Complete => Self::Aftermath,
            _ => return Err(RepositoryError::InvalidCandidate),
        })
    }

    /// Forward order: a legal transition never lowers the rank.
    pub(super) fn rank(self) -> u8 {
        match self {
            Self::Lobby => 0,
            Self::Story => 1,
            Self::Conversation => 2,
            Self::Fight => 3,
            Self::Aftermath => 4,
            Self::Ended => 5,
        }
    }
}

/// What a submitted command asks the session to do, named by progress.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Event {
    CreateHero,
    Begin,
    StoryChoice,
    /// A pure authored Continue step: one of the loaded script's own
    /// validated forward edges.
    ScriptStep,
    StartFight,
    CombatAction,
    EndTurn,
    Rest,
    Resolve,
}

impl Event {
    /// The event of a selected offer, from its authored effect. Selection
    /// already maps every tactical combat action onto a fight-beat offer
    /// (attack-like actions onto the attack offer), so the effect names the
    /// progress for those too.
    pub(super) fn of(effect: BeatEffect) -> Self {
        match effect {
            BeatEffect::CreateCharacter => Self::CreateHero,
            BeatEffect::BeginStory => Self::Begin,
            BeatEffect::Continue => Self::ScriptStep,
            BeatEffect::Resolve => Self::Resolve,
            BeatEffect::DefendCourier => Self::StartFight,
            BeatEffect::ShortRest => Self::Rest,
            BeatEffect::EndTurn => Self::EndTurn,
            BeatEffect::AskCourier
            | BeatEffect::EscortCourier
            | BeatEffect::ThirdApproach
            | BeatEffect::Observe
            | BeatEffect::ChooseHarborScene => Self::StoryChoice,
            BeatEffect::GreatswordAttack | BeatEffect::SecondWind => Self::CombatAction,
        }
    }
}

/// Where a legal event takes the session. `FightMayEnd` and
/// `ResolveMayEnd` are decided by the rules outcome of the same commit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Transition {
    To(SessionState),
    FightMayEnd,
    ResolveMayEnd,
}

/// A command that is illegal in the session's current state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MachineRefusal {
    pub(super) state: SessionState,
    pub(super) event: Event,
}

/// The one table: for the current state and an event, the next state or a
/// typed refusal.
pub(super) fn next(state: SessionState, event: Event) -> Result<Transition, MachineRefusal> {
    use Event as E;
    use SessionState as S;
    let transition = match (state, event) {
        (S::Lobby, E::CreateHero | E::ScriptStep) => Transition::To(S::Lobby),
        (S::Lobby, E::Begin) => Transition::To(S::Story),
        (S::Story, E::StoryChoice) => Transition::To(S::Conversation),
        (S::Story, E::ScriptStep) => Transition::To(S::Story),
        (S::Story, E::StartFight) => Transition::To(S::Fight),
        (S::Conversation, E::StoryChoice | E::ScriptStep) => Transition::To(S::Conversation),
        (S::Conversation, E::StartFight) => Transition::To(S::Fight),
        (S::Fight, E::CombatAction | E::EndTurn) => Transition::FightMayEnd,
        // Time passing for a dying hero after the fight (death saves) is the
        // one EndTurn outside a fight.
        (S::Aftermath, E::EndTurn | E::Rest | E::StoryChoice | E::ScriptStep) => {
            Transition::To(S::Aftermath)
        }
        (S::Aftermath, E::Resolve) => Transition::ResolveMayEnd,
        _ => return Err(MachineRefusal { state, event }),
    };
    // Forward only, whatever a future table edit says.
    if let Transition::To(target) = transition
        && target.rank() < state.rank()
    {
        return Err(MachineRefusal { state, event });
    }
    Ok(transition)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATES: [SessionState; 6] = [
        SessionState::Lobby,
        SessionState::Story,
        SessionState::Conversation,
        SessionState::Fight,
        SessionState::Aftermath,
        SessionState::Ended,
    ];
    const EVENTS: [Event; 9] = [
        Event::CreateHero,
        Event::Begin,
        Event::StoryChoice,
        Event::ScriptStep,
        Event::StartFight,
        Event::CombatAction,
        Event::EndTurn,
        Event::Rest,
        Event::Resolve,
    ];

    #[test]
    fn the_table_names_every_state_and_event() {
        // Every cell is decided: a transition or a refusal naming the cell.
        for state in STATES {
            for event in EVENTS {
                match next(state, event) {
                    Ok(Transition::To(target)) => {
                        assert!(
                            target.rank() >= state.rank(),
                            "{state:?} -{event:?}-> {target:?} goes backwards"
                        );
                    }
                    Ok(Transition::FightMayEnd) => {
                        assert_eq!(state, SessionState::Fight, "{event:?}");
                    }
                    Ok(Transition::ResolveMayEnd) => {
                        assert_eq!(state, SessionState::Aftermath, "{event:?}");
                    }
                    Err(refusal) => {
                        assert_eq!(refusal, MachineRefusal { state, event });
                    }
                }
            }
        }
        // The spine of a game, and its end.
        assert_eq!(
            next(SessionState::Lobby, Event::Begin),
            Ok(Transition::To(SessionState::Story))
        );
        assert_eq!(
            next(SessionState::Conversation, Event::StartFight),
            Ok(Transition::To(SessionState::Fight))
        );
        for event in EVENTS {
            assert!(next(SessionState::Ended, event).is_err(), "{event:?}");
            assert!(
                next(SessionState::Fight, event).is_err()
                    || matches!(event, Event::CombatAction | Event::EndTurn),
                "only combat events are legal in a fight: {event:?}"
            );
        }
    }

    /// A tiny deterministic generator, so the walk needs no dependency.
    fn xorshift(state: &mut u64) -> u64 {
        let mut value = *state;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        *state = value;
        value
    }

    #[test]
    fn random_walks_never_go_backwards_and_forward_play_always_ends() {
        for seed in 1u64..=200 {
            let mut rng = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
            let mut state = SessionState::Lobby;
            // Random legal and illegal events: refusals change nothing, legal
            // events never lower the rank, and the state is always one of the
            // six named states.
            for _ in 0..512 {
                let event = EVENTS[usize::try_from(xorshift(&mut rng) % 9).unwrap()];
                match next(state, event) {
                    Ok(Transition::To(target)) => {
                        assert!(target.rank() >= state.rank(), "seed {seed}");
                        state = target;
                    }
                    Ok(Transition::FightMayEnd) => {
                        // The rules outcome chooses; take the later one half
                        // of the time so walks can reach the aftermath.
                        if xorshift(&mut rng) % 2 == 0 {
                            state = SessionState::Aftermath;
                        }
                    }
                    Ok(Transition::ResolveMayEnd) => state = SessionState::Ended,
                    Err(refusal) => {
                        assert_eq!(refusal.state, state, "seed {seed}");
                    }
                }
            }
            // From wherever the walk ended, playing only forward spine events
            // reaches Ended: no state is a dead end before the end.
            let spine = [
                Event::Begin,
                Event::StoryChoice,
                Event::StartFight,
                Event::EndTurn,
                Event::Resolve,
            ];
            let mut guard = 0;
            while state != SessionState::Ended && guard < 16 {
                guard += 1;
                for event in spine {
                    match next(state, event) {
                        Ok(Transition::To(target)) => state = target,
                        Ok(Transition::FightMayEnd) => state = SessionState::Aftermath,
                        Ok(Transition::ResolveMayEnd) => state = SessionState::Ended,
                        Err(_) => {}
                    }
                }
            }
            assert_eq!(state, SessionState::Ended, "seed {seed}");
        }
    }
}
