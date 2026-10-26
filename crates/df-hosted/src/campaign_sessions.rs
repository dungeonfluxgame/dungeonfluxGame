//! PostgreSQL session records. O1 replaces this adapter with its runtime-command facade.
//! Starting requires the real current owner fence; these records never manufacture a lease.

use crate::database::Connection;
use df_campaign::{
    errors::{StoreError, StoreResult},
    lobby_state::SeatedMember,
    store::{SessionDirectory, SessionFacts, SessionPhase},
};
use df_types::{CampaignId, SessionId};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_postgres::Row;

#[derive(Clone)]
pub(super) struct Sessions(Arc<Mutex<Connection>>);

impl Sessions {
    pub(super) async fn cancel_before_start(
        client: &tokio_postgres::Client,
        account: df_types::AccountId,
        session: SessionId,
    ) -> StoreResult<()> {
        let row = client
            .query_one(
                "SELECT df_runtime.cancel_host_lobby($1, $2)",
                &[
                    &session.as_bytes().as_slice(),
                    &account.as_bytes().as_slice(),
                ],
            )
            .await
            .map_err(|error| {
                let unknown = error.as_db_error().is_none();
                query_error(error);
                if unknown {
                    StoreError::Unknown
                } else {
                    StoreError::Unavailable
                }
            })?;
        if !row
            .try_get::<_, bool>(0)
            .map_err(|_| StoreError::InvalidRecord)?
        {
            return Err(StoreError::InvalidRecord);
        }
        df_observe::record_audit_line(
            "hosted.session_cancelled_before_start",
            format!(
                "SESSION cancelled_before_start session={}",
                df_commerce::finance_ids::hex(session.as_bytes())
            ),
        );
        Ok(())
    }

    pub(super) fn new(connection: Connection) -> Self {
        Self(Arc::new(Mutex::new(connection)))
    }
}

fn query_error(error: tokio_postgres::Error) -> StoreError {
    df_observe::record_audit_line(
        "hosted.session_store_failed",
        format!(
            "SESSION store_failed sqlstate={}",
            error.code().map_or("connection", |code| code.code())
        ),
    );
    StoreError::Unavailable
}

fn phase(value: &str) -> StoreResult<SessionPhase> {
    Ok(match value {
        "scheduled" | "lobby" => SessionPhase::Lobby,
        "preparing" => SessionPhase::PreparingFundingOpen,
        "active" | "paused" | "settling" => SessionPhase::Playing,
        "ended" => SessionPhase::Ended,
        _ => return Err(StoreError::InvalidRecord),
    })
}

fn facts(row: Row) -> StoreResult<(SessionId, SessionFacts)> {
    let session: Vec<u8> = row
        .try_get("session_id")
        .map_err(|_| StoreError::InvalidRecord)?;
    let campaign: Vec<u8> = row
        .try_get("campaign_id")
        .map_err(|_| StoreError::InvalidRecord)?;
    let state: &str = row
        .try_get("phase")
        .map_err(|_| StoreError::InvalidRecord)?;
    Ok((
        SessionId::from_bytes(&session).map_err(|_| StoreError::InvalidRecord)?,
        SessionFacts {
            campaign: CampaignId::from_bytes(&campaign).map_err(|_| StoreError::InvalidRecord)?,
            phase: phase(state)?,
        },
    ))
}

impl SessionDirectory for Sessions {
    async fn session(&self, session: SessionId) -> StoreResult<Option<SessionFacts>> {
        self.0.lock().await.client.query_opt(
            "SELECT session_id, campaign_id, phase FROM df_runtime.table_sessions WHERE session_id = $1",
            &[&session.as_bytes().as_slice()],
        ).await.map_err(query_error)?.map(facts).transpose().map(|value| value.map(|(_, facts)| facts))
    }

    async fn live_session_of(
        &self,
        campaign: CampaignId,
    ) -> StoreResult<Option<(SessionId, SessionPhase)>> {
        self.0.lock().await.client.query_opt(
            "SELECT session_id, campaign_id, phase FROM df_runtime.table_sessions WHERE campaign_id = $1 AND phase <> 'ended'",
            &[&campaign.as_bytes().as_slice()],
        ).await.map_err(query_error)?.map(facts).transpose().map(|value| value.map(|(id, facts)| (id, facts.phase)))
    }

