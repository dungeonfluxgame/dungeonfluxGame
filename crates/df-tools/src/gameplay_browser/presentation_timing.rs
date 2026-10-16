//! Page-owned best-effort presentation capture. Input/render never awaits telemetry.

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::{Rc, Weak},
    time::Duration,
};

use df_observe::presentation::{
    ComponentClock, ComponentPhase, EffectiveVisibility, PresentationComponent, TimingBuffer,
    TimingEvent, VisualPlateState, VisualShot, VisualSource, VisualSourceDetails,
};
use futures::future::{AbortHandle, Abortable};
use gloo_timers::future::TimeoutFuture;
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
    components: HashMap<(PresentationComponent, u64, u64), ActiveComponent>,
    abort: Option<AbortHandle>,
    usable: bool,
    painted: bool,
    frames: Vec<PaintFrame>,
}

struct ActiveComponent {
    clock: ComponentClock,
    read_ms: u32,
    phase: ComponentPhase,
    readable_pending: bool,
    visibility: EffectiveVisibility,
    close_reason: &'static str,
    element: Option<web_sys::Element>,
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
    static NEXT_COMPONENT_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
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

/// Unique page-local identity for short-lived presentation elements without a server line id.
pub(super) fn next_component_id() -> u64 {
    NEXT_COMPONENT_ID.with(|next| {
        let id = next.get();
        next.set(id.saturating_add(1).max(1));
        id
    })
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
        for ((_, component_id, _), component) in &mut capture.components {
            if *component_id == id {
                component.close_reason = reason;
            }
        }
    });
}

/// Record one real presentation-component transition after the UI applies it. The element is
/// sampled for computed style, viewport intersection and occlusion; a mounted DOM node alone
/// never counts as readable.
pub(super) fn component(
    element: Option<&web_sys::Element>,
    component: PresentationComponent,
    id: u64,
    scene: &str,
    phase: ComponentPhase,
    reason: &'static str,
    read_ms: u32,
) {
    let Some(now) = now_ms() else {
        return;
    };
    let scene = identity(scene);
    displays(|capture| {
        let key = (component, id, scene);
        let closing = matches!(phase, ComponentPhase::Hidden | ComponentPhase::Failed);
        let visibility = element.map_or_else(
            || {
                if closing {
                    EffectiveVisibility::Hidden
                } else {
                    EffectiveVisibility::Unknown
                }
            },
            |element| effective_visibility(Some(element)),
        );
        if phase == ComponentPhase::Failed && !capture.components.contains_key(&key) {
            let mut event = TimingEvent::component_lifecycle(
                id, scene, now, component, phase, visibility, reason,
            );
            event.read_ms = read_ms;
            event.closed_ms = Some(now);
            event.visible_held_ms = Some(0);
            retain(capture, event);
            return;
        }
        let created = !capture.components.contains_key(&key) && !closing;
        let readable_pending =
            phase == ComponentPhase::Readable && visibility != EffectiveVisibility::Visible;
        let active_phase = if readable_pending {
            ComponentPhase::Opened
        } else {
            phase
        };
        if created {
            if capture.components.len() >= 256 {
                capture.buffer.report_loss(1);
                web_sys::console::warn_1(&"Presentation timing active component capacity".into());
                return;
            }
            capture.components.insert(
                key,
                ActiveComponent {
                    clock: ComponentClock::default(),
                    read_ms,
                    phase: active_phase,
                    readable_pending,
                    visibility,
                    close_reason: if active_phase == ComponentPhase::Closing {
                        reason
                    } else {
                        "ended"
                    },
                    element: element.cloned(),
                },
            );
        }
        let Some(active) = capture.components.get_mut(&key) else {
            return;
        };
        let event = {
            if !created
                && !closing
                && active.phase == phase
                && active.visibility == visibility
                && read_ms <= active.read_ms
            {
                return;
            }
            active.read_ms = read_ms.max(active.read_ms);
            let previous_phase = active.phase;
            if phase == ComponentPhase::Readable {
                active.readable_pending = readable_pending;
            } else if matches!(phase, ComponentPhase::Hidden | ComponentPhase::Failed) {
                active.readable_pending = false;
            }
            active.phase = if phase == ComponentPhase::Readable && readable_pending {
                previous_phase
            } else {
                active_phase
            };
            active.visibility = visibility;
            if element.is_some() {
                active.element = element.cloned();
            }
            let visible_held_ms = active.clock.observe(now, active.phase, visibility);
            let mut event = TimingEvent::component_lifecycle(
                id,
                scene,
                now,
                component,
                active.phase,
                visibility,
                reason,
            );
            event.read_ms = active.read_ms;
            event.opened_ms = active.clock.opened_ms();
            event.visible_held_ms = Some(visible_held_ms);
            if active.phase == ComponentPhase::Closing && active.close_reason == "ended" {
                active.close_reason = reason;
            }
            if active.phase == ComponentPhase::Closing {
                event.reason = active.close_reason;
            } else if closing && previous_phase == ComponentPhase::Closing {
                event.reason = active.close_reason;
            } else if closing {
                event.reason = reason;
            }
            if closing {
                event.closed_ms = Some(now);
            }
            event
        };
        if closing {
            capture.components.remove(&key);
        }
        retain(capture, event);
    });
    refresh_component_visibility();
}

