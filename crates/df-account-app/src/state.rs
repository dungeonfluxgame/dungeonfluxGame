//! The app's whole state and the one function that changes it.
//!
//! `update` takes a message (an intent from the screen, an edited field, a reply, a timer) and
//! returns effects as values; the wasm runtime performs them. Every reply carries the generation
//! it was requested under, and navigation starts a new generation, so a late reply for a screen the
//! person has left changes nothing (coding style 7, stale completions). The server decides every
//! permission; routing here only picks which screen to show (decision 28).

mod admin_flow;
mod auth_flow;
mod campaign_flow;
mod customer_flow;
mod integration_flow;
pub use integration_flow::{
    IntegrationField, IntegrationIntent, IntegrationScreen, SharingScreen, TokensScreen,
};

pub use admin_flow::{
    AccountScreen, ActionTarget, AdminScreen, CreditGrantForm, ReasonAction, RefundForm,
    RowsScreen, SearchScreen,
};
pub use auth_flow::{ForgotForm, ResetForm, SignInForm, SignUpForm, VerifyConfirm, VerifyPending};
pub use campaign_flow::{
    CampaignScreen, JoinScreen, LobbyScreen, MAXIMUM_CAMPAIGN_NAME_CHARS,
    MAXIMUM_GAME_START_WAIT_MS, PlayScreen, RequestDecision, SLOW_START_POLLS, WIZARD_STEPS,
    WizardScreen,
};
pub use customer_flow::{
    AllocateEdit, AmountChoice, AssetsScreen, CHECKOUT_POLL_DELAYS_MS, CreditForm, CreditReturn,
    DashboardScreen, FundingScreen, MAXIMUM_STATEMENT_LINES, PaceEdit, PlaySessionsScreen,
    SettingsScreen, StatementsScreen,
};

use crate::{
    api::{ApiCall, Reply},
    route::{Audience, Route},
    wire::{AccountView, ApiError, ErrorCode, OperationView, SessionView},
};

/// A value loaded from the server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Remote<T> {
    Loading,
    Ready(T),
    Failed(ApiError),
}

impl<T> Remote<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Self::Ready(value) => Some(value),
            Self::Loading | Self::Failed(_) => None,
        }
    }

    fn from_result(result: Result<T, ApiError>) -> Self {
        match result {
            Ok(value) => Self::Ready(value),
            Err(error) => Self::Failed(error),
        }
    }
}

/// A form's send state. A refused form keeps what the person typed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Submit {
    Idle,
    Sending,
    Refused(ApiError),
}

impl Submit {
    pub fn is_sending(self) -> bool {
        self == Self::Sending
    }

    pub fn error(self) -> Option<ApiError> {
        match self {
            Self::Refused(error) => Some(error),
            Self::Idle | Self::Sending => None,
        }
    }
}

/// A one-time message shown at the top of the next screen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Notice {
    SignInToContinue,
    SessionEnded,
    EmailVerified,
    /// Signing out could not be confirmed; the person may still be signed in.
    SignOutFailed,
    PasswordChanged,
    AccountClosed {
        delete_after_ms: u64,
    },
}

/// What the person asked for by clicking or submitting.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Intent {
    Integration(IntegrationIntent),
    Go(Route),
    /// Load the current screen again after a failure.
    Retry,
    Submit(Form),
    SignOut,
    UseDevSignIn(usize),
    ResendVerification,
    ChooseAmount(u32),
    ChooseCustomAmount,
    EditPace {
        campaign_id: String,
    },
    EditAllocation {
        session_id: String,
    },
    CancelEdit,
    LoadOlderStatements,
    LoadOlderPlaySessions,
    LoadOlderAssets,
    RevokeOtherSessions,
    RevokeAllSessions,
    RevokeSession {
        session_id: String,
    },
    DownloadStatementsCsv,
    SetStatementFilter(crate::wire::StatementFilter),
    SetAssetKind(Option<crate::wire::AssetKind>),
    SelectAdminTab(crate::wire::AdminTab),
    SetAdminStanding(String),
    SetAdminVerified(String),
    SetAdminHasBalance(String),
    SetAdminSort(String),
    ChooseLot {
        lot_id: String,
    },
    OpenAction(ActionTarget),
    AcceptPendingInvitation,
    AcceptInvitation,
    /// Remember the invitation on the server (the join cookie), then go to sign in or sign up.
    HoldInvitation {
        then: Route,
    },
    /// Ask the host to approve a room-code request.
    AskToJoin,
    ClearCodeLookup,
    WizardBack,
    CreateInvitation(crate::wire_campaign::InvitationKind),
    RevokeInvitation {
        invitation_id: String,
    },
    RotateCode,
    DecideRequest {
        request_id: String,
        decision: RequestDecision,
    },
    AskLeave,
    LeaveCampaign,
    OpenLobby,
    PlayNow,
    CancelLobby,
    CopyText(String),
    StartSession,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Form {
    SignUp,
    SignIn,
    VerifyEmail,
    Forgot,
    Reset,
    Credit,
    Pace,
    Allocate,
    Profile,
    ChangeEmail,
    ChangePassword,
    Preferences,
    Export,
    Close,
    Search,
    Refund,
    Goodwill,
    Action,
    WizardStep,
    CreateCampaign,
    CampaignSettings,
    CodeLookup,
}

