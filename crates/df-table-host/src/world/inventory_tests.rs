//! Tests of the standard inventory: the pure rules, every route, every typed error, the transfer's
//! atomicity, the judged path, the stored shape and many owners.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::inventory::cli::{InventoryCommand, parse as parse_inventory};
use super::inventory::request::{
    Changed, History, InventoryError, InventoryReply, InventoryView, OwnerList,
};
use super::inventory::routes::path;
use super::inventory::{
    ChangeKind, GainContext, GainSource, Inventory, InventoryChange, InventoryDelta, InventoryItem,
    InventoryReason, ItemCount, ItemUse, Judged, OwnerKind, SCHEMA, apply_changes, change_of,
    entries_token, inventory_of, judge_deltas, sheet_name, take_judged_changes, transfer,
};
use crate::command::{CommandError, split_words};
use crate::dev_client::{get_json, post_json};
use crate::dev_http::{DevReply, RouteError};
use crate::test_support::{Running, STARTER_FILES, start};

fn item(id: &str, count: u64) -> InventoryItem {
    InventoryItem {
        id: id.to_owned(),
        name: id.replace('_', " "),
        look: format!("the {id} as it looks"),
        usage: ItemUse::Consumed,
        count,
    }
}

fn inventory(owner: &str, kind: OwnerKind, items: &[(&str, u64)]) -> Inventory {
    let items = items.iter().map(|(id, count)| item(id, *count)).collect();
    Inventory::new(owner, kind, items).expect("a valid inventory")
}

fn consume(id: &str, count: u64) -> InventoryChange {
    InventoryChange::Consume {
        item: id.to_owned(),
        count,
        why: "used".to_owned(),
    }
}

fn remove(id: &str, count: u64) -> InventoryChange {
    InventoryChange::Remove {
        item: id.to_owned(),
        count,
        why: String::new(),
    }
}

fn add(id: &str, count: u64) -> InventoryChange {
    InventoryChange::Add {
        item: id.to_owned(),
        count,
        name: None,
        look: None,
        usage: None,
        why: String::new(),
    }
}

fn counts(inventory: &Inventory) -> Vec<(&str, u64)> {
    inventory
        .items
        .iter()
        .map(|item| (item.id.as_str(), item.count))
        .collect()
}

#[test]
fn closed_keys_are_their_serde_names() {
    for kind in OwnerKind::ALL {
        assert_eq!(json!(kind), json!(kind.key()));
        assert_eq!(OwnerKind::from_key(kind.key()), Some(kind));
    }
    for kind in ChangeKind::ALL {
        assert_eq!(json!(kind), json!(kind.key()));
        assert_eq!(ChangeKind::from_key(kind.key()), Some(kind));
    }
    assert_eq!(OwnerKind::from_key("bank"), None);
}

#[test]
fn an_inventory_is_checked_when_it_is_made() {
    let word = |id: &str, name: &str, look: &str| InventoryItem {
        id: id.to_owned(),
        name: name.to_owned(),
        look: look.to_owned(),
        usage: ItemUse::Consumed,
        count: 1,
    };
    let make = |owner: &str, items: Vec<InventoryItem>| {
        Inventory::new(owner, OwnerKind::Character, items).unwrap_err()
    };
    assert!(matches!(
        make("Danny", vec![]),
        InventoryError::InvalidOwner { .. }
    ));
    assert!(matches!(
        make("", vec![]),
        InventoryError::InvalidOwner { .. }
    ));
    assert!(matches!(
        make("danny", vec![word("Easel", "easel", "pale")]),
        InventoryError::InvalidItem { .. }
    ));
    let twice = vec![
        word("easel", "easel", "pale"),
        word("easel", "easel", "pale"),
    ];
    assert_eq!(
        make("danny", twice),
        InventoryError::DuplicateItem {
            item: "easel".to_owned()
        }
    );
    assert_eq!(
        make("danny", vec![word("easel", " ", "pale")]),
        InventoryError::MissingWords {
            item: "easel".to_owned()
        }
    );
    let mut stored = inventory("danny", OwnerKind::Character, &[("easel", 1)]);
    stored.schema = SCHEMA + 1;
    assert_eq!(
        stored.check(),
        Err(InventoryError::InvalidSchema { schema: SCHEMA + 1 })
    );
}

#[test]
fn a_batch_is_taken_whole_with_each_change_as_it_came_to() {
    let before = inventory("danny", OwnerKind::Character, &[("ochre", 3), ("rags", 5)]);
    assert_eq!(
        apply_changes(
            &before,
            &[consume("ochre", 2), add("rags", 4), remove("ochre", 1)]
        )
        .unwrap_err(),
        InventoryError::DuplicateItem {
            item: "ochre".to_owned()
        }
    );

    let (after, applied) = apply_changes(&before, &[consume("ochre", 2), add("rags", 4)]).unwrap();
    assert_eq!(counts(&after), [("ochre", 1), ("rags", 9)]);
    assert_eq!(
        applied
            .iter()
            .map(|taken| (taken.item.as_str(), taken.kind, taken.before, taken.after))
            .collect::<Vec<_>>(),
        [
            ("ochre", ChangeKind::Consume, 3, 1),
            ("rags", ChangeKind::Add, 5, 9)
        ]
    );
    assert_eq!(
        counts(&before),
        [("ochre", 3), ("rags", 5)],
        "the input is untouched"
    );
}

#[test]
fn every_refusal_of_a_change_is_typed_and_changes_nothing() {
    let danny = inventory(
        "danny",
        OwnerKind::Character,
        &[("ochre", 3), ("canvas", 0)],
    );
    let refused = |changes: &[InventoryChange]| apply_changes(&danny, changes).unwrap_err();
    assert_eq!(refused(&[]), InventoryError::EmptyBatch);
    assert_eq!(
        refused(&[consume("lute", 1)]),
        InventoryError::UnknownItem {
            owner: "danny".to_owned(),
            item: "lute".to_owned()
        }
    );
    assert_eq!(
        refused(&[consume("ochre", 4)]),
        InventoryError::Insufficient {
            owner: "danny".to_owned(),
            item: "ochre".to_owned(),
            have: 3,
            asked: 4
        }
    );
    assert_eq!(
        refused(&[remove("canvas", 1)]).key(),
        "insufficient",
        "an item at zero cannot go below it"
    );
    assert_eq!(
        refused(&[consume("ochre", 0)]),
        InventoryError::ZeroCount {
            item: "ochre".to_owned()
        }
    );
    assert_eq!(refused(&[consume("Ochre", 1)]).key(), "invalid_item");
    assert_eq!(
        refused(&[consume("ochre", 1), add("ochre", 1)]).key(),
        "duplicate_item"
    );
    assert_eq!(
        refused(&[add("ochre", u64::MAX)]),
        InventoryError::Overflow {
            owner: "danny".to_owned(),
            item: "ochre".to_owned()
        }
    );
    assert_eq!(refused(&[add("brush", 1)]).key(), "missing_words");
    // One bad change anywhere in a batch refuses the rest too.
    assert_eq!(
        refused(&[consume("ochre", 1), consume("ochre_2", 1)]).key(),
        "unknown_item"
    );
    // A new item comes with its words, and an item that is held keeps its own.
    let new = InventoryChange::Add {
        item: "brush".to_owned(),
        count: 2,
        name: Some("brush".to_owned()),
        look: Some("splayed bristles".to_owned()),
        usage: Some(ItemUse::Kept),
        why: "bought".to_owned(),
    };
    let (after, _) = apply_changes(&danny, &[new]).unwrap();
    assert_eq!(after.item("brush").map(|brush| brush.count), Some(2));
    assert_eq!(
        after.item("brush").map(|brush| brush.usage),
        Some(ItemUse::Kept)
    );
    // An item that reaches zero stays listed.
    let (after, _) = apply_changes(&danny, &[consume("ochre", 3)]).unwrap();
    assert_eq!(after.item("ochre").map(|ochre| ochre.count), Some(0));
    assert_eq!(after.held().count(), 0);
}

#[test]
fn a_transfer_changes_both_sides_or_neither() {
    let giver = inventory("danny", OwnerKind::Character, &[("ochre", 3), ("rags", 1)]);
    let shop = inventory("baker", OwnerKind::Shop, &[("rags", 2)]);
    let (giver_after, shop_after, applied) = transfer(&giver, &shop, ("ochre", 2), "sold").unwrap();
    assert_eq!(counts(&giver_after), [("ochre", 1), ("rags", 1)]);
    assert_eq!(counts(&shop_after), [("rags", 2), ("ochre", 2)]);
    assert_eq!(
        shop_after.item("ochre").map(|ochre| ochre.look.as_str()),
        Some("the ochre as it looks"),
        "the receiver gains the item with the giver's words"
    );
    assert_eq!(
        applied
            .iter()
            .map(|taken| (taken.owner.as_str(), taken.kind))
            .collect::<Vec<_>>(),
        [
            ("danny", ChangeKind::TransferOut),
            ("baker", ChangeKind::TransferIn)
        ]
    );
    assert_eq!(
        transfer(&giver, &giver, ("ochre", 1), "").unwrap_err(),
        InventoryError::SameOwner {
            owner: "danny".to_owned()
        }
    );
    assert_eq!(
        transfer(&giver, &shop, ("ochre", 4), "").unwrap_err().key(),
        "insufficient"
    );
    assert_eq!(
        transfer(&giver, &shop, ("lute", 1), "").unwrap_err().key(),
        "unknown_item"
    );
    assert_eq!(
        transfer(&giver, &shop, ("ochre", 0), "").unwrap_err().key(),
        "zero_count"
    );
    let huge = inventory("baker", OwnerKind::Shop, &[("ochre", u64::MAX)]);
    assert_eq!(
        transfer(&giver, &huge, ("ochre", 1), "").unwrap_err().key(),
        "overflow",
        "a receiver that cannot take it refuses the whole transfer"
    );
}

fn delta(item: &str, change: i64, reason: InventoryReason) -> InventoryDelta {
    InventoryDelta {
        item: item.to_owned(),
        change,
        reason: Some(reason),
        why: "the line shows it".to_owned(),
        name: None,
        look: None,
        usage: None,
        source: None,
        paid_with: Vec::new(),
        made_from: Vec::new(),
        place_thing: None,
        giver: None,
        what_happened: Some("it happened in the line".to_owned()),
        to: None,
    }
}

