//! The PostgreSQL adapter of the campaign store (`planning/campaigns-and-table-research.md`
//! section 6). Every mutating method is one transaction that also appends its events; every
//! method takes the locks in the order of launch decision 29 (accounts, then O1 rows, then
//! `campaigns.campaign`, then campaign child rows). Queries are parameterized and schema-qualified.
//!
//! Round 1 staging: this module is compiled by the integration tests of `df-campaign` until B
//! declares `mod campaign_store` in `df-persistence/src/lib.rs` and adds the Cargo edge to
//! `df-campaign`.

mod campaigns;
mod closure;
mod conn;
mod display;
mod feed;
mod glue;
mod invites;
mod members;
mod queries;
use self::queries as q;
mod rows;
mod shared;
mod transfers;

use df_campaign::{
    config::Tunables,
    errors::{Failure, Outcome, StoreResult},
    ids::{InvitationId, PairingId, ProposalId, RequestId, TicketId},
    join_invitation::JoinRole,
    records::{
        CampaignChange, CampaignHead, CampaignRow, CodeRow, EventRow, InvitationRow, MemberRow,
        MemberSnapshot, NewCampaign, PairingRow, RequestRow, RevokeReason, TicketRow, TransferKind,
    },
    roles::Delegations,
    store::{
        Accepted, CampaignStore, ChangeKey, Decision, NewInvitation, NewPairing, NewProposal,
        OwnerFence, PreviewFacts, Redeemed, RetentionReport, VerifiedTicket,
    },
};
use df_types::{AccountId, CampaignId, MemberId, OperationId, SessionId};

pub use conn::{Connect, ConnectFuture, Pool};
pub use glue::PgClosureParticipant;

/// What the startup check requires of the role's `transaction_timeout` (5.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeoutPolicy {
    /// Hosted composition: positive and at most the feed's bound, or refuse to start.
    Enforced,
    /// Local development and tests on a scratch server whose roles are not provisioned.
    NotChecked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenRefusal {
    /// The role's default is `0` (disabled) or could not be read.
    TimeoutNotSet,
    /// The role's default is longer than `feed_transaction_timeout_seconds`.
    TimeoutTooLong,
    /// The configuration cannot work (the overlap must exceed the timeout).
    Configuration(df_campaign::config::TunablesRefusal),
    /// The commit-time bound in the database (`campaigns.feed_bound`) is longer than
    /// `feed_transaction_timeout_seconds`, so the poll-overlap argument would not hold.
    EventBoundTooLong,
    Unavailable,
}

pub struct PgCampaignStore {
    pool: Pool,
    /// The membership and feed reads have their own pool: they fail fast (the read deadline) and
    /// never queue behind a write's lock wait.
    reads: Pool,
    tunables: Tunables,
}

impl PgCampaignStore {
    /// Opens the store and, under `Enforced`, refuses to start unless the role's effective
    /// `transaction_timeout` on a fresh connection is positive and at most the feed's bound.
    /// `SET LOCAL` shortens nothing already running, so the role default is the guarantee.
    pub async fn open(
        connector: Box<dyn Connect>,
        read_connector: Box<dyn Connect>,
        pool_size: usize,
        tunables: Tunables,
        policy: TimeoutPolicy,
    ) -> Result<Self, OpenRefusal> {
        tunables.validate().map_err(OpenRefusal::Configuration)?;
        let pool = Pool::new(connector, pool_size);
        let reads = Pool::for_reads(
            read_connector,
            pool_size,
            tunables.membership_read_deadline_ms,
        );
        if policy == TimeoutPolicy::Enforced {
            let (client, driver) = pool
                .fresh_client()
                .await
                .map_err(|_| OpenRefusal::Unavailable)?;
            let answer = client
                .query_one("SELECT current_setting('transaction_timeout')", &[])
                .await;
            let bound = client
                .query_one("SELECT max_transaction_ms FROM campaigns.feed_bound", &[])
                .await;
            driver.abort();
            let text: String = answer
                .and_then(|row| row.try_get(0))
                .map_err(|_| OpenRefusal::Unavailable)?;
            let bound_ms: i32 = bound
                .and_then(|row| row.try_get(0))
                .map_err(|_| OpenRefusal::Unavailable)?;
            let maximum = u64::from(tunables.feed_transaction_timeout_seconds) * 1_000;
            crate::account_closure::check_timeout(
                crate::account_closure::TimeoutPolicy::AtMostMs(maximum),
                &text,
            )
            .map_err(|refusal| match refusal {
                crate::account_closure::TimeoutRefusal::NotSet => OpenRefusal::TimeoutNotSet,
                crate::account_closure::TimeoutRefusal::TooLong => OpenRefusal::TimeoutTooLong,
            })?;
            // The commit-time check must be at least as strict as the configured timeout.
            if u64::try_from(bound_ms).map_or(true, |bound| bound > maximum) {
                return Err(OpenRefusal::EventBoundTooLong);
            }
        }
        Ok(Self {
            pool,
            reads,
            tunables,
        })
    }

    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    pub fn tunables(&self) -> &Tunables {
        &self.tunables
    }

