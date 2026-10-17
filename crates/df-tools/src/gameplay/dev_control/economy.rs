//! `dev scene-changes`: the scene changes in the decision journal with their receipts, read as
//! stored. Edits to an NPC purse, a hero's coin or an offer would be staged decisions through the
//! owner's override path; that path is disabled server-side, so those writes answer unavailable.

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
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

/// `POST /dev/economy/edit`: purse, coin and offer edits are not performed by this surface.
pub(super) async fn unsupported_edit(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Outcome {
    authorized(&headers, &state.token)?;
    write_unavailable(
        "economy",
        "NPC purse, hero coin and offer stock edits are staged decisions through the owner's override path, which is disabled server-side until the milestone 3 projection-safety proof",
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

    #[test]
    fn economy_edits_answer_unavailable_as_a_named_refusal() {
        let Ok((status, Json(answer))) =
            write_unavailable("economy", "staged decisions are disabled")
        else {
            panic!("an unavailable write is an answer, not a failure");
        };
        assert_eq!(status, StatusCode::OK);
        assert_eq!(answer["status"], "unavailable");
        assert_eq!(answer["edited"], false);
    }
}
