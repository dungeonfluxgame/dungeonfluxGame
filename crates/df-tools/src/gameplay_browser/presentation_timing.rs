//! Page-owned best-effort presentation capture. Input/render never awaits telemetry.

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::{Rc, Weak},
    time::Duration,
};

use df_observe::presentation::{
    TimingBuffer, TimingEvent, VisualPlateState, VisualShot, VisualSource, VisualSourceDetails,
};
use futures::future::{AbortHandle, Abortable};
use wasm_bindgen::{JsCast, closure::Closure};

use super::{BrowserChannel, Client, rpc};
use crate::stage_machines::{ClipState, LineState};

type TimingClient =
    rpc::presentation_telemetry_service_client::PresentationTelemetryServiceClient<BrowserChannel>;

type PaintFrame = (i32, Closure<dyn FnMut(f64)>);

struct Capture {
    client: Weak<RefCell<Client>>,
    root: web_sys::Element,
    display: bool,
    scope: rpc::WatchViewRequest,
    producer: Vec<u8>,
    generation: u64,
    buffer: TimingBuffer,
    lines: HashMap<u64, TimingEvent>,
    closed_lines: VecDeque<TimingEvent>,
    abort: Option<AbortHandle>,
    usable: bool,
    painted: bool,
    frames: Vec<PaintFrame>,
}

impl Drop for Capture {
    fn drop(&mut self) {
        if let Some(abort) = self.abort.take() {
            abort.abort();
        }
        if let Some(window) = web_sys::window() {
            for (id, _) in &self.frames {
                let _cancelled = window.cancel_animation_frame(*id);
            }
        }
        if self.buffer.dropped() != 0 || self.buffer.batch().is_ok() {
            web_sys::console::warn_1(
                &"Presentation timing page closed with unreported records".into(),
            );
        }
    }
}

thread_local! {
    static CAPTURES: RefCell<Vec<Capture>> = const { RefCell::new(Vec::new()) };
    // The join controls may be usable before a credential exists. Retain only their time;
    // uploading still waits for authenticated scope and the existing connection.
    static FIRST_USABLE: RefCell<Vec<(Weak<RefCell<Client>>, u64)>> = const { RefCell::new(Vec::new()) };
}

fn now_ms() -> Option<u64> {
    let measured = web_sys::window()
        .and_then(|window| window.performance())
        .map(|clock| clock.now())
        .filter(|time| time.is_finite() && *time >= 0.0);
    if measured.is_none() {
        web_sys::console::warn_1(&"Presentation timing clock unavailable".into());
    }
    measured.map(|time| time as u64)
}

// Stable content-free local identifiers. Clear the sign bit for the OTLP integer domain.
pub(super) fn identity(text: &str) -> u64 {
    df_observe::presentation::local_identity(text)
}

fn retain(capture: &mut Capture, event: TimingEvent) {
    if let Err(error) = capture.buffer.push(event) {
        web_sys::console::warn_1(
            &format!(
                "Presentation timing capture refused: {error:?}; dropped={}",
                capture.buffer.dropped()
            )
            .into(),
        );
    }
}

fn displays(mut record: impl FnMut(&mut Capture)) {
    CAPTURES.with_borrow_mut(|captures| {
        captures.retain(|capture| capture.client.strong_count() != 0);
        for capture in captures {
            if capture.display {
                record(capture);
            }
        }
    });
}

pub(super) fn media(
    kind: &'static str,
    scene: &str,
    asset: &str,
    reason: &'static str,
    crossfade_ms: u32,
) {
    let Some(now) = now_ms() else {
        return;
    };
    let mut event = TimingEvent::lifecycle(kind, reason, identity(asset), identity(scene), now);
    event.crossfade_ms = crossfade_ms;
    displays(|capture| retain(capture, event.clone()));
}

/// Report the requested and displayed scene source using only closed categories and local hashes.
pub(super) fn visual_source(
    scene: &str,
    requested: VisualSource,
    effective: VisualSource,
    shot: VisualShot,
    plate_state: VisualPlateState,
    requested_asset_id: Option<u64>,
    effective_asset_id: Option<u64>,
) {
    let Some(now) = now_ms() else {
        return;
    };
    let event = TimingEvent::visual_source(
        effective_asset_id
            .or(requested_asset_id)
            .unwrap_or_default(),
        identity(scene),
        now,
        VisualSourceDetails {
            requested,
            effective,
            shot,
            plate_state,
            requested_asset: requested_asset_id,
            effective_asset: effective_asset_id,
        },
    );
    displays(|capture| retain(capture, event.clone()));
}

