//! Operator-only mounted RPC evidence on a separately owned, empty local database.
use super::{Service, actor, hex, journey, model, wire};
use df_model::checkpoint::Checkpoint;
use df_persistence::local_demo_scope::{self, LocalDemoAuthority, LocalDemoScopeIssuer};
use df_persistence::{
    NativeCodecLimits, NativeRepositoryOptions, NativeTransactionBounds, PostgresRepository,
};
use df_protocol::common as rpc;
use df_session::inbox::{InboxHandle, bounded_inbox};
use df_session::submission::DurableOwner;
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Semaphore, oneshot, watch};
use tokio::time::timeout;
use tokio_postgres::{Config, NoTls};
use tonic::Request;
type Error = Box<dyn std::error::Error + Send + Sync>;
fn required(value: bool) -> Result<(), Error> {
    if value {
        Ok(())
    } else {
        Err(io::Error::other("native dialogue contract assertion").into())
    }
}
fn repository_error(error: impl std::fmt::Debug) -> Error {
    io::Error::other(format!("native dialogue repository {error:?}")).into()
}
fn configuration() -> Result<Config, Error> {
    required(
        std::env::var("DF_DIALOGUE_QUALIFICATION").as_deref() == Ok("owned-intent-dialogue-d02"),
    )?;
    let port = std::env::var("DF_DIALOGUE_PG_PORT")?.parse::<u16>()?;
    let database = std::env::var("DF_DIALOGUE_PG_DATABASE")?;
    let user = std::env::var("DF_DIALOGUE_PG_USER")?;
    required(
        port >= 1024
            && database == "df_gameplay_demo_intent_dialogue_d02"
            && user == "df_intent_dialogue_d02_owner",
    )?;
    let mut config = Config::new();
    config
        .host("127.0.0.1")
        .port(port)
        .dbname(&database)
        .user(&user)
        .ssl_mode(tokio_postgres::config::SslMode::Disable)
        .application_name("df-intent-dialogue-d02-owned-proof");
    Ok(config)
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
        Self::start_observed(repository, issuer, checkpoint, codec, counter, None, 100).await
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
        let actor = actor::Actor {
            dialogue: super::dialogue::DialogueState::default(),
            owner,
            bootstrap_credential: [0x81; 32],
            issuer,
            codec,
            fenced: false,
            recovery_wakeup: updates,
            intent_notifications,
            calls_remaining,
            qualification_inputs: Some(Vec::new()),
            qualification_joins: Some(Vec::new()),
            completion_retry: None,
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

fn authenticated<T>(value: T, credential: &[u8; 32]) -> Request<T> {
    let mut request = Request::new(value);
    request.metadata_mut().insert(
        "x-df-local-binding",
        hex(credential).parse().expect("fixed credential encoding"),
    );
    request
}
fn input_id(operation: u8) -> Option<rpc::OperationId> {
    Some(rpc::OperationId {
        value: Some(vec![operation; 16]),
    })
}
fn raw(
    checkpoint: &Checkpoint,
    operation: u8,
    context: rpc::DialogueContext,
    finality: rpc::DialogueFinality,
    offer: String,
    text: &str,
) -> rpc::RawDialogueRequest {
    let basis = checkpoint.basis();
    rpc::RawDialogueRequest {
        session_id: Some(rpc::SessionId {
            value: Some(basis.session.as_bytes().to_vec()),
        }),
        run_id: Some(rpc::RunId {
            value: Some(basis.run.as_bytes().to_vec()),
        }),
        observed_revision: Some(wire::revision(basis.revision)),
        input_id: input_id(operation),
        text: text.to_owned(),
        context: context as i32,
        finality: finality as i32,
        offer_id: offer,
    }
}
async fn counts(client: &tokio_postgres::Client) -> Result<[i64; 4], Error> {
    let row = client.query_one("SELECT (SELECT count(*) FROM df_game.operations), (SELECT count(*) FROM df_game.facts), (SELECT count(*) FROM df_game.checkpoints), (SELECT count(*) FROM df_game.intents)", &[]).await?;
    Ok([
        row.try_get(0)?,
        row.try_get(1)?,
        row.try_get(2)?,
        row.try_get(3)?,
    ])
}

async fn current_projection(
    sessions: &mut rpc::session_service_client::SessionServiceClient<tonic::transport::Channel>,
    checkpoint: &Checkpoint,
    credential: &[u8; 32],
    binding: rpc::ClientBindingId,
) -> Result<rpc::ViewMessage, Error> {
    let basis = checkpoint.basis();
    let mut stream = sessions
        .watch(authenticated(
            rpc::WatchViewRequest {
                session_id: Some(rpc::SessionId {
                    value: Some(basis.session.as_bytes().to_vec()),
                }),
                run_id: Some(rpc::RunId {
                    value: Some(basis.run.as_bytes().to_vec()),
                }),
                client_binding_id: Some(binding),
                after_revision: Some(wire::revision(basis.revision)),
            },
            credential,
        ))
        .await?
        .into_inner();
    timeout(Duration::from_secs(2), stream.message())
        .await??
        .ok_or_else(|| io::Error::other("mounted current projection absent").into())
}
fn pending_ai(checkpoint: &Checkpoint) -> usize {
    checkpoint
        .state()
        .intents
        .iter()
        .filter(|intent| {
            intent.kind == df_model::checkpoint::EffectKind::RunAi
                && intent.status == df_model::checkpoint::DurableStatus::Pending
        })
        .count()
}

async fn exercise() -> Result<(), Error> {
    let config = configuration()?;
    let (mut inspector, connection) =
        timeout(Duration::from_secs(2), config.connect(NoTls)).await??;
    let inspector_driver = tokio::spawn(connection);
    let codec = NativeCodecLimits {
        maximum_document_bytes: 1024 * 1024,
        maximum_allocated_bytes: 2 * 1024 * 1024,
        maximum_collection_items: 1024,
        maximum_text_bytes: 4096,
    };
    let initial = journey::initial().map_err(repository_error)?;
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
    .map_err(repository_error)?;
    let (grant_client, grant_connection) =
        timeout(Duration::from_secs(2), config.connect(NoTls)).await??;
    let grant_driver = tokio::spawn(grant_connection);
    let mut issuer = LocalDemoScopeIssuer::new(
        tokio::runtime::Handle::current(),
        grant_client,
        initial.basis().session,
        fence,
    )
    .map_err(repository_error)?;
    issuer
        .configure_reconnect(config.clone(), grant_driver)
        .map_err(|(error, _)| repository_error(error))?;
    let counter = Arc::new(AtomicUsize::new(0));
    let mut running = RunningActor::start(
        repository(&config, &config, codec).await?,
        issuer,
        initial,
        codec,
        counter.clone(),
    )
    .await?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = listener.local_addr()?;
    let (stop, shutdown) = oneshot::channel();
    let service = running.service.clone();
    let server = tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(
                rpc::dialogue_service_server::DialogueServiceServer::new(service.clone())
                    .max_decoding_message_size(8192)
                    .max_encoding_message_size(8192),
            )
            .add_service(rpc::room_service_server::RoomServiceServer::new(
                service.clone(),
            ))
            .add_service(rpc::action_service_server::ActionServiceServer::new(
                service.clone(),
            ))
            .add_service(rpc::session_service_server::SessionServiceServer::new(
                service,
            ))
            .serve_with_incoming_shutdown(
                tokio_stream::wrappers::TcpListenerStream::new(listener),
                async {
                    let _ = shutdown.await;
                },
            ),
    );
    let result = async {
        let channel = tonic::transport::Endpoint::from_shared(format!("http://{address}"))?
            .connect()
            .await?;
        let mut rooms = rpc::room_service_client::RoomServiceClient::new(channel.clone());
        let mut dialogue =
            rpc::dialogue_service_client::DialogueServiceClient::new(channel.clone());
        let mut actions = rpc::action_service_client::ActionServiceClient::new(channel.clone());
        let mut sessions = rpc::session_service_client::SessionServiceClient::new(channel);
        let mut credentials = Vec::new();
        let mut bindings = Vec::new();
        for operation in [0x91_u8, 0x92] {
            let joined = rooms
                .join(rpc::JoinRoomRequest {
                    room_code: journey::ROOM_CODE.to_owned(),
                    join_secret: vec![operation; 32],
                    operation_id: input_id(operation),
                })
                .await?
                .into_inner();
            let Some(rpc::join_room_response::Outcome::Joined(joined)) = joined.outcome else {
                return Err(io::Error::other("actual mounted room join refused").into());
            };
            required(joined.local_binding.len() == 64)?;
            let mut credential = [0_u8; 32];
            for (i, pair) in joined
                .local_binding
                .as_bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .enumerate()
            {
                let digit = |b| match b {
                    b'0'..=b'9' => Ok(b - b'0'),
                    b'a'..=b'f' => Ok(b - b'a' + 10),
                    _ => Err(io::Error::other("native credential encoding")),
                };
                credential[i] = digit(pair[0])? * 16 + digit(pair[1])?;
            }
            bindings.push(
                joined
                    .client_binding_id
                    .clone()
                    .ok_or_else(|| io::Error::other("joined member binding absent"))?,
            );
            credentials.push(credential);
        }
        let first = credentials[0];
        let second = credentials[1];
        let baseline = running.service.updates.borrow().clone();
        let before = counts(&inspector).await?;
        let preparations = counter.load(Ordering::SeqCst);
        for (index, (context, text)) in [
            (rpc::DialogueContext::Question, "Can I attack the courier?"),
            (
                rpc::DialogueContext::Joke,
                "I attack the moon, just kidding",
            ),
            (rpc::DialogueContext::Meta, "Pause the game"),
            (rpc::DialogueContext::PlanOnly, "Later I might attack"),
            (rpc::DialogueContext::Social, "Thanks, that was funny"),
            (rpc::DialogueContext::Unspecified, "attack the courier"),
        ]
        .into_iter()
        .enumerate()
        {
            let response = dialogue
                .submit(authenticated(
                    raw(
                        &baseline,
                        0x20 + index as u8,
                        context,
                        rpc::DialogueFinality::Final,
                        String::new(),
                        text,
                    ),
                    &first,
                ))
                .await?
                .into_inner();
            required(
                response.disposition != rpc::DialogueDisposition::Action as i32
                    && response.confirmation_token.is_empty()
                    && !response.confirmation_required,
            )?;
        }
        let create_offer = journey::offer_id(&baseline, rpc::GameplayActionKind::CreateCharacter);
        let partial = dialogue
            .submit(authenticated(
                raw(
                    &baseline,
                    0x30,
                    rpc::DialogueContext::Unspecified,
                    rpc::DialogueFinality::Partial,
                    create_offer.clone(),
                    "create",
                ),
                &first,
            ))
            .await?
            .into_inner();
        required(partial.confirmation_token.is_empty())?;
        let question_offer = dialogue
            .submit(authenticated(
                raw(
                    &baseline,
                    0x36,
                    rpc::DialogueContext::Question,
                    rpc::DialogueFinality::Final,
                    create_offer.clone(),
                    "Could I create a character?",
                ),
                &first,
            ))
            .await?
            .into_inner();
        required(
            question_offer.confirmation_token.is_empty() && !question_offer.confirmation_required,
        )?;
        let absent = dialogue
            .submit(authenticated(
                raw(
                    &baseline,
                    0x31,
                    rpc::DialogueContext::Unspecified,
                    rpc::DialogueFinality::Final,
                    "unsupported".to_owned(),
                    "attack",
                ),
                &first,
            ))
            .await;
        required(
            absent
                .err()
                .is_some_and(|error| error.code() == tonic::Code::FailedPrecondition),
        )?;
        required(
            counts(&inspector).await? == before && counter.load(Ordering::SeqCst) == preparations,
        )?;
        required(*running.service.updates.borrow() == baseline)?;

        let cancelled = dialogue
            .submit(authenticated(
                raw(
                    &baseline,
                    0x32,
                    rpc::DialogueContext::Unspecified,
                    rpc::DialogueFinality::Final,
                    create_offer.clone(),
                    "perhaps",
                ),
                &first,
            ))
            .await?
            .into_inner();
        required(
            cancelled.input_id == input_id(0x32)
                && cancelled.disposition == rpc::DialogueDisposition::Clarify as i32
                && cancelled.confirmation_required
                && cancelled.confirmation_token.len() == 32
                && cancelled.revision == Some(wire::revision(baseline.basis().revision)),
        )?;
        let before_decline = counts(&inspector).await?;
        let cancelled_action = create(&baseline, 0x40);
        let confirm = rpc::ConfirmDialogueRequest {
            input_id: input_id(0x32),
            confirmation_token: cancelled.confirmation_token.clone(),
            action: Some(cancelled_action),
        };
        let declined = dialogue
            .cancel(authenticated(
                rpc::CancelDialogueRequest {
                    input_id: input_id(0x32),
                    confirmation_token: cancelled.confirmation_token,
                },
                &first,
            ))
            .await?
            .into_inner();
        required(
            declined.input_id == input_id(0x32)
                && declined.disposition == rpc::DialogueDisposition::Rejected as i32
                && !declined.confirmation_required
                && declined.confirmation_token.is_empty()
                && declined.revision == Some(wire::revision(baseline.basis().revision))
                && *running.service.updates.borrow() == baseline
                && counts(&inspector).await? == before_decline
                && counter.load(Ordering::SeqCst) == preparations,
        )?;
        required(
            dialogue
                .confirm(authenticated(confirm, &first))
                .await
                .err()
                .is_some_and(|error| error.code() == tonic::Code::FailedPrecondition),
        )?;

        let pending = dialogue
            .submit(authenticated(
                raw(
                    &baseline,
                    0x33,
                    rpc::DialogueContext::Unspecified,
                    rpc::DialogueFinality::Final,
                    create_offer,
                    "maybe create",
                ),
                &first,
            ))
            .await?
            .into_inner();
        let create_action = create(&baseline, 0x41);
        let confirm = rpc::ConfirmDialogueRequest {
            input_id: input_id(0x33),
            confirmation_token: pending.confirmation_token,
            action: Some(create_action.clone()),
        };
        required(
            dialogue
                .confirm(authenticated(confirm.clone(), &second))
                .await
                .err()
                .is_some_and(|error| error.code() == tonic::Code::FailedPrecondition),
        )?;
        required(
            counts(&inspector).await? == before && counter.load(Ordering::SeqCst) == preparations,
        )?;
        let committed = dialogue
            .confirm(authenticated(confirm.clone(), &first))
            .await?
            .into_inner();
        let Some(rpc::submit_action_response::Outcome::CommittedDecision(created_receipt)) =
            committed.outcome.as_ref()
        else {
            return Err(io::Error::other("confirmed registered creation receipt absent").into());
        };
        required(
            !created_receipt.replayed
                && matches!(
                    &created_receipt.outcome,
                    Some(rpc::decision_receipt::Outcome::Accepted(_))
                ),
        )?;
        required(
            running.service.updates.borrow().state().characters.len()
                == baseline.state().characters.len() + 1,
        )?;
        required(counter.load(Ordering::SeqCst) == preparations + 1)?;
        let after_commit = counts(&inspector).await?;
        let committed_checkpoint = running.service.updates.borrow().clone();
        required(committed_checkpoint.basis().revision != baseline.basis().revision)?;
        let player_projection = current_projection(
            &mut sessions,
            &committed_checkpoint,
            &first,
            bindings[0].clone(),
        )
        .await?;
        let display_projection = current_projection(
            &mut sessions,
            &committed_checkpoint,
            &[0x82; 32],
            rpc::ClientBindingId {
                value: Some(vec![0x72; 16]),
            },
        )
        .await?;
        required(
            player_projection.revision
                == Some(wire::revision(committed_checkpoint.basis().revision)),
        )?;
        required(display_projection.revision == player_projection.revision)?;
        let Some(rpc::view_message::Audience::Player(player)) = player_projection.audience.as_ref()
        else {
            return Err(io::Error::other("player watch audience absent").into());
        };
        let Some(rpc::view_message::Audience::Display(display)) =
            display_projection.audience.as_ref()
        else {
            return Err(io::Error::other("display watch audience absent").into());
        };
        let own = player
            .journey
            .as_ref()
            .and_then(|journey| journey.own_character.as_ref())
            .ok_or_else(|| {
                io::Error::other("confirmed character missing from current player projection")
            })?;
        let shared = display
            .journey
            .as_ref()
            .ok_or_else(|| io::Error::other("current display journey absent"))?;
        required(shared.own_character.is_none() && shared.creation.is_none())?;
        required(
            player
                .journey
                .as_ref()
                .is_some_and(|journey| journey.party == shared.party),
        )?;
        let private_sheet = prost::Message::encode_to_vec(own);
        let display_bytes = prost::Message::encode_to_vec(&display_projection);
        required(
            !private_sheet.is_empty()
                && !display_bytes
                    .windows(private_sheet.len())
                    .any(|window| window == private_sheet),
        )?;
        required(
            counts(&inspector).await? == after_commit
                && counter.load(Ordering::SeqCst) == preparations + 1,
        )?;

        let replay = dialogue
            .confirm(authenticated(confirm.clone(), &first))
            .await?
            .into_inner();
        let mut replay_expected = committed;
        let Some(rpc::submit_action_response::Outcome::CommittedDecision(receipt)) =
            replay_expected.outcome.as_mut()
        else {
            return Err(io::Error::other("committed receipt absent").into());
        };
        receipt.replayed = true;
        required(
            replay == replay_expected
                && counts(&inspector).await? == after_commit
                && counter.load(Ordering::SeqCst) == preparations + 1,
        )?;
        let mut changed = confirm;
        changed
            .action
            .as_mut()
            .ok_or_else(|| io::Error::other("fixture action absent"))?
            .operation_id = input_id(0x42);
        required(
            dialogue
                .confirm(authenticated(changed, &first))
                .await
                .err()
                .is_some_and(|error| error.code() == tonic::Code::FailedPrecondition),
        )?;

        let second_current = running.service.updates.borrow().clone();
        let second_created = actions
            .submit(authenticated(create(&second_current, 0x45), &second))
            .await?
            .into_inner();
        required(matches!(
            &second_created.outcome,
            Some(rpc::submit_action_response::Outcome::CommittedDecision(_))
        ))?;
        let current = running.service.updates.borrow().clone();
        let stale = dialogue
            .submit(authenticated(
                raw(
                    &current,
                    0x34,
                    rpc::DialogueContext::Unspecified,
                    rpc::DialogueFinality::Final,
                    journey::offer_id(&current, rpc::GameplayActionKind::BeginStory),
                    "perhaps begin",
                ),
                &second,
            ))
            .await?
            .into_inner();
        let begin = action(&current, 0x43, rpc::GameplayActionKind::BeginStory);
        let stale_confirm = rpc::ConfirmDialogueRequest {
            input_id: input_id(0x34),
            confirmation_token: stale.confirmation_token,
            action: Some(begin.clone()),
        };

        let entered = actions
            .submit(authenticated(begin, &first))
            .await?
            .into_inner();
        required(matches!(
            &entered.outcome,
            Some(rpc::submit_action_response::Outcome::CommittedDecision(_))
        ))?;
        let opening = running.service.updates.borrow().clone();
        let after_enter = counts(&inspector).await?;
        let enter_preparations = counter.load(Ordering::SeqCst);
        required(
            dialogue
                .confirm(authenticated(stale_confirm, &second))
                .await
                .err()
                .is_some_and(|error| error.code() == tonic::Code::FailedPrecondition),
        )?;
        required(
            counts(&inspector).await? == after_enter
                && counter.load(Ordering::SeqCst) == enter_preparations,
        )?;

        let cancelled_pending = dialogue
            .submit(authenticated(
                raw(
                    &opening,
                    0x6a,
                    rpc::DialogueContext::Unspecified,
                    rpc::DialogueFinality::Final,
                    journey::offer_id(&opening, rpc::GameplayActionKind::EscortCourier),
                    "perhaps escort",
                ),
                &second,
            ))
            .await?
            .into_inner();
        required(!cancelled_pending.confirmation_token.is_empty())?;
        let selected_begin = dialogue
            .submit(authenticated(
                raw(
                    &opening,
                    0x36,
                    rpc::DialogueContext::Unspecified,
                    rpc::DialogueFinality::Final,
                    journey::offer_id(&opening, rpc::GameplayActionKind::AskCourier),
                    "perhaps ask",
                ),
                &first,
            ))
            .await?
            .into_inner();
        let typed = dialogue
            .confirm(authenticated(
                rpc::ConfirmDialogueRequest {
                    input_id: input_id(0x36),
                    confirmation_token: selected_begin.confirmation_token,
                    action: Some(action(&opening, 0x46, rpc::GameplayActionKind::AskCourier)),
                },
                &first,
            ))
            .await?
            .into_inner();
        required(matches!(
            &typed.outcome,
            Some(rpc::submit_action_response::Outcome::CommittedDecision(_))
        ))?;
        let pending_checkpoint = running.service.updates.borrow().clone();
        required(pending_ai(&pending_checkpoint) > 0)?;
        let pending_rows = counts(&inspector).await?;
        let pending_preparations = counter.load(Ordering::SeqCst);
        let mut pending_updates = running.service.updates.clone();
        pending_updates.borrow_and_update();
        for (index, context) in [
            rpc::DialogueContext::Question,
            rpc::DialogueContext::Joke,
            rpc::DialogueContext::Meta,
            rpc::DialogueContext::PlanOnly,
            rpc::DialogueContext::Social,
            rpc::DialogueContext::Unspecified,
        ]
        .into_iter()
        .enumerate()
        {
            for finality in [rpc::DialogueFinality::Partial, rpc::DialogueFinality::Final] {
                let response = dialogue
                    .submit(authenticated(
                        raw(
                            &pending_checkpoint,
                            0x60 + index as u8,
                            context,
                            finality,
                            String::new(),
                            "attack the courier?",
                        ),
                        &first,
                    ))
                    .await?
                    .into_inner();
                required(response.disposition != rpc::DialogueDisposition::Action as i32)?;
            }
        }
        let refused_raw = raw(
            &pending_checkpoint,
            0x69,
            rpc::DialogueContext::Question,
            rpc::DialogueFinality::Final,
            String::new(),
            "attack the courier?",
        );
        required(
            dialogue
                .submit(Request::new(refused_raw.clone()))
                .await
                .err()
                .is_some_and(|error| error.code() == tonic::Code::Unauthenticated),
        )?;
        // The valid bootstrap grant reaches Actor authentication but has no joined member.
        required(
            dialogue
                .submit(authenticated(refused_raw, &[0x81; 32]))
                .await
                .is_err(),
        )?;
        required(
            dialogue
                .cancel(authenticated(
                    rpc::CancelDialogueRequest {
                        input_id: input_id(0x69),
                        confirmation_token: vec![0; 32],
                    },
                    &first,
                ))
                .await
                .is_err(),
        )?;
        dialogue
            .cancel(authenticated(
                rpc::CancelDialogueRequest {
                    input_id: input_id(0x6a),
                    confirmation_token: cancelled_pending.confirmation_token,
                },
                &second,
            ))
            .await?;
        required(
            inspector
                .execute(
                    "UPDATE df_local_demo.grants SET active=false WHERE credential=$1::bytea",
                    &[&first.as_slice()],
                )
                .await?
                == 1,
        )?;
        required(
            dialogue
                .submit(authenticated(
                    raw(
                        &pending_checkpoint,
                        0x6b,
                        rpc::DialogueContext::Question,
                        rpc::DialogueFinality::Final,
                        String::new(),
                        "can I attack?",
                    ),
                    &first,
                ))
                .await
                .is_err(),
        )?;
        required(
            inspector
                .execute(
                    "UPDATE df_local_demo.grants SET active=true WHERE credential=$1::bytea",
                    &[&first.as_slice()],
                )
                .await?
                == 1,
        )?;
        required(
            *running.service.updates.borrow() == pending_checkpoint
                && counts(&inspector).await? == pending_rows
                && counter.load(Ordering::SeqCst) == pending_preparations
                && !pending_updates.has_changed()?,
        )?;
        // A deliberate, authenticated Session watch is the ordinary non-dialogue poll path.
        let polled = current_projection(
            &mut sessions,
            &pending_checkpoint,
            &first,
            bindings[0].clone(),
        )
        .await?;
        let progressed = running.service.updates.borrow().clone();
        required(
            pending_ai(&progressed) < pending_ai(&pending_checkpoint)
                && progressed.basis().revision > pending_checkpoint.basis().revision
                && polled.revision == Some(wire::revision(progressed.basis().revision)),
        )?;

        let Some(rpc::view_message::Audience::Player(private_player)) = polled.audience.as_ref()
        else {
            return Err(io::Error::other("completed private player watch absent").into());
        };
        required(private_player.private_clue == super::courier_ai::RESPONSE)?;
        let other_player =
            current_projection(&mut sessions, &progressed, &second, bindings[1].clone()).await?;
        let shared_display = current_projection(
            &mut sessions,
            &progressed,
            &[0x82; 32],
            rpc::ClientBindingId {
                value: Some(vec![0x72; 16]),
            },
        )
        .await?;
        required(
            other_player.revision == polled.revision && shared_display.revision == polled.revision,
        )?;
        let Some(rpc::view_message::Audience::Player(other)) = other_player.audience.as_ref()
        else {
            return Err(io::Error::other("other player watch absent").into());
        };
        required(
            other.private_clue.is_empty()
                && matches!(
                    shared_display.audience,
                    Some(rpc::view_message::Audience::Display(_))
                ),
        )?;
        for bytes in [
            prost::Message::encode_to_vec(&other_player),
            prost::Message::encode_to_vec(&shared_display),
        ] {
            required(
                !bytes
                    .windows(super::courier_ai::RESPONSE.len())
                    .any(|window| window == super::courier_ai::RESPONSE.as_bytes()),
            )?;
        }
        let after_advance = counts(&inspector).await?;
        let advance_preparations = counter.load(Ordering::SeqCst);

        let current = running.service.updates.borrow().clone();
        let revoked = dialogue
            .submit(authenticated(
                raw(
                    &current,
                    0x35,
                    rpc::DialogueContext::Unspecified,
                    rpc::DialogueFinality::Final,
                    journey::offer_id(&current, rpc::GameplayActionKind::DefendCourier),
                    "perhaps defend",
                ),
                &first,
            ))
            .await?
            .into_inner();
        let revoked_confirm = rpc::ConfirmDialogueRequest {
            input_id: input_id(0x35),
            confirmation_token: revoked.confirmation_token,
            action: Some(action(
                &current,
                0x44,
                rpc::GameplayActionKind::DefendCourier,
            )),
        };
        let changed = inspector
            .execute(
                "UPDATE df_local_demo.grants SET active=false WHERE credential=$1::bytea",
                &[&first.as_slice()],
            )
            .await?;
        required(changed == 1)?;
        required(
            dialogue
                .confirm(authenticated(revoked_confirm, &first))
                .await
                .err()
                .is_some_and(|error| {
                    matches!(
                        error.code(),
                        tonic::Code::Unavailable
                            | tonic::Code::PermissionDenied
                            | tonic::Code::Unauthenticated
                    )
                }),
        )?;
        required(
            counts(&inspector).await? == after_advance
                && counter.load(Ordering::SeqCst) == advance_preparations,
        )?;
        Ok::<_, Error>(second)
    }
    .await;
    let _ = stop.send(());
    timeout(Duration::from_secs(3), server).await???;
    let mut exited = running.close().await?;
    let result = match result {
        Ok(second) => {
            uncertain_confirmation(
                &config,
                exited.checkpoint,
                exited.issuer,
                codec,
                second,
                &inspector,
            )
            .await
        }
        Err(error) => {
            exited
                .issuer
                .close_owned()
                .await
                .map_err(repository_error)?;
            Err(error)
        }
    };
    drop(inspector);
    timeout(Duration::from_secs(2), inspector_driver).await???;
    result
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Root-owned separately registered PostgreSQL runtime release"]
async fn mounted_native_dialogue_confirmation_contract() -> Result<(), Error> {
    timeout(Duration::from_secs(45), exercise()).await?
}

async fn uncertain_confirmation(
    config: &Config,
    baseline: Checkpoint,
    issuer: LocalDemoScopeIssuer,
    codec: NativeCodecLimits,
    credential: [u8; 32],
    inspector: &tokio_postgres::Client,
) -> Result<(), Error> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let proxy_address = listener.local_addr()?;
    let postgres_port = std::env::var("DF_DIALOGUE_PG_PORT")?.parse::<u16>()?;
    let mut proxy = tokio::spawn(async move {
        let (frontend, _) = timeout(Duration::from_secs(5), listener.accept())
            .await
            .map_err(|_| local_demo_scope::ProxyError::Deadline)?
            .map_err(|_| local_demo_scope::ProxyError::Io)?;
        drop(listener);
        let postgres = timeout(
            Duration::from_secs(2),
            tokio::net::TcpStream::connect(("127.0.0.1", postgres_port)),
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
                deadline: tokio::time::Instant::now() + Duration::from_secs(25),
            },
        )
        .await
    });
    let mut proxied = Config::new();
    proxied
        .host("127.0.0.1")
        .port(proxy_address.port())
        .dbname("df_gameplay_demo_intent_dialogue_d02")
        .user("df_intent_dialogue_d02_owner")
        .ssl_mode(tokio_postgres::config::SslMode::Disable)
        .application_name("df-intent-dialogue-d02-commit-ack-proof");
    let counter = Arc::new(AtomicUsize::new(0));
    let mut running = RunningActor::start(
        repository(&proxied, config, codec).await?,
        issuer,
        baseline.clone(),
        codec,
        counter.clone(),
    )
    .await?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = listener.local_addr()?;
    let (stop, shutdown) = oneshot::channel();
    let server = tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(
                rpc::dialogue_service_server::DialogueServiceServer::new(running.service.clone())
                    .max_decoding_message_size(8192)
                    .max_encoding_message_size(8192),
            )
            .serve_with_incoming_shutdown(
                tokio_stream::wrappers::TcpListenerStream::new(listener),
                async {
                    let _ = shutdown.await;
                },
            ),
    );
    let result = async {
        let channel = tonic::transport::Endpoint::from_shared(format!("http://{address}"))?
            .connect()
            .await?;
        let mut dialogue = rpc::dialogue_service_client::DialogueServiceClient::new(channel);
        let offer = journey::offer_id(&baseline, rpc::GameplayActionKind::DefendCourier);
        let pending = dialogue
            .submit(authenticated(
                raw(
                    &baseline,
                    0x51,
                    rpc::DialogueContext::Unspecified,
                    rpc::DialogueFinality::Final,
                    offer,
                    "perhaps defend",
                ),
                &credential,
            ))
            .await?
            .into_inner();
        required(pending.confirmation_token.len() == 32)?;
        let confirm = rpc::ConfirmDialogueRequest {
            input_id: input_id(0x51),
            confirmation_token: pending.confirmation_token,
            action: Some(action(
                &baseline,
                0x50,
                rpc::GameplayActionKind::DefendCourier,
            )),
        };
        let first = dialogue
            .confirm(authenticated(confirm.clone(), &credential))
            .await;
        required(
            first
                .err()
                .is_some_and(|error| error.code() == tonic::Code::Unavailable),
        )?;
        required(
            counter.load(Ordering::SeqCst) == 1 && *running.service.updates.borrow() == baseline,
        )?;
        let committed_rows = counts(inspector).await?;
        let row = inspector
            .query_one(
                "SELECT receipt FROM df_game.operations WHERE operation_id=$1::bytea",
                &[&[0x50_u8; 16].as_slice()],
            )
            .await?;
        let durable_receipt: Vec<u8> = row.try_get(0)?;
        required(!durable_receipt.is_empty() && durable_receipt.len() <= 4096)?;
        let unsafe_next = dialogue
            .submit(authenticated(
                raw(
                    &baseline,
                    0x52,
                    rpc::DialogueContext::Unspecified,
                    rpc::DialogueFinality::Final,
                    String::new(),
                    "do another action",
                ),
                &credential,
            ))
            .await;
        required(
            unsafe_next
                .err()
                .is_some_and(|error| error.code() == tonic::Code::Unavailable),
        )?;
        // A confirmed operation may already be committed even when its
        // acknowledgement is uncertain. Cancellation cannot discard its retry.
        required(
            dialogue
                .cancel(authenticated(
                    rpc::CancelDialogueRequest {
                        input_id: confirm.input_id.clone(),
                        confirmation_token: confirm.confirmation_token.clone(),
                    },
                    &credential,
                ))
                .await
                .err()
                .is_some_and(|error| error.code() == tonic::Code::FailedPrecondition),
        )?;
        required(
            counts(inspector).await? == committed_rows && counter.load(Ordering::SeqCst) == 1,
        )?;
        let mut changed = confirm.clone();
        changed
            .action
            .as_mut()
            .ok_or_else(|| io::Error::other("confirmation action absent"))?
            .operation_id = input_id(0x53);
        required(
            dialogue
                .confirm(authenticated(changed, &credential))
                .await
                .err()
                .is_some_and(|error| error.code() == tonic::Code::FailedPrecondition),
        )?;
        required(
            counts(inspector).await? == committed_rows && counter.load(Ordering::SeqCst) == 1,
        )?;
        let replay = dialogue
            .confirm(authenticated(confirm, &credential))
            .await?
            .into_inner();
        let Some(rpc::submit_action_response::Outcome::CommittedDecision(receipt)) = replay.outcome
        else {
            return Err(io::Error::other(
                "exact confirmed uncertainty retry did not return stored receipt",
            )
            .into());
        };
        required(
            receipt.replayed
                && matches!(
                    &receipt.outcome,
                    Some(rpc::decision_receipt::Outcome::Accepted(_))
                ),
        )?;
        required(
            counts(inspector).await? == committed_rows && counter.load(Ordering::SeqCst) == 1,
        )?;
        let after: Vec<u8> = inspector
            .query_one(
                "SELECT receipt FROM df_game.operations WHERE operation_id=$1::bytea",
                &[&[0x50_u8; 16].as_slice()],
            )
            .await?
            .try_get(0)?;
        required(
            after == durable_receipt
                && running.service.updates.borrow().basis().revision != baseline.basis().revision,
        )?;
        Ok::<_, Error>(())
    }
    .await;
    let _ = stop.send(());
    timeout(Duration::from_secs(3), server).await???;
    let mut exited = running.close().await?;
    exited
        .issuer
        .close_owned()
        .await
        .map_err(repository_error)?;
    let observed = match timeout(Duration::from_secs(3), &mut proxy).await {
        Ok(joined) => {
            joined?.map_err(|error| io::Error::other(format!("owned commit proxy {error:?}")))?
        }
        Err(_) => {
            proxy.abort();
            let _ = timeout(Duration::from_secs(2), proxy).await;
            return Err(io::Error::other("owned commit proxy join deadline").into());
        }
    };
    required(
        observed.frontend_commit_forwarded
            && observed.postgres_commit_complete_observed
            && observed.postgres_ready_idle_observed
            && observed.suppressed_response_bytes > 0,
    )?;
    result?;
    println!(
        "native dialogue lost-ack witness: commit complete, ready idle, suppressed bytes {}, exact receipt retry, one mechanics preparation",
        observed.suppressed_response_bytes
    );
    Ok(())
}
