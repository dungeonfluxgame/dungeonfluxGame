//! Owner-tunable values with the design's defaults (`content/campaigns.json`, section 11).
//! Validated at load: a missing, malformed or out-of-range value falls back to that knob's
//! default and is named through `report` (the caller turns it into an audit line); a combination
//! that cannot work is refused outright.

use std::time::Duration;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tunables {
    pub maximum_seats: u32,
    pub default_seats: u32,
    pub maximum_contributors: u32,
    pub pending_join_request_cap: u32,
    pub invitation_lifetime_hours: u32,
    pub join_code_lifetime_hours: u32,
    pub lookup_limit_per_ip: u32,
    pub lookup_limit_per_account: u32,
    pub lookup_limit_process_per_minute: u32,
    /// The windows of the per-address and per-account guess layers: 10 per 15 minutes per address
    /// and 20 per hour per account (plan 14.1).
    pub lookup_window_ip_minutes: u32,
    pub lookup_window_account_minutes: u32,
    pub preview_limit_per_ip_per_minute: u32,
    pub accept_limit_per_account_per_hour: u32,
    pub invitation_create_limit_per_account_per_hour: u32,
    pub display_approval_limit_per_account: u32,
    pub display_status_limit_per_ip_per_minute: u32,
    /// Redeem and renew together, per address: the layer in front of the per-pairing and per-ticket
    /// windows, which are keyed by cookies the caller chooses.
    pub display_ticket_limit_per_ip_per_minute: u32,
    pub display_redeem_limit_per_pairing_per_hour: u32,
    pub display_renew_limit_per_ticket_per_hour: u32,
    pub member_change_limit_per_account_per_minute: u32,
    pub campaigns_per_account: u32,
    /// 0 means the campaign's seat count (players) or the contributor limit (contributors).
    pub invitation_default_max_uses: u32,
    pub retention_tick_seconds: u32,
    /// The notice an owner's closure gives before an archived campaign is deleted; never below
    /// 14 days.
    pub closure_archived_delete_notice_days: u32,
    pub throttle_loose_multiplier: u32,
    pub throttle_maximum_keys_tight: usize,
    pub throttle_maximum_keys_loose: usize,
    pub throttle_maximum_keys_account: usize,
    pub display_challenge_minutes: u32,
    pub display_ticket_hours: u32,
    pub display_renewal_window_minutes: u32,
    pub display_pairing_maximum_hours: u32,
    pub display_reissue_window_seconds: u32,
    pub display_challenge_limit_per_ip_per_minute: u32,
    pub display_pending_per_ip_maximum: u32,
    pub display_pending_hard_limit: u32,
    pub maximum_displays_per_session: u32,
    pub membership_read_deadline_ms: u32,
    pub feed_poll_seconds: u32,
    pub feed_maximum_lag_seconds: u32,
    pub feed_reconcile_seconds: u32,
    pub feed_transaction_timeout_seconds: u32,
    pub feed_overlap_seconds: u32,
    pub leave_all_chunk: u32,
    pub transfer_expiry_hours: u32,
    pub require_verified_email_to_join: bool,
}

