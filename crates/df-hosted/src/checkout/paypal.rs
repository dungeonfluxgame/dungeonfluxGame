//! Durable PayPal receipts bind server offers, orders and captures to one finance purchase.

use std::sync::{Arc, Mutex};

use df_account_app::wire::{CheckoutCreated, CheckoutStatus, CheckoutView};
use df_portal::app_api::ports::{PortError, PortResult};
use df_types::{AccountId, CheckoutId, OperationId};

use super::{
    paypal_provider::{Capture, Order, Provider, dollars, valid_id},
    provider::Error,
    stripe::{CreationState, claim_creation},
};
use crate::{database::Connection, finance::Finance};

pub(super) struct Paypal {
    pub(super) connection: tokio::sync::Mutex<Connection>,
    pub(super) provider: Provider,
    pub(super) creations: Arc<Mutex<CreationState>>,
}

impl Paypal {
    pub(super) async fn create(
        &self,
        account: AccountId,
        cents: u32,
        request: OperationId,
    ) -> PortResult<CheckoutCreated> {
        self.create_receipt(account, cents, request)
            .await
            .map_err(|error| {
                error.audit_for("paypal");
                match error {
                    Error::Mismatch => PortError::Forbidden,
                    Error::InProgress => PortError::Busy,
                    Error::AtCapacity => PortError::Throttled,
                    _ => PortError::UnknownOutcome,
                }
            })
    }

    async fn create_receipt(
        &self,
        account: AccountId,
        cents: u32,
        request: OperationId,
    ) -> Result<CheckoutCreated, Error> {
        let token = df_customer::SecretToken::generate().map_err(|_| Error::Configuration)?;
        let bytes = df_commerce::finance_ids::unhex(&token.text()).ok_or(Error::Configuration)?;
        let id = bytes.get(..16).ok_or(Error::Configuration)?;
        let amount_cents = i32::try_from(cents).map_err(|_| Error::Mismatch)?;
        let (operation, redirect_url, within_window) = {
            let connection = self.connection.lock().await;
            let row = connection.client.query_one(
                "INSERT INTO hosted.paypal_checkout(operation_id,account_id,request_id,amount_cents,currency,is_test_mode) \
                 VALUES($1,$2,$3,$4,'USD',$5) ON CONFLICT(account_id,request_id) DO UPDATE SET request_id=excluded.request_id \
                 RETURNING operation_id,amount_cents,is_test_mode,redirect_url,created_at > clock_timestamp()-interval '5 hours'",
                &[&id, &&account.as_bytes()[..], &&request.as_bytes()[..], &amount_cents, &self.provider.settings.is_test_mode]
            ).await.map_err(Error::Database)?;
            if row.try_get::<_, i32>(1).map_err(Error::Database)? != amount_cents
                || row.try_get::<_, bool>(2).map_err(Error::Database)?
                    != self.provider.settings.is_test_mode
            {
                return Err(Error::Mismatch);
            }
            let operation: Vec<u8> = row.try_get(0).map_err(Error::Database)?;
            let operation: [u8; 16] = operation.try_into().map_err(|_| Error::Mismatch)?;
            (
                operation,
                row.try_get::<_, Option<String>>(3)
                    .map_err(Error::Database)?,
                row.try_get::<_, bool>(4).map_err(Error::Database)?,
            )
        };
        let operation_hex = df_commerce::finance_ids::hex(&operation);
        if let Some(redirect_url) = redirect_url {
            return Ok(CheckoutCreated {
                checkout_id: operation_hex,
                redirect_url,
            });
        }
        // PayPal's default request-id retention is six hours; an older unknown send needs reconciliation.
        if !within_window {
            return Err(Error::Mismatch);
        }
        let _claim = claim_creation(&self.creations, *account.as_bytes(), operation)?;
        let (order, url) = self.provider.create(&operation_hex, cents).await?;
        let connection = self.connection.lock().await;
        let row = connection.client.query_one(
            "UPDATE hosted.paypal_checkout SET paypal_order_id=coalesce(paypal_order_id,$2),redirect_url=coalesce(redirect_url,$3) \
             WHERE operation_id=$1 AND (paypal_order_id IS NULL OR paypal_order_id=$2) RETURNING redirect_url",
            &[&&operation[..], &order, &url]
        ).await.map_err(Error::Database)?;
        let redirect_url = row.try_get(0).map_err(Error::Database)?;
        df_observe::record_audit_line(
            "hosted.paypal_created",
            format!(
                "SPEND paypal_created operation={operation_hex} order={order} amount_cents={cents}"
            ),
        );
        Ok(CheckoutCreated {
            checkout_id: operation_hex,
            redirect_url,
        })
    }

