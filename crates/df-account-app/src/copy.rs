//! Every sentence the app shows about an outcome: what happened and what to do next (plan 11; the
//! UX clarity bar). One table per closed set, so a new error code or refusal cannot ship silent.

use crate::{
    state::{AdminScreen, Field, Model, Screen},
    wire::{
        ApiError, AuditAction, CheckoutStatus, DeviceKind, DisputeStatus, ErrorCode, LedgerKind,
        LineStatus, OperatorCapability, PacePreset, ProviderHealth, Quantity, ReconciliationKind,
        Refusal, ReviewFlag, StatementCategory,
    },
    wire_campaign::{CampaignRole, Reason, StartBlocker},
};

/// What happened, and what the person can do about it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Problem {
    pub what: &'static str,
    pub fix: &'static str,
}

pub fn problem(error: ApiError) -> Problem {
    if let Some(reason) = error.reason {
        return reason_problem(reason);
    }
    if let Some(refusal) = error.refusal {
        return Problem {
            what: refusal_message(refusal),
            fix: "",
        };
    }
    let (what, fix) = match error.code {
        ErrorCode::SignedOut => (
            "You are signed out.",
            "Sign in again to continue; nothing you saved is lost.",
        ),
        ErrorCode::Forbidden => (
            "Your account cannot do this.",
            "If you think it should, contact support with what you were trying to do.",
        ),
        ErrorCode::NotVerified => (
            "Your email address is not verified yet.",
            "Open the link we emailed you, or send a new one from your dashboard.",
        ),
        ErrorCode::CsrfRefused => (
            "For your safety, this request was not accepted.",
            "Reload the page and try again.",
        ),
        ErrorCode::Throttled => (
            "Too many attempts in a short time.",
            "Wait a few minutes, then try again.",
        ),
        ErrorCode::NotFound => (
            "We could not find that.",
            "Check the link, or go back to your dashboard.",
        ),
        ErrorCode::Invalid => (
            "Something in the form needs changing.",
            "Check each field and send it again.",
        ),
        ErrorCode::StaleForm => (
            "This form was open too long.",
            "Send it again; we have refreshed it for you.",
        ),
        ErrorCode::Unavailable => (
            "We could not reach DungeonFlux just now. Nothing was changed.",
            "Check your connection and try again in a moment.",
        ),
        ErrorCode::Busy => (
            "DungeonFlux is busy right now. Nothing was changed.",
            "Try again in a few seconds.",
        ),
        ErrorCode::UnknownOutcome => (
            "We could not confirm whether that went through.",
            "Reload this page to see the current state before trying again, so nothing happens twice.",
        ),
    };
    Problem { what, fix }
}

pub fn refusal_message(refusal: Refusal) -> &'static str {
    match refusal {
        Refusal::EmailEmpty => "Enter your email address.",
        Refusal::EmailTooLong => {
            "That email address is too long. Use one of 254 characters or fewer."
        }
        Refusal::EmailMalformed => "Enter an email address like name@example.com.",
        Refusal::PasswordTooShort => "Use a password of at least 12 characters.",
        Refusal::PasswordTooLong => "Use a password of at most 128 characters.",
        Refusal::PasswordMatchesEmail => "Choose a password that is not your email address.",
        Refusal::PasswordCommon => {
            "That password is too common and easy to guess. Choose another one."
        }
        Refusal::PasswordMismatch => {
            "The two passwords are different. Type the same password twice."
        }
        Refusal::WrongPassword => {
            "That email and password do not match an account. Check both and try again."
        }
        Refusal::ReauthenticationRequired => {
            "Confirm it is you with Google in Sign-in methods, then try again within five minutes."
        }
        Refusal::AdultRequired => "DungeonFlux is for adults. Confirm that you are 18 or older.",
        Refusal::TermsRequired => "Accept the terms and privacy notice to create an account.",
        Refusal::DisplayNameTooLong => "Use a display name of 40 characters or fewer.",
        Refusal::DisplayNameControlCharacter => {
            "Use letters, numbers and spaces in your display name."
        }
        Refusal::AmountOutOfRange => "Choose an amount, or enter one within the range shown.",
        Refusal::ReasonRequired => "Give a reason; it is kept in the audit log.",
        Refusal::ReasonTooLong => "Keep the reason under 500 characters.",
        Refusal::SupportCaseRequired => "Enter the support case this credit belongs to.",
        Refusal::CeilingOutOfRange => "Enter an hourly maximum within the range shown.",
        Refusal::ConfirmationRequired => {
            "Tick the box to confirm you understand what closing does."
        }
        Refusal::ConsentRequired => "Tick the box to agree before you add credit to this table.",
        Refusal::CampaignNameRequired
        | Refusal::CampaignNameTooLong
        | Refusal::AdventureRequired
        | Refusal::ContentRatingRequired
        | Refusal::SeatsOutOfRange => "This request is no longer available.",
        Refusal::CodeRequired => "Enter the code shown on the screen.",
    }
}

