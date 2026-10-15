//! Private operator proof through the real generated service in two real native processes.
//! Phase files live only in the Root-owned protected scratch directory; never RPC input.
use super::{Service, actor, hex, journey, wire};
use df_model::checkpoint::{Checkpoint, EntityId, GameState};
use df_persistence::local_demo_scope::{self, DemoInitializationError};
use df_protocol::common as rpc;
use futures::StreamExt;
use prost::Message;
use std::{
    io,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{sync::oneshot, time::timeout};
use tokio_postgres::{Client, Config, NoTls};
use tonic::Request;

type Error = Box<dyn std::error::Error + Send + Sync>;
fn source<T>(result: Result<T, df_session::submission::RepositoryError>) -> Result<T, io::Error> {
    result.map_err(|_| io::Error::other("rest canonical source invalid"))
}
fn resource_value(state: &GameState, entity: EntityId, key: &str) -> Result<u32, io::Error> {
    source(journey::value(state, entity, key))
}
fn victory(state: &GameState) -> Result<bool, io::Error> {
    source(journey::combat_victory(state))
}
fn current_phase(checkpoint: &Checkpoint) -> Result<rpc::JourneyPhase, io::Error> {
    source(journey::phase(checkpoint))
}
fn entity(bytes: [u8; 16]) -> Result<EntityId, io::Error> {
    source(journey::entity(bytes))
}
const ROOT: &str = "/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/engine-rest-20261005";
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Phase {
    A,
    B,
    Contender,
}
pub(super) fn phase() -> Option<Phase> {
    match std::env::args().nth(4).as_deref() {
        Some("--rest-qualification-a") => Some(Phase::A),
        Some("--rest-qualification-b") => Some(Phase::B),
        Some("--rest-qualification-contender") => Some(Phase::Contender),
        _ => None,
    }
}
#[track_caller]
fn required(value: bool) -> Result<(), io::Error> {
    if value {
        Ok(())
    } else {
        let at = std::panic::Location::caller();
        Err(io::Error::other(format!(
            "rest assertion {}:{}",
            at.file(),
            at.line()
        )))
    }
}
fn now() -> Result<u64, io::Error> {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_millis(),
    )
    .map_err(io::Error::other)
}
fn elapsed(start: u64) -> Result<u64, io::Error> {
    now()?
        .checked_sub(start)
        .ok_or_else(|| io::Error::other("rest clock regressed"))
}
fn path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(ROOT).join(name)
}
fn save(name: &str, bytes: &[u8]) -> Result<(), io::Error> {
    use std::io::Write;
    required(bytes.len() <= 4 * 1024 * 1024)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path(name))?;
    file.write_all(bytes)?;
    file.sync_all()
}
fn read(name: &str, maximum: u64) -> Result<Vec<u8>, io::Error> {
    let metadata = std::fs::symlink_metadata(path(name))?;
    required(metadata.is_file() && metadata.len() <= maximum)?;
    std::fs::read(path(name))
}
fn report(name: &str, body: &str) -> Result<(), io::Error> {
    required(body.len() <= 16 * 1024 && !path(name).exists())?;
    let temporary = format!("{name}.pending");
    save(&temporary, body.as_bytes())?;
    std::fs::rename(path(&temporary), path(name))
}
fn configured() -> Result<(), io::Error> {
    required(
        std::env::var("DF_REST_QUALIFICATION").as_deref() == Ok("owned-loopback-rest01")
            && std::env::var("DF_GAMEPLAY_DEMO_DATABASE").as_deref()
                == Ok("df_gameplay_demo_20261005_rest01"),
    )
}
fn metadata() -> Result<(u64, usize, usize), io::Error> {
    let bytes = read("rest-private-metadata-01.bin", 11)?;
    required(bytes.len() == 11)?;
    Ok((
        u64::from_be_bytes(bytes[..8].try_into().map_err(io::Error::other)?),
        usize::from(u16::from_be_bytes(
            bytes[8..10].try_into().map_err(io::Error::other)?,
        )),
        usize::from(bytes[10]),
    ))
}
pub(super) fn call_budget(phase: Phase) -> Result<u16, io::Error> {
    configured()?;
    if phase != Phase::B {
        return Ok(32);
    }
    let (start, calls, index) = metadata()?;
    required(calls <= 19 && index >= 0xa8 && elapsed(start)? <= 45_000)?;
    required(read("rest-private-build-01.bin", 96)? == crate::BUILD_ID.as_bytes())?;
    Ok(32 - calls as u16)
}
pub(super) fn record_owner(phase: Phase, fence: [u8; 16]) -> Result<(), io::Error> {
    if phase == Phase::A {
        save("rest-private-old-fence-01.bin", &fence)?;
    }
    Ok(())
}
async fn stale_release(client: &Client, checkpoint: &Checkpoint) -> Result<(), Error> {
    let fence: [u8; 16] = read("rest-private-old-fence-01.bin", 16)?
        .try_into()
        .map_err(|_| io::Error::other("old fence length"))?;
    let result = local_demo_scope::release_owner(client, checkpoint.basis().session, fence).await;
    required(result == Err(df_session::submission::RepositoryError::RevisionConflict))?;
    let row=client.query_one("SELECT owner_fence,lease_until>clock_timestamp() AS live FROM df_game.sessions WHERE tenant_id=$1::bytea AND session_id=$2::bytea", &[&local_demo_scope::TENANT.as_slice(),&checkpoint.basis().session.as_bytes().as_slice()]).await?;
    let replacement: Vec<u8> = row.try_get("owner_fence")?;
    required(replacement.len() == 16 && replacement != fence && row.try_get::<_, bool>("live")?)?;
    Ok(())
}
pub(super) fn refusal(error: &DemoInitializationError) -> Result<(), io::Error> {
    configured()?;
    required(
        error.stage == "live_owner"
            && error.class == df_session::submission::RepositoryError::RevisionConflict,
    )?;
    report(
        "rest-contender-refused-01.json",
        "{\"pass\":true,\"live_owner_refused\":true,\"listener_bound\":false,\"actor_calls\":0,\"stage\":\"live_owner\",\"class\":\"RevisionConflict\"}",
    )
}
struct Calls {
    count: usize,
    started_milliseconds: u64,
}
fn admit(calls: &mut Calls) -> Result<(), Error> {
    required(calls.count < 32 && elapsed(calls.started_milliseconds)? <= 45_000)?;
    calls.count = calls
        .count
        .checked_add(1)
        .ok_or_else(|| io::Error::other("fixture call overflow"))?;
    Ok(())
}
async fn join(service: &Service, operation: u8, calls: &mut Calls) -> Result<[u8; 32], Error> {
    admit(calls)?;
    let body = rpc::JoinRoomRequest {
        room_code: journey::ROOM_CODE.to_owned(),
        join_secret: vec![operation; 32],
        operation_id: Some(rpc::OperationId {
            value: Some(vec![operation; 16]),
        }),
    };
    let decoded = rpc::JoinRoomRequest::decode(body.encode_to_vec().as_slice())?;
    let response = rpc::room_service_server::RoomService::join(service, Request::new(decoded))
        .await?
        .into_inner();
    let Some(rpc::join_room_response::Outcome::Joined(joined)) = response.outcome else {
        return Err(io::Error::other("recovery real room join refused").into());
    };
    let bytes = joined.local_binding.as_bytes();
    required(bytes.len() == 64)?;
    let mut credential = [0; 32];
    for (index, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
        let digit = |b: u8| match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            _ => None,
        };
        credential[index] =
            digit(pair[0]).ok_or_else(|| io::Error::other("join grant encoding"))? * 16
                + digit(pair[1]).ok_or_else(|| io::Error::other("join grant encoding"))?;
    }
    Ok(credential)
}
fn create(checkpoint: &Checkpoint, operation: u8) -> rpc::SubmitActionRequest {
    let mut body = action(
        checkpoint,
        operation,
        rpc::GameplayActionKind::CreateCharacter,
    );
    body.character = Some(rpc::CharacterSelection {
        identity: None,
        name: "Rest-Fighter".to_owned(),
        choices: [
            ("species", "dwarf"),
            ("class", "fighter-1"),
            ("background", "soldier"),
            ("array", "stalwart"),
            ("alignment", "lawful-good"),
            ("language-1", "dwarvish"),
            ("language-2", "elvish"),
            ("skill-1", "perception"),
            ("skill-2", "survival"),
            ("gaming-set", "dice"),
            ("equipment", "fighter-a-soldier-a"),
            ("style-masteries", "defense-greatsword-flail-javelin"),
            ("allied-ties", "join-order"),
        ]
        .into_iter()
        .map(|(group, option)| rpc::JourneyChoice {
            group_id: group.to_owned(),
            option_id: option.to_owned(),
        })
        .collect(),
    });
    body
}
fn action(
    checkpoint: &Checkpoint,
    operation: u8,
    kind: rpc::GameplayActionKind,
) -> rpc::SubmitActionRequest {
    let basis = checkpoint.basis();
    rpc::SubmitActionRequest {
        session_id: Some(rpc::SessionId {
            value: Some(basis.session.as_bytes().to_vec()),
        }),
        run_id: Some(rpc::RunId {
            value: Some(basis.run.as_bytes().to_vec()),
        }),
        operation_id: Some(rpc::OperationId {
            value: Some(vec![operation; 16]),
        }),
        observed_revision: Some(wire::revision(basis.revision)),
        offer_id: journey::offer_id(checkpoint, kind),
        action_kind: kind as i32,
        ..Default::default()
    }
}
async fn submit(
    service: &Service,
    body: &rpc::SubmitActionRequest,
    credential: [u8; 32],
    calls: &mut Calls,
) -> Result<rpc::SubmitActionResponse, tonic::Status> {
    admit(calls).map_err(|_| tonic::Status::resource_exhausted("fixture call bound"))?;
    let decoded = rpc::SubmitActionRequest::decode(body.encode_to_vec().as_slice())
        .map_err(|_| tonic::Status::internal("fixture encoding"))?;
    let mut request = Request::new(decoded);
    request.metadata_mut().insert(
        "x-df-local-binding",
        hex(&credential)
            .parse()
            .map_err(|_| tonic::Status::internal("fixture metadata"))?,
    );
    rpc::action_service_server::ActionService::submit(service, request)
        .await
        .map(|value| value.into_inner())
}
fn committed(value: rpc::SubmitActionResponse) -> Result<rpc::DecisionReceipt, Error> {
    match value.outcome {
        Some(rpc::submit_action_response::Outcome::CommittedDecision(value))
            if matches!(
                value.outcome,
                Some(rpc::decision_receipt::Outcome::Accepted(_))
            ) =>
        {
            Ok(*value)
        }
        _ => Err(io::Error::other("fixture receipt absent").into()),
    }
}
async fn snapshot(
    service: &Service,
    calls: &mut Calls,
) -> Result<actor::QualificationSnapshot, Error> {
    admit(calls)?;
    let (reply, wait) = oneshot::channel();
    service
        .actor
        .try_submit(actor::Call::QualificationInputs { reply })
        .map_err(|_| io::Error::other("fixture snapshot admission"))?;
    Ok(timeout(Duration::from_secs(5), wait).await???)
}
#[derive(Eq, PartialEq)]
struct Durable {
    counts: [i64; 4],
    operations: Vec<String>,
    facts: Vec<String>,
    intents: Vec<String>,
    envelope: Vec<u8>,
    receipt: Vec<u8>,
}
async fn durable(client: &Client, operation: u8) -> Result<Durable, Error> {
    let row=client.query_one("SELECT (SELECT count(*) FROM df_game.operations) AS operations,(SELECT count(*) FROM df_game.facts) AS facts,(SELECT count(*) FROM df_game.checkpoints) AS checkpoints,(SELECT count(*) FROM df_game.intents) AS intents,CASE WHEN octet_length(c.complete_envelope)<=1048576 THEN c.complete_envelope END AS envelope,CASE WHEN octet_length(o.receipt)<=4096 THEN o.receipt END AS receipt FROM df_game.sessions s JOIN df_game.checkpoints c ON (c.tenant_id,c.session_id,c.recovery_epoch,c.in_epoch_sequence)=(s.tenant_id,s.session_id,s.recovery_epoch,s.in_epoch_sequence) JOIN df_game.operations o ON o.tenant_id=s.tenant_id AND o.session_id=s.session_id WHERE s.tenant_id=$1 AND s.session_id=$2 AND o.operation_id=$3",&[&local_demo_scope::TENANT.as_slice(),&[0x41_u8;16].as_slice(),&[operation;16].as_slice()]).await?;
    Ok(Durable {
        counts: [
            row.try_get("operations")?,
            row.try_get("facts")?,
            row.try_get("checkpoints")?,
            row.try_get("intents")?,
        ],
        operations: physical_rows(client, "SELECT CASE WHEN octet_length(body)<=262144 AND sum(octet_length(body)) OVER ()<=1048576 THEN body END FROM (SELECT row_to_json(t)::text AS body,row_number() OVER (ORDER BY tenant_id,session_id,principal_id,command_namespace,recovery_epoch,operation_id) AS row_ordinal FROM df_game.operations t ORDER BY tenant_id,session_id,principal_id,command_namespace,recovery_epoch,operation_id LIMIT 257) bounded ORDER BY row_ordinal").await?,
        facts: physical_rows(client, "SELECT CASE WHEN octet_length(body)<=262144 AND sum(octet_length(body)) OVER ()<=1048576 THEN body END FROM (SELECT row_to_json(t)::text AS body,row_number() OVER (ORDER BY tenant_id,session_id,committed_epoch,committed_sequence,ordinal) AS row_ordinal FROM df_game.facts t ORDER BY tenant_id,session_id,committed_epoch,committed_sequence,ordinal LIMIT 257) bounded ORDER BY row_ordinal").await?,
        intents: physical_rows(client, "SELECT CASE WHEN octet_length(body)<=262144 AND sum(octet_length(body)) OVER ()<=1048576 THEN body END FROM (SELECT row_to_json(t)::text AS body,row_number() OVER (ORDER BY tenant_id,session_id,principal_id,command_namespace,recovery_epoch,operation_id,slot) AS row_ordinal FROM df_game.intents t ORDER BY tenant_id,session_id,principal_id,command_namespace,recovery_epoch,operation_id,slot LIMIT 257) bounded ORDER BY row_ordinal").await?,
        envelope: row.try_get("envelope")?,
        receipt: row.try_get("receipt")?,
    })
}
// Compare complete physical rows, including encoded payload bytes, without logging them.
// The SQL caps returned bytes before transfer; the extra row detects row-cap overflow.
async fn physical_rows(client: &Client, sql: &'static str) -> Result<Vec<String>, Error> {
    let rows = client.query(sql, &[]).await?;
    required(rows.len() <= 256)?;
    rows.into_iter()
        .map(|row| row.try_get(0).map_err(Into::into))
        .collect()
}

