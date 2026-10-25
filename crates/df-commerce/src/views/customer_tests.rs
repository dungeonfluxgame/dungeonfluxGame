//! D26 (18.9): the customer views match their allowlist golden schema, show no price that belongs to
//! the supplier or the table, build `quantity_text` and the price version label only through their
//! closed producers, and import nothing from the operator module. Each check has a mutant that it
//! must catch.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, to_value};

use super::customer::*;
use crate::finance_ids::PriceVersionId;

const GOLDEN: &str = include_str!("customer_golden.txt");

fn money(micro_dollars: u64) -> MoneyView {
    MoneyView::new(micro_dollars)
}

const ID_A: &str = "0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a";
const ID_B: &str = "0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b";

/// One value of every view, with every closed vocabulary covered across the set. The amounts are
/// the table's retail amounts; the supplier side of the fixtures (markup 7/4, platform rate $2.37 an
/// hour, tariff digits 8123, supplier cost 7319, provider "zephyr-ai", model "zephyr-9-ultra") is
/// deliberately absent from every view.
fn fixtures() -> Vec<Value> {
    let label = price_version_label(PriceVersionId::from_array([9; 16]));
    let allocation = OwnAllocationView {
        authorized: money(12_500_000),
        drawn: money(4_000_000),
        authorization_remaining: money(8_500_000),
        consent: ConsentStatus::Current,
    };
    let mut views = vec![
        to_value(BalanceView {
            schema: BALANCE_SCHEMA,
            currency: "USD".into(),
            purchased_available: money(20_000_000),
            promotional_available: money(6_000_000),
            allocated_to_sessions: money(4_000_000),
            reserved_for_pending_work: money(1_312_500),
            consumed_this_period: money(2_250_000),
            proposed_pilot: true,
        }),
        to_value(BudgetChipView {
            schema: BUDGET_CHIP_SCHEMA,
            pot: money(9_000_000),
            held: money(1_312_500),
            hourly_maximum: money(5_000_000),
        }),
        to_value(PurchaseStatusView {
            schema: PURCHASE_STATUS_SCHEMA,
            state: PurchaseState::Pending,
            lot: None,
        }),
        to_value(PurchaseStatusView {
            schema: PURCHASE_STATUS_SCHEMA,
            state: PurchaseState::Succeeded,
            lot: Some(ID_A.into()),
        }),
        to_value(PurchaseStatusView {
            schema: PURCHASE_STATUS_SCHEMA,
            state: PurchaseState::Failed,
            lot: None,
        }),
        to_value(RefundPreviewView {
            schema: REFUND_PREVIEW_SCHEMA,
            lots: vec![RefundLotView {
                lot: ID_A.into(),
                amount: money(3_000_000),
            }],
            released_allocations: money(500_000),
            pending_holds: money(250_000),
            tax: money(180_000),
            card_suffix: Some("4242".into()),
            manual_payout: false,
        }),
        to_value(TransactionListView {
            schema: TRANSACTION_LIST_SCHEMA,
            rows: [
                TransactionKind::Purchase,
                TransactionKind::Allocation,
                TransactionKind::Release,
                TransactionKind::Usage,
                TransactionKind::Credit,
                TransactionKind::Refund,
                TransactionKind::Dispute,
                TransactionKind::Adjustment,
            ]
            .into_iter()
            .map(|kind| TransactionRowView {
                kind,
                at_ms: 1_700_000_000_000,
                amount: money(1_000_000),
                reference: ID_B.into(),
            })
            .collect(),
            next_cursor: Some("c2xvdA".into()),
        }),
    ];
    for (funds_state, end, consent, runway, own, ceiling) in [
        (
            FundsState::Funded,
            EndTreatment::ReturnToAvailable,
            ConsentStatus::Current,
            Some(RunwayView {
                at_selected_maximum_ms: 600_000,
                at_recent_rate_ms: 5_400_000,
            }),
            Some(allocation.clone()),
            Some(money(30_000_000)),
        ),
        (
            FundsState::Low,
            EndTreatment::CarryToNextSession,
            ConsentStatus::NeedsReconsent,
            None,
            None,
            None,
        ),
        (
            FundsState::Unfunded,
            EndTreatment::ReturnToAvailable,
            ConsentStatus::Withdrawn,
            None,
            None,
            None,
        ),
    ] {
        views.push(to_value(SessionFundingView {
            schema: SESSION_FUNDING_SCHEMA,
            price_version_label: label.clone(),
            price_effective_at_ms: 1_700_000_000_000,
            hourly_ceiling: money(5_000_000),
            session_ceiling: ceiling,
            pot_available: money(9_000_000),
            open_holds: money(1_312_500),
            spent: money(2_250_000),
            runway,
            funds_state,
            own_allocation: own,
            consent: vec![
                ConsentLineView {
                    status: consent,
                    payer: None,
                    amount: None,
                },
                ConsentLineView {
                    status: consent,
                    payer: Some(ID_B.into()),
                    amount: Some(money(2_000_000)),
                },
            ],
            end_of_session: end,
        }));
    }
    for (output_type, cancellation) in [
        (OutputType::Narration, CancellationPolicy::FreeBeforeSend),
        (OutputType::Image, CancellationPolicy::ChargedIfDelivered),
        (OutputType::Voice, CancellationPolicy::FreeBeforeSend),
    ] {
        views.push(to_value(QuoteView {
            schema: QUOTE_SCHEMA,
            output_type,
            estimate: money(750_000),
            maximum: money(1_312_500),
            own_share_estimate: money(375_000),
            expires_at_ms: 1_700_000_300_000,
            latency: Some(LatencyRangeView {
                from_ms: 2_000,
                to_ms: 9_000,
            }),
            cancellation,
            price_version_label: label.clone(),
        }));
    }
    views.push(to_value(QuoteView {
        schema: QUOTE_SCHEMA,
        output_type: OutputType::Image,
        estimate: money(750_000),
        maximum: money(1_312_500),
        own_share_estimate: money(375_000),
        expires_at_ms: 1_700_000_300_000,
        latency: None,
        cancellation: CancellationPolicy::FreeBeforeSend,
        price_version_label: label.clone(),
    }));
    let lines = [
        (
            CategoryLabel::Narration,
            LineStatus::Settled,
            QuantityUnit::Tokens,
            41_250,
        ),
        (
            CategoryLabel::Image,
            LineStatus::PendingHold,
            QuantityUnit::Images,
            2,
        ),
        (
            CategoryLabel::Voice,
            LineStatus::Credited,
            QuantityUnit::Seconds,
            95,
        ),
        (
            CategoryLabel::TableTime,
            LineStatus::TechnicalFailureCredited,
            QuantityUnit::Calls,
            1,
        ),
        (
            CategoryLabel::Narration,
            LineStatus::ReleasedNotCharged,
            QuantityUnit::Tokens,
            7,
        ),
    ]
    .into_iter()
    .map(|(category, status, unit, quantity)| StatementLineView {
        category,
        at_ms: 1_700_000_100_000,
        session: ID_A.into(),
        campaign: ID_B.into(),
        quantity_text: quantity_text(unit, quantity),
        price_version_label: label.clone(),
        charge: money(1_234_567),
        reference: ID_A.into(),
        status,
    })
    .collect();
    views.push(to_value(StatementView {
        schema: STATEMENT_SCHEMA,
        lines,
        totals: StatementTotalsView {
            charged: money(6_172_835),
            credited: money(1_234_567),
        },
        provisional: false,
    }));
    views.push(to_value(AdmissionView {
        reservation: ID_A.into(),
        attempt: ID_B.into(),
        held: money(1_312_500),
        price_version_label: label,
    }));
    views
        .into_iter()
        .map(|view| view.expect("a view serializes"))
        .collect()
}

