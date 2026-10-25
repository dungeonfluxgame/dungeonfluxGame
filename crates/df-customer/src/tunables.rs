//! Owner-tunable portal values (coding style 8.9): `content/portal.json`, schema
//! `dungeonflux.portal/1`. Every knob has a built-in default; a missing, malformed or
//! out-of-range value falls back to that default and is named in a `SETTING` audit line.

use std::time::Duration;

use df_auth::{throttle::ThrottlePolicy, web_session_policy::SessionLifetimes};
use serde_json::Value;

use crate::throttle::ThrottleScope;

pub const PORTAL_SCHEMA: &str = "dungeonflux.portal/1";
const MINIMUM_SIGN_IN_FAILURE_FLOOR_MS: u64 = 250;
const DEFAULT_SIGN_IN_FAILURE_FLOOR_MS: u64 = 750;
const MAXIMUM_SIGN_IN_FAILURE_FLOOR_MS: u64 = 2_000;
const MAXIMUM_TERMS_VERSION_BYTES: usize = 64;

const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;

/// Every throttled scope, in the order of the tunables file.
pub const SCOPES: [ThrottleScope; 12] = [
    ThrottleScope::LoginFailuresPerEmail,
    ThrottleScope::LoginPerIp,
    ThrottleScope::SignupPerIp,
    ThrottleScope::SignupPerEmail,
    ThrottleScope::ResetPerEmail,
    ThrottleScope::ResetPerIp,
    ThrottleScope::VerificationResendPerAccount,
    ThrottleScope::TokenPagePerIp,
    ThrottleScope::PasswordChangePerAccount,
    ThrottleScope::EmailChangePerAccount,
    ThrottleScope::ProfilePerAccount,
    ThrottleScope::DataPerAccount,
];

/// Plan 6.2 and `service-operations.md` defaults.
pub const fn default_policy(scope: ThrottleScope) -> ThrottlePolicy {
    let (limit, window_s) = match scope {
        ThrottleScope::LoginFailuresPerEmail => (5, 15 * MINUTE),
        ThrottleScope::LoginPerIp => (20, 15 * MINUTE),
        ThrottleScope::SignupPerIp => (5, HOUR),
        ThrottleScope::SignupPerEmail => (3, HOUR),
        ThrottleScope::ResetPerEmail => (3, HOUR),
        ThrottleScope::ResetPerIp => (10, MINUTE),
        ThrottleScope::VerificationResendPerAccount => (3, HOUR),
        ThrottleScope::TokenPagePerIp => (10, MINUTE),
        ThrottleScope::PasswordChangePerAccount => (5, 15 * MINUTE),
        ThrottleScope::EmailChangePerAccount => (3, HOUR),
        ThrottleScope::ProfilePerAccount => (30, HOUR),
        ThrottleScope::DataPerAccount => (3, DAY),
    };
    ThrottlePolicy {
        limit,
        window: Duration::from_secs(window_s),
    }
}

/// The loaded values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tunables {
    pub sessions: SessionLifetimes,
    /// How long a closed account can be reopened before its personal data is deleted.
    pub closure_window: Duration,
    /// Current terms and privacy text version required for new account consent.
    pub current_terms_version: String,
    /// Minimum response duration for failed sign-ins, above the store deadline.
    pub sign_in_failure_floor: Duration,
    policies: [ThrottlePolicy; SCOPES.len()],
}

impl Default for Tunables {
    fn default() -> Self {
        Self {
            sessions: SessionLifetimes {
                idle: Duration::from_secs(14 * DAY),
                absolute: Duration::from_secs(30 * DAY),
                touch_interval: Duration::from_secs(5 * MINUTE),
            },
            closure_window: Duration::from_secs(30 * DAY),
            current_terms_version: "terms-2026-10".to_owned(),
            sign_in_failure_floor: Duration::from_millis(DEFAULT_SIGN_IN_FAILURE_FLOOR_MS),
            policies: SCOPES.map(default_policy),
        }
    }
}

impl Tunables {
    pub fn policy(&self, scope: ThrottleScope) -> ThrottlePolicy {
        SCOPES
            .iter()
            .position(|known| *known == scope)
            .map_or_else(|| default_policy(scope), |index| self.policies[index])
    }