    pub(super) async fn status(
        &self,
        account: AccountId,
        operation: CheckoutId,
    ) -> PortResult<CheckoutView> {
        let connection = self.connection.lock().await;
        let row = connection.client.query_opt("SELECT amount_cents,paid,cancelled,capture_marker FROM hosted.paypal_checkout WHERE account_id=$1 AND operation_id=$2", &[&&account.as_bytes()[..], &&operation.as_bytes()[..]]).await.map_err(|_| PortError::Unavailable)?.ok_or(PortError::NotFound)?;
        let paid: bool = row.try_get(1).map_err(|_| PortError::Unavailable)?;
        let cancelled: bool = row.try_get(2).map_err(|_| PortError::Unavailable)?;
        let marker: Option<String> = row.try_get(3).map_err(|_| PortError::Unavailable)?;
        Ok(CheckoutView {
            checkout_id: df_commerce::finance_ids::hex(operation.as_bytes()),
            amount_cents: u32::try_from(
                row.try_get::<_, i32>(0)
                    .map_err(|_| PortError::Unavailable)?,
            )
            .map_err(|_| PortError::Unavailable)?,
            status: if paid {
                CheckoutStatus::Paid
            } else if marker.is_some() {
                CheckoutStatus::Failed
            } else if cancelled {
                CheckoutStatus::Cancelled
            } else {
                CheckoutStatus::Pending
            },
        })
    }

    pub(super) async fn complete(
        &self,
        operation: CheckoutId,
        order_id: &str,
        finance: &Finance,
    ) -> Result<(), Error> {
        let (account, should_inspect) = {
            let connection = self.connection.lock().await;
            let row = connection.client.query_opt(
                "UPDATE hosted.paypal_checkout SET capture_started_at=coalesce(capture_started_at,clock_timestamp()) \
                 WHERE operation_id=$1 AND paypal_order_id=$2 AND is_test_mode=$3 \
                 RETURNING account_id,paid,cancelled,capture_marker,capture_started_at < clock_timestamp()-interval '5 hours'",
                &[&&operation.as_bytes()[..], &order_id, &self.provider.settings.is_test_mode]
            ).await.map_err(Error::Database)?.ok_or(Error::Mismatch)?;
            if row.try_get::<_, bool>(1).map_err(Error::Database)? {
                return Ok(());
            }
            if row.try_get::<_, bool>(2).map_err(Error::Database)?
                || row
                    .try_get::<_, Option<String>>(3)
                    .map_err(Error::Database)?
                    .is_some()
            {
                return Err(Error::Mismatch);
            }
            let account: Vec<u8> = row.try_get(0).map_err(Error::Database)?;
            let account: [u8; 16] = account.try_into().map_err(|_| Error::Mismatch)?;
            (account, row.try_get::<_, bool>(4).map_err(Error::Database)?)
        };
        let _claim = claim_creation(&self.creations, account, *operation.as_bytes())?;
        let operation_hex = df_commerce::finance_ids::hex(operation.as_bytes());
        if !should_inspect {
            let captured = self.provider.capture(order_id, &operation_hex).await?;
            if captured.id != order_id || captured.status != "COMPLETED" {
                return Err(Error::Mismatch);
            }
        }
        // Inspect also supplies custom_id when the capture response omits purchase-unit details.
        let order = self.provider.inspect(order_id).await?;
        let capture = verified_capture(&order, order_id, &operation_hex)?;
        self.credit(operation, order_id, capture, None, finance)
            .await
    }

