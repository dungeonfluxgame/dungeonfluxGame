//! Session entry against S1's real authority store and a provider-free fake runtime.
#![cfg(not(target_arch = "wasm32"))]
#![allow(
    dead_code,
    reason = "existing PostgreSQL fixtures expose helpers for other campaign tests"
)]

#[path = "../../df-persistence/src/account_closure.rs"]
mod account_closure;
#[path = "../../df-persistence/src/campaign_store/mod.rs"]
mod campaign_store;
#[path = "postgres/fixture.rs"]
mod fixture;

use std::{
    net::IpAddr,
    sync::Arc,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::Request,
    response::{Html, IntoResponse, Response},
};
use df_campaign::{
    clock::FakeClock,
    config::Tunables,
    fakes::{FakeAccounts, FakeFunding, FakeSessions},
    join_invitation::JoinRole,
    lobby::CharacterChoice,
    records::{AccountStatus, DeviceLabel},
    routes::{AuthClass, Caller, Guard, GuardRefusal},
    secrets::hex,
    service::{CampaignService, Ports, ServiceKeys, StartOutcome},
    session_link::{
        self, Audience, Placement, Room, RuntimeError, RuntimeLauncher, SessionLauncher, Surface,
    },
    store::SessionPhase,
};
use df_types::{AccountId, CampaignId, SessionId};
use http::{StatusCode, header};
use tower::ServiceExt;

use campaign_store::{PgCampaignStore, TimeoutPolicy};
use fixture::{World, tenant_id};

const ORIGIN: &str = "https://table.example.test";

#[derive(Default)]
struct FakeRuntime {
    down: AtomicBool,
    served: AtomicUsize,
}

#[derive(Clone)]
struct FakeLauncher(Arc<FakeRuntime>);

impl RuntimeLauncher for FakeLauncher {
    async fn allocate(&self, _: &Placement) -> Result<Room, RuntimeError> {
        if self.0.down.load(Ordering::SeqCst) {
            Err(RuntimeError::Unavailable)
        } else {
            Ok(Room {
                code: "LANTERN".to_owned(),
                run_identity: "demo-run-1".to_owned(),
            })
        }
    }

    async fn serve(
        &self,
        _: &Placement,
        _: &Room,
        audience: Audience,
        surface: Surface,
        request: Request,
    ) -> Result<Response, RuntimeError> {
        assert!(!request.headers().contains_key(header::COOKIE));
        assert!(!request.headers().contains_key(header::AUTHORIZATION));
        assert!(!request.headers().contains_key("x-test-account"));
        assert!(!request.headers().contains_key("x-dungeonflux-launcher"));
        self.0.served.fetch_add(1, Ordering::SeqCst);
        let text = match (surface, audience) {
            (Surface::Play, Audience::Player(_)) => "<html>player game</html>",
            (Surface::Display, Audience::Display) => "<html>display game</html>",
            (Surface::Tunnel, _) => "tunnel reached",
            _ => return Err(RuntimeError::InvalidReply),
        };
        Ok(Html(text).into_response())
    }
}

struct TestPorts;
impl Ports for TestPorts {
    type Store = PgCampaignStore;
    type Accounts = FakeAccounts;
    type Sessions = SessionLauncher<FakeSessions, FakeLauncher>;
    type Funding = FakeFunding;
    type Time = FakeClock;
}

struct TestGuard;
impl Guard for TestGuard {
    async fn caller(
        &self,
        parts: &http::request::Parts,
        _: AuthClass,
    ) -> Result<Caller, GuardRefusal> {
        let account = parts
            .headers
            .get("x-test-account")
            .and_then(|value| value.to_str().ok())
            .and_then(|text| AccountId::from_hex(text).ok())
            .ok_or(GuardRefusal::Unauthenticated)?;
        Ok(Caller {
            account: Some(account),
            client_ip: IpAddr::from([203, 0, 113, 1]),
        })
    }
}

struct Table {
    world: World,
    service: CampaignService<TestPorts>,
    router: Router,
    runtime: Arc<FakeRuntime>,
    sessions: FakeSessions,
    host: AccountId,
    campaign: CampaignId,
    session: SessionId,
}

impl Table {
    async fn new(label: &str) -> Self {
        let world = World::new(label).await;
        let host = world.account(1).await;
        let campaign = world.campaign(host, 2).await.id;
        let accounts = FakeAccounts::new();
        accounts.add(host, "Host", true, Some(tenant_id()), AccountStatus::Active);
        let sessions = FakeSessions::new();
        let runtime = Arc::new(FakeRuntime::default());
        let launcher = SessionLauncher::new(
            sessions.clone(),
            FakeLauncher(Arc::clone(&runtime)),
            Duration::from_secs(2),
        );
        let store = PgCampaignStore::open(
            world.db.connector(&world.role),
            world.db.connector(&world.role),
            2,
            Tunables::default(),
            TimeoutPolicy::Enforced,
        )
        .await
        .unwrap();
        let service = CampaignService::<TestPorts>::new(
            store,
            accounts,
            launcher.clone(),
            FakeFunding::new(),
            world.clock.clone(),
            Tunables::default(),
            ServiceKeys::from_hex(&"a1".repeat(32), &"b2".repeat(32)).unwrap(),
        );
        let (session, _) = service.open_lobby(host, campaign).await.unwrap();
        service
            .choose_character(
                host,
                campaign,
                session,
                CharacterChoice::Template,
                Some([9; 16]),
            )
            .await
            .unwrap();
        service
            .set_ready(host, campaign, session, true)
            .await
            .unwrap();
        let router = session_link::router(session_link::App {
            service: service.clone(),
            guard: Arc::new(TestGuard),
            launcher,
            origin: ORIGIN.into(),
        });
        Self {
            world,
            service,
            router,
            runtime,
            sessions,
            host,
            campaign,
            session,
        }
    }