/// A device's observed sound state, scoped to Client.root (including phones). Source is a
/// static category such as "microphone" or "playback", never a device label or user content.
pub(super) fn sound(root: &web_sys::Element, source: &'static str, reason: &'static str) {
    let Some(now) = now_ms() else {
        return;
    };
    let event = TimingEvent::lifecycle("sound", reason, identity(source), 0, now);
    CAPTURES.with_borrow_mut(|captures| {
        captures.retain(|capture| capture.client.strong_count() != 0);
        for capture in captures.iter_mut().filter(|capture| capture.root == *root) {
            retain(capture, event.clone());
        }
    });
}

/// Called after the adapter successfully applies the conductor's line command. Never pass text.
pub(super) fn line(
    id: u64,
    state: LineState,
    slot: Option<usize>,
    characters: u32,
    read_ms: u32,
    scene: &str,
) {
    let Some(now) = now_ms() else {
        return;
    };
    displays(|capture| {
        if matches!(state, LineState::Opening | LineState::Lingering)
            && !capture.lines.contains_key(&id)
        {
            if capture.lines.len() == 128 {
                capture.buffer.report_loss(1);
                web_sys::console::warn_1(&"Presentation timing active line capacity".into());
                return;
            }
            let mut event = TimingEvent::lifecycle("line-open", "ready", id, identity(scene), now);
            event.slot = slot
                .and_then(|slot| u32::try_from(slot).ok())
                .filter(|slot| *slot <= 31);
            event.chars = characters;
            event.read_ms = read_ms;
            event.opened_ms = Some(now);
            capture.lines.insert(id, event.clone());
            retain(capture, event);
        }
        if state == LineState::Done {
            let Some(mut event) = capture.lines.remove(&id) else {
                return;
            };
            event.kind = "line";
            event.at_ms = now;
            event.closed_ms = Some(now);
            // A missing end remains visible as a cut/unknown in timeline; never fabricate an end.
            event.reason = if event.reason != "ready" {
                event.reason
            } else if event
                .voice_end_ms
                .zip(event.opened_ms)
                .is_some_and(|(ended, opened)| {
                    ended.saturating_sub(opened) >= u64::from(event.read_ms)
                })
            {
                "voice-ended"
            } else if event.voice_start_ms.is_some() && event.voice_end_ms.is_none() {
                "skipped"
            } else if event
                .opened_ms
                .is_some_and(|opened| now.saturating_sub(opened) >= u64::from(event.read_ms))
            {
                "read"
            } else {
                "scene-change"
            };
            if capture.closed_lines.len() == 128 {
                capture.closed_lines.pop_front();
            }
            capture.closed_lines.push_back(event.clone());
            retain(capture, event);
        }
    });
}

/// The conductor retains the actual transition cause (including watchdog expiry). Set before
/// applying the closing command; do not infer successful voice completion from its Ended state.
pub(super) fn close_reason(id: u64, reason: &'static str) {
    displays(|capture| {
        if let Some(event) = capture.lines.get_mut(&id) {
            event.reason = reason;
        }
    });
}

/// Preserve the timestamp captured at the actual callback even when its line mounts later.
pub(super) fn voice_at(id: u64, state: ClipState, now: u64) {
    displays(|capture| {
        // Media callbacks can arrive after the line's DOM closed. Keep that observation so
        // the summary can report an actual cut instead of losing the late voice end.
        let event = match capture.lines.get_mut(&id) {
            Some(event) => Some(event),
            None => capture.closed_lines.iter_mut().find(|event| event.id == id),
        };
        let Some(event) = event else {
            return;
        };
        let (kind, reason) = match state {
            ClipState::Playing => {
                if event.voice_start_ms.is_some() {
                    return;
                }
                event.voice_start_ms = Some(now);
                ("voice-start", "ready")
            }
            ClipState::Ended => {
                if event.voice_end_ms.is_some() {
                    return;
                }
                event.voice_end_ms = Some(now);
                ("voice-end", "ended")
            }
            ClipState::Failed => ("voice-failed", "failed"),
            _ => return,
        };
        let mut event = event.clone();
        event.kind = kind;
        event.reason = reason;
        event.at_ms = now;
        retain(capture, event);
    });
}