    /// Begins the store's own transaction with the second guard: `SET LOCAL transaction_timeout`.
    async fn begin(&self) -> StoreResult<conn::Tx<'static>> {
        conn::Tx::begin(
            &self.pool,
            self.tunables.feed_transaction_timeout_seconds * 1_000,
        )
        .await
    }

    /// Runs statements in one of the store's own transactions and commits (contract tests of the
    /// timeout and commit-uncertainty behaviour; not part of the production surface).
    #[cfg(test)]
    pub async fn debug_statements(&self, statements: &[&str]) -> StoreResult<()> {
        let tx = self.begin().await?;
        for statement in statements {
            if let Err(error) = tx.query(statement, &[]).await {
                tx.rollback().await;
                return Err(error);
            }
        }
        tx.commit().await
    }

    /// The first column of the first row of a query, as text, read in the store's transaction.
    #[cfg(test)]
    pub async fn debug_text(&self, sql: &str) -> StoreResult<String> {
        let tx = self.begin().await?;
        let row = match tx.query_one(sql, &[]).await {
            Ok(row) => row,
            Err(error) => {
                tx.rollback().await;
                return Err(error);
            }
        };
        let text: String = row
            .try_get(0)
            .map_err(|_| df_campaign::errors::StoreError::InvalidRecord)?;
        tx.commit().await?;
        Ok(text)
    }

    /// The participant B registers for account closure.
    pub fn closure_participant(
        &self,
        clock: std::sync::Arc<dyn df_campaign::clock::Clock>,
    ) -> PgClosureParticipant {
        PgClosureParticipant::new(self.tunables.clone(), clock)
    }
}

/// Commits on success, rolls back on any failure. A failed commit is `Unknown`, never mapped to
/// failure or success.
async fn finish<T>(tx: conn::Tx<'static>, result: Outcome<T>) -> Outcome<T> {
    match result {
        Ok(value) => tx.commit().await.map(|()| value).map_err(Failure::Store),
        Err(failure) => {
            tx.rollback().await;
            Err(failure)
        }
    }
}

impl CampaignStore for PgCampaignStore {
    async fn member_snapshot(
        &self,
        account: AccountId,
        campaign: CampaignId,
    ) -> StoreResult<Option<MemberSnapshot>> {
        feed::member_snapshot(&self.reads, account, campaign).await
    }

    async fn campaign_heads(&self, campaigns: &[CampaignId]) -> StoreResult<Vec<CampaignHead>> {
        feed::campaign_heads(&self.reads, campaigns).await
    }

    async fn campaign_heads_after(
        &self,
        after: Option<CampaignId>,
        limit: u32,
    ) -> StoreResult<Vec<CampaignHead>> {
        feed::campaign_heads_after(&self.reads, after, limit).await
    }

    async fn campaign_changes_since(
        &self,
        lower_bound_ms: u64,
        after: Option<ChangeKey>,
        limit: u32,
    ) -> StoreResult<Vec<CampaignChange>> {
        feed::campaign_changes_since(&self.reads, lower_bound_ms, after, limit).await
    }

    async fn events_after(
        &self,
        campaign: CampaignId,
        after_seq: u64,
        limit: u32,
    ) -> StoreResult<Vec<EventRow>> {
        feed::events_after(&self.reads, campaign, after_seq, limit).await
    }

    async fn database_time_ms(&self) -> StoreResult<u64> {
        feed::database_time_ms(&self.reads).await
    }

