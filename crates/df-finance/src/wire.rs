//! The JSON shapes between the pure plans and the database boundary
//! (planning/finance-ledger-research.md 17.6): a [`FinancePlan`] becomes the event document
//! `finance.commit_event` takes, and the snapshot `finance.lock_flow` returns becomes the pure
//! snapshot types. Ids in entry documents are plain hex; bytea columns inside row images are
//! written `\x` plus hex because `jsonb_populate_record` reads that form.

use std::collections::BTreeMap;

use df_commerce::admission::AdmissionSnapshot;
use df_commerce::finance_ids::{
    AllocationId, AttemptId, CampaignKey, LotId, PayerId, PriceVersionId, QuoteId, ReservationId,
    SessionKey, TenantKey, UsageEventId, hex, unhex,
};
use df_commerce::funding::{LotClass, LotState};
use df_commerce::funding_flows::{
    AllocateSnapshot, ExistingAllocation, OtherAllocation, PayerLot, TrialSnapshot, WithdrawLot,
    WithdrawalSnapshot,
};
use df_commerce::ledger::{Actor, CurrencyTag, JournalEntry};
use df_commerce::plan::{FinancePlan, Row, RowChange, Value as RowValue};
use df_commerce::price_schedule::{PriceVersion, TariffLine};
use df_commerce::returns::AllocationStatus;
use df_commerce::settlement::{CreditShare, CreditSnapshot, HeldSnapshot};
use df_commerce::snapshot::{
    AllocationLotState, AllocationRow, AllocationState, AttemptRow, DispatchState, LotRow,
    QuoteRow, QuoteStatus, ReceiptRow, ReservationRow, SessionState, ShareRow, UsageRow,
};
use serde_json::{Value, json};

/// A snapshot or result document did not have the shape the pure types need.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WireError(pub String);

impl std::fmt::Display for WireError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "wire: {}", self.0)
    }
}

impl std::error::Error for WireError {}

type Wire<T> = Result<T, WireError>;

fn row_value(value: &RowValue) -> Value {
    match value {
        RowValue::Null => Value::Null,
        RowValue::Bytes(bytes) => Value::String(format!("\\x{}", hex(bytes))),
        RowValue::Text(text) => Value::String(text.clone()),
        RowValue::Int(number) => json!(number),
        RowValue::Bool(flag) => json!(flag),
        RowValue::Json(text) => serde_json::from_str(text).unwrap_or(Value::Null),
    }
}

fn row_json(row: &Row) -> Value {
    Value::Object(
        row.0
            .iter()
            .map(|(column, value)| ((*column).to_owned(), row_value(value)))
            .collect(),
    )
}

fn opt_hex(bytes: Option<&[u8; 16]>) -> Value {
    bytes.map_or(Value::Null, |bytes| Value::String(hex(bytes)))
}

