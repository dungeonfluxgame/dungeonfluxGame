//! Persistent display character-creation presentation. The caller supplies an
//! already authorized and audience-filtered projection; this is not an RPC adapter.

#[cfg(target_arch = "wasm32")]
use df_ui::{
    CharacterDisplayConnection, CharacterDisplayLimits, CharacterDisplaySubmission,
    CharacterDisplayView, CharacterValidationError,
};
#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use df_ui::{CharacterDisplaySurface, CharacterPhaseError};
    use std::{cell::Cell, rc::Rc};
    use wasm_bindgen::JsCast;
    use web_sys::{Document, Element, HtmlElement, HtmlFieldSetElement};

    /// One character-creation mount, retained by the role shell through updates
    /// and reconnect. Rendering, revision checks, drafts and asset lifetimes stay
    /// with df-ui. No navigation value grants access or generates game offers.
    pub struct DisplayCharacterScreen {
        document: Document,
        root: Element,
        status: Element,
        fieldset: HtmlFieldSetElement,
        surface: CharacterDisplaySurface,
        submission_enabled: Rc<Cell<bool>>,
        scope: Option<(u64, String)>,
        connection: CharacterDisplayConnection,
        resume_focus: Option<HtmlElement>,
    }

    impl DisplayCharacterScreen {
        /// Mount into a caller-owned slot. The callback emits exactly the supplied
        /// component's advertised submission; authorization and RPC remain caller-owned.
        pub fn mount(
            document: &Document,
            slot: &Element,
            view: &CharacterDisplayView,
            limits: CharacterDisplayLimits,
            connection: CharacterDisplayConnection,
            mut on_submit: impl FnMut(CharacterDisplaySubmission) + 'static,
        ) -> Result<Self, CharacterPhaseError> {
            let root = document.create_element("section")?;
            root.set_attribute("data-character-client", "display")?;
            let status = document.create_element("p")?;
            status.set_attribute("role", "status")?;
            status.set_attribute("aria-live", "polite")?;
            status.set_attribute("style", "margin:0;padding:12px 4vw;background:#211d17;color:#ead9b9;border-bottom:1px solid #6c5638;font:13px/1.5 system-ui,sans-serif;overflow-wrap:anywhere")?;
            let fieldset = document
                .create_element("fieldset")?
                .dyn_into::<HtmlFieldSetElement>()
                .map_err(|_| wasm_bindgen::JsValue::from_str("character fieldset unavailable"))?;
            fieldset.set_attribute("aria-label", "Display character creation")?;
            fieldset.set_attribute("style", "border:0;padding:0;margin:0;min-inline-size:0")?;
            root.append_child(&status)?;
            root.append_child(&fieldset)?;
            let submission_enabled = Rc::new(Cell::new(
                connection == CharacterDisplayConnection::Connected,
            ));
            let callback_gate = Rc::clone(&submission_enabled);
            let surface =
                CharacterDisplaySurface::create(document, view, limits, move |submission| {
                    if callback_gate.get() {
                        on_submit(submission);
                    }
                })?;
            fieldset.append_child(surface.root())?;
            let mut screen = Self {
                document: document.clone(),
                root,
                status,
                fieldset,
                surface,
                submission_enabled,
                scope: Some((view.generation, view.public_scope_key.clone())),
                connection,
                resume_focus: None,
            };
            screen.set_connection(connection)?;
            slot.append_child(screen.root())?;
            Ok(screen)
        }

        pub fn root(&self) -> &Element {
            &self.root
        }

        /// Same-scope updates keep the same component and valid drafts/focus.
        /// A new ownership scope fences old callbacks before fallible validation
        /// or rendering. Failed replacement revokes rather than exposing old data.
        pub fn update(&mut self, view: &CharacterDisplayView) -> Result<(), CharacterPhaseError> {
            let current_scope = self
                .scope
                .as_ref()
                .ok_or(CharacterValidationError::Disposed)?;
            let is_new_scope =
                current_scope.0 != view.generation || current_scope.1 != view.public_scope_key;
            if is_new_scope {
                self.resume_focus = None;
                self.submission_enabled.set(false);
            }
            match self.surface.update(view) {
                Ok(()) => {
                    if is_new_scope {
                        self.scope = Some((view.generation, view.public_scope_key.clone()));
                    }
                    self.submission_enabled
                        .set(self.connection == CharacterDisplayConnection::Connected);
                    Ok(())
                }
                Err(update) => {
                    if !is_new_scope && matches!(&update, CharacterPhaseError::InvalidView(_)) {
                        return Err(update);
                    }
                    match self.revoke() {
                        Ok(()) => Err(update),
                        Err(cleanup) => Err(CharacterPhaseError::UpdateCleanup {
                            update: Box::new(update),
                            cleanup: Box::new(cleanup),
                        }),
                    }
                }
            }
        }

        /// Suspend input while keeping this mount and drafts. Reconnect resumes
        /// only the current scope; callers must revoke when authorization is lost.
        /// This connection state never changes reported server readiness or revision.
        pub fn set_connection(
            &mut self,
            connection: CharacterDisplayConnection,
        ) -> Result<(), CharacterPhaseError> {
            if self.scope.is_none() {
                return Err(CharacterValidationError::Disposed.into());
            }
            if connection != CharacterDisplayConnection::Connected
                && self.connection == CharacterDisplayConnection::Connected
            {
                self.resume_focus = self
                    .document
                    .active_element()
                    .filter(|node| self.root.contains(Some(node)))
                    .and_then(|node| node.dyn_into::<HtmlElement>().ok());
            }
            self.connection = connection;
            let connected = connection == CharacterDisplayConnection::Connected;
            self.submission_enabled.set(connected);
            self.fieldset.set_disabled(!connected);
            self.status.set_text_content(Some(match connection {
                CharacterDisplayConnection::Connected => "Connected · display character creation",
                CharacterDisplayConnection::Reconnecting => {
                    "Reconnecting · character input suspended"
                }
                CharacterDisplayConnection::Offline => "Offline · character input suspended",
            }));
            if connected
                && let Some(element) = self.resume_focus.take()
                && element.is_connected()
                && !element.matches(":disabled")?
            {
                element.focus()?;
            }
            Ok(())
        }

        /// Explicit optional portrait retry does not submit an action.
        pub fn retry_failed_portraits(&mut self) -> Result<(), CharacterPhaseError> {
            if self.scope.is_none() {
                return Err(CharacterValidationError::Disposed.into());
            }
            match self.surface.retry_failed_portraits() {
                Ok(()) => Ok(()),
                Err(update) => match self.revoke() {
                    Ok(()) => Err(update),
                    Err(cleanup) => Err(CharacterPhaseError::UpdateCleanup {
                        update: Box::new(update),
                        cleanup: Box::new(cleanup),
                    }),
                },
            }
        }

        /// Terminal, idempotent teardown. Fencing precedes private DOM cleanup.
        pub fn revoke(&mut self) -> Result<(), CharacterPhaseError> {
            self.submission_enabled.set(false);
            self.scope = None;
            self.resume_focus = None;
            self.fieldset.set_disabled(true);
            let cleanup = self.surface.dispose();
            self.status.set_text_content(None);
            self.root.remove();
            cleanup
        }
    }

    impl Drop for DisplayCharacterScreen {
        fn drop(&mut self) {
            // Explicit revoke returns cleanup errors. Drop still fences, scrubs
            // the component and detaches the mount when reporting is unavailable.
            if self.revoke().is_err() {
                self.root.remove();
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::DisplayCharacterScreen;
