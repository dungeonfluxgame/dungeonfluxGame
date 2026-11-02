//! Following the host: the frames it pushes, the controls it keeps, the slow look at its files and
//! the loop that runs them.
//!
//! Three paths bring controls to the page (a frame, a poll and the answer to a change the page
//! sent), and they can arrive out of order, so every one goes through `apply_controls`, which
//! takes an answer only when it is from another run of the host or newer than the one the page has.

use gloo_timers::future::TimeoutFuture;
use wasm_bindgen_futures::spawn_local;

use super::assets::{load_selected_fixture, refresh_assets};
use super::page::{render, update_stage};
use super::report::send_report;
use super::sound::sync_audio;
use super::state::{
    CANVAS_ADDRESS, CHANGE_ADDRESS, CONTROLS_ADDRESS, FRAME_ADDRESS, LiveCanvas, LiveFrame,
    PageError, REVISION_ADDRESS, log_error, set_problem, with_app, with_app_mut,
};
use crate::audio;
use crate::backoff::retry_delay_ms;
use crate::continuity::accepts;
use crate::controls::{ControlsChange, ControlsReply};
use crate::dom::{js_message, with_continuity};
use crate::draw_plan::{frame_action, wants_live};
use crate::limits::{HEARTBEAT_POLLS, POLL_MS};
use crate::net::{fetch_text, post_json};

/// What a look at the host's controls found.
#[derive(Debug, Clone, Copy, Default)]
pub struct ControlsUpdate {
    pub changed: bool,
    pub fixture_changed: bool,
    /// The answer is from another run of the host than the one the page was following.
    pub boot_changed: bool,
}

/// Takes the host's controls as the page's own when they are from another run of the host or newer
/// than the ones it has. An older answer, such as a slow one that was overtaken, changes nothing.
pub fn apply_controls(reply: ControlsReply) -> ControlsUpdate {
    with_app_mut(|app| {
        let current = app
            .controls_seen
            .as_ref()
            .map(|(boot_id, revision)| (boot_id.as_str(), *revision));
        if !accepts(current, &reply.boot_id, reply.revision) {
            return ControlsUpdate::default();
        }
        // A new run of the host numbers everything again, so what the page remembers by number
        // (moves, cues, the canvas) is dropped.
        let boot_changed = with_continuity(|continuity| continuity.note_boot(&reply.boot_id));
        if boot_changed {
            audio::forget_cues();
            app.live = None;
            // Nothing drawn can be trusted to match the new run, so everything is drawn again.
            app.drawn = None;
        }
        let fixture_changed = app.controls.fixture != reply.controls.fixture
            || app.controls_seen.is_none()
            || boot_changed;
        app.controls = reply.controls;
        app.scene = reply.scene;
        app.controls_seen = Some((reply.boot_id, reply.revision));
        app.reshow();
        ControlsUpdate {
            changed: true,
            fixture_changed,
            boot_changed,
        }
    })
}

/// Reads the canvas the host serves, for a page that has just met another run of the host and has
/// no frame of it yet. The canvas is what the frame carries, so the next frame replaces it.
async fn refresh_canvas() {
    let canvas = match fetch_text(CANVAS_ADDRESS).await {
        Ok(text) => serde_json::from_str::<LiveCanvas>(&text).map_err(|error| PageError::NotRead {
            address: CANVAS_ADDRESS,
            wanted: "a canvas",
            reason: error.to_string(),
        }),
        Err(error) => Err(error.into()),
    };
    match canvas {
        Ok(canvas) => with_app_mut(|app| app.live = Some(canvas)),
        Err(error) => set_problem(error),
    }
}

/// Reads the host's controls. An unreadable answer is shown as a connection problem.
async fn refresh_controls() -> ControlsUpdate {
    let reply = match fetch_text(CONTROLS_ADDRESS).await {
        Ok(text) => {
            serde_json::from_str::<ControlsReply>(&text).map_err(|error| PageError::NotRead {
                address: CONTROLS_ADDRESS,
                wanted: "the controls",
                reason: error.to_string(),
            })
        }
        Err(error) => Err(error.into()),
    };
    match reply {
        Ok(reply) => {
            let update = apply_controls(reply);
            if update.boot_changed {
                refresh_canvas().await;
            }
            update
        }
        Err(error) => {
            set_problem(error);
            ControlsUpdate::default()
        }
    }
}

