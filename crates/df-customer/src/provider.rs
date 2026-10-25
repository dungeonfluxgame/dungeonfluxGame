//! The identity provider port (coordinator launch decision 28): hosted production keeps its
//! passwords at an identity provider (WorkOS) that this server calls from the back end. The browser
//! never visits the provider and no provider token is kept.
//!
//! The port is the provider's side of five person-level actions: sign up (create the user), sign in
//! (check a password), email verification (mark the address verified once OUR link was used),
//! password reset (the provider issues the token, our mail carries it) and sign-out. Everything
//! else stays ours: the account row, the session cookie, the throttles, every email and the
//! verified flag that gates campaign creation.
//!
//! `IdentitySource` is the one seam the service has: `LocalPasswords` (the development and
//! self-hosting provider, whose hashes live in the account store and which hosted production
//! refuses) or a remote `IdentityProvider`. The service entry points are the same for both.
//!
//! Failure classes are kept apart. `Unavailable` means nothing was learned and nothing changed;
//! `Unknown` means a mutation may have happened; `Malformed` means the provider answered with a
//! shape we do not accept. A refused credential is an ordinary answer, not a failure.

use std::{fmt, future::Future, net::IpAddr, pin::Pin, sync::Arc};

use crate::records::EmailAddress;

/// Which provider holds the credentials; stored with the identity link.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum IdentityProviderKind {
    WorkOs,
    /// The faithful fake used by tests.
    Fake,
}

impl IdentityProviderKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::WorkOs => "workos",
            Self::Fake => "fake",
        }
    }

    pub fn parse(label: &str) -> Option<Self> {
        [Self::WorkOs, Self::Fake]
            .into_iter()
            .find(|kind| kind.label() == label)
    }
}

/// Longest provider subject accepted.
pub const MAXIMUM_SUBJECT_BYTES: usize = 256;
/// Longest provider session id or reset token accepted.
pub const MAXIMUM_PROVIDER_TOKEN_BYTES: usize = 512;

/// A user as the provider describes it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderUser {
    /// The provider's stable user id.
    pub subject: String,
    pub email: EmailAddress,
    /// Whether the provider itself considers the address verified.
    pub email_verified: bool,
}

/// What the provider may be told about the caller, for its own abuse screening.
#[derive(Clone, Copy, Debug, Default)]
pub struct CallerContext {
    pub ip: Option<IpAddr>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityFailure {
    /// The provider could not be reached or answered with a failure; nothing was learned.
    Unavailable,
    /// A mutation timed out or its answer was lost; it may have happened.
    Unknown,
    /// The answer did not have the shape we require.
    Malformed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CreateUser {
    Created(ProviderUser),
    /// The provider already has a user at this address.
    EmailNotAvailable,
    /// The provider's own password policy refused the password.
    PasswordRejected,
}

#[derive(Clone, Eq, PartialEq)]
pub enum Authentication {
    Authenticated {
        user: ProviderUser,
        /// The provider-side session this sign-in created. We never use it, so the service ends
        /// it at once.
        provider_session: Option<String>,
    },
    /// The provider will not authenticate an address it has not verified. Our own verification
    /// is the only one in use, so the service counts this as a failed sign-in and answers like
    /// any other refusal.
    EmailVerificationRequired,
    /// Wrong password or unknown address: one answer for both.
    InvalidCredentials,
}

impl fmt::Debug for Authentication {
    /// A provider session id is a bearer capability at the provider: never printed.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Authenticated { user, .. } => formatter
                .debug_struct("Authenticated")
                .field("user", user)
                .field("provider_session", &"<redacted>")
                .finish(),
            Self::EmailVerificationRequired => formatter.write_str("EmailVerificationRequired"),
            Self::InvalidCredentials => formatter.write_str("InvalidCredentials"),
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum ResetStart {
    /// The token travels in our email; the provider does not send its own.
    Started {
        token: String,
    },
    NoSuchUser,
}

impl fmt::Debug for ResetStart {
    /// The token resets a password: never printed.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Started { .. } => formatter
                .debug_struct("Started")
                .field("token", &"<redacted>")
                .finish(),
            Self::NoSuchUser => formatter.write_str("NoSuchUser"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetPassword {
    Set,
    /// The provider's own password policy refused the password.
    Rejected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResetFinish {
    Reset(ProviderUser),
    /// Unknown, used or expired token.
    InvalidToken,
    PasswordRejected,
}

/// The provider port. `df-workos` implements it for WorkOS; `fakes::FakeIdentityProvider` is the
/// in-process fake. Every call is bounded by the service's provider deadline.
pub trait IdentityProvider: Send + Sync + 'static {
    fn kind(&self) -> IdentityProviderKind;

    /// Creates a user with an unverified address and the given password.
    fn create_user(
        &self,
        email: &EmailAddress,
        password: &str,
        caller: CallerContext,
    ) -> impl Future<Output = Result<CreateUser, IdentityFailure>> + Send;

    /// Checks a password. Never changes a user.
    fn authenticate(
        &self,
        email: &EmailAddress,
        password: &str,
        caller: CallerContext,
    ) -> impl Future<Output = Result<Authentication, IdentityFailure>> + Send;

    fn find_user(
        &self,
        email: &EmailAddress,
    ) -> impl Future<Output = Result<Option<ProviderUser>, IdentityFailure>> + Send;

    /// Marks the user's address verified at the provider (idempotent).
    fn mark_email_verified(
        &self,
        subject: &str,
    ) -> impl Future<Output = Result<(), IdentityFailure>> + Send;

    /// Replaces the user's password (the verify-and-set-password step: whoever signed up with
    /// an address does not keep the password they chose once its owner proves the mailbox).
    fn set_password(
        &self,
        subject: &str,
        password: &str,
    ) -> impl Future<Output = Result<SetPassword, IdentityFailure>> + Send;

    fn begin_password_reset(
        &self,
        email: &EmailAddress,
    ) -> impl Future<Output = Result<ResetStart, IdentityFailure>> + Send;

    /// Sets the new password if `token` is valid, ends the user's provider sessions and spends
    /// the token.
    fn finish_password_reset(
        &self,
        token: &str,
        new_password: &str,
    ) -> impl Future<Output = Result<ResetFinish, IdentityFailure>> + Send;

    /// Ends a provider-side session (idempotent).
    fn end_session(
        &self,
        provider_session: &str,
    ) -> impl Future<Output = Result<(), IdentityFailure>> + Send;

    /// Deletes the user (idempotent; a missing user is success).
    fn delete_user(
        &self,
        subject: &str,
    ) -> impl Future<Output = Result<(), IdentityFailure>> + Send;
}

type Call<'a, T> = Pin<Box<dyn Future<Output = Result<T, IdentityFailure>> + Send + 'a>>;

/// Object-safe form of [`IdentityProvider`], so the service does not carry a provider type
/// parameter into every router handler.
pub(crate) trait ErasedProvider: Send + Sync {
    fn kind(&self) -> IdentityProviderKind;
    fn create_user<'a>(
        &'a self,
        email: &'a EmailAddress,
        password: &'a str,
        caller: CallerContext,
    ) -> Call<'a, CreateUser>;
    fn authenticate<'a>(
        &'a self,
        email: &'a EmailAddress,
        password: &'a str,
        caller: CallerContext,
    ) -> Call<'a, Authentication>;
    fn find_user<'a>(&'a self, email: &'a EmailAddress) -> Call<'a, Option<ProviderUser>>;
    fn mark_email_verified<'a>(&'a self, subject: &'a str) -> Call<'a, ()>;
    fn set_password<'a>(&'a self, subject: &'a str, password: &'a str) -> Call<'a, SetPassword>;
    fn begin_password_reset<'a>(&'a self, email: &'a EmailAddress) -> Call<'a, ResetStart>;
    fn finish_password_reset<'a>(
        &'a self,
        token: &'a str,
        new_password: &'a str,
    ) -> Call<'a, ResetFinish>;
    fn end_session<'a>(&'a self, provider_session: &'a str) -> Call<'a, ()>;
    fn delete_user<'a>(&'a self, subject: &'a str) -> Call<'a, ()>;
}

impl<P: IdentityProvider> ErasedProvider for P {
    fn kind(&self) -> IdentityProviderKind {
        IdentityProvider::kind(self)
    }