impl Default for Tunables {
    fn default() -> Self {
        Self {
            maximum_seats: 6,
            default_seats: 4,
            maximum_contributors: 12,
            pending_join_request_cap: 20,
            invitation_lifetime_hours: 7 * 24,
            join_code_lifetime_hours: 24,
            lookup_limit_per_ip: 10,
            lookup_limit_per_account: 20,
            lookup_limit_process_per_minute: 600,
            lookup_window_ip_minutes: 15,
            lookup_window_account_minutes: 60,
            preview_limit_per_ip_per_minute: 30,
            accept_limit_per_account_per_hour: 10,
            invitation_create_limit_per_account_per_hour: 20,
            display_approval_limit_per_account: 10,
            display_status_limit_per_ip_per_minute: 30,
            display_ticket_limit_per_ip_per_minute: 30,
            display_redeem_limit_per_pairing_per_hour: 12,
            display_renew_limit_per_ticket_per_hour: 12,
            member_change_limit_per_account_per_minute: 60,
            campaigns_per_account: 20,
            invitation_default_max_uses: 0,
            retention_tick_seconds: 30,
            closure_archived_delete_notice_days: 30,
            throttle_loose_multiplier: 10,
            throttle_maximum_keys_tight: 262_144,
            throttle_maximum_keys_loose: 65_536,
            throttle_maximum_keys_account: 65_536,
            display_challenge_minutes: 10,
            display_ticket_hours: 12,
            display_renewal_window_minutes: 60,
            display_pairing_maximum_hours: 24,
            display_reissue_window_seconds: 60,
            display_challenge_limit_per_ip_per_minute: 6,
            display_pending_per_ip_maximum: 5,
            display_pending_hard_limit: 100_000,
            maximum_displays_per_session: 4,
            membership_read_deadline_ms: 1_000,
            feed_poll_seconds: 2,
            feed_maximum_lag_seconds: 5,
            feed_reconcile_seconds: 30,
            feed_transaction_timeout_seconds: 10,
            feed_overlap_seconds: 15,
            leave_all_chunk: 20,
            transfer_expiry_hours: 48,
            require_verified_email_to_join: false,
        }
    }
}

/// A combination of values that cannot work. Loading refuses it instead of falling back.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TunablesRefusal {
    /// The overlap must exceed the transaction timeout, or a late commit could be read past.
    OverlapNotAboveTimeout,
    DefaultSeatsAboveMaximum,
}

impl Tunables {
    pub fn validate(&self) -> Result<(), TunablesRefusal> {
        if self.feed_overlap_seconds <= self.feed_transaction_timeout_seconds {
            return Err(TunablesRefusal::OverlapNotAboveTimeout);
        }
        if self.default_seats > self.maximum_seats {
            return Err(TunablesRefusal::DefaultSeatsAboveMaximum);
        }
        Ok(())
    }

    pub fn invitation_lifetime_ms(&self) -> u64 {
        hours_ms(self.invitation_lifetime_hours)
    }

    pub fn join_code_lifetime_ms(&self) -> u64 {
        hours_ms(self.join_code_lifetime_hours)
    }

    pub fn display_challenge_ms(&self) -> u64 {
        u64::from(self.display_challenge_minutes) * 60_000
    }

    pub fn display_ticket_ms(&self) -> u64 {
        hours_ms(self.display_ticket_hours)
    }

    pub fn display_renewal_window_ms(&self) -> u64 {
        u64::from(self.display_renewal_window_minutes) * 60_000
    }

    pub fn display_pairing_maximum_ms(&self) -> u64 {
        hours_ms(self.display_pairing_maximum_hours)
    }

    pub fn display_reissue_window_ms(&self) -> u64 {
        u64::from(self.display_reissue_window_seconds) * 1_000
    }

    pub fn transfer_expiry_ms(&self) -> u64 {
        hours_ms(self.transfer_expiry_hours)
    }

    pub fn closure_archived_delete_notice_ms(&self) -> u64 {
        hours_ms(self.closure_archived_delete_notice_days * 24)
    }

    pub fn retention_tick(&self) -> Duration {
        Duration::from_secs(u64::from(self.retention_tick_seconds))
    }

    pub fn feed_poll(&self) -> Duration {
        Duration::from_secs(u64::from(self.feed_poll_seconds))
    }

