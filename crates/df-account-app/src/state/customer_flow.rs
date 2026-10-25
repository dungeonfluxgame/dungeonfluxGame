//! The customer dashboard: balance and trial (with S1's campaign list), add credit and the payment
//! return, pace presets, statements and settings (plan 9.4, 9.7-9.11, J14; decision 26).

use super::{
    Effect, Field, FocusTarget, Form, Intent, Model, Notice, OperationSlot, Remote, Screen, Submit,
};
use crate::{
    api::{ApiCall, Reply},
    money::parse_dollars,
    route::Route,
    wire::{
        AccountStats, AllocateRequest, ApiError, AssetKind, AssetRow, AssetsView,
        ChangeEmailRequest, ChangePasswordRequest, CheckoutStatus, CheckoutView,
        CloseAccountRequest, CreateCheckoutRequest, CreditOffers, EmailPreferencesRequest,
        ErrorCode, ExportRequest, ExportView, FundingView, OverviewView, PacePreset,
        PlaySessionRow, ProfileRequest, Refusal, RevokeSessionRequest, SetPaceRequest,
        SettingsView, StatementFilter, StatementLine,
    },
    wire_campaign::{DashboardView, PendingInvitation},
};

/// Statement lines kept on screen; older pages stop loading at this bound.
pub const MAXIMUM_STATEMENT_LINES: usize = 500;
/// Waits between payment-status reads after returning from checkout (plan 9.8: backoff).
pub const CHECKOUT_POLL_DELAYS_MS: [u32; 8] =
    [1_000, 2_000, 3_000, 5_000, 8_000, 10_000, 10_000, 10_000];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DashboardScreen {
    pub overview: Remote<OverviewView>,
    pub campaigns: Remote<DashboardView>,
    /// The invitation remembered in the join cookie, offered after signing in (S1 P-3).
    pub pending: Remote<PendingInvitation>,
    pub resend: Submit,
    pub is_resent: bool,
    pub join_operation: Remote<String>,
    pub join: Submit,
    pub stats: Remote<AccountStats>,
    pub latest_assets: Remote<AssetsView>,
    pub topups: Remote<Vec<StatementLine>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlaySessionsScreen {
    pub sessions: Remote<Vec<PlaySessionRow>>,
    pub next_cursor: Option<String>,
    pub more: Submit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetsScreen {
    pub assets: Remote<Vec<AssetRow>>,
    pub next_cursor: Option<String>,
    pub total: u64,
    pub kind: Option<AssetKind>,
    pub campaign: String,
    pub more: Submit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AmountChoice {
    Preset(u32),
    Custom,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditForm {
    pub offers: Remote<CreditOffers>,
    pub choice: Option<AmountChoice>,
    pub custom: String,
    pub operation: Remote<String>,
    pub submit: Submit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditReturn {
    pub checkout_id: Option<String>,
    pub status: Remote<CheckoutView>,
    /// Status reads made so far; past the last delay the screen offers "Check again".
    pub polls: usize,
}

impl CreditReturn {
    pub fn has_stopped_waiting(&self) -> bool {
        self.polls >= CHECKOUT_POLL_DELAYS_MS.len()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaceEdit {
    pub campaign_id: String,
    pub preset: PacePreset,
    pub custom: String,
    pub operation: Remote<String>,
    pub submit: Submit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllocateEdit {
    pub session_id: String,
    pub amount: String,
    pub consents_to_host_spending: bool,
    pub operation: Remote<String>,
    pub submit: Submit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FundingScreen {
    pub funding: Remote<FundingView>,
    pub edit: Option<PaceEdit>,
    pub allocation: Option<AllocateEdit>,
    pub saved_campaign: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatementsScreen {
    pub lines: Remote<Vec<StatementLine>>,
    pub next_cursor: Option<String>,
    pub more: Submit,
    pub filter: StatementFilter,
    pub from_text: String,
    pub to_text: String,
    pub totals: Option<crate::wire::StatementTotals>,
    pub monthly: Vec<crate::wire::MonthlySummary>,
    pub per_campaign: Vec<crate::wire::CampaignSpend>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsScreen {
    pub settings: Remote<SettingsView>,
    pub display_name: String,
    pub profile_operation: Remote<String>,
    pub profile: Submit,
    pub is_profile_saved: bool,
    pub revoke: Submit,
    pub revoked: Option<u64>,
    pub close_password: String,
    pub close_understood: bool,
    pub close_operation: Remote<String>,
    pub close: Submit,
    pub new_email: String,
    pub email_password: String,
    pub email_operation: Remote<String>,
    pub email_change: Submit,
    pub is_email_change_sent: bool,
    pub current_password: String,
    pub new_password: String,
    pub new_confirmation: String,
    pub password_change: Submit,
    pub is_password_changed: bool,
    pub product_news: bool,
    pub session_reminders: bool,
    pub preferences_operation: Remote<String>,
    pub preferences: Submit,
    pub is_preferences_saved: bool,
    pub export_password: String,
    pub export_operation: Remote<String>,
    pub export: Submit,
    pub export_view: Option<ExportView>,
}

/// Parses a `YYYY-MM-DD` date the person typed into epoch ms (start or end of that day, UTC).
pub fn parse_date_ms(text: &str, end_of_day: bool) -> Option<u64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let (year, rest) = text.split_once('-')?;
    let (month, day) = rest.split_once('-')?;
    let year: i64 = year.parse().ok()?;
    let month: i64 = month.parse().ok()?;
    let day: i64 = day.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Days from civil (Howard Hinnant's algorithm), inverse of the fake's month helper.
    let shifted_year = if month <= 2 { year - 1 } else { year };
    let era = shifted_year.div_euclid(400);
    let year_of_era = shifted_year.rem_euclid(400);
    let month_prime = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    let start_ms = u64::try_from(days).ok()? * 86_400_000;
    Some(if end_of_day {
        start_ms + 86_400_000 - 1
    } else {
        start_ms
    })
}

fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

/// The filtered statement lines as CSV, generated in the browser from what the server sent.
pub fn statements_csv(lines: &[StatementLine]) -> String {
    let mut csv = String::from("when,what,campaign,direction,amount_micro_dollars,status\n");
    for line in lines {
        csv.push_str(&format!(
            "{},{},{},{},{},{}\n",
            line.at_ms,
            csv_field(crate::copy::category_name(line.category)),
            csv_field(line.campaign_name.as_deref().unwrap_or("")),
            match line.direction {
                crate::wire::Direction::Credit => "credit",
                crate::wire::Direction::Charge => "charge",
            },
            line.amount_micro_dollars,
            csv_field(crate::copy::line_status(line.status)),
        ));
    }
    csv
}

pub(super) fn preset_from_name(name: &str) -> Option<PacePreset> {
    [
        PacePreset::Story,
        PacePreset::Immersive,
        PacePreset::Cinematic,
        PacePreset::Custom,
    ]
    .into_iter()
    .find(|preset| crate::copy::preset_value(*preset) == name)
}

/// Only our own pages or the processor's hosted checkout may receive the browser.
fn is_checkout_destination(url: &str) -> bool {
    (url.starts_with('/') && !url.starts_with("//"))
        || url.starts_with("https://checkout.stripe.com/")
}

impl Model {
    pub(super) fn enter_customer(&mut self, route: Route) -> Vec<Effect> {
        match route {
            Route::Dashboard => {
                self.screen = Screen::Dashboard(DashboardScreen {
                    overview: Remote::Loading,
                    campaigns: Remote::Loading,
                    pending: Remote::Loading,
                    resend: Submit::Idle,
                    is_resent: false,
                    join_operation: Remote::Loading,
                    join: Submit::Idle,
                    stats: Remote::Loading,
                    latest_assets: Remote::Loading,
                    topups: Remote::Loading,
                });
                vec![
                    self.api(ApiCall::Overview),
                    self.api(ApiCall::Dashboard),
                    self.api(ApiCall::JoinPending),
                    self.api(ApiCall::AccountStats),
                    self.api(ApiCall::Assets {
                        kind: None,
                        campaign: None,
                        cursor: None,
                    }),
                    self.api(ApiCall::Statements {
                        cursor: None,
                        filter: StatementFilter::TopUps,
                        from_ms: None,
                        to_ms: None,
                    }),
                ]
            }
            Route::AddCredit => {
                self.screen = Screen::AddCredit(CreditForm {
                    offers: Remote::Loading,
                    choice: None,
                    custom: String::new(),
                    operation: Remote::Loading,
                    submit: Submit::Idle,
                });
                vec![
                    self.api(ApiCall::CreditOffers),
                    self.issue_operation(OperationSlot::Primary),
                ]
            }
            Route::CreditReturn { checkout } => {
                let effects = match &checkout {
                    Some(checkout_id) => vec![self.api(ApiCall::Checkout {
                        checkout_id: checkout_id.clone(),
                    })],
                    None => Vec::new(),
                };
                self.screen = Screen::CreditReturn(CreditReturn {
                    status: if checkout.is_some() {
                        Remote::Loading
                    } else {
                        Remote::Failed(ApiError::of(ErrorCode::NotFound))
                    },
                    checkout_id: checkout,
                    polls: 0,
                });
                effects
            }
            Route::Funding => {
                self.screen = Screen::Funding(FundingScreen {
                    funding: Remote::Loading,
                    edit: None,
                    allocation: None,
                    saved_campaign: None,
                });
                vec![self.api(ApiCall::Funding)]
            }
            Route::Statements => {
                self.screen = Screen::Statements(StatementsScreen {
                    lines: Remote::Loading,
                    next_cursor: None,
                    more: Submit::Idle,
                    filter: StatementFilter::All,
                    from_text: String::new(),
                    to_text: String::new(),
                    totals: None,
                    monthly: Vec::new(),
                    per_campaign: Vec::new(),
                });
                vec![self.api(ApiCall::Statements {
                    cursor: None,
                    filter: StatementFilter::All,
                    from_ms: None,
                    to_ms: None,
                })]
            }
            Route::PlaySessions => {
                self.screen = Screen::PlaySessions(PlaySessionsScreen {
                    sessions: Remote::Loading,
                    next_cursor: None,
                    more: Submit::Idle,
                });
                vec![self.api(ApiCall::PlaySessions { cursor: None })]
            }
            Route::Assets => {
                self.screen = Screen::Assets(AssetsScreen {
                    assets: Remote::Loading,
                    next_cursor: None,
                    total: 0,
                    kind: None,
                    campaign: String::new(),
                    more: Submit::Idle,
                });
                vec![self.api(ApiCall::Assets {
                    kind: None,
                    campaign: None,
                    cursor: None,
                })]
            }
            Route::Settings | Route::Security => {
                self.screen = Screen::Settings(SettingsScreen {
                    settings: Remote::Loading,
                    display_name: String::new(),
                    profile_operation: Remote::Loading,
                    profile: Submit::Idle,
                    is_profile_saved: false,
                    revoke: Submit::Idle,
                    revoked: None,
                    close_password: String::new(),
                    close_understood: false,
                    close_operation: Remote::Loading,
                    close: Submit::Idle,
                    new_email: String::new(),
                    email_password: String::new(),
                    email_operation: Remote::Loading,
                    email_change: Submit::Idle,
                    is_email_change_sent: false,
                    current_password: String::new(),
                    new_password: String::new(),
                    new_confirmation: String::new(),
                    password_change: Submit::Idle,
                    is_password_changed: false,
                    product_news: false,
                    session_reminders: true,
                    preferences_operation: Remote::Loading,
                    preferences: Submit::Idle,
                    is_preferences_saved: false,
                    export_password: String::new(),
                    export_operation: Remote::Loading,
                    export: Submit::Idle,
                    export_view: None,
                });
                vec![
                    self.api(ApiCall::Settings),
                    self.issue_operation(OperationSlot::Primary),
                    self.issue_operation(OperationSlot::Secondary),
                ]
            }
            Route::NewCampaign
            | Route::Campaign { .. }
            | Route::Lobby { .. }
            | Route::Join { .. } => self.enter_campaign(route),
            _ => Vec::new(),
        }
    }

    pub(super) fn customer_intent(&mut self, intent: Intent) -> Vec<Effect> {
        match (intent, &mut self.screen) {
            (Intent::ResendVerification, _) => self.resend_verification(),
            (Intent::ChooseAmount(cents), Screen::AddCredit(form)) => {
                form.choice = Some(AmountChoice::Preset(cents));
                form.submit = Submit::Idle;
                Vec::new()
            }
            (Intent::ChooseCustomAmount, Screen::AddCredit(form)) => {
                form.choice = Some(AmountChoice::Custom);
                form.submit = Submit::Idle;
                Vec::new()
            }
            (Intent::EditPace { campaign_id }, Screen::Funding(screen)) => {
                let Some(campaign) = screen.funding.ready().and_then(|funding| {
                    funding
                        .campaigns
                        .iter()
                        .find(|campaign| campaign.campaign_id == campaign_id)
                }) else {
                    return Vec::new();
                };
                if !campaign.can_choose_pace {
                    return Vec::new();
                }
                screen.edit = Some(PaceEdit {
                    preset: campaign.preset,
                    custom: crate::money::cents(u64::from(campaign.ceiling_cents_per_hour))
                        .trim_start_matches('$')
                        .to_owned(),
                    campaign_id,
                    operation: Remote::Loading,
                    submit: Submit::Idle,
                });
                screen.saved_campaign = None;
                vec![self.issue_operation(OperationSlot::Primary)]
            }
            (Intent::LoadOlderStatements, Screen::Statements(screen)) => {
                let Some(cursor) = screen.next_cursor.clone() else {
                    return Vec::new();
                };
                if screen.more.is_sending() {
                    return Vec::new();
                }
                screen.more = Submit::Sending;
                let (filter, from_text, to_text) = (
                    screen.filter,
                    screen.from_text.clone(),
                    screen.to_text.clone(),
                );
                vec![self.api(ApiCall::Statements {
                    cursor: Some(cursor),
                    filter,
                    from_ms: parse_date_ms(&from_text, false),
                    to_ms: parse_date_ms(&to_text, true),
                })]
            }
            (Intent::SetStatementFilter(filter), Screen::Statements(screen)) => {
                screen.filter = filter;
                screen.lines = Remote::Loading;
                screen.next_cursor = None;
                let (from_text, to_text) = (screen.from_text.clone(), screen.to_text.clone());
                vec![self.api(ApiCall::Statements {
                    cursor: None,
                    filter,
                    from_ms: parse_date_ms(&from_text, false),
                    to_ms: parse_date_ms(&to_text, true),
                })]
            }
            (Intent::DownloadStatementsCsv, Screen::Statements(screen)) => {
                let Some(lines) = screen.lines.ready() else {
                    return Vec::new();
                };
                vec![Effect::DownloadCsv {
                    filename: "dungeonflux-statements.csv".to_owned(),
                    content: statements_csv(lines),
                }]
            }
            (Intent::LoadOlderPlaySessions, Screen::PlaySessions(screen)) => {
                let Some(cursor) = screen.next_cursor.clone() else {
                    return Vec::new();
                };
                if screen.more.is_sending() {
                    return Vec::new();
                }
                screen.more = Submit::Sending;
                vec![self.api(ApiCall::PlaySessions {
                    cursor: Some(cursor),
                })]
            }
            (Intent::SetAssetKind(kind), Screen::Assets(screen)) => {
                screen.kind = kind;
                screen.assets = Remote::Loading;
                screen.next_cursor = None;
                let campaign = (!screen.campaign.is_empty()).then(|| screen.campaign.clone());
                vec![self.api(ApiCall::Assets {
                    kind,
                    campaign,
                    cursor: None,
                })]
            }
            (Intent::LoadOlderAssets, Screen::Assets(screen)) => {
                let Some(cursor) = screen.next_cursor.clone() else {
                    return Vec::new();
                };
                if screen.more.is_sending() {
                    return Vec::new();
                }
                screen.more = Submit::Sending;
                let (kind, campaign) = (
                    screen.kind,
                    (!screen.campaign.is_empty()).then(|| screen.campaign.clone()),
                );
                vec![self.api(ApiCall::Assets {
                    kind,
                    campaign,
                    cursor: Some(cursor),
                })]
            }
            (Intent::RevokeSession { session_id }, Screen::Settings(screen)) => {
                if screen.revoke.is_sending() {
                    return Vec::new();
                }
                screen.revoke = Submit::Sending;
                vec![self.api(ApiCall::RevokeSession(RevokeSessionRequest { session_id }))]
            }
            (Intent::RevokeAllSessions, Screen::Settings(screen)) => {
                if screen.revoke.is_sending() {
                    return Vec::new();
                }
                screen.revoke = Submit::Sending;
                vec![self.api(ApiCall::RevokeAllSessions)]
            }
            (Intent::RevokeOtherSessions, Screen::Settings(screen)) => {
                if screen.revoke.is_sending() {
                    return Vec::new();
                }
                screen.revoke = Submit::Sending;
                vec![self.api(ApiCall::RevokeOtherSessions)]
            }
            (Intent::EditAllocation { session_id }, Screen::Funding(screen)) => {
                let has_session = screen.funding.ready().is_some_and(|funding| {
                    funding.campaigns.iter().any(|campaign| {
                        campaign
                            .session
                            .as_ref()
                            .is_some_and(|session| session.session_id == session_id)
                    })
                });
                if !has_session {
                    return Vec::new();
                }
                screen.allocation = Some(AllocateEdit {
                    session_id,
                    amount: String::new(),
                    consents_to_host_spending: false,
                    operation: Remote::Loading,
                    submit: Submit::Idle,
                });
                screen.saved_campaign = None;
                vec![self.issue_operation(OperationSlot::Secondary)]
            }
            (Intent::CancelEdit, Screen::Funding(screen)) => {
                screen.edit = None;
                screen.allocation = None;
                Vec::new()
            }
            (intent, _) => self.campaign_intent(intent),
        }
    }

    pub(super) fn edit_customer(&mut self, field: Field, value: &str) -> bool {
        match (&mut self.screen, field) {
            (Screen::AddCredit(form), Field::CustomAmount) => {
                value.clone_into(&mut form.custom);
                form.choice = Some(AmountChoice::Custom);
            }
            (Screen::Funding(screen), Field::PacePreset) => {
                if let (Some(edit), Some(preset)) = (screen.edit.as_mut(), preset_from_name(value))
                {
                    edit.preset = preset;
                }
            }
            (Screen::Funding(screen), Field::CustomCeiling) => {
                if let Some(edit) = screen.edit.as_mut() {
                    value.clone_into(&mut edit.custom);
                }
            }
            (Screen::Funding(screen), Field::AllocateAmount) => {
                if let Some(edit) = screen.allocation.as_mut() {
                    value.clone_into(&mut edit.amount);
                }
            }
            (Screen::Settings(screen), Field::DisplayName) => {
                value.clone_into(&mut screen.display_name);
                screen.is_profile_saved = false;
            }
            (Screen::Settings(screen), Field::NewEmail) => {
                value.clone_into(&mut screen.new_email);
                screen.is_email_change_sent = false;
            }
            (Screen::Settings(screen), Field::ClosePassword) => {
                value.clone_into(&mut screen.close_password);
            }
            (Screen::Settings(screen), Field::EmailPassword) => {
                value.clone_into(&mut screen.email_password);
            }
            (Screen::Settings(screen), Field::ExportPassword) => {
                value.clone_into(&mut screen.export_password);
            }
            (Screen::Settings(screen), Field::CurrentPassword) => {
                value.clone_into(&mut screen.current_password);
            }
            (Screen::Settings(screen), Field::Password) => {
                value.clone_into(&mut screen.new_password);
            }
            (Screen::Settings(screen), Field::Confirmation) => {
                value.clone_into(&mut screen.new_confirmation);
            }
            (Screen::Statements(screen), Field::StatementFrom) => {
                value.clone_into(&mut screen.from_text);
            }
            (Screen::Statements(screen), Field::StatementTo) => {
                value.clone_into(&mut screen.to_text);
            }
            (Screen::Statements(screen), Field::AssetKindFilter) => {
                screen.filter = match value {
                    "top_ups" => StatementFilter::TopUps,
                    "spend" => StatementFilter::Spend,
                    _ => StatementFilter::All,
                };
            }
            (Screen::Assets(screen), Field::AssetCampaignFilter) => {
                value.clone_into(&mut screen.campaign);
            }
            (Screen::Assets(screen), Field::AssetKindFilter) => {
                screen.kind = match value {
                    "portrait" => Some(AssetKind::Portrait),
                    "scene" => Some(AssetKind::Scene),
                    "music" => Some(AssetKind::Music),
                    "video" => Some(AssetKind::Video),
                    _ => None,
                };
            }
            _ => return self.edit_campaign(field, value),
        }
        true
    }

    pub(super) fn submit_customer(&mut self, form: Form) -> Vec<Effect> {
        let generation = self.generation;
        let call = match (&mut self.screen, form) {
            (Screen::AddCredit(form), Form::Credit) => {
                let Some(offers) = form.offers.ready() else {
                    return Vec::new();
                };
                let Some(operation) = form.operation.ready().cloned() else {
                    return Vec::new();
                };
                if form.submit.is_sending() {
                    return Vec::new();
                }
                let amount_cents = match form.choice {
                    Some(AmountChoice::Preset(cents)) => Some(cents),
                    Some(AmountChoice::Custom) => {
                        parse_dollars(&form.custom).ok().filter(|cents| {
                            (offers.custom_minimum_cents..=offers.custom_maximum_cents)
                                .contains(cents)
                        })
                    }
                    None => None,
                };
                let Some(amount_cents) = amount_cents else {
                    form.submit = Submit::Refused(ApiError::refused(Refusal::AmountOutOfRange));
                    return Vec::new();
                };
                form.submit = Submit::Sending;
                ApiCall::CreateCheckout(CreateCheckoutRequest {
                    amount_cents,
                    operation,
                })
            }
            (Screen::Funding(screen), Form::Pace) => {
                let Some(funding) = screen.funding.ready() else {
                    return Vec::new();
                };
                let Some(edit) = screen.edit.as_mut() else {
                    return Vec::new();
                };
                let Some(operation) = edit.operation.ready().cloned() else {
                    return Vec::new();
                };
                if edit.submit.is_sending() {
                    return Vec::new();
                }
                let custom_ceiling_cents_per_hour = if edit.preset == PacePreset::Custom {
                    let range = funding.custom_minimum_cents_per_hour
                        ..=funding.custom_maximum_cents_per_hour;
                    match parse_dollars(&edit.custom) {
                        Ok(cents) if range.contains(&cents) => Some(cents),
                        _ => {
                            edit.submit =
                                Submit::Refused(ApiError::refused(Refusal::CeilingOutOfRange));
                            return Vec::new();
                        }
                    }
                } else {
                    None
                };
                edit.submit = Submit::Sending;
                ApiCall::SetPace(SetPaceRequest {
                    campaign_id: edit.campaign_id.clone(),
                    preset: edit.preset,
                    custom_ceiling_cents_per_hour,
                    operation,
                })
            }
            (Screen::Funding(screen), Form::Allocate) => {
                let Some(edit) = screen.allocation.as_mut() else {
                    return Vec::new();
                };
                let Some(operation) = edit.operation.ready().cloned() else {
                    return Vec::new();
                };
                if edit.submit.is_sending() {
                    return Vec::new();
                }
                let Some(amount_cents) =
                    parse_dollars(&edit.amount).ok().filter(|cents| *cents > 0)
                else {
                    edit.submit = Submit::Refused(ApiError::refused(Refusal::AmountOutOfRange));
                    return vec![Effect::Focus(FocusTarget::ErrorSummary)];
                };
                if !edit.consents_to_host_spending {
                    edit.submit = Submit::Refused(ApiError::refused(Refusal::ConsentRequired));
                    return vec![Effect::Focus(FocusTarget::ErrorSummary)];
                }
                edit.submit = Submit::Sending;
                ApiCall::Allocate(AllocateRequest {
                    session_id: edit.session_id.clone(),
                    amount_cents,
                    consents_to_host_spending: true,
                    operation,
                })
            }
            (Screen::Settings(screen), Form::Profile) => {
                let Some(operation) = screen.profile_operation.ready().cloned() else {
                    return Vec::new();
                };
                if screen.profile.is_sending() {
                    return Vec::new();
                }
                screen.profile = Submit::Sending;
                ApiCall::SaveProfile(ProfileRequest {
                    display_name: screen.display_name.clone(),
                    operation,
                })
            }
            (Screen::Settings(screen), Form::ChangeEmail) => {
                let Some(operation) = screen.email_operation.ready().cloned() else {
                    return Vec::new();
                };
                if screen.email_change.is_sending() {
                    return Vec::new();
                }
                if screen.new_email.trim().is_empty() {
                    screen.email_change = Submit::Refused(ApiError::refused(Refusal::EmailEmpty));
                    return Vec::new();
                }
                screen.email_change = Submit::Sending;
                ApiCall::ChangeEmail(ChangeEmailRequest {
                    new_email: screen.new_email.trim().to_owned(),
                    password: screen.email_password.clone(),
                    operation,
                })
            }
            (Screen::Settings(screen), Form::ChangePassword) => {
                if screen.password_change.is_sending() {
                    return Vec::new();
                }
                if screen.new_password != screen.new_confirmation {
                    screen.password_change =
                        Submit::Refused(ApiError::refused(Refusal::PasswordMismatch));
                    return Vec::new();
                }
                if screen.new_password.is_empty() {
                    screen.password_change =
                        Submit::Refused(ApiError::refused(Refusal::PasswordTooShort));
                    return Vec::new();
                }
                screen.password_change = Submit::Sending;
                ApiCall::ChangePassword(ChangePasswordRequest {
                    current_password: screen.current_password.clone(),
                    password: screen.new_password.clone(),
                    confirmation: screen.new_confirmation.clone(),
                })
            }
            (Screen::Settings(screen), Form::Preferences) => {
                let Some(operation) = screen.preferences_operation.ready().cloned() else {
                    return Vec::new();
                };
                if screen.preferences.is_sending() {
                    return Vec::new();
                }
                screen.preferences = Submit::Sending;
                ApiCall::SavePreferences(EmailPreferencesRequest {
                    product_news: screen.product_news,
                    session_reminders: screen.session_reminders,
                    operation,
                })
            }
            (Screen::Settings(screen), Form::Export) => {
                let Some(operation) = screen.export_operation.ready().cloned() else {
                    return Vec::new();
                };
                if screen.export.is_sending() {
                    return Vec::new();
                }
                screen.export = Submit::Sending;
                ApiCall::RequestExport(ExportRequest {
                    password: screen.export_password.clone(),
                    operation,
                })
            }
            (Screen::Settings(screen), Form::Close) => {
                let Some(operation) = screen.close_operation.ready().cloned() else {
                    return Vec::new();
                };
                if screen.close.is_sending() {
                    return Vec::new();
                }
                if !screen.close_understood {
                    screen.close =
                        Submit::Refused(ApiError::refused(Refusal::ConfirmationRequired));
                    return Vec::new();
                }
                screen.close = Submit::Sending;
                ApiCall::CloseAccount(CloseAccountRequest {
                    password: screen.close_password.clone(),
                    operation,
                })
            }
            (_, form) => return self.submit_campaign(form),
        };
        vec![Effect::Api { generation, call }]
    }

    pub(super) fn customer_reply(&mut self, reply: Reply) -> Vec<Effect> {
        let generation = self.generation;
        match (reply, &mut self.screen) {
            (Reply::Overview(result), Screen::Dashboard(screen)) => {
                screen.overview = Remote::from_result(result);
                Vec::new()
            }
            (Reply::Dashboard(result), Screen::Dashboard(screen)) => {
                screen.campaigns = Remote::from_result(result);
                Vec::new()
            }
            (Reply::CreditOffers(result), Screen::AddCredit(form)) => {
                form.offers = Remote::from_result(result);
                Vec::new()
            }
            (Reply::CheckoutCreated(result), Screen::AddCredit(form)) => match result {
                Ok(created) if is_checkout_destination(&created.redirect_url) => {
                    vec![Effect::Leave(created.redirect_url)]
                }
                Ok(_) => {
                    form.submit = Submit::Refused(ApiError::of(ErrorCode::Unavailable));
                    vec![Effect::Focus(FocusTarget::ErrorSummary)]
                }
                Err(error) => {
                    form.submit = Submit::Refused(error);
                    vec![Effect::Focus(FocusTarget::ErrorSummary)]
                }
            },
            (Reply::Checkout(result), Screen::CreditReturn(screen)) => {
                let is_pending =
                    matches!(&result, Ok(view) if view.status == CheckoutStatus::Pending);
                screen.status = Remote::from_result(result);
                match CHECKOUT_POLL_DELAYS_MS.get(screen.polls) {
                    Some(delay_ms) if is_pending => {
                        screen.polls += 1;
                        vec![Effect::After {
                            generation,
                            delay_ms: *delay_ms,
                        }]
                    }
                    _ => {
                        if is_pending {
                            screen.polls = CHECKOUT_POLL_DELAYS_MS.len();
                        }
                        Vec::new()
                    }
                }
            }
            (Reply::Funding(result), Screen::Funding(screen)) => {
                screen.funding = Remote::from_result(result);
                Vec::new()
            }
            (Reply::Allocated(result), Screen::Funding(screen)) => match result {
                Ok(funding) => {
                    screen.saved_campaign = screen.allocation.take().and_then(|edit| {
                        funding.campaigns.iter().find_map(|campaign| {
                            let session = campaign.session.as_ref()?;
                            (session.session_id == edit.session_id)
                                .then(|| campaign.campaign_id.clone())
                        })
                    });
                    screen.funding = Remote::Ready(funding);
                    Vec::new()
                }
                Err(error) => {
                    if let Some(edit) = screen.allocation.as_mut() {
                        edit.submit = Submit::Refused(error);
                    }
                    vec![Effect::Focus(FocusTarget::ErrorSummary)]
                }
            },
            (Reply::PaceSaved(result), Screen::Funding(screen)) => match result {
                Ok(funding) => {
                    screen.saved_campaign = screen.edit.take().map(|edit| edit.campaign_id);
                    screen.funding = Remote::Ready(funding);
                    Vec::new()
                }
                Err(error) => {
                    if let Some(edit) = screen.edit.as_mut() {
                        edit.submit = Submit::Refused(error);
                    }
                    vec![Effect::Focus(FocusTarget::ErrorSummary)]
                }
            },
            (
                Reply::Statements {
                    is_continuation,
                    result,
                },
                Screen::Statements(screen),
            ) => {
                match result {
                    Ok(page) => {
                        if !is_continuation {
                            screen.totals = Some(page.totals);
                            screen.monthly = page.monthly.clone();
                            screen.per_campaign = page.per_campaign.clone();
                        }
                        let mut lines = match (is_continuation, &mut screen.lines) {
                            (true, Remote::Ready(lines)) => std::mem::take(lines),
                            _ => Vec::new(),
                        };
                        lines.extend(page.lines);
                        lines.truncate(MAXIMUM_STATEMENT_LINES);
                        screen.next_cursor = page
                            .next_cursor
                            .filter(|_| lines.len() < MAXIMUM_STATEMENT_LINES);
                        screen.lines = Remote::Ready(lines);
                        screen.more = Submit::Idle;
                    }
                    Err(error) if is_continuation => screen.more = Submit::Refused(error),
                    Err(error) => screen.lines = Remote::Failed(error),
                }
                Vec::new()
            }
            (Reply::AccountStats(result), Screen::Dashboard(screen)) => {
                screen.stats = Remote::from_result(result);
                Vec::new()
            }
            (Reply::AccountStats(result), Screen::Settings(_)) => {
                let _ = result;
                Vec::new()
            }
            (
                Reply::Assets {
                    is_continuation,
                    result,
                },
                Screen::Dashboard(screen),
            ) => {
                if !is_continuation {
                    screen.latest_assets = Remote::from_result(result);
                }
                Vec::new()
            }
            (
                Reply::Statements {
                    is_continuation: false,
                    result: Ok(page),
                },
                Screen::Dashboard(screen),
            ) if page
                .lines
                .iter()
                .all(|line| line.direction == crate::wire::Direction::Credit)
                && screen.topups.ready().is_none() =>
            {
                screen.topups = Remote::Ready(page.lines.into_iter().take(5).collect());
                Vec::new()
            }
            (
                Reply::PlaySessions {
                    is_continuation,
                    result,
                },
                Screen::PlaySessions(screen),
            ) => {
                match result {
                    Ok(page) => {
                        let mut sessions = match (is_continuation, &mut screen.sessions) {
                            (true, Remote::Ready(rows)) => std::mem::take(rows),
                            _ => Vec::new(),
                        };
                        sessions.extend(page.sessions);
                        screen.next_cursor = page.next_cursor;
                        screen.sessions = Remote::Ready(sessions);
                        screen.more = Submit::Idle;
                    }
                    Err(error) if is_continuation => screen.more = Submit::Refused(error),
                    Err(error) => screen.sessions = Remote::Failed(error),
                }
                Vec::new()
            }
            (
                Reply::Assets {
                    is_continuation,
                    result,
                },
                Screen::Assets(screen),
            ) => {
                match result {
                    Ok(page) => {
                        let mut assets = match (is_continuation, &mut screen.assets) {
                            (true, Remote::Ready(rows)) => std::mem::take(rows),
                            _ => Vec::new(),
                        };
                        assets.extend(page.assets);
                        screen.total = page.total;
                        screen.next_cursor = page.next_cursor;
                        screen.assets = Remote::Ready(assets);
                        screen.more = Submit::Idle;
                    }
                    Err(error) if is_continuation => screen.more = Submit::Refused(error),
                    Err(error) => screen.assets = Remote::Failed(error),
                }
                Vec::new()
            }
            (Reply::Settings(result), Screen::Settings(screen)) => {
                if let Ok(view) = &result {
                    screen.display_name = view.display_name.clone().unwrap_or_default();
                    screen.product_news = view.email_preferences.product_news;
                    screen.session_reminders = view.email_preferences.session_reminders;
                    if view.export_status != crate::wire::ExportStatus::NotRequested {
                        screen.export_view = Some(ExportView {
                            status: view.export_status,
                            requested_at_ms: None,
                        });
                    }
                }
                screen.settings = Remote::from_result(result);
                Vec::new()
            }
            (Reply::EmailChangeSent(result), Screen::Settings(screen)) => match result {
                Ok(_) => {
                    screen.email_change = Submit::Idle;
                    screen.is_email_change_sent = true;
                    screen.email_password.clear();
                    Vec::new()
                }
                Err(error) => {
                    screen.email_change = Submit::Refused(error);
                    vec![Effect::Focus(FocusTarget::ErrorSummary)]
                }
            },
            (Reply::PasswordChanged(result), Screen::Settings(screen)) => match result {
                Ok(session) => {
                    screen.password_change = Submit::Idle;
                    screen.is_password_changed = true;
                    screen.current_password.clear();
                    screen.new_password.clear();
                    screen.new_confirmation.clear();
                    self.session = Remote::Ready(session);
                    Vec::new()
                }
                Err(error) => {
                    screen.password_change = Submit::Refused(error);
                    vec![Effect::Focus(FocusTarget::ErrorSummary)]
                }
            },
            (Reply::PreferencesSaved(result), Screen::Settings(screen)) => match result {
                Ok(_) => {
                    screen.preferences = Submit::Idle;
                    screen.is_preferences_saved = true;
                    Vec::new()
                }
                Err(error) => {
                    screen.preferences = Submit::Refused(error);
                    vec![Effect::Focus(FocusTarget::ErrorSummary)]
                }
            },
            (Reply::ExportRequested(result), Screen::Settings(screen)) => match result {
                Ok(view) => {
                    screen.export = Submit::Idle;
                    screen.export_view = Some(view);
                    screen.export_password.clear();
                    Vec::new()
                }
                Err(error) => {
                    screen.export = Submit::Refused(error);
                    vec![Effect::Focus(FocusTarget::ErrorSummary)]
                }
            },
            (Reply::ProfileSaved(result), Screen::Settings(screen)) => match result {
                Ok(_) => {
                    screen.profile = Submit::Idle;
                    screen.is_profile_saved = true;
                    screen.profile_operation = Remote::Loading;
                    vec![
                        self.issue_operation(OperationSlot::Primary),
                        self.api(ApiCall::Session),
                    ]
                }
                Err(error) => {
                    screen.profile = Submit::Refused(error);
                    vec![Effect::Focus(FocusTarget::ErrorSummary)]
                }
            },
            (Reply::Revoked(result), Screen::Settings(screen)) => match result {
                Ok(view) => {
                    screen.revoke = Submit::Idle;
                    screen.revoked = Some(view.revoked);
                    vec![self.api(ApiCall::Settings)]
                }
                Err(error) => {
                    screen.revoke = Submit::Refused(error);
                    vec![Effect::Focus(FocusTarget::ErrorSummary)]
                }
            },
            (Reply::Closed(result), Screen::Settings(screen)) => match result {
                Ok(view) => {
                    self.notice = Some(Notice::AccountClosed {
                        delete_after_ms: view.delete_after_ms,
                    });
                    self.session = Remote::Loading;
                    self.route = Route::SignedOut;
                    self.screen = Screen::Booting;
                    vec![
                        Effect::CancelPending,
                        Effect::PushUrl(Route::SignedOut.path()),
                        self.api(ApiCall::Session),
                    ]
                }
                Err(error) => {
                    screen.close_password.clear();
                    screen.close = Submit::Refused(error);
                    vec![Effect::Focus(FocusTarget::ErrorSummary)]
                }
            },
            (reply, _) => self.campaign_reply(reply),
        }
    }

    pub(super) fn timer_fired(&mut self) -> Vec<Effect> {
        match &self.screen {
            Screen::CreditReturn(CreditReturn {
                checkout_id: Some(checkout_id),
                ..
            }) => vec![self.api(ApiCall::Checkout {
                checkout_id: checkout_id.clone(),
            })],
            Screen::Lobby(_) => self.refresh_lobby(),
            Screen::Display(_) => self.display_timer(),
            _ => Vec::new(),
        }
    }
}
