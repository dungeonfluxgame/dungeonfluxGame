//! `IdentityProvider` over WorkOS's User Management API, called from our server (launch
//! decision 28). Endpoints used, all documented at workos.com/docs/reference/authkit:
//!
//! | Port call | WorkOS call |
//! | --- | --- |
//! | `create_user` | `POST /user_management/users` (unverified address, password) |
//! | `authenticate` | `POST /user_management/authenticate`, `grant_type=password` |
//! | `find_user` | `GET /user_management/users?email=` |
//! | `mark_email_verified` | `PUT /user_management/users/{id}` with `email_verified` |
//! | `begin_password_reset` | `POST /user_management/password_reset` |
//! | `finish_password_reset` | `POST /user_management/password_reset/confirm` |
//! | `end_session` | `POST /user_management/sessions/revoke` |
//! | `delete_user` | `DELETE /user_management/users/{id}` |
//!
//! Open points, to check against the WorkOS sandbox before launch (the published reference does
//! not show them, so the fake and this adapter share one reading): the error `code` strings for a
//! taken address, a weak password, a bad credential and a spent reset token; that a failed password
//! check answers before the verification requirement; and whether WorkOS also emails its own reset
//! or verification message (the deployment turns its email verification off, because the links in
//! our emails are the only ones we want people to follow).
//!
//! Unrecognized answers fail closed and never say which accounts exist: an unrecognized 4xx at
//! sign-in is a refused credential; at sign-up it asks the provider whether the address already
//! has a user (taken) and otherwise reads as a refused password; at a reset it is a spent token.
//! `Unavailable` is only for 429, 5xx, a rejected API key and the named sign-in steps this
//! server does not support (multi-factor, organization choice, single sign-on, Radar). Every
//! unrecognized answer writes one audit line with its status and (sanitized) code.
//!
//! Provider tokens are never kept: the session `authenticate` creates is ended by the service
//! right away (`end_session`), and the access and refresh tokens are dropped here.

use std::fmt;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use df_customer::{
    Authentication, CallerContext, CreateUser, EmailAddress, HostConfig, IdentityFailure,
    IdentityProvider, IdentityProviderKind, ProviderUser, ResetFinish, ResetStart, SetPassword,
};
use serde_json::{Value, json};

use crate::transport::{HttpRequest, HttpResponse, HttpTransport, Method, TransportError};

/// A WorkOS API key. Its `Debug` output is redacted.
#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn new(key: String) -> Self {
        Self(key)
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ApiKey(redacted)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigRefusal {
    ClientId,
    ApiKey,
}

/// Everything the adapter needs from deployment configuration.
#[derive(Clone, Debug)]
pub struct WorkOsConfig {
    client_id: String,
    api_key: ApiKey,
    redirect_allowlist: Vec<String>,
}

fn plain(text: &str) -> bool {
    !text.is_empty() && !text.chars().any(|c| c.is_whitespace() || c.is_control())
}

impl WorkOsConfig {
    /// `hosts` supplies the redirect allowlist (the URLs to register in the WorkOS dashboard).
    pub fn new(
        client_id: &str,
        api_key: String,
        hosts: &HostConfig,
    ) -> Result<Self, ConfigRefusal> {
        if !plain(client_id) {
            return Err(ConfigRefusal::ClientId);
        }
        if !plain(&api_key) {
            return Err(ConfigRefusal::ApiKey);
        }
        Ok(Self {
            client_id: client_id.to_owned(),
            api_key: ApiKey(api_key),
            redirect_allowlist: hosts.identity_redirect_allowlist(),
        })
    }

    /// The redirect URLs the WorkOS dashboard must list for this deployment.
    pub fn redirect_allowlist(&self) -> &[String] {
        &self.redirect_allowlist
    }
}

pub struct WorkOsIdentity<T> {
    transport: T,
    config: WorkOsConfig,
}

impl<T: HttpTransport> WorkOsIdentity<T> {
    pub fn new(transport: T, config: WorkOsConfig) -> Self {
        Self { transport, config }
    }

    fn request(&self, method: Method, path: String, body: Option<Value>, key: bool) -> HttpRequest {
        HttpRequest {
            method,
            path,
            bearer: key.then(|| self.config.api_key.0.clone()),
            body,
        }
    }

