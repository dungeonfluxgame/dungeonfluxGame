//! LAUNCH decision 34: authenticated session routes and a bounded demo placement port.
//! The directory owns session truth; a launcher never starts a second gameplay owner.

mod routes;
mod runtime_http;
pub mod runtime_protocol;
pub use runtime_http::HttpRuntime;
#[cfg(test)]
mod tests;

use std::{future::Future, sync::Arc, time::Duration};

use axum::{extract::Request, response::Response};
use df_types::{CampaignId, MemberId, SessionId};
use tokio::sync::Mutex;

use crate::{
    audit::AuditLine,
    errors::{StoreError, StoreResult},
    lobby_state::SeatedMember,
    store::{SessionDirectory, SessionFacts, SessionPhase},
};

pub use routes::{App, router};

/// The current demo has two seats and one gameplay room per process.
pub const MAXIMUM_DEMO_SEATS: usize = 2;

/// The exact authorized campaign roster frozen by the Start attempt.
#[derive(Clone, Debug)]
pub struct Placement {
    pub campaign: CampaignId,
    pub session: SessionId,
    pub roster: Vec<SeatedMember>,
}

/// An allocated gameplay room and its process generation, never a browser credential.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Room {
    pub code: String,
    pub run_identity: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Audience {
    Player(MemberId),
    Display,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Surface {
    Play,
    Display,
    Tunnel,
}

/// Closed failure reasons suitable for audit output; private upstream details stay upstream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    Unavailable,
    Occupied,
    InvalidReply,
    Deadline,
}

/// Allocation is idempotent for the exact placement. `serve` forwards no caller credentials.
/// Implementations must bound upstream I/O and own upgraded connections until they close.
pub trait RuntimeLauncher: Send + Sync + 'static {
    fn allocate(
        &self,
        placement: &Placement,
    ) -> impl Future<Output = Result<Room, RuntimeError>> + Send;

    fn serve(
        &self,
        placement: &Placement,
        room: &Room,
        audience: Audience,
        surface: Surface,
        request: Request,
    ) -> impl Future<Output = Result<Response, RuntimeError>> + Send;
}

struct Inner<D, L> {
    directory: D,
    runtime: L,
    deadline: Duration,
    slot: Mutex<Option<Placement>>,
    room: Mutex<Option<Room>>,
    start: Mutex<()>,
}

/// Install this decorator as `Ports::Sessions`, and pass its clone to the route app.
/// It retains one placement for its process lifetime; it never recycles another table's room.
pub struct SessionLauncher<D: SessionDirectory, L: RuntimeLauncher> {
    inner: Arc<Inner<D, L>>,
}

impl<D: SessionDirectory, L: RuntimeLauncher> Clone for SessionLauncher<D, L> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<D: SessionDirectory, L: RuntimeLauncher> SessionLauncher<D, L> {
    pub fn new(directory: D, runtime: L, deadline: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                directory,
                runtime,
                deadline,
                slot: Mutex::new(None),
                room: Mutex::new(None),
                start: Mutex::new(()),
            }),
        }
    }

    /// Read-only placement inspection for the composition's authenticated developer surface.
    pub async fn placement(&self) -> Option<Placement> {
        self.inner.slot.lock().await.clone()
    }

    /// The room last confirmed for this session, for authenticated operator inspection.
    pub async fn room(&self) -> Option<Room> {
        self.inner.room.lock().await.clone()
    }

    async fn allocate(&self, placement: &Placement) -> Result<Room, RuntimeError> {
        let result =
            tokio::time::timeout(self.inner.deadline, self.inner.runtime.allocate(placement))
                .await
                .map_err(|_| RuntimeError::Deadline)
                .and_then(|result| result);
        let result = match result {
            Ok(room) => {
                let mut assigned = self.inner.room.lock().await;
                if room.code.is_empty()
                    || room.code.len() > 32
                    || !room.code.bytes().all(|byte| byte.is_ascii_alphanumeric())
                    || room.run_identity.is_empty()
                    || room.run_identity.len() > 128
                    || !room
                        .run_identity
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
                {
                    Err(RuntimeError::InvalidReply)
                } else if assigned.as_ref().is_some_and(|existing| existing != &room) {
                    Err(RuntimeError::Occupied)
                } else {
                    *assigned = Some(room.clone());
                    Ok(room)
                }
            }
            Err(error) => Err(error),
        };
        AuditLine::new("SESSION", "placement")
            .campaign(placement.campaign)
            .session(placement.session)
            .reason(match &result {
                Ok(_) => "ready",
                Err(RuntimeError::Unavailable) => "unavailable",
                Err(RuntimeError::Occupied) => "occupied",
                Err(RuntimeError::InvalidReply) => "invalid_reply",
                Err(RuntimeError::Deadline) => "deadline",
            })
            .emit("session.placement");
        result
    }
}

