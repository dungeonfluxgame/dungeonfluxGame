//! Asking the page's own server: one typed call (`call`) for every route that answers with the
//! host's reply envelope (`dev_reply::HostReply`), and plain text (`text`, `fetch_text`,
//! `post_json`) for the files, the controls and the frame.
//!
//! It owns one request at a time and reports a failure as a typed error that names the address and
//! carries the exception the browser threw, so the page can show it and a bug is not mistaken for
//! a refusal. Every request names the page (`path::CLIENT_HEADER`), carries the phone's seat key
//! only when asked (`path::SEAT_KEY_HEADER`), and is given up after its timeout
//! (`limits::REQUEST_TIMEOUT_MS` unless the call says otherwise) as `NetError::TimedOut`, so a
//! connection that died under a phone does not hold its poll loop. It never retries; the caller's
//! poll loop does that.

use std::cell::RefCell;
use std::fmt;

use gloo_timers::callback::Timeout;
use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{AbortController, Headers, RequestInit, Response};

use crate::dev_reply::{ErrorTag, HostReply, Rejection};
use crate::dom::BrowserError;
use crate::limits::{LONGEST_BROWSER_TIMER_MS, REQUEST_TIMEOUT_MS};
use crate::path::{CLIENT_HEADER, MEDIA_LINKS_HEADER, SEAT_KEY_HEADER};

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
    /// No whole answer came within `after_ms`, so the request was given up.
    TimedOut { address: String, after_ms: u64 },
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
            Self::TimedOut { address, after_ms } => {
                write!(formatter, "{address} did not answer within {after_ms} ms")
            }
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
    // The signed links to the media the answer names (the player listener's), kept for the page.
    if let Ok(Some(links)) = response.headers().get(MEDIA_LINKS_HEADER) {
        crate::path::remember_media_links(&links);
    }
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

thread_local! {
    /// The page's name for itself, sent with every request so the host can say who asked.
    static CLIENT: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Names the page in every request from now on.
pub fn set_client(name: &str) {
    CLIENT.with(|client| *client.borrow_mut() = name.to_owned());
}

/// The headers every request carries: the page's name, when it has one.
fn request_headers() -> Result<Headers, NetError> {
    let headers = Headers::new().map_err(request_error("make the request headers"))?;
    let name = CLIENT.with(|client| client.borrow().clone());
    if !name.is_empty() {
        headers
            .set(CLIENT_HEADER, &name)
            .map_err(request_error("name the page in the request"))?;
    }
    Ok(headers)
}

/// What a request sends.
enum Body<'a> {
    /// Nothing: a `GET`.
    Empty,
    /// JSON, or why the value could not be written as JSON.
    Json(Result<String, String>),
    /// Bytes of `content_type`, zeroed in the browser's copy once the answer is in: a recording
    /// lives only as long as its request.
    Bytes {
        content_type: &'a str,
        bytes: &'a [u8],
    },
}

/// One request to the host: its address, what it sends, the phone's seat key when it is a seat's
/// and how long it may take.
pub struct Call<'a> {
    address: &'a str,
    body: Body<'a>,
    seat_key: Option<&'a str>,
    timeout_ms: u64,
}

impl<'a> Call<'a> {
    /// A `GET` of `address`.
    pub fn get(address: &'a str) -> Self {
        Self::of(address, Body::Empty)
    }

    /// A `POST` of `body` as JSON to `address`.
    pub fn post(address: &'a str, body: &impl Serialize) -> Self {
        let json = serde_json::to_string(body).map_err(|cause| cause.to_string());
        Self::of(address, Body::Json(json))
    }

    /// A `POST` of JSON text already written to `address`.
    pub fn post_text(address: &'a str, json: &str) -> Self {
        Self::of(address, Body::Json(Ok(json.to_owned())))
    }

    /// A `POST` of `bytes` as `content_type` to `address`.
    pub fn post_bytes(address: &'a str, content_type: &'a str, bytes: &'a [u8]) -> Self {
        Self::of(
            address,
            Body::Bytes {
                content_type,
                bytes,
            },
        )
    }

    fn of(address: &'a str, body: Body<'a>) -> Self {
        Self {
            address,
            body,
            seat_key: None,
            timeout_ms: REQUEST_TIMEOUT_MS,
        }
    }

    /// The address it asks.
    pub fn address(&self) -> &'a str {
        self.address
    }

    /// Sends the phone's seat key with it (in its header only, never in an address).
    pub fn with_seat_key(mut self, seat_key: &'a str) -> Self {
        self.seat_key = Some(seat_key);
        self
    }

    /// Gives the request `timeout_ms` instead of `REQUEST_TIMEOUT_MS`.
    pub fn within(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }
}