/// The field a refusal is about, so the error summary can link to it.
pub fn refusal_field(refusal: Refusal) -> Option<Field> {
    Some(match refusal {
        Refusal::EmailEmpty | Refusal::EmailTooLong | Refusal::EmailMalformed => Field::Email,
        Refusal::PasswordTooShort
        | Refusal::PasswordTooLong
        | Refusal::PasswordMatchesEmail
        | Refusal::PasswordCommon
        | Refusal::WrongPassword => Field::Password,
        Refusal::PasswordMismatch => Field::Confirmation,
        Refusal::AdultRequired => Field::IsAdult,
        Refusal::TermsRequired => Field::AcceptsTerms,
        Refusal::DisplayNameTooLong | Refusal::DisplayNameControlCharacter => Field::DisplayName,
        Refusal::CeilingOutOfRange => Field::CustomCeiling,
        Refusal::ConfirmationRequired => Field::CloseUnderstood,
        Refusal::ConsentRequired => Field::AllocateConsent,
        Refusal::AmountOutOfRange
        | Refusal::ReauthenticationRequired
        | Refusal::ReasonRequired
        | Refusal::ReasonTooLong
        | Refusal::SupportCaseRequired
        | Refusal::CampaignNameRequired
        | Refusal::CampaignNameTooLong
        | Refusal::AdventureRequired
        | Refusal::ContentRatingRequired
        | Refusal::SeatsOutOfRange
        | Refusal::CodeRequired => return None,
    })
}

/// The refusal of the current screen's form, if one was refused.
pub fn refused_form_error(model: &Model) -> Option<ApiError> {
    match &model.screen {
        Screen::SignUp(form) => form.submit.error(),
        Screen::SignIn(form) => form.submit.error(),
        Screen::VerifyConfirm(screen) => screen.submit.error(),
        Screen::VerifyPending(screen) => screen.resend.error(),
        Screen::Forgot(form) => form.submit.error(),
        Screen::Reset(form) => form.submit.error(),
        Screen::Dashboard(screen) => screen.resend.error().or(screen.join.error()),
        Screen::AddCredit(form) => form.submit.error(),
        Screen::Funding(screen) => screen
            .edit
            .as_ref()
            .and_then(|edit| edit.submit.error())
            .or(screen
                .allocation
                .as_ref()
                .and_then(|edit| edit.submit.error())),
        Screen::Settings(screen) => screen
            .profile
            .error()
            .or(screen.email_change.error())
            .or(screen.password_change.error())
            .or(screen.preferences.error())
            .or(screen.export.error())
            .or(screen.revoke.error())
            .or(screen.close.error()),
        Screen::PlaySessions(_) | Screen::Assets(_) => None,
        Screen::Campaign(screen) => screen.action.error(),
        Screen::Lobby(screen) => screen.action.error(),
        Screen::Play(_) => None,
        Screen::Join(screen) => screen.submit.error(),
        Screen::Admin(AdminScreen::Account(screen)) => screen
            .refund
            .submit
            .error()
            .or(screen.credit.submit.error()),
        Screen::Admin(AdminScreen::Disputes(screen)) => screen
            .action
            .as_ref()
            .and_then(|action| action.submit.error()),
        Screen::Admin(AdminScreen::Providers(screen)) => screen
            .action
            .as_ref()
            .and_then(|action| action.submit.error()),
        Screen::Admin(AdminScreen::Review(screen)) => screen
            .action
            .as_ref()
            .and_then(|action| action.submit.error()),
        _ => None,
    }
}

