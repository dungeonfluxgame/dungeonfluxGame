//! `dev scene-changes`: the scene changes in the decision journal with their receipts, read as
//! stored. Economy edits must use the staged override path for durable checkpoint changes.

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

use super::{Outcome, Shared, authorized, bad_request, failed, parse, write_unavailable};
use crate::gameplay::journey::living_scene::{self, JournalSceneChange};

/// `GET /dev/scene-changes?last=<n>`: the newest scene changes, oldest first.
pub(super) async fn scene_changes(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let last =
        parse::parse_last(params.get("last").map_or("50", String::as_str)).map_err(bad_request)?;
    let checkpoint = state.service.updates.borrow().clone();
    let journal = living_scene::scene_change_journal(&checkpoint)
        .map_err(|_| failed("scene journal unavailable"))?;
    Ok((StatusCode::OK, Json(journal_document(journal, last)?)))
}

/// The newest `last` journal entries as the operator reads them. The receipt of a theft, a
/// pickpocket, a plant or a coerced handover is the change's own `kind` object, as stored.
fn journal_document(
    journal: Vec<JournalSceneChange>,
    last: usize,
) -> Result<Value, (StatusCode, Json<Value>)> {
    let total = journal.len();
    let mut changes = Vec::new();
    for change in journal.into_iter().skip(total.saturating_sub(last)) {
        let kind = serde_json::to_value(&change.change.kind)
            .map_err(|_| failed("scene change unavailable"))?;
        changes.push(json!({
            "revision": {
                "epoch": change.revision.epoch().get(),
                "sequence": change.revision.sequence(),
            },
            "place": change.place,
            "hero": change.hero,
            "paid_cp": change.paid_cp,
            "entity_id": change.change.entity_id,
            "description": change.change.description,
            "kind": kind,
        }));
    }
    Ok(json!({
        "count": total,
        "returned": changes.len(),
        "changes": changes,
        "scope": "the committed decision journal of the current checkpoint; read as stored, never re-judged",
    }))
}

/// One economy edit accepted by `POST /dev/economy/edit`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EditBody {
    op: String,
    target: String,
    #[serde(default)]
    cp: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum EconomyEdit {
    Purse { target: String, cp: u32 },
    Coin { target: String, cp: u32 },
    OfferAdd { target: String },
    OfferRemove { target: String },
    OfferRestock { target: String },
}

fn validate_edit(body: EditBody) -> Result<EconomyEdit, (StatusCode, Json<Value>)> {
    let target = body.target.trim();
    if target.is_empty() {
        return Err(bad_request("economy edit needs a non-empty target"));
    }
    let target = target.to_owned();
    match (body.op.as_str(), body.cp) {
        ("purse", Some(cp)) => Ok(EconomyEdit::Purse { target, cp }),
        ("coin", Some(cp)) => Ok(EconomyEdit::Coin { target, cp }),
        ("offer-add", None) => Ok(EconomyEdit::OfferAdd { target }),
        ("offer-remove", None) => Ok(EconomyEdit::OfferRemove { target }),
        ("offer-restock", None) => Ok(EconomyEdit::OfferRestock { target }),
        ("purse" | "coin", None) => Err(bad_request("purse and coin edits need cp")),
        ("offer-add" | "offer-remove" | "offer-restock", Some(_)) => {
            Err(bad_request("offer edits do not take cp"))
        }
        _ => Err(bad_request(
            "economy edit takes purse|coin with cp or offer-add|offer-remove|offer-restock",
        )),
    }
}

/// The override record shared with the edit response and `/dev/log`.
fn override_record(edit: &EconomyEdit, before: &str, after: &str) -> Value {
    let (kind, target) = match edit {
        EconomyEdit::Purse { target, .. } => ("purse", target),
        EconomyEdit::Coin { target, .. } => ("coin", target),
        EconomyEdit::OfferAdd { target } => ("offer_add", target),
        EconomyEdit::OfferRemove { target } => ("offer_remove", target),
        EconomyEdit::OfferRestock { target } => ("offer_restock", target),
    };
    json!({"kind": "economy", "operation": kind, "target": target, "before": before, "after": after})
}

