//! The JSON routes against the real store: the route table walked as a whole (the matrix, the
//! display ticket, confused deputies), the display cookies and the join flow.

use df_campaign::store::NewInvitation;
use std::{net::IpAddr, sync::Arc};

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use df_campaign::{
    clock::FakeClock,
    config::Tunables,
    fakes::{FakeAccounts, FakeFunding, FakeSessions},
    join_invitation::JoinRole,
    records::AccountStatus,
    roles::{CampaignRole, Capability, Delegations, authorize},
    routes::{
        App, AuthClass, Caller, DEVICE_COOKIE, Gate, Guard, GuardRefusal, ROUTES, RouteId,
        RouteSpec, TICKET_COOKIE, Verb, router,
    },
    secrets::hex,
    service::{CampaignService, Ports, ServiceKeys},
    store::{CampaignStore, SessionPhase},
};
use df_types::{AccountId, CampaignId, MemberId, SessionId};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::{
    campaign_store::{PgCampaignStore, TimeoutPolicy},
    fixture::{World, account_id, digest_of, session_id, tenant_id, token_digest},
};

/// The configured keys every replica of the test service shares.
pub fn shared_keys() -> ServiceKeys {
    ServiceKeys::from_hex(&"a1".repeat(32), &"b2".repeat(32)).expect("valid configured keys")
}

pub struct TestPorts;

impl Ports for TestPorts {
    type Store = PgCampaignStore;
    type Accounts = FakeAccounts;
    type Sessions = FakeSessions;
    type Funding = FakeFunding;
    type Time = FakeClock;
}

/// B's guard, stood in for: a session is a header naming the account. A display ticket or device
/// cookie never makes an identity.
pub struct TestGuard;

impl Guard for TestGuard {
    async fn caller(
        &self,
        parts: &axum::http::request::Parts,
        class: AuthClass,
    ) -> Result<Caller, GuardRefusal> {
        let client_ip: IpAddr = parts
            .headers
            .get("x-test-ip")
            .and_then(|value| value.to_str().ok())
            .and_then(|text| text.parse().ok())
            .unwrap_or_else(|| "203.0.113.1".parse().unwrap());
        let account = parts
            .headers
            .get("x-test-account")
            .and_then(|value| value.to_str().ok())
            .and_then(|text| AccountId::from_hex(text).ok());
        match class {
            AuthClass::Session => match account {
                Some(account) => Ok(Caller {
                    account: Some(account),
                    client_ip,
                }),
                None => Err(GuardRefusal::Unauthenticated),
            },
            AuthClass::PublicWithPresessionCsrf => {
                if parts.headers.contains_key("x-presession") {
                    Ok(Caller {
                        account: None,
                        client_ip,
                    })
                } else {
                    Err(GuardRefusal::CsrfRefused)
                }
            }
            AuthClass::Public | AuthClass::Device => Ok(Caller {
                account: None,
                client_ip,
            }),
        }
    }
}

pub const ORIGIN: &str = "https://play.example.test";

pub struct Response {
    pub status: StatusCode,
    pub headers: axum::http::HeaderMap,
    pub cookies: Vec<String>,
    pub body: Value,
    pub text: String,
}

pub struct Site {
    pub world: World,
    pub router: Router,
    pub service: CampaignService<TestPorts>,
    pub sessions: FakeSessions,
    pub accounts: FakeAccounts,
    pub funding: FakeFunding,
}

impl Site {
    pub async fn new(label: &str) -> Self {
        Self::with_tunables(label, Tunables::default()).await
    }

    pub async fn with_tunables(label: &str, tunables: Tunables) -> Self {
        let mut world = World::with_tunables(label, tunables.clone()).await;
        let role = world.db.login_role(Some("10s")).await;
        let store = PgCampaignStore::open(
            world.db.connector(&role),
            world.db.connector(&role),
            4,
            tunables.clone(),
            TimeoutPolicy::Enforced,
        )
        .await
        .expect("the service store opens");
        let accounts = FakeAccounts::new();
        let sessions = FakeSessions::new();
        let funding = FakeFunding::new();
        let service = CampaignService::<TestPorts>::new(
            store,
            accounts.clone(),
            sessions.clone(),
            funding.clone(),
            world.clock.clone(),
            tunables,
            shared_keys(),
        );
        let app = App {
            service: service.clone(),
            guard: Arc::new(TestGuard),
            origin: ORIGIN.into(),
        };
        let router = router(app);
        Self {
            world,
            router,
            service,
            sessions,
            accounts,
            funding,
        }
    }

    /// A second service over the same database with its own store pool and the given keys: a
    /// second replica (or a restart).
    pub async fn another_service(&self, keys: ServiceKeys) -> CampaignService<TestPorts> {
        let tunables = self.service.tunables().clone();
        let store = PgCampaignStore::open(
            self.world.db.connector(&self.world.role),
            self.world.db.connector(&self.world.role),
            2,
            tunables.clone(),
            TimeoutPolicy::Enforced,
        )
        .await
        .expect("the second store opens");
        CampaignService::<TestPorts>::new(
            store,
            self.accounts.clone(),
            self.sessions.clone(),
            self.funding.clone(),
            self.world.clock.clone(),
            tunables,
            keys,
        )
    }

