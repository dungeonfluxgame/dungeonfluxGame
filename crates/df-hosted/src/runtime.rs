//! Owns account background tasks, PG connectors and HTTP serving for the hosted portal.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    Router,
    extract::{Form, Path, State},
    http::{HeaderMap, StatusCode},
    response::{Html, Redirect},
    routing::get,
};
use df_customer::{
    BuiltInCommonPasswords, CustomerConfig, CustomerService, DevOutbox, HashingParameters,
    IdentitySource, ServiceTasks, SystemClock, Tunables,
};
use df_persistence::{account_closure::TimeoutPolicy, account_store::PostgresAccountStore};
use df_portal::{
    PortalOrigin,
    app_api::guard::{GuardState, SignedIn},
};

use crate::{
    config::{Config, Identity, Mail, Profile},
    database::{Connection, Connector},
    mail::Sender,
    ports::HostedPorts,
};

/// Startup failure classes are safe to print; provider and database credentials are never retained.
#[derive(Debug)]
pub enum OpenError {
    Database,
    Migration(i32),
    AccountStore,
    CampaignStore,
    Identity,
    Mail,
    Customer,
    Origin,
    Random,
    Bundle,
    Port,
}

#[derive(Clone)]
struct Development {
    service: CustomerService<PostgresAccountStore, Sender>,
    ports: Arc<HostedPorts>,
    origin: Arc<str>,
}

impl GuardState for Development {
    fn origin(&self) -> &str {
        &self.origin
    }
    async fn authenticate(
        &self,
        token: &str,
    ) -> Result<Option<df_customer::Authenticated>, df_customer::ServiceError> {
        self.service.authenticate(token).await
    }
}

/// An opened composition. Dropping it cancels account workers and drops PG pools/drivers.
pub struct Hosted {
    app: Router,
    listen: SocketAddr,
    tasks: ServiceTasks,
    support_worker: Option<(crate::support_ingest::WorkerHandle, crate::database::Driver)>,
    telemetry: Arc<crate::telemetry::HostedTelemetry>,
}

impl Hosted {
    pub async fn open(config: Config) -> Result<Self, OpenError> {
        if !config.bundle.join("index.html").is_file() {
            return Err(OpenError::Bundle);
        }
        let telemetry = crate::telemetry::HostedTelemetry::open(config.telemetry_dir.as_deref());
        let (accounts, campaign_store) = open_stores(&config).await?;
        telemetry.record_db_outcome(true);
        let payment_mode = match (&config.stripe, config.profile) {
            (Some(_), _) => "stripe",
            (None, Profile::Development) => "development",
            (None, _) => "unconfigured",
        };
        let mut finance_connection = Connection::open(&config.database, &config.transport)
            .await
            .map_err(|_| OpenError::Database)?;
        crate::finance::publish_customer_prices(&mut finance_connection.client)
            .await
            .map_err(|_| {
                df_observe::record_audit_line(
                    "hosted.customer_prices_failed",
                    "SPEND customer_prices_failed".to_owned(),
                );
                OpenError::Port
            })?;
        let finance = crate::finance::Finance::new(
            finance_connection,
            accounts.clone(),
            config.trial_email_key,
        )
        .map_err(|_| OpenError::Port)?;
        let checkout = crate::checkout::Checkout::configured(
            config.profile,
            Connection::open(&config.database, &config.transport)
                .await
                .map_err(|_| OpenError::Database)?,
            finance.clone(),
            config.stripe,
            &config.origin,
        )
        .await
        .map_err(|_| OpenError::Port)?;
        let campaigns = crate::campaigns::Campaigns::new(
            campaign_store,
            crate::campaign_accounts::Accounts::new(
                accounts.clone(),
                Connection::open(&config.database, &config.transport)
                    .await
                    .map_err(|_| OpenError::Database)?,
            ),
            config.campaign_keys,
            crate::campaign_sessions::Sessions::new(
                Connection::open(&config.database, &config.transport)
                    .await
                    .map_err(|_| OpenError::Database)?,
            ),
            crate::campaign_funding::Funding::new(
                Connection::open(&config.database, &config.transport)
                    .await
                    .map_err(|_| OpenError::Database)?,
            ),
        );
        let (service, tasks) =
            start_accounts(config.hosts, config.identity, config.mail, accounts).await?;
        let ports = Arc::new(HostedPorts {
            finance,
            campaigns,
            checkout,
            unavailable: crate::ports::Refusing,
        });
        let payments = crate::checkout::router(ports.checkout.clone());
        let mut app = df_portal::app_api::compose(
            service.clone(),
            PortalOrigin::parse(&config.origin).map_err(|_| OpenError::Origin)?,
            Arc::clone(&ports),
            config.bundle,
        )
        .merge(payments);
        if config.profile == Profile::Development {
            app = app.merge(
                Router::new()
                    .route(
                        "/app/dev/checkouts/{checkout}",
                        get(checkout_page).post(pay),
                    )
                    .with_state(Development {
                        service,
                        ports,
                        origin: Arc::from(config.origin),
                    }),
            );
        }
        app = app
            .route("/app/healthz", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(private_headers))
            .layer(axum::Extension(df_portal::ClientAddressPolicy {
                proxies: config.proxies,
            }));
        app = crate::accounts_gate::apply(app, config.accounts_open);
        app = app.layer(axum::middleware::from_fn_with_state(
            Arc::clone(&telemetry),
            crate::telemetry::observe,
        ));
        df_observe::record_audit_line(
            "hosted.composed",
            format!(
                "HOSTED composed accounts=postgres finance=postgres campaigns=postgres payments={payment_mode}"
            ),
        );
        // Support ingest worker: only when the mailbox env names are configured.
        // The handle and its connection driver are owned here and abort on drop.
        let support_worker = match Connection::open(&config.database, &config.transport).await {
            Ok(connection) => {
                let crate::database::Connection { client, driver } = connection;
                let store =
                    std::sync::Arc::new(df_persistence::support_store::SupportStore::new(client));
                crate::support_ingest::from_env(store).map(|worker| {
                    let interval_s: u64 = std::env::var("DF_SUPPORT_INGEST_INTERVAL_S")
                        .ok()
                        .and_then(|value| value.parse().ok())
                        .filter(|seconds: &u64| *seconds >= 30)
                        .unwrap_or(300);
                    (
                        crate::support_ingest::spawn(
                            worker,
                            std::time::Duration::from_secs(interval_s),
                        ),
                        driver,
                    )
                })
            }
            Err(_) => None,
        };
        if support_worker.is_none() {
            df_observe::record_audit_line(
                "hosted.support_ingest_disabled",
                "HOSTED support_ingest disabled (mailbox not configured)".to_owned(),
            );
        }
        Ok(Self {
            app,
            listen: config.listen,
            tasks,
            support_worker,
            telemetry,
        })
    }

