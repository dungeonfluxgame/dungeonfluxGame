//! The session's one forward state machine. See
//! `artifacts/tmp/campaign/flow-a2/STATE-MACHINE.md`.
//!
//! The state is derived from the checkpoint (never stored), every command is
//! checked against the table in `next` before any handler runs in
//! `stage_using`, and the table only ever moves forward. The fight is a
//! sub-machine: its turn order and dying/death-save facts live in the
//! encounter, and it is entered (`StartFight`) and left (a combat event whose
//! rules outcome ends it) only through this table.
//!
//! The executors `stage_using` calls once an edge is accepted, one per
//! transition into or out of the sub-machines: `enter_fight` (the only
//! production caller of `tactical::start` / `start_combat`),
//! `enter_conversation` (the only staging caller of the conversation
//! narrative transition), and inside fight resolution `finish_if_done` /
//! `finish_combat` for the exit. After they run, `outcome_legal` verifies
//! the candidate landed where the transition named. Offers are derived
//! from the same table (`kind_accepted`), and the Aftermath rest rule is
//! decided here too (`rest_accepted`).

use df_model::checkpoint::Checkpoint;
use df_protocol::common as rpc;
use df_session::submission::RepositoryError;

use super::phase;
use crate::gameplay::beat_script::BeatEffect;

/// Where a session is, from lobby to the end card.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionState {
    Lobby,
    Story,
    Conversation,
    Fight,
    Aftermath,
    Ended,
    Preparing,
}

impl SessionState {
    /// The state of a checkpoint: its journey phase, with the end card split
    /// out of `Complete`.
    pub(crate) fn of(current: &Checkpoint) -> Result<Self, RepositoryError> {
        let phase = phase(current)?;
        if super::new_adventure_requested(current) {
            return Ok(Self::Preparing);
        }
        Ok(match phase {
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
            Self::Preparing => 6,
        }
    }
}

/// What a submitted command asks the session to do, named by progress.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Event {
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
    NewAdventure,
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
            BeatEffect::Observe => Self::ScriptStep,
            BeatEffect::Resolve => Self::Resolve,
            BeatEffect::DefendCourier => Self::StartFight,
            BeatEffect::ShortRest => Self::Rest,
            BeatEffect::EndTurn => Self::EndTurn,
            BeatEffect::AskCourier
            | BeatEffect::EscortCourier
            | BeatEffect::ThirdApproach
            | BeatEffect::ChooseHarborScene => Self::StoryChoice,
            BeatEffect::GreatswordAttack | BeatEffect::SecondWind => Self::CombatAction,
        }
    }

    /// The machine event represented by an action kind in this state. The
    /// BeginStory kind is shared by the lobby's Begin offer and authored
    /// script Continue steps; the event list keeps that distinction ordered.
    pub(crate) fn for_kind(state: SessionState, kind: rpc::GameplayActionKind) -> Option<Self> {
        events_for_kind(kind)
            .iter()
            .copied()
            .find(|event| next(state, *event).is_ok())
            .or_else(|| events_for_kind(kind).first().copied())
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

/// Every event, so the accepted set of a state is derived from `next` and
/// never restated.
pub(super) const EVENTS: [Event; 10] = [
    Event::CreateHero,
    Event::Begin,
    Event::StoryChoice,
    Event::ScriptStep,
    Event::StartFight,
    Event::CombatAction,
    Event::EndTurn,
    Event::Rest,
    Event::Resolve,
    Event::NewAdventure,
];

/// The events `next` accepts from `state`.
pub(super) fn accepted_events(state: SessionState) -> impl Iterator<Item = Event> {
    EVENTS
        .into_iter()
        .filter(move |event| next(state, *event).is_ok())
}

/// The events an offered action kind can mean, mirroring `Event::of` over
/// every effect whose `BeatEffect::action` is `kind`, plus the tactical
/// kinds selection maps onto the fight beat's attack offer. Empty for kinds
/// that are never journey offers (table settings, unspecified).
pub(super) fn events_for_kind(kind: rpc::GameplayActionKind) -> &'static [Event] {
    use rpc::GameplayActionKind as K;
    match kind {
        K::CreateCharacter => &[Event::CreateHero],
        // A pure authored Continue step is also submitted as BeginStory.
        K::BeginStory => &[Event::Begin, Event::ScriptStep],
        K::ExamineHarborSeal => &[Event::ScriptStep, Event::StoryChoice],
        K::AskCourier | K::EscortCourier | K::ChooseHarborScene => &[Event::StoryChoice],
        K::DefendCourier => &[Event::StartFight],
        K::GreatswordAttack
        | K::SecondWind
        | K::Move
        | K::Dash
        | K::Dodge
        | K::Disengage
        | K::OpportunityAttack
        | K::DeclineReaction
        | K::Stand
        | K::PickUpWeapon => &[Event::CombatAction],
        K::EndTurn => &[Event::EndTurn],
        K::ShortRest => &[Event::Rest],
        K::ResolveErrand => &[Event::Resolve],
        K::StartNewAdventure => &[Event::NewAdventure],
        _ => &[],
    }
}

