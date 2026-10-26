//! Bounded customer-safe directory and finance read models; never grant trial credit on GET.
use crate::database::Connection;
use df_customer::integration::TokenId;
use df_portal::integration_api::{
    CampaignSummary, FundingSummary, IntegrationReads, Page, ReadFailure, SessionSummary, timestamp,
};
use df_types::AccountId;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_postgres::Row;

#[derive(Clone)]
pub(crate) struct Reads(Arc<Mutex<Connection>>);
impl Reads {
    pub(crate) fn new(connection: Connection) -> Self {
        Self(Arc::new(Mutex::new(connection)))
    }
}
fn id(row: &Row, index: usize) -> Result<TokenId, ReadFailure> {
    let bytes: Vec<u8> = row.try_get(index).map_err(|_| ReadFailure)?;
    Ok(TokenId(bytes.try_into().map_err(|_| ReadFailure)?))
}
fn role(row:&Row,index:usize)->Result<df_account_app::wire_campaign::CampaignRole,ReadFailure>{
    use df_account_app::wire_campaign::CampaignRole;
    match row.try_get::<_,&str>(index).map_err(|_|ReadFailure)?{
        "host"=>Ok(CampaignRole::Host),"co_host"=>Ok(CampaignRole::CoHost),
        "player"=>Ok(CampaignRole::Player),"contributor"=>Ok(CampaignRole::Contributor),_=>Err(ReadFailure),
    }
}
fn page<T>(mut rows: Vec<(TokenId, T)>, limit: u32) -> Page<T> {
    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next = if has_more {
        rows.last().map(|r| r.0)
    } else {
        None
    };
    Page {
        rows: rows.into_iter().map(|r| r.1).collect(),
        next,
    }
}
fn number(row: &Row, index: usize) -> Result<u64, ReadFailure> {
    u64::try_from(row.try_get::<_, i64>(index).map_err(|_| ReadFailure)?).map_err(|_| ReadFailure)
}
impl IntegrationReads for Reads {
    async fn campaigns(
        &self,
        account: AccountId,
        after: Option<TokenId>,
        limit: u32,
    ) -> Result<Page<CampaignSummary>, ReadFailure> {
        let connection = self.0.lock().await;
        let rows = connection.client.query("SELECT c.campaign_id,c.name,m.role FROM campaigns.campaign c JOIN \
                    campaigns.campaign_member m USING(campaign_id) WHERE m.account_id = $1 AND m.status = \
                    'active' AND c.status IN ('active','archived') AND ($2::bytea IS NULL OR \
                    c.campaign_id > $2) ORDER BY c.campaign_id LIMIT $3", &[&account.as_bytes().as_slice(), &after.map(|id| id.0.to_vec()), &i64::from(limit.min(100)+1)]).await.map_err(|_| ReadFailure)?;
        let values = rows
            .iter()
            .map(|r| {
                let key = id(r, 0)?;
                Ok((
                    key,
                    CampaignSummary {
                        campaign_id: key.text(),
                        name: r.try_get(1).map_err(|_| ReadFailure)?,
                        role: role(r,2)?,
                    },
                ))
            })
            .collect::<Result<Vec<_>, ReadFailure>>()?;
        Ok(page(values, limit.min(100)))
    }
    async fn sessions(
        &self,
        account: AccountId,
        after: Option<TokenId>,
        limit: u32,
    ) -> Result<Page<SessionSummary>, ReadFailure> {
        let connection = self.0.lock().await;
        let rows = connection.client.query(
            "SELECT s.session_id,c.campaign_id,c.name,l.frozen_at_ms, \
             (extract(epoch FROM s.ended_at)*1000)::bigint,m.role, \
             (SELECT count(*) FROM campaigns.lobby_member roster \
              WHERE roster.session_id=s.session_id AND roster.frozen_at_ms IS NOT NULL) \
             FROM df_runtime.table_sessions s JOIN campaigns.campaign c USING(campaign_id) \
             JOIN campaigns.campaign_member m USING(campaign_id) \
             JOIN campaigns.lobby_member l ON l.member_id=m.member_id \
                 AND l.session_id=s.session_id AND l.campaign_id=c.campaign_id \
             WHERE m.account_id=$1 AND m.status='active' AND c.status IN ('active','archived') \
                 AND l.frozen_at_ms IS NOT NULL \
                 AND ($2::bytea IS NULL OR s.session_id>$2) ORDER BY s.session_id LIMIT $3",
            &[&account.as_bytes().as_slice(), &after.map(|id| id.0.to_vec()), &i64::from(limit.min(100)+1)],
        ).await.map_err(|_| ReadFailure)?;
        let values = rows
            .iter()
            .map(|r| {
                let key = id(r, 0)?;
                let start = number(r, 3)?;
                let end: Option<i64> = r.try_get(4).map_err(|_| ReadFailure)?;
                let end = end
                    .map(|value| u64::try_from(value).map_err(|_| ReadFailure))
                    .transpose()?;
                Ok((
                    key,
                    SessionSummary {
                        session_id: key.text(),
                        campaign_id: id(r, 1)?.text(),
                        campaign_name: r.try_get(2).map_err(|_| ReadFailure)?,
                        started_at: timestamp(start),
                        ended_at: end.map(timestamp),
                        duration_seconds: end.map_or(0, |end| end.saturating_sub(start) / 1000),
                        role: role(r,5)?,
                        players: u32::try_from(number(r,6)?).map_err(|_|ReadFailure)?,
                    },
                ))
            })
            .collect::<Result<Vec<_>, ReadFailure>>()?;
        Ok(page(values, limit.min(100)))
    }
    async fn funding(&self, account: AccountId) -> Result<FundingSummary, ReadFailure> {
        let connection = self.0.lock().await;
        let trial=connection.client.query_opt("SELECT l.original_micro_dollars, \
             l.original_micro_dollars-l.consumed_micro_dollars-l.expired_micro_dollars-l.forfeited_micro_dollars \
             FROM finance.trial_grant t JOIN finance.funding_lot l USING(lot_id) WHERE t.account_id=$1",
             &[&account.as_bytes().as_slice()]).await.map_err(|_|ReadFailure)?;
        let row = connection.client.query_one("SELECT coalesce(sum(b.natural_micro_dollars) FILTER (WHERE k.code = \
                    2110),0)::bigint,coalesce(sum(b.natural_micro_dollars) FILTER (WHERE k.code = \
                    2210),0)::bigint,coalesce(sum(b.natural_micro_dollars) FILTER (WHERE k.code IN \
                    (2120,2220)),0)::bigint,coalesce(sum(b.natural_micro_dollars) FILTER (WHERE k.code IN \
                    (2130,2230)),0)::bigint FROM finance.account_balance b JOIN finance.account_key k \
                    USING(key_id) LEFT JOIN finance.allocation a ON a.allocation_id = k.allocation_id \
                    WHERE k.payer_account_id = $1 OR a.payer_account_id = $1", &[&account.as_bytes().as_slice()]).await.map_err(|_| ReadFailure)?;
        Ok(FundingSummary {
            purchased_available_micro_dollars: number(&row, 0)?,
            promotional_available_micro_dollars: number(&row, 1)?,
            allocated_micro_dollars: number(&row, 2)?,
            reserved_micro_dollars: number(&row, 3)?,
            trial_granted_micro_dollars: trial.as_ref().map(|row|number(row,0)).transpose()?,
            trial_remaining_micro_dollars: trial.as_ref().map(|row|number(row,1)).transpose()?,
        })
    }
}
