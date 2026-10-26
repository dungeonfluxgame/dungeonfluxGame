use super::*;
use df_persistence::local_demo_scope::SEAT_AWAY_MILLIS;
use std::collections::BTreeMap;

// Faithful durable-port fixture: checkpoint/hero is independent of credential authority.
// Time is an explicit input and operation receipts survive construction of another owner.
#[derive(Clone)]
struct StoredSeats {
    member: [u8; 16],
    credential: [u8; 32],
    token: [u8; 32],
    last_seen: i64,
    receipts: BTreeMap<[u8; 16], ([u8; 32], LocalReclaimedSeat)>,
}
impl SeatAuthority for StoredSeats {
    fn reclaim(
        &mut self,
        request: LocalSeatReclaim,
    ) -> Result<Result<LocalReclaimedSeat, SeatReclaimRefusal>, RepositoryError> {
        let key = *request.operation.as_bytes();
        if let Some((fingerprint, saved)) = self.receipts.get(&key) {
            if *fingerprint != request.fingerprint {
                return Ok(Err(SeatReclaimRefusal::OperationConflict));
            }
            let mut replay = saved.clone();
            replay.replayed = true;
            return Ok(Ok(replay));
        }
        if request.token != self.token {
            return Ok(Err(SeatReclaimRefusal::UnknownToken));
        }
        if request.now_millis - self.last_seen < SEAT_AWAY_MILLIS {
            return Ok(Err(SeatReclaimRefusal::NotAway));
        }
        let answer = LocalReclaimedSeat {
            member: self.member,
            credential: request.new_credential,
            basis: request.basis,
            replayed: false,
        };
        self.credential = request.new_credential;
        self.token = request.next_token;
        self.last_seen = request.now_millis;
        self.receipts
            .insert(key, (request.fingerprint, answer.clone()));
        Ok(Ok(answer))
    }
}
fn fixture() -> (df_model::checkpoint::Checkpoint, StoredSeats) {
    let initial = journey::initial().unwrap();
    let input = GameInput::Game(CommandInput {
        basis: initial.basis(),
        operation: df_types::OperationId::from_bytes(&[7; 16]).unwrap(),
        member: journey::bootstrap_member().unwrap(),
        observed_revision: initial.basis().revision,
        command: GameCommand::ProposeAction {
            actor: journey::room_entity().unwrap(),
            action: model::content("join-room").unwrap(),
            targets: vec![],
            choices: vec![],
        },
    });
    let joined = journey::stage_join(&initial, &input).unwrap();
    let link = journey::participants(&joined)[0];
    let seats = StoredSeats {
        member: *link.member.as_bytes(),
        credential: [1; 32],
        token: [2; 32],
        last_seen: 1000,
        receipts: BTreeMap::new(),
    };
    (joined, seats)
}
fn request(operation: u8) -> ReclaimRequest {
    ReclaimRequest {
        room_code: "ROOM".to_owned(),
        operation: df_types::OperationId::from_bytes(&[operation; 16]).unwrap(),
        token: [2; 32],
    }
}
fn reclaim(
    seats: &mut StoredSeats,
    checkpoint: &df_model::checkpoint::Checkpoint,
    operation: u8,
    now: i64,
) -> Result<rpc::JoinedRoom, ReclaimRefusal> {
    admit_reclaim(
        seats,
        request(operation),
        true,
        checkpoint,
        now,
        [3; 32],
        [4; 32],
    )
    .unwrap()
}

#[test]
fn reclaim_after_thirty_seconds_preserves_member_hero_and_changes_binding() {
    let (checkpoint, mut seats) = fixture();
    let before = checkpoint.clone();
    let joined = reclaim(&mut seats, &checkpoint, 9, 31_000).unwrap();
    assert_eq!(
        joined.client_binding_id.unwrap().value.unwrap(),
        seats.member
    );
    assert_eq!(joined.local_binding, journey::hex(&[3; 32]));
    assert_ne!(seats.credential, [1; 32]);
    assert_eq!(checkpoint, before);
    assert_eq!(
        journey::participants(&checkpoint)[0].member.as_bytes(),
        &seats.member
    );
}

