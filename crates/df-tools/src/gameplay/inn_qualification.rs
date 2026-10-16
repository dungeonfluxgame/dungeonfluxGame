//! Private operator proof through the real generated service in two real native processes.
//! Phase files live only in the Root-owned protected scratch directory; never RPC input.
use super::{Service, actor, hex, journey, wire};
use df_model::checkpoint::{Checkpoint, DurableStatus, GameState};
use df_persistence::local_demo_scope;
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
    result.map_err(|_| io::Error::other("inn canonical source invalid"))
}
fn victory(state: &GameState) -> Result<bool, io::Error> {
    source(journey::combat_victory(state))
}
fn current_phase(checkpoint: &Checkpoint) -> Result<rpc::JourneyPhase, io::Error> {
    source(journey::phase(checkpoint))
}
const ROOT: &str = "/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/engine-inn-20261005";
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Phase {
    A,
    B,
}
pub(super) fn phase() -> Option<Phase> {
    match std::env::args().nth(4).as_deref() {
        Some("--inn-qualification-a") => Some(Phase::A),
        Some("--inn-qualification-b") => Some(Phase::B),
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
            "inn assertion {}:{}",
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
        .ok_or_else(|| io::Error::other("inn clock regressed"))
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
        std::env::var("DF_INN_QUALIFICATION").as_deref() == Ok("owned-loopback-inn01")
            && std::env::var("DF_GAMEPLAY_DEMO_DATABASE").as_deref()
                == Ok("df_gameplay_demo_20261005_inn01"),
    )
}
fn metadata() -> Result<(u64, usize, u8, u32), io::Error> {
    let bytes = read("inn-private-metadata-01.bin", 15)?;
    required(bytes.len() == 15)?;
    Ok((
        u64::from_be_bytes(bytes[..8].try_into().map_err(io::Error::other)?),
        usize::from(u16::from_be_bytes(
            bytes[8..10].try_into().map_err(io::Error::other)?,
        )),
        bytes[10],
        u32::from_be_bytes(bytes[11..15].try_into().map_err(io::Error::other)?),
    ))
}
pub(super) fn call_budget(phase: Phase) -> Result<u16, io::Error> {
    configured()?;
    if phase == Phase::A {
        return Ok(32);
    }
    let (start, calls, _, pid) = metadata()?;
    required(calls <= 25 && elapsed(start)? <= 90_000 && pid != std::process::id())?;
    required(read("inn-private-build-01.bin", 96)? == crate::BUILD_ID.as_bytes())?;
    required(read("inn-a-closed-01.json", 16 * 1024)?.starts_with(b"{\"pass\":true,"))?;
    Ok(32 - calls as u16)
}
pub(super) fn record_owner(phase: Phase, fence: [u8; 16]) -> Result<(), io::Error> {
    if phase == Phase::A {
        save("inn-private-old-fence-01.bin", &fence)?;
    }
    Ok(())
}
async fn stale_release(client: &Client, checkpoint: &Checkpoint) -> Result<(), Error> {
    let fence: [u8; 16] = read("inn-private-old-fence-01.bin", 16)?
        .try_into()
        .map_err(|_| io::Error::other("old fence length"))?;
    let result = local_demo_scope::release_owner(client, checkpoint.basis().session, fence).await;
    required(result == Err(df_session::submission::RepositoryError::RevisionConflict))?;
    let row=client.query_one("SELECT owner_fence,lease_until>clock_timestamp() AS live FROM df_game.sessions WHERE tenant_id=$1::bytea AND session_id=$2::bytea", &[&local_demo_scope::TENANT.as_slice(),&checkpoint.basis().session.as_bytes().as_slice()]).await?;
    let replacement: Vec<u8> = row.try_get("owner_fence")?;
    required(replacement.len() == 16 && replacement != fence && row.try_get::<_, bool>("live")?)?;
    Ok(())
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
        name: "Inn-Fighter".to_owned(),
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
        .ok_or_else(|| io::Error::other("inn first watch absent"))??;
    let bytes = first.encode_to_vec();
    drop(stream); // The actual unfold owns no spawned worker; this also releases its stream permit.
    required(service.streams.available_permits() == 6)?;
    Ok(bytes)
}
type RetainedGrants = ([[u8; 32]; 2], [[u8; 16]; 2]);
fn private_grants() -> Result<RetainedGrants, Error> {
    let bytes = read("inn-private-grants-01.bin", 96)?;
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
                    .map_err(|_| io::Error::other("inn member identity invalid"))?,
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

    required(spent && strikes <= 4 && ends <= 3 && victory(service.updates.borrow().state())?)?;
    let original_operation = operation - 1;
    let before = snapshot(service, &mut calls).await?;
    required(before.checkpoint == service.updates.borrow().clone())?;
    let mut eligible = None;
    for (index, member) in members.iter().enumerate() {
        let member = df_types::MemberId::from_bytes(member)
            .map_err(|_| io::Error::other("inn member identity"))?;
        if source(journey::offered(&before.checkpoint, member))?
            .iter()
            .any(|(kind, _)| *kind == rpc::GameplayActionKind::ChooseHarborScene)
        {
            eligible = Some(index);
            break;
        }
    }
    let eligible =
        eligible.ok_or_else(|| io::Error::other("current conscious victory author absent"))?;
    let mut body = action(
        &before.checkpoint,
        0xc1,
        rpc::GameplayActionKind::ChooseHarborScene,
    );
    body.destination = rpc::HarborDestination::HarborInn as i32;
    let mut forged = body.clone();
    forged.operation_id = Some(rpc::OperationId {
        value: Some(vec![0xbf; 16]),
    });
    forged.destination = rpc::HarborDestination::LoadingPier as i32;
    refused(
        submit(service, &forged, credentials[eligible], &mut calls).await?,
        rpc::RejectionCode::InvalidSelection,
    )?;
    required(service.updates.borrow().clone() == before.checkpoint)?;
    let (authority_before, authority_rows) = physical(client, original_operation).await?;
    let denied = submit(service, &body, display, &mut calls).await;
    required(
        denied
            .as_ref()
            .is_err_and(|error| error.code() == tonic::Code::PermissionDenied),
    )?;
    let (authority_after, authority_after_rows) = physical(client, original_operation).await?;
    required(authority_before == authority_after && authority_rows == authority_after_rows)?;
    let receipt = committed(submit(service, &body, credentials[eligible], &mut calls).await?)?;
    required(!receipt.replayed)?;
    let arrived = service.updates.borrow().clone();
    preserved(&before.checkpoint, &arrived)?;
    let (accepted_state, accepted_rows) = physical(client, 0xc1).await?;
    required(
        accepted_state.counts[0] == authority_before.counts[0] + 1
            && accepted_state.counts[1] == authority_before.counts[1] + 1
            && accepted_state.counts[2] == authority_before.counts[2] + 1
            && accepted_state.counts[3] == authority_before.counts[3],
    )?;
    let mut replay = committed(submit(service, &body, credentials[eligible], &mut calls).await?)?;
    required(replay.replayed)?;
    replay.replayed = false;
    required(replay == receipt)?;
    let (retried, retry_rows) = physical(client, 0xc1).await?;
    required(retried == accepted_state && retry_rows == accepted_rows)?;
    let mut stale = body.clone();
    stale.operation_id = Some(rpc::OperationId {
        value: Some(vec![0xc2; 16]),
    });
    refused(
        submit(service, &stale, credentials[eligible], &mut calls).await?,
        rpc::RejectionCode::StaleOffer,
    )?;
    let captured = snapshot(service, &mut calls).await?;
    required(
        captured.checkpoint == arrived && captured.calls_remaining == 32 - calls.count as u16,
    )?;
    let views = three_views(service, [first, second], members, display, &mut calls).await?;
    for view in &views {
        verify_view(view)?;
    }
    required(calls.count <= 25 && last_receipt.is_some())?;
    let (state, rows) = physical(client, 0xc1).await?;
    required(
        state.envelope
            == local_demo_scope::encode_owned_demo_checkpoint(&arrived, codec)
                .map_err(|_| io::Error::other("inn envelope encoding"))?,
    )?;
    required(
        state.facts == accepted_state.facts
            && state.intents == accepted_state.intents
            && state.envelope == accepted_state.envelope,
    )?;
    let mut grants = Vec::from(first);
    grants.extend_from_slice(&second);
    grants.extend_from_slice(&members[0]);
    grants.extend_from_slice(&members[1]);
    let mut metadata = Vec::from(calls.started_milliseconds.to_be_bytes());
    metadata.extend_from_slice(&(calls.count as u16).to_be_bytes());
    metadata.push(eligible as u8);
    metadata.extend_from_slice(&std::process::id().to_be_bytes());
    for (name, bytes) in [
        ("inn-private-grants-01.bin", grants),
        ("inn-private-metadata-01.bin", metadata),
        (
            "inn-private-build-01.bin",
            crate::BUILD_ID.as_bytes().to_vec(),
        ),
        ("inn-private-request-01.bin", body.encode_to_vec()),
        ("inn-private-receipt-01.bin", receipt.encode_to_vec()),
        ("inn-private-envelope-01.bin", state.envelope),
        ("inn-private-rows-01.bin", rows),
    ] {
        save(name, &bytes)?;
    }
    for (index, view) in views.iter().enumerate() {
        save(&format!("inn-private-view-{index}-01.bin"), view)?;
    }
    save("inn-a-ready-body-01.bin", format!(
        "{{\"pass\":true,\"actor_calls\":{},\"pid\":{},\"exercise_milliseconds\":{},\"genuine_victory_source\":true,\"current_authorized_party\":true,\"inn_committed\":true,\"mechanical_time_knowledge_packet_preserved\":true,\"exact_retry_equal\":true,\"four_families_retry_equal\":true,\"forged_destination_refused\":true,\"display_authority_refused\":true,\"stale_offer_refused\":true,\"private_views_correct\":true,\"watches_closed\":true}}",
        calls.count, std::process::id(), elapsed(calls.started_milliseconds)?
    ).as_bytes())?;
    Ok(())
}