/// The browser tab title for a screen.
pub fn title(screen: &Screen) -> String {
    let name = match screen {
        Screen::Integration(_) => "Integrations",
        Screen::Booting => "Loading",
        Screen::BootFailed(_) => "Cannot reach DungeonFlux",
        Screen::Landing => "Get $10 of free play",
        Screen::SignUp(_) => "Create your account",
        Screen::SignIn(_) => "Sign in",
        Screen::VerifyPending(_) => "Check your email",
        Screen::VerifyConfirm(_) => "Verify your email",
        Screen::Forgot(_) => "Reset your password",
        Screen::Reset(_) => "Choose a new password",
        Screen::SignedOut => "Signed out",
        Screen::Dashboard(_) => "Your tables",
        Screen::AddCredit(_) => "Add credit",
        Screen::CreditReturn(_) => "Payment status",
        Screen::Funding(_) => "Table pace",
        Screen::Statements(_) => "Statements",
        Screen::PlaySessions(_) => "Play sessions",
        Screen::Assets(_) => "Assets",
        Screen::Settings(_) => "Account settings",
        Screen::Campaign(_) => "Campaign",
        Screen::Lobby(_) => "Lobby",
        Screen::Play(_) => "Start your game",
        Screen::Join(_) => "Join a table",
        Screen::Admin(_) => "Operator console",
        Screen::NotPermitted => "Not available",
        Screen::NeedsSecondFactor => "Second step needed",
        Screen::NotFound => "Page not found",
    };
    format!("{name} | DungeonFlux")
}

/// The form value of a preset (radio inputs carry it).
pub const fn preset_value(preset: PacePreset) -> &'static str {
    match preset {
        PacePreset::Story => "story",
        PacePreset::Immersive => "immersive",
        PacePreset::Cinematic => "cinematic",
        PacePreset::Custom => "custom",
    }
}

pub const fn preset_name(preset: PacePreset) -> &'static str {
    match preset {
        PacePreset::Story => "Story",
        PacePreset::Immersive => "Immersive",
        PacePreset::Cinematic => "Cinematic",
        PacePreset::Custom => "Custom",
    }
}

/// What a preset buys (plan 4.3), in the customer's words.
pub const fn preset_description(preset: PacePreset) -> &'static str {
    match preset {
        PacePreset::Story => "Narration and text first, with art the table already has. No video.",
        PacePreset::Immersive => "Spoken voices and new scene pictures from time to time.",
        PacePreset::Cinematic => "More frequent new pictures. Video always asks first.",
        PacePreset::Custom => "You set the most the table may spend in an hour.",
    }
}

pub const fn role_name(role: CampaignRole) -> &'static str {
    match role {
        CampaignRole::Host => "Host",
        CampaignRole::CoHost => "Co-host",
        CampaignRole::Player => "Player",
        CampaignRole::Contributor => "Contributor",
    }
}

