//! An in-memory WorkOS for tests: the same endpoints, request bodies and answer shapes the adapter
//! speaks, with the behaviors that matter. Addresses are trimmed, lowercased and unique. A wrong
//! password and an unknown address answer alike. An unverified address is refused when
//! verification is required (the dashboard setting). Reset tokens are single use, end the user's
//! sessions and verify the address. Every authentication opens a session whose id is the `sid`
//! claim of the access token, and stays open until revoked. A wrong API key is a 401.
//!
//! `fail_next` makes the next request fail at the transport; `lose_next_response` applies the
//! next request and then loses the answer, as a timeout after WorkOS acted would.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};

use crate::transport::{HttpRequest, HttpResponse, HttpTransport, Method, TransportError};

struct User {
    id: String,
    email: String,
    password: String,
    verified: bool,
}

struct State {
    users: Vec<User>,
    next: u64,
    reset_tokens: Vec<(String, String)>,
    sessions: Vec<(String, String)>,
    require_verified: bool,
    weak_passwords: Vec<String>,
    reset_url: String,
    fail_next: Option<TransportError>,
    lose_next_response: bool,
    canned: Option<(u16, String)>,
    log: Vec<(Method, String)>,
}

/// The fake WorkOS. Clones share one state.
#[derive(Clone)]
pub struct FakeWorkOsServer {
    api_key: String,
    state: Arc<Mutex<State>>,
}

fn response(status: u16, body: &Value) -> HttpResponse {
    HttpResponse {
        status,
        body: body.to_string().into_bytes(),
    }
}

fn error(status: u16, code: &str, nested: Option<&str>) -> HttpResponse {
    let mut body = json!({"code": code, "message": "fake WorkOS error"});
    if let Some(nested) = nested {
        body["errors"] = json!([{"code": nested, "message": "fake WorkOS error"}]);
    }
    response(status, &body)
}

fn user_json(user: &User) -> Value {
    json!({
        "object": "user",
        "id": user.id,
        "email": user.email,
        "email_verified": user.verified,
        "first_name": null,
        "last_name": null,
        "created_at": "2026-10-25T00:00:00.000Z",
        "updated_at": "2026-10-25T00:00:00.000Z",
    })
}