pub(super) fn report_loss(records: u64) {
    displays(|capture| capture.buffer.report_loss(records));
}

/// Bind once per live connection. Reconnect preserves pending bytes/identity, aborts its old loop.
pub(super) fn connect(client: &Rc<RefCell<Client>>, channel: BrowserChannel) {
    let weak = Rc::downgrade(client);
    let (scope, credential, root, display) = {
        let state = client.borrow();
        (
            rpc::WatchViewRequest {
                session_id: state.session.clone(),
                run_id: state.run.clone(),
                client_binding_id: state.binding.clone(),
                after_revision: None,
            },
            state.credential.clone(),
            state.root.clone(),
            state.role == "display",
        )
    };
    let (abort, registration) = AbortHandle::new_pair();
    let setup = CAPTURES.with_borrow_mut(|captures| {
        captures.retain(|capture| {
            capture.client.strong_count() != 0
                && (!capture.client.ptr_eq(&weak) || capture.scope == scope)
        });
        if !captures.iter().any(|capture| capture.client.ptr_eq(&weak)) {
            if captures.len() == 8 {
                return None;
            }
            let mut producer = vec![0; 16];
            let crypto = web_sys::window().and_then(|window| window.crypto().ok())?;
            if crypto
                .get_random_values_with_u8_array(&mut producer)
                .is_err()
                || producer.iter().all(|byte| *byte == 0)
            {
                return None;
            }
            captures.push(Capture {
                client: weak.clone(),
                root,
                display,
                scope: scope.clone(),
                producer,
                generation: 0,
                buffer: TimingBuffer::default(),
                lines: HashMap::new(),
                closed_lines: VecDeque::new(),
                abort: None,
                usable: false,
                painted: false,
                frames: Vec::new(),
            });
        }
        let capture = captures
            .iter_mut()
            .find(|capture| capture.client.ptr_eq(&weak))?;
        if let Some(old) = capture.abort.replace(abort) {
            old.abort();
        }
        capture.generation += 1;
        Some(capture.generation)
    });
    let Some(generation) = setup else {
        web_sys::console::warn_1(&"Presentation timing producer unavailable".into());
        return;
    };
    wasm_bindgen_futures::spawn_local(async move {
        let upload = async {
            let mut rpc_client = TimingClient::new(channel)
                .max_encoding_message_size(df_rpc_bridge::RPC_MESSAGE_BYTES)
                .max_decoding_message_size(1024);
            loop {
                gloo_timers::future::TimeoutFuture::new(1_000).await;
                if weak.strong_count() == 0 {
                    break;
                }
                let batch = CAPTURES.with_borrow_mut(|captures| {
                    let capture = captures.iter_mut().find(|capture| {
                        capture.client.ptr_eq(&weak) && capture.generation == generation
                    })?;
                    if capture.display {
                        let (events, lost) = df_observe::presentation::drain_display_capture();
                        capture.buffer.report_loss(lost);
                        for event in events {
                            retain(capture, event);
                        }
                    }
                    Some((capture.producer.clone(), capture.buffer.batch().ok()?))
                });
                let Some((producer, batch)) = batch else {
                    continue;
                };
                let body = rpc::PresentationTelemetryRequest {
                    scope: Some(scope.clone()),
                    producer_id: producer,
                    batch_sequence: batch.sequence,
                    otlp_logs: batch.bytes,
                    dropped_records: batch.dropped,
                };
                let Ok(mut request) = super::request(body, &credential) else {
                    break;
                };
                request.set_timeout(Duration::from_secs(10));
                match rpc_client.upload(request).await {
                    Ok(response) => {
                        let response = response.into_inner();
                        if response.batch_sequence == batch.sequence
                            && response.accepted_records as usize == batch.records
                        {
                            CAPTURES.with_borrow_mut(|captures| {
                                if let Some(capture) = captures.iter_mut().find(|capture| {
                                    capture.client.ptr_eq(&weak) && capture.generation == generation
                                }) {
                                    capture.buffer.acknowledge(batch.sequence);
                                }
                            });
                        } else {
                            web_sys::console::warn_1(
                                &"Presentation timing receipt mismatch".into(),
                            );
                        }
                    }
                    Err(error) => {
                        // Stable retry is bounded to one request/second, with only one in flight.
                        // Capture remains best-effort across a native process restart.
                        web_sys::console::warn_1(
                            &format!("Presentation timing upload failed: {:?}", error.code())
                                .into(),
                        );
                        gloo_timers::future::TimeoutFuture::new(4_000).await;
                    }
                }
            }
        };
        let _closed = Abortable::new(upload, registration).await;
    });
}