fn entry_json(entry: &JournalEntry) -> Value {
    let (actor_kind, actor_id, approver_id, case_ref) = match &entry.actor {
        Actor::System => ("system", Value::Null, Value::Null, Value::Null),
        Actor::Operator {
            actor_id,
            case_ref,
            approver_id,
        } => (
            "operator",
            Value::String(hex(actor_id)),
            approver_id
                .as_ref()
                .map_or(Value::Null, |id| Value::String(hex(id))),
            Value::String(case_ref.clone()),
        ),
    };
    let (ref_kind, ref_value) = match &entry.external_ref {
        Some((kind, value)) => (kind.label(), Value::String(value.clone())),
        None => ("none", Value::Null),
    };
    let lines: Vec<Value> = entry
        .lines
        .iter()
        .map(|line| {
            let dims = &line.key.dims;
            json!({
                "code": line.key.code.number(),
                "currency": line.key.currency.as_str(),
                "payer": opt_hex(dims.payer.as_ref().map(PayerId::as_bytes)),
                "lot": opt_hex(dims.lot.as_ref().map(LotId::as_bytes)),
                "allocation": opt_hex(dims.allocation.as_ref().map(AllocationId::as_bytes)),
                "provider": dims.provider,
                "processor": dims.processor,
                "loss_class": dims.loss_class,
                "jurisdiction": dims.jurisdiction,
                "promo_source": dims.promo_source,
                "amount": line.amount_micro_dollars,
            })
        })
        .collect();
    json!({
        "entry_id": entry.entry_id.hex(),
        "kind": entry.kind.label(),
        "business_key": hex(&entry.business_key),
        "external_ref_kind": ref_kind,
        "external_ref": ref_value,
        "stream_kind": entry.stream.0.label(),
        "stream_id": hex(&entry.stream.1),
        "effective_at_ms": entry.effective_at_ms,
        "reason": entry.reason.label(),
        "actor_kind": actor_kind,
        "actor_id": actor_id,
        "approver_id": approver_id,
        "case_ref": case_ref,
        "campaign_id": opt_hex(entry.correlation.campaign.as_ref().map(CampaignKey::as_bytes)),
        "session_id": opt_hex(entry.correlation.session.as_ref().map(SessionKey::as_bytes)),
        "attempt_id": opt_hex(entry.correlation.attempt.as_ref().map(AttemptId::as_bytes)),
        "reservation_id": opt_hex(entry.correlation.reservation.as_ref().map(ReservationId::as_bytes)),
        "price_version_id": opt_hex(entry.correlation.price_version.as_ref().map(PriceVersionId::as_bytes)),
        "lines": lines,
    })
}

fn change_json(change: &RowChange) -> Value {
    let mut object = json!({"kind": change.kind.label(), "row": row_json(&change.row)});
    if let Some(revision) = change.expected_revision {
        object["expected_revision"] = json!(revision);
    }
    object
}

/// The event document of a plan.
pub fn event_json(plan: &FinancePlan) -> Value {
    json!({
        "event_id": plan.event_id.hex(),
        "event_kind": plan.event_kind.label(),
        "payload_schema": 1,
        "entries": plan.entries.iter().map(entry_json).collect::<Vec<_>>(),
        "changes": plan.changes.iter().map(change_json).collect::<Vec<_>>(),
    })
}

// ------------------------------------------------------------------------------------------------
// Reading documents.

fn field<'a>(value: &'a Value, key: &str) -> Wire<&'a Value> {
    value
        .get(key)
        .ok_or_else(|| WireError(format!("missing {key}")))
}

fn bytes_of(value: &Value) -> Wire<Vec<u8>> {
    let text = value
        .as_str()
        .ok_or_else(|| WireError("bytes are not text".into()))?;
    unhex(text.trim_start_matches("\\x")).ok_or_else(|| WireError("bad hex".into()))
}

fn id_of<T>(value: &Value, make: fn(&[u8]) -> Option<T>) -> Wire<T> {
    make(&bytes_of(value)?).ok_or_else(|| WireError("bad id length".into()))
}

fn id_field<T>(value: &Value, key: &str, make: fn(&[u8]) -> Option<T>) -> Wire<T> {
    id_of(field(value, key)?, make)
}

fn opt_id_field<T>(value: &Value, key: &str, make: fn(&[u8]) -> Option<T>) -> Wire<Option<T>> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(inner) => Ok(Some(id_of(inner, make)?)),
    }
}

fn int_field(value: &Value, key: &str) -> Wire<i64> {
    field(value, key)?
        .as_i64()
        .ok_or_else(|| WireError(format!("{key} is not an integer")))
}

fn uint_field(value: &Value, key: &str) -> Wire<u64> {
    field(value, key)?
        .as_u64()
        .ok_or_else(|| WireError(format!("{key} is not a non-negative integer")))
}

fn opt_uint_field(value: &Value, key: &str) -> Wire<Option<u64>> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => uint_field(value, key).map(Some),
    }
}