    async fn campaign_counts(&self) -> StoreResult<Vec<(String, u64)>> {
        let tx = self.begin().await?;
        let counts = q::campaign_counts(&tx).await;
        match counts {
            Ok(counts) => tx.commit().await.map(|()| counts),
            Err(error) => {
                tx.rollback().await;
                Err(error)
            }
        }
    }

    async fn create_campaign(
        &self,
        new: NewCampaign,
        operation: OperationId,
        fingerprint: [u8; 32],
        now_ms: u64,
    ) -> Outcome<CampaignRow> {
        let tx = self.begin().await?;
        let result =
            campaigns::create_campaign(&tx, &self.tunables, new, operation, fingerprint, now_ms)
                .await;
        finish(tx, result).await
    }

    async fn archive_campaign(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        typed_name: String,
        session_past_lobby: bool,
        now_ms: u64,
    ) -> Outcome<()> {
        let tx = self.begin().await?;
        let result = campaigns::archive_campaign(
            &tx,
            actor,
            campaign,
            &typed_name,
            session_past_lobby,
            now_ms,
        )
        .await;
        finish(tx, result).await
    }

    async fn unarchive_campaign(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        now_ms: u64,
    ) -> Outcome<()> {
        let tx = self.begin().await?;
        let result = campaigns::unarchive_campaign(&tx, actor, campaign, now_ms).await;
        finish(tx, result).await
    }

    async fn request_delete(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        typed_name: String,
        session_past_lobby: bool,
        now_ms: u64,
    ) -> Outcome<()> {
        let tx = self.begin().await?;
        let result = campaigns::request_delete(
            &tx,
            actor,
            campaign,
            &typed_name,
            session_past_lobby,
            now_ms,
        )
        .await;
        finish(tx, result).await
    }

    async fn execute_scheduled_deletions(&self, now_ms: u64) -> Outcome<RetentionReport> {
        let due = self
            .pool
            .read(
                "SELECT campaign_id FROM campaigns.campaign WHERE status = 'archived' \
                 AND scheduled_delete_at_ms IS NOT NULL AND scheduled_delete_at_ms <= $1 \
                 ORDER BY campaign_id LIMIT 100",
                &[&rows::to_i64(now_ms)?],
            )
            .await?;
        let mut report = RetentionReport::default();
        for row in &due {
            let bytes: &[u8] = row
                .try_get(0)
                .map_err(|_| df_campaign::errors::StoreError::InvalidRecord)?;
            let id = rows::campaign_id(bytes)?;
            // One transaction per campaign: the flip is decided under that campaign's lock.
            let tx = self.begin().await?;
            let result = campaigns::flip_scheduled_deletion(&tx, id, now_ms).await;
            if finish(tx, result).await? {
                report.deletions_started += 1;
            }
        }
        let tx = self.begin().await?;
        let expired = tx
            .execute(
                "UPDATE campaigns.display_pairing SET state = 'expired', user_code = NULL \
                 WHERE state IN ('pending', 'approved') AND expires_at_ms <= $1",
                &[&rows::to_i64(now_ms)?],
            )
            .await;
        let expired = finish(tx, expired.map_err(Failure::Store)).await?;
        report.pairings_expired = u32::try_from(expired).unwrap_or(u32::MAX);
        Ok(report)
    }

    async fn create_invitation(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        invitation: NewInvitation,
        now_ms: u64,
    ) -> Outcome<InvitationRow> {
        let tx = self.begin().await?;
        let result = invites::create_invitation(&tx, actor, campaign, invitation, now_ms).await;
        finish(tx, result).await
    }

    async fn revoke_invitation(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        invitation: InvitationId,
        now_ms: u64,
    ) -> Outcome<()> {
        let tx = self.begin().await?;
        let result = invites::revoke_invitation(&tx, actor, campaign, invitation, now_ms).await;
        finish(tx, result).await
    }

    async fn preview_invitation(
        &self,
        token_digest: [u8; 32],
        now_ms: u64,
    ) -> Outcome<PreviewFacts> {
        let tx = self.begin().await?;
        let result = invites::preview_invitation(&tx, &self.tunables, token_digest, now_ms).await;
        finish(tx, result).await
    }

