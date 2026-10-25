//! Persistent local composition of existing role mounts. The caller owns the
//! authorized connection and supplies its current audience-filtered presentation;
//! this module does not implement boot, credentials, RPC, audio or game authority.
mod mounted;
mod overlay;
mod presentation;

use df_client::revisions::{ViewAcceptance, ViewStore};
use df_types::{ClientBindingId, SessionRevision};
use df_ui::{SessionConnection, SessionOverlayView};
use mounted::MountedPhase;
use overlay::MountedOverlay;
use presentation::ScopeKey;
pub use presentation::{DisplayPhase, PlayerPhase, RoleInput, RolePhase};
use std::{
    cell::{Cell, RefCell},
    fmt,
    rc::Rc,
};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, Element, HtmlElement, HtmlFieldSetElement, HtmlInputElement};

#[derive(Debug)]
pub enum RoleShellError {
    Dom(JsValue),
    Disposed,
    GenerationExhausted,
    SnapshotMismatch,
    /// The shared watermark refused a view that the checks above had admitted.
    ViewRefused(ViewAcceptance),
    InvalidInputIdentifier,
    PlayerJoin(df_player::PlayerJoinError),
    DisplayJoin(df_display::DisplayJoinError),
    Character(df_ui::CharacterPhaseError),
    Sheet(df_ui::SheetError),
    PlayerExploration(df_player::ExplorationMountError),
    DisplayExploration(df_display::ExplorationMountError),
    PlayerCombat(df_player::CombatMountError),
    DisplayCombat(df_display::CombatMountError),
    Aftermath(df_ui::AftermathError),
    Campfire(df_ui::CampfireError),
    Transition(df_ui::SceneTransitionError),
    PlayerOverlay(df_player::SessionOverlayMountError),
    DisplayOverlay(df_display::SessionOverlayMountError),
    Cleanup {
        operation: Box<RoleShellError>,
        cleanup: Box<RoleShellError>,
    },
}
impl fmt::Display for RoleShellError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dom(_) => formatter.write_str("role shell DOM operation failed"),
            Self::Disposed => formatter.write_str("role shell is disposed"),
            Self::GenerationExhausted => formatter.write_str("role shell generation exhausted"),
            Self::InvalidInputIdentifier => {
                formatter.write_str("role shell input identifier is invalid")
            }
            Self::SnapshotMismatch => {
                formatter.write_str("role presentation does not match its accepted snapshot")
            }
            Self::ViewRefused(acceptance) => {
                write!(formatter, "role view admission refused: {acceptance:?}")
            }
            Self::PlayerJoin(error) => fmt::Display::fmt(error, formatter),
            Self::DisplayJoin(error) => fmt::Display::fmt(error, formatter),
            Self::Character(error) => fmt::Display::fmt(error, formatter),
            Self::Sheet(error) => fmt::Display::fmt(error, formatter),
            Self::PlayerExploration(error) => fmt::Display::fmt(error, formatter),
            Self::DisplayExploration(error) => fmt::Display::fmt(error, formatter),
            Self::PlayerCombat(error) => fmt::Display::fmt(error, formatter),
            Self::DisplayCombat(error) => fmt::Display::fmt(error, formatter),
            Self::Aftermath(error) => fmt::Display::fmt(error, formatter),
            Self::Campfire(error) => fmt::Display::fmt(error, formatter),
            Self::Transition(error) => fmt::Display::fmt(error, formatter),
            Self::PlayerOverlay(error) => fmt::Display::fmt(error, formatter),
            Self::DisplayOverlay(error) => fmt::Display::fmt(error, formatter),
            Self::Cleanup { operation, cleanup } => {
                write!(formatter, "{operation}; cleanup failed: {cleanup}")
            }
        }
    }
}
impl std::error::Error for RoleShellError {}
impl From<JsValue> for RoleShellError {
    fn from(error: JsValue) -> Self {
        Self::Dom(error)
    }
}
impl From<df_player::PlayerJoinError> for RoleShellError {
    fn from(error: df_player::PlayerJoinError) -> Self {
        Self::PlayerJoin(error)
    }
}
impl From<df_display::DisplayJoinError> for RoleShellError {
    fn from(error: df_display::DisplayJoinError) -> Self {
        Self::DisplayJoin(error)
    }
}
impl From<df_ui::CharacterPhaseError> for RoleShellError {
    fn from(error: df_ui::CharacterPhaseError) -> Self {
        Self::Character(error)
    }
}
impl From<df_ui::SheetError> for RoleShellError {
    fn from(error: df_ui::SheetError) -> Self {
        Self::Sheet(error)
    }
}
impl From<df_player::ExplorationMountError> for RoleShellError {
    fn from(error: df_player::ExplorationMountError) -> Self {
        Self::PlayerExploration(error)
    }
}
impl From<df_display::ExplorationMountError> for RoleShellError {
    fn from(error: df_display::ExplorationMountError) -> Self {
        Self::DisplayExploration(error)
    }
}
impl From<df_player::CombatMountError> for RoleShellError {
    fn from(error: df_player::CombatMountError) -> Self {
        Self::PlayerCombat(error)
    }
}
impl From<df_display::CombatMountError> for RoleShellError {
    fn from(error: df_display::CombatMountError) -> Self {
        Self::DisplayCombat(error)
    }
}
impl From<df_player::SessionOverlayMountError> for RoleShellError {
    fn from(error: df_player::SessionOverlayMountError) -> Self {
        Self::PlayerOverlay(error)
    }
}
impl From<df_display::SessionOverlayMountError> for RoleShellError {
    fn from(error: df_display::SessionOverlayMountError) -> Self {
        Self::DisplayOverlay(error)
    }
}

