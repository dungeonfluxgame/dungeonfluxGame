//! Funding readiness reads the finance owner's persisted allocations and consent.
//! A wallet balance or a campaign's sponsor intention never substitutes for that consent.

use crate::database::Connection;
use df_campaign::{
    errors::{StoreError, StoreResult},
    lobby_state::StartReadiness,
    store::FundingDirectory,
};
use df_types::{CampaignId, SessionId};
use tokio::sync::Mutex;

const FEED_BATCH: i64 = 256;

pub(super) struct Funding(Mutex<Connection>);

impl Funding {
    pub(super) fn new(connection: Connection) -> Self {
        Self(Mutex::new(connection))
    }

    async fn consume(&self, campaign: CampaignId) -> StoreResult<()> {
        // The connection owns this bounded transaction; cancellation rolls it back on drop.
        let mut connection = self.0.lock().await;
        let tx = connection.client.transaction().await.map_err(failure)?;
        let head = tx
            .query_opt(
                "SELECT feed_epoch FROM campaigns.campaign WHERE campaign_id = $1",
                &[&campaign.as_bytes().as_slice()],
            )
            .await
            .map_err(failure)?
            .ok_or(StoreError::InvalidRecord)?;
        let epoch: Vec<u8> = head.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
        tx.execute("INSERT INTO finance.funding_feed_position (campaign_id, last_event_seq, feed_epoch, updated_at_ms) VALUES ($1, 0, $2, 0) ON CONFLICT DO NOTHING", &[&campaign.as_bytes().as_slice(), &epoch]).await.map_err(failure)?;
        let position = tx.query_one("SELECT last_event_seq, feed_epoch FROM finance.funding_feed_position WHERE campaign_id = $1 FOR UPDATE", &[&campaign.as_bytes().as_slice()]).await.map_err(failure)?;
        let after: i64 = position.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
        let previous: Vec<u8> = position.try_get(1).map_err(|_| StoreError::InvalidRecord)?;
        if epoch != previous {
            return Err(StoreError::InvalidRecord);
        }
        let events = tx.query("SELECT event_seq, kind, member_id, feed_epoch FROM campaigns.campaign_event WHERE campaign_id = $1 AND event_seq > $2 ORDER BY event_seq LIMIT $3", &[&campaign.as_bytes().as_slice(), &after, &FEED_BATCH]).await.map_err(failure)?;
        let mut next = after;
        for event in events {
            let seq: i64 = event.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
            let kind: &str = event.try_get(1).map_err(|_| StoreError::InvalidRecord)?;
            let kind =
                df_campaign::records::EventKind::parse(kind).ok_or(StoreError::InvalidRecord)?;
            let event_epoch: Vec<u8> = event.try_get(3).map_err(|_| StoreError::InvalidRecord)?;
            if event_epoch != epoch {
                return Err(StoreError::InvalidRecord);
            }
            if next.checked_add(1) != Some(seq) {
                return Err(StoreError::InvalidRecord);
            }
            if matches!(
                kind,
                df_campaign::records::EventKind::Left | df_campaign::records::EventKind::Removed
            ) {
                let member: Option<Vec<u8>> =
                    event.try_get(2).map_err(|_| StoreError::InvalidRecord)?;
                let member = member.ok_or(StoreError::InvalidRecord)?;
                let allocation = tx.query_one("SELECT EXISTS (SELECT 1 FROM finance.allocation a JOIN finance.session_funding s USING (session_id) JOIN campaigns.campaign_member m ON m.account_id = a.payer_account_id AND m.campaign_id = s.campaign_id WHERE s.campaign_id = $1 AND m.member_id = $2 AND a.status = 'active')", &[&campaign.as_bytes().as_slice(), &member]).await.map_err(failure)?;
                if allocation
                    .try_get::<_, bool>(0)
                    .map_err(|_| StoreError::InvalidRecord)?
                {
                    // FIN-08: only finance's departure flow may release unreserved credit.
                    return Err(StoreError::Unavailable);
                }
            }
            tx.execute("INSERT INTO finance.funding_effect (campaign_id, event_seq, effect, applied_at_ms) VALUES ($1, $2, 'none', (extract(epoch FROM clock_timestamp()) * 1000)::bigint) ON CONFLICT DO NOTHING", &[&campaign.as_bytes().as_slice(), &seq]).await.map_err(failure)?;
            next = seq;
        }
        tx.execute("UPDATE finance.funding_feed_position SET last_event_seq = $2, updated_at_ms = (extract(epoch FROM clock_timestamp()) * 1000)::bigint WHERE campaign_id = $1", &[&campaign.as_bytes().as_slice(), &next]).await.map_err(failure)?;
        tx.commit().await.map_err(|error| {
            failure(error);
            StoreError::Unknown
        })?;
        if next > after {
            df_observe::record_audit_line(
                "hosted.funding_feed_applied",
                format!(
                    "SPEND funding_feed_applied campaign={} event_seq={next}",
                    df_commerce::finance_ids::hex(campaign.as_bytes())
                ),
            );
        }
        Ok(())
    }
}

