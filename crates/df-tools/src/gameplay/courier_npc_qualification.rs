//! Private, finite native proof of the courier's accepted contact and durable reaction.
//! Both phases use the same fresh owned PostgreSQL data through the real Session owner.
use super::{Service, actor, hex, journey, model, wire};
use df_model::checkpoint::{AudienceScope, Checkpoint, FactValue, GameFact};
use df_persistence::local_demo_scope;
use df_protocol::common as rpc;
use prost::Message;
use std::{
    io,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{sync::oneshot, time::timeout};
use tokio_postgres::{Client, Config, NoTls};
use tonic::Request;

type Error = Box<dyn std::error::Error + Send + Sync>;
const ROOT: &str =
    "/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/wave24-courier-durable-20261005";
const CALL_LIMIT: usize = 20;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Phase {
    A,
    B,
}

pub(super) fn phase() -> Option<Phase> {
    match std::env::args().nth(4).as_deref() {
        Some("--courier-npc-qualification-a") => Some(Phase::A),
        Some("--courier-npc-qualification-b") => Some(Phase::B),
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
            "courier NPC assertion {}:{}",
            at.file(),
            at.line()
        )))
    }
}

fn source<T>(value: Result<T, df_session::submission::RepositoryError>) -> Result<T, io::Error> {
    value.map_err(|_| io::Error::other("courier NPC source binding refused"))
}

fn repo(error: df_session::submission::RepositoryError) -> io::Error {
    io::Error::other(format!("courier NPC repository {error:?}"))
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
        .ok_or_else(|| io::Error::other("courier NPC clock regressed"))
}

fn validate_qualification_root(
    base: &std::path::Path,
    supplied: Option<&std::ffi::OsStr>,
) -> Result<std::path::PathBuf, io::Error> {
    let supplied = supplied
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| io::Error::other("courier NPC qualification root missing or non-Unicode"))?;
    let candidate = std::path::Path::new(supplied);
    let name = candidate
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| io::Error::other("courier NPC qualification root name invalid"))?;
    let digits = name
        .strip_prefix("attempt-")
        .ok_or_else(|| io::Error::other("courier NPC qualification root name invalid"))?;
    let attempt = digits
        .parse::<u16>()
        .map_err(|_| io::Error::other("courier NPC qualification attempt invalid"))?;
    required(
        base.is_absolute()
            && candidate.parent() == Some(base)
            && supplied == format!("{}{}{}", base.display(), std::path::MAIN_SEPARATOR, name)
            && !digits.is_empty()
            && digits.len() <= 4
            && digits.bytes().all(|byte| byte.is_ascii_digit())
            && (1..=9999).contains(&attempt),
    )?;
    let base_identity = crate::file_identity::snapshot_path(base)?;
    let candidate_identity = crate::file_identity::snapshot_path(candidate)?;
    required(base_identity.is_private_dir() && candidate_identity.is_private_dir())?;
    Ok(candidate.to_path_buf())
}

fn qualification_root() -> Result<std::path::PathBuf, io::Error> {
    let supplied = std::env::var_os("DF_COURIER_NPC_QUALIFICATION_ROOT");
    validate_qualification_root(std::path::Path::new(ROOT), supplied.as_deref())
}

fn path(name: &str) -> Result<std::path::PathBuf, io::Error> {
    use std::path::Component;
    let mut components = std::path::Path::new(name).components();
    required(
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none(),
    )?;
    Ok(qualification_root()?.join(name))
}

fn save(name: &str, bytes: &[u8]) -> Result<(), io::Error> {
    use std::io::Write;
    required(bytes.len() <= 4 * 1024 * 1024)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path(name)?)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn read(name: &str, maximum: u64) -> Result<Vec<u8>, io::Error> {
    let location = path(name)?;
    let metadata = std::fs::symlink_metadata(&location)?;
    required(metadata.is_file() && metadata.len() <= maximum)?;
    std::fs::read(location)
}

fn report(name: &str, body: &str) -> Result<(), io::Error> {
    let location = path(name)?;
    required(body.len() <= 16 * 1024 && !location.exists())?;
    let temporary = format!("{name}.pending");
    save(&temporary, body.as_bytes())?;
    std::fs::rename(path(&temporary)?, location)
}

fn configured() -> Result<(), io::Error> {
    required(
        std::env::var("DF_COURIER_NPC_QUALIFICATION").as_deref()
            == Ok("owned-loopback-courier-npc01")
            && std::env::var("DF_GAMEPLAY_DEMO_DATABASE").as_deref()
                == Ok("df_gameplay_demo_20261005_courier_npc01"),
    )?;
    qualification_root()?;
    Ok(())
}