#[test]
fn a_judged_delta_is_a_gain_a_loss_a_removal_or_nothing_by_its_reason() {
    use InventoryReason::{Broken, Gained, GivenAway, Lost, Sold, UsedUp, WornOut};
    let change = |delta: &InventoryDelta| change_of(delta).unwrap().unwrap();
    assert!(matches!(
        change(&delta("rags", 3, Gained)),
        InventoryChange::Add { count: 3, .. }
    ));
    assert!(matches!(
        change(&delta("ochre", -2, UsedUp)),
        InventoryChange::Consume { count: 2, .. }
    ));
    for reason in [Sold, WornOut, Broken, Lost, GivenAway] {
        assert!(
            matches!(
                change(&delta("brush", -1, reason)),
                InventoryChange::Remove { count: 1, .. }
            ),
            "{reason:?} takes the item away without using it up"
        );
    }
    assert_eq!(change_of(&delta("canvas", 0, UsedUp)), Ok(None));
    assert!(matches!(
        change(&delta("ochre", i64::MIN, UsedUp)),
        InventoryChange::Consume {
            count: 9_223_372_036_854_775_808,
            ..
        }
    ));
    // A reason that does not fit the direction is refused.
    for bad in [
        delta("ochre", -1, Gained),
        delta("ochre", 1, UsedUp),
        delta("ochre", 1, Lost),
    ] {
        assert_eq!(
            change_of(&bad),
            Err(InventoryError::InvalidReason {
                item: "ochre".to_owned()
            })
        );
    }
    assert_eq!(sheet_name("danny", "easel"), "item-danny-easel.png");
}

#[test]
fn a_kept_item_is_never_used_up_but_leaves_by_a_removal() {
    let mut easel = item("easel", 1);
    easel.usage = ItemUse::Kept;
    let mut brushes = item("brushes", 5);
    brushes.usage = ItemUse::Kept;
    let danny = Inventory::new(
        "danny",
        OwnerKind::Character,
        vec![easel, brushes, item("ochre", 3)],
    )
    .unwrap();
    assert_eq!(
        apply_changes(&danny, &[consume("easel", 1)]).unwrap_err(),
        InventoryError::KeptItem {
            owner: "danny".to_owned(),
            item: "easel".to_owned()
        }
    );
    assert_eq!(
        apply_changes(&danny, &[consume("ochre", 1), consume("brushes", 1)])
            .unwrap_err()
            .key(),
        "kept_item",
        "one kept item in a batch refuses the batch"
    );
    // Wearing out, breaking, losing and giving away are removals, which a kept item allows.
    let (after, applied) =
        apply_changes(&danny, &[remove("brushes", 2), consume("ochre", 1)]).unwrap();
    assert_eq!(counts(&after), [("easel", 1), ("brushes", 3), ("ochre", 2)]);
    assert_eq!(applied[0].kind, ChangeKind::Remove);
    assert_eq!(
        apply_changes(&danny, &[remove("easel", 2)])
            .unwrap_err()
            .key(),
        "insufficient",
        "a kept item cannot go below zero either"
    );
    // A transfer gives the receiver the item with its use.
    let shop = inventory("baker", OwnerKind::Shop, &[]);
    let (_, shop_after, _) = transfer(&danny, &shop, ("brushes", 1), "sold").unwrap();
    assert_eq!(
        shop_after.item("brushes").map(|item| item.usage),
        Some(ItemUse::Kept)
    );
    assert_eq!(ItemUse::from_key("kept"), Some(ItemUse::Kept));
    assert_eq!(json!(ItemUse::Kept), json!("kept"));
    assert_eq!(ItemUse::default(), ItemUse::Consumed);
}

#[test]
fn closed_reasons_are_their_serde_names() {
    for reason in InventoryReason::ALL {
        assert_eq!(json!(reason), json!(reason.key()));
        assert_eq!(InventoryReason::from_key(reason.key()), Some(reason));
    }
    for usage in ItemUse::ALL {
        assert_eq!(json!(usage), json!(usage.key()));
    }
    assert_eq!(InventoryReason::from_key("stolen"), None);
}

async fn host() -> Running {
    start(&STARTER_FILES).await
}

async fn post<B: Serialize, T: DeserializeOwned>(
    host: &Running,
    address: &str,
    body: B,
) -> InventoryReply<T> {
    post_json(host.port, address, &body).await.unwrap()
}

async fn read<T: DeserializeOwned>(host: &Running, address: &str) -> InventoryReply<T> {
    get_json(host.port, address).await.unwrap()
}

fn done<T: std::fmt::Debug>(reply: InventoryReply<T>) -> T {
    match reply {
        DevReply::Done { value } => value,
        DevReply::Rejected { error, message } => panic!("refused: {error:?} {message}"),
    }
}

fn refusal<T: std::fmt::Debug>(reply: InventoryReply<T>) -> InventoryError {
    match reply {
        DevReply::Done { value } => panic!("done: {value:?}"),
        DevReply::Rejected { error, .. } => error,
    }
}

async fn seed(host: &Running, owner: &str, kind: &str, items: &[(&str, u64)]) -> Changed {
    let items: Vec<Value> = items
        .iter()
        .map(|(id, count)| json!(item(id, *count)))
        .collect();
    done(
        post(
            host,
            path::INVENTORY_SET,
            json!({"owner": owner, "kind": kind, "items": items}),
        )
        .await,
    )
}

async fn owner_view(host: &Running, owner: &str) -> InventoryReply<InventoryView> {
    read(host, &format!("{}?owner={owner}", path::INVENTORY_OWNER)).await
}

async fn held(host: &Running, owner: &str) -> Vec<(String, u64)> {
    done(owner_view(host, owner).await)
        .inventory
        .items
        .into_iter()
        .map(|item| (item.id, item.count))
        .collect()
}

fn lines(host: &Running) -> Vec<String> {
    host.state
        .log()
        .after_sequence(0, usize::MAX)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("WORLD"))
        .collect()
}

#[tokio::test]
async fn owners_of_every_kind_are_set_listed_and_read() {
    let host = host().await;
    assert!(
        done(read::<OwnerList>(&host, path::INVENTORY).await)
            .owners
            .is_empty()
    );
    seed(&host, "danny", "character", &[("ochre", 3), ("rags", 2)]).await;
    seed(&host, "party-stash", "party", &[("rope", 1)]).await;
    seed(&host, "chest", "container", &[]).await;
    seed(&host, "baker", "shop", &[("loaf", 12)]).await;

    let list = done(read::<OwnerList>(&host, path::INVENTORY).await);
    let summary: Vec<_> = list
        .owners
        .iter()
        .map(|owner| (owner.owner.as_str(), owner.kind, owner.items, owner.units))
        .collect();
    assert_eq!(
        summary,
        [
            ("baker", OwnerKind::Shop, 1, 12),
            ("chest", OwnerKind::Container, 0, 0),
            ("danny", OwnerKind::Character, 2, 5),
            ("party-stash", OwnerKind::Party, 1, 1),
        ],
        "in id order, with what each holds"
    );

    let view = done(owner_view(&host, "danny").await);
    assert_eq!(view.inventory.schema, SCHEMA);
    assert_eq!(view.inventory.kind, OwnerKind::Character);
    // The page's list panel: a loot row for each item held, with its count as the server words it.
    let rows: Vec<_> = view
        .ledger
        .items
        .iter()
        .map(|row| (row.id.as_str(), row.name.as_str(), row.quantity.as_deref()))
        .collect();
    assert_eq!(
        rows,
        [("ochre", "ochre", Some("x3")), ("rags", "rags", Some("x2"))]
    );

    // Setting again replaces the whole inventory and may change the kind.
    seed(&host, "danny", "party", &[("ochre", 1)]).await;
    assert_eq!(held(&host, "danny").await, [("ochre".to_owned(), 1)]);
    let again = done(read::<OwnerList>(&host, path::INVENTORY).await);
    assert_eq!(again.owners.len(), 4, "an owner is listed once");
    assert!(
        lines(&host).contains(&"WORLD inventory_set owner=chest kind=container items=0".to_owned())
    );
}

#[tokio::test]
async fn add_remove_consume_and_batch_change_one_owner() {
    let host = host().await;
    seed(&host, "danny", "character", &[("ochre", 3), ("rags", 2)]).await;

    let changed: Changed = done(
        post(
            &host,
            path::INVENTORY_CONSUME,
            json!({"owner": "danny", "item": "ochre", "count": 2, "why": "sky"}),
        )
        .await,
    );
    assert_eq!(
        (
            changed.applied[0].kind,
            changed.applied[0].before,
            changed.applied[0].after
        ),
        (ChangeKind::Consume, 3, 1)
    );
    assert_eq!(
        changed.owners[0].ledger.items[0].quantity.as_deref(),
        Some("x1")
    );

    let _: Changed = done(
        post(
            &host,
            path::INVENTORY_ADD,
            json!({"owner": "danny", "item": "brush", "count": 2,
                "name": "brush", "look": "splayed bristles", "why": "bought"}),
        )
        .await,
    );
    let _: Changed = done(
        post(
            &host,
            path::INVENTORY_REMOVE,
            json!({"owner": "danny", "item": "rags", "count": 2}),
        )
        .await,
    );
    assert_eq!(
        held(&host, "danny").await,
        [
            ("ochre".to_owned(), 1),
            ("rags".to_owned(), 0),
            ("brush".to_owned(), 2)
        ]
    );

    // A batch is all or none.
    let refused = refusal::<Changed>(
        post(
            &host,
            path::INVENTORY_BATCH,
            json!({"owner": "danny", "changes": [
                {"op": "add", "item": "ochre", "count": 5},
                {"op": "consume", "item": "brush", "count": 3}
            ]}),
        )
        .await,
    );
    assert_eq!(refused.key(), "insufficient");
    assert_eq!(
        held(&host, "danny").await[0],
        ("ochre".to_owned(), 1),
        "nothing was taken"
    );
    let batch: Changed = done(
        post(
            &host,
            path::INVENTORY_BATCH,
            json!({"owner": "danny", "changes": [
                {"op": "add", "item": "ochre", "count": 5, "why": "bought"},
                {"op": "consume", "item": "brush", "count": 1}
            ]}),
        )
        .await,
    );
    assert_eq!(batch.applied.len(), 2);
    assert_eq!(held(&host, "danny").await[0], ("ochre".to_owned(), 6));

    let logged = lines(&host);
    assert!(logged.contains(
        &"WORLD inventory_changed owner=danny item=ochre kind=consume before=3 after=1".to_owned()
    ));
    assert!(logged.contains(
        &"WORLD inventory_changed owner=danny item=ochre kind=add before=1 after=6".to_owned()
    ));
}

