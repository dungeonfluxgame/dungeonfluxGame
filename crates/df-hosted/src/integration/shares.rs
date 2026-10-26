//! Campaign-owned curated projections. Current host and exact per-person consent are durable.
mod cleanup;
pub(crate) use cleanup::Cleanup;
use crate::database::Connection;
use df_account_app::wire_share::*;
use df_customer::integration::TokenId;
use df_portal::share::{PublicSummary, ShareFailure, Sharing};
use df_types::AccountId;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_postgres::{GenericClient, Row, Transaction};
type Result<T> = std::result::Result<T, ShareFailure>;
#[derive(Clone)]
pub(crate) struct Shares(Arc<Mutex<Connection>>);
impl Shares {
    pub(crate) fn new(connection: Connection) -> Self {
        Self(Arc::new(Mutex::new(connection)))
    }
}
fn failed(error: tokio_postgres::Error) -> ShareFailure {
    df_observe::record_audit_line("campaign.share_store_failed",format!("CAMPAIGN share_store_failed sqlstate={}",error.code().map_or("connection",|code|code.code())));
    ShareFailure::Unavailable
}
fn id(text: &str) -> Result<Vec<u8>> {
    TokenId::parse(text)
        .map(|t| t.0.to_vec())
        .ok_or(ShareFailure::NotFound)
}
fn valid(text: &str, maximum: usize) -> bool {
    !text.trim().is_empty()
        && text.chars().count() <= maximum
        && !text.chars().any(char::is_control)
}
fn hex(bytes: &[u8]) -> String {
    df_campaign::secrets::hex(bytes)
}
async fn begin(connection: &mut Connection) -> Result<Transaction<'_>> {
    let tx = connection.client.transaction().await.map_err(failed)?;
    tx.batch_execute("SET LOCAL transaction_timeout='10s'")
        .await
        .map_err(failed)?;
    Ok(tx)
}
async fn now(client: &impl GenericClient) -> Result<i64> {
    client
        .query_one(
            "SELECT (extract(epoch FROM clock_timestamp())*1000)::bigint",
            &[],
        )
        .await
        .map_err(failed)?
        .try_get(0)
        .map_err(failed)
}
async fn host(tx: &Transaction<'_>, account: AccountId, session: &[u8]) -> Result<Row> {
    tx.query_opt("SELECT c.campaign_id,to_char(s.created_at AT TIME ZONE \
                    'UTC','YYYY-MM-DD'),GREATEST(0,round(extract(epoch FROM \
                    (s.ended_at-s.created_at))/60))::bigint FROM df_runtime.table_sessions s JOIN \
                    campaigns.campaign c USING(campaign_id) JOIN campaigns.campaign_member m \
                    USING(campaign_id) JOIN accounts.account a ON a.account_id=m.account_id WHERE \
                    s.session_id=$1 AND s.phase='ended' AND c.status IN ('active','archived') AND \
                    m.account_id=$2 AND m.role='host' AND m.status='active' AND a.status='active' AND \
                    a.email_verified_at_ms IS NOT NULL FOR UPDATE OF c",&[&session,&account.as_bytes().as_slice()]).await.map_err(failed)?.ok_or(ShareFailure::NotFound)
}
async fn preview(
    client: &impl GenericClient,
    account: AccountId,
    session: &[u8],
) -> Result<SharePreview> {
    let row=client.query_opt("SELECT \
                    COALESCE(p.revision,''),COALESCE(p.public_title,c.name),COALESCE(p.highlights,ARRAY[]::text[]),COALESCE(p.public_date,to_char(s.created_at AT TIME ZONE 'UTC','YYYY-MM-DD')),COALESCE(p.duration_minutes,GREATEST(0,round(extract(epoch FROM (s.ended_at-s.created_at))/60))::bigint),m.role='host' FROM df_runtime.table_sessions s JOIN campaigns.campaign c USING(campaign_id) JOIN campaigns.campaign_member m USING(campaign_id) JOIN accounts.account a ON a.account_id=m.account_id LEFT JOIN campaigns.share_preview p ON p.session_id=s.session_id WHERE s.session_id=$1 AND s.phase='ended' AND c.status IN ('active','archived') AND m.account_id=$2 AND m.status='active' AND a.status='active'",&[&session,&account.as_bytes().as_slice()]).await.map_err(failed)?.ok_or(ShareFailure::NotFound)?;
    let people = client
        .query(
            "SELECT account_id,public_name,consented_at_ms IS NOT NULL FROM \
                    campaigns.share_consent WHERE session_id=$1 ORDER BY account_id",
            &[&session],
        )
        .await
        .map_err(failed)?;
    let mut included = Vec::new();
    let mut consented = Vec::new();
    for person in people {
        let bytes: Vec<u8> = person.try_get(0).map_err(failed)?;
        let account = hex(&bytes);
        included.push(SharePerson {
            account_id: account.clone(),
            public_name: person.try_get(1).map_err(failed)?,
        });
        if person.try_get::<_, bool>(2).map_err(failed)? {
            consented.push(account);
        }
    }
    let candidates=client.query("SELECT m.account_id,COALESCE(a.display_name,'Party member') FROM \
                    df_runtime.table_sessions s JOIN campaigns.campaign_member m USING(campaign_id) JOIN \
                    accounts.account a ON a.account_id=m.account_id WHERE s.session_id=$1 AND \
                    m.status='active' AND a.status='active' ORDER BY m.account_id LIMIT 100",&[&session]).await.map_err(failed)?.iter().map(|r|{let bytes:Vec<u8>=r.try_get(0).map_err(failed)?;Ok(SharePerson{account_id:hex(&bytes),public_name:r.try_get(1).map_err(failed)?})}).collect::<Result<Vec<_>>>()?;
    let can_consent=included.iter().any(|person|person.account_id==hex(account.as_bytes()));
    Ok(SharePreview {
        session_id: hex(session),
        revision: row.try_get(0).map_err(failed)?,
        draft: ShareDraft {
            title: row.try_get(1).map_err(failed)?,
            highlights: row.try_get(2).map_err(failed)?,
            people: included,
        },
        date: row.try_get(3).map_err(failed)?,
        duration_minutes: u64::try_from(row.try_get::<_, i64>(4).map_err(failed)?)
            .map_err(|_| ShareFailure::Unavailable)?,
        consented_accounts: consented,
        can_publish: row.try_get(5).map_err(failed)?,
        can_consent,
        candidates,
    })
}
fn link(row: &Row) -> Result<ShareLinkView> {
    let bytes: Vec<u8> = row.try_get(0).map_err(failed)?;
    Ok(ShareLinkView {
        link_id: hex(&bytes),
        expires_at_ms: u64::try_from(row.try_get::<_, i64>(1).map_err(failed)?)
            .map_err(|_| ShareFailure::Unavailable)?,
        revoked: row.try_get(2).map_err(failed)?,
        allow_indexing: row.try_get(3).map_err(failed)?,
        allow_embeds: row.try_get(4).map_err(failed)?,
    })
}
fn audit(event: &str, session: &[u8]) {
    df_observe::record_audit_line(
        "campaign.share",
        format!("CAMPAIGN share status={event} session={}", hex(session)),
    );
}

