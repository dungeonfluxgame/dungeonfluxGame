//! Durable checkout receipts bridge Stripe acknowledgements and idempotent finance posting.

use df_account_app::wire::{CheckoutCreated, CheckoutStatus, CheckoutView};
use df_portal::app_api::ports::{PortError, PortResult};
use df_types::{AccountId, CheckoutId, OperationId};

use super::{
    provider::{Error, Provider},
    webhook::PaidSession,
};
use crate::{database::Connection, finance::Finance};

pub(super) struct Stripe {
    pub(super) connection: tokio::sync::Mutex<Connection>,
    pub(super) provider: Provider,
}

impl Stripe {
    pub(super) async fn create(
        &self,
        account: AccountId,
        amount_cents: u32,
        request: OperationId,
    ) -> PortResult<CheckoutCreated> {
        self.create_receipt(account, amount_cents, request)
            .await
            .map_err(|error| {
                error.audit();
                match error {
                    Error::Mismatch => PortError::Forbidden,
                    _ => PortError::UnknownOutcome,
                }
            })
    }

    async fn create_receipt(
        &self,
        account: AccountId,
        amount_cents: u32,
        request: OperationId,
    ) -> Result<CheckoutCreated, Error> {
        let connection = self.connection.lock().await;
        let token = df_customer::SecretToken::generate().map_err(|_| Error::Configuration)?;
        let id = df_commerce::finance_ids::unhex(&token.text()).ok_or(Error::Configuration)?;
        let id = id.get(..16).ok_or(Error::Configuration)?;
        let cents = i32::try_from(amount_cents).map_err(|_| Error::Mismatch)?;
        let row = connection.client.query_one(
            "INSERT INTO hosted.stripe_checkout(operation_id,account_id,request_id,amount_cents,currency,is_test_mode) \
             VALUES($1,$2,$3,$4,'usd',$5) ON CONFLICT(account_id,request_id) DO UPDATE SET request_id=excluded.request_id \
             RETURNING operation_id,amount_cents,redirect_url,is_test_mode,created_at > clock_timestamp() - interval '23 hours'",
            &[&id, &&account.as_bytes()[..], &&request.as_bytes()[..], &cents, &self.provider.settings.is_test_mode],
        ).await.map_err(Error::Database)?;
        let original: i32 = row.try_get(1).map_err(Error::Database)?;
        let test_mode: bool = row.try_get(3).map_err(Error::Database)?;
        if original != cents || test_mode != self.provider.settings.is_test_mode {
            return Err(Error::Mismatch);
        }
        let operation: Vec<u8> = row.try_get(0).map_err(Error::Database)?;
        let operation = df_commerce::finance_ids::hex(&operation);
        if let Some(redirect_url) = row.try_get(2).map_err(Error::Database)? {
            return Ok(CheckoutCreated {
                checkout_id: operation,
                redirect_url,
            });
        }
        let within_idempotency_window: bool = row.try_get(4).map_err(Error::Database)?;
        // Stripe may prune keys after 24 hours. An uncertain older dispatch requires reconciliation.
        if !within_idempotency_window {
            return Err(Error::Mismatch);
        }
        let session = self.provider.create(&operation, amount_cents).await?;
        let updated = connection
            .client
            .execute(
                "UPDATE hosted.stripe_checkout SET stripe_session_id=$2,redirect_url=$3 \
             WHERE operation_id=$1 AND (stripe_session_id IS NULL OR stripe_session_id=$2)",
                &[
                    &df_commerce::finance_ids::unhex(&operation).ok_or(Error::Configuration)?,
                    &session.id,
                    &session.url,
                ],
            )
            .await
            .map_err(Error::Database)?;
        if updated != 1 {
            return Err(Error::Mismatch);
        }
        df_observe::record_audit_line(
            "hosted.stripe_created",
            format!("SPEND stripe_created operation={operation} amount_cents={amount_cents}"),
        );
        Ok(CheckoutCreated {
            checkout_id: operation,
            redirect_url: session.url,
        })
    }