fn is_hex32(text: &str) -> bool {
    text.len() == 32
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_label(text: &str) -> bool {
    text.strip_prefix("pv-").is_some_and(|rest| {
        rest.len() == 5
            && rest
                .bytes()
                .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
    })
}

fn is_quantity_text(text: &str) -> bool {
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0 && ["tok", "img", "s", "call"].contains(&&text[digits..])
}

fn is_schema(text: &str) -> bool {
    text.strip_prefix("dungeonflux.finance.")
        .and_then(|rest| rest.split_once('/'))
        .is_some_and(|(name, number)| {
            !name.is_empty()
                && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                && number.bytes().all(|b| b.is_ascii_digit())
        })
}

/// The closed class of a string value, or `None` for anything free-form.
fn class_of(path: &str, text: &str) -> Option<String> {
    if is_hex32(text) {
        return Some("hex32".into());
    }
    if is_label(text) {
        return Some("price_version_label".into());
    }
    if path.ends_with("quantity_text") {
        return is_quantity_text(text).then(|| "quantity_text".into());
    }
    if path.ends_with("schema") {
        return is_schema(text).then(|| format!("schema:{text}"));
    }
    if path.ends_with("card_suffix") {
        return (text.len() == 4 && text.bytes().all(|b| b.is_ascii_digit()))
            .then(|| "digits4".into());
    }
    if path.ends_with("next_cursor") {
        return (!text.is_empty()
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
        .then(|| "cursor".into());
    }
    if path.ends_with("currency") {
        return (text.len() == 3 && text.bytes().all(|b| b.is_ascii_uppercase()))
            .then(|| "currency".into());
    }
    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
        .then(|| "word".into())
}

/// Every leaf of `value` as `path -> (json type, class, words seen)`.
fn leaves(
    prefix: &str,
    value: &Value,
    out: &mut BTreeMap<String, (String, String, BTreeSet<String>)>,
) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                let path = format!("{prefix}.{key}");
                leaves(&path, inner, out);
            }
        }
        Value::Array(items) => {
            for inner in items {
                leaves(&format!("{prefix}[]"), inner, out);
            }
        }
        Value::String(text) => {
            let class = class_of(prefix, text).unwrap_or_else(|| "FREE_TEXT".into());
            let entry = out.entry(prefix.to_owned()).or_insert((
                "string".into(),
                class.clone(),
                BTreeSet::new(),
            ));
            if class == "word" {
                entry.2.insert(text.clone());
            }
            if entry.1 != class {
                entry.1 = format!("{}|{class}", entry.1);
            }
        }
        Value::Number(_) => {
            out.entry(prefix.to_owned()).or_insert((
                "number".into(),
                String::new(),
                BTreeSet::new(),
            ));
        }
        Value::Bool(_) => {
            out.entry(prefix.to_owned())
                .or_insert(("bool".into(), String::new(), BTreeSet::new()));
        }
        Value::Null => {}
    }
}

