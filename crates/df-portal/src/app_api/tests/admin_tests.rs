use axum::http::StatusCode;
use df_account_app::wire::{
    OperatorCapability, ProviderHealth, Quantity, ReconciliationKind, ReviewFlag, StatementCategory,
};
use serde_json::{Value, json};

use super::fixtures::{Browser, Harness};
use crate::app_api::{
    ids::account_text,
    ports::{FinancePort, OperatorGrant},
};

struct Console<'h> {
    operator: Browser<'h>,
    customer_id: String,
    customer: df_types::AccountId,
}

/// An operator with every capability and a customer who bought $20 of credit.
async fn console(harness: &Harness) -> Console<'_> {
    let mut customer = harness.browser();
    customer.sign_up_verified("robin@example.test").await;
    let account = customer.account_id().await;
    customer.get("/app/api/overview").await;
    harness
        .ports
        .finance
        .credit_purchase(account, 2_000, "seed");
    let mut operator = harness.browser();
    operator.become_operator("ops@example.test").await;
    Console {
        operator,
        customer_id: account_text(account),
        customer: account,
    }
}

async fn lot_of(console: &mut Console<'_>) -> String {
    let detail = console
        .operator
        .get(&format!("/app/api/admin/accounts/{}", console.customer_id))
        .await
        .json();
    detail["refundable_lots"][0]["lot_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn refund(
    console: &mut Console<'_>,
    lot: &str,
    cents: u32,
    reason: &str,
    operation: &str,
) -> Value {
    console
        .operator
        .post(
            "/app/api/admin/refunds",
            &json!({"account_id": console.customer_id, "lot_id": lot, "amount_cents": cents,
                "reason": reason, "operation": operation}),
        )
        .await
        .json()
}

#[tokio::test]
async fn account_search_shows_masked_addresses_and_balances_to_operators() {
    let harness = Harness::new().await;
    let mut console = console(&harness).await;
    let rows = console
        .operator
        .get("/app/api/admin/accounts?q=robin")
        .await
        .json();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["email_masked"], json!("r***@e***.test"));
    assert_eq!(rows[0]["balance_micro_dollars"], json!(30_000_000));
    assert!(!rows.to_string().contains("robin@example.test"));
    let none = console
        .operator
        .get("/app/api/admin/accounts?q=nobody")
        .await
        .json();
    assert_eq!(none, json!([]));
}

#[tokio::test]
async fn the_operator_ledger_keeps_the_full_breakdown_a_customer_never_sees() {
    let harness = Harness::new().await;
    let mut console = console(&harness).await;
    harness.ports.finance.seed_charge(
        console.customer,
        StatementCategory::Narration,
        "The Lantern Road",
        Quantity::Seconds { seconds: 120 },
        300_000,
    );
    let detail = console
        .operator
        .get(&format!("/app/api/admin/accounts/{}", console.customer_id))
        .await
        .json();
    let usage = detail["ledger"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["kind"] == json!("usage"))
        .unwrap();
    assert_eq!(usage["multiplier_permille"], json!(1_500));
    assert!(usage["table_fee_micro_dollars"].as_u64().unwrap() > 0);
    assert!(usage["provider"].is_string());
}

#[tokio::test]
async fn an_unknown_or_malformed_account_id_is_not_found_to_an_operator() {
    let harness = Harness::new().await;
    let mut console = console(&harness).await;
    for path in [
        format!("/app/api/admin/accounts/{}", "ab".repeat(16)),
        "/app/api/admin/accounts/nope".to_owned(),
    ] {
        assert_eq!(
            console.operator.get(&path).await.code(),
            "not_found",
            "{path}"
        );
    }
}

#[tokio::test]
async fn a_refund_needs_the_refund_capability_even_for_an_operator() {
    let harness = Harness::new().await;
    let mut console = console(&harness).await;
    let lot = lot_of(&mut console).await;
    let id = console.operator.account_id().await;
    harness.ports.operators.set_grant(
        id,
        OperatorGrant {
            capabilities: vec![OperatorCapability::SupportInspect],
            has_second_factor: true,
        },
    );
    let operation = console.operator.operation().await;
    let reply = refund(&mut console, &lot, 100, "Customer asked", &operation).await;
    assert_eq!(reply["code"], json!("forbidden"));
    assert_eq!(
        harness
            .ports
            .finance
            .operator_balance(console.customer)
            .await
            .unwrap(),
        30_000_000
    );
}

