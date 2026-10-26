//! Generation-bound integration screens. Navigation and sign-out drop one-time reveals.
use super::*;
use crate::{
    api::integration::{Answer, Call},
    wire_integration::*,
    wire_share::*,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntegrationField {
    Label,
    Expiry,
    Password,
    Title,
    Highlights,
    ShareExpiry,
    Indexing,
    Embeds,
}
impl IntegrationField {
    pub const ALL: [Self; 8] = [
        Self::Label,
        Self::Expiry,
        Self::Password,
        Self::Title,
        Self::Highlights,
        Self::ShareExpiry,
        Self::Indexing,
        Self::Embeds,
    ];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Label => "token-label",
            Self::Expiry => "token-expiry",
            Self::Password => "token-password",
            Self::Title => "share-title",
            Self::Highlights => "share-highlights",
            Self::ShareExpiry => "share-expiry",
            Self::Indexing => "share-indexing",
            Self::Embeds => "share-embeds",
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IntegrationIntent {
    CreateToken,
    RevokeToken(String),
    ToggleScope(String),
    Person(String),
    SavePreview,
    Consent(bool),
    CreateShare,
    RevokeShare(String),
    Copy,
    Dismiss,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokensScreen {
    pub rows: Remote<Vec<PersonalTokenView>>,
    pub label: String,
    pub scopes: Vec<String>,
    pub expiry: String,
    pub password: SecretReveal,
    pub reveal: Option<SecretReveal>,
    pub submit: Submit,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SharingScreen {
    pub session: String,
    pub preview: Remote<SharePreview>,
    pub title: String,
    pub highlights: String,
    pub expiry: String,
    pub indexing: bool,
    pub embeds: bool,
    pub dirty: bool,
    pub links: Remote<Vec<ShareLinkView>>,
    pub reveal: Option<SecretReveal>,
    pub submit: Submit,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IntegrationScreen {
    Tokens(TokensScreen),
    Share(SharingScreen),
}

impl Model {
    pub(super) fn enter_integration(&mut self, route: Route) -> Vec<Effect> {
        let (screen, calls) = match route {
            Route::DeveloperTokens => (
                IntegrationScreen::Tokens(TokensScreen {
                    rows: Remote::Loading,
                    label: String::new(),
                    scopes: vec!["account:read".to_owned()],
                    expiry: "90".to_owned(),
                    password: SecretReveal(String::new()),
                    reveal: None,
                    submit: Submit::Idle,
                }),
                vec![Call::Tokens],
            ),
            Route::Share { session_id } => (
                IntegrationScreen::Share(SharingScreen {
                    session: session_id.clone(),
                    preview: Remote::Loading,
                    title: String::new(),
                    highlights: String::new(),
                    expiry: "90".to_owned(),
                    indexing: false,
                    embeds: false,
                    dirty: false,
                    links: Remote::Loading,
                    reveal: None,
                    submit: Submit::Idle,
                }),
                vec![Call::Preview(session_id)],
            ),
            _ => return Vec::new(),
        };
        self.screen = Screen::Integration(Box::new(screen));
        calls
            .into_iter()
            .map(|c| self.api(ApiCall::Integration(c)))
            .collect()
    }
    pub(super) fn edit_integration(&mut self, field: Field, value: &str) -> bool {
        let Field::Integration(field) = field else {
            return false;
        };
        let Screen::Integration(screen) = &mut self.screen else {
            return false;
        };
        if match &**screen {
            IntegrationScreen::Tokens(s) => s.submit.is_sending(),
            IntegrationScreen::Share(s) => s.submit.is_sending(),
        } {
            return true;
        }
        match (&mut **screen, field) {
            (IntegrationScreen::Tokens(s), IntegrationField::Label) => s.label = value.to_owned(),
            (IntegrationScreen::Tokens(s), IntegrationField::Expiry) => s.expiry = value.to_owned(),
            (IntegrationScreen::Tokens(s), IntegrationField::Password) => {
                s.password = SecretReveal(value.to_owned())
            }
            (IntegrationScreen::Share(s), IntegrationField::Title) => {
                s.title = value.to_owned();
                s.dirty = true;
            }
            (IntegrationScreen::Share(s), IntegrationField::Highlights) => {
                s.highlights = value.to_owned();
                s.dirty = true;
            }
            (IntegrationScreen::Share(s), IntegrationField::ShareExpiry) => {
                s.expiry = value.to_owned()
            }
            _ => {}
        }
        true
    }
    pub(super) fn integration_intent(&mut self, intent: IntegrationIntent) -> Vec<Effect> {
        let Screen::Integration(screen) = &mut self.screen else {
            return Vec::new();
        };
        let call = match (&mut **screen, intent) {
            (IntegrationScreen::Tokens(s), IntegrationIntent::ToggleScope(scope))
                if !s.submit.is_sending() =>
            {
                if s.scopes.contains(&scope) {
                    s.scopes.retain(|v| v != &scope);
                } else {
                    s.scopes.push(scope);
                }
                return Vec::new();
            }
            (IntegrationScreen::Tokens(s), IntegrationIntent::CreateToken)
                if !s.submit.is_sending() =>
            {
                s.reveal = None;
                let password = (!s.password.0.is_empty()).then(|| s.password.clone());
                s.password.0.clear();
                s.submit = Submit::Sending;
                Call::CreateToken(CreatePersonalToken {
                    label: s.label.clone(),
                    scopes: s.scopes.clone(),
                    expiry_days: s.expiry.parse().unwrap_or(0),
                    password,
                })
            }
            (IntegrationScreen::Tokens(s), IntegrationIntent::RevokeToken(id))
                if !s.submit.is_sending() =>
            {
                s.reveal = None;
                s.submit = Submit::Sending;
                let password = (!s.password.0.is_empty()).then(|| s.password.clone());
                s.password.0.clear();
                Call::RevokeToken(id, RevokePersonalToken { password })
            }
            (IntegrationScreen::Tokens(s), IntegrationIntent::Copy) => {
                return s
                    .reveal
                    .clone()
                    .map(Effect::CopySecret)
                    .into_iter()
                    .collect();
            }
            (IntegrationScreen::Tokens(s), IntegrationIntent::Dismiss) => {
                s.reveal = None;
                return Vec::new();
            }
            (IntegrationScreen::Share(s), IntegrationIntent::Person(account))
                if !s.submit.is_sending() =>
            {
                if let Remote::Ready(p) = &mut s.preview {
                    if p.draft
                        .people
                        .iter()
                        .any(|person| person.account_id == account)
                    {
                        p.draft.people.retain(|person| person.account_id != account);
                    } else if let Some(person) = p
                        .candidates
                        .iter()
                        .find(|person| person.account_id == account)
                    {
                        p.draft.people.push(person.clone());
                    }
                    s.dirty = true;
                }
                return Vec::new();
            }
            (IntegrationScreen::Share(s), IntegrationIntent::SavePreview)
                if !s.submit.is_sending() =>
            {
                s.reveal = None;
                s.submit = Submit::Sending;
                Call::SavePreview(
                    s.session.clone(),
                    ShareDraft {
                        title: s.title.clone(),
                        highlights: s
                            .highlights
                            .lines()
                            .filter(|l| !l.trim().is_empty())
                            .map(str::to_owned)
                            .collect(),
                        people: s
                            .preview
                            .ready()
                            .map(|p| p.draft.people.clone())
                            .unwrap_or_default(),
                    },
                )
            }
            (IntegrationScreen::Share(s), IntegrationIntent::Consent(agrees))
                if !s.submit.is_sending() && (!agrees || !s.dirty) =>
            {
                let Some(p) = s.preview.ready() else {
                    return Vec::new();
                };
                s.submit = Submit::Sending;
                if !agrees {
                    s.reveal = None;
                }
                Call::Consent(
                    s.session.clone(),
                    ShareConsent {
                        revision: p.revision.clone(),
                        agrees,
                    },
                )
            }
            (IntegrationScreen::Share(s), IntegrationIntent::CreateShare)
                if !s.submit.is_sending() && !s.dirty =>
            {
                let Some(p) = s.preview.ready() else {
                    return Vec::new();
                };
                s.submit = Submit::Sending;
                s.reveal = None;
                Call::CreateShare(
                    s.session.clone(),
                    CreateShare {
                        revision: p.revision.clone(),
                        expiry_days: s.expiry.parse().unwrap_or(0),
                        allow_indexing: s.indexing,
                        allow_embeds: s.embeds,
                    },
                )
            }
            (IntegrationScreen::Share(s), IntegrationIntent::RevokeShare(key))
                if !s.submit.is_sending() =>
            {
                s.reveal = None;
                s.submit = Submit::Sending;
                Call::RevokeShare(s.session.clone(), key)
            }
            (IntegrationScreen::Share(s), IntegrationIntent::Copy) => {
                return s
                    .reveal
                    .clone()
                    .map(Effect::CopySecret)
                    .into_iter()
                    .collect();
            }
            (IntegrationScreen::Share(s), IntegrationIntent::Dismiss) => {
                s.reveal = None;
                return Vec::new();
            }
            _ => return Vec::new(),
        };
        vec![self.api(ApiCall::Integration(call))]
    }
    pub(super) fn integration_reply(&mut self, answer: Answer) -> Vec<Effect> {
        let Screen::Integration(screen) = &mut self.screen else {
            return Vec::new();
        };
        let error = answer.error();
        let mut calls = Vec::new();
        match (&mut **screen, answer) {
            (IntegrationScreen::Tokens(s), Answer::Tokens(r)) => s.rows = Remote::from_result(r),
            (IntegrationScreen::Tokens(s), Answer::TokenIssued(Ok(issued))) => {
                s.reveal = Some(issued.secret);
                s.submit = Submit::Idle;
                calls.push(Call::Tokens);
            }
            (IntegrationScreen::Tokens(s), Answer::TokenRevoked(Ok(_))) => {
                s.submit = Submit::Idle;
                calls.push(Call::Tokens);
            }
            (IntegrationScreen::Share(s), Answer::Preview(Ok(p))) => {
                if p.can_publish {
                    calls.push(Call::Shares(s.session.clone()));
                }
                s.title = p.draft.title.clone();
                s.highlights = p.draft.highlights.join("\n");
                s.preview = Remote::Ready(p);
                s.dirty = false;
                s.submit = Submit::Idle;
            }
            (IntegrationScreen::Share(s), Answer::Preview(Err(e))) => s.preview = Remote::Failed(e),
            (IntegrationScreen::Share(s), Answer::Shares(r)) => s.links = Remote::from_result(r),
            (IntegrationScreen::Share(s), Answer::ShareIssued(Ok(issued))) => {
                s.reveal = Some(issued.url);
                s.submit = Submit::Idle;
                calls.push(Call::Shares(s.session.clone()));
            }
            (IntegrationScreen::Share(s), Answer::Consent(Ok(_))) => {
                s.submit = Submit::Idle;
                calls.push(Call::Preview(s.session.clone()));
            }
            (IntegrationScreen::Share(s), Answer::ShareRevoked(Ok(_))) => {
                s.submit = Submit::Idle;
                calls.push(Call::Shares(s.session.clone()));
            }
            _ => {}
        }
        if let Some(error) = error {
            match &mut **screen {
                IntegrationScreen::Tokens(s) => s.submit = Submit::Refused(error),
                IntegrationScreen::Share(s) => s.submit = Submit::Refused(error),
            }
        }
        calls
            .into_iter()
            .map(|c| self.api(ApiCall::Integration(c)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn signed_in() -> Model {
        Model {
            session: Remote::Ready(SessionView {
                google_enabled: false,
                csrf: "synthetic-csrf".into(),
                account: Some(AccountView {
                    email: "synthetic@example.test".into(),
                    display_name: None,
                    is_verified: true,
                    operator: None,
                }),
                dev_sign_in: None,
            }),
            ..Model::default()
        }
    }
    #[test]
    fn integration_token_reveal_and_password_are_redacted_and_cleared_on_navigation() {
        let mut model = signed_in();
        model.update(Msg::Intent(Intent::Go(Route::DeveloperTokens)));
        model.update(Msg::Edit(
            Field::Integration(IntegrationField::Password),
            "synthetic-password".into(),
        ));
        let effects = model.update(Msg::Intent(Intent::Integration(
            IntegrationIntent::CreateToken,
        )));
        assert!(!format!("{effects:?}").contains("synthetic-password"));
        assert!(!format!("{model:?}").contains("synthetic-password"));
        let generation = model.generation;
        model.update(Msg::Replied {
            generation,
            reply: Reply::Integration(Answer::TokenIssued(Ok(PersonalTokenIssued {
                token: PersonalTokenView {
                    token_id: "01".repeat(16),
                    label: "script".into(),
                    scopes: vec!["account:read".into()],
                    created_at_ms: 1,
                    expires_at_ms: 2,
                    revoked_at_ms: None,
                    last_used_at_ms: None,
                },
                secret: SecretReveal("synthetic-one-time-secret".into()),
            }))),
        });
        assert!(!format!("{model:?}").contains("synthetic-one-time-secret"));
        let effects = model.update(Msg::Intent(Intent::Integration(IntegrationIntent::Copy)));
        assert!(
            matches!(&effects[..],[Effect::CopySecret(secret)] if secret.0=="synthetic-one-time-secret")
        );
        model.update(Msg::Intent(Intent::Go(Route::Settings)));
        assert!(!format!("{:?}", crate::view::view(&model)).contains("synthetic-one-time-secret"));
        model.update(Msg::Replied {
            generation,
            reply: Reply::Integration(Answer::TokenIssued(Err(ApiError::of(
                ErrorCode::Unavailable,
            )))),
        });
        assert!(matches!(model.screen, Screen::Settings(_)));
    }
    #[test]
    fn integration_share_edit_requires_a_new_exact_preview_before_consent_or_publish() {
        let mut model = signed_in();
        model.update(Msg::Intent(Intent::Go(Route::Share {
            session_id: "09".repeat(16),
        })));
        model.integration_reply(Answer::Preview(Ok(SharePreview {
            session_id: "09".repeat(16),
            revision: "revision".into(),
            draft: ShareDraft {
                title: "Public title".into(),
                highlights: Vec::new(),
                people: Vec::new(),
            },
            date: "2026-10-25".into(),
            duration_minutes: 60,
            consented_accounts: Vec::new(),
            can_publish: true,
            can_consent: false,
            candidates: Vec::new(),
        })));
        model.update(Msg::Edit(
            Field::Integration(IntegrationField::Title),
            "Changed title".into(),
        ));
        assert!(
            model
                .update(Msg::Intent(Intent::Integration(
                    IntegrationIntent::Consent(true)
                )))
                .is_empty()
        );
        assert!(
            model
                .update(Msg::Intent(Intent::Integration(
                    IntegrationIntent::CreateShare
                )))
                .is_empty()
        );
        assert!(
            !model
                .update(Msg::Intent(Intent::Integration(
                    IntegrationIntent::SavePreview
                )))
                .is_empty()
        );
    }
    #[test]
    fn integration_mutations_use_csrf_and_expiry_and_scopes_are_explicit() {
        let create = Call::CreateToken(CreatePersonalToken {
            label: "script".into(),
            scopes: vec!["funding:read".into()],
            expiry_days: 30,
            password: None,
        })
        .request();
        assert_eq!(create.guard, crate::api::Guard::Csrf);
        let body: serde_json::Value =
            serde_json::from_str(create.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["scopes"], serde_json::json!(["funding:read"]));
        assert_eq!(body["expiry_days"], 30);
        assert_eq!(Call::Tokens.request().guard, crate::api::Guard::None);
    }
}