    fn create_user<'a>(
        &'a self,
        email: &'a EmailAddress,
        password: &'a str,
        caller: CallerContext,
    ) -> Call<'a, CreateUser> {
        Box::pin(IdentityProvider::create_user(self, email, password, caller))
    }

    fn authenticate<'a>(
        &'a self,
        email: &'a EmailAddress,
        password: &'a str,
        caller: CallerContext,
    ) -> Call<'a, Authentication> {
        Box::pin(IdentityProvider::authenticate(
            self, email, password, caller,
        ))
    }

    fn find_user<'a>(&'a self, email: &'a EmailAddress) -> Call<'a, Option<ProviderUser>> {
        Box::pin(IdentityProvider::find_user(self, email))
    }

    fn mark_email_verified<'a>(&'a self, subject: &'a str) -> Call<'a, ()> {
        Box::pin(IdentityProvider::mark_email_verified(self, subject))
    }

    fn set_password<'a>(&'a self, subject: &'a str, password: &'a str) -> Call<'a, SetPassword> {
        Box::pin(IdentityProvider::set_password(self, subject, password))
    }

    fn begin_password_reset<'a>(&'a self, email: &'a EmailAddress) -> Call<'a, ResetStart> {
        Box::pin(IdentityProvider::begin_password_reset(self, email))
    }

    fn finish_password_reset<'a>(
        &'a self,
        token: &'a str,
        new_password: &'a str,
    ) -> Call<'a, ResetFinish> {
        Box::pin(IdentityProvider::finish_password_reset(
            self,
            token,
            new_password,
        ))
    }

    fn end_session<'a>(&'a self, provider_session: &'a str) -> Call<'a, ()> {
        Box::pin(IdentityProvider::end_session(self, provider_session))
    }

    fn delete_user<'a>(&'a self, subject: &'a str) -> Call<'a, ()> {
        Box::pin(IdentityProvider::delete_user(self, subject))
    }
}

/// Where the credentials live. Chosen once by composition.
#[derive(Clone)]
pub struct IdentitySource(Source);

#[derive(Clone)]
enum Source {
    LocalPasswords,
    Provider(Arc<dyn ErasedProvider>),
}

impl IdentitySource {
    /// Argon2id hashes in the account store: development and self-hosting only. Hosted
    /// production configures a provider instead.
    pub fn local_passwords() -> Self {
        Self(Source::LocalPasswords)
    }

    pub fn provider(provider: impl IdentityProvider) -> Self {
        Self(Source::Provider(Arc::new(provider)))
    }

    pub fn is_local(&self) -> bool {
        matches!(self.0, Source::LocalPasswords)
    }

    /// The remote provider's kind, or `None` for local passwords.
    pub fn provider_kind(&self) -> Option<IdentityProviderKind> {
        self.erased().map(|provider| provider.kind())
    }

    pub(crate) fn erased(&self) -> Option<&Arc<dyn ErasedProvider>> {
        match &self.0 {
            Source::LocalPasswords => None,
            Source::Provider(provider) => Some(provider),
        }
    }
}