    async fn accept_invitation(
        &self,
        account: AccountId,
        token_digest: [u8; 32],
        operation: OperationId,
        now_ms: u64,
    ) -> Outcome<Accepted> {
        let tx = self.begin().await?;
        let result = invites::accept_invitation(
            &tx,
            &self.tunables,
            account,
            token_digest,
            operation,
            now_ms,
        )
        .await;
        finish(tx, result).await
    }

    async fn rotate_code(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        code_digest: [u8; 32],
        lifetime_ms: u64,
        now_ms: u64,
    ) -> Outcome<CodeRow> {
        let tx = self.begin().await?;
        let result =
            invites::rotate_code(&tx, actor, campaign, code_digest, lifetime_ms, now_ms).await;
        finish(tx, result).await
    }

    async fn lookup_code(
        &self,
        account: AccountId,
        code_digest: [u8; 32],
        now_ms: u64,
    ) -> Outcome<PreviewFacts> {
        let tx = self.begin().await?;
        let result = invites::lookup_code(&tx, account, code_digest, now_ms).await;
        finish(tx, result).await
    }

    async fn request_join(
        &self,
        account: AccountId,
        code_digest: [u8; 32],
        now_ms: u64,
    ) -> Outcome<RequestRow> {
        let tx = self.begin().await?;
        let result = invites::request_join(&tx, &self.tunables, account, code_digest, now_ms).await;
        finish(tx, result).await
    }

    async fn decide_request(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        request: RequestId,
        decision: Decision,
        now_ms: u64,
    ) -> Outcome<Option<MemberRow>> {
        let tx = self.begin().await?;
        let result = invites::decide_request(
            &tx,
            &self.tunables,
            actor,
            campaign,
            request,
            decision,
            now_ms,
        )
        .await;
        finish(tx, result).await
    }

    async fn remove_member(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        member: MemberId,
        now_ms: u64,
    ) -> Outcome<()> {
        let tx = self.begin().await?;
        let result = members::remove_member(&tx, actor, campaign, member, now_ms).await;
        finish(tx, result).await
    }

    async fn leave_campaign(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        now_ms: u64,
    ) -> Outcome<()> {
        let tx = self.begin().await?;
        let result = members::leave_campaign(&tx, actor, campaign, now_ms).await;
        finish(tx, result).await
    }

    async fn allow_rejoin(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        member: MemberId,
        now_ms: u64,
    ) -> Outcome<()> {
        let tx = self.begin().await?;
        let result = members::allow_rejoin(&tx, actor, campaign, member, now_ms).await;
        finish(tx, result).await
    }

    async fn set_co_host(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        member: MemberId,
        delegations: Delegations,
        now_ms: u64,
    ) -> Outcome<()> {
        let tx = self.begin().await?;
        let result = members::set_co_host(&tx, actor, campaign, member, delegations, now_ms).await;
        finish(tx, result).await
    }

    async fn demote_co_host(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        member: MemberId,
        now_ms: u64,
    ) -> Outcome<()> {
        let tx = self.begin().await?;
        let result =
            members::demote_co_host(&tx, &self.tunables, actor, campaign, member, now_ms).await;
        finish(tx, result).await
    }

    async fn set_seat(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        seated: bool,
        now_ms: u64,
    ) -> Outcome<()> {
        let tx = self.begin().await?;
        let result = members::set_seat(&tx, actor, campaign, seated, now_ms).await;
        finish(tx, result).await
    }

    async fn set_player_role(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        member: MemberId,
        role: JoinRole,
        now_ms: u64,
    ) -> Outcome<()> {
        let tx = self.begin().await?;
        let result =
            members::set_player_role(&tx, &self.tunables, actor, campaign, member, role, now_ms)
                .await;
        finish(tx, result).await
    }

    async fn propose_transfer(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        proposal: NewProposal,
        now_ms: u64,
    ) -> Outcome<ProposalId> {
        let tx = self.begin().await?;
        let result =
            transfers::propose_transfer(&tx, &self.tunables, actor, campaign, proposal, now_ms)
                .await;
        finish(tx, result).await
    }

    async fn accept_transfer(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        proposal: ProposalId,
        now_ms: u64,
    ) -> Outcome<TransferKind> {
        let tx = self.begin().await?;
        let result = transfers::accept_transfer(&tx, actor, campaign, proposal, now_ms).await;
        finish(tx, result).await
    }