#[tokio::test]
async fn every_route_refuses_with_a_typed_error() {
    let host = host().await;
    seed(&host, "danny", "character", &[("ochre", 3)]).await;
    seed(&host, "baker", "shop", &[("loaf", 1)]).await;
    let take =
        |owner: &str, item: &str, count: u64| json!({"owner": owner, "item": item, "count": count});

    let error = |reply: InventoryReply<Changed>| refusal(reply);
    assert_eq!(
        error(post(&host, path::INVENTORY_CONSUME, take("nobody", "ochre", 1)).await),
        InventoryError::UnknownOwner {
            owner: "nobody".to_owned()
        }
    );
    assert_eq!(
        error(post(&host, path::INVENTORY_CONSUME, take("Danny", "ochre", 1)).await).key(),
        "invalid_owner"
    );
    assert_eq!(
        error(post(&host, path::INVENTORY_CONSUME, take("danny", "lute", 1)).await).key(),
        "unknown_item"
    );
    assert_eq!(
        error(post(&host, path::INVENTORY_CONSUME, take("danny", "Ochre", 1)).await).key(),
        "invalid_item"
    );
    assert_eq!(
        error(post(&host, path::INVENTORY_CONSUME, take("danny", "ochre", 0)).await).key(),
        "zero_count"
    );
    assert_eq!(
        error(post(&host, path::INVENTORY_REMOVE, take("danny", "ochre", 4)).await).key(),
        "insufficient"
    );
    assert_eq!(
        error(post(&host, path::INVENTORY_ADD, take("danny", "ochre", u64::MAX),).await).key(),
        "overflow"
    );
    assert_eq!(
        error(post(&host, path::INVENTORY_ADD, take("danny", "brush", 1)).await).key(),
        "missing_words"
    );
    assert_eq!(
        error(
            post(
                &host,
                path::INVENTORY_BATCH,
                json!({"owner": "danny", "changes": []})
            )
            .await
        )
        .key(),
        "empty_batch"
    );
    assert_eq!(
        error(
            post(
                &host,
                path::INVENTORY_BATCH,
                json!({"owner": "danny", "changes": [
                    {"op": "remove", "item": "ochre", "count": 1},
                    {"op": "consume", "item": "ochre", "count": 1}
                ]}),
            )
            .await
        )
        .key(),
        "duplicate_item"
    );
    assert_eq!(
        error(
            post(
                &host,
                path::INVENTORY_SET,
                json!({"owner": "danny", "kind": "character",
                    "items": [item("a", 1), item("a", 2)]}),
            )
            .await
        )
        .key(),
        "duplicate_item"
    );
    assert_eq!(
        error(
            post(
                &host,
                path::INVENTORY_TRANSFER,
                json!({"from": "danny", "to": "danny", "item": "ochre", "count": 1}),
            )
            .await
        )
        .key(),
        "same_owner"
    );
    assert_eq!(
        error(post(&host, path::INVENTORY_REMOVE, json!({"owner": "danny"})).await).key(),
        "invalid_body"
    );
    assert_eq!(
        error(
            post(
                &host,
                path::INVENTORY_REMOVE,
                json!({"owner": "danny", "item": "ochre", "count": -1}),
            )
            .await
        )
        .key(),
        "invalid_body",
        "a count is never negative"
    );
    assert_eq!(
        error(
            post(
                &host,
                path::INVENTORY_REMOVE,
                json!({"owner": "danny", "item": "ochre", "count": 1, "name": "x"}),
            )
            .await
        )
        .key(),
        "invalid_body",
        "words are for add alone"
    );
    assert_eq!(
        refusal(read::<InventoryView>(&host, path::INVENTORY_OWNER).await).key(),
        "invalid_query"
    );
    assert_eq!(
        refusal(owner_view(&host, "nobody").await).key(),
        "unknown_owner"
    );
    assert_eq!(
        refusal(owner_view(&host, "Danny").await).key(),
        "invalid_owner"
    );
    assert_eq!(
        refusal(read::<History>(&host, &format!("{}?owner=Danny", path::INVENTORY_HISTORY)).await)
            .key(),
        "invalid_owner"
    );
    assert_eq!(
        held(&host, "danny").await,
        [("ochre".to_owned(), 3)],
        "refusals changed nothing"
    );
    assert!(
        lines(&host)
            .contains(&"WORLD refused verb=inventory_consume error=unknown_owner".to_owned())
    );
}

#[tokio::test]
async fn a_transfer_across_the_routes_is_atomic() {
    let host = host().await;
    seed(&host, "danny", "character", &[("ochre", 3)]).await;
    seed(&host, "stash", "party", &[("rope", 1)]).await;
    let transfer = |from: &str, to: &str, item: &str, count: u64| {
        let why = "shared";
        json!({"from": from, "to": to, "item": item, "count": count, "why": why})
    };

    let moved: Changed = done(
        post(
            &host,
            path::INVENTORY_TRANSFER,
            transfer("danny", "stash", "ochre", 2),
        )
        .await,
    );
    assert_eq!(moved.owners.len(), 2);
    assert_eq!(moved.owners[0].inventory.owner, "danny", "the giver first");
    assert_eq!(held(&host, "danny").await, [("ochre".to_owned(), 1)]);
    assert_eq!(
        held(&host, "stash").await,
        [("rope".to_owned(), 1), ("ochre".to_owned(), 2)]
    );

    // Every refusal leaves both sides as they were.
    let before = (held(&host, "danny").await, held(&host, "stash").await);
    for (body, key) in [
        (transfer("danny", "stash", "ochre", 2), "insufficient"),
        (transfer("danny", "nobody", "ochre", 1), "unknown_owner"),
        (transfer("nobody", "stash", "ochre", 1), "unknown_owner"),
        (transfer("danny", "stash", "lute", 1), "unknown_item"),
        (transfer("danny", "stash", "ochre", 0), "zero_count"),
    ] {
        assert_eq!(
            refusal::<Changed>(post(&host, path::INVENTORY_TRANSFER, body).await).key(),
            key
        );
        assert_eq!(
            (held(&host, "danny").await, held(&host, "stash").await),
            before
        );
    }
    let logged = lines(&host);
    assert!(
        logged.contains(
            &"WORLD inventory_changed owner=danny item=ochre kind=transfer_out before=3 after=1"
                .to_owned()
        )
    );
    assert!(
        logged.contains(
            &"WORLD inventory_changed owner=stash item=ochre kind=transfer_in before=0 after=2"
                .to_owned()
        )
    );
}

#[cfg(windows)]
#[tokio::test]
async fn a_transfer_that_cannot_write_its_second_side_puts_the_first_back() {
    let host = host().await;
    seed(&host, "danny", "character", &[("ochre", 3)]).await;
    seed(&host, "stash", "party", &[("rope", 1)]).await;
    let stash = host.playground_path("assets/world/inventory/stash.json");
    let mut permissions = std::fs::metadata(&stash).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&stash, permissions).unwrap();

    let refused = refusal::<Changed>(
        post(
            &host,
            path::INVENTORY_TRANSFER,
            json!({"from": "danny", "to": "stash", "item": "ochre", "count": 2}),
        )
        .await,
    );
    assert_eq!(refused.key(), "store_unwritable");
    assert_eq!(
        held(&host, "danny").await,
        [("ochre".to_owned(), 3)],
        "the giver is back"
    );
    assert_eq!(held(&host, "stash").await, [("rope".to_owned(), 1)]);
    let writable = std::fs::metadata(stash.with_file_name("index.json"))
        .unwrap()
        .permissions();
    std::fs::set_permissions(&stash, writable).unwrap();
}

#[tokio::test]
async fn the_history_is_the_hosts_change_lines_for_one_owner_or_all() {
    let host = host().await;
    seed(&host, "danny", "character", &[("ochre", 5)]).await;
    seed(&host, "baker", "shop", &[("loaf", 5)]).await;
    for count in [1, 1, 1] {
        let _: Changed = done(
            post(
                &host,
                path::INVENTORY_CONSUME,
                json!({"owner": "danny", "item": "ochre", "count": count}),
            )
            .await,
        );
    }
    let _: Changed = done(
        post(
            &host,
            path::INVENTORY_CONSUME,
            json!({"owner": "baker", "item": "loaf", "count": 2}),
        )
        .await,
    );
    let lines_of = |history: History| -> Vec<String> {
        history
            .entries
            .into_iter()
            .map(|entry| entry.line)
            .collect()
    };
    let danny =
        done(read::<History>(&host, &format!("{}?owner=danny", path::INVENTORY_HISTORY)).await);
    assert_eq!(danny.entries.len(), 4, "the set and three consumes");
    assert!(
        danny
            .entries
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence)
    );
    let newest = done(
        read::<History>(
            &host,
            &format!("{}?owner=danny&limit=2", path::INVENTORY_HISTORY),
        )
        .await,
    );
    assert_eq!(newest.entries, danny.entries[2..]);
    assert!(
        lines_of(newest)
            .iter()
            .all(|line| line.contains("owner=danny ") && line.contains("kind=consume"))
    );
    let all = done(read::<History>(&host, path::INVENTORY_HISTORY).await);
    assert_eq!(all.entries.len(), 6);
    let nobody =
        done(read::<History>(&host, &format!("{}?owner=dan", path::INVENTORY_HISTORY)).await);
    assert!(nobody.entries.is_empty(), "an owner id is matched whole");
}

#[tokio::test]
async fn any_speaking_character_has_its_own_judged_inventory() {
    use InventoryReason::{Gained, UsedUp};
    let host = host().await;
    seed(&host, "danny", "character", &[("ochre", 3)]).await;
    seed(&host, "baker", "shop", &[("loaf", 4)]).await;
    let taken = take_judged_changes(&host.state, "baker", &[delta("loaf", -1, UsedUp)], &[]);
    assert_eq!(
        (taken[0].owner.as_str(), taken[0].before, taken[0].after),
        ("baker", 4, 3)
    );
    assert_eq!(
        inventory_of(&host.state, "danny")[0].count,
        3,
        "another owner's list is not touched"
    );
    // Each delta is judged on its own: a refused one is one line, and a line with a refused entry
    // changes nothing at all.
    for refused in [
        delta("ochre", -4, UsedUp),
        delta("loaf", 1, Gained),
        delta("ochre", -1, Gained),
    ] {
        assert!(take_judged_changes(&host.state, "danny", &[refused], &[]).is_empty());
    }
    assert_eq!(inventory_of(&host.state, "danny")[0].count, 3);
    let both = take_judged_changes(
        &host.state,
        "danny",
        &[
            delta("lute", -1, UsedUp),
            delta("ochre", -1, UsedUp),
            delta("ochre", -1, UsedUp),
        ],
        &[],
    );
    assert!(both.is_empty(), "the unknown item refuses the whole line");
    assert_eq!(inventory_of(&host.state, "danny")[0].count, 3);
    let twice = take_judged_changes(
        &host.state,
        "danny",
        &[delta("ochre", -1, UsedUp), delta("ochre", -1, UsedUp)],
        &[],
    );
    assert_eq!(twice.len(), 1, "the two entries of one item are one change");
    assert_eq!(inventory_of(&host.state, "danny")[0].count, 1);
    let refused = lines(&host)
        .into_iter()
        .filter(|line| line.starts_with("WORLD refused verb=inventory_judged"))
        .collect::<Vec<_>>();
    assert_eq!(
        refused,
        [
            "WORLD refused verb=inventory_judged error=insufficient",
            "WORLD refused verb=inventory_judged error=missing_words",
            "WORLD refused verb=inventory_judged error=invalid_reason",
            "WORLD refused verb=inventory_judged error=unknown_item",
        ]
    );
    // A character with no inventory, or an empty judgement, is nothing to do.
    assert!(
        take_judged_changes(&host.state, "nobody", &[delta("ochre", -1, UsedUp)], &[]).is_empty()
    );
    assert!(take_judged_changes(&host.state, "danny", &[], &[]).is_empty());
    assert!(inventory_of(&host.state, "nobody").is_empty());
}