    /// Sends a request. `mutation` marks calls whose lost answer may hide a change.
    async fn call(
        &self,
        request: HttpRequest,
        mutation: bool,
    ) -> Result<HttpResponse, IdentityFailure> {
        let response = self
            .transport
            .send(request)
            .await
            .map_err(|error| match error {
                TransportError::NotSent => IdentityFailure::Unavailable,
                TransportError::Unknown if mutation => IdentityFailure::Unknown,
                TransportError::Unknown => IdentityFailure::Unavailable,
            })?;
        match response.status {
            // A rejected key and an authentication answer share these statuses; the body names
            // which: sign-in answers carry their own code and are for the caller to read.
            401 | 403
                if !has_code(
                    &error_codes(&response),
                    &[
                        "email_verification_required",
                        "invalid_credentials",
                        "invalid_grant",
                    ],
                ) =>
            {
                df_observe::record_audit_line(
                    "identity.provider_rejected_key",
                    format!("IDENTITY provider_rejected_key status={}", response.status),
                );
                Err(IdentityFailure::Unavailable)
            }
            429 | 503 => Err(IdentityFailure::Unavailable),
            500..=599 if mutation => Err(IdentityFailure::Unknown),
            500..=599 => Err(IdentityFailure::Unavailable),
            _ => Ok(response),
        }
    }
}

fn json_of(response: &HttpResponse) -> Result<Value, IdentityFailure> {
    serde_json::from_slice(&response.body).map_err(|_| IdentityFailure::Malformed)
}

/// Every error code in a response body: the top-level `code` or `error`, and `errors[].code`.
fn error_codes(response: &HttpResponse) -> Vec<String> {
    let Ok(value) = serde_json::from_slice::<Value>(&response.body) else {
        return Vec::new();
    };
    let mut codes = Vec::new();
    for key in ["code", "error"] {
        if let Some(code) = value.get(key).and_then(Value::as_str) {
            codes.push(code.to_owned());
        }
    }
    if let Some(errors) = value.get("errors").and_then(Value::as_array) {
        codes.extend(
            errors
                .iter()
                .filter_map(|error| error.get("code").and_then(Value::as_str))
                .map(str::to_owned),
        );
    }
    codes
}

/// A provider code safe to write to an audit line: lowercase letters, digits and underscores,
/// at most 48 characters; anything else is named `other`.
fn audited_code(codes: &[String]) -> String {
    codes
        .first()
        .filter(|code| {
            code.len() <= 48
                && code
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        })
        .cloned()
        .unwrap_or_else(|| "other".to_owned())
}

/// One audit line for an answer this adapter did not recognize.
fn audit_unrecognized(call: &str, response: &HttpResponse) {
    df_observe::record_audit_line(
        "identity.provider_unrecognized",
        format!(
            "IDENTITY provider_unrecognized call={call} status={} code={}",
            response.status,
            audited_code(&error_codes(response))
        ),
    );
}

fn has_code(codes: &[String], wanted: &[&str]) -> bool {
    codes.iter().any(|code| wanted.contains(&code.as_str()))
}

/// Codes WorkOS gives a taken address, a weak or breached password, a spent token, a bad
/// credential. See the module notes: these strings are the adapter's one reading of them.
const TAKEN: &[&str] = &["email_not_available", "user_already_exists"];
const WEAK_PASSWORD: &[&str] = &[
    "password_too_weak",
    "password_strength_error",
    "password_pwned",
    "password_reuse",
    "password_too_short",
];
const BAD_CREDENTIALS: &[&str] = &["invalid_credentials", "invalid_grant"];
/// Sign-in steps this server does not support (it never sends the person to a provider page).
const UNSUPPORTED_STEPS: &[&str] = &[
    "mfa_challenge",
    "mfa_enrollment",
    "organization_selection_required",
    "organization_authentication_methods_required",
    "sso_required",
    "radar_email_challenge",
    "radar_sms_challenge",
];
const BAD_TOKEN: &[&str] = &[
    "password_reset_token_expired",
    "password_reset_token_used",
    "password_reset_error",
    "invalid_token",
    "entity_not_found",
];

fn user_of(value: &Value) -> Result<ProviderUser, IdentityFailure> {
    let subject = value
        .get("id")
        .and_then(Value::as_str)
        .ok_or(IdentityFailure::Malformed)?;
    let email = value
        .get("email")
        .and_then(Value::as_str)
        .and_then(|email| EmailAddress::parse(email).ok())
        .ok_or(IdentityFailure::Malformed)?;
    Ok(ProviderUser {
        subject: subject.to_owned(),
        email,
        email_verified: value
            .get("email_verified")
            .and_then(Value::as_bool)
            .ok_or(IdentityFailure::Malformed)?,
    })
}

