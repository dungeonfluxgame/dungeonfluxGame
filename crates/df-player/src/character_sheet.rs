//! Persistent personal-sheet presentation. Inputs are already authorized private
//! df-ui views; this boundary neither projects server records nor grants access.

/// Transport presentation, independent of inventory, progression and readiness.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlayerSheetConnection {
    Connected,
    Reconnecting,
    Offline,
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::PlayerSheetConnection;
    use df_ui::{
        CharacterSheetSurface, CharacterSheetView, SheetError, SheetOwnerGeneration,
        SheetSubmission,
    };
    use std::{cell::Cell, rc::Rc};
    use wasm_bindgen::JsCast;
    use web_sys::{Document, Element, HtmlButtonElement, HtmlElement};

    /// A retained player-sheet mount. df-ui owns keyed content, tabs, filter,
    /// expanded rows, focus reconciliation and exact offer selection. The role
    /// owner must revoke this mount when its private-view authorization ends.
    pub struct PlayerSheetScreen {
        document: Document,
        root: Element,
        connection_status: Element,
        surface: CharacterSheetSurface,
        owner: Option<SheetOwnerGeneration>,
        submission_enabled: Rc<Cell<bool>>,
        connection: PlayerSheetConnection,
        visible: bool,
        suspended_offers: Vec<(HtmlButtonElement, bool)>,
        resume_focus: Option<HtmlElement>,
    }

    impl PlayerSheetScreen {
        /// Mount a caller-filtered private view into a caller-owned slot. The
        /// callback receives the exact component submission; RPC is caller-owned.
        pub fn mount(
            document: &Document,
            slot: &Element,
            mount_id: &str,
            view: &CharacterSheetView<'_>,
            connection: PlayerSheetConnection,
            mut on_submit: impl FnMut(SheetSubmission) + 'static,
        ) -> Result<Self, SheetError> {
            let root = document.create_element("section")?;
            root.set_attribute("data-player-client", "character-sheet")?;
            let connection_status = document.create_element("p")?;
            connection_status.set_attribute("role", "status")?;
            connection_status.set_attribute("aria-live", "polite")?;
            connection_status.set_attribute("tabindex", "-1")?;
            connection_status.set_attribute("style", "margin:0;padding:12px 4vw;background:#211d17;color:#ead9b9;border-bottom:1px solid #6c5638;font:13px/1.5 system-ui,sans-serif;overflow-wrap:anywhere")?;
            root.append_child(&connection_status)?;
            let submission_enabled = Rc::new(Cell::new(false));
            let callback_gate = Rc::clone(&submission_enabled);
            let surface =
                CharacterSheetSurface::create(document, mount_id, view, move |submission| {
                    if callback_gate.get() {
                        on_submit(submission);
                    }
                })?;
            root.append_child(surface.root())?;
            let mut screen = Self {
                document: document.clone(),
                root,
                connection_status,
                surface,
                owner: Some(view.owner),
                submission_enabled,
                connection,
                visible: true,
                suspended_offers: Vec::new(),
                resume_focus: None,
            };
            screen.apply_connection()?;
            slot.append_child(screen.root())?;
            Ok(screen)
        }

        pub fn root(&self) -> &Element {
            &self.root
        }

        /// Accepted same-owner updates retain the component and local navigation.
        /// Owner replacement is terminal: the caller supplies a fresh authorized
        /// mount rather than keeping another player's local state or callbacks.
        pub fn update(&mut self, view: &CharacterSheetView<'_>) -> Result<(), SheetError> {
            let owner = self.owner.ok_or(SheetError::Disposed)?;
            self.submission_enabled.set(false);
            if owner != view.owner {
                self.revoke()?;
                return Err(SheetError::OwnerChanged);
            }
            match self.surface.update(view) {
                Ok(()) => {
                    // Rendering establishes the current advertised disabled state;
                    // never restore a previous revision's offer eligibility.
                    self.suspended_offers.clear();
                    self.apply_connection_or_revoke()
                }
                Err(SheetError::StaleView) => {
                    self.submission_enabled.set(self.can_submit());
                    Err(SheetError::StaleView)
                }
                Err(error) => {
                    self.revoke()?;
                    Err(error)
                }
            }
        }

        /// Connection loss suspends advertised offers, while local tabs, search
        /// and inspection stay usable. Reconnect does not advance game state.
        pub fn set_connection(
            &mut self,
            connection: PlayerSheetConnection,
        ) -> Result<(), SheetError> {
            if self.owner.is_none() {
                return Err(SheetError::Disposed);
            }
            self.submission_enabled.set(false);
            self.connection = connection;
            self.apply_connection_or_revoke()
        }

        /// In-place local panel navigation, with the same private scope retained.
        /// Visibility confers no permissions. The role shell revokes on access loss.
        pub fn set_visible(&mut self, visible: bool) -> Result<(), SheetError> {
            if self.owner.is_none() {
                return Err(SheetError::Disposed);
            }
            self.submission_enabled.set(false);
            if !visible && self.visible {
                self.capture_focus();
            }
            self.visible = visible;
            let result = if visible {
                self.surface.root().remove_attribute("hidden")
            } else {
                self.surface.root().set_attribute("hidden", "")
            };
            if let Err(error) = result {
                self.revoke()?;
                return Err(error.into());
            }
            self.apply_connection_or_revoke()
        }

        fn can_submit(&self) -> bool {
            self.owner.is_some()
                && self.visible
                && self.connection == PlayerSheetConnection::Connected
        }

        fn capture_focus(&mut self) {
            self.resume_focus = self
                .document
                .active_element()
                .filter(|node| self.surface.root().contains(Some(node)))
                .and_then(|node| node.dyn_into::<HtmlElement>().ok());
        }

        fn apply_connection_or_revoke(&mut self) -> Result<(), SheetError> {
            match self.apply_connection() {
                Ok(()) => Ok(()),
                Err(error) => {
                    self.revoke()?;
                    Err(error)
                }
            }
        }

        fn apply_connection(&mut self) -> Result<(), SheetError> {
            let connected = self.connection == PlayerSheetConnection::Connected;
            self.connection_status
                .set_text_content(Some(match self.connection {
                    PlayerSheetConnection::Connected => "Connected · personal character sheet",
                    PlayerSheetConnection::Reconnecting => {
                        "Reconnecting · offered actions suspended; local inspection available"
                    }
                    PlayerSheetConnection::Offline => {
                        "Offline · offered actions suspended; local inspection available"
                    }
                }));
            if !connected && self.suspended_offers.is_empty() {
                let focused = self.document.active_element();
                if focused
                    .as_ref()
                    .is_some_and(|node| node.has_attribute("data-sheet-offer"))
                {
                    self.capture_focus();
                }
                // Traverse only this bounded component; no document-wide lookup
                // or retained private content is copied into the client wrapper.
                let mut pending = vec![self.surface.root().clone()];
                while let Some(node) = pending.pop() {
                    if node.has_attribute("data-sheet-offer") {
                        let button =
                            node.clone().dyn_into::<HtmlButtonElement>().map_err(|_| {
                                wasm_bindgen::JsValue::from_str("sheet offer is not a button")
                            })?;
                        self.suspended_offers
                            .push((button.clone(), button.disabled()));
                        button.set_disabled(true);
                    }
                    let mut child = node.first_element_child();
                    while let Some(element) = child {
                        child = element.next_element_sibling();
                        pending.push(element);
                    }
                }
            } else if connected {
                for (button, disabled) in self.suspended_offers.drain(..) {
                    if self.surface.root().contains(Some(&button)) {
                        button.set_disabled(disabled);
                    }
                }
            }
            if self.visible && connected {
                if let Some(focus) = self.resume_focus.take() {
                    // A new user focus wins over restoring the pre-suspension one.
                    let idle = self.document.active_element().is_none_or(|active| {
                        active.tag_name() == "BODY"
                            || active.is_same_node(Some(&self.connection_status))
                    });
                    if idle && self.focus_usable(&focus)? {
                        focus.focus()?;
                    }
                }
            } else if !self.visible {
                self.connection_status
                    .dyn_ref::<HtmlElement>()
                    .ok_or(SheetError::FocusUnavailable)?
                    .focus()?;
            }
            self.submission_enabled.set(self.can_submit());
            Ok(())
        }

        fn focus_usable(&self, element: &HtmlElement) -> Result<bool, SheetError> {
            if !self.surface.root().contains(Some(element)) || element.matches(":disabled")? {
                return Ok(false);
            }
            let mut ancestor = element.parent_element();
            while let Some(node) = ancestor {
                if node.has_attribute("hidden") {
                    return Ok(false);
                }
                if node.is_same_node(Some(self.surface.root())) {
                    return Ok(true);
                }
                ancestor = node.parent_element();
            }
            Ok(false)
        }

        /// Terminal, idempotent teardown, including retained element/Text nodes
        /// owned by df-ui. Callback fencing always precedes fallible DOM cleanup.
        pub fn revoke(&mut self) -> Result<(), SheetError> {
            self.submission_enabled.set(false);
            self.owner = None;
            self.resume_focus = None;
            self.suspended_offers.clear();
            let cleanup = self.surface.dispose();
            self.connection_status.set_text_content(None);
            self.root.remove();
            cleanup
        }
    }

    impl Drop for PlayerSheetScreen {
        fn drop(&mut self) {
            // Explicit revoke reports errors; Drop still fences and asks the
            // component to scrub its private values before removing the mount.
            if self.revoke().is_err() {
                self.root.remove();
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::PlayerSheetScreen;