fn failure(error: tokio_postgres::Error) -> StoreError {
    df_observe::record_audit_line(
        "hosted.funding_store_failed",
        format!(
            "SPEND funding_store_failed sqlstate={}",
            error.code().map_or("connection", |code| code.code())
        ),
    );
    StoreError::Unavailable
}

impl FundingDirectory for Funding {
    async fn membership_event(&self, campaign: CampaignId, _event_seq: u64) {
        if self.consume(campaign).await.is_err() {
            df_observe::record_audit_line(
                "hosted.funding_feed_pending",
                format!(
                    "SPEND funding_feed_pending campaign={}",
                    df_commerce::finance_ids::hex(campaign.as_bytes())
                ),
            );
        }
    }

    async fn start_readiness(
        &self,
        campaign: CampaignId,
        session: SessionId,
    ) -> StoreResult<StartReadiness> {
        self.consume(campaign).await?;
        let ready = {
            match df_finance::host_funding::start_fundable(
                &self.0.lock().await.client,
                df_commerce::finance_ids::CampaignKey::from_array(*campaign.as_bytes()),
                df_commerce::finance_ids::SessionKey::from_array(*session.as_bytes()),
                now_ms()?,
            )
            .await
            {
                Ok(ready) => ready,
                Err(df_finance::host_funding::Error::Finance(error)) => {
                    df_observe::record_audit_line(
                        "hosted.funding_unavailable",
                        format!(
                            "SPEND funding_unavailable sqlstate={}",
                            error.sqlstate().unwrap_or("connection")
                        ),
                    );
                    return Err(StoreError::Unavailable);
                }
                Err(_) => false,
            }
        };
        Ok(StartReadiness {
            ready,
            waiting_for_sponsor: !ready,
            no_charge_until_start: true,
        })
    }

    async fn open_session_funding(
        &self,
        campaign: CampaignId,
        session: SessionId,
    ) -> StoreResult<()> {
        self.consume(campaign).await?;
        df_finance::host_funding::open(
            &mut self.0.lock().await.client,
            df_commerce::finance_ids::CampaignKey::from_array(*campaign.as_bytes()),
            df_commerce::finance_ids::SessionKey::from_array(*session.as_bytes()),
            now_ms()?,
        )
        .await
        .map_err(|error| {
            let sqlstate = match &error {
                df_finance::host_funding::Error::Finance(error) => {
                    error.sqlstate().unwrap_or("connection")
                }
                _ => "refused",
            };
            df_observe::record_audit_line(
                "hosted.host_funding_refused",
                format!(
                    "SPEND host_funding_refused session={} sqlstate={sqlstate}",
                    df_commerce::finance_ids::hex(session.as_bytes())
                ),
            );
            match error {
                df_finance::host_funding::Error::Finance(
                    df_finance::service::FinanceError::Database(_),
                ) => StoreError::Unknown,
                _ => StoreError::Unavailable,
            }
        })?;
        df_observe::record_audit_line(
            "hosted.host_funding_opened",
            format!(
                "SPEND host_funding_opened session={}",
                df_commerce::finance_ids::hex(session.as_bytes())
            ),
        );
        Ok(())
    }
}

fn now_ms() -> StoreResult<i64> {
    i64::try_from(df_customer::Clock::now_ms(&df_customer::SystemClock))
        .map_err(|_| StoreError::InvalidRecord)
}