#[tokio::test]
async fn an_operator_reason_must_be_between_one_and_five_hundred_characters() {
    let harness = Harness::new().await;
    let mut console = console(&harness).await;
    let lot = lot_of(&mut console).await;
    for (reason, refusal) in [
        (String::new(), "reason_required"),
        ("   ".to_owned(), "reason_required"),
        ("x".repeat(501), "reason_too_long"),
    ] {
        let operation = console.operator.operation().await;
        let reply = refund(&mut console, &lot, 100, &reason, &operation).await;
        assert_eq!(reply["refusal"], json!(refusal), "{}", reason.len());
    }
    let operation = console.operator.operation().await;
    let reply = refund(&mut console, &lot, 100, &"x".repeat(500), &operation).await;
    assert_eq!(reply["outcome"], json!("pending"));
}

#[tokio::test]
async fn a_refund_cannot_exceed_the_refundable_lot() {
    let harness = Harness::new().await;
    let mut console = console(&harness).await;
    let lot = lot_of(&mut console).await;
    let operation = console.operator.operation().await;
    let too_much = refund(&mut console, &lot, 2_500, "Customer asked", &operation).await;
    assert_eq!(too_much["refusal"], json!("amount_out_of_range"));
    let operation = console.operator.operation().await;
    assert_eq!(
        refund(&mut console, &lot, 1_500, "Customer asked", &operation).await["outcome"],
        json!("pending")
    );
    let operation = console.operator.operation().await;
    let rest = refund(&mut console, &lot, 600, "Customer asked", &operation).await;
    assert_eq!(rest["refusal"], json!("amount_out_of_range"));
    let balance = harness
        .ports
        .finance
        .operator_balance(console.customer)
        .await
        .unwrap();
    assert_eq!(balance, 30_000_000 - 15_000_000);
}

#[tokio::test]
async fn a_repeated_operation_id_does_not_repeat_a_refund_or_its_audit_entry() {
    let harness = Harness::new().await;
    let mut console = console(&harness).await;
    let lot = lot_of(&mut console).await;
    let operation = console.operator.operation().await;
    let first = refund(&mut console, &lot, 500, "Customer asked", &operation).await;
    let again = refund(&mut console, &lot, 500, "Customer asked", &operation).await;
    assert_eq!(first, again);
    assert_eq!(
        harness
            .ports
            .finance
            .operator_balance(console.customer)
            .await
            .unwrap(),
        30_000_000 - 5_000_000
    );
    let audit = console.operator.get("/app/api/admin/audit").await.json();
    let refunds: Vec<&Value> = audit
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["action"] == json!("refund"))
        .collect();
    assert_eq!(refunds.len(), 1, "{audit}");
    assert_eq!(refunds[0]["reason"], json!("Customer asked"));
}

#[tokio::test]
async fn a_goodwill_credit_needs_a_support_case_and_applies_once_per_operation() {
    let harness = Harness::new().await;
    let mut console = console(&harness).await;
    let operation = console.operator.operation().await;
    let body = |case: &str, operation: &str| {
        json!({"account_id": console.customer_id, "amount_cents": 250, "reason": "Apology",
            "support_case": case, "operation": operation})
    };
    let missing = console
        .operator
        .post("/app/api/admin/credits", &body("  ", &operation))
        .await;
    assert_eq!(missing.json()["refusal"], json!("support_case_required"));
    let first = console
        .operator
        .post("/app/api/admin/credits", &body("CASE-7", &operation))
        .await;
    assert_eq!(first.json()["outcome"], json!("done"));
    let again = console
        .operator
        .post("/app/api/admin/credits", &body("CASE-7", &operation))
        .await;
    assert_eq!(again.json(), first.json());
    assert_eq!(
        harness
            .ports
            .finance
            .operator_balance(console.customer)
            .await
            .unwrap(),
        30_000_000 + 2_500_000
    );
}