#[test]
fn reclaim_just_before_thirty_seconds_or_for_active_seat_is_refused() {
    let (checkpoint, mut seats) = fixture();
    for now in [1000, 30_999] {
        assert!(matches!(
            reclaim(&mut seats, &checkpoint, 9, now),
            Err(ReclaimRefusal::Seat(SeatReclaimRefusal::NotAway))
        ));
        assert_eq!(seats.credential, [1; 32]);
    }
    seats.last_seen = 31_000;
    assert!(matches!(
        reclaim(&mut seats, &checkpoint, 9, 31_001),
        Err(ReclaimRefusal::Seat(SeatReclaimRefusal::NotAway))
    ));
}

#[test]
fn reclaim_exact_operation_replay_returns_same_new_binding_after_restart() {
    let (checkpoint, mut seats) = fixture();
    let first = reclaim(&mut seats, &checkpoint, 9, 31_000).unwrap();
    // Reconstruct the owner from retained authority rows, keeping the saved game unchanged.
    let mut restored = seats.clone();
    let replay = reclaim(&mut restored, &checkpoint, 9, 31_001).unwrap();
    assert_eq!(first.local_binding, replay.local_binding);
    assert_eq!(first.client_binding_id, replay.client_binding_id);
    assert_eq!(first.session_id, replay.session_id);
    assert_eq!(first.run_id, replay.run_id);
    assert!(replay.receipt.unwrap().replayed);
    assert_eq!(restored.credential, seats.credential);
}

#[test]
fn serialized_racing_reclaims_allow_one_success_and_refuse_reused_token() {
    let (checkpoint, seats) = fixture();
    let seats = std::sync::Mutex::new(seats);
    let ready = std::sync::Barrier::new(3);
    let answers = std::thread::scope(|threads| {
        let claim = |operation| {
            ready.wait();
            // The actor admits one call at a time, as does the durable session-row lock.
            reclaim(&mut seats.lock().unwrap(), &checkpoint, operation, 31_000)
        };
        let first = threads.spawn(move || claim(9));
        let second = threads.spawn(move || claim(10));
        ready.wait();
        [first.join().unwrap(), second.join().unwrap()]
    });
    assert_eq!(answers.iter().filter(|answer| answer.is_ok()).count(), 1);
    assert_eq!(
        answers
            .iter()
            .filter(|answer| matches!(
                answer,
                Err(ReclaimRefusal::Seat(SeatReclaimRefusal::UnknownToken))
            ))
            .count(),
        1
    );
    let mut seats = seats.into_inner().unwrap();
    assert!(matches!(
        reclaim(&mut seats, &checkpoint, 11, 31_001),
        Err(ReclaimRefusal::Seat(SeatReclaimRefusal::UnknownToken))
    ));
    assert_eq!(seats.receipts.len(), 1);
}

#[test]
fn reclaim_wrong_room_and_changed_retry_do_not_replace_grant() {
    let (checkpoint, mut seats) = fixture();
    let wrong = admit_reclaim(
        &mut seats,
        request(9),
        false,
        &checkpoint,
        31_000,
        [3; 32],
        [4; 32],
    )
    .unwrap();
    assert!(matches!(wrong, Err(ReclaimRefusal::WrongRoomCode)));
    assert!(seats.receipts.is_empty());
    reclaim(&mut seats, &checkpoint, 9, 31_000).unwrap();
    let mut changed = request(9);
    changed.token = [8; 32];
    let changed = admit_reclaim(
        &mut seats,
        changed,
        true,
        &checkpoint,
        31_001,
        [5; 32],
        [6; 32],
    )
    .unwrap();
    assert!(matches!(
        changed,
        Err(ReclaimRefusal::Seat(SeatReclaimRefusal::OperationConflict))
    ));
    assert_eq!(seats.credential, [3; 32]);
}

