//! Fetching text from, and posting JSON to, the page's own server.
//!
//! It owns one request at a time and reports a failure as plain words naming the address, so the
//! page can show it. It never retries; the caller's poll loop does that.

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Headers, RequestInit, Response};

async fn read_body(address: &str, response: Response) -> Result<String, String> {
    let status = response.status();
    let ok = response.ok();
    let text = response
        .text()
        .map_err(|_| format!("{address} has no readable body"))?;
    let body = JsFuture::from(text)
        .await
        .map_err(|_| format!("{address} could not be read"))?
        .as_string()
        .ok_or_else(|| format!("{address} is not text"))?;
    if ok {
        Ok(body)
    } else {
        Err(format!("{address} answered with status {status}: {body}"))
    }
}

/// Reads `address` as text. A non-success status is an error that names the address and status.
pub async fn fetch_text(address: &str) -> Result<String, String> {
    let window = web_sys::window().ok_or_else(|| "the page has no window".to_owned())?;
    let response = JsFuture::from(window.fetch_with_str(address))
        .await
        .map_err(|_| format!("could not reach {address}"))?
        .dyn_into::<Response>()
        .map_err(|_| format!("{address} did not answer with a response"))?;
    read_body(address, response).await
}

/// Posts `body` (JSON) to `address` and reads the answer as text. A refusal is an error that
/// carries the host's own explanation.
pub async fn post_json(address: &str, body: &str) -> Result<String, String> {
    let window = web_sys::window().ok_or_else(|| "the page has no window".to_owned())?;
    let headers = Headers::new().map_err(|_| "could not make the request headers".to_owned())?;
    headers
        .set("Content-Type", "application/json")
        .map_err(|_| "could not set the content type".to_owned())?;
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_headers(&headers);
    init.set_body(&wasm_bindgen::JsValue::from_str(body));
    let response = JsFuture::from(window.fetch_with_str_and_init(address, &init))
        .await
        .map_err(|_| format!("could not reach {address}"))?
        .dyn_into::<Response>()
        .map_err(|_| format!("{address} did not answer with a response"))?;
    read_body(address, response).await
}
