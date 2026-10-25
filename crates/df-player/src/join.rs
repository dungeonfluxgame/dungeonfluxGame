//! Persistent player join/lobby composition over already audience-filtered props.
//! This mount collects advertised input; it does not admit members or infer authority.

/// Transport presentation is independent of membership, readiness and pending offers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlayerJoinConnection {
    Connected,
    Reconnecting,
    Offline,
}

/// Initial local drafts and the caller's unique HTML input identifier. These
/// values never constitute a session grant and are never persisted by the mount.
pub struct PlayerJoinInput<'a> {
    pub identifier: &'a str,
    pub invitation: &'a str,
    pub player_name: &'a str,
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::{PlayerJoinConnection, PlayerJoinInput};
    use df_ui::{
        CampaignLimits, JoinPhaseError, JoinPhaseIntent, JoinPhaseSurface, JoinPhaseView,
        JoinUpdate, JoinValidationError,
    };
    use std::{cell::Cell, fmt, rc::Rc};
    use wasm_bindgen::{JsCast, JsValue, closure::Closure};
    use web_sys::{
        Document, Element, Event, HtmlElement, HtmlFieldSetElement, HtmlImageElement, Node,
    };

    #[derive(Debug)]
    pub enum PlayerJoinError {
        Presentation(JoinPhaseError),
        Cleanup {
            update: Box<JoinPhaseError>,
            cleanup: Box<JoinPhaseError>,
        },
    }
    impl fmt::Display for PlayerJoinError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Presentation(error) => fmt::Display::fmt(error, formatter),
                Self::Cleanup { update, cleanup } => {
                    write!(formatter, "{update}; join cleanup failed: {cleanup}")
                }
            }
        }
    }
    impl std::error::Error for PlayerJoinError {}
    impl From<JoinPhaseError> for PlayerJoinError {
        fn from(error: JoinPhaseError) -> Self {
            Self::Presentation(error)
        }
    }
    impl From<JsValue> for PlayerJoinError {
        fn from(error: JsValue) -> Self {
            Self::Presentation(error.into())
        }
    }

    fn collect_nodes(node: &Node, nodes: &mut Vec<Node>) {
        nodes.push(node.clone());
        let mut child = node.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            collect_nodes(&current, nodes);
        }
    }
    fn scrub_nodes(nodes: Vec<Node>) -> Result<(), JoinPhaseError> {
        let mut failure = None;
        for node in nodes {
            if node.node_type() == Node::TEXT_NODE {
                node.set_node_value(Some(""));
            }
            if let Some(element) = node.dyn_ref::<Element>() {
                for attribute in element
                    .get_attribute_names()
                    .iter()
                    .filter_map(|value| value.as_string())
                {
                    if let Err(error) = element.remove_attribute(&attribute) {
                        failure.get_or_insert(JoinPhaseError::from(error));
                    }
                }
                element.set_text_content(None);
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    struct OptionalArt {
        image: HtmlImageElement,
        fallback: HtmlElement,
        load: Closure<dyn FnMut(Event)>,
        error: Closure<dyn FnMut(Event)>,
    }
    impl OptionalArt {
        fn mount(
            document: &Document,
            root: &Element,
            selector: &str,
            is_scene: bool,
            is_alive: &Rc<Cell<bool>>,
        ) -> Result<Self, JoinPhaseError> {
            let image = root
                .query_selector(selector)?
                .ok_or_else(|| JsValue::from_str("join artwork node missing"))?
                .dyn_into::<HtmlImageElement>()
                .map_err(|_| JsValue::from_str("join artwork node has wrong type"))?;
            let fallback = document
                .create_element(if is_scene { "div" } else { "span" })?
                .dyn_into::<HtmlElement>()
                .map_err(|_| JsValue::from_str("join artwork fallback has wrong type"))?;
            fallback.set_attribute(
                "data-join-art-fallback",
                if is_scene { "scene" } else { "narrator" },
            )?;
            fallback.set_attribute("role", "status")?;
            fallback.set_attribute("style", if is_scene {
                "position:absolute;inset:0;z-index:-3;background:radial-gradient(ellipse at top,#244354,#070d14 70%);color:#c5d1d8;display:grid;place-items:center;font:14px system-ui;text-align:center;padding:24px"
            } else {
                "min-width:46px;min-height:46px;border-radius:50%;border:1px solid #d1b67c;color:#d1b67c;display:inline-grid;place-items:center;font:20px Georgia"
            })?;
            fallback.set_text_content(Some(if is_scene {
                "The story continues · scene art unavailable"
            } else {
                "✦"
            }));
            image
                .parent_node()
                .ok_or_else(|| JsValue::from_str("join artwork parent missing"))?
                .append_child(&fallback)?;
            let update =
                |image: HtmlImageElement, fallback: HtmlElement, is_alive: Rc<Cell<bool>>| {
                    Closure::wrap(Box::new(move |_: Event| {
                        // A queued completion for an earlier src cannot publish while
                        // the browser reports the current image is still loading.
                        if is_alive.get() && image.complete() {
                            let missing = image.natural_width() == 0;
                            image.set_hidden(missing);
                            fallback.set_hidden(!missing);
                        }
                    }) as Box<dyn FnMut(Event)>)
                };
            let load = update(image.clone(), fallback.clone(), Rc::clone(is_alive));
            let error = update(image.clone(), fallback.clone(), Rc::clone(is_alive));
            image.add_event_listener_with_callback("load", load.as_ref().unchecked_ref())?;
            if let Err(error_value) =
                image.add_event_listener_with_callback("error", error.as_ref().unchecked_ref())
            {
                image.remove_event_listener_with_callback("load", load.as_ref().unchecked_ref())?;
                return Err(error_value.into());
            }
            let art = Self {
                image,
                fallback,
                load,
                error,
            };
            art.refresh();
            Ok(art)
        }
        fn refresh(&self) {
            let missing = self.image.complete() && self.image.natural_width() == 0;
            self.image.set_hidden(missing);
            self.fallback.set_hidden(!missing);
        }
        fn dispose(&self) -> Result<(), JoinPhaseError> {
            let mut failure = None;
            for (name, callback) in [("load", &self.load), ("error", &self.error)] {
                if let Err(error) = self
                    .image
                    .remove_event_listener_with_callback(name, callback.as_ref().unchecked_ref())
                {
                    failure.get_or_insert(JoinPhaseError::from(error));
                }
            }
            if let Err(error) = self.image.remove_attribute("src") {
                failure.get_or_insert(JoinPhaseError::from(error));
            }
            self.fallback.set_text_content(None);
            self.fallback.remove();
            match failure {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
    }

    /// Retains one role's current JoinPhaseSurface through ordinary updates and
    /// reconnect. The caller assigns generation and independently filters every prop.
    /// No credentials, views from another role or production RPC objects are stored.
    pub struct PlayerJoinScreen {
        document: Document,
        root: Element,
        status: Element,
        fieldset: HtmlFieldSetElement,
        surface: JoinPhaseSurface,
        connection: PlayerJoinConnection,
        generation: Option<u64>,
        is_current_render_ready: bool,
        is_input_enabled: Rc<Cell<bool>>,
        is_alive: Rc<Cell<bool>>,
        resume_focus: Option<HtmlElement>,
        art: Vec<OptionalArt>,
    }
    impl PlayerJoinScreen {
        pub fn mount(
            document: &Document,
            slot: &Element,
            view: &JoinPhaseView<'_>,
            input: PlayerJoinInput<'_>,
            limits: CampaignLimits,
            connection: PlayerJoinConnection,
            mut on_intent: impl FnMut(JoinPhaseIntent) + 'static,
        ) -> Result<Self, PlayerJoinError> {
            let root = document.create_element("section")?;
            root.set_attribute("data-join-client", "player")?;
            let status = document.create_element("p")?;
            status.set_attribute("role", "status")?;
            status.set_attribute("aria-live", "polite")?;
            status.set_attribute(
                "style",
                "margin:0;padding:12px 4vw;background:#0a1521;color:#c5d1d8;font:13px system-ui",
            )?;
            let fieldset = document
                .create_element("fieldset")?
                .dyn_into::<HtmlFieldSetElement>()
                .map_err(|_| JsValue::from_str("join fieldset unavailable"))?;
            fieldset.set_attribute("aria-label", "player join and lobby")?;
            fieldset.set_attribute("style", "border:0;padding:0;margin:0;min-inline-size:0")?;
            root.append_child(&status)?;
            root.append_child(&fieldset)?;
            let is_input_enabled =
                Rc::new(Cell::new(connection == PlayerJoinConnection::Connected));
            let gate = Rc::clone(&is_input_enabled);
            let surface = JoinPhaseSurface::create(
                document,
                input.identifier,
                view,
                input.invitation,
                input.player_name,
                limits,
            )?;
            surface.on_intent(move |intent| {
                if gate.get() {
                    on_intent(intent);
                }
            })?;
            fieldset.append_child(surface.root())?;
            let mut screen = Self {
                document: document.clone(),
                root,
                status,
                fieldset,
                surface,
                connection,
                generation: Some(view.stamp.generation),
                is_current_render_ready: true,
                is_input_enabled,
                is_alive: Rc::new(Cell::new(true)),
                resume_focus: None,
                art: Vec::new(),
            };
            screen.art.push(OptionalArt::mount(
                document,
                screen.surface.root(),
                ".scene-art",
                true,
                &screen.is_alive,
            )?);
            screen.art.push(OptionalArt::mount(
                document,
                screen.surface.root(),
                ".narrator",
                false,
                &screen.is_alive,
            )?);
            screen.set_connection(connection)?;
            slot.append_child(screen.root())?;
            Ok(screen)
        }
        pub fn root(&self) -> &Element {
            &self.root
        }

        /// Applies only the existing ownership generation. Stale views never
        /// resume a failed render or alter local transport state. Equal-stamp
        /// recovery stays with the component's exact failed-attempt watermark.
        pub fn update(&mut self, view: &JoinPhaseView<'_>) -> Result<JoinUpdate, PlayerJoinError> {
            if self.generation.is_none() {
                return Err(JoinPhaseError::from(JoinValidationError::Disposed).into());
            }
            let mut nodes = Vec::new();
            collect_nodes(self.surface.root().as_ref(), &mut nodes);
            let was_input_enabled = self.is_input_enabled.replace(false);
            let result = self.surface.update(view);
            let detached = nodes
                .into_iter()
                .filter(|node| !self.surface.root().contains(Some(node)))
                .collect();
            if let Err(cleanup) = scrub_nodes(detached) {
                self.is_current_render_ready = false;
                self.fieldset.set_disabled(true);
                return Err(self.retire_after_error(cleanup));
            }
            match result {
                Ok(JoinUpdate::Applied) => {
                    self.is_current_render_ready = true;
                    self.is_input_enabled
                        .set(self.connection == PlayerJoinConnection::Connected);
                    self.fieldset
                        .set_disabled(self.connection != PlayerJoinConnection::Connected);
                    for art in &self.art {
                        art.refresh();
                    }
                    Ok(JoinUpdate::Applied)
                }
                Ok(stale) => {
                    self.is_input_enabled.set(was_input_enabled);
                    Ok(stale)
                }
                Err(error @ JoinPhaseError::InvalidView(_)) => {
                    self.is_input_enabled.set(was_input_enabled);
                    Err(error.into())
                }
                Err(error) => {
                    self.is_current_render_ready = false;
                    self.fieldset.set_disabled(true);
                    self.status.set_text_content(Some(
                        "Join presentation unavailable · awaiting a current view",
                    ));
                    Err(error.into())
                }
            }
        }
        /// Explicit ownership change retires obsolete callbacks before validating
        /// replacement props. Failure revokes this mount; it cannot expose the
        /// previous role/member's drafts or permit reconnect to revive them.
        pub fn replace_owner(&mut self, view: &JoinPhaseView<'_>) -> Result<(), PlayerJoinError> {
            let generation = self
                .generation
                .ok_or_else(|| JoinPhaseError::from(JoinValidationError::Disposed))?;
            if view.stamp.generation <= generation {
                return Err(
                    JoinPhaseError::from(JoinValidationError::GenerationNotAdvanced).into(),
                );
            }
            self.is_input_enabled.set(false);
            self.is_current_render_ready = false;
            self.resume_focus = None;
            let mut nodes = Vec::new();
            collect_nodes(self.surface.root().as_ref(), &mut nodes);
            let result = self.surface.replace_owner(view);
            let detached = nodes
                .into_iter()
                .filter(|node| !self.surface.root().contains(Some(node)))
                .collect();
            if let Err(error) = scrub_nodes(detached) {
                return Err(self.retire_after_error(error));
            }
            if let Err(error) = result {
                return Err(self.retire_after_error(error));
            }
            self.generation = Some(view.stamp.generation);
            self.is_current_render_ready = true;
            for art in &self.art {
                art.refresh();
            }
            self.set_connection(self.connection)
        }
        pub fn set_connection(
            &mut self,
            connection: PlayerJoinConnection,
        ) -> Result<(), PlayerJoinError> {
            if self.generation.is_none() {
                return Err(JoinPhaseError::from(JoinValidationError::Disposed).into());
            }
            if self.connection == PlayerJoinConnection::Connected
                && connection != PlayerJoinConnection::Connected
            {
                self.resume_focus = self
                    .document
                    .active_element()
                    .filter(|node| self.root.contains(Some(node)))
                    .and_then(|node| node.dyn_into::<HtmlElement>().ok());
            }
            self.connection = connection;
            let is_enabled =
                connection == PlayerJoinConnection::Connected && self.is_current_render_ready;
            self.is_input_enabled.set(is_enabled);
            self.fieldset.set_disabled(!is_enabled);
            self.status.set_text_content(Some(match connection {
                PlayerJoinConnection::Connected if self.is_current_render_ready => {
                    "Connected · player join and lobby"
                }
                PlayerJoinConnection::Connected => {
                    "Connected · awaiting a current join presentation"
                }
                PlayerJoinConnection::Reconnecting => {
                    "Reconnecting · join input suspended; membership unchanged"
                }
                PlayerJoinConnection::Offline => {
                    "Offline · join input suspended; membership unchanged"
                }
            }));
            if is_enabled
                && let Some(element) = self.resume_focus.take()
                && element.is_connected()
                && !element.matches(":disabled")?
                && element.closest("[hidden]")?.is_none()
            {
                element.focus()?;
            }
            Ok(())
        }
        fn retire_after_error(&mut self, update: JoinPhaseError) -> PlayerJoinError {
            match self.revoke() {
                Ok(()) => update.into(),
                Err(PlayerJoinError::Presentation(cleanup)) => PlayerJoinError::Cleanup {
                    update: Box::new(update),
                    cleanup: Box::new(cleanup),
                },
                Err(cleanup) => cleanup,
            }
        }
        /// Terminal, idempotent disposal. Fences callbacks first, releases owned
        /// optional-art listeners, and scrubs raw Text/Element references before
        /// component disposal can detach them from the retained DOM tree.
        pub fn revoke(&mut self) -> Result<(), PlayerJoinError> {
            self.is_input_enabled.set(false);
            self.is_alive.set(false);
            self.generation = None;
            self.is_current_render_ready = false;
            self.resume_focus = None;
            self.fieldset.set_disabled(true);
            self.surface.invitation().input().set_value("");
            self.surface.player_name().input().set_value("");
            let mut nodes = Vec::new();
            collect_nodes(self.root.as_ref(), &mut nodes);
            let mut failure = None;
            for art in &self.art {
                if let Err(error) = art.dispose() {
                    failure.get_or_insert(error);
                }
            }
            if let Err(error) = scrub_nodes(nodes) {
                failure.get_or_insert(error);
            }
            if let Err(error) = self.surface.dispose() {
                failure.get_or_insert(error);
            }
            self.root.remove();
            match failure {
                Some(error) => Err(error.into()),
                None => Ok(()),
            }
        }
    }
    impl Drop for PlayerJoinScreen {
        fn drop(&mut self) {
            // Explicit revoke reports cleanup errors; Drop still retires this
            // scope and scrubs/detaches all owned resources when no caller exists.
            if self.revoke().is_err() {
                self.root.remove();
            }
        }
    }
}
#[cfg(target_arch = "wasm32")]
pub use browser::{PlayerJoinError, PlayerJoinScreen};
