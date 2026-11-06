//! The typed client of the `dev` command line: one request to a dev route, its answer read back
//! as a typed reply.
//!
//! It owns writing a request as JSON, sending it through `client`, and reading the answer's body
//! as the route's reply whatever its status (a dev route answers a refusal with a typed reply and
//! a 4xx status). An answer that does not read is a `DevError::UnexpectedAnswer` carrying the
//! status, the start of the body and the reason. Each verb's module keeps its own formatting.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::client::{self, Response};
use crate::dev::DevError;
use crate::limits::MAXIMUM_ECHOED_ANSWER_CHARACTERS;

/// The answer as an unexpected one: its status, the start of its body and, when it was text that
/// should have been JSON of a known shape, why it did not read.
pub(crate) fn unexpected_answer(response: &Response, reason: Option<String>) -> DevError {
    DevError::UnexpectedAnswer {
        status: response.status,
        body: response
            .body
            .chars()
            .take(MAXIMUM_ECHOED_ANSWER_CHARACTERS)
            .collect(),
        reason,
    }
}

/// Reads the body of `response` as `Reply`, whatever its status.
pub(crate) fn read_answer<Reply: DeserializeOwned>(response: &Response) -> Result<Reply, DevError> {
    serde_json::from_str(&response.body)
        .map_err(|error| unexpected_answer(response, Some(error.to_string())))
}

/// Writes `request` as the JSON body of a request.
pub(crate) fn request_body<Request: Serialize>(request: &Request) -> Result<String, DevError> {
    serde_json::to_string(request).map_err(|error| DevError::Unwritable {
        what: "request",
        reason: error.to_string(),
    })
}

/// Sends a GET to `path` on the host on `port` and returns the raw answer.
pub(crate) async fn get(port: u16, path: &str) -> Result<Response, DevError> {
    client::get(port, path).await.map_err(DevError::Client)
}

/// Posts `body` (JSON text, possibly empty) to `path` on the host on `port` and returns the raw
/// answer.
pub(crate) async fn post(port: u16, path: &str, body: &str) -> Result<Response, DevError> {
    client::post_json(port, path, body)
        .await
        .map_err(DevError::Client)
}

/// Sends a GET to `path` on the host on `port` and reads the answer as `Reply`.
pub(crate) async fn get_json<Reply: DeserializeOwned>(
    port: u16,
    path: &str,
) -> Result<Reply, DevError> {
    read_answer(&get(port, path).await?)
}

/// Posts `request` as JSON to `path` on the host on `port` and reads the answer as `Reply`.
pub(crate) async fn post_json<Request: Serialize, Reply: DeserializeOwned>(
    port: u16,
    path: &str,
    request: &Request,
) -> Result<Reply, DevError> {
    let body = request_body(request)?;
    read_answer(&post(port, path, &body).await?)
}
