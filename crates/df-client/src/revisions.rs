use df_types::{ClientBindingId, SessionRevision};

/// Observable admission of a complete server-provided view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewAcceptance {
    Applied,
    Stale { current: SessionRevision },
    Duplicate { current: SessionRevision },
    WrongBinding,
}

/// One current permitted view for one binding, with a monotonic revision watermark.
///
/// The owner supplies already authorized and bounded complete views. A binding ID
/// and revision are correlation/order values, not credentials or recovery authority.
/// This store never derives game outcomes or invents a view wire representation.
/// Reconnect retains the same store; changing its authorized scope requires owner
/// disposal. There is deliberately no reset that could admit an earlier snapshot.
#[derive(Debug)]
pub struct ViewStore<T> {
    binding_id: ClientBindingId,
    current: Option<(SessionRevision, T)>,
}

impl<T> ViewStore<T> {
    pub fn new(binding_id: ClientBindingId) -> Self {
        Self {
            binding_id,
            current: None,
        }
    }

    /// Replaces the single current view only for this binding and a newer revision.
    ///
    /// Revision ordering is the canonical lexicographic recovery epoch/sequence
    /// ordering. Gaps are allowed because inputs are complete snapshots. Equal
    /// revisions cannot replace a view, including when their payload differs.
    /// Rejected inputs are disposed; the accepted view and watermark stay intact.
    pub fn accept(
        &mut self,
        binding_id: ClientBindingId,
        revision: SessionRevision,
        view: T,
    ) -> ViewAcceptance {
        if binding_id != self.binding_id {
            return ViewAcceptance::WrongBinding;
        }
        if let Some((current, _)) = &self.current {
            if revision < *current {
                return ViewAcceptance::Stale { current: *current };
            }
            if revision == *current {
                return ViewAcceptance::Duplicate { current: *current };
            }
        }
        self.current = Some((revision, view));
        ViewAcceptance::Applied
    }

    // PLAY-L01: lets the caller amend presentation-only parts (server-generated art) of the
    // current view when `revision` is exactly its revision. The revision, the binding and every
    // game-content field stay as accepted: equal revisions otherwise remain `Duplicate`, and
    // `amend` must touch presentation alone. This is the narrow stand-in for the planned
    // presentation cursor. Returns whether `amend` reported a change.
    pub fn amend_presentation(
        &mut self,
        binding_id: ClientBindingId,
        revision: SessionRevision,
        amend: impl FnOnce(&mut T) -> bool,
    ) -> bool {
        if binding_id != self.binding_id {
            return false;
        }
        match &mut self.current {
            Some((current, view)) if *current == revision => amend(view),
            _ => false,
        }
    }

    /// Borrows the current complete view and its exact accepted revision.
    pub fn current(&self) -> Option<(SessionRevision, &T)> {
        self.current
            .as_ref()
            .map(|(revision, view)| (*revision, view))
    }
}
