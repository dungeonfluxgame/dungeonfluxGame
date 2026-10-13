//! Fully retire a run before composing its successor, using the same startup/provider path.
use super::*;
use sha2::{Digest, Sha256};

static IDENTITY: std::sync::RwLock<String> = std::sync::RwLock::new(String::new());

pub(super) fn install_identity(identity: &str) -> Result<(), io::Error> {
    *IDENTITY
        .write()
        .map_err(|_| io::Error::other("run identity lock poisoned"))? = identity.to_owned();
    Ok(())
}

pub(super) fn identity() -> Result<String, io::Error> {
    IDENTITY
        .read()
        .map(|identity| identity.clone())
        .map_err(|_| io::Error::other("run identity lock poisoned"))
}

pub(super) async fn serve(
    #[cfg(windows)] console_stop: watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut database = current_database(database_config()?).await?;
    let original = database_config()?.get_dbname().map(str::to_owned);
    let mut first = true;
    loop {
        let configuration = database.clone();
        let fresh_seed = configuration.get_dbname() != original.as_deref();
        #[cfg(windows)]
        let stop = console_stop.clone();
        let requested = tokio::task::spawn_blocking(move || {
            // Dropping this owned runtime joins/cancels every old socket, timer and provider
            // future, including tasks spawned by HTTP connection handlers. No old task can
            // observe the pack installed for the next run.
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            let result = runtime
                .block_on(super::serve_run(
                    #[cfg(windows)]
                    stop,
                    configuration,
                    first,
                    fresh_seed,
                ))
                .map_err(|error| io::Error::other(error.to_string()));
            drop(runtime);
            result
        })
        .await??;
        super::home_video::retire()?;
        if !requested {
            return Ok(());
        }
        actor::record_audit_line(
            "gameplay.run_stopped",
            "RUN stopped; owner and runtime released".to_owned(),
        );
        let next = successor_name(database.get_dbname().ok_or("run database missing")?);
        let settings = startup_configuration::session_settings(
            startup_configuration::SessionInputs::from_environment(),
        )?;
        let preparation = preparing(settings.listen, settings.allowed_origins.clone()).await?;
        let advanced = advance_database(database, &next).await;
        preparation.abort();
        if let Err(error) = preparation.await
            && !error.is_cancelled()
        {
            return Err(error.into());
        }
        database = advanced?;
        first = false;
        actor::record_audit_line(
            "gameplay.run_starting",
            format!("RUN starting database={next}"),
        );
    }
}

fn successor_name(old: &str) -> String {
    let digest = Sha256::digest(old.as_bytes());
    format!("df_gameplay_demo_next_{}", super::hex(&digest[..16]))
}

async fn connected(
    configuration: &tokio_postgres::Config,
) -> Result<
    (
        tokio_postgres::Client,
        tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
    ),
    io::Error,
> {
    let (client, connection) = tokio::time::timeout(
        Duration::from_secs(20),
        configuration.connect(tokio_postgres::NoTls),
    )
    .await
    .map_err(|_| io::Error::other("new adventure database connection deadline"))?
    .map_err(|_| io::Error::other("new adventure database connection failed"))?;
    Ok((client, tokio::spawn(connection)))
}

async fn allocated(configuration: &tokio_postgres::Config, next: &str) -> Result<(), io::Error> {
    let mut admin_configuration = configuration.clone();
    admin_configuration.dbname("postgres");
    let (admin, driver) = connected(&admin_configuration).await?;
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        df_persistence::local_demo_scope::ensure_run_database(&admin, next),
    )
    .await;
    drop(admin);
    super::join_grant_driver(driver).await?;
    result
        .map_err(|_| io::Error::other("new adventure allocation deadline"))?
        .map_err(|_| io::Error::other("new adventure allocation failed"))
}

async fn advance_database(
    mut configuration: tokio_postgres::Config,
    next: &str,
) -> Result<tokio_postgres::Config, io::Error> {
    // Retain the destination first. A crash before CREATE DATABASE is repaired by
    // current_database, and a crash after it cannot allocate a second destination.
    let (client, driver) = connected(&configuration).await?;
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        df_persistence::local_demo_scope::retain_run_successor(&client, next),
    )
    .await;
    drop(client);
    super::join_grant_driver(driver).await?;
    result
        .map_err(|_| io::Error::other("new adventure successor deadline"))?
        .map_err(|_| io::Error::other("new adventure successor refused"))?;
    allocated(&configuration, next).await?;
    configuration.dbname(next);
    Ok(configuration)
}

async fn current_database(
    mut configuration: tokio_postgres::Config,
) -> Result<tokio_postgres::Config, io::Error> {
    for _ in 0..256 {
        let (client, driver) = connected(&configuration).await?;
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            df_persistence::local_demo_scope::run_successor(&client),
        )
        .await;
        drop(client);
        super::join_grant_driver(driver).await?;
        let next = result
            .map_err(|_| io::Error::other("current adventure lookup deadline"))?
            .map_err(|_| io::Error::other("current adventure lookup failed"))?;
        let Some(next) = next else {
            return Ok(configuration);
        };
        allocated(&configuration, &next).await?;
        configuration.dbname(&next);
    }
    Err(io::Error::other(
        "current adventure successor limit exceeded",
    ))
}