/// Whether an offer of `kind` may be built in `state`: some event the kind
/// can mean is accepted there. This is the one place offer kinds are
/// decided; handlers keep building labels and inputs for the kinds it
/// allows.
pub(super) fn kind_accepted(state: SessionState, kind: rpc::GameplayActionKind) -> bool {
    events_for_kind(kind)
        .iter()
        .any(|event| next(state, *event).is_ok())
}

/// Why an accepted event may have no offer on a given screen, for the pairs
/// whose offers are not the handler's to build. `None` means the event is
/// expected to reach the player as an ordinary offer somewhere in its
/// state.
pub(super) fn internal_reason(state: SessionState, event: Event) -> Option<&'static str> {
    use Event as E;
    use SessionState as S;
    match (state, event) {
        // Pure authored steps are offered by the loaded script's own beats
        // (its validated forward edges), never by a handler.
        (_, E::ScriptStep) => Some("offered by the loaded script's beats, not a handler"),
        // A script whose fight follows the opening directly offers the
        // fight from Story; the canonical journey reaches it by talking.
        (S::Story, E::StartFight) => {
            Some("offered by scripts whose fight follows the opening directly")
        }
        // Conversation choices are the script's conversation-beat offers
        // and typed dialogue lines, not handler offers.
        (S::Conversation, E::StoryChoice) => {
            Some("offered by the script's conversation beats and typed dialogue")
        }
        _ => None,
    }
}

