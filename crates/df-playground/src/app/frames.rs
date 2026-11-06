//! Following the host: the frames it pushes, the controls it keeps, the slow look at its files and
//! the loop that runs them.
//!
//! Three paths bring controls to the page (a frame, a poll and the answer to a change the page
//! sent), and they can arrive out of order, so every one goes through `apply_controls`, which asks
//! the page's `ReplyGate`: an answer from an older run of the host, or an older revision of this
//! one, is left. The canvas is ordered the same way (`canvas_accepts`). The decisions are in
//! `continuity` and `decisions`, where native tests cover them.

use gloo_timers::future::TimeoutFuture;
use wasm_bindgen_futures::spawn_local;

use super::assets::{load_selected_fixture, refresh_assets};
use super::page::{render, update_stage};
use super::report::{schedule_forced_report, send_report};
use super::sound::sync_audio;
use super::state::{PageError, log_error, set_problem, with_app, with_app_mut};
use crate::audio;
use crate::backoff::retry_delay_ms;
use crate::continuity::{Admit, canvas_accepts};
use crate::controls::{ControlsChange, ControlsChangeReply, ControlsReply};
use crate::decisions::{controls_effect, fixture_load_needed};
use crate::dom::{js_message, with_continuity};
use crate::draw_plan::{frame_action, wants_live};
use crate::frame::{FrameReply, LiveCanvas, LiveFrame, frame_address, parse_frame_reply};
use crate::limits::{HEARTBEAT_POLLS, LONGEST_BROWSER_TIMER_MS, POLL_MS};
use crate::net::{NetError, fetch_text, post_json};
use crate::path;
use crate::timeline::ErrorSource;

/// What a look at the host's controls found.
#[derive(Debug, Clone, Copy, Default)]
pub struct ControlsUpdate {
    pub changed: bool,
    pub fixture_changed: bool,
    /// The answer is from another run of the host than the one the page was following.
    pub boot_changed: bool,
    /// The answer is from an older run than the one followed, and was left.
    pub stale_run: bool,
}

/// Takes the host's controls as the page's own when the gate lets them through: from a newer run
/// of the host or a newer revision of this one. An older answer, such as a slow one that was
/// overtaken, changes nothing.
pub fn apply_controls(reply: ControlsReply) -> ControlsUpdate {
    with_app_mut(|app| {
        let first = !app.gate.is_started();
        let new_run = match app.gate.admit(&reply.boot_id, reply.revision) {
            Admit::Refuse { older_run } => {
                return ControlsUpdate {
                    stale_run: older_run,
                    ..ControlsUpdate::default()
                };
            }
            Admit::Take { new_run } => new_run,
        };
        // A new run of the host numbers everything again, so what the page remembers by number
        // (moves, cues, the canvas) is dropped.
        let boot_changed = with_continuity(|continuity| continuity.note_boot(&reply.boot_id));
        let boot_changed = boot_changed || new_run;
        if boot_changed {
            audio::forget_cues();
            app.live = None;
            // Nothing drawn can be trusted to match the new run, so everything is drawn again.
            app.drawn = None;
        }
        let effect = controls_effect(
            app.controls.fixture.as_deref(),
            reply.controls.fixture.as_deref(),
            first,
            boot_changed,
        );
        app.controls = reply.controls;
        app.scene = reply.scene;
        app.reshow();
        ControlsUpdate {
            changed: true,
            fixture_changed: effect.fixture_changed,
            boot_changed,
            stale_run: false,
        }
    })
}

/// Takes a canvas as the page's own when it is newer than the one it has, from the run of the
/// host it is following.
fn apply_canvas(boot_id: &str, canvas: LiveCanvas) -> bool {
    with_app_mut(|app| {
        let known = app
            .live
            .as_ref()
            .map(|live| (app.live_boot.as_str(), live.revision));
        if !canvas_accepts(known, boot_id, canvas.revision) {
            return false;
        }
        app.live = Some(canvas);
        app.live_boot = boot_id.to_owned();
        true
    })
}

/// Reads the canvas the host serves, for a page that has just met another run of the host and has
/// no frame of it yet. A canvas is taken only if the page still follows that run and has nothing
/// newer, so a slow read never undoes a frame that arrived meanwhile.
async fn refresh_canvas(boot_id: String) {
    let canvas = match fetch_text(path::CANVAS).await {
        Ok(text) => serde_json::from_str::<LiveCanvas>(&text)
            .map_err(PageError::not_read(path::CANVAS, "a canvas")),
        Err(error) => Err(error.into()),
    };
    match canvas {
        Ok(canvas) => {
            let following = with_app(|app| app.gate.boot_id() == Some(boot_id.as_str()));
            if following {
                apply_canvas(&boot_id, canvas);
            }
        }
        Err(error) => set_problem(error),
    }
}

