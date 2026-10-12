//! Ignored operator-only generated-service recovery proof on Root-registered loopback PG.
//! No ordinary service route, public fault message or hosted authority is introduced.
use super::{Service, actor, hex, journey, model, wire};
use df_model::checkpoint::Checkpoint;
use df_persistence::local_demo_scope::{
    self, LocalDemoAuthority, LocalDemoRole, LocalDemoScopeIssuer,
};
use df_persistence::{
    NativeCodecLimits, NativeRepositoryOptions, NativeTransactionBounds, PostgresRepository,
};
use df_protocol::common as rpc;
use df_session::inbox::{InboxHandle, bounded_inbox};
use df_session::submission::DurableOwner;
use prost::Message;
use sha2::{Digest, Sha256};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{Semaphore, oneshot, watch},
    task::JoinHandle,
    time::{Instant, timeout},
};
use tokio_postgres::{Client, Config, NoTls};
use tonic::Request;

const REPORT: &str = "/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/engine-recovery-20261005/engine-recovery-qualification-report-04.json";
const COURIER_REPORT: &str = "/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/engine-courier-ai-20261005/courier-pending-restart-report-01.json";
type Error = Box<dyn std::error::Error + Send + Sync>;
#[track_caller]
fn required(value: bool) -> Result<(), Error> {
    if value {
        Ok(())
    } else {
        let caller = std::panic::Location::caller();
        Err(io::Error::other(format!(
            "recovery assertion {}:{}",
            caller.file(),
            caller.line()
        ))
        .into())
    }
}
fn configuration() -> Result<Config, Error> {
    required(
        std::env::var("DF_RECOVERY_QUALIFICATION").as_deref() == Ok("owned-loopback-recovery04"),
    )?;
    Ok(trusted_configuration(
        55517,
        "df-engine-recovery-inspector-01",
    ))
}
fn trusted_configuration(port: u16, application: &'static str) -> Config {
    let mut configuration = Config::new();
    configuration
        .host("127.0.0.1")
        .port(port)
        .dbname("df_gameplay_demo_20261005_recovery05")
        .user("df_gameplay_demo_admin_20261004")
        .ssl_mode(tokio_postgres::config::SslMode::Disable)
        .application_name(application);
    configuration
}
fn options(codec: NativeCodecLimits) -> Result<NativeRepositoryOptions, Error> {
    Ok(NativeRepositoryOptions {
        transaction_bounds: NativeTransactionBounds {
            transaction: Duration::from_secs(2),
            rollback: Duration::from_millis(250),
            driver_join: Duration::from_secs(2),
        },
        codec_limits: codec,
        maximum_receipt_bytes: 4096,
        verifier: Some(local_demo_scope::verifier()),
        recovery: Some(model::recovery().map_err(repository_error)?),
    })
}
async fn repository(
    configuration: &Config,
    reconnect: &Config,
    codec: NativeCodecLimits,
) -> Result<PostgresRepository<LocalDemoAuthority>, Error> {
    let (client, connection) =
        timeout(Duration::from_secs(2), configuration.connect(NoTls)).await??;
    let options = options(codec)?;
    let context = df_observe::OperationContext {
        trace_parent: String::new(),
        build: crate::BUILD_ID.to_owned(),
    };
    let mut repository = match PostgresRepository::from_connected_no_tls(
        tokio::runtime::Handle::current(),
        client,
        connection,
        options.clone(),
        &context,
    )
    .await
    {
        Ok(repository) => repository,
        Err(mut failure) => {
            let error = failure.error();
            failure.close().await.map_err(repository_error)?;
            return Err(io::Error::other(format!("recovery setup {error:?}")).into());
        }
    };
    repository
        .configure_reconnect(reconnect.clone(), options)
        .map_err(repository_error)?;
    Ok(repository)
}
struct ActorExit {
    checkpoint: Checkpoint,
    issuer: LocalDemoScopeIssuer,
}
struct RunningActor {
    service: Service,
    sender: InboxHandle<actor::Call>,
    thread: Option<std::thread::JoinHandle<Result<ActorExit, Error>>>,
}
impl RunningActor {
    async fn start(
        repository: PostgresRepository<LocalDemoAuthority>,
        issuer: LocalDemoScopeIssuer,
        checkpoint: Checkpoint,
        codec: NativeCodecLimits,
        counter: Arc<AtomicUsize>,
    ) -> Result<Self, Error> {
        Self::start_observed(repository, issuer, checkpoint, codec, counter, None, 20).await
    }
    async fn start_observed(
        repository: PostgresRepository<LocalDemoAuthority>,
        mut issuer: LocalDemoScopeIssuer,
        checkpoint: Checkpoint,
        codec: NativeCodecLimits,
        counter: Arc<AtomicUsize>,
        completion_observer: Option<actor::CompletionObserver>,
        calls_remaining: u16,
    ) -> Result<Self, Error> {
        let (updates, receiver) = watch::channel(checkpoint.clone());
        let (publication, intent_notifications) = actor::Publication::new(updates.clone());
        let owner = match DurableOwner::new(
            repository,
            actor::Engine(Some(counter)),
            publication,
            checkpoint,
            4096,
        ) {
            Ok(owner) => owner,
            Err(error) => {
                issuer.close_owned().await.map_err(repository_error)?;
                return Err(repository_error(error));
            }
        };
        let (sender, inbox) = bounded_inbox::<actor::Call>();
        let table_log = super::table_log::TableLog::disabled();
        let actor = actor::Actor {
            table_log: table_log.clone(),
            social: super::social::SocialLedger::none(),
            tempo: None,
            dialogue: super::dialogue::DialogueState::default(),
            owner,
            bootstrap_credential: [0x81; 32],
            issuer,
            codec,
            fenced: false,
            recovery_wakeup: updates,
            campaign_observer: None,
            intent_notifications,
            calls_remaining,
            calls_unbounded: false,
            qualification_inputs: Some(Vec::new()),
            qualification_joins: Some(Vec::new()),
            completion_retry: None,
            session: Default::default(),
            completion_observer,
        };
        let (handoff, receive) = std::sync::mpsc::sync_channel::<actor::Actor>(1);
        let thread = std::thread::Builder::new()
            .name("df-engine-recovery-owner".to_owned())
            .spawn(move || {
                let mut actor = receive
                    .recv_timeout(Duration::from_secs(2))
                    .map_err(|_| io::Error::other("qualification actor handoff deadline"))?;
                actor.run_committed_intents();
                let drained = inbox.run_with_owner_wake(
                    &mut actor,
                    actor::Actor::next_completion_wake,
                    actor::Actor::wake_completion,
                );
                let checkpoint = actor.owner.checkpoint().clone();
                let mut repository = actor.owner.into_repository();
                let first_close = repository.close();
                if first_close.is_err() {
                    let second_close = repository.close();
                    actor.issuer.close().map_err(repository_error)?;
                    second_close.map_err(repository_error)?;
                }
                first_close.map_err(repository_error)?;
                if drained.is_err() {
                    actor.issuer.close().map_err(repository_error)?;
                    return Err(io::Error::other("recovery actor drain failed").into());
                }
                Ok(ActorExit {
                    checkpoint,
                    issuer: actor.issuer,
                })
            });
        let thread = match thread {
            Ok(thread) => thread,
            Err(error) => {
                super::close_unstarted_actor(actor).await?;
                return Err(error.into());
            }
        };
        if let Err(error) = handoff.send(actor) {
            let closed = super::close_unstarted_actor(error.0).await;
            let joined = tokio::task::spawn_blocking(move || thread.join()).await?;
            closed?;
            let _outcome =
                joined.map_err(|_| io::Error::other("qualification actor handoff join failed"))?;
            return Err(io::Error::other("qualification actor handoff failed").into());
        }
        let service = Service {
            actor: sender.clone(),
            updates: receiver,
            streams: Arc::new(Semaphore::new(6)),
            voice: super::voice::Voice::disabled(
                super::startup_configuration::VoiceOff::Qualification,
            ),
            npc_voice: Arc::new(super::npc_reply::NpcVoice::silent()),
            table_log,
            living: super::living_scenes::LivingScenes::disabled(),
        };
        Ok(Self {
            service,
            sender,
            thread: Some(thread),
        })
    }
    async fn close(&mut self) -> Result<ActorExit, Error> {
        let stopped = self.sender.stop();
        let thread = self
            .thread
            .take()
            .ok_or_else(|| io::Error::other("actor join already consumed"))?;
        let joined = tokio::task::spawn_blocking(move || thread.join())
            .await?
            .map_err(|_| io::Error::other("recovery actor panicked"))?;
        stopped.map_err(|_| io::Error::other("recovery actor stop failed"))?;
        joined
    }
}
struct Calls {
    count: usize,
    deadline: Instant,
}
fn admit(calls: &mut Calls) -> Result<(), Error> {
    required(calls.count < 20 && Instant::now() < calls.deadline)?;
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
        name: "Recovery-Fighter".to_owned(),
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
        checkpoints: physical_rows(client, "SELECT CASE WHEN octet_length(body)<=262144 AND sum(octet_length(body)) OVER ()<=1048576 THEN body END FROM (SELECT row_to_json(t)::text AS body,row_number() OVER (ORDER BY tenant_id,session_id,recovery_epoch,in_epoch_sequence) AS row_ordinal FROM df_game.checkpoints t ORDER BY tenant_id,session_id,recovery_epoch,in_epoch_sequence LIMIT 257) bounded ORDER BY row_ordinal").await?,
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
async fn join_driver(
    mut driver: JoinHandle<Result<(), tokio_postgres::Error>>,
) -> Result<(), Error> {
    match timeout(Duration::from_secs(2), &mut driver).await {
        Ok(joined) => {
            joined??;
            Ok(())
        }
        Err(_) => {
            driver.abort();
            if timeout(Duration::from_secs(2), &mut driver).await.is_err() {
                eprintln!("recovery driver join pending; Root owned-process cleanup required");
                std::process::exit(1);
            }
            Err(io::Error::other("recovery driver joined only after deadline").into())
        }
    }
}
async fn exercise() -> Result<(), Error> {
    exercise_grant_recovery(false).await
}

async fn exercise_grant_recovery(recover_grant: bool) -> Result<(), Error> {
    let started = Instant::now();
    let configuration = if recover_grant {
        ordinary_grant_configuration("df_gameplay_demo_20261006_grant_uncertain01")?
    } else {
        configuration()?
    };
    let (mut inspector, connection) =
        timeout(Duration::from_secs(2), configuration.connect(NoTls)).await??;
    let inspector_driver = tokio::spawn(connection);
    let initial = journey::initial().map_err(repository_error)?;
    let codec = NativeCodecLimits {
        maximum_document_bytes: 1024 * 1024,
        maximum_allocated_bytes: 2 * 1024 * 1024,
        maximum_collection_items: 1024,
        maximum_text_bytes: 4096,
    };
    let fence = [0x83; 16];
    local_demo_scope::initialize(
        &mut inspector,
        &initial,
        fence,
        [0x81; 32],
        [0x82; 32],
        codec,
    )
    .await
    .map_err(|_| io::Error::other("registered local recovery database initialization failed"))?;
    let mut direct = configuration.clone();
    direct.application_name("df-engine-recovery-repository-01");
    let mut grants = configuration.clone();
    grants.application_name("df-engine-recovery-grant-01");
    let (grant_client, grant_connection) =
        timeout(Duration::from_secs(2), grants.connect(NoTls)).await??;
    let (grant_closed, grant_close) = oneshot::channel();
    let mut grant_driver = Some(tokio::spawn(async move {
        let result = grant_connection.await;
        let _observed = grant_closed.send(());
        result
    }));
    let grant_pid: i32 = grant_client
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .try_get(0)?;
    let mut issuer = LocalDemoScopeIssuer::new(
        tokio::runtime::Handle::current(),
        grant_client,
        initial.basis().session,
        fence,
    )
    .map_err(repository_error)?;
    if recover_grant {
        let driver = grant_driver
            .take()
            .ok_or_else(|| io::Error::other("grant driver absent"))?;
        if let Err((error, driver)) = issuer.configure_reconnect(grants.clone(), driver) {
            let closed = issuer.close_owned().await;
            let joined = join_driver(driver).await;
            closed.map_err(repository_error)?;
            joined?;
            return Err(repository_error(error));
        }
    }
    let mut calls = Calls {
        count: 0,
        deadline: started + Duration::from_secs(45),
    };
    let prep_counter = Arc::new(AtomicUsize::new(0));
    let mut prep = RunningActor::start(
        repository(&direct, &direct, codec).await?,
        issuer,
        initial,
        codec,
        prep_counter.clone(),
    )
    .await?;
    let preparation = async {
        let first = join(&prep.service, 0x91, &mut calls).await?;
        let second = join(&prep.service, 0x92, &mut calls).await?;
        Ok::<_, Error>((first, second))
    }
    .await;
    let prepared = prep.close().await?;
    let (first, second) = preparation?;
    required(prep_counter.load(Ordering::SeqCst) == 2)?;
    let baseline = prepared.checkpoint;
    let listener = timeout(
        Duration::from_secs(2),
        tokio::net::TcpListener::bind(("127.0.0.1", 55518)),
    )
    .await??;
    let mut proxy = tokio::spawn(async move {
        let (frontend, _) = timeout(Duration::from_secs(5), listener.accept())
            .await
            .map_err(|_| local_demo_scope::ProxyError::Deadline)?
            .map_err(|_| local_demo_scope::ProxyError::Io)?;
        drop(listener);
        let postgres = timeout(
            Duration::from_secs(2),
            tokio::net::TcpStream::connect(("127.0.0.1", 55517)),
        )
        .await
        .map_err(|_| local_demo_scope::ProxyError::Deadline)?
        .map_err(|_| local_demo_scope::ProxyError::Io)?;
        local_demo_scope::drop_commit_acknowledgement(
            frontend,
            postgres,
            local_demo_scope::ProxyBounds {
                maximum_frame_bytes: 2 * 1024 * 1024,
                maximum_suppressed_response_bytes: 4096,
                deadline: Instant::now() + Duration::from_secs(30),
            },
        )
        .await
    });
    // Config::port appends, so build a fresh single-host proxy configuration.
    let mut proxied = trusted_configuration(55518, "df-engine-recovery-repository-01");
    if recover_grant {
        proxied.dbname("df_gameplay_demo_20261006_grant_uncertain01");
    }
    let counter = Arc::new(AtomicUsize::new(0));
    let mut fault = RunningActor::start(
        repository(&proxied, &direct, codec).await?,
        prepared.issuer,
        baseline.clone(),
        codec,
        counter.clone(),
    )
    .await?;
    let original = create(&baseline, 0x93);
    let flow = async {
        expect_unknown_initial_submission(
            submit(&fault.service, &original, first, &mut calls).await,
        )?;
        require_reductions(&counter, 1, "original unknown commit")?;
        required(*fault.service.updates.borrow() == baseline)?;
        let before = durable(&inspector, 0x93).await?;
        required(before.counts[3] == 0 && before.intents.is_empty())?;
        if recover_grant {
            let stopped: bool = inspector.query_one("SELECT pg_terminate_backend($1::integer)", &[&grant_pid]).await?.try_get(0)?;
            required(stopped)?;
            timeout(Duration::from_secs(2), grant_close).await??;
        }
        let mut different = create(&baseline, 0x94);
        required(
            submit(&fault.service, &different, second, &mut calls)
                .await
                .is_err_and(|status| status.code() == tonic::Code::Unavailable),
        )?;
        let mut conflict = original.clone();
        conflict
            .character
            .as_mut()
            .ok_or_else(|| io::Error::other("fixture creation absent"))?
            .name = "Different request".to_owned();
        required(
            submit(&fault.service, &conflict, first, &mut calls)
                .await
                .is_err_and(|status| status.code() == tonic::Code::Unavailable),
        )?;
        required(
            submit(&fault.service, &original, [0x82; 32], &mut calls)
                .await
                .is_err_and(|status| status.code() == tonic::Code::PermissionDenied),
        )?;
        inspector
            .execute(
                "UPDATE df_local_demo.grants SET active=false WHERE credential=$1::bytea",
                &[&first.as_slice()],
            )
            .await?;
        required(
            submit(&fault.service, &original, first, &mut calls)
                .await
                .is_err_and(|status| status.code() == tonic::Code::PermissionDenied),
        )?;
        inspector
            .execute(
                "UPDATE df_local_demo.grants SET active=true WHERE credential=$1::bytea",
                &[&first.as_slice()],
            )
            .await?;
        required(
            counter.load(Ordering::SeqCst) == 1 && durable(&inspector, 0x93).await? == before,
        )?;
        let recovered = committed(submit(&fault.service, &original, first, &mut calls).await?)?;
        required(
            recovered.replayed
                && matches!(
                    recovered.outcome,
                    Some(rpc::decision_receipt::Outcome::Accepted(_))
                ),
        )?;
        required(
            counter.load(Ordering::SeqCst) == 1 && durable(&inspector, 0x93).await? == before,
        )?;
        let restored = snapshot(&fault.service, &mut calls).await?;
        let restored_bytes =
            local_demo_scope::encode_owned_demo_checkpoint(&restored.checkpoint, codec)
                .map_err(repository_error)?;
        required(restored_bytes == before.envelope)?;
        let decision = restored
            .checkpoint
            .state()
            .decisions
            .iter()
            .find(|decision| decision.operation.as_bytes() == &[0x93; 16])
            .ok_or_else(|| io::Error::other("retained original decision absent"))?;
        let expected = wire::receipt(
            restored.checkpoint.basis(),
            decision.operation,
            rpc::decision_receipt::Outcome::Accepted(Box::new(
                journey::accepted(decision).map_err(repository_error)?,
            )),
            true,
        );
        required(recovered == expected && !before.receipt.is_empty())?;

        required(
            restored.checkpoint.basis().revision
                == baseline
                    .basis()
                    .revision
                    .next_sequence()
                    .map_err(|_| io::Error::other("fixture revision exhausted"))?,
        )?;
        let retry = committed(submit(&fault.service, &original, first, &mut calls).await?)?;
        required(
            retry.replayed
                && retry.outcome == recovered.outcome
                && retry.revision == recovered.revision,
        )?;
        different = create(&restored.checkpoint, 0x94);
        let later = committed(submit(&fault.service, &different, second, &mut calls).await?)?;
        required(!later.replayed)?;
        require_reductions(&counter, 2, "later distinct character creation")?;
        let after = durable(&inspector, 0x94).await?;
        required(
            after.counts[0] == before.counts[0] + 1 && after.counts[2] == before.counts[2] + 1,
        )?;
        let duplicate = committed(submit(&fault.service, &different, second, &mut calls).await?)?;
        required(
            duplicate.replayed
                && duplicate.outcome == later.outcome
                && durable(&inspector, 0x94).await? == after,
        )?;
        let current = snapshot(&fault.service, &mut calls).await?;
        required(
            local_demo_scope::encode_owned_demo_checkpoint(&current.checkpoint, codec)
                .map_err(repository_error)?
                == after.envelope,
        )?;
        // A real second authorized owner advances the durable head. The original
        // actor's known CAS failure must retain a reload route distinct from Unknown.
        let (external_client, external_connection) =
            timeout(Duration::from_secs(2), grants.connect(NoTls)).await??;
        let external_driver = tokio::spawn(external_connection);
        let external_issuer = LocalDemoScopeIssuer::new(
            tokio::runtime::Handle::current(),
            external_client,
            current.checkpoint.basis().session,
            fence,
        )
        .map_err(repository_error)?;
        let mut external = RunningActor::start(
            repository(&direct, &direct, codec).await?,
            external_issuer,
            current.checkpoint.clone(),
            codec,
            Arc::new(AtomicUsize::new(0)),
        )
        .await?;
        let advance = async {
            let begin = action(
                &current.checkpoint,
                0x95,
                rpc::GameplayActionKind::BeginStory,
            );
            required(
                !committed(submit(&external.service, &begin, first, &mut calls).await?)?.replayed,
            )?;
            snapshot(&external.service, &mut calls)
                .await
                .map(|snapshot| snapshot.checkpoint)
        }
        .await;
        let external_exit = external.close().await?;
        drop(external_exit.issuer);
        join_driver(external_driver).await?;
        let advanced = advance?;
        let before_courier = durable(&inspector, 0x95).await?;
        let stale = action(
            &current.checkpoint,
            0x96,
            rpc::GameplayActionKind::BeginStory,
        );
        required(
            submit(&fault.service, &stale, first, &mut calls)
                .await
                .is_err_and(|status| status.code() == tonic::Code::Unavailable),
        )?;
        let known = snapshot(&fault.service, &mut calls).await?;
        required(known.fenced && !known.uncertain && known.checkpoint == current.checkpoint)?;
        require_reductions(&counter, 3, "stale compare-and-swap reduction")?;
        required(durable(&inspector, 0x95).await? == before_courier)?;
        let courier = action(&advanced, 0x97, rpc::GameplayActionKind::AskCourier);
        let courier_receipt = committed(submit(&fault.service, &courier, first, &mut calls).await?)?;
        required(!courier_receipt.replayed)?;
        let resumed = snapshot(&fault.service, &mut calls).await?;
        required(!resumed.fenced && !resumed.uncertain)?;
        // Engine::decide counts both the accepted Ask and its separate Job input.
        // They follow original creation, later creation, and the rejected stale CAS.
        require_reductions(&counter, 5, "AskCourier and its committed completion")?;
        let completed = &resumed.checkpoint;
        let after_courier = durable(&inspector, 0x97).await?;
        required(after_courier.envelope == local_demo_scope::encode_owned_demo_checkpoint(completed, codec)
            .map_err(repository_error)?)?;
        let [intent] = completed.state().intents.as_slice() else {
            return Err(io::Error::other("expected one courier completion intent").into());
        };
        required(intent.kind == df_model::checkpoint::EffectKind::RunAi
            && intent.status == df_model::checkpoint::DurableStatus::Completed
            && intent.operation.as_bytes() == &[0x97; 16])?;
        let completion = super::courier_ai::expected_completion(intent)
            .map_err(|_| io::Error::other("courier completion binding"))?;
        let operation = super::courier_ai::completion_operation(intent)
            .map_err(|_| io::Error::other("courier completion operation"))?;
        let recipient = super::courier_ai::recipient(completed, intent)
            .map_err(|_| io::Error::other("courier completion recipient"))?;
        let completion_decisions = completed.state().decisions.iter()
            .filter(|decision| decision.operation == operation).count();
        required(completion_decisions == 1
            && super::courier_ai::saved_response(completed, recipient).map_err(repository_error)?
                == Some(super::courier_ai::RESPONSE))?;
        // Recover the pre-completion envelope by undoing only its documented
        // status/decision/revision changes, then bind it to the actual Ask checkpoint.
        let mut pending_state = completed.state().clone();
        pending_state.decisions.retain(|decision| decision.operation != operation);
        pending_state.intents[0].status = df_model::checkpoint::DurableStatus::Pending;
        let pending = model::checkpoint(intent.basis, pending_state).map_err(repository_error)?;
        let pending_bytes = local_demo_scope::encode_owned_demo_checkpoint(&pending, codec)
            .map_err(repository_error)?;
        let epoch = intent.basis.revision.epoch().get().to_string();
        let source_sequence = intent.basis.revision.sequence().to_string();
        let source_envelope: Vec<u8> = inspector.query_one("SELECT CASE WHEN octet_length(complete_envelope)<=1048576 THEN complete_envelope END FROM df_game.checkpoints WHERE tenant_id=$1::bytea AND session_id=$2::bytea AND recovery_epoch=$3::text::numeric AND in_epoch_sequence=$4::text::numeric", &[&local_demo_scope::TENANT.as_slice(), &intent.basis.session.as_bytes().as_slice(), &epoch, &source_sequence]).await?.try_get(0)?;
        required(source_envelope == pending_bytes
            && super::courier_ai::stage_completion(&pending, &completion, operation)
                .map_err(|_| io::Error::other("expected courier completion checkpoint"))? == *completed)?;
        let fingerprint = super::courier_ai::completion_fingerprint(&completion)
            .map_err(|_| io::Error::other("courier completion fingerprint"))?;
        let sequence = completed.basis().revision.sequence().to_string();
        let completion_rows: i64 = inspector.query_one("SELECT count(*)::bigint FROM df_game.operations WHERE tenant_id=$1::bytea AND session_id=$2::bytea AND principal_id=$3::bytea AND command_namespace=$4::bytea AND recovery_epoch=$5::text::numeric AND operation_id=$6::bytea AND fingerprint_version=1 AND canonical_fingerprint=$7::bytea AND committed_epoch=$5::text::numeric AND committed_sequence=$8::text::numeric AND receipt_version=2 AND receipt IS NOT NULL", &[&local_demo_scope::TENANT.as_slice(), &intent.basis.session.as_bytes().as_slice(), &recipient.as_bytes().as_slice(), &b"local-courier-completion-v1".as_slice(), &epoch, &operation.as_bytes().as_slice(), &fingerprint.as_slice(), &sequence]).await?.try_get(0)?;
        required(completion_rows == 1
            && after_courier.counts[0] == before_courier.counts[0] + 2
            && after_courier.counts[2] == before_courier.counts[2] + 2
            && after_courier.counts[3] == before_courier.counts[3] + 1
            && before_courier.operations.iter().all(|row| after_courier.operations.contains(row))
            && before_courier.facts.iter().all(|row| after_courier.facts.contains(row))
            && before_courier.checkpoints.iter().all(|row| after_courier.checkpoints.contains(row)))?;
        let replayed_courier = committed(submit(&fault.service, &courier, first, &mut calls).await?)?;
        required(replayed_courier.replayed && replayed_courier.outcome == courier_receipt.outcome
            && replayed_courier.revision == courier_receipt.revision
            && durable(&inspector, 0x97).await? == after_courier)?;
        require_reductions(&counter, 5, "exact AskCourier receipt replay")?;
        Ok::<_, Error>(resumed.checkpoint)
    }
    .await;
    let mut fault_exit = fault.close().await?;
    let proxy_join = timeout(Duration::from_secs(2), &mut proxy).await;
    let observation = match proxy_join {
        Ok(joined) => joined?.map_err(|error| {
            // Enum-only diagnostics never contain protocol bytes or credentials.
            eprintln!("owned ACK proxy {error:?}");
            io::Error::other(format!("owned ACK proxy {error:?}"))
        }),
        Err(_) => {
            proxy.abort();
            if timeout(Duration::from_secs(2), &mut proxy).await.is_err() {
                eprintln!("recovery proxy join pending; Root owned-process cleanup required");
                std::process::exit(1);
            }
            Err(io::Error::other("owned ACK proxy joined after deadline"))
        }
    };
    fault_exit
        .issuer
        .close_owned()
        .await
        .map_err(repository_error)?;
    drop(fault_exit.issuer);
    if let Some(driver) = grant_driver {
        join_driver(driver).await?;
    }
    // Preserve the original fault result while still closing the inspector and
    // releasing the new fixture's exact fence on both proof success and failure.
    let verified = async {
        let final_checkpoint = flow?;
        let observation = observation?;
        required(fault_exit.checkpoint == final_checkpoint)?;
        required(
            observation.frontend_commit_forwarded
                && observation.postgres_commit_complete_observed
                && observation.postgres_ready_idle_observed
                && observation.suppressed_response_bytes > 0,
        )?;
        let remaining = timeout(Duration::from_secs(2), async {
            loop {
                let count: i64 = inspector.query_one("SELECT count(*) FROM pg_stat_activity WHERE application_name LIKE 'df-engine-recovery-%' AND application_name <> 'df-engine-recovery-inspector-01'", &[]).await?.try_get(0)?;
                if count == 0 { return Ok::<_, tokio_postgres::Error>(count); }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await??;
        required(remaining == 0 && calls.count <= 20 && started.elapsed() <= Duration::from_secs(45))?;
        Ok::<_, Error>((observation, remaining))
    }.await;
    let released = if recover_grant {
        local_demo_scope::release_owner(&inspector, baseline.basis().session, fence).await
    } else {
        Ok(())
    };
    drop(inspector);
    let inspector_joined = join_driver(inspector_driver).await;
    released.map_err(repository_error)?;
    inspector_joined?;
    let (observation, remaining) = verified?;
    let calls = calls.count;
    let grant_drivers = if recover_grant { 3 } else { 2 };
    std::fs::write(
        if recover_grant {
            "/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/wave28-grant-recovery-20261006/ordinary-grant-uncertainty-report-02.json"
        } else {
            REPORT
        },
        format!(
            "{{\"pass\":true,\"actor_calls\":{calls},\"actor_calls_limit\":20,\"exercise_milliseconds\":{},\"exercise_milliseconds_limit\":45000,\"original_engine_reductions\":1,\"total_fault_engine_reductions\":5,\"stale_cas_reductions\":1,\"courier_action_reductions\":1,\"courier_completion_reductions\":1,\"completed_courier_intents\":1,\"courier_exact_replay_unchanged\":true,\"exact_retry_stored_receipt\":true,\"physical_operation_rows_unchanged\":true,\"physical_fact_rows_unchanged\":true,\"physical_intent_rows_unchanged\":true,\"observed_intent_rows\":0,\"checkpoint_reload_byte_equality\":true,\"later_action_once\":true,\"known_reload_resumed\":true,\"proxy_commit_complete\":true,\"proxy_ready_idle\":true,\"proxy_suppressed_bytes\":{},\"proxy_task_joined\":true,\"actor_threads_joined\":3,\"native_repository_drivers_joined\":4,\"auxiliary_grant_drivers_joined\":{grant_drivers},\"grant_reconnect_while_uncertain\":{recover_grant},\"inspector_driver_joined\":true,\"remaining_owned_backends\":{remaining},\"owned_backend_application_prefix\":\"df-engine-recovery-\"}}\n",
            started.elapsed().as_millis(),
            observation.suppressed_response_bytes
        ),
    )?;
    Ok(())
}
fn repository_error(error: df_session::submission::RepositoryError) -> Error {
    io::Error::other(format!("recovery repository {error:?}")).into()
}
fn require_reductions(
    counter: &AtomicUsize,
    expected: usize,
    stage: &'static str,
) -> Result<(), Error> {
    let actual = counter.load(Ordering::SeqCst);
    if actual != expected {
        return Err(io::Error::other(format!(
            "recovery reductions at {stage}: expected {expected}, observed {actual}"
        ))
        .into());
    }
    Ok(())
}
fn expect_unknown_initial_submission(
    outcome: Result<rpc::SubmitActionResponse, tonic::Status>,
) -> Result<(), Error> {
    match outcome {
        Err(status) if status.code() == tonic::Code::Unavailable => Ok(()),
        Err(status) => Err(io::Error::other(format!(
            "initial submit expected unavailable; actual status {:?}",
            status.code()
        ))
        .into()),
        Ok(response) => {
            let classification = match response.outcome {
                Some(rpc::submit_action_response::Outcome::CommittedDecision(receipt)) => {
                    match receipt.outcome {
                        Some(rpc::decision_receipt::Outcome::Accepted(_)) => {
                            "committed accepted".to_owned()
                        }
                        Some(rpc::decision_receipt::Outcome::Rejected(rejection)) => format!(
                            "committed rejection {:?}",
                            rpc::RejectionCode::try_from(rejection.code)
                        ),
                        None => "committed outcome absent".to_owned(),
                    }
                }
                Some(rpc::submit_action_response::Outcome::OperationObservation(observation)) => {
                    format!(
                        "operation observation {:?}",
                        rpc::RejectionCode::try_from(observation.code)
                    )
                }
                None => "outcome absent".to_owned(),
            };
            Err(io::Error::other(format!(
                "initial submit expected unavailable; actual RPC success {classification}"
            ))
            .into())
        }
    }
}

/// Operator-only connecting proof using the existing registered recovery database.
/// Run separately on its fresh database; this does not reset or create a database.
#[test]
#[ignore = "requires ROOT registered fresh loopback recovery database and original resource guard"]
fn native_courier_committed_pending_actor_restart() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime
        .block_on(courier_pending_actor_restart(None))
        .unwrap();
}

async fn courier_pending_actor_restart(idle_recovery: Option<bool>) -> Result<(), Error> {
    let started = Instant::now();
    let configuration = if let Some(revoked) = idle_recovery {
        required(
            std::env::var("DF_NARRATIVE_RECOVERY_QUALIFICATION").as_deref()
                == Ok("owned-loopback-narrative-recovery01"),
        )?;
        let mut config = trusted_configuration(55517, "df-narrative-recovery-inspector-01");
        config.dbname(if revoked {
            "df_gameplay_demo_20261005_narrative_revoked01"
        } else {
            "df_gameplay_demo_20261005_narrative_recovery01"
        });
        config
    } else {
        configuration()?
    };
    let (mut inspector, inspector_connection) =
        timeout(Duration::from_secs(2), configuration.connect(NoTls)).await??;
    let inspector_driver = tokio::spawn(inspector_connection);
    let (grant_client, grant_connection) =
        timeout(Duration::from_secs(2), configuration.connect(NoTls)).await??;
    let mut grant_driver = Some(tokio::spawn(grant_connection));
    let grant_pid: i32 = grant_client
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .try_get(0)?;
    let codec = NativeCodecLimits {
        maximum_document_bytes: 1024 * 1024,
        maximum_allocated_bytes: 2 * 1024 * 1024,
        maximum_collection_items: 1024,
        maximum_text_bytes: 4096,
    };
    let initial = journey::initial().map_err(repository_error)?;
    let session = initial.basis().session;
    let fence = [0x83; 16];
    let initialized = local_demo_scope::initialize(
        &mut inspector,
        &initial,
        fence,
        [0x81; 32],
        [0x82; 32],
        codec,
    )
    .await;
    if initialized.is_err() {
        drop(grant_client);
        drop(inspector);
        join_driver(
            grant_driver
                .take()
                .ok_or_else(|| io::Error::other("grant driver missing"))?,
        )
        .await?;
        timeout(Duration::from_secs(2), inspector_driver).await???;
        return Err(io::Error::other("fresh registered courier database required").into());
    }
    let issuer = LocalDemoScopeIssuer::new(
        tokio::runtime::Handle::current(),
        grant_client,
        initial.basis().session,
        fence,
    )
    .map_err(repository_error)?;
    let reductions = Arc::new(AtomicUsize::new(0));
    let mut actor = RunningActor::start(
        repository(&configuration, &configuration, codec).await?,
        issuer,
        initial,
        codec,
        reductions.clone(),
    )
    .await?;
    let mut calls = Calls {
        count: 0,
        deadline: started + Duration::from_secs(45),
    };
    let preparation = async {
        let first = join(&actor.service, 0x91, &mut calls).await?;
        let second = join(&actor.service, 0x92, &mut calls).await?;
        let current = actor.service.updates.borrow().clone();
        committed(submit(&actor.service, &create(&current, 0x93), first, &mut calls).await?)?;
        let current = actor.service.updates.borrow().clone();
        committed(submit(&actor.service, &create(&current, 0x94), second, &mut calls).await?)?;
        let current = actor.service.updates.borrow().clone();
        committed(
            submit(
                &actor.service,
                &action(&current, 0x95, rpc::GameplayActionKind::BeginStory),
                first,
                &mut calls,
            )
            .await?,
        )?;
        Ok::<_, Error>(first)
    }
    .await;
    let prepared = actor.close().await?;
    let result = async {
    if let Ok(first) = preparation {
        let body = action(&prepared.checkpoint, 0x96, rpc::GameplayActionKind::AskCourier);
        let principal = journey::participants(&prepared.checkpoint).first().ok_or_else(|| io::Error::other("courier recipient absent"))?.member;
        let input = wire::journey_input(&body, principal, &prepared.checkpoint)?;
        let native_repository = repository(&configuration, &configuration, codec).await?;
        let count = reductions.clone();
        let commit_thread = std::thread::Builder::new().name("df-courier-source-commit-owner".to_owned()).spawn(move || {
            use df_session::inbox::{AdmissionSequence, Reducer};
            use df_session::submission::{OwnedInput, SubmissionOutcome};
            let mut issuer = prepared.issuer;
            let scope = issuer.issue(&first, model::random().map_err(repository_error)?, Sha256::digest(body.encode_to_vec()).into(), input.clone(), model::pins().map_err(repository_error)?).map_err(repository_error)?;
            let (updates, _) = watch::channel(prepared.checkpoint.clone());
            let (publication, notifications) = actor::Publication::new(updates);
            let mut owner = DurableOwner::new(native_repository, actor::Engine(Some(count)), publication, prepared.checkpoint, 4096).map_err(repository_error)?;
            let (item, receipt) = OwnedInput::new(df_observe::OperationContext { trace_parent: String::new(), build: crate::BUILD_ID.to_owned() }, scope, input);
            owner.reduce(AdmissionSequence(0), item);
            let confirmed = matches!(receipt.try_recv(), Ok(SubmissionOutcome::Confirmed(_)));
            let woken = notifications.try_recv().is_ok();
            let checkpoint = owner.checkpoint().clone();
            let mut repository = owner.into_repository();
            repository.close().map_err(repository_error)?;
            required(confirmed && woken)?;
            Ok::<_, Error>(ActorExit { checkpoint, issuer })
        })?;
        let pending = tokio::task::spawn_blocking(move || commit_thread.join()).await?.map_err(|_| io::Error::other("courier source commit owner panicked"))??;
        required(reductions.load(Ordering::SeqCst) == 6)?;
        let effect = pending.checkpoint.state().intents.first().ok_or_else(|| io::Error::other("durable courier effect absent"))?;
        required(effect.status == df_model::checkpoint::DurableStatus::Pending)?;
        required(super::courier_ai::saved_response(&pending.checkpoint, principal).map_err(repository_error)?.is_none())?;
        let physical_pending = durable(&inspector, 0x96).await?;
        required(physical_pending.counts[3] == 1 && !physical_pending.intents.is_empty())?;
        if let Some(revoked) = idle_recovery {
            let driver = grant_driver.take().ok_or_else(|| io::Error::other("grant driver missing"))?;
            return courier_idle_recovery(&mut inspector, &configuration, pending, driver, grant_pid,
                codec, reductions.clone(), principal, first, revoked, &mut calls).await;
        }
        let mut restarted = RunningActor::start(repository(&configuration, &configuration, codec).await?, pending.issuer, pending.checkpoint, codec, reductions.clone()).await?;
        let resumed = snapshot(&restarted.service, &mut calls).await;
        let resumed_exit = restarted.close().await?;
        let resumed = resumed?;
        required(reductions.load(Ordering::SeqCst) == 7)?;
        required(super::courier_ai::saved_response(&resumed.checkpoint, principal).map_err(repository_error)? == Some(super::courier_ai::RESPONSE))?;
        let accepted = durable(&inspector, 0x96).await?;
        required(accepted.counts[0] == physical_pending.counts[0] + 1 && accepted.intents == physical_pending.intents)?;
        let mut again = RunningActor::start(repository(&configuration, &configuration, codec).await?, resumed_exit.issuer, resumed_exit.checkpoint, codec, reductions.clone()).await?;
        let unchanged = snapshot(&again.service, &mut calls).await;
        let final_exit = again.close().await?;
        drop(final_exit.issuer);
        let unchanged = unchanged?;
        required(unchanged.checkpoint == resumed.checkpoint && reductions.load(Ordering::SeqCst) == 7)?;
        required(durable(&inspector, 0x96).await? == accepted)?;
        required(Instant::now() < calls.deadline)?;
        std::fs::write(COURIER_REPORT, format!("{{\"pass\":true,\"actor_calls\":{},\"actor_calls_limit\":20,\"engine_reductions\":7,\"source_commit_then_real_wake\":true,\"pending_durable_intent_on_actor_restart\":true,\"native_startup_completed_once\":true,\"second_actor_restart_unchanged\":true,\"immutable_declaration_unchanged\":true,\"process_restart_verified\":false}}\n", calls.count))?;
        Ok(())
    } else {
        let issuer = prepared.issuer;
        drop(issuer);
        Err(io::Error::other("courier source preparation refused").into())
    }
    }.await;
    let released = local_demo_scope::release_owner(&inspector, session, fence).await;
    drop(inspector);
    if let Some(driver) = grant_driver {
        join_driver(driver).await?;
    }
    timeout(Duration::from_secs(2), inspector_driver).await???;
    released.map_err(repository_error)?;
    result
}

#[test]
#[ignore = "Root registered fresh narrative_recovery01 PG database and finite resource release required"]
fn actual_pending_completion_recovers_idle_after_owned_grant_connection_failure() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime
        .block_on(courier_pending_actor_restart(Some(false)))
        .unwrap();
}

#[test]
#[ignore = "Root registered fresh narrative_revoked01 PG database and finite resource release required"]
fn actual_idle_reconnect_refuses_current_revoked_recipient_without_execution() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime
        .block_on(courier_pending_actor_restart(Some(true)))
        .unwrap();
}

#[allow(clippy::too_many_arguments)]
async fn courier_idle_recovery(
    inspector: &mut Client,
    configuration: &Config,
    mut pending: ActorExit,
    driver: JoinHandle<Result<(), tokio_postgres::Error>>,
    grant_pid: i32,
    codec: NativeCodecLimits,
    reductions: Arc<AtomicUsize>,
    recipient: df_types::MemberId,
    credential: [u8; 32],
    revoked: bool,
    calls: &mut Calls,
) -> Result<(), Error> {
    use df_model::checkpoint::DurableStatus;
    use df_session::submission::RepositoryError;
    if let Err((error, driver)) = pending
        .issuer
        .configure_reconnect(configuration.clone(), driver)
    {
        pending
            .issuer
            .close_owned()
            .await
            .map_err(repository_error)?;
        join_driver(driver).await?;
        return Err(repository_error(error));
    }
    let setup = async {
        let physical = durable(inspector, 0x96).await?;
        let repository = repository(configuration, configuration, codec).await?;
        Ok::<_, Error>((physical, repository))
    }
    .await;
    let (physical_pending, mut native_repository) = match setup {
        Ok(setup) => setup,
        Err(error) => {
            pending
                .issuer
                .close_owned()
                .await
                .map_err(repository_error)?;
            return Err(error);
        }
    };
    let transaction = {
        let locked = async {
        let transaction = inspector.transaction().await?;
        transaction.batch_execute("SET LOCAL statement_timeout='2000ms'; LOCK TABLE df_local_demo.grants IN ACCESS EXCLUSIVE MODE").await?;
        let terminated: bool = transaction.query_one("SELECT pg_terminate_backend($1::integer)", &[&grant_pid]).await?.try_get(0)?;
        required(terminated)?;
        Ok::<_, Error>(transaction)
    }.await;
        match locked {
            Ok(transaction) => transaction,
            Err(error) => {
                let closed = native_repository.close_owned().await;
                let grants = pending.issuer.close_owned().await;
                closed.map_err(repository_error)?;
                grants.map_err(repository_error)?;
                return Err(error);
            }
        }
    };
    let (observe, outcomes) = std::sync::mpsc::sync_channel(8);
    let mut running = RunningActor::start_observed(
        native_repository,
        pending.issuer,
        pending.checkpoint.clone(),
        codec,
        reductions.clone(),
        Some(observe),
        20,
    )
    .await?;
    let before_calls = calls.count;
    let mut observed_completion = None;
    let flow = async {
        let (outcomes, first) = tokio::task::spawn_blocking(move || {
            let first = outcomes.recv_timeout(Duration::from_secs(4));
            (outcomes, first)
        })
        .await?;
        let first = first
            .map_err(|_| io::Error::other("actual completion failure observation deadline"))?;
        required(first == (Err(RepositoryError::Unavailable), 20))?;
        required(
            running.service.updates.borrow().state().intents[0].status == DurableStatus::Pending
                && reductions.load(Ordering::SeqCst) == 6
                && calls.count == before_calls,
        )?;
        if revoked {
            transaction
                .execute(
                    "UPDATE df_local_demo.grants SET active=false WHERE credential=$1::bytea",
                    &[&credential.as_slice()],
                )
                .await?;
            transaction.commit().await?;
        } else {
            transaction.rollback().await?;
        }
        if revoked {
            let (outcomes, second) = tokio::task::spawn_blocking(move || {
                let second = outcomes.recv_timeout(Duration::from_secs(4));
                (outcomes, second)
            })
            .await?;
            required(
                second.map_err(|_| io::Error::other("revoked completion observation deadline"))?
                    == (Err(RepositoryError::Unauthorized), 20),
            )?;
            // This finite absence check is synchronized with the actual terminal refusal.
            let additional = tokio::task::spawn_blocking(move || {
                outcomes.recv_timeout(Duration::from_millis(600))
            })
            .await?;
            required(matches!(
                additional,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ))?;
            required(
                *running.service.updates.borrow() == pending.checkpoint
                    && reductions.load(Ordering::SeqCst) == 6
                    && calls.count == before_calls,
            )?;
            for member in journey::participants(&pending.checkpoint)
                .iter()
                .map(|entry| entry.member)
                .chain(std::iter::once(
                    df_types::MemberId::from_bytes(&local_demo_scope::DISPLAY)
                        .map_err(|_| io::Error::other("display identity"))?,
                ))
            {
                let role = if member.as_bytes() == &local_demo_scope::DISPLAY {
                    LocalDemoRole::Display
                } else {
                    LocalDemoRole::Player
                };
                let view = wire::journey_view(&pending.checkpoint, role, member)
                    .map_err(repository_error)?;
                required(
                    !view
                        .encode_to_vec()
                        .windows(super::courier_ai::RESPONSE.len())
                        .any(|bytes| bytes == super::courier_ai::RESPONSE.as_bytes()),
                )?;
            }
        } else {
            let mut updates = running.service.updates.clone();
            timeout(Duration::from_secs(5), async {
                loop {
                    if updates.borrow_and_update().state().intents[0].status
                        == DurableStatus::Completed
                    {
                        break Ok::<_, Error>(());
                    }
                    updates
                        .changed()
                        .await
                        .map_err(|_| io::Error::other("ordinary completion publication closed"))?;
                }
            })
            .await??;
            required(calls.count == before_calls && reductions.load(Ordering::SeqCst) == 7)?;
            let completed = updates.borrow().clone();
            observed_completion = Some(completed.clone());
            required(
                completed.state().facts == pending.checkpoint.state().facts
                    && completed.state().draws == pending.checkpoint.state().draws
                    && completed.state().resources == pending.checkpoint.state().resources
                    && completed.state().narrative == pending.checkpoint.state().narrative
                    && completed.state().logical_time == pending.checkpoint.state().logical_time,
            )?;
            required(
                super::courier_ai::saved_response(&completed, recipient)
                    .map_err(repository_error)?
                    == Some(super::courier_ai::RESPONSE),
            )?;
            let actual = snapshot(&running.service, calls).await?;
            required(actual.checkpoint == completed && actual.calls_remaining == 19)?;
            for member in journey::participants(&completed)
                .iter()
                .map(|entry| entry.member)
                .chain(std::iter::once(
                    df_types::MemberId::from_bytes(&local_demo_scope::DISPLAY)
                        .map_err(|_| io::Error::other("display identity"))?,
                ))
            {
                let role = if member.as_bytes() == &local_demo_scope::DISPLAY {
                    LocalDemoRole::Display
                } else {
                    LocalDemoRole::Player
                };
                let view =
                    wire::journey_view(&completed, role, member).map_err(repository_error)?;
                let private = view
                    .encode_to_vec()
                    .windows(super::courier_ai::RESPONSE.len())
                    .any(|bytes| bytes == super::courier_ai::RESPONSE.as_bytes());
                required(private == (member == recipient))?;
            }
        }
        required(Instant::now() < calls.deadline)?;
        Ok::<_, Error>(())
    }
    .await;
    let mut exited = running.close().await?;
    exited
        .issuer
        .close_owned()
        .await
        .map_err(repository_error)?;
    flow?;
    let after = durable(inspector, 0x96).await?;
    required(
        local_demo_scope::encode_owned_demo_checkpoint(&pending.checkpoint, codec)
            .map_err(repository_error)?
            == physical_pending.envelope,
    )?;
    if revoked {
        required(after == physical_pending)?;
    } else {
        let completed = observed_completion
            .ok_or_else(|| io::Error::other("idle completion was not observed"))?;
        let intent = pending
            .checkpoint
            .state()
            .intents
            .first()
            .ok_or_else(|| io::Error::other("pending completion intent absent"))?;
        let completion = super::courier_ai::expected_completion(intent)
            .map_err(|_| io::Error::other("expected completion binding invalid"))?;
        let operation = super::courier_ai::completion_operation(intent)
            .map_err(|_| io::Error::other("expected completion identity invalid"))?;
        let fingerprint = super::courier_ai::completion_fingerprint(&completion)
            .map_err(|_| io::Error::other("expected completion fingerprint invalid"))?;
        let expected =
            super::courier_ai::stage_completion(&pending.checkpoint, &completion, operation)
                .map_err(|_| io::Error::other("expected completion checkpoint invalid"))?;
        // Bind every checkpoint family, pin and basis to the ordinary idle publication,
        // and the exact canonical encoding to the persisted complete envelope.
        required(
            completed == expected
                && exited.checkpoint == completed
                && after.envelope
                    == local_demo_scope::encode_owned_demo_checkpoint(&completed, codec)
                        .map_err(repository_error)?,
        )?;
        required(
            after.counts[0] == physical_pending.counts[0] + 1
                && after.counts[1] == physical_pending.counts[1]
                && after.counts[2] == physical_pending.counts[2] + 1
                && after.counts[3] == physical_pending.counts[3]
                && after.facts == physical_pending.facts
                && after.intents == physical_pending.intents
                && after.receipt == physical_pending.receipt
                && after.operations.len() == physical_pending.operations.len() + 1
                && physical_pending
                    .operations
                    .iter()
                    .all(|row| after.operations.contains(row))
                && after.checkpoints.len() == physical_pending.checkpoints.len() + 1
                && physical_pending
                    .checkpoints
                    .iter()
                    .all(|row| after.checkpoints.contains(row)),
        )?;
        let basis = completed.basis();
        let epoch = basis.revision.epoch().get().to_string();
        let sequence = basis.revision.sequence().to_string();
        let added_operation: String = inspector.query_one("SELECT CASE WHEN octet_length(row_to_json(t)::text)<=262144 THEN row_to_json(t)::text END FROM df_game.operations t WHERE tenant_id=$1::bytea AND session_id=$2::bytea AND principal_id=$3::bytea AND command_namespace=$4::bytea AND recovery_epoch=$5::text::numeric AND operation_id=$6::bytea AND fingerprint_version=1 AND canonical_fingerprint=$7::bytea AND committed_epoch=$5::text::numeric AND committed_sequence=$8::text::numeric AND receipt_version=2 AND receipt IS NOT NULL", &[&local_demo_scope::TENANT.as_slice(), &basis.session.as_bytes().as_slice(), &recipient.as_bytes().as_slice(), &b"local-courier-completion-v1".as_slice(), &epoch, &operation.as_bytes().as_slice(), &fingerprint.as_slice(), &sequence]).await?.try_get(0)?;
        required(
            after
                .operations
                .iter()
                .filter(|row| !physical_pending.operations.contains(row))
                .collect::<Vec<_>>()
                == vec![&added_operation],
        )?;
        let added_checkpoint: String = inspector.query_one("SELECT CASE WHEN octet_length(row_to_json(t)::text)<=262144 THEN row_to_json(t)::text END FROM df_game.checkpoints t WHERE tenant_id=$1::bytea AND session_id=$2::bytea AND recovery_epoch=$3::text::numeric AND in_epoch_sequence=$4::text::numeric AND run_id=$5::bytea AND complete_envelope=$6::bytea", &[&local_demo_scope::TENANT.as_slice(), &basis.session.as_bytes().as_slice(), &epoch, &sequence, &basis.run.as_bytes().as_slice(), &after.envelope.as_slice()]).await?.try_get(0)?;
        required(
            after
                .checkpoints
                .iter()
                .filter(|row| !physical_pending.checkpoints.contains(row))
                .collect::<Vec<_>>()
                == vec![&added_checkpoint],
        )?;
    }
    let remaining: i64 = inspector.query_one("SELECT count(*)::bigint FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid() AND application_name='df-narrative-recovery-inspector-01'", &[]).await?.try_get(0)?;
    required(remaining == 0)?;
    let report = if revoked {
        "narrative-revoked-recovery-report-01.json"
    } else {
        "narrative-idle-recovery-report-01.json"
    };
    std::fs::write(
        format!(
            "/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/engine-narrative-recovery-20261005/{report}"
        ),
        format!(
            "{{\"pass\":true,\"pid\":{},\"actor_calls_before_restoration\":{before_calls},\"actor_calls_after_proof\":{},\"failed_completion_call_budget\":20,\"engine_reductions\":{},\"ordinary_idle_completed_publication\":{},\"revoked_recipient_refused\":{revoked},\"actor_joined\":true,\"original_and_replacement_grant_drivers_joined\":true,\"repository_driver_joined\":true}}\n",
            std::process::id(),
            calls.count,
            reductions.load(Ordering::SeqCst),
            !revoked
        ),
    )?;
    Ok(())
}
#[test]
fn initial_submission_diagnostics_retain_status_without_private_payload() {
    assert!(expect_unknown_initial_submission(Err(tonic::Status::unavailable("private"))).is_ok());
    let denied =
        expect_unknown_initial_submission(Err(tonic::Status::permission_denied("private")))
            .unwrap_err()
            .to_string();
    assert_eq!(
        denied,
        "initial submit expected unavailable; actual status PermissionDenied"
    );
    assert!(!denied.contains("private"));
    assert!(
        expect_unknown_initial_submission(Ok(rpc::SubmitActionResponse::default()))
            .unwrap_err()
            .to_string()
            .contains("RPC success")
    );
}
#[test]
fn recovery_creation_body_passes_actual_source_character_validator() {
    let initial = journey::initial().unwrap();
    let body = create(&initial, 0x93);
    let character = body.character.unwrap();
    let selections = character
        .choices
        .iter()
        .map(|choice| (choice.group_id.as_str(), choice.option_id.as_str()))
        .collect::<Vec<_>>();
    assert!(
        df_rules::local_journey::validate_character(
            &df_rules::local_journey::CreationRules::srd_521(),
            &character.name,
            &selections
        )
        .is_ok()
    );
    assert!(matches!(
        df_rules::local_journey::validate_character(
            &df_rules::local_journey::CreationRules::srd_521(),
            "Recovery Fighter",
            &selections
        ),
        Err(df_rules::local_journey::JourneyRuleError::InvalidName)
    ));
}
#[test]
fn trusted_direct_and_proxy_configurations_each_have_one_host_and_one_port() {
    for port in [55517, 55518] {
        let configuration = trusted_configuration(port, "df-engine-recovery-config-test");
        assert_eq!(configuration.get_ports(), &[port]);
        assert!(
            matches!(configuration.get_hosts(), [tokio_postgres::config::Host::Tcp(host)] if host == "127.0.0.1")
        );
        assert_eq!(
            configuration.get_dbname(),
            Some("df_gameplay_demo_20261005_recovery05")
        );
        assert_eq!(
            configuration.get_user(),
            Some("df_gameplay_demo_admin_20261004")
        );
    }
}
#[test]
#[ignore = "Root registered recovery05 PG55517/proxy55518 and explicit finite release required"]
fn actual_registered_pg_generated_service_recovers_lost_commit_ack() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("fixture runtime");
    runtime
        .block_on(exercise())
        .expect("actual generated-service recovery proof");
}