/// Every editable field. The DOM carries `name()`; nothing else identifies a field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Field {
    Integration(IntegrationField),
    Email,
    Password,
    Confirmation,
    IsAdult,
    AcceptsTerms,
    CustomAmount,
    PacePreset,
    CustomCeiling,
    DisplayName,
    ClosePassword,
    CloseUnderstood,
    SearchQuery,
    RefundAmount,
    RefundReason,
    CreditAmount,
    CreditReason,
    SupportCase,
    ActionReason,
    CampaignName,
    Adventure,
    Rating,
    SeatCount,
    HasVoice,
    Pictures,
    CoHostsCanInvite,
    CoHostsCanStart,
    CoHostsCanExport,
    JoinModeChoice,
    FundingChoice,
    PairingCode,
    RoomCode,
    AllocateAmount,
    AllocateConsent,
    NewEmail,
    CurrentPassword,
    EmailPassword,
    ExportPassword,
    ProductNews,
    SessionReminders,
    StatementFrom,
    StatementTo,
    AssetKindFilter,
    AssetCampaignFilter,
}

const FIELDS: [Field; 43] = [
    Field::Email,
    Field::Password,
    Field::Confirmation,
    Field::IsAdult,
    Field::AcceptsTerms,
    Field::CustomAmount,
    Field::PacePreset,
    Field::CustomCeiling,
    Field::DisplayName,
    Field::ClosePassword,
    Field::CloseUnderstood,
    Field::SearchQuery,
    Field::RefundAmount,
    Field::RefundReason,
    Field::CreditAmount,
    Field::CreditReason,
    Field::SupportCase,
    Field::ActionReason,
    Field::CampaignName,
    Field::Adventure,
    Field::Rating,
    Field::SeatCount,
    Field::HasVoice,
    Field::Pictures,
    Field::CoHostsCanInvite,
    Field::CoHostsCanStart,
    Field::CoHostsCanExport,
    Field::JoinModeChoice,
    Field::FundingChoice,
    Field::PairingCode,
    Field::RoomCode,
    Field::AllocateAmount,
    Field::AllocateConsent,
    Field::NewEmail,
    Field::CurrentPassword,
    Field::EmailPassword,
    Field::ExportPassword,
    Field::ProductNews,
    Field::SessionReminders,
    Field::StatementFrom,
    Field::StatementTo,
    Field::AssetKindFilter,
    Field::AssetCampaignFilter,
];