/// Reads the host's controls. An unreadable answer is shown as a connection problem.
async fn refresh_controls() -> ControlsUpdate {
    let reply = match fetch_text(path::CONTROLS).await {
        Ok(text) => serde_json::from_str::<ControlsReply>(&text)
            .map_err(PageError::not_read(path::CONTROLS, "the controls")),
        Err(error) => Err(error.into()),
    };
    match reply {
        Ok(reply) => {
            let boot_id = reply.boot_id.clone();
            let update = apply_controls(reply);
            if update.boot_changed {
                refresh_canvas(boot_id).await;
            }
            update
        }
        Err(error) => {
            set_problem(error);
            ControlsUpdate::default()
        }
    }
}

/// Reads the host's answer to a change: the controls after it, or the host's sentence when it
/// refused, whatever the status. An answer that is neither keeps its network error.
fn read_change_answer(answer: Result<String, NetError>) -> Result<ControlsReply, PageError> {
    let address = path::CONTROLS_CHANGE;
    let refused = |message| PageError::Rejected { address, message };
    match answer {
        Ok(text) => serde_json::from_str::<ControlsChangeReply>(&text)
            .map_err(PageError::not_read(address, "a reply to a change"))?
            .into_result()
            .map_err(refused),
        Err(error) => {
            let NetError::Status { body, .. } = &error else {
                return Err(error.into());
            };
            match serde_json::from_str::<ControlsChangeReply>(body) {
                Ok(ControlsChangeReply::Rejected { message }) => Err(refused(message)),
                Ok(ControlsChangeReply::Done { .. }) | Err(_) => Err(error.into()),
            }
        }
    }
}

/// Sends one change to the host and draws what it answers.
pub async fn send_change(change: ControlsChange) {
    let body = match serde_json::to_string(&change) {
        Ok(body) => body,
        Err(error) => {
            log_error(
                ErrorSource::Net,
                &format!("could not write the change: {error}"),
            );
            return;
        }
    };
    match read_change_answer(post_json(path::CONTROLS_CHANGE, &body).await) {
        Ok(reply) => {
            let boot_id = reply.boot_id.clone();
            let update = apply_controls(reply);
            if update.boot_changed {
                refresh_canvas(boot_id).await;
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
    let boot_id = frame.controls.boot_id.clone();
    let update = apply_controls(frame.controls);
    if update.stale_run {
        // A frame of an older run of the host: nothing in it is newer than what the page has.
        return;
    }
    let canvas_changed = apply_canvas(&boot_id, frame.canvas);
    with_app_mut(|app| app.frame = Some(frame.frame));
    if was_down {
        with_app_mut(|app| app.connection_problem = None);
        // The link is back: the host hears it at once.
        schedule_forced_report();
    }
    let mut reloaded = false;
    if with_app(|app| {
        fixture_load_needed(
            update.fixture_changed,
            app.fixture.is_some(),
            app.controls.fixture.is_some(),
        )
    }) {
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
/// has changed, so a change reaches the page as it happens; an empty answer means nothing changed
/// in its wait, and the page asks again. If the host cannot be reached the page says so and tries
/// again.
async fn follow_frames() {
    loop {
        let address = with_app(|app| frame_address(path::FRAME, app.frame, app.gate.boot_id()));
        let answer = fetch_text(&address)
            .await
            .map_err(PageError::from)
            .and_then(|text| {
                parse_frame_reply(&text).map_err(PageError::not_read(path::FRAME, "a frame"))
            });
        match answer {
            Ok(FrameReply::Frame(frame)) => {
                with_app_mut(|app| app.failures = 0);
                apply_frame(*frame).await;
            }
            Ok(FrameReply::NoChange) => {
                // The host is there and has nothing new; a problem shown earlier is over.
                let was_down = with_app_mut(|app| {
                    app.failures = 0;
                    app.connection_problem.take().is_some()
                });
                if was_down {
                    render();
                    schedule_forced_report();
                }
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
                    // The link went down: the report may still reach the host if only the frame
                    // poll failed.
                    schedule_forced_report();
                }
                wait_ms(retry_delay_ms(failures)).await;
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
    let Ok(revision) = fetch_text(path::REVISION).await else {
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
            log_error(
                ErrorSource::Asset,
                &format!("could not reload the page: {}", js_message(&error)),
            );
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
    // A change of the files is a change of the page's revision, which the host hears at once.
    send_report(beat || files_changed).await;
}

pub async fn run() {
    refresh_assets().await;
    refresh_controls().await;
    load_selected_fixture().await;
    render();
    spawn_local(follow_frames());
    loop {
        wait_ms(POLL_MS.max(retry_delay_ms(with_app(|app| app.failures)))).await;
        poll().await;
    }
}

/// Waits `ms` milliseconds, held at the longest wait a browser timer takes.
fn wait_ms(ms: u64) -> TimeoutFuture {
    let held = ms.min(LONGEST_BROWSER_TIMER_MS);
    TimeoutFuture::new(u32::try_from(held).unwrap_or(u32::MAX))
}
