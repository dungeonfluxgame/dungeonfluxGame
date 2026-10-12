use df_types::{ClientBindingId, SessionRevision};

use crate::{
    connection::ConnectionGeneration,
    revisions::{ViewAcceptance, ViewStore},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionViewAcceptance {
    View(ViewAcceptance),
    ObsoleteGeneration,
}

/// Retains the existing canonical binding/revision view store across reconnect.
/// Complete view data must already be authorized, bounded and mapped by its owner.
/// Scope fencing supplements revision order: even a newer revision from a retired
/// connection cannot overwrite the active connection's permitted view.
pub struct ConnectionViews<View> {
    views: ViewStore<View>,
    generation: ConnectionGeneration,
}

impl<View> ConnectionViews<View> {
    pub fn new(binding: ClientBindingId, generation: ConnectionGeneration) -> Self {
        Self {
            views: ViewStore::new(binding),
            generation,
        }
    }

    /// Owner-initiated reconnect preserves the binding and revision watermark.
    /// A changed authorized binding requires a new store, as ViewStore requires.
    pub fn reconnect(&mut self, generation: ConnectionGeneration) -> bool {
        if !generation.is_active() {
            return false;
        }
        self.generation = generation;
        true
    }

    pub fn accept(
        &mut self,
        generation: &ConnectionGeneration,
        binding: ClientBindingId,
        revision: SessionRevision,
        view: View,
    ) -> ConnectionViewAcceptance {
        if !self.generation.same_scope(generation) || !generation.is_active() {
            return ConnectionViewAcceptance::ObsoleteGeneration;
        }
        ConnectionViewAcceptance::View(self.views.accept(binding, revision, view))
    }

    // PLAY-L01: see `ViewStore::amend_presentation`; only the active connection may do this.
    pub fn amend_presentation(
        &mut self,
        generation: &ConnectionGeneration,
        binding: ClientBindingId,
        revision: SessionRevision,
        amend: impl FnOnce(&mut View) -> bool,
    ) -> bool {
        self.generation.same_scope(generation)
            && generation.is_active()
            && self.views.amend_presentation(binding, revision, amend)
    }

    pub fn generation(&self) -> &ConnectionGeneration {
        &self.generation
    }

    /// Read-only access for existing mounted consumers; admission stays here.
    pub fn views(&self) -> &ViewStore<View> {
        &self.views
    }

    pub fn current(&self) -> Option<(SessionRevision, &View)> {
        self.views.current()
    }
}