impl Sharing for Shares {
    async fn preview(&self, account: AccountId, session: &str) -> Result<SharePreview> {
        let session = id(session)?;
        let c = self.0.lock().await;
        preview(&c.client, account, &session).await
    }
    async fn save_preview(
        &self,
        account: AccountId,
        session: &str,
        mut draft: ShareDraft,
    ) -> Result<SharePreview> {
        let session = id(session)?;
        if !valid(&draft.title, 120)
            || draft.highlights.len() > 5
            || draft.highlights.iter().any(|s| !valid(s, 500))
            || draft.people.len() > 32
            || draft.people.iter().any(|p| !valid(&p.public_name, 80))
        {
            return Err(ShareFailure::Invalid);
        }
        draft.people.sort_by(|a, b| a.account_id.cmp(&b.account_id));
        if draft
            .people
            .windows(2)
            .any(|p| p[0].account_id == p[1].account_id)
        {
            return Err(ShareFailure::Invalid);
        }
        let mut c = self.0.lock().await;
        let tx = begin(&mut c).await?;
        let source = host(&tx, account, &session).await?;
        let campaign: Vec<u8> = source.try_get(0).map_err(failed)?;
        let date: String = source.try_get(1).map_err(failed)?;
        let duration: i64 = source.try_get(2).map_err(failed)?;
        for person in &draft.people {
            let person = id(&person.account_id)?;
            if tx.query_opt("SELECT 1 FROM campaigns.campaign_member m JOIN accounts.account a USING(account_id) \
                    WHERE m.campaign_id=$1 AND m.account_id=$2 AND m.status='active' AND a.status='active'",&[&campaign,&person]).await.map_err(failed)?.is_none(){return Err(ShareFailure::NotFound);}
        }
        let nonce =
            df_campaign::secrets::random_identity().map_err(|_| ShareFailure::Unavailable)?;
        let bytes = serde_json::to_vec(&(&draft, &date, duration, nonce))
            .map_err(|_| ShareFailure::Invalid)?;
        let revision = hex(&Sha256::digest(bytes));
        let at = now(&tx).await?;
        tx.execute("INSERT INTO \
                    campaigns.share_preview(session_id,campaign_id,creator_account_id,revision,public_title,highlights,public_date,duration_minutes,updated_at_ms) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT(session_id) DO UPDATE SET creator_account_id=EXCLUDED.creator_account_id,revision=EXCLUDED.revision,public_title=EXCLUDED.public_title,highlights=EXCLUDED.highlights,public_date=EXCLUDED.public_date,duration_minutes=EXCLUDED.duration_minutes,updated_at_ms=EXCLUDED.updated_at_ms",&[&session,&campaign,&account.as_bytes().as_slice(),&revision,&draft.title,&draft.highlights,&date,&duration,&at]).await.map_err(failed)?;
        tx.execute("UPDATE campaigns.share_link SET revoked_at_ms=COALESCE(revoked_at_ms,$2) WHERE session_id=$1",&[&session,&at]).await.map_err(failed)?;
        tx.execute(
            "DELETE FROM campaigns.share_consent WHERE session_id=$1",
            &[&session],
        )
        .await
        .map_err(failed)?;
        for person in &draft.people {
            tx.execute("INSERT INTO campaigns.share_consent(session_id,account_id,public_name,revision) VALUES($1,$2,$3,$4)",&[&session,&id(&person.account_id)?,&person.public_name,&revision]).await.map_err(failed)?;
        }
        let result = preview(&tx, account, &session).await?;
        tx.commit().await.map_err(|_| ShareFailure::Unknown)?;
        audit("previewed", &session);
        Ok(result)
    }
    async fn consent(
        &self,
        account: AccountId,
        session: &str,
        consent: ShareConsent,
    ) -> Result<()> {
        let session = id(session)?;
        let mut c = self.0.lock().await;
        let tx = begin(&mut c).await?;
        let current = tx
            .query_opt(
                "SELECT revision FROM campaigns.share_preview WHERE session_id=$1 FOR UPDATE",
                &[&session],
            )
            .await
            .map_err(failed)?
            .ok_or(ShareFailure::NotFound)?;
        if current.try_get::<_, String>(0).map_err(failed)? != consent.revision {
            return Err(ShareFailure::Invalid);
        }
        let at = now(&tx).await?;
        let count = tx
            .execute(
                "UPDATE campaigns.share_consent p SET consented_at_ms=$3 WHERE session_id=$1 AND \
                    account_id=$2 AND EXISTS(SELECT 1 FROM campaigns.share_preview v JOIN \
                    campaigns.campaign_member m USING(campaign_id) JOIN accounts.account a ON \
                    a.account_id=m.account_id WHERE v.session_id=p.session_id AND \
                    m.account_id=p.account_id AND m.status='active' AND a.status='active')",
                &[
                    &session,
                    &account.as_bytes().as_slice(),
                    &consent.agrees.then_some(at),
                ],
            )
            .await
            .map_err(failed)?;
        if count != 1 {
            return Err(ShareFailure::NotFound);
        }
        tx.execute("UPDATE campaigns.share_preview SET updated_at_ms=$2 WHERE session_id=$1",&[&session,&at]).await.map_err(failed)?;
        if !consent.agrees {
            tx.execute("UPDATE campaigns.share_link SET revoked_at_ms=COALESCE(revoked_at_ms,$2) WHERE session_id=$1",&[&session,&at]).await.map_err(failed)?;
        }
        tx.commit().await.map_err(|_| ShareFailure::Unknown)?;
        audit(
            if consent.agrees {
                "consented"
            } else {
                "withdrawn"
            },
            &session,
        );
        Ok(())
    }
    async fn create(
        &self,
        account: AccountId,
        session: &str,
        choices: CreateShare,
        digest: [u8; 32],
    ) -> Result<ShareLinkView> {
        if ![7, 30, 90].contains(&choices.expiry_days) {
            return Err(ShareFailure::Invalid);
        }
        let session = id(session)?;
        let mut c = self.0.lock().await;
        let tx = begin(&mut c).await?;
        host(&tx, account, &session).await?;
        let locked = tx
            .query_opt(
                "SELECT revision FROM campaigns.share_preview WHERE session_id=$1 FOR UPDATE",
                &[&session],
            )
            .await
            .map_err(failed)?
            .ok_or(ShareFailure::NotFound)?;
        if locked.try_get::<_, String>(0).map_err(failed)? != choices.revision {
            return Err(ShareFailure::Invalid);
        }
        let missing:bool=tx.query_one("SELECT EXISTS(SELECT 1 FROM campaigns.share_consent p JOIN campaigns.share_preview v \
                    USING(session_id) WHERE p.session_id=$1 AND (p.consented_at_ms IS NULL OR \
                    p.revision<>v.revision OR NOT EXISTS(SELECT 1 FROM campaigns.campaign_member m JOIN \
                    accounts.account a USING(account_id) WHERE m.campaign_id=v.campaign_id AND \
                    m.account_id=p.account_id AND m.status='active' AND a.status='active')))",&[&session]).await.map_err(failed)?.try_get(0).map_err(failed)?;
        if missing {
            return Err(ShareFailure::ConsentRequired);
        }
        let at = now(&tx).await?;
        let expires = at + i64::from(choices.expiry_days) * 86_400_000;
        let key = df_campaign::secrets::random_identity().map_err(|_| ShareFailure::Unavailable)?;
        tx.execute("INSERT INTO \
                    campaigns.share_link(link_id,token_digest,session_id,creator_account_id,revision,created_at_ms,expires_at_ms,allow_indexing,allow_embeds) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)",&[&key.as_slice(),&digest.as_slice(),&session,&account.as_bytes().as_slice(),&choices.revision,&at,&expires,&choices.allow_indexing,&choices.allow_embeds]).await.map_err(failed)?;
        tx.execute("UPDATE campaigns.share_preview SET updated_at_ms=$2 WHERE session_id=$1",&[&session,&at]).await.map_err(failed)?;
        tx.commit().await.map_err(|_| ShareFailure::Unknown)?;
        audit("created", &session);
        Ok(ShareLinkView {
            link_id: hex(&key),
            expires_at_ms: u64::try_from(expires).map_err(|_| ShareFailure::Unavailable)?,
            revoked: false,
            allow_indexing: choices.allow_indexing,
            allow_embeds: choices.allow_embeds,
        })
    }
    async fn list(&self, account: AccountId, session: &str) -> Result<Vec<ShareLinkView>> {
        let session = id(session)?;
        let mut c = self.0.lock().await;
        let tx = begin(&mut c).await?;
        host(&tx, account, &session).await?;
        let rows=tx.query("SELECT link_id,expires_at_ms,revoked_at_ms IS NOT NULL,allow_indexing,allow_embeds \
                    FROM campaigns.share_link WHERE session_id=$1 ORDER BY created_at_ms DESC,link_id \
                    LIMIT 100",&[&session]).await.map_err(failed)?;
        rows.iter().map(link).collect()
    }
    async fn revoke(&self, account: AccountId, session: &str, key: &str) -> Result<()> {
        let session = id(session)?;
        let key = id(key)?;
        let mut c = self.0.lock().await;
        let tx = begin(&mut c).await?;
        host(&tx, account, &session).await?;
        let at = now(&tx).await?;
        if tx
            .execute(
                "UPDATE campaigns.share_link SET revoked_at_ms=COALESCE(revoked_at_ms,$3) WHERE \
                    session_id=$1 AND link_id=$2",
                &[&session, &key, &at],
            )
            .await
            .map_err(failed)?
            != 1
        {
            return Err(ShareFailure::NotFound);
        }
        tx.commit().await.map_err(|_| ShareFailure::Unknown)?;
        audit("revoked", &session);
        Ok(())
    }
    async fn public(&self, digest: [u8; 32]) -> Result<PublicSummary> {
        let mut c = self.0.lock().await;
        let tx = begin(&mut c).await?;
        let row=tx.query_opt("SELECT \
                    l.link_id,p.session_id,p.public_title,p.highlights,p.public_date,p.duration_minutes,l.allow_indexing,l.allow_embeds FROM campaigns.share_link l JOIN campaigns.share_preview p USING(session_id,revision) JOIN campaigns.campaign_member host ON host.campaign_id=p.campaign_id AND host.account_id=l.creator_account_id JOIN accounts.account a ON a.account_id=host.account_id WHERE l.token_digest=$1 AND l.revoked_at_ms IS NULL AND l.expires_at_ms>(extract(epoch FROM clock_timestamp())*1000)::bigint AND host.role='host' AND host.status='active' AND a.status='active' AND NOT EXISTS(SELECT 1 FROM campaigns.share_consent p2 WHERE p2.session_id=p.session_id AND (p2.consented_at_ms IS NULL OR p2.revision<>p.revision OR NOT EXISTS(SELECT 1 FROM campaigns.campaign_member m JOIN accounts.account a2 USING(account_id) WHERE m.campaign_id=p.campaign_id AND m.account_id=p2.account_id AND m.status='active' AND a2.status='active'))) FOR SHARE OF p",&[&digest.as_slice()]).await.map_err(failed)?.ok_or(ShareFailure::NotFound)?;
        let session: Vec<u8> = row.try_get(1).map_err(failed)?;
        let names=tx.query("SELECT public_name FROM campaigns.share_consent WHERE session_id=$1 ORDER BY account_id",&[&session]).await.map_err(failed)?.iter().map(|r|r.try_get(0).map_err(failed)).collect::<Result<Vec<String>>>()?;
        let key: Vec<u8> = row.try_get(0).map_err(failed)?;
        tx.execute("UPDATE campaigns.share_link SET views=LEAST(views,9223372036854775806)+1 WHERE link_id=$1",&[&key]).await.map_err(failed)?;
        let summary = PublicSummary {
            title: row.try_get(2).map_err(failed)?,
            highlights: row.try_get(3).map_err(failed)?,
            date: row.try_get(4).map_err(failed)?,
            duration_minutes: u64::try_from(row.try_get::<_, i64>(5).map_err(failed)?)
                .map_err(|_| ShareFailure::Unavailable)?,
            names,
            allow_indexing: row.try_get(6).map_err(failed)?,
            allow_embeds: row.try_get(7).map_err(failed)?,
        };
        tx.commit().await.map_err(|_| ShareFailure::Unknown)?;
        Ok(summary)
    }
}