/// Sends one change to the host and draws what it answers.
pub async fn send_change(change: ControlsChange) {
    let body = match serde_json::to_string(&change) {
        Ok(body) => body,
        Err(error) => {
            log_error(&format!("could not write the change: {error}"));
            return;
        }
    };
    let answer = post_json(CHANGE_ADDRESS, &body)
        .await
        .map_err(PageError::from)
        .and_then(|text| {
            serde_json::from_str::<ControlsReply>(&text).map_err(|error| PageError::NotRead {
                address: CHANGE_ADDRESS,
                wanted: "the controls",
                reason: error.to_string(),
            })
        });
    match answer {
        Ok(reply) => {
            let update = apply_controls(reply);
            if update.boot_changed {
                refresh_canvas().await;
            }
            if update.fixture_changed {
                load_selected_fixture().await;
            } else if update.changed {
                sync_audio();
            }
        }
        Err(error) => set_problem(error),
    }
    render();
}

/// Takes one frame from the host as the page's own and draws what it changed.
async fn apply_frame(frame: LiveFrame) {
    let was_down = with_app(|app| app.connection_problem.is_some());
    let update = apply_controls(frame.controls);
    let canvas_changed = with_app_mut(|app| {
        app.frame = Some(frame.frame);
        let changed = app
            .live
            .as_ref()
            .is_none_or(|known| known.revision != frame.canvas.revision);
        if changed {
            app.live = Some(frame.canvas);
        }
        changed
    });
    if was_down {
        with_app_mut(|app| app.connection_problem = None);
    }
    let mut reloaded = false;
    if update.fixture_changed
        || with_app(|app| app.fixture.is_none() && app.controls.fixture.is_some())
    {
        load_selected_fixture().await;
        reloaded = true;
    }
    if update.changed && !reloaded {
        sync_audio();
    }
    let live_on_tv =
        with_app(|app| wants_live(app.shown.as_deref()) && app.controls.device.shows_tv());
    let action = frame_action(
        was_down,
        update.changed,
        reloaded,
        canvas_changed,
        live_on_tv,
    );
    if action.redraw {
        render();
    }
    if action.stage {
        update_stage();
    }
}

/// Asks the host for the next frame, over and over. The host holds each request until something
/// has changed, so a change reaches the page as it happens; if the host cannot be reached the
/// page says so and tries again.
async fn follow_frames() {
    loop {
        let address = match with_app(|app| app.frame) {
            Some(count) => format!("{FRAME_ADDRESS}?after={count}"),
            None => FRAME_ADDRESS.to_owned(),
        };
        let answer = fetch_text(&address)
            .await
            .map_err(PageError::from)
            .and_then(|text| {
                serde_json::from_str::<LiveFrame>(&text).map_err(|error| PageError::NotRead {
                    address: FRAME_ADDRESS,
                    wanted: "a frame",
                    reason: error.to_string(),
                })
            });
        match answer {
            Ok(frame) => {
                with_app_mut(|app| app.failures = 0);
                apply_frame(frame).await;
            }
            Err(error) => {
                let failures = with_app_mut(|app| {
                    app.failures = app.failures.saturating_add(1);
                    app.failures
                });
                let first_failure = with_app(|app| app.connection_problem.is_none());
                set_problem(error);
                if first_failure {
                    render();
                }
                TimeoutFuture::new(retry_delay_ms(failures)).await;
            }
        }
    }
}

/// The package half of a revision (`<assets>.<package>`).
fn package_part(revision: &str) -> Option<&str> {
    revision.split_once('.').map(|(_, package)| package)
}

/// One slow look at the server: a rebuilt package reloads the page, changed files reload the
/// theme, the fixture list and the fixture, and the controls are read once more in case a frame
/// was lost.
async fn poll() {
    let Ok(revision) = fetch_text(REVISION_ADDRESS).await else {
        // The frame loop reports the host being unreachable.
        return;
    };
    let package_changed = with_app(|app| {
        matches!(
            (package_part(&app.asset_revision), package_part(&revision)),
            (Some(known), Some(now)) if known != now
        )
    });
    if package_changed {
        if let Some(Err(error)) = web_sys::window().map(|window| window.location().reload()) {
            log_error(&format!(
                "could not reload the page: {}",
                js_message(&error)
            ));
        }
        return;
    }
    let files_changed = with_app(|app| app.asset_revision != revision);
    if files_changed {
        refresh_assets().await;
    }
    let update = refresh_controls().await;
    if files_changed || update.fixture_changed {
        load_selected_fixture().await;
    } else if update.changed {
        sync_audio();
    }
    if files_changed || update.changed {
        render();
    }
    let beat = with_app_mut(|app| {
        app.polls_since_report += 1;
        if app.polls_since_report >= HEARTBEAT_POLLS {
            app.polls_since_report = 0;
            true
        } else {
            false
        }
    });
    send_report(beat).await;
}

pub async fn run() {
    refresh_assets().await;
    refresh_controls().await;
    load_selected_fixture().await;
    render();
    spawn_local(follow_frames());
    loop {
        let wait = POLL_MS.max(retry_delay_ms(with_app(|app| app.failures)));
        TimeoutFuture::new(wait).await;
        poll().await;
    }
}
