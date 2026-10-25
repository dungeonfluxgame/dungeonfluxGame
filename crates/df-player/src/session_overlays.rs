//! Persistent player session presentation over an independently owned essential surface.
//! Props must already be authorized and filtered for player; this is not an RPC adapter.

#[cfg(target_arch = "wasm32")]
mod browser {
    use df_ui::{
        SessionOverlayError, SessionOverlayPhase, SessionOverlaySelection,
        SessionOverlayValidationError, SessionOverlayView,
    };
    use std::{cell::Cell, fmt, rc::Rc};
    use wasm_bindgen::JsCast;
    use web_sys::{Document, Element, HtmlElement, Node};

    #[derive(Debug)]
    pub enum SessionOverlayMountError {
        Surface(SessionOverlayError),
        EssentialParentMissing,
        Revoked,
        Cleanup {
            update: Box<SessionOverlayError>,
            cleanup: Box<SessionOverlayError>,
        },
    }
    impl fmt::Display for SessionOverlayMountError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Surface(error) => fmt::Display::fmt(error, formatter),
                Self::EssentialParentMissing => {
                    formatter.write_str("essential surface requires an owned parent")
                }
                Self::Revoked => formatter.write_str("session overlay mount is revoked"),
                Self::Cleanup { update, cleanup } => write!(
                    formatter,
                    "overlay update failed: {update}; cleanup failed: {cleanup}"
                ),
            }
        }
    }
    impl std::error::Error for SessionOverlayMountError {}
    impl From<SessionOverlayError> for SessionOverlayMountError {
        fn from(error: SessionOverlayError) -> Self {
            Self::Surface(error)
        }
    }

    /// A nonmodal session layer. The essential surface belongs to the caller and
    /// survives overlay teardown. Revoking the underlying role must separately
    /// revoke that surface; this mount cannot grant or revoke gameplay authority.
    pub struct PlayerSessionOverlays {
        document: Document,
        phase: SessionOverlayPhase,
        essential: Element,
        essential_parent: Node,
        essential_next: Option<Node>,
        generation: u64,
        active: Rc<Cell<bool>>,
    }
    impl PlayerSessionOverlays {
        /// Mount an existing essential component, not a substitute input. Public
        /// display callers must construct their projection independently of player data.
        pub fn mount(
            document: &Document,
            parent: &Element,
            essential: &Element,
            view: &SessionOverlayView<'_>,
            mut on_selection: impl FnMut(SessionOverlaySelection) + 'static,
        ) -> Result<Self, SessionOverlayMountError> {
            let essential_parent = essential
                .parent_node()
                .ok_or(SessionOverlayMountError::EssentialParentMissing)?;
            let essential_next = essential.next_sibling();
            let mount_focus = document
                .active_element()
                .filter(|node| essential.contains(Some(node)))
                .and_then(|node| node.dyn_into::<HtmlElement>().ok());
            let phase = SessionOverlayPhase::create(document, view)?;
            phase
                .root()
                .set_attribute("data-session-client", "player")
                .map_err(SessionOverlayError::from)?;
            let active = Rc::new(Cell::new(true));
            let callback_gate = Rc::clone(&active);
            phase.on_selection(move |selection| {
                if callback_gate.get() {
                    on_selection(selection);
                }
            })?;
            let mut mounted = Self {
                document: document.clone(),
                phase,
                essential: essential.clone(),
                essential_parent,
                essential_next,
                generation: view.generation,
                active,
            };
            // Once Self exists, Drop restores the essential surface if a fallible
            // insertion fails. No ownership of the essential callback is transferred.
            mounted
                .phase
                .essential_input()
                .append_child(essential)
                .map_err(SessionOverlayError::from)?;
            if let Err(error) = parent.append_child(mounted.root()) {
                let update = SessionOverlayError::from(error);
                return match mounted.revoke_surface() {
                    Ok(()) => Err(update.into()),
                    Err(cleanup) => Err(SessionOverlayMountError::Cleanup {
                        update: Box::new(update),
                        cleanup: Box::new(cleanup),
                    }),
                };
            }
            if let Some(focused) = mount_focus
                && focused.is_connected()
                && !focused
                    .matches(":disabled")
                    .map_err(SessionOverlayError::from)?
            {
                focused.focus().map_err(SessionOverlayError::from)?;
            }
            Ok(mounted)
        }
        pub fn root(&self) -> &Element {
            self.phase.root()
        }
        pub fn essential_root(&self) -> &Element {
            &self.essential
        }

        /// Updates reuse df-ui's keyed navigation, focus and offers. A generation
        /// replacement fences callbacks and scrubs the obsolete layer before returning;
        /// the owner must explicitly mount the new accepted scope.
        pub fn update(
            &mut self,
            view: &SessionOverlayView<'_>,
        ) -> Result<(), SessionOverlayMountError> {
            if !self.active.get() {
                return Err(SessionOverlayMountError::Revoked);
            }
            let update_result = if view.generation != self.generation {
                Err(SessionOverlayValidationError::WrongGeneration.into())
            } else {
                self.phase.update(view)
            };
            if let Err(update) = update_result {
                // Invalid ordinary views leave df-ui's current snapshot untouched.
                // Changed owners and partial DOM failures must fail closed.
                if matches!(&update, SessionOverlayError::Validation(error)
                    if *error != SessionOverlayValidationError::WrongGeneration
                        && *error != SessionOverlayValidationError::Disposed)
                {
                    return Err(update.into());
                }
                return match self.revoke_surface() {
                    Ok(()) => Err(update.into()),
                    Err(cleanup) => Err(SessionOverlayMountError::Cleanup {
                        update: Box::new(update),
                        cleanup: Box::new(cleanup),
                    }),
                };
            }
            Ok(())
        }

        /// Terminal and idempotent. Essential input, its draft and focus are retained;
        /// only this layer's prose, retained Text, listeners and offers are cleared.
        pub fn revoke(&mut self) -> Result<(), SessionOverlayMountError> {
            self.revoke_surface()
                .map_err(SessionOverlayMountError::from)
        }
        fn revoke_surface(&mut self) -> Result<(), SessionOverlayError> {
            self.active.set(false);
            let focused = self
                .document
                .active_element()
                .filter(|node| self.essential.contains(Some(node)))
                .and_then(|node| node.dyn_into::<HtmlElement>().ok());
            let mut failure = None;
            if self.phase.essential_input().contains(Some(&self.essential)) {
                let next_sibling = self.essential_next.as_ref().filter(|node| {
                    node.parent_node()
                        .is_some_and(|parent| parent.is_same_node(Some(&self.essential_parent)))
                });
                if let Err(error) = self
                    .essential_parent
                    .insert_before(&self.essential, next_sibling)
                {
                    failure = Some(SessionOverlayError::from(error));
                }
            }
            if let Err(error) = self.phase.dispose() {
                failure.get_or_insert(error);
            }
            if let Some(focused) = focused
                && focused.is_connected()
            {
                match focused.matches(":disabled") {
                    Ok(false) => {
                        if let Err(error) = focused.focus() {
                            failure.get_or_insert(SessionOverlayError::from(error));
                        }
                    }
                    Ok(true) => {}
                    Err(error) => {
                        failure.get_or_insert(SessionOverlayError::from(error));
                    }
                }
            }
            match failure {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
    }
    impl Drop for PlayerSessionOverlays {
        fn drop(&mut self) {
            // Explicit revoke reports failures. Drop still fences and clears the
            // overlay; df-ui's own Drop provides a second cleanup boundary.
            if self.revoke_surface().is_err() {
                self.phase.root().remove();
            }
        }
    }
}
#[cfg(target_arch = "wasm32")]
pub use browser::{PlayerSessionOverlays, SessionOverlayMountError};