fn preserved(before: &Checkpoint, after: &Checkpoint) -> Result<(), Error> {
    required(
        after.basis().revision
            == before
                .basis()
                .revision
                .next_sequence()
                .map_err(|_| io::Error::other("inn revision"))?
            && before.pins() == after.pins(),
    )?;
    let mut comparable = after.state().clone();
    comparable.facts = before.state().facts.clone();
    comparable.decisions = before.state().decisions.clone();
    comparable.narrative = before.state().narrative.clone();
    required(&comparable == before.state())?;
    required(
        after.state().facts.len() == before.state().facts.len() + 1
            && after.state().decisions.len() == before.state().decisions.len() + 1,
    )?;
    required(
        after.state().narrative.open_threads == before.state().narrative.open_threads
            && after.state().narrative.active_beats
                == [source(super::model::content("harbor-inn"))?]
            && after
                .state()
                .narrative
                .open_threads
                .contains(&source(super::model::content(
                    "sealed-packet-delivery-thread",
                ))?),
    )?;
    let decision = after
        .state()
        .decisions
        .last()
        .ok_or_else(|| io::Error::other("inn decision absent"))?;
    required(
        source(journey::accepted(decision))?.scene == Some(journey::inn_scene())
            && decision.draws.is_empty()
            && decision.effects.is_empty(),
    )?;
    required(
        after.state().intents.len() == 1
            && after
                .state()
                .intents
                .iter()
                .all(|intent| intent.status == DurableStatus::Completed),
    )?;
    Ok(())
}
fn verify_view(bytes: &[u8]) -> Result<(), Error> {
    let view = rpc::ViewMessage::decode(bytes)?;
    let (scene, narration, party) = match view.audience {
        Some(rpc::view_message::Audience::Player(view)) => {
            required(!view.offers.iter().any(|offer| {
                offer.action_kind == rpc::GameplayActionKind::ChooseHarborScene as i32
            }))?;
            (
                view.scene,
                view.narration,
                view.journey.map(|journey| journey.party),
            )
        }
        Some(rpc::view_message::Audience::Display(view)) => (
            view.scene,
            view.narration,
            view.journey.map(|journey| journey.party),
        ),
        None => return Err(io::Error::other("inn audience absent").into()),
    };
    required(
        scene == Some(journey::inn_scene())
            && party.is_some_and(|party| party.len() == 2)
            && !narration.contains("Vell"),
    )?;
    Ok(())
}
fn refused(value: rpc::SubmitActionResponse, expected: rpc::RejectionCode) -> Result<(), Error> {
    let code = match value.outcome {
        Some(rpc::submit_action_response::Outcome::CommittedDecision(receipt)) => {
            match receipt.outcome {
                Some(rpc::decision_receipt::Outcome::Rejected(rejection)) => rejection.code,
                _ => return Err(io::Error::other("inn expected typed rejection").into()),
            }
        }
        Some(rpc::submit_action_response::Outcome::OperationObservation(observation)) => {
            observation.code
        }
        None => return Err(io::Error::other("inn rejection absent").into()),
    };
    required(code == expected as i32)?;
    Ok(())
}