    /// An account known to the database and to the account directory.
    pub async fn person(&self, number: u8) -> AccountId {
        let id = self.world.account(number).await;
        self.accounts.add(
            id,
            &format!("Person {number}"),
            true,
            Some(tenant_id()),
            AccountStatus::Active,
        );
        id
    }

    pub async fn send(
        &self,
        method: Method,
        path: &str,
        account: Option<AccountId>,
        body: Option<&Value>,
        extra: &[(&str, String)],
    ) -> Response {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(account) = account {
            request = request.header("x-test-account", hex(account.as_bytes()));
        }
        if body.is_some() {
            request = request.header(header::CONTENT_TYPE, "application/json");
        }
        for (name, value) in extra {
            request = request.header(*name, value.as_str());
        }
        let bytes = body
            .map(|value| serde_json::to_vec(value).unwrap())
            .unwrap_or_default();
        let response = self
            .router
            .clone()
            .oneshot(request.body(Body::from(bytes)).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let cookies = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|value| value.to_str().unwrap().to_owned())
            .collect();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Response {
            status,
            headers,
            cookies,
            body,
            text,
        }
    }

    pub async fn post(&self, path: &str, account: AccountId, body: &Value) -> Response {
        self.send(Method::POST, path, Some(account), Some(body), &[])
            .await
    }

    pub async fn cleanup(self) {
        let Self {
            world,
            router,
            service,
            ..
        } = self;
        drop(router);
        drop(service);
        world.cleanup().await;
    }
}

#[tokio::test]
async fn malformed_and_unknown_invites_check_verification_before_lookup_and_match_responses() {
    let site = Site::with_tunables(
        "unverified_invites",
        Tunables {
            require_verified_email_to_join: true,
            ..Tunables::default()
        },
    )
    .await;
    let account = site.person(20).await;
    site.accounts.add(
        account,
        "Person 20",
        false,
        Some(tenant_id()),
        AccountStatus::Active,
    );
    let malformed = site
        .post(
            "/join/accept",
            account,
            &json!({"token": "!", "operation": operation()}),
        )
        .await;
    let unknown = site
        .post(
            "/join/accept",
            account,
            &json!({"token": "a".repeat(64), "operation": operation()}),
        )
        .await;

    assert_eq!(malformed.status, unknown.status);
    assert_eq!(malformed.headers, unknown.headers);
    assert_eq!(malformed.text, unknown.text);
    assert_eq!(malformed.body, json!({"reason": "not_allowed"}));
    site.cleanup().await;
}

