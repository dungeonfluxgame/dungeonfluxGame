//! Private operator proof through the real generated service in two real native processes.
//! Phase files live only in the Root-owned protected scratch directory; never RPC input.
use super::{Service, actor, hex, journey, wire};
use df_model::checkpoint::Checkpoint;
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
const ROOT: &str = "/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/engine-restart-20261005";
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Phase {
    A,
    B,
    Contender,
}
pub(super) fn phase() -> Option<Phase> {
    match std::env::args().nth(4).as_deref() {
        Some("--restart-qualification-a") => Some(Phase::A),
        Some("--restart-qualification-b") => Some(Phase::B),
        Some("--restart-qualification-contender") => Some(Phase::Contender),
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
            "restart assertion {}:{}",
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
        .ok_or_else(|| io::Error::other("restart clock regressed"))
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
        std::env::var("DF_RESTART_QUALIFICATION").as_deref() == Ok("owned-loopback-restart01")
            && std::env::var("DF_GAMEPLAY_DEMO_DATABASE").as_deref()
                == Ok("df_gameplay_demo_20261005_restart01"),
    )
}
fn metadata() -> Result<(u64, usize, usize), io::Error> {
    let bytes = read("restart-private-metadata-01.bin", 11)?;
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
        return Ok(20);
    }
    let (start, calls, index) = metadata()?;
    required(calls == 12 && index < 2 && elapsed(start)? <= 45_000)?;
    required(read("restart-private-build-01.bin", 96)? == crate::BUILD_ID.as_bytes())?;
    Ok(20 - calls as u16)
}
pub(super) fn record_owner(phase: Phase, fence: [u8; 16]) -> Result<(), io::Error> {
    if phase == Phase::A {
        save("restart-private-old-fence-01.bin", &fence)?;
    }
    Ok(())
}
async fn stale_release(client: &Client, checkpoint: &Checkpoint) -> Result<(), Error> {
    let fence: [u8; 16] = read("restart-private-old-fence-01.bin", 16)?
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
        "restart-contender-refused-01.json",
        "{\"pass\":true,\"live_owner_refused\":true,\"listener_bound\":false,\"actor_calls\":0,\"stage\":\"live_owner\",\"class\":\"RevisionConflict\"}",
    )
}
struct Calls {
    count: usize,
    started_milliseconds: u64,
}
fn admit(calls: &mut Calls) -> Result<(), Error> {
    required(calls.count < 20 && elapsed(calls.started_milliseconds)? <= 45_000)?;
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
        name: "Restart-Fighter".to_owned(),
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
        Some(rpc::submit_action_response::Outcome::CommittedDecision(value)) => Ok(*value),
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
        .ok_or_else(|| io::Error::other("restart first watch absent"))??;
    let bytes = first.encode_to_vec();
    drop(stream); // The actual unfold owns no spawned worker; this also releases its stream permit.
    required(service.streams.available_permits() == 6)?;
    Ok(bytes)
}
type RetainedGrants = ([[u8; 32]; 2], [[u8; 16]; 2]);
fn private_grants() -> Result<RetainedGrants, Error> {
    let bytes = read("restart-private-grants-01.bin", 96)?;
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
    let active = (0..2)
        .find(|index| {
            journey::offered(
                &current,
                df_types::MemberId::from_bytes(&members[*index]).expect("source-owned member"),
            )
            .is_ok_and(|offers| {
                offers
                    .iter()
                    .any(|(kind, _)| *kind == rpc::GameplayActionKind::SecondWind)
            })
        })
        .ok_or_else(|| io::Error::other("normally offered Second Wind absent"))?;
    let body = action(&current, 0xa8, rpc::GameplayActionKind::SecondWind);
    let receipt = committed(submit(service, &body, [first, second][active], &mut calls).await?)?;
    required(!receipt.replayed)?;
    let snapshot = snapshot(service, &mut calls).await?;
    required(snapshot.calls_remaining == 20 - calls.count as u16 && snapshot.joins.len() == 2)?;
    required(
        snapshot
            .checkpoint
            .state()
            .decisions
            .last()
            .is_some_and(|decision| !decision.draws.is_empty()),
    )?;
    let views = three_views(service, [first, second], members, display, &mut calls).await?;
    required(calls.count == 12)?;
    let (state, rows) = physical(client, 0xa8).await?;
    required(
        local_demo_scope::encode_owned_demo_checkpoint(&snapshot.checkpoint, codec)
            .map_err(|_| io::Error::other("restart envelope encoding"))?
            == state.envelope,
    )?;
    let mut grants = Vec::from(first);
    grants.extend_from_slice(&second);
    grants.extend_from_slice(&members[0]);
    grants.extend_from_slice(&members[1]);
    let mut metadata = Vec::from(calls.started_milliseconds.to_be_bytes());
    metadata.extend_from_slice(&(calls.count as u16).to_be_bytes());
    metadata.push(active as u8);
    for (name, bytes) in [
        ("restart-private-grants-01.bin", grants),
        ("restart-private-metadata-01.bin", metadata),
        (
            "restart-private-build-01.bin",
            crate::BUILD_ID.as_bytes().to_vec(),
        ),
        ("restart-private-request-01.bin", body.encode_to_vec()),
        ("restart-private-receipt-01.bin", receipt.encode_to_vec()),
        ("restart-private-envelope-01.bin", state.envelope),
        ("restart-private-rows-01.bin", rows),
    ] {
        save(name, &bytes)?;
    }
    for (index, view) in views.iter().enumerate() {
        save(&format!("restart-private-view-{index}-01.bin"), view)?;
    }
    save(
        "restart-a-ready-body-01.bin",
        format!(
            "{{\"pass\":true,\"actor_calls\":12,\"rpc_calls\":11,\"exercise_milliseconds\":{},\"canonical_persisted\":true,\"private_views_correct\":true,\"watches_closed\":true,\"normal_second_wind_committed\":true}}",
            elapsed(calls.started_milliseconds)?
        ).as_bytes(),
    )?;
    Ok(())
}
async fn phase_b(
    service: &Service,
    client: &Client,
    display: [u8; 32],
    codec: df_persistence::NativeCodecLimits,
) -> Result<(), Error> {
    let (start, count, active) = metadata()?;
    let restored = service.updates.borrow().clone();
    stale_release(client, &restored).await?;
    let mut calls = Calls {
        count,
        started_milliseconds: start,
    };
    let (credentials, members) = private_grants()?;
    let views = three_views(service, credentials, members, display, &mut calls).await?;
    for (index, view) in views.iter().enumerate() {
        required(*view == read(&format!("restart-private-view-{index}-01.bin"), 8192)?)?;
    }
    let snapshot = snapshot(service, &mut calls).await?;
    required(snapshot.calls_remaining == 20 - calls.count as u16)?;
    let envelope = local_demo_scope::encode_owned_demo_checkpoint(&snapshot.checkpoint, codec)
        .map_err(|_| io::Error::other("restart envelope encoding"))?;
    required(envelope == read("restart-private-envelope-01.bin", 1048576)?)?;
    let (before, rows) = physical(client, 0xa8).await?;
    required(
        before.envelope == envelope && rows == read("restart-private-rows-01.bin", 4 * 1048576)?,
    )?;
    report(
        "restart-b-restored-01.json",
        &format!(
            "{{\"pass\":true,\"actor_calls\":{},\"canonical_equal\":true,\"four_families_equal\":true,\"private_views_equal\":true,\"watches_closed\":true,\"exercise_milliseconds\":{}}}",
            calls.count,
            elapsed(start)?
        ),
    )?;
    let original =
        rpc::SubmitActionRequest::decode(read("restart-private-request-01.bin", 8192)?.as_slice())?;
    let mut replay = committed(submit(service, &original, credentials[active], &mut calls).await?)?;
    required(replay.replayed)?;
    replay.replayed = false;
    required(replay.encode_to_vec() == read("restart-private-receipt-01.bin", 8192)?)?;
    let (after, after_rows) = physical(client, 0xa8).await?;
    required(
        before == after
            && rows == after_rows
            && envelope
                == local_demo_scope::encode_owned_demo_checkpoint(&service.updates.borrow(), codec)
                    .map_err(|_| io::Error::other("restart envelope encoding"))?,
    )?;
    let current = service.updates.borrow().clone();
    let later = action(&current, 0xa9, rpc::GameplayActionKind::GreatswordAttack);
    let accepted = committed(submit(service, &later, credentials[active], &mut calls).await?)?;
    required(!accepted.replayed)?;
    let (committed_state, committed_rows) = physical(client, 0xa9).await?;
    required(
        committed_state.counts[0] == before.counts[0] + 1
            && committed_state.counts[2] == before.counts[2] + 1,
    )?;
    let mut again = committed(submit(service, &later, credentials[active], &mut calls).await?)?;
    required(again.replayed)?;
    again.replayed = false;
    required(again == accepted)?;
    let (retried, retried_rows) = physical(client, 0xa9).await?;
    required(retried == committed_state && retried_rows == committed_rows && calls.count == 19)?;
    save(
        "restart-b-proof-body-01.bin",
        format!(
            "{{\"pass\":true,\"actor_calls\":19,\"rpc_calls\":17,\"exercise_milliseconds\":{},\"original_receipt_equal\":true,\"no_reroll_or_duplicate\":true,\"later_action_once\":true,\"later_retry_equal\":true,\"stale_release_refused\":true,\"inspector_driver_joined\":true}}",
            elapsed(start)?
        ).as_bytes(),
    )?;
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
        .map_err(|_| io::Error::other("restart inspector connect deadline"))?
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
        .map_err(|_| io::Error::other("restart exercise deadline"))?
        .map_err(|error| io::Error::other(format!("restart phase failed: {error}")))?;
    let (body, name) = match phase {
        Phase::A => ("restart-a-ready-body-01.bin", "restart-a-ready-01.json"),
        Phase::B => ("restart-b-proof-body-01.bin", "restart-b-proof-01.json"),
        Phase::Contender => return Err(io::Error::other("live contender incorrectly admitted")),
    };
    let bytes = read(body, 16 * 1024)?;
    report(name, std::str::from_utf8(&bytes).map_err(io::Error::other)?)
}
pub(super) fn closed(phase: Phase, remaining: u16) -> Result<(), io::Error> {
    let (start, expected, name) = match phase {
        Phase::A => {
            let (start, _, _) = metadata()?;
            (start, 8, "restart-a-closed-01.json")
        }
        Phase::B => {
            let (start, _, _) = metadata()?;
            (start, 1, "restart-b-closed-01.json")
        }
        Phase::Contender => return Err(io::Error::other("contender close cannot qualify")),
    };
    required(remaining == expected && elapsed(start)? <= 45_000)?;
    report(
        name,
        &format!(
            "{{\"pass\":true,\"actor_calls\":{},\"exercise_milliseconds\":{},\"admission_stopped\":true,\"actor_drained\":true,\"exact_fence_released\":true,\"actor_joined\":true,\"repository_driver_joined\":true,\"grant_driver_joined\":true}}",
            20 - remaining,
            elapsed(start)?
        ),
    )
}