#[test]
fn seat_clock_uses_controlled_monotonic_elapsed_time() {
    let origin = Instant::now();
    let clock = SeatClock {
        origin,
        unix_millis: Ok(1000),
        next_poll: origin,
        activity: Default::default(),
        rejoins: Default::default(),
    };
    assert_eq!(
        clock
            .millis(origin + std::time::Duration::from_millis(29_999))
            .unwrap(),
        30_999
    );
    assert_eq!(
        clock
            .millis(origin + std::time::Duration::from_secs(30))
            .unwrap(),
        31_000
    );
}

#[test]
fn seat_activity_is_debounced_per_credential_and_reactivation_is_immediate() {
    let origin = Instant::now();
    let mut clock = SeatClock {
        origin,
        unix_millis: Ok(1000),
        next_poll: origin,
        activity: Default::default(),
        rejoins: Default::default(),
    };

    assert!(clock.activity_write_due(&[1; 32], origin));
    assert!(!clock.activity_write_due(&[1; 32], origin + std::time::Duration::from_secs(4)));
    assert!(clock.activity_write_due(&[2; 32], origin + std::time::Duration::from_secs(4)));
    assert!(clock.activity_write_due(&[1; 32], origin + std::time::Duration::from_secs(5)));
    assert!(clock.activity_write_due(&[1; 32], origin + std::time::Duration::from_secs(35)));
    assert!(!clock.activity_write_due(&[1; 32], origin + std::time::Duration::from_secs(36)));
}

#[path = "pg_tests.rs"]
mod pg_tests;

fn away_hero(seats: &StoredSeats) -> AwayHero {
    AwayHero {
        member: seats.member,
        name: "Brannoc".to_owned(),
        token: seats.token,
    }
}

#[test]
fn public_reclaim_listing_never_discloses_tokens_or_proofs() {
    let (_, seats) = fixture();
    let public = listing(vec![away_hero(&seats)]);
    assert_eq!(public[0].name, "Brannoc");
    assert_eq!(public[0].seat_token, seats.member);
    assert!(
        !public[0]
            .encode_to_vec()
            .windows(32)
            .any(|bytes| bytes == seats.token)
    );
}

type Asks = std::collections::HashMap<[u8; 16], RejoinAsk>;

fn ask_rejoin(
    seats: &mut StoredSeats,
    asks: &mut Asks,
    checkpoint: &df_model::checkpoint::Checkpoint,
    member: [u8; 16],
    operation: u8,
    at: (Instant, i64),
    away: &[AwayHero],
) -> Result<rpc::JoinedRoom, ReclaimRefusal> {
    let (instant, now) = at;
    let request = ReclaimRequest {
        room_code: "ROOM".to_owned(),
        operation: df_types::OperationId::from_bytes(&[operation; 16]).unwrap(),
        token: rejoin_request_token(&member),
    };
    rejoin(
        seats, asks, request, true, checkpoint, instant, now, away, [3; 32], [4; 32],
    )
    .unwrap()
}

