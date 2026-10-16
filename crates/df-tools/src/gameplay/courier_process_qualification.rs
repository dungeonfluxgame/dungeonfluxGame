//! Private finite proof across three distinct executable processes; never player RPC control.
use super::{Service, actor, courier_ai, hex, journey, model, wire};
use df_model::checkpoint::{Checkpoint, DurableStatus, EffectKind, GameInput};
use df_persistence::local_demo_scope;
use df_protocol::common as rpc;
use df_session::inbox::{AdmissionSequence, Reducer};
use df_session::submission::{OwnedInput, SubmissionOutcome};
use futures::StreamExt;
use prost::Message;
use sha2::{Digest, Sha256};
use std::{
    io,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{sync::oneshot, time::timeout};
use tokio_postgres::{Client, Config, NoTls};
use tonic::Request;

type Error = Box<dyn std::error::Error + Send + Sync>;
const ROOT: &str =
    "/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/engine-courier-ai-20261005";
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Phase {
    A,
    B,
    C,
}
pub(super) fn phase() -> Option<Phase> {
    match std::env::args().nth(4).as_deref() {
        Some("--courier-process-a") => Some(Phase::A),
        Some("--courier-process-b") => Some(Phase::B),
        Some("--courier-process-c") => Some(Phase::C),
        _ => None,
    }
}
fn configured() -> Result<(), io::Error> {
    required(
        std::env::var("DF_COURIER_PROCESS_QUALIFICATION").as_deref()
            == Ok("owned-loopback-courier01")
            && std::env::var("DF_GAMEPLAY_DEMO_DATABASE").as_deref()
                == Ok("df_gameplay_demo_20261005_courier01"),
    )
}
fn metadata(name: &str) -> Result<(u64, u16, u32), io::Error> {
    let bytes = read(name, 14)?;
    required(bytes.len() == 14)?;
    Ok((
        u64::from_be_bytes(bytes[..8].try_into().map_err(io::Error::other)?),
        u16::from_be_bytes(bytes[8..10].try_into().map_err(io::Error::other)?),
        u32::from_be_bytes(bytes[10..14].try_into().map_err(io::Error::other)?),
    ))
}
fn save_metadata(name: &str, calls: &Calls) -> Result<(), Error> {
    let mut bytes = Vec::from(calls.started_milliseconds.to_be_bytes());
    bytes.extend_from_slice(&u16::try_from(calls.count)?.to_be_bytes());
    bytes.extend_from_slice(&std::process::id().to_be_bytes());
    save(name, &bytes)?;
    Ok(())
}
pub(super) fn call_budget(phase: Phase) -> Result<u16, io::Error> {
    configured()?;
    if phase == Phase::A {
        return Ok(32);
    }
    required(read("courier-private-build-01.bin", 96)? == crate::BUILD_ID.as_bytes())?;
    let closed = if phase == Phase::B {
        "courier-a-closed-01.json"
    } else {
        "courier-b-closed-01.json"
    };
    required(read(closed, 16 * 1024)?.starts_with(b"{\"pass\":true,"))?;
    let name = if phase == Phase::B {
        "courier-private-metadata-a-01.bin"
    } else {
        "courier-private-metadata-b-01.bin"
    };
    let (start, count, pid) = metadata(name)?;
    required(count == if phase == Phase::B { 9 } else { 14 })?;
    required(pid != std::process::id() && elapsed(start)? <= 90_000)?;
    if phase == Phase::C {
        required(metadata("courier-private-metadata-a-01.bin")?.2 != std::process::id())?;
    }
    Ok(32 - count)
}
fn repo(error: df_session::submission::RepositoryError) -> io::Error {
    io::Error::other(format!("courier native repository {error:?}"))
}
fn fixture(error: courier_ai::CourierError) -> io::Error {
    io::Error::other(format!("courier recording {error:?}"))
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
struct Calls {
    count: usize,
    started_milliseconds: u64,
}
fn admit(calls: &mut Calls) -> Result<(), Error> {
    required(calls.count < 32 && elapsed(calls.started_milliseconds)? <= 90_000)?;
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
        name: "Courier-Fighter".to_owned(),
        choices: [
            ("portrait", "portrait-hero-1"),
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
    checkpoints: Vec<String>,
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
        checkpoints: Vec::new(),
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
    let bytes = read("courier-private-grants-01.bin", 96)?;
    required(bytes.len() == 96)?;
    Ok((
        [bytes[..32].try_into()?, bytes[32..64].try_into()?],
        [bytes[64..80].try_into()?, bytes[80..96].try_into()?],
    ))
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
    let mut state = durable(client, operation).await?;
    let checkpoints=physical_rows(client,"SELECT CASE WHEN octet_length(body)<=262144 AND sum(octet_length(body)) OVER ()<=1048576 THEN body END FROM (SELECT row_to_json(t)::text AS body,row_number() OVER (ORDER BY tenant_id,session_id,recovery_epoch,in_epoch_sequence) AS row_ordinal FROM df_game.checkpoints t ORDER BY tenant_id,session_id,recovery_epoch,in_epoch_sequence LIMIT 257) bounded ORDER BY row_ordinal").await?;
    let bytes = packet(&state, &checkpoints)?;
    state.checkpoints = checkpoints;
    Ok((state, bytes))
}

async fn three_views(
    service: &Service,
    credentials: [[u8; 32]; 2],
    members: [[u8; 16]; 2],
    display: [u8; 32],
    calls: &mut Calls,
    answered: bool,
) -> Result<[Vec<u8>; 3], Error> {
    let first = first_watch(service, credentials[0], members[0], calls).await?;
    let second = first_watch(service, credentials[1], members[1], calls).await?;
    let public = first_watch(service, display, [0x72; 16], calls).await?;
    let first_decoded = rpc::ViewMessage::decode(first.as_slice())?;
    required(
        matches!(first_decoded.audience,Some(rpc::view_message::Audience::Player(view))
        if view.private_clue == if answered {courier_ai::RESPONSE} else {""}),
    )?;
    required(
        matches!(rpc::ViewMessage::decode(second.as_slice())?.audience,
        Some(rpc::view_message::Audience::Player(view)) if view.private_clue.is_empty()),
    )?;
    required(
        matches!(rpc::ViewMessage::decode(public.as_slice())?.audience,
        Some(rpc::view_message::Audience::Display(view)) if view.journey.as_ref()
            .is_some_and(|journey|journey.own_character.is_none() && journey.creation.is_none())),
    )?;
    required(
        !public
            .windows(courier_ai::RESPONSE.len())
            .any(|part| part == courier_ai::RESPONSE.as_bytes()),
    )?;
    Ok([first, second, public])
}
async fn phase_a(
    service: &Service,
    display: [u8; 32],
    codec: df_persistence::NativeCodecLimits,
) -> Result<(), Error> {
    let mut calls = Calls {
        count: 0,
        started_milliseconds: now()?,
    };
    let first = join(service, 0xc1, &mut calls).await?;
    let second = join(service, 0xc2, &mut calls).await?;
    for (index, credential) in [first, second].into_iter().enumerate() {
        let current = service.updates.borrow().clone();
        committed(
            submit(
                service,
                &create(&current, 0xc3 + index as u8),
                credential,
                &mut calls,
            )
            .await?,
        )?;
    }
    let current = service.updates.borrow().clone();
    committed(
        submit(
            service,
            &action(&current, 0xc5, rpc::GameplayActionKind::BeginStory),
            first,
            &mut calls,
        )
        .await?,
    )?;
    let prepared = snapshot(service, &mut calls).await?;
    required(
        prepared.calls_remaining == 32 - calls.count as u16
            && prepared.checkpoint.state().intents.is_empty(),
    )?;
    let participants = journey::participants(&prepared.checkpoint);
    required(participants.len() == 2)?;
    let members = [
        *participants[0].member.as_bytes(),
        *participants[1].member.as_bytes(),
    ];
    three_views(
        service,
        [first, second],
        members,
        display,
        &mut calls,
        false,
    )
    .await?;
    required(calls.count == 9)?;
    let request = action(
        &prepared.checkpoint,
        0xc6,
        rpc::GameplayActionKind::AskCourier,
    );
    let mut grants = Vec::from(first);
    grants.extend_from_slice(&second);
    grants.extend_from_slice(&members[0]);
    grants.extend_from_slice(&members[1]);
    save("courier-private-grants-01.bin", &grants)?;
    save("courier-private-build-01.bin", crate::BUILD_ID.as_bytes())?;
    save("courier-private-request-01.bin", &request.encode_to_vec())?;
    save(
        "courier-private-prepared-envelope-01.bin",
        &local_demo_scope::encode_owned_demo_checkpoint(&prepared.checkpoint, codec)
            .map_err(repo)?,
    )?;
    save_metadata("courier-private-metadata-a-01.bin", &calls)?;
    report(
        "courier-a-prepared-01.json",
        &format!(
            "{{\"pass\":true,\"pid\":{},\"actor_calls\":9,\"source_commit_pending_until_inbox_drained\":true,\"private_response_absent\":true,\"watches_closed\":true}}",
            std::process::id()
        ),
    )?;
    Ok(())
}

/// Only the named private A fixture calls this after normal inbox stop and drain.
/// The genuine owner/issuer commit remains before normal exact-fence release and close.
/// No Actor input boundary runs after this source submission, so the queued durable
/// work is left for the next executable's ordinary startup.
pub(super) fn after_drained(
    phase: Phase,
    actor: &mut actor::Actor,
    database: &Config,
    runtime: &tokio::runtime::Handle,
    reductions: &std::sync::atomic::AtomicUsize,
) -> Result<(), io::Error> {
    configured()?;
    if phase != Phase::A {
        return Ok(());
    }
    let (start, count, pid) = metadata("courier-private-metadata-a-01.bin")?;
    required(
        pid == std::process::id()
            && count == 9
            && actor.calls_remaining == 23
            && elapsed(start)? < 45_000,
    )?;
    required(read("courier-a-prepared-01.json", 16 * 1024)?.starts_with(b"{\"pass\":true,"))?;
    required(!actor.fenced && actor.owner.is_current() && !actor.owner.has_uncertain_operation())?;
    required(
        local_demo_scope::encode_owned_demo_checkpoint(actor.owner.checkpoint(), actor.codec)
            .map_err(repo)?
            == read("courier-private-prepared-envelope-01.bin", 1048576)?,
    )?;
    let (grants, members) = private_grants().map_err(io::Error::other)?;
    let request =
        rpc::SubmitActionRequest::decode(read("courier-private-request-01.bin", 8192)?.as_slice())
            .map_err(io::Error::other)?;
    let member = df_types::MemberId::from_bytes(&members[0])
        .map_err(|error| io::Error::other(format!("courier member {error:?}")))?;
    let input = wire::journey_input(&request, member, actor.owner.checkpoint())
        .map_err(io::Error::other)?;
    let GameInput::Game(command) = &input else {
        return Err(io::Error::other("courier source command absent"));
    };
    let operation = command.operation;
    let scope = actor
        .issuer
        .issue(
            &grants[0],
            model::random().map_err(repo)?,
            Sha256::digest(request.encode_to_vec()).into(),
            input.clone(),
            model::pins().map_err(repo)?,
        )
        .map_err(repo)?;
    let (item, wait) = OwnedInput::new(
        df_observe::OperationContext {
            trace_parent: String::new(),
            build: crate::BUILD_ID.to_owned(),
        },
        scope,
        input,
    );
    actor.owner.reduce(AdmissionSequence(0), item);
    let SubmissionOutcome::Confirmed(receipt) = wait.try_recv().map_err(io::Error::other)? else {
        return Err(io::Error::other("courier source commit not confirmed"));
    };
    required(
        actor.owner.is_current()
            && !actor.owner.has_uncertain_operation()
            && actor.intent_notifications.try_recv().is_ok(),
    )?;
    let pending = actor.owner.checkpoint();
    required(
        pending.state().intents.len() == 1
            && pending.state().intents[0].kind == EffectKind::RunAi
            && pending.state().intents[0].status == DurableStatus::Pending,
    )?;
    required(
        courier_ai::recipient(pending, &pending.state().intents[0]).map_err(fixture)? == member,
    )?;
    required(
        courier_ai::saved_response(pending, member)
            .map_err(repo)?
            .is_none(),
    )?;
    let generated = wire::receipt(
        receipt.basis(),
        operation,
        rpc::decision_receipt::Outcome::Accepted(Box::new(
            journey::accepted(receipt.decision()).map_err(repo)?,
        )),
        false,
    );
    save("courier-private-receipt-01.bin", &generated.encode_to_vec())?;
    required(reductions.load(std::sync::atomic::Ordering::SeqCst) == 6)?;
    let remaining = 45_000_u64
        .checked_sub(elapsed(start)?)
        .ok_or_else(|| io::Error::other("courier A work deadline"))?;
    runtime
        .block_on(inspect_pending(
            database,
            pending,
            actor.codec,
            Duration::from_millis(remaining),
        ))
        .map_err(io::Error::other)?;
    required(elapsed(start)? <= 45_000)?;
    report(
        "courier-a-pending-01.json",
        &format!(
            "{{\"pass\":true,\"pid\":{},\"actor_calls\":9,\"post_drain_owner_submissions\":1,\"native_engine_reductions\":6,\"source_commit_confirmed\":true,\"actual_wake_queued\":true,\"intent_pending\":true,\"response_absent\":true,\"executor_not_drained_after_source_commit\":true}}",
            std::process::id()
        ),
    )
}
async fn inspect_pending(
    database: &Config,
    pending: &Checkpoint,
    codec: df_persistence::NativeCodecLimits,
    work: Duration,
) -> Result<(), Error> {
    let (client, connection) = timeout(Duration::from_secs(2), database.connect(NoTls)).await??;
    let driver = tokio::spawn(connection);
    let result = timeout(work, async {
        let row = client
            .query_one(
                "SELECT owner_fence FROM df_game.sessions WHERE tenant_id=$1 AND session_id=$2",
                &[
                    &local_demo_scope::TENANT.as_slice(),
                    &pending.basis().session.as_bytes().as_slice(),
                ],
            )
            .await?;
        let fence: Vec<u8> = row.try_get(0)?;
        required(fence.len() == 16)?;
        save("courier-private-old-fence-01.bin", &fence)?;
        let (state, rows) = physical(&client, 0xc6).await?;
        let envelope =
            local_demo_scope::encode_owned_demo_checkpoint(pending, codec).map_err(repo)?;
        required(
            state.counts == [6, i64::try_from(pending.state().facts.len())?, 7, 1]
                && state.envelope == envelope,
        )?;
        for (name, bytes) in [
            ("courier-private-pending-envelope-01.bin", envelope),
            ("courier-private-pending-rows-01.bin", rows),
            (
                "courier-private-pending-facts-01.bin",
                packet_rows(&state.facts)?,
            ),
            (
                "courier-private-pending-intents-01.bin",
                packet_rows(&state.intents)?,
            ),
            (
                "courier-private-pending-operations-01.bin",
                packet_rows(&state.operations)?,
            ),
            (
                "courier-private-pending-checkpoints-01.bin",
                packet_rows(&state.checkpoints)?,
            ),
        ] {
            save(name, &bytes)?;
        }
        let mut counts = Vec::new();
        for count in state.counts {
            counts.extend_from_slice(&count.to_be_bytes());
        }
        save("courier-private-pending-counts-01.bin", &counts)?;
        Ok::<_, Error>(())
    })
    .await;
    drop(client);
    super::join_grant_driver(driver).await?;
    result.map_err(|error| Box::new(error) as Error)?
}
fn packet_rows(rows: &[String]) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    for row in rows {
        required(
            bytes
                .len()
                .checked_add(row.len() + 4)
                .is_some_and(|size| size <= 4 * 1048576),
        )?;
        bytes.extend_from_slice(&u32::try_from(row.len())?.to_be_bytes());
        bytes.extend_from_slice(row.as_bytes());
    }
    Ok(bytes)
}
fn retained_rows(name: &str) -> Result<Vec<String>, Error> {
    let bytes = read(name, 4 * 1048576)?;
    let mut at = 0;
    let mut rows = Vec::new();
    while at < bytes.len() {
        required(rows.len() < 256 && at + 4 <= bytes.len())?;
        let len = u32::from_be_bytes(bytes[at..at + 4].try_into()?) as usize;
        at += 4;
        required(len <= 262144 && at.checked_add(len).is_some_and(|end| end <= bytes.len()))?;
        rows.push(std::str::from_utf8(&bytes[at..at + len])?.to_owned());
        at += len;
    }
    Ok(rows)
}
async fn stale_release(client: &Client, checkpoint: &Checkpoint) -> Result<(), Error> {
    let fence: [u8; 16] = read("courier-private-old-fence-01.bin", 16)?
        .try_into()
        .map_err(|_| io::Error::other("courier old fence length"))?;
    required(
        local_demo_scope::release_owner(client, checkpoint.basis().session, fence).await
            == Err(df_session::submission::RepositoryError::RevisionConflict),
    )?;
    Ok(())
}
fn verify_completed(
    checkpoint: &Checkpoint,
    codec: df_persistence::NativeCodecLimits,
    member: df_types::MemberId,
) -> Result<(), Error> {
    required(checkpoint.state().intents.len() == 1)?;
    let effect = &checkpoint.state().intents[0];
    required(effect.kind == EffectKind::RunAi && effect.status == DurableStatus::Completed)?;
    required(
        courier_ai::saved_response(checkpoint, member).map_err(repo)? == Some(courier_ai::RESPONSE),
    )?;
    let operation = courier_ai::completion_operation(effect).map_err(fixture)?;
    let mut state = checkpoint.state().clone();
    let last = state
        .decisions
        .pop()
        .ok_or_else(|| io::Error::other("courier completion decision absent"))?;
    required(
        last.operation == operation
            && last.source_policy.as_str() == courier_ai::POLICY
            && last.semantic_output.as_deref() == Some(courier_ai::RESPONSE)
            && last.facts.is_empty()
            && last.draws.is_empty()
            && last.effects.is_empty(),
    )?;
    required(
        checkpoint.basis().revision
            == effect
                .basis
                .revision
                .next_sequence()
                .map_err(|error| io::Error::other(format!("courier revision {error:?}")))?,
    )?;
    state.intents[0].status = DurableStatus::Pending;
    let original = model::checkpoint(effect.basis, state).map_err(repo)?;
    required(
        local_demo_scope::encode_owned_demo_checkpoint(&original, codec).map_err(repo)?
            == read("courier-private-pending-envelope-01.bin", 1048576)?,
    )?;
    Ok(())
}
async fn phase_b(
    service: &Service,
    client: &Client,
    display: [u8; 32],
    codec: df_persistence::NativeCodecLimits,
    reductions: &std::sync::atomic::AtomicUsize,
) -> Result<(), Error> {
    let (start, count, _) = metadata("courier-private-metadata-a-01.bin")?;
    let mut calls = Calls {
        count: usize::from(count),
        started_milliseconds: start,
    };
    let (credentials, members) = private_grants()?;
    let member = df_types::MemberId::from_bytes(&members[0])
        .map_err(|error| io::Error::other(format!("courier member {error:?}")))?;
    // Observe ordinary startup publication before any Actor or generated Service input.
    // A failed startup cannot be rescued by the view calls that follow this proof.
    let startup = timeout(Duration::from_secs(5), async {
        let mut published = service.updates.clone();
        loop {
            let checkpoint = published.borrow().clone();
            required(reductions.load(std::sync::atomic::Ordering::SeqCst) <= 1)?;
            if checkpoint.state().intents.first().is_some_and(|intent| {
                intent.kind == EffectKind::RunAi && intent.status == DurableStatus::Completed
            }) {
                verify_completed(&checkpoint, codec, member)?;
                required(reductions.load(std::sync::atomic::Ordering::SeqCst) == 1)?;
                return Ok::<_, Error>(checkpoint);
            }
            published.changed().await?;
        }
    })
    .await??;
    required(calls.count == 9)?;
    let views = three_views(service, credentials, members, display, &mut calls, true).await?;
    let snapshot = snapshot(service, &mut calls).await?;
    required(snapshot.calls_remaining == 32 - calls.count as u16)?;
    required(snapshot.checkpoint == startup)?;
    verify_completed(&snapshot.checkpoint, codec, member)?;
    stale_release(client, &snapshot.checkpoint).await?;
    let (before, rows) = physical(client, 0xc6).await?;
    required(before.counts[0] == 7 && before.counts[2] == 8 && before.counts[3] == 1)?;
    required(
        packet_rows(&before.facts)? == read("courier-private-pending-facts-01.bin", 4 * 1048576)?
            && packet_rows(&before.intents)?
                == read("courier-private-pending-intents-01.bin", 4 * 1048576)?,
    )?;
    let originals = retained_rows("courier-private-pending-operations-01.bin")?;
    required(
        before.operations.len() == originals.len() + 1
            && originals.iter().all(|old| before.operations.contains(old)),
    )?;
    let old_checkpoints = retained_rows("courier-private-pending-checkpoints-01.bin")?;
    required(
        before.checkpoints.len() == old_checkpoints.len() + 1
            && old_checkpoints
                .iter()
                .all(|old| before.checkpoints.contains(old)),
    )?;
    required(
        before.envelope
            == local_demo_scope::encode_owned_demo_checkpoint(&snapshot.checkpoint, codec)
                .map_err(repo)?,
    )?;
    let operation = courier_ai::completion_operation(&snapshot.checkpoint.state().intents[0])
        .map_err(fixture)?;
    let row=client.query_one("SELECT principal_id,command_namespace FROM df_game.operations WHERE tenant_id=$1 AND session_id=$2 AND operation_id=$3",
        &[&local_demo_scope::TENANT.as_slice(),&snapshot.checkpoint.basis().session.as_bytes().as_slice(),&operation.as_bytes().as_slice()]).await?;
    required(
        row.try_get::<_, Vec<u8>>("principal_id")? == members[0]
            && row.try_get::<_, Vec<u8>>("command_namespace")? == b"local-courier-completion-v1",
    )?;
    let request =
        rpc::SubmitActionRequest::decode(read("courier-private-request-01.bin", 8192)?.as_slice())?;
    let mut receipt = committed(submit(service, &request, credentials[0], &mut calls).await?)?;
    required(receipt.replayed)?;
    receipt.replayed = false;
    required(receipt.encode_to_vec() == read("courier-private-receipt-01.bin", 8192)?)?;
    let (after, after_rows) = physical(client, 0xc6).await?;
    required(before == after && rows == after_rows && calls.count == 14)?;
    save(
        "courier-private-completed-envelope-01.bin",
        &before.envelope,
    )?;
    save("courier-private-completed-rows-01.bin", &rows)?;
    for (index, view) in views.iter().enumerate() {
        save(
            &format!("courier-private-completed-view-{index}-01.bin"),
            view,
        )?;
    }
    save_metadata("courier-private-metadata-b-01.bin", &calls)?;
    required(reductions.load(std::sync::atomic::Ordering::SeqCst) == 1)?;
    report(
        "courier-b-proof-01.json",
        &format!(
            "{{\"pass\":true,\"pid\":{},\"aggregate_actor_calls\":14,\"native_engine_reductions\":1,\"native_startup_pending_completed_once\":true,\"startup_completion_observed_before_any_actor_input\":true,\"startup_publication_wait_milliseconds_limit\":5000,\"source_sibling_state_equal\":true,\"immutable_intent_and_fact_rows_equal\":true,\"one_completion_operation_and_checkpoint\":true,\"current_native_recipient_namespace_verified\":true,\"exact_old_ask_receipt_replayed\":true,\"retry_four_families_equal\":true,\"three_private_audiences_correct\":true,\"stale_fence_release_refused\":true,\"process_restart_verified\":true}}",
            std::process::id()
        ),
    )?;
    Ok(())
}
async fn phase_c(
    service: &Service,
    client: &Client,
    display: [u8; 32],
    codec: df_persistence::NativeCodecLimits,
    reductions: &std::sync::atomic::AtomicUsize,
) -> Result<(), Error> {
    let (start, count, _) = metadata("courier-private-metadata-b-01.bin")?;
    let mut calls = Calls {
        count: usize::from(count),
        started_milliseconds: start,
    };
    let (credentials, members) = private_grants()?;
    let views = three_views(service, credentials, members, display, &mut calls, true).await?;
    for (index, view) in views.iter().enumerate() {
        required(
            *view
                == read(
                    &format!("courier-private-completed-view-{index}-01.bin"),
                    8192,
                )?,
        )?;
    }
    let snapshot = snapshot(service, &mut calls).await?;
    required(snapshot.calls_remaining == 32 - calls.count as u16 && calls.count == 18)?;
    verify_completed(
        &snapshot.checkpoint,
        codec,
        df_types::MemberId::from_bytes(&members[0])
            .map_err(|error| io::Error::other(format!("courier member {error:?}")))?,
    )?;
    let (state, rows) = physical(client, 0xc6).await?;
    required(
        rows == read("courier-private-completed-rows-01.bin", 4 * 1048576)?
            && state.envelope == read("courier-private-completed-envelope-01.bin", 1048576)?
            && state.envelope
                == local_demo_scope::encode_owned_demo_checkpoint(&snapshot.checkpoint, codec)
                    .map_err(repo)?,
    )?;
    required(reductions.load(std::sync::atomic::Ordering::SeqCst) == 0)?;
    report(
        "courier-c-proof-01.json",
        &format!(
            "{{\"pass\":true,\"pid\":{},\"aggregate_actor_calls\":18,\"native_engine_reductions\":0,\"completed_restart_no_new_decision_or_rows\":true,\"canonical_envelope_equal\":true,\"three_views_equal\":true,\"process_restart_verified\":true}}",
            std::process::id()
        ),
    )?;
    Ok(())
}
pub(super) async fn run(
    service: Service,
    display: [u8; 32],
    codec: df_persistence::NativeCodecLimits,
    database: Config,
    phase: Phase,
    reductions: std::sync::Arc<std::sync::atomic::AtomicUsize>,
) -> Result<(), io::Error> {
    configured()?;
    if phase == Phase::A {
        return timeout(Duration::from_secs(45), phase_a(&service, display, codec))
            .await
            .map_err(io::Error::other)?
            .map_err(io::Error::other);
    }
    let (client, connection) = timeout(Duration::from_secs(2), database.connect(NoTls))
        .await
        .map_err(io::Error::other)?
        .map_err(io::Error::other)?;
    let driver = tokio::spawn(connection);
    let result = timeout(Duration::from_secs(45), async {
        match phase {
            Phase::B => phase_b(&service, &client, display, codec, reductions.as_ref()).await,
            Phase::C => phase_c(&service, &client, display, codec, reductions.as_ref()).await,
            Phase::A => Err(io::Error::other("courier phase mismatch").into()),
        }
    })
    .await;
    drop(client);
    super::join_grant_driver(driver).await?;
    result
        .map_err(io::Error::other)?
        .map_err(io::Error::other)?;
    required(
        reductions.load(std::sync::atomic::Ordering::SeqCst)
            == if phase == Phase::B { 1 } else { 0 },
    )
}
pub(super) fn closed(phase: Phase, remaining: u16, reductions: usize) -> Result<(), io::Error> {
    let (start, _, _) = metadata("courier-private-metadata-a-01.bin")?;
    let (name, expected) = match phase {
        Phase::A => ("courier-a-closed-01.json", 23),
        Phase::B => ("courier-b-closed-01.json", 18),
        Phase::C => ("courier-c-closed-01.json", 14),
    };
    required(
        remaining == expected
            && elapsed(start)? <= 90_000
            && reductions
                == match phase {
                    Phase::A => 6,
                    Phase::B => 1,
                    Phase::C => 0,
                },
    )?;
    report(
        name,
        &format!(
            "{{\"pass\":true,\"pid\":{},\"aggregate_actor_calls\":{},\"actor_calls_limit\":32,\"native_engine_reductions\":{reductions},\"exercise_milliseconds\":{},\"total_milliseconds_limit\":90000,\"admission_stopped\":true,\"inbox_drained\":true,\"exact_fence_released\":true,\"actor_joined\":true,\"repository_driver_joined\":true,\"grant_driver_joined\":true}}",
            std::process::id(),
            32 - remaining,
            elapsed(start)?
        ),
    )
}
