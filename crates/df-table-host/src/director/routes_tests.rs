//! The Director's routes against a host's playground, with no sleeping: the apply gate, a move
//! that fails to apply, a camera moved by something else, the host tick's own view of the stage,
//! and a hold that runs out on a paused clock.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use df_table::director::Request;
use df_table::director_reason::Reason;
use df_table::element::ElementId;
use df_table::inspect::InspectAudience;

use super::host_tests::{CAMERA_DEMO, camera_of, lines_starting, said, with_three_avatars};
use super::request::DirectorAct;
use super::routes::{DirectorRoutes, HOST_TICK_MS};
use super::workbench::Workbench;
use crate::test_support::{Running, STARTER_FILES, start};

/// The camera demo fixture with these elements on its stage, written inline.
fn inline_fixture(ids: &[&str]) -> String {
    let elements: Vec<String> = ids
        .iter()
        .map(|id| {
            format!(
                r##"{{ "id": "{id}", "kind": "character", "label": "{id}", "x": 50, "y": 40, "color": "#d9a441", "shape": "circle", "width_units": 5, "height_units": 5 }}"##
            )
        })
        .collect();
    CAMERA_DEMO.replace(
        r#""elements": "live""#,
        &format!(r#""elements": [{}]"#, elements.join(",")),
    )
}

async fn with_inline_stage(ids: &[&str]) -> Running {
    let running = start(&STARTER_FILES).await;
    running.write_asset("fixtures/camera_demo.json", &inline_fixture(ids));
    running
}

/// Routes of their own over a workbench whose clock only `director advance` moves.
fn manual_routes(running: &Running) -> Arc<DirectorRoutes> {
    Arc::new(DirectorRoutes::new(
        Arc::clone(&running.state),
        Workbench::manual(InspectAudience::Operator),
    ))
}

fn focus(element: &str, reason: &str, hold: Option<u64>) -> DirectorAct {
    DirectorAct::Focus {
        element: element.to_owned(),
        reason: Some(reason.to_owned()),
        zoom: None,
        over: None,
        hold,
    }
}

fn camera_target(running: &Running) -> Option<String> {
    running.state.playground().camera_target()
}

fn camera_serial(running: &Running) -> Option<u64> {
    running
        .state
        .playground()
        .snapshot()
        .scene
        .camera
        .map(|camera| camera.serial)
}

fn logged(running: &Running) -> Vec<String> {
    running
        .state
        .log()
        .newest(10_000)
        .into_iter()
        .map(|line| line.text)
        .collect()
}

fn director_serial(routes: &DirectorRoutes) -> u64 {
    routes
        .with(|workbench| Ok(workbench.show().report.serial))
        .expect("a workbench")
}

/// Lets every task that can run, run: a task blocked at the gate stays blocked.
async fn settle() {
    for _ in 0..20_000 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn a_tick_and_a_verb_cannot_apply_camera_moves_out_of_order() {
    let running = with_inline_stage(&["finn", "sigrun"]).await;
    let routes = manual_routes(&running);
    routes
        .act(focus("finn", "speaker", None))
        .await
        .expect("applies");
    routes
        .act(DirectorAct::Advance { ms: 1_000 })
        .await
        .expect("advances");
    // A request waits in the Director's inbox for the tick, which decides the focus on Sigrun and
    // is held just before it applies the move.
    routes
        .with(|workbench| {
            let view = workbench.view();
            let now = workbench.director_mut().now_ms();
            let request =
                Request::focus(Reason::Speaker, ElementId::new("sigrun").expect("a name"));
            workbench
                .director_mut()
                .request(now, &view, request)
                .expect("well formed");
            Ok(())
        })
        .expect("a workbench");
    routes.hook.hold_next.store(true, Ordering::SeqCst);
    let tick = tokio::spawn({
        let routes = Arc::clone(&routes);
        async move { routes.host_tick().await }
    });
    routes.hook.held.notified().await;
    assert_eq!(director_serial(&routes), 2, "the tick decided the focus");
    let verb = tokio::spawn({
        let routes = Arc::clone(&routes);
        async move { routes.act(DirectorAct::Wide { over: None }).await }
    });
    settle().await;
    assert_eq!(
        director_serial(&routes),
        2,
        "the verb waits at the gate and has not decided"
    );
    routes.hook.release.notify_one();
    tick.await.expect("the tick ends");
    verb.await.expect("the verb ends").expect("applies");
    assert_eq!(director_serial(&routes), 3);
    assert_eq!(camera_target(&running), None, "the wide came last");
    assert_eq!(camera_serial(&running), Some(3));
    let lines = logged(&running);
    let position = |text: &str| lines.iter().position(|line| line.starts_with(text));
    assert!(
        position("CAMERA focus element=sigrun") < position("CAMERA wide reason=operator"),
        "{lines:?}"
    );
}

#[tokio::test]
async fn a_pull_back_that_failed_to_apply_is_made_again_and_the_camera_ends_wide() {
    let running = with_inline_stage(&["finn", "sigrun"]).await;
    let routes = manual_routes(&running);
    routes
        .act(focus("finn", "reveal", Some(1_000)))
        .await
        .expect("applies");
    assert_eq!(camera_target(&running).as_deref(), Some("finn"));
    routes.hook.fail_next.store(1, Ordering::SeqCst);
    let reply = routes
        .act(DirectorAct::Advance { ms: 1_000 })
        .await
        .expect("the verb is accepted");
    assert_eq!(reply.problems.len(), 1, "{reply:?}");
    assert_eq!(
        camera_target(&running).as_deref(),
        Some("finn"),
        "the move did not reach the screen"
    );
    routes.host_tick().await;
    assert_eq!(camera_target(&running), None);
    let lines = logged(&running);
    let count = |text: &str| lines.iter().filter(|line| line.contains(text)).count();
    assert_eq!(count("CAMERA apply_failed status=409"), 1, "{lines:?}");
    assert_eq!(count("CAMERA wide reason=timeout serial=2"), 1, "{lines:?}");
    assert_eq!(count("CAMERA wide reason=timeout serial=3"), 1, "{lines:?}");
}

#[tokio::test]
async fn a_focus_whose_element_left_the_fixture_before_it_applied_is_forgotten() {
    let running = with_inline_stage(&["finn", "sigrun"]).await;
    let routes = manual_routes(&running);
    routes.hook.hold_next.store(true, Ordering::SeqCst);
    let first = tokio::spawn({
        let routes = Arc::clone(&routes);
        async move { routes.act(focus("finn", "speaker", None)).await }
    });
    routes.hook.held.notified().await;
    running.write_asset("fixtures/camera_demo.json", &inline_fixture(&["sigrun"]));
    routes.hook.release.notify_one();
    let reply = first.await.expect("ends").expect("the verb is accepted");
    assert!(
        reply
            .problems
            .iter()
            .any(|problem| problem.contains("no element 'finn'")),
        "{reply:?}"
    );
    assert_eq!(camera_target(&running), None);
    let held = routes
        .with(|workbench| Ok(workbench.show().report.active))
        .expect("a workbench");
    assert_eq!(
        held, None,
        "the Director does not hold what was never shown"
    );
    // Finn comes back; the same request is a move, not already_focused.
    running.write_asset(
        "fixtures/camera_demo.json",
        &inline_fixture(&["finn", "sigrun"]),
    );
    routes
        .act(DirectorAct::Advance { ms: 1_000 })
        .await
        .expect("advances");
    let reply = routes
        .act(focus("finn", "speaker", None))
        .await
        .expect("applies");
    assert!(reply.problems.is_empty(), "{reply:?}");
    assert_eq!(camera_target(&running).as_deref(), Some("finn"));
    assert!(
        !logged(&running)
            .iter()
            .any(|line| line.contains("already_focused"))
    );
}

#[tokio::test]
async fn the_host_tick_pulls_back_from_an_element_removed_from_the_fixture() {
    let running = with_inline_stage(&["finn", "sigrun"]).await;
    let routes = manual_routes(&running);
    routes
        .act(focus("finn", "speaker", None))
        .await
        .expect("applies");
    running.write_asset("fixtures/camera_demo.json", &inline_fixture(&["sigrun"]));
    routes.host_tick().await;
    assert_eq!(camera_target(&running), None);
    assert!(
        logged(&running)
            .iter()
            .any(|line| line.starts_with("CAMERA wide reason=target_gone")),
        "{:?}",
        logged(&running)
    );
}

#[tokio::test]
async fn a_scene_camera_reset_is_seen_so_the_next_focus_on_the_same_element_moves() {
    let running = with_three_avatars().await;
    said(&running, "camera focus finn reason=speaker").await;
    said(&running, "scene camera-reset").await;
    assert_eq!(camera_of(&running).await.expect("a camera").target, None);
    said(&running, "director advance 1000").await;
    let answer = said(&running, "camera focus finn reason=speaker").await;
    assert!(!answer.contains("already_focused"), "{answer}");
    assert_eq!(
        lines_starting(&running, "CAMERA focus").await.len(),
        2,
        "a new CAMERA focus line"
    );
    assert_eq!(
        camera_of(&running)
            .await
            .expect("a camera")
            .target
            .as_deref(),
        Some("finn")
    );
    assert_eq!(
        lines_starting(&running, "CAMERA released reason=moved_elsewhere")
            .await
            .len(),
        1
    );
}

#[tokio::test(start_paused = true)]
async fn the_host_tick_returns_the_camera_when_its_hold_ends_with_nobody_asking() {
    let running = with_inline_stage(&["finn"]).await;
    let routes = Arc::new(DirectorRoutes::new(
        Arc::clone(&running.state),
        Workbench::new(InspectAudience::Operator),
    ));
    routes
        .act(focus("finn", "reveal", Some(5_000)))
        .await
        .expect("applies");
    assert_eq!(camera_target(&running).as_deref(), Some("finn"));
    tokio::time::advance(Duration::from_millis(5_000 + HOST_TICK_MS)).await;
    routes.host_tick().await;
    assert_eq!(camera_target(&running), None);
    let wide: Vec<String> = logged(&running)
        .into_iter()
        .filter(|line| line.starts_with("CAMERA wide"))
        .collect();
    assert_eq!(wide, ["CAMERA wide reason=timeout serial=2 over=1200"]);
}
