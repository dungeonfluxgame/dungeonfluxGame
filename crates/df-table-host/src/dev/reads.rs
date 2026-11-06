//! The reads of the host itself that `dev` runs: events and the event level, the host status, the
//! frame, a media file's headers, the open pages and one page's timeline, and the services.
//!
//! Each reads its route's answer as the route's typed reply, so a refusal reaches the caller as
//! `DevError::Rejected` carrying the route's error, never as text to match.

use axum::http::StatusCode;
use df_playground::timeline::ClientTimeline;

use super::{DevError, ReplyError};
use crate::client::Response;
use crate::command::{EventsRead, TimelineRead};
use crate::dev_client::{self, get_json, post_json, read_answer, reply_value, unexpected_answer};
use crate::engine::report::EngineReply;
use crate::engine::report::reach::EventLevelReport;
use crate::engine::request::EventLevelRequest;
use crate::engine::routes::path as engine_path;
use crate::observe::EventLevel;
use crate::playground::{ClientTimelineReply, ClientsError};
use crate::report::{
    MediaListing, format_clients, format_events, format_frame, format_frame_unchanged,
    format_host_status, format_media_head, format_media_listing, format_timeline, frame_count,
};
use crate::server::path as host_path;
use crate::server::request::{HostReply, HostStatus};
use crate::services::{self, ServicesReport, format_services};
use crate::wire::{
    ClientView, EVENTS_AFTER_PARAMETER, EventsReply, FRAME_AFTER_PARAMETER, FRAME_IDLE_NO_CONTENT,
    FRAME_IDLE_PARAMETER, route,
};

/// The query of an events read: the parameters given, in the order the grammar names them.
fn events_query(read: &EventsRead) -> String {
    let mut parameters = Vec::new();
    if let Some(after) = read.after {
        parameters.push(format!("{EVENTS_AFTER_PARAMETER}={after}"));
    }
    if let Some(area) = &read.area {
        parameters.push(format!("area={}", area.as_str()));
    }
    if let Some(limit) = read.limit {
        parameters.push(format!("limit={limit}"));
    }
    query_text(&parameters)
}

/// `?a=1&b=2`, or nothing when there are no parameters.
fn query_text(parameters: &[String]) -> String {
    if parameters.is_empty() {
        String::new()
    } else {
        format!("?{}", parameters.join("&"))
    }
}

/// The refusal a host route answered with a status that is not a success.
fn host_refusal(response: &Response) -> DevError {
    match read_answer::<HostReply<serde_json::Value>>(response).map(reply_value) {
        Ok(Err(refusal)) => refusal,
        Ok(Ok(_)) => unexpected_answer(response, None),
        Err(unreadable) => unreadable,
    }
}

/// Reads an events answer: the lines, or the host's refusal of the query.
pub(super) async fn show_events(port: u16, read: &EventsRead) -> Result<String, DevError> {
    let path = format!("{}{}", route::EVENTS, events_query(read));
    let response = dev_client::get(port, &path).await?;
    if response.status != StatusCode::OK {
        return Err(host_refusal(&response));
    }
    let reply: EventsReply = read_answer(&response)?;
    Ok(format_events(&reply, read.after))
}

/// Reads the event level of the host's sinks, or sets it.
pub(super) async fn events_level(port: u16, set: Option<EventLevel>) -> Result<String, DevError> {
    let path = engine_path::EVENTS_LEVEL;
    let reply: EngineReply<EventLevelReport> = match set {
        None => get_json(port, path).await?,
        Some(level) => post_json(port, path, &EventLevelRequest { level }).await?,
    };
    let report = reply_value(reply)?;
    let verb = if set.is_some() { "set to" } else { "is" };
    Ok(format!("event level {verb} {}", report.level.key()))
}

/// Reads what the host's log, event sinks and clock are doing.
pub(super) async fn host_status(port: u16) -> Result<String, DevError> {
    let reply: HostReply<HostStatus> = get_json(port, host_path::HOST_STATUS).await?;
    Ok(format_host_status(&reply_value(reply)?))
}

