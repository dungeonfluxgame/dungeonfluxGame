//! The account records the service and its stores exchange. Store adapters map these to rows;
//! nothing here knows SQL or HTTP.

use df_types::{AccountId, OperationId, WebSessionId};

use crate::tokens::{CsrfSecret, PasswordHash, TokenDigest};

/// Longest email address accepted, in bytes (RFC 5321 path limit).
pub const MAXIMUM_EMAIL_BYTES: usize = 254;
/// Longest display name, in characters.
pub const MAXIMUM_DISPLAY_NAME_CHARACTERS: usize = 60;

/// An email address as entered, with the normalized form uniqueness and lookups use.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmailAddress {
    normalized: String,
    display: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmailRefusal {
    Empty,
    TooLong,
    Malformed,
}

impl EmailAddress {
    /// Accepts one `@` between non-empty parts, a dot in the domain, and no whitespace or control
    /// characters. Normalization trims and lowercases ASCII; delivery is the real proof.
    pub fn parse(text: &str) -> Result<Self, EmailRefusal> {
        let display = text.trim();
        if display.is_empty() {
            return Err(EmailRefusal::Empty);
        }
        if display.len() > MAXIMUM_EMAIL_BYTES {
            return Err(EmailRefusal::TooLong);
        }
        let Some((local, domain)) = display.split_once('@') else {
            return Err(EmailRefusal::Malformed);
        };
        let well_formed = !local.is_empty()
            && !domain.is_empty()
            && !domain.contains('@')
            && domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
            // Characters that make a header or an address list ambiguous: a second recipient,
            // a display name, a comment or a quoted local part.
            && !display.chars().any(|character| {
                character.is_whitespace()
                    || character.is_control()
                    || matches!(character, ',' | '"' | '<' | '>' | ';')
            });
        if !well_formed {
            return Err(EmailRefusal::Malformed);
        }
        Ok(Self {
            normalized: display.to_ascii_lowercase(),
            display: display.to_owned(),
        })
    }

    /// Rebuilds an address read back from a store that already validated it.
    pub fn from_stored(normalized: String, display: String) -> Self {
        Self {
            normalized,
            display,
        }
    }

    pub fn normalized(&self) -> &str {
        &self.normalized
    }

    pub fn display(&self) -> &str {
        &self.display
    }

    /// `a***@e***.test`: enough for a person to recognise their own address, not to read it.
    pub fn masked(&self) -> String {
        let (local, domain) = self
            .display
            .split_once('@')
            .unwrap_or((self.display.as_str(), ""));
        let (host, top) = domain.rsplit_once('.').unwrap_or((domain, ""));
        let first = |part: &str| part.chars().next().map(String::from).unwrap_or_default();
        format!("{}***@{}***.{top}", first(local), first(host))
    }
}

/// A display name the person chose: 1 to 60 characters, no control characters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisplayName(String);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayNameRefusal {
    TooLong,
    ControlCharacter,
}

impl DisplayName {
    /// An empty name is `None` (the person cleared it), not an error.
    pub fn parse(text: &str) -> Result<Option<Self>, DisplayNameRefusal> {
        let name = text.trim();
        if name.is_empty() {
            return Ok(None);
        }
        if name.chars().count() > MAXIMUM_DISPLAY_NAME_CHARACTERS {
            return Err(DisplayNameRefusal::TooLong);
        }
        if name.chars().any(char::is_control) {
            return Err(DisplayNameRefusal::ControlCharacter);
        }
        Ok(Some(Self(name.to_owned())))
    }

    pub fn from_stored(name: String) -> Self {
        Self(name)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountStatus {
    Active,
    /// Closed by its owner; reopened by logging in before `delete_after_ms`.
    Closing,
    /// Personal data deleted; only pseudonymous records remain.
    Closed,
}

impl AccountStatus {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Closing => "closing",
            Self::Closed => "closed",
        }
    }

    pub fn parse(label: &str) -> Option<Self> {
        [Self::Active, Self::Closing, Self::Closed]
            .into_iter()
            .find(|status| status.label() == label)
    }
}