impl Field {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Password => "password",
            Self::Confirmation => "confirmation",
            Self::IsAdult => "is-adult",
            Self::AcceptsTerms => "accepts-terms",
            Self::CustomAmount => "custom-amount",
            Self::PacePreset => "pace-preset",
            Self::CustomCeiling => "custom-ceiling",
            Self::DisplayName => "display-name",
            Self::ClosePassword => "close-password",
            Self::CloseUnderstood => "close-understood",
            Self::SearchQuery => "search-query",
            Self::RefundAmount => "refund-amount",
            Self::RefundReason => "refund-reason",
            Self::CreditAmount => "credit-amount",
            Self::CreditReason => "credit-reason",
            Self::SupportCase => "support-case",
            Self::ActionReason => "action-reason",
            Self::CampaignName => "campaign-name",
            Self::Adventure => "adventure",
            Self::Rating => "content-rating",
            Self::SeatCount => "seat-count",
            Self::HasVoice => "has-voice",
            Self::Pictures => "pictures",
            Self::CoHostsCanInvite => "co-hosts-can-invite",
            Self::CoHostsCanStart => "co-hosts-can-start",
            Self::CoHostsCanExport => "co-hosts-can-export",
            Self::JoinModeChoice => "join-mode",
            Self::FundingChoice => "funding",
            Self::PairingCode => "pairing-code",
            Self::RoomCode => "room-code",
            Self::AllocateAmount => "allocate-amount",
            Self::AllocateConsent => "allocate-consent",
            Self::NewEmail => "new-email",
            Self::CurrentPassword => "current-password",
            Self::EmailPassword => "email-password",
            Self::ExportPassword => "export-password",
            Self::ProductNews => "product-news",
            Self::SessionReminders => "session-reminders",
            Self::StatementFrom => "statement-from",
            Self::StatementTo => "statement-to",
            Self::AssetKindFilter => "asset-kind",
            Self::AssetCampaignFilter => "asset-campaign",
            Self::Integration(field) => field.name(),
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        FIELDS
            .into_iter()
            .chain(IntegrationField::ALL.map(Field::Integration))
            .find(|field| field.name() == name)
    }
}

/// Which of a screen's forms an issued operation id belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationSlot {
    Primary,
    Secondary,
}