    async fn start(&self) {
        assert!(matches!(
            self.service
                .start_session(self.host, self.campaign, self.session)
                .await
                .unwrap(),
            StartOutcome::Started { .. }
        ));
    }

    async fn get(
        &self,
        surface: &str,
        account: Option<AccountId>,
        cookies: Option<&str>,
        origin: Option<&str>,
    ) -> Response {
        let mut request = http::Request::builder().uri(format!(
            "/sessions/{}/{surface}",
            hex(self.session.as_bytes())
        ));
        request = request.header("x-dungeonflux-launcher", "untrusted-browser-value");
        if let Some(account) = account {
            request = request.header("x-test-account", hex(account.as_bytes()));
        }
        if let Some(cookies) = cookies {
            request = request.header(header::COOKIE, cookies);
        }
        if let Some(origin) = origin {
            request = request.header(header::ORIGIN, origin);
        }
        self.router
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    async fn cleanup(self) {
        drop(self.router);
        drop(self.service);
        self.world.cleanup().await;
    }
}

async fn text(response: Response) -> String {
    String::from_utf8(
        axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}

#[tokio::test]
async fn session_link_started_session_serves_the_game_to_its_seated_member() {
    let table = Table::new("link_member").await;
    assert_eq!(
        table
            .get("play", Some(table.host), None, None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    table.start().await;
    let response = table.get("play", Some(table.host), None, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(text(response).await, "<html>player game</html>");
    table.cleanup().await;
}

#[tokio::test]
async fn session_link_nonmembers_unseated_members_and_anonymous_callers_are_concealed() {
    let table = Table::new("link_stranger").await;
    let stranger = table.world.account(2).await;
    let contributor = table.world.account(3).await;
    table
        .world
        .join(
            table.campaign,
            table.host,
            contributor,
            JoinRole::Contributor,
        )
        .await;
    table.start().await;
    for account in [None, Some(stranger), Some(contributor)] {
        assert_eq!(
            table.get("play", account, None, None).await.status(),
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(table.runtime.served.load(Ordering::SeqCst), 0);
    table.cleanup().await;
}

#[tokio::test]
async fn session_link_runtime_down_serves_a_waiting_page_then_retry_opens_the_game() {
    let table = Table::new("link_down").await;
    table.runtime.down.store(true, Ordering::SeqCst);
    table.start().await;
    let response = table.get("play", Some(table.host), None, None).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()[header::RETRY_AFTER], "3");
    let body = text(response).await;
    assert!(body.contains("The table is getting ready"));
    assert!(body.contains("Retry"));
    table.runtime.down.store(false, Ordering::SeqCst);
    assert_eq!(
        table
            .get("play", Some(table.host), None, None)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(table.sessions.started().len(), 1);
    table.cleanup().await;
}

#[tokio::test]
async fn session_link_a_paired_display_gets_only_the_display_and_wrong_device_is_concealed() {
    let table = Table::new("link_display").await;
    let ip = IpAddr::from([203, 0, 113, 2]);
    let challenge = table
        .service
        .begin_display_pairing(ip, DeviceLabel::Desktop)
        .await
        .unwrap();
    table
        .service
        .approve_display_pairing(
            table.host,
            table.campaign,
            table.session,
            challenge.pairing.id,
        )
        .await
        .unwrap();
    let ticket = table
        .service
        .redeem_display_ticket(ip, &challenge.device_secret, None)
        .await
        .unwrap();
    let cookies = format!(
        "__Host-df_display_device={}; __Host-df_display_ticket={}",
        challenge.device_secret, ticket.ticket_secret
    );
    table.start().await;
    let response = table.get("display", None, Some(&cookies), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(text(response).await, "<html>display game</html>");
    assert_eq!(
        table.get("play", None, Some(&cookies), None).await.status(),
        StatusCode::NOT_FOUND
    );
    let wrong = format!(
        "__Host-df_display_device={}; __Host-df_display_ticket={}",
        "aa".repeat(32),
        ticket.ticket_secret
    );
    assert_eq!(
        table
            .get("display", None, Some(&wrong), None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    table.cleanup().await;
}

#[tokio::test]
async fn session_link_tunnel_requires_a_current_seat_and_exact_same_origin() {
    let table = Table::new("link_tunnel").await;
    table.start().await;
    assert_eq!(
        table
            .get("tunnel", Some(table.host), None, None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        table
            .get(
                "tunnel",
                Some(table.host),
                None,
                Some("https://foreign.test")
            )
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        table.get("tunnel", None, None, Some(ORIGIN)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        text(
            table
                .get("tunnel", Some(table.host), None, Some(ORIGIN))
                .await
        )
        .await,
        "tunnel reached"
    );
    table
        .sessions
        .set(table.session, table.campaign, SessionPhase::Ended);
    assert_eq!(
        table
            .get("play", Some(table.host), None, None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    table.cleanup().await;
}
