//! Following the host: the frames it pushes (the page's only view stream: the canvas, the controls
//! and the revision of its files), the page's heartbeat and the loop that runs them.
//!
//! Two paths bring controls to the page (a frame and the answer to a change the page sent), and
//! they can arrive out of order, so both go through `apply_controls`, which asks
//! the page's `ReplyGate`: an answer from an older run of the host, or an older revision of this
//! one, is left. The canvas is ordered the same way (`canvas_accepts`). The decisions are in
//! `continuity` and `decisions`, where native tests cover them.

use std::cell::Cell;

use gloo_timers::future::TimeoutFuture;
use wasm_bindgen_futures::spawn_local;

use super::assets::{load_selected_fixture, refresh_assets};
use super::page::{render, update_stage};
use super::report::{schedule_forced_report, send_report};
use super::sound::{sync_audio, take_host_prefs};
use super::state::{PageError, log_error, set_problem, with_app, with_app_mut};
use super::voice;
use crate::audio;
use crate::backoff::retry_delay_ms;
use crate::continuity::{Admit, canvas_accepts};
use crate::controls::{ControlsChange, ControlsReply};
use crate::decisions::{controls_effect, fixture_load_needed};
use crate::dom::{js_message, with_continuity};
use crate::draw_plan::{frame_action, wants_live};
use crate::frame::{Frame, FrameCanvas, FrameReply, frame_address, parse_frame_reply};
use crate::limits::{
    FRAME_REQUEST_TIMEOUT_MS, HEARTBEAT_POLLS, LONGEST_BROWSER_TIMER_MS, POLL_MS, WAKE_CHECK_MS,
};
use crate::net::{Call, CallError, NetError, call, text};
use crate::path;
use crate::sound_plan::AudioPrefs;
use crate::timeline::ErrorSource;
use crate::voice_wire::SpeechErrorKind;

/// What a look at the host's controls found.
#[derive(Debug, Clone, Copy, Default)]
pub struct ControlsUpdate {
    pub changed: bool,
    pub fixture_changed: bool,
    /// The answer is from another run of the host than the one the page was following.
    pub boot_changed: bool,
    /// The answer is from an older run than the one followed, and was left.
    pub stale_run: bool,
    /// The sound preferences the host now holds, when they are not the ones the page had from it.
    pub audio: Option<AudioPrefs>,
}

/// Takes the host's controls as the page's own when the gate lets them through: from a newer run
/// of the host or a newer revision of this one. An older answer, such as a slow one that was
/// overtaken, changes nothing.
pub fn apply_controls(reply: ControlsReply) -> ControlsUpdate {
    let following = with_app(|app| app.gate.is_started());
    let update = with_app_mut(|app| {
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
        super::toolbar::note_plate_reply(reply.controls.plate.now_ms);
        let audio_before = app.controls.audio;
        let audio_after = reply.controls.audio;
        app.controls = reply.controls;
        app.controls.device = app.role.shown_device(app.controls.device);
        app.scene = reply.scene;
        app.reshow();
        ControlsUpdate {
            audio: (audio_before != audio_after).then_some(audio_after),
            changed: true,
            fixture_changed: effect.fixture_changed,
            boot_changed,
            stale_run: false,
        }
    });
    if update.changed {
        super::toolbar::refresh_plate();
    }
    if update.boot_changed && following {
        // The host restarted: its media links died with it, and a phone says hello again.
        crate::path::forget_media_links();
        super::seat::host_restarted();
    }
    if let Some(prefs) = update.audio {
        take_host_prefs(prefs);
    }
    update
}