/// One account as stored. Closed accounts have no email or password hash.
#[derive(Clone)]
pub struct AccountRecord {
    pub id: AccountId,
    pub email: Option<EmailAddress>,
    pub is_verified: bool,
    pub display_name: Option<DisplayName>,
    pub status: AccountStatus,
    pub credential_generation: u64,
    pub password_hash: Option<PasswordHash>,
    pub created_at_ms: u64,
    pub delete_after_ms: Option<u64>,
}

/// A coarse device class from the request's user agent, shown on the sessions page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceLabel {
    Phone,
    Tablet,
    Desktop,
    Unknown,
}

impl DeviceLabel {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Phone => "phone",
            Self::Tablet => "tablet",
            Self::Desktop => "desktop",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(label: &str) -> Option<Self> {
        [Self::Phone, Self::Tablet, Self::Desktop, Self::Unknown]
            .into_iter()
            .find(|device| device.label() == label)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenPurpose {
    VerifyEmail,
    PasswordReset,
    EmailChangeOld,
    EmailChangeNew,
}

impl TokenPurpose {
    pub const fn label(self) -> &'static str {
        match self {
            Self::VerifyEmail => "verify_email",
            Self::PasswordReset => "password_reset",
            Self::EmailChangeOld => "email_change_old",
            Self::EmailChangeNew => "email_change_new",
        }
    }

    pub fn parse(label: &str) -> Option<Self> {
        [
            Self::VerifyEmail,
            Self::PasswordReset,
            Self::EmailChangeOld,
            Self::EmailChangeNew,
        ]
        .into_iter()
        .find(|purpose| purpose.label() == label)
    }
}

/// A single-use emailed token to store. Only its digest is kept.
#[derive(Clone)]
pub struct NewToken {
    pub id: [u8; 16],
    pub digest: TokenDigest,
    pub account: AccountId,
    pub purpose: TokenPurpose,
    pub email_change: Option<[u8; 16]>,
    pub credential_generation: u64,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub attempt_limit: u32,
}

#[derive(Clone)]
pub struct NewAccount {
    pub id: AccountId,
    pub email: EmailAddress,
    pub password_hash: PasswordHash,
    pub created_at_ms: u64,
    pub adult_attested_at: u64,
    pub terms_version: String,
    pub terms_accepted_at: u64,
    pub verification: NewToken,
}

/// An account created on first sign-in through an identity provider: no password credential.
#[derive(Clone)]
pub struct NewIdentityAccount {
    pub id: AccountId,
    pub email: EmailAddress,
    pub is_verified: bool,
    pub provider: crate::provider::IdentityProviderKind,
    pub subject: String,
    pub created_at_ms: u64,
    pub adult_attested_at: Option<u64>,
    pub terms_version: Option<String>,
    pub terms_accepted_at: Option<u64>,
    /// The verification link's token, stored in the same transaction, when the address is not
    /// yet verified (a password sign-up through the provider).
    pub verification: Option<NewToken>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateOutcome {
    Created,
    EmailTaken,
}

/// A verification link that is still open, found without consuming it: whose it is and whether
/// the address was already verified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerificationTarget {
    pub account: AccountId,
    pub already_verified: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerifyOutcome {
    Verified(AccountId),
    AlreadyVerified(AccountId),
    Invalid,
}

/// Why a session ended; a closed set shown on the security history.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevokeReason {
    Logout,
    RevokedByUser,
    SignOutEverywhere,
    PasswordChanged,
    PasswordReset,
    EmailChanged,
    Rotated,
    AccountClosed,
}

impl RevokeReason {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Logout => "logout",
            Self::RevokedByUser => "revoked_by_user",
            Self::SignOutEverywhere => "sign_out_everywhere",
            Self::PasswordChanged => "password_changed",
            Self::PasswordReset => "password_reset",
            Self::EmailChanged => "email_changed",
            Self::Rotated => "rotated",
            Self::AccountClosed => "account_closed",
        }
    }
}

