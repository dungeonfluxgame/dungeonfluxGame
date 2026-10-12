use df_client::{
    cache::CacheScope,
    connection::ConnectionGeneration,
    connection_views::{ConnectionViewAcceptance, ConnectionViews},
    revisions::ViewAcceptance,
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
/// The server's table-talk counter of a view: it moves when a line is logged or a reply starts
/// or ends, while the game revision does not.
fn presentation_sequence(view: &rpc::ViewMessage) -> Option<u64> {
    let journey = match view.audience.as_ref()? {
        rpc::view_message::Audience::Display(display) => display.journey.as_ref(),
        rpc::view_message::Audience::Player(player) => player.journey.as_ref(),
    };
    journey.map(|journey| journey.presentation_sequence)
}

/// An equal revision never replaces a view, with one exception: table talk (what heroes and
/// speakers said) changes the room's screens without moving the game revision, so a same-revision
/// view whose `presentation_sequence` differs is kept beside the store as the current view
/// until a newer revision arrives. Local receipt redraws borrow the retained view.
pub(super) struct GameplayViews {
    scope: ViewScope,
    views: ConnectionViews<rpc::ViewMessage>,
    table_talk: Option<rpc::ViewMessage>,
}

impl GameplayViews {
    pub(super) fn new(scope: ViewScope, generation: ConnectionGeneration) -> Self {
        Self {
            scope,
            views: ConnectionViews::new(scope.binding, generation),
            table_talk: None,
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
        if journey.party.len() > 2 || offer_count > 4 {
            return Err(ViewDeliveryError::InvalidProjection);
        }
        let refresh = self
            .current()
            .is_some_and(|current| presentation_sequence(current) != presentation_sequence(&view))
            .then(|| view.clone());
        let acceptance = self.views.accept(generation, scope.binding, revision, view);
        match (acceptance, refresh) {
            (ConnectionViewAcceptance::View(ViewAcceptance::Applied), _) => {
                self.table_talk = None;
                Ok(acceptance)
            }
            (ConnectionViewAcceptance::View(ViewAcceptance::Duplicate { .. }), Some(refresh)) => {
                self.table_talk = Some(refresh);
                Ok(ConnectionViewAcceptance::View(ViewAcceptance::Applied))
            }
            _ => Ok(acceptance),
        }
    }

    // PLAY-L01: the generated art of a view of the revision already shown is copied into
    // the view on screen by `merge_art` (`live_art::merge_art`); nothing else may change at an
    // equal revision. Returns whether the art changed.
    pub(super) fn accept_art_update(
        &mut self,
        generation: &ConnectionGeneration,
        scope: ViewScope,
        view: &rpc::ViewMessage,
        merge_art: impl FnMut(&mut rpc::ViewMessage, &rpc::ViewMessage) -> bool,
    ) -> bool {
        let Some(revision) = view
            .revision
            .as_ref()
            .and_then(|wire| Some((wire.epoch.as_ref()?.value?, wire.sequence?)))
            .and_then(|(epoch, sequence)| {
                Some(SessionRevision::new(
                    RecoveryEpoch::new(epoch).ok()?,
                    sequence,
                ))
            })
        else {
            return false;
        };
        if scope != self.scope {
            return false;
        }
        // The view on screen is the table-talk view when there is one, so the art goes into it
        // as well as into the stored view that replaces it later; merging into the stored view
        // alone left finished art invisible after any line was spoken.
        let mut merge_art = merge_art;
        let stored = self
            .views
            .amend_presentation(generation, scope.binding, revision, |shown| {
                merge_art(shown, view)
            });
        let on_screen = self.table_talk.as_mut().is_some_and(|talk| {
            let same_revision = talk.revision.as_ref().is_some_and(|wire| {
                wire.epoch == view.revision.as_ref().and_then(|v| v.epoch)
                    && wire.sequence == view.revision.as_ref().and_then(|v| v.sequence)
            });
            same_revision && merge_art(talk, view)
        });
        stored || on_screen
    }

    pub(super) fn current_revision(&self) -> Option<SessionRevision> {
        self.views.current().map(|(revision, _)| revision)
    }

    pub(super) fn current(&self) -> Option<&rpc::ViewMessage> {
        self.table_talk
            .as_ref()
            .or_else(|| self.views.current().map(|(_, view)| view))
    }
}