async fn phase_b(
    service: &Service,
    client: &Client,
    display: [u8; 32],
    codec: df_persistence::NativeCodecLimits,
) -> Result<(), Error> {
    let (start, count, eligible, old_pid) = metadata()?;
    required(old_pid != std::process::id() && eligible < 2)?;
    let restored = service.updates.borrow().clone();
    let envelope = local_demo_scope::encode_owned_demo_checkpoint(&restored, codec)
        .map_err(|_| io::Error::other("inn restored encoding"))?;
    required(envelope == read("inn-private-envelope-01.bin", 1048576)?)?;
    required(
        restored.state().narrative.active_beats == [source(super::model::content("harbor-inn"))?]
            && restored
                .state()
                .intents
                .iter()
                .all(|intent| intent.status == DurableStatus::Completed),
    )?;
    stale_release(client, &restored).await?;
    let (before, rows) = physical(client, 0xc1).await?;
    required(rows == read("inn-private-rows-01.bin", 4 * 1048576)?)?;
    let mut calls = Calls {
        count,
        started_milliseconds: start,
    };
    let (credentials, members) = private_grants()?;
    let views = three_views(service, credentials, members, display, &mut calls).await?;
    for (index, view) in views.iter().enumerate() {
        verify_view(view)?;
        required(*view == read(&format!("inn-private-view-{index}-01.bin"), 8192)?)?;
    }
    let request =
        rpc::SubmitActionRequest::decode(read("inn-private-request-01.bin", 8192)?.as_slice())?;
    let receipt =
        rpc::DecisionReceipt::decode(read("inn-private-receipt-01.bin", 8192)?.as_slice())?;
    let mut replay = committed(
        submit(
            service,
            &request,
            credentials[usize::from(eligible)],
            &mut calls,
        )
        .await?,
    )?;
    required(replay.replayed)?;
    replay.replayed = false;
    required(replay == receipt)?;
    let captured = snapshot(service, &mut calls).await?;
    required(
        captured.checkpoint == restored && captured.calls_remaining == 32 - calls.count as u16,
    )?;
    let (retried, retry_rows) = physical(client, 0xc1).await?;
    required(
        before == retried && rows == retry_rows && calls.count == count + 5 && calls.count <= 32,
    )?;
    save(
        "inn-private-final-count-01.bin",
        &(calls.count as u16).to_be_bytes(),
    )?;
    save("inn-b-proof-body-01.bin", format!(
        "{{\"pass\":true,\"actor_calls\":{},\"pid\":{},\"previous_pid\":{},\"exercise_milliseconds\":{},\"fresh_executable_verified\":true,\"startup_inn_observed_before_actor_input\":true,\"canonical_equal\":true,\"four_families_equal\":true,\"private_views_equal\":true,\"exact_retry_equal\":true,\"no_new_event_draw_time\":true,\"stale_release_refused\":true,\"watches_closed\":true,\"inspector_driver_joined\":true}}",
        calls.count,std::process::id(),old_pid,elapsed(start)?
    ).as_bytes())?;
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
        .map_err(|_| io::Error::other("inn inspector connect deadline"))?
        .map_err(io::Error::other)?;
    let driver = tokio::spawn(connection);
    let result = timeout(Duration::from_secs(45), async {
        match phase {
            Phase::A => phase_a(&service, &client, display, codec).await,
            Phase::B => phase_b(&service, &client, display, codec).await,
        }
    })
    .await;
    drop(client);
    super::join_grant_driver(driver).await?;
    result
        .map_err(|_| io::Error::other("inn exercise deadline"))?
        .map_err(|error| io::Error::other(format!("inn phase failed: {error}")))?;
    let (body, name) = match phase {
        Phase::A => ("inn-a-ready-body-01.bin", "inn-a-ready-01.json"),
        Phase::B => ("inn-b-proof-body-01.bin", "inn-b-proof-01.json"),
    };
    let bytes = read(body, 16 * 1024)?;
    report(name, std::str::from_utf8(&bytes).map_err(io::Error::other)?)
}
pub(super) fn closed(phase: Phase, remaining: u16) -> Result<(), io::Error> {
    let (start, count, _, _) = metadata()?;
    let (expected, name) = match phase {
        Phase::A => (count, "inn-a-closed-01.json"),
        Phase::B => (
            usize::from(u16::from_be_bytes(
                read("inn-private-final-count-01.bin", 2)?
                    .try_into()
                    .map_err(|_| io::Error::other("inn final count invalid"))?,
            )),
            "inn-b-closed-01.json",
        ),
    };
    required(expected <= 32 && remaining == 32 - expected as u16 && elapsed(start)? <= 90000)?;
    report(
        name,
        &format!(
            "{{\"pass\":true,\"actor_calls\":{},\"exercise_milliseconds\":{},\"admission_stopped\":true,\"actor_drained\":true,\"exact_fence_released\":true,\"actor_joined\":true,\"repository_driver_joined\":true,\"grant_driver_joined\":true}}",
            expected,
            elapsed(start)?
        ),
    )
}