#[test]
fn rejoin_waits_for_the_host_then_allow_moves_the_seat_and_replays() {
    let (checkpoint, mut seats) = fixture();
    let away = vec![away_hero(&seats)];
    let member = seats.member;
    let origin = Instant::now();
    let mut asks = Asks::new();
    // The ask alone moves nothing; the phone is told it is waiting.
    assert!(matches!(
        ask_rejoin(
            &mut seats,
            &mut asks,
            &checkpoint,
            member,
            9,
            (origin, 31_000),
            &away
        ),
        Err(ReclaimRefusal::RejoinPending)
    ));
    assert_eq!(seats.credential, [1; 32]);
    // A second device asking for the same hero does not replace the ask.
    assert!(matches!(
        ask_rejoin(
            &mut seats,
            &mut asks,
            &checkpoint,
            member,
            10,
            (origin, 31_000),
            &away
        ),
        Err(ReclaimRefusal::RejoinPending)
    ));
    assert_eq!(asks.len(), 1);
    // Allow hands this device the seat, rotating the credential.
    assert_eq!(
        decide_rejoin(&mut asks, &decision_token(&seats.member, true), origin),
        ReclaimRefusal::RejoinDecided
    );
    let joined = ask_rejoin(
        &mut seats,
        &mut asks,
        &checkpoint,
        member,
        9,
        (origin, 31_000),
        &away,
    )
    .unwrap();
    assert_eq!(
        joined.client_binding_id.unwrap().value.unwrap(),
        seats.member
    );
    assert_eq!(joined.local_binding, journey::hex(&[3; 32]));
    assert_ne!(seats.credential, [1; 32]);
    // The old credential's token no longer takes the seat.
    assert!(matches!(
        reclaim(&mut seats, &checkpoint, 11, 31_001),
        Err(ReclaimRefusal::Seat(SeatReclaimRefusal::UnknownToken))
    ));
    // A lost reply replays the same grant for the same operation.
    let replay = ask_rejoin(
        &mut seats,
        &mut asks,
        &checkpoint,
        member,
        9,
        (origin, 31_001),
        &[],
    )
    .unwrap();
    assert_eq!(replay.local_binding, joined.local_binding);
    assert!(replay.receipt.unwrap().replayed);
}

#[test]
fn rejoin_decline_and_expiry_refuse_the_phone_plainly() {
    let (checkpoint, mut seats) = fixture();
    let away = vec![away_hero(&seats)];
    let member = seats.member;
    let origin = Instant::now();
    let mut asks = Asks::new();
    ask_rejoin(
        &mut seats,
        &mut asks,
        &checkpoint,
        member,
        9,
        (origin, 31_000),
        &away,
    )
    .err()
    .unwrap();
    assert_eq!(
        decide_rejoin(&mut asks, &decision_token(&seats.member, false), origin),
        ReclaimRefusal::RejoinDecided
    );
    assert!(matches!(
        ask_rejoin(
            &mut seats,
            &mut asks,
            &checkpoint,
            member,
            9,
            (origin, 31_000),
            &away
        ),
        Err(ReclaimRefusal::RejoinDeclined)
    ));
    assert_eq!(seats.credential, [1; 32]);
    // An ask nobody answers expires after about five minutes.
    let mut asks = Asks::new();
    ask_rejoin(
        &mut seats,
        &mut asks,
        &checkpoint,
        member,
        9,
        (origin, 31_000),
        &away,
    )
    .err()
    .unwrap();
    let later = origin + REJOIN_ASK_TTL + Duration::from_secs(1);
    assert!(matches!(
        ask_rejoin(
            &mut seats,
            &mut asks,
            &checkpoint,
            member,
            9,
            (later, 31_000),
            &away
        ),
        Err(ReclaimRefusal::RejoinDeclined)
    ));
    // After expiry a fresh ask can be made again.
    assert!(matches!(
        ask_rejoin(
            &mut seats,
            &mut asks,
            &checkpoint,
            member,
            10,
            (later, 31_000),
            &away
        ),
        Err(ReclaimRefusal::RejoinPending)
    ));
    assert_eq!(seats.credential, [1; 32]);
}

