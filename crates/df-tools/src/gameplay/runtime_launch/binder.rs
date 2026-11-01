//! Trusted same-database placement binding; no account or owner identity comes from a join.
use super::{AllocationBinder, BinderError, HostBindingCommand};
use df_finance::ports::FenceRequest;
use df_persistence::imagery_identity::{
    self as identity, HostRunBinding, HostSeatBinding, HostedRunGrant, IdentityError,
};
use std::{future::Future, pin::Pin};
use tokio::sync::Mutex;
use tokio_postgres::Client;

pub(in crate::gameplay) struct CanonicalAllocationBinder {
    client: Mutex<Client>,
    run: HostRunBinding,
    fence: FenceRequest,
    grant: HostedRunGrant,
}

impl CanonicalAllocationBinder {
    /// The startup owner retains the connection driver and supplies its actual captured run/fences.
    pub(in crate::gameplay) async fn new(
        client: Client,
        grant: HostedRunGrant,
        fence: FenceRequest,
    ) -> Result<Self, BinderError> {
        let run = grant.binding.clone();
        if run.tenant != *fence.tenant.as_bytes() || run.campaign != *fence.campaign.as_bytes() {
            return Err(BinderError::Occupied);
        }
        let binder = Self {
            client: Mutex::new(client),
            run,
            fence,
            grant,
        };
        let mut client = binder.client.lock().await;
        let tx = client
            .transaction()
            .await
            .map_err(|_| BinderError::Unavailable)?;
        identity::require_hosted_identity_schema(&tx)
            .await
            .map_err(identity_error)?;
        binder.verify_fence(&tx).await?;
        identity::bind_host_run(&tx, &binder.run, now_ms()?)
            .await
            .map_err(identity_error)?;
        identity::resolve_hosted_run(&tx, &binder.grant)
            .await
            .map_err(identity_error)?;
        tx.commit().await.map_err(|_| BinderError::Unavailable)?;
        drop(client);
        Ok(binder)
    }

    async fn verify_fence(&self, tx: &tokio_postgres::Transaction<'_>) -> Result<(), BinderError> {
        let f = &self.fence;
        let current: bool = tx
            .query_one(
                "SELECT df_runtime.fence_current($1::bytea,$2::bytea,$3,$4,$5::bytea,'game')",
                &[
                    &f.tenant.as_bytes().as_slice(),
                    &f.campaign.as_bytes().as_slice(),
                    &f.recovery_epoch,
                    &f.claim_seq,
                    &f.placement_id.as_slice(),
                ],
            )
            .await
            .map_err(|_| BinderError::Unavailable)?
            .get(0);
        if current {
            Ok(())
        } else {
            Err(BinderError::Occupied)
        }
    }
}

fn now_ms() -> Result<i64, BinderError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_millis()).ok())
        .ok_or(BinderError::Unavailable)
}
fn identity_error(error: IdentityError) -> BinderError {
    match error {
        IdentityError::Conflict | IdentityError::Refused => BinderError::Occupied,
        IdentityError::Unavailable { .. } | IdentityError::Unknown { .. } => {
            BinderError::Unavailable
        }
    }
}

impl AllocationBinder for CanonicalAllocationBinder {
    fn bind_host_allocation<'a>(
        &'a self,
        command: &'a HostBindingCommand,
    ) -> Pin<Box<dyn Future<Output = Result<(), BinderError>> + Send + 'a>> {
        Box::pin(async move {
            let run = &self.run;
            if command.allocation.campaign != run.campaign
                || command.allocation.session != run.hosted_session
                || command.gameplay_session != run.gameplay_session
                || command.gameplay_run != run.run
                || command.seats.len() != command.allocation.roster.len()
            {
                return Err(BinderError::Occupied);
            }
            let generation: [u8; 16] = super::super::journey::unhex(&command.generation)
                .map_err(|_| BinderError::Occupied)?
                .try_into()
                .map_err(|_| BinderError::Occupied)?;
            let now = now_ms()?;
            let mut client = self.client.lock().await;
            let tx = client
                .transaction()
                .await
                .map_err(|_| BinderError::Unavailable)?;
            self.verify_fence(&tx).await?;
            identity::resolve_hosted_run(&tx, &self.grant)
                .await
                .map_err(identity_error)?;
            identity::bind_host_run(&tx, run, now)
                .await
                .map_err(identity_error)?;
            // Lock the whole canonical roster, so a removed or additional seated member refuses.
            let roster = tx.query("SELECT member_id,account_id FROM campaigns.campaign_member WHERE campaign_id=$1::bytea AND status='active' AND holds_seat ORDER BY member_id FOR SHARE",
                &[&run.campaign.as_slice()]).await.map_err(|_| BinderError::Unavailable)?;
            if roster.len() != command.seats.len()
                || roster.iter().any(|row| {
                    let member: Vec<u8> = row.get(0);
                    let account: Vec<u8> = row.get(1);
                    !command.seats.iter().any(|seat| {
                        seat.member.as_slice() == member && seat.account.as_slice() == account
                    })
                })
            {
                return Err(BinderError::Occupied);
            }
            for seat in &command.seats {
                if !command.allocation.roster.iter().any(|candidate| {
                    candidate.member == seat.member && candidate.account == seat.account
                }) {
                    return Err(BinderError::Occupied);
                }
                identity::verify_local_credential(
                    &tx,
                    &seat.gameplay_member,
                    &run.gameplay_session,
                    &seat.gameplay_credential,
                )
                .await
                .map_err(identity_error)?;
                identity::bind_host_seat(
                    &tx,
                    &HostSeatBinding {
                        tenant: run.tenant,
                        campaign: run.campaign,
                        hosted_session: run.hosted_session,
                        account: seat.account,
                        member: seat.member,
                        gameplay_member: seat.gameplay_member,
                        run: run.run,
                        gameplay_session: run.gameplay_session,
                        generation,
                        active: true,
                    },
                    now,
                )
                .await
                .map_err(identity_error)?;
            }
            // A failed acknowledgement retains the router slot. Exact retry verifies identical rows.
            tx.commit().await.map_err(|_| BinderError::Unavailable)
        })
    }
}
