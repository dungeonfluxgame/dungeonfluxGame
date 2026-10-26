//! A loopback preview of the account web app for browser checks: the built bundle plus the JSON
//! API over the in-memory account store, the development outbox and the fake ports. Nothing
//! persists, no mail is sent and no money moves.
//!
//! `cargo run -p df-portal --example account_app_preview --features test-fakes -- [bundle directory] [port] [site directory] [--dev-accounts]`
//!
//! The bundle directory defaults to `artifacts/build/account-app-web`; port 0 (the default) picks
//! a free port. The site directory defaults to `docs`. The server prints its address and process id
//! when it is ready, and stops cleanly on ctrl-c or when its standard input closes (set
//! `DF_PREVIEW_IGNORE_STDIN=1` to run detached with no standard input).
//!
//! Test accounts, all with the test password printed in the banner: a customer (verified, trial
//! plus purchased credit, a hosted campaign with an open lobby, statement lines), an operator
//! with every capability and the second factor, and a host with a campaign Cam can be invited
//! to. `GET /dev/outbox` (loopback only) lists the mails with their links; `/dev/checkout/<id>`
//! is the fake processor's page.

#[cfg(not(target_arch = "wasm32"))]
#[path = "account_app_preview/dev_routes.rs"]
mod dev_routes;
#[cfg(not(target_arch = "wasm32"))]
#[path = "account_app_preview/seed.rs"]
mod seed;
#[cfg(not(target_arch = "wasm32"))]
#[path = "account_app_preview/site.rs"]
mod site;

/// A TEST value for this loopback preview only; it is printed on purpose.
#[cfg(not(target_arch = "wasm32"))]
const TEST_PASSWORD: &str = "correct horse battery staple";
/// The port the game server uses; the preview never binds it.
#[cfg(not(target_arch = "wasm32"))]
const GAME_PORT: u16 = 63309;
/// Limits relaxed so a tester can sign up and in as often as they like.
#[cfg(not(target_arch = "wasm32"))]
const RELAXED_LIMITS: &str = r#"{"schema": "dungeonflux.portal/1",
    "limits": {
        "login_failures_per_email": {"limit": 1000, "window_minutes": 15},
        "login_per_ip": {"limit": 1000, "window_minutes": 15},
        "signup_per_ip": {"limit": 1000, "window_minutes": 60},
        "signup_per_email": {"limit": 1000, "window_minutes": 60},
        "reset_per_email": {"limit": 1000, "window_minutes": 60},
        "reset_per_ip": {"limit": 1000, "window_minutes": 1},
        "verification_resend_per_account": {"limit": 1000, "window_minutes": 60},
        "token_page_per_ip": {"limit": 1000, "window_minutes": 1}}}"#;

#[cfg(not(target_arch = "wasm32"))]
fn other(error: impl std::fmt::Debug) -> std::io::Error {
    std::io::Error::other(format!("{error:?}"))
}