impl From<df_ui::AftermathError> for RoleShellError {
    fn from(error: df_ui::AftermathError) -> Self {
        Self::Aftermath(error)
    }
}
impl From<df_ui::CampfireError> for RoleShellError {
    fn from(error: df_ui::CampfireError) -> Self {
        Self::Campfire(error)
    }
}
impl From<df_ui::SceneTransitionError> for RoleShellError {
    fn from(error: df_ui::SceneTransitionError) -> Self {
        Self::Transition(error)
    }
}

type InputCallback = Box<dyn FnMut(RoleInput)>;
struct Dispatch {
    generation: Cell<Option<u64>>,
    enabled: Cell<bool>,
    disposed: Cell<bool>,
    callback: RefCell<Option<InputCallback>>,
}
impl Dispatch {
    fn emit(&self, generation: u64, input: RoleInput) {
        if !self.enabled.get() || self.generation.get() != Some(generation) {
            return;
        }
        // Release the RefCell borrow before user code: a current selection may
        // synchronously replace or dispose its shell through the connection owner.
        let saved = self.callback.borrow_mut().take();
        if let Some(mut callback) = saved {
            callback(input);
            if !self.disposed.get() {
                *self.callback.borrow_mut() = Some(callback);
            }
        }
    }
}

struct SuspendedFocus {
    element: HtmlElement,
    selection: Option<(u32, u32)>,
}