fn bind<T>(value: T, credential: [u8; 32]) -> Result<Request<T>, Error> {
    let mut request = Request::new(value);
    request
        .metadata_mut()
        .insert("x-df-local-binding", hex(&credential).parse()?);
    Ok(request)
}
async fn first_watch(
    service: &Service,
    credential: [u8; 32],
    member: [u8; 16],
    calls: &mut Calls,
) -> Result<Vec<u8>, Error> {
    admit(calls)?;
    let checkpoint = service.updates.borrow().clone();
    let request = rpc::WatchViewRequest {
        session_id: Some(rpc::SessionId {
            value: Some(checkpoint.basis().session.as_bytes().to_vec()),
        }),
        run_id: Some(rpc::RunId {
            value: Some(checkpoint.basis().run.as_bytes().to_vec()),
        }),
        client_binding_id: Some(rpc::ClientBindingId {
            value: Some(member.to_vec()),
        }),
        after_revision: None,
    };
    let mut stream =
        rpc::session_service_server::SessionService::watch(service, bind(request, credential)?)
            .await?
            .into_inner();
    let first = timeout(Duration::from_secs(2), stream.next())
        .await?
        .ok_or_else(|| io::Error::other("rest first watch absent"))??;
    let bytes = first.encode_to_vec();
    drop(stream); // The actual unfold owns no spawned worker; this also releases its stream permit.
    required(service.streams.available_permits() == 6)?;
    Ok(bytes)
}
type RetainedGrants = ([[u8; 32]; 2], [[u8; 16]; 2]);
fn private_grants() -> Result<RetainedGrants, Error> {
    let bytes = read("rest-private-grants-01.bin", 96)?;
    required(bytes.len() == 96)?;
    Ok((
        [bytes[..32].try_into()?, bytes[32..64].try_into()?],
        [bytes[64..80].try_into()?, bytes[80..96].try_into()?],
    ))
}
async fn three_views(
    service: &Service,
    credentials: [[u8; 32]; 2],
    members: [[u8; 16]; 2],
    display: [u8; 32],
    calls: &mut Calls,
) -> Result<[Vec<u8>; 3], Error> {
    let first = first_watch(service, credentials[0], members[0], calls).await?;
    let second = first_watch(service, credentials[1], members[1], calls).await?;
    let public = first_watch(service, display, [0x72; 16], calls).await?;
    let first_decoded = rpc::ViewMessage::decode(first.as_slice())?;
    let second_decoded = rpc::ViewMessage::decode(second.as_slice())?;
    required(
        matches!(first_decoded.audience,Some(rpc::view_message::Audience::Player(view)) if !view.private_clue.is_empty()),
    )?;
    required(
        matches!(second_decoded.audience,Some(rpc::view_message::Audience::Player(view)) if view.private_clue.is_empty()),
    )?;
    required(
        matches!(rpc::ViewMessage::decode(public.as_slice())?.audience,Some(rpc::view_message::Audience::Display(view)) if view.journey.as_ref().is_some_and(|view|view.own_character.is_none() && view.creation.is_none())),
    )?;
    Ok([first, second, public])
}
fn packet(state: &Durable, checkpoints: &[String]) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    for count in state.counts {
        bytes.extend_from_slice(&count.to_be_bytes());
    }
    for row in state
        .operations
        .iter()
        .chain(&state.facts)
        .chain(&state.intents)
        .chain(checkpoints)
    {
        let size = u32::try_from(row.len())?;
        required(
            bytes
                .len()
                .checked_add(row.len() + 4)
                .is_some_and(|size| size <= 4 * 1024 * 1024),
        )?;
        bytes.extend_from_slice(&size.to_be_bytes());
        bytes.extend_from_slice(row.as_bytes());
    }
    Ok(bytes)
}
async fn physical(client: &Client, operation: u8) -> Result<(Durable, Vec<u8>), Error> {
    let state = durable(client, operation).await?;
    let checkpoints=physical_rows(client,"SELECT CASE WHEN octet_length(body)<=262144 AND sum(octet_length(body)) OVER ()<=1048576 THEN body END FROM (SELECT row_to_json(t)::text AS body,row_number() OVER (ORDER BY tenant_id,session_id,recovery_epoch,in_epoch_sequence) AS row_ordinal FROM df_game.checkpoints t ORDER BY tenant_id,session_id,recovery_epoch,in_epoch_sequence LIMIT 257) bounded ORDER BY row_ordinal").await?;
    let bytes = packet(&state, &checkpoints)?;
    Ok((state, bytes))
}
async fn phase_a(
    service: &Service,
    client: &Client,
    display: [u8; 32],
    codec: df_persistence::NativeCodecLimits,
) -> Result<(), Error> {
    let mut calls = Calls {
        count: 0,
        started_milliseconds: now()?,
    };
    let first = join(service, 0xa1, &mut calls).await?;
    let second = join(service, 0xa2, &mut calls).await?;
    for (index, credential) in [first, second].into_iter().enumerate() {
        let current = service.updates.borrow().clone();
        committed(
            submit(
                service,
                &create(&current, 0xa3 + index as u8),
                credential,
                &mut calls,
            )
            .await?,
        )?;
    }
    for (operation, kind) in [
        (0xa5, rpc::GameplayActionKind::BeginStory),
        (0xa6, rpc::GameplayActionKind::AskCourier),
        (0xa7, rpc::GameplayActionKind::DefendCourier),
    ] {
        let current = service.updates.borrow().clone();
        committed(
            submit(
                service,
                &action(&current, operation, kind),
                first,
                &mut calls,
            )
            .await?,
        )?;
    }
    let current = service.updates.borrow().clone();
    let participants = journey::participants(&current);
    required(participants.len() == 2)?;
    let members = [
        *participants[0].member.as_bytes(),
        *participants[1].member.as_bytes(),
    ];
    let credentials = [first, second];
    let mut spent = false;
    let mut strikes = 0;
    let mut ends = 0;
    let mut operation = 0xa8;
    let mut last_receipt = None;
    while current_phase(&service.updates.borrow())
        .map_err(|_| io::Error::other("phase unavailable"))?
        != rpc::JourneyPhase::Complete
    {
        let current = service.updates.borrow().clone();
        let mut active = None;
        for (index, member) in members.iter().enumerate() {
            let offers = source(journey::offered(
                &current,
                df_types::MemberId::from_bytes(member)
                    .map_err(|_| io::Error::other("rest member identity invalid"))?,
            ))?;
            if !offers.is_empty() {
                active = Some((index, offers));
                break;
            }
        }
        let (index, offers) =
            active.ok_or_else(|| io::Error::other("active combat offer unavailable"))?;
        let kind = if !spent
            && offers
                .iter()
                .any(|(kind, _)| *kind == rpc::GameplayActionKind::SecondWind)
        {
            spent = true;
            rpc::GameplayActionKind::SecondWind
        } else if offers
            .iter()
            .any(|(kind, _)| *kind == rpc::GameplayActionKind::GreatswordAttack)
        {
            required(strikes < 4)?;
            strikes += 1;
            rpc::GameplayActionKind::GreatswordAttack
        } else {
            required(
                ends < 3
                    && offers
                        .iter()
                        .any(|(kind, _)| *kind == rpc::GameplayActionKind::EndTurn),
            )?;
            ends += 1;
            rpc::GameplayActionKind::EndTurn
        };
        let mut body = action(&current, operation, kind);
        body.graze = kind == rpc::GameplayActionKind::GreatswordAttack;
        let receipt = committed(submit(service, &body, credentials[index], &mut calls).await?)?;
        required(!receipt.replayed)?;
        last_receipt = Some(receipt);
        operation = operation
            .checked_add(1)
            .ok_or_else(|| io::Error::other("fixture operation overflow"))?;
    }
    required(spent && strikes <= 4 && ends <= 3)?;
    let original_operation = operation - 1;
    let receipt = last_receipt.ok_or_else(|| io::Error::other("battle receipt unavailable"))?;
    let captured = snapshot(service, &mut calls).await?;
    required(captured.calls_remaining == 32 - calls.count as u16 && captured.joins.len() == 2)?;
    required(
        !captured.checkpoint.state().schedules.is_empty()
            && current_phase(&captured.checkpoint)? == rpc::JourneyPhase::Complete,
    )?;
    required(
        captured
            .checkpoint
            .state()
            .characters
            .iter()
            .any(|character| {
                resource_value(captured.checkpoint.state(), character.entity, "second-wind")
                    .is_ok_and(|uses| uses == 1)
            }),
    )?;
    let views = three_views(service, [first, second], members, display, &mut calls).await?;
    required(calls.count <= 19)?;
    let (state, rows) = physical(client, original_operation).await?;
    required(
        local_demo_scope::encode_owned_demo_checkpoint(&captured.checkpoint, codec)
            .map_err(|_| io::Error::other("rest envelope encoding"))?
            == state.envelope,
    )?;
    let mut grants = Vec::from(first);
    grants.extend_from_slice(&second);
    grants.extend_from_slice(&members[0]);
    grants.extend_from_slice(&members[1]);
    let mut metadata = Vec::from(calls.started_milliseconds.to_be_bytes());
    metadata.extend_from_slice(&(calls.count as u16).to_be_bytes());
    metadata.push(original_operation);
    for (name, bytes) in [
        ("rest-private-grants-01.bin", grants),
        ("rest-private-metadata-01.bin", metadata),
        (
            "rest-private-build-01.bin",
            crate::BUILD_ID.as_bytes().to_vec(),
        ),
        ("rest-private-receipt-01.bin", receipt.encode_to_vec()),
        ("rest-private-envelope-01.bin", state.envelope),
        ("rest-private-rows-01.bin", rows),
    ] {
        save(name, &bytes)?;
    }
    for (index, view) in views.iter().enumerate() {
        save(&format!("rest-private-view-{index}-01.bin"), view)?;
    }
    save(
        "rest-a-ready-body-01.bin",
        format!(
            "{{\"pass\":true,\"actor_calls\":{},\"rpc_calls\":{},\"exercise_milliseconds\":{},\"canonical_persisted\":true,\"private_views_correct\":true,\"watches_closed\":true,\"normal_second_wind_committed\":true,\"pending_knockout_rest\":true,\"strikes\":{},\"end_turns\":{},\"combat_victory\":{}}}",
            calls.count,calls.count-1,elapsed(calls.started_milliseconds)?,strikes,ends,victory(captured.checkpoint.state())?
        ).as_bytes(),
    )?;
    Ok(())
}
fn preserved(before: &Checkpoint, after: &Checkpoint, hours: u64) -> Result<(), Error> {
    let a = before.state();
    let b = after.state();
    required(
        b.logical_time.ticks
            == a.logical_time.ticks + 3600 * hours * u64::from(a.logical_time.ticks_per_second),
    )?;
    required(
        a.draws == b.draws
            && a.inventory == b.inventory
            && a.entities == b.entities
            && a.schedules == b.schedules
            && a.knowledge == b.knowledge
            && a.narrative.open_threads == b.narrative.open_threads
            && victory(a)? == victory(b)?,
    )?;
    for resource in &a.resources {
        if resource.resource.as_str() == "hit-points" {
            required(
                resource_value(b, resource.owner, "hit-points")? == u32::try_from(resource.value)?,
            )?;
        }
    }
    for who in a
        .characters
        .iter()
        .map(|character| character.entity)
        .chain([entity([0x65; 16])?])
    {
        required(resource_value(b, who, "unconscious")? == 0)?;
        required(
            resource_value(a, who, "prone")? == resource_value(b, who, "prone")?
                && resource_value(a, who, "held-weapon")? == resource_value(b, who, "held-weapon")?,
        )?;
        if a.characters.iter().any(|character| character.entity == who) {
            required(
                resource_value(b, who, "second-wind")?
                    == 2.min(resource_value(a, who, "second-wind")? + hours as u32),
            )?;
        }
    }
    Ok(())
}
fn refused(value: rpc::SubmitActionResponse) -> Result<(), Error> {
    required(
        !matches!(value.outcome,Some(rpc::submit_action_response::Outcome::CommittedDecision(receipt)) if matches!(receipt.outcome,Some(rpc::decision_receipt::Outcome::Accepted(_)))),
    )?;
    Ok(())
}
async fn phase_b(
    service: &Service,
    client: &Client,
    display: [u8; 32],
    codec: df_persistence::NativeCodecLimits,
) -> Result<(), Error> {
    let (start, count, original_operation) = metadata()?;
    let restored = service.updates.borrow().clone();
    stale_release(client, &restored).await?;
    let mut calls = Calls {
        count,
        started_milliseconds: start,
    };
    let (credentials, members) = private_grants()?;
    let views = three_views(service, credentials, members, display, &mut calls).await?;
    for (index, view) in views.iter().enumerate() {
        required(*view == read(&format!("rest-private-view-{index}-01.bin"), 8192)?)?;
    }
    let captured = snapshot(service, &mut calls).await?;
    required(captured.calls_remaining == 32 - calls.count as u16)?;
    let envelope = local_demo_scope::encode_owned_demo_checkpoint(&captured.checkpoint, codec)
        .map_err(|_| io::Error::other("rest encoding failed"))?;
    required(envelope == read("rest-private-envelope-01.bin", 1048576)?)?;
    let (before, rows) = physical(client, original_operation as u8).await?;
    required(
        before.envelope == envelope && rows == read("rest-private-rows-01.bin", 4 * 1048576)?,
    )?;
    report(
        "rest-b-restored-01.json",
        &format!(
            "{{\"pass\":true,\"actor_calls\":{},\"canonical_equal\":true,\"four_families_equal\":true,\"private_views_equal\":true,\"pending_knockout_rest\":true,\"watches_closed\":true,\"exercise_milliseconds\":{}}}",
            calls.count,
            elapsed(start)?
        ),
    )?;
    let body = action(&restored, 0xc1, rpc::GameplayActionKind::ShortRest);
    let receipt = committed(submit(service, &body, credentials[0], &mut calls).await?)?;
    required(!receipt.replayed)?;
    let rested = service.updates.borrow().clone();
    preserved(&restored, &rested, 1)?;
    let (committed_state, committed_rows) = physical(client, 0xc1).await?;
    let mut replay = committed(submit(service, &body, credentials[0], &mut calls).await?)?;
    required(replay.replayed)?;
    replay.replayed = false;
    required(replay == receipt)?;
    let (retried, retried_rows) = physical(client, 0xc1).await?;
    required(committed_state == retried && committed_rows == retried_rows)?;
    let mut stale = body.clone();
    stale.operation_id = Some(rpc::OperationId {
        value: Some(vec![0xc2; 16]),
    });
    refused(submit(service, &stale, credentials[0], &mut calls).await?)?;
    let mut malformed = action(&rested, 0xc3, rpc::GameplayActionKind::ShortRest);
    malformed.graze = true;
    refused(submit(service, &malformed, credentials[0], &mut calls).await?)?;
    let (refused_state, refused_rows) = physical(client, 0xc1).await?;
    required(refused_state == committed_state && refused_rows == committed_rows)?;
    let fresh = action(&rested, 0xc4, rpc::GameplayActionKind::ShortRest);
    required(!committed(submit(service, &fresh, credentials[0], &mut calls).await?)?.replayed)?;
    let completed = snapshot(service, &mut calls).await?;
    preserved(&restored, &completed.checkpoint, 2)?;
    required(completed.calls_remaining == 32 - calls.count as u16)?;
    let (final_state, _) = physical(client, original_operation as u8).await?;
    required(
        final_state.receipt == before.receipt
            && local_demo_scope::encode_owned_demo_checkpoint(&completed.checkpoint, codec)
                .map_err(|_| io::Error::other("rest encoding failed"))?
                == final_state.envelope,
    )?;
    three_views(service, credentials, members, display, &mut calls).await?;
    required(calls.count == count + 13 && calls.count <= 32)?;
    save(
        "rest-private-final-count-01.bin",
        &(calls.count as u16).to_be_bytes(),
    )?;
    save("rest-b-proof-body-01.bin",format!("{{\"pass\":true,\"actor_calls\":{},\"rpc_calls\":{},\"exercise_milliseconds\":{},\"rest_hour_accepted\":true,\"pending_rest_restored\":true,\"no_hit_dice_or_healing\":true,\"second_wind_recovered\":true,\"prone_and_dropped_retained\":true,\"battle_outcome_retained\":true,\"original_battle_receipt_equal\":true,\"exact_rest_retry_equal\":true,\"four_families_retry_equal\":true,\"stale_rest_refused\":true,\"malformed_rest_refused\":true,\"fresh_second_hour_accepted\":true,\"private_views_correct\":true,\"watches_closed\":true,\"stale_release_refused\":true,\"inspector_driver_joined\":true}}",calls.count,calls.count-3,elapsed(start)?).as_bytes())?;
    Ok(())
}