fn metadata() -> Result<(u64, usize, u32), io::Error> {
    let bytes = read("courier-npc-private-metadata-01.bin", 14)?;
    required(bytes.len() == 14)?;
    Ok((
        u64::from_be_bytes(bytes[..8].try_into().map_err(io::Error::other)?),
        usize::from(u16::from_be_bytes(
            bytes[8..10].try_into().map_err(io::Error::other)?,
        )),
        u32::from_be_bytes(bytes[10..14].try_into().map_err(io::Error::other)?),
    ))
}

pub(super) fn call_budget(phase: Phase) -> Result<u16, io::Error> {
    configured()?;
    if phase == Phase::A {
        return Ok(CALL_LIMIT as u16);
    }
    let (start, count, pid) = metadata()?;
    required(count == 8 && pid != std::process::id() && elapsed(start)? <= 90_000)?;
    required(read("courier-npc-private-build-01.bin", 96)? == crate::BUILD_ID.as_bytes())?;
    required(read("courier-npc-a-closed-01.json", 16 * 1024)?.starts_with(b"{\"pass\":true,"))?;
    Ok(CALL_LIMIT as u16 - count as u16)
}

pub(super) fn record_owner(phase: Phase, fence: [u8; 16]) -> Result<(), io::Error> {
    if phase == Phase::A {
        save("courier-npc-private-old-fence-01.bin", &fence)?;
    }
    Ok(())
}

struct Calls {
    count: usize,
    started_milliseconds: u64,
}

fn admit(calls: &mut Calls) -> Result<(), Error> {
    required(calls.count < CALL_LIMIT && elapsed(calls.started_milliseconds)? <= 90_000)?;
    calls.count += 1;
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
        return Err(io::Error::other("courier NPC room join refused").into());
    };
    let bytes = joined.local_binding.as_bytes();
    required(bytes.len() == 64)?;
    let mut credential = [0; 32];
    for (index, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
        let digit = |byte| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        };
        credential[index] =
            digit(pair[0]).ok_or_else(|| io::Error::other("courier NPC grant encoding"))? * 16
                + digit(pair[1]).ok_or_else(|| io::Error::other("courier NPC grant encoding"))?;
    }
    Ok(credential)
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