    pub(super) async fn cancel(&self, operation: CheckoutId) -> Result<(), Error> {
        let connection = self.connection.lock().await;
        if connection.client.execute("UPDATE hosted.paypal_checkout SET cancelled=true WHERE operation_id=$1 AND NOT paid", &[&&operation.as_bytes()[..]]).await.map_err(Error::Database)? == 0 {
            // A paid receipt stays paid; an unknown operation is rejected by the read below.
            if connection.client.query_opt("SELECT operation_id FROM hosted.paypal_checkout WHERE operation_id=$1", &[&&operation.as_bytes()[..]]).await.map_err(Error::Database)?.is_none() { return Err(Error::Mismatch); }
        }
        df_observe::record_audit_line(
            "hosted.paypal_cancelled",
            format!(
                "SPEND paypal_cancelled operation={}",
                df_commerce::finance_ids::hex(operation.as_bytes())
            ),
        );
        Ok(())
    }

    pub(super) async fn credit(
        &self,
        operation: CheckoutId,
        order_id: &str,
        capture: &Capture,
        event: Option<&str>,
        finance: &Finance,
    ) -> Result<(), Error> {
        if capture.status != "COMPLETED" || !valid_id(&capture.id) {
            return Err(Error::Mismatch);
        }
        let reference = df_commerce::finance_ids::hex(operation.as_bytes());
        if capture
            .custom_id
            .as_ref()
            .is_some_and(|id| id != &reference)
        {
            return Err(Error::Mismatch);
        }
        // Serializes receipt settlement; cancellation rolls back this acknowledgement, while Finance
        // repairs its separate durable commit via the canonical processor/operation business key.
        let mut connection = self.connection.lock().await;
        let transaction = connection
            .client
            .transaction()
            .await
            .map_err(Error::Database)?;
        let row = transaction.query_opt("SELECT account_id,amount_cents,currency,is_test_mode,paypal_order_id,capture_id,paid,capture_marker FROM hosted.paypal_checkout WHERE operation_id=$1 FOR UPDATE", &[&&operation.as_bytes()[..]]).await.map_err(Error::Database)?.ok_or(Error::Mismatch)?;
        let cents: i32 = row.try_get(1).map_err(Error::Database)?;
        let cents = u32::try_from(cents).map_err(|_| Error::Mismatch)?;
        let previous_capture: Option<String> = row.try_get(5).map_err(Error::Database)?;
        if capture.amount.value != dollars(cents)
            || capture.amount.currency_code
                != row.try_get::<_, String>(2).map_err(Error::Database)?
            || row.try_get::<_, bool>(3).map_err(Error::Database)?
                != self.provider.settings.is_test_mode
            || row
                .try_get::<_, Option<String>>(4)
                .map_err(Error::Database)?
                .as_deref()
                != Some(order_id)
            || previous_capture
                .as_deref()
                .is_some_and(|id| id != capture.id)
            || row
                .try_get::<_, Option<String>>(7)
                .map_err(Error::Database)?
                .is_some()
        {
            return Err(Error::Mismatch);
        }
        if let Some(event) = event {
            bind_event(&transaction, event, operation, &capture.id, "COMPLETED").await?;
        }
        let paid: bool = row.try_get(6).map_err(Error::Database)?;
        let updated_capture = transaction
            .execute(
                "UPDATE hosted.paypal_checkout SET capture_id=$2 WHERE operation_id=$1 \
                 AND NOT EXISTS(SELECT 1 FROM hosted.paypal_checkout WHERE capture_id=$2 AND operation_id<>$1)",
                &[&&operation.as_bytes()[..], &capture.id],
            )
            .await
            .map_err(Error::Database)?;
        if updated_capture != 1 {
            return Err(Error::Mismatch);
        }
        if !paid {
            let account: Vec<u8> = row.try_get(0).map_err(Error::Database)?;
            finance
                .paypal_purchase(
                    AccountId::from_bytes(&account).map_err(|_| Error::Mismatch)?,
                    cents,
                    &reference,
                )
                .await
                .map_err(Error::Finance)?;
        }
        transaction
            .execute(
                "UPDATE hosted.paypal_checkout SET paid=true WHERE operation_id=$1",
                &[&&operation.as_bytes()[..]],
            )
            .await
            .map_err(Error::Database)?;
        if let Some(event) = event {
            transaction
                .execute(
                    "UPDATE hosted.paypal_event SET credited=true WHERE event_id=$1",
                    &[&event],
                )
                .await
                .map_err(Error::Database)?;
        }
        transaction.commit().await.map_err(Error::Database)?;
        df_observe::record_audit_line(
            "hosted.paypal_credited",
            format!(
                "SPEND paypal_credited operation={reference} capture={} replay={paid}",
                capture.id
            ),
        );
        Ok(())
    }

