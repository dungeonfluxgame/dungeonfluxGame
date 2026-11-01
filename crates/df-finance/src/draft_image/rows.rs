use df_commerce::draft_image::Maximums;
use df_commerce::finance_ids::{AttemptId, ReservationId, hex};
use df_types::AccountId;
use serde_json::{Value, json};
use tokio_postgres::Transaction;

use super::commands::Settlement;
use super::{AdmissionCommand, AdmissionRefusal, Error, Receipt, admission_fingerprint};

pub(super) fn admission_document(command: &AdmissionCommand) -> Value {
    json!({"consent":hex(&command.consent.0), "consent_revision":command.consent_revision,
        "quote":command.quote.hex(), "binding":command.binding,
        "fingerprint":hex(&admission_fingerprint(command)),
        "price_version":hex(&command.binding.scope.price_version),
        "operation":hex(&command.binding.operation), "attempt":hex(&command.binding.attempt),
        "effect":hex(&command.binding.effect), "asset":hex(&command.binding.asset.0)})
}

pub(super) async fn execute(
    tx: &Transaction<'_>,
    account: AccountId,
    document: &Value,
) -> Result<Value, Error> {
    tx.query_one(
        "SELECT finance.draft_image_command($1,$2::text::jsonb)::text",
        &[&account.as_bytes().as_slice(), &document.to_string()],
    )
    .await
    .map_err(boundary)?
    .try_get::<_, String>(0)
    .map_err(|_| Error::InvalidRecord)
    .and_then(|text| serde_json::from_str(&text).map_err(|_| Error::InvalidRecord))
}

pub(super) fn uint(document: &Value, name: &str) -> Result<u64, Error> {
    document
        .get(name)
        .and_then(Value::as_u64)
        .ok_or(Error::InvalidRecord)
}

pub(super) fn is_new(document: &Value) -> Result<bool, Error> {
    document
        .get("replay")
        .and_then(Value::as_bool)
        .map(|replay| !replay)
        .ok_or(Error::InvalidRecord)
}

pub(super) fn receipt(document: &Value, fingerprint: [u8; 32]) -> Result<Receipt, Error> {
    let id = |name| {
        document
            .get(name)
            .and_then(Value::as_str)
            .ok_or(Error::InvalidRecord)
    };
    Ok(Receipt {
        reservation: ReservationId::from_hex(id("reservation")?).ok_or(Error::InvalidRecord)?,
        attempt: AttemptId::from_hex(id("attempt")?).ok_or(Error::InvalidRecord)?,
        price_version: df_commerce::finance_ids::PriceVersionId::from_hex(id("price_version")?)
            .map(|id| *id.as_bytes())
            .ok_or(Error::InvalidRecord)?,
        maximums: Maximums {
            retail_micro_dollars: uint(document, "retail_maximum")?,
            supplier_micro_dollars: uint(document, "supplier_maximum")?,
        },
        admitted_at_ms: document
            .get("admitted_at_ms")
            .and_then(Value::as_i64)
            .ok_or(Error::InvalidRecord)?,
        fingerprint,
        replay: !is_new(document)?,
    })
}

pub(super) fn settlement(document: &Value) -> Result<Settlement, Error> {
    Ok(Settlement {
        charged_micro_dollars: uint(document, "charged")?,
        released_micro_dollars: uint(document, "released")?,
        replay: !is_new(document)?,
    })
}

pub(super) fn commit_unknown(error: tokio_postgres::Error) -> Error {
    Error::CommitUnknown {
        sqlstate: error.as_db_error().map(|db| db.code().code().to_owned()),
    }
}

fn boundary(error: tokio_postgres::Error) -> Error {
    let refusal = match error.as_db_error().map(|db| db.code().code()) {
        Some("FD001") => AdmissionRefusal::InvalidTransaction,
        Some("FD002") => AdmissionRefusal::AccountUnavailable,
        Some("FD003") => AdmissionRefusal::Conflict,
        Some("FD004") => AdmissionRefusal::MissingOrForeign,
        Some("FD005") => AdmissionRefusal::Revoked,
        Some("FD006") => AdmissionRefusal::ConsentExpired,
        Some("FD007") => AdmissionRefusal::BindingChanged,
        Some("FD008") => AdmissionRefusal::QuoteExpired,
        Some("FD009") => AdmissionRefusal::ConsentCapExceeded,
        Some("FD010") => AdmissionRefusal::InvalidUnits,
        Some("FD011") => AdmissionRefusal::Unpriced,
        Some("FD012") => AdmissionRefusal::TooManyFundingLots,
        Some("FD013") => AdmissionRefusal::InsufficientFunds,
        Some("FD014") => AdmissionRefusal::InvalidDispatch,
        Some("FD015") => AdmissionRefusal::PotentiallySent,
        Some("FD016") => AdmissionRefusal::RestrictedLot,
        Some("FD017") => AdmissionRefusal::SupplierBlocked,
        Some("FD018") => AdmissionRefusal::CurrencyMismatch,
        _ => return Error::Finance(error.into()),
    };
    Error::Refused(refusal)
}

/// Stage metadata is validated before delegating to the same account money command.
pub(super) async fn execute_stage(
    tx: &Transaction<'_>,
    account: AccountId,
    document: &Value,
) -> Result<Value, Error> {
    let row = tx
        .query_one(
            "SELECT finance.draft_stage_command($1,$2::text::jsonb)::text",
            &[&account.as_bytes().as_slice(), &document.to_string()],
        )
        .await
        .map_err(boundary)?;
    serde_json::from_str(
        &row.try_get::<_, String>(0)
            .map_err(|_| Error::InvalidRecord)?,
    )
    .map_err(|_| Error::InvalidRecord)
}