/// Takes a canvas as the page's own when it is newer than the one it has, from the run of the
/// host it is following.
fn apply_canvas(boot_id: &str, canvas: FrameCanvas) -> bool {
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

/// Sends one change to the host and draws what it answers: the controls after it, or the host's
/// sentence when it refused. An answer that is neither is noted.
pub async fn send_change(change: ControlsChange) {
    let address = path::CONTROLS_CHANGE;
    let answer = call::<Box<ControlsReply>, SpeechErrorKind>(Call::post(address, &change)).await;
    match answer {
        Ok(reply) => {
            // A new run of the host met here sends its canvas in the next frame, at once.
            let update = apply_controls(*reply);
            if update.fixture_changed {
                load_selected_fixture().await;
            } else if update.changed {
                sync_audio();
            }
        }
        Err(CallError::Refused(rejection)) => set_problem(PageError::Rejected {
            address,
            message: rejection.message,
        }),
        Err(CallError::Net(error)) => set_problem(error),
        Err(error @ (CallError::Unreadable { .. } | CallError::Unwritable { .. })) => {
            log_error(ErrorSource::Net, &error.to_string());
        }
    }
    render();
}

/// Takes the revision of the host's files a frame carries: a rebuilt package reloads the page, and
/// changed files read the theme and the fixture list again (the host hears of it at once). The
/// first revision is the one the page started with. Whether the files changed.
async fn follow_files(revision: String) -> bool {
    let known = with_app_mut(|app| std::mem::replace(&mut app.asset_revision, revision.clone()));
    if known.is_empty() || known == revision {
        return false;
    }
    if package_part(&known) != package_part(&revision) {
        if let Some(Err(error)) = web_sys::window().map(|window| window.location().reload()) {
            log_error(
                ErrorSource::Asset,
                &format!("could not reload the page: {}", js_message(&error)),
            );
        }
        return false;
    }
    refresh_assets().await;
    schedule_forced_report();
    true
}

/// Takes one frame from the host as the page's own and draws what it changed.
async fn apply_frame(frame: Frame) {
    let was_down = with_app(|app| app.connection_problem.is_some());
    let boot_id = frame.controls.boot_id.clone();
    let update = apply_controls(frame.controls);
    if update.stale_run {
        // A frame of an older run of the host: nothing in it is newer than what the page has.
        return;
    }
    let canvas_changed = apply_canvas(&boot_id, frame.canvas);
    let files_changed = follow_files(frame.asset_revision).await;
    with_app_mut(|app| app.frame = Some(frame.frame));
    if was_down {
        with_app_mut(|app| app.connection_problem = None);
        // The link is back: the host hears it at once, and a tap waiting on it goes now.
        wake();
        spawn_local(voice::refresh_host_state());
    }
    let mut reloaded = false;
    if files_changed
        || with_app(|app| {
            fixture_load_needed(
                update.fixture_changed,
                app.fixture.is_some(),
                app.controls.fixture.is_some(),
            )
        })
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
        update.changed || files_changed,
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
        // The host holds a frame request until something changes; a dead connection is given up
        // after the hold, so the loop asks again.
        let answer = text(Call::get(&address).within(FRAME_REQUEST_TIMEOUT_MS))
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
                    wake();
                    spawn_local(voice::refresh_host_state());
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
                    voice::on_host_lost();
                    render();
                    // The link went down: the report may still reach the host if only the frame
                    // poll failed.
                    schedule_forced_report();
                }
                wait_or_wake(retry_delay_ms(failures)).await;
            }
        }
    }
}

/// The package half of a revision (`<assets>.<package>`).
fn package_part(revision: &str) -> Option<&str> {
    revision.split_once('.').map(|(_, package)| package)
}

/// The page's heartbeat: it reports what it shows when that changed, and every `HEARTBEAT_POLLS`
/// looks even when nothing did, so the host can tell the page is alive; with that beat it asks
/// whether speech and the personas changed.
async fn heartbeat() {
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
    if beat {
        voice::refresh_host_state().await;
    }
}

/// Reads the theme and the scenario list, then follows the frames (the first answers at once,
/// with the controls, the canvas and the files' revision) and beats.
pub async fn run() {
    refresh_assets().await;
    render();
    spawn_local(follow_frames());
    loop {
        wait_or_wake(POLL_MS.max(retry_delay_ms(with_app(|app| app.failures)))).await;
        heartbeat().await;
    }
}

/// Waits `ms` milliseconds, held at the longest wait a browser timer takes.
pub(super) fn wait_ms(ms: u64) -> TimeoutFuture {
    let held = ms.min(LONGEST_BROWSER_TIMER_MS);
    TimeoutFuture::new(u32::try_from(held).unwrap_or(u32::MAX))
}

thread_local! {
    /// How many times the page has been woken (`wake`).
    static WAKES: Cell<u64> = const { Cell::new(0) };
}

/// The page is in front again (a phone back from its lock screen or another app), back online, or
/// its link to the host is back: every loop waiting out a backoff in `wait_or_wake` stops waiting
/// and asks the host now (a seat action that did not get through is sent again), the failures in
/// a row are forgotten, and a report goes at once, so the host hears the page is back (its
/// `CLIENT link client= state=heard silent_ms=` line, when it had gone quiet).
pub(super) fn wake() {
    WAKES.with(|wakes| wakes.set(wakes.get().wrapping_add(1)));
    with_app_mut(|app| app.failures = 0);
    schedule_forced_report();
}

/// A seat route could not reach the host: the link line shows, as when the frame loop fails, and
/// the host hears of it if the report gets through.
pub(super) fn seat_link_lost(error: NetError) {
    let first = with_app(|app| app.connection_problem.is_none());
    set_problem(error);
    if first {
        render();
        schedule_forced_report();
    }
}

/// A seat route answered: the link line, when it shows, is over, and what waited on the link goes.
pub(super) fn seat_link_back() {
    let was_down = with_app_mut(|app| app.connection_problem.take().is_some());
    if was_down {
        render();
        wake();
    }
}

/// Waits `ms` milliseconds, or less when the page is woken meanwhile (`wake`): it looks every
/// `WAKE_CHECK_MS`. Whether it was woken.
pub(super) async fn wait_or_wake(ms: u64) -> bool {
    let wakes_before = WAKES.with(Cell::get);
    let mut left = ms;
    while left > 0 {
        let slice = left.min(WAKE_CHECK_MS);
        wait_ms(slice).await;
        left = left.saturating_sub(slice);
        if WAKES.with(Cell::get) != wakes_before {
            return true;
        }
    }
    false
}
