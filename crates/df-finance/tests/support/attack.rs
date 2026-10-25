//! Hand-driven use of the database boundary: take a flow's locks, build an event (honestly, forged,
//! or by hand), and commit it as `df_finance`. The attack tests use these to put events no honest
//! planner would produce in front of the boundary, and the mutant tests run the same attacks
//! against a database with one guard removed.

#![allow(
    dead_code,
    reason = "each test binary uses a different part of the attack kit"
)]

use df_commerce::digest::business_key;
use df_commerce::finance_ids::{EntryId, EventId, LotId, hex};
use df_commerce::funding::TrialPolicy;
use df_commerce::funding_flows::{TrialRequest, plan_trial};
use df_finance::wire::{event_json, parse_trial_snapshot};
use serde_json::{Value, json};
use tokio_postgres::Client;

use super::world::{CURRENCY, World, payer};

/// A connection acting as the service role.
pub async fn service_client(world: &World) -> Client {
    let client = world.scratch.connect().await;
    client
        .batch_execute("SET ROLE df_finance")
        .await
        .expect("set role");
    client
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).expect("json")
}

/// Takes the flow's locks (when `flow` names one), builds the event from the snapshot, commits it
/// in one READ COMMITTED transaction. Any database error is returned as it came.
pub async fn run_event(
    world: &World,
    flow: Option<(&str, Value)>,
    build: impl FnOnce(&Value) -> Value,
) -> Result<Value, tokio_postgres::Error> {
    let mut client = service_client(world).await;
    let transaction = client.transaction().await?;
    let snapshot = match flow {
        Some((name, args)) => {
            let row = transaction
                .query_one(
                    "SELECT finance.lock_flow($1, $2::text::jsonb)::text",
                    &[&name, &args.to_string()],
                )
                .await?;
            parse(&row.get::<_, String>(0))
        }
        None => Value::Null,
    };
    let event = build(&snapshot);
    let row = transaction
        .query_one(
            "SELECT finance.commit_event($1::text::jsonb)::text",
            &[&event.to_string()],
        )
        .await?;
    let result = parse(&row.get::<_, String>(0));
    transaction.commit().await?;
    Ok(result)
}

/// Like [`run_event`] without a flow, but the session first sets `settings` (as `SET LOCAL`), the
/// way a hostile caller would to try to talk a check out of running.
pub async fn run_event_with_settings(
    world: &World,
    settings: &[(String, String)],
    event: Value,
) -> Result<Value, tokio_postgres::Error> {
    let mut client = service_client(world).await;
    let transaction = client.transaction().await?;
    for (name, value) in settings {
        transaction
            .batch_execute(&format!("SET LOCAL \"{name}\" = '{value}'"))
            .await?;
    }
    let row = transaction
        .query_one(
            "SELECT finance.commit_event($1::text::jsonb)::text",
            &[&event.to_string()],
        )
        .await?;
    let result = parse(&row.get::<_, String>(0));
    transaction.commit().await?;
    Ok(result)
}

fn line(code: i64, amount: i64) -> Value {
    let dimension = match code {
        1100 | 5400 => json!({"processor": "fake"}),
        _ => json!({}),
    };
    let mut line = json!({
        "code": code, "currency": "USD", "payer": null, "lot": null, "allocation": null,
        "provider": null, "processor": null, "loss_class": null, "jurisdiction": null,
        "promo_source": null, "amount": amount,
    });
    if let Some(object) = dimension.as_object() {
        for (key, value) in object {
            line[key] = value.clone();
        }
    }
    line
}

/// A processing-fee event by hand: Dr 5400 `debit`, Cr 1100 `credit` (balanced when equal). It
/// touches no lot, allocation or row change, so only the journal's own guards stand in its way.
pub fn fee_event(tag: u8, debit: i64, credit: i64) -> Value {
    let id = |salt: u8| hex(&[tag, salt, 0xfe, 0xe0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    json!({
        "event_id": id(1),
        "event_kind": "processing_fee_posted",
        "payload_schema": 1,
        "changes": [
            {"kind": "key_create", "row": {"code": 5400, "currency": "USD", "processor": "fake"}},
            {"kind": "key_create", "row": {"code": 1100, "currency": "USD", "processor": "fake"}},
        ],
        "entries": [{
            "entry_id": id(2),
            "kind": "processing_fee",
            "business_key": hex(&business_key("test_fee", &[&[tag]])),
            "external_ref_kind": "processor_balance_txn",
            "external_ref": format!("txn_{tag}"),
            "stream_kind": "platform",
            "stream_id": hex(&[0x99; 16]),
            "effective_at_ms": null,
            "reason": "processor_fee",
            "actor_kind": "system",
            "actor_id": null,
            "approver_id": null,
            "case_ref": null,
            "campaign_id": null, "session_id": null, "attempt_id": null, "reservation_id": null,
            "price_version_id": null,
            "lines": [line(5400, debit), line(1100, -credit)],
        }],
    })
}

/// The honest trial plan for `tag`'s payer, altered by `mutate`, committed by hand.
pub async fn forged_trial(
    world: &World,
    tag: u8,
    mutate: impl FnOnce(&mut Value),
) -> Result<Value, tokio_postgres::Error> {
    let request = trial_request(tag);
    run_event(
        world,
        Some(("trial", json!({"payer": payer(tag).hex()}))),
        |snapshot| {
            let plan =
                plan_trial(parse_trial_snapshot(snapshot), request).expect("an honest trial plan");
            let mut event = event_json(&plan);
            mutate(&mut event);
            event
        },
    )
    .await
}

pub fn trial_request(tag: u8) -> TrialRequest {
    TrialRequest {
        event_id: EventId::from_array([0xe0 + tag; 16]),
        lot: LotId::from_array([0xe1 + tag; 16]),
        entry_id: EntryId::from_array([0xe2 + tag; 16]),
        payer: payer(tag),
        email_grant_key: [tag; 32],
        currency: CURRENCY,
        policy: TrialPolicy::default(),
        now_ms: now_ms(),
    }
}

/// The wall clock in milliseconds, the service clock of the tests.
pub fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_millis(),
    )
    .expect("ms")
}