// One message per user or network event, moved once into `Model::update`; see `Reply`.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Msg {
    /// The page loaded at this location; the viewer's zone is this many minutes east of UTC.
    Started {
        route: Route,
        offset_east_minutes: i32,
    },
    /// Back or forward in the browser.
    LocationChanged(Route),
    /// The browser's clock, sent just before each reply so deadlines can say "overdue".
    Clock {
        now_ms: u64,
    },
    Intent(Intent),
    Edit(Field, String),
    Toggle(Field, bool),
    Replied {
        generation: u64,
        reply: Reply,
    },
    OperationIssued {
        generation: u64,
        slot: OperationSlot,
        result: Result<OperationView, ApiError>,
    },
    /// A delay the app asked for has passed.
    TimerFired {
        generation: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusTarget {
    /// The screen's `h1`, after navigation (screen readers hear the new screen).
    Heading,
    /// The error summary, after a refused form.
    ErrorSummary,
    /// The one-time notice at the top of the page.
    Notice,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Effect {
    Api {
        generation: u64,
        call: ApiCall,
    },
    IssueOperation {
        generation: u64,
        slot: OperationSlot,
    },
    PushUrl(String),
    ReplaceUrl(String),
    /// Leave the app for this address (the hosted checkout page).
    Leave(String),
    /// Abort every request still in flight and clear timers (navigation).
    CancelPending,
    After {
        generation: u64,
        delay_ms: u32,
    },
    Focus(FocusTarget),
    Title(String),
    /// Put this text on the clipboard (the once-shown invitation link).
    CopyText(String),
    CopySecret(crate::wire_integration::SecretReveal),
    /// Download a text file generated in the browser (the statements CSV).
    DownloadCsv {
        filename: String,
        content: String,
    },
}

// One screen at a time in the model; boxing each screen would ripple through all state
// handling for no runtime benefit.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Screen {
    Integration(Box<IntegrationScreen>),
    Booting,
    BootFailed(ApiError),
    Landing,
    SignUp(SignUpForm),
    SignIn(SignInForm),
    VerifyPending(VerifyPending),
    VerifyConfirm(VerifyConfirm),
    Forgot(ForgotForm),
    Reset(ResetForm),
    SignedOut,
    Dashboard(DashboardScreen),
    AddCredit(CreditForm),
    CreditReturn(CreditReturn),
    Funding(FundingScreen),
    Statements(StatementsScreen),
    PlaySessions(PlaySessionsScreen),
    Assets(AssetsScreen),
    Settings(SettingsScreen),
    NewCampaign(WizardScreen),
    Campaign(CampaignScreen),
    Lobby(LobbyScreen),
    Play(PlayScreen),
    Join(JoinScreen),
    Admin(AdminScreen),
    /// Signed in, but the server does not grant this account the operator console.
    NotPermitted,
    /// An operator whose sign-in has not passed the second factor yet.
    NeedsSecondFactor,
    NotFound,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum History {
    Push,
    /// The browser already moved (back or forward).
    Keep,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Model {
    pub route: Route,
    pub session: Remote<SessionView>,
    pub screen: Screen,
    pub generation: u64,
    pub offset_east_minutes: i32,
    /// The browser's clock as of the last message that carried it; views never read a clock.
    pub now_ms: u64,
    pub notice: Option<Notice>,
    after_sign_in: Option<Route>,
    sent_to: Option<String>,
    pub play_now: Submit,
    pub play_operation: Remote<String>,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            route: Route::Landing,
            session: Remote::Loading,
            screen: Screen::Booting,
            generation: 0,
            offset_east_minutes: 0,
            now_ms: 0,
            notice: None,
            after_sign_in: None,
            sent_to: None,
            play_now: Submit::Idle,
            play_operation: Remote::Loading,
        }
    }
}

impl Model {
    pub fn account(&self) -> Option<&AccountView> {
        self.session
            .ready()
            .and_then(|session| session.account.as_ref())
    }

    /// The CSRF token every mutating request carries; empty until the session has loaded.
    pub fn csrf(&self) -> &str {
        self.session
            .ready()
            .map_or("", |session| session.csrf.as_str())
    }

    pub fn update(&mut self, msg: Msg) -> Vec<Effect> {
        match msg {
            Msg::Started {
                route,
                offset_east_minutes,
            } => {
                self.offset_east_minutes = offset_east_minutes;
                // `entry::boot` already removed any token from the address bar.
                self.route = route;
                vec![self.api(ApiCall::Session)]
            }
            Msg::LocationChanged(route) => self.navigate(route, History::Keep),
            Msg::Clock { now_ms } => {
                self.now_ms = now_ms;
                Vec::new()
            }
            Msg::Intent(intent) => self.intent(intent),
            Msg::Edit(field, value) => {
                self.edit(field, value);
                Vec::new()
            }
            Msg::Toggle(field, is_on) => {
                self.toggle(field, is_on);
                Vec::new()
            }
            Msg::Replied { generation, reply } if generation == self.generation => {
                self.reply(reply)
            }
            Msg::OperationIssued {
                generation,
                slot,
                result,
            } if generation == self.generation => {
                self.operation_issued(slot, result.map(|view| view.operation));
                Vec::new()
            }
            Msg::TimerFired { generation } if generation == self.generation => self.timer_fired(),
            Msg::Replied { .. } | Msg::OperationIssued { .. } | Msg::TimerFired { .. } => {
                Vec::new()
            }
        }
    }

    pub(crate) fn api(&self, call: ApiCall) -> Effect {
        Effect::Api {
            generation: self.generation,
            call,
        }
    }

    pub(crate) fn issue_operation(&self, slot: OperationSlot) -> Effect {
        Effect::IssueOperation {
            generation: self.generation,
            slot,
        }
    }

    fn intent(&mut self, intent: Intent) -> Vec<Effect> {
        match intent {
            Intent::Integration(intent) => self.integration_intent(intent),
            Intent::Go(route) => {
                self.notice = None;
                self.navigate(route, History::Push)
            }
            Intent::PlayNow => self.play_now_intent(),
            Intent::Retry => {
                if matches!(self.screen, Screen::BootFailed(_)) {
                    self.screen = Screen::Booting;
                    self.session = Remote::Loading;
                    return vec![self.api(ApiCall::Session)];
                }
                self.navigate(self.route.clone(), History::Keep)
            }
            Intent::SignOut => {
                self.notice = None;
                vec![self.api(ApiCall::SignOut)]
            }
            Intent::UseDevSignIn(index) => {
                self.use_dev_sign_in(index);
                Vec::new()
            }
            Intent::Submit(form) => self.submit(form),
            other => match &self.screen {
                Screen::Admin(_) => self.admin_intent(other),
                _ => self.customer_intent(other),
            },
        }
    }

    fn submit(&mut self, form: Form) -> Vec<Effect> {
        let mut effects = match form {
            Form::SignUp | Form::SignIn | Form::VerifyEmail | Form::Forgot | Form::Reset => {
                self.submit_auth(form)
            }
            Form::Credit
            | Form::Pace
            | Form::Allocate
            | Form::Profile
            | Form::ChangeEmail
            | Form::ChangePassword
            | Form::Preferences
            | Form::Export
            | Form::Close => self.submit_customer(form),
            Form::Search
            | Form::Refund
            | Form::Goodwill
            | Form::Action
            | Form::WizardStep
            | Form::CreateCampaign
            | Form::CampaignSettings
            | Form::CodeLookup => self.submit_customer(form),
        };
        if effects.is_empty() && self.has_refused_form() {
            effects.push(Effect::Focus(FocusTarget::ErrorSummary));
        }
        effects
    }

    fn edit(&mut self, field: Field, value: String) {
        if self.edit_integration(field, &value) {
            return;
        }
        if !self.edit_auth(field, &value) && !self.edit_customer(field, &value) {
            self.edit_admin(field, value);
        }
    }

    fn toggle(&mut self, field: Field, is_on: bool) {
        if let (Screen::Integration(screen), Field::Integration(field)) = (&mut self.screen, field)
        {
            if let IntegrationScreen::Share(s) = &mut **screen {
                if s.submit.is_sending() {
                    return;
                }
                match field {
                    IntegrationField::Indexing => s.indexing = is_on,
                    IntegrationField::Embeds => s.embeds = is_on,
                    _ => {}
                }
            }
            return;
        }
        match (&mut self.screen, field) {
            (Screen::SignUp(form), Field::IsAdult) => form.is_adult = is_on,
            (Screen::SignUp(form), Field::AcceptsTerms) => form.accepts_terms = is_on,
            (Screen::Settings(screen), Field::CloseUnderstood) => screen.close_understood = is_on,
            (Screen::Settings(screen), Field::ProductNews) => screen.product_news = is_on,
            (Screen::Settings(screen), Field::SessionReminders) => {
                screen.session_reminders = is_on;
            }
            (Screen::Funding(screen), Field::AllocateConsent) => {
                if let Some(edit) = screen.allocation.as_mut() {
                    edit.consents_to_host_spending = is_on;
                }
            }
            _ => self.toggle_campaign(field, is_on),
        }
    }

    fn operation_issued(&mut self, slot: OperationSlot, result: Result<String, ApiError>) {
        let operation = Remote::from_result(result);
        match (&mut self.screen, slot) {
            (Screen::SignUp(form), _) => form.operation = operation,
            (Screen::AddCredit(form), _) => form.operation = operation,
            (Screen::Dashboard(_) | Screen::PlaySessions(_), OperationSlot::Primary) => {
                self.play_operation = operation
            }
            (Screen::Dashboard(screen), OperationSlot::Secondary) => {
                screen.join_operation = operation
            }
            (Screen::NewCampaign(screen), _) => screen.operation = operation,
            (Screen::Campaign(screen), _) => screen.operation = operation,
            (Screen::Lobby(screen), _) => screen.operation = operation,
            (Screen::Join(screen), _) => screen.operation = operation,
            (Screen::Funding(screen), OperationSlot::Primary) => {
                if let Some(edit) = screen.edit.as_mut() {
                    edit.operation = operation;
                }
            }
            (Screen::Funding(screen), OperationSlot::Secondary) => {
                if let Some(edit) = screen.allocation.as_mut() {
                    edit.operation = operation;
                }
            }
            (Screen::Settings(screen), OperationSlot::Primary) => {
                screen.profile_operation = operation.clone();
                screen.email_operation = operation.clone();
                screen.preferences_operation = operation.clone();
                screen.export_operation = operation;
            }
            (Screen::Settings(screen), OperationSlot::Secondary) => {
                screen.close_operation = operation;
            }
            (Screen::Admin(screen), slot) => screen.operation_issued(slot, operation),
            _ => {}
        }
    }

    fn reply(&mut self, reply: Reply) -> Vec<Effect> {
        if let Some(error) = reply_error(&reply)
            && error.code == ErrorCode::SignedOut
            && !matches!(reply, Reply::Session(_) | Reply::SignedIn(_))
        {
            return self.session_ended();
        }
        match reply {
            Reply::Integration(answer) => self.integration_reply(answer),
            Reply::Session(result) => self.session_loaded(result),
            Reply::SignedOut(result) => self.signed_out(result),
            reply @ (Reply::SignedUp(_)
            | Reply::SignedIn(_)
            | Reply::Verified(_)
            | Reply::Resent(_)
            | Reply::ForgotSent(_)
            | Reply::Reset(_)) => self.auth_reply(reply),
            reply @ (Reply::AdminSummary(_)
            | Reply::AdminAccounts(_)
            | Reply::AdminAccount(_)
            | Reply::AdminAction(_)
            | Reply::Disputes(_)
            | Reply::Reconciliation(_)
            | Reply::Providers(_)
            | Reply::ReviewQueue(_)
            | Reply::Audit(_)) => self.admin_reply(reply),
            reply => self.customer_reply(reply),
        }
    }

    fn session_loaded(&mut self, result: Result<SessionView, ApiError>) -> Vec<Effect> {
        match result {
            Ok(session) => {
                self.session = Remote::Ready(session);
                if self.screen == Screen::Booting {
                    return self.navigate(self.route.clone(), History::Keep);
                }
                if matches!(
                    &self.screen,
                    Screen::VerifyConfirm(screen)
                        if screen.outcome == Some(crate::wire::VerifyOutcome::Verified)
                ) && self.account().is_some()
                {
                    self.notice = Some(Notice::EmailVerified);
                    return self.navigate(Route::Dashboard, History::Push);
                }
                // A refresh after a change (verification, profile): the screen stays.
                Vec::new()
            }
            Err(_error) if self.screen != Screen::Booting => Vec::new(),
            Err(error) => {
                self.session = Remote::Failed(error);
                self.screen = Screen::BootFailed(error);
                vec![Effect::Focus(FocusTarget::Heading)]
            }
        }
    }

    fn signed_out(&mut self, result: Result<SessionView, ApiError>) -> Vec<Effect> {
        match result {
            Ok(session) => {
                self.session = Remote::Ready(session);
                self.after_sign_in = None;
                self.navigate(Route::SignedOut, History::Push)
            }
            Err(error) if error.code == ErrorCode::SignedOut => self.session_ended(),
            // The cookie may still be valid: stay put and say so, so the person tries again.
            Err(_) => {
                self.notice = Some(Notice::SignOutFailed);
                vec![Effect::Focus(FocusTarget::Notice)]
            }
        }
    }

    /// The server no longer accepts the session: reload it, then ask the person to sign in.
    fn session_ended(&mut self) -> Vec<Effect> {
        self.generation += 1;
        if self.route.audience() != Audience::Anyone {
            self.after_sign_in = Some(self.route.clone());
        }
        self.notice = Some(Notice::SessionEnded);
        self.route = Route::SignIn;
        self.session = Remote::Loading;
        self.screen = Screen::Booting;
        vec![
            Effect::CancelPending,
            Effect::ReplaceUrl(Route::SignIn.path()),
            self.api(ApiCall::Session),
        ]
    }

    fn navigate(&mut self, requested: Route, history: History) -> Vec<Effect> {
        if self.session.ready().is_none() {
            self.route = requested;
            return Vec::new();
        }
        self.generation += 1;
        let route = self.guard(requested.clone());
        let mut effects = vec![Effect::CancelPending];
        match history {
            History::Push => effects.push(Effect::PushUrl(route.path())),
            History::Keep if route != requested || requested.carries_token() => {
                effects.push(Effect::ReplaceUrl(route.path()));
            }
            History::Keep => {}
        }
        self.route = route;
        effects.extend(self.enter());
        effects.push(Effect::Title(crate::copy::title(&self.screen)));
        effects.push(Effect::Focus(FocusTarget::Heading));
        effects
    }

    /// Picks the screen a person may see here; access itself is the server's decision.
    fn guard(&mut self, route: Route) -> Route {
        let account = self.account();
        match (route.audience(), account) {
            (Audience::SignedOut, Some(_)) => Route::Dashboard,
            (Audience::Anyone, Some(_)) if route == Route::Landing => Route::Dashboard,
            (Audience::Customer | Audience::Operator, None) => {
                self.after_sign_in = Some(route);
                if self.notice.is_none() {
                    self.notice = Some(Notice::SignInToContinue);
                }
                Route::SignIn
            }
            _ => route,
        }
    }

    /// Builds the current route's screen and asks for what it needs.
    fn enter(&mut self) -> Vec<Effect> {
        let route = self.route.clone();
        match route {
            Route::DeveloperTokens | Route::Share { .. } => self.enter_integration(route),
            Route::GoogleWelcome => {
                self.notice = Some(Notice::EmailVerified);
                self.route = Route::Dashboard;
                self.enter_customer(Route::Dashboard)
            }
            Route::Landing => {
                self.screen = Screen::Landing;
                Vec::new()
            }
            Route::SignedOut => {
                self.screen = Screen::SignedOut;
                Vec::new()
            }
            Route::NotFound => {
                self.screen = Screen::NotFound;
                Vec::new()
            }
            Route::SignUp
            | Route::SignIn
            | Route::VerifyEmail { .. }
            | Route::ForgotPassword
            | Route::ResetPassword { .. } => self.enter_auth(route),
            Route::Admin(admin) => {
                let Some(operator) = self.account().and_then(|account| account.operator.as_ref())
                else {
                    self.screen = Screen::NotPermitted;
                    return Vec::new();
                };
                if !operator.has_second_factor {
                    self.screen = Screen::NeedsSecondFactor;
                    return Vec::new();
                }
                self.enter_admin(admin)
            }
            Route::Dashboard
            | Route::AddCredit
            | Route::CreditReturn { .. }
            | Route::Funding
            | Route::Statements
            | Route::PlaySessions
            | Route::Assets
            | Route::Settings
            | Route::Security
            | Route::NewCampaign
            | Route::Campaign { .. }
            | Route::Lobby { .. }
            | Route::Play { .. }
            | Route::Join { .. } => self.enter_customer(route),
        }
    }

    /// Consumes the one-time notice when a screen shows it.
    pub fn take_notice(&mut self) -> Option<Notice> {
        self.notice.take()
    }

    fn has_refused_form(&self) -> bool {
        crate::copy::refused_form_error(self).is_some()
    }
}

fn reply_error(reply: &Reply) -> Option<ApiError> {
    fn of<T>(result: &Result<T, ApiError>) -> Option<ApiError> {
        result.as_ref().err().copied()
    }
    match reply {
        Reply::Integration(answer) => answer.error(),
        Reply::Session(result) | Reply::SignedIn(result) | Reply::SignedOut(result) => of(result),
        Reply::Operation(result) => of(result),
        Reply::SignedUp(result)
        | Reply::Resent(result)
        | Reply::ForgotSent(result)
        | Reply::ProfileSaved(result) => of(result),
        Reply::Verified(result) => of(result),
        Reply::Reset(result) => of(result),
        Reply::Overview(result) => of(result),
        Reply::CreditOffers(result) => of(result),
        Reply::CheckoutCreated(result) => of(result),
        Reply::Checkout(result) => of(result),
        Reply::Funding(result) | Reply::PaceSaved(result) | Reply::Allocated(result) => of(result),
        Reply::Statements { result, .. } => of(result),
        Reply::AccountStats(result) => of(result),
        Reply::PlaySessions { result, .. } => of(result),
        Reply::Assets { result, .. } => of(result),
        Reply::EmailChangeSent(result) => of(result),
        Reply::PasswordChanged(result) => of(result),
        Reply::PreferencesSaved(result) => of(result),
        Reply::ExportRequested(result) => of(result),
        Reply::Settings(result) => of(result),
        Reply::Revoked(result) => of(result),
        Reply::Closed(result) => of(result),
        Reply::AdminSummary(result) => of(result),
        Reply::AdminAccounts(result) => of(result),
        Reply::AdminAccount(result) => of(result),
        Reply::AdminAction(result) => of(result),
        Reply::Disputes(result) => of(result),
        Reply::Reconciliation(result) => of(result),
        Reply::Providers(result) => of(result),
        Reply::ReviewQueue(result) => of(result),
        Reply::Audit(result) => of(result),
        Reply::Dashboard(result) => of(result),
        Reply::PlayNow(result) => of(result),
        Reply::CampaignSettings(result) => of(result),
        Reply::WizardCatalog(result) => of(result),
        Reply::Draft(result) => of(result),
        Reply::CampaignCreated(result) => of(result),
        Reply::Campaign(result) => of(result),
        Reply::JoinPending(result) => of(result),
        Reply::InvitationCreated(result) => of(result),
        Reply::Left(result) | Reply::JoinHeld(result) => of(result),
        Reply::LobbyOpened(result) => of(result),
        Reply::Lobby(result) => of(result),
        Reply::GameLaunch(result) => of(result),
        Reply::LobbyCancelled(result) => of(result),
        Reply::JoinPreview(result) => of(result),
        Reply::Joined(result) => of(result),
        Reply::CodeRequested(result) => of(result),
    }
}

#[cfg(test)]
mod state_tests;
