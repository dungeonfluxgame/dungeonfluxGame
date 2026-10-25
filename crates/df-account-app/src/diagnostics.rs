//! The only text the app ever writes to the browser console.
//!
//! Lines are `AREA verb key=value` with ids, counts and fixed names, never URLs, never form
//! values. An invitation or reset token reaches the app in the address bar, so every free-text
//! field (a panic message, a failure detail) passes `scrub` first: a line never contains `token=`.

use crate::api::Method;

/// The marker `scrub` removes. Tokens reach the page as `#token=...` (or `?token=...` from older
/// links); anything that quotes that text must not be logged.
const TOKEN_MARKERS: [&str; 2] = ["token=", "token%3d"];
const REMOVED: &str = "[removed]";
/// Longest panic message kept in a line.
const MAXIMUM_DETAIL_CHARS: usize = 200;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Event<'a> {
    Booted {
        route: &'static str,
    },
    Panicked {
        message: &'a str,
    },
    RequestFailed {
        method: Method,
        status: u16,
    },
    RequestTimedOut {
        method: Method,
    },
    /// The browser refused something the app asked of it (clipboard, microphone, navigation).
    BrowserRefused {
        what: &'static str,
    },
}

/// The console line for an event.
pub fn line(event: Event<'_>) -> String {
    match event {
        Event::Booted { route } => format!("ACCOUNT_APP booted route={route}"),
        Event::Panicked { message } => {
            format!("ACCOUNT_APP panicked detail={:?}", scrub(message))
        }
        Event::RequestFailed { method, status } => {
            format!(
                "ACCOUNT_APP request_failed method={} status={status}",
                method.as_str()
            )
        }
        Event::RequestTimedOut { method } => {
            format!("ACCOUNT_APP request_timed_out method={}", method.as_str())
        }
        Event::BrowserRefused { what } => format!("ACCOUNT_APP browser_refused what={what}"),
    }
}

/// Removes every `token=<value>` (any letter case, also percent-encoded) and bounds the length.
pub fn scrub(text: &str) -> String {
    let scrubbed = TOKEN_MARKERS
        .iter()
        .fold(text.to_owned(), |current, marker| {
            remove_values(&current, marker)
        });
    scrubbed.chars().take(MAXIMUM_DETAIL_CHARS).collect()
}

fn remove_values(text: &str, marker: &str) -> String {
    let lowered = text.to_ascii_lowercase();
    let mut kept = String::with_capacity(text.len().min(MAXIMUM_DETAIL_CHARS * 4));
    let mut cursor = 0;
    while let Some(found) = lowered[cursor..].find(marker) {
        let start = cursor + found;
        kept.push_str(&text[cursor..start]);
        kept.push_str(REMOVED);
        let value_start = start + marker.len();
        cursor = lowered[value_start..]
            .find(|character: char| {
                character.is_whitespace() || matches!(character, '&' | '#' | '"' | '\'' | ')')
            })
            .map_or(lowered.len(), |end| value_start + end);
    }
    kept.push_str(&text[cursor..]);
    kept
}

#[cfg(test)]
mod diagnostics_tests;