type Identity = (&'static str, AccountId, Option<(CampaignRole, Delegations)>);

pub fn id_text(bytes: &[u8; 16]) -> String {
    hex(bytes)
}

pub(crate) fn cookie_pair(response: &Response, name: &str) -> Option<String> {
    response
        .cookies
        .iter()
        .find(|cookie| cookie.starts_with(&format!("{name}=")))
        .map(|cookie| cookie.split(';').next().unwrap().to_owned())
}

pub(crate) fn max_age(response: &Response, name: &str) -> Option<u64> {
    response
        .cookies
        .iter()
        .find(|cookie| cookie.starts_with(&format!("{name}=")))
        .and_then(|cookie| {
            cookie
                .split(';')
                .find_map(|part| part.trim().strip_prefix("Max-Age=").map(str::to_owned))
        })
        .and_then(|text| text.parse().ok())
}

pub(crate) fn operation() -> String {
    hex(&df_campaign::secrets::random_identity().unwrap())
}

pub(crate) fn create_body(name: &str) -> Value {
    json!({
        "name": name, "seat_count": 6, "content_rating": "standard", "room_codes": true,
        "default_co_host": {"invite_players": false, "start_or_pause_session": false, "export_campaign": false},
        "operation": operation(),
    })
}

/// The people at one table.
pub struct Table {
    pub campaign: CampaignId,
    pub host: AccountId,
    pub cohost_all: AccountId,
    pub cohost_none: AccountId,
    pub player: AccountId,
    pub contributor: AccountId,
    pub outsider: AccountId,
}

pub async fn table(site: &Site) -> Table {
    table_with(site, 6).await
}

pub async fn table_with(site: &Site, seats: u32) -> Table {
    let host = site.person(1).await;
    let cohost_all = site.person(2).await;
    let cohost_none = site.person(3).await;
    let player = site.person(4).await;
    let contributor = site.person(5).await;
    let outsider = site.person(6).await;
    let campaign = site.world.campaign(host, seats).await.id;
    let world = &site.world;
    let all = world
        .join(campaign, host, cohost_all, JoinRole::Player)
        .await;
    let none = world
        .join(campaign, host, cohost_none, JoinRole::Player)
        .await;
    world.join(campaign, host, player, JoinRole::Player).await;
    world
        .join(campaign, host, contributor, JoinRole::Contributor)
        .await;
    world
        .store
        .set_co_host(host, campaign, all, Delegations::ALL, world.now())
        .await
        .unwrap();
    world
        .store
        .set_co_host(host, campaign, none, Delegations::NONE, world.now())
        .await
        .unwrap();
    Table {
        campaign,
        host,
        cohost_all,
        cohost_none,
        player,
        contributor,
        outsider,
    }
}

pub(crate) fn fill(path: &str, ids: &[(&str, String)]) -> String {
    let mut text = path.to_owned();
    for (name, value) in ids {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

pub(crate) fn random_hex() -> String {
    operation()
}

/// A body that is valid for the route, so a refusal is about authority and not about shape.
fn sample_body(route: &RouteSpec, campaign_name: &str) -> Option<Value> {
    use RouteId::*;
    Some(match route.id {
        CreateCampaign => create_body("Another"),
        StartDraft => json!({}),
        OpenLobby | LobbyStart => json!({}),
        SaveDraftStep => {
            json!({"name": "Another", "adventure_kind": "bundled", "content_rating": "standard", "seat_count": 4})
        }
        FinishDraft => json!({"operation": operation()}),
        Archive | RequestDelete => json!({"typed_name": campaign_name}),
        CreateInvitation => json!({"joins_as": "player", "lifetime_hours": null, "max_uses": null}),
        JoinPreview => json!({"token": random_hex() + &random_hex()}),
        JoinAccept => json!({"token": random_hex() + &random_hex(), "operation": operation()}),
        CodeLookup | CodeRequest => json!({"code": "ABCD-EFGH"}),
        ApproveRequest => json!({"role": "player"}),
        SetCoHost => {
            json!({"invite_players": true, "start_or_pause_session": false, "export_campaign": false})
        }
        SetSeat => json!({"seated": false}),
        SetPlayerRole => json!({"role": "contributor"}),
        ProposeTransfer => {
            json!({"kind": "host", "to_member": random_hex(), "old_host_becomes": null})
        }
        Unarchive | RevokeInvitation | RotateCode | DenyRequest | RemoveMember | AllowRejoin
        | DemoteCoHost | Leave | AcceptTransfer | CancelTransfer | MyMembership | GetDraft
        | Dashboard | Overview | CampaignSettings | CampaignSessions | LobbyView => {
            return None;
        }
    })
}

pub(crate) fn method_of(route: &RouteSpec) -> Method {
    match route.verb {
        Verb::Get => Method::GET,
        Verb::Post => Method::POST,
        Verb::Put => Method::PUT,
    }
}

// ---- The table as a whole --------------------------------------------------------------------

#[tokio::test]
async fn unregistered_paths_and_methods_are_not_served() {
    let site = Site::new("unserved").await;
    let table = table(&site).await;
    let campaign = id_text(table.campaign.as_bytes());
    let probes = [
        (Method::GET, format!("/campaigns/{campaign}/unregistered")),
        (Method::DELETE, format!("/campaigns/{campaign}/archive")),
        (Method::PUT, "/campaigns".to_owned()),
        (Method::GET, "/nothing".to_owned()),
        (Method::POST, format!("/campaigns/{campaign}/code")),
        (Method::POST, "/display".to_owned()),
    ];
    for (method, path) in probes {
        let response = site
            .send(method.clone(), &path, Some(table.host), None, &[])
            .await;
        assert!(
            response.status == StatusCode::NOT_FOUND
                || response.status == StatusCode::METHOD_NOT_ALLOWED,
            "{method} {path} -> {}",
            response.status
        );
        assert!(response.text.is_empty() || !response.text.contains("The Sunken Bell"));
    }
    site.cleanup().await;
}

#[tokio::test]
async fn every_session_route_refuses_a_request_with_no_session() {
    let site = Site::new("nosession").await;
    let table = table(&site).await;
    for route in ROUTES
        .iter()
        .filter(|route| route.auth == AuthClass::Session)
    {
        let ids = [
            ("campaign", id_text(table.campaign.as_bytes())),
            ("member", random_hex()),
            ("invitation", random_hex()),
            ("request", random_hex()),
            ("proposal", random_hex()),
            ("draft", random_hex()),
            ("step", "1".to_owned()),
            ("session", id_text(session_id(1).as_bytes())),
        ];
        let path = fill(route.path, &ids);
        let body = sample_body(route, "The Sunken Bell");
        let response = site
            .send(method_of(route), &path, None, body.as_ref(), &[])
            .await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED, "{:?}", route.id);
    }
    site.cleanup().await;
}

/// The game-owned service supplies a valid ticket; website routes never issue one.
pub(crate) async fn paired_display(
    site: &Site,
    table: &Table,
    session: SessionId,
) -> (String, String) {
    let ip = "203.0.113.80".parse().unwrap();
    let challenge = site
        .service
        .begin_display_pairing(ip, df_campaign::records::DeviceLabel::Desktop)
        .await
        .unwrap();
    site.service
        .approve_display_pairing(table.host, table.campaign, session, challenge.pairing.id)
        .await
        .unwrap();
    let ticket = site
        .service
        .redeem_display_ticket(ip, &challenge.device_secret, None)
        .await
        .unwrap();
    (
        format!("{DEVICE_COOKIE}={}", challenge.device_secret),
        format!("{TICKET_COOKIE}={}", ticket.ticket_secret),
    )
}

#[tokio::test]
async fn a_display_ticket_is_refused_by_every_non_public_route() {
    let site = Site::new("ticketrefused").await;
    let table = table(&site).await;
    let session = session_id(1);
    site.sessions
        .set(session, table.campaign, SessionPhase::Lobby);
    let (device, ticket) = paired_display(&site, &table, session).await;
    let before = site.world.events(table.campaign).await.len();
    let marker = "The Sunken Bell";
    for route in ROUTES
        .iter()
        .filter(|route| route.auth == AuthClass::Session)
    {
        let ids = [
            ("campaign", id_text(table.campaign.as_bytes())),
            ("member", random_hex()),
            ("invitation", random_hex()),
            ("request", random_hex()),
            ("proposal", random_hex()),
            ("draft", random_hex()),
            ("step", "1".to_owned()),
            ("session", id_text(session.as_bytes())),
        ];
        let path = fill(route.path, &ids);
        let body = sample_body(route, marker);
        let cookies = format!("{device}; {ticket}");
        let response = site
            .send(
                method_of(route),
                &path,
                None,
                body.as_ref(),
                &[
                    ("cookie", cookies),
                    ("x-df-device", "1".into()),
                    ("origin", ORIGIN.into()),
                ],
            )
            .await;
        assert_eq!(
            response.status,
            StatusCode::UNAUTHORIZED,
            "{:?} {}",
            route.id,
            route.path
        );
        assert!(
            !response.text.contains(marker),
            "{:?} leaked private content",
            route.id
        );
        assert!(!response.text.contains("Person"), "{:?}", route.id);
    }
    assert_eq!(
        site.world.events(table.campaign).await.len(),
        before,
        "nothing changed"
    );
    site.cleanup().await;
}

#[tokio::test]
async fn every_route_admits_exactly_the_roles_the_matrix_allows() {
    let tunables = Tunables {
        maximum_seats: 64,
        ..Tunables::default()
    };
    let site = Site::with_tunables("matrix", tunables).await;
    let table = table_with(&site, 64).await;
    let world = &site.world;
    let session = session_id(1);
    site.sessions
        .set(session, table.campaign, SessionPhase::Lobby);
    let campaign = id_text(table.campaign.as_bytes());
    let identities: [Identity; 6] = [
        (
            "host",
            table.host,
            Some((CampaignRole::Host, Delegations::NONE)),
        ),
        (
            "cohost_all",
            table.cohost_all,
            Some((CampaignRole::CoHost, Delegations::ALL)),
        ),
        (
            "cohost_none",
            table.cohost_none,
            Some((CampaignRole::CoHost, Delegations::NONE)),
        ),
        (
            "player",
            table.player,
            Some((CampaignRole::Player, Delegations::NONE)),
        ),
        (
            "contributor",
            table.contributor,
            Some((CampaignRole::Contributor, Delegations::NONE)),
        ),
        ("outsider", table.outsider, None),
    ];
    let mut next_account = 30_u8;

    let routes: Vec<&RouteSpec> = ROUTES
        .iter()
        .filter(|route| matches!(route.gate, Gate::Capability(_)))
        .collect();
    assert!(routes.len() >= 8, "the table has capability routes to walk");
    for route in routes {
        let Gate::Capability(capability) = route.gate else {
            unreachable!()
        };
        for (index, (label, account, standing)) in identities.iter().enumerate() {
            if route.id == RouteId::LobbyStart {
                site.sessions
                    .set(session, table.campaign, SessionPhase::Lobby);
            }
            // A fresh target for every attempt, prepared by a person who may prepare it.
            let mut ids = vec![
                ("campaign", campaign.clone()),
                ("session", id_text(session.as_bytes())),
            ];
            let body = sample_body(route, "The Sunken Bell");
            next_account += 1;
            let target = site.person(next_account).await;
            let target_member: MemberId = match route.id {
                RouteId::RemoveMember | RouteId::SetPlayerRole => {
                    world
                        .join(table.campaign, table.host, target, JoinRole::Player)
                        .await
                }
                RouteId::AllowRejoin => {
                    let member = world
                        .join(table.campaign, table.host, target, JoinRole::Player)
                        .await;
                    world
                        .store
                        .remove_member(table.cohost_all, table.campaign, member, world.now())
                        .await
                        .unwrap();
                    member
                }
                _ => MemberId::from_bytes(&[9; 16]).unwrap(),
            };
            ids.push(("member", id_text(target_member.as_bytes())));
            match route.id {
                RouteId::RevokeInvitation => {
                    let row = world
                        .store
                        .create_invitation(
                            table.host,
                            table.campaign,
                            NewInvitation {
                                joins_as: JoinRole::Player,
                                lifetime_ms: 60_000,
                                max_uses: 3,
                                token_digest: token_digest(),
                            },
                            world.now(),
                        )
                        .await
                        .unwrap();
                    ids.push(("invitation", row.id.text()));
                }
                RouteId::ApproveRequest | RouteId::DenyRequest => {
                    let code = digest_of(&format!("matrix-code-{index}-{next_account}"));
                    world
                        .store
                        .rotate_code(table.host, table.campaign, code, 60_000, world.now())
                        .await
                        .unwrap();
                    let request = world
                        .store
                        .request_join(target, code, world.now())
                        .await
                        .unwrap();
                    ids.push(("request", request.id.text()));
                }
                _ => {}
            }
            let path = fill(route.path, &ids);
            let response = site
                .send(
                    method_of(route),
                    &path,
                    Some(*account),
                    body.as_ref(),
                    &[("x-test-ip", format!("198.51.100.{}", 10 + next_account))],
                )
                .await;
            let allowed = standing.is_some_and(|(role, delegations)| {
                authorize(role, delegations, capability) == Ok(df_campaign::roles::Authorized::Full)
            });
            assert_eq!(
                response.status == StatusCode::OK,
                allowed,
                "{:?} as {label}: {} {}",
                route.id,
                response.status,
                response.text
            );
            if !allowed {
                let expected = if standing.is_none() {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::FORBIDDEN
                };
                assert_eq!(response.status, expected, "{:?} as {label}", route.id);
            }
        }
    }
    site.cleanup().await;
}

#[tokio::test]
async fn account_b_ids_answer_like_missing_ids_on_every_route() {
    let site = Site::new("confused").await;
    let table = table(&site).await;
    let session = session_id(1);
    site.sessions
        .set(session, table.campaign, SessionPhase::Lobby);
    let real = id_text(table.campaign.as_bytes());
    for route in ROUTES
        .iter()
        .filter(|route| route.auth == AuthClass::Session)
    {
        if !route.path.contains("{campaign}") {
            continue;
        }
        let known = |campaign: String| {
            fill(
                route.path,
                &[
                    ("campaign", campaign),
                    (
                        "member",
                        id_text(MemberId::from_bytes(&[9; 16]).unwrap().as_bytes()),
                    ),
                    ("invitation", random_hex()),
                    ("request", random_hex()),
                    ("proposal", random_hex()),
                    ("draft", random_hex()),
                    ("step", "1".to_owned()),
                    ("session", id_text(session.as_bytes())),
                ],
            )
        };
        let body = sample_body(route, "The Sunken Bell");
        let account_b = Some(table.outsider);
        let on_real = site
            .send(
                method_of(route),
                &known(real.clone()),
                account_b,
                body.as_ref(),
                &[],
            )
            .await;
        let on_random = site
            .send(
                method_of(route),
                &known(random_hex()),
                account_b,
                body.as_ref(),
                &[],
            )
            .await;
        assert_eq!(on_real.status, on_random.status, "{:?}", route.id);
        assert_eq!(on_real.text, on_random.text, "{:?}", route.id);
    }
    site.cleanup().await;
}

#[tokio::test]
async fn a_child_of_another_campaign_the_same_account_manages_answers_like_a_missing_id() {
    let site = Site::new("otherchild").await;
    let table = table(&site).await;
    let world = &site.world;
    let other = world.campaign(table.host, 6).await.id;
    let guest = site.person(20).await;
    let foreign_member = world.join(other, table.host, guest, JoinRole::Player).await;
    let foreign_invitation = world
        .store
        .create_invitation(
            table.host,
            other,
            NewInvitation {
                joins_as: JoinRole::Player,
                lifetime_ms: 60_000,
                max_uses: 3,
                token_digest: token_digest(),
            },
            world.now(),
        )
        .await
        .unwrap();
    let x = id_text(table.campaign.as_bytes());
    let calls = [
        (
            format!(
                "/campaigns/{x}/members/{}/remove",
                id_text(foreign_member.as_bytes())
            ),
            json!({}),
        ),
        (
            format!(
                "/campaigns/{x}/members/{}/allow-rejoin",
                id_text(foreign_member.as_bytes())
            ),
            json!({}),
        ),
        (
            format!(
                "/campaigns/{x}/members/{}/role",
                id_text(foreign_member.as_bytes())
            ),
            json!({"role": "contributor"}),
        ),
        (
            format!(
                "/campaigns/{x}/invitations/{}/revoke",
                foreign_invitation.id.text()
            ),
            json!({}),
        ),
    ];
    let missing = [
        (
            format!("/campaigns/{x}/members/{}/remove", random_hex()),
            json!({}),
        ),
        (
            format!("/campaigns/{x}/members/{}/allow-rejoin", random_hex()),
            json!({}),
        ),
        (
            format!("/campaigns/{x}/members/{}/role", random_hex()),
            json!({"role": "contributor"}),
        ),
        (
            format!("/campaigns/{x}/invitations/{}/revoke", random_hex()),
            json!({}),
        ),
    ];
    for ((foreign_path, body), (missing_path, _)) in calls.iter().zip(missing.iter()) {
        let foreign = site.post(foreign_path, table.host, body).await;
        let absent = site.post(missing_path, table.host, body).await;
        assert_eq!(foreign.status, StatusCode::NOT_FOUND, "{foreign_path}");
        assert_eq!(foreign.status, absent.status);
        assert_eq!(foreign.text, absent.text);
    }
    site.cleanup().await;
}

// ---- Join flow, preview and the guess throttle -----------------------------------------------

#[tokio::test]
async fn the_join_flow_runs_over_the_routes_and_the_token_is_shown_once() {
    let site = Site::new("joinflow").await;
    let host = site.person(1).await;
    let guest = site.person(2).await;
    let created = site
        .post("/campaigns", host, &create_body("The Sunken Bell"))
        .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.text);
    let campaign = created.body["campaign_id"].as_str().unwrap().to_owned();

    let invitation = site
        .post(
            &format!("/campaigns/{campaign}/invitations"),
            host,
            &json!({"joins_as": "player", "lifetime_hours": null, "max_uses": null}),
        )
        .await;
    assert_eq!(invitation.status, StatusCode::OK, "{}", invitation.text);
    let token = invitation.body["token"].as_str().unwrap().to_owned();
    assert_eq!(token.len(), 64);
    assert_eq!(invitation.body["link"], format!("/app/join#token={token}"));

    // The preview is anonymous and has exactly four fields.
    let preview = site
        .send(
            Method::POST,
            "/join/preview",
            None,
            Some(&json!({"token": token})),
            &[],
        )
        .await;
    assert_eq!(preview.status, StatusCode::OK, "{}", preview.text);
    let mut keys: Vec<_> = preview.body["preview"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "campaign_name",
            "content_rating",
            "host_display_name",
            "scheduled_at_ms"
        ]
    );
    assert_eq!(preview.body["preview"]["host_display_name"], "Person 1");

    let operation = operation();
    let accept_body = json!({"token": token, "operation": operation});
    let accepted = site.post("/join/accept", guest, &accept_body).await;
    assert_eq!(accepted.status, StatusCode::OK, "{}", accepted.text);
    assert_eq!(accepted.body["rejoined"], false);
    let repeat = site.post("/join/accept", guest, &accept_body).await;
    assert_eq!(
        repeat.body, accepted.body,
        "a repeated operation returns the first result"
    );

    let me = site
        .send(
            Method::GET,
            &format!("/campaigns/{campaign}/me"),
            Some(guest),
            None,
            &[],
        )
        .await;
    assert_eq!(me.body["role"], "player");
    assert!(me.body["authorized"].as_bool().unwrap());
    // The secret appears in the creation response only.
    for response in [&preview, &accepted, &me, &repeat] {
        assert!(!response.text.contains(&token));
    }
    // F is told, exactly once per membership event, and can do nothing else.
    let calls = site.funding.calls();
    let mut calls = calls;
    calls.dedup();
    assert_eq!(
        calls.len(),
        1,
        "a repeat carries the same event and F is idempotent on it"
    );
    assert_eq!(id_text(calls[0].0.as_bytes()), campaign);
    site.cleanup().await;
}

