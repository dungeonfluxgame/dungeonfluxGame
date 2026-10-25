//! Audit lines (design section 10): one per event through `df_observe::record_audit_line`, a
//! closed family and verb, then ids and hashes only. Never a token, a code, a secret or an
//! address: an address appears only as a truncated keyed hash, for throttle events.

use std::fmt::Write as _;

use df_types::{AccountId, CampaignId, MemberId, SessionId};

use crate::{
    errors::{Failure, Refusal, StoreError},
    ids::{InvitationId, hex_of},
};

/// A line under construction. `Family VERB k=v ...`.
#[must_use = "an audit line is recorded by emit"]
pub struct AuditLine {
    family: &'static str,
    verb: &'static str,
    fields: Vec<(&'static str, String)>,
}

impl AuditLine {
    pub fn new(family: &'static str, verb: &'static str) -> Self {
        Self {
            family,
            verb,
            fields: Vec::new(),
        }
    }

    pub fn campaign(mut self, id: CampaignId) -> Self {
        self.fields.push(("campaign", hex_of(id.as_bytes())));
        self
    }

    pub fn actor(mut self, id: AccountId) -> Self {
        self.fields.push(("actor", hex_of(id.as_bytes())));
        self
    }

    pub fn member(mut self, id: MemberId) -> Self {
        self.fields.push(("member", hex_of(id.as_bytes())));
        self
    }

    pub fn invitation(mut self, id: InvitationId) -> Self {
        self.fields.push(("invitation", id.text()));
        self
    }

    pub fn session(mut self, id: SessionId) -> Self {
        self.fields.push(("session", hex_of(id.as_bytes())));
        self
    }

    /// Marks the line for the operator's alert rule (`alert=yes`). Only the events in
    /// [`ALERT_EVENTS`] carry it.
    pub fn alert(mut self) -> Self {
        self.fields.push(("alert", "yes".to_owned()));
        self
    }

    /// A closed reason word (`expired`, `throttled`, `ok`, ...), never free text.
    pub fn reason(mut self, reason: &'static str) -> Self {
        self.fields.push(("reason", reason.to_owned()));
        self
    }

    /// The first eight hex digits of a keyed address hash.
    pub fn ip_hash(mut self, key: &[u8; 32]) -> Self {
        self.fields.push(("ip_hash", hex_of(&key[..4])));
        self
    }

    pub fn ms(mut self, elapsed_ms: u128) -> Self {
        self.fields.push(("ms", elapsed_ms.to_string()));
        self
    }

    /// The text of the line (also what the tests read).
    pub fn text(&self) -> String {
        let mut text = format!("{} {}", self.family, self.verb);
        for (name, value) in &self.fields {
            let _ = write!(text, " {name}={value}");
        }
        text
    }

    /// Records the line under `event`, a closed dotted name such as `campaign.archived`.
    pub fn emit(self, event: &'static str) {
        df_observe::record_audit_line(event, self.text());
    }
}

/// The audit events that carry `alert=yes`: a throttle layer refused because its map was full of live
/// windows (a flood of new keys, or a limit set too low for the load). S2's alert rule fires on
/// the field or on these names; the dev view's saturation counters show the same thing per layer.
pub const ALERT_EVENTS: [&str; 13] = [
    "roomcode.saturated",
    "invite.saturated",
    "invite.create_saturated",
    "campaign.change_saturated",
    "campaign.draft_saturated",
    "lobby.change_saturated",
    "lobby.host_saturated",
    "display.challenge_saturated",
    "display.status_saturated",
    "display.ticket_saturated",
    "display.approval_saturated",
    "display.redeem_saturated",
    "display.renew_saturated",
];

/// The closed reason word of a refusal, for the `refused` lines.
pub fn refusal_word(refusal: Refusal) -> &'static str {
    match refusal {
        Refusal::NotFound => "not_found",
        Refusal::NotAllowed | Refusal::OwnerOnly => "not_allowed",
        Refusal::ConfirmationRequired => "confirmation_required",
        Refusal::Join(failure) => join_word(failure),
        Refusal::InvitationUnknown | Refusal::CodeUnknown => "unknown",
        Refusal::Throttled => "throttled",
        Refusal::StaleRevision => "stale_revision",
        Refusal::CampaignUnavailable => "campaign_unavailable",
        Refusal::AccountClosing
        | Refusal::OwnerAccountNotActive
        | Refusal::HostAccountNotActive => "account_not_active",
        Refusal::SessionInProgress => "session_in_progress",
        Refusal::SessionEnded => "session_ended",
        Refusal::DisplayLimit => "display_limit",
        Refusal::DisplayNeutral | Refusal::Pairing(_) | Refusal::Ticket(_) => "display_refused",
        Refusal::OperationConflict | Refusal::OperationExpired => "operation",
        Refusal::RequestsFull => "requests_full",
        _ => "refused",
    }
}

pub fn join_word(failure: crate::join_invitation::JoinFailure) -> &'static str {
    use crate::join_invitation::JoinFailure as J;
    match failure {
        J::Expired => "expired",
        J::Revoked => "revoked",
        J::Exhausted => "exhausted",
        J::Full => "full",
        J::ContributorsFull => "contributors_full",
        J::AlreadyJoined => "already_joined",
        J::NoLongerValid => "no_longer_valid",
        J::CampaignUnavailable => "campaign_unavailable",
        J::AccountClosing => "account_closing",
    }
}

/// The word of a store error for the audit stream.
pub fn store_word(error: StoreError) -> &'static str {
    match error {
        StoreError::Unavailable => "unavailable",
        StoreError::Unknown => "outcome_unknown",
        StoreError::InvalidRecord => "invalid_record",
        StoreError::Retryable => "retryable",
        StoreError::ServiceHalting => "service_halting",
        StoreError::FenceStale => "fence_stale",
        StoreError::TimeoutNotSet => "timeout_not_set",
        StoreError::RandomUnavailable => "random_unavailable",
    }
}

pub fn failure_word(failure: Failure) -> &'static str {
    match failure {
        Failure::Refused(refusal) => refusal_word(refusal),
        Failure::Store(error) => store_word(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_carries_the_family_the_verb_and_only_ids_and_closed_words() {
        let campaign = CampaignId::from_bytes(&[1; 16]).unwrap();
        let line = AuditLine::new("INVITE", "refused")
            .campaign(campaign)
            .reason("expired")
            .ms(3);
        assert_eq!(
            line.text(),
            "INVITE refused campaign=01010101010101010101010101010101 reason=expired ms=3"
        );
    }

    #[test]
    fn an_address_appears_only_as_a_truncated_keyed_hash() {
        let line = AuditLine::new("ROOMCODE", "throttled").ip_hash(&[0xab; 32]);
        assert_eq!(line.text(), "ROOMCODE throttled ip_hash=abababab");
    }
}
