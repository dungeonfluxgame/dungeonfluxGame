//! How long the demonstration waits for the database and for a reply, set once per run.
//!
//! On a busy machine a commit can take longer than a tight bound; its outcome then becomes
//! unknown and the session fences itself until it has reconciled. The bounds are therefore
//! settings with generous defaults, and the browser's request timeout is derived from the same
//! reply bound and served to the page, so the two cannot drift apart.
//!
//! * `DF_GAMEPLAY_COMMIT_SECONDS` (1 to 120, default 12): the longest a database commit may run.
//! * `DF_GAMEPLAY_REPLY_SECONDS` (1 to 300, default 20): the longest a request waits for the
//!   game owner. It must exceed the commit bound.
//! * `DF_GAMEPLAY_LEASE_SECONDS` (10 to 3600, default 300): how long the session's owner lease
//!   and access grants last. The owner renews them every fifth of that, so an idle table never
//!   loses its game; a short value exists to test exactly that.

use std::{env::VarError, io, sync::OnceLock, time::Duration};

const DEFAULT_COMMIT_SECONDS: u64 = 12;
const DEFAULT_REPLY_SECONDS: u64 = 20;
const DEFAULT_LEASE_SECONDS: u64 = 300;
/// The browser waits this much longer than the server, so the server's answer arrives first.
const BROWSER_MARGIN: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Timing {
    /// The longest a database commit may run before its outcome is unknown.
    pub(super) commit: Duration,
    /// The longest a handler waits for the game owner.
    pub(super) reply: Duration,
    /// How long the owner lease and access grants last before they are renewed.
    pub(super) lease: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            commit: Duration::from_secs(DEFAULT_COMMIT_SECONDS),
            reply: Duration::from_secs(DEFAULT_REPLY_SECONDS),
            lease: Duration::from_secs(DEFAULT_LEASE_SECONDS),
        }
    }
}

fn seconds(
    input: Result<String, VarError>,
    name: &str,
    maximum: u64,
    default: u64,
) -> Result<Duration, io::Error> {
    let value = match input {
        Ok(value) => value,
        Err(VarError::NotPresent) => return Ok(Duration::from_secs(default)),
        Err(VarError::NotUnicode(_)) => return Err(refused(name)),
    };
    value
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|seconds| (1..=maximum).contains(seconds))
        .map(Duration::from_secs)
        .ok_or_else(|| refused(name))
}

fn refused(name: &str) -> io::Error {
    io::Error::other(format!(
        "{name} refused: a whole number of seconds in range expected"
    ))
}

fn reply_setting(reply: Result<String, VarError>, codex: bool) -> Result<String, VarError> {
    match reply {
        Err(VarError::NotPresent) if codex => Ok("300".to_owned()),
        value => value,
    }
}

impl Timing {
    pub(super) fn from_inputs(
        commit: Result<String, VarError>,
        reply: Result<String, VarError>,
        lease: Result<String, VarError>,
    ) -> Result<Self, io::Error> {
        let lease = seconds(
            lease,
            "DF_GAMEPLAY_LEASE_SECONDS",
            3600,
            DEFAULT_LEASE_SECONDS,
        )?;
        if lease < Duration::from_secs(10) {
            return Err(refused("DF_GAMEPLAY_LEASE_SECONDS"));
        }
        let timing = Self {
            lease,
            commit: seconds(
                commit,
                "DF_GAMEPLAY_COMMIT_SECONDS",
                120,
                DEFAULT_COMMIT_SECONDS,
            )?,
            reply: seconds(
                reply,
                "DF_GAMEPLAY_REPLY_SECONDS",
                300,
                DEFAULT_REPLY_SECONDS,
            )?,
        };
        if timing.reply <= timing.commit {
            return Err(io::Error::other(
                "DF_GAMEPLAY_REPLY_SECONDS refused: it must exceed DF_GAMEPLAY_COMMIT_SECONDS",
            ));
        }
        Ok(timing)
    }

