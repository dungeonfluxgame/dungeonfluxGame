//! The portal through its router, with the memory store and the recording mail sender: cookies,
//! CSRF, enumeration-safe answers, headers, escaping, limits and the full sign-up flow.

use std::{
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{HeaderMap, Request, StatusCode, header},
};
use df_customer::{
    BuiltInCommonPasswords, CustomerConfig, CustomerService, FakeClock, HashingParameters,
    HostConfig, IdentitySource, MailPurpose, ServiceTasks, Tunables,
    contract::{PASSWORD, START_MS},
    fakes::{MemoryAccountStore, RecordingMail},
};
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::{PortalOrigin, PortalState, router};

const ORIGIN: &str = "http://127.0.0.1:8099";
const EMAIL: &str = "robin@example.test";

struct Portal {
    router: Router,
    mail: RecordingMail,
    service: CustomerService<MemoryAccountStore, RecordingMail>,
    _tasks: ServiceTasks,
}

async fn portal() -> Portal {
    portal_with(IdentitySource::local_passwords()).await
}

async fn portal_with(identity: IdentitySource) -> Portal {
    let mail = RecordingMail::default();
    let (service, tasks) = CustomerService::start(
        CustomerConfig {
            hosts: HostConfig::local(ORIGIN).unwrap(),
            identity,
            tunables: Tunables::default(),
            hashing: HashingParameters::TEST,
            maximum_concurrent_hashes: 4,
            hash_permit_wait: Duration::from_secs(5),
            provider_signup_floor: Duration::ZERO,
            dev_allow_local_passwords_on_https: false,
        },
        MemoryAccountStore::default(),
        mail.clone(),
        Arc::new(FakeClock::at(START_MS)),
        Arc::new(BuiltInCommonPasswords),
    )
    .await
    .unwrap();
    let state = PortalState::new(service.clone(), PortalOrigin::parse(ORIGIN).unwrap());
    Portal {
        router: router(state),
        mail,
        service,
        _tasks: tasks,
    }
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            b' ' => "+".to_owned(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

fn form(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(key, value)| format!("{key}={}", encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

impl Portal {
    /// Sends the request as a server would: with the connection's peer address.
    async fn send(&self, request: Request<Body>) -> Reply {
        self.send_from(request, Some([203, 0, 113, 9])).await
    }

    async fn send_from(&self, mut request: Request<Body>, peer: Option<[u8; 4]>) -> Reply {
        if let Some(octets) = peer {
            request
                .extensions_mut()
                .insert(ConnectInfo(std::net::SocketAddr::from((octets, 40_000))));
        }
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            headers,
            body: String::from_utf8(bytes.to_vec()).unwrap(),
        }
    }

    async fn get(&self, path: &str, cookies: &str) -> Reply {
        self.send(
            Request::get(path)
                .header(header::COOKIE, cookies)
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    async fn post(&self, path: &str, cookies: &str, fields: &[(&str, &str)]) -> Reply {
        self.send(
            Request::post(path)
                .header(header::COOKIE, cookies)
                .header(header::ORIGIN, ORIGIN)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(form(fields)))
                .unwrap(),
        )
        .await
    }

    /// A pre-session cookie and the CSRF token (and operation, when present) of `path`'s form.
    async fn anonymous_form(&self, path: &str) -> (String, String, String) {
        let page = self.get(path, "").await;
        let cookie = set_cookie_value(&page.headers, crate::PRESESSION_COOKIE).unwrap();
        (
            format!("{}={cookie}", crate::PRESESSION_COOKIE),
            hidden(&page.body, "csrf").unwrap(),
            hidden(&page.body, "operation").unwrap_or_default(),
        )
    }

    async fn sign_up(&self, email: &str) -> Reply {
        let (cookie, csrf, operation) = self.anonymous_form("/signup").await;
        self.post(
            "/signup",
            &cookie,
            &[
                ("csrf", &csrf),
                ("operation", &operation),
                ("email", email),
                ("password", PASSWORD),
                ("adult_attested", "true"),
                ("terms_version", "terms-2026-10"),
            ],
        )
        .await
    }

    /// Signs up and returns the session cookie issued by email verification.
    async fn signed_in(&self, email: &str) -> String {
        let sent = self.mail.messages().len();
        assert_eq!(self.sign_up(email).await.status, StatusCode::OK);
        self.mail.wait_for(sent + 1).await;
        let token = self
            .mail
            .last_token(email, MailPurpose::VerifyEmail)
            .unwrap();
        let (cookie, csrf, _) = self
            .anonymous_form(&format!("/verify-email?token={token}"))
            .await;
        let verified = self
            .post(
                "/verify-email",
                &cookie,
                &[
                    ("csrf", &csrf),
                    ("token", &token),
                    ("password", PASSWORD),
                    ("confirmation", PASSWORD),
                ],
            )
            .await;
        assert_eq!(verified.status, StatusCode::SEE_OTHER);
        assert_eq!(verified.headers.get("location").unwrap(), "/app/dashboard");
        let session = set_cookie_value(&verified.headers, crate::SESSION_COOKIE)
            .filter(|value| !value.is_empty())
            .expect("verification issues a session cookie");
        format!("{}={session}", crate::SESSION_COOKIE)
    }

    async fn log_in(&self, email: &str, password: &str) -> Option<String> {
        let (cookie, csrf, _) = self.anonymous_form("/login").await;
        let reply = self
            .post(
                "/login",
                &cookie,
                &[("csrf", &csrf), ("email", email), ("password", password)],
            )
            .await;
        set_cookie_value(&reply.headers, crate::SESSION_COOKIE)
            .filter(|value| !value.is_empty())
            .map(|value| format!("{}={value}", crate::SESSION_COOKIE))
    }
}

fn set_cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|value| {
            let (pair, _) = value.split_once(';')?;
            let (key, value) = pair.split_once('=')?;
            (key == name).then(|| value.to_owned())
        })
}

fn set_cookie_header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(&format!("{name}=")))
        .map(str::to_owned)
}