    async fn open_session(&self, campaign: CampaignId) -> StoreResult<(SessionId, SessionPhase)> {
        let token =
            df_customer::SecretToken::generate().map_err(|_| StoreError::RandomUnavailable)?;
        let id =
            SessionId::from_hex(&token.text()[..32]).map_err(|_| StoreError::RandomUnavailable)?;
        // O1 row first, then campaign: the same lock order as Start and archive.
        let mut connection = self.0.lock().await;
        let tx = connection.client.transaction().await.map_err(query_error)?;
        let row = tx.query_opt(
            "INSERT INTO df_runtime.table_sessions (tenant_id, campaign_id, session_id, phase, created_at) \
             SELECT tenant_id, campaign_id, $2, 'lobby', clock_timestamp() FROM campaigns.campaign WHERE campaign_id = $1 AND status = 'active' \
             ON CONFLICT (tenant_id, campaign_id) WHERE phase <> 'ended' DO UPDATE SET campaign_id = EXCLUDED.campaign_id \
             RETURNING session_id, campaign_id, phase",
            &[&campaign.as_bytes().as_slice(), &id.as_bytes().as_slice()],
        ).await.map_err(query_error)?.ok_or(StoreError::InvalidRecord)?;
        let (id, facts) = facts(row)?;
        let active = tx.query_opt("SELECT campaign_id FROM campaigns.campaign WHERE campaign_id = $1 AND status = 'active' FOR UPDATE", &[&campaign.as_bytes().as_slice()]).await.map_err(query_error)?;
        if active.is_none() {
            return Err(StoreError::InvalidRecord);
        }
        tx.commit().await.map_err(|error| {
            query_error(error);
            StoreError::Unknown
        })?;
        Ok((id, facts.phase))
    }