fn ordinary_grant_configuration(database: &'static str) -> Result<Config, Error> {
    required(
        std::env::var("DF_ORDINARY_GRANT_QUALIFICATION").as_deref()
            == Ok("owned-loopback-grant-recovery01"),
    )?;
    let mut config = trusted_configuration(55517, "df-ordinary-grant-inspector-01");
    config.dbname(database);
    Ok(config)
}

async fn ordinary_grant_issuer(
    configuration: &Config,
    reconnect: &Config,
    checkpoint: &Checkpoint,
    fence: [u8; 16],
) -> Result<(LocalDemoScopeIssuer, oneshot::Receiver<()>), Error> {
    let (client, connection) =
        timeout(Duration::from_secs(2), configuration.connect(NoTls)).await??;
    let (closed, observed) = oneshot::channel();
    let driver = tokio::spawn(async move {
        let result = connection.await;
        let _observed = closed.send(());
        result
    });
    let mut issuer = match LocalDemoScopeIssuer::new(
        tokio::runtime::Handle::current(),
        client,
        checkpoint.basis().session,
        fence,
    ) {
        Ok(issuer) => issuer,
        Err(error) => {
            join_driver(driver).await?;
            return Err(repository_error(error));
        }
    };
    if let Err((error, driver)) = issuer.configure_reconnect(reconnect.clone(), driver) {
        let closed = issuer.close_owned().await;
        let joined = join_driver(driver).await;
        closed.map_err(repository_error)?;
        joined?;
        return Err(repository_error(error));
    }
    Ok((issuer, observed))
}

