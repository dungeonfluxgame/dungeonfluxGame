//! In-process gameplay server with the real PostgreSQL owner and disabled provider adapters.
use super::*;
use df_session::submission::SessionEngine;
use tokio::task::JoinHandle;

pub(super) struct Harness {
    pub(super) state: LaunchState,
    pub(super) incoming: Option<df_rpc_bridge::NativeIncoming>,
    pub(super) service: super::super::Service,
    owner: Option<std::thread::JoinHandle<()>>,
    admin: Option<tokio_postgres::Client>,
    admin_driver: Option<JoinHandle<Result<(), tokio_postgres::Error>>>,
    database: String,
    pub(super) tasks: tokio::task::JoinSet<()>,
}

impl Harness {
    pub(super) async fn new() -> Self {
        let database = format!(
            "df_gameplay_demo_gamelaunch_{}",
            super::super::hex(&super::super::model::random::<8>().unwrap())
        );
        let mut config = tokio_postgres::Config::new();
        config
            .host("127.0.0.1")
            .port(55911)
            .user("postgres")
            .dbname("postgres")
            .connect_timeout(std::time::Duration::from_secs(3));
        let (admin, connection) = config.connect(tokio_postgres::NoTls).await.unwrap();
        let admin_driver = tokio::spawn(connection);
        admin
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .unwrap();
        config.dbname(&database);
        let (actor, updates) = owner(&config).await;
        let (sender, inbox) = df_session::inbox::bounded_inbox();
        let owner = std::thread::Builder::new()
            .name("gamelaunch-test-owner".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(move || {
                let mut actor = actor;
                inbox.run(&mut actor).unwrap();
                actor.issuer.release_owner().unwrap();
                actor.owner.into_repository().close().unwrap();
                actor.issuer.close().unwrap();
            })
            .unwrap();
        let service = service(sender.clone(), updates);
        let (admission, incoming) =
            df_rpc_bridge::NativeIncoming::bounded(std::num::NonZeroUsize::new(4).unwrap())
                .unwrap();
        let page = PageState {
            run_identity: Arc::from("gamelaunch-test-run"),
            incoming: admission,
            connections: Arc::new(tokio::sync::Semaphore::new(8)),
            allowed_origins: vec!["http://127.0.0.1".to_owned()].into(),
            display: service.display_credential,
            glue: bytes::Bytes::new(),
            wasm: bytes::Bytes::new(),
            battle_scene: None,
            battle_fps: 30,
            splat_lod: 1,
            battle_grid: false,
        };
        let state = LaunchState {
            actor: sender,
            page,
            secret: LaunchSecret::new(&"g".repeat(32)).unwrap(),
            generation: Arc::from("gamelaunch-test-generation"),
            code: super::super::journey::room_code().into(),
            slot: Arc::new(Mutex::new(None)),
        };
        Self {
            state,
            incoming: Some(incoming),
            service,
            owner: Some(owner),
            admin: Some(admin),
            admin_driver: Some(admin_driver),
            database,
            tasks: tokio::task::JoinSet::new(),
        }
    }

    pub(super) async fn listen(&mut self, app: Router) -> SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        self.tasks.spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        address
    }

    pub(super) fn start_rpc(&mut self) {
        let incoming = self.incoming.take().unwrap();
        let service = self.service.clone();
        self.tasks.spawn(async move {
            tonic::transport::Server::builder()
                .add_service(rpc::session_service_server::SessionServiceServer::new(
                    service,
                ))
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });
    }

    pub(super) async fn shutdown(mut self) {
        self.tasks.shutdown().await;
        self.state.actor.stop().unwrap();
        let owner = self.owner.take().unwrap();
        tokio::task::spawn_blocking(move || owner.join().unwrap())
            .await
            .unwrap();
        let admin = self.admin.take().unwrap();
        admin
            .batch_execute(&format!("DROP DATABASE {}", self.database))
            .await
            .unwrap();
        drop(admin);
        self.admin_driver.take().unwrap().await.unwrap().unwrap();
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.tasks.abort_all();
        // A failing assertion still releases the owned actor; normal shutdown also joins it.
        let _already_stopped = self.state.actor.stop();
        if let Some(driver) = self.admin_driver.take() {
            driver.abort();
        }
    }
}