/// `POST /dev/economy/edit`: validation is local; ledger changes must be committed by the owner.
pub(super) async fn unsupported_edit(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<EditBody>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let edit = validate_edit(body)?;
    let _record = override_record(&edit, "", "");
    write_unavailable(
        "economy",
        "economy edits are unavailable until the durable ledger mutation path is connected",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::{
        adventure::{RulesMode, record, shop},
        beat_script_tests::input,
        beat_script_variety_tests as native, journey, model, pack,
        startup_configuration::ContentRating,
    };
    use df_model::checkpoint::Checkpoint;

    /// One step of the Marek clothes trade, as the ledger tests stage it.
    fn trade_clothes(current: &Checkpoint, operation: u8, payment: &str) -> Checkpoint {
        let choices = [
            ("scene-target", "clothes-travelers"),
            ("scene-option", "trade"),
            ("scene-payment", payment),
        ]
        .into_iter()
        .map(|(key, value)| (model::label(key).unwrap(), model::label(value).unwrap()))
        .collect();
        journey::stage_with_supplier(
            current,
            &input(
                current,
                native::first(),
                operation,
                "script-choice",
                choices,
            ),
            &mut |_| panic!("ordinary transfer does not roll"),
        )
        .unwrap()
    }

    #[test]
    fn scene_changes_list_the_purchase_as_stored_and_bound_the_count() {
        let generated = shop::generate(46, RulesMode::Standard, ContentRating::default()).unwrap();
        let (loaded, _) = record::reload(&generated.documents(true).unwrap()).unwrap();
        pack::with_test_pack(loaded, || {
            let opening = native::act(&native::prepared(), native::first(), 5, "begin-story");
            let empty = journal_document(living_scene::scene_change_journal(&opening).unwrap(), 50)
                .unwrap();
            assert_eq!(empty["count"], 0);

            let quoted = trade_clothes(&opening, 6, "100");
            let bought = trade_clothes(&quoted, 7, "200");
            let shown =
                journal_document(living_scene::scene_change_journal(&bought).unwrap(), 50).unwrap();
            assert_eq!(shown["count"], 1);
            let change = &shown["changes"][0];
            assert_eq!(change["place"], "place-mareks-cloth-shop");
            assert_eq!(change["paid_cp"], 200);
            assert_eq!(change["kind"]["kind"], "purchased");
            assert_eq!(change["kind"]["cost_cp"], 200);

            let newest =
                journal_document(living_scene::scene_change_journal(&bought).unwrap(), 1).unwrap();
            assert_eq!(newest["returned"], 1);
            assert_eq!(newest["count"], 1);
        });
    }

    fn body(op: &str, target: &str, cp: Option<u32>) -> EditBody {
        EditBody {
            op: op.to_owned(),
            target: target.to_owned(),
            cp,
        }
    }

    #[test]
    fn economy_edits_validate_required_values_and_reject_wrong_arguments() {
        assert_eq!(
            validate_edit(body("purse", "merchant", Some(25))).unwrap(),
            EconomyEdit::Purse {
                target: "merchant".to_owned(),
                cp: 25,
            }
        );
        assert!(validate_edit(body("coin", "Brynn", None)).is_err());
        assert!(validate_edit(body("offer-add", "item-cloak", Some(2))).is_err());
        assert!(validate_edit(body("offer-restock", " ", None)).is_err());
        assert!(validate_edit(body("bogus", "merchant", None)).is_err());
    }

    #[test]
    fn economy_override_record_identifies_operation_target_and_before_after() {
        let record = override_record(
            &EconomyEdit::Coin {
                target: "Brynn".to_owned(),
                cp: 125,
            },
            "100 cp",
            "125 cp",
        );
        assert_eq!(record["kind"], "economy");
        assert_eq!(record["operation"], "coin");
        assert_eq!(record["target"], "Brynn");
        assert_eq!(record["before"], "100 cp");
        assert_eq!(record["after"], "125 cp");
    }
}
