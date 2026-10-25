//! A browser stand-in over the router: it keeps cookies and the CSRF token the way the app does,
//! and sends requests with the headers a same-origin `fetch` would, each of which a test can
//! change or drop.

use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use df_customer::{
    BuiltInCommonPasswords, CustomerConfig, CustomerService, FakeClock, HashingParameters,
    HostConfig, IdentitySource, LinkStyle, MailIdentity, MailPurpose, PublicOrigin, ServiceTasks,
    Tunables,
    contract::START_MS,
    fakes::{MemoryAccountStore, RecordingMail},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::{
    PortalOrigin,
    app_api::{AppApiState, fakes::FakePorts, router},
};

pub const ORIGIN: &str = "http://127.0.0.1:8099";
pub const PASSWORD: &str = "lantern under the old mill";
pub const OTHER_PASSWORD: &str = "the miller keeps a second key";

pub struct Harness {
    pub router: Router,
    pub service: CustomerService<MemoryAccountStore, RecordingMail>,
    pub mail: RecordingMail,
    pub ports: Arc<FakePorts>,
    pub clock: FakeClock,
    pub bundle: Bundle,
    next_client: AtomicU8,
    _tasks: ServiceTasks,
}

/// A bundle directory with the files a build produces, removed when dropped.
pub struct Bundle(pub PathBuf);

impl Bundle {
    fn create() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "df-app-api-bundle-{}",
            crate::app_api::fakes::unique_name()
        ));
        let assets = directory.join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(
            directory.join("index.html"),
            "<!doctype html><title>DungeonFlux</title><div id=\"app\"></div>",
        )
        .unwrap();
        std::fs::write(assets.join("account_app.js"), "export default 1;").unwrap();
        std::fs::write(assets.join("account_app_bg.wasm"), b"\0asm\x01\0\0\0").unwrap();
        std::fs::write(assets.join("app.css"), "body{margin:0}").unwrap();
        std::fs::write(assets.join("hero.webp"), b"RIFF").unwrap();
        std::fs::write(directory.join("secret.txt"), "outside the assets").unwrap();
        Self(directory)
    }
}

impl Drop for Bundle {
    fn drop(&mut self) {
        let _removed = std::fs::remove_dir_all(&self.0);
    }
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: String,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }

    pub fn code(&self) -> String {
        self.json()["code"].as_str().unwrap_or_default().to_owned()
    }

    pub fn set_cookies(&self) -> Vec<String> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok().map(str::to_owned))
            .collect()
    }

    pub fn set_cookie(&self, name: &str) -> Option<String> {
        self.set_cookies()
            .into_iter()
            .find(|cookie| cookie.starts_with(&format!("{name}=")))
    }
}

impl Harness {
    pub async fn new() -> Self {
        let mail = RecordingMail::default();
        let clock = FakeClock::at(START_MS);
        let hosts = HostConfig::new(
            PublicOrigin::parse(ORIGIN).unwrap(),
            MailIdentity::new(
                "DungeonFlux",
                "accounts@mail.example.test",
                "support@example.test",
            )
            .unwrap(),
            LinkStyle::App,
        );
        let (service, tasks) = CustomerService::start(
            CustomerConfig {
                hosts,
                identity: IdentitySource::local_passwords(),
                tunables: Tunables::default(),
                hashing: HashingParameters::TEST,
                maximum_concurrent_hashes: 4,
                hash_permit_wait: Duration::from_secs(5),
                provider_signup_floor: Duration::ZERO,
                dev_allow_local_passwords_on_https: false,
            },
            MemoryAccountStore::default(),
            mail.clone(),
            Arc::new(clock.clone()),
            Arc::new(BuiltInCommonPasswords),
        )
        .await
        .unwrap();
        let ports = Arc::new(FakePorts::new(ORIGIN, Arc::new(clock.clone())));
        let bundle = Bundle::create();
        let state = AppApiState::new(
            service.clone(),
            PortalOrigin::parse(ORIGIN).unwrap(),
            Arc::clone(&ports),
        );
        Self {
            router: router(state, bundle.0.clone()),
            service,
            mail,
            ports,
            clock,
            bundle,
            next_client: AtomicU8::new(1),
            _tasks: tasks,
        }
    }

