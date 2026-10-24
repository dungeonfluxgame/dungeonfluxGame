use df_client::{
    cache::CacheScope,
    connection::ConnectionGeneration,
    connection_views::{ConnectionViewAcceptance, ConnectionViews},
    revisions::ViewAcceptance,
};
use df_protocol::common as rpc;
use df_types::{ClientBindingId, RecoveryEpoch, RunId, SessionId, SessionRevision};
use std::future::Future;

/// Watch sends an idle snapshot every five seconds. Bound local waits as well as RPC
/// metadata: a half-open browser tunnel cannot deliver the server's deadline status.
pub(super) const VIEW_SILENCE_MS: u32 = 8_000;

/// Brief visibility changes do not retire a tunnel that has just delivered a view.
pub(super) fn needs_foreground_reconnect(
    connected: bool,
    last_view_ms: Option<f64>,
    now_ms: f64,
) -> bool {
    !connected || last_view_ms.is_none_or(|last| now_ms - last >= f64::from(VIEW_SILENCE_MS))
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ViewWaitExpired;

pub(super) async fn bounded_view_wait<T>(
    work: impl Future<Output = T>,
    deadline: impl Future<Output = ()>,
) -> Result<T, ViewWaitExpired> {
    futures::pin_mut!(work, deadline);
    match futures::future::select(work, deadline).await {
        futures::future::Either::Left((value, _)) => Ok(value),
        futures::future::Either::Right(((), _)) => Err(ViewWaitExpired),
    }
}

#[cfg(target_arch = "wasm32")]
pub(super) async fn wait_for_view<T>(
    work: impl Future<Output = Result<T, tonic::Status>>,
) -> Result<T, tonic::Status> {
    bounded_view_wait(
        work,
        gloo_timers::future::TimeoutFuture::new(VIEW_SILENCE_MS),
    )
    .await
    .map_err(|_| tonic::Status::deadline_exceeded("game view stream stopped responding"))?
}

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

    #[cfg(test)]
    pub(super) fn accept(
        &mut self,
        generation: &ConnectionGeneration,
        scope: ViewScope,
        view: rpc::ViewMessage,
    ) -> Result<ConnectionViewAcceptance, ViewDeliveryError> {
        self.accept_with_art(generation, scope, view, |_, _| false)
            .map(|(acceptance, _)| acceptance)
    }

    /// Art delivery is independent of the table-talk cursor: both can move in one snapshot.
    pub(super) fn accept_with_art(
        &mut self,
        generation: &ConnectionGeneration,
        scope: ViewScope,
        view: rpc::ViewMessage,
        merge_art: impl FnMut(&mut rpc::ViewMessage, &rpc::ViewMessage) -> bool,
    ) -> Result<(ConnectionViewAcceptance, bool), ViewDeliveryError> {
        if !self.views.generation().same_scope(generation) || !generation.is_active() {
            return Ok((ConnectionViewAcceptance::ObsoleteGeneration, false));
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
        if journey.party.len() > df_rules::local_journey::MAXIMUM_PARTY || offer_count > 12 {
            return Err(ViewDeliveryError::InvalidProjection);
        }
        // Compare and amend before table talk replaces `current()`. Otherwise a ready plate
        // arriving with a new talk cursor becomes Applied, bypasses the art redraw, and all
        // following identical snapshots report no change while the director holds its render.
        let art_changed = self.accept_art_update(generation, scope, &view, merge_art);
        let refresh = self
            .current()
            .is_some_and(|current| presentation_sequence(current) != presentation_sequence(&view))
            .then(|| view.clone());
        let acceptance = self.views.accept(generation, scope.binding, revision, view);
        let acceptance = match (acceptance, refresh) {
            (ConnectionViewAcceptance::View(ViewAcceptance::Applied), _) => {
                self.table_talk = None;
                acceptance
            }
            (ConnectionViewAcceptance::View(ViewAcceptance::Duplicate { .. }), Some(refresh)) => {
                self.table_talk = Some(refresh);
                ConnectionViewAcceptance::View(ViewAcceptance::Applied)
            }
            _ => acceptance,
        };
        Ok((acceptance, art_changed))
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
        if scope != self.scope
            || !self.views.generation().same_scope(generation)
            || !generation.is_active()
        {
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

#[cfg(test)]
mod tests {
    use super::*;
    use df_client::connection::RpcConnection;
    use futures::{FutureExt, StreamExt, channel::oneshot};

    fn scope(role: ViewRole) -> ViewScope {
        ViewScope::from_wire(
            Some(&rpc::SessionId {
                value: Some(vec![1; 16]),
            }),
            Some(&rpc::RunId {
                value: Some(vec![2; 16]),
            }),
            Some(&rpc::ClientBindingId {
                value: Some(vec![3; 16]),
            }),
            role,
        )
        .unwrap()
    }

    fn view(sequence: u64, talk: u64, state: rpc::PlateState) -> rpc::ViewMessage {
        rpc::ViewMessage {
            revision: Some(rpc::SessionRevision {
                epoch: Some(rpc::RecoveryEpoch { value: Some(1) }),
                sequence: Some(sequence),
            }),
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView {
                    journey: Some(rpc::JourneyView {
                        presentation_sequence: talk,
                        party: (0..6)
                            .map(|index| rpc::PartyMemberView {
                                name: format!("Hero {}", index + 1),
                                ..Default::default()
                            })
                            .collect(),
                        ..Default::default()
                    }),
                    scene: Some(rpc::GameplayScene {
                        title: "Salt quay".to_owned(),
                        plate_state: state as i32,
                        live_plate: if state == rpc::PlateState::Ready {
                            format!("/gameplay/imagery/{}", "a".repeat(64))
                        } else {
                            String::new()
                        },
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
        }
    }

    #[test]
    fn same_revision_art_redraw_is_independent_of_table_talk_and_overridden_narration() {
        for current_talk in [0, 1] {
            for incoming_talk in [current_talk, current_talk + 1] {
                let connection = RpcConnection::new(());
                let generation = connection.generation();
                let scope = scope(ViewRole::Display);
                let mut views = GameplayViews::new(scope, generation.clone());
                let mut pending = view(14, 0, rpc::PlateState::Pending);
                if let Some(rpc::view_message::Audience::Display(display)) = &mut pending.audience {
                    display.narration = "The enforcer has stepped aside.".into();
                }
                views.accept(&generation, scope, pending.clone()).unwrap();
                if current_talk != 0 {
                    let mut talk = pending.clone();
                    if let Some(rpc::view_message::Audience::Display(display)) = &mut talk.audience
                    {
                        display.journey.as_mut().unwrap().presentation_sequence = current_talk;
                    }
                    views.accept(&generation, scope, talk).unwrap();
                }
                let mut ready = view(14, incoming_talk, rpc::PlateState::Ready);
                if let Some(rpc::view_message::Audience::Display(display)) = &mut ready.audience {
                    display.narration = "The enforcer has stepped aside.".into();
                }
                // Reproduce the old receiver: promotion to Applied bypasses its duplicate-only
                // art branch, even though ConnectionViews itself returned Duplicate.
                let mut old_receiver = GameplayViews::new(scope, generation.clone());
                old_receiver
                    .accept(&generation, scope, views.current().unwrap().clone())
                    .unwrap();
                let accepted = old_receiver
                    .accept(&generation, scope, ready.clone())
                    .unwrap();
                let old_redraw = matches!(
                    accepted,
                    ConnectionViewAcceptance::View(ViewAcceptance::Duplicate { .. })
                ) && old_receiver.accept_art_update(
                    &generation,
                    scope,
                    &ready,
                    crate::live_art::merge_art,
                );
                assert_eq!(old_redraw, current_talk == incoming_talk);
                let (_, redraw_art) = views
                    .accept_with_art(
                        &generation,
                        scope,
                        ready.clone(),
                        crate::live_art::merge_art,
                    )
                    .unwrap();
                assert!(
                    redraw_art,
                    "ready art must redraw immediately: table cursor {current_talk}->{incoming_talk}"
                );
                assert_eq!(views.current(), Some(&ready));
                assert_eq!(views.current_revision().unwrap().sequence(), 14);
            }
        }
    }

    #[test]
    fn stalled_view_wait_expires_and_resubscription_applies_latest_equal_revision_plate() {
        futures::executor::block_on(async {
            let connection = RpcConnection::new(());
            let generation = connection.generation();
            let scope = scope(ViewRole::Display);
            let mut views = GameplayViews::new(scope, generation.clone());
            views
                .accept(&generation, scope, view(4, 0, rpc::PlateState::Pending))
                .unwrap();
            views
                .accept(&generation, scope, view(4, 1, rpc::PlateState::Pending))
                .unwrap();
            // The browser's socket can remain apparently open after the last frame. Only
            // this controlled deadline fires; no server status or EOF is delivered.
            let (_send, mut stream) = futures::channel::mpsc::unbounded::<rpc::ViewMessage>();
            let (expire, expired) = oneshot::channel::<()>();
            let wait = bounded_view_wait(stream.next(), async { expired.await.unwrap() });
            futures::pin_mut!(wait);
            assert!(futures::poll!(wait.as_mut()).is_pending());
            expire.send(()).unwrap();
            assert_eq!(wait.await, Err(ViewWaitExpired));
            connection.close();

            let replacement = RpcConnection::new(());
            let replacement_generation = replacement.generation();
            assert!(views.reconnect(replacement_generation.clone()));
            let latest = view(4, 1, rpc::PlateState::Ready);
            assert!(matches!(
                views
                    .accept(&replacement_generation, scope, latest.clone())
                    .unwrap(),
                ConnectionViewAcceptance::View(ViewAcceptance::Duplicate { .. })
            ));
            assert!(views.accept_art_update(
                &replacement_generation,
                scope,
                &latest,
                crate::live_art::merge_art
            ));
            assert_eq!(views.current(), Some(&latest));
            assert_eq!(views.current_revision().unwrap().sequence(), 4);
            assert_eq!(views.scope().cache_scope(), scope.cache_scope());
            assert!(!views.accept_art_update(
                &generation,
                scope,
                &view(4, 1, rpc::PlateState::Failed),
                crate::live_art::merge_art
            ));
            assert_eq!(
                views.current(),
                Some(&latest),
                "old callbacks cannot undo resync"
            );
            assert_eq!(
                views.accept(&generation, scope, view(5, 2, rpc::PlateState::Pending)),
                Ok(ConnectionViewAcceptance::ObsoleteGeneration)
            );
            assert!(!views.reconnect(generation));
            assert!(VIEW_SILENCE_MS > 5_000 && VIEW_SILENCE_MS <= 10_000);
        });
    }

    #[test]
    fn dropped_stream_wakes_the_view_wait_without_waiting_for_the_watchdog() {
        futures::executor::block_on(async {
            let (send, mut stream) = futures::channel::mpsc::unbounded::<rpc::ViewMessage>();
            let wait = bounded_view_wait(stream.next(), futures::future::pending());
            futures::pin_mut!(wait);
            assert!(futures::poll!(wait.as_mut()).is_pending());
            drop(send);
            assert_eq!(wait.await, Ok(None));
        });
    }

    #[test]
    fn a_heartbeat_wins_over_a_simultaneous_deadline() {
        futures::executor::block_on(async {
            let next = view(4, 0, rpc::PlateState::Ready);
            assert_eq!(
                bounded_view_wait(
                    futures::future::ready(next.clone()),
                    futures::future::ready(())
                )
                .await,
                Ok(next)
            );
            assert!(
                bounded_view_wait(futures::future::pending::<()>(), futures::future::pending())
                    .now_or_never()
                    .is_none()
            );
        });
    }

    #[test]
    fn unchanged_five_second_snapshots_keep_the_eight_second_watch_alive() {
        futures::executor::block_on(async {
            let connection = RpcConnection::new(());
            let generation = connection.generation();
            let scope = scope(ViewRole::Display);
            let mut views = GameplayViews::new(scope, generation.clone());
            for tick in 0..=60 {
                let (send_view, mut stream) = futures::channel::mpsc::unbounded();
                let (expire, expired) = oneshot::channel::<()>();
                let wait = bounded_view_wait(stream.next(), async { expired.await.unwrap() });
                futures::pin_mut!(wait);
                assert!(futures::poll!(wait.as_mut()).is_pending());
                // Controlled signals represent the server's 5 s snapshot arriving before
                // each fresh 8 s deadline, over five minutes at one unchanged revision.
                assert!(5_000 < VIEW_SILENCE_MS);
                send_view
                    .unbounded_send(view(4, 0, rpc::PlateState::Pending))
                    .unwrap();
                let next = wait.await.unwrap().unwrap();
                drop(expire);
                let accepted = views.accept(&generation, scope, next).unwrap();
                assert!(matches!(
                    accepted,
                    ConnectionViewAcceptance::View(
                        ViewAcceptance::Applied | ViewAcceptance::Duplicate { .. }
                    )
                ));
                assert!(!needs_foreground_reconnect(
                    true,
                    Some(f64::from(tick * 5_000)),
                    f64::from(tick * 5_000 + 100)
                ));
            }
            assert!(generation.is_active());
            assert_eq!(views.current_revision().unwrap().sequence(), 4);
        });
    }

    #[test]
    fn foreground_only_replaces_a_disconnected_or_silent_tunnel() {
        assert!(!needs_foreground_reconnect(true, Some(1_000.0), 8_999.0));
        assert!(needs_foreground_reconnect(true, Some(1_000.0), 9_000.0));
        assert!(needs_foreground_reconnect(false, Some(1_000.0), 1_001.0));
        assert!(needs_foreground_reconnect(true, None, 1_001.0));
    }

    #[test]
    fn invalid_scope_and_wrong_audience_never_replace_the_display() {
        assert_eq!(
            ViewScope::from_wire(None, None, None, ViewRole::Display),
            Err(ViewDeliveryError::InvalidScope)
        );
        let connection = RpcConnection::new(());
        let generation = connection.generation();
        let mut views = GameplayViews::new(scope(ViewRole::Display), generation.clone());
        assert_eq!(
            views.accept(
                &generation,
                scope(ViewRole::Player),
                view(4, 0, rpc::PlateState::Pending)
            ),
            Err(ViewDeliveryError::WrongScope)
        );
        let wrong_audience = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Player(
                rpc::PlayerGameplayView::default(),
            )),
            ..view(4, 0, rpc::PlateState::Pending)
        };
        assert_eq!(
            views.accept(&generation, scope(ViewRole::Display), wrong_audience),
            Err(ViewDeliveryError::WrongAudience)
        );
    }
}