#[tokio::test]
async fn a_judged_line_cannot_use_up_a_kept_item_but_may_wear_it_out_and_bring_in_new_ones() {
    use InventoryReason::{Gained, UsedUp, WornOut};
    let host = host().await;
    let kept = |id: &str, count: u64| {
        let look = "as it looks";
        json!({"id": id, "name": id, "look": look, "use": "kept", "count": count})
    };
    let ochre = json!({"id": "ochre", "name": "ochre", "look": "yellow earth", "count": 3});
    let _: Changed = done(
        post(
            &host,
            path::INVENTORY_SET,
            json!({"owner": "danny", "kind": "character",
                "items": [kept("easel", 1), kept("brushes", 5), ochre]}),
        )
        .await,
    );
    // The easel and the brushes are used to paint, and the ochre is used up with them.
    let taken = take_judged_changes(
        &host.state,
        "danny",
        &[
            delta("easel", -1, UsedUp),
            delta("brushes", -1, UsedUp),
            delta("ochre", -1, UsedUp),
        ],
        &[],
    );
    assert!(
        taken.is_empty(),
        "a kept item used up refuses the whole line"
    );
    assert_eq!(
        held(&host, "danny").await,
        [
            ("easel".to_owned(), 1),
            ("brushes".to_owned(), 5),
            ("ochre".to_owned(), 3)
        ]
    );
    let ochre_only = take_judged_changes(&host.state, "danny", &[delta("ochre", -1, UsedUp)], &[]);
    assert_eq!(ochre_only.len(), 1);
    // A brush that wears out, judged so, drops by one.
    let worn = take_judged_changes(&host.state, "danny", &[delta("brushes", -1, WornOut)], &[]);
    assert_eq!((worn[0].before, worn[0].after), (5, 4));

    // Buying something new: the judge gives it an id, a name, a look and its use.
    let new = |id: &str, count: i64| InventoryDelta {
        name: Some("Blue lapis".to_owned()),
        look: Some("deep blue stone, ground fine".to_owned()),
        usage: Some(ItemUse::Consumed),
        source: Some(GainSource::Given),
        giver: Some("baker".to_owned()),
        ..delta(id, count, Gained)
    };
    let baker = ["baker".to_owned()];
    let bought = take_judged_changes(&host.state, "danny", &[new("lapis", 2)], &baker);
    assert_eq!(
        (bought[0].kind, bought[0].before, bought[0].after),
        (ChangeKind::Add, 0, 2)
    );
    let stored = done(owner_view(&host, "danny").await).inventory;
    let lapis = stored.item("lapis").unwrap();
    assert_eq!(
        (lapis.name.as_str(), lapis.usage),
        ("Blue lapis", ItemUse::Consumed)
    );
    // Without its words, or with an id that is not one, it is refused.
    for refused in [delta("saffron", 1, Gained), new("Saffron Gold", 1)] {
        assert!(take_judged_changes(&host.state, "danny", &[refused], &baker).is_empty());
    }
    assert!(stored.item("saffron").is_none());
    let errors: Vec<_> = lines(&host)
        .into_iter()
        .filter(|line| line.starts_with("WORLD refused verb=inventory_judged"))
        .collect();
    assert_eq!(
        errors,
        [
            "WORLD refused verb=inventory_judged error=unreasoned_loss",
            "WORLD refused verb=inventory_judged error=unreasoned_loss",
            "WORLD refused verb=inventory_judged error=missing_words",
            "WORLD refused verb=inventory_judged error=invalid_item",
        ]
    );
    // No count of a new item is too large: only zero and overflow are refused.
    let sack = take_judged_changes(&host.state, "danny", &[new("pigment_sack", 50_000)], &baker);
    assert_eq!(sack[0].after, 50_000);
    // The same refusal on the explicit route, and a kept item added by the route keeps its use.
    assert_eq!(
        refusal::<Changed>(
            post(
                &host,
                path::INVENTORY_CONSUME,
                json!({"owner": "danny", "item": "easel", "count": 1}),
            )
            .await
        )
        .key(),
        "kept_item"
    );
    let _: Changed = done(
        post(
            &host,
            path::INVENTORY_ADD,
            json!({"owner": "danny", "item": "satchel", "count": 1, "name": "satchel",
                "look": "worn leather", "use": "kept"}),
        )
        .await,
    );
    assert_eq!(
        done(owner_view(&host, "danny").await)
            .inventory
            .item("satchel")
            .map(|satchel| satchel.usage),
        Some(ItemUse::Kept)
    );
    assert_eq!(
        refusal::<Changed>(
            post(
                &host,
                path::INVENTORY_ADD,
                json!({"owner": "danny", "item": "twine", "count": 1, "name": "twine",
                    "look": "coarse", "use": "forever"}),
            )
            .await
        )
        .key(),
        "invalid_body"
    );
}

#[tokio::test]
async fn the_stored_shape_is_a_file_for_each_owner_and_an_index() {
    let host = host().await;
    seed(&host, "danny", "character", &[("ochre", 3)]).await;
    seed(&host, "baker", "shop", &[("loaf", 4)]).await;
    let directory = host.playground_path("assets/world/inventory");
    let stored: Value =
        serde_json::from_str(&std::fs::read_to_string(directory.join("danny.json")).unwrap())
            .unwrap();
    assert_eq!(
        stored,
        json!({
            "schema": 1, "owner": "danny", "kind": "character",
            "items": [{"id": "ochre", "name": "ochre", "look": "the ochre as it looks",
                "use": "consumed", "count": 3}]
        })
    );
    let index: Value =
        serde_json::from_str(&std::fs::read_to_string(directory.join("index.json")).unwrap())
            .unwrap();
    assert_eq!(
        index,
        json!({"schema": 1, "owners": [
            {"owner": "baker", "kind": "shop", "items": 1, "units": 4},
            {"owner": "danny", "kind": "character", "items": 1, "units": 3}
        ]})
    );
    // A file edited by hand is read again on the next call.
    std::fs::write(
        directory.join("danny.json"),
        stored.to_string().replace("\"count\":3", "\"count\":9"),
    )
    .unwrap();
    assert_eq!(held(&host, "danny").await, [("ochre".to_owned(), 9)]);
    // A file that is not an inventory is a typed error, never a panic.
    std::fs::write(directory.join("danny.json"), "{\"owner\": 7}").unwrap();
    assert_eq!(
        refusal(owner_view(&host, "danny").await).key(),
        "store_invalid"
    );
    std::fs::write(
        directory.join("danny.json"),
        stored.to_string().replace("\"schema\":1", "\"schema\":2"),
    )
    .unwrap();
    assert_eq!(
        refusal(owner_view(&host, "danny").await).key(),
        "invalid_schema"
    );
    std::fs::write(
        directory.join("danny.json"),
        stored
            .to_string()
            .replace("\"owner\":\"danny\"", "\"owner\":\"baker\""),
    )
    .unwrap();
    assert_eq!(
        refusal(owner_view(&host, "danny").await).key(),
        "invalid_owner"
    );
    // So is an index that is not one, or a path that cannot be read as a file.
    std::fs::write(directory.join("index.json"), "nonsense").unwrap();
    assert_eq!(
        refusal(read::<OwnerList>(&host, path::INVENTORY).await).key(),
        "store_invalid"
    );
    std::fs::remove_file(directory.join("index.json")).unwrap();
    std::fs::create_dir(directory.join("index.json")).unwrap();
    assert_eq!(
        refusal(read::<OwnerList>(&host, path::INVENTORY).await).key(),
        "store_unreadable"
    );
}

#[tokio::test]
async fn many_owners_behave_as_one_does() {
    let host = host().await;
    const OWNERS: usize = 60;
    for number in 0..OWNERS {
        let kind = if number % 10 == 0 {
            "shop"
        } else {
            "character"
        };
        seed(
            &host,
            &format!("npc-{number:02}"),
            kind,
            &[("coin", 10), ("rags", 1)],
        )
        .await;
    }
    let list = done(read::<OwnerList>(&host, path::INVENTORY).await);
    assert_eq!(list.owners.len(), OWNERS);
    assert!(
        list.owners
            .windows(2)
            .all(|pair| pair[0].owner < pair[1].owner)
    );
    let directory = host.playground_path("assets/world/inventory");
    let files = std::fs::read_dir(&directory)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .is_ok_and(|entry| entry.file_name() != "index.json")
        })
        .count();
    assert_eq!(files, OWNERS, "one file for each owner");

    // A change to one owner writes that owner's file and the index, and no other owner's.
    let untouched = std::fs::read_to_string(directory.join("npc-07.json")).unwrap();
    let _: Changed = done(
        post(
            &host,
            path::INVENTORY_CONSUME,
            json!({"owner": "npc-03", "item": "coin", "count": 4}),
        )
        .await,
    );
    let _: Changed = done(
        post(
            &host,
            path::INVENTORY_TRANSFER,
            json!({"from": "npc-03", "to": "npc-10", "item": "coin", "count": 6}),
        )
        .await,
    );
    assert_eq!(
        std::fs::read_to_string(directory.join("npc-07.json")).unwrap(),
        untouched
    );
    assert_eq!(held(&host, "npc-03").await[0], ("coin".to_owned(), 0));
    assert_eq!(held(&host, "npc-10").await[0], ("coin".to_owned(), 16));
    let list = done(read::<OwnerList>(&host, path::INVENTORY).await);
    let units = |owner: &str| {
        list.owners
            .iter()
            .find(|row| row.owner == owner)
            .map(|row| row.units)
    };
    assert_eq!(units("npc-03"), Some(1));
    assert_eq!(units("npc-10"), Some(17));
    assert_eq!(list.owners.len(), OWNERS);

    // Every owner is judged on its own list: each of them spends one coin at once.
    let spend = delta("coin", -1, InventoryReason::UsedUp);
    let handles: Vec<_> = (0..OWNERS)
        .filter(|number| *number != 3)
        .map(|number| {
            let state = std::sync::Arc::clone(&host.state);
            let spend = spend.clone();
            std::thread::spawn(move || {
                take_judged_changes(&state, &format!("npc-{number:02}"), &[spend], &[]).len()
            })
        })
        .collect();
    let taken: usize = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .sum();
    assert_eq!(taken, OWNERS - 1);
    assert_eq!(held(&host, "npc-59").await[0], ("coin".to_owned(), 9));
    assert_eq!(held(&host, "npc-10").await[0], ("coin".to_owned(), 15));
}