fn create(checkpoint: &Checkpoint, operation: u8) -> rpc::SubmitActionRequest {
    let mut body = action(
        checkpoint,
        operation,
        rpc::GameplayActionKind::CreateCharacter,
    );
    body.character = Some(rpc::CharacterSelection {
        identity: None,
        name: "Courier-NPC-Fighter".to_owned(),
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

async fn submit(
    service: &Service,
    body: &rpc::SubmitActionRequest,
    credential: [u8; 32],
    calls: &mut Calls,
) -> Result<rpc::SubmitActionResponse, tonic::Status> {
    admit(calls).map_err(|_| tonic::Status::resource_exhausted("courier NPC call bound"))?;
    let decoded = rpc::SubmitActionRequest::decode(body.encode_to_vec().as_slice())
        .map_err(|_| tonic::Status::internal("courier NPC encoding"))?;
    let mut request = Request::new(decoded);
    request.metadata_mut().insert(
        "x-df-local-binding",
        hex(&credential)
            .parse()
            .map_err(|_| tonic::Status::internal("courier NPC metadata"))?,
    );
    rpc::action_service_server::ActionService::submit(service, request)
        .await
        .map(|value| value.into_inner())
}

fn committed(value: rpc::SubmitActionResponse) -> Result<rpc::DecisionReceipt, Error> {
    match value.outcome {
        Some(rpc::submit_action_response::Outcome::CommittedDecision(value)) => Ok(*value),
        _ => Err(io::Error::other("courier NPC committed receipt absent").into()),
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
        .map_err(|_| io::Error::other("courier NPC snapshot admission"))?;
    Ok(timeout(Duration::from_secs(5), wait).await???)
}

fn contact(
    checkpoint: &Checkpoint,
    hero: df_model::checkpoint::EntityId,
) -> Result<&GameFact, Error> {
    let expected = source(model::content("courier-escort-contact"))?;
    let mut contacts = checkpoint.state().facts.iter().filter(|fact| {
        matches!(&fact.value, FactValue::ContentEvent { definition, subjects }
            if *definition == expected && subjects.as_slice() == [hero])
    });
    let found = contacts
        .next()
        .ok_or_else(|| io::Error::other("accepted courier contact absent"))?;
    required(contacts.next().is_none())?;
    required(found.operation.as_bytes() == &[0xd6; 16] && found.audience == AudienceScope::Shared)?;
    Ok(found)
}

fn verify_npc(
    checkpoint: &Checkpoint,
    hero: df_model::checkpoint::EntityId,
    other: df_model::checkpoint::EntityId,
    defended: bool,
) -> Result<(), Error> {
    let state = checkpoint.state();
    let courier = source(journey::entity([0x67; 16]))?;
    let contact = contact(checkpoint, hero)?;
    let escort_definition = source(model::content("escort-courier"))?;
    let reaction_definition = source(model::content("courier-escort-reaction"))?;
    let courier_definition = source(model::content("lantern-wharf-courier"))?;
    required(
        checkpoint.pins() == &source(model::pins())?
            && state
                .entities
                .iter()
                .any(|entity| entity.id == courier && entity.definition == courier_definition),
    )?;
    let npc = state
        .continuity
        .npcs
        .iter()
        .find(|npc| npc.entity == courier)
        .ok_or_else(|| io::Error::other("source courier absent"))?;
    required(
        state.continuity.npcs.len() == 1
            && npc.personality == source(model::content("cautious-courier"))?
            && npc.motivations == [source(model::content("deliver-dispatch"))?]
            && npc.known_facts == [contact.id]
            && state.continuity.witnesses.len() == 1,
    )?;
    let witness = &state.continuity.witnesses[0];
    required(
        witness.observer == courier
            && witness.fact == contact.id
            && witness.source == source(model::content("courier-escort-perception"))?,
    )?;
    let escort = state
        .facts
        .iter()
        .find(|fact| {
            fact.operation == contact.operation
                && fact.cause == Some(contact.id)
                && matches!(&fact.value, FactValue::ContentEvent { definition, subjects }
                    if *definition == escort_definition && subjects.is_empty())
        })
        .ok_or_else(|| io::Error::other("terminal escort fact absent"))?;
    required(escort.audience == AudienceScope::Shared)?;
    let escort_decision = state
        .decisions
        .iter()
        .find(|decision| decision.operation == contact.operation)
        .ok_or_else(|| io::Error::other("escort decision absent"))?;
    required(
        escort_decision.facts.contains(&contact.id)
            && escort_decision.facts.contains(&escort.id)
            && escort_decision.draws.is_empty()
            && escort_decision.effects.is_empty(),
    )?;
    required(state.relationships.len() == 2)?;
    for (target, label) in [
        (
            hero,
            if defended {
                "escort-supported"
            } else {
                "unfamiliar"
            },
        ),
        (other, "unfamiliar"),
    ] {
        let mut relationships = state.relationships.iter().filter(|relationship| {
            relationship.subject == courier && relationship.object == target
        });
        let relationship = relationships
            .next()
            .ok_or_else(|| io::Error::other("directional courier relationship absent"))?;
        required(
            relationships.next().is_none()
                && relationship.policy == source(model::content("courier-escort-relationship"))?
                && relationship.state == source(model::label(label))?,
        )?;
    }
    let reactions: Vec<_> = state
        .facts
        .iter()
        .filter(|fact| {
            matches!(&fact.value, FactValue::ContentEvent { definition, .. }
            if *definition == reaction_definition)
        })
        .collect();
    if defended {
        required(reactions.len() == 1)?;
        let reaction = reactions[0];
        required(
            reaction.operation.as_bytes() == &[0xd7; 16]
                && reaction.cause == Some(contact.id)
                && reaction.audience == AudienceScope::Shared
                && matches!(&reaction.value, FactValue::ContentEvent { subjects, .. }
                    if subjects.as_slice() == [courier, hero]),
        )?;
        let decision = state
            .decisions
            .iter()
            .find(|decision| decision.operation == reaction.operation)
            .ok_or_else(|| io::Error::other("Defend decision absent"))?;
        required(
            decision.facts.contains(&reaction.id)
                && !decision.draws.is_empty()
                && source(journey::accepted(decision))?.phase == rpc::JourneyPhase::Combat as i32,
        )?;
    } else {
        required(reactions.is_empty())?;
    }
    Ok(())
}

#[derive(Eq, PartialEq)]
struct Durable {
    counts: [i64; 4],
    operations: Vec<String>,
    facts: Vec<String>,
    checkpoints: Vec<String>,
    intents: Vec<String>,
    envelope: Vec<u8>,
    escort_receipt: Vec<u8>,
    defend_receipt: Vec<u8>,
}

async fn physical_rows(client: &Client, sql: &'static str) -> Result<Vec<String>, Error> {
    let rows = client.query(sql, &[]).await?;
    required(rows.len() <= 256)?;
    rows.into_iter()
        .map(|row| row.try_get(0).map_err(Into::into))
        .collect()
}

async fn physical(client: &Client, checkpoint: &Checkpoint) -> Result<(Durable, Vec<u8>), Error> {
    let tenant = local_demo_scope::TENANT.as_slice();
    let session = *checkpoint.basis().session.as_bytes();
    let row = client.query_one(
        "SELECT (SELECT count(*) FROM df_game.operations) AS operations, (SELECT count(*) FROM df_game.facts) AS facts, (SELECT count(*) FROM df_game.checkpoints) AS checkpoints, (SELECT count(*) FROM df_game.intents) AS intents, CASE WHEN octet_length(c.complete_envelope)<=1048576 THEN c.complete_envelope END AS envelope FROM df_game.sessions s JOIN df_game.checkpoints c ON (c.tenant_id,c.session_id,c.recovery_epoch,c.in_epoch_sequence)=(s.tenant_id,s.session_id,s.recovery_epoch,s.in_epoch_sequence) WHERE s.tenant_id=$1 AND s.session_id=$2",
        &[&tenant, &session.as_slice()],
    ).await?;
    let receipt = async |operation: u8| -> Result<Vec<u8>, Error> {
        let row = client.query_one(
            "SELECT CASE WHEN octet_length(receipt)<=4096 THEN receipt END FROM df_game.operations WHERE tenant_id=$1 AND session_id=$2 AND operation_id=$3",
            &[&tenant, &session.as_slice(), &[operation; 16].as_slice()],
        ).await?;
        Ok(row.try_get(0)?)
    };
    let state = Durable {
        counts: [
            row.try_get("operations")?,
            row.try_get("facts")?,
            row.try_get("checkpoints")?,
            row.try_get("intents")?,
        ],
        operations: physical_rows(client, "SELECT CASE WHEN octet_length(body)<=262144 AND sum(octet_length(body)) OVER ()<=1048576 THEN body END FROM (SELECT row_to_json(t)::text AS body,row_number() OVER (ORDER BY tenant_id,session_id,principal_id,command_namespace,recovery_epoch,operation_id) AS row_ordinal FROM df_game.operations t ORDER BY tenant_id,session_id,principal_id,command_namespace,recovery_epoch,operation_id LIMIT 257) bounded ORDER BY row_ordinal").await?,
        facts: physical_rows(client, "SELECT CASE WHEN octet_length(body)<=262144 AND sum(octet_length(body)) OVER ()<=1048576 THEN body END FROM (SELECT row_to_json(t)::text AS body,row_number() OVER (ORDER BY tenant_id,session_id,committed_epoch,committed_sequence,ordinal) AS row_ordinal FROM df_game.facts t ORDER BY tenant_id,session_id,committed_epoch,committed_sequence,ordinal LIMIT 257) bounded ORDER BY row_ordinal").await?,
        checkpoints: physical_rows(client, "SELECT CASE WHEN octet_length(body)<=262144 AND sum(octet_length(body)) OVER ()<=1048576 THEN body END FROM (SELECT row_to_json(t)::text AS body,row_number() OVER (ORDER BY tenant_id,session_id,recovery_epoch,in_epoch_sequence) AS row_ordinal FROM df_game.checkpoints t ORDER BY tenant_id,session_id,recovery_epoch,in_epoch_sequence LIMIT 257) bounded ORDER BY row_ordinal").await?,
        intents: physical_rows(client, "SELECT CASE WHEN octet_length(body)<=262144 AND sum(octet_length(body)) OVER ()<=1048576 THEN body END FROM (SELECT row_to_json(t)::text AS body,row_number() OVER (ORDER BY tenant_id,session_id,principal_id,command_namespace,recovery_epoch,operation_id,slot) AS row_ordinal FROM df_game.intents t ORDER BY tenant_id,session_id,principal_id,command_namespace,recovery_epoch,operation_id,slot LIMIT 257) bounded ORDER BY row_ordinal").await?,
        envelope: row.try_get("envelope")?,
        escort_receipt: receipt(0xd6).await?,
        defend_receipt: receipt(0xd7).await?,
    };
    let mut bytes = Vec::new();
    for count in state.counts {
        bytes.extend_from_slice(&count.to_be_bytes());
    }
    for item in state
        .operations
        .iter()
        .chain(&state.facts)
        .chain(&state.checkpoints)
        .chain(&state.intents)
    {
        required(
            bytes
                .len()
                .checked_add(item.len() + 4)
                .is_some_and(|size| size <= 4 * 1024 * 1024),
        )?;
        bytes.extend_from_slice(&u32::try_from(item.len())?.to_be_bytes());
        bytes.extend_from_slice(item.as_bytes());
    }
    Ok((state, bytes))
}

fn native_receipt_matches(
    document: &[u8],
    reply: &rpc::DecisionReceipt,
    checkpoint: &Checkpoint,
    operation_byte: u8,
    codec: df_persistence::NativeCodecLimits,
) -> Result<bool, Error> {
    let operation = df_types::OperationId::from_bytes(&[operation_byte; 16])
        .map_err(|_| io::Error::other("courier NPC operation identity"))?;
    let native = local_demo_scope::decode_owned_demo_receipt(
        document,
        checkpoint.basis().session,
        operation,
        codec,
    )
    .map_err(repo)?;
    let mut decisions = checkpoint
        .state()
        .decisions
        .iter()
        .filter(|decision| decision.operation == operation);
    let Some(expected) = decisions.next() else {
        return Ok(false);
    };
    required(decisions.next().is_none())?;
    let projected = wire::receipt(
        native.basis(),
        operation,
        rpc::decision_receipt::Outcome::Accepted(Box::new(source(journey::accepted(
            native.decision(),
        ))?)),
        false,
    );
    Ok(native.basis().run == checkpoint.basis().run
        && native.decision() == expected
        && projected == *reply)
}

async fn stale_release(client: &Client, checkpoint: &Checkpoint) -> Result<(), Error> {
    let old: [u8; 16] = read("courier-npc-private-old-fence-01.bin", 16)?
        .try_into()
        .map_err(|_| io::Error::other("courier NPC old fence length"))?;
    required(
        local_demo_scope::release_owner(client, checkpoint.basis().session, old).await
            == Err(df_session::submission::RepositoryError::RevisionConflict),
    )?;
    Ok(())
}

async fn phase_a(
    service: &Service,
    client: &Client,
    codec: df_persistence::NativeCodecLimits,
) -> Result<(), Error> {
    let mut calls = Calls {
        count: 0,
        started_milliseconds: now()?,
    };
    let first = join(service, 0xd1, &mut calls).await?;
    let second = join(service, 0xd2, &mut calls).await?;
    for (index, credential) in [first, second].into_iter().enumerate() {
        let current = service.updates.borrow().clone();
        required(
            !committed(
                submit(
                    service,
                    &create(&current, 0xd3 + index as u8),
                    credential,
                    &mut calls,
                )
                .await?,
            )?
            .replayed,
        )?;
    }
    let current = service.updates.borrow().clone();
    required(
        !committed(
            submit(
                service,
                &action(&current, 0xd5, rpc::GameplayActionKind::BeginStory),
                first,
                &mut calls,
            )
            .await?,
        )?
        .replayed,
    )?;
    let opening = service.updates.borrow().clone();
    let participants = journey::participants(&opening);
    required(participants.len() == 2)?;
    let hero = participants[0]
        .character
        .ok_or_else(|| io::Error::other("escort hero absent"))?;
    let other = participants[1]
        .character
        .ok_or_else(|| io::Error::other("other hero absent"))?;
    let members = [
        *participants[0].member.as_bytes(),
        *participants[1].member.as_bytes(),
    ];
    let courier = source(journey::entity([0x67; 16]))?;
    required(
        opening.pins() == &source(model::pins())?
            && opening.state().continuity.npcs.len() == 1
            && opening.state().continuity.npcs[0].entity == courier
            && opening.state().continuity.npcs[0].known_facts.is_empty()
            && opening.state().continuity.witnesses.is_empty()
            && opening.state().relationships.len() == 2,
    )?;
    let escort = action(&opening, 0xd6, rpc::GameplayActionKind::EscortCourier);
    let escort_receipt = committed(submit(service, &escort, first, &mut calls).await?)?;
    required(
        !escort_receipt.replayed
            && matches!(
                escort_receipt.outcome.as_ref(),
                Some(rpc::decision_receipt::Outcome::Accepted(_))
            ),
    )?;
    let escorted = service.updates.borrow().clone();
    verify_npc(&escorted, hero, other, false)?;
    required(source(journey::phase(&escorted))? == rpc::JourneyPhase::Dialogue)?;
    let escort_row = client.query_one(
        "SELECT CASE WHEN octet_length(c.complete_envelope)<=1048576 THEN c.complete_envelope END AS envelope FROM df_game.sessions s JOIN df_game.checkpoints c ON (c.tenant_id,c.session_id,c.recovery_epoch,c.in_epoch_sequence)=(s.tenant_id,s.session_id,s.recovery_epoch,s.in_epoch_sequence) WHERE s.tenant_id=$1 AND s.session_id=$2",
        &[&local_demo_scope::TENANT.as_slice(), &escorted.basis().session.as_bytes().as_slice()],
    ).await?;
    let escort_envelope: Vec<u8> = escort_row.try_get("envelope")?;
    required(
        escort_envelope
            == local_demo_scope::encode_owned_demo_checkpoint(&escorted, codec).map_err(repo)?,
    )?;
    let defend = action(&escorted, 0xd7, rpc::GameplayActionKind::DefendCourier);
    let defend_receipt = committed(submit(service, &defend, first, &mut calls).await?)?;
    required(
        !defend_receipt.replayed
            && matches!(
                defend_receipt.outcome.as_ref(),
                Some(rpc::decision_receipt::Outcome::Accepted(_))
            ),
    )?;
    let snapshot = snapshot(service, &mut calls).await?;
    required(
        snapshot.checkpoint == service.updates.borrow().clone()
            && snapshot.calls_remaining == CALL_LIMIT as u16 - calls.count as u16,
    )?;
    verify_npc(&snapshot.checkpoint, hero, other, true)?;
    required(source(journey::phase(&snapshot.checkpoint))? == rpc::JourneyPhase::Combat)?;
    let (durable, rows) = physical(client, &snapshot.checkpoint).await?;
    required(
        calls.count == 8
            && durable.counts[0] == 7
            && durable.counts[2] == 8
            && durable.counts[3] == 0
            && durable.counts[1] == i64::try_from(snapshot.checkpoint.state().facts.len())?
            && durable.envelope
                == local_demo_scope::encode_owned_demo_checkpoint(&snapshot.checkpoint, codec)
                    .map_err(repo)?
            && native_receipt_matches(
                &durable.escort_receipt,
                &escort_receipt,
                &snapshot.checkpoint,
                0xd6,
                codec,
            )?
            && native_receipt_matches(
                &durable.defend_receipt,
                &defend_receipt,
                &snapshot.checkpoint,
                0xd7,
                codec,
            )?,
    )?;
    let mut grants = Vec::from(first);
    grants.extend_from_slice(&second);
    grants.extend_from_slice(&members[0]);
    grants.extend_from_slice(&members[1]);
    let mut metadata = Vec::from(calls.started_milliseconds.to_be_bytes());
    metadata.extend_from_slice(&(calls.count as u16).to_be_bytes());
    metadata.extend_from_slice(&std::process::id().to_be_bytes());
    for (name, bytes) in [
        ("courier-npc-private-grants-01.bin", grants),
        ("courier-npc-private-metadata-01.bin", metadata),
        (
            "courier-npc-private-build-01.bin",
            crate::BUILD_ID.as_bytes().to_vec(),
        ),
        (
            "courier-npc-private-escort-request-01.bin",
            escort.encode_to_vec(),
        ),
        (
            "courier-npc-private-defend-request-01.bin",
            defend.encode_to_vec(),
        ),
        (
            "courier-npc-private-escort-receipt-01.bin",
            escort_receipt.encode_to_vec(),
        ),
        (
            "courier-npc-private-defend-receipt-01.bin",
            defend_receipt.encode_to_vec(),
        ),
        ("courier-npc-private-envelope-01.bin", durable.envelope),
        ("courier-npc-private-rows-01.bin", rows),
    ] {
        save(name, &bytes)?;
    }
    save("courier-npc-a-ready-body-01.bin", format!(
        "{{\"pass\":true,\"pid\":{},\"actor_calls\":8,\"exercise_milliseconds\":{},\"joined_created_begin_escort_defend\":true,\"accepted_shared_contact\":true,\"courier_known_fact_and_witness\":true,\"exact_directional_reaction\":true,\"checkpoint_and_receipts_durable\":true,\"four_families_captured\":true}}",
        std::process::id(), elapsed(calls.started_milliseconds)?
    ).as_bytes())?;
    Ok(())
}

struct PrivateGrants {
    credentials: [[u8; 32]; 2],
    members: [[u8; 16]; 2],
}

fn private_grants() -> Result<PrivateGrants, Error> {
    let bytes = read("courier-npc-private-grants-01.bin", 96)?;
    required(bytes.len() == 96)?;
    Ok(PrivateGrants {
        credentials: [bytes[..32].try_into()?, bytes[32..64].try_into()?],
        members: [bytes[64..80].try_into()?, bytes[80..96].try_into()?],
    })
}

async fn phase_b(
    service: &Service,
    client: &Client,
    codec: df_persistence::NativeCodecLimits,
) -> Result<(), Error> {
    let (start, count, old_pid) = metadata()?;
    required(old_pid != std::process::id())?;
    // Startup publishes the restored checkpoint before any new Actor input.
    let restored = service.updates.borrow().clone();
    let PrivateGrants {
        credentials,
        members,
    } = private_grants()?;
    let participants = journey::participants(&restored);
    required(
        participants.len() == 2
            && *participants[0].member.as_bytes() == members[0]
            && *participants[1].member.as_bytes() == members[1],
    )?;
    let hero = participants[0]
        .character
        .ok_or_else(|| io::Error::other("restored hero absent"))?;
    let other = participants[1]
        .character
        .ok_or_else(|| io::Error::other("restored other hero absent"))?;
    verify_npc(&restored, hero, other, true)?;
    required(
        local_demo_scope::encode_owned_demo_checkpoint(&restored, codec).map_err(repo)?
            == read("courier-npc-private-envelope-01.bin", 1048576)?,
    )?;
    stale_release(client, &restored).await?;
    let (before, rows) = physical(client, &restored).await?;
    required(
        rows == read("courier-npc-private-rows-01.bin", 4 * 1048576)?
            && before.envelope == read("courier-npc-private-envelope-01.bin", 1048576)?,
    )?;
    let mut calls = Calls {
        count,
        started_milliseconds: start,
    };
    for (request_name, receipt_name) in [
        (
            "courier-npc-private-escort-request-01.bin",
            "courier-npc-private-escort-receipt-01.bin",
        ),
        (
            "courier-npc-private-defend-request-01.bin",
            "courier-npc-private-defend-receipt-01.bin",
        ),
    ] {
        let request = rpc::SubmitActionRequest::decode(read(request_name, 8192)?.as_slice())?;
        let mut replay = committed(submit(service, &request, credentials[0], &mut calls).await?)?;
        required(replay.replayed)?;
        replay.replayed = false;
        required(replay.encode_to_vec() == read(receipt_name, 8192)?)?;
        let (after, after_rows) = physical(client, &restored).await?;
        required(after == before && after_rows == rows)?;
    }
    let snapshot = snapshot(service, &mut calls).await?;
    required(
        snapshot.checkpoint == restored
            && snapshot.calls_remaining == CALL_LIMIT as u16 - calls.count as u16
            && calls.count == 11,
    )?;
    verify_npc(&snapshot.checkpoint, hero, other, true)?;
    save(
        "courier-npc-private-final-count-01.bin",
        &(calls.count as u16).to_be_bytes(),
    )?;
    save("courier-npc-b-proof-body-01.bin", format!(
        "{{\"pass\":true,\"pid\":{},\"previous_pid\":{},\"actor_calls\":11,\"exercise_milliseconds\":{},\"restored_before_actor_input\":true,\"canonical_npc_witness_reaction_equal\":true,\"exact_escort_and_defend_replay\":true,\"no_new_draws_facts_state_or_rows\":true,\"four_families_equal\":true,\"stale_fence_release_refused\":true}}",
        std::process::id(), old_pid, elapsed(start)?
    ).as_bytes())?;
    Ok(())
}

pub(super) async fn run(
    service: Service,
    _display: [u8; 32],
    codec: df_persistence::NativeCodecLimits,
    database: Config,
    phase: Phase,
    fence: [u8; 16],
) -> Result<(), io::Error> {
    configured()?;
    record_owner(phase, fence)?;
    let (client, connection) = timeout(Duration::from_secs(2), database.connect(NoTls))
        .await
        .map_err(|_| io::Error::other("courier NPC inspector connect deadline"))?
        .map_err(io::Error::other)?;
    let driver = tokio::spawn(connection);
    let result = timeout(Duration::from_secs(45), async {
        match phase {
            Phase::A => phase_a(&service, &client, codec).await,
            Phase::B => phase_b(&service, &client, codec).await,
        }
    })
    .await;
    drop(client);
    super::join_grant_driver(driver).await?;
    result
        .map_err(|_| io::Error::other("courier NPC exercise deadline"))?
        .map_err(|error| io::Error::other(format!("courier NPC phase failed: {error}")))?;
    let (body, name) = match phase {
        Phase::A => (
            "courier-npc-a-ready-body-01.bin",
            "courier-npc-a-ready-01.json",
        ),
        Phase::B => (
            "courier-npc-b-proof-body-01.bin",
            "courier-npc-b-proof-01.json",
        ),
    };
    let bytes = read(body, 16 * 1024)?;
    report(name, std::str::from_utf8(&bytes).map_err(io::Error::other)?)
}

pub(super) fn closed(phase: Phase, remaining: u16) -> Result<(), io::Error> {
    let (start, count, _) = metadata()?;
    let (expected, name) = match phase {
        Phase::A => (count, "courier-npc-a-closed-01.json"),
        Phase::B => (
            usize::from(u16::from_be_bytes(
                read("courier-npc-private-final-count-01.bin", 2)?
                    .try_into()
                    .map_err(|_| io::Error::other("courier NPC final count invalid"))?,
            )),
            "courier-npc-b-closed-01.json",
        ),
    };
    required(
        expected <= CALL_LIMIT
            && remaining == CALL_LIMIT as u16 - expected as u16
            && elapsed(start)? <= 90_000,
    )?;
    report(
        name,
        &format!(
            "{{\"pass\":true,\"pid\":{},\"actor_calls\":{},\"exercise_milliseconds\":{},\"admission_stopped\":true,\"actor_drained\":true,\"exact_fence_released\":true,\"actor_joined\":true,\"repository_driver_joined\":true,\"grant_driver_joined\":true}}",
            std::process::id(),
            expected,
            elapsed(start)?
        ),
    )
}

#[cfg(test)]
mod path_admission_tests {
    use super::validate_qualification_root;
    use std::ffi::{OsStr, OsString};
    // Directory modes and symlinks are Unix-only cases.
    #[cfg(unix)]
    use std::os::unix::fs::{PermissionsExt, symlink};
    #[cfg(windows)]
    use std::os::windows::ffi::OsStringExt;

    #[test]
    fn only_existing_private_direct_attempt_children_are_admitted() {
        let unique = format!(
            "df-courier-npc-path-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let base = std::env::temp_dir().join(unique);
        std::fs::create_dir(&base).unwrap();
        // Directory modes are a Unix-only property.
        #[cfg(unix)]
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700)).unwrap();
        for name in ["attempt-03", "attempt-04"] {
            let child = base.join(name);
            std::fs::create_dir(&child).unwrap();
            #[cfg(unix)]
            std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o700)).unwrap();
            assert_eq!(
                validate_qualification_root(&base, Some(child.as_os_str())).unwrap(),
                child
            );
        }
        let nested = base.join("other");
        std::fs::create_dir(&nested).unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o700)).unwrap();
        let wrong_parent = nested.join("attempt-03");
        std::fs::create_dir(&wrong_parent).unwrap();
        assert!(validate_qualification_root(&base, Some(wrong_parent.as_os_str())).is_err());
        assert!(
            validate_qualification_root(
                &base,
                Some(base.join("attempt-03/../attempt-04").as_os_str())
            )
            .is_err()
        );
        for name in [
            "attempt-x",
            "attempt-0000",
            "attempt-10000",
            "attempt-05/child",
        ] {
            assert!(validate_qualification_root(&base, Some(base.join(name).as_os_str())).is_err());
        }
        assert!(
            validate_qualification_root(&base, Some(base.join("attempt-05").as_os_str())).is_err()
        );
        // The symlinked child is a Unix-only case.
        #[cfg(unix)]
        {
            symlink(base.join("attempt-03"), base.join("attempt-06")).unwrap();
            assert!(
                validate_qualification_root(&base, Some(base.join("attempt-06").as_os_str()))
                    .is_err()
            );
        }
        assert!(validate_qualification_root(&base, None).is_err());
        assert!(validate_qualification_root(&base, Some(OsStr::new(""))).is_err());
        // Non-Unicode input is built per platform: an invalid UTF-8 byte on
        // Unix, a lone surrogate on Windows.
        #[cfg(unix)]
        use std::os::unix::ffi::OsStringExt;
        #[cfg(unix)]
        let non_unicode = OsString::from_vec(vec![0xff]);
        #[cfg(windows)]
        let non_unicode = OsString::from_wide(&[0xD800]);
        assert!(validate_qualification_root(base.as_path(), Some(&non_unicode)).is_err());
        std::fs::remove_dir_all(&base).unwrap();
    }
}