/// One binding's mounted presentation shell. Binding and revision are ordering
/// correlations, never credentials. Rebinding requires disposal and a fresh shell.
/// Role/phase replacements retain this root and retire old callback generations
/// before any fallible DOM operation. No navigation operation submits an action.
pub struct RoleShell {
    document: Document,
    root: Element,
    status: Element,
    slot: HtmlFieldSetElement,
    input_identifier: String,
    binding: ClientBindingId,
    order: ViewStore<()>,
    key: Option<ScopeKey>,
    mounted: Option<MountedPhase>,
    overlay: Option<MountedOverlay>,
    overlay_generation: Option<u64>,
    generation: u64,
    dispatch: Rc<Dispatch>,
    connection: SessionConnection,
    suspended_focus: Option<SuspendedFocus>,
    disposed: bool,
}
impl RoleShell {
    /// Create an empty shell in a caller-owned slot. The callback must forward or
    /// enqueue exact selections; nested synthetic callback invocation is unsupported.
    /// Synchronous shell replacement or disposal from the callback is supported.
    /// This function is composition, not production startup validation or admission.
    pub fn mount(
        document: &Document,
        parent: &Element,
        binding: ClientBindingId,
        input_identifier: &str,
        on_input: impl FnMut(RoleInput) + 'static,
    ) -> Result<Self, RoleShellError> {
        if input_identifier.is_empty()
            || input_identifier.len() > 128
            || !input_identifier
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(RoleShellError::InvalidInputIdentifier);
        }
        let root = document.create_element("main")?;
        root.set_attribute("data-role-shell", "")?;
        root.set_attribute(
            "style",
            "min-inline-size:0;color:#d6c7ab;font-family:system-ui,sans-serif",
        )?;
        let status = document.create_element("p")?;
        status.set_attribute("role", "status")?;
        status.set_attribute("aria-live", "polite")?;
        status.set_attribute("style", "margin:0;padding:8px 4%;background:#0a1521;color:#c8b992;border-block-end:1px solid #34404a;font-size:12px;line-height:1.5;overflow-wrap:anywhere")?;
        status.set_text_content(Some("Waiting for a permitted view"));
        let slot = document
            .create_element("fieldset")?
            .dyn_into::<HtmlFieldSetElement>()
            .map_err(|_| RoleShellError::SnapshotMismatch)?;
        slot.set_attribute("style", "border:0;padding:0;margin:0;min-inline-size:0")?;
        slot.set_attribute("aria-label", "Current game presentation")?;
        slot.set_disabled(true);
        root.append_child(&status)?;
        root.append_child(&slot)?;
        parent.append_child(&root)?;
        Ok(Self {
            document: document.clone(),
            root,
            status,
            slot,
            input_identifier: input_identifier.to_owned(),
            binding,
            order: ViewStore::new(binding),
            key: None,
            mounted: None,
            overlay: None,
            overlay_generation: None,
            generation: 0,
            dispatch: Rc::new(Dispatch {
                generation: Cell::new(None),
                enabled: Cell::new(false),
                disposed: Cell::new(false),
                callback: RefCell::new(Some(Box::new(on_input))),
            }),
            connection: SessionConnection::Connecting,
            suspended_focus: None,
            disposed: false,
        })
    }
    pub fn root(&self) -> &Element {
        &self.root
    }
    pub fn revision(&self) -> Option<SessionRevision> {
        self.order.current().map(|(revision, ())| revision)
    }

    /// Supply a newer complete presentation mapped from the current authorized
    /// connection view. Stale/duplicate/wrong-binding snapshots make no DOM change.
    /// Failed rendering retains the attempted watermark, visibly suspends input,
    /// and needs a newer snapshot. Same-phase updates preserve valid keyed state.
    /// Epoch, owner, role or phase changes dispose obsolete private surfaces first.
    pub fn present(
        &mut self,
        binding: ClientBindingId,
        revision: SessionRevision,
        phase: RolePhase<'_>,
        overlay: Option<&SessionOverlayView<'_>>,
    ) -> Result<ViewAcceptance, RoleShellError> {
        if self.disposed {
            return Err(RoleShellError::Disposed);
        }
        if binding != self.binding {
            return Ok(ViewAcceptance::WrongBinding);
        }
        if let Some(current) = self.revision() {
            if revision < current {
                return Ok(ViewAcceptance::Stale { current });
            }
            if revision == current {
                return Ok(ViewAcceptance::Duplicate { current });
            }
        }
        let replaced = !phase.matches_key(self.key.as_ref())
            || self
                .revision()
                .is_some_and(|current| current.epoch() != revision.epoch());
        let was_enabled = self.dispatch.enabled.replace(false);
        let was_disabled = self.slot.disabled();
        // Fencing callbacks is sufficient during a synchronous same-phase update.
        // Disabling its ancestor first would blur the input before its adapter can
        // capture focus. Replacement has no valid old focus to retain.
        if replaced {
            self.slot.set_disabled(true);
            if let Err(error) = self.retire() {
                return Err(self.failed(error));
            }
        }
        if self.overlay_generation != overlay.map(|view| view.generation)
            && let Err(error) = self.retire_overlay()
        {
            return Err(self.failed(error));
        }
        if let Err(error) = mounted::validate(&phase, binding, revision).and_then(|()| {
            overlay.map_or(Ok(()), |view| {
                view.validate().map_err(|error| {
                    let surface = df_ui::SessionOverlayError::from(error);
                    if phase.is_player() {
                        df_player::SessionOverlayMountError::from(surface).into()
                    } else {
                        df_display::SessionOverlayMountError::from(surface).into()
                    }
                })
            })
        }) {
            if replaced {
                return Err(self.failed(error));
            }
            self.slot.set_disabled(was_disabled);
            self.dispatch.enabled.set(was_enabled);
            return Err(error);
        }
        let key = phase.key();
        // Invariant: the stale, duplicate and binding checks above read this same watermark, so
        // an accepted view is never refused here. A refusal would leave dispatch fenced with the
        // slot disabled and no mounted phase, so it fails the presentation closed and keeps the
        // refused admission as its cause.
        let admission = self.order.accept(binding, revision, ());
        if admission != ViewAcceptance::Applied {
            return Err(self.failed(RoleShellError::ViewRefused(admission)));
        }
        let player = phase.is_player();
        if !replaced && self.suspended_focus.is_some() {
            match self.capture_focus() {
                Ok(Some(current)) => self.suspended_focus = Some(current),
                Ok(None) => {}
                Err(error) => return Err(self.failed(error)),
            }
        }
        // Combat restores its captured focus during update, and exploration
        // renders into the same owned input. Their ancestor must be enabled now,
        // after validated admission, with shell dispatch still fenced.
        // A suspended adapter can retain its own old focus. Keep its ancestor
        // disabled during update if focus moved elsewhere, so that restoration
        // cannot steal focus from another visible UI.
        let competing_focus = self.suspended_focus.as_ref().is_some_and(|focus| {
            self.document.active_element().is_some_and(|active| {
                active.tag_name() != "BODY"
                    && !active.is_same_node(Some(&self.status))
                    && !active.is_same_node(Some(&focus.element))
            })
        });
        self.slot.set_disabled(competing_focus);
        let result = match self.mounted.as_mut() {
            Some(mounted) => mounted.update(binding, revision, phase),
            None => self.mount_phase(binding, revision, phase, key),
        };
        if let Err(error) = result {
            return Err(self.failed(error));
        }
        self.slot.set_disabled(false);
        if let Err(error) = self.apply_overlay(player, overlay) {
            return Err(self.failed(error));
        }
        self.connection = overlay.map_or(SessionConnection::Connected, |view| view.connection);
        self.status.set_text_content(Some(match self.connection {
            SessionConnection::Connected => "Connected",
            SessionConnection::Connecting => "Connecting · input suspended",
            SessionConnection::Reconnecting => "Reconnecting · input suspended",
            SessionConnection::Offline => "Offline · input suspended",
        }));
        if let Some(mounted) = &mut self.mounted
            && let Err(error) = mounted.connection(self.connection)
        {
            return Err(self.failed(error));
        }
        if self.connection == SessionConnection::Connected
            && let Err(error) = self.restore_suspended_focus()
        {
            return Err(self.failed(error));
        }
        self.resume_gate();
        Ok(admission)
    }

    fn mount_phase(
        &mut self,
        binding: ClientBindingId,
        revision: SessionRevision,
        phase: RolePhase<'_>,
        key: ScopeKey,
    ) -> Result<(), RoleShellError> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(RoleShellError::GenerationExhausted)?;
        self.dispatch.generation.set(Some(self.generation));
        let mounted = MountedPhase::mount(
            &self.document,
            self.slot.as_ref(),
            &self.input_identifier,
            (binding, revision),
            phase,
            Rc::clone(&self.dispatch),
            self.generation,
        )?;
        self.mounted = Some(mounted);
        self.key = Some(key);
        Ok(())
    }
    fn apply_overlay(
        &mut self,
        player: bool,
        view: Option<&SessionOverlayView<'_>>,
    ) -> Result<(), RoleShellError> {
        if self.overlay_generation != view.map(|view| view.generation) {
            self.retire_overlay()?;
        }
        match (self.overlay.as_mut(), view) {
            (Some(mounted), Some(view)) => mounted.update(view),
            (None, Some(view)) => {
                let essential = self
                    .mounted
                    .as_ref()
                    .ok_or(RoleShellError::SnapshotMismatch)?
                    .root();
                self.overlay = Some(MountedOverlay::mount(
                    &self.document,
                    self.slot.as_ref(),
                    essential,
                    player,
                    view,
                    Rc::clone(&self.dispatch),
                    self.generation,
                )?);
                self.overlay_generation = Some(view.generation);
                Ok(())
            }
            (_, None) => self.retire_overlay(),
        }
    }
    fn resume_gate(&self) {
        let enabled = self.mounted.is_some() && self.connection == SessionConnection::Connected;
        self.slot
            .set_disabled(!enabled && !matches!(self.mounted, Some(MountedPhase::Sheet(_))));
        self.dispatch.enabled.set(enabled);
    }
    /// Suspend on connection loss or hidden-tab recovery without removing nodes
    /// or clearing valid drafts. Setting Connected cannot revive old offers; only
    /// present with a newer accepted snapshot resumes input.
    pub fn suspend(&mut self, connection: SessionConnection) -> Result<(), RoleShellError> {
        if self.disposed {
            return Err(RoleShellError::Disposed);
        }
        self.dispatch.enabled.set(false);
        match self.capture_focus() {
            Ok(Some(current)) => self.suspended_focus = Some(current),
            Ok(None) => {}
            Err(error) => return Err(self.failed(error)),
        }
        self.connection = if connection == SessionConnection::Connected {
            SessionConnection::Reconnecting
        } else {
            connection
        };
        self.status.set_text_content(Some(match self.connection {
            SessionConnection::Offline => "Offline · awaiting a current permitted view",
            _ => "Reconnecting · awaiting a current permitted view",
        }));
        if let Some(mounted) = &mut self.mounted
            && let Err(error) = mounted.connection(self.connection)
        {
            return Err(self.failed(error));
        }
        self.slot
            .set_disabled(!matches!(self.mounted, Some(MountedPhase::Sheet(_))));
        Ok(())
    }
    fn capture_focus(&self) -> Result<Option<SuspendedFocus>, RoleShellError> {
        let Some(active) = self
            .document
            .active_element()
            .filter(|active| self.root.contains(Some(active)))
        else {
            return Ok(None);
        };
        let Ok(element) = active.dyn_into::<HtmlElement>() else {
            return Ok(None);
        };
        let selection = match element.dyn_ref::<HtmlInputElement>() {
            Some(input) => match (input.selection_start()?, input.selection_end()?) {
                (Some(start), Some(end)) => Some((start, end)),
                _ => None,
            },
            None => None,
        };
        Ok(Some(SuspendedFocus { element, selection }))
    }
    fn restore_suspended_focus(&mut self) -> Result<(), RoleShellError> {
        let Some(focus) = self.suspended_focus.take() else {
            return Ok(());
        };
        if !focus.element.is_connected()
            || !self.root.contains(Some(&focus.element))
            || focus.element.offset_width() == 0
            || focus.element.offset_height() == 0
            || focus.element.matches(":disabled")?
            || focus.element.closest("[hidden], [inert]")?.is_some()
        {
            return Ok(());
        }
        let may_restore = self.document.active_element().is_none_or(|active| {
            active.tag_name() == "BODY"
                || active.is_same_node(Some(&self.status))
                || active.is_same_node(Some(&focus.element))
        });
        if !may_restore {
            return Ok(());
        }
        focus.element.focus()?;
        if let Some((start, end)) = focus.selection
            && let Some(input) = focus.element.dyn_ref::<HtmlInputElement>()
        {
            input.set_selection_range(start, end)?;
        }
        Ok(())
    }
    fn retire_overlay(&mut self) -> Result<(), RoleShellError> {
        self.overlay_generation = None;
        match self.overlay.take() {
            Some(mut overlay) => overlay.revoke(),
            None => Ok(()),
        }
    }
    fn retire(&mut self) -> Result<(), RoleShellError> {
        self.dispatch.generation.set(None);
        self.dispatch.enabled.set(false);
        self.key = None;
        self.suspended_focus = None;
        let mut failure = self.retire_overlay().err();
        if let Some(mut mounted) = self.mounted.take()
            && let Err(error) = mounted.revoke()
        {
            failure.get_or_insert(error);
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    fn failed(&mut self, operation: RoleShellError) -> RoleShellError {
        self.slot.set_disabled(true);
        self.status.set_text_content(Some(
            "Presentation unavailable · awaiting a current permitted view",
        ));
        match self.retire() {
            Ok(()) => operation,
            Err(cleanup) => RoleShellError::Cleanup {
                operation: Box::new(operation),
                cleanup: Box::new(cleanup),
            },
        }
    }
    /// Terminal, idempotent scope release. Overlays restore their essential root
    /// before the role mount scrubs private raw Text/input references and resources.
    pub fn dispose(&mut self) -> Result<(), RoleShellError> {
        if self.disposed {
            return Ok(());
        }
        self.disposed = true;
        self.dispatch.disposed.set(true);
        let result = self.retire();
        self.dispatch.callback.borrow_mut().take();
        self.status.set_text_content(None);
        self.root.remove();
        result
    }
}
impl Drop for RoleShell {
    fn drop(&mut self) {
        // Explicit dispose reports errors. Drop still fences and removes this root;
        // the owned adapters perform their own final resource/privacy teardown.
        if self.dispose().is_err() {
            self.root.remove();
        }
    }
}