/// The golden text of a set of views: one line per leaf path with its type, class and vocabulary.
fn golden_of(values: &[Value]) -> String {
    let mut all = BTreeMap::new();
    for value in values {
        let view = value
            .get("schema")
            .and_then(Value::as_str)
            .unwrap_or("admission");
        leaves(view, value, &mut all);
    }
    all.iter()
        .map(|(path, (kind, class, words))| {
            let vocabulary = words.iter().cloned().collect::<Vec<_>>().join("|");
            let line = format!("{path} {kind} {class} {vocabulary}");
            format!("{line}\n", line = line.trim_end())
        })
        .collect()
}

/// Lines of the observed golden that are not in the checked-in one, and the reverse.
fn golden_violations(values: &[Value]) -> Vec<String> {
    let observed: BTreeSet<String> = golden_of(values).lines().map(str::to_owned).collect();
    let allowed: BTreeSet<String> = GOLDEN.lines().map(str::to_owned).collect();
    observed
        .symmetric_difference(&allowed)
        .cloned()
        .chain(
            observed
                .iter()
                .filter(|line| line.contains("FREE_TEXT"))
                .cloned(),
        )
        .collect()
}

#[test]
fn every_customer_view_matches_its_allowlist_golden_schema() {
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/views/customer_golden.txt");
        std::fs::write(path, golden_of(&fixtures())).expect("write the golden");
    }
    assert_eq!(golden_violations(&fixtures()), Vec::<String>::new());
}

/// The numbers and names of the supplier and table side that no customer view may contain.
const FORBIDDEN: [&str; 14] = [
    "1.75", "7/4", "2.37", "2370000", "0.571", "0.5714", "8123", "7319", "zephyr", "ultrafox",
    "markup", "provider", "supplier", "margin",
];

fn scan_violations(text: &str) -> Vec<String> {
    FORBIDDEN
        .iter()
        .filter(|needle| text.contains(**needle))
        .map(|needle| (*needle).to_owned())
        .collect()
}

