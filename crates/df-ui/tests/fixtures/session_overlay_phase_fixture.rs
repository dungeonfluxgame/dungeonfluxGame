//! Synthetic presentation fixture; no session, transport, providers or audio.
#[cfg(target_arch = "wasm32")]
mod browser {
    use df_ui::{
        ActionView, ControlledAction, ControlledTextInput, DraftUpdate, FeedbackView,
        InputFeedback, SessionBookend, SessionBookendKind, SessionConnection, SessionOverlayOffer,
        SessionOverlayPhase, SessionOverlaySelection, SessionOverlayView, TextInputView,
    };
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    use wasm_bindgen::{JsCast, JsValue, prelude::wasm_bindgen};
    use web_sys::{Document, Element, HtmlButtonElement, KeyboardEvent, KeyboardEventInit, Node};

    struct Fixture {
        phase: SessionOverlayPhase,
        input: ControlledTextInput,
        controls: Vec<Rc<ControlledAction>>,
        status: Element,
        revision: u64,
        callback_count: Rc<Cell<u32>>,
        last_selection: Rc<RefCell<Option<SessionOverlaySelection>>>,
    }
    thread_local! { static FIXTURE: RefCell<Option<Fixture>> = const { RefCell::new(None) }; }
    const HOST: [SessionOverlayOffer<'static>; 2] = [
        SessionOverlayOffer {
            key: "synthetic-pause-current",
            label: "Request session pause",
            enabled: true,
            pending: false,
        },
        SessionOverlayOffer {
            key: "synthetic-end-pending",
            label: "End session · awaiting confirmation",
            enabled: true,
            pending: true,
        },
    ];
    fn error(error: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&error.to_string())
    }

    fn session_overlay_view(state: u32, revision: u64) -> SessionOverlayView<'static> {
        let connection = match state {
            1 | 2 => SessionConnection::Reconnecting,
            3 => SessionConnection::Offline,
            _ => SessionConnection::Connected,
        };
        let feedback = match state {
            0 => FeedbackView::Pending("Your supplied selection is awaiting a server outcome."),
            1 => FeedbackView::Uncertain(
                "Delivery was interrupted. This action's commit outcome remains unknown.",
            ),
            2 => FeedbackView::Error {
                uncertainty: "This action's commit outcome remains unknown.",
                message: "Receipt lookup is unavailable. No retry has been authorized.",
            },
            3 => FeedbackView::Error {
                uncertainty: "No confirmed result has been received.",
                message: "The connection is offline. Your local draft remains uncommitted.",
            },
            4 => FeedbackView::Refused(
                "The supplied selection was refused. Your draft is still editable.",
            ),
            _ => FeedbackView::Uncertain(
                "Synthetic fixture: no committed receipt or live gameplay is connected.",
            ),
        };
        let (kind, title, context, caption, attribution, still) = match state {
            6 => (
                SessionBookendKind::SpeculativeTrailer,
                "Beyond the harbor",
                "Possible future · This montage predicts no outcome and creates no campaign facts.",
                "The road may lead north, where broken pillars rise from the snow.",
                "Supplied speculative narration · Synthetic story",
                "assets/concept-art/scene-mountain-ruins-snowy-ridge-trek.webp",
            ),
            7 => (
                SessionBookendKind::CriticalCue,
                "A light below the water",
                "Cosmetic still of a supplied committed event · Essential input remains available.",
                "The lantern caught the carved archway, and the party saw the way into the sunken hall.",
                "Supplied event caption · Synthetic story",
                "assets/concept-art/scene-flooded-hall-party-wading-torchlit.webp",
            ),
            _ => (
                SessionBookendKind::Recap,
                "Under the same stars",
                "Session 04 · Authorized events supplied by the view owner",
                "You followed the lantern through the rain. Vell said the light had not gone out for twenty years. By nightfall, the road had carried you beyond Greyhaven.",
                "Vell's claim is attributed to Vell; it is not established truth. · Synthetic story",
                "assets/concept-art/scene-campfire-under-stars.webp",
            ),
        };
        SessionOverlayView {
            generation: 41,
            revision,
            title: "The Drowned Lantern",
            subtitle: "Session 04 · A moment between adventures",
            connection,
            connection_label: match connection {
                SessionConnection::Connected => "Connected · synthetic current view",
                SessionConnection::Reconnecting => {
                    "Reconnecting · recovering the current permitted view"
                }
                SessionConnection::Offline => "Offline · current view may be out of date",
                SessionConnection::Connecting => "Connecting",
            },
            connection_detail: "Connection status and action status are separate. This fixture never reconnects a real session, reloads the page or retries an action.",
            operation: Some(feedback),
            bookend: if state == 8 {
                None
            } else {
                Some(SessionBookend {
                    key: if state == 6 {
                        "synthetic-trailer"
                    } else if state == 7 {
                        "synthetic-critical"
                    } else {
                        "synthetic-recap"
                    },
                    kind,
                    title,
                    context,
                    caption,
                    attribution,
                    position: "Supplied still & captions · No audio playback or client timeline",
                    still: if state == 9 {
                        Some((
                            "assets/fixture-missing-still.webp",
                            "Still unavailable; the supplied captions remain readable",
                        ))
                    } else {
                        Some((still, "Supplied permitted story still"))
                    },
                    skip_offer: if state == 8 {
                        None
                    } else {
                        Some(SessionOverlayOffer {
                            key: "synthetic-skip-current",
                            label: "Skip this presentation",
                            enabled: true,
                            pending: false,
                        })
                    },
                })
            },
            host_context: if state == 5 {
                "Host authorization revoked in this synthetic view. No host selections are supplied."
            } else {
                "Only the current owner-advertised host selections appear below. These fixture callbacks have no gameplay effect."
            },
            host_offers: if state == 5 || connection != SessionConnection::Connected {
                &[]
            } else {
                &HOST
            },
            reduced_motion: state == 10,
        }
    }

    fn button(
        document: &Document,
        parent: &Element,
        label: &str,
        callback: impl FnMut() + 'static,
    ) -> Result<Rc<ControlledAction>, JsValue> {
        let action = Rc::new(
            ControlledAction::create(
                document,
                ActionView {
                    label,
                    enabled: true,
                    pending: false,
                },
            )
            .map_err(error)?,
        );
        action.on_activate(callback).map_err(error)?;
        parent.append_child(action.element())?;
        Ok(action)
    }

    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        if FIXTURE.with(|slot| slot.borrow().is_some()) {
            return Err(JsValue::from_str("fixture already mounted"));
        }
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| JsValue::from_str("document unavailable"))?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("body unavailable"))?;
        body.set_attribute("style", "margin:0;background:#070d14")?;
        document.set_title("DungeonFlux · Session overlays · Synthetic Rust/WASM fixture");
        let phase =
            SessionOverlayPhase::create(&document, &session_overlay_view(0, 1)).map_err(error)?;
        let fixture_bar = document.create_element("section")?;
        fixture_bar.set_attribute("style", "padding:20px;color:#f5eee0;background:#182535;font:14px/1.6 system-ui;display:flex;gap:10px;flex-wrap:wrap")?;
        let label = document.create_element("p")?;
        label.set_text_content(Some("SYNTHETIC PRESENTATION FIXTURE · Buttons below supply local test views. No server, receipt, host authority, retry, provider or audio is connected."));
        label.set_attribute("style", "width:100%;margin:0")?;
        fixture_bar.append_child(&label)?;
        let status = document.create_element("p")?;
        status.set_attribute("role", "status")?;
        status.set_attribute("data-fixture-result", "idle")?;
        status.set_attribute("style", "width:100%;margin:0")?;
        status.set_text_content(Some("Select Session details to open the nonmodal recap and host panel. Essential input stays available."));
        fixture_bar.append_child(&status)?;
        let callback_status = status.clone();
        let callback_count = Rc::new(Cell::new(0));
        let last_selection = Rc::new(RefCell::new(None));
        let counted = Rc::clone(&callback_count);
        let selected = Rc::clone(&last_selection);
        phase
            .on_selection(move |selection| {
                counted.set(counted.get() + 1);
                callback_status.set_text_content(Some(&format!(
                    "Synthetic callback only · generation {} · view {} · {}",
                    selection.generation, selection.revision, selection.key
                )));
                *selected.borrow_mut() = Some(selection);
            })
            .map_err(error)?;
        let input = ControlledTextInput::create(
            &document,
            "session-fixture-draft",
            TextInputView {
                label: "Uncommitted local draft · essential input",
                enabled: true,
                feedback: InputFeedback::None,
            },
            "Ask Vell about the lantern",
        )
        .map_err(error)?;
        input.on_change(|_| {}).map_err(error)?;
        phase.essential_input().append_child(input.root())?;
        let mut controls = Vec::new();
        for (state, label) in [
            (0, "Pending"),
            (1, "Reconnecting / unknown"),
            (2, "Unknown + error"),
            (3, "Offline"),
            (4, "Refusal"),
            (5, "Host revoked"),
            (6, "Speculative trailer"),
            (7, "Critical cue"),
            (8, "No cue"),
            (9, "Missing still"),
            (10, "Reduced motion"),
        ] {
            let failure = status.clone();
            controls.push(button(&document, &fixture_bar, label, move || {
                if synthetic_update(state).is_err() {
                    failure.set_text_content(Some("FAIL · synthetic view update failed"));
                }
            })?);
        }
        let check_status = status.clone();
        controls.push(button(&document, &fixture_bar, "Exercise mounted lifecycle", move || { match exercise() { Ok(()) => check_status.set_text_content(Some("PASS · repeated updates retained draft/input/focus; stale and replaced owners were rejected; Escape restored focus; current callbacks stayed bounded.")), Err(_) => check_status.set_text_content(Some("FAIL · mounted lifecycle check failed")) } })?);
        let focus_status = status.clone();
        controls.push(button(&document, &fixture_bar, "Exercise offered-action keyboard focus", move || {
            match action_focus_exercise() {
                Ok(()) => focus_status.set_text_content(Some("PASS · surviving keyed action focus survived reorder and label updates; removed, disabled, pending and replaced actions moved to local close navigation; essential draft focus stayed usable.")),
                Err(_) => focus_status.set_text_content(Some("FAIL · offered-action keyboard focus exercise failed")),
            }
        })?);
        let privacy_status = status.clone();
        controls.push(button(&document, &fixture_bar, "Exercise retained-node privacy cleanup", move || {
            match privacy_exercise() {
                Ok(()) => privacy_status.set_text_content(Some("PASS · retained Elements and Text nodes were scrubbed through offer/cue removal, owner replacement, explicit disposal and implicit Drop; captured buttons stayed fenced.")),
                Err(_) => privacy_status.set_text_content(Some("FAIL · retained-node privacy cleanup failed")),
            }
        })?);
        let terminal = document.create_element("p")?;
        terminal.set_attribute("hidden", "")?;
        terminal.set_attribute("role", "status")?;
        terminal.set_attribute("style", "color:#f5eee0;padding:30px;font:16px system-ui")?;
        let terminal_status = terminal.clone();
        let terminal_document = document.clone();
        controls.push(button(&document, &fixture_bar, "Dispose twice", move || {
            let result = shutdown().and_then(|()| shutdown()).and_then(|()| {
                if terminal_document.query_selector(".df-session-phase")?.is_some() { return Err(JsValue::from_str("phase remains")); }
                terminal_status.remove_attribute("hidden")?;
                terminal_status.set_attribute("data-fixture-disposal", "pass")?;
                terminal_status.set_text_content(Some("PASS · owned phase and controls disposed twice; no mounted session tree remains."));
                Ok(())
            });
            if result.is_err() { terminal_status.set_text_content(Some("FAIL · disposal did not complete")); }
        })?);
        body.append_child(&fixture_bar)?;
        body.append_child(phase.root())?;
        body.append_child(&terminal)?;
        FIXTURE.with(|slot| {
            *slot.borrow_mut() = Some(Fixture {
                phase,
                input,
                controls,
                status,
                revision: 1,
                callback_count,
                last_selection,
            })
        });
        Ok(())
    }

    #[wasm_bindgen]
    pub fn synthetic_update(state: u32) -> Result<(), JsValue> {
        if state > 10 {
            return Err(JsValue::from_str("unknown synthetic state"));
        }
        FIXTURE.with(|slot| {
            let mut owned = slot.borrow_mut();
            let fixture = owned
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
            fixture.revision += 1;
            fixture
                .phase
                .update(&session_overlay_view(state, fixture.revision))
                .map_err(error)?;
            fixture
                .input
                .update(
                    TextInputView {
                        label: "Uncommitted local draft · essential input",
                        enabled: true,
                        feedback: InputFeedback::None,
                    },
                    DraftUpdate::Preserve,
                )
                .map_err(error)?;
            fixture
                .status
                .set_attribute("data-fixture-state", &state.to_string())?;
            Ok(())
        })
    }

    #[wasm_bindgen]
    pub fn exercise() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut owned = slot.borrow_mut();
            let fixture = owned
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
            let document = web_sys::window()
                .and_then(|window| window.document())
                .ok_or_else(|| JsValue::from_str("document unavailable"))?;
            fixture.input.input().focus()?;
            let draft = fixture.input.draft();
            let input_node = fixture.input.input().clone();
            let heading = fixture
                .phase
                .root()
                .query_selector("h1")?
                .ok_or_else(|| JsValue::from_str("heading missing"))?;
            for index in 0..64 {
                fixture.revision += 1;
                fixture
                    .phase
                    .update(&session_overlay_view(index % 11, fixture.revision))
                    .map_err(error)?;
            }
            if fixture.input.draft() != draft
                || !document
                    .active_element()
                    .is_some_and(|element| element.is_same_node(Some(&input_node)))
                || !fixture
                    .phase
                    .root()
                    .query_selector("h1")?
                    .is_some_and(|element| element.is_same_node(Some(&heading)))
            {
                return Err(JsValue::from_str("node, draft or focus changed"));
            }
            let mut stale = session_overlay_view(0, fixture.revision - 1);
            if fixture.phase.update(&stale).is_ok() {
                return Err(JsValue::from_str("stale update accepted"));
            }
            stale.revision = fixture.revision + 1;
            stale.generation = 42;
            if fixture.phase.update(&stale).is_ok() {
                return Err(JsValue::from_str("foreign generation accepted"));
            }
            let open = fixture
                .phase
                .root()
                .query_selector(".session-header button")?
                .ok_or_else(|| JsValue::from_str("open missing"))?
                .dyn_into::<HtmlButtonElement>()?;
            open.focus()?;
            open.click();
            let drawer = fixture
                .phase
                .root()
                .query_selector(".session-drawer")?
                .ok_or_else(|| JsValue::from_str("drawer missing"))?;
            let event = KeyboardEventInit::new();
            event.set_key("Escape");
            event.set_bubbles(true);
            let keyboard_event: web_sys::Event =
                KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &event)?.into();
            drawer.dispatch_event(&keyboard_event)?;
            if !drawer.has_attribute("hidden")
                || !document
                    .active_element()
                    .is_some_and(|element| element.is_same_node(Some(&open)))
            {
                return Err(JsValue::from_str("keyboard close or focus restore failed"));
            }
            fixture.revision += 1;
            fixture
                .phase
                .update(&session_overlay_view(0, fixture.revision))
                .map_err(error)?;
            let host = fixture
                .phase
                .root()
                .query_selector("[data-offer-key=synthetic-pause-current]")?
                .ok_or_else(|| JsValue::from_str("host selection missing"))?
                .dyn_into::<HtmlButtonElement>()?;
            let pending = fixture
                .phase
                .root()
                .query_selector("[data-offer-key=synthetic-end-pending]")?
                .ok_or_else(|| JsValue::from_str("pending selection missing"))?
                .dyn_into::<HtmlButtonElement>()?;
            let count = fixture.callback_count.get();
            host.click();
            pending.click();
            if fixture.callback_count.get() != count + 1
                || !fixture
                    .last_selection
                    .borrow()
                    .as_ref()
                    .is_some_and(|selection| {
                        selection.revision == fixture.revision
                            && selection.key == "synthetic-pause-current"
                    })
            {
                return Err(JsValue::from_str(
                    "current selection callback duplicated or stale",
                ));
            }
            fixture.revision += 1;
            fixture
                .phase
                .update(&session_overlay_view(5, fixture.revision))
                .map_err(error)?;
            host.click();
            if fixture.callback_count.get() != count + 1 {
                return Err(JsValue::from_str("revoked selection dispatched"));
            }
            fixture.revision += 1;
            fixture
                .phase
                .update(&session_overlay_view(2, fixture.revision))
                .map_err(error)?;
            host.click();
            let unresolved = fixture
                .phase
                .root()
                .query_selector("[data-df-feedback=unresolved-error]")?
                .ok_or_else(|| JsValue::from_str("unknown error widget missing"))?;
            if fixture.callback_count.get() != count + 1
                || !unresolved.text_content().is_some_and(|text| {
                    text.contains("commit outcome remains unknown")
                        && text.contains("Receipt lookup is unavailable")
                })
            {
                return Err(JsValue::from_str(
                    "recovery erased uncertainty, error or offer fence",
                ));
            }
            fixture
                .status
                .set_attribute("data-fixture-result", "pass")?;
            Ok(())
        })
    }

    fn require_active(document: &Document, node: &Node) -> Result<(), JsValue> {
        if !document
            .active_element()
            .is_some_and(|active| active.is_same_node(Some(node)))
        {
            return Err(JsValue::from_str(
                "keyboard focus is not on the expected mounted node",
            ));
        }
        Ok(())
    }

    fn offered_button(
        phase: &SessionOverlayPhase,
        key: &str,
    ) -> Result<HtmlButtonElement, JsValue> {
        phase
            .root()
            .query_selector(&format!("[data-offer-key={key}]"))?
            .ok_or_else(|| JsValue::from_str("current offered button missing"))?
            .dyn_into::<HtmlButtonElement>()
            .map_err(JsValue::from)
    }

    /// Exercises actual browser focus on mounted action nodes, including a
    /// surviving key whose array order and supplied label change together.
    #[wasm_bindgen]
    pub fn action_focus_exercise() -> Result<(), JsValue> {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| JsValue::from_str("document unavailable"))?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("body unavailable"))?;
        let mut phase =
            SessionOverlayPhase::create(&document, &session_overlay_view(0, 1)).map_err(error)?;
        body.append_child(phase.root())?;
        let input = ControlledTextInput::create(
            &document,
            "session-focus-fixture-draft",
            TextInputView {
                label: "Synthetic essential draft",
                enabled: true,
                feedback: InputFeedback::None,
            },
            "Keep this uncommitted draft",
        )
        .map_err(error)?;
        input.on_change(|_| {}).map_err(error)?;
        phase.essential_input().append_child(input.root())?;
        let count = Rc::new(Cell::new(0));
        let last = Rc::new(RefCell::new(None::<SessionOverlaySelection>));
        let counted = Rc::clone(&count);
        let selected = Rc::clone(&last);
        phase
            .on_selection(move |selection| {
                counted.set(counted.get() + 1);
                *selected.borrow_mut() = Some(selection);
            })
            .map_err(error)?;
        let result = (|| -> Result<(), JsValue> {
            let open = phase
                .root()
                .query_selector(".session-header button")?
                .ok_or_else(|| JsValue::from_str("open navigation missing"))?
                .dyn_into::<HtmlButtonElement>()?;
            let close = phase
                .root()
                .query_selector(".session-drawer>button")?
                .ok_or_else(|| JsValue::from_str("close navigation missing"))?
                .dyn_into::<HtmlButtonElement>()?;
            open.focus()?;
            open.click();
            let host = offered_button(&phase, "synthetic-pause-current")?;
            host.focus()?;
            require_active(&document, host.as_ref())?;
            let old_label = host
                .first_child()
                .ok_or_else(|| JsValue::from_str("host label Text missing"))?;
            let reordered = [
                HOST[1],
                SessionOverlayOffer {
                    label: "Request pause · current view label",
                    ..HOST[0]
                },
            ];
            let current = SessionOverlayView {
                host_offers: &reordered,
                ..session_overlay_view(0, 2)
            };
            phase.update(&current).map_err(error)?;
            require_active(&document, host.as_ref())?;
            if !offered_button(&phase, "synthetic-pause-current")?.is_same_node(Some(&host))
                || host.text_content().as_deref() != Some("Request pause · current view label")
                || old_label.node_value().is_some_and(|text| !text.is_empty())
            {
                return Err(JsValue::from_str(
                    "keyed focus, current label or old Text cleanup failed",
                ));
            }
            host.click();
            if count.get() != 1
                || !last.borrow().as_ref().is_some_and(|selection| {
                    selection.revision == 2 && selection.key == "synthetic-pause-current"
                })
            {
                return Err(JsValue::from_str(
                    "focused action did not deliver exactly the current selection",
                ));
            }
            phase.update(&session_overlay_view(0, 3)).map_err(error)?;
            require_active(&document, host.as_ref())?;
            let disabled = [
                SessionOverlayOffer {
                    enabled: false,
                    ..HOST[0]
                },
                HOST[1],
            ];
            phase
                .update(&SessionOverlayView {
                    host_offers: &disabled,
                    ..session_overlay_view(0, 4)
                })
                .map_err(error)?;
            require_active(&document, close.as_ref())?;
            host.click();
            if !host.disabled() || count.get() != 1 {
                return Err(JsValue::from_str("disabled focused action was not fenced"));
            }
            phase.update(&session_overlay_view(0, 5)).map_err(error)?;
            require_active(&document, close.as_ref())?;
            host.focus()?;
            let pending = [
                SessionOverlayOffer {
                    pending: true,
                    ..HOST[0]
                },
                HOST[1],
            ];
            phase
                .update(&SessionOverlayView {
                    host_offers: &pending,
                    ..session_overlay_view(0, 6)
                })
                .map_err(error)?;
            require_active(&document, close.as_ref())?;
            host.click();
            if !host.disabled() || count.get() != 1 {
                return Err(JsValue::from_str("pending focused action was not fenced"));
            }
            phase.update(&session_overlay_view(0, 7)).map_err(error)?;
            host.focus()?;
            let removed_label = host
                .first_child()
                .ok_or_else(|| JsValue::from_str("restored host label missing"))?;
            phase.update(&session_overlay_view(5, 8)).map_err(error)?;
            require_active(&document, close.as_ref())?;
            host.click();
            if !host.disabled()
                || host.has_attribute("data-offer-key")
                || host.text_content().is_some_and(|text| !text.is_empty())
                || removed_label
                    .node_value()
                    .is_some_and(|text| !text.is_empty())
                || count.get() != 1
            {
                return Err(JsValue::from_str(
                    "removed focused action retained payload or delivered a selection",
                ));
            }
            phase.update(&session_overlay_view(0, 9)).map_err(error)?;
            host.focus()?;
            let replacement = [
                SessionOverlayOffer {
                    key: "synthetic-new-current",
                    label: "A different current selection",
                    enabled: true,
                    pending: false,
                },
                HOST[1],
            ];
            phase
                .update(&SessionOverlayView {
                    host_offers: &replacement,
                    ..session_overlay_view(0, 10)
                })
                .map_err(error)?;
            require_active(&document, close.as_ref())?;
            if offered_button(&phase, "synthetic-new-current")?.disabled() {
                return Err(JsValue::from_str(
                    "replacement selection is unexpectedly disabled",
                ));
            }
            phase.update(&session_overlay_view(0, 11)).map_err(error)?;
            let recovered_host = offered_button(&phase, "synthetic-pause-current")?;
            recovered_host.focus()?;
            phase.update(&session_overlay_view(2, 12)).map_err(error)?;
            require_active(&document, close.as_ref())?;
            recovered_host.click();
            if count.get() != 1 {
                return Err(JsValue::from_str(
                    "reconnecting focused selection was not fenced",
                ));
            }
            phase.update(&session_overlay_view(0, 13)).map_err(error)?;
            let skip = offered_button(&phase, "synthetic-skip-current")?;
            skip.focus()?;
            phase.update(&session_overlay_view(8, 14)).map_err(error)?;
            require_active(&document, close.as_ref())?;
            skip.click();
            if !skip.disabled() || count.get() != 1 {
                return Err(JsValue::from_str("removed focused skip was not fenced"));
            }
            input.input().focus()?;
            for (revision, state) in [(15, 0), (16, 2), (17, 5)] {
                phase
                    .update(&session_overlay_view(state, revision))
                    .map_err(error)?;
                require_active(&document, input.input().as_ref())?;
                if input.draft() != "Keep this uncommitted draft"
                    || input.input().value() != "Keep this uncommitted draft"
                {
                    return Err(JsValue::from_str(
                        "essential draft changed during action reconciliation",
                    ));
                }
            }
            Ok(())
        })();
        input.dispose().map_err(error)?;
        phase.dispose().map_err(error)?;
        result
    }

    struct HeldView {
        elements: Vec<Element>,
        texts: Vec<Node>,
        image: Element,
        bookend: Element,
        offers: Vec<HtmlButtonElement>,
        root: Element,
    }

    fn hold_view(phase: &SessionOverlayPhase) -> Result<HeldView, JsValue> {
        let mut elements = Vec::new();
        let mut texts = Vec::new();
        for selector in [
            "h1",
            ".session-subtitle",
            ".connection-label",
            ".connection-detail",
            ".bookend-kind",
            ".bookend-copy h2",
            ".bookend-context",
            ".bookend-caption",
            ".bookend-attribution",
            ".bookend-position",
            ".session-host p",
            ".df-ui-panel [role=status]",
        ] {
            let element = phase
                .root()
                .query_selector(selector)?
                .ok_or_else(|| JsValue::from_str("owned prose node missing"))?;
            if let Some(text) = element.first_child() {
                texts.push(text);
            }
            elements.push(element);
        }
        let mut offers = Vec::new();
        for key in [
            "synthetic-pause-current",
            "synthetic-end-pending",
            "synthetic-skip-current",
        ] {
            let element = phase
                .root()
                .query_selector(&format!("[data-offer-key={key}]"))?
                .ok_or_else(|| JsValue::from_str("owned selection node missing"))?
                .dyn_into::<HtmlButtonElement>()?;
            if let Some(text) = element.first_child() {
                texts.push(text);
            }
            offers.push(element);
        }
        Ok(HeldView {
            elements,
            texts,
            offers,
            image: phase
                .root()
                .query_selector("img")?
                .ok_or_else(|| JsValue::from_str("image missing"))?,
            bookend: phase
                .root()
                .query_selector(".session-bookend")?
                .ok_or_else(|| JsValue::from_str("bookend missing"))?,
            root: phase.root().clone(),
        })
    }

    fn require_scrubbed(held: &HeldView) -> Result<(), JsValue> {
        if held
            .elements
            .iter()
            .any(|element| element.text_content().is_some_and(|text| !text.is_empty()))
            || held
                .texts
                .iter()
                .any(|node| node.node_value().is_some_and(|text| !text.is_empty()))
            || held.offers.iter().any(|offer| {
                offer.text_content().is_some_and(|text| !text.is_empty())
                    || offer.has_attribute("data-offer-key")
                    || !offer.disabled()
            })
            || held.image.has_attribute("src")
            || held.image.has_attribute("alt")
            || held.bookend.has_attribute("data-cue-key")
            || held.root.has_attribute("data-view-revision")
            || held.root.has_attribute("data-session-connection")
        {
            return Err(JsValue::from_str(
                "retained owned node still contains obsolete presentation",
            ));
        }
        Ok(())
    }

    /// Holds actual DOM Elements and raw Text nodes while changing the mounted
    /// scope. Detaching the root alone cannot satisfy these assertions.
    #[wasm_bindgen]
    pub fn privacy_exercise() -> Result<(), JsValue> {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| JsValue::from_str("document unavailable"))?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("body unavailable"))?;
        let mut phase =
            SessionOverlayPhase::create(&document, &session_overlay_view(0, 1)).map_err(error)?;
        body.append_child(phase.root())?;
        let callbacks = Rc::new(Cell::new(0));
        let counted = Rc::clone(&callbacks);
        phase
            .on_selection(move |_| counted.set(counted.get() + 1))
            .map_err(error)?;
        let removed = hold_view(&phase)?;
        phase.update(&session_overlay_view(8, 2)).map_err(error)?;
        phase.update(&session_overlay_view(5, 3)).map_err(error)?;
        // Former cue and host selections are now absent, including a previously
        // pending button and its retained label Text. Skip is still a current cue.
        if removed
            .texts
            .iter()
            .any(|node| node.node_value().is_some_and(|text| !text.is_empty()))
            || removed.offers.iter().take(2).any(|offer| {
                offer.text_content().is_some_and(|text| !text.is_empty())
                    || offer.has_attribute("data-offer-key")
            })
        {
            return Err(JsValue::from_str(
                "removed selection retained prose or label Text",
            ));
        }
        phase.update(&session_overlay_view(8, 4)).map_err(error)?;
        if removed.image.has_attribute("src")
            || removed.image.has_attribute("alt")
            || !removed.image.has_attribute("hidden")
            || removed.bookend.has_attribute("data-cue-key")
        {
            return Err(JsValue::from_str("removed cue retained asset attributes"));
        }
        let mut no_still = session_overlay_view(0, 5);
        if let Some(bookend) = no_still.bookend.as_mut() {
            bookend.still = None;
        }
        phase.update(&no_still).map_err(error)?;
        if removed.image.has_attribute("src") || removed.image.has_attribute("alt") {
            return Err(JsValue::from_str(
                "no-still view retained previous alt or source",
            ));
        }
        phase.update(&session_overlay_view(0, 6)).map_err(error)?;
        let disposed = hold_view(&phase)?;
        phase.dispose().map_err(error)?;
        phase.dispose().map_err(error)?;
        let mut replacement_view = session_overlay_view(0, 1);
        replacement_view.generation = 42;
        let mut replacement =
            SessionOverlayPhase::create(&document, &replacement_view).map_err(error)?;
        body.append_child(replacement.root())?;
        require_scrubbed(&disposed)?;
        for button in &disposed.offers {
            button.click();
        }
        if callbacks.get() != 0 || phase.update(&session_overlay_view(0, 7)).is_ok() {
            return Err(JsValue::from_str(
                "disposed owner accepted a captured selection or view",
            ));
        }
        replacement.dispose().map_err(error)?;
        let dropped = {
            let phase = SessionOverlayPhase::create(&document, &session_overlay_view(0, 1))
                .map_err(error)?;
            body.append_child(phase.root())?;
            let held = hold_view(&phase)?;
            drop(phase);
            held
        };
        require_scrubbed(&dropped)?;
        Ok(())
    }

    #[wasm_bindgen]
    pub fn shutdown() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut owned = slot.borrow_mut();
            if let Some(fixture) = owned.as_mut() {
                fixture.input.dispose().map_err(error)?;
                for control in &fixture.controls {
                    control.dispose().map_err(error)?;
                }
                fixture.phase.dispose().map_err(error)?;
                fixture.phase.dispose().map_err(error)?;
                if fixture
                    .phase
                    .update(&session_overlay_view(0, fixture.revision + 1))
                    .is_ok()
                {
                    return Err(JsValue::from_str("disposed update accepted"));
                }
            }
            *owned = None;
            Ok(())
        })
    }
}