#[tokio::test]
async fn failure_responses_show_no_campaign_details_and_unknown_equals_malformed() {
    let site = Site::new("nodetails").await;
    let host = site.person(1).await;
    let campaign = site.world.campaign(host, 2).await;
    let world = &site.world;
    let mut tokens = Vec::new();
    // Expired, revoked and exhausted links share the unknown-token response.
    let expired = df_campaign::secrets::SecretToken::generate().unwrap();
    let revoked = df_campaign::secrets::SecretToken::generate().unwrap();
    let spent = df_campaign::secrets::SecretToken::generate().unwrap();
    let tag = df_campaign::secrets::INVITATION_TOKEN_TAG;
    world
        .store
        .create_invitation(
            host,
            campaign.id,
            NewInvitation {
                joins_as: JoinRole::Player,
                lifetime_ms: 1,
                max_uses: 1,
                token_digest: expired.digest(tag),
            },
            world.now(),
        )
        .await
        .unwrap();
    let revoked_row = world
        .store
        .create_invitation(
            host,
            campaign.id,
            NewInvitation {
                joins_as: JoinRole::Player,
                lifetime_ms: 60_000,
                max_uses: 1,
                token_digest: revoked.digest(tag),
            },
            world.now(),
        )
        .await
        .unwrap();
    world
        .store
        .revoke_invitation(host, campaign.id, revoked_row.id, world.now())
        .await
        .unwrap();
    world
        .store
        .create_invitation(
            host,
            campaign.id,
            NewInvitation {
                joins_as: JoinRole::Player,
                lifetime_ms: 60_000,
                max_uses: 1,
                token_digest: spent.digest(tag),
            },
            world.now(),
        )
        .await
        .unwrap();
    let guest = site.person(2).await;
    world
        .store
        .accept_invitation(
            guest,
            spent.digest(tag),
            crate::fixture::operation_id(),
            world.now(),
        )
        .await
        .unwrap();
    world.clock.advance_ms(10);
    tokens.push(expired.text());
    tokens.push(revoked.text());
    tokens.push(spent.text());
    tokens.push("a".repeat(64));
    tokens.push("not a token".to_owned());
    let mut refusal: Option<Response> = None;
    for token in tokens {
        let response = site
            .send(
                Method::POST,
                "/join/preview",
                None,
                Some(&json!({"token": token})),
                &[],
            )
            .await;
        assert_eq!(
            response.body,
            json!({"reason": "invitation_unknown"}),
            "{}",
            response.text
        );
        assert!(!response.text.contains("Sunken") && !response.text.contains("Person"));
        if let Some(first) = &refusal {
            assert_eq!(response.status, first.status);
            assert_eq!(response.headers, first.headers);
            assert_eq!(response.text, first.text);
        } else {
            refusal = Some(response);
        }
    }
    // A full table: the player link of a two-seat table that is full.
    let full = df_campaign::secrets::SecretToken::generate().unwrap();
    world
        .store
        .create_invitation(
            host,
            campaign.id,
            NewInvitation {
                joins_as: JoinRole::Player,
                lifetime_ms: 60_000,
                max_uses: 3,
                token_digest: full.digest(tag),
            },
            world.now(),
        )
        .await
        .unwrap();
    let response = site
        .send(
            Method::POST,
            "/join/preview",
            None,
            Some(&json!({"token": full.text()})),
            &[],
        )
        .await;
    assert_eq!(response.body, json!({"reason": "full"}));
    site.cleanup().await;
}

