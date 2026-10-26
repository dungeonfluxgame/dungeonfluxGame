//! Every route of the API with a body the route accepts, so a refusal in a test is the guard's
//! or the capability's and never a malformed request.

use serde_json::{Value, json};

const ID: &str = "11111111111111111111111111111111";
const OPERATION: &str = "22222222222222222222222222222222";

fn api(tail: &str) -> String {
    format!("/app/api/{tail}")
}

/// Customer reads that need a signed-in caller.
pub fn customer_reads() -> Vec<String> {
    [
        "overview",
        "credit/offers",
        &format!("credit/checkouts/{ID}"),
        "funding",
        "statements",
        "settings",
        "campaigns",
        "campaigns/new",
        &format!("campaigns/{ID}/settings"),
        &format!("sessions/{ID}/game"),
        &format!("campaigns/{ID}"),
        &format!("sessions/{ID}/lobby"),
    ]
    .iter()
    .map(|tail| api(tail))
    .collect()
}

/// Customer mutations that need a signed-in caller.
pub fn customer_posts() -> Vec<(String, Value)> {
    let operation = json!({ "operation": OPERATION });
    let draft = json!({"name": "x", "adventure_id": null, "content_rating": null, "seat_count": 4,
        "hero_choice": "in_lobby", "has_voice": true, "pictures": "automatic", "pace": "story",
        "custom_ceiling_cents_per_hour": null, "co_hosts_can_invite": false,
        "co_hosts_can_start": true, "co_hosts_can_export": false, "join_mode": "link_only",
        "funding": "sponsor"});
    vec![
        ("sign-out".into(), json!({})),
        ("verify-email/resend".into(), json!({})),
        (
            "credit/checkouts".into(),
            json!({"amount_cents": 1000, "operation": OPERATION}),
        ),
        (
            "funding/pace".into(),
            json!({"campaign_id": ID, "preset": "story", "custom_ceiling_cents_per_hour": null,
                "operation": OPERATION}),
        ),
        (
            "funding/allocate".into(),
            json!({"session_id": ID, "amount_cents": 500, "consents_to_host_spending": true,
                "operation": OPERATION}),
        ),
        (
            "settings/profile".into(),
            json!({"display_name": "x", "operation": OPERATION}),
        ),
        ("settings/sessions/revoke-others".into(), json!({})),
        (
            "settings/close".into(),
            json!({"password": "x", "operation": OPERATION}),
        ),
        ("campaigns/drafts".into(), json!({})),
        ("campaigns/play-now".into(), operation.clone()),
        (format!("campaigns/drafts/{ID}/finish"), operation.clone()),
        (
            format!("campaigns/{ID}/invitations"),
            json!({"kind": "player", "operation": OPERATION}),
        ),
        (format!("campaigns/{ID}/lobby"), operation.clone()),
        (format!("campaigns/{ID}/leave"), operation.clone()),
        (format!("campaigns/{ID}/code/rotate"), operation.clone()),
        (format!("sessions/{ID}/lobby/start"), operation.clone()),
        (
            "join/accept".into(),
            json!({"token": null, "operation": OPERATION}),
        ),
        ("join/code/lookup".into(), json!({"code": "ABCD-EFGH"})),
        (
            "join/code/request".into(),
            json!({"code": "ABCD-EFGH", "operation": OPERATION}),
        ),
        ("campaigns/drafts/x/steps/1".into(), json!({"draft": draft})),
    ]
    .into_iter()
    .map(|(tail, body)| (api(&tail), body))
    .collect()
}

pub fn admin_reads() -> Vec<String> {
    [
        "admin/summary",
        "admin/accounts",
        &format!("admin/accounts/{ID}"),
        "admin/disputes",
        "admin/reconciliation",
        "admin/providers",
        "admin/review",
        "admin/audit",
    ]
    .iter()
    .map(|tail| api(tail))
    .collect()
}

pub fn admin_posts() -> Vec<(String, Value)> {
    let reason = json!({"reason": "Checked with the customer", "operation": OPERATION});
    vec![
        (
            "admin/refunds".to_owned(),
            json!({"account_id": ID, "lot_id": "lot-1", "amount_cents": 100,
                "reason": "Customer asked", "operation": OPERATION}),
        ),
        (
            "admin/credits".to_owned(),
            json!({"account_id": ID, "amount_cents": 100, "reason": "Apology",
                "support_case": "CASE-1", "operation": OPERATION}),
        ),
        (
            "admin/disputes/dispute-1/evidence".to_owned(),
            reason.clone(),
        ),
        ("admin/providers/p1/pause".to_owned(), reason.clone()),
        ("admin/providers/p1/resume".to_owned(), reason),
        (
            "admin/review/item-1/decision".to_owned(),
            json!({"decision": "approve", "reason": "Looks fine", "operation": OPERATION}),
        ),
    ]
    .into_iter()
    .map(|(tail, body)| (api(&tail), body))
    .collect()
}
