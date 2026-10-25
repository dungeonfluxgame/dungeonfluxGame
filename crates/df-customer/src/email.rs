//! Account email messages. Plain text, one per purpose. A message body is the only place a token
//! ever appears; message metadata in audit lines is the purpose and a sequence number.
//!
//! Every link base, the From and the Reply-To come from the `HostConfig` (decision 32); nothing
//! here names a host.

use crate::{hosts::HostConfig, tokens::SecretToken};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MailPurpose {
    VerifyEmail,
    PasswordReset,
    EmailChangeOld,
    EmailChangeNew,
    /// Sent to an existing address when someone tries to sign up with it.
    SignupAttempt,
    PasswordChanged,
    EmailChanged,
    /// Sent to every member of an archived campaign its owner's closing account will delete.
    CampaignDeletionScheduled,
}

impl MailPurpose {
    pub const fn label(self) -> &'static str {
        match self {
            Self::VerifyEmail => "verify_email",
            Self::PasswordReset => "password_reset",
            Self::EmailChangeOld => "email_change_old",
            Self::EmailChangeNew => "email_change_new",
            Self::SignupAttempt => "signup_attempt",
            Self::PasswordChanged => "password_changed",
            Self::EmailChanged => "email_changed",
            Self::CampaignDeletionScheduled => "campaign_deletion_scheduled",
        }
    }
}

#[derive(Clone)]
pub struct Message {
    /// The `From` header value, from configuration.
    pub from: String,
    /// The `Reply-To` header value (the support mailbox), from configuration.
    pub reply_to: String,
    pub to: String,
    pub purpose: MailPurpose,
    pub subject: &'static str,
    pub body: String,
}

/// Which pages the links in a message open: the account web app (`/app/...`, tokens in the URL
/// fragment so they never reach a server log or a Referer header) or the server-rendered pages
/// (`?token=`), which serve local development and self-hosting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkStyle {
    App,
    Classic,
}

/// Builds the messages, with links on the configured canonical origin.
#[derive(Clone)]
pub struct MessageBuilder {
    origin: String,
    from: String,
    reply_to: String,
    style: LinkStyle,
}

const SIGNATURE: &str = "\n\nDungeonFlux\n";

impl MessageBuilder {
    pub fn new(hosts: &HostConfig) -> Self {
        Self {
            origin: hosts.origin().as_str().to_owned(),
            from: hosts.mail().from_header(),
            reply_to: hosts.mail().reply_to_header(),
            style: hosts.style(),
        }
    }

    fn message(
        &self,
        to: &str,
        purpose: MailPurpose,
        subject: &'static str,
        body: String,
    ) -> Message {
        Message {
            from: self.from.clone(),
            reply_to: self.reply_to.clone(),
            to: to.to_owned(),
            purpose,
            subject,
            body,
        }
    }

    fn verify_link(&self, token: &str) -> String {
        match self.style {
            LinkStyle::App => format!("{}/app/verify-email#token={token}", self.origin),
            LinkStyle::Classic => format!("{}/verify-email?token={token}", self.origin),
        }
    }

    fn reset_link(&self, token: &str) -> String {
        match self.style {
            LinkStyle::App => format!("{}/app/reset-password#token={token}", self.origin),
            LinkStyle::Classic => format!("{}/password-reset/confirm?token={token}", self.origin),
        }
    }

    /// Email change is a server-rendered page in both styles; the app has no screen for it.
    fn email_change_link(&self, token: &SecretToken) -> String {
        format!(
            "{}/email-change/confirm?token={}",
            self.origin,
            token.text()
        )
    }

    fn sign_in_url(&self) -> String {
        match self.style {
            LinkStyle::App => format!("{}/app/sign-in", self.origin),
            LinkStyle::Classic => format!("{}/login", self.origin),
        }
    }

    fn forgot_password_url(&self) -> String {
        match self.style {
            LinkStyle::App => format!("{}/app/forgot-password", self.origin),
            LinkStyle::Classic => format!("{}/password-reset", self.origin),
        }
    }

    pub fn verify_email(&self, to: &str, token: &SecretToken) -> Message {
        self.message(
            to,
            MailPurpose::VerifyEmail,
            "Confirm your DungeonFlux email address",
            format!(
                "Welcome to DungeonFlux.\n\nConfirm this address by opening the link below. The \
                 link works for 24 hours.\n\n{}\n\nIf you did not create an account, ignore this \
                 message.{SIGNATURE}",
                self.verify_link(&token.text())
            ),
        )
    }

    /// `token` is whatever the password authority issued (our own hexadecimal token for local
    /// passwords, the identity provider's token otherwise).
    pub fn password_reset(&self, to: &str, token: &str) -> Message {
        self.message(
            to,
            MailPurpose::PasswordReset,
            "Reset your DungeonFlux password",
            format!(
                "Someone asked to reset the password of the DungeonFlux account for this \
                 address.\n\nTo choose a new password, open the link below within 15 minutes. It \
                 works once.\n\n{}\n\nIf it was not you, ignore this message; your password stays \
                 as it is.{SIGNATURE}",
                self.reset_link(token)
            ),
        )
    }

