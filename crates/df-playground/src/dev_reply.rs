//! The answer of every dev route of the host: `{"outcome":"done","value":..}` or
//! `{"outcome":"rejected","error":{"error":"<key>",..},"message":".."}`.
//!
//! The page owns this type and the host (`df_table_host::dev_http`) re-exports it, so the envelope
//! cannot drift. The host fills `E` with a route module's typed error. The page reads a
//! `HostReply`: the value, the closed kind of the typed error when it is one the page words (a
//! speech refusal, a seat refusal), and the host's own sentence. A refusal from the guard in
//! front of every route has no typed error.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

use crate::voice_wire::SpeechErrorKind;

/// The answer of a dev route.
///
/// How a host route module adopts it:
///
/// 1. Give the module one `<Module>Error` tagged `"error"`, implement `RouteError` for it (its
///    `status()`), and `From<BodyError>` into a variant that wraps the `BodyError`.
/// 2. Replace the module's own reply enum by an alias, which keeps its wire JSON:
///    `pub type EngineReply<T> = DevReply<T, EngineError>;`.
/// 3. Read bodies with `parse_body(&bytes)?` and answer with `reply(result)`, which picks the
///    status from the error and writes this envelope.
/// 4. In the module's CLI, read the answer as the module's reply through `dev_client` and match
///    the error's variant, never the message text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum DevReply<T, E> {
    Done {
        value: T,
    },
    /// `message` is the error's `Display`, written by the host so a reader needs no table of
    /// texts.
    Rejected {
        error: E,
        message: String,
    },
}

impl<T, E: fmt::Display> DevReply<T, E> {
    /// The refusal of `error`, with its sentence as the message.
    pub fn rejected(error: E) -> Self {
        let message = error.to_string();
        Self::Rejected { error, message }
    }

    /// `Done` with the value or `Rejected` with the error.
    pub fn from_result(result: Result<T, E>) -> Self {
        match result {
            Ok(value) => Self::Done { value },
            Err(error) => Self::rejected(error),
        }
    }
}

/// A closed set of refusals the page words, read from the tag of a route module's typed error.
pub trait ErrorTag: Sized {
    /// The refusal `tag` names, or none for a tag of another route module.
    fn from_tag(tag: &str) -> Option<Self>;
}

impl ErrorTag for SpeechErrorKind {
    fn from_tag(tag: &str) -> Option<Self> {
        Self::from_key(tag)
    }
}

/// The host's answer as the page reads it, with the refusals of kind `K` it words. The guard's
/// refusal has no typed error, so it reads as `None`.
pub type HostReply<T, K = SpeechErrorKind> = DevReply<T, Option<ErrorKey<K>>>;

/// The typed error of a refusal, as far as the page reads it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(bound(deserialize = "K: ErrorTag"))]
pub struct ErrorKey<K> {
    /// The refusal it is; none when the tag is another route module's.
    #[serde(rename = "error", default = "no_kind", deserialize_with = "tag_kind")]
    pub kind: Option<K>,
}

fn no_kind<K>() -> Option<K> {
    None
}

/// Reads the tag of an error: the refusal of kind `K` it names, or none for a tag this page does
/// not word (the one place the tag's text is compared).
fn tag_kind<'de, D: Deserializer<'de>, K: ErrorTag>(
    deserializer: D,
) -> Result<Option<K>, D::Error> {
    let tag = String::deserialize(deserializer)?;
    Ok(K::from_tag(&tag))
}

/// A refusal: which refusal of kind `K` the host's typed error is (none when it is not one, or
/// when it has no typed error) and its sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection<K = SpeechErrorKind> {
    pub kind: Option<K>,
    pub message: String,
}

impl<T, K> DevReply<T, Option<ErrorKey<K>>> {
    /// The value, or the refusal.
    pub fn into_result(self) -> Result<T, Rejection<K>> {
        match self {
            Self::Done { value } => Ok(value),
            Self::Rejected { error, message } => Err(Rejection {
                kind: error.and_then(|error| error.kind),
                message,
            }),
        }
    }
}