/// Render has completed and controls have their current callbacks. Use the browser's retained
/// first-paint entry (including loading HTML before WASM), never a later RPC/RAF as first paint.
pub(super) fn paint(client: &Rc<RefCell<Client>>) {
    let Some(now) = now_ms() else {
        return;
    };
    let weak = Rc::downgrade(client);
    let root = client.borrow().root.clone();
    let controls_ready = root
        .query_selector("button:not(:disabled),input:not(:disabled),select:not(:disabled),textarea:not(:disabled)")
        .ok()
        .flatten()
        .is_some();
    let usable_at = FIRST_USABLE.with_borrow_mut(|times| {
        times.retain(|(owner, _)| owner.strong_count() != 0);
        if let Some((_, time)) = times.iter().find(|(owner, _)| owner.ptr_eq(&weak)) {
            return Some(*time);
        }
        if !controls_ready {
            return None;
        }
        if times.len() == 8 {
            web_sys::console::warn_1(&"Presentation first-usable capacity".into());
            return None;
        }
        times.push((weak.clone(), now));
        Some(now)
    });
    CAPTURES.with_borrow_mut(|captures| {
        let Some(capture) = captures
            .iter_mut()
            .find(|capture| capture.client.ptr_eq(&weak))
        else {
            return;
        };
        if !capture.usable
            && let Some(time) = usable_at
        {
            capture.usable = true;
            retain(
                capture,
                TimingEvent::lifecycle("usable", "ready", 0, 0, time),
            );
        }
        if capture.painted {
            return;
        }
        capture.painted = true;
        if let Some(first_paint) = first_paint_ms() {
            retain(
                capture,
                TimingEvent::lifecycle("paint", "ready", 0, 0, first_paint),
            );
        } else {
            // Unsupported Paint Timing stays explicitly missing in the summary.
            schedule_paint(capture, weak, false);
        }
    });
}

fn first_paint_ms() -> Option<u64> {
    let clock = web_sys::window()?.performance()?;
    for entry in clock.get_entries_by_type("paint").iter() {
        let name = js_sys::Reflect::get(&entry, &"name".into())
            .ok()?
            .as_string()?;
        if name == "first-paint" {
            let start = js_sys::Reflect::get(&entry, &"startTime".into())
                .ok()?
                .as_f64()?;
            return (start.is_finite() && start >= 0.0).then_some(start as u64);
        }
    }
    None
}

fn schedule_paint(capture: &mut Capture, weak: Weak<RefCell<Client>>, second: bool) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let callback = Closure::wrap(Box::new(move |_: f64| {
        CAPTURES.with_borrow_mut(|captures| {
            if let Some(capture) = captures
                .iter_mut()
                .find(|capture| capture.client.ptr_eq(&weak))
            {
                if second {
                    let Some(now) = now_ms() else {
                        return;
                    };
                    let measured = first_paint_ms();
                    retain(
                        capture,
                        TimingEvent::lifecycle(
                            if measured.is_some() {
                                "paint"
                            } else {
                                "paint-opportunity"
                            },
                            "ready",
                            0,
                            0,
                            measured.unwrap_or(now),
                        ),
                    );
                } else {
                    schedule_paint(capture, weak.clone(), true);
                }
            }
        });
    }) as Box<dyn FnMut(f64)>);
    match window.request_animation_frame(callback.as_ref().unchecked_ref()) {
        Ok(id) => capture.frames.push((id, callback)),
        Err(_) => web_sys::console::warn_1(&"Presentation timing paint frame unavailable".into()),
    }
}

pub(super) fn disconnect(client: &Client) {
    CAPTURES.with_borrow_mut(|captures| {
        for capture in captures
            .iter_mut()
            .filter(|capture| capture.root == client.root)
        {
            if let Some(abort) = capture.abort.take() {
                abort.abort();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identifiers_are_stable_content_free_and_fit_otlp_integers() {
        assert_eq!(identity("scene"), identity("scene"));
        assert_ne!(identity("scene"), identity("other-scene"));
        assert!(identity("speaker private text") <= i64::MAX as u64);
    }
}
