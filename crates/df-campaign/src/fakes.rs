//! Faithful in-memory directories for the tests of this crate and its consumers (feature
//! `test-fakes`). Each stands in for another owner's port; the real adapters are theirs.
//!
//! TODO(S1-02): the account directory reads B's account tables. TODO(S1-03): the session directory calls
//! O1's `open_session`, `campaign_of` and the phase read. TODO(S1-04): the funding consumer reads the
//! feed through its own cursor; the notification here records exactly what the facade sends.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use df_types::{AccountId, CampaignId, SessionId, TenantId};

use crate::{
    errors::{StoreError, StoreResult},
    lobby_state::{SeatedMember, StartReadiness},
    records::AccountStatus,
    store::{
        AccountDirectory, AccountProfile, FundingDirectory, SessionDirectory, SessionFacts,
        SessionPhase,
    },
};

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Clone, Default)]
pub struct FakeAccounts {
    profiles: Arc<Mutex<HashMap<AccountId, AccountProfile>>>,
}

impl FakeAccounts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(
        &self,
        account: AccountId,
        display_name: &str,
        verified: bool,
        tenant: Option<TenantId>,
        status: AccountStatus,
    ) {
        lock(&self.profiles).insert(
            account,
            AccountProfile {
                display_name: Some(display_name.to_owned()),
                verified,
                tenant,
                status,
            },
        );
    }
}

impl AccountDirectory for FakeAccounts {
    async fn profile(&self, account: AccountId) -> StoreResult<Option<AccountProfile>> {
        Ok(lock(&self.profiles).get(&account).cloned())
    }
}

#[derive(Clone, Default)]
pub struct FakeSessions {
    sessions: Arc<Mutex<HashMap<SessionId, SessionFacts>>>,
    fail: Arc<AtomicBool>,
    started: StartedSessions,
}

type StartedSessions = Arc<Mutex<Vec<(SessionId, Vec<SeatedMember>)>>>;

impl FakeSessions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, session: SessionId, campaign: CampaignId, phase: SessionPhase) {
        lock(&self.sessions).insert(session, SessionFacts { campaign, phase });
    }

    /// The rosters handed to `start_session`, in order.
    pub fn started(&self) -> Vec<(SessionId, Vec<SeatedMember>)> {
        lock(&self.started).clone()
    }

    /// Makes every read fail, as an unreachable O1 would.
    pub fn fail_reads(&self, fail: bool) {
        self.fail.store(fail, Ordering::SeqCst);
    }
}

impl SessionDirectory for FakeSessions {
    async fn session(&self, session: SessionId) -> StoreResult<Option<SessionFacts>> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(StoreError::Unavailable);
        }
        Ok(lock(&self.sessions).get(&session).copied())
    }

    async fn live_session_of(
        &self,
        campaign: CampaignId,
    ) -> StoreResult<Option<(SessionId, SessionPhase)>> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(StoreError::Unavailable);
        }
        Ok(lock(&self.sessions)
            .iter()
            .find(|(_, facts)| facts.campaign == campaign && facts.phase != SessionPhase::Ended)
            .map(|(id, facts)| (*id, facts.phase)))
    }

    async fn open_session(&self, campaign: CampaignId) -> StoreResult<(SessionId, SessionPhase)> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(StoreError::Unavailable);
        }
        let mut sessions = lock(&self.sessions);
        if let Some((id, facts)) = sessions
            .iter()
            .find(|(_, facts)| facts.campaign == campaign && facts.phase != SessionPhase::Ended)
        {
            return Ok((*id, facts.phase));
        }
        let bytes = crate::secrets::random_identity().map_err(|_| StoreError::RandomUnavailable)?;
        let id = SessionId::from_bytes(&bytes).map_err(|_| StoreError::InvalidRecord)?;
        sessions.insert(
            id,
            SessionFacts {
                campaign,
                phase: SessionPhase::Lobby,
            },
        );
        Ok((id, SessionPhase::Lobby))
    }

    async fn start_session(
        &self,
        session: SessionId,
        campaign: CampaignId,
        roster: &[SeatedMember],
    ) -> StoreResult<()> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(StoreError::Unavailable);
        }
        let mut sessions = lock(&self.sessions);
        match sessions.get_mut(&session) {
            Some(facts) if facts.campaign == campaign && facts.phase == SessionPhase::Lobby => {
                facts.phase = SessionPhase::Playing;
            }
            _ => return Err(StoreError::Unavailable),
        }
        lock(&self.started).push((session, roster.to_vec()));
        Ok(())
    }
}

/// Records every notification. `FundingDirectory` has no method that could cancel, release or
/// reassign anything, so this is all the facade can do to F.
#[derive(Clone, Default)]
pub struct FakeFunding {
    calls: Arc<Mutex<Vec<(CampaignId, u64)>>>,
    readiness: Arc<Mutex<Option<StartReadiness>>>,
    opened: Arc<Mutex<Vec<(CampaignId, SessionId)>>>,
    fail_opening: Arc<AtomicBool>,
}

impl FakeFunding {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn calls(&self) -> Vec<(CampaignId, u64)> {
        lock(&self.calls).clone()
    }

    /// What `start_readiness` answers from now on (ready, no notice, by default).
    pub fn set_readiness(&self, readiness: StartReadiness) {
        *lock(&self.readiness) = Some(readiness);
    }

    /// The sessions whose funding was opened, in order.
    pub fn opened(&self) -> Vec<(CampaignId, SessionId)> {
        lock(&self.opened).clone()
    }

    /// Makes `open_session_funding` fail, as an unreachable finance owner would.
    pub fn fail_opening(&self, fail: bool) {
        self.fail_opening.store(fail, Ordering::SeqCst);
    }
}

impl FundingDirectory for FakeFunding {
    async fn membership_event(&self, campaign: CampaignId, event_seq: u64) {
        lock(&self.calls).push((campaign, event_seq));
    }

    async fn start_readiness(
        &self,
        _campaign: CampaignId,
        _session: SessionId,
    ) -> StoreResult<StartReadiness> {
        Ok((*lock(&self.readiness)).unwrap_or(StartReadiness {
            ready: true,
            waiting_for_sponsor: false,
            no_charge_until_start: true,
        }))
    }

    async fn open_session_funding(
        &self,
        campaign: CampaignId,
        session: SessionId,
    ) -> StoreResult<()> {
        if self.fail_opening.load(Ordering::SeqCst) {
            return Err(StoreError::Unavailable);
        }
        lock(&self.opened).push((campaign, session));
        Ok(())
    }
}