pub const fn reason_problem(reason: Reason) -> Problem {
    let (what, fix) = match reason {
        Reason::Expired => (
            "This invitation has expired.",
            "Ask whoever sent it for a new one.",
        ),
        Reason::Revoked => (
            "This invitation was withdrawn by the host.",
            "Ask the host for a new invitation.",
        ),
        Reason::Exhausted => (
            "This invitation has been used as many times as it allows.",
            "Ask the host for a new invitation.",
        ),
        Reason::Full => (
            "All seats are taken.",
            "Ask the host to free a seat, or to send you a contributor invitation, which needs no seat.",
        ),
        Reason::ContributorsFull => (
            "This table has as many contributors as it allows.",
            "Ask the host whether a seat can open for you instead.",
        ),
        Reason::AlreadyJoined => (
            "You are already a member of this table.",
            "Find it on your dashboard.",
        ),
        Reason::NoLongerValid => (
            "This invitation or code is not valid.",
            "Check that you copied all of it, or ask the host for a new one.",
        ),
        Reason::NotTakingRequests => (
            "This table is not taking join requests right now.",
            "Ask the host for an invitation link instead.",
        ),
        Reason::RequestsFull => (
            "This table has as many waiting requests as it allows.",
            "Try again later, or ask the host for an invitation link.",
        ),
        Reason::AllowBackFirst => (
            "This person was removed from the table.",
            "Allow them back first, then approve their request.",
        ),
        Reason::DisplayLimit => (
            "This table already has as many screens paired as it allows.",
            "Unpair one screen in the lobby, then show a new code on this one.",
        ),
        Reason::StaleRevision => (
            "Someone else changed this table just now.",
            "Reload the page to see the latest, then try again.",
        ),
        Reason::AccountClosing => (
            "This account is being closed.",
            "Sign in again to reopen it before joining or hosting tables.",
        ),
        Reason::EmailUnverified => (
            "Verify your email address first.",
            "Open the link we emailed you, or send a new one from your dashboard.",
        ),
    };
    Problem { what, fix }
}

pub const fn start_blocker(blocker: StartBlocker) -> &'static str {
    match blocker {
        StartBlocker::NoSeatedMembers => "Nobody has taken a seat yet.",
        StartBlocker::FundingNotReady => "The table's funding is not ready yet.",
        StartBlocker::SessionNotInLobby => "This session is no longer in the lobby.",
        StartBlocker::CampaignArchived => "This campaign is archived.",
    }
}

pub const fn device_name(device: DeviceKind) -> &'static str {
    match device {
        DeviceKind::Phone => "Phone",
        DeviceKind::Tablet => "Tablet",
        DeviceKind::Desktop => "Computer or TV browser",
        DeviceKind::Unknown => "Unrecognised device",
    }
}

pub const fn category_name(category: StatementCategory) -> &'static str {
    match category {
        StatementCategory::TableTime => "Active table time",
        StatementCategory::Narration => "Spoken narration",
        StatementCategory::SceneImage => "Scene picture",
        StatementCategory::Portrait => "Character portrait",
        StatementCategory::Music => "Music",
        StatementCategory::Purchase => "Credit added",
        StatementCategory::TrialCredit => "Free play credit",
        StatementCategory::GoodwillCredit => "Credit from support",
        StatementCategory::TechnicalCredit => "Technical problem, credited",
        StatementCategory::Refund => "Refund to your card",
    }
}

pub fn quantity(quantity: Quantity) -> Option<String> {
    match quantity {
        Quantity::None => None,
        Quantity::Seconds { seconds } => Some(crate::money::duration(seconds)),
        Quantity::AudioTenthsOfMinute { tenths } => {
            Some(format!("{}.{} audio minutes", tenths / 10, tenths % 10))
        }
        Quantity::Images { count: 1 } => Some("1 picture".to_owned()),
        Quantity::Images { count } => Some(format!("{count} pictures")),
    }
}

pub const fn line_status(status: LineStatus) -> &'static str {
    match status {
        LineStatus::Settled => "Final",
        LineStatus::Provisional => "Provisional: can only go down",
        LineStatus::Pending => "Pending",
        LineStatus::Refunded => "Refunded",
    }
}

pub const fn checkout_status(status: CheckoutStatus) -> Problem {
    let (what, fix) = match status {
        CheckoutStatus::Pending => (
            "We are waiting for your card company to confirm the payment.",
            "This usually takes a few seconds. You can leave this page; the credit appears once.",
        ),
        CheckoutStatus::Paid => ("Your credit has been added.", ""),
        CheckoutStatus::Failed => (
            "The payment did not go through, and nothing was charged.",
            "Try again with the same or another card.",
        ),
        CheckoutStatus::Cancelled => (
            "You cancelled the payment. Nothing was charged.",
            "Choose an amount again whenever you are ready.",
        ),
        CheckoutStatus::Expired => (
            "The payment page timed out. Nothing was charged.",
            "Start again from Add credit.",
        ),
    };
    Problem { what, fix }
}

