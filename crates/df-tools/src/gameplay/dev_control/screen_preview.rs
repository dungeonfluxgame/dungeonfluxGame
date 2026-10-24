//! One-use, short-lived links to a real development seat. No game state is forged.
use super::{Outcome, Shared, authorized, bad_request, failed};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

const MAXIMUM_TICKETS: usize = 16;
const TICKET_LIFETIME: Duration = Duration::from_secs(60);

struct Ticket {
    saved_player: String,
    run_identity: String,
    expires: Instant,
}

impl Ticket {
    fn belongs_to_run(&self, current: &str) -> bool {
        self.run_identity == current
    }
}

#[derive(Default)]
pub(super) struct Tickets(HashMap<String, Ticket>);

impl Tickets {
    fn insert(&mut self, token: String, ticket: Ticket, now: Instant) -> Result<(), ()> {
        self.0.retain(|_, ticket| ticket.expires > now);
        if self.0.len() >= MAXIMUM_TICKETS || self.0.contains_key(&token) {
            return Err(());
        }
        self.0.insert(token, ticket);
        Ok(())
    }

    fn take(&mut self, token: &str, now: Instant) -> Option<Ticket> {
        self.0.remove(token).filter(|ticket| ticket.expires > now)
    }
}

pub(super) async fn issue(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let seat = body["seat"]
        .as_str()
        .ok_or_else(|| bad_request("seat must be 1 to 6"))?;
    let index = super::seat_index(seat).map_err(bad_request)?;
    let seats = state.seats.lock().await;
    let credential = super::seat_credential(&seats, index)?;
    let (session, run) = super::scope_of(&seats)?;
    let member = seats.seats[index]
        .member
        .as_deref()
        .filter(|member| member.len() == 16)
        .ok_or_else(|| bad_request("join this development seat first"))?;
    if session.len() != 16 || run.len() != 16 {
        return Err(failed("development seat scope invalid"));
    }
    let run_identity =
        super::super::run_restart::identity().map_err(|_| failed("run identity unavailable"))?;
    let saved_player = format!(
        "v2.{}.{}.{}.{}.{}",
        super::super::hex(&credential),
        super::super::hex(&session),
        super::super::hex(&run),
        super::super::hex(member),
        run_identity
    );
    drop(seats);
    let token = super::config::fresh_token().map_err(|_| failed("preview ticket unavailable"))?;
    let now = Instant::now();
    state
        .screen_tickets
        .lock()
        .await
        .insert(
            token.clone(),
            Ticket {
                saved_player,
                run_identity,
                expires: now + TICKET_LIFETIME,
            },
            now,
        )
        .map_err(|_| {
            bad_request("too many pending preview links; consume one or wait 60 seconds")
        })?;
    Ok((
        StatusCode::OK,
        Json(
            json!({"status":"issued", "path":format!("/gameplay/dev/screens/{token}"), "expires_seconds":60}),
        ),
    ))
}

fn private_response(status: StatusCode, body: Html<String>) -> Response {
    (
        status,
        [
            ("Cache-Control", "no-store"),
            ("Referrer-Policy", "no-referrer"),
        ],
        body,
    )
        .into_response()
}

pub(super) async fn consume(
    State(state): State<Arc<Shared>>,
    Path(token): Path<String>,
) -> Response {
    if token.len() != 64
        || !token
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return private_response(
            StatusCode::NOT_FOUND,
            Html("Preview link unavailable.".to_owned()),
        );
    }
    let ticket = state
        .screen_tickets
        .lock()
        .await
        .take(&token, Instant::now());
    let Some(ticket) = ticket else {
        return private_response(
            StatusCode::GONE,
            Html("Preview link expired or already used. Request another link.".to_owned()),
        );
    };
    let current_run = super::super::run_restart::identity();
    if !current_run
        .as_ref()
        .is_ok_and(|current| ticket.belongs_to_run(current))
    {
        return private_response(
            StatusCode::GONE,
            Html("The preview run changed. Request another link.".to_owned()),
        );
    }
    let Html(body) = super::super::room::player_page(
        state.service.actor.clone(),
        ticket.run_identity.clone().into(),
    )
    .await;
    let body = body.replace(
        "id=\"player\"",
        &format!("id=\"player\" data-dev-player=\"{}\"", ticket.saved_player),
    );
    private_response(StatusCode::OK, Html(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ticket(now: Instant) -> Ticket {
        Ticket {
            saved_player: "scope".to_owned(),
            run_identity: "run".to_owned(),
            expires: now + TICKET_LIFETIME,
        }
    }
    #[test]
    fn screen_ticket_rejects_another_run_and_marks_pages_private() {
        let ticket = ticket(Instant::now());
        assert!(ticket.belongs_to_run("run"));
        assert!(!ticket.belongs_to_run("another-run"));
        let response = private_response(StatusCode::GONE, Html("expired".to_owned()));
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["referrer-policy"], "no-referrer");
    }

    #[test]
    fn screen_tickets_are_one_use_expiring_and_bounded() {
        let now = Instant::now();
        let mut tickets = Tickets::default();
        tickets.insert("one".to_owned(), ticket(now), now).unwrap();
        assert!(tickets.take("one", now).is_some());
        assert!(tickets.take("one", now).is_none());
        tickets
            .insert("expired".to_owned(), ticket(now), now)
            .unwrap();
        assert!(tickets.take("expired", now + TICKET_LIFETIME).is_none());
        for index in 0..MAXIMUM_TICKETS {
            tickets.insert(index.to_string(), ticket(now), now).unwrap();
        }
        assert!(
            tickets
                .insert("overflow".to_owned(), ticket(now), now)
                .is_err()
        );
        assert!(
            tickets
                .insert(
                    "after-expiry".to_owned(),
                    ticket(now + TICKET_LIFETIME),
                    now + TICKET_LIFETIME
                )
                .is_ok()
        );
    }
}
