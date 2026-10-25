#![cfg(not(target_arch = "wasm32"))]

//! What the journal copy of an event may hold (planning/finance-ledger-research.md 22.1): only the
//! columns each change kind lists, never a personal one. The golden list pins the allowlist, a lot
//! carrying a billing region proves the value never reaches a payload, and a mutant that adds the
//! column to the allowlist must fail both.

mod support;

use df_commerce::finance_ids::{EntryId, EventId, LotId};
use df_commerce::funding_flows::{PurchaseRequest, PurchaseSnapshot, plan_purchase};
use df_finance::wire::event_json;
use serde_json::json;
use support::attack::{now_ms, run_event};
use support::mutants::{self, Mutant};
use support::world::{CURRENCY, World, payer};

const GOLDEN: &str = include_str!("golden/journal_columns.txt");
const REGION: &str = "Qxz-Region-7";

async fn allowlist(world: &World) -> String {
    world
        .admin
        .query(
            "SELECT rtrim(kind || ' ' || array_to_string(journal_cols, ',')) FROM finance.change_kind ORDER BY kind",
            &[],
        )
        .await
        .expect("allowlist")
        .iter()
        .map(|row| format!("{}\n", row.get::<_, String>(0)))
        .collect()
}

/// A purchase whose lot carries a billing country and region, committed by hand.
async fn purchase_with_a_billing_region(world: &World) {
    let request = PurchaseRequest {
        event_id: EventId::from_array([0xb1; 16]),
        lot: LotId::from_array([0xb2; 16]),
        entry_id: EntryId::from_array([0xb3; 16]),
        payer: payer(1),
        processor: "fake".into(),
        payment_ref: "pay_region".into(),
        charge_ref: None,
        currency: CURRENCY,
        amount_micro_dollars: 5_000_000,
        now_ms: now_ms(),
    };
    run_event(
        world,
        Some((
            "purchase",
            json!({"payer": payer(1).hex(), "processor": "fake", "payment_ref": "pay_region"}),
        )),
        |_| {
            let plan = plan_purchase(
                PurchaseSnapshot {
                    payer_exists: false,
                    payment_state_exists: false,
                },
                &request,
            )
            .expect("an honest purchase plan");
            let mut event = event_json(&plan);
            let lot = event["changes"]
                .as_array_mut()
                .expect("changes")
                .iter_mut()
                .find(|change| change["kind"] == "funding_lot_insert")
                .expect("the lot");
            lot["row"]["billing_country"] = json!("ZZ");
            lot["row"]["billing_region"] = json!(REGION);
            event
        },
    )
    .await
    .expect("a purchase with a billing region");
}

/// How many journal payloads hold the region or a billing column name.
async fn leaks(world: &World) -> i64 {
    world
        .admin
        .query_one(
            "SELECT count(*) FROM finance.pending_event \
              WHERE payload::text LIKE '%' || $1 || '%' OR payload::text LIKE '%billing_%'",
            &[&REGION],
        )
        .await
        .expect("scan")
        .get(0)
}

#[test]
fn the_journal_payload_keeps_only_the_listed_columns_and_never_a_billing_region() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        if std::env::var("UPDATE_GOLDEN").is_ok() {
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/golden/journal_columns.txt"
            );
            std::fs::write(path, allowlist(&world).await).expect("write the golden");
        }
        assert_eq!(
            allowlist(&world).await,
            GOLDEN,
            "the allowlist is the golden list"
        );
        purchase_with_a_billing_region(&world).await;
        let stored: String = world
            .admin
            .query_one("SELECT billing_region FROM finance.funding_lot", &[])
            .await
            .expect("lot")
            .get(0);
        assert_eq!(stored, REGION, "the lot itself keeps the region");
        assert_eq!(leaks(&world).await, 0, "no payload holds it");
        world.finish().await;
    });
}

#[test]
fn a_list_that_adds_the_billing_region_fails_the_golden_list_and_the_payload_scan() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        mutants::apply(&world.admin, Mutant::JournalLeaksBillingRegion).await;
        assert_ne!(
            allowlist(&world).await,
            GOLDEN,
            "the golden list catches the added column"
        );
        purchase_with_a_billing_region(&world).await;
        assert!(
            leaks(&world).await > 0,
            "and the payload scan catches the value"
        );
        world.finish().await;
    });
}