pub const fn capability_name(capability: OperatorCapability) -> &'static str {
    match capability {
        OperatorCapability::SupportInspect => "Inspect",
        OperatorCapability::Refund => "Refund",
        OperatorCapability::GoodwillCredit => "Goodwill credit",
        OperatorCapability::DisputeResponse => "Disputes",
        OperatorCapability::ProviderControl => "Providers",
        OperatorCapability::ContentReview => "Review",
    }
}

pub const fn ledger_kind(kind: LedgerKind) -> &'static str {
    match kind {
        LedgerKind::Purchase => "Purchase",
        LedgerKind::TrialGrant => "Trial grant",
        LedgerKind::GoodwillCredit => "Goodwill credit",
        LedgerKind::Usage => "Usage",
        LedgerKind::Refund => "Refund",
        LedgerKind::DisputeHold => "Dispute hold",
    }
}

pub const fn dispute_status(status: DisputeStatus) -> &'static str {
    match status {
        DisputeStatus::NeedsResponse => "Needs a response",
        DisputeStatus::UnderReview => "Under review by the card company",
        DisputeStatus::Won => "Won",
        DisputeStatus::Lost => "Lost",
    }
}

pub const fn reconciliation_kind(kind: ReconciliationKind) -> &'static str {
    match kind {
        ReconciliationKind::ProcessorCapture => "Processor capture",
        ReconciliationKind::ProcessorRefund => "Processor refund",
        ReconciliationKind::Payout => "Payout",
        ReconciliationKind::SupplierUsage => "Supplier usage",
        ReconciliationKind::AgedReservation => "Reservation held too long",
    }
}

pub const fn provider_health(health: ProviderHealth) -> &'static str {
    match health {
        ProviderHealth::Healthy => "Healthy",
        ProviderHealth::Degraded => "Degraded",
        ProviderHealth::Paused => "Paused by an operator",
    }
}

pub const fn review_flag(flag: ReviewFlag) -> &'static str {
    match flag {
        ReviewFlag::PossibleRealPerson => "May show a real person",
        ReviewFlag::PossibleExplicitContent => "May be explicit",
        ReviewFlag::ScreenUnavailable => "Automatic screen was unavailable",
    }
}

pub const fn audit_action(action: AuditAction) -> &'static str {
    match action {
        AuditAction::Refund => "Refund",
        AuditAction::GoodwillCredit => "Goodwill credit",
        AuditAction::DisputeEvidence => "Dispute evidence",
        AuditAction::ProviderPaused => "Provider paused",
        AuditAction::ProviderResumed => "Provider resumed",
        AuditAction::ReviewApproved => "Review approved",
        AuditAction::ReviewRejected => "Review rejected",
        AuditAction::EmailViewed => "Email viewed",
        AuditAction::SignInSessionRevoked => "Sign-in session revoked",
        AuditAction::AssetHeld => "Asset held",
        AuditAction::AssetReleased => "Asset released",
    }
}

pub const fn asset_kind_name(kind: crate::wire::AssetKind) -> &'static str {
    match kind {
        crate::wire::AssetKind::Portrait => "Portrait",
        crate::wire::AssetKind::Scene => "Scene picture",
        crate::wire::AssetKind::Music => "Music",
        crate::wire::AssetKind::Video => "Video",
    }
}

pub const fn asset_hold_name(state: crate::wire::AssetHoldState) -> &'static str {
    match state {
        crate::wire::AssetHoldState::Ready => "Ready",
        crate::wire::AssetHoldState::Held => "On hold",
        crate::wire::AssetHoldState::Retained => "Kept for retention",
    }
}