    async fn start_session(
        &self,
        session: SessionId,
        campaign: CampaignId,
        roster: &[SeatedMember],
    ) -> StoreResult<()> {
        // This connection serializes one bounded transaction; cancellation rolls it back on drop.
        let mut connection = self.0.lock().await;
        let tx = connection.client.transaction().await.map_err(query_error)?;
        tx.batch_execute("SELECT df_runtime.enter_service()")
            .await
            .map_err(query_error)?;
        let accounts: Vec<Vec<u8>> = roster
            .iter()
            .map(|member| member.account.as_bytes().to_vec())
            .collect();
        let locked = tx.query("SELECT account_id FROM accounts.account WHERE account_id = ANY($1) AND status = 'active' ORDER BY account_id FOR UPDATE", &[&accounts]).await.map_err(query_error)?;
        if locked.len() != roster.len() || roster.is_empty() {
            return Err(StoreError::InvalidRecord);
        }
        let current = tx.query_opt(
            "SELECT s.tenant_id, o.recovery_epoch, o.claim_seq, o.placement_id FROM df_runtime.table_sessions s \
             JOIN df_runtime.campaign_owners o ON o.tenant_id = s.tenant_id AND o.campaign_id = s.campaign_id \
             WHERE s.session_id = $1 AND s.campaign_id = $2 AND s.phase = 'lobby' \
             AND df_runtime.fence_current(o.tenant_id, o.campaign_id, o.recovery_epoch, o.claim_seq, o.placement_id, 'game') FOR UPDATE OF s",
            &[&session.as_bytes().as_slice(), &campaign.as_bytes().as_slice()],
        ).await.map_err(query_error)?.ok_or(StoreError::FenceStale)?;
        let tenant: Vec<u8> = current.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
        let active = tx.query_opt("SELECT campaign_id FROM campaigns.campaign WHERE campaign_id = $1 AND tenant_id = $2 AND status = 'active' FOR UPDATE", &[&campaign.as_bytes().as_slice(), &tenant]).await.map_err(query_error)?;
        if active.is_none() {
            return Err(StoreError::InvalidRecord);
        }
        let rows = tx.query("SELECT m.member_id, m.account_id FROM campaigns.campaign_member m JOIN campaigns.lobby_member l USING (campaign_id, member_id) JOIN accounts.account b ON b.account_id = m.account_id WHERE m.campaign_id = $1 AND l.session_id = $2 AND m.status = 'active' AND m.holds_seat AND b.status = 'active' AND l.ready AND l.character_choice <> 'undecided' ORDER BY m.member_id", &[&campaign.as_bytes().as_slice(), &session.as_bytes().as_slice()]).await.map_err(query_error)?;
        let total = tx.query_one("SELECT count(*) FROM campaigns.campaign_member m JOIN accounts.account b USING (account_id) WHERE m.campaign_id = $1 AND m.status = 'active' AND m.holds_seat AND b.status = 'active'", &[&campaign.as_bytes().as_slice()]).await.map_err(query_error)?;
        let total: i64 = total.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
        if rows.len() != roster.len() || usize::try_from(total) != Ok(roster.len()) {
            return Err(StoreError::InvalidRecord);
        }
        for row in rows {
            let member: Vec<u8> = row.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
            let account: Vec<u8> = row.try_get(1).map_err(|_| StoreError::InvalidRecord)?;
            if !roster.iter().any(|seat| {
                seat.member.as_bytes().as_slice() == member
                    && seat.account.as_bytes().as_slice() == account
            }) {
                return Err(StoreError::InvalidRecord);
            }
        }
        tx.execute("UPDATE campaigns.lobby_member SET frozen_at_ms = (extract(epoch FROM clock_timestamp()) * 1000)::bigint WHERE session_id = $1 AND campaign_id = $2", &[&session.as_bytes().as_slice(), &campaign.as_bytes().as_slice()]).await.map_err(query_error)?;
        tx.execute("UPDATE df_runtime.table_sessions SET phase = 'preparing' WHERE tenant_id = $1 AND session_id = $2", &[&tenant, &session.as_bytes().as_slice()]).await.map_err(query_error)?;
        tx.commit().await.map_err(|error| {
            query_error(error);
            StoreError::Unknown
        })?;
        df_observe::record_audit_line(
            "hosted.session_record_started",
            format!(
                "SESSION record_started campaign={} session={}",
                df_commerce::finance_ids::hex(campaign.as_bytes()),
                df_commerce::finance_ids::hex(session.as_bytes())
            ),
        );
        Ok(())
    }
}

#[cfg(test)]
mod fundingwire_tests {
    use super::*;
    use df_persistence::{account_closure::TimeoutPolicy, account_store::PostgresAccountStore};
    use df_portal::app_api::ports::FinancePort;