impl<D: SessionDirectory, L: RuntimeLauncher> SessionDirectory for SessionLauncher<D, L> {
    async fn session(&self, session: SessionId) -> StoreResult<Option<SessionFacts>> {
        self.inner.directory.session(session).await
    }

    async fn live_session_of(
        &self,
        campaign: CampaignId,
    ) -> StoreResult<Option<(SessionId, SessionPhase)>> {
        self.inner.directory.live_session_of(campaign).await
    }

    async fn open_session(&self, campaign: CampaignId) -> StoreResult<(SessionId, SessionPhase)> {
        self.inner.directory.open_session(campaign).await
    }

    async fn start_session(
        &self,
        session: SessionId,
        campaign: CampaignId,
        roster: &[SeatedMember],
    ) -> StoreResult<()> {
        if roster.is_empty() || roster.len() > MAXIMUM_DEMO_SEATS {
            return Err(StoreError::InvalidRecord);
        }
        for (index, seat) in roster.iter().enumerate() {
            if roster
                .iter()
                .skip(index + 1)
                .any(|other| other.account == seat.account || other.member == seat.member)
            {
                return Err(StoreError::InvalidRecord);
            }
        }
        // Cancellation releases serialization but retains the reserved slot: an uncertain Start
        // must never make the single runtime available to a different session.
        let _start = self.inner.start.lock().await;
        let placement = Placement {
            campaign,
            session,
            roster: roster.to_vec(),
        };
        let reserved = {
            let slot = self.inner.slot.lock().await;
            if let Some(existing) = slot.as_ref() {
                if !same_placement(existing, &placement) {
                    return Err(StoreError::Unavailable);
                }
                true
            } else {
                false
            }
        };
        let facts =
            tokio::time::timeout(self.inner.deadline, self.inner.directory.session(session))
                .await
                .map_err(|_| StoreError::Unavailable)??
                .filter(|facts| facts.campaign == campaign)
                .ok_or(StoreError::InvalidRecord)?;
        let already_started = reserved
            && matches!(
                facts.phase,
                SessionPhase::PreparingFundingOpen | SessionPhase::Playing
            );
        if !already_started {
            if facts.phase != SessionPhase::Lobby {
                return Err(StoreError::Unavailable);
            }
            *self.inner.slot.lock().await = Some(placement.clone());
            tokio::time::timeout(
                self.inner.deadline,
                self.inner
                    .directory
                    .start_session(session, campaign, roster),
            )
            .await
            .map_err(|_| StoreError::Unknown)??;
        }
        // Allocation failure has already been audited. Start remains true; entry offers retry.
        match self.allocate(&placement).await {
            Ok(_) | Err(_) => Ok(()),
        }
    }
}

fn same_placement(left: &Placement, right: &Placement) -> bool {
    left.campaign == right.campaign
        && left.session == right.session
        && left.roster.len() == right.roster.len()
        && left
            .roster
            .iter()
            .zip(&right.roster)
            .all(|(left, right)| left.account == right.account && left.member == right.member)
}