/// Delay an element phase until its entrance, readable window or authored hold completes. The
/// callback rechecks the live element and carries only its closed component id and timings.
pub(super) fn schedule_component_transition(
    element: web_sys::Element,
    component_type: PresentationComponent,
    id: u64,
    scene: String,
    phase: ComponentPhase,
    reason: &'static str,
    read_ms: u32,
    delay_ms: u32,
) {
    wasm_bindgen_futures::spawn_local(async move {
        TimeoutFuture::new(delay_ms).await;
        if !component_is_active(component_type, id, &scene, &element, phase) {
            return;
        }
        if phase == ComponentPhase::Hidden {
            // The timer starts a bounded browser visibility probe; it never fabricates a close.
            for _ in 0..60 {
                let visibility = effective_visibility(Some(&element));
                if visibility != EffectiveVisibility::Visible
                    && visibility != EffectiveVisibility::Unknown
                {
                    component(
                        Some(&element),
                        component_type,
                        id,
                        &scene,
                        ComponentPhase::Hidden,
                        reason,
                        read_ms,
                    );
                    return;
                }
                TimeoutFuture::new(100).await;
                if !component_is_active(component_type, id, &scene, &element, phase) {
                    return;
                }
            }
            return;
        }
        component(
            Some(&element),
            component_type,
            id,
            &scene,
            phase,
            reason,
            read_ms,
        );
        if phase != ComponentPhase::Readable {
            return;
        }
        // A card or overlay may still cover the element when its entrance delay expires. Keep
        // checking briefly for the first genuinely readable frame; later visibility refreshes
        // can still complete the phase if this bounded probe ends first.
        for _ in 0..30 {
            TimeoutFuture::new(100).await;
            if !component_is_active(component_type, id, &scene, &element, phase) {
                return;
            }
            if effective_visibility(Some(&element)) == EffectiveVisibility::Visible {
                component(
                    Some(&element),
                    component_type,
                    id,
                    &scene,
                    ComponentPhase::Readable,
                    reason,
                    read_ms,
                );
                return;
            }
        }
    });
}

fn component_is_active(
    component: PresentationComponent,
    id: u64,
    scene: &str,
    element: &web_sys::Element,
    next_phase: ComponentPhase,
) -> bool {
    if !element.is_connected() {
        return false;
    }
    let scene_id = identity(scene);
    CAPTURES.with_borrow(|captures| {
        captures.iter().any(|capture| {
            capture.display
                && capture
                    .components
                    .get(&(component, id, scene_id))
                    .is_some_and(|active| {
                        active.element.as_ref().is_some_and(|active_element| {
                            active_element.is_same_node(Some(element.as_ref()))
                        }) && match next_phase {
                            ComponentPhase::Readable => active.phase != ComponentPhase::Closing,
                            ComponentPhase::Hidden => active.phase == ComponentPhase::Closing,
                            _ => true,
                        }
                    })
        })
    })
}

