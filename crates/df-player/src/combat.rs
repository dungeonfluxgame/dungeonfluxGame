//! Persistent player combat presentation. Callers supply separately server-filtered
//! props; this boundary neither grants permission nor computes game outcomes.

#[cfg(target_arch = "wasm32")]
mod browser {
    use df_client::revisions::{ViewAcceptance, ViewStore};
    use df_types::{ClientBindingId, SessionRevision};
    use df_ui::{CombatError, CombatIntent, CombatLimits, CombatPhaseSurface, CombatPhaseView};
    use std::{
        cell::{Cell, RefCell},
        fmt,
        rc::Rc,
    };
    use wasm_bindgen::JsCast;
    use web_sys::{Document, Element, HtmlElement, Node};

    #[derive(Debug)]
    pub enum CombatMountError {
        Surface(CombatError),
        Admission(ViewAcceptance),
        Revoked,
    }
    impl fmt::Display for CombatMountError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Surface(error) => fmt::Display::fmt(error, formatter),
                Self::Admission(admission) => {
                    write!(formatter, "combat view was not accepted: {admission:?}")
                }
                Self::Revoked => formatter.write_str("combat mount is revoked"),
            }
        }
    }
    impl std::error::Error for CombatMountError {}
    impl From<CombatError> for CombatMountError {
        fn from(error: CombatError) -> Self {
            Self::Surface(error)
        }
    }

    type Callback = Box<dyn FnMut(CombatIntent)>;
    struct Dispatch {
        is_enabled: Cell<bool>,
        is_revoked: Cell<bool>,
        callback: RefCell<Option<Callback>>,
    }

    /// Erase existing Text/attributes in place before detaching nodes. Clearing
    /// only a parent would leave externally retained private descendants readable.
    fn scrub(node: &Node) -> Result<(), CombatError> {
        let mut failure = None;
        let mut next = node.first_child();
        while let Some(child) = next {
            next = child.next_sibling();
            if let Err(error) = scrub(&child)
                && failure.is_none()
            {
                failure = Some(error);
            }
        }
        if let Some(element) = node.dyn_ref::<Element>() {
            let names = element.get_attribute_names();
            for index in 0..names.length() {
                if let Some(name) = names.get(index).as_string()
                    && let Err(error) = element.remove_attribute(&name)
                    && failure.is_none()
                {
                    failure = Some(CombatError::from(error));
                }
            }
        }
        node.set_node_value(Some(""));
        node.set_text_content(None);
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn retain_descendants(node: &Node, retained: &mut Vec<Node>) {
        let mut next = node.first_child();
        while let Some(child) = next {
            next = child.next_sibling();
            retain_descendants(&child, retained);
            retained.push(child);
        }
    }
    fn scrub_retired(root: &Element, retained: Vec<Node>) -> Result<(), CombatError> {
        let mut failure = None;
        for node in retained {
            if !root.contains(Some(&node))
                && let Err(error) = scrub(&node)
                && failure.is_none()
            {
                failure = Some(error);
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// One authorized binding's persistent mount. Ordinary snapshots retain the
    /// surface's keyed controls/focus/drafts. A new recovery epoch replaces only
    /// the obsolete scoped subtree and erases its private presentation state.
    pub struct PlayerCombat {
        document: Document,
        root: Element,
        draft_identifier: String,
        binding: ClientBindingId,
        limits: CombatLimits,
        order: RefCell<ViewStore<()>>,
        phase: RefCell<Option<Rc<CombatPhaseSurface>>>,
        dispatch: Rc<Dispatch>,
        focused: RefCell<Option<HtmlElement>>,
    }
    impl PlayerCombat {
        /// The supplied props must already be filtered for player. The callback
        /// receives the exact advertised intent; submission is not an outcome.
        pub fn mount(
            document: &Document,
            parent: &Element,
            draft_identifier: &str,
            binding_revision: (ClientBindingId, SessionRevision),
            view: &CombatPhaseView<'_>,
            limits: CombatLimits,
            on_intent: impl FnMut(CombatIntent) + 'static,
        ) -> Result<Self, CombatMountError> {
            let (binding, revision) = binding_revision;
            view.validate(limits).map_err(CombatError::from)?;
            let root = document
                .create_element("section")
                .map_err(CombatError::from)?;
            root.set_attribute("data-combat-client", "player")
                .map_err(CombatError::from)?;
            let mount = Self {
                document: document.clone(),
                root,
                draft_identifier: draft_identifier.to_owned(),
                binding,
                limits,
                order: RefCell::new(ViewStore::new(binding)),
                phase: RefCell::new(None),
                focused: RefCell::new(None),
                dispatch: Rc::new(Dispatch {
                    is_enabled: Cell::new(false),
                    is_revoked: Cell::new(false),
                    callback: RefCell::new(Some(Box::new(on_intent))),
                }),
            };
            mount.update(binding, revision, view)?;
            parent
                .append_child(mount.root())
                .map_err(CombatError::from)?;
            Ok(mount)
        }
        pub fn root(&self) -> &Element {
            &self.root
        }
        pub fn revision(&self) -> Option<SessionRevision> {
            self.order.borrow().current().map(|(revision, ())| revision)
        }
        fn retain_focus(&self) {
            if let Some(active) = self.document.active_element()
                && self.root.contains(Some(&active))
                && let Ok(active) = active.dyn_into::<HtmlElement>()
            {
                *self.focused.borrow_mut() = Some(active);
            }
        }
        fn restore_focus(&self) -> Result<(), CombatError> {
            let focused = self.focused.borrow_mut().take();
            if let Some(focused) = focused
                && self.root.contains(Some(&focused))
                && !focused.has_attribute("disabled")
                && !self
                    .document
                    .active_element()
                    .is_some_and(|active| active.is_same_node(Some(&focused)))
            {
                focused.focus()?;
            }
            Ok(())
        }
        fn fresh_phase(
            &self,
            revision: SessionRevision,
            view: &CombatPhaseView<'_>,
        ) -> Result<Rc<CombatPhaseSurface>, CombatError> {
            let phase = Rc::new(CombatPhaseSurface::create(
                &self.document,
                &self.draft_identifier,
                self.binding,
                revision,
                view,
                self.limits,
            )?);
            let dispatch = Rc::clone(&self.dispatch);
            phase.on_intent(move |intent| {
                if !dispatch.is_enabled.get() || dispatch.is_revoked.get() {
                    return;
                }
                let callback = dispatch.callback.borrow_mut().take();
                if let Some(mut callback) = callback {
                    // No borrow crosses application code, which may update/revoke.
                    callback(intent);
                    if !dispatch.is_revoked.get() {
                        *dispatch.callback.borrow_mut() = Some(callback);
                    }
                }
            })?;
            self.root.append_child(phase.root())?;
            Ok(phase)
        }
        /// Valid ordered snapshots update in place. Failed rendering retains the
        /// attempted watermark and suspends submissions; a newer snapshot recovers.
        pub fn update(
            &self,
            binding: ClientBindingId,
            revision: SessionRevision,
            view: &CombatPhaseView<'_>,
        ) -> Result<(), CombatMountError> {
            if self.dispatch.is_revoked.get() {
                return Err(CombatMountError::Revoked);
            }
            view.validate(self.limits).map_err(CombatError::from)?;
            let previous = self.revision();
            let admission = self.order.borrow_mut().accept(binding, revision, ());
            if admission != ViewAcceptance::Applied {
                return Err(CombatMountError::Admission(admission));
            }
            self.retain_focus();
            self.dispatch.is_enabled.set(false);
            self.root
                .set_attribute("inert", "")
                .map_err(CombatError::from)?;
            let replace = previous.is_some_and(|previous| previous.epoch() != revision.epoch());
            if replace {
                let old = self.phase.borrow_mut().take();
                if let Some(old) = old {
                    // Controls clear draft values/listeners; scrub clears retained
                    // prose/attributes even if a later removal reports an error.
                    let scrubbed = scrub(old.root());
                    let disposed = old.dispose();
                    scrubbed?;
                    disposed?;
                }
            }
            let existing = self.phase.borrow().as_ref().map(Rc::clone);
            if let Some(phase) = existing {
                let mut retained = Vec::new();
                retain_descendants(phase.root(), &mut retained);
                let updated = phase.update(binding, revision, view);
                // Detached old text/actor/roll/offer nodes may still have callers;
                // erase them even when a partially rendered update failed.
                let retired = scrub_retired(phase.root(), retained);
                let admission = updated?;
                retired?;
                if admission != ViewAcceptance::Applied {
                    return Err(CombatMountError::Admission(admission));
                }
            } else {
                let phase = self.fresh_phase(revision, view)?;
                *self.phase.borrow_mut() = Some(phase);
            }
            self.root
                .remove_attribute("inert")
                .map_err(CombatError::from)?;
            self.restore_focus()?;
            self.dispatch.is_enabled.set(true);
            Ok(())
        }
        /// Offline/reconnecting presentation disables even synthetic events while
        /// retaining valid local focus/drafts. Only a newer accepted view resumes.
        pub fn suspend_input(&self) -> Result<(), CombatMountError> {
            if self.dispatch.is_revoked.get() {
                return Err(CombatMountError::Revoked);
            }
            self.retain_focus();
            self.dispatch.is_enabled.set(false);
            self.root
                .set_attribute("inert", "")
                .map_err(CombatError::from)?;
            Ok(())
        }
        /// Repeatable terminal disposal clears caller-retained private DOM and
        /// drafts before releasing listeners. Old offers cannot submit afterward.
        pub fn revoke(&self) -> Result<(), CombatMountError> {
            self.dispatch.is_revoked.set(true);
            self.dispatch.is_enabled.set(false);
            self.dispatch.callback.borrow_mut().take();
            self.focused.borrow_mut().take();
            let mut failure = None;
            let phase = self.phase.borrow_mut().take();
            if let Some(phase) = phase {
                if let Err(error) = scrub(phase.root()) {
                    failure = Some(error);
                }
                if let Err(error) = phase.dispose()
                    && failure.is_none()
                {
                    failure = Some(error);
                }
            }
            if let Err(error) = scrub(&self.root)
                && failure.is_none()
            {
                failure = Some(error);
            }
            if let Some(parent) = self.root.parent_node()
                && let Err(error) = parent.remove_child(&self.root)
                && failure.is_none()
            {
                failure = Some(CombatError::from(error));
            }
            match failure {
                Some(error) => Err(error.into()),
                None => Ok(()),
            }
        }
    }
    impl Drop for PlayerCombat {
        fn drop(&mut self) {
            // Errors cannot escape Drop; fencing and in-place scrubbing already
            // ran before fallible removal. Explicit revoke reports cleanup errors.
            if self.revoke().is_err() {
                self.root.set_text_content(None);
            }
        }
    }
}
#[cfg(target_arch = "wasm32")]
pub use browser::{CombatMountError, PlayerCombat};