#[derive(Clone)]
pub struct NewSession {
    pub id: WebSessionId,
    pub digest: TokenDigest,
    pub account: AccountId,
    pub credential_generation: u64,
    pub created_at_ms: u64,
    pub absolute_expires_at_ms: u64,
    pub csrf_secret: CsrfSecret,
    pub device: DeviceLabel,
}

/// A stored session read by its token digest, with its account.
#[derive(Clone)]
pub struct SessionRecord {
    pub id: WebSessionId,
    pub account: AccountId,
    pub credential_generation: u64,
    pub created_at_ms: u64,
    pub last_seen_at_ms: u64,
    pub is_revoked: bool,
    pub csrf_secret: CsrfSecret,
    pub device: DeviceLabel,
}

/// One active session as listed to its owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSummary {
    pub id: WebSessionId,
    pub device: DeviceLabel,
    pub created_at_ms: u64,
    pub last_seen_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevokeScope {
    One(WebSessionId),
    AllExcept(WebSessionId),
    All,
}

/// A password change committed with the session that replaces the current one.
#[derive(Clone)]
pub struct PasswordChange {
    pub account: AccountId,
    pub expected_generation: u64,
    pub new_hash: PasswordHash,
    pub replacement: NewSession,
    pub at_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PasswordChangeOutcome {
    Changed,
    StaleGeneration,
}

/// What a reset transaction loads for `df_auth::recovery::recover_credentials` and writes back.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResetSnapshot {
    pub account: AccountId,
    pub purpose_is_reset: bool,
    pub expires_at_ms: u64,
    pub attempts: u32,
    pub attempt_limit: u32,
    pub is_consumed: bool,
    pub credential_generation: u64,
    pub is_account_active: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResetOutcome {
    Reset(AccountId),
    Refused(df_auth::recovery::AccountRefusal),
    Invalid,
}

#[derive(Clone)]
pub struct EmailChangeRequest {
    pub id: [u8; 16],
    pub account: AccountId,
    pub new_email: EmailAddress,
    pub old_address_token: NewToken,
    pub new_address_token: NewToken,
    pub expires_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmailChangeOutcome {
    Requested,
    AddressTaken,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmailChangeDecision {
    Confirm,
    Cancel,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfirmChangeOutcome {
    AwaitingOtherAddress,
    Completed {
        account: AccountId,
        old_email: EmailAddress,
        new_email: EmailAddress,
    },
    Cancelled {
        account: AccountId,
    },
    AddressTaken,
    Invalid,
}

/// A security event shown to the account's owner. Closed set; no addresses or tokens.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    SignedUp,
    Verified,
    LoggedIn,
    LoginRefused,
    LoggedOut,
    PasswordChanged,
    PasswordReset,
    EmailChangeRequested,
    EmailChanged,
    SessionsRevoked,
    DataExported,
    Closed,
    Reopened,
}

impl EventKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::SignedUp => "signed_up",
            Self::Verified => "verified",
            Self::LoggedIn => "logged_in",
            Self::LoginRefused => "login_refused",
            Self::LoggedOut => "logged_out",
            Self::PasswordChanged => "password_changed",
            Self::PasswordReset => "password_reset",
            Self::EmailChangeRequested => "email_change_requested",
            Self::EmailChanged => "email_changed",
            Self::SessionsRevoked => "sessions_revoked",
            Self::DataExported => "data_exported",
            Self::Closed => "closed",
            Self::Reopened => "reopened",
        }
    }

