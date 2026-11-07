//! The answer of every dev route of the host, as the page reads it:
//! `{"outcome":"done","value":..}` or
//! `{"outcome":"rejected","error":{"error":"<key>",..},"message":".."}` (the host's
//! `df_table_host::dev_http::DevReply`).
//!
//! The page cannot depend on the host crate, so it reads only what it shows: the value, the closed
//! key of the typed error and the host's own sentence. A refusal from the guard in front of every
//! route has no typed error; its key is empty.

use serde::Deserialize;

/// The host's answer to a dev request.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum DevReply<T> {
    Done {
        value: T,
    },
    Rejected {
        #[serde(default)]
        error: ErrorKey,
        message: String,
    },
}

/// The typed error of a refusal, as far as the page reads it: its tag.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ErrorKey {
    #[serde(default)]
    pub error: String,
}

/// A refusal: the closed key of the host's typed error (empty when it has none) and its sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    pub key: String,
    pub message: String,
}

impl<T> DevReply<T> {
    /// The value, or the refusal.
    pub fn into_result(self) -> Result<T, Rejection> {
        match self {
            Self::Done { value } => Ok(value),
            Self::Rejected { error, message } => Err(Rejection {
                key: error.error,
                message,
            }),
        }
    }
}