#[test]
fn a_brand_new_person_cannot_take_a_seat_without_host_approval() {
    let (checkpoint, mut seats) = fixture();
    let away = vec![away_hero(&seats)];
    let member = seats.member;
    let origin = Instant::now();
    let mut asks = Asks::new();
    // An invented member id is refused and records nothing.
    assert!(matches!(
        ask_rejoin(
            &mut seats,
            &mut asks,
            &checkpoint,
            [9; 16],
            9,
            (origin, 31_000),
            &away
        ),
        Err(ReclaimRefusal::Seat(SeatReclaimRefusal::UnknownToken))
    ));
    assert!(asks.is_empty());
    // A hero who is not away (active phone) cannot be asked for at all.
    assert!(matches!(
        ask_rejoin(
            &mut seats,
            &mut asks,
            &checkpoint,
            member,
            9,
            (origin, 31_000),
            &[]
        ),
        Err(ReclaimRefusal::Seat(SeatReclaimRefusal::UnknownToken))
    ));
    // Presenting the public member id as a direct reclaim token also fails.
    let public = listing(vec![away_hero(&seats)]);
    let mut forged = [0; 32];
    forged[..16].copy_from_slice(&public[0].seat_token);
    let mut forged_request = request(9);
    forged_request.token = forged;
    assert!(matches!(
        admit_reclaim(
            &mut seats,
            forged_request,
            true,
            &checkpoint,
            31_000,
            [3; 32],
            [4; 32]
        )
        .unwrap(),
        Err(ReclaimRefusal::Seat(SeatReclaimRefusal::UnknownToken))
    ));
    assert_eq!(seats.credential, [1; 32]);
    assert!(seats.receipts.is_empty());
}

#[test]
fn host_allowed_rejoin_preserves_the_current_hero_and_adventure_in_every_phase() {
    use crate::gameplay::beat_script_variety_tests as native;
    let room = native::prepared();
    let opening = native::act(&room, native::first(), 5, "begin-story");
    let dialogue = native::act(&opening, native::first(), 6, "escort-courier");
    let combat = native::act(&dialogue, native::first(), 7, "defend-courier");
    for (checkpoint, phase) in [
        (room, rpc::JourneyPhase::Room),
        (opening, rpc::JourneyPhase::Opening),
        (dialogue, rpc::JourneyPhase::Dialogue),
        (combat, rpc::JourneyPhase::Combat),
    ] {
        assert_eq!(journey::phase(&checkpoint).unwrap(), phase);
        let before = checkpoint.clone();
        let (_, mut seats) = fixture();
        let away = vec![away_hero(&seats)];
        let member = seats.member;
        let origin = Instant::now();
        let mut asks = Asks::new();
        assert!(matches!(
            ask_rejoin(
                &mut seats,
                &mut asks,
                &checkpoint,
                member,
                9,
                (origin, 31_000),
                &away
            ),
            Err(ReclaimRefusal::RejoinPending)
        ));
        assert_eq!(
            decide_rejoin(&mut asks, &decision_token(&seats.member, true), origin),
            ReclaimRefusal::RejoinDecided
        );
        let joined = ask_rejoin(
            &mut seats,
            &mut asks,
            &checkpoint,
            member,
            9,
            (origin, 31_000),
            &away,
        )
        .unwrap();
        assert_eq!(
            joined.client_binding_id.unwrap().value.unwrap(),
            native::first().as_bytes()
        );
        assert_eq!(checkpoint, before);
        let resumed = wire::journey_view(
            &checkpoint,
            df_persistence::local_demo_scope::LocalDemoRole::Player,
            native::first(),
        )
        .unwrap();
        let rpc::view_message::Audience::Player(player) = resumed.audience.unwrap() else {
            panic!("player view");
        };
        let view = player.journey.unwrap();
        assert_eq!(view.phase, phase as i32);
        assert!(!view.own_character.unwrap().name.is_empty());
        if phase != rpc::JourneyPhase::Room {
            let input = crate::gameplay::beat_script_tests::input(
                &checkpoint,
                journey::bootstrap_member().unwrap(),
                40,
                "join-room",
                vec![],
            );
            assert!(journey::stage_join(&checkpoint, &input).is_err());
        }
        assert_eq!(
            joined.session_id.unwrap().value.unwrap(),
            checkpoint.basis().session.as_bytes()
        );
        assert_eq!(
            joined.run_id.unwrap().value.unwrap(),
            checkpoint.basis().run.as_bytes()
        );
    }
}