async fn owner(
    config: &tokio_postgres::Config,
) -> (
    super::super::actor::Actor,
    tokio::sync::watch::Receiver<df_model::checkpoint::Checkpoint>,
) {
    use super::super::*;
    let runtime = tokio::runtime::Handle::current();
    let codec = live_codec_limits();
    let recovery = model::recovery().unwrap();
    let cold = journey::initial().unwrap();
    let (mut grants, grant_connection) = config.connect(tokio_postgres::NoTls).await.unwrap();
    let grant_driver = tokio::spawn(grant_connection);
    let fence = model::random().unwrap();
    let admitted = df_persistence::local_demo_scope::admit_startup(
        &mut grants,
        &cold,
        df_persistence::local_demo_scope::DemoStartupIdentity {
            fence,
            player_credential: [0x51; 32],
            display_credential: [0x52; 32],
        },
        codec,
        &recovery,
        |checkpoint| {
            SessionEngine::<df_persistence::NativeScope<LocalDemoAuthority>>::validate_recovery(
                &mut model::HarborEngine,
                checkpoint,
            )
        },
    )
    .await
    .unwrap();
    let initial = admitted.checkpoint;
    let mut issuer =
        LocalDemoScopeIssuer::new(runtime.clone(), grants, initial.basis().session, fence).unwrap();
    assert!(
        issuer
            .configure_reconnect(config.clone(), grant_driver)
            .is_ok()
    );
    let (client, connection) = config.connect(tokio_postgres::NoTls).await.unwrap();
    let repository = match PostgresRepository::<LocalDemoAuthority>::from_connected_no_tls(
        runtime,
        client,
        connection,
        NativeRepositoryOptions {
            transaction_bounds: timing::active().transaction_bounds(),
            codec_limits: codec,
            maximum_receipt_bytes: df_persistence::local_demo_scope::RECEIPT_RETAINED_BYTES_MAX,
            verifier: Some(df_persistence::local_demo_scope::verifier()),
            recovery: Some(recovery),
        },
        &OperationContext {
            trace_parent: String::new(),
            build: crate::BUILD_ID.to_owned(),
        },
    )
    .await
    {
        Ok(repository) => repository,
        Err(_) => panic!("repository setup failed"),
    };
    let (updates, receiver) = tokio::sync::watch::channel(initial.clone());
    let (publication, intent_notifications) = actor::Publication::new(updates.clone());
    let owner = DurableOwner::new(
        repository,
        actor::Engine(None),
        publication,
        initial,
        df_persistence::local_demo_scope::RECEIPT_RETAINED_BYTES_MAX,
    )
    .unwrap();
    (
        actor::Actor {
            session: Default::default(),
            owner,
            table_log: table_log::TableLog::disabled(),
            notebook: notebook::NotebookLedger::none(),
            social: social::SocialLedger::none(),
            table_settings: table_settings::TableSettings::startup(None, false),
            dialogue: Default::default(),
            tempo: None,
            bootstrap_credential: admitted.player_credential,
            issuer,
            codec,
            fenced: false,
            recovery_wakeup: updates,
            campaign_observer: None,
            intent_notifications,
            calls_remaining: u16::MAX,
            calls_unbounded: true,
            queued_wait: None,
            qualification_inputs: None,
            qualification_joins: None,
            completion_retry: None,
            dev_history: None,
            completion_observer: None,
        },
        receiver,
    )
}

fn service(
    actor: super::super::InboxHandle<super::super::actor::Call>,
    updates: tokio::sync::watch::Receiver<df_model::checkpoint::Checkpoint>,
) -> super::super::Service {
    use super::super::*;
    Service {
        actor,
        display_credential: [0x52; 32],
        updates,
        streams: Arc::new(tokio::sync::Semaphore::new(8)),
        voice: voice::Voice::disabled(startup_configuration::VoiceOff::Disabled),
        npc_voice: Arc::new(npc_reply::NpcVoice::silent()),
        table_log: table_log::TableLog::disabled(),
        social: social::SocialLedger::none(),
        living: living_scenes::LivingScenes::disabled(),
        look_ahead: Arc::default(),
        layout: dev_control::layout::LayoutState::new(),
        portraits: creator_portraits::Portraits::disabled(),
    }
}
