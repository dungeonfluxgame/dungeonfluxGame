//! Synthetic design/contract fixture. No gameplay session, rules catalog, provider
//! or character-creation service is connected. Status controls simulate view updates.

#[cfg(target_arch = "wasm32")]
mod browser {
    use df_ui::{
        ActionView, CharacterAction, CharacterActionKind, CharacterAppearanceDraft, CharacterFact,
        CharacterGroup, CharacterLimits, CharacterOption, CharacterPhaseSurface,
        CharacterPhaseView, CharacterPortrait, CharacterStatus, ControlledAction,
    };
    use std::{
        cell::{Cell, RefCell},
        rc::{Rc, Weak},
    };
    use wasm_bindgen::prelude::*;
    use web_sys::{Document, Element, Event, Node};

    struct Fixture {
        surface: Rc<CharacterPhaseSurface>,
        controls: Vec<ControlledAction>,
    }
    thread_local! { static FIXTURE: RefCell<Option<Fixture>> = const { RefCell::new(None) }; }
    fn error(error: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&error.to_string())
    }
    fn child(
        document: &Document,
        parent: &Element,
        tag: &str,
        class: &str,
        text: &str,
    ) -> Result<Element, JsValue> {
        let node = document.create_element(tag)?;
        node.set_class_name(class);
        node.set_text_content(Some(text));
        parent.append_child(&node)?;
        Ok(node)
    }
    fn limits() -> CharacterLimits {
        CharacterLimits {
            max_groups: 16,
            max_options: 128,
            max_facts: 128,
            max_actions: 16,
            max_text_bytes: 4096,
        }
    }
    fn option(
        id: &str,
        label: &str,
        description: &str,
        portrait: CharacterPortrait,
    ) -> CharacterOption {
        CharacterOption {
            id: id.into(),
            label: label.into(),
            description: description.into(),
            availability: "Synthetic offered option · no rules legality claim".into(),
            enabled: true,
            portrait: Some(portrait),
        }
    }
    fn character_view() -> CharacterPhaseView {
        CharacterPhaseView {
            generation: 1, owner_key: "synthetic-player-one".into(), revision: 1,
            chapter: "Prologue · Character creation".into(), title: "Who will you become?".into(),
            description: "A name, a memory, a spark of courage. Every great adventure starts with a story waiting to be told.".into(),
            connection: "Design fixture · Synthetic offers and server states · No live gameplay session or rules catalog connected".into(),
            status: CharacterStatus::Editing,
            status_message: "Review these synthetic choices and draft your identity. Sending input does not create a character.".into(),
            editable: true, name: "Mara".into(), flavor: "Carries a lantern that never quite goes out".into(),
            appearance: Some(CharacterAppearanceDraft { features: "Freckled cheeks and a silver braid".into(), outfit: "A weathered green cloak over travel clothes".into() }),
            portrait: Some(CharacterPortrait::Narrator),
            groups: vec![
                CharacterGroup { id: "story-calling".into(), label: "Choose a story calling".into(),
                    description: "A design example of reviewed options. Production choices come from the server's pinned catalog.".into(),
                    selected: Some("lantern-keeper".into()), options: vec![
                        option("lantern-keeper", "Lantern keeper", "You know the roads that disappear when the sun goes down. Someone always needs a light to follow.", CharacterPortrait::Narrator),
                        option("harbor-witness", "Harbor witness", "You have listened to a hundred stories over rain and candlelight. One of them is about to become your own.", CharacterPortrait::Vell),
                    ] },
                CharacterGroup { id: "story-memory".into(), label: "A memory that follows you".into(),
                    description: "Flavor options illustrate selection and offer ownership. They do not grant mechanics or calculate statistics.".into(),
                    selected: None, options: vec![
                        option("unopened-letter", "The unopened letter", "A letter from someone you thought you had lost. The seal is unbroken; the journey has already begun.", CharacterPortrait::Narrator),
                        option("old-promise", "An old promise", "You promised you would return before the first winter snow. You still remember the way home.", CharacterPortrait::Vell),
                    ] },
            ],
            facts: vec![
                CharacterFact { label: "View source".into(), value: "Synthetic fixture".into() },
                CharacterFact { label: "Build validation".into(), value: "No rules service connected".into() },
                CharacterFact { label: "Portrait".into(), value: "AI-generated v2 concept-inspired portrait".into() },
            ],
            actions: vec![CharacterAction { id: "fixture-review-draft".into(), kind: CharacterActionKind::SubmitDraft, label: "Send draft · fixture only".into(), enabled: true }],
        }
    }
    fn report(output: &Element, result: Result<(), JsValue>, success: &str) {
        output.set_text_content(Some(if result.is_ok() {
            success
        } else {
            "FAIL · presentation check failed; no gameplay action was completed."
        }));
    }

    fn retained_private_nodes(root: &Element) -> Result<Vec<Node>, JsValue> {
        let mut retained = Vec::new();
        for selector in [
            ".character-hero .character-overline",
            "h1",
            ".character-description",
            ".character-connection",
            ".character-message",
            ".character-status",
            ".character-facts",
            ".character-groups",
            ".character-actions",
            ".df-ui-field",
            ".identity-panel .df-ui-field:last-child",
            ".character-appearance",
        ] {
            if let Some(element) = root.query_selector(selector)? {
                retain_descendants(element.as_ref(), &mut retained);
            }
        }
        Ok(retained)
    }
    fn retain_descendants(node: &Node, retained: &mut Vec<Node>) {
        retained.push(node.clone());
        let mut child = node.first_child();
        while let Some(descendant) = child {
            child = descendant.next_sibling();
            retain_descendants(&descendant, retained);
        }
    }
    fn private_view() -> CharacterPhaseView {
        let mut data = character_view();
        data.chapter = "Private synthetic chapter".into();
        data.title = "Private synthetic title".into();
        data.description = "Private synthetic description".into();
        data.connection = "Private synthetic connection".into();
        data.status = CharacterStatus::Rejected;
        data.status_message = "Private synthetic feedback".into();
        data.name = "Private synthetic name".into();
        data.flavor = "Private synthetic flavor".into();
        data.appearance = Some(CharacterAppearanceDraft {
            features: "Private synthetic features".into(),
            outfit: "Private synthetic outfit".into(),
        });
        data
    }
    fn check_cleanup(document: &Document, implicit: bool) -> Result<(), JsValue> {
        let deliveries = Rc::new(Cell::new(0));
        let recorded = Rc::clone(&deliveries);
        let surface = CharacterPhaseSurface::create(
            document,
            "retention-check",
            &private_view(),
            limits(),
            move |_| recorded.set(recorded.get() + 1),
        )
        .map_err(error)?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("body unavailable"))?;
        body.append_child(surface.root())?;
        let root = surface.root().clone();
        let retained = retained_private_nodes(&root)?;
        let name = surface.name_input().input().clone();
        let flavor = surface.flavor_input().input().clone();
        let features = surface
            .features_input()
            .ok_or_else(|| JsValue::from_str("features missing"))?
            .input()
            .clone();
        let outfit = surface
            .outfit_input()
            .ok_or_else(|| JsValue::from_str("outfit missing"))?
            .input()
            .clone();
        let button = root
            .query_selector(".character-actions button")?
            .ok_or_else(|| JsValue::from_str("action missing"))?;
        if implicit {
            drop(surface);
        } else {
            surface.dispose().map_err(error)?;
            surface.dispose().map_err(error)?;
            drop(surface);
        }
        if root.is_connected()
            || !name.value().is_empty()
            || !flavor.value().is_empty()
            || !features.value().is_empty()
            || !outfit.value().is_empty()
            || retained.iter().any(|node| {
                node.text_content().is_some_and(|text| !text.is_empty())
                    || node.node_value().is_some_and(|text| !text.is_empty())
            })
        {
            return Err(JsValue::from_str(
                "retained DOM element/Text or input value survived cleanup",
            ));
        }
        button.dispatch_event(&Event::new("click")?)?;
        name.set_value("after disposal");
        name.dispatch_event(&Event::new("input")?)?;
        if deliveries.get() != 0 {
            return Err(JsValue::from_str("disposed callback delivered"));
        }
        Ok(())
    }
    fn check_replacement_text(document: &Document) -> Result<(), JsValue> {
        let surface = CharacterPhaseSurface::create(
            document,
            "replacement-check",
            &private_view(),
            limits(),
            |_| {},
        )
        .map_err(error)?;
        let retained = retained_private_nodes(surface.root())?;
        let texts: Vec<_> = retained
            .iter()
            .filter(|node| node.node_type() == Node::TEXT_NODE)
            .cloned()
            .collect();
        let mut next = character_view();
        next.generation += 1;
        next.owner_key = "replacement-owner".into();
        next.groups.clear();
        next.facts.clear();
        next.actions.clear();
        surface.update(&next).map_err(error)?;
        let result = if texts
            .iter()
            .any(|node| node.node_value().is_some_and(|value| !value.is_empty()))
        {
            Err(JsValue::from_str("old owner Text survived replacement"))
        } else {
            Ok(())
        };
        surface.dispose().map_err(error)?;
        result
    }
    fn check_failed_generation(document: &Document) -> Result<(), JsValue> {
        let deliveries = Rc::new(Cell::new(0));
        let recorded = Rc::clone(&deliveries);
        let surface = CharacterPhaseSurface::create(
            document,
            "generation-failure-check",
            &private_view(),
            limits(),
            move |_| recorded.set(recorded.get() + 1),
        )
        .map_err(error)?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("body unavailable"))?;
        body.append_child(surface.root())?;
        let retained = retained_private_nodes(surface.root())?;
        let name = surface.name_input().input().clone();
        let flavor = surface.flavor_input().input().clone();
        let features = surface
            .features_input()
            .ok_or_else(|| JsValue::from_str("features missing"))?
            .input()
            .clone();
        let outfit = surface
            .outfit_input()
            .ok_or_else(|| JsValue::from_str("outfit missing"))?
            .input()
            .clone();
        let action = surface
            .root()
            .query_selector(".character-actions button")?
            .ok_or_else(|| JsValue::from_str("action missing"))?;
        surface.name_input().dispose().map_err(error)?;
        let mut next = character_view();
        next.generation += 1;
        next.owner_key = "new-owner-after-failure".into();
        if surface.update(&next).is_ok() {
            return Err(JsValue::from_str("disposed field allowed update"));
        }
        action.dispatch_event(&Event::new("click")?)?;
        if !name.value().is_empty()
            || !flavor.value().is_empty()
            || !features.value().is_empty()
            || !outfit.value().is_empty()
            || surface.root().is_connected()
            || deliveries.get() != 0
            || retained
                .iter()
                .any(|node| node.text_content().is_some_and(|value| !value.is_empty()))
            || surface.update(&next).is_ok()
        {
            return Err(JsValue::from_str(
                "failed generation retained mixed owner state/control",
            ));
        }
        surface.dispose().map_err(error)?;
        Ok(())
    }
    fn check_equal_revision(document: &Document) -> Result<(), JsValue> {
        let delivered = Rc::new(RefCell::new(None));
        let captured = Rc::clone(&delivered);
        let initial = character_view();
        let surface = CharacterPhaseSurface::create(
            document,
            "equal-revision-check",
            &initial,
            limits(),
            move |submission| *captured.borrow_mut() = Some(submission),
        )
        .map_err(error)?;
        surface.update(&initial).map_err(error)?;
        let name = surface.name_input().input();
        let features = surface
            .features_input()
            .ok_or_else(|| JsValue::from_str("features missing"))?;
        name.set_value("Retained draft");
        name.dispatch_event(&Event::new("input")?)?;
        features.input().set_value("Retained visual draft");
        features.input().dispatch_event(&Event::new("input")?)?;
        let mut conflict = initial.clone();
        conflict.status = CharacterStatus::Locked;
        conflict.actions.clear();
        conflict.title = "Conflicting title".into();
        if surface.update(&conflict).is_ok()
            || surface.name_input().draft() != "Retained draft"
            || features.draft() != "Retained visual draft"
            || surface.root().get_attribute("data-status").as_deref() != Some("editing")
        {
            return Err(JsValue::from_str("equal revision conflict mutated view"));
        }
        let action = surface
            .root()
            .query_selector(".character-actions button")?
            .ok_or_else(|| JsValue::from_str("original action removed"))?;
        action.dispatch_event(&Event::new("click")?)?;
        let correct = delivered.borrow().as_ref().is_some_and(|request| {
            request.generation == initial.generation
                && request.owner_key == initial.owner_key
                && request.revision == initial.revision
                && request.name == "Retained draft"
                && request
                    .appearance
                    .as_ref()
                    .is_some_and(|appearance| appearance.features == "Retained visual draft")
        });
        surface.dispose().map_err(error)?;
        if correct {
            Ok(())
        } else {
            Err(JsValue::from_str("original command scope changed"))
        }
    }

    fn bitmap(frame: &Element) -> Result<Element, JsValue> {
        frame
            .query_selector(".portrait-bitmap")?
            .ok_or_else(|| JsValue::from_str("portrait bitmap missing"))
    }
    fn check_portrait_lifetime(document: &Document) -> Result<(), JsValue> {
        let mut data = private_view();
        let deliveries = Rc::new(Cell::new(0));
        let recorded = Rc::clone(&deliveries);
        let surface = CharacterPhaseSurface::create(
            document,
            "portrait-lifecycle-check",
            &data,
            limits(),
            move |_| recorded.set(recorded.get() + 1),
        )
        .map_err(error)?;
        document
            .body()
            .ok_or_else(|| JsValue::from_str("body unavailable"))?
            .append_child(surface.root())?;
        let frame = surface
            .root()
            .query_selector(".character-portrait")?
            .ok_or_else(|| JsValue::from_str("portrait frame missing"))?;
        let flat = frame
            .query_selector(".portrait-fallback")?
            .ok_or_else(|| JsValue::from_str("flat portrait missing"))?;
        let failed = bitmap(&frame)?;
        let option_frame = surface
            .root()
            .query_selector(".option-portrait")?
            .ok_or_else(|| JsValue::from_str("option portrait missing"))?;
        let option_bitmap = bitmap(&option_frame)?;
        let option_flat = option_frame
            .query_selector(".portrait-fallback")?
            .ok_or_else(|| JsValue::from_str("option fallback missing"))?;
        let option_button = surface
            .root()
            .query_selector(".character-option button")?
            .ok_or_else(|| JsValue::from_str("option control missing"))?;
        let draft = surface.name_input().draft();
        failed.dispatch_event(&Event::new("error")?)?;
        option_bitmap.dispatch_event(&Event::new("error")?)?;
        if !failed.has_attribute("hidden")
            || flat.has_attribute("hidden")
            || frame.has_attribute("hidden")
            || !option_bitmap.has_attribute("hidden")
            || option_flat.has_attribute("hidden")
            || option_button
                .text_content()
                .is_none_or(|text| text.is_empty())
            || surface.name_input().draft() != draft
            || deliveries.get() != 0
        {
            return Err(JsValue::from_str(
                "portrait error did not preserve the flat frame",
            ));
        }
        for _ in 0..64 {
            data.revision += 1;
            surface.update(&data).map_err(error)?;
        }
        if !bitmap(&frame)?.is_same_node(Some(&failed))
            || !failed.has_attribute("hidden")
            || flat.has_attribute("hidden")
        {
            return Err(JsValue::from_str(
                "unchanged failed portrait retried or became visible",
            ));
        }
        if !bitmap(&option_frame)?.is_same_node(Some(&option_bitmap))
            || !option_bitmap.has_attribute("hidden")
            || option_flat.has_attribute("hidden")
        {
            return Err(JsValue::from_str(
                "failed option portrait retried during view updates",
            ));
        }
        surface.retry_failed_portraits().map_err(error)?;
        let retried = bitmap(&frame)?;
        let option_retried = bitmap(&option_frame)?;
        if retried.is_same_node(Some(&failed)) {
            return Err(JsValue::from_str(
                "explicit retry did not replace failed image",
            ));
        }
        retried.dispatch_event(&Event::new("load")?)?;
        option_retried.dispatch_event(&Event::new("load")?)?;
        option_bitmap.dispatch_event(&Event::new("error")?)?;
        failed.dispatch_event(&Event::new("error")?)?;
        if retried.has_attribute("hidden")
            || !flat.has_attribute("hidden")
            || option_retried.has_attribute("hidden")
            || !option_flat.has_attribute("hidden")
        {
            return Err(JsValue::from_str(
                "retired error changed recovered portrait",
            ));
        }
        data.revision += 1;
        data.portrait = Some(CharacterPortrait::Vell);
        surface.update(&data).map_err(error)?;
        let replaced = bitmap(&frame)?;
        replaced.dispatch_event(&Event::new("load")?)?;
        retried.dispatch_event(&Event::new("error")?)?;
        if replaced.has_attribute("hidden") || !flat.has_attribute("hidden") {
            return Err(JsValue::from_str(
                "source replacement accepted retired image event",
            ));
        }
        data.generation += 1;
        data.owner_key = "replacement-portrait-owner".into();
        surface.update(&data).map_err(error)?;
        let current = bitmap(&frame)?;
        current.dispatch_event(&Event::new("load")?)?;
        replaced.dispatch_event(&Event::new("error")?)?;
        if current.has_attribute("hidden") || !flat.has_attribute("hidden") {
            return Err(JsValue::from_str(
                "old generation image event changed new portrait",
            ));
        }
        let removed_option_bitmap = bitmap(&option_frame)?;
        data.revision += 1;
        data.groups.clear();
        surface.update(&data).map_err(error)?;
        removed_option_bitmap.dispatch_event(&Event::new("load")?)?;
        removed_option_bitmap.dispatch_event(&Event::new("error")?)?;
        if !option_frame.has_attribute("hidden") || !removed_option_bitmap.has_attribute("hidden") {
            return Err(JsValue::from_str(
                "removed option portrait listeners published",
            ));
        }
        surface.dispose().map_err(error)?;
        current.dispatch_event(&Event::new("load")?)?;
        current.dispatch_event(&Event::new("error")?)?;
        if !frame.has_attribute("hidden")
            || !flat.has_attribute("hidden")
            || !current.has_attribute("hidden")
        {
            return Err(JsValue::from_str("disposed portrait listeners published"));
        }
        let dropped = CharacterPhaseSurface::create(
            document,
            "portrait-drop-check",
            &private_view(),
            limits(),
            |_| {},
        )
        .map_err(error)?;
        document
            .body()
            .ok_or_else(|| JsValue::from_str("body unavailable"))?
            .append_child(dropped.root())?;
        let dropped_frame = dropped
            .root()
            .query_selector(".character-portrait")?
            .ok_or_else(|| JsValue::from_str("drop portrait missing"))?;
        let dropped_bitmap = bitmap(&dropped_frame)?;
        drop(dropped);
        dropped_bitmap.dispatch_event(&Event::new("load")?)?;
        if dropped_frame.is_connected()
            || !dropped_frame.has_attribute("hidden")
            || !dropped_bitmap.has_attribute("hidden")
        {
            return Err(JsValue::from_str(
                "implicit Drop portrait listener published",
            ));
        }
        if deliveries.get() != 0 {
            return Err(JsValue::from_str(
                "portrait events submitted a game command",
            ));
        }
        Ok(())
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
        document.set_title("DungeonFlux · Character creation · Synthetic design fixture");
        let output = document.create_element("p")?;
        output.set_class_name("character-fixture-output");
        output.set_attribute("role", "status")?;
        output.set_attribute("aria-live", "polite")?;
        output.set_text_content(Some("Fixture input is local only. Use the view controls to inspect reported pending, rejected, ready and locked states."));
        let model = Rc::new(RefCell::new(character_view()));
        let owner: Rc<RefCell<Option<Weak<CharacterPhaseSurface>>>> = Rc::new(RefCell::new(None));
        let submitted_output = output.clone();
        let submitted_owner = Rc::clone(&owner);
        let submitted_model = Rc::clone(&model);
        let surface = Rc::new(CharacterPhaseSurface::create(&document, "character-fixture", &model.borrow(), limits(), move |submission| {
            submitted_output.set_text_content(Some(&format!("Local fixture received offered action {} with {} choice IDs. This is input delivery; no character was created.", submission.action_id, submission.choices.len())));
            let mut view = submitted_model.borrow().clone();
            view.revision += 1; view.status = CharacterStatus::Pending;
            view.status_message = "Synthetic pending view · awaiting a simulated server response. Drafts remain readable.".into();
            if let Some(surface) = submitted_owner.borrow().as_ref().and_then(Weak::upgrade) {
                match surface.update(&view) {
                    Ok(()) => *submitted_model.borrow_mut() = view,
                    Err(error) => submitted_output.set_text_content(Some(&format!("Fixture pending view failed: {error}"))),
                }
            }
        }).map_err(error)?);
        *owner.borrow_mut() = Some(Rc::downgrade(&surface));
        surface.root().append_child(&output)?;
        let controls_root = child(
            &document,
            surface.root(),
            "div",
            "character-fixture-controls",
            "",
        )?;
        controls_root.set_attribute("aria-label", "Synthetic server view controls")?;
        let mut controls = Vec::new();
        for (status, label, message) in [
            (
                CharacterStatus::Editing,
                "Editing view",
                "Synthetic editable view · draft corrections are available.",
            ),
            (
                CharacterStatus::Pending,
                "Pending view",
                "Synthetic pending view · no validation result has arrived.",
            ),
            (
                CharacterStatus::Rejected,
                "Rejected view",
                "Synthetic server rejection · the offered draft needs review. Your name and flavor remain editable.",
            ),
            (
                CharacterStatus::Ready,
                "Ready view",
                "Synthetic server reports ready · no live character creation occurred.",
            ),
            (
                CharacterStatus::Locked,
                "Locked view",
                "Synthetic server reports locked · editing is unavailable. No reopen offer is advertised.",
            ),
        ] {
            let control = ControlledAction::create(
                &document,
                ActionView {
                    label,
                    enabled: true,
                    pending: false,
                },
            )
            .map_err(error)?;
            let updated_surface = Rc::clone(&surface);
            let updated_model = Rc::clone(&model);
            let updated_output = output.clone();
            control.on_activate(move || {
                let mut next = updated_model.borrow().clone();
                next.revision += 1; next.status = status; next.status_message = message.into();
                if matches!(status, CharacterStatus::Ready | CharacterStatus::Locked) {
                    next.name = "Mara · server confirmed".into();
                    next.flavor = "Carries a lantern · server confirmed".into();
                    next.appearance = Some(CharacterAppearanceDraft {
                        features: "Silver braid · server confirmed".into(),
                        outfit: "Weathered green cloak · server confirmed".into(),
                    });
                }
                next.actions = match status {
                    CharacterStatus::Ready => vec![CharacterAction { id: "fixture-mark-ready".into(), kind: CharacterActionKind::MarkReady, label: "Confirm readiness · fixture only".into(), enabled: true }],
                    CharacterStatus::Locked => vec![],
                    _ => character_view().actions,
                };
                let result = updated_surface.update(&next).map_err(error);
                if result.is_ok() { *updated_model.borrow_mut() = next; }
                report(&updated_output, result, "Synthetic view applied in place. Reported state is independent of the local submit callback.");
            }).map_err(error)?;
            controls_root.append_child(control.element())?;
            controls.push(control);
        }
        let repeated = ControlledAction::create(
            &document,
            ActionView {
                label: "Check 64 updates + focus",
                enabled: true,
                pending: false,
            },
        )
        .map_err(error)?;
        let checked_surface = Rc::clone(&surface);
        let checked_model = Rc::clone(&model);
        let checked_document = document.clone();
        let checked_output = output.clone();
        repeated.on_activate(move || {
            let result = (|| -> Result<(), JsValue> {
                let mut next = checked_model.borrow().clone(); next.status = CharacterStatus::Editing; next.revision += 1;
                checked_surface.update(&next).map_err(error)?;
                let input = checked_surface.name_input().input(); input.set_value("Retained local draft"); input.dispatch_event(&Event::new("input")?)?; input.focus()?;
                let before = checked_document.active_element();
                let field_before = input.clone();
                for _ in 0..64 { next.revision += 1; checked_surface.update(&next).map_err(error)?; }
                if checked_surface.name_input().draft() != "Retained local draft"
                    || !before.as_ref().is_some_and(|node| checked_document.active_element().as_ref().is_some_and(|after| node.is_same_node(Some(after))))
                    || !field_before.is_same_node(Some(checked_surface.name_input().input())) {
                    return Err(JsValue::from_str("draft, focus or node identity changed"));
                }
                *checked_model.borrow_mut() = next; Ok(())
            })();
            report(&checked_output, result, "PASS · 64 same-scope updates retained the native name field, focus and local draft.");
        }).map_err(error)?;
        controls_root.append_child(repeated.element())?;
        controls.push(repeated);
        for (case, label) in [
            (0, "Check retained DOM disposal"),
            (1, "Check implicit Drop"),
            (2, "Check owner Text replacement"),
            (3, "Check failed owner reset"),
            (4, "Check equal revision conflict"),
            (5, "Check portrait event ownership"),
        ] {
            let control = ControlledAction::create(
                &document,
                ActionView {
                    label,
                    enabled: true,
                    pending: false,
                },
            )
            .map_err(error)?;
            let checked_document = document.clone();
            let checked_output = output.clone();
            control.on_activate(move || {
                let result = match case {
                    0 => check_cleanup(&checked_document, false),
                    1 => check_cleanup(&checked_document, true),
                    2 => check_replacement_text(&checked_document),
                    3 => check_failed_generation(&checked_document),
                    4 => check_equal_revision(&checked_document),
                    _ => check_portrait_lifetime(&checked_document),
                };
                report(&checked_output, result, match case {
                    0 => "PASS · Explicit disposal scrubbed retained Elements, Text nodes and input values; old callbacks were fenced.",
                    1 => "PASS · Implicit Drop detached the mounted root and scrubbed retained DOM/private drafts; old callbacks were fenced.",
                    2 => "PASS · Owner replacement blanked retained old Text nodes, including removed options and facts.",
                    3 => "PASS · A disposed name field made generation reset fail closed; both drafts and old action callbacks were cleared.",
                    4 => "PASS · An identical view duplicate was accepted; conflicting equal revision left the original draft, state and callback metadata intact.",
                    _ => "PASS · Portrait error used flat fallback; 64 updates did not retry; retry/source/generation replacement and disposal fenced old image events.",
                });
            }).map_err(error)?;
            controls_root.append_child(control.element())?;
            controls.push(control);
        }
        for (portrait, label) in [
            (CharacterPortrait::Vell, "Load Vell portraits"),
            (CharacterPortrait::Narrator, "Recover narrator portraits"),
        ] {
            let control = ControlledAction::create(
                &document,
                ActionView {
                    label,
                    enabled: true,
                    pending: false,
                },
            )
            .map_err(error)?;
            let updated_surface = Rc::clone(&surface);
            let updated_model = Rc::clone(&model);
            let updated_output = output.clone();
            control.on_activate(move || {
                let mut next = updated_model.borrow().clone(); next.revision += 1; next.portrait = Some(portrait);
                for group in &mut next.groups { for option in &mut group.options { option.portrait = Some(portrait); } }
                let result = updated_surface.update(&next).map_err(error);
                if result.is_ok() { *updated_model.borrow_mut() = next; }
                report(&updated_output, result, "Optional portrait source updated. Actual browser load/error selects bitmap or flat silhouette; controls and game state are unchanged.");
            }).map_err(error)?;
            controls_root.append_child(control.element())?;
            controls.push(control);
        }
        let retry = ControlledAction::create(
            &document,
            ActionView {
                label: "Retry failed portraits",
                enabled: true,
                pending: false,
            },
        )
        .map_err(error)?;
        let retried_surface = Rc::clone(&surface);
        let retried_output = output.clone();
        retry.on_activate(move || {
            report(&retried_output, retried_surface.retry_failed_portraits().map_err(error), "Explicit optional-portrait retry requested. Flat silhouettes remain until actual image load succeeds.");
        }).map_err(error)?;
        controls_root.append_child(retry.element())?;
        controls.push(retry);
        for (generation_change, label) in [
            (false, "Withdraw selected offer"),
            (true, "New owner generation"),
        ] {
            let control = ControlledAction::create(
                &document,
                ActionView {
                    label,
                    enabled: true,
                    pending: false,
                },
            )
            .map_err(error)?;
            let changed_surface = Rc::clone(&surface);
            let changed_model = Rc::clone(&model);
            let changed_output = output.clone();
            control.on_activate(move || {
                let mut next = changed_model.borrow().clone(); next.revision += 1;
                if generation_change {
                    next = character_view(); next.generation = changed_model.borrow().generation + 1;
                    next.owner_key = format!("synthetic-player-{}", next.generation); next.name.clear(); next.flavor.clear();
                } else if let Some(group) = next.groups.first_mut() {
                    group.options.retain(|option| option.id != "lantern-keeper"); group.selected = None;
                }
                let result = changed_surface.update(&next).map_err(error);
                if result.is_ok() { *changed_model.borrow_mut() = next; }
                report(&changed_output, result, if generation_change { "New synthetic owner applied · obsolete name/flavor and local selections were cleared." } else { "Offer withdrawn · obsolete control removed and its callback disposed." });
            }).map_err(error)?;
            controls_root.append_child(control.element())?;
            controls.push(control);
        }
        let terminal = child(&document, body.as_ref(), "p", "", "")?;
        terminal.set_attribute("hidden", "")?;
        terminal.set_attribute("role", "status")?;
        terminal.set_attribute(
            "style",
            "padding:40px;color:#f5eee0;font:18px Georgia,serif",
        )?;
        let disposed = ControlledAction::create(
            &document,
            ActionView {
                label: "Dispose fixture",
                enabled: true,
                pending: false,
            },
        )
        .map_err(error)?;
        let terminal_output = terminal.clone();
        let terminal_document = document.clone();
        disposed.on_activate(move || {
            let result = (|| -> Result<(), JsValue> {
                shutdown()?; shutdown()?;
                if terminal_document.query_selector(".df-character")?.is_some() { return Err(JsValue::from_str("scope still mounted")); }
                terminal_output.remove_attribute("hidden")?; terminal_output.set_attribute("data-disposal", "pass")?; Ok(())
            })();
            terminal_output.set_text_content(Some(if result.is_ok() { "PASS · Fixture disposed twice. Its owned controls and mounted character scope were removed." } else { "FAIL · Fixture disposal failed." }));
        }).map_err(error)?;
        controls_root.append_child(disposed.element())?;
        controls.push(disposed);
        body.append_child(surface.root())?;
        FIXTURE.with(|slot| *slot.borrow_mut() = Some(Fixture { surface, controls }));
        Ok(())
    }
    #[wasm_bindgen]
    pub fn shutdown() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut fixture = slot.borrow_mut();
            if let Some(owned) = fixture.as_ref() {
                for control in &owned.controls {
                    control.dispose().map_err(error)?;
                }
                owned.surface.dispose().map_err(error)?;
            }
            *fixture = None;
            Ok(())
        })
    }
}