/// Re-sample tracked elements after a display overlay or scene transition. This emits only when
/// effective visibility changed, so a logically active speech bubble hidden behind a card shows
/// up explicitly in the same timeline.
pub(super) fn refresh_component_visibility() {
    let Some(now) = now_ms() else {
        return;
    };
    displays(|capture| {
        let mut changed = Vec::new();
        let mut closed = Vec::new();
        for ((component, id, scene), active) in &mut capture.components {
            let visibility = effective_visibility(active.element.as_ref());
            let hidden_while_closing = active.phase == ComponentPhase::Closing
                && visibility != EffectiveVisibility::Visible
                && visibility != EffectiveVisibility::Unknown;
            let became_readable =
                active.readable_pending && visibility == EffectiveVisibility::Visible;
            if visibility == active.visibility && !became_readable && !hidden_while_closing {
                continue;
            }
            if hidden_while_closing {
                let visible_held_ms = active
                    .clock
                    .observe(now, ComponentPhase::Hidden, visibility);
                let mut event = TimingEvent::component_lifecycle(
                    *id,
                    *scene,
                    now,
                    *component,
                    ComponentPhase::Hidden,
                    visibility,
                    active.close_reason,
                );
                event.read_ms = active.read_ms;
                event.opened_ms = active.clock.opened_ms();
                event.visible_held_ms = Some(visible_held_ms);
                event.closed_ms = Some(now);
                changed.push(event);
                closed.push((*component, *id, *scene));
                continue;
            }
            if became_readable {
                active.phase = ComponentPhase::Readable;
                active.readable_pending = false;
            }
            active.visibility = visibility;
            let visible_held_ms = active.clock.observe(now, active.phase, visibility);
            let mut event = TimingEvent::component_lifecycle(
                *id,
                *scene,
                now,
                *component,
                active.phase,
                visibility,
                if visibility == EffectiveVisibility::Visible {
                    "changed"
                } else {
                    "visibility-lost"
                },
            );
            event.read_ms = active.read_ms;
            event.opened_ms = active.clock.opened_ms();
            event.visible_held_ms = Some(visible_held_ms);
            changed.push(event);
        }
        for event in changed {
            retain(capture, event);
        }
        for key in closed {
            capture.components.remove(&key);
        }
    });
}

fn effective_visibility(element: Option<&web_sys::Element>) -> EffectiveVisibility {
    let Some(element) = element else {
        return EffectiveVisibility::Unknown;
    };
    if !element.is_connected() {
        return EffectiveVisibility::Hidden;
    }
    let Some(window) = web_sys::window() else {
        return EffectiveVisibility::Unknown;
    };
    let Some(document) = element.owner_document() else {
        return EffectiveVisibility::Unknown;
    };
    let mut opacity = 1.0_f64;
    let mut current = Some(element.clone());
    while let Some(node) = current {
        let Ok(style) = window.get_computed_style(&node) else {
            return EffectiveVisibility::Unknown;
        };
        let Some(style) = style else {
            return EffectiveVisibility::Unknown;
        };
        let display = style.get_property_value("display").unwrap_or_default();
        let visibility = style.get_property_value("visibility").unwrap_or_default();
        let content_visibility = style
            .get_property_value("content-visibility")
            .unwrap_or_default();
        if display == "none"
            || visibility == "hidden"
            || visibility == "collapse"
            || content_visibility == "hidden"
        {
            return EffectiveVisibility::Hidden;
        }
        if let Ok(value) = style.get_property_value("opacity")
            && let Ok(value) = value.parse::<f64>()
        {
            opacity *= value.clamp(0.0, 1.0);
        }
        current = node.parent_element();
    }
    if opacity < 0.05 {
        return EffectiveVisibility::Transparent;
    }
    let rect = element.get_bounding_client_rect();
    if rect.width() <= 1.0 || rect.height() <= 1.0 {
        return EffectiveVisibility::Hidden;
    }
    let (Ok(viewport_width), Ok(viewport_height)) = (window.inner_width(), window.inner_height())
    else {
        return EffectiveVisibility::Unknown;
    };
    let (Some(viewport_width), Some(viewport_height)) =
        (viewport_width.as_f64(), viewport_height.as_f64())
    else {
        return EffectiveVisibility::Unknown;
    };
    if rect.right() <= 0.0
        || rect.bottom() <= 0.0
        || rect.left() >= viewport_width
        || rect.top() >= viewport_height
    {
        return EffectiveVisibility::Offscreen;
    }
    // `pointer-events:none` is common for decorative text layers; in that case a hit-test cannot
    // distinguish a visible node from an overlay, so report computed visibility only.
    if let Ok(style) = window.get_computed_style(element)
        && style
            .and_then(|style| style.get_property_value("pointer-events").ok())
            .is_some_and(|value| value == "none")
    {
        return EffectiveVisibility::Visible;
    }
    let x = (rect.left() + rect.right()) / 2.0;
    let y = (rect.top() + rect.bottom()) / 2.0;
    let Some(hit) = document.element_from_point(x as f32, y as f32) else {
        return EffectiveVisibility::Unknown;
    };
    let mut hit = Some(hit);
    while let Some(node) = hit {
        if node.is_same_node(Some(element.as_ref())) {
            return EffectiveVisibility::Visible;
        }
        hit = node.parent_element();
    }
    EffectiveVisibility::Occluded
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
                components: HashMap::new(),
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