/// Preparation has the same public recovery RPC, so even a saved phone reconnecting
/// after the old runtime was retired learns that this is a new-adventure transition.
#[derive(Clone)]
struct Preparation;
#[tonic::async_trait]
impl rpc::room_service_server::RoomService for Preparation {
    async fn current_adventure(
        &self,
        _: tonic::Request<rpc::CurrentAdventureRequest>,
    ) -> Result<tonic::Response<rpc::CurrentAdventureResponse>, tonic::Status> {
        Ok(tonic::Response::new(rpc::CurrentAdventureResponse {
            room_code: journey::room_code(),
            run_identity: identity()
                .map_err(|_| tonic::Status::internal("run identity unavailable"))?,
            preparing: true,
        }))
    }
    async fn join(
        &self,
        _: tonic::Request<rpc::JoinRoomRequest>,
    ) -> Result<tonic::Response<rpc::JoinRoomResponse>, tonic::Status> {
        Err(tonic::Status::unavailable(
            "A new adventure is being prepared...",
        ))
    }
    async fn reclaim(
        &self,
        _: tonic::Request<rpc::ReclaimRoomRequest>,
    ) -> Result<tonic::Response<rpc::ReclaimRoomResponse>, tonic::Status> {
        Err(tonic::Status::unavailable(
            "A new adventure is being prepared...",
        ))
    }
}

pub(super) async fn preparing(
    address: std::net::SocketAddr,
    origins: Vec<String>,
) -> Result<tokio::task::JoinHandle<Result<(), io::Error>>, io::Error> {
    async fn page() -> Html<&'static str> {
        Html(
            "<!doctype html><html><head><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"refresh\" content=\"1\"><title>DungeonFlux</title></head><body style=\"background:#080f17;color:#efe6ce;font:24px Georgia;text-align:center;padding-top:30vh\"><p role=\"status\">A new adventure is being prepared...</p></body></html>",
        )
    }
    async fn tunnel(
        State((incoming, origins)): State<(NativeAdmission, Arc<[String]>)>,
        headers: HeaderMap,
        upgrade: WebSocketUpgrade,
    ) -> axum::response::Response {
        if !headers
            .get("origin")
            .and_then(|origin| origin.to_str().ok())
            .is_some_and(|origin| origins.iter().any(|allowed| allowed == origin))
        {
            return axum::http::StatusCode::FORBIDDEN.into_response();
        }
        let Ok(admission) = incoming.try_reserve() else {
            return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
        };
        upgrade
            .max_message_size(MESSAGE_BYTES)
            .max_frame_size(MESSAGE_BYTES)
            .on_upgrade(move |socket| async move {
                let result = admission
                    .accept_websocket_measured(
                        socket,
                        Arc::new(df_rpc_bridge::ConnectionMetrics::new(false)),
                    )
                    .await;
                let context = OperationContext {
                    trace_parent: String::new(),
                    build: crate::BUILD_ID.to_owned(),
                };
                let mut span = df_observe::begin(&context, "gameplay.preparation_connection");
                span.finish_unmeasured(if result.is_ok() {
                    "connection_closed"
                } else {
                    "connection_failed"
                });
            })
            .into_response()
    }
    let listener = tokio::net::TcpListener::bind(address).await?;
    let (admission, incoming) = NativeIncoming::bounded(
        NonZeroUsize::new(4).ok_or_else(|| io::Error::other("preparation incoming bound"))?,
    )
    .map_err(io::Error::other)?;
    let grpc = tonic::transport::Server::builder()
        .max_concurrent_streams(CONCURRENT_STREAMS)
        .add_service(
            rpc::room_service_server::RoomServiceServer::new(Preparation)
                .max_decoding_message_size(8192)
                .max_encoding_message_size(8192),
        )
        .serve_with_incoming(incoming);
    let router = Router::new()
        .route("/gameplay", get(page))
        .route("/gameplay/player", get(page))
        .route("/gameplay/display", get(page))
        .route("/gameplay/tunnel", get(tunnel))
        .with_state((admission, Arc::<[String]>::from(origins)));
    Ok(tokio::spawn(async move {
        tokio::select! {
            result = axum::serve(listener, router).into_future() => result,
            result = grpc => result.map_err(io::Error::other),
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_requests_or_a_crash_retry_select_exactly_one_successor() {
        let old = "df_gameplay_demo_s03_20261013";
        let next = successor_name(old);
        assert_eq!(next, successor_name(old));
        assert_ne!(next, successor_name(&next));
        assert!(next.len() <= 63);
    }
}