    /// Parses the file's text. Returns the values and the names of the knobs that fell back.
    pub fn parse(text: &str) -> (Self, Vec<String>) {
        let mut tunables = Self::default();
        let mut fallbacks = Vec::new();
        let Ok(document) = serde_json::from_str::<Value>(text) else {
            fallbacks.push("file".to_owned());
            audit_fallbacks(&fallbacks);
            return (tunables, fallbacks);
        };
        if document.get("schema").and_then(Value::as_str) != Some(PORTAL_SCHEMA) {
            fallbacks.push("schema".to_owned());
            audit_fallbacks(&fallbacks);
            return (tunables, fallbacks);
        }
        let mut read = |path: &str, minimum: u64, maximum: u64| -> Option<u64> {
            let value = path
                .split('.')
                .try_fold(&document, |node, part| node.get(part));
            match value
                .and_then(Value::as_u64)
                .filter(|value| (minimum..=maximum).contains(value))
            {
                Some(value) => Some(value),
                None => {
                    fallbacks.push(path.to_owned());
                    None
                }
            }
        };
        if let Some(hours) = read("sessions.idle_hours", 1, 90 * 24) {
            tunables.sessions.idle = Duration::from_secs(hours * HOUR);
        }
        if let Some(hours) = read("sessions.absolute_hours", 1, 365 * 24) {
            tunables.sessions.absolute = Duration::from_secs(hours * HOUR);
        }
        if let Some(minutes) = read("sessions.touch_minutes", 1, 60) {
            tunables.sessions.touch_interval = Duration::from_secs(minutes * MINUTE);
        }
        if let Some(days) = read("closure_window_days", 1, 365) {
            tunables.closure_window = Duration::from_secs(days * DAY);
        }
        let mut terms_version_fallback = false;
        match document
            .get("terms_version")
            .and_then(Value::as_str)
            .filter(|version| !version.is_empty() && version.len() <= MAXIMUM_TERMS_VERSION_BYTES)
        {
            Some(version) => tunables.current_terms_version = version.to_owned(),
            None => terms_version_fallback = true,
        }
        for (index, scope) in SCOPES.iter().enumerate() {
            let base = format!("limits.{}", scope.label());
            let limit = read(&format!("{base}.limit"), 1, 10_000);
            let window = read(&format!("{base}.window_minutes"), 1, 7 * 24 * 60);
            if let (Some(limit), Some(window)) = (limit, window) {
                tunables.policies[index] = ThrottlePolicy {
                    limit: u32::try_from(limit).unwrap_or(u32::MAX),
                    window: Duration::from_secs(window * MINUTE),
                };
            }
        }
        if terms_version_fallback {
            fallbacks.push("terms_version".to_owned());
        }
        if let Some(configured) = document.get("sign_in_failure_floor_ms") {
            match configured.as_u64().filter(|millis| {
                (MINIMUM_SIGN_IN_FAILURE_FLOOR_MS..=MAXIMUM_SIGN_IN_FAILURE_FLOOR_MS)
                    .contains(millis)
            }) {
                Some(millis) => tunables.sign_in_failure_floor = Duration::from_millis(millis),
                None => fallbacks.push("sign_in_failure_floor_ms".to_owned()),
            }
        }
        audit_fallbacks(&fallbacks);
        (tunables, fallbacks)
    }
}

fn audit_fallbacks(fallbacks: &[String]) {
    for name in fallbacks {
        df_observe::record_audit_line("settings.fallback", format!("SETTING fallback name={name}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_file_loads_without_a_fallback() {
        let text = include_str!("../../../content/portal.json");
        let (tunables, fallbacks) = Tunables::parse(text);
        assert!(fallbacks.is_empty(), "{fallbacks:?}");
        assert_eq!(tunables, Tunables::default());
    }

    #[test]
    fn the_current_terms_version_is_loaded_from_portal_configuration() {
        let text = r#"{"schema":"dungeonflux.portal/1","terms_version":"2026-10-r2","sign_in_failure_floor_ms":1250}"#;
        let (tunables, _) = Tunables::parse(text);
        assert_eq!(tunables.current_terms_version, "2026-10-r2");
        assert_eq!(tunables.sign_in_failure_floor, Duration::from_millis(1_250));
    }

    #[test]
    fn missing_empty_and_overlong_terms_versions_fall_back_and_are_named() {
        let texts = [
            r#"{"schema":"dungeonflux.portal/1"}"#.to_owned(),
            r#"{"schema":"dungeonflux.portal/1","terms_version":""}"#.to_owned(),
            format!(
                r#"{{"schema":"dungeonflux.portal/1","terms_version":"{}"}}"#,
                "x".repeat(MAXIMUM_TERMS_VERSION_BYTES + 1)
            ),
        ];
        for text in texts {
            let (tunables, fallbacks) = Tunables::parse(&text);
            assert_eq!(tunables.current_terms_version, "terms-2026-10");
            assert!(fallbacks.contains(&"terms_version".to_owned()));
        }
    }

    #[test]
    fn an_out_of_range_knob_falls_back_alone_and_is_named() {
        let text = r#"{"schema": "dungeonflux.portal/1",
            "sessions": {"idle_hours": 0, "absolute_hours": 48},
            "limits": {"login_per_ip": {"limit": 7, "window_minutes": 5}}}"#;
        let (tunables, fallbacks) = Tunables::parse(text);
        assert!(fallbacks.contains(&"sessions.idle_hours".to_owned()));
        assert!(
            fallbacks.contains(&"sessions.touch_minutes".to_owned()),
            "missing is named"
        );
        assert!(!fallbacks.contains(&"sessions.absolute_hours".to_owned()));
        assert!(
            !fallbacks
                .iter()
                .any(|knob| knob.starts_with("limits.login_per_ip"))
        );
        assert_eq!(tunables.sessions.idle, Tunables::default().sessions.idle);
        assert_eq!(tunables.sessions.absolute, Duration::from_secs(48 * HOUR));
        assert_eq!(
            tunables.policy(ThrottleScope::LoginPerIp),
            ThrottlePolicy {
                limit: 7,
                window: Duration::from_secs(5 * MINUTE)
            }
        );
    }

    #[test]
    fn a_malformed_file_or_wrong_schema_keeps_every_default() {
        assert_eq!(Tunables::parse("{not json").1, ["file"]);
        assert_eq!(Tunables::parse(r#"{"schema": "other/1"}"#).1, ["schema"]);
    }
}
