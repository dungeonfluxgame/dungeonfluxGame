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

use std::{env::VarError, io, sync::OnceLock, time::Duration};

const DEFAULT_COMMIT_SECONDS: u64 = 12;
const DEFAULT_REPLY_SECONDS: u64 = 20;
/// The browser waits this much longer than the server, so the server's answer arrives first.
const BROWSER_MARGIN: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Timing {
    /// The longest a database commit may run before its outcome is unknown.
    pub(super) commit: Duration,
    /// The longest a handler waits for the game owner.
    pub(super) reply: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            commit: Duration::from_secs(DEFAULT_COMMIT_SECONDS),
            reply: Duration::from_secs(DEFAULT_REPLY_SECONDS),
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

impl Timing {
    pub(super) fn from_inputs(
        commit: Result<String, VarError>,
        reply: Result<String, VarError>,
    ) -> Result<Self, io::Error> {
        let timing = Self {
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
        Self::from_inputs(
            std::env::var("DF_GAMEPLAY_COMMIT_SECONDS"),
            std::env::var("DF_GAMEPLAY_REPLY_SECONDS"),
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
        Timing::from_inputs(Ok(commit.to_owned()), Ok(reply.to_owned()))
    }

    #[test]
    fn the_defaults_are_the_generous_bounds() {
        let timing =
            Timing::from_inputs(Err(VarError::NotPresent), Err(VarError::NotPresent)).unwrap();
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
    fn the_browser_timeout_follows_the_reply_bound() {
        let timing = ok("2", "9").unwrap();
        assert_eq!(timing.browser_request(), Duration::from_secs(14));
        let bounds = timing.transaction_bounds();
        assert!(bounds.rollback < bounds.transaction && bounds.driver_join < timing.reply);
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