async fn terminate_ordinary_grant(inspector: &Client) -> Result<i32, Error> {
    // Exactly one issuer is expected in this dedicated fresh fixture database.
    let row = inspector.query_one("SELECT pid FROM pg_stat_activity WHERE datname=current_database() AND application_name='df-ordinary-grant-issuer-01'", &[]).await?;
    let pid: i32 = row.try_get(0)?;
    let stopped: bool = inspector
        .query_one("SELECT pg_terminate_backend($1::integer)", &[&pid])
        .await?
        .try_get(0)?;
    required(stopped)?;
    timeout(Duration::from_secs(2), async {
        loop {
            let gone: bool = inspector
                .query_one(
                    "SELECT NOT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid=$1::integer)",
                    &[&pid],
                )
                .await?
                .try_get(0)?;
            if gone {
                return Ok::<_, Error>(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await??;
    Ok(pid)
}

async fn ordinary_watch(
    service: &Service,
    checkpoint: &Checkpoint,
    member: df_types::MemberId,
    credential: [u8; 32],
    calls: &mut Calls,
) -> Result<rpc::ViewMessage, tonic::Status> {
    use futures::StreamExt;
    admit(calls).map_err(|_| tonic::Status::resource_exhausted("fixture call bound"))?;
    let body = rpc::WatchViewRequest {
        session_id: Some(rpc::SessionId {
            value: Some(checkpoint.basis().session.as_bytes().to_vec()),
        }),
        run_id: Some(rpc::RunId {
            value: Some(checkpoint.basis().run.as_bytes().to_vec()),
        }),
        client_binding_id: Some(rpc::ClientBindingId {
            value: Some(member.as_bytes().to_vec()),
        }),
        ..Default::default()
    };
    let decoded = rpc::WatchViewRequest::decode(body.encode_to_vec().as_slice())
        .map_err(|_| tonic::Status::internal("fixture watch encoding"))?;
    let mut request = Request::new(decoded);
    request.metadata_mut().insert(
        "x-df-local-binding",
        hex(&credential)
            .parse()
            .map_err(|_| tonic::Status::internal("fixture watch metadata"))?,
    );
    let mut stream = rpc::session_service_server::SessionService::watch(service, request)
        .await?
        .into_inner();
    timeout(Duration::from_secs(5), stream.next())
        .await
        .map_err(|_| tonic::Status::deadline_exceeded("fixture watch deadline"))?
        .ok_or_else(|| tonic::Status::unavailable("fixture watch closed"))?
}

#[test]
#[ignore = "Root fresh owned grant_recovery01 DB and finite native resource grant required"]
fn ordinary_calls_reconnect_without_pending_ai_and_refuse_revoked_or_unavailable_grants() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(ordinary_grant_recovery()).unwrap();
}

async fn ordinary_grant_recovery() -> Result<(), Error> {
    let started = Instant::now();
    let configuration = ordinary_grant_configuration("df_gameplay_demo_20261006_grant_recovery01")?;
    let (mut inspector, connection) =
        timeout(Duration::from_secs(2), configuration.connect(NoTls)).await??;
    let inspector_driver = tokio::spawn(connection);
    let initial = journey::initial().map_err(repository_error)?;
    let codec = NativeCodecLimits {
        maximum_document_bytes: 1024 * 1024,
        maximum_allocated_bytes: 2 * 1024 * 1024,
        maximum_collection_items: 1024,
        maximum_text_bytes: 4096,
    };
    let fence = [0x83; 16];
    let initialized = local_demo_scope::initialize(
        &mut inspector,
        &initial,
        fence,
        [0x81; 32],
        [0x82; 32],
        codec,
    )
    .await;
    if initialized.is_err() {
        drop(inspector);
        join_driver(inspector_driver).await?;
        return Err(io::Error::other("fresh registered ordinary-grant database required").into());
    }
    let result = async {
        let mut grants = configuration.clone();
        grants.application_name("df-ordinary-grant-issuer-01");
        let mut native_repository = repository(&configuration, &configuration, codec).await?;
        let (issuer, grant_closed) = match ordinary_grant_issuer(&grants, &grants, &initial, fence).await {
            Ok(issuer) => issuer,
            Err(error) => {
                native_repository.close_owned().await.map_err(repository_error)?;
                return Err(error);
            }
        };
        let reductions = Arc::new(AtomicUsize::new(0));
        let mut running = RunningActor::start(native_repository, issuer, initial.clone(), codec, reductions.clone()).await?;
        let mut calls = Calls { count: 0, deadline: started + Duration::from_secs(45) };
        let flow = async {
            let credential = join(&running.service, 0x91, &mut calls).await?;
            let joined = running.service.updates.borrow().clone();
            let member = journey::participants(&joined).first()
                .ok_or_else(|| io::Error::other("joined member absent"))?.member;
            let request = create(&joined, 0x93);
            let original = committed(submit(&running.service, &request, credential, &mut calls).await?)?;
            let current = running.service.updates.borrow().clone();
            required(current.state().intents.is_empty() && reductions.load(Ordering::SeqCst) == 2)?;
            let physical = durable(&inspector, 0x93).await?;
            required(physical.envelope == local_demo_scope::encode_owned_demo_checkpoint(&current, codec)
                .map_err(repository_error)?)?;
            let expected = ordinary_watch(&running.service, &current, member, credential, &mut calls).await?;
            terminate_ordinary_grant(&inspector).await?;
            timeout(Duration::from_secs(2), grant_closed).await??;
            let recovered = ordinary_watch(&running.service, &current, member, credential, &mut calls).await?;
            required(recovered == expected && durable(&inspector, 0x93).await? == physical)?;
            let replayed = committed(submit(&running.service, &request, credential, &mut calls).await?)?;
            required(replayed.replayed && replayed.outcome == original.outcome && replayed.revision == original.revision)?;
            required(join(&running.service, 0x91, &mut calls).await? == credential)?;
            let observed = snapshot(&running.service, &mut calls).await?;
            required(observed.calls_remaining == 13 && observed.checkpoint == current
                && !observed.fenced && !observed.uncertain && reductions.load(Ordering::SeqCst) == 2
                && durable(&inspector, 0x93).await? == physical)?;
            Ok::<_, Error>((credential, member, request, physical, current))
        }.await;
        let mut exited = running.close().await?;
        exited.issuer.close_owned().await.map_err(repository_error)?;
        let (credential, member, request, physical, current) = flow?;
        required(exited.checkpoint == current)?;

        let mut native_repository = repository(&configuration, &configuration, codec).await?;
        let (issuer, grant_closed) = match ordinary_grant_issuer(&grants, &grants, &current, fence).await {
            Ok(issuer) => issuer,
            Err(error) => {
                native_repository.close_owned().await.map_err(repository_error)?;
                return Err(error);
            }
        };
        let mut joining = RunningActor::start(native_repository, issuer, current.clone(), codec, reductions.clone()).await?;
        let replayed_join = async {
            terminate_ordinary_grant(&inspector).await?;
            timeout(Duration::from_secs(2), grant_closed).await??;
            required(join(&joining.service, 0x91, &mut calls).await? == credential)?;
            let observed = snapshot(&joining.service, &mut calls).await?;
            required(observed.calls_remaining == 18 && observed.checkpoint == current
                && reductions.load(Ordering::SeqCst) == 2
                && durable(&inspector, 0x93).await? == physical)?;
            Ok::<_, Error>(())
        }.await;
        let mut exited = joining.close().await?;
        exited.issuer.close_owned().await.map_err(repository_error)?;
        replayed_join?;
        required(exited.checkpoint == current)?;

        let mut native_repository = repository(&configuration, &configuration, codec).await?;
        let (issuer, grant_closed) = match ordinary_grant_issuer(&grants, &grants, &current, fence).await {
            Ok(issuer) => issuer,
            Err(error) => {
                native_repository.close_owned().await.map_err(repository_error)?;
                return Err(error);
            }
        };
        let mut revoked = RunningActor::start(native_repository, issuer, current.clone(), codec, reductions.clone()).await?;
        let denied = async {
            inspector.execute("UPDATE df_local_demo.grants SET active=false WHERE credential=$1::bytea", &[&credential.as_slice()]).await?;
            terminate_ordinary_grant(&inspector).await?;
            timeout(Duration::from_secs(2), grant_closed).await??;
            required(ordinary_watch(&revoked.service, &current, member, credential, &mut calls).await
                .is_err_and(|error| error.code() == tonic::Code::PermissionDenied))?;
            let observed = snapshot(&revoked.service, &mut calls).await?;
            required(observed.calls_remaining == 18 && observed.checkpoint == current
                && !observed.fenced && !observed.uncertain && reductions.load(Ordering::SeqCst) == 2
                && durable(&inspector, 0x93).await? == physical)?;
            Ok::<_, Error>(())
        }.await;
        let mut exited = revoked.close().await?;
        exited.issuer.close_owned().await.map_err(repository_error)?;
        denied?;
        required(exited.checkpoint == current)?;

        // A failed trusted reconnect setup must refuse all three ordinary calls
        // before their original handlers and retain their exact canonical state.
        let absent: bool = inspector.query_one("SELECT NOT EXISTS (SELECT 1 FROM pg_database WHERE datname='df_gameplay_demo_20261006_grant_absent01')", &[]).await?.try_get(0)?;
        required(absent)?;
        let mut unavailable_configuration = grants.clone();
        unavailable_configuration.dbname("df_gameplay_demo_20261006_grant_absent01");
        let mut native_repository = repository(&configuration, &configuration, codec).await?;
        let (issuer, grant_closed) = match ordinary_grant_issuer(&grants, &unavailable_configuration, &current, fence).await {
            Ok(issuer) => issuer,
            Err(error) => {
                native_repository.close_owned().await.map_err(repository_error)?;
                return Err(error);
            }
        };
        let mut failed = RunningActor::start(native_repository, issuer, current.clone(), codec, reductions.clone()).await?;
        let refused = async {
            terminate_ordinary_grant(&inspector).await?;
            timeout(Duration::from_secs(2), grant_closed).await??;
            required(ordinary_watch(&failed.service, &current, member, credential, &mut calls).await
                .is_err_and(|error| error.code() == tonic::Code::Unavailable))?;
            required(submit(&failed.service, &request, credential, &mut calls).await
                .is_err_and(|error| error.code() == tonic::Code::Unavailable))?;
            admit(&mut calls)?;
            let request = rpc::JoinRoomRequest { room_code: journey::ROOM_CODE.to_owned(),
                join_secret: vec![0x91; 32], operation_id: Some(rpc::OperationId { value: Some(vec![0x91; 16]) }) };
            required(rpc::room_service_server::RoomService::join(&failed.service, Request::new(request)).await
                .is_err_and(|error| error.code() == tonic::Code::Unavailable))?;
            let observed = snapshot(&failed.service, &mut calls).await?;
            required(observed.calls_remaining == 16 && observed.checkpoint == current
                && !observed.fenced && !observed.uncertain && reductions.load(Ordering::SeqCst) == 2
                && durable(&inspector, 0x93).await? == physical)?;
            Ok::<_, Error>(())
        }.await;
        let mut exited = failed.close().await?;
        exited.issuer.close_owned().await.map_err(repository_error)?;
        refused?;
        required(exited.checkpoint == current)?;

        // Budget refusal must precede even transport repair. A healthy replacement
        // configuration makes an accidental reconnect observable as a new backend.
        let mut native_repository = repository(&configuration, &configuration, codec).await?;
        let (issuer, grant_closed) = match ordinary_grant_issuer(&grants, &grants, &current, fence).await {
            Ok(issuer) => issuer,
            Err(error) => {
                native_repository.close_owned().await.map_err(repository_error)?;
                return Err(error);
            }
        };
        let mut exhausted = RunningActor::start_observed(native_repository, issuer, current.clone(),
            codec, reductions.clone(), None, 0).await?;
        let exhausted_flow = async {
            terminate_ordinary_grant(&inspector).await?;
            timeout(Duration::from_secs(2), grant_closed).await??;
            required(ordinary_watch(&exhausted.service, &current, member, credential, &mut calls).await
                .is_err_and(|error| error.code() == tonic::Code::ResourceExhausted))?;
            required(submit(&exhausted.service, &request, credential, &mut calls).await
                .is_err_and(|error| error.code() == tonic::Code::ResourceExhausted))?;
            admit(&mut calls)?;
            let request = rpc::JoinRoomRequest { room_code: journey::ROOM_CODE.to_owned(),
                join_secret: vec![0x91; 32], operation_id: Some(rpc::OperationId { value: Some(vec![0x91; 16]) }) };
            required(rpc::room_service_server::RoomService::join(&exhausted.service, Request::new(request)).await
                .is_err_and(|error| error.code() == tonic::Code::ResourceExhausted))?;
            let replacements: i64 = inspector.query_one("SELECT count(*)::bigint FROM pg_stat_activity WHERE datname=current_database() AND application_name='df-ordinary-grant-issuer-01'", &[]).await?.try_get(0)?;
            let observed = exhausted.service.updates.borrow().clone();
            required(replacements == 0 && reductions.load(Ordering::SeqCst) == 2
                && observed == current
                && durable(&inspector, 0x93).await? == physical)?;
            Ok::<_, Error>(())
        }.await;
        let mut exited = exhausted.close().await?;
        exited.issuer.close_owned().await.map_err(repository_error)?;
        exhausted_flow?;
        required(exited.checkpoint == current && calls.count == 18 && Instant::now() < calls.deadline)?;
        let remaining: i64 = inspector.query_one("SELECT count(*)::bigint FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid() AND application_name LIKE 'df-ordinary-grant-%'", &[]).await?.try_get(0)?;
        required(remaining == 0)?;
        Ok::<_, Error>(calls.count)
    }.await;
    let released =
        local_demo_scope::release_owner(&inspector, initial.basis().session, fence).await;
    drop(inspector);
    let joined = join_driver(inspector_driver).await;
    released.map_err(repository_error)?;
    joined?;
    let calls = result?;
    std::fs::write(
        "/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/wave28-grant-recovery-20261006/ordinary-grant-recovery-report-02.json",
        format!(
            "{{\"pass\":true,\"pid\":{},\"actor_calls\":{calls},\"engine_reductions\":2,\"pending_ai\":0,\"ordinary_view_recovered\":true,\"exact_submit_and_join_replayed\":true,\"revoked_denied\":true,\"failed_reconnect_three_calls_refused\":true,\"canonical_rows_unchanged\":true,\"original_and_replacement_drivers_joined\":true,\"actor_threads_joined\":5,\"zero_budget_three_calls_refused_without_reconnect\":true,\"exact_fence_released\":true,\"inspector_joined\":true}}\n",
            std::process::id()
        ),
    )?;
    Ok(())
}

#[test]
#[ignore = "Root fresh owned grant_uncertain01 DB, ACK proxy55518 and finite native resource grant required"]
fn ordinary_submit_reconnect_keeps_the_exact_uncertain_operation() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(exercise_grant_recovery(true)).unwrap();
}

// Postimage for insertion in df-tools/src/gameplay/recovery_qualification.rs.
// This uses the registered native journey and physical Postgres owner. It is an
// ignored, operator-owned qualification, not an ordinary cargo test or a launch.

#[test]
#[ignore = "Root must register a fresh narrative_history01 PG database and release its owner"]
fn owned_pg_story_structure_survives_fenced_startup_restore_and_exact_retry() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(owned_pg_story_structure()).unwrap();
}

async fn owned_pg_story_structure() -> Result<(), Error> {
    required(
        std::env::var("DF_NARRATIVE_HISTORY_QUALIFICATION").as_deref()
            == Ok("owned-loopback-narrative-history01"),
    )?;
    let started = Instant::now();
    let mut configuration = trusted_configuration(55517, "df-narrative-history-inspector-01");
    configuration.dbname("df_gameplay_demo_20261007_narrative_history01");
    let (mut inspector, connection) =
        timeout(Duration::from_secs(2), configuration.connect(NoTls)).await??;
    let inspector_driver = tokio::spawn(connection);
    let codec = NativeCodecLimits {
        maximum_document_bytes: 1024 * 1024,
        maximum_allocated_bytes: 2 * 1024 * 1024,
        maximum_collection_items: 1024,
        maximum_text_bytes: 4096,
    };
    let cold = journey::initial().map_err(repository_error)?;
    let session = cold.basis().session;
    let first_fence = [0x83; 16];
    let second_fence = [0x84; 16];
    let admitted = local_demo_scope::admit_startup(
        &mut inspector,
        &cold,
        local_demo_scope::DemoStartupIdentity {
            fence: first_fence,
            player_credential: [0x81; 32],
            display_credential: [0x82; 32],
        },
        codec,
        &model::recovery().map_err(repository_error)?,
        |checkpoint| journey::phase(checkpoint).map(|_| ()),
    )
    .await
    .map_err(|_| io::Error::other("fresh source-qualified narrative database required"))?;
    required(!admitted.restored && admitted.checkpoint == cold)?;
    let (grant_client, grant_connection) =
        timeout(Duration::from_secs(2), configuration.connect(NoTls)).await??;
    let grant_driver = tokio::spawn(grant_connection);
    let issuer = LocalDemoScopeIssuer::new(
        tokio::runtime::Handle::current(),
        grant_client,
        session,
        first_fence,
    )
    .map_err(repository_error)?;
    let reductions = Arc::new(AtomicUsize::new(0));
    let mut running = RunningActor::start(
        repository(&configuration, &configuration, codec).await?,
        issuer,
        admitted.checkpoint,
        codec,
        reductions.clone(),
    )
    .await?;
    let mut calls = Calls {
        count: 0,
        deadline: started + Duration::from_secs(45),
    };
    let first_flow = async {
        let first = join(&running.service, 0x91, &mut calls).await?;
        let second = join(&running.service, 0x92, &mut calls).await?;
        let current = running.service.updates.borrow().clone();
        committed(submit(&running.service, &create(&current, 0x93), first, &mut calls).await?)?;
        let current = running.service.updates.borrow().clone();
        committed(
            submit(
                &running.service,
                &create(&current, 0x94),
                second,
                &mut calls,
            )
            .await?,
        )?;
        let current = running.service.updates.borrow().clone();
        let begin = action(&current, 0x95, rpc::GameplayActionKind::BeginStory);
        required(!committed(submit(&running.service, &begin, first, &mut calls).await?)?.replayed)?;
        let current = running.service.updates.borrow().clone();
        let escort = action(&current, 0x96, rpc::GameplayActionKind::EscortCourier);
        let receipt = committed(submit(&running.service, &escort, first, &mut calls).await?)?;
        required(!receipt.replayed)?;
        let current = running.service.updates.borrow().clone();
        required(
            journey::phase(&current).map_err(repository_error)? == rpc::JourneyPhase::Dialogue,
        )?;
        required(current.pins() == &model::pins().map_err(repository_error)?)?;
        required(
            current.state().narrative.completed_beats
                == [
                    model::content("room").map_err(repository_error)?,
                    model::content("opening").map_err(repository_error)?,
                ]
                && current.state().narrative.active_beats
                    == [model::content("courier-answer-escort").map_err(repository_error)?]
                && current.state().narrative.open_threads
                    == [model::content("sealed-packet-delivery-thread")
                        .map_err(repository_error)?]
                && current.state().narrative.accepted_facts.len() == 2,
        )?;
        for id in &current.state().narrative.accepted_facts {
            let fact = current
                .state()
                .facts
                .iter()
                .find(|fact| &fact.id == id)
                .ok_or_else(|| io::Error::other("narrative cause absent"))?;
            required(current.state().decisions.iter().any(|decision| {
                decision.operation == fact.operation
                    && decision.revision == fact.revision
                    && decision.facts.get(fact.ordinal as usize) == Some(id)
                    && decision.source_policy.as_str() == journey::THREAD_POLICY
            }))?;
        }
        let physical = durable(&inspector, 0x96).await?;
        required(
            physical.envelope
                == local_demo_scope::encode_owned_demo_checkpoint(&current, codec)
                    .map_err(repository_error)?,
        )?;
        Ok::<_, Error>((first, escort, receipt, current, physical))
    }
    .await;
    let first_closed = match running.close().await {
        Ok(mut exited) => {
            let closed = exited.issuer.close_owned().await.map_err(repository_error);
            closed.map(|()| exited)
        }
        Err(error) => Err(error),
    };
    let first_release = local_demo_scope::release_owner(&inspector, session, first_fence).await;
    let first_grant_join = join_driver(grant_driver).await;
    let exited = first_closed?;
    first_release.map_err(repository_error)?;
    first_grant_join?;
    let (first, escort, receipt, committed_checkpoint, physical) = first_flow?;
    required(exited.checkpoint == committed_checkpoint)?;

    let restored = local_demo_scope::admit_startup(
        &mut inspector,
        &cold,
        local_demo_scope::DemoStartupIdentity {
            fence: second_fence,
            player_credential: [0x81; 32],
            display_credential: [0x82; 32],
        },
        codec,
        &model::recovery().map_err(repository_error)?,
        |checkpoint| journey::phase(checkpoint).map(|_| ()),
    )
    .await
    .map_err(|_| io::Error::other("fenced narrative startup restore refused"))?;
    required(restored.restored && restored.checkpoint == committed_checkpoint)?;
    required(
        physical.envelope
            == local_demo_scope::encode_owned_demo_checkpoint(&restored.checkpoint, codec)
                .map_err(repository_error)?,
    )?;
    let (grant_client, grant_connection) =
        timeout(Duration::from_secs(2), configuration.connect(NoTls)).await??;
    let grant_driver = tokio::spawn(grant_connection);
    let issuer = LocalDemoScopeIssuer::new(
        tokio::runtime::Handle::current(),
        grant_client,
        session,
        second_fence,
    )
    .map_err(repository_error)?;
    let mut resumed = RunningActor::start(
        repository(&configuration, &configuration, codec).await?,
        issuer,
        restored.checkpoint,
        codec,
        reductions.clone(),
    )
    .await?;
    let before_retry = reductions.load(Ordering::SeqCst);
    let retry = async {
        let replay = committed(submit(&resumed.service, &escort, first, &mut calls).await?)?;
        required(
            replay.replayed
                && replay.outcome == receipt.outcome
                && replay.revision == receipt.revision
                && reductions.load(Ordering::SeqCst) == before_retry,
        )?;
        let current = snapshot(&resumed.service, &mut calls).await?;
        required(current.checkpoint == committed_checkpoint)?;
        required(durable(&inspector, 0x96).await? == physical)?;
        Ok::<_, Error>(())
    }
    .await;
    let second_closed = match resumed.close().await {
        Ok(mut exited) => {
            let closed = exited.issuer.close_owned().await.map_err(repository_error);
            closed.map(|()| exited)
        }
        Err(error) => Err(error),
    };
    let release = local_demo_scope::release_owner(&inspector, session, second_fence).await;
    let grant_join = join_driver(grant_driver).await;
    drop(inspector);
    let inspector_join = join_driver(inspector_driver).await;
    release.map_err(repository_error)?;
    grant_join?;
    inspector_join?;
    let exited = second_closed?;
    retry?;
    required(exited.checkpoint == committed_checkpoint && calls.count <= 20)?;
    Ok(())
}