#[tokio::test]
async fn disputes_reconciliation_and_providers_are_listed_and_acted_on_with_an_audit_entry() {
    let harness = Harness::new().await;
    let mut console = console(&harness).await;
    let dispute = harness.ports.finance.seed_dispute(console.customer, 2_000);
    harness.ports.finance.seed_reconciliation_difference(
        ReconciliationKind::ProcessorCapture,
        "ch_1",
        5_000,
    );
    harness.ports.finance.seed_provider(
        "voice-a",
        "Aurora Speech",
        ProviderHealth::Degraded,
        80,
        1_000_000,
    );
    let summary = console.operator.get("/app/api/admin/summary").await.json();
    assert_eq!(summary["open_disputes"], json!(1));
    assert_eq!(summary["reconciliation_differences"], json!(1));
    assert_eq!(summary["providers_degraded"], json!(1));
    assert_eq!(
        console.operator.get("/app/api/admin/disputes").await.json()[0]["status"],
        json!("needs_response")
    );

    let operation = console.operator.operation().await;
    let evidence = console
        .operator
        .post(
            &format!("/app/api/admin/disputes/{dispute}/evidence"),
            &json!({"reason": "Receipt and usage attached", "operation": operation}),
        )
        .await;
    assert_eq!(evidence.json()["outcome"], json!("done"));
    assert_eq!(
        console.operator.get("/app/api/admin/disputes").await.json()[0]["status"],
        json!("under_review")
    );

    let operation = console.operator.operation().await;
    let paused = console
        .operator
        .post(
            "/app/api/admin/providers/voice-a/pause",
            &json!({"reason": "Error rate is high", "operation": operation}),
        )
        .await;
    assert_eq!(paused.status, StatusCode::OK);
    let providers = console
        .operator
        .get("/app/api/admin/providers")
        .await
        .json();
    assert_eq!(providers[0]["health"], json!("paused"));
    let operation = console.operator.operation().await;
    console
        .operator
        .post(
            "/app/api/admin/providers/voice-a/resume",
            &json!({"reason": "Recovered", "operation": operation}),
        )
        .await;
    assert_eq!(
        console
            .operator
            .get("/app/api/admin/providers")
            .await
            .json()[0]["health"],
        json!("healthy")
    );
    let operation = console.operator.operation().await;
    let missing = console
        .operator
        .post(
            "/app/api/admin/providers/nope/pause",
            &json!({"reason": "x", "operation": operation}),
        )
        .await;
    assert_eq!(missing.code(), "not_found");
    let audit = console.operator.get("/app/api/admin/audit").await.json();
    let actions: Vec<&str> = audit
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["action"].as_str().unwrap())
        .collect();
    assert_eq!(
        actions,
        ["provider_resumed", "provider_paused", "dispute_evidence"]
    );
    assert_eq!(audit[0]["operator"], json!("o***@e***.test"));
}

#[tokio::test]
async fn a_review_decision_removes_the_item_and_is_audited() {
    let harness = Harness::new().await;
    let mut console = console(&harness).await;
    harness.ports.review.seed_item(
        "portrait-1",
        "The Lantern Road",
        ReviewFlag::PossibleRealPerson,
    );
    let queue = console.operator.get("/app/api/admin/review").await.json();
    assert_eq!(queue[0]["item_id"], json!("portrait-1"));
    assert!(
        queue[0]["review_deadline_ms"].as_u64().unwrap()
            > queue[0]["submitted_at_ms"].as_u64().unwrap()
    );
    let operation = console.operator.operation().await;
    let decided = console
        .operator
        .post(
            "/app/api/admin/review/portrait-1/decision",
            &json!({"decision": "reject", "reason": "Looks like a real person", "operation": operation}),
        )
        .await;
    assert_eq!(decided.json()["outcome"], json!("done"));
    assert_eq!(
        console.operator.get("/app/api/admin/review").await.json(),
        json!([])
    );
    let audit = console.operator.get("/app/api/admin/audit").await.json();
    assert_eq!(audit[0]["action"], json!("review_rejected"));
}