    /// Loads the tunables from the JSON text of `content/campaigns.json`. Every knob is optional;
    /// a missing, mistyped or out-of-range knob keeps its default and is reported once through
    /// `report`. A combination that cannot work is refused.
    pub fn from_json(
        text: &str,
        mut report: impl FnMut(&'static str),
    ) -> Result<Self, TunablesRefusal> {
        let mut tunables = Self::default();
        let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
            report("file");
            return Ok(tunables);
        };
        macro_rules! knob {
            ($field:ident, $low:expr, $high:expr) => {
                if let Some(raw) = value.get(stringify!($field)) {
                    match raw.as_u64().and_then(|n| u32::try_from(n).ok()) {
                        Some(n) if ($low..=$high).contains(&n) => tunables.$field = n,
                        _ => report(stringify!($field)),
                    }
                }
            };
        }
        knob!(maximum_seats, 1, 64);
        knob!(default_seats, 1, 64);
        knob!(maximum_contributors, 0, 1_000);
        knob!(pending_join_request_cap, 1, 1_000);
        knob!(invitation_lifetime_hours, 1, 24 * 90);
        knob!(join_code_lifetime_hours, 1, 24 * 7);
        knob!(lookup_limit_per_ip, 1, 10_000);
        knob!(lookup_limit_per_account, 1, 10_000);
        knob!(lookup_limit_process_per_minute, 1, 1_000_000);
        knob!(lookup_window_ip_minutes, 1, 24 * 60);
        knob!(lookup_window_account_minutes, 1, 24 * 60);
        knob!(preview_limit_per_ip_per_minute, 1, 10_000);
        knob!(accept_limit_per_account_per_hour, 1, 10_000);
        knob!(invitation_create_limit_per_account_per_hour, 1, 10_000);
        knob!(display_approval_limit_per_account, 1, 10_000);
        knob!(display_status_limit_per_ip_per_minute, 1, 10_000);
        knob!(display_ticket_limit_per_ip_per_minute, 1, 10_000);
        knob!(display_redeem_limit_per_pairing_per_hour, 1, 10_000);
        knob!(display_renew_limit_per_ticket_per_hour, 1, 10_000);
        knob!(member_change_limit_per_account_per_minute, 1, 10_000);
        knob!(campaigns_per_account, 1, 10_000);
        knob!(invitation_default_max_uses, 0, 1_000);
        knob!(retention_tick_seconds, 1, 3_600);
        knob!(closure_archived_delete_notice_days, 14, 365);
        knob!(throttle_loose_multiplier, 1, 1_000);
        knob!(display_challenge_minutes, 1, 120);
        knob!(display_ticket_hours, 1, 72);
        knob!(display_renewal_window_minutes, 1, 24 * 60);
        knob!(display_pairing_maximum_hours, 1, 24 * 7);
        knob!(display_reissue_window_seconds, 1, 3_600);
        knob!(display_challenge_limit_per_ip_per_minute, 1, 10_000);
        knob!(display_pending_per_ip_maximum, 1, 1_000);
        knob!(display_pending_hard_limit, 1, 100_000_000);
        knob!(maximum_displays_per_session, 1, 32);
        knob!(membership_read_deadline_ms, 50, 60_000);
        knob!(feed_poll_seconds, 1, 60);
        knob!(feed_maximum_lag_seconds, 1, 600);
        knob!(feed_reconcile_seconds, 1, 3_600);
        knob!(feed_transaction_timeout_seconds, 1, 300);
        knob!(feed_overlap_seconds, 2, 3_600);
        knob!(leave_all_chunk, 1, 1_000);
        knob!(transfer_expiry_hours, 1, 24 * 30);
        if let Some(raw) = value.get("require_verified_email_to_join") {
            match raw.as_bool() {
                Some(flag) => tunables.require_verified_email_to_join = flag,
                None => report("require_verified_email_to_join"),
            }
        }
        tunables.validate()?;
        Ok(tunables)
    }
}