#[tokio::test]
async fn a_throttled_guess_looks_like_an_unknown_one_and_the_layers_are_separate() {
    let site = Site::new("guessthrottle").await;
    let ip: IpAddr = "198.51.100.77".parse().unwrap();
    // Ten guesses from one address pass; the eleventh does not, and a neighbour in the same /24
    // still has its own tight allowance.
    for _ in 0..10 {
        assert!(site.service.probe_guess_gate(ip, None));
    }
    assert!(!site.service.probe_guess_gate(ip, None));
    let neighbour: IpAddr = "198.51.100.78".parse().unwrap();
    assert!(site.service.probe_guess_gate(neighbour, None));
    // The account layer is its own: a different address with a busy account is refused.
    let account = account_id(1);
    let mut passed = 0;
    for host in 1..=40_u8 {
        let other: IpAddr = format!("192.0.2.{host}").parse().unwrap();
        if site.service.probe_guess_gate(other, Some(account)) {
            passed += 1;
        }
    }
    assert_eq!(passed, 20, "twenty lookups per account per minute");
    // Over the routes the throttled answer is the unknown answer.
    let response = site
        .send(
            Method::POST,
            "/join/preview",
            None,
            Some(&json!({"token": "b".repeat(64)})),
            &[("x-test-ip", "198.51.100.77".into())],
        )
        .await;
    assert_eq!(response.body, json!({"reason": "invitation_unknown"}));
    site.cleanup().await;
}

