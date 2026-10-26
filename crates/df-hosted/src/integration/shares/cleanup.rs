//! One owned, cancellable minute pass; each transaction removes at most 100 links and 100 drafts.
use super::*;
use tokio::task::JoinHandle;
const CLEANUP_BATCH:u32=100;
pub(crate) struct Cleanup(Option<JoinHandle<()>>);
impl Drop for Cleanup{fn drop(&mut self){if let Some(task)=self.0.take(){task.abort();}}}
impl Cleanup{
    pub(crate) async fn shutdown(mut self){if let Some(task)=self.0.take(){task.abort();if let Err(error)=task.await{if !error.is_cancelled(){df_observe::record_audit_line("campaign.share_cleanup_stopped","CAMPAIGN share_cleanup status=worker_failed".to_owned());}}}}
}
impl Shares{
    pub(crate) fn start_cleanup(&self,retention_days:u16)->Cleanup{
        let shares=self.clone();
        let task=tokio::spawn(async move{
            let mut interval=tokio::time::interval(std::time::Duration::from_secs(60));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop{
                interval.tick().await;
                match tokio::time::timeout(std::time::Duration::from_secs(15),shares.cleanup(retention_days)).await{
                    Ok(Ok((links,previews)))=>df_observe::record_audit_line("campaign.share_cleanup",format!("CAMPAIGN share_cleanup status=ready links={links} previews={previews} retention_days={retention_days}")),
                    Ok(Err(_))=>{},
                    Err(_)=>df_observe::record_audit_line("campaign.share_cleanup_timeout","CAMPAIGN share_cleanup status=timeout".to_owned()),
                }
            }
        });
        Cleanup(Some(task))
    }
    pub(crate) async fn cleanup(&self,retention_days:u16)->Result<(u64,u64)>{
        let mut c=self.0.lock().await;let tx=begin(&mut c).await?;
        let cutoff=now(&tx).await?-i64::from(retention_days)*86_400_000;
        let links=tx.execute("DELETE FROM campaigns.share_link l USING \
            (SELECT link_id FROM campaigns.share_link \
             WHERE COALESCE(revoked_at_ms,expires_at_ms)<$1 \
             ORDER BY COALESCE(revoked_at_ms,expires_at_ms),link_id LIMIT $2 \
             FOR UPDATE SKIP LOCKED) old WHERE l.link_id=old.link_id",
             &[&cutoff,&i64::from(CLEANUP_BATCH)]).await.map_err(failed)?;
        let previews=tx.execute("DELETE FROM campaigns.share_preview p USING \
            (SELECT session_id FROM campaigns.share_preview p2 WHERE p2.updated_at_ms<$1 \
             AND NOT EXISTS(SELECT 1 FROM campaigns.share_link l WHERE l.session_id=p2.session_id) \
             ORDER BY p2.updated_at_ms,p2.session_id LIMIT $2 FOR UPDATE OF p2 SKIP LOCKED) old \
             WHERE p.session_id=old.session_id",&[&cutoff,&i64::from(CLEANUP_BATCH)]).await.map_err(failed)?;
        tx.commit().await.map_err(|_|ShareFailure::Unknown)?;Ok((links,previews))
    }
}