    #[tokio::test]
    async fn fundingwire_real_hosted_migrations_publish_prices_and_cancel_returns_reserved_credit()
    {
        let transport = df_persistence::postgres_tls::PostgresTransport::PlaintextLoopbackOnly;
        let config: tokio_postgres::Config =
            "host=127.0.0.1 port=55911 user=postgres dbname=postgres"
                .parse()
                .unwrap();
        let admin = Connection::open(&config, &transport).await.unwrap();
        let token = df_customer::SecretToken::generate().unwrap();
        let database = format!("fundingwire_{}", &token.text()[..16]);
        admin
            .client
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .unwrap();
        let mut config = config;
        config.dbname(&database);
        let accounts = PostgresAccountStore::open(
            config.clone(),
            transport.clone(),
            1,
            TimeoutPolicy::NotChecked,
        )
        .await
        .unwrap();
        let mut db = Connection::open(&config, &transport).await.unwrap();
        crate::database::migrate(&mut db).await.unwrap();
        crate::database::migrate(&mut db).await.unwrap();
        assert!(
            db.client
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM accounts.schema_migration WHERE version = 30)",
                    &[],
                )
                .await
                .unwrap()
                .get::<_, bool>(0)
        );

        let prices: serde_json::Value = serde_json::from_str(
            &db.client
                .query_one("SELECT presets::text FROM finance.price_version", &[])
                .await
                .unwrap()
                .get::<_, String>(0),
        )
        .unwrap();
        assert_eq!(prices["story"]["hourly_ceiling_micro_dollars"], 3_000_000);
        assert_eq!(
            prices["immersive"]["hourly_ceiling_micro_dollars"],
            8_000_000
        );
        assert_eq!(
            prices["cinematic"]["hourly_ceiling_micro_dollars"],
            20_000_000
        );
        let account = df_types::AccountId::from_bytes(&[0x91; 16]).unwrap();
        let campaign = CampaignId::from_bytes(&[0x92; 16]).unwrap();
        db.client.execute("INSERT INTO accounts.account (account_id,email_normalized,email_display,email_verified_at_ms,status,credential_generation,created_at_ms) VALUES ($1,'fundingwire@example.test','fundingwire@example.test',1,'active',1,1)", &[&account.as_bytes().as_slice()]).await.unwrap();
        let tenant: Vec<u8> = db
            .client
            .query_one(
                "INSERT INTO accounts.tenant (owner_account_id) VALUES ($1) RETURNING tenant_id",
                &[&account.as_bytes().as_slice()],
            )
            .await
            .unwrap()
            .get(0);
        db.client.execute("INSERT INTO campaigns.campaign (campaign_id,tenant_id,owner_account_id,name,seat_count,status,created_at_ms,last_event_at_ms,feed_epoch) VALUES ($1,$3,$2,'Funding test',1,'active',1,1,$1)", &[&campaign.as_bytes().as_slice(), &account.as_bytes().as_slice(), &tenant]).await.unwrap();
        db.client.execute("INSERT INTO campaigns.campaign_member (member_id,campaign_id,account_id,role,holds_seat,status,joined_at_ms) VALUES ($1,$1,$2,'host',true,'active',1)", &[&campaign.as_bytes().as_slice(), &account.as_bytes().as_slice()]).await.unwrap();
        let sessions = Sessions::new(Connection::open(&config, &transport).await.unwrap());
        let (session, _) = sessions.open_session(campaign).await.unwrap();
        let finance_connection = Connection::open(&config, &transport).await.unwrap();
        finance_connection
            .client
            .batch_execute("SET ROLE df_finance")
            .await
            .unwrap();
        let finance =
            crate::finance::Finance::new(finance_connection, accounts.clone(), [9; 32]).unwrap();
        finance
            .purchase(account, 300, "fundingwire-cancel")
            .await
            .unwrap();
        let quote = finance
            .host_credit(account, session)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(quote.available_micro_dollars, 3_000_000);
        let operation = df_types::OperationId::from_bytes(&[0x93; 16]).unwrap();
        finance
            .consent_host_credit(
                account,
                session,
                df_account_app::wire_campaign::HostConsentRequest {
                    operation: df_commerce::finance_ids::hex(operation.as_bytes()),
                    price_version: quote.price_version,
                    cap_micro_dollars: quote.cap_micro_dollars,
                    terms_version: quote.terms_version,
                },
                operation,
            )
            .await
            .unwrap();
        db.client
            .batch_execute("SET ROLE df_finance")
            .await
            .unwrap();
        df_finance::host_funding::open(
            &mut db.client,
            df_commerce::finance_ids::CampaignKey::from_array(*campaign.as_bytes()),
            df_commerce::finance_ids::SessionKey::from_array(*session.as_bytes()),
            i64::try_from(df_customer::Clock::now_ms(&df_customer::SystemClock)).unwrap(),
        )
        .await
        .unwrap();
        finance.cancel_host_lobby(account, session).await.unwrap();
        finance.cancel_host_lobby(account, session).await.unwrap();
        assert_eq!(
            sessions.session(session).await.unwrap().unwrap().phase,
            SessionPhase::Ended
        );
        assert_eq!(
            finance
                .overview(account, false)
                .await
                .unwrap()
                .balance
                .purchased_available_micro_dollars,
            3_000_000
        );
        assert_eq!(
            db.client
                .query_one("SELECT status FROM finance.session_funding", &[])
                .await
                .unwrap()
                .get::<_, String>(0),
            "closed"
        );
        drop(finance);
        drop(accounts);
        drop(sessions);
        drop(db);
        admin
            .client
            .batch_execute(&format!("DROP DATABASE {database} WITH (FORCE)"))
            .await
            .unwrap();
    }
}