impl FakeWorkOsServer {
    /// A server that accepts `api_key`, its reset links pointing at `reset_url`.
    pub fn new(api_key: &str, reset_url: &str) -> Self {
        Self {
            api_key: api_key.to_owned(),
            state: Arc::new(Mutex::new(State {
                users: Vec::new(),
                next: 0,
                reset_tokens: Vec::new(),
                sessions: Vec::new(),
                require_verified: false,
                weak_passwords: Vec::new(),
                reset_url: reset_url.to_owned(),
                fail_next: None,
                lose_next_response: false,
                canned: None,
                log: Vec::new(),
            })),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn require_verified(&self, required: bool) {
        self.state().require_verified = required;
    }

    pub fn reject_password(&self, password: &str) {
        self.state().weak_passwords.push(password.to_owned());
    }

    pub fn fail_next(&self, failure: TransportError) {
        self.state().fail_next = Some(failure);
    }

    pub fn lose_next_response(&self) {
        self.state().lose_next_response = true;
    }

    /// The next request is answered `status` with error code `code` and changes nothing: how an
    /// answer the adapter does not recognize is simulated.
    pub fn answer_next(&self, status: u16, code: &str) {
        self.state().canned = Some((status, code.to_owned()));
    }

    pub fn add_user(&self, email: &str, password: &str, verified: bool) -> String {
        let mut state = self.state();
        state.next += 1;
        let id = format!("user_{:06}", state.next);
        state.users.push(User {
            id: id.clone(),
            email: email.trim().to_ascii_lowercase(),
            password: password.to_owned(),
            verified,
        });
        id
    }

    pub fn has_user(&self, email: &str) -> bool {
        let email = email.trim().to_ascii_lowercase();
        self.state().users.iter().any(|user| user.email == email)
    }

    pub fn is_verified(&self, email: &str) -> bool {
        let email = email.trim().to_ascii_lowercase();
        self.state()
            .users
            .iter()
            .any(|user| user.email == email && user.verified)
    }

    /// Provider sessions created and not yet revoked.
    pub fn open_sessions(&self) -> usize {
        self.state().sessions.len()
    }

    /// The method and path of every request so far (never bodies: they hold passwords).
    pub fn requests(&self) -> Vec<(Method, String)> {
        self.state().log.clone()
    }

    fn handle(&self, request: &HttpRequest) -> HttpResponse {
        let mut guard = self.state();
        let state = &mut *guard;
        let (path, query) = request
            .path
            .split_once('?')
            .unwrap_or((request.path.as_str(), ""));
        let body = request.body.clone().unwrap_or(Value::Null);
        let text = |key: &str| {
            body.get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let authorized = if path == "/user_management/authenticate" {
            text("client_secret") == self.api_key
        } else {
            request.bearer.as_deref() == Some(self.api_key.as_str())
        };
        if !authorized {
            return error(401, "unauthorized", None);
        }
        let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        match (request.method, segments.as_slice()) {
            (Method::Post, ["user_management", "users"]) => {
                let email = text("email").trim().to_ascii_lowercase();
                let password = text("password");
                if email.is_empty() || !email.contains('@') {
                    return error(422, "invalid_request_parameters", Some("invalid_email"));
                }
                if state.users.iter().any(|user| user.email == email) {
                    return error(422, "user_creation_error", Some("email_not_available"));
                }
                if password.len() < 10 || state.weak_passwords.contains(&password) {
                    return error(422, "user_creation_error", Some("password_too_weak"));
                }
                state.next += 1;
                let user = User {
                    id: format!("user_{:06}", state.next),
                    email,
                    password,
                    verified: body.get("email_verified").and_then(Value::as_bool) == Some(true),
                };
                let view = user_json(&user);
                state.users.push(user);
                response(201, &view)
            }
            (Method::Post, ["user_management", "authenticate"]) => {
                if text("grant_type") != "password" {
                    return error(400, "invalid_grant", None);
                }
                let email = text("email").trim().to_ascii_lowercase();
                let password = text("password");
                let Some(user) = state
                    .users
                    .iter()
                    .find(|user| user.email == email && user.password == password)
                else {
                    return error(400, "invalid_credentials", None);
                };
                if !user.verified && state.require_verified {
                    return response(
                        403,
                        &json!({
                            "code": "email_verification_required",
                            "message": "Email ownership must be verified before authentication.",
                            "pending_authentication_token": "pending_fake",
                            "email": user.email,
                            "email_verification_id": "email_verification_fake",
                        }),
                    );
                }
                let view = user_json(user);
                let session = format!("session_{:06}", state.sessions.len() + state.log.len());
                state.sessions.push((user.id.clone(), session.clone()));
                let claims = json!({"sid": session, "sub": user.id});
                let token = format!(
                    "{}.{}.fake",
                    URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#),
                    URL_SAFE_NO_PAD.encode(claims.to_string())
                );
                response(
                    200,
                    &json!({
                        "user": view,
                        "organization_id": null,
                        "access_token": token,
                        "refresh_token": "refresh_fake",
                        "authentication_method": "Password",
                    }),
                )
            }
            (Method::Get, ["user_management", "users"]) => {
                let wanted = query
                    .split('&')
                    .find_map(|pair| pair.strip_prefix("email="))
                    .unwrap_or("")
                    .replace("%40", "@")
                    .to_ascii_lowercase();
                let data: Vec<Value> = state
                    .users
                    .iter()
                    .filter(|user| wanted.is_empty() || user.email == wanted)
                    .map(user_json)
                    .collect();
                response(
                    200,
                    &json!({"object": "list", "data": data, "list_metadata": {}}),
                )
            }
            (Method::Put, ["user_management", "users", id]) => {
                let weak = |password: &str| {
                    password.len() < 10 || state.weak_passwords.iter().any(|w| w == password)
                };
                let new_password = body
                    .get("password")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if new_password.as_deref().is_some_and(weak) {
                    return error(422, "user_update_error", Some("password_too_weak"));
                }
                let owner = id.to_string();
                let Some(user) = state.users.iter_mut().find(|user| user.id == *id) else {
                    return error(404, "entity_not_found", None);
                };
                if let Some(password) = new_password {
                    user.password = password;
                    state
                        .sessions
                        .retain(|(session_owner, _)| *session_owner != owner);
                }
                if let Some(verified) = body.get("email_verified").and_then(Value::as_bool) {
                    user.verified = verified;
                }
                response(200, &user_json(user))
            }
            (Method::Delete, ["user_management", "users", id]) => {
                let before = state.users.len();
                state.users.retain(|user| user.id != *id);
                state.sessions.retain(|(owner, _)| owner.as_str() != *id);
                state
                    .reset_tokens
                    .retain(|(_, owner)| owner.as_str() != *id);
                if state.users.len() == before {
                    error(404, "entity_not_found", None)
                } else {
                    HttpResponse {
                        status: 204,
                        body: Vec::new(),
                    }
                }
            }
            (Method::Post, ["user_management", "password_reset"]) => {
                let email = text("email").trim().to_ascii_lowercase();
                let Some(user) = state.users.iter().find(|user| user.email == email) else {
                    return error(404, "entity_not_found", None);
                };
                let owner = user.id.clone();
                let address = user.email.clone();
                state.next += 1;
                let token = format!("reset_token_{:06}", state.next);
                state.reset_tokens.push((token.clone(), owner.clone()));
                response(
                    201,
                    &json!({
                        "object": "password_reset",
                        "id": format!("password_reset_{:06}", state.next),
                        "user_id": owner,
                        "email": address,
                        "password_reset_token": token,
                        "password_reset_url": format!("{}?token={token}", state.reset_url),
                        "expires_at": "2026-10-25T01:00:00.000Z",
                        "created_at": "2026-10-25T00:00:00.000Z",
                    }),
                )
            }
            (Method::Post, ["user_management", "password_reset", "confirm"]) => {
                let token = text("token");
                let new_password = text("new_password");
                let Some(position) = state
                    .reset_tokens
                    .iter()
                    .position(|(known, _)| *known == token)
                else {
                    return error(404, "entity_not_found", None);
                };
                if new_password.len() < 10 || state.weak_passwords.contains(&new_password) {
                    return error(422, "password_reset_error", Some("password_too_weak"));
                }
                let (_, owner) = state.reset_tokens.remove(position);
                state
                    .sessions
                    .retain(|(session_owner, _)| *session_owner != owner);
                let Some(user) = state.users.iter_mut().find(|user| user.id == owner) else {
                    return error(404, "entity_not_found", None);
                };
                user.password = new_password;
                user.verified = true;
                response(200, &json!({"user": user_json(user)}))
            }
            (Method::Post, ["user_management", "sessions", "revoke"]) => {
                let id = text("session_id");
                state.sessions.retain(|(_, session)| *session != id);
                response(200, &json!({}))
            }
            _ => error(404, "not_found", None),
        }
    }
}

impl HttpTransport for FakeWorkOsServer {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let lose = {
            let mut state = self.state();
            state.log.push((request.method, request.path.clone()));
            if let Some(failure) = state.fail_next.take() {
                return Err(failure);
            }
            if let Some((status, code)) = state.canned.take() {
                return Ok(error(status, &code, None));
            }
            std::mem::take(&mut state.lose_next_response)
        };
        let answer = self.handle(&request);
        if lose {
            return Err(TransportError::Unknown);
        }
        Ok(answer)
    }
}