fn text_field<'a>(value: &'a Value, key: &str) -> Wire<&'a str> {
    field(value, key)?
        .as_str()
        .ok_or_else(|| WireError(format!("{key} is not text")))
}

fn opt_text_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn currency(value: &Value, key: &str) -> Wire<CurrencyTag> {
    CurrencyTag::parse(text_field(value, key)?).ok_or_else(|| WireError("bad currency".into()))
}

fn lot_state(value: &Value) -> Wire<LotState> {
    LotState::from_label(text_field(value, "state")?)
        .ok_or_else(|| WireError("bad lot state".into()))
}

fn lot_class(value: &Value) -> Wire<LotClass> {
    LotClass::from_label(text_field(value, "class")?)
        .ok_or_else(|| WireError("bad lot class".into()))
}

pub fn parse_session(value: &Value) -> Wire<SessionState> {
    Ok(SessionState {
        session: id_field(value, "session_id", SessionKey::from_slice)?,
        tenant: id_field(value, "tenant_id", TenantKey::from_slice)?,
        campaign: id_field(value, "campaign_id", CampaignKey::from_slice)?,
        price_version: id_field(value, "price_version_id", PriceVersionId::from_slice)?,
        currency: currency(value, "currency")?,
        open: text_field(value, "status")? == "open",
        current_epoch: int_field(value, "current_epoch")?,
        charged_total_micro_dollars: uint_field(value, "charged_total_micro_dollars")?,
        hourly_ceiling_micro_dollars: uint_field(value, "hourly_ceiling_micro_dollars")?,
        session_ceiling_micro_dollars: value
            .get("session_ceiling_micro_dollars")
            .and_then(Value::as_u64),
        per_request_threshold_micro_dollars: uint_field(
            value,
            "per_request_threshold_micro_dollars",
        )?,
        revision: int_field(value, "revision")?,
    })
}