    /// A fresh browser with its own client address, so per-address limits never cross tests' steps.
    pub fn browser(&self) -> Browser<'_> {
        let octet = self.next_client.fetch_add(1, Ordering::SeqCst);
        Browser {
            harness: self,
            cookies: BTreeMap::new(),
            csrf: String::new(),
            client: IpAddr::from([192, 0, 2, octet]),
        }
    }

    pub async fn send(&self, request: Request<Body>) -> Reply {
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            headers,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }

    /// The token in the newest mail of `purpose` to `to`, once `before` earlier mails exist.
    pub async fn mailed_token(&self, before: usize, to: &str, purpose: MailPurpose) -> String {
        self.mail.wait_for(before + 1).await;
        let message = self
            .mail
            .messages()
            .into_iter()
            .rev()
            .find(|message| message.to == to && message.purpose == purpose)
            .unwrap();
        let (_, rest) = message.body.split_once("#token=").unwrap();
        rest.chars().take_while(char::is_ascii_hexdigit).collect()
    }

    /// The whole link line of the newest mail of `purpose` to `to`.
    pub fn mailed_link(&self, to: &str, purpose: MailPurpose) -> String {
        let message = self
            .mail
            .messages()
            .into_iter()
            .rev()
            .find(|message| message.to == to && message.purpose == purpose)
            .unwrap();
        message
            .body
            .split_whitespace()
            .find(|word| word.starts_with("http"))
            .unwrap()
            .to_owned()
    }
}

/// What a request carries; the defaults are a same-origin `fetch` of a guarded mutation.
pub struct Req {
    method: Method,
    path: String,
    body: Option<String>,
    csrf: Option<Option<String>>,
    origin: Option<Option<String>>,
    fetch_site: Option<Option<String>>,
    content_type: Option<String>,
    device_header: bool,
    cookies: Option<Vec<(String, String)>>,
}

impl Req {
    pub fn new(method: Method, path: &str) -> Self {
        Self {
            method,
            path: path.to_owned(),
            body: None,
            csrf: None,
            origin: None,
            fetch_site: None,
            content_type: None,
            device_header: false,
            cookies: None,
        }
    }

    pub fn json(mut self, body: &Value) -> Self {
        self.body = Some(body.to_string());
        self
    }

    pub fn csrf(mut self, token: Option<&str>) -> Self {
        self.csrf = Some(token.map(str::to_owned));
        self
    }

    pub fn origin(mut self, origin: Option<&str>) -> Self {
        self.origin = Some(origin.map(str::to_owned));
        self
    }

    pub fn fetch_site(mut self, site: Option<&str>) -> Self {
        self.fetch_site = Some(site.map(str::to_owned));
        self
    }

    pub fn content_type(mut self, content_type: &str) -> Self {
        self.content_type = Some(content_type.to_owned());
        self
    }

    pub fn device_header(mut self) -> Self {
        self.device_header = true;
        self
    }

    /// Sends exactly these cookies instead of the browser's jar.
    pub fn only_cookies(mut self, cookies: &[(&str, &str)]) -> Self {
        self.cookies = Some(
            cookies
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        );
        self
    }
}

pub struct Browser<'h> {
    pub harness: &'h Harness,
    pub cookies: BTreeMap<String, String>,
    pub csrf: String,
    client: IpAddr,
}