/// Resolves when standard input reaches its end, or never when it is disabled.
#[cfg(not(target_arch = "wasm32"))]
fn stdin_closed() -> tokio::sync::oneshot::Receiver<()> {
    use std::io::Read;

    let (closed, wait) = tokio::sync::oneshot::channel();
    if std::env::var_os("DF_PREVIEW_IGNORE_STDIN").is_some() {
        // Keep the sender alive for the life of the process so the receiver never resolves.
        std::mem::forget(closed);
        return wait;
    }
    // A blocked read cannot be cancelled; the thread ends with the process.
    std::thread::spawn(move || {
        let mut sink = [0_u8; 256];
        let mut stdin = std::io::stdin();
        while matches!(stdin.read(&mut sink), Ok(count) if count > 0) {}
        let _sent = closed.send(());
    });
    wait
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> std::io::Result<()> {
    use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

    use df_customer::{
        BuiltInCommonPasswords, CustomerConfig, CustomerService, DevOutbox, HashingParameters,
        HostConfig, IdentitySource, LinkStyle, MailIdentity, PublicOrigin, SystemClock, Tunables,
        fakes::MemoryAccountStore,
    };
    use df_portal::{
        PortalOrigin,
        app_api::{AppApiState, fakes::FakePorts, router},
    };

    let mut arguments: Vec<String> = std::env::args().skip(1).collect();
    let dev_accounts_enabled = arguments
        .iter()
        .any(|argument| argument == "--dev-accounts");
    arguments.retain(|argument| argument != "--dev-accounts");
    let mut arguments = arguments.into_iter();
    let bundle = arguments
        .next()
        .unwrap_or_else(|| "artifacts/build/account-app-web".to_owned());
    let port: u16 = match arguments.next() {
        Some(text) => text.parse().map_err(other)?,
        None => 0,
    };
    let site_directory = arguments.next().unwrap_or_else(|| "docs".to_owned());
    let site_directory = std::fs::canonicalize(site_directory)?;
    if port == GAME_PORT {
        return Err(other("port 63309 belongs to the game"));
    }
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).await?;
    let origin_text = format!("http://127.0.0.1:{}", listener.local_addr()?.port());

    let outbox_directory =
        std::env::temp_dir().join(format!("df-account-preview-{}", std::process::id()));
    let outbox = DevOutbox::open(outbox_directory.clone()).await?;
    let hosts = HostConfig::new(
        PublicOrigin::parse(&origin_text).map_err(other)?,
        MailIdentity::new(
            "DungeonFlux",
            "accounts@mail.example.test",
            "support@example.test",
        )
        .map_err(other)?,
        LinkStyle::App,
    );
    let (tunables, _fallbacks) = Tunables::parse(RELAXED_LIMITS);
    let google_accounts = MemoryAccountStore::default();
    let (service, tasks) = CustomerService::start(
        CustomerConfig {
            hosts,
            identity: IdentitySource::local_passwords(),
            tunables,
            hashing: HashingParameters::TEST,
            maximum_concurrent_hashes: 4,
            hash_permit_wait: Duration::from_secs(5),
            provider_signup_floor: Duration::ZERO,
            dev_allow_local_passwords_on_https: false,
        },
        google_accounts.clone(),
        Arc::clone(&outbox),
        Arc::new(SystemClock),
        Arc::new(BuiltInCommonPasswords),
    )
    .await
    .map_err(other)?;
    let google = df_google::fake::FakeGoogle::new(&origin_text);
    let service = service.with_google(google.clone(), google_accounts);
    let ports = Arc::new(FakePorts::new(&origin_text, Arc::new(SystemClock)));
    let seeded = seed::seed(&service, &outbox, &ports, TEST_PASSWORD).await?;
    if dev_accounts_enabled {
        seed::seed_dev_accounts(&service, &outbox, &ports, TEST_PASSWORD).await?;
    }

    let state = AppApiState::new(
        service,
        PortalOrigin::parse(&origin_text).map_err(other)?,
        Arc::clone(&ports),
    );
    let mut app = router(state, &bundle)
        .merge(google_controls(google, origin_text.clone()))
        .merge(dev_routes::router(dev_routes::DevState {
            outbox: Arc::clone(&outbox),
            ports,
        }))
        .fallback(site::fallback)
        .layer(axum::Extension(Arc::new(PathBuf::from(site_directory))));
    if dev_accounts_enabled {
        app = dev_routes::with_dev_accounts(
            app,
            vec![
                df_account_app::wire::DevSignIn {
                    label: "Customer".to_owned(),
                    email: seed::DEV_CUSTOMER_EMAIL.to_owned(),
                    password: TEST_PASSWORD.to_owned(),
                },
                df_account_app::wire::DevSignIn {
                    label: "Admin".to_owned(),
                    email: seed::DEV_OPERATOR_EMAIL.to_owned(),
                    password: TEST_PASSWORD.to_owned(),
                },
            ],
        );
    }

    println!("test password for every account: {TEST_PASSWORD}");
    println!(
        "customer {}  operator {}",
        seed::CUSTOMER_EMAIL,
        seed::OPERATOR_EMAIL
    );
    if dev_accounts_enabled {
        println!(
            "dev customer {}  dev admin {} (password: {TEST_PASSWORD})",
            seed::DEV_CUSTOMER_EMAIL,
            seed::DEV_OPERATOR_EMAIL
        );
    }
    println!(
        "invitation to Moonfall Keep (once shown): {}",
        seeded.invitation_link
    );
    if let Some(code) = &seeded.room_code {
        println!("room code of The Lantern Road: {code}");
    }
    println!("bundle directory: {bundle}");
    println!("pid {}", std::process::id());
    println!("listening on {origin_text}");

    let closed = stdin_closed();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = closed => {}
        }
    })
    .await?;
    tasks.shutdown().await;
    let _removed = std::fs::remove_dir_all(outbox_directory);
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
struct GoogleControl {
    google: df_google::fake::FakeGoogle,
    origin: String,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct GoogleFixture {
    subject: String,
    email: String,
    outcome: GoogleOutcome,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum GoogleOutcome {
    Normal,
    Canceled,
    EmailNotVerified,
    Unavailable,
    Malformed,
    ReauthenticationRequired,
}

#[cfg(not(target_arch = "wasm32"))]
fn google_controls(google: df_google::fake::FakeGoogle, origin: String) -> axum::Router {
    axum::Router::new()
        .route(
            "/dev/google",
            axum::routing::get(google_fixture_page).post(set_google_fixture),
        )
        .layer(axum::extract::DefaultBodyLimit::max(4096))
        .with_state(GoogleControl { google, origin })
}

#[cfg(not(target_arch = "wasm32"))]
async fn google_fixture_page(
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if !peer.ip().is_loopback() {
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }
    axum::response::Html(concat!(
        "<!doctype html><html lang=en><meta charset=utf-8><title>Local Google fake</title>",
        "<main><h1>Local Google fake</h1><p>Only synthetic example.test identities; no Google request is made.</p>",
        "<form method=post action=/dev/google>",
        "<p><label>Google subject <input name=subject value=fake-google-subject required></label></p>",
        "<p><label>Email <input name=email value=google@example.test required></label></p>",
        "<p><label>Next result <select name=outcome>",
        "<option value=normal>Normal</option><option value=canceled>Canceled</option>",
        "<option value=email_not_verified>Email not verified</option>",
        "<option value=unavailable>Provider unavailable</option><option value=malformed>Malformed response</option>",
        "<option value=reauthentication_required>Missing fresh auth_time</option></select></label></p>",
        "<button>Set fixture</button></form><p><a href=/app/sign-up>Sign up</a> <a href=/app/sign-in>Sign in</a> <a href=/app/settings>Settings</a></p></main></html>"
    )).into_response()
}

#[cfg(not(target_arch = "wasm32"))]
async fn set_google_fixture(
    axum::extract::State(state): axum::extract::State<GoogleControl>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: axum::http::HeaderMap,
    axum::extract::Form(form): axum::extract::Form<GoogleFixture>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    use df_customer::OAuthFailure;
    if !peer.ip().is_loopback()
        || headers
            .get(axum::http::header::ORIGIN)
            .is_none_or(|value| value.as_bytes() != state.origin.as_bytes())
        || headers
            .get("sec-fetch-site")
            .is_some_and(|value| value.as_bytes() != b"same-origin")
    {
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }
    if !form.email.ends_with("@example.test")
        || state
            .google
            .set_identity(&form.subject, &form.email)
            .is_err()
    {
        return axum::http::StatusCode::UNPROCESSABLE_ENTITY.into_response();
    }
    let outcome = match form.outcome {
        GoogleOutcome::Normal => Ok(()),
        GoogleOutcome::Canceled => state.google.cancel_next_authorization(),
        GoogleOutcome::EmailNotVerified => state
            .google
            .fail_next_exchange(OAuthFailure::EmailNotVerified),
        GoogleOutcome::Unavailable => state.google.fail_next_exchange(OAuthFailure::Unavailable),
        GoogleOutcome::Malformed => state.google.fail_next_exchange(OAuthFailure::Malformed),
        GoogleOutcome::ReauthenticationRequired => state
            .google
            .fail_next_exchange(OAuthFailure::FreshAuthenticationRequired),
    };
    if outcome.is_err() {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    axum::response::Redirect::to("/dev/google").into_response()
}