    pub fn parse(label: &str) -> Option<Self> {
        [
            Self::SignedUp,
            Self::Verified,
            Self::LoggedIn,
            Self::LoginRefused,
            Self::LoggedOut,
            Self::PasswordChanged,
            Self::PasswordReset,
            Self::EmailChangeRequested,
            Self::EmailChanged,
            Self::SessionsRevoked,
            Self::DataExported,
            Self::Closed,
            Self::Reopened,
        ]
        .into_iter()
        .find(|kind| kind.label() == label)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccountEvent {
    pub kind: EventKind,
    pub at_ms: u64,
    pub device: DeviceLabel,
}

/// Mutations deduplicated by a form's operation id (plan 6.4): the two whose repeat would send
/// extra email. Every other account mutation is idempotent by itself (setting a name, revoking
/// sessions, closing) or refuses its own repeat (a password change needs the old password).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationNamespace {
    Signup,
    EmailChange,
}

impl OperationNamespace {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Signup => "signup",
            Self::EmailChange => "email_change",
        }
    }
}

/// One form submission's identity: namespace, principal (account id, or for signup a digest of
/// the address) and the form's operation id.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationKey {
    pub namespace: OperationNamespace,
    pub principal: [u8; 32],
    pub operation: OperationId,
}

/// What a committed operation did, so a repeat can show the same result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiptOutcome {
    Done,
}

impl ReceiptOutcome {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Done => "done",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationClaim {
    New,
    Replayed(ReceiptOutcome),
    InProgress,
    Conflict,
}

/// Retention bounds the maintenance task applies (`service-operations.md` retention classes).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetentionCutoffs {
    pub now_ms: u64,
    /// Sessions revoked, or past their absolute expiry, before this are deleted.
    pub sessions_ended_before_ms: u64,
    /// Sessions last used before this ended by idle expiry long enough ago to delete.
    pub sessions_last_seen_before_ms: u64,
    /// Expired tokens older than this are deleted.
    pub tokens_before_ms: u64,
    /// Receipts older than this are deleted (90 days).
    pub receipts_before_ms: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PurgeCounts {
    pub sessions: u64,
    pub tokens: u64,
    pub receipts: u64,
    pub deleted_accounts: u64,
}

/// Counts and the most recent accounts, for `dev accounts`.
#[derive(Clone, Default)]
pub struct AccountsSummary {
    pub active: u64,
    pub closing: u64,
    pub closed: u64,
    pub verified: u64,
    pub recent: Vec<RecentAccount>,
}

#[derive(Clone)]
pub struct RecentAccount {
    pub id: AccountId,
    pub email: Option<EmailAddress>,
    pub status: AccountStatus,
    pub is_verified: bool,
    pub active_sessions: u64,
    pub last_event: Option<AccountEvent>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_trimmed_and_lowercased_for_lookup_but_kept_as_entered_for_display() {
        let address = EmailAddress::parse("  Robin@Example.Test ").unwrap();
        assert_eq!(address.normalized(), "robin@example.test");
        assert_eq!(address.display(), "Robin@Example.Test");
        assert_eq!(address.masked(), "R***@E***.Test");
    }

    #[test]
    fn malformed_addresses_are_refused() {
        for text in [
            "robin",
            "@example.test",
            "robin@",
            "robin@example",
            "a b@example.test",
            "a@example.test,b@example.test",
            "\"a\"@example.test",
            "Robin <a@example.test>",
            "a@example.test;b@example.test",
        ] {
            assert_eq!(
                EmailAddress::parse(text),
                Err(EmailRefusal::Malformed),
                "{text}"
            );
        }
        assert_eq!(EmailAddress::parse(" "), Err(EmailRefusal::Empty));
        let long = format!("{}@example.test", "a".repeat(250));
        assert_eq!(EmailAddress::parse(&long), Err(EmailRefusal::TooLong));
    }

    #[test]
    fn a_display_name_is_optional_bounded_and_free_of_control_characters() {
        assert_eq!(DisplayName::parse("   "), Ok(None));
        assert_eq!(
            DisplayName::parse(&"x".repeat(61)),
            Err(DisplayNameRefusal::TooLong)
        );
        assert_eq!(
            DisplayName::parse("a\u{7}b"),
            Err(DisplayNameRefusal::ControlCharacter)
        );
        assert_eq!(
            DisplayName::parse(" Robin ").unwrap().unwrap().as_str(),
            "Robin"
        );
    }
}