impl Browser<'_> {
    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies.get(name).map(String::as_str)
    }

    fn absorb(&mut self, reply: &Reply) {
        for cookie in reply.set_cookies() {
            let Some((pair, _)) = cookie.split_once(';') else {
                continue;
            };
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            if value.is_empty() || cookie.contains("Max-Age=0") {
                self.cookies.remove(name);
            } else {
                self.cookies.insert(name.to_owned(), value.to_owned());
            }
        }
    }

    pub async fn send(&mut self, req: Req) -> Reply {
        let is_read = req.method == Method::GET;
        let mut builder = Request::builder().method(req.method).uri(&req.path);
        let cookies: Vec<(String, String)> = req.cookies.unwrap_or_else(|| {
            self.cookies
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect()
        });
        if !cookies.is_empty() {
            let line: Vec<String> = cookies
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect();
            builder = builder.header(header::COOKIE, line.join("; "));
        }
        if !is_read {
            let content_type = req.content_type.as_deref().unwrap_or("application/json");
            builder = builder.header(header::CONTENT_TYPE, content_type);
            match req.origin {
                None => builder = builder.header(header::ORIGIN, ORIGIN),
                Some(Some(origin)) => builder = builder.header(header::ORIGIN, origin),
                Some(None) => {}
            }
            match req.fetch_site {
                None => builder = builder.header("sec-fetch-site", "same-origin"),
                Some(Some(site)) => builder = builder.header("sec-fetch-site", site),
                Some(None) => {}
            }
            match req.csrf {
                None => builder = builder.header("x-df-csrf", &self.csrf),
                Some(Some(token)) => builder = builder.header("x-df-csrf", token),
                Some(None) => {}
            }
            if req.device_header {
                builder = builder.header("x-df-device", "1");
            }
        }
        let body = req
            .body
            .unwrap_or_else(|| if is_read { String::new() } else { "{}".into() });
        let mut request = builder.body(Body::from(body)).unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::new(self.client, 40_000)));
        let reply = self.harness.send(request).await;
        self.absorb(&reply);
        reply
    }

    pub async fn get(&mut self, path: &str) -> Reply {
        self.send(Req::new(Method::GET, path)).await
    }

    pub async fn post(&mut self, path: &str, body: &Value) -> Reply {
        self.send(Req::new(Method::POST, path).json(body)).await
    }

    /// Reads the session, remembering its CSRF token as the app does.
    pub async fn session(&mut self) -> Value {
        let reply = self.get("/app/api/session").await;
        let view = reply.json();
        self.csrf = view["csrf"].as_str().unwrap_or_default().to_owned();
        view
    }

    pub async fn operation(&mut self) -> String {
        let reply = self.get("/app/api/operation").await;
        reply.json()["operation"].as_str().unwrap().to_owned()
    }

    /// Signs up, verifies through the emailed link and signs in; returns the session view.
    pub async fn sign_up_verified(&mut self, email: &str) -> Value {
        self.session().await;
        let before = self.harness.mail.messages().len();
        let operation = self.operation().await;
        let reply = self
            .post(
                "/app/api/sign-up",
                &json!({"email": email, "password": PASSWORD, "is_adult": true,
                      "accepts_terms": true,
                      "terms_version": df_account_app::wire::CURRENT_TERMS_VERSION,
                      "operation": operation}),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        let token = self
            .harness
            .mailed_token(before, email, MailPurpose::VerifyEmail)
            .await;
        let reply = self
            .post(
                "/app/api/verify-email",
                &json!({"token": token, "password": PASSWORD, "confirmation": PASSWORD}),
            )
            .await;
        assert_eq!(reply.json(), json!("verified"));
        self.sign_in(email, PASSWORD).await
    }

    /// Signs in and adopts the new session's CSRF token.
    pub async fn sign_in(&mut self, email: &str, password: &str) -> Value {
        if self.csrf.is_empty() {
            self.session().await;
        }
        let reply = self
            .post(
                "/app/api/sign-in",
                &json!({"email": email, "password": password}),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        let view = reply.json();
        self.csrf = view["csrf"].as_str().unwrap().to_owned();
        view
    }

    pub async fn account_id(&self) -> df_types::AccountId {
        let token = self.cookie(crate::SESSION_COOKIE).unwrap();
        self.harness
            .service
            .authenticate(token)
            .await
            .unwrap()
            .unwrap()
            .account
            .id
    }

    /// A verified, signed-in account that holds every operator capability.
    pub async fn become_operator(&mut self, email: &str) {
        self.sign_up_verified(email).await;
        let id = self.account_id().await;
        self.harness.ports.operators.grant_all(id, true);
    }
}