/// A number outside the money, timestamp and count fields would be a rate or a cost by another name.
fn stray_numbers(values: &[Value]) -> Vec<String> {
    let mut all = BTreeMap::new();
    for value in values {
        let view = value
            .get("schema")
            .and_then(Value::as_str)
            .unwrap_or("admission");
        leaves(view, value, &mut all);
    }
    all.into_iter()
        .filter(|(path, (kind, _, _))| {
            kind == "number"
                && !["micro_dollars", "display_cents", "_ms"]
                    .iter()
                    .any(|suffix| path.ends_with(suffix))
        })
        .map(|(path, _)| path)
        .collect()
}

#[test]
fn distinctive_rates_appear_in_no_key_or_value_of_any_customer_view() {
    let values = fixtures();
    let text = serde_json::to_string(&values).expect("json");
    assert_eq!(scan_violations(&text), Vec::<String>::new());
    assert_eq!(stray_numbers(&values), Vec::<String>::new());
}

#[test]
fn the_price_version_label_is_opaque() {
    let a = price_version_label(PriceVersionId::from_array([1; 16]));
    let b = price_version_label(PriceVersionId::from_array([2; 16]));
    assert!(is_label(&a) && is_label(&b));
    assert_ne!(a, b);
    assert_eq!(a, price_version_label(PriceVersionId::from_array([1; 16])));
}

#[test]
fn quantity_text_is_produced_only_by_the_closed_formatter() {
    assert_eq!(quantity_text(QuantityUnit::Tokens, 1500), "1500tok");
    assert_eq!(quantity_text(QuantityUnit::Seconds, 95), "95s");
    for unit in [
        QuantityUnit::Tokens,
        QuantityUnit::Images,
        QuantityUnit::Seconds,
        QuantityUnit::Calls,
    ] {
        assert!(is_quantity_text(&quantity_text(unit, 12)));
    }
    assert!(!is_quantity_text("12tok*7/4"));
}

#[test]
fn the_customer_module_imports_nothing_from_the_operator_module() {
    let source = include_str!("customer.rs");
    for needle in ["operator", "views::operator", "super::operator"] {
        let uses = source
            .lines()
            .filter(|line| line.contains("use ") && line.contains(needle))
            .count();
        assert_eq!(uses, 0, "customer.rs imports {needle}");
    }
}

#[test]
fn a_request_with_an_unknown_field_is_refused() {
    let refused: Result<TransactionListRequest, _> =
        serde_json::from_str("{\"cursor\":null,\"limit\":10,\"markup\":1}");
    assert!(refused.is_err());
    let accepted: TransactionListRequest =
        serde_json::from_str("{\"cursor\":null,\"limit\":10}").expect("request");
    assert_eq!(accepted.limit, 10);
}

#[test]
fn a_view_with_an_added_label_string_field_fails_the_golden_schema() {
    let mut values = fixtures();
    values[0]["label"] = Value::String("hello".into());
    assert!(!golden_violations(&values).is_empty());
}

#[test]
fn a_markup_in_quantity_text_fails_the_golden_schema_and_the_value_scan() {
    let mut values = fixtures();
    let statement = values
        .iter_mut()
        .find(|view| view.get("lines").is_some())
        .expect("statement");
    statement["lines"][0]["quantity_text"] = Value::String("41250tok*7/4".into());
    assert!(!golden_violations(&values).is_empty());
    assert!(!scan_violations(&serde_json::to_string(&values).expect("json")).is_empty());
}

#[test]
fn a_provider_name_in_the_version_label_fails_the_golden_schema_and_the_value_scan() {
    let mut values = fixtures();
    let quote = values
        .iter_mut()
        .find(|view| view.get("cancellation").is_some())
        .expect("quote");
    quote["price_version_label"] = Value::String("pv-zephyr".into());
    assert!(!golden_violations(&values).is_empty());
    assert!(!scan_violations(&serde_json::to_string(&values).expect("json")).is_empty());
}

#[test]
fn a_supplier_amount_added_to_a_view_fails_the_stray_number_scan() {
    let mut values = fixtures();
    values[0]["supplier_cost"] = Value::from(7_319);
    assert!(!stray_numbers(&values).is_empty());
}