/// The Aftermath rest rule, decided here like every other transition: a
/// short rest is accepted at most once per aftermath (per fight), and only
/// while some living hero has something to regain: hit points below the
/// maximum, a Second Wind use below the maximum, or a knockout to wake
/// from. (The rest itself currently restores Second Wind and wakes the
/// knocked out; a Hit Point Die choice can remain pending in the aftermath.)
/// Derived from the checkpoint like the state itself, so a stored game that
/// resumes during those choices can continue with the unresolved hero.
pub(super) fn rest_accepted(current: &Checkpoint) -> Result<bool, RepositoryError> {
    if SessionState::of(current)? != SessionState::Aftermath {
        return Ok(false);
    }
    let state = current.state();
    if super::short_rest_started(state) {
        return if super::short_rest_completed(state) {
            Ok(false)
        } else {
            super::pending_hit_point_die_choices(state)
        };
    }
    for character in &state.characters {
        let who = character.entity;
        let hit_points = super::value(state, who, "hit-points")?;
        if hit_points > 0
            && ((hit_points < super::maximum_hit_points(state, who)?
                && super::value(state, who, "hit-point-dice")? > 0)
                || super::value(state, who, "second-wind")?
                    < u32::from(super::super::pack::second_wind().uses_max)
                || super::value(state, who, "unconscious")? != 0)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether a staged candidate landed where the transition said it would.
/// Checked in `stage_using` after the executors run, so no handler can move
/// the session somewhere the table did not name: the fight is entered only
/// into `Fight` and left only into `Aftermath`, a resolution ends only in
/// `Aftermath` or `Ended`, and nothing ever moves backwards. Pure script
/// steps are the loaded script's own forward edges, so for them only the
/// forward rank is checked.
pub(super) fn outcome_legal(
    before: SessionState,
    event: Event,
    transition: Transition,
    after: SessionState,
) -> bool {
    if after.rank() < before.rank() {
        return false;
    }
    if event == Event::StartFight && after != SessionState::Fight {
        return false;
    }
    // And the fight is entered by no other event, however far a script
    // edge travels.
    if after == SessionState::Fight && before != SessionState::Fight && event != Event::StartFight {
        return false;
    }
    match transition {
        // Story choices and pure script steps follow the loaded script's
        // own validated forward edges: they may stay, reach the named
        // state, or move further forward along the script (a conversation
        // choice that travels to its destination's aftermath). The rank
        // check above still forbids any backward move.
        Transition::To(target) => {
            after == target || matches!(event, Event::StoryChoice | Event::ScriptStep)
        }
        Transition::FightMayEnd => {
            matches!(after, SessionState::Fight | SessionState::Aftermath)
        }
        Transition::ResolveMayEnd => {
            matches!(after, SessionState::Aftermath | SessionState::Ended)
        }
    }
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
        (S::Ended, E::NewAdventure) => Transition::To(S::Preparing),
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

    const STATES: [SessionState; 7] = [
        SessionState::Lobby,
        SessionState::Story,
        SessionState::Conversation,
        SessionState::Fight,
        SessionState::Aftermath,
        SessionState::Ended,
        SessionState::Preparing,
    ];
    // `EVENTS` comes from the module: the table and the tests share it.

    #[test]
    fn new_adventure_is_an_ended_only_machine_edge() {
        let kind = rpc::GameplayActionKind::try_from(21).expect("new adventure wire kind");
        let event = Event::for_kind(SessionState::Ended, kind).expect("restart event");
        assert!(next(SessionState::Ended, event).is_ok());
        for state in STATES
            .into_iter()
            .filter(|state| *state != SessionState::Ended)
        {
            assert!(next(state, event).is_err(), "{state:?}");
        }
    }

    #[test]
    fn audit_event_distinguishes_begin_story_from_script_continue() {
        assert_eq!(
            Event::for_kind(SessionState::Lobby, rpc::GameplayActionKind::BeginStory),
            Some(Event::Begin)
        );
        assert_eq!(
            Event::for_kind(SessionState::Story, rpc::GameplayActionKind::BeginStory),
            Some(Event::ScriptStep)
        );
    }

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
            assert_eq!(
                next(SessionState::Ended, event).is_ok(),
                event == Event::NewAdventure
            );
            assert!(
                next(SessionState::Fight, event).is_err()
                    || matches!(event, Event::CombatAction | Event::EndTurn),
                "only combat events are legal in a fight: {event:?}"
            );
        }
    }

    #[test]
    fn outcomes_must_land_where_the_transition_named() {
        use Event as E;
        use SessionState as S;
        // The fight is entered only into Fight and left only into Aftermath.
        assert!(outcome_legal(
            S::Conversation,
            E::StartFight,
            Transition::To(S::Fight),
            S::Fight
        ));
        assert!(!outcome_legal(
            S::Conversation,
            E::StartFight,
            Transition::To(S::Fight),
            S::Aftermath
        ));
        assert!(outcome_legal(
            S::Fight,
            E::CombatAction,
            Transition::FightMayEnd,
            S::Aftermath
        ));
        assert!(!outcome_legal(
            S::Fight,
            E::CombatAction,
            Transition::FightMayEnd,
            S::Ended
        ));
        assert!(!outcome_legal(
            S::Fight,
            E::EndTurn,
            Transition::FightMayEnd,
            S::Conversation
        ));
        // A conversation is entered only where StoryChoice names it, and a
        // resolution ends only in Aftermath or Ended.
        assert!(outcome_legal(
            S::Story,
            E::StoryChoice,
            Transition::To(S::Conversation),
            S::Conversation
        ));
        // A story choice never lands in a fight: only StartFight enters one.
        assert!(!outcome_legal(
            S::Story,
            E::StoryChoice,
            Transition::To(S::Conversation),
            S::Fight
        ));
        // But a look around the opening scene may stay in Story, and a
        // conversation choice may travel on to its aftermath.
        assert!(outcome_legal(
            S::Story,
            E::StoryChoice,
            Transition::To(S::Conversation),
            S::Story
        ));
        assert!(outcome_legal(
            S::Conversation,
            E::StoryChoice,
            Transition::To(S::Conversation),
            S::Aftermath
        ));
        assert!(outcome_legal(
            S::Aftermath,
            E::Resolve,
            Transition::ResolveMayEnd,
            S::Ended
        ));
        // Nothing moves backwards, script steps included.
        assert!(!outcome_legal(
            S::Aftermath,
            E::ScriptStep,
            Transition::To(S::Aftermath),
            S::Story
        ));
    }

    #[test]
    fn every_effects_kind_carries_its_event() {
        // The kind->events map offer building uses must agree with the
        // effect->event map command admission uses, for every effect.
        let effects = [
            BeatEffect::CreateCharacter,
            BeatEffect::BeginStory,
            BeatEffect::AskCourier,
            BeatEffect::EscortCourier,
            BeatEffect::DefendCourier,
            BeatEffect::GreatswordAttack,
            BeatEffect::SecondWind,
            BeatEffect::EndTurn,
            BeatEffect::ShortRest,
            BeatEffect::ChooseHarborScene,
            BeatEffect::ThirdApproach,
            BeatEffect::Observe,
            BeatEffect::Resolve,
            BeatEffect::Continue,
        ];
        for effect in effects {
            assert!(
                events_for_kind(effect.action()).contains(&Event::of(effect)),
                "{effect:?}"
            );
        }
        // And the accepted set is exactly the table's Ok cells.
        for state in STATES {
            let accepted: Vec<_> = accepted_events(state).collect();
            for event in EVENTS {
                assert_eq!(
                    accepted.contains(&event),
                    next(state, event).is_ok(),
                    "{state:?} {event:?}"
                );
            }
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