    pub fn signup_attempt(&self, to: &str) -> Message {
        self.message(
            to,
            MailPurpose::SignupAttempt,
            "Someone tried to sign up with your address",
            format!(
                "Someone tried to create a DungeonFlux account with this address, which already \
                 has one.\n\nIf it was you, sign in at {} or reset your password at \
                 {}.\n\nIf it was not you, no action is needed.{SIGNATURE}",
                self.sign_in_url(),
                self.forgot_password_url()
            ),
        )
    }

    pub fn email_change_old(&self, to: &str, new_address: &str, token: &SecretToken) -> Message {
        self.message(
            to,
            MailPurpose::EmailChangeOld,
            "Confirm or cancel your DungeonFlux email change",
            format!(
                "Someone asked to change the email address of your DungeonFlux account to \
                 {new_address}.\n\nOpen the link below to confirm or cancel. The change happens \
                 only when this address and the new one both confirm within 24 hours.\n\n{}\n\nIf \
                 it was not you, open the link and choose \"Cancel the change\", then change your \
                 password.{SIGNATURE}",
                self.email_change_link(token)
            ),
        )
    }

    pub fn email_change_new(&self, to: &str, token: &SecretToken) -> Message {
        self.message(
            to,
            MailPurpose::EmailChangeNew,
            "Confirm your new DungeonFlux email address",
            format!(
                "Open the link below to confirm this address for your DungeonFlux account. The \
                 change happens when your current address confirms too, within 24 hours.\n\n{}\n\n\
                 If you did not ask for this, ignore this message.{SIGNATURE}",
                self.email_change_link(token)
            ),
        )
    }

    pub fn password_changed(&self, to: &str) -> Message {
        self.message(
            to,
            MailPurpose::PasswordChanged,
            "Your DungeonFlux password was changed",
            format!(
                "The password of your DungeonFlux account was just changed and every other \
                 device was signed out.\n\nIf it was not you, reset your password at \
                 {}.{SIGNATURE}",
                self.forgot_password_url()
            ),
        )
    }

    pub fn email_changed(&self, to: &str, new_address: &str) -> Message {
        self.message(
            to,
            MailPurpose::EmailChanged,
            "Your DungeonFlux email address was changed",
            format!(
                "The email address of your DungeonFlux account is now {new_address}. Every device \
                 was signed out.{SIGNATURE}"
            ),
        )
    }

    /// Tells a member that the campaign's owner closed their account and the archived campaign
    /// will be deleted on `date` (UTC, `YYYY-MM-DD`); the campaign can be exported until then.
    pub fn campaign_deletion_scheduled(
        &self,
        to: &str,
        campaign_name: &str,
        date: &str,
    ) -> Message {
        self.message(
            to,
            MailPurpose::CampaignDeletionScheduled,
            "A DungeonFlux campaign you play in will be deleted",
            format!(
                "The owner of the archived campaign \"{campaign_name}\" closed their account, so \
                 the campaign will be deleted on {date}.\n\nUntil then you can export what you \
                 may keep from the campaign's page. If the owner signs in again before that \
                 date, the deletion is canceled.{SIGNATURE}"
            ),
        )
    }
}

/// `YYYY-MM-DD` (UTC) for a millisecond timestamp, by the civil-from-days algorithm.
pub fn utc_date(ms: u64) -> String {
    let days = i64::try_from(ms / 86_400_000).unwrap_or(i64::MAX / 2);
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_civil_utc() {
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(951_782_400_000), "2000-02-29");
        assert_eq!(utc_date(1_800_000_000_000), "2027-01-15");
    }

    fn builder(style_json: &str) -> MessageBuilder {
        let text = format!(
            r#"{{"schema": "dungeonflux.hosts/1", "origin": "https://staging.dungeonfluxdnd.com",
            "mail": {{"from_name": "DungeonFlux", "from_address": "accounts@mail.dungeonfluxdnd.com",
            "reply_to": "support@dungeonfluxdnd.com"}}{style_json}}}"#
        );
        MessageBuilder::new(&HostConfig::parse(&text).unwrap())
    }

    #[test]
    fn app_links_carry_the_token_in_the_fragment_on_the_configured_origin() {
        let message = builder("").password_reset("a@example.test", "tok123");
        assert!(
            message
                .body
                .contains("https://staging.dungeonfluxdnd.com/app/reset-password#token=tok123")
        );
        assert_eq!(
            message.from,
            "DungeonFlux <accounts@mail.dungeonfluxdnd.com>"
        );
        assert_eq!(message.reply_to, "support@dungeonfluxdnd.com");
    }

    #[test]
    fn classic_links_use_the_server_rendered_pages() {
        let message = builder(r#", "links": {"style": "classic"}"#)
            .password_reset("a@example.test", "tok123");
        assert!(
            message
                .body
                .contains("https://staging.dungeonfluxdnd.com/password-reset/confirm?token=tok123")
        );
    }
}