#[test]
fn the_verbs_read_their_words() {
    let parse = |line: &str| parse_inventory(&split_words(line).unwrap());
    assert_eq!(parse("").unwrap(), InventoryCommand::List);
    assert_eq!(parse("list").unwrap(), InventoryCommand::List);
    assert_eq!(
        parse("get danny").unwrap(),
        InventoryCommand::Get {
            owner: "danny".to_owned()
        }
    );
    assert!(matches!(
        parse("set danny character items.json").unwrap(),
        InventoryCommand::Set {
            kind: OwnerKind::Character,
            ..
        }
    ));
    assert!(matches!(
        parse("set danny bank items.json"),
        Err(CommandError::BadChoice { .. })
    ));
    let InventoryCommand::Add(add) =
        parse(r#"add danny brush 2 name="a brush" look="splayed bristles" why=bought"#).unwrap()
    else {
        panic!("an add");
    };
    assert_eq!(
        (
            add.count,
            add.name.as_deref(),
            add.look.as_deref(),
            add.why.as_str()
        ),
        (2, Some("a brush"), Some("splayed bristles"), "bought")
    );
    assert!(matches!(
        parse("consume danny ochre 1 why=sky").unwrap(),
        InventoryCommand::Consume(_)
    ));
    assert!(matches!(
        parse("remove danny ochre 1").unwrap(),
        InventoryCommand::Remove(_)
    ));
    assert!(matches!(
        parse("transfer danny stash ochre 2 why=shared").unwrap(),
        InventoryCommand::Transfer(_)
    ));
    assert!(matches!(
        parse("batch danny changes.json").unwrap(),
        InventoryCommand::Batch { .. }
    ));
    assert_eq!(
        parse("history danny limit=5").unwrap(),
        InventoryCommand::History {
            owner: Some("danny".to_owned()),
            limit: Some(5)
        }
    );
    assert_eq!(
        parse("history").unwrap(),
        InventoryCommand::History {
            owner: None,
            limit: None
        }
    );
    assert!(matches!(
        parse("sheets danny").unwrap(),
        InventoryCommand::Sheets { .. }
    ));
    for bad in [
        "consume danny ochre",
        "consume danny ochre many",
        "consume danny ochre 1 name=x",
        "consume danny ochre 1 why=a why=b",
        "transfer danny stash ochre",
        "help",
    ] {
        assert!(parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn every_error_has_its_own_key_a_sentence_and_a_status() {
    use crate::dev_http::BodyError;
    use crate::io_cause::IoCause;
    let errors = [
        InventoryError::InvalidQuery {
            detail: "x".to_owned(),
        },
        InventoryError::InvalidOwner {
            owner: "X".to_owned(),
        },
        InventoryError::InvalidItem {
            item: "X".to_owned(),
        },
        InventoryError::InvalidSchema { schema: 2 },
        InventoryError::UnknownOwner {
            owner: "x".to_owned(),
        },
        InventoryError::UnknownItem {
            owner: "x".to_owned(),
            item: "y".to_owned(),
        },
        InventoryError::DuplicateItem {
            item: "y".to_owned(),
        },
        InventoryError::MissingWords {
            item: "y".to_owned(),
        },
        InventoryError::EmptyBatch,
        InventoryError::ZeroCount {
            item: "y".to_owned(),
        },
        InventoryError::Insufficient {
            owner: "x".to_owned(),
            item: "y".to_owned(),
            have: 1,
            asked: 2,
        },
        InventoryError::InvalidReason {
            item: "y".to_owned(),
        },
        InventoryError::UnsourcedGain {
            item: "y".to_owned(),
        },
        InventoryError::UnreasonedLoss {
            item: "y".to_owned(),
        },
        InventoryError::KeptItem {
            owner: "x".to_owned(),
            item: "y".to_owned(),
        },
        InventoryError::Overflow {
            owner: "x".to_owned(),
            item: "y".to_owned(),
        },
        InventoryError::SameOwner {
            owner: "x".to_owned(),
        },
        InventoryError::StoreUnreadable {
            cause: IoCause::Other,
        },
        InventoryError::StoreInvalid {
            cause: BodyError::from_json(&serde_json::from_str::<u8>("x").unwrap_err()),
        },
        InventoryError::StoreUnwritable {
            cause: IoCause::StorageFull,
        },
        InventoryError::Stopped,
    ];
    let mut keys: Vec<&str> = errors.iter().map(InventoryError::key).collect();
    keys.push(
        InventoryError::InvalidBody(BodyError::from_json(
            &serde_json::from_str::<u8>("x").unwrap_err(),
        ))
        .key(),
    );
    let distinct = keys.len();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), distinct, "no two errors share a key");
    for error in &errors {
        assert!(!error.to_string().is_empty());
        assert_eq!(
            json!(error)["error"],
            error.key(),
            "the key is the serde tag"
        );
        let _ = error.status();
    }
}

fn gain(item: &str, count: i64, source: Option<GainSource>) -> InventoryDelta {
    InventoryDelta {
        name: Some(item.to_owned()),
        look: Some(format!("{item} up close")),
        usage: Some(ItemUse::Consumed),
        source,
        ..delta(item, count, InventoryReason::Gained)
    }
}

fn counted(items: &[(&str, u64)]) -> Vec<ItemCount> {
    items
        .iter()
        .map(|(item, count)| ItemCount {
            item: (*item).to_owned(),
            count: *count,
        })
        .collect()
}

/// A gain by exchange, paying with `paid`.
fn bought(item: &str, count: i64, paid: &[(&str, u64)]) -> InventoryDelta {
    InventoryDelta {
        paid_with: counted(paid),
        ..gain(item, count, Some(GainSource::Exchange))
    }
}

fn spent(item: &str, count: i64) -> InventoryDelta {
    delta(item, -count, InventoryReason::UsedUp)
}

fn purse() -> Inventory {
    inventory(
        "danny",
        OwnerKind::Character,
        &[
            ("coins", 14),
            ("ochre", 3),
            ("boards", 4),
            ("painting", 1),
            ("charcoal", 6),
        ],
    )
    .with_kept("painting")
}

fn judged(deltas: &[InventoryDelta], things: &[&str], present: &[&str]) -> Judged {
    let things: Vec<String> = things.iter().map(|thing| (*thing).to_owned()).collect();
    let present: Vec<String> = present.iter().map(|who| (*who).to_owned()).collect();
    judge_deltas(
        &purse(),
        deltas,
        &GainContext {
            things: &things,
            present: &present,
        },
    )
}

fn unsourced(item: &str) -> InventoryError {
    InventoryError::UnsourcedGain {
        item: item.to_owned(),
    }
}

#[test]
fn a_gain_with_no_source_is_refused_goods_do_not_appear_from_nowhere() {
    // Held or new, with words or without, a gain that names no source is refused.
    let out = judged(&[gain("ochre", 2, None), gain("boards", 1, None)], &[], &[]);
    assert!(out.changes.is_empty());
    assert_eq!(out.refused, [unsourced("ochre"), unsourced("boards")]);
    // The live case: goods taken at a road, one coin down, nothing offered gone.
    let road = judged(
        &[
            spent("coins", 1),
            bought("boards", 1, &[("painting", 1)]),
            bought("ochre", 1, &[("painting", 1)]),
        ],
        &[],
        &[],
    );
    assert!(
        road.changes
            .iter()
            .all(|change| matches!(change, InventoryChange::Consume { .. })),
        "no gain stands without the painting going down: {:?}",
        road.changes
    );
    assert_eq!(road.refused, [unsourced("boards"), unsourced("ochre")]);
}

#[test]
fn an_exchange_with_the_payment_in_the_same_batch_applies_both_sides() {
    let out = judged(
        &[spent("coins", 2), bought("ochre", 2, &[("coins", 2)])],
        &[],
        &[],
    );
    assert!(out.refused.is_empty());
    assert_eq!(out.changes.len(), 2);
    let (after, applied) = apply_changes(&purse(), &out.changes).unwrap();
    assert_eq!(counts(&after)[..2], [("coins", 12), ("ochre", 5)]);
    assert_eq!(applied.len(), 2);
    // Selling a thing is that thing going down for the coin.
    let sold = judged(
        &[
            delta("painting", -1, InventoryReason::Sold),
            bought("coins", 3, &[("painting", 1)]),
        ],
        &[],
        &[],
    );
    assert!(sold.refused.is_empty());
    assert_eq!(sold.changes.len(), 2);
    let (after, _) = apply_changes(&purse(), &sold.changes).unwrap();
    assert_eq!(
        after.item("painting").map(|painting| painting.count),
        Some(0)
    );
    assert_eq!(after.item("coins").map(|coins| coins.count), Some(17));
}

#[test]
fn an_exchange_whose_payment_is_missing_or_short_is_refused_with_its_payment() {
    // No payment in the batch at all.
    let none = judged(&[bought("ochre", 2, &[("coins", 2)])], &[], &[]);
    assert!(none.changes.is_empty());
    assert_eq!(none.refused, [unsourced("ochre")]);
    // A payment short of what the gain names: the gain is refused, and the coin that was to pay for
    // it does not go down either, so neither side applies.
    let short = judged(
        &[spent("coins", 1), bought("ochre", 2, &[("coins", 2)])],
        &[],
        &[],
    );
    assert!(short.changes.is_empty(), "{:?}", short.changes);
    assert_eq!(short.refused, [unsourced("ochre")]);
    // A payment that cannot be made (more coins than held) refuses the payment and the gain.
    let broke = judged(
        &[spent("coins", 99), bought("ochre", 2, &[("coins", 99)])],
        &[],
        &[],
    );
    assert!(broke.changes.is_empty());
    assert_eq!(
        broke
            .refused
            .iter()
            .map(InventoryError::key)
            .collect::<Vec<_>>(),
        ["insufficient", "unsourced_gain"]
    );
    // An exchange that names nothing paid, or a count of zero, has no source.
    let nothing = judged(&[bought("ochre", 1, &[])], &[], &[]);
    let zero = judged(
        &[spent("coins", 1), bought("ochre", 1, &[("coins", 0)])],
        &[],
        &[],
    );
    assert_eq!(nothing.refused, [unsourced("ochre")]);
    assert_eq!(zero.refused, [unsourced("ochre")]);
    // A coin pays for one gain only: the second claim on the same coin is refused.
    let twice = judged(
        &[
            spent("coins", 1),
            bought("ochre", 1, &[("coins", 1)]),
            bought("boards", 1, &[("coins", 1)]),
        ],
        &[],
        &[],
    );
    assert!(
        twice.changes.is_empty(),
        "a deal applies whole or not at all"
    );
    assert_eq!(twice.refused, [unsourced("boards")]);
}

#[test]
fn a_found_gain_needs_a_thing_offered_here() {
    let found = |thing: Option<&str>| InventoryDelta {
        place_thing: thing.map(str::to_owned),
        ..gain("pebble", 1, Some(GainSource::Found))
    };
    let standing = judged(&[found(Some("well"))], &["well", "gate"], &[]);
    assert!(standing.refused.is_empty());
    for (thing, offered) in [
        (Some("cart"), vec!["well"]),
        (Some("well"), vec![]),
        (None, vec!["well"]),
    ] {
        let out = judged(&[found(thing)], &offered, &[]);
        assert!(out.changes.is_empty());
        assert_eq!(out.refused, [unsourced("pebble")]);
    }
}

#[test]
fn a_given_gain_needs_another_character_present() {
    let given = |from: Option<&str>| InventoryDelta {
        giver: from.map(str::to_owned),
        ..gain("loaf", 1, Some(GainSource::Given))
    };
    let standing = judged(&[given(Some("baker"))], &[], &["baker", "danny"]);
    assert!(standing.refused.is_empty());
    for from in [Some("nobody"), Some("danny"), None] {
        let out = judged(&[given(from)], &[], &["baker", "danny"]);
        assert!(out.changes.is_empty());
        assert_eq!(out.refused, [unsourced("loaf")]);
    }
}

#[test]
fn a_made_gain_needs_its_inputs_used_up_in_the_same_batch() {
    let made = |inputs: &[(&str, u64)]| InventoryDelta {
        made_from: counted(inputs),
        ..gain("study", 1, Some(GainSource::Made))
    };
    let ok = judged(
        &[
            spent("boards", 1),
            spent("ochre", 1),
            made(&[("boards", 1), ("ochre", 1)]),
        ],
        &[],
        &[],
    );
    assert!(ok.refused.is_empty());
    let (after, _) = apply_changes(&purse(), &ok.changes).unwrap();
    assert_eq!(after.item("study").map(|study| study.count), Some(1));
    assert_eq!(after.item("boards").map(|boards| boards.count), Some(3));
    // An input that is not used up refuses the gain and the input that was.
    let missing = judged(
        &[spent("boards", 1), made(&[("boards", 1), ("ochre", 1)])],
        &[],
        &[],
    );
    assert!(missing.changes.is_empty());
    assert_eq!(missing.refused, [unsourced("study")]);
}

#[tokio::test]
async fn a_judged_exchange_is_both_sides_or_neither_through_the_host() {
    use crate::world::request::{PlaceSet, WorldReply};
    use crate::world::routes::path as world_path;
    let host = host().await;
    seed(&host, "danny", "character", &[("coins", 14), ("ochre", 3)]).await;
    // Both sides.
    let paid = take_judged_changes(
        &host.state,
        "danny",
        &[spent("coins", 2), bought("ochre", 2, &[("coins", 2)])],
        &[],
    );
    assert_eq!(paid.len(), 2);
    assert_eq!(
        held(&host, "danny").await,
        [("coins".to_owned(), 12), ("ochre".to_owned(), 5)]
    );
    // Neither: the gain has no payment and the coin line is tied to it.
    let none = take_judged_changes(
        &host.state,
        "danny",
        &[spent("coins", 1), bought("ochre", 2, &[("coins", 2)])],
        &[],
    );
    assert!(none.is_empty());
    assert_eq!(
        held(&host, "danny").await,
        [("coins".to_owned(), 12), ("ochre".to_owned(), 5)]
    );
    // A found gain is checked against the things of the active place.
    let find = |thing: &str| InventoryDelta {
        place_thing: Some(thing.to_owned()),
        ..gain("pebble", 1, Some(GainSource::Found))
    };
    assert!(take_judged_changes(&host.state, "danny", &[find("well")], &[]).is_empty());
    let _: WorldReply<PlaceSet> = post_json(
        host.port,
        world_path::WORLD_ARRIVE,
        &json!({"place": {"id": "roam-01", "name": "a lane",
            "things": [{"id": "well", "name": "the old well", "look": "mossy stones"}]}}),
    )
    .await
    .unwrap();
    assert!(take_judged_changes(&host.state, "danny", &[find("cart")], &[]).is_empty());
    assert_eq!(
        take_judged_changes(&host.state, "danny", &[find("well")], &[]).len(),
        1
    );
    let refused: Vec<_> = lines(&host)
        .into_iter()
        .filter(|line| line.starts_with("WORLD refused verb=inventory_judged"))
        .collect();
    assert_eq!(
        refused,
        [
            "WORLD refused verb=inventory_judged error=unsourced_gain",
            "WORLD refused verb=inventory_judged error=unsourced_gain",
            "WORLD refused verb=inventory_judged error=unsourced_gain",
        ]
    );
}

/// A trait for the tests' one-line way to say an item of an inventory is kept.
trait WithKept {
    fn with_kept(self, item: &str) -> Self;
}

impl WithKept for Inventory {
    fn with_kept(mut self, item: &str) -> Self {
        if let Some(held) = self.items.iter_mut().find(|held| held.id == item) {
            held.usage = ItemUse::Kept;
        }
        self
    }
}

fn losing(item: &str, reason: Option<InventoryReason>) -> InventoryDelta {
    InventoryDelta {
        reason,
        what_happened: None,
        ..delta(item, -1, InventoryReason::Lost)
    }
}

fn unreasoned(item: &str) -> InventoryError {
    InventoryError::UnreasonedLoss {
        item: item.to_owned(),
    }
}

#[test]
fn a_kept_item_that_vanishes_with_no_reason_is_refused() {
    use InventoryReason::{Broken, GivenAway, Lost, Sold, UsedUp, WornOut};
    // No reason, a reason that does not fit a kept item, or a reason without what it needs.
    for vanishing in [
        losing("painting", None),
        losing("painting", Some(UsedUp)),
        losing("painting", Some(Lost)),
        InventoryDelta {
            what_happened: Some("   ".to_owned()),
            ..losing("painting", Some(Broken))
        },
        losing("painting", Some(WornOut)),
        losing("painting", Some(GivenAway)),
        InventoryDelta {
            to: Some("stranger".to_owned()),
            ..losing("painting", Some(GivenAway))
        },
        InventoryDelta {
            to: Some("danny".to_owned()),
            ..losing("painting", Some(GivenAway))
        },
        losing("painting", Some(Sold)),
    ] {
        let out = judged(std::slice::from_ref(&vanishing), &[], &["baker", "danny"]);
        assert!(out.changes.is_empty(), "{vanishing:?}");
        assert_eq!(out.refused, [unreasoned("painting")], "{vanishing:?}");
    }
    // A consumed item still needs a reason: used up, or sold with its payment.
    let out = judged(&[losing("ochre", None)], &[], &[]);
    assert_eq!(out.refused, [unreasoned("ochre")]);
}

#[test]
fn a_loss_with_a_reason_that_fits_applies() {
    use InventoryReason::{Broken, GivenAway, Lost, UsedUp, WornOut};
    let said = |what: &str| Some(what.to_owned());
    for (loss, item) in [
        (
            InventoryDelta {
                what_happened: said("it fell in the river"),
                ..losing("painting", Some(Lost))
            },
            "painting",
        ),
        (
            InventoryDelta {
                what_happened: said("the board cracked in the cold"),
                ..losing("boards", Some(Broken))
            },
            "boards",
        ),
        (
            InventoryDelta {
                what_happened: said("worn bald with use"),
                ..losing("painting", Some(WornOut))
            },
            "painting",
        ),
        (
            InventoryDelta {
                to: said("baker"),
                ..losing("painting", Some(GivenAway))
            },
            "painting",
        ),
        (losing("ochre", Some(UsedUp)), "ochre"),
    ] {
        let out = judged(std::slice::from_ref(&loss), &[], &["baker", "danny"]);
        assert!(out.refused.is_empty(), "{loss:?} {:?}", out.refused);
        assert_eq!(out.changes.len(), 1);
        let (after, applied) = apply_changes(&purse(), &out.changes).unwrap();
        assert_eq!(applied[0].before - applied[0].after, 1);
        assert_eq!(
            after.item(item).map(|held| held.count),
            purse().item(item).map(|held| held.count - 1)
        );
    }
}

#[test]
fn a_sale_goes_down_only_with_its_payment_in_the_same_batch() {
    use InventoryReason::Sold;
    // The sale and its payment apply together.
    let both = judged(
        &[
            losing("painting", Some(Sold)),
            bought("coins", 3, &[("painting", 1)]),
        ],
        &[],
        &[],
    );
    assert!(both.refused.is_empty());
    assert_eq!(both.changes.len(), 2);
    let (after, _) = apply_changes(&purse(), &both.changes).unwrap();
    assert_eq!(after.item("painting").map(|held| held.count), Some(0));
    assert_eq!(after.item("coins").map(|held| held.count), Some(17));
    // A payment that names another item, or fewer than are sold, does not pay for the sale, and the
    // gain that leaned on the sale goes with it: neither side applies.
    for payment in [
        bought("coins", 3, &[("ochre", 1)]),
        bought("coins", 3, &[("painting", 0)]),
    ] {
        let out = judged(&[losing("painting", Some(Sold)), payment], &[], &[]);
        assert!(out.changes.is_empty(), "{:?}", out.changes);
        assert_eq!(out.refused.len(), 2);
    }
    let two = InventoryDelta {
        change: -2,
        ..losing("boards", Some(Sold))
    };
    let short = judged(&[two, bought("coins", 3, &[("boards", 1)])], &[], &[]);
    assert!(short.changes.is_empty());
    // A sale whose payment is refused leaves the item where it was.
    let unpaid = judged(
        &[
            losing("painting", Some(Sold)),
            InventoryDelta {
                source: None,
                ..bought("coins", 3, &[("painting", 1)])
            },
        ],
        &[],
        &[],
    );
    assert!(unpaid.changes.is_empty());
}

#[tokio::test]
async fn a_vanishing_painting_changes_nothing_through_the_host_and_is_logged() {
    use InventoryReason::{Lost, Sold};
    let host = host().await;
    let painting = json!({"id": "painting", "name": "painting", "look": "a small study",
        "use": "kept", "count": 1});
    let coins = json!({"id": "coins", "name": "coins", "look": "silver", "count": 5});
    let _: Changed = done(
        post(
            &host,
            path::INVENTORY_SET,
            json!({"owner": "danny", "kind": "character", "items": [painting, coins]}),
        )
        .await,
    );
    assert!(
        take_judged_changes(
            &host.state,
            "danny",
            &[losing("painting", None), losing("painting", Some(Lost))],
            &[],
        )
        .is_empty()
    );
    assert_eq!(held(&host, "danny").await[0], ("painting".to_owned(), 1));
    // Sold with its payment: both sides, atomically.
    let sold = take_judged_changes(
        &host.state,
        "danny",
        &[
            losing("painting", Some(Sold)),
            bought("coins", 4, &[("painting", 1)]),
        ],
        &[],
    );
    assert_eq!(sold.len(), 2);
    assert_eq!(
        held(&host, "danny").await,
        [("painting".to_owned(), 0), ("coins".to_owned(), 9)]
    );
    let refused: Vec<_> = lines(&host)
        .into_iter()
        .filter(|line| line.starts_with("WORLD refused verb=inventory_judged"))
        .collect();
    assert_eq!(
        refused,
        [
            "WORLD refused verb=inventory_judged error=unreasoned_loss",
            "WORLD refused verb=inventory_judged error=unreasoned_loss",
        ]
    );
}

fn poor_purse() -> Inventory {
    inventory(
        "danny",
        OwnerKind::Character,
        &[("coins", 2), ("painting", 1), ("charcoal", 6)],
    )
    .with_kept("painting")
}

/// A wall study sold for `sale` coins, and `price` coins paid for five bundles of charcoal, in one
/// line: the coin appears twice, once up and once down.
fn sale_and_purchase(sale: i64, price: u64) -> Vec<InventoryDelta> {
    vec![
        losing("painting", Some(InventoryReason::Sold)),
        bought("coins", sale, &[("painting", 1)]),
        spent("coins", i64::try_from(price).unwrap()),
        bought("charcoal", 5, &[("coins", price)]),
    ]
}

#[test]
fn a_sale_and_a_purchase_in_one_line_net_the_coin_that_rises_and_falls() {
    // Five coins up for the sale, eight down for the charcoal: the coin is three down in all.
    let out = judged(&sale_and_purchase(5, 8), &[], &[]);
    assert!(out.refused.is_empty(), "{:?}", out.refused);
    assert_eq!(out.changes.len(), 3, "one change for each item");
    let (after, applied) = apply_changes(&purse(), &out.changes).unwrap();
    assert_eq!(after.item("coins").map(|held| held.count), Some(11));
    assert_eq!(after.item("painting").map(|held| held.count), Some(0));
    assert_eq!(after.item("charcoal").map(|held| held.count), Some(11));
    assert_eq!(applied.len(), 3);
    // The same amount up and down cancels: the coin is not changed at all.
    let even = judged(&sale_and_purchase(8, 8), &[], &[]);
    assert!(even.refused.is_empty());
    assert_eq!(even.changes.len(), 2, "the painting and the charcoal");
    let (after, _) = apply_changes(&purse(), &even.changes).unwrap();
    assert_eq!(after.item("coins").map(|held| held.count), Some(14));
}

#[test]
fn a_sale_and_a_purchase_whose_net_is_impossible_change_nothing() {
    let things: [String; 0] = [];
    let judged_poor = |deltas: &[InventoryDelta]| {
        judge_deltas(
            &poor_purse(),
            deltas,
            &GainContext {
                things: &things,
                present: &[],
            },
        )
    };
    // The coin held, two, is not enough for a net of three down: the coin is refused, and the
    // purchase and the sale that leaned on it with it.
    let out = judged_poor(&sale_and_purchase(5, 8));
    assert!(out.changes.is_empty(), "{:?}", out.changes);
    assert_eq!(
        out.refused,
        [
            InventoryError::Insufficient {
                owner: "danny".to_owned(),
                item: "coins".to_owned(),
                have: 2,
                asked: 3
            },
            unsourced("charcoal"),
            unreasoned("painting"),
        ]
    );
    // Enough proceeds make the same purchase possible from the same purse.
    let funded = judged_poor(&sale_and_purchase(8, 8));
    assert!(funded.refused.is_empty(), "{:?}", funded.refused);
}

#[test]
fn entries_of_one_item_in_the_same_direction_merge_on_the_judged_path_only() {
    use InventoryReason::UsedUp;
    let merged = judged(
        &[delta("ochre", -1, UsedUp), delta("ochre", -2, UsedUp)],
        &[],
        &[],
    );
    assert!(merged.refused.is_empty());
    assert!(matches!(
        merged.changes[..],
        [InventoryChange::Consume { count: 3, .. }]
    ));
    let gains = judged(
        &[
            spent("coins", 2),
            bought("ochre", 1, &[("coins", 1)]),
            bought("ochre", 1, &[("coins", 1)]),
        ],
        &[],
        &[],
    );
    assert!(gains.refused.is_empty(), "{:?}", gains.refused);
    assert!(
        gains
            .changes
            .iter()
            .any(|change| matches!(change, InventoryChange::Add { count: 2, .. }))
    );
    // The explicit batch is operator input and still refuses an item twice.
    assert_eq!(
        apply_changes(&purse(), &[consume("ochre", 1), consume("ochre", 2)]).unwrap_err(),
        InventoryError::DuplicateItem {
            item: "ochre".to_owned()
        }
    );
}

#[tokio::test]
async fn a_sale_and_a_purchase_in_one_line_apply_through_the_host() {
    let host = host().await;
    let painting = json!({"id": "painting", "name": "painting", "look": "a small study",
        "use": "kept", "count": 1});
    let coins = json!({"id": "coins", "name": "coins", "look": "silver", "count": 14});
    let charcoal = json!({"id": "charcoal", "name": "charcoal", "look": "sticks", "count": 6});
    let _: Changed = done(
        post(
            &host,
            path::INVENTORY_SET,
            json!({"owner": "danny", "kind": "character", "items": [painting, coins, charcoal]}),
        )
        .await,
    );
    let taken = take_judged_changes(&host.state, "danny", &sale_and_purchase(5, 8), &[]);
    assert_eq!(taken.len(), 3);
    assert_eq!(
        held(&host, "danny").await,
        [
            ("painting".to_owned(), 0),
            ("coins".to_owned(), 11),
            ("charcoal".to_owned(), 11)
        ]
    );
    assert!(
        lines(&host).contains(
            &"WORLD inventory_changed owner=danny item=coins kind=consume before=14 after=11"
                .to_owned()
        )
    );
}

/// The entries of "I'll sell you my study for five coins, and I'll take five apples for them" as a
/// model may fill them: the sale's coin gain has no payment named, or the wrong one.
fn sale_and_apples(proceeds_paid_with: &[(&str, u64)]) -> Vec<InventoryDelta> {
    vec![
        InventoryDelta {
            to: Some("baker".to_owned()),
            ..losing("painting", Some(InventoryReason::Sold))
        },
        bought("coins", 5, proceeds_paid_with),
        spent("coins", 5),
        bought("apples", 5, &[("coins", 5)]),
    ]
}

#[test]
fn a_sale_whose_proceeds_name_no_payment_is_paired_with_it_and_applies_fully() {
    // The coin gain names nothing, or names the wrong thing: the sold item is paired with it.
    for named in [vec![], vec![("ochre", 1)], vec![("coins", 5)]] {
        let out = judged(&sale_and_apples(&named), &[], &["baker"]);
        assert!(out.refused.is_empty(), "{named:?} {:?}", out.refused);
        assert_eq!(out.repaired, ["painting"]);
        assert_eq!(
            out.changes.len(),
            2,
            "painting down, apples up, coins net zero"
        );
        let (after, _) = apply_changes(&purse(), &out.changes).unwrap();
        assert_eq!(after.item("painting").map(|held| held.count), Some(0));
        assert_eq!(after.item("coins").map(|held| held.count), Some(14));
        assert_eq!(after.item("apples").map(|held| held.count), Some(5));
    }
    // Already right: nothing to repair.
    let right = judged(&sale_and_apples(&[("painting", 1)]), &[], &["baker"]);
    assert!(right.refused.is_empty());
    assert!(right.repaired.is_empty());
}

#[test]
fn a_barter_with_no_coins_applies() {
    use InventoryReason::Sold;
    let out = judged(
        &[
            losing("painting", Some(Sold)),
            bought("apples", 5, &[("painting", 1)]),
        ],
        &[],
        &[],
    );
    assert!(out.refused.is_empty(), "{:?}", out.refused);
    assert!(out.repaired.is_empty());
    let (after, _) = apply_changes(&purse(), &out.changes).unwrap();
    assert_eq!(after.item("painting").map(|held| held.count), Some(0));
    assert_eq!(after.item("apples").map(|held| held.count), Some(5));
    assert_eq!(after.item("coins").map(|held| held.count), Some(14));
}

#[test]
fn a_sale_with_no_counterpart_in_the_line_is_still_refused() {
    use InventoryReason::Sold;
    // Nothing to pair the sold painting with: it is refused, and the purchase of the same deal
    // does not apply half of the line.
    let out = judged(
        &[
            losing("painting", Some(Sold)),
            spent("coins", 5),
            bought("apples", 5, &[("coins", 5)]),
        ],
        &[],
        &[],
    );
    assert!(out.repaired.is_empty());
    assert_eq!(out.refused, [unreasoned("painting")]);
    assert!(out.changes.is_empty(), "{:?}", out.changes);
    // Two gains that could be the proceeds: code does not guess between them.
    let two = judged(
        &[
            losing("painting", Some(Sold)),
            bought("coins", 3, &[]),
            bought("ochre", 1, &[]),
            spent("boards", 1),
            bought("apples", 5, &[("coins", 3), ("ochre", 1)]),
        ],
        &[],
        &[],
    );
    assert!(two.repaired.is_empty());
    assert!(two.refused.contains(&unreasoned("painting")));
}

/// The entries of run 8, a sale for five silver pennies and five sticks of charcoal bought with
/// them, as the model gave them: the sale's payment names the sold item with the price as its
/// count.
fn run_eight_line() -> Vec<InventoryDelta> {
    vec![
        InventoryDelta {
            to: Some("seller".to_owned()),
            ..losing("painting", Some(InventoryReason::Sold))
        },
        bought("coins", 5, &[("painting", 5)]),
        spent("coins", 5),
        bought("charcoal", 5, &[("coins", 5)]),
    ]
}

#[test]
fn a_sale_paid_with_the_price_as_its_count_applies_fully() {
    let out = judged(&run_eight_line(), &[], &["seller"]);
    assert!(out.refused.is_empty(), "{:?}", out.refused);
    assert_eq!(out.repaired, ["painting"]);
    let (after, _) = apply_changes(&purse(), &out.changes).unwrap();
    assert_eq!(after.item("painting").map(|held| held.count), Some(0));
    assert_eq!(after.item("coins").map(|held| held.count), Some(14));
    assert_eq!(after.item("charcoal").map(|held| held.count), Some(11));
}

#[test]
fn a_deal_that_cannot_apply_fully_applies_nothing() {
    // The purchase cannot be paid (this purse holds two coins): the sale and the proceeds do not
    // apply without it.
    let things: [String; 0] = [];
    let poor = judge_deltas(
        &poor_purse(),
        &sale_and_purchase(1, 9),
        &GainContext {
            things: &things,
            present: &[],
        },
    );
    assert!(poor.changes.is_empty(), "{:?}", poor.changes);
    assert!(!poor.refused.is_empty());
    let broke = judged(&sale_and_purchase(5, 99), &[], &[]);
    assert!(broke.changes.is_empty(), "{:?}", broke.changes);
    // An entry of an item that takes no part in the deal is refused with it: the line is whole.
    let mut line = sale_and_purchase(5, 99);
    line.push(spent("ochre", 1));
    let mixed = judged(&line, &[], &[]);
    assert!(mixed.changes.is_empty(), "{:?}", mixed.changes);
}

#[test]
fn the_plain_deals_apply_with_correct_counts_or_refuse_whole() {
    use InventoryReason::Sold;
    let applied = |out: &Judged| {
        assert!(out.refused.is_empty(), "{:?}", out.refused);
        apply_changes(&purse(), &out.changes).unwrap().0
    };
    let count = |after: &Inventory, id: &str| after.item(id).map(|held| held.count);
    // A pure sale for coins.
    let sale = judged(
        &[
            losing("painting", Some(Sold)),
            bought("coins", 5, &[("painting", 1)]),
        ],
        &[],
        &[],
    );
    let after = applied(&sale);
    assert_eq!(
        (count(&after, "painting"), count(&after, "coins")),
        (Some(0), Some(19))
    );
    // A pure purchase with coins.
    let purchase = judged(
        &[spent("coins", 4), bought("charcoal", 2, &[("coins", 4)])],
        &[],
        &[],
    );
    let after = applied(&purchase);
    assert_eq!(
        (count(&after, "coins"), count(&after, "charcoal")),
        (Some(10), Some(8))
    );
    // A swap of one item for another.
    let swap = judged(
        &[
            losing("painting", Some(Sold)),
            bought("boards", 2, &[("painting", 1)]),
        ],
        &[],
        &[],
    );
    let after = applied(&swap);
    assert_eq!(
        (count(&after, "painting"), count(&after, "boards")),
        (Some(0), Some(6))
    );
    // A sale and a purchase with its proceeds, the sale's payment naming a larger count.
    let both = judged(&run_eight_line(), &[], &["seller"]);
    assert_eq!(count(&applied(&both), "charcoal"), Some(11));
    // The same deals with an item not held, too few of it, or too few coins refuse whole.
    for line in [
        vec![
            delta("sword", -1, Sold),
            bought("coins", 5, &[("sword", 1)]),
        ],
        vec![
            InventoryDelta {
                change: -3,
                ..losing("painting", Some(Sold))
            },
            bought("coins", 5, &[("painting", 3)]),
        ],
        vec![spent("coins", 40), bought("charcoal", 2, &[("coins", 40)])],
    ] {
        let out = judged(&line, &[], &[]);
        assert!(out.changes.is_empty(), "{line:?} {:?}", out.changes);
        assert!(!out.refused.is_empty());
    }
}

#[tokio::test]
async fn the_run_eight_line_applies_through_the_host_with_no_half_applied_deal() {
    let host = host().await;
    let painting = json!({"id": "painting", "name": "painting", "look": "a small study",
        "use": "kept", "count": 1});
    let coins = json!({"id": "coins", "name": "coins", "look": "silver", "count": 20});
    let charcoal = json!({"id": "charcoal", "name": "charcoal", "look": "sticks", "count": 6});
    let _: Changed = done(
        post(
            &host,
            path::INVENTORY_SET,
            json!({"owner": "danny", "kind": "character", "items": [painting, coins, charcoal]}),
        )
        .await,
    );
    let seller = ["seller".to_owned()];
    let taken = take_judged_changes(&host.state, "danny", &run_eight_line(), &seller);
    assert_eq!(
        taken.len(),
        2,
        "painting down, charcoal up, coins untouched"
    );
    assert_eq!(
        held(&host, "danny").await,
        [
            ("painting".to_owned(), 0),
            ("coins".to_owned(), 20),
            ("charcoal".to_owned(), 11)
        ]
    );
    // A deal that cannot apply fully applies nothing and says why.
    let broke = [spent("coins", 99), bought("charcoal", 5, &[("coins", 99)])];
    assert!(take_judged_changes(&host.state, "danny", &broke, &[]).is_empty());
    assert_eq!(held(&host, "danny").await[1], ("coins".to_owned(), 20));
    assert!(lines(&host).iter().any(|line| {
        line.starts_with("WORLD judged_entries owner=danny")
            && line.contains("refused=insufficient")
    }));
}

/// The entries of run 9, three coins for the study and a pigment and two boards for them, with
/// `pigment` and `boards` the payments the two goods name.
fn run_nine_line(pigment: u64, boards: u64) -> Vec<InventoryDelta> {
    vec![
        losing("painting", Some(InventoryReason::Sold)),
        bought("coins", 3, &[("painting", 1)]),
        spent("coins", 3),
        bought("ochre", 1, &[("coins", pigment)]),
        bought("boards", 2, &[("coins", boards)]),
    ]
}

#[test]
fn one_payment_split_between_two_goods_applies_and_named_in_full_for_each_applies_nothing() {
    let split = judged(&run_nine_line(2, 1), &[], &[]);
    assert!(split.refused.is_empty(), "{:?}", split.refused);
    let (after, _) = apply_changes(&purse(), &split.changes).unwrap();
    let count = |id: &str| after.item(id).map(|held| held.count);
    assert_eq!(
        (
            count("painting"),
            count("coins"),
            count("ochre"),
            count("boards")
        ),
        (Some(0), Some(14), Some(4), Some(6))
    );
    // Six coins named for three that go down: the second gain is refused, and the deal with it.
    let twice = judged(&run_nine_line(3, 3), &[], &[]);
    assert_eq!(twice.refused, [unsourced("boards")]);
    assert!(twice.changes.is_empty(), "{:?}", twice.changes);
}

/// What the painter held in run 9 move 16: ochre and lamp black, and none of the rest.
fn painters_shelf() -> Inventory {
    inventory(
        "danny",
        OwnerKind::Character,
        &[
            ("boards", 0),
            ("ochre", 1),
            ("lamp_black", 2),
            ("linseed_oil", 0),
            ("rags", 0),
        ],
    )
}

#[test]
fn a_line_of_uses_with_one_short_item_applies_none_of_them() {
    let things: [String; 0] = [];
    let judge = |uses: &[(&str, u64)]| {
        let deltas: Vec<InventoryDelta> = uses
            .iter()
            .map(|(item, count)| spent(item, i64::try_from(*count).unwrap()))
            .collect();
        judge_deltas(
            &painters_shelf(),
            &deltas,
            &GainContext {
                things: &things,
                present: &[],
            },
        )
    };
    // The five uses of the line: a board, ochre, lamp black, oil and a rag.
    let all = judge(&[
        ("boards", 1),
        ("ochre", 1),
        ("lamp_black", 1),
        ("linseed_oil", 1),
        ("rags", 1),
    ]);
    assert_eq!(all.refused.len(), 3, "{:?}", all.refused);
    assert!(all.refused.iter().all(|error| matches!(
        error,
        InventoryError::Insufficient {
            have: 0,
            asked: 1,
            ..
        }
    )));
    assert!(
        all.changes.is_empty(),
        "no pigment is spent: {:?}",
        all.changes
    );
    // What the painter really holds applies, whole.
    let held = judge(&[("ochre", 1), ("lamp_black", 1)]);
    assert!(held.refused.is_empty());
    let (after, _) = apply_changes(&painters_shelf(), &held.changes).unwrap();
    assert_eq!(after.item("ochre").map(|item| item.count), Some(0));
    assert_eq!(after.item("lamp_black").map(|item| item.count), Some(1));
}

#[tokio::test]
async fn the_painted_line_of_run_nine_changes_nothing_through_the_host() {
    let host = host().await;
    let items: Vec<Value> = painters_shelf()
        .items
        .iter()
        .map(|item| {
            json!({"id": item.id, "name": item.name, "look": item.look, "count": item.count})
        })
        .collect();
    let _: Changed = done(
        post(
            &host,
            path::INVENTORY_SET,
            json!({"owner": "danny", "kind": "character", "items": items}),
        )
        .await,
    );
    let uses = ["boards", "ochre", "lamp_black", "linseed_oil", "rags"].map(|item| spent(item, 1));
    assert!(take_judged_changes(&host.state, "danny", &uses, &[]).is_empty());
    assert_eq!(
        held(&host, "danny").await,
        [
            ("boards".to_owned(), 0),
            ("ochre".to_owned(), 1),
            ("lamp_black".to_owned(), 2),
            ("linseed_oil".to_owned(), 0),
            ("rags".to_owned(), 0)
        ]
    );
}

#[test]
fn the_entries_token_is_ids_and_numbers_only_and_bounded() {
    let mut deal = sale_and_apples(&[]);
    deal[0].why = "free text with spaces".to_owned();
    deal[1].item = "Not An Id!".to_owned();
    let token = entries_token(&deal);
    assert_eq!(
        token,
        "painting:-1:sold:-:to=baker;?:+5:gained:exchange;coins:-5:used_up:-;\
         apples:+5:gained:exchange:pay=coins*5"
    );
    assert!(!token.contains(' '));
    let many = vec![delta("ochre", -1, InventoryReason::UsedUp); 45];
    assert!(entries_token(&many).ends_with(";+5"));
    assert_eq!(entries_token(&[]), "-");
}

#[tokio::test]
async fn a_judged_line_that_is_refused_or_repaired_logs_its_entries() {
    let host = host().await;
    let painting = json!({"id": "painting", "name": "painting", "look": "a small study",
        "use": "kept", "count": 1});
    let coins = json!({"id": "coins", "name": "coins", "look": "silver", "count": 20});
    let _: Changed = done(
        post(
            &host,
            path::INVENTORY_SET,
            json!({"owner": "danny", "kind": "character", "items": [painting, coins]}),
        )
        .await,
    );
    let baker = ["baker".to_owned()];
    // The run 7 shape applies fully, and says it was paired.
    let taken = take_judged_changes(&host.state, "danny", &sale_and_apples(&[]), &baker);
    assert_eq!(taken.len(), 2);
    assert_eq!(
        held(&host, "danny").await,
        [
            ("painting".to_owned(), 0),
            ("coins".to_owned(), 20),
            ("apples".to_owned(), 5)
        ]
    );
    // A refusal logs the entries and the keys.
    let none = take_judged_changes(
        &host.state,
        "danny",
        &[losing("coins", Some(InventoryReason::Lost))],
        &baker,
    );
    assert!(none.is_empty());
    let logged: Vec<String> = lines(&host)
        .into_iter()
        .filter(|line| line.starts_with("WORLD judged_entries"))
        .collect();
    assert_eq!(logged.len(), 2);
    assert!(logged[0].contains("owner=danny entries=painting:-1:sold:-:to=baker;"));
    assert!(logged[0].ends_with("refused=- repaired=painting"));
    assert!(logged[1].ends_with("entries=coins:-1:lost:- refused=unreasoned_loss repaired=-"));
}