    pub(super) fn from_environment() -> Result<Self, io::Error> {
        let reply = reply_setting(
            std::env::var("DF_GAMEPLAY_REPLY_SECONDS"),
            std::env::var("DF_GAMEPLAY_CODEX_MODE").as_deref() == Ok("on"),
        );
        Self::from_inputs(
            std::env::var("DF_GAMEPLAY_COMMIT_SECONDS"),
            reply,
            std::env::var("DF_GAMEPLAY_LEASE_SECONDS"),
        )
    }

    /// The database bounds: a rollback and a driver join are shorter than the commit.
    pub(super) fn transaction_bounds(self) -> df_persistence::NativeTransactionBounds {
        df_persistence::NativeTransactionBounds {
            transaction: self.commit,
            rollback: (self.commit / 6).clamp(Duration::from_millis(250), Duration::from_secs(2)),
            driver_join: (self.commit * 5 / 12)
                .clamp(Duration::from_millis(500), Duration::from_secs(5)),
        }
    }

    /// What the browser waits for one request.
    pub(super) fn browser_request(self) -> Duration {
        self.reply + BROWSER_MARGIN
    }
}

static ACTIVE: OnceLock<Timing> = OnceLock::new();

/// Fixes the timing of this run. Later calls keep the first.
pub(super) fn install(timing: Timing) {
    let _first = ACTIVE.set(timing);
}

/// The timing of this run (the defaults until one is installed).
pub(super) fn active() -> Timing {
    ACTIVE.get().copied().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(commit: &str, reply: &str) -> Result<Timing, io::Error> {
        Timing::from_inputs(
            Ok(commit.to_owned()),
            Ok(reply.to_owned()),
            Err(VarError::NotPresent),
        )
    }

    #[test]
    fn the_defaults_are_the_generous_bounds() {
        let timing = Timing::from_inputs(
            Err(VarError::NotPresent),
            Err(VarError::NotPresent),
            Err(VarError::NotPresent),
        )
        .unwrap();
        assert_eq!(timing.lease, Duration::from_secs(300));
        assert_eq!(timing, Timing::default());
        assert_eq!(timing.commit, Duration::from_secs(12));
        assert_eq!(timing.reply, Duration::from_secs(20));
        let bounds = timing.transaction_bounds();
        assert_eq!(bounds.transaction, Duration::from_secs(12));
        assert_eq!(bounds.rollback, Duration::from_secs(2));
        assert_eq!(bounds.driver_join, Duration::from_secs(5));
        assert_eq!(timing.browser_request(), Duration::from_secs(25));
    }

    #[test]
    fn codex_reply_default_allows_the_backfill_chain_and_preserves_overrides() {
        assert_eq!(
            reply_setting(Err(VarError::NotPresent), true).unwrap(),
            "300"
        );
        assert!(matches!(
            reply_setting(Err(VarError::NotPresent), false),
            Err(VarError::NotPresent)
        ));
        assert_eq!(reply_setting(Ok("45".to_owned()), true).unwrap(), "45");
    }

    #[test]
    fn the_browser_timeout_follows_the_reply_bound() {
        let timing = ok("2", "9").unwrap();
        assert_eq!(timing.browser_request(), Duration::from_secs(14));
        let bounds = timing.transaction_bounds();
        assert!(bounds.rollback < bounds.transaction && bounds.driver_join < timing.reply);
    }

    #[test]
    fn the_lease_length_is_validated() {
        let with = |lease: &str| {
            Timing::from_inputs(
                Err(VarError::NotPresent),
                Err(VarError::NotPresent),
                Ok(lease.to_owned()),
            )
        };
        assert_eq!(with("20").unwrap().lease, Duration::from_secs(20));
        for bad in ["9", "0", "3601", "soon", ""] {
            assert!(with(bad).is_err(), "{bad:?} was accepted");
        }
    }

    #[test]
    fn invalid_values_are_refused_without_echoing_them() {
        for (commit, reply) in [
            ("0", "20"),
            ("121", "200"),
            ("x", "20"),
            ("12", "301"),
            ("12", "12"),
            ("30", "20"),
            ("", "20"),
        ] {
            let error = ok(commit, reply).unwrap_err().to_string();
            assert!(error.contains("DF_GAMEPLAY_"), "{commit}/{reply}: {error}");
            assert!(!error.contains(&format!("'{commit}'")));
        }
    }
}
