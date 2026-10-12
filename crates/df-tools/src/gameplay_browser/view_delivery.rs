use df_client::{
    cache::CacheScope,
    connection::ConnectionGeneration,
    connection_views::{ConnectionViewAcceptance, ConnectionViews},
};
use df_protocol::common as rpc;
use df_types::{ClientBindingId, RecoveryEpoch, RunId, SessionId, SessionRevision};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ViewRole {
    Player,
    Display,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ViewScope {
    session: SessionId,
    run: RunId,
    binding: ClientBindingId,
    role: ViewRole,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ViewDeliveryError {
    InvalidScope,
    WrongScope,
    InvalidRevision,
    WrongAudience,
    InvalidProjection,
}

impl ViewScope {
    pub(super) fn cache_scope(self) -> CacheScope {
        CacheScope {
            session: self.session,
            run: self.run,
            binding: self.binding,
        }
    }

    pub(super) fn from_wire(
        session: Option<&rpc::SessionId>,
        run: Option<&rpc::RunId>,
        binding: Option<&rpc::ClientBindingId>,
        role: ViewRole,
    ) -> Result<Self, ViewDeliveryError> {
        let session = session
            .and_then(|id| id.value.as_deref())
            .ok_or(ViewDeliveryError::InvalidScope)?;
        let run = run
            .and_then(|id| id.value.as_deref())
            .ok_or(ViewDeliveryError::InvalidScope)?;
        let binding = binding
            .and_then(|id| id.value.as_deref())
            .ok_or(ViewDeliveryError::InvalidScope)?;
        Ok(Self {
            session: SessionId::from_bytes(session).map_err(|_| ViewDeliveryError::InvalidScope)?,
            run: RunId::from_bytes(run).map_err(|_| ViewDeliveryError::InvalidScope)?,
            binding: ClientBindingId::from_bytes(binding)
                .map_err(|_| ViewDeliveryError::InvalidScope)?,
            role,
        })
    }
}

/// Admission of the actual generated gameplay snapshots for one authorized scope.
/// The connection owner issues generations; reconnect preserves the durable watermark.
/// The current wire has no independent presentation sequence, so equal revisions
/// cannot deliver changed content. Local receipt redraws borrow the retained view.
pub(super) struct GameplayViews {
    scope: ViewScope,
    views: ConnectionViews<rpc::ViewMessage>,
}

impl GameplayViews {
    pub(super) fn new(scope: ViewScope, generation: ConnectionGeneration) -> Self {
        Self {
            scope,
            views: ConnectionViews::new(scope.binding, generation),
        }
    }

    pub(super) fn scope(&self) -> ViewScope {
        self.scope
    }

    pub(super) fn reconnect(&mut self, generation: ConnectionGeneration) -> bool {
        self.views.reconnect(generation)
    }

    pub(super) fn accept(
        &mut self,
        generation: &ConnectionGeneration,
        scope: ViewScope,
        view: rpc::ViewMessage,
    ) -> Result<ConnectionViewAcceptance, ViewDeliveryError> {
        if !self.views.generation().same_scope(generation) || !generation.is_active() {
            return Ok(ConnectionViewAcceptance::ObsoleteGeneration);
        }
        if scope != self.scope {
            return Err(ViewDeliveryError::WrongScope);
        }
        let wire_revision = view
            .revision
            .as_ref()
            .ok_or(ViewDeliveryError::InvalidRevision)?;
        let epoch = wire_revision
            .epoch
            .as_ref()
            .and_then(|epoch| epoch.value)
            .ok_or(ViewDeliveryError::InvalidRevision)?;
        let sequence = wire_revision
            .sequence
            .ok_or(ViewDeliveryError::InvalidRevision)?;
        let revision = SessionRevision::new(
            RecoveryEpoch::new(epoch).map_err(|_| ViewDeliveryError::InvalidRevision)?,
            sequence,
        );
        let (journey, offer_count) = match (&view.audience, self.scope.role) {
            (Some(rpc::view_message::Audience::Player(player)), ViewRole::Player) => {
                (player.journey.as_ref(), player.offers.len())
            }
            (Some(rpc::view_message::Audience::Display(display)), ViewRole::Display) => {
                (display.journey.as_ref(), 0)
            }
            _ => return Err(ViewDeliveryError::WrongAudience),
        };
        let journey = journey.ok_or(ViewDeliveryError::InvalidProjection)?;
        if journey.party.len() > 2 || offer_count > 12 {
            return Err(ViewDeliveryError::InvalidProjection);
        }
        Ok(self.views.accept(generation, scope.binding, revision, view))
    }

    pub(super) fn current_revision(&self) -> Option<SessionRevision> {
        self.views.current().map(|(revision, _)| revision)
    }

    pub(super) fn current(&self) -> Option<&rpc::ViewMessage> {
        self.views.current().map(|(_, view)| view)
    }
}