    pub(super) async fn marker(
        &self,
        event: &str,
        capture: &Capture,
        kind: &str,
    ) -> Result<(), Error> {
        let order = capture
            .supplementary_data
            .as_ref()
            .ok_or(Error::Mismatch)?
            .related_ids
            .order_id
            .as_str();
        if !valid_id(&capture.id) || !valid_id(order) {
            return Err(Error::Mismatch);
        }
        let mut connection = self.connection.lock().await;
        let transaction = connection
            .client
            .transaction()
            .await
            .map_err(Error::Database)?;
        let row = transaction.query_opt("SELECT operation_id,capture_id FROM hosted.paypal_checkout WHERE paypal_order_id=$1 AND is_test_mode=$2 FOR UPDATE", &[&order, &self.provider.settings.is_test_mode]).await.map_err(Error::Database)?.ok_or(Error::Mismatch)?;
        let operation: Vec<u8> = row.try_get(0).map_err(Error::Database)?;
        let operation = CheckoutId::from_bytes(&operation).map_err(|_| Error::Mismatch)?;
        let previous: Option<String> = row.try_get(1).map_err(Error::Database)?;
        if previous.as_deref().is_some_and(|id| id != capture.id) {
            return Err(Error::Mismatch);
        }
        bind_event(&transaction, event, operation, &capture.id, kind).await?;
        transaction.execute("UPDATE hosted.paypal_checkout SET capture_marker=CASE WHEN capture_marker='REVERSED' THEN capture_marker ELSE $2 END WHERE operation_id=$1", &[&&operation.as_bytes()[..], &kind]).await.map_err(Error::Database)?;
        transaction.commit().await.map_err(Error::Database)?;
        df_observe::record_audit_line(
            "hosted.paypal_marked",
            format!(
                "SPEND paypal_marked operation={} status={kind}",
                df_commerce::finance_ids::hex(operation.as_bytes())
            ),
        );
        Ok(())
    }
}

pub(super) fn verified_capture<'a>(
    order: &'a Order,
    order_id: &str,
    operation: &str,
) -> Result<&'a Capture, Error> {
    if order.id != order_id || order.status != "COMPLETED" || order.purchase_units.len() != 1 {
        return Err(Error::Mismatch);
    }
    let unit = order.purchase_units.first().ok_or(Error::Mismatch)?;
    if unit.custom_id != operation {
        return Err(Error::Mismatch);
    }
    let captures = &unit.payments.as_ref().ok_or(Error::Mismatch)?.captures;
    if captures.len() != 1 {
        return Err(Error::Mismatch);
    }
    let capture = captures.first().ok_or(Error::Mismatch)?;
    if capture.status != "COMPLETED"
        || unit
            .amount
            .as_ref()
            .is_some_and(|amount| amount != &capture.amount)
    {
        return Err(Error::Mismatch);
    }
    Ok(capture)
}

async fn bind_event(
    transaction: &tokio_postgres::Transaction<'_>,
    event: &str,
    operation: CheckoutId,
    capture: &str,
    kind: &str,
) -> Result<(), Error> {
    let row = transaction.query_one("INSERT INTO hosted.paypal_event(event_id,operation_id,capture_id,event_kind) VALUES($1,$2,$3,$4) ON CONFLICT(event_id) DO UPDATE SET event_id=excluded.event_id RETURNING operation_id,capture_id,event_kind", &[&event, &&operation.as_bytes()[..], &capture, &kind]).await.map_err(Error::Database)?;
    if row.try_get::<_, Vec<u8>>(0).map_err(Error::Database)? != operation.as_bytes()
        || row.try_get::<_, String>(1).map_err(Error::Database)? != capture
        || row.try_get::<_, String>(2).map_err(Error::Database)? != kind
    {
        return Err(Error::Mismatch);
    }
    Ok(())
}