    pub fn router(&self) -> Router {
        self.app.clone()
    }

    pub async fn shutdown(self) {
        if let Some((worker, _driver)) = self.support_worker {
            worker.shutdown().await;
        }
        self.telemetry.shutdown();
        self.tasks.shutdown().await;
    }

    pub async fn serve(self) -> std::io::Result<()> {
        let listener = tokio::net::TcpListener::bind(self.listen).await?;
        df_observe::record_audit_line(
            "hosted.listening",
            format!("HOSTED listening address={}", self.listen),
        );
        #[cfg(unix)]
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let result = axum::serve(
            listener,
            self.app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            #[cfg(unix)]
            tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
            #[cfg(not(unix))]
            {
                let _signal = tokio::signal::ctrl_c().await;
            }
        })
        .await;
        self.tasks.shutdown().await;
        result
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PaymentForm {
    csrf: String,
}

async fn checkout_page(
    State(state): State<Development>,
    SignedIn(caller): SignedIn,
    Path(checkout): Path<String>,
) -> Result<Html<String>, StatusCode> {
    use df_portal::app_api::ports::CheckoutPort;
    let id = df_types::CheckoutId::from_hex(&checkout).map_err(|_| StatusCode::NOT_FOUND)?;
    let view = state
        .ports
        .checkout
        .status(caller.account.id, id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let csrf = caller.csrf.form_token(df_account_app::wire::CSRF_FORM);
    Ok(Html(format!(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>Development checkout</title><h1>Test payment</h1><p>No real payment will be taken. Add {} cents of test credit.</p><form method=\"post\"><input type=\"hidden\" name=\"csrf\" value=\"{}\"><button type=\"submit\">Pay test credit</button></form></html>",
        view.amount_cents, csrf
    )))
}

async fn pay(
    State(state): State<Development>,
    SignedIn(caller): SignedIn,
    Path(checkout): Path<String>,
    headers: HeaderMap,
    Form(form): Form<PaymentForm>,
) -> Result<Redirect, StatusCode> {
    let origin = headers.get("origin").and_then(|value| value.to_str().ok());
    let site = headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok());
    if origin != Some(&state.origin)
        || site != Some("same-origin")
        || !caller
            .csrf
            .accepts(df_account_app::wire::CSRF_FORM, &form.csrf)
    {
        return Err(StatusCode::FORBIDDEN);
    }
    let id = df_types::CheckoutId::from_hex(&checkout).map_err(|_| StatusCode::NOT_FOUND)?;
    state
        .ports
        .checkout
        .complete(caller.account.id, id)
        .await
        .map_err(|error| match error {
            df_portal::app_api::ports::PortError::NotFound => StatusCode::NOT_FOUND,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        })?;
    Ok(Redirect::to(&format!(
        "/app/credit/return?checkout={checkout}"
    )))
}

#[path = "google.rs"]
mod google;

async fn start_accounts(
    hosts: df_customer::HostConfig,
    identity: Identity,
    mail: Mail,
    accounts: PostgresAccountStore,
) -> Result<(CustomerService<PostgresAccountStore, Sender>, ServiceTasks), OpenError> {
    let google_hosts = hosts.clone();
    let google_accounts = accounts.clone();
    let identity = match identity {
        Identity::Local => IdentitySource::local_passwords(),
        Identity::WorkOs { client_id, api_key } => {
            IdentitySource::provider(df_workos::WorkOsIdentity::new(
                df_workos::ReqwestTransport::new(
                    df_workos::WorkOsEndpoint::Production,
                    Duration::from_secs(9),
                )
                .map_err(|_| OpenError::Identity)?,
                df_workos::WorkOsConfig::new(&client_id, api_key, &hosts)
                    .map_err(|_| OpenError::Identity)?,
            ))
        }
    };
    let mail = match mail {
        Mail::Outbox(path) => {
            Sender::Outbox(DevOutbox::open(path).await.map_err(|_| OpenError::Mail)?)
        }
        Mail::Resend(key) => Sender::resend(key).map_err(|_| OpenError::Mail)?,
    };
    let (service, tasks) = CustomerService::start(
        CustomerConfig {
            hosts,
            identity,
            tunables: Tunables::default(),
            hashing: HashingParameters::PRODUCTION,
            maximum_concurrent_hashes: 2,
            hash_permit_wait: Duration::from_secs(5),
            provider_signup_floor: Duration::from_secs(2),
            dev_allow_local_passwords_on_https: false,
        },
        accounts,
        mail,
        Arc::new(SystemClock),
        Arc::new(BuiltInCommonPasswords),
    )
    .await
    .map_err(|_| OpenError::Customer)?;
    Ok((
        google::configure(service, google_accounts, &google_hosts)
            .map_err(|_| OpenError::Identity)?,
        tasks,
    ))
}

async fn open_stores(
    config: &Config,
) -> Result<(PostgresAccountStore, crate::campaign_store::PgCampaignStore), OpenError> {
    let timeout = if config.profile == Profile::Development {
        TimeoutPolicy::NotChecked
    } else {
        TimeoutPolicy::AtMostMs(10_000)
    };
    let accounts = PostgresAccountStore::open(
        config
            .migrate_database
            .clone()
            .unwrap_or_else(|| config.database.clone()),
        config.transport.clone(),
        2,
        timeout,
    )
    .await
    .map_err(|_| OpenError::AccountStore)?;
    if let Some(migrator) = &config.migrate_database {
        let mut connection = Connection::open(migrator, &config.transport)
            .await
            .map_err(|_| OpenError::Database)?;
        crate::database::migrate(&mut connection)
            .await
            .map_err(database_error)?;
    }
    drop(accounts);
    let connector = Connector {
        config: config.database.clone(),
        transport: config.transport.clone(),
    };
    let campaign_store = crate::campaign_store::PgCampaignStore::open(
        Box::new(connector.clone()),
        Box::new(connector),
        2,
        df_campaign::config::Tunables::default(),
        if config.profile == Profile::Development {
            crate::campaign_store::TimeoutPolicy::NotChecked
        } else {
            crate::campaign_store::TimeoutPolicy::Enforced
        },
    )
    .await
    .map_err(|_| OpenError::CampaignStore)?;
    let accounts = PostgresAccountStore::open(
        config.database.clone(),
        config.transport.clone(),
        4,
        timeout,
    )
    .await
    .map_err(|_| OpenError::AccountStore)?
    .with_closure_participant(Arc::new(
        campaign_store.closure_participant(Arc::new(df_campaign::clock::SystemClock)),
    ));
    Ok((accounts, campaign_store))
}

fn database_error(error: crate::database::DatabaseError) -> OpenError {
    let (number, code) = match &error {
        crate::database::DatabaseError::Migration { number, cause } => (*number, cause.code()),
        crate::database::DatabaseError::Query(cause) => (0, cause.code()),
        crate::database::DatabaseError::Connect => (0, None),
    };
    df_observe::record_audit_line(
        "hosted.database_failed",
        format!(
            "HOSTED database_failed migration={number} sqlstate={}",
            code.map_or("connection", |code| code.code())
        ),
    );
    if number == 0 {
        OpenError::Database
    } else {
        OpenError::Migration(number)
    }
}

async fn private_headers(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        axum::http::header::REFERRER_POLICY,
        axum::http::HeaderValue::from_static("no-referrer"),
    );
    response
}
