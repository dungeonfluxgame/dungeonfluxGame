//! The display cookies and the request checks that stand in for a CSRF token on redeem and renew.

use http::{HeaderMap, header};

pub const DEVICE_COOKIE: &str = "__Host-df_display_device";
pub const TICKET_COOKIE: &str = "__Host-df_display_ticket";
/// The custom header that forces a CORS preflight no server answers.
pub const DEVICE_HEADER: &str = "x-df-device";

/// `Secure; HttpOnly; SameSite=Strict; Path=/` and a `Max-Age` in seconds.
pub fn set_cookie(name: &str, value: &str, max_age_seconds: u64) -> String {
    format!("{name}={value}; Max-Age={max_age_seconds}; Path=/; Secure; HttpOnly; SameSite=Strict")
}

pub fn clear_cookie(name: &str) -> String {
    format!("{name}=; Max-Age=0; Path=/; Secure; HttpOnly; SameSite=Strict")
}

/// The value of a cookie by name, across every `Cookie` header.
pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|line| line.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_owned())
}

/// Why a display request was refused before it reached the service.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayRequestRefusal {
    MissingOrForeignOrigin,
    CrossSite,
    MissingDeviceHeader,
    NotJson,
    MissingDeviceCookie,
}

/// The checks that replace a CSRF token on `display/redeem` and `display/renew` (8.5): the
/// `Strict` device cookie, an `Origin` equal to the configured origin, a `Sec-Fetch-Site` that,
/// when present, is `same-origin` (absent is accepted only because the Origin, the custom header
/// and the Strict cookie are all present), a JSON content type and the custom `X-DF-Device`
/// header.
pub fn check_display_request(
    headers: &HeaderMap,
    origin: &str,
) -> Result<(), DisplayRequestRefusal> {
    match headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        Some(value) if value == origin => {}
        _ => return Err(DisplayRequestRefusal::MissingOrForeignOrigin),
    }
    if let Some(site) = headers.get("sec-fetch-site")
        && site.to_str().ok() != Some("same-origin")
    {
        return Err(DisplayRequestRefusal::CrossSite);
    }
    if headers
        .get(DEVICE_HEADER)
        .is_none_or(|value| value.is_empty())
    {
        return Err(DisplayRequestRefusal::MissingDeviceHeader);
    }
    let json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|kind| kind.trim() == "application/json")
        });
    if !json {
        return Err(DisplayRequestRefusal::NotJson);
    }
    if cookie_value(headers, DEVICE_COOKIE).is_none() {
        return Err(DisplayRequestRefusal::MissingDeviceCookie);
    }
    Ok(())
}