/// Reads the playground's frame, or waits for the one after `after`.
pub(super) async fn frame(port: u16, after: Option<u64>) -> Result<String, DevError> {
    let path = match after {
        Some(after) => format!(
            "{}?{FRAME_AFTER_PARAMETER}={after}&{FRAME_IDLE_PARAMETER}={FRAME_IDLE_NO_CONTENT}",
            route::PLAYGROUND_FRAME
        ),
        None => route::PLAYGROUND_FRAME.to_owned(),
    };
    let response = dev_client::get(port, &path).await?;
    match (response.status, after) {
        (StatusCode::NO_CONTENT, Some(after)) => Ok(format_frame_unchanged(after)),
        (StatusCode::OK, _) => format_frame(&response.body)
            .map_err(|error| unexpected_answer(&response, Some(error.to_string()))),
        _ => Err(host_refusal(&response)),
    }
}

/// The host's frame count, or `None` when it cannot be read: a page's frame is shown against it.
async fn host_frame(port: u16) -> Option<u64> {
    let response = dev_client::get(port, route::PLAYGROUND_FRAME).await.ok()?;
    if response.status != StatusCode::OK {
        return None;
    }
    frame_count(&response.body).ok()
}

/// The media listing with the library's limits.
pub(super) async fn show_media(port: u16) -> Result<String, DevError> {
    let listing: MediaListing = super::get_success(port, route::PLAYGROUND_MEDIA_LIST).await?;
    Ok(format_media_listing(&listing))
}

/// What a HEAD of the media file answers, without fetching the file.
pub(super) async fn media_head(port: u16, name: &str) -> Result<String, DevError> {
    let path = route::PLAYGROUND_MEDIA_FILE.replace("{name}", name);
    let answer = crate::client::head(port, &path)
        .await
        .map_err(DevError::Client)?;
    Ok(format_media_head(name, &answer))
}

/// The files revision the playground page reads, `<assets>.<package>`.
pub(super) async fn files_revision(port: u16) -> Result<String, DevError> {
    let response = dev_client::get(port, route::PLAYGROUND_REVISION).await?;
    if response.status != StatusCode::OK {
        return Err(unexpected_answer(&response, None));
    }
    Ok(response.body.trim().to_owned())
}

/// The open pages as text, each against the host's frame.
pub(super) async fn clients_text(port: u16, clients: &[ClientView]) -> String {
    format_clients(clients, host_frame(port).await)
}

/// Every open page.
pub(super) async fn show_clients(port: u16) -> Result<String, DevError> {
    let clients: Vec<ClientView> = super::get_success(port, route::PLAYGROUND_CLIENTS).await?;
    Ok(clients_text(port, &clients).await)
}

/// One open page, or the typed refusal that no such page is open.
pub(super) async fn show_client(port: u16, client: &str) -> Result<String, DevError> {
    let clients: Vec<ClientView> = super::get_success(port, route::PLAYGROUND_CLIENTS).await?;
    let found = clients
        .into_iter()
        .find(|view| view.client == client)
        .ok_or_else(|| {
            let error = ClientsError::NoSuchClient {
                client: client.to_owned(),
            };
            DevError::Rejected {
                message: error.to_string(),
                error: ReplyError::Clients(error),
            }
        })?;
    Ok(clients_text(port, &[found]).await)
}

/// One page's timeline.
pub(super) async fn client_timeline(port: u16, read: &TimelineRead) -> Result<String, DevError> {
    let mut parameters = Vec::new();
    if let Some(kind) = read.kind {
        parameters.push(format!("kind={}", kind.key()));
    }
    if let Some(after) = read.after {
        parameters.push(format!("after={after}"));
    }
    let address = df_playground::path::client_timeline(&read.client);
    let path = format!("{address}{}", query_text(&parameters));
    let reply: ClientTimelineReply = get_json(port, &path).await?;
    let timeline: ClientTimeline = reply_value(reply)?;
    Ok(format_timeline(&timeline))
}

/// How the media and text services are set up and stand.
pub(super) async fn services(port: u16) -> Result<String, DevError> {
    let reply = services::fetch(port).await?;
    let report: ServicesReport = reply_value(reply)?;
    Ok(format_services(&report))
}
