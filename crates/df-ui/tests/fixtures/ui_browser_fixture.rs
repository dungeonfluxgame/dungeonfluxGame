#[cfg(target_arch = "wasm32")]
mod browser {
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    use df_ui::{LayoutRole, LayoutRoot, UiError, action_button, panel, text_input};
    use wasm_bindgen::{JsCast, JsValue, closure::Closure, prelude::wasm_bindgen};
    use web_sys::{Document, Element, Event, HtmlButtonElement, HtmlInputElement};

    struct Click {
        button: HtmlButtonElement,
        callback: Closure<dyn FnMut(Event)>,
    }

    impl Click {
        fn bind(
            button: HtmlButtonElement,
            callback: impl FnMut(Event) + 'static,
        ) -> Result<Self, JsValue> {
            let callback = Closure::wrap(Box::new(callback) as Box<dyn FnMut(Event)>);
            button.add_event_listener_with_callback("click", callback.as_ref().unchecked_ref())?;
            Ok(Self { button, callback })
        }

        fn dispose(&self) -> Result<(), JsValue> {
            self.button.remove_event_listener_with_callback(
                "click",
                self.callback.as_ref().unchecked_ref(),
            )
        }
    }

    // Empty after successful disposal; repeated unmount/shutdown are harmless.
    fn dispose_layout_controls(controls: &RefCell<Vec<Click>>) -> Result<(), JsValue> {
        let mut owned = controls.borrow_mut();
        for control in owned.iter() {
            control.dispose()?;
        }
        owned.clear();
        Ok(())
    }

    struct Fixture {
        root: Element,
        player: Rc<LayoutRoot>,
        display: Rc<LayoutRoot>,
        controls: Vec<Click>,
        layout_controls: Rc<RefCell<Vec<Click>>>,
        player_input: HtmlInputElement,
        display_input: HtmlInputElement,
        player_count: Rc<Cell<u32>>,
        display_count: Rc<Cell<u32>>,
        disabled_count: Rc<Cell<u32>>,
    }

    thread_local! {
        static FIXTURE: RefCell<Option<Fixture>> = const { RefCell::new(None) };
    }

    fn error(error: UiError) -> JsValue {
        JsValue::from_str(&error.to_string())
    }