#[tokio::test]
async fn a_room_code_flow_over_the_routes_always_ends_in_approval() {
    let site = Site::new("coderoutes").await;
    let table = table(&site).await;
    let campaign = id_text(table.campaign.as_bytes());
    let guest = site.person(20).await;
    let rotated = site
        .post(
            &format!("/campaigns/{campaign}/code/rotate"),
            table.host,
            &json!({}),
        )
        .await;
    assert_eq!(rotated.status, StatusCode::OK, "{}", rotated.text);
    let code = rotated.body["code"].as_str().unwrap().to_owned();
    assert_eq!(code.len(), 9);
    let lookup = site
        .post(
            "/join/code/lookup",
            guest,
            &json!({"code": code.to_lowercase()}),
        )
        .await;
    assert_eq!(lookup.status, StatusCode::OK, "{}", lookup.text);
    let request = site
        .post("/join/code/request", guest, &json!({"code": code}))
        .await;
    let request_id = request.body["request_id"].as_str().unwrap().to_owned();
    // Not a member until approved; a co-host without the bit cannot decide.
    assert_eq!(
        site.send(
            Method::GET,
            &format!("/campaigns/{campaign}/me"),
            Some(guest),
            None,
            &[]
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    let refused = site
        .post(
            &format!("/campaigns/{campaign}/requests/{request_id}/approve"),
            table.cohost_none,
            &json!({"role": "player"}),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    let approved = site
        .post(
            &format!("/campaigns/{campaign}/requests/{request_id}/approve"),
            table.cohost_all,
            &json!({"role": "contributor"}),
        )
        .await;
    assert_eq!(approved.status, StatusCode::OK, "{}", approved.text);
    let me = site
        .send(
            Method::GET,
            &format!("/campaigns/{campaign}/me"),
            Some(guest),
            None,
            &[],
        )
        .await;
    assert_eq!(me.body["role"], "contributor");
    site.cleanup().await;
}

// ---- Display over the routes -----------------------------------------------------------------

// ---- Service-level rules ---------------------------------------------------------------------

#[tokio::test]
async fn a_runtime_may_revoke_only_the_sessions_it_leases_and_the_control_plane_any() {
    use df_campaign::{
        errors::{Failure, Refusal},
        records::RevokeReason,
        store::RevocationCaller,
    };
    let site = Site::new("revokescope").await;
    let table = table(&site).await;
    let mine = session_id(1);
    let other = session_id(2);
    let runtime = RevocationCaller::Runtime {
        leased_sessions: vec![mine],
    };
    let refused = site
        .service
        .revoke_display_tickets(
            &runtime,
            other,
            table.campaign,
            RevokeReason::SessionEnded,
            None,
        )
        .await;
    assert_eq!(refused, Err(Failure::Refused(Refusal::NotAllowed)));
    assert_eq!(
        site.service
            .revoke_display_tickets(
                &runtime,
                mine,
                table.campaign,
                RevokeReason::SessionEnded,
                None
            )
            .await,
        Ok(0)
    );
    assert_eq!(
        site.service
            .revoke_display_tickets(
                &RevocationCaller::ControlPlane,
                other,
                table.campaign,
                RevokeReason::SessionEnded,
                None
            )
            .await,
        Ok(0)
    );
    site.cleanup().await;
}

#[tokio::test]
async fn pause_authority_is_authorize_over_the_member_snapshot_and_a_closing_account_has_none() {
    use df_campaign::{
        errors::{Failure, Refusal},
        roles::Authorized,
        service::CapabilityDecision,
    };
    let site = Site::new("pauseauth").await;
    let table = table(&site).await;
    let service = &site.service;
    let host = service
        .authorize_capability(table.host, table.campaign, Capability::StartOrPauseSession)
        .await
        .unwrap();
    assert!(matches!(host, CapabilityDecision::Allowed(_)));
    let player = service
        .authorize_capability(
            table.player,
            table.campaign,
            Capability::StartOrPauseSession,
        )
        .await
        .unwrap();
    assert!(
        matches!(
            player,
            CapabilityDecision::Limited(_, Authorized::RequestOnly)
        ),
        "a player requests a pause; it never pauses"
    );
    // The action helper treats the same limited grant as a refusal.
    assert_eq!(
        service
            .require_full_capability(
                table.player,
                table.campaign,
                Capability::StartOrPauseSession
            )
            .await
            .map(|_| ()),
        Err(Failure::Refused(Refusal::NotAllowed))
    );
    assert!(
        service
            .authorize_capability(
                table.cohost_none,
                table.campaign,
                Capability::StartOrPauseSession
            )
            .await
            .is_err()
    );
    assert_eq!(
        service
            .authorize_capability(
                table.outsider,
                table.campaign,
                Capability::StartOrPauseSession
            )
            .await
            .map(|_| ()),
        Err(Failure::Refused(Refusal::NotFound))
    );
    site.world
        .db
        .set_account_status(table.host, "closing")
        .await;
    assert_eq!(
        service
            .authorize_capability(table.host, table.campaign, Capability::StartOrPauseSession)
            .await
            .map(|_| ()),
        Err(Failure::Refused(Refusal::NotAllowed))
    );
    site.cleanup().await;
}

#[tokio::test]
async fn campaign_ids_are_validated_through_the_ports_at_creation() {
    let site = Site::new("portcreate").await;
    let verified = site.person(1).await;
    let unverified = site.world.account(2).await;
    site.accounts.add(
        unverified,
        "Unverified",
        false,
        Some(tenant_id()),
        AccountStatus::Active,
    );
    let no_tenant = site.world.account(3).await;
    site.accounts
        .add(no_tenant, "Tenantless", true, None, AccountStatus::Active);
    let unknown = account_id(99);
    assert_eq!(
        site.post("/campaigns", verified, &create_body("Ok"))
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        site.post("/campaigns", unverified, &create_body("No"))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        site.post("/campaigns", no_tenant, &create_body("No"))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        site.post("/campaigns", unknown, &create_body("No"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    // Seats and names are bounded.
    let mut too_many = create_body("Big");
    too_many["seat_count"] = json!(99);
    assert_eq!(
        site.post("/campaigns", verified, &too_many).await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        site.post("/campaigns", verified, &create_body(""))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    site.cleanup().await;
}

#[tokio::test]
async fn archive_and_delete_refuse_with_session_in_progress_only_past_the_lobby() {
    let site = Site::new("pastlobby").await;
    let table = table(&site).await;
    let campaign = id_text(table.campaign.as_bytes());
    let body = json!({"typed_name": "The Sunken Bell"});
    for (phase, expected) in [
        (SessionPhase::Playing, StatusCode::CONFLICT),
        (SessionPhase::PreparingFundingOpen, StatusCode::CONFLICT),
    ] {
        site.sessions.set(session_id(1), table.campaign, phase);
        let response = site
            .post(&format!("/campaigns/{campaign}/archive"), table.host, &body)
            .await;
        assert_eq!(response.status, expected);
        assert_eq!(response.body, json!({"reason": "session_in_progress"}));
    }
    // A lobby, or preparing without funding, does not block.
    site.sessions.set(
        session_id(1),
        table.campaign,
        SessionPhase::PreparingFundingClosed,
    );
    assert_eq!(
        site.post(&format!("/campaigns/{campaign}/archive"), table.host, &body)
            .await
            .status,
        StatusCode::OK
    );
    site.cleanup().await;
}