/// Why a call gave no value.
#[derive(Debug, Clone, PartialEq)]
pub enum CallError<K> {
    /// The host refused: the kind of its typed error, when one of `K`, and its own sentence.
    Refused(Rejection<K>),
    /// The host could not be asked, did not answer in time, or answered with something else than
    /// its reply (a guard's refusal, a proxy's page).
    Net(NetError),
    /// The host's success is not the reply the page expects.
    Unreadable { address: String, cause: String },
    /// The request's body could not be written.
    Unwritable { address: String, cause: String },
}

impl<K> fmt::Display for CallError<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(rejection) => write!(formatter, "refused: {}", rejection.message),
            Self::Net(error) => error.fmt(formatter),
            Self::Unreadable { address, cause } => {
                write!(formatter, "{address} did not answer as expected: {cause}")
            }
            Self::Unwritable { address, cause } => {
                write!(
                    formatter,
                    "the request to {address} could not be written: {cause}"
                )
            }
        }
    }
}

/// Asks the host and reads its reply envelope: the value, or the refusal with its kind of `K`. A
/// refusal arrives with an error status, so the body of one is read as a reply too; an error
/// status whose body is not a reply keeps its network error.
pub async fn call<T: DeserializeOwned, K: ErrorTag>(call: Call<'_>) -> Result<T, CallError<K>> {
    let address = call.address;
    if let Body::Json(Err(cause)) = &call.body {
        return Err(CallError::Unwritable {
            address: address.to_owned(),
            cause: cause.clone(),
        });
    }
    let (body, status_error) = match text(call).await {
        Ok(body) => (body, None),
        Err(NetError::Status {
            address,
            status,
            body,
        }) => (
            body.clone(),
            Some(NetError::Status {
                address,
                status,
                body,
            }),
        ),
        Err(error) => return Err(CallError::Net(error)),
    };
    match serde_json::from_str::<HostReply<T, K>>(&body) {
        Ok(reply) => reply.into_result().map_err(CallError::Refused),
        Err(cause) => Err(match status_error {
            Some(error) => CallError::Net(error),
            None => CallError::Unreadable {
                address: address.to_owned(),
                cause: cause.to_string(),
            },
        }),
    }
}

/// Asks the host and reads its answer as text. A non-success status is an error that names the
/// address and status and carries the host's own explanation.
pub async fn text(call: Call<'_>) -> Result<String, NetError> {
    let Call {
        address,
        body,
        seat_key,
        timeout_ms,
    } = call;
    let window = web_sys::window().ok_or(NetError::NoWindow)?;
    let headers = request_headers()?;
    if let Some(key) = seat_key {
        headers
            .set(SEAT_KEY_HEADER, key)
            .map_err(request_error("put the seat key in the request"))?;
    }
    let init = RequestInit::new();
    let mut sent_bytes = None;
    match body {
        Body::Empty => {}
        Body::Json(json) => {
            let json = json.map_err(|cause| NetError::Request {
                what: "write the request body",
                cause: JsValue::from_str(&cause).into(),
            })?;
            headers
                .set("Content-Type", "application/json")
                .map_err(request_error("set the content type"))?;
            init.set_method("POST");
            init.set_body(&JsValue::from_str(&json));
        }
        Body::Bytes {
            content_type,
            bytes,
        } => {
            headers
                .set("Content-Type", content_type)
                .map_err(request_error("set the content type"))?;
            let array = js_sys::Uint8Array::from(bytes);
            init.set_method("POST");
            init.set_body(&array);
            sent_bytes = Some(array);
        }
    }
    init.set_headers(&headers);
    // The request is given up when its timer ends; the timer is cancelled when it is dropped with
    // the answer.
    let controller = AbortController::new().map_err(request_error("make the request's timer"))?;
    init.set_signal(Some(&controller.signal()));
    let abort = controller.clone();
    let held_ms = timeout_ms.min(LONGEST_BROWSER_TIMER_MS);
    let _timer = Timeout::new(u32::try_from(held_ms).unwrap_or(u32::MAX), move || {
        abort.abort();
    });
    let answer = response_of(address, window.fetch_with_str_and_init(address, &init)).await;
    if let Some(array) = &sent_bytes {
        array.fill(0, 0, array.length());
    }
    let read = match answer {
        Ok(response) => read_body(address, response).await,
        Err(error) => Err(error),
    };
    match read {
        Err(error)
            if controller.signal().aborted() && !matches!(error, NetError::Status { .. }) =>
        {
            Err(NetError::TimedOut {
                address: address.to_owned(),
                after_ms: timeout_ms,
            })
        }
        read => read,
    }
}

/// Reads `address` as text.
pub async fn fetch_text(address: &str) -> Result<String, NetError> {
    text(Call::get(address)).await
}

/// Posts `body` (JSON already written) to `address` and reads the answer as text.
pub async fn post_json(address: &str, body: &str) -> Result<String, NetError> {
    text(Call::post_text(address, body)).await
}