fn hidden(body: &str, name: &str) -> Option<String> {
    let marker = format!("name=\"{name}\" value=\"");
    let start = body.find(&marker)? + marker.len();
    Some(
        body[start..]
            .chars()
            .take_while(|character| *character != '"')
            .collect(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn signing_up_verifying_and_logging_in_and_out_works_through_the_pages() {
    let portal = portal().await;
    let session = portal.signed_in(EMAIL).await;
    let account = portal.get("/account", &session).await;
    assert_eq!(account.status, StatusCode::OK);
    assert!(account.body.contains(EMAIL));
    let csrf = hidden(&account.body, "csrf").unwrap();
    let logout = portal.post("/logout", &session, &[("csrf", &csrf)]).await;
    assert_eq!(logout.status, StatusCode::SEE_OTHER);
    let after = portal.get("/account", &session).await;
    assert_eq!(after.status, StatusCode::SEE_OTHER);
    assert_eq!(after.headers[header::LOCATION], "/login");
}

#[tokio::test(flavor = "multi_thread")]
async fn cookies_are_host_only_secure_http_only_and_lax() {
    let portal = portal().await;
    let page = portal.get("/login", "").await;
    let presession = set_cookie_header(&page.headers, crate::PRESESSION_COOKIE).unwrap();
    assert!(presession.ends_with("; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=3600"));
    // Host-only everywhere (S2 critic): no Domain, so staging can never receive production's.
    assert!(!presession.contains("Domain"));
    portal.sign_up(EMAIL).await;
    let (cookie, csrf, _) = portal.anonymous_form("/login").await;
    let reply = portal
        .post(
            "/login",
            &cookie,
            &[("csrf", &csrf), ("email", EMAIL), ("password", PASSWORD)],
        )
        .await;
    let session = set_cookie_header(&reply.headers, crate::SESSION_COOKIE).unwrap();
    assert!(session.ends_with(&format!(
        "; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={}",
        30 * 24 * 60 * 60
    )));
    assert!(!session.contains("Domain"));
    let cleared = set_cookie_header(&reply.headers, crate::PRESESSION_COOKIE).unwrap();
    assert!(
        cleared.contains("Max-Age=0"),
        "the pre-session is spent at login"
    );
    assert!(!cleared.contains("Domain"));
    assert!(cleared.starts_with("__Host-"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_post_without_the_right_token_or_from_another_site_is_refused() {
    let portal = portal().await;
    let (cookie, csrf, _) = portal.anonymous_form("/login").await;
    let signup_token = portal.anonymous_form("/signup").await.1;
    let zeros = "00".repeat(32);
    for (token, case) in [
        ("", "a missing token"),
        (zeros.as_str(), "a forged token"),
        (signup_token.as_str(), "a token for another form"),
    ] {
        let reply = portal
            .post(
                "/login",
                &cookie,
                &[("csrf", token), ("email", EMAIL), ("password", PASSWORD)],
            )
            .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{case}");
    }
    let cross_site = portal
        .send(
            Request::post("/login")
                .header(header::COOKIE, &cookie)
                .header("sec-fetch-site", "cross-site")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(form(&[
                    ("csrf", &csrf),
                    ("email", EMAIL),
                    ("password", PASSWORD),
                ])))
                .unwrap(),
        )
        .await;
    assert_eq!(cross_site.status, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cookie_set_before_login_is_never_promoted_to_a_session() {
    let portal = portal().await;
    portal.sign_up(EMAIL).await;
    let (cookie, csrf, _) = portal.anonymous_form("/login").await;
    let planted = cookie.replace(crate::PRESESSION_COOKIE, crate::SESSION_COOKIE);
    let reply = portal
        .post(
            "/login",
            &format!("{cookie}; {planted}"),
            &[("csrf", &csrf), ("email", EMAIL), ("password", PASSWORD)],
        )
        .await;
    let issued = set_cookie_value(&reply.headers, crate::SESSION_COOKIE).unwrap();
    assert_ne!(format!("{}={issued}", crate::SESSION_COOKIE), planted);
    assert_eq!(
        portal.get("/account", &planted).await.status,
        StatusCode::SEE_OTHER
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn signup_for_a_new_and_a_registered_address_returns_the_same_page() {
    let fresh = portal().await;
    let new = fresh.sign_up(EMAIL).await;
    let registered = portal().await;
    registered.sign_up(EMAIL).await;
    let again = registered.sign_up(EMAIL).await;
    assert_eq!(new.status, again.status);
    assert_eq!(new.body, again.body);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reset_request_for_a_known_and_an_unknown_address_returns_the_same_page() {
    let portal = portal().await;
    portal.signed_in(EMAIL).await;
    let (cookie, csrf, _) = portal.anonymous_form("/password-reset").await;
    let known = portal
        .post(
            "/password-reset",
            &cookie,
            &[("csrf", &csrf), ("email", EMAIL)],
        )
        .await;
    let unknown = portal
        .post(
            "/password-reset",
            &cookie,
            &[("csrf", &csrf), ("email", "nobody@example.test")],
        )
        .await;
    assert_eq!(known.status, unknown.status);
    assert_eq!(known.body, unknown.body);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_login_with_an_unknown_address_and_with_a_wrong_password_look_the_same() {
    let unknown_portal = portal().await;
    let known_portal = portal().await;
    known_portal.sign_up(EMAIL).await;
    let (cookie, csrf, _) = unknown_portal.anonymous_form("/login").await;
    let fields = [
        ("csrf", csrf.as_str()),
        ("email", EMAIL),
        ("password", "not the password"),
    ];
    let unknown = unknown_portal.post("/login", &cookie, &fields).await;
    let before = known_portal.service.verification_count();
    // The pre-session cookie is the CSRF key, so the same cookie works on both portals.
    let wrong = known_portal.post("/login", &cookie, &fields).await;
    assert_eq!(unknown.status, wrong.status);
    assert_eq!(unknown.body, wrong.body);
    assert_eq!(known_portal.service.verification_count(), before + 1);
    let unknown_before = unknown_portal.service.verification_count();
    unknown_portal.post("/login", &cookie, &fields).await;
    assert_eq!(
        unknown_portal.service.verification_count(),
        unknown_before + 1,
        "an unknown address still costs one verification"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn every_page_carries_the_security_headers_and_account_pages_are_not_cached() {
    let portal = portal().await;
    let session = portal.signed_in(EMAIL).await;
    for (path, cookies) in [("/", ""), ("/login", ""), ("/account", session.as_str())] {
        let reply = portal.get(path, cookies).await;
        let policy = reply.headers[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap();
        assert!(policy.contains("default-src 'none'") && policy.contains("frame-ancestors 'none'"));
        assert!(!policy.contains("script-src"), "no script is ever allowed");
        assert_eq!(reply.headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(reply.headers[header::CACHE_CONTROL], "no-store");
    }
    let css = portal.get("/portal/portal.css", "").await;
    assert_eq!(css.headers[header::CACHE_CONTROL], "public, max-age=3600");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hostile_display_name_is_shown_as_text() {
    let portal = portal().await;
    let session = portal.signed_in(EMAIL).await;
    let profile = portal.get("/account/profile", &session).await;
    let csrf = hidden(&profile.body, "csrf").unwrap();
    portal
        .post(
            "/account/profile",
            &session,
            &[
                ("csrf", &csrf),
                ("display_name", "<img src=x onerror=alert(1)>"),
            ],
        )
        .await;
    let account = portal.get("/account", &session).await;
    assert!(account.body.contains("&lt;img src=x onerror=alert(1)&gt;"));
    assert!(!account.body.contains("<img src=x"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_form_body_over_sixteen_kibibytes_is_refused() {
    let portal = portal().await;
    let (cookie, csrf, _) = portal.anonymous_form("/login").await;
    let huge = "x".repeat(crate::MAXIMUM_FORM_BYTES);
    let reply = portal
        .post(
            "/login",
            &cookie,
            &[("csrf", &csrf), ("email", &huge), ("password", PASSWORD)],
        )
        .await;
    assert_eq!(reply.status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_password_change_through_the_page_ends_the_old_cookie_and_sets_a_new_one() {
    let portal = portal().await;
    let session = portal.signed_in(EMAIL).await;
    let security = portal.get("/account/security", &session).await;
    let csrf = hidden(&security.body, "csrf").unwrap();
    let new_password = "the miller keeps a second key";
    let reply = portal
        .post(
            "/account/security/password",
            &session,
            &[
                ("csrf", &csrf),
                ("current", PASSWORD),
                ("password", new_password),
                ("confirmation", new_password),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    let replacement = set_cookie_value(&reply.headers, crate::SESSION_COOKIE).unwrap();
    assert_eq!(
        portal.get("/account", &session).await.status,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        portal
            .get(
                "/account",
                &format!("{}={replacement}", crate::SESSION_COOKIE)
            )
            .await
            .status,
        StatusCode::OK
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn one_account_cannot_sign_out_another_accounts_device_through_the_form() {
    let portal = portal().await;
    let robin = portal.signed_in(EMAIL).await;
    let marian = portal.signed_in("marian@example.test").await;
    let marian_page = portal.get("/account/sessions", &marian).await;
    assert!(marian_page.body.contains("This device"));
    let marian_session = portal
        .service
        .authenticate(marian.split_once('=').unwrap().1)
        .await
        .unwrap()
        .unwrap()
        .session;
    // A second device gives Robin's page a per-device form, whose token is the page's first.
    portal.log_in(EMAIL, PASSWORD).await.unwrap();
    let robin_page = portal.get("/account/sessions", &robin).await;
    let csrf = hidden(&robin_page.body, "csrf").unwrap();
    let reply = portal
        .post(
            "/account/sessions/revoke",
            &robin,
            &[
                ("csrf", &csrf),
                ("session", &crate::html::hex(marian_session.as_bytes())),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(portal.get("/account", &marian).await.status, StatusCode::OK);
}

fn captured() -> &'static Mutex<Vec<String>> {
    static LINES: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    LINES.get_or_init(|| Mutex::new(Vec::new()))
}

fn capture(_event: &'static str, line: String) {
    captured().lock().unwrap().push(line);
}

#[tokio::test(flavor = "multi_thread")]
async fn audit_lines_from_the_whole_flow_hold_no_token_address_or_password() {
    df_observe::install_audit_sink(capture);
    let portal = portal().await;
    let session = portal.signed_in(EMAIL).await;
    let (cookie, csrf, _) = portal.anonymous_form("/password-reset").await;
    portal
        .post(
            "/password-reset",
            &cookie,
            &[("csrf", &csrf), ("email", EMAIL)],
        )
        .await;
    portal.mail.wait_for(2).await;
    let reset = portal
        .mail
        .last_token(EMAIL, MailPurpose::PasswordReset)
        .unwrap();
    portal
        .get(&format!("/password-reset/confirm?token={reset}"), &cookie)
        .await;
    let lines = captured().lock().unwrap().clone();
    assert!(!lines.is_empty());
    let session_token = session.split_once('=').unwrap().1;
    for line in &lines {
        assert!(!line.contains(EMAIL), "{line}");
        assert!(!line.contains(PASSWORD), "{line}");
        assert!(!line.contains(session_token), "{line}");
        assert!(!line.contains(&reset), "{line}");
        assert!(!line.contains("token="), "{line}");
    }
}

async fn provider_portal() -> (Portal, df_customer::fakes::FakeIdentityProvider) {
    let provider = df_customer::fakes::FakeIdentityProvider::default();
    let portal = portal_with(IdentitySource::provider(provider.clone())).await;
    (portal, provider)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_pages_sign_a_person_up_and_in_when_the_identity_provider_holds_the_password() {
    let (portal, provider) = provider_portal().await;
    let session = portal.signed_in(EMAIL).await;
    assert!(provider.is_verified(EMAIL));
    assert_eq!(provider.open_sessions(), 0);
    let account = portal.get("/account", &session).await;
    assert_eq!(account.status, StatusCode::OK);
    assert!(account.body.contains(EMAIL));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_provider_that_wants_its_own_verification_is_a_plain_refusal_on_the_login_page() {
    let (portal, provider) = provider_portal().await;
    provider.require_verified(true);
    assert_eq!(portal.sign_up(EMAIL).await.status, StatusCode::OK);
    let (cookie, csrf, _) = portal.anonymous_form("/login").await;
    let reply = portal
        .post(
            "/login",
            &cookie,
            &[("csrf", &csrf), ("email", EMAIL), ("password", PASSWORD)],
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(reply.body.contains("Email or password is incorrect."));
    assert!(set_cookie_value(&reply.headers, crate::SESSION_COOKIE).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_security_page_has_no_password_forms_when_the_provider_holds_the_password() {
    let (portal, _provider) = provider_portal().await;
    let session = portal.signed_in(EMAIL).await;
    let security = portal.get("/account/security", &session).await;
    assert_eq!(security.status, StatusCode::OK);
    assert!(security.body.contains("managed by our sign-in service"));
    assert!(!security.body.contains("/account/security/password"));
    assert!(!security.body.contains("/account/security/email"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_composed_router_serves_the_app_shell_at_join_with_no_redirect_beside_the_portal() {
    use crate::app_api::fakes::FakePorts;

    let portal = portal().await;
    let directory = std::env::temp_dir().join(format!(
        "df-portal-bundle-{}",
        crate::app_api::fakes::unique_name()
    ));
    std::fs::create_dir_all(directory.join("assets")).unwrap();
    std::fs::write(
        directory.join("index.html"),
        "<!doctype html><title>DungeonFlux</title><div id=\"app\"></div>",
    )
    .unwrap();
    let ports = Arc::new(FakePorts::new(ORIGIN, Arc::new(FakeClock::at(START_MS))));
    let state = PortalState::new(portal.service.clone(), PortalOrigin::parse(ORIGIN).unwrap());
    let mut composed = portal;
    composed.router = crate::router_with_app(state, ports, directory.clone());
    for path in ["/app/join", "/app/join/", "/app/join?x=1", "/app"] {
        let reply = composed.get(path, "").await;
        assert_eq!(reply.status, StatusCode::OK, "{path}");
        assert!(reply.headers.get(header::LOCATION).is_none(), "{path}");
        assert!(reply.body.contains("<div id=\"app\">"), "{path}");
    }
    let shell_policy = composed.get("/app/join", "").await.headers[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(shell_policy.contains("script-src 'self'"));
    let login = composed.get("/login", "").await;
    assert_eq!(login.status, StatusCode::OK);
    assert!(
        !login.headers[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("script-src"),
        "the portal's own pages keep the no-script policy"
    );
    let _removed = std::fs::remove_dir_all(directory);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_with_no_peer_address_is_refused_and_nothing_is_counted() {
    let portal = portal().await;
    let blind = portal
        .send_from(Request::get("/login").body(Body::empty()).unwrap(), None)
        .await;
    assert_eq!(
        blind.status,
        StatusCode::OK,
        "a page that needs no address still renders"
    );
    let (cookie, csrf, _) = portal.anonymous_form("/login").await;
    let body = form(&[("csrf", &csrf), ("email", EMAIL), ("password", PASSWORD)]);
    let refused = portal
        .send_from(
            Request::post("/login")
                .header(header::COOKIE, &cookie)
                .header(header::ORIGIN, ORIGIN)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
            None,
        )
        .await;
    assert_eq!(refused.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(set_cookie_value(&refused.headers, crate::SESSION_COOKIE).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn forwarding_headers_count_only_from_a_configured_proxy_network() {
    let portal = portal().await;
    let proxies = crate::TrustedProxies::parse(&["10.0.0.0/8"]).unwrap();
    let state = PortalState::new(portal.service.clone(), PortalOrigin::parse(ORIGIN).unwrap())
        .with_trusted_proxies(proxies);
    let mut behind = portal;
    behind.router = router(state);
    let (cookie, csrf, _) = behind.anonymous_form("/login").await;
    let login = |forwarded: Option<&'static str>| {
        let mut request = Request::post("/login")
            .header(header::COOKIE, cookie.clone())
            .header(header::ORIGIN, ORIGIN)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        if let Some(forwarded) = forwarded {
            request = request.header("x-forwarded-for", forwarded);
        }
        request
            .body(Body::from(form(&[
                ("csrf", &csrf),
                ("email", EMAIL),
                ("password", PASSWORD),
            ])))
            .unwrap()
    };
    // A proxy that forwards nothing cannot say who the client is: refused.
    assert_eq!(
        behind
            .send_from(login(None), Some([10, 0, 0, 1]))
            .await
            .status,
        StatusCode::SERVICE_UNAVAILABLE
    );
    // A proxy that forwards a client: served.
    assert_ne!(
        behind
            .send_from(login(Some("198.51.100.7")), Some([10, 0, 0, 1]))
            .await
            .status,
        StatusCode::SERVICE_UNAVAILABLE
    );
    // The same header from a stranger is ignored, not believed and not refused.
    assert_ne!(
        behind
            .send_from(login(Some("not an address")), Some([203, 0, 113, 50]))
            .await
            .status,
        StatusCode::SERVICE_UNAVAILABLE
    );
}
