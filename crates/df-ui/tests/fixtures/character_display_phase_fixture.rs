//! Shared-display character-creation design fixture. Every roster, state and host
//! offer is synthetic. No game session, private player projection or rules service
//! is connected; the separate player fixture remains character_phase_fixture.

#[cfg(target_arch = "wasm32")]
mod browser {
    use df_ui::{
        ActionView, CharacterDisplayConnection, CharacterDisplayHostOffer, CharacterDisplayLimits,
        CharacterDisplaySurface, CharacterDisplayView, CharacterPortrait, CharacterPublicMember,
        CharacterPublicReadiness, ControlledAction,
    };
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    use wasm_bindgen::{JsCast, prelude::*};
    use web_sys::{Document, Element, Event, Node};
    struct Fixture {
        surface: Rc<CharacterDisplaySurface>,
        controls: Vec<ControlledAction>,
    }
    thread_local! { static FIXTURE: RefCell<Option<Fixture>> = const { RefCell::new(None) }; }
    fn error(error: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&error.to_string())
    }
    fn limits() -> CharacterDisplayLimits {
        CharacterDisplayLimits {
            max_members: 64,
            max_host_offers: 16,
            max_text_bytes: 4096,
        }
    }
    fn synthetic_view() -> CharacterDisplayView {
        CharacterDisplayView { generation: 1, public_scope_key: "synthetic-public-party".into(), revision: 1,
            chapter: "Prologue · A party takes shape".into(), title: "Every story needs its heroes".into(),
            description: "A lantern in the darkness. A promise waiting to be kept. Together, your stories will become an adventure.".into(),
            readiness: CharacterPublicReadiness::Choosing, progress_label: "Server-supplied public progress · Synthetic roster".into(),
            public_notice: "Shared-display design fixture · Concept portraits · includes AI-generated v2 artwork · No live character builds or private player drafts.".into(),
            connection: CharacterDisplayConnection::Connected,
            connection_label: "Synthetic connected view · No production session, RPC adapter or client shell mounted".into(),
            members: vec![
                CharacterPublicMember { key: "public-mara".into(), character_name: "Mara".into(), portrait: Some(CharacterPortrait::Narrator), readiness: CharacterPublicReadiness::Choosing, progress_label: "Public report · Choosing character".into(), appearance_summary: None },
                CharacterPublicMember { key: "public-elian".into(), character_name: "Elian".into(), portrait: Some(CharacterPortrait::Vell), readiness: CharacterPublicReadiness::Reviewing, progress_label: "Public report · Awaiting review".into(), appearance_summary: None },
                CharacterPublicMember { key: "public-aster".into(), character_name: "Aster".into(), portrait: Some(CharacterPortrait::Narrator), readiness: CharacterPublicReadiness::Ready, progress_label: "Public report · Ready".into(), appearance_summary: Some("A silver braid and blue travel coat".into()) },
                CharacterPublicMember { key: "public-rowan".into(), character_name: "Rowan".into(), portrait: Some(CharacterPortrait::Vell), readiness: CharacterPublicReadiness::Locked, progress_label: "Public report · Locked".into(), appearance_summary: None },
            ], host_offers: vec![] }
    }
    fn host_offer() -> CharacterDisplayHostOffer {
        CharacterDisplayHostOffer {
            id: "synthetic-public-host-offer".into(),
            label: "Host offer · fixture only".into(),
            enabled: true,
            pending: false,
        }
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
    fn retain(node: &Node, retained_nodes: &mut Vec<Node>) {
        retained_nodes.push(node.clone());
        let mut child = node.first_child();
        while let Some(node) = child {
            child = node.next_sibling();
            retain(&node, retained_nodes);
        }
    }
    fn public_nodes(root: &Element) -> Result<Vec<Node>, JsValue> {
        let mut nodes = Vec::new();
        for selector in [
            ".character-hero",
            ".display-roster",
            ".display-host-offers",
            ".character-connection",
        ] {
            if let Some(node) = root.query_selector(selector)? {
                retain(node.as_ref(), &mut nodes);
            }
        }
        Ok(nodes)
    }
    fn check_disposal(document: &Document, drop_without_dispose: bool) -> Result<(), JsValue> {
        let mut display_view = synthetic_view();
        display_view.host_offers.push(host_offer());
        let deliveries = Rc::new(Cell::new(0));
        let captured = Rc::clone(&deliveries);
        let surface =
            CharacterDisplaySurface::create(document, &display_view, limits(), move |_| {
                captured.set(captured.get() + 1)
            })
            .map_err(error)?;
        document
            .body()
            .ok_or_else(|| JsValue::from_str("body unavailable"))?
            .append_child(surface.root())?;
        let root = surface.root().clone();
        let retained = public_nodes(&root)?;
        let button = root
            .query_selector(".display-host-offers button")?
            .ok_or_else(|| JsValue::from_str("host offer missing"))?;
        let image = root
            .query_selector(".portrait-bitmap")?
            .ok_or_else(|| JsValue::from_str("portrait missing"))?;
        if drop_without_dispose {
            drop(surface);
        } else {
            surface.dispose().map_err(error)?;
            surface.dispose().map_err(error)?;
            drop(surface);
        }
        button.dispatch_event(&Event::new("click")?)?;
        image.dispatch_event(&Event::new("load")?)?;
        if root.is_connected()
            || deliveries.get() != 0
            || !image.has_attribute("hidden")
            || retained.iter().any(|node| {
                node.text_content().is_some_and(|text| !text.is_empty())
                    || node.node_value().is_some_and(|text| !text.is_empty())
            })
        {
            return Err(JsValue::from_str(
                "shared display teardown retained prose or active resources",
            ));
        }
        Ok(())
    }
    fn check_updates_and_offers(document: &Document) -> Result<(), JsValue> {
        let mut display_view = synthetic_view();
        display_view.host_offers.push(host_offer());
        let deliveries = Rc::new(RefCell::new(Vec::new()));
        let captured = Rc::clone(&deliveries);
        let surface =
            CharacterDisplaySurface::create(document, &display_view, limits(), move |input| {
                captured.borrow_mut().push(input)
            })
            .map_err(error)?;
        document
            .body()
            .ok_or_else(|| JsValue::from_str("body unavailable"))?
            .append_child(surface.root())?;
        let root = surface.root().clone();
        if root
            .query_selector("input, .df-ui-field, .character-choice-group")?
            .is_some()
        {
            return Err(JsValue::from_str(
                "private player form mounted in shared display",
            ));
        }
        let member = root
            .query_selector(".display-member")?
            .ok_or_else(|| JsValue::from_str("member missing"))?;
        let member_text_node = member
            .query_selector("h2")?
            .and_then(|node| node.first_child())
            .ok_or_else(|| JsValue::from_str("member text missing"))?;
        let button = root
            .query_selector(".display-host-offers button")?
            .ok_or_else(|| JsValue::from_str("host offer missing"))?;
        button
            .dyn_ref::<web_sys::HtmlElement>()
            .ok_or_else(|| JsValue::from_str("focusable button missing"))?
            .focus()?;
        for _ in 0..64 {
            display_view.revision += 1;
            surface.update(&display_view).map_err(error)?;
        }
        if !root
            .query_selector(".display-member")?
            .as_ref()
            .is_some_and(|node| node.is_same_node(Some(&member)))
            || !document
                .active_element()
                .as_ref()
                .is_some_and(|node| node.is_same_node(Some(&button)))
            || member_text_node
                .node_value()
                .is_some_and(|text| !text.is_empty())
        {
            return Err(JsValue::from_str("keyed member/focus/text cleanup changed"));
        }
        button.dispatch_event(&Event::new("click")?)?;
        if deliveries.borrow().len() != 1
            || root.get_attribute("data-readiness").as_deref() != Some("choosing")
        {
            return Err(JsValue::from_str(
                "host callback did not preserve reported readiness",
            ));
        }
        let mut conflict = display_view.clone();
        conflict.readiness = CharacterPublicReadiness::Locked;
        if surface.update(&conflict).is_ok() {
            return Err(JsValue::from_str("conflicting equal revision accepted"));
        }
        display_view.revision += 1;
        display_view.connection = CharacterDisplayConnection::Offline;
        surface.update(&display_view).map_err(error)?;
        button.dispatch_event(&Event::new("click")?)?;
        if deliveries.borrow().len() != 1 {
            return Err(JsValue::from_str("offline host input admitted"));
        }
        display_view.revision += 1;
        display_view.connection = CharacterDisplayConnection::Connected;
        display_view.host_offers.clear();
        surface.update(&display_view).map_err(error)?;
        button.dispatch_event(&Event::new("click")?)?;
        if deliveries.borrow().len() != 1 {
            return Err(JsValue::from_str("withdrawn host input admitted"));
        }
        display_view.revision += 1;
        display_view.host_offers.push(host_offer());
        surface.update(&display_view).map_err(error)?;
        let stale_offer_button = root
            .query_selector(".display-host-offers button")?
            .ok_or_else(|| JsValue::from_str("replacement offer missing"))?;
        let retained_nodes = public_nodes(&root)?;
        display_view.generation += 1;
        display_view.public_scope_key = "new-synthetic-public-party".into();
        display_view.revision = 1;
        display_view.members.clear();
        surface.update(&display_view).map_err(error)?;
        stale_offer_button.dispatch_event(&Event::new("click")?)?;
        if deliveries.borrow().len() != 1
            || retained_nodes
                .iter()
                .filter(|node| node.node_type() == Node::TEXT_NODE)
                .any(|node| node.node_value().is_some_and(|text| !text.is_empty()))
        {
            return Err(JsValue::from_str(
                "old generation host target or retained Text survived",
            ));
        }
        let current_offer_button = root
            .query_selector(".display-host-offers button")?
            .ok_or_else(|| JsValue::from_str("current offer missing"))?;
        current_offer_button.dispatch_event(&Event::new("click")?)?;
        if !deliveries.borrow().last().is_some_and(|input| {
            input.generation == 2 && input.public_scope_key == display_view.public_scope_key
        }) {
            return Err(JsValue::from_str("host callback used wrong generation"));
        }
        surface.dispose().map_err(error)?;
        Ok(())
    }
    fn check_portraits(document: &Document) -> Result<(), JsValue> {
        let mut display_view = synthetic_view();
        let surface = CharacterDisplaySurface::create(document, &display_view, limits(), |_| {})
            .map_err(error)?;
        document
            .body()
            .ok_or_else(|| JsValue::from_str("body unavailable"))?
            .append_child(surface.root())?;
        let frame = surface
            .root()
            .query_selector(".display-portrait")?
            .ok_or_else(|| JsValue::from_str("portrait frame missing"))?;
        let previous_image = frame
            .query_selector(".portrait-bitmap")?
            .ok_or_else(|| JsValue::from_str("portrait bitmap missing"))?;
        let portrait_fallback = frame
            .query_selector(".portrait-fallback")?
            .ok_or_else(|| JsValue::from_str("portrait fallback missing"))?;
        previous_image.dispatch_event(&Event::new("error")?)?;
        for _ in 0..64 {
            display_view.revision += 1;
            surface.update(&display_view).map_err(error)?;
        }
        if !previous_image.has_attribute("hidden")
            || portrait_fallback.has_attribute("hidden")
            || !frame
                .query_selector(".portrait-bitmap")?
                .as_ref()
                .is_some_and(|image| image.is_same_node(Some(&previous_image)))
        {
            return Err(JsValue::from_str(
                "shared portrait failed source retried/unhidden",
            ));
        }
        display_view.revision += 1;
        if let Some(member) = display_view.members.first_mut() {
            member.portrait = Some(CharacterPortrait::Vell);
        }
        surface.update(&display_view).map_err(error)?;
        let current_image = frame
            .query_selector(".portrait-bitmap")?
            .ok_or_else(|| JsValue::from_str("replacement image missing"))?;
        current_image.dispatch_event(&Event::new("load")?)?;
        previous_image.dispatch_event(&Event::new("error")?)?;
        if current_image.has_attribute("hidden") || !portrait_fallback.has_attribute("hidden") {
            return Err(JsValue::from_str("retired shared portrait event published"));
        }
        surface.dispose().map_err(error)?;
        current_image.dispatch_event(&Event::new("load")?)?;
        if !current_image.has_attribute("hidden") {
            return Err(JsValue::from_str("disposed shared portrait published"));
        }
        Ok(())
    }
    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        if FIXTURE.with(|slot| slot.borrow().is_some()) {
            return Err(JsValue::from_str("shared fixture already mounted"));
        }
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| JsValue::from_str("document unavailable"))?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("body unavailable"))?;
        body.set_attribute("style", "margin:0;background:#070d14")?;
        document.set_title("DungeonFlux · Shared character creation · Synthetic design fixture");
        let output = document.create_element("p")?;
        output.set_class_name("display-fixture-output");
        output.set_attribute("role", "status")?;
        output.set_attribute("aria-live", "polite")?;
        output.set_text_content(Some("Public roster fixture only. The player character form is a separate Rust component; no production adapters or client shells are connected."));
        let model = Rc::new(RefCell::new(synthetic_view()));
        let submitted = output.clone();
        let surface = Rc::new(CharacterDisplaySurface::create(&document, &model.borrow(), limits(), move |offer| {
            submitted.set_text_content(Some(&format!("Local synthetic host input received advertised offer {} at revision {}. Reported public readiness remains server-owned.", offer.host_offer_id, offer.revision)));
        }).map_err(error)?);
        surface.root().append_child(&output)?;
        let tools = child(
            &document,
            surface.root(),
            "div",
            "display-fixture-controls",
            "",
        )?;
        tools.set_attribute("aria-label", "Synthetic public server view controls")?;
        let mut controls = Vec::new();
        for (case, label) in [
            (0, "Public review view"),
            (1, "Public ready view"),
            (2, "Public locked view"),
            (3, "Offline view"),
            (4, "Public error notice"),
            (5, "Connected view"),
            (6, "Advertise host offer"),
            (7, "Withdraw host offer"),
            (8, "New public generation"),
            (9, "Load Vell portraits"),
            (10, "Recover narrator portraits"),
            (11, "Reconnecting view"),
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
            let owner = Rc::clone(&surface);
            let state = Rc::clone(&model);
            let feedback = output.clone();
            control.on_activate(move || {
                let mut next = state.borrow().clone(); next.revision += 1;
                match case {
                    0 => { next.readiness = CharacterPublicReadiness::Reviewing; next.public_notice = "Synthetic public review pending · no private rejection payload displayed.".into(); },
                    1 => { next.readiness = CharacterPublicReadiness::Ready; next.public_notice = "Synthetic server reports party ready · no local readiness decision.".into(); },
                    2 => { next.readiness = CharacterPublicReadiness::Locked; next.public_notice = "Synthetic server reports public party locked.".into(); },
                    3 => { next.connection = CharacterDisplayConnection::Offline; next.connection_label = "Synthetic offline view · showing the last supplied public roster".into(); next.public_notice = "Connection unavailable · advertised input is temporarily unavailable.".into(); },
                    4 => next.public_notice = "Synthetic public service error · character editing details remain on permitted player clients.".into(),
                    5 => {
                        next.connection = CharacterDisplayConnection::Connected;
                        next.connection_label = synthetic_view().connection_label;
                        next.public_notice = synthetic_view().public_notice;
                    }
                    6 => next.host_offers = vec![host_offer()],
                    7 => next.host_offers.clear(),
                    8 => {
                        let generation = next.generation + 1;
                        next = synthetic_view();
                        next.generation = generation;
                        next.public_scope_key = format!("synthetic-public-party-{generation}");
                    }
                    11 => { next.connection = CharacterDisplayConnection::Reconnecting; next.connection_label = "Synthetic reconnecting view · recovering public roster".into(); next.public_notice = "Connection recovery pending · host input is temporarily unavailable.".into(); },
                    _ => for member in &mut next.members { member.portrait = Some(if case == 9 { CharacterPortrait::Vell } else { CharacterPortrait::Narrator }); },
                }
                if case <= 2 {
                    for member in &mut next.members {
                        member.readiness = next.readiness;
                        member.progress_label = "Synthetic public server report updated".into();
                    }
                }
                match owner.update(&next) { Ok(()) => { *state.borrow_mut() = next; feedback.set_text_content(Some("Synthetic public view updated in place · no game mutation or private-player data rendered.")); }, Err(error) => feedback.set_text_content(Some(&format!("Public presentation update failed: {error}"))) }
            }).map_err(error)?;
            tools.append_child(control.element())?;
            controls.push(control);
        }
        for (case, label) in [
            (0, "Check public roster + offers"),
            (1, "Check retained-node disposal"),
            (2, "Check shared Drop"),
            (3, "Check shared portrait events"),
            (4, "Retry failed portraits"),
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
            let feedback = output.clone();
            let owner = Rc::clone(&surface);
            control.on_activate(move || {
                let result = match case { 0 => check_updates_and_offers(&checked_document), 1 => check_disposal(&checked_document, false), 2 => check_disposal(&checked_document, true), 3 => check_portraits(&checked_document), _ => owner.retry_failed_portraits().map_err(error) };
                feedback.set_text_content(Some(if result.is_ok() { "PASS · Shared-display mounted presentation boundary check completed. This is synthetic UI evidence only." } else { "FAIL · Shared-display presentation check failed." }));
            }).map_err(error)?;
            tools.append_child(control.element())?;
            controls.push(control);
        }
        let terminal = child(&document, body.as_ref(), "p", "", "")?;
        terminal.set_attribute("hidden", "")?;
        terminal.set_attribute("role", "status")?;
        terminal.set_attribute(
            "style",
            "padding:40px;color:#f5eee0;font:18px Georgia,serif",
        )?;
        let dispose = ControlledAction::create(
            &document,
            ActionView {
                label: "Dispose shared fixture",
                enabled: true,
                pending: false,
            },
        )
        .map_err(error)?;
        let feedback = terminal.clone();
        let checked_document = document.clone();
        dispose.on_activate(move || {
            let result = (|| -> Result<(), JsValue> { shutdown()?; shutdown()?; if checked_document.query_selector(".df-character-display")?.is_some() { return Err(JsValue::from_str("shared scope mounted after teardown")); } feedback.remove_attribute("hidden")?; Ok(()) })();
            feedback.set_text_content(Some(if result.is_ok() { "PASS · Shared fixture disposed twice; owned public roster and host controls removed." } else { "FAIL · Shared fixture teardown failed." }));
        }).map_err(error)?;
        tools.append_child(dispose.element())?;
        controls.push(dispose);
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