pub(super) async fn run(
    service: Service,
    display: [u8; 32],
    codec: df_persistence::NativeCodecLimits,
    database: Config,
    phase: Phase,
    fence: [u8; 16],
) -> Result<(), io::Error> {
    configured()?;
    record_owner(phase, fence)?;
    let (client, connection) = timeout(Duration::from_secs(2), database.connect(NoTls))
        .await
        .map_err(|_| io::Error::other("rest inspector connect deadline"))?
        .map_err(io::Error::other)?;
    let driver = tokio::spawn(connection);
    let result = timeout(Duration::from_secs(45), async {
        match phase {
            Phase::A => phase_a(&service, &client, display, codec).await,
            Phase::B => phase_b(&service, &client, display, codec).await,
            Phase::Contender => Err(io::Error::other("live contender incorrectly admitted").into()),
        }
    })
    .await;
    drop(client);
    super::join_grant_driver(driver).await?;
    result
        .map_err(|_| io::Error::other("rest exercise deadline"))?
        .map_err(|error| io::Error::other(format!("rest phase failed: {error}")))?;
    let (body, name) = match phase {
        Phase::A => ("rest-a-ready-body-01.bin", "rest-a-ready-01.json"),
        Phase::B => ("rest-b-proof-body-01.bin", "rest-b-proof-01.json"),
        Phase::Contender => return Err(io::Error::other("live contender incorrectly admitted")),
    };
    let bytes = read(body, 16 * 1024)?;
    report(name, std::str::from_utf8(&bytes).map_err(io::Error::other)?)
}
pub(super) fn closed(phase: Phase, remaining: u16) -> Result<(), io::Error> {
    let (start, count, _) = metadata()?;
    let (expected, name) = match phase {
        Phase::A => (count, "rest-a-closed-01.json"),
        Phase::B => (
            usize::from(u16::from_be_bytes(
                read("rest-private-final-count-01.bin", 2)?
                    .try_into()
                    .map_err(|_| io::Error::other("rest final count invalid"))?,
            )),
            "rest-b-closed-01.json",
        ),
        Phase::Contender => return Err(io::Error::other("contender cannot qualify closed")),
    };
    required(expected <= 32 && remaining == 32 - expected as u16 && elapsed(start)? <= 45000)?;
    report(
        name,
        &format!(
            "{{\"pass\":true,\"actor_calls\":{},\"exercise_milliseconds\":{},\"admission_stopped\":true,\"actor_drained\":true,\"exact_fence_released\":true,\"actor_joined\":true,\"repository_driver_joined\":true,\"grant_driver_joined\":true}}",
            expected,
            elapsed(start)?
        ),
    )
}