pub fn parse_price_version(version: &Value, tariff: &Value) -> Wire<PriceVersion> {
    let lines = tariff
        .as_array()
        .ok_or_else(|| WireError("tariff is not a list".into()))?
        .iter()
        .map(|line| {
            Ok(TariffLine {
                provider: text_field(line, "provider")?.to_owned(),
                model: text_field(line, "model")?.to_owned(),
                category: text_field(line, "category")?.to_owned(),
                context_tier_max_tokens: uint_field(line, "context_tier_max_tokens")?,
                nano_usd: uint_field(line, "rate_nano_dollars")?,
                per: uint_field(line, "per_units")?,
            })
        })
        .collect::<Wire<Vec<_>>>()?;
    Ok(PriceVersion {
        id: id_field(version, "price_version_id", PriceVersionId::from_slice)?,
        label: text_field(version, "label")?.to_owned(),
        currency: currency(version, "currency")?,
        platform_rate_micro_dollars_per_hour: uint_field(
            version,
            "platform_rate_micro_dollars_per_hour",
        )?,
        markup_numerator: uint_field(version, "markup_numerator")?,
        markup_denominator: uint_field(version, "markup_denominator")?,
        tariff: lines,
        proposed_pilot: version
            .get("proposed_pilot")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

pub fn parse_quote_row(value: &Value) -> Wire<Option<QuoteRow>> {
    if value.is_null() {
        return Ok(None);
    }
    let digest: [u8; 32] = bytes_of(field(value, "request_digest")?)?
        .try_into()
        .map_err(|_| WireError("bad digest".into()))?;
    Ok(Some(QuoteRow {
        quote: id_field(value, "quote_id", QuoteId::from_slice)?,
        session: id_field(value, "session_id", SessionKey::from_slice)?,
        request_digest: digest,
        price_version: id_field(value, "price_version_id", PriceVersionId::from_slice)?,
        retail_maximum_micro_dollars: uint_field(value, "retail_maximum_micro_dollars")?,
        expires_at_ms: int_field(value, "expires_at_ms")?,
        status: QuoteStatus::from_label(text_field(value, "state")?)
            .ok_or_else(|| WireError("bad quote state".into()))?,
        revision: int_field(value, "revision")?,
    }))
}

/// The receipt row's outcome document back into a [`ReceiptRow`].
pub fn parse_receipt(value: &Value) -> Wire<Option<ReceiptRow>> {
    if value.is_null() {
        return Ok(None);
    }
    let fingerprint: [u8; 32] = bytes_of(field(value, "fingerprint")?)?
        .try_into()
        .map_err(|_| WireError("bad fingerprint".into()))?;
    let outcome = field(value, "outcome")?;
    let text_id = |key: &str| -> Wire<Vec<u8>> {
        unhex(text_field(outcome, key)?).ok_or_else(|| WireError("bad hex".into()))
    };
    Ok(Some(ReceiptRow {
        fingerprint,
        reservation: ReservationId::from_slice(&text_id("reservation")?)
            .ok_or_else(|| WireError("bad id".into()))?,
        attempt: AttemptId::from_slice(&text_id("attempt")?)
            .ok_or_else(|| WireError("bad id".into()))?,
        retail_maximum_micro_dollars: uint_field(outcome, "retail_maximum")?,
        supplier_maximum_micro_dollars: uint_field(outcome, "supplier_maximum")?,
        price_version: PriceVersionId::from_slice(&text_id("price_version")?)
            .ok_or_else(|| WireError("bad id".into()))?,
    }))
}

pub fn parse_admission_snapshot(value: &Value) -> Wire<AdmissionSnapshot> {
    let session = parse_session(field(value, "session")?)?;
    let price_version =
        parse_price_version(field(value, "price_version")?, field(value, "tariff")?)?;
    let weights: BTreeMap<AllocationId, u64> = field(value, "epoch_shares")?
        .as_array()
        .ok_or_else(|| WireError("epoch shares".into()))?
        .iter()
        .map(|share| {
            Ok((
                id_field(share, "allocation_id", AllocationId::from_slice)?,
                uint_field(share, "weight")?,
            ))
        })
        .collect::<Wire<_>>()?;
    let mut allocations = Vec::new();
    for entry in field(value, "allocations")?
        .as_array()
        .ok_or_else(|| WireError("allocations".into()))?
    {
        let row = field(entry, "allocation")?;
        let allocation = id_field(row, "allocation_id", AllocationId::from_slice)?;
        let lots = field(entry, "lots")?
            .as_array()
            .ok_or_else(|| WireError("lots".into()))?
            .iter()
            .map(|lot| {
                Ok(AllocationLotState {
                    lot: id_field(lot, "lot_id", LotId::from_slice)?,
                    class: lot_class(lot)?,
                    state: lot_state(lot)?,
                    created_at_ms: int_field(lot, "created_at_ms")?,
                    draw_rank: int_field(lot, "draw_rank")?,
                    allocated_micro_dollars: uint_field(lot, "allocated")?,
                })
            })
            .collect::<Wire<Vec<_>>>()?;
        let state = AllocationState {
            allocation,
            payer: id_field(row, "payer_account_id", PayerId::from_slice)?,
            status: AllocationStatus::from_label(text_field(row, "status")?)
                .ok_or_else(|| WireError("bad status".into()))?,
            authorized_micro_dollars: uint_field(row, "authorized_micro_dollars")?,
            drawn_micro_dollars: uint_field(row, "drawn_micro_dollars")?,
            returned_micro_dollars: uint_field(row, "returned_micro_dollars")?,
            charged_micro_dollars: uint_field(row, "charged_micro_dollars")?,
            revision: int_field(row, "revision")?,
            payer_active: entry.get("standing").and_then(Value::as_str) == Some("active"),
            approved_hourly_ceiling_micro_dollars: uint_field(entry, "approved_hourly_ceiling")?,
            approved_session_ceiling_micro_dollars: opt_uint_field(
                entry,
                "approved_session_ceiling",
            )?,
            charged_last_hour_micro_dollars: uint_field(entry, "charged_last_hour")?,
            open_holds_micro_dollars: uint_field(entry, "open_holds")?,
            lots,
        };
        let weight = weights.get(&allocation).copied().unwrap_or(0);
        allocations.push((state, weight));
    }
    Ok(AdmissionSnapshot {
        session,
        price_version,
        allocations,
        quote: parse_quote_row(value.get("quote").unwrap_or(&Value::Null))?,
        open_holds_micro_dollars: uint_field(value, "open_holds")?,
        charged_last_hour_micro_dollars: uint_field(value, "charged_last_hour")?,
        receipt: parse_receipt(value.get("receipt").unwrap_or(&Value::Null))?,
    })
}

fn parse_lot_row(value: &Value) -> Wire<LotRow> {
    Ok(LotRow {
        lot: id_field(value, "lot_id", LotId::from_slice)?,
        payer: id_field(value, "payer_account_id", PayerId::from_slice)?,
        class: lot_class(value)?,
        promo_source: opt_text_field(value, "promo_source"),
        processor: opt_text_field(value, "processor"),
        currency: currency(value, "currency")?,
        state: lot_state(value)?,
        original_micro_dollars: uint_field(value, "original_micro_dollars")?,
        consumed_micro_dollars: uint_field(value, "consumed_micro_dollars")?,
        extinguished_micro_dollars: uint_field(value, "extinguished_micro_dollars")?,
        created_at_ms: int_field(value, "created_at_ms")?,
        revision: int_field(value, "revision")?,
    })
}

fn parse_share(value: &Value) -> Wire<ShareRow> {
    Ok(ShareRow {
        reservation: id_field(value, "reservation_id", ReservationId::from_slice)?,
        allocation: id_field(value, "allocation_id", AllocationId::from_slice)?,
        lot: id_field(value, "lot_id", LotId::from_slice)?,
        class: lot_class(value)?,
        held_micro_dollars: uint_field(value, "held_micro_dollars")?,
        charged_micro_dollars: uint_field(value, "charged_micro_dollars")?,
        released_micro_dollars: uint_field(value, "released_micro_dollars")?,
        reversed_micro_dollars: uint_field(value, "reversed_micro_dollars")?,
        revision: int_field(value, "revision")?,
    })
}

pub fn parse_attempt_row(value: &Value) -> Wire<AttemptRow> {
    Ok(AttemptRow {
        attempt: id_field(value, "attempt_id", AttemptId::from_slice)?,
        reservation: id_field(value, "reservation_id", ReservationId::from_slice)?,
        provider: text_field(value, "provider")?.to_owned(),
        model: text_field(value, "model")?.to_owned(),
        supplier_exposure_micro_dollars: uint_field(value, "supplier_exposure_micro_dollars")?,
        dispatch: DispatchState::from_label(text_field(value, "dispatch_state")?)
            .ok_or_else(|| WireError("bad dispatch state".into()))?,
        customer_held: text_field(value, "customer_state")? == "held",
        supplier_open: text_field(value, "supplier_state")? == "exposure_open",
        media_asset: match value.get("media_asset_id") {
            None | Some(Value::Null) => None,
            Some(inner) => Some(
                bytes_of(inner)?
                    .try_into()
                    .map_err(|_| WireError("bad asset id".into()))?,
            ),
        },
        campaign: opt_id_field(value, "campaign_id", CampaignKey::from_slice)?,
        revision: int_field(value, "revision")?,
    })
}

fn parse_allocation_row(value: &Value) -> Wire<AllocationRow> {
    Ok(AllocationRow {
        allocation: id_field(value, "allocation_id", AllocationId::from_slice)?,
        payer: id_field(value, "payer_account_id", PayerId::from_slice)?,
        status: AllocationStatus::from_label(text_field(value, "status")?)
            .ok_or_else(|| WireError("bad status".into()))?,
        returned_micro_dollars: uint_field(value, "returned_micro_dollars")?,
        charged_micro_dollars: uint_field(value, "charged_micro_dollars")?,
        revision: int_field(value, "revision")?,
    })
}

fn list<T>(value: &Value, key: &str, parse: fn(&Value) -> Wire<T>) -> Wire<Vec<T>> {
    field(value, key)?
        .as_array()
        .ok_or_else(|| WireError(format!("{key} is not a list")))?
        .iter()
        .map(parse)
        .collect()
}

/// A settlement or release snapshot, or `None` when the flow reported it stale.
pub fn parse_held_snapshot(value: &Value) -> Wire<Option<HeldSnapshot>> {
    if value.get("stale").and_then(Value::as_bool) == Some(true) {
        return Ok(None);
    }
    let reservation = field(value, "reservation")?;
    Ok(Some(HeldSnapshot {
        session: parse_session(field(value, "session")?)?,
        reservation: ReservationRow {
            reservation: id_field(reservation, "reservation_id", ReservationId::from_slice)?,
            session: id_field(reservation, "session_id", SessionKey::from_slice)?,
            retail_maximum_micro_dollars: uint_field(reservation, "retail_maximum_micro_dollars")?,
            held: text_field(reservation, "state")? == "held",
            price_version: id_field(reservation, "price_version_id", PriceVersionId::from_slice)?,
            revision: int_field(reservation, "revision")?,
        },
        attempt: parse_attempt_row(field(value, "attempt")?)?,
        shares: list(value, "shares", parse_share)?,
        allocations: list(value, "allocations", parse_allocation_row)?,
        lots: list(value, "lots", parse_lot_row)?,
        price_version: parse_price_version(
            field(value, "price_version")?,
            field(value, "tariff")?,
        )?,
    }))
}

pub fn parse_credit_snapshot(value: &Value) -> Wire<CreditSnapshot> {
    let usage = field(value, "usage_event")?;
    let shares = field(value, "shares")?
        .as_array()
        .ok_or_else(|| WireError("shares".into()))?
        .iter()
        .map(|entry| {
            Ok(CreditShare {
                share: parse_share(field(entry, "share")?)?,
                payer: id_field(entry, "payer", PayerId::from_slice)?,
                allocation_status: AllocationStatus::from_label(text_field(
                    entry,
                    "allocation_status",
                )?)
                .ok_or_else(|| WireError("bad status".into()))?,
            })
        })
        .collect::<Wire<Vec<_>>>()?;
    let session = id_field(usage, "session_id", SessionKey::from_slice)?;
    Ok(CreditSnapshot {
        session: parse_session(field(value, "session")?)?,
        usage: UsageRow {
            usage_event: id_field(usage, "usage_event_id", UsageEventId::from_slice)?,
            attempt: id_field(usage, "attempt_id", AttemptId::from_slice)?,
            session,
            retail_micro_dollars: uint_field(usage, "retail_micro_dollars")?,
            reversed_micro_dollars: uint_field(usage, "reversed_micro_dollars")?,
            price_version: id_field(usage, "price_version_id", PriceVersionId::from_slice)?,
            revision: int_field(usage, "revision")?,
        },
        attempt: parse_attempt_row(field(value, "attempt")?)?,
        shares,
        lots: list(value, "lots", parse_lot_row)?,
    })
}

pub fn parse_trial_snapshot(value: &Value) -> TrialSnapshot {
    let present = |key: &str| value.get(key).is_some_and(|inner| !inner.is_null());
    TrialSnapshot {
        payer_exists: present("payer"),
        trial_by_account: present("trial_by_account"),
        trial_by_email: present("trial_by_email"),
    }
}

pub fn parse_allocate_snapshot(value: &Value) -> Wire<AllocateSnapshot> {
    let payer_row = field(value, "payer")?;
    let session = parse_session(field(value, "session")?)?;
    let existing = match value.get("allocation") {
        None | Some(Value::Null) => None,
        Some(row) => Some(ExistingAllocation {
            allocation: id_field(row, "allocation_id", AllocationId::from_slice)?,
            authorized_micro_dollars: uint_field(row, "authorized_micro_dollars")?,
            drawn_micro_dollars: uint_field(row, "drawn_micro_dollars")?,
            consent_revision: int_field(row, "consent_revision")?,
            revision: int_field(row, "revision")?,
            lots_registered: field(value, "allocation_lots")?
                .as_array()
                .ok_or_else(|| WireError("allocation lots".into()))?
                .iter()
                .map(|lot| id_of(lot, LotId::from_slice))
                .collect::<Wire<_>>()?,
        }),
    };
    let payer_lots = field(value, "lots")?
        .as_array()
        .ok_or_else(|| WireError("lots".into()))?
        .iter()
        .map(|entry| {
            let lot = field(entry, "lot")?;
            Ok(PayerLot {
                lot: id_field(lot, "lot_id", LotId::from_slice)?,
                class: lot_class(lot)?,
                state: lot_state(lot)?,
                created_at_ms: int_field(lot, "created_at_ms")?,
                available_micro_dollars: uint_field(entry, "available")?,
            })
        })
        .collect::<Wire<Vec<_>>>()?;
    let other_allocations = field(value, "session_allocations")?
        .as_array()
        .ok_or_else(|| WireError("session allocations".into()))?
        .iter()
        .map(|row| {
            Ok(OtherAllocation {
                allocation: id_field(row, "allocation_id", AllocationId::from_slice)?,
                authorized_micro_dollars: uint_field(row, "authorized_micro_dollars")?,
                active: text_field(row, "status")? == "active",
            })
        })
        .collect::<Wire<Vec<_>>>()?;
    Ok(AllocateSnapshot {
        payer_active: payer_row.get("standing").and_then(Value::as_str) == Some("active"),
        session,
        existing,
        payer_lots,
        other_allocations,
    })
}

pub fn parse_withdrawal_snapshot(value: &Value) -> Wire<WithdrawalSnapshot> {
    let row = field(value, "allocation")?;
    let allocation = AllocationState {
        allocation: id_field(row, "allocation_id", AllocationId::from_slice)?,
        payer: id_field(row, "payer_account_id", PayerId::from_slice)?,
        status: AllocationStatus::from_label(text_field(row, "status")?)
            .ok_or_else(|| WireError("bad status".into()))?,
        authorized_micro_dollars: uint_field(row, "authorized_micro_dollars")?,
        drawn_micro_dollars: uint_field(row, "drawn_micro_dollars")?,
        returned_micro_dollars: uint_field(row, "returned_micro_dollars")?,
        charged_micro_dollars: uint_field(row, "charged_micro_dollars")?,
        revision: int_field(row, "revision")?,
        payer_active: true,
        approved_hourly_ceiling_micro_dollars: 0,
        approved_session_ceiling_micro_dollars: None,
        charged_last_hour_micro_dollars: 0,
        open_holds_micro_dollars: 0,
        lots: Vec::new(),
    };
    let lots = field(value, "lots")?
        .as_array()
        .ok_or_else(|| WireError("lots".into()))?
        .iter()
        .map(|lot| {
            Ok(WithdrawLot {
                lot: id_field(lot, "lot_id", LotId::from_slice)?,
                class: lot_class(lot)?,
                allocated_micro_dollars: uint_field(lot, "allocated")?,
            })
        })
        .collect::<Wire<Vec<_>>>()?;
    Ok(WithdrawalSnapshot {
        session: parse_session(field(value, "session")?)?,
        allocation,
        lots,
        receipt_exists: value.get("receipt_exists").and_then(Value::as_bool) == Some(true),
    })
}
