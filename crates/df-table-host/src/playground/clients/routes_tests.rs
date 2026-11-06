//! The client routes against a running host: what a report writes, how a refusal is recorded, and
//! the timeline read.

use axum::http::StatusCode;
use df_playground::media::{ClientReport, ClientReportPost};
use df_playground::path::{CLIENT_REPORT, client_timeline};
use df_playground::timeline::{EndReason, TimelineKind, TimelineRecord};

use super::ClientTimelineReply;
use super::ClientsError;
use crate::client::{get, post_json};
use crate::dev_http::DevReply;
use crate::test_support::{Running, STARTER_FILES, post_request, raw_exchange, start};

fn lines(running: &Running) -> Vec<String> {
    running
        .state
        .log()
        .newest(10_000)
        .into_iter()
        .map(|line| line.text)
        .collect()
}

fn lines_starting(running: &Running, prefix: &str) -> Vec<String> {
    lines(running)
        .into_iter()
        .filter(|line| line.starts_with(prefix))
        .collect()
}

async fn send(running: &Running, client: &str, report: ClientReport) -> StatusCode {
    let body = serde_json::to_string(&ClientReportPost {
        client: client.to_owned(),
        report,
    })
    .expect("writes");
    post_json(running.port, CLIENT_REPORT, &body)
        .await
        .expect("answers")
        .status
}

#[tokio::test]
async fn a_report_with_a_bad_name_is_refused_and_recorded_with_the_header_client() {
    let running = start(&STARTER_FILES).await;
    let body = serde_json::to_string(&ClientReportPost {
        client: "has space".to_owned(),
        report: ClientReport::default(),
    })
    .expect("writes");
    let answer = raw_exchange(
        running.port,
        &post_request(CLIENT_REPORT, &body, "X-Df-Client: page-1a2b3c4d\r\n"),
    )
    .await;
    assert!(answer.starts_with("HTTP/1.1 400"), "{answer}");
    assert_eq!(
        lines_starting(&running, "CLIENT report_refused"),
        ["CLIENT report_refused client=page-1a2b3c4d reason=name"]
    );
}

#[tokio::test]
async fn a_report_that_is_not_json_is_refused_as_a_body() {
    let running = start(&STARTER_FILES).await;
    // Sent without the `X-Df-Client` header, so the line cannot say who asked.
    let answer = raw_exchange(running.port, &post_request(CLIENT_REPORT, "nope", "")).await;
    assert!(answer.starts_with("HTTP/1.1 400"), "{answer}");
    assert_eq!(
        lines_starting(&running, "CLIENT report_refused"),
        ["CLIENT report_refused client=- reason=body"]
    );
}

#[tokio::test]
async fn a_page_on_older_files_than_the_host_is_said_to_be_stale_once() {
    let running = start(&STARTER_FILES).await;
    let old = ClientReport {
        asset_revision: "0000000000000000.0000000000000000".to_owned(),
        ..ClientReport::default()
    };
    assert_eq!(send(&running, "page-1", old.clone()).await, StatusCode::OK);
    assert_eq!(send(&running, "page-1", old).await, StatusCode::OK);
    let stale = lines_starting(&running, "CLIENT stale");
    assert_eq!(stale.len(), 1, "{stale:?}");
    assert!(
        stale[0]
            .starts_with("CLIENT stale client=page-1 has=0000000000000000.0000000000000000 host=")
    );
    assert_eq!(lines_starting(&running, "CLIENT seen").len(), 1);
}

#[tokio::test]
async fn a_pages_timeline_is_read_by_kind_and_a_bad_read_is_refused_and_recorded() {
    let running = start(&STARTER_FILES).await;
    let plate = TimelineRecord {
        seq: 1,
        kind: TimelineKind::Plate,
        id: "tavern.png".to_owned(),
        start_page_ms: 0,
        end_page_ms: Some(4_000),
        end_reason: Some(EndReason::Removed),
        planned_ms: None,
        detail: None,
    };
    let sound = TimelineRecord {
        seq: 2,
        kind: TimelineKind::Sound,
        id: "bell.wav".to_owned(),
        end_page_ms: None,
        end_reason: None,
        ..plate.clone()
    };
    let report = ClientReport {
        timeline: vec![plate, sound],
        ..ClientReport::default()
    };
    assert_eq!(send(&running, "page-1", report).await, StatusCode::OK);
    assert_eq!(lines_starting(&running, "TIMING plate").len(), 1);

    let answer = get(
        running.port,
        &format!("{}?kind=plate", client_timeline("page-1")),
    )
    .await
    .expect("answers");
    assert_eq!(answer.status, StatusCode::OK);
    let DevReply::Done { value } =
        serde_json::from_str::<ClientTimelineReply>(&answer.body).expect("a reply")
    else {
        panic!("refused: {}", answer.body);
    };
    assert_eq!(value.client, "page-1");
    assert_eq!(value.entries.len(), 1);
    assert_eq!(value.entries[0].shown_ms, Some(4_000));

    let answer = get(running.port, &client_timeline("page-9"))
        .await
        .expect("answers");
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
    let reply: ClientTimelineReply = serde_json::from_str(&answer.body).expect("a reply");
    assert!(matches!(
        reply,
        DevReply::Rejected {
            error: ClientsError::NoSuchClient { .. },
            ..
        }
    ));
    let answer = get(
        running.port,
        &format!("{}?kind=nope", client_timeline("page-1")),
    )
    .await
    .expect("answers");
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        lines_starting(&running, "CLIENT timeline_refused"),
        [
            "CLIENT timeline_refused client=page-9 error=no_such_client",
            "CLIENT timeline_refused client=page-1 error=invalid_query",
        ]
    );
}

#[tokio::test]
async fn a_report_wakes_no_page() {
    let running = start(&STARTER_FILES).await;
    let before = running.state.change_count();
    send(&running, "page-1", ClientReport::default()).await;
    assert_eq!(running.state.change_count(), before);
}