    async fn cancel_transfer(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        proposal: ProposalId,
        _now_ms: u64,
    ) -> Outcome<()> {
        let tx = self.begin().await?;
        let result = transfers::cancel_transfer(&tx, actor, campaign, proposal).await;
        finish(tx, result).await
    }

    async fn begin_pairing(
        &self,
        new: NewPairing,
        lifetime_ms: u64,
        now_ms: u64,
    ) -> Outcome<PairingRow> {
        let tx = self.begin().await?;
        let result = display::begin_pairing(&tx, &self.tunables, new, lifetime_ms, now_ms).await;
        finish(tx, result).await
    }

    async fn pairing_status(&self, device_digest: [u8; 32], now_ms: u64) -> Outcome<PairingRow> {
        let tx = self.begin().await?;
        let result = display::pairing_status(&tx, device_digest, now_ms).await;
        finish(tx, result).await
    }

    async fn lookup_pairing(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        user_code: String,
        now_ms: u64,
    ) -> Outcome<PairingRow> {
        let tx = self.begin().await?;
        let result = display::lookup_pairing(&tx, actor, campaign, &user_code, now_ms).await;
        finish(tx, result).await
    }

    async fn approve_pairing(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        session: SessionId,
        pairing: PairingId,
        now_ms: u64,
    ) -> Outcome<()> {
        let tx = self.begin().await?;
        let result = display::approve_pairing(
            &tx,
            &self.tunables,
            actor,
            campaign,
            session,
            pairing,
            now_ms,
        )
        .await;
        finish(tx, result).await
    }

    async fn redeem_pairing(
        &self,
        device_digest: [u8; 32],
        presented_ticket: Option<[u8; 32]>,
        new_ticket_digest: [u8; 32],
        session_usable: bool,
        now_ms: u64,
    ) -> Outcome<Redeemed> {
        let tx = self.begin().await?;
        let result = display::redeem_pairing(
            &tx,
            &self.tunables,
            device_digest,
            presented_ticket,
            new_ticket_digest,
            session_usable,
            now_ms,
        )
        .await;
        finish(tx, result).await
    }

    async fn verify_ticket(
        &self,
        ticket_digest: [u8; 32],
        device_digest: [u8; 32],
        session_usable: bool,
        now_ms: u64,
    ) -> Outcome<VerifiedTicket> {
        let tx = self.begin().await?;
        let result =
            display::verify_ticket(&tx, ticket_digest, device_digest, session_usable, now_ms).await;
        finish(tx, result).await
    }

    async fn extend_ticket(
        &self,
        ticket_digest: [u8; 32],
        device_digest: [u8; 32],
        session_usable: bool,
        now_ms: u64,
    ) -> Outcome<TicketRow> {
        let tx = self.begin().await?;
        let result = display::extend_ticket(
            &tx,
            &self.tunables,
            ticket_digest,
            device_digest,
            session_usable,
            now_ms,
        )
        .await;
        finish(tx, result).await
    }

    async fn unpair(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        session: SessionId,
        ticket: Option<TicketId>,
        now_ms: u64,
    ) -> Outcome<u32> {
        let tx = self.begin().await?;
        let result = display::unpair(&tx, actor, campaign, session, ticket, now_ms).await;
        finish(tx, result).await
    }

    async fn revoke_session_tickets(
        &self,
        session: SessionId,
        campaign: CampaignId,
        reason: RevokeReason,
        writer_fence: Option<OwnerFence>,
        now_ms: u64,
    ) -> Outcome<u32> {
        let tx = self.begin().await?;
        let result =
            display::revoke_session_tickets(&tx, session, campaign, reason, writer_fence, now_ms)
                .await;
        finish(tx, result).await
    }

    async fn session_started(
        &self,
        campaign: CampaignId,
        fence: OwnerFence,
        _now_ms: u64,
    ) -> Outcome<()> {
        let tx = self.begin().await?;
        let result = campaigns::session_started(&tx, campaign, fence).await;
        finish(tx, result).await
    }
}

/// The text of the membership snapshot statement, so a test can pin that it is one statement.
#[cfg(test)]
pub fn snapshot_sql() -> &'static str {
    feed::SNAPSHOT_SQL
}