/// The `sid` claim of an access token: the provider session it belongs to. The token came over
/// TLS from WorkOS in answer to our own request, so its signature is not checked and nothing
/// here trusts it for anything but naming the session to end.
fn session_of(access_token: &str) -> Option<String> {
    let payload = access_token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims.get("sid").and_then(Value::as_str).map(str::to_owned)
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// A path segment from a provider id: the same set of safe characters, else refused.
fn segment(id: &str) -> Result<String, IdentityFailure> {
    (!id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')))
    .then(|| id.to_owned())
    .ok_or(IdentityFailure::Malformed)
}

impl<T: HttpTransport> IdentityProvider for WorkOsIdentity<T> {
    fn kind(&self) -> IdentityProviderKind {
        IdentityProviderKind::WorkOs
    }

    async fn create_user(
        &self,
        email: &EmailAddress,
        password: &str,
        caller: CallerContext,
    ) -> Result<CreateUser, IdentityFailure> {
        let mut body = json!({
            "email": email.normalized(),
            "password": password,
            "email_verified": false,
        });
        if let Some(ip) = caller.ip {
            body["ip_address"] = json!(ip.to_string());
        }
        let response = self
            .call(
                self.request(
                    Method::Post,
                    "/user_management/users".to_owned(),
                    Some(body),
                    true,
                ),
                true,
            )
            .await?;
        match response.status {
            200 | 201 => Ok(CreateUser::Created(user_of(&json_of(&response)?)?)),
            400..=499 => {
                let codes = error_codes(&response);
                if has_code(&codes, TAKEN) {
                    Ok(CreateUser::EmailNotAvailable)
                } else if has_code(&codes, WEAK_PASSWORD) {
                    Ok(CreateUser::PasswordRejected)
                } else {
                    // An unrecognized refusal: ask whether the address already has a user, so
                    // a taken address is never told apart from a new one by its error shape.
                    audit_unrecognized("create_user", &response);
                    Ok(if self.find_user(email).await?.is_some() {
                        CreateUser::EmailNotAvailable
                    } else {
                        CreateUser::PasswordRejected
                    })
                }
            }
            _ => {
                audit_unrecognized("create_user", &response);
                Err(IdentityFailure::Malformed)
            }
        }
    }

    async fn authenticate(
        &self,
        email: &EmailAddress,
        password: &str,
        caller: CallerContext,
    ) -> Result<Authentication, IdentityFailure> {
        let mut body = json!({
            "client_id": self.config.client_id,
            "client_secret": self.config.api_key.0,
            "grant_type": "password",
            "email": email.normalized(),
            "password": password,
        });
        if let Some(ip) = caller.ip {
            body["ip_address"] = json!(ip.to_string());
        }
        let response = self
            .call(
                self.request(
                    Method::Post,
                    "/user_management/authenticate".to_owned(),
                    Some(body),
                    false,
                ),
                false,
            )
            .await?;
        match response.status {
            200 => {
                let value = json_of(&response)?;
                let user = user_of(value.get("user").ok_or(IdentityFailure::Malformed)?)?;
                // The tokens are dropped here; only the session's name is kept, to end it.
                let provider_session = value
                    .get("access_token")
                    .and_then(Value::as_str)
                    .and_then(session_of);
                Ok(Authentication::Authenticated {
                    user,
                    provider_session,
                })
            }
            400..=499 => {
                let codes = error_codes(&response);
                if has_code(&codes, &["email_verification_required"]) {
                    Ok(Authentication::EmailVerificationRequired)
                } else if has_code(&codes, BAD_CREDENTIALS) {
                    Ok(Authentication::InvalidCredentials)
                } else if has_code(&codes, UNSUPPORTED_STEPS) {
                    // MFA, organization selection, SSO required, Radar challenges: steps this
                    // sign-in does not support. Named, audited, and never a wrong password.
                    audit_unrecognized("authenticate", &response);
                    Err(IdentityFailure::Unavailable)
                } else {
                    // Fail closed: an answer we do not recognize refuses the credential.
                    audit_unrecognized("authenticate", &response);
                    Ok(Authentication::InvalidCredentials)
                }
            }
            _ => {
                audit_unrecognized("authenticate", &response);
                Err(IdentityFailure::Malformed)
            }
        }
    }

    async fn find_user(
        &self,
        email: &EmailAddress,
    ) -> Result<Option<ProviderUser>, IdentityFailure> {
        let path = format!(
            "/user_management/users?email={}",
            encode(email.normalized())
        );
        let response = self
            .call(self.request(Method::Get, path, None, true), false)
            .await?;
        if response.status != 200 {
            return Err(IdentityFailure::Malformed);
        }
        let value = json_of(&response)?;
        let users = value
            .get("data")
            .and_then(Value::as_array)
            .ok_or(IdentityFailure::Malformed)?;
        for candidate in users {
            let user = user_of(candidate)?;
            if user.email.normalized() == email.normalized() {
                return Ok(Some(user));
            }
        }
        Ok(None)
    }

    async fn mark_email_verified(&self, subject: &str) -> Result<(), IdentityFailure> {
        let path = format!("/user_management/users/{}", segment(subject)?);
        let response = self
            .call(
                self.request(
                    Method::Put,
                    path,
                    Some(json!({"email_verified": true})),
                    true,
                ),
                true,
            )
            .await?;
        match response.status {
            200 | 204 | 404 => Ok(()),
            _ => Err(IdentityFailure::Malformed),
        }
    }

    async fn set_password(
        &self,
        subject: &str,
        password: &str,
    ) -> Result<SetPassword, IdentityFailure> {
        let path = format!("/user_management/users/{}", segment(subject)?);
        let response = self
            .call(
                self.request(Method::Put, path, Some(json!({"password": password})), true),
                true,
            )
            .await?;
        match response.status {
            200 | 204 => Ok(SetPassword::Set),
            400..=499 if response.status != 404 => {
                let codes = error_codes(&response);
                if !has_code(&codes, WEAK_PASSWORD) {
                    audit_unrecognized("set_password", &response);
                }
                // Fail closed: the password was not set, and the person is told to choose another.
                Ok(SetPassword::Rejected)
            }
            _ => {
                audit_unrecognized("set_password", &response);
                Err(IdentityFailure::Malformed)
            }
        }
    }

    async fn begin_password_reset(
        &self,
        email: &EmailAddress,
    ) -> Result<ResetStart, IdentityFailure> {
        let response = self
            .call(
                self.request(
                    Method::Post,
                    "/user_management/password_reset".to_owned(),
                    Some(json!({"email": email.normalized()})),
                    true,
                ),
                true,
            )
            .await?;
        match response.status {
            200 | 201 => {
                let value = json_of(&response)?;
                // A reset the dashboard would send people to a URL we did not register is not
                // used: the dashboard is misconfigured.
                if let Some(url) = value.get("password_reset_url").and_then(Value::as_str)
                    && !self
                        .config
                        .redirect_allowlist
                        .iter()
                        .any(|allowed| url.starts_with(allowed.as_str()))
                {
                    df_observe::record_audit_line(
                        "identity.redirect_not_allowed",
                        "IDENTITY redirect_not_allowed purpose=password_reset".to_owned(),
                    );
                    return Err(IdentityFailure::Malformed);
                }
                let token = value
                    .get("password_reset_token")
                    .and_then(Value::as_str)
                    .ok_or(IdentityFailure::Malformed)?;
                Ok(ResetStart::Started {
                    token: token.to_owned(),
                })
            }
            404 => Ok(ResetStart::NoSuchUser),
            _ => {
                audit_unrecognized("begin_password_reset", &response);
                Err(IdentityFailure::Malformed)
            }
        }
    }

    async fn finish_password_reset(
        &self,
        token: &str,
        new_password: &str,
    ) -> Result<ResetFinish, IdentityFailure> {
        let response = self
            .call(
                self.request(
                    Method::Post,
                    "/user_management/password_reset/confirm".to_owned(),
                    Some(json!({"token": token, "new_password": new_password})),
                    true,
                ),
                true,
            )
            .await?;
        match response.status {
            200 => {
                let value = json_of(&response)?;
                let user = value.get("user").unwrap_or(&value);
                Ok(ResetFinish::Reset(user_of(user)?))
            }
            400..=499 => {
                let codes = error_codes(&response);
                if has_code(&codes, WEAK_PASSWORD) {
                    Ok(ResetFinish::PasswordRejected)
                } else {
                    // A spent token and an unrecognized refusal read alike, and the second is
                    // audited: fail closed.
                    if !has_code(&codes, BAD_TOKEN) {
                        audit_unrecognized("finish_password_reset", &response);
                    }
                    Ok(ResetFinish::InvalidToken)
                }
            }
            _ => {
                audit_unrecognized("finish_password_reset", &response);
                Err(IdentityFailure::Malformed)
            }
        }
    }

    async fn end_session(&self, provider_session: &str) -> Result<(), IdentityFailure> {
        let response = self
            .call(
                self.request(
                    Method::Post,
                    "/user_management/sessions/revoke".to_owned(),
                    Some(json!({"session_id": provider_session})),
                    true,
                ),
                true,
            )
            .await?;
        match response.status {
            200 | 204 | 404 => Ok(()),
            _ => Err(IdentityFailure::Malformed),
        }
    }

    async fn delete_user(&self, subject: &str) -> Result<(), IdentityFailure> {
        let path = format!("/user_management/users/{}", segment(subject)?);
        let response = self
            .call(self.request(Method::Delete, path, None, true), true)
            .await?;
        match response.status {
            200 | 204 | 404 => Ok(()),
            _ => Err(IdentityFailure::Malformed),
        }
    }
}