#[tokio::test]
async fn account_search_filters_and_sorts_and_detail_has_every_section() {
    let harness = Harness::new().await;
    let mut console = console(&harness).await;
    // Filters and sorting.
    let verified = console
        .operator
        .get("/app/api/admin/accounts?q=robin&verified=yes&has_balance=yes&sort=balance")
        .await
        .json();
    assert_eq!(verified.as_array().unwrap().len(), 1);
    assert_eq!(
        verified[0]["lifetime_topups_micro_dollars"],
        json!(20_000_000)
    );
    let unverified = console
        .operator
        .get("/app/api/admin/accounts?q=robin&verified=no")
        .await
        .json();
    assert_eq!(unverified, json!([]));
    let bad = console
        .operator
        .get("/app/api/admin/accounts?q=robin&sort=nope")
        .await;
    assert_eq!(bad.code(), "invalid");
    // The full record: unmasked email, balances, funding, history totals, audit.
    let detail = console
        .operator
        .get(&format!("/app/api/admin/accounts/{}", console.customer_id))
        .await
        .json();
    assert_eq!(detail["email"], json!("robin@example.test"));
    assert_eq!(detail["account"]["email_masked"], json!("r***@e***.test"));
    assert!(detail["balance"].is_object());
    assert!(detail["statement_totals"].is_object());
    // Viewing the unmasked email wrote an audit entry naming the account.
    let audit = console.operator.get("/app/api/admin/audit").await.json();
    assert!(
        audit
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["action"] == json!("email_viewed")
                && row["target"] == json!(console.customer_id)),
        "{audit}"
    );
    // A customer is refused everywhere in the console.
    let mut customer = harness.browser();
    customer
        .sign_in("robin@example.test", super::fixtures::PASSWORD)
        .await;
    assert_eq!(
        customer
            .get(&format!("/app/api/admin/accounts/{}", console.customer_id))
            .await
            .code(),
        "forbidden"
    );
}

#[tokio::test]
async fn holding_an_asset_and_revoking_a_session_are_audited() {
    let harness = Harness::new().await;
    let mut console = console(&harness).await;
    harness.ports.assets.seed_asset(
        console.customer,
        df_account_app::wire::AssetRow {
            asset_id: "asset-9".to_owned(),
            kind: df_account_app::wire::AssetKind::Scene,
            campaign_id: None,
            campaign_name: None,
            created_at_ms: 1,
            size_bytes: 10,
            thumbnail_url: None,
            hold_state: df_account_app::wire::AssetHoldState::Ready,
        },
    );
    let operation = console.operator.operation().await;
    let held = console
        .operator
        .post(
            &format!(
                "/app/api/admin/accounts/{}/assets/asset-9/hold",
                console.customer_id
            ),
            &json!({"held": true, "reason": "Reported by a player", "operation": operation}),
        )
        .await;
    assert_eq!(held.json()["outcome"], json!("done"), "{}", held.body);
    let detail = console
        .operator
        .get(&format!("/app/api/admin/accounts/{}", console.customer_id))
        .await
        .json();
    assert_eq!(detail["assets"][0]["hold_state"], json!("held"));
    let operation = console.operator.operation().await;
    let revoked = console
        .operator
        .post(
            &format!(
                "/app/api/admin/accounts/{}/sessions/revoke",
                console.customer_id
            ),
            &json!({"session_id": "aa".repeat(16), "reason": "Account takeover report", "operation": operation}),
        )
        .await;
    assert_eq!(revoked.json()["outcome"], json!("done"));
    let audit = console.operator.get("/app/api/admin/audit").await.json();
    let actions: Vec<Value> = audit
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["action"].clone())
        .collect();
    assert!(actions.contains(&json!("asset_held")), "{actions:?}");
    assert!(
        actions.contains(&json!("sign_in_session_revoked")),
        "{actions:?}"
    );
    // A foreign asset is not found, not a different error.
    let operation = console.operator.operation().await;
    let missing = console
        .operator
        .post(
            &format!(
                "/app/api/admin/accounts/{}/assets/nope/hold",
                console.customer_id
            ),
            &json!({"held": true, "reason": "Trying", "operation": operation}),
        )
        .await;
    assert_eq!(missing.code(), "not_found");
}