    pub(super) async fn status(
        &self,
        account: AccountId,
        checkout: CheckoutId,
    ) -> PortResult<CheckoutView> {
        let connection = self.connection.lock().await;
        let row = connection.client.query_opt(
            "SELECT amount_cents,paid FROM hosted.stripe_checkout WHERE account_id=$1 AND operation_id=$2",
            &[&&account.as_bytes()[..], &&checkout.as_bytes()[..]],
        ).await.map_err(|_| PortError::Unavailable)?.ok_or(PortError::NotFound)?;
        let cents: i32 = row.try_get(0).map_err(|_| PortError::Unavailable)?;
        let paid: bool = row.try_get(1).map_err(|_| PortError::Unavailable)?;
        Ok(CheckoutView {
            checkout_id: df_commerce::finance_ids::hex(checkout.as_bytes()),
            amount_cents: u32::try_from(cents).map_err(|_| PortError::Unavailable)?,
            status: if paid {
                CheckoutStatus::Paid
            } else {
                CheckoutStatus::Pending
            },
        })
    }

    pub(super) async fn credit(
        &self,
        event: &str,
        session: PaidSession,
        finance: &Finance,
    ) -> Result<(), Error> {
        let operation =
            CheckoutId::from_hex(&session.metadata.operation_id).map_err(|_| Error::Mismatch)?;
        let reference = df_commerce::finance_ids::hex(operation.as_bytes());
        let mut connection = self.connection.lock().await;
        let transaction = connection
            .client
            .transaction()
            .await
            .map_err(Error::Database)?;
        let row = transaction
            .query_opt(
                "SELECT account_id,amount_cents,currency,is_test_mode,stripe_session_id,paid \
             FROM hosted.stripe_checkout WHERE operation_id=$1 FOR UPDATE",
                &[&&operation.as_bytes()[..]],
            )
            .await
            .map_err(Error::Database)?
            .ok_or(Error::Mismatch)?;
        let account: Vec<u8> = row.try_get(0).map_err(Error::Database)?;
        let cents: i32 = row.try_get(1).map_err(Error::Database)?;
        let currency: String = row.try_get(2).map_err(Error::Database)?;
        let is_test_mode: bool = row.try_get(3).map_err(Error::Database)?;
        let stripe_session: Option<String> = row.try_get(4).map_err(Error::Database)?;
        let paid: bool = row.try_get(5).map_err(Error::Database)?;
        if session.amount_total != i64::from(cents)
            || session.currency != currency
            || session.livemode == is_test_mode
            || session.mode != "payment"
            || stripe_session.as_deref() != Some(&session.id)
        {
            return Err(Error::Mismatch);
        }
        let event_row = transaction
            .query_one(
                "INSERT INTO hosted.stripe_event(event_id,operation_id) VALUES($1,$2) \
             ON CONFLICT(event_id) DO UPDATE SET event_id=excluded.event_id RETURNING operation_id",
                &[&event, &&operation.as_bytes()[..]],
            )
            .await
            .map_err(Error::Database)?;
        let event_operation: Vec<u8> = event_row.try_get(0).map_err(Error::Database)?;
        if event_operation != operation.as_bytes() {
            return Err(Error::Mismatch);
        }
        if !paid {
            let account = AccountId::from_bytes(&account).map_err(|_| Error::Mismatch)?;
            // Finance's processor/payment unique key repairs a crash between its commit and this commit.
            finance
                .stripe_purchase(
                    account,
                    u32::try_from(cents).map_err(|_| Error::Mismatch)?,
                    &reference,
                )
                .await
                .map_err(Error::Finance)?;
        }
        transaction
            .execute(
                "UPDATE hosted.stripe_checkout SET paid=true WHERE operation_id=$1",
                &[&&operation.as_bytes()[..]],
            )
            .await
            .map_err(Error::Database)?;
        transaction
            .execute(
                "UPDATE hosted.stripe_event SET credited=true WHERE event_id=$1",
                &[&event],
            )
            .await
            .map_err(Error::Database)?;
        transaction.commit().await.map_err(Error::Database)?;
        df_observe::record_audit_line(
            "hosted.stripe_credited",
            format!(
                "SPEND stripe_credited operation={} replay={paid}",
                reference
            ),
        );
        Ok(())
    }
}
