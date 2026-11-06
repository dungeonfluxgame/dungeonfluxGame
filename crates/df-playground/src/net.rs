//! Fetching text from, and posting JSON to, the page's own server.
//!
//! It owns one request at a time and reports a failure as a typed error that names the address and
//! carries the exception the browser threw, so the page can show it and a bug is not mistaken for
//! a refusal. It never retries; the caller's poll loop does that.

use std::fmt;

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Headers, RequestInit, Response};

use crate::dom::BrowserError;

/// Why a request did not give text.
#[derive(Debug, Clone, PartialEq)]
pub enum NetError {
    /// The page has no window to ask from.
    NoWindow,
    /// The request could not be made at all.
    Request {
        what: &'static str,
        cause: BrowserError,
    },
    /// The host could not be reached.
    Unreachable {
        address: String,
        cause: BrowserError,
    },
    /// Something answered that is not a response.
    NotAResponse { address: String },
    /// The answer's body could not be read.
    Unreadable {
        address: String,
        cause: BrowserError,
    },
    /// The answer's body is not text.
    NotText { address: String },
    /// The host answered with a refusal; `body` is its own explanation.
    Status {
        address: String,
        status: u16,
        body: String,
    },
}

impl fmt::Display for NetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoWindow => formatter.write_str("the page has no window"),
            Self::Request { what, cause } => write!(formatter, "could not {what}: {cause}"),
            Self::Unreachable { address, cause } => {
                write!(formatter, "could not reach {address}: {cause}")
            }
            Self::NotAResponse { address } => {
                write!(formatter, "{address} did not answer with a response")
            }
            Self::Unreadable { address, cause } => {
                write!(formatter, "{address} could not be read: {cause}")
            }
            Self::NotText { address } => write!(formatter, "{address} is not text"),
            Self::Status {
                address,
                status,
                body,
            } => write!(formatter, "{address} answered with status {status}: {body}"),
        }
    }
}

impl std::error::Error for NetError {}

fn request_error(what: &'static str) -> impl Fn(JsValue) -> NetError {
    move |cause| NetError::Request {
        what,
        cause: cause.into(),
    }
}

async fn read_body(address: &str, response: Response) -> Result<String, NetError> {
    let status = response.status();
    let ok = response.ok();
    let unreadable = |cause: JsValue| NetError::Unreadable {
        address: address.to_owned(),
        cause: cause.into(),
    };
    let text = response.text().map_err(unreadable)?;
    let body = JsFuture::from(text)
        .await
        .map_err(unreadable)?
        .as_string()
        .ok_or_else(|| NetError::NotText {
            address: address.to_owned(),
        })?;
    if ok {
        Ok(body)
    } else {
        Err(NetError::Status {
            address: address.to_owned(),
            status,
            body,
        })
    }
}

async fn response_of(address: &str, request: js_sys::Promise) -> Result<Response, NetError> {
    JsFuture::from(request)
        .await
        .map_err(|cause| NetError::Unreachable {
            address: address.to_owned(),
            cause: cause.into(),
        })?
        .dyn_into::<Response>()
        .map_err(|_| NetError::NotAResponse {
            address: address.to_owned(),
        })
}

/// Reads `address` as text. A non-success status is an error that names the address and status.
pub async fn fetch_text(address: &str) -> Result<String, NetError> {
    let window = web_sys::window().ok_or(NetError::NoWindow)?;
    let response = response_of(address, window.fetch_with_str(address)).await?;
    read_body(address, response).await
}

/// Posts `body` (JSON) to `address` and reads the answer as text. A refusal is an error that
/// carries the host's own explanation.
pub async fn post_json(address: &str, body: &str) -> Result<String, NetError> {
    let window = web_sys::window().ok_or(NetError::NoWindow)?;
    let headers = Headers::new().map_err(request_error("make the request headers"))?;
    headers
        .set("Content-Type", "application/json")
        .map_err(request_error("set the content type"))?;
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_headers(&headers);
    init.set_body(&JsValue::from_str(body));
    let response = response_of(address, window.fetch_with_str_and_init(address, &init)).await?;
    read_body(address, response).await
}