    fn build_role(
        document: &Document,
        role: LayoutRole,
        name: &str,
    ) -> Result<
        (
            Rc<LayoutRoot>,
            HtmlInputElement,
            HtmlButtonElement,
            HtmlButtonElement,
        ),
        UiError,
    > {
        let layout = Rc::new(LayoutRoot::create(document, role)?);
        layout
            .root()
            .set_class_name("df-ui-root df-accessibility-motion");
        let content = panel(document, name)?;
        let (field, input) = text_input(document, &format!("{name} draft"))?;
        content.append_child(&field)?;
        let native_action = action_button(document, "Native keyboard action", true)?;
        let unavailable_action = action_button(document, "Unavailable action", false)?;
        native_action.set_attribute("data-fixture-native-action", name)?;
        unavailable_action.set_attribute("data-fixture-disabled-action", name)?;
        content.append_child(&native_action)?;
        content.append_child(&unavailable_action)?;
        layout.content().append_child(&content)?;
        let short_action = action_button(document, "Go", true)?;
        short_action.set_class_name("df-ui-action df-accessibility-short");
        short_action.set_attribute("style", "padding:0;width:1px")?;
        content.append_child(&short_action)?;
        let motion_style = document.create_element("style")?;
        motion_style.set_text_content(Some(
            ".df-accessibility-motion{transition:opacity 2s linear;animation:df-accessibility-pulse 2s linear infinite;scroll-behavior:smooth}.df-accessibility-motion::before,.df-accessibility-motion::after{content:'.';transition:opacity 2s linear;animation:df-accessibility-pulse 2s linear infinite}@keyframes df-accessibility-pulse{from{opacity:.95}to{opacity:1}}",
        ));
        content.append_child(&motion_style)?;
        let motion = document.create_element("p")?;
        motion.set_class_name("df-accessibility-motion");
        motion.set_text_content(Some("Motion changes keep this essential text visible."));
        content.append_child(&motion)?;
        Ok((layout, input, native_action, unavailable_action))
    }

    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        if FIXTURE.with(|slot| slot.borrow().is_some()) {
            return Err(JsValue::from_str("fixture already mounted"));
        }
        let window =
            web_sys::window().ok_or_else(|| JsValue::from_str("browser window missing"))?;
        let document = window
            .document()
            .ok_or_else(|| JsValue::from_str("browser document missing"))?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("browser body missing"))?;
        let root = document.create_element("main")?;
        let heading = document.create_element("h1")?;
        heading.set_text_content(Some("DungeonFlux shared UI foundation fixture"));
        root.append_child(&heading)?;
        let notice = document.create_element("p")?;
        notice.set_text_content(Some(
            "Synthetic presentation fixture. Server authorization and gameplay are untested.",
        ));
        root.append_child(&notice)?;
        let status = document.create_element("p")?;
        status.set_attribute("role", "status")?;
        let failures_pass = matches!(panel(&document, " "), Err(UiError::EmptyLabel))
            && matches!(action_button(&document, "", true), Err(UiError::EmptyLabel))
            && matches!(text_input(&document, "\n"), Err(UiError::EmptyLabel));
        if !failures_pass {
            return Err(JsValue::from_str("empty-label rejection fixture failed"));
        }
        status.set_text_content(Some("Empty labels rejected. Ready for browser operation."));
        root.append_child(&status)?;
        let (player, player_input, player_action, player_disabled) =
            build_role(&document, LayoutRole::Player, "Player").map_err(error)?;
        let (display, display_input, display_action, display_disabled) =
            build_role(&document, LayoutRole::SharedDisplay, "Shared display").map_err(error)?;
        let literal_heading = panel(&document, "<img src=x onerror=alert(1)>").map_err(error)?;
        display.content().append_child(&literal_heading)?;
        let long_heading = panel(&document, "A deliberately long presentation heading keeps every word readable when this shared foundation is viewed on a narrow screen").map_err(error)?;
        display.content().append_child(&long_heading)?;
        let motion_sample =
            action_button(&document, "Reduced motion sample", true).map_err(error)?;
        motion_sample.set_attribute("style", "transition: opacity 2s linear")?;
        display.content().append_child(&motion_sample)?;
        let update = action_button(&document, "Update layout and preserve draft focus", true)
            .map_err(error)?;
        let remove = action_button(&document, "Unmount layouts", true).map_err(error)?;
        root.append_child(&update)?;
        root.append_child(&remove)?;
        root.append_child(player.root())?;
        root.append_child(display.root())?;
        let player_count = Rc::new(Cell::new(0_u32));
        let display_count = Rc::new(Cell::new(0_u32));
        let disabled_count = Rc::new(Cell::new(0_u32));
        let mut layout_controls = Vec::new();
        for (button, count, name) in [
            (player_action, Rc::clone(&player_count), "Player"),
            (display_action, Rc::clone(&display_count), "Shared display"),
        ] {
            let native_status = status.clone();
            layout_controls.push(Click::bind(button, move |_| {
                count.set(count.get().saturating_add(1));
                native_status.set_text_content(Some(&format!(
                    "{name} native action activations: {}",
                    count.get()
                )));
            })?);
        }
        for button in [player_disabled, display_disabled] {
            let count = Rc::clone(&disabled_count);
            layout_controls.push(Click::bind(button, move |_| {
                count.set(count.get().saturating_add(1));
            })?);
        }
        let update_input = player_input.clone();
        let update_player = Rc::clone(&player);
        let update_display = Rc::clone(&display);
        let update_status = status.clone();
        let update_document = document.clone();
        let update_click = Click::bind(update, move |_| {
            let result = (|| -> Result<(), UiError> {
                update_input.focus()?;
                let active_before = update_document.active_element();
                let draft_before = update_input.value();
                update_player.set_role(LayoutRole::SharedDisplay)?;
                update_display.set_role(LayoutRole::Player)?;
                update_player.set_role(LayoutRole::Player)?;
                update_display.set_role(LayoutRole::SharedDisplay)?;
                let focused = active_before.as_ref().is_some_and(|before| {
                    update_document
                        .active_element()
                        .as_ref()
                        .is_some_and(|after| before.is_same_node(Some(after)))
                });
                if !focused || update_input.value() != draft_before {
                    return Err(UiError::Browser(JsValue::from_str(
                        "focus or draft changed during update",
                    )));
                }
                Ok(())
            })();
            match result {
                Ok(()) => update_status.set_text_content(Some(
                    "Update passed: same focused input and draft retained.",
                )),
                Err(error) => {
                    update_status.set_text_content(Some(&format!("Update failed: {error}")))
                }
            }
        })?;
        let layout_controls = Rc::new(RefCell::new(layout_controls));
        let remove_controls = Rc::clone(&layout_controls);
        let remove_counts = [
            Rc::clone(&player_count),
            Rc::clone(&display_count),
            Rc::clone(&disabled_count),
        ];
        let remove_player = Rc::clone(&player);
        let remove_display = Rc::clone(&display);
        let remove_status = status;
        let remove_click = Click::bind(remove, move |_| {
            let result = (|| -> Result<(), UiError> {
                let detached_buttons: Vec<_> = remove_controls
                    .borrow()
                    .iter()
                    .map(|control| control.button.clone())
                    .collect();
                let counts_before = remove_counts.each_ref().map(|count| count.get());
                let status_before = remove_status.text_content();
                dispose_layout_controls(&remove_controls)?;
                remove_player.unmount()?;
                remove_display.unmount()?;
                remove_player.unmount()?;
                remove_display.unmount()?;
                // Teardown-only probe, separate from trusted keyboard/pointer checks.
                // Enable detached disabled controls to test removed listeners rather
                // than relying on native disabled-click suppression.
                for button in detached_buttons {
                    if button.is_connected() {
                        return Err(UiError::Browser(JsValue::from_str(
                            "layout button remained connected after unmount",
                        )));
                    }
                    let disabled = button.disabled();
                    button.set_disabled(false);
                    button.click();
                    button.set_disabled(disabled);
                }
                if remove_counts.each_ref().map(|count| count.get()) != counts_before
                    || remove_status.text_content() != status_before
                {
                    return Err(UiError::Browser(JsValue::from_str(
                        "detached layout button retained an action listener",
                    )));
                }
                Ok(())
            })();
            match result {
                Ok(()) => remove_status.set_text_content(Some(
                    "Both layouts unmounted twice; detached actions inert and scoped resources disposed.",
                )),
                Err(error) => {
                    remove_status.set_text_content(Some(&format!("Unmount failed: {error}")))
                }
            }
        })?;
        let controls = vec![update_click, remove_click];
        body.append_child(&root)?;
        FIXTURE.with(|slot| {
            *slot.borrow_mut() = Some(Fixture {
                root,
                player,
                display,
                controls,
                layout_controls,
                player_input,
                display_input,
                player_count,
                display_count,
                disabled_count,
            });
        });
        Ok(())
    }

    // Fixed browser APIs use the existing js-sys dependency; no manifest or dynamic
    // server script is introduced. Values describe observed computed presentation.
    fn computed_property(
        element: &Element,
        pseudo: Option<&str>,
        property: &str,
    ) -> Result<String, JsValue> {
        let window = web_sys::window().ok_or_else(|| JsValue::from_str("window unavailable"))?;
        let function =
            js_sys::Reflect::get(window.as_ref(), &JsValue::from_str("getComputedStyle"))?
                .dyn_into::<js_sys::Function>()?;
        let pseudo = pseudo.map(JsValue::from_str).unwrap_or(JsValue::NULL);
        let styles = function.call2(window.as_ref(), element.as_ref(), &pseudo)?;
        js_sys::Reflect::get(&styles, &JsValue::from_str(property))?
            .as_string()
            .ok_or_else(|| JsValue::from_str("computed property unavailable"))
    }

    fn os_reduces_motion() -> Result<bool, JsValue> {
        let window = web_sys::window().ok_or_else(|| JsValue::from_str("window unavailable"))?;
        let function = js_sys::Reflect::get(window.as_ref(), &JsValue::from_str("matchMedia"))?
            .dyn_into::<js_sys::Function>()?;
        let query = function.call1(
            window.as_ref(),
            &JsValue::from_str("(prefers-reduced-motion: reduce)"),
        )?;
        js_sys::Reflect::get(&query, &JsValue::from_str("matches"))?
            .as_bool()
            .ok_or_else(|| JsValue::from_str("motion media preference unavailable"))
    }

    fn require_motion(sample: &Element, reduced: bool) -> Result<(), JsValue> {
        for pseudo in [None, Some("::before"), Some("::after")] {
            let duration = computed_property(sample, pseudo, "transitionDuration")?;
            let animation = computed_property(sample, pseudo, "animationName")?;
            let expected_duration = if reduced { "0s" } else { "2s" };
            let expected_animation = if reduced {
                "none"
            } else {
                "df-accessibility-pulse"
            };
            if duration != expected_duration || animation != expected_animation {
                return Err(JsValue::from_str(
                    "motion preference did not control computed motion",
                ));
            }
        }
        let expected_scroll = if reduced { "auto" } else { "smooth" };
        if computed_property(sample, None, "scrollBehavior")? != expected_scroll {
            return Err(JsValue::from_str(
                "motion preference did not control smooth scrolling",
            ));
        }
        Ok(())
    }

    /// Tests mounted presentation directly. This does not claim real keyboard or
    /// physical touch operation; the independent evaluator must perform those separately.
    #[wasm_bindgen]
    pub fn accessibility_exercise() -> Result<String, JsValue> {
        FIXTURE.with(|slot| {
            let owned = slot.borrow();
            let fixture = owned.as_ref().ok_or_else(|| JsValue::from_str("fixture absent"))?;
            let document = fixture.root.owner_document().ok_or_else(|| JsValue::from_str("document absent"))?;
            let os_reduced = os_reduces_motion()?;
            for (layout, input, role) in [
                (&fixture.player, &fixture.player_input, LayoutRole::Player),
                (&fixture.display, &fixture.display_input, LayoutRole::SharedDisplay),
            ] {
                if !layout.root().is_connected() {
                    return Err(JsValue::from_str("layout has been unmounted"));
                }
                let sample = layout.root().query_selector(".df-accessibility-motion")?
                    .ok_or_else(|| JsValue::from_str("motion sample absent"))?;
                let short = layout.root().query_selector(".df-accessibility-short")?
                    .ok_or_else(|| JsValue::from_str("short action absent"))?
                    .dyn_into::<HtmlButtonElement>()?;
                if short.offset_width() < 44 || short.offset_height() < 44 {
                    return Err(JsValue::from_str("action target fell below 44px in one dimension"));
                }
                let field = input.parent_element().ok_or_else(|| JsValue::from_str("input label absent"))?;
                if field.tag_name() != "LABEL" || !field.contains(Some(input)) {
                    return Err(JsValue::from_str("native input label association lost"));
                }
                input.focus()?;
                let draft = input.value();
                let selection = (input.selection_start()?, input.selection_end()?);
                for _ in 0..16 {
                    layout.set_role(role).map_err(error)?;
                    layout.set_reduced_motion(true).map_err(error)?;
                    require_motion(&sample, true)?;
                    require_motion(layout.root(), true)?;
                    layout.set_reduced_motion(false).map_err(error)?;
                    require_motion(&sample, os_reduced)?;
                    require_motion(layout.root(), os_reduced)?;
                    if !document.active_element().is_some_and(|active| active.is_same_node(Some(input)))
                        || input.value() != draft
                        || (input.selection_start()?, input.selection_end()?) != selection
                    {
                        return Err(JsValue::from_str("accessibility update changed focused draft or selection"));
                    }
                }
                if sample.text_content().as_deref() != Some("Motion changes keep this essential text visible.") {
                    return Err(JsValue::from_str("motion preference removed essential text"));
                }
            }
            Ok(format!("PASS mounted layout targets, native labels, draft/focus/selection and additive motion; OS reduced={os_reduced}. Real keyboard/pointer/device checks remain separate."))
        })
    }

    /// Reflects actual native click delivery for independent keyboard/pointer operation.
    #[wasm_bindgen]
    pub fn native_action_counts() -> Result<String, JsValue> {
        FIXTURE.with(|slot| {
            let owned = slot.borrow();
            let fixture = owned
                .as_ref()
                .ok_or_else(|| JsValue::from_str("fixture absent"))?;
            Ok(format!(
                "player={} display={} disabled={}",
                fixture.player_count.get(),
                fixture.display_count.get(),
                fixture.disabled_count.get()
            ))
        })
    }

    #[wasm_bindgen]
    pub fn shutdown() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut owned = slot.borrow_mut();
            if let Some(fixture) = owned.as_ref() {
                dispose_layout_controls(&fixture.layout_controls)?;
                for control in &fixture.controls {
                    control.dispose()?;
                }
                fixture.player.unmount().map_err(error)?;
                fixture.display.unmount().map_err(error)?;
                if let Some(parent) = fixture.root.parent_node() {
                    parent.remove_child(&fixture.root)?;
                }
            }
            *owned = None;
            Ok(())
        })
    }
}
