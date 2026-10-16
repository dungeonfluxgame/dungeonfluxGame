//! Operator-only mounted RPC evidence on a separately owned, empty local database.
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
use serde_json::{Value, json};
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
fn npc_event_configuration() -> Result<Config, Error> {
    required(std::env::var("DF_NPC_EVENT_QUALIFICATION").as_deref() == Ok("owned-npc-event-e01"))?;
    let port = std::env::var("DF_DIALOGUE_PG_PORT")?.parse::<u16>()?;
    let mut config = Config::new();
    config
        .host("127.0.0.1")
        .port(port)
        .dbname("df_gameplay_demo_npc_event_e01")
        .user("df_npc_event_e01_owner")
        .ssl_mode(tokio_postgres::config::SslMode::Disable)
        .application_name("df-npc-event-e01-owned-proof");
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
        issuer: LocalDemoScopeIssuer,
        checkpoint: Checkpoint,
        codec: NativeCodecLimits,
        counter: Arc<AtomicUsize>,
        completion_observer: Option<actor::CompletionObserver>,
        calls_remaining: u16,
    ) -> Result<Self, Error> {
        Self::start_observed_with_npc(
            repository,
            issuer,
            checkpoint,
            codec,
            counter,
            super::social::SocialLedger::none(),
            Arc::new(super::npc_reply::NpcVoice::silent()),
            completion_observer,
            calls_remaining,
        )
        .await
    }
    async fn start_with_npc(
        repository: PostgresRepository<LocalDemoAuthority>,
        issuer: LocalDemoScopeIssuer,
        checkpoint: Checkpoint,
        codec: NativeCodecLimits,
        counter: Arc<AtomicUsize>,
        social: Arc<super::social::SocialLedger>,
        npc_voice: Arc<super::npc_reply::NpcVoice>,
    ) -> Result<Self, Error> {
        Self::start_observed_with_npc(
            repository, issuer, checkpoint, codec, counter, social, npc_voice, None, 100,
        )
        .await
    }
    async fn start_observed_with_npc(
        repository: PostgresRepository<LocalDemoAuthority>,
        mut issuer: LocalDemoScopeIssuer,
        checkpoint: Checkpoint,
        codec: NativeCodecLimits,
        counter: Arc<AtomicUsize>,
        social: Arc<super::social::SocialLedger>,
        npc_voice: Arc<super::npc_reply::NpcVoice>,
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
        let table_log = super::table_log::TableLog::new();
        let actor = actor::Actor {
            table_log: table_log.clone(),
            notebook: super::notebook::NotebookLedger::none(),
            social: social.clone(),
            table_settings: super::table_settings::TableSettings::startup(None, false),
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
            dev_history: None,
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
            display_credential: [0x82; 32],
            layout: super::dev_control::layout::LayoutState::new(),
            actor: sender.clone(),
            updates: receiver,
            streams: Arc::new(Semaphore::new(6)),
            voice: super::voice::Voice::disabled(
                super::startup_configuration::VoiceOff::Qualification,
            ),
            npc_voice,
            table_log,
            social,
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
fn create(checkpoint: &Checkpoint, operation: u8) -> rpc::SubmitActionRequest {
    let mut body = action(
        checkpoint,
        operation,
        rpc::GameplayActionKind::CreateCharacter,
    );
    body.character = Some(rpc::CharacterSelection {
        identity: None,
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
fn create_named(checkpoint: &Checkpoint, operation: u8, name: &str) -> rpc::SubmitActionRequest {
    let mut request = create(checkpoint, operation);
    if let Some(character) = request.character.as_mut() {
        character.name = name.to_owned();
    }
    request
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

fn event_line_proposal(
    actor_id: &str,
    npc_id: &str,
    item_id: &str,
    recipient_id: &str,
    scene_identity: &str,
    revision: &str,
    typed_words: &str,
    reply_text: &str,
    source_line_sequence: u64,
) -> super::npc_event_classifier::Proposal {
    use super::npc_event_classifier::{EventKind, Proposal};
    Proposal {
        kind: EventKind::IntentToGive,
        actor_id: actor_id.to_owned(),
        npc_id: npc_id.to_owned(),
        item_id: Some(item_id.to_owned()),
        recipient_id: Some(recipient_id.to_owned()),
        reply_text: reply_text.to_owned(),
        typed_words: typed_words.to_owned(),
        scene_identity: scene_identity.to_owned(),
        source_line_id: Proposal::source_line_id(
            actor_id,
            npc_id,
            scene_identity,
            revision,
            typed_words,
            reply_text,
        ),
        source_line_sequence: Some(source_line_sequence),
        revision: revision.to_owned(),
    }
}

async fn direct_join(service: &Service, operation: u8) -> Result<rpc::JoinedRoom, Error> {
    let response = rpc::room_service_server::RoomService::join(
        service,
        Request::new(rpc::JoinRoomRequest {
            room_code: journey::ROOM_CODE.to_owned(),
            join_secret: vec![operation; 32],
            operation_id: input_id(operation),
        }),
    )
    .await?
    .into_inner();
    match response.outcome {
        Some(rpc::join_room_response::Outcome::Joined(joined)) => Ok(*joined),
        _ => Err(io::Error::other("NPC-event room join refused").into()),
    }
}

async fn direct_submit_action(
    service: &Service,
    body: rpc::SubmitActionRequest,
    credential: &[u8; 32],
) -> Result<rpc::SubmitActionResponse, Error> {
    Ok(
        rpc::action_service_server::ActionService::submit(service, authenticated(body, credential))
            .await?
            .into_inner(),
    )
}

fn revision_label(revision: df_types::SessionRevision) -> String {
    format!("{}:{}", revision.epoch().get(), revision.sequence())
}

fn joined_identity(joined: &rpc::JoinedRoom) -> Result<([u8; 32], df_types::MemberId), Error> {
    let credential = super::parse_binding(&joined.local_binding)
        .ok_or_else(|| io::Error::other("NPC-event local binding invalid"))?;
    let member_bytes: [u8; 16] = joined
        .client_binding_id
        .as_ref()
        .and_then(|binding| binding.value.as_deref())
        .ok_or_else(|| io::Error::other("NPC-event member binding absent"))?
        .try_into()
        .map_err(|_| io::Error::other("NPC-event member binding invalid"))?;
    let member = df_types::MemberId::from_bytes(&member_bytes)
        .map_err(|_| io::Error::other("NPC-event member identity invalid"))?;
    Ok((credential, member))
}

async fn start_npc_event_actor(
    config: &Config,
    checkpoint: Checkpoint,
    fence: [u8; 16],
    codec: NativeCodecLimits,
    counter: Arc<AtomicUsize>,
    social: Arc<super::social::SocialLedger>,
    voice: Arc<super::npc_reply::NpcVoice>,
) -> Result<RunningActor, Error> {
    let (grant_client, grant_connection) =
        timeout(Duration::from_secs(2), config.connect(NoTls)).await??;
    let grant_driver = tokio::spawn(grant_connection);
    let mut issuer = LocalDemoScopeIssuer::new(
        tokio::runtime::Handle::current(),
        grant_client,
        checkpoint.basis().session,
        fence,
    )
    .map_err(repository_error)?;
    issuer
        .configure_reconnect(config.clone(), grant_driver)
        .map_err(|(error, _)| repository_error(error))?;
    RunningActor::start_with_npc(
        repository(config, config, codec).await?,
        issuer,
        checkpoint,
        codec,
        counter,
        social,
        voice,
    )
    .await
}

fn latest_npc_speech(service: &Service, actor_id: &str) -> Result<rpc::TableLine, Error> {
    service
        .table_log
        .lines()
        .into_iter()
        .rev()
        .find(|line| {
            line.role == rpc::TableRole::Npc as i32
                && line.kind == rpc::TableKind::Speech as i32
                && line.actor_id == actor_id
        })
        .ok_or_else(|| io::Error::other("NPC-event public speech line absent").into())
}

async fn submit_npc_event_dialogue(
    service: &Service,
    checkpoint: &Checkpoint,
    credential: &[u8; 32],
    operation: u8,
    words: &str,
) -> Result<rpc::DialogueResponse, Error> {
    Ok(rpc::dialogue_service_server::DialogueService::submit(
        service,
        authenticated(
            raw(
                checkpoint,
                operation,
                rpc::DialogueContext::Question,
                rpc::DialogueFinality::Final,
                String::new(),
                words,
            ),
            credential,
        ),
    )
    .await?
    .into_inner())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Root-owned fresh npc-event-e01 PostgreSQL runtime"]
async fn owned_npc_event_e01_commits_the_spoken_loan_through_dialogue_and_replays_after_restart()
-> Result<(), Error> {
    use super::adventure::{self, RulesMode};
    use super::storyteller::tests::{FakeEndpoint, Reply};

    let config = npc_event_configuration()?;
    super::model::install_database_run("df_gameplay_demo_npc_event_e01")
        .map_err(repository_error)?;
    let generated = adventure::shop::generate(
        45,
        RulesMode::Standard,
        super::startup_configuration::ContentRating::default(),
    )
    .map_err(repository_error)?;
    let documents = generated.documents(false).map_err(repository_error)?;
    let (loaded, record) = adventure::record::reload(&documents).map_err(repository_error)?;
    super::pack::install(loaded).map_err(repository_error)?;
    let social = super::social::SocialLedger::open(record.dossiers.clone(), &record.seed, None)
        .map_err(repository_error)?;

    let first_ask =
        "Marek, may I hold the fine clothes for inspection? I will give them straight back.";
    let first_reply = "Here, have a look.";
    let second_ask = "Marek, what is the cloth made of?";
    let second_reply = "The cloth is tightly woven and sturdy.";
    let wrong_item_ask =
        "Marek, may I hold the fine clothes and the brass hand bell for inspection?";
    let wrong_item_reply = "Here, have a look at the fine clothes.";
    let contaminated_ask = "Marek, may I hold the brass hand bell for a moment? I will give it straight back without ringing it.";
    let contaminated_reply = "brass key is a brass key wrapped in blue cloth.";
    let storyteller_endpoint = FakeEndpoint::start(vec![
        Reply::Output(json!({"clean": first_ask})),
        Reply::Output(json!({"reply": first_reply, "approach": "question"})),
        Reply::Output(json!({"clean": second_ask})),
        Reply::Output(json!({"reply": second_reply, "approach": "question"})),
        Reply::Output(json!({"clean": wrong_item_ask})),
        Reply::Output(json!({"reply": wrong_item_reply, "approach": "question"})),
        Reply::Output(json!({"clean": contaminated_ask})),
        Reply::Output(json!({"reply": contaminated_reply, "approach": "question"})),
    ]);
    let classify_jev = Arc::new(move |body: &Value| {
        let state = &body["state"];
        if state["npc_reply"].as_str() == Some(second_reply) {
            return (503, json!({"error": "fixture-provider-failure"}));
        }
        let criteria = body["questions"]["q1"]["criteria"]
            .as_object()
            .expect("choice criteria object");
        let target = if state.get("npc_reply").is_some() {
            let typed = state["player_words"].as_str().unwrap_or_default();
            if typed.contains("brass hand bell") {
                let bell_id = state["canonical_items"].as_array().and_then(|items| {
                    items.iter().find_map(|item| {
                        (item["name"].as_str() == Some("brass hand bell"))
                            .then(|| item["id"].as_str())
                            .flatten()
                    })
                });
                bell_id.map(|id| format!("intent_to_give:{id}:"))
            } else {
                Some("intent_to_give:item-clothes-travelers:".to_owned())
            }
        } else if criteria.values().any(|value| {
            value
                .as_str()
                .is_some_and(|text| text.starts_with("character-speech:"))
        }) {
            Some("character-speech:".to_owned())
        } else if criteria.values().any(|value| {
            value
                .as_str()
                .is_some_and(|text| text.starts_with("question:"))
        }) {
            Some("question:".to_owned())
        } else if criteria.values().any(|value| {
            value
                .as_str()
                .is_some_and(|text| text.starts_with("other:"))
        }) {
            Some("other:".to_owned())
        } else if criteria.values().any(|value| {
            value
                .as_str()
                .is_some_and(|text| text.starts_with("unmoved:"))
        }) {
            Some("unmoved:".to_owned())
        } else {
            None
        };
        let choice = target
            .and_then(|prefix| {
                criteria.iter().find_map(|(option, criterion)| {
                    criterion
                        .as_str()
                        .is_some_and(|value| value.starts_with(prefix.as_str()))
                        .then(|| option.clone())
                })
            })
            .unwrap_or_else(|| "o1".to_owned());
        (
            200,
            json!({
                "answers": {"q1": {"type": "choice", "choice": choice, "confidence": 0.99}}
            }),
        )
    });
    let jev_endpoint = FakeEndpoint::start(
        (0..32)
            .map(|_| Reply::ComputeResponse(classify_jev.clone()))
            .collect(),
    );
    let cache = super::storyteller::tests::cache_directory("npc-event-e01");
    let storyteller = Arc::new(super::storyteller::tests::teller(
        storyteller_endpoint.address,
        20_000,
        Duration::from_secs(5),
        &cache,
    ));
    let voice = Arc::new(
        super::npc_reply::NpcVoice::new(storyteller)
            .with_approach_model(super::approach_model::tests::model(&jev_endpoint)),
    );

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
    let (mut inspector, inspector_connection) =
        timeout(Duration::from_secs(2), config.connect(NoTls)).await??;
    let inspector_driver = tokio::spawn(inspector_connection);
    let admitted = local_demo_scope::admit_startup_with_adventure(
        &mut inspector,
        &cold,
        local_demo_scope::DemoStartupIdentity {
            fence: first_fence,
            player_credential: [0x81; 32],
            display_credential: [0x82; 32],
        },
        codec,
        &model::recovery().map_err(repository_error)?,
        Some(&documents),
        |checkpoint| journey::phase(checkpoint).map(|_| ()),
    )
    .await
    .map_err(repository_error)?;
    required(!admitted.restored && admitted.checkpoint == cold)?;
    let counter = Arc::new(AtomicUsize::new(0));
    let mut running = Some(
        start_npc_event_actor(
            &config,
            admitted.checkpoint,
            first_fence,
            codec,
            counter.clone(),
            social,
            voice,
        )
        .await?,
    );
    let mut active_fence = first_fence;
    let mut accepted_proposal = None;
    let result = async {
        let service = &running
            .as_ref()
            .ok_or_else(|| io::Error::other("NPC-event owner absent"))?
            .service;
        let joined = direct_join(service, 0x91).await?;
        let (credential, member) = joined_identity(&joined)?;
        let created = direct_submit_action(
            service,
            create_named(&service.updates.borrow(), 0x92, "Sigrun"),
            &credential,
        )
        .await?;
        required(matches!(
            created.outcome,
            Some(rpc::submit_action_response::Outcome::CommittedDecision(_))
        ))?;
        let created_at = service.updates.borrow().clone();
        let begun = direct_submit_action(
            service,
            action(&created_at, 0x93, rpc::GameplayActionKind::BeginStory),
            &credential,
        )
        .await?;
        required(matches!(
            begun.outcome,
            Some(rpc::submit_action_response::Outcome::CommittedDecision(_))
        ))?;
        let opening = service.updates.borrow().clone();
        required(
            journey::phase(&opening).map_err(repository_error)? == rpc::JourneyPhase::Dialogue,
        )?;
        let scene = super::beat_script_runtime::scene(opening.state()).map_err(repository_error)?;
        let scene_identity = format!("{}|{}", scene.title, scene.asset);
        let before_snapshot = journey::living_scene::snapshot(opening.state(), &scene)
            .map_err(repository_error)?
            .ok_or_else(|| io::Error::other("shop scene snapshot absent"))?;
        let offer = before_snapshot
            .model
            .offers
            .iter()
            .find(|offer| offer.id == "clothes-travelers")
            .ok_or_else(|| io::Error::other("clothes stock offer absent"))?;
        required(offer.item_id == "item-clothes-travelers" && offer.seller_id == "merchant")?;
        let bell_id = before_snapshot
            .model
            .things
            .iter()
            .find(|thing| thing.name == "brass hand bell")
            .map(|thing| thing.id.clone())
            .ok_or_else(|| io::Error::other("brass hand bell absent from fresh shop"))?;
        let hero = journey::player_entity(member, &opening).map_err(repository_error)?;
        let balance_before =
            journey::living_scene::balance_cp(opening.state(), hero).map_err(repository_error)?;

        let response =
            submit_npc_event_dialogue(service, &opening, &credential, 0xa1, first_ask).await?;
        required(response.npc_reply == first_reply)?;
        required(response.npc_actor_id == "merchant" && response.npc_speaker == "Marek")?;
        let source_line = latest_npc_speech(service, "merchant")?;
        required(source_line.speaker == "Marek" && source_line.text == first_reply)?;
        let committed = service.updates.borrow().clone();
        let next_revision = opening
            .basis()
            .revision
            .next_sequence()
            .map_err(repository_error)?;
        required(committed.basis().revision == next_revision)?;
        required(response.revision == Some(wire::revision(next_revision)))?;
        required(committed.state().decisions.len() == opening.state().decisions.len() + 1)?;
        let after_scene =
            super::beat_script_runtime::scene(committed.state()).map_err(repository_error)?;
        let after_snapshot = journey::living_scene::snapshot(committed.state(), &after_scene)
            .map_err(repository_error)?
            .ok_or_else(|| io::Error::other("committed shop snapshot absent"))?;
        let held = after_snapshot
            .model
            .things
            .iter()
            .find(|thing| thing.id == "item-clothes-travelers")
            .ok_or_else(|| io::Error::other("loan item was not materialized"))?;
        required(held.owner_id.as_deref() == Some("merchant"))?;
        required(held.holder_id.as_deref() == Some("Sigrun"))?;
        required(
            held.provenance.kind
                == super::adventure::scene_model::SceneItemProvenanceKind::GeneratedOffer,
        )?;
        required(held.provenance.source_id == "clothes-travelers")?;
        required(held.provenance.seller_id.as_deref() == Some("merchant"))?;
        required(
            !after_snapshot
                .model
                .offer_available("item-clothes-travelers"),
        )?;
        required(
            journey::living_scene::balance_cp(committed.state(), hero).map_err(repository_error)?
                == balance_before,
        )?;

        let interaction = committed
            .state()
            .decisions
            .last()
            .and_then(|decision| decision.semantic_output.as_deref())
            .and_then(|output| output.strip_prefix("living-scene-1:"))
            .ok_or_else(|| io::Error::other("NPC inventory semantic receipt absent"))?;
        let receipt: Value = serde_json::from_str(interaction)?;
        required(receipt["option"] == "npc-inventory")?;
        required(receipt["target"] == "item-clothes-travelers")?;
        required(
            receipt["source_line_id"].as_str()
                == Some(
                    source_line_id_for(
                        "merchant",
                        "patron:merchant",
                        &scene_identity,
                        &revision_label(opening.basis().revision),
                        first_ask,
                        first_reply,
                    )
                    .as_str(),
                ),
        )?;
        required(receipt["source_revision"] == revision_label(opening.basis().revision))?;
        required(receipt["source_actor_id"] == "merchant")?;
        required(receipt["source_text"] == first_reply)?;
        required(receipt["source_line_sequence"].as_u64() == Some(source_line.sequence))?;

        let projection = wire::project_journey(&committed, LocalDemoRole::Player, member)
            .map_err(repository_error)?;
        let Some(rpc::view_message::Audience::Player(phone)) = projection.view.audience else {
            return Err(io::Error::other("player phone projection absent").into());
        };
        let phone_scene = phone
            .scene
            .ok_or_else(|| io::Error::other("player phone scene absent"))?;
        let phone_card = phone_scene
            .hand_items
            .iter()
            .find(|card| card.thing_id == "item-clothes-travelers")
            .ok_or_else(|| io::Error::other("loan missing from phone hand inventory"))?;
        required(phone_card.observation_only && phone_card.holder == "Sigrun")?;
        required(phone_card.detail == held.detail)?;
        let first_proposal = event_line_proposal(
            "merchant",
            "patron:merchant",
            "item-clothes-travelers",
            "Sigrun",
            &scene_identity,
            &revision_label(opening.basis().revision),
            first_ask,
            first_reply,
            source_line.sequence,
        );
        accepted_proposal = Some(first_proposal.clone());

        let second_response =
            submit_npc_event_dialogue(service, &committed, &credential, 0xa2, second_ask).await?;
        required(second_response.npc_reply == second_reply)?;
        let second_line = latest_npc_speech(service, "merchant")?;
        required(second_line.text == second_reply)?;
        let after_provider_failure = service.updates.borrow().clone();
        required(after_provider_failure == committed)?;

        use super::npc_event_classifier::EventKind;
        let mut refusal = event_line_proposal(
            "merchant",
            "patron:merchant",
            "item-clothes-travelers",
            "Sigrun",
            &scene_identity,
            &revision_label(committed.basis().revision),
            second_ask,
            second_reply,
            second_line.sequence,
        );
        refusal.kind = EventKind::Refusal;
        refusal.item_id = None;
        refusal.recipient_id = None;
        let refusal_result = service
            .commit_npc_event(credential, refusal.clone())
            .await?;
        required(refusal_result.status == "refusal_no_commit")?;
        required(service.updates.borrow().clone() == committed)?;
        let mut conditional = refusal.clone();
        conditional.kind = EventKind::Conditional;
        let conditional_result = service.commit_npc_event(credential, conditional).await?;
        required(conditional_result.status == "conditional_no_commit")?;
        required(service.updates.borrow().clone() == committed)?;
        let wrong_speaker = event_line_proposal(
            "shop-adventurer",
            "bystander:shop-adventurer",
            "item-clothes-travelers",
            "Sigrun",
            &scene_identity,
            &revision_label(committed.basis().revision),
            second_ask,
            second_reply,
            second_line.sequence,
        );
        required(
            service
                .commit_npc_event(credential, wrong_speaker)
                .await?
                .status
                == "ignored_line_not_current",
        )?;
        required(service.updates.borrow().clone() == committed)?;
        let wrong_dossier = event_line_proposal(
            "merchant",
            "bystander:merchant",
            "item-clothes-travelers",
            "Sigrun",
            &scene_identity,
            &revision_label(committed.basis().revision),
            second_ask,
            second_reply,
            second_line.sequence,
        );
        required(
            service
                .commit_npc_event(credential, wrong_dossier)
                .await?
                .status
                == "ignored_wrong_speaker",
        )?;
        required(service.updates.borrow().clone() == committed)?;
        let stale = event_line_proposal(
            "merchant",
            "patron:merchant",
            "item-clothes-travelers",
            "Sigrun",
            &scene_identity,
            &revision_label(opening.basis().revision),
            second_ask,
            second_reply,
            second_line.sequence,
        );
        required(service.commit_npc_event(credential, stale).await?.status == "stale")?;
        required(service.updates.borrow().clone() == committed)?;

        let wrong_reply_response =
            submit_npc_event_dialogue(service, &committed, &credential, 0xa3, wrong_item_ask)
                .await?;
        required(wrong_reply_response.npc_reply == wrong_item_reply)?;
        let wrong_reply_line = latest_npc_speech(service, "merchant")?;
        required(wrong_reply_line.text == wrong_item_reply)?;
        required(service.updates.borrow().clone() == committed)?;
        let wrong_item_proposal = event_line_proposal(
            "merchant",
            "patron:merchant",
            &bell_id,
            "Sigrun",
            &scene_identity,
            &revision_label(committed.basis().revision),
            wrong_item_ask,
            wrong_item_reply,
            wrong_reply_line.sequence,
        );
        required(
            service
                .commit_npc_event(credential, wrong_item_proposal)
                .await?
                .status
                == "rejected",
        )?;
        required(service.updates.borrow().clone() == committed)?;

        let contaminated_response =
            submit_npc_event_dialogue(service, &committed, &credential, 0xa4, contaminated_ask)
                .await?;
        required(contaminated_response.npc_reply == contaminated_reply)?;
        let contaminated_line = latest_npc_speech(service, "merchant")?;
        required(contaminated_line.text == contaminated_reply)?;
        required(service.updates.borrow().clone() == committed)?;

        let mut exited = running
            .take()
            .ok_or_else(|| io::Error::other("NPC-event owner absent during restart"))?
            .close()
            .await?;
        required(exited.checkpoint == committed)?;
        exited
            .issuer
            .close_owned()
            .await
            .map_err(repository_error)?;
        local_demo_scope::release_owner(&inspector, session, first_fence)
            .await
            .map_err(repository_error)?;

        active_fence = second_fence;
        let restored_admitted = local_demo_scope::admit_startup_with_adventure(
            &mut inspector,
            &cold,
            local_demo_scope::DemoStartupIdentity {
                fence: second_fence,
                player_credential: [0x85; 32],
                display_credential: [0x86; 32],
            },
            codec,
            &model::recovery().map_err(repository_error)?,
            Some(&documents),
            |checkpoint| journey::phase(checkpoint).map(|_| ()),
        )
        .await
        .map_err(repository_error)?;
        required(restored_admitted.restored && restored_admitted.checkpoint == committed)?;
        let restored_semantic = restored_admitted
            .checkpoint
            .state()
            .decisions
            .last()
            .and_then(|decision| decision.semantic_output.as_deref())
            .and_then(|output| output.strip_prefix("living-scene-1:"))
            .ok_or_else(|| io::Error::other("durable NPC source receipt absent after restore"))?;
        let restored_receipt: Value = serde_json::from_str(restored_semantic)?;
        required(restored_receipt["source_line_id"] == receipt["source_line_id"])?;
        required(restored_receipt["source_line_sequence"] == receipt["source_line_sequence"])?;
        required(restored_receipt["source_text"] == receipt["source_text"])?;
        running = Some(
            start_npc_event_actor(
                &config,
                restored_admitted.checkpoint,
                second_fence,
                codec,
                Arc::new(AtomicUsize::new(0)),
                super::social::SocialLedger::open(record.dossiers.clone(), &record.seed, None)
                    .map_err(repository_error)?,
                Arc::new(super::npc_reply::NpcVoice::silent()),
            )
            .await?,
        );
        let restarted_service = &running
            .as_ref()
            .ok_or_else(|| io::Error::other("restarted NPC-event owner absent"))?
            .service;
        let replay_joined = direct_join(restarted_service, 0x91).await?;
        let (replay_credential, replay_member) = joined_identity(&replay_joined)?;
        required(replay_credential == credential && replay_member == member)?;
        let duplicate = restarted_service
            .commit_npc_event(
                replay_credential,
                accepted_proposal
                    .clone()
                    .ok_or_else(|| io::Error::other("accepted proposal absent"))?,
            )
            .await?;
        required(duplicate.status == "duplicate")?;
        required(restarted_service.updates.borrow().clone() == committed)?;
        Ok::<(), Error>(())
    }
    .await;

    if let Some(mut running) = running.take()
        && let Ok(mut exited) = running.close().await
    {
        let _ = exited.issuer.close_owned().await;
    }
    let release = local_demo_scope::release_owner(&inspector, session, active_fence).await;
    drop(inspector);
    let inspector_joined = timeout(Duration::from_secs(2), inspector_driver).await;
    result?;
    release.map_err(repository_error)?;
    inspector_joined???;
    Ok(())
}

fn source_line_id_for(
    actor_id: &str,
    npc_id: &str,
    scene_identity: &str,
    revision: &str,
    typed_words: &str,
    reply: &str,
) -> String {
    super::npc_event_classifier::Proposal::source_line_id(
        actor_id,
        npc_id,
        scene_identity,
        revision,
        typed_words,
        reply,
    )
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