fn hours_ms(hours: u32) -> u64 {
    u64::from(hours) * 3_600_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_designs_and_valid() {
        let tunables = Tunables::default();
        assert_eq!(tunables.maximum_seats, 6);
        assert_eq!(tunables.maximum_contributors, 12);
        assert_eq!(tunables.pending_join_request_cap, 20);
        assert_eq!(tunables.feed_transaction_timeout_seconds, 10);
        assert_eq!(tunables.feed_overlap_seconds, 15);
        assert_eq!(tunables.validate(), Ok(()));
    }

    #[test]
    fn configuration_is_refused_when_the_overlap_does_not_exceed_the_transaction_timeout() {
        let tunables = Tunables {
            feed_overlap_seconds: 10,
            ..Tunables::default()
        };
        assert_eq!(
            tunables.validate(),
            Err(TunablesRefusal::OverlapNotAboveTimeout)
        );
        assert_eq!(
            Tunables::from_json(r#"{"feed_overlap_seconds": 10}"#, |_| {}),
            Err(TunablesRefusal::OverlapNotAboveTimeout)
        );
    }

    #[test]
    fn a_bad_knob_keeps_its_default_and_is_reported_once() {
        let mut reported = Vec::new();
        let tunables = Tunables::from_json(
            r#"{"maximum_seats": 8, "maximum_contributors": "many", "feed_poll_seconds": 0}"#,
            |name| reported.push(name),
        )
        .unwrap();
        assert_eq!(tunables.maximum_seats, 8);
        assert_eq!(tunables.maximum_contributors, 12);
        assert_eq!(tunables.feed_poll_seconds, 2);
        assert_eq!(reported, ["maximum_contributors", "feed_poll_seconds"]);
    }

    #[test]
    fn the_guess_windows_are_the_designs_and_configurable() {
        let tunables = Tunables::default();
        assert_eq!(tunables.lookup_window_ip_minutes, 15);
        assert_eq!(tunables.lookup_window_account_minutes, 60);
        assert_eq!(tunables.preview_limit_per_ip_per_minute, 30);
        assert_eq!(tunables.accept_limit_per_account_per_hour, 10);
        assert_eq!(tunables.invitation_create_limit_per_account_per_hour, 20);
        assert_eq!(tunables.display_approval_limit_per_account, 10);
        assert_eq!(tunables.display_status_limit_per_ip_per_minute, 30);
        assert_eq!(tunables.display_ticket_limit_per_ip_per_minute, 30);
        assert_eq!(tunables.display_redeem_limit_per_pairing_per_hour, 12);
        assert_eq!(tunables.display_renew_limit_per_ticket_per_hour, 12);
        assert_eq!(tunables.campaigns_per_account, 20);
        assert_eq!(tunables.retention_tick_seconds, 30);
        let loaded = Tunables::from_json(
            r#"{"lookup_window_ip_minutes": 30, "campaigns_per_account": 3}"#,
            |_| {},
        )
        .unwrap();
        assert_eq!(loaded.lookup_window_ip_minutes, 30);
        assert_eq!(loaded.campaigns_per_account, 3);
    }

    #[test]
    fn the_closure_notice_is_never_shorter_than_fourteen_days() {
        let mut reported = Vec::new();
        let tunables =
            Tunables::from_json(r#"{"closure_archived_delete_notice_days": 7}"#, |name| {
                reported.push(name);
            })
            .unwrap();
        assert_eq!(
            tunables.closure_archived_delete_notice_days, 30,
            "the knob keeps its default"
        );
        assert_eq!(reported, ["closure_archived_delete_notice_days"]);
        let shortest =
            Tunables::from_json(r#"{"closure_archived_delete_notice_days": 14}"#, |_| {}).unwrap();
        assert_eq!(
            shortest.closure_archived_delete_notice_ms(),
            14 * 24 * 3_600_000
        );
    }

    #[test]
    fn a_file_that_is_not_json_falls_back_to_the_defaults() {
        let mut reported = Vec::new();
        let tunables = Tunables::from_json("not json", |name| reported.push(name)).unwrap();
        assert_eq!(tunables, Tunables::default());
        assert_eq!(reported, ["file"]);
    }
}
