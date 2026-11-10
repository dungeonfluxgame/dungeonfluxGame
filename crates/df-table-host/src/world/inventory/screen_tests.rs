//! The inventory on the shared screen: the panel built from an inventory and its changes (the
//! rows, their order, the grouping of a numbered family, the rows kept at zero, the moves), and the
//! host's routes and the judged path that keep it up to date.

use df_playground::inventory::{InventoryPanel, MoveKind, RowUse};
use serde_json::{Value, json};

use super::request::{Changed, InventoryReply};
use super::routes::path;
use super::screen::{Basis, Screen, panel_of};
use super::{
    ChangeKind, InventoryApplied, InventoryDelta, InventoryItem, ItemUse, follow_judged,
    take_judged_changes,
};
use crate::dev_client::post_json;
use crate::dev_http::DevReply;
use crate::test_support::{Running, STARTER_FILES, start};

const FIXTURE: &str = r#"{
    "name": "scene",
    "tv": {"place": "Here", "status": {"kind": "ok"}, "elements": []},
    "phone": {"hero": "Pip", "status": {"kind": "ok"}}
}"#;

fn item(id: &str, count: u64, usage: ItemUse) -> InventoryItem {
    InventoryItem {
        id: id.to_owned(),
        name: id.replace('_', " "),
        look: format!("the {id} as it looks"),
        usage,
        count,
    }
}

fn supply(id: &str, count: u64) -> InventoryItem {
    item(id, count, ItemUse::Consumed)
}

fn tool(id: &str, count: u64) -> InventoryItem {
    item(id, count, ItemUse::Kept)
}

fn basis(serial: u64) -> Basis<'static> {
    Basis {
        owner: "danny",
        title: "Danny",
        serial,
    }
}

fn taken(item: &str, kind: ChangeKind, before: u64, after: u64) -> InventoryApplied {
    InventoryApplied {
        owner: "danny".to_owned(),
        item: item.to_owned(),
        kind,
        before,
        after,
        why: String::new(),
    }
}

fn ids(panel: &InventoryPanel) -> Vec<&str> {
    panel.rows.iter().map(|row| row.id.as_str()).collect()
}

fn counts(panel: &InventoryPanel) -> Vec<(&str, u64)> {
    panel
        .rows
        .iter()
        .map(|row| (row.id.as_str(), row.count))
        .collect()
}

#[test]
fn the_rows_are_the_held_items_supplies_first_then_the_kept_things() {
    let items = [
        tool("easel", 1),
        supply("boards", 6),
        supply("ochre", 0),
        tool("brushes", 5),
        supply("coins", 20),
    ];
    let panel = panel_of(&items, &basis(10), None, &[]);
    assert_eq!(
        ids(&panel),
        ["boards", "coins", "easel", "brushes"],
        "supplies in the order held, then the kept things; an item never held here is not listed"
    );
    assert_eq!(panel.owner, "danny");
    assert_eq!(panel.title, "Danny");
    assert_eq!(panel.serial, 10);
    assert!(panel.moves.is_empty());
    assert_eq!(panel.check(), Ok(()));
    assert_eq!(panel.rows[0].usage, RowUse::Consumed);
    assert_eq!(panel.rows[2].usage, RowUse::Kept);
}

#[test]
fn a_supply_used_up_stays_listed_at_zero_and_a_never_held_one_does_not_appear() {
    let before = [supply("boards", 1), supply("ochre", 0), supply("rags", 4)];
    let shown = panel_of(&before, &basis(1), None, &[]);
    assert_eq!(counts(&shown), [("boards", 1), ("rags", 4)]);
    let after = [supply("boards", 0), supply("ochre", 0), supply("rags", 4)];
    let used = taken("boards", ChangeKind::Consume, 1, 0);
    let next = panel_of(&after, &basis(2), Some(&shown), &[used]);
    assert_eq!(
        counts(&next),
        [("boards", 0), ("rags", 4)],
        "the board that ran out stays, struck by the page; the ochre never held is not listed"
    );
    // Later applications keep it for the rest of the scene.
    let later = [supply("boards", 0), supply("ochre", 0), supply("rags", 3)];
    let again = panel_of(
        &later,
        &basis(3),
        Some(&next),
        &[taken("rags", ChangeKind::Consume, 4, 3)],
    );
    assert_eq!(counts(&again), [("boards", 0), ("rags", 3)]);
}

#[test]
fn the_first_application_after_a_show_lists_what_ran_out_in_it() {
    let after = [supply("boards", 0), supply("rags", 4)];
    let panel = panel_of(
        &after,
        &basis(2),
        None,
        &[taken("boards", ChangeKind::Consume, 1, 0)],
    );
    assert_eq!(counts(&panel), [("boards", 0), ("rags", 4)]);
}

#[test]
fn a_move_carries_the_count_before_and_after_and_the_kind_of_the_change() {
    let items = [supply("ochre", 2), supply("coins", 14), tool("brushes", 4)];
    let applied = [
        taken("ochre", ChangeKind::Consume, 3, 2),
        taken("coins", ChangeKind::Add, 11, 14),
        taken("brushes", ChangeKind::Remove, 5, 4),
    ];
    let panel = panel_of(&items, &basis(4), None, &applied);
    let moves: Vec<(&str, u64, u64, MoveKind)> = panel
        .moves
        .iter()
        .map(|moved| (moved.row.as_str(), moved.before, moved.after, moved.kind))
        .collect();
    assert_eq!(
        moves,
        [
            ("ochre", 3, 2, MoveKind::Consume),
            ("coins", 11, 14, MoveKind::Add),
            ("brushes", 5, 4, MoveKind::Remove),
        ]
    );
    assert_eq!(panel.check(), Ok(()));
}

#[test]
fn the_changes_of_another_owner_move_no_row() {
    let items = [supply("ochre", 2)];
    let mut other = taken("ochre", ChangeKind::TransferIn, 1, 2);
    other.owner = "stash".to_owned();
    let panel = panel_of(&items, &basis(1), None, &[other]);
    assert!(panel.moves.is_empty());
}

#[test]
fn a_numbered_family_of_two_or_more_collapses_into_one_row_listed_first_that_counts_them_all() {
    let mut items = vec![supply("ochre", 3), tool("easel", 1), tool("painting_01", 1)];
    let one = panel_of(&items, &basis(1), None, &[]);
    assert_eq!(
        ids(&one),
        ["ochre", "easel", "painting_01"],
        "one is not a family"
    );
    items.push(tool("painting_02", 1));
    let two = panel_of(&items, &basis(2), None, &[]);
    assert_eq!(ids(&two), ["painting_*", "ochre", "easel"]);
    let family = &two.rows[0];
    assert_eq!((family.name.as_str(), family.count), ("Paintings", 2));
    assert_eq!(family.usage, RowUse::Kept);
    items.extend((3..=7).map(|number| tool(&format!("painting_{number:02}"), 1)));
    let many = panel_of(&items, &basis(3), None, &[]);
    assert_eq!(
        counts(&many),
        [("painting_*", 7), ("ochre", 3), ("easel", 1)]
    );
}

#[test]
fn a_family_row_is_named_from_its_stem_in_the_plural() {
    let items: Vec<InventoryItem> = (1..=5)
        .map(|number| tool(&format!("wax_seal-{number}"), 1))
        .collect();
    let panel = panel_of(&items, &basis(1), None, &[]);
    assert_eq!(ids(&panel), ["wax_seal_*"]);
    assert_eq!(panel.rows[0].name, "Wax seals");
    let plural = [
        tool("canvas_1", 1),
        tool("canvas_2", 1),
        tool("canvas_3", 1),
        tool("canvas_4", 1),
    ];
    assert_eq!(
        panel_of(&plural, &basis(1), None, &[]).rows[0].name,
        "Canvas"
    );
}

#[test]
fn an_item_sold_from_a_family_moves_the_family_row_and_it_stays_one_row() {
    let mut items: Vec<InventoryItem> = (1..=5)
        .map(|number| tool(&format!("painting_{number:02}"), 1))
        .collect();
    let shown = panel_of(&items, &basis(1), None, &[]);
    assert_eq!(counts(&shown), [("painting_*", 5)]);
    items[1].count = 0;
    items[3].count = 0;
    let sold = [
        taken("painting_02", ChangeKind::Remove, 1, 0),
        taken("painting_04", ChangeKind::Remove, 1, 0),
    ];
    let next = panel_of(&items, &basis(2), Some(&shown), &sold);
    assert_eq!(
        counts(&next),
        [("painting_*", 3)],
        "down to three, the family stays one row"
    );
    assert_eq!(next.moves.len(), 1);
    let moved = &next.moves[0];
    assert_eq!(
        (moved.row.as_str(), moved.before, moved.after),
        ("painting_*", 5, 3)
    );
    assert_eq!(next.check(), Ok(()));
}

#[test]
fn a_gain_in_a_family_flashes_the_family_row() {
    let items: Vec<InventoryItem> = (1..=5)
        .map(|number| tool(&format!("painting_{number:02}"), 1))
        .collect();
    let gained = [taken("painting_05", ChangeKind::Add, 0, 1)];
    let panel = panel_of(&items, &basis(1), None, &gained);
    assert_eq!(panel.moves.len(), 1);
    assert_eq!(
        (
            panel.moves[0].before,
            panel.moves[0].after,
            panel.moves[0].kind
        ),
        (4, 5, MoveKind::Add)
    );
}

#[test]
fn fifteen_things_and_two_paintings_fit_as_sixteen_rows_with_the_paintings_first() {
    let mut items: Vec<InventoryItem> = (0..15)
        .map(|number| supply(&format!("thing{}", char::from(b'a' + number)), 2))
        .collect();
    items.push(tool("painting_01", 1));
    items.push(tool("painting_02", 1));
    let panel = panel_of(&items, &basis(1), None, &[]);
    assert_eq!(panel.rows.len(), 16);
    assert_eq!(
        (panel.rows[0].id.as_str(), panel.rows[0].count),
        ("painting_*", 2)
    );
    assert!(panel.rows.iter().all(|row| row.id != "others_*"));
}

#[test]
fn rows_past_the_screens_room_fold_the_lowest_counts_first_never_a_family_or_a_row_at_zero() {
    let mut items: Vec<InventoryItem> = (0..18)
        .map(|number| {
            supply(
                &format!("thing{}", char::from(b'a' + number)),
                5 + u64::from(number),
            )
        })
        .collect();
    items[0].count = 1;
    items[1].count = 2;
    items[2].count = 0;
    items.push(tool("painting_01", 1));
    items.push(tool("painting_02", 1));
    let before = panel_of(&items[..2], &basis(1), None, &[]);
    assert_eq!(before.rows.len(), 2);
    let used = taken("thingc", ChangeKind::Consume, 1, 0);
    let panel = panel_of(&items, &basis(2), Some(&before), &[used]);
    assert_eq!(panel.rows.len(), 16);
    assert_eq!(panel.rows[0].id, "painting_*", "the family is first");
    let last = panel.rows.last().expect("a last row");
    assert_eq!(
        (last.id.as_str(), last.name.as_str()),
        ("others_*", "Other items")
    );
    assert!(
        panel
            .rows
            .iter()
            .any(|row| row.id == "thingc" && row.count == 0),
        "the supply that ran out stays listed"
    );
    assert!(
        panel
            .rows
            .iter()
            .all(|row| row.id != "thinga" && row.id != "thingb"),
        "the lowest counts are the ones folded"
    );
    let folded: u64 = panel.rows.iter().map(|row| row.count).sum::<u64>();
    let total: u64 = items.iter().map(|item| item.count).sum();
    assert_eq!(folded, total, "nothing is lost by folding");
    assert_eq!(panel.check(), Ok(()));
}

#[test]
fn a_move_is_stamped_with_the_time_of_the_application() {
    let items = [supply("ochre", 2)];
    let panel = panel_of(
        &items,
        &basis(7_000),
        None,
        &[taken("ochre", ChangeKind::Consume, 3, 2)],
    );
    assert_eq!(panel.moves[0].at_ms, 7_000);
}

#[test]
fn two_applications_close_together_both_flash() {
    let first_items = [supply("boards", 5), supply("ochre", 4), supply("rags", 2)];
    let first = panel_of(
        &first_items,
        &basis(1_000),
        None,
        &[
            taken("boards", ChangeKind::Consume, 6, 5),
            taken("ochre", ChangeKind::Consume, 5, 4),
        ],
    );
    assert_eq!(first.moves.len(), 2);
    let second_items = [supply("boards", 5), supply("ochre", 4), supply("rags", 1)];
    let second = panel_of(
        &second_items,
        &basis(1_700),
        Some(&first),
        &[taken("rags", ChangeKind::Consume, 2, 1)],
    );
    let moves: Vec<(&str, u64, u64, u64)> = second
        .moves
        .iter()
        .map(|moved| (moved.row.as_str(), moved.before, moved.after, moved.at_ms))
        .collect();
    assert_eq!(
        moves,
        [
            ("boards", 6, 5, 1_000),
            ("ochre", 5, 4, 1_000),
            ("rags", 2, 1, 1_700)
        ],
        "the later push keeps the earlier moves, with their own times"
    );
}

#[test]
fn a_row_moved_by_two_applications_keeps_where_it_began_and_where_it_is() {
    let first = panel_of(
        &[supply("boards", 5)],
        &basis(1_000),
        None,
        &[taken("boards", ChangeKind::Consume, 6, 5)],
    );
    let second = panel_of(
        &[supply("boards", 4)],
        &basis(1_500),
        Some(&first),
        &[taken("boards", ChangeKind::Consume, 5, 4)],
    );
    assert_eq!(second.moves.len(), 1);
    let moved = &second.moves[0];
    assert_eq!(
        (moved.before, moved.after, moved.at_ms),
        (6, 4, 1_500),
        "6 to 4, stamped by the later"
    );
    let back = panel_of(
        &[supply("boards", 6)],
        &basis(1_800),
        Some(&second),
        &[taken("boards", ChangeKind::Add, 4, 6)],
    );
    assert!(back.moves.is_empty(), "back where it began is no move");
}

#[test]
fn a_move_older_than_the_flash_is_not_carried() {
    let first = panel_of(
        &[supply("boards", 5), supply("rags", 2)],
        &basis(1_000),
        None,
        &[taken("boards", ChangeKind::Consume, 6, 5)],
    );
    let later = panel_of(
        &[supply("boards", 5), supply("rags", 1)],
        &basis(1_000 + df_playground::inventory::FLASH_MS),
        Some(&first),
        &[taken("rags", ChangeKind::Consume, 2, 1)],
    );
    let rows: Vec<&str> = later.moves.iter().map(|moved| moved.row.as_str()).collect();
    assert_eq!(rows, ["rags"]);
}

async fn host() -> Running {
    let host = start(&STARTER_FILES).await;
    host.write_asset("fixtures/scene.json", FIXTURE);
    host
}

async fn post<T: serde::de::DeserializeOwned>(
    host: &Running,
    address: &str,
    body: Value,
) -> InventoryReply<T> {
    post_json(host.port, address, &body).await.expect("posts")
}

fn done<T: std::fmt::Debug>(reply: InventoryReply<T>) -> T {
    match reply {
        DevReply::Done { value } => value,
        DevReply::Rejected { error, message } => panic!("refused: {error:?} {message}"),
    }
}

async fn seed(host: &Running, owner: &str, items: &[InventoryItem]) {
    let _: Changed = done(
        post(
            host,
            path::INVENTORY_SET,
            json!({"owner": owner, "kind": "character", "items": items}),
        )
        .await,
    );
}

fn panel_on_screen(host: &Running) -> Option<InventoryPanel> {
    host.state
        .playground()
        .snapshot()
        .scene
        .inventory
        .set_value()
        .cloned()
}

fn logged(host: &Running) -> Vec<String> {
    host.state
        .log()
        .after_sequence(0, usize::MAX)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("WORLD inventory_"))
        .collect()
}

#[tokio::test]
async fn nothing_is_on_the_screen_until_an_owner_is_shown() {
    let host = host().await;
    seed(&host, "danny", &[supply("ochre", 3)]).await;
    let _: Changed = done(
        post(
            &host,
            path::INVENTORY_CONSUME,
            json!({"owner": "danny", "item": "ochre", "count": 1}),
        )
        .await,
    );
    assert_eq!(panel_on_screen(&host), None, "hidden by default");
}

#[tokio::test]
async fn a_shown_owner_is_followed_through_every_route_and_a_hidden_screen_is_clear() {
    let host = host().await;
    seed(
        &host,
        "danny",
        &[supply("ochre", 3), supply("rags", 2), tool("easel", 1)],
    )
    .await;
    seed(&host, "baker", &[supply("loaf", 4)]).await;
    let shown: Screen = done(post(&host, path::INVENTORY_SHOW, json!({"owner": "danny"})).await);
    assert_eq!((shown.owner.as_deref(), shown.rows), (Some("danny"), 3));
    let first = panel_on_screen(&host).expect("a panel");
    assert_eq!(
        (first.owner.as_str(), first.title.as_str()),
        ("danny", "Satchel")
    );
    assert!(first.moves.is_empty());

    let _: Changed = done(
        post(
            &host,
            path::INVENTORY_CONSUME,
            json!({"owner": "danny", "item": "rags", "count": 2, "why": "wiped"}),
        )
        .await,
    );
    let next = panel_on_screen(&host).expect("a panel");
    assert!(next.serial > first.serial);
    assert_eq!(counts(&next), [("ochre", 3), ("rags", 0), ("easel", 1)]);
    assert_eq!(next.moves.len(), 1);
    assert_eq!(
        (
            next.moves[0].row.as_str(),
            next.moves[0].before,
            next.moves[0].after,
            next.moves[0].kind
        ),
        ("rags", 2, 0, MoveKind::Consume)
    );

    let _: Changed = done(
        post(
            &host,
            path::INVENTORY_ADD,
            json!({"owner": "danny", "item": "boards", "count": 6,
                "name": "boards", "look": "primed limewood", "why": "bought"}),
        )
        .await,
    );
    assert_eq!(
        counts(&panel_on_screen(&host).expect("a panel")),
        [("ochre", 3), ("rags", 0), ("boards", 6), ("easel", 1)],
        "a new item is listed, the one that ran out stays"
    );

    let _: Changed = done(
        post(
            &host,
            path::INVENTORY_TRANSFER,
            json!({"from": "danny", "to": "baker", "item": "ochre", "count": 1}),
        )
        .await,
    );
    let out = panel_on_screen(&host).expect("a panel");
    assert_eq!(out.rows[0].count, 2);
    let ochre = out.moves.iter().find(|moved| moved.row == "ochre");
    assert_eq!(ochre.map(|moved| moved.kind), Some(MoveKind::TransferOut));
    assert!(
        out.moves.len() >= 2,
        "the moves of the earlier applications still flashing are carried"
    );

    let _: Changed = done(
        post(
            &host,
            path::INVENTORY_CONSUME,
            json!({"owner": "baker", "item": "loaf", "count": 1}),
        )
        .await,
    );
    assert_eq!(
        panel_on_screen(&host).expect("a panel").serial,
        out.serial,
        "another owner's change is not this panel's"
    );

    let hidden: Screen = done(post(&host, path::INVENTORY_HIDE, json!({})).await);
    assert_eq!((hidden.owner, hidden.rows), (None, 0));
    assert_eq!(panel_on_screen(&host), None);
    let lines = logged(&host);
    assert!(lines.contains(&"WORLD inventory_shown owner=danny rows=3".to_owned()));
    assert!(lines.contains(&"WORLD inventory_followed owner=danny rows=3 moves=1".to_owned()));
    assert!(lines.contains(&"WORLD inventory_hidden".to_owned()));
}

#[tokio::test]
async fn showing_an_owner_with_no_inventory_is_refused_and_shows_nothing() {
    let host = host().await;
    let refused: InventoryReply<Screen> =
        post(&host, path::INVENTORY_SHOW, json!({"owner": "nobody"})).await;
    match refused {
        DevReply::Rejected { error, .. } => assert_eq!(error.key(), "unknown_owner"),
        DevReply::Done { value } => panic!("done: {value:?}"),
    }
    assert_eq!(panel_on_screen(&host), None);
}

#[tokio::test]
async fn a_judged_line_that_used_a_supply_up_reaches_the_screen() {
    let host = host().await;
    seed(&host, "danny", &[supply("charcoal", 1), tool("easel", 1)]).await;
    let _: Screen = done(post(&host, path::INVENTORY_SHOW, json!({"owner": "danny"})).await);
    let delta: InventoryDelta = serde_json::from_value(json!({
        "item": "charcoal", "change": -1, "reason": "used_up", "why": "sketched the fountain"
    }))
    .expect("a delta reads");
    let applied = take_judged_changes(&host.state, "danny", &[delta], &[]);
    assert_eq!(applied.len(), 1);
    follow_judged(&host.state, "danny", &applied).await;
    let panel = panel_on_screen(&host).expect("a panel");
    assert_eq!(counts(&panel), [("charcoal", 0), ("easel", 1)]);
    assert_eq!(
        (
            panel.moves[0].row.as_str(),
            panel.moves[0].before,
            panel.moves[0].after
        ),
        ("charcoal", 1, 0)
    );
}

#[tokio::test]
async fn a_judged_line_changes_nothing_on_a_screen_that_shows_no_one() {
    let host = host().await;
    seed(&host, "danny", &[supply("charcoal", 2)]).await;
    follow_judged(
        &host.state,
        "danny",
        &[taken("charcoal", ChangeKind::Consume, 2, 1)],
    )
    .await;
    assert_eq!(panel_on_screen(&host), None);
    assert!(logged(&host).iter().all(|line| !line.contains("followed")));
}

#[tokio::test]
async fn the_show_and_hide_verbs_are_read_by_the_command_line() {
    use super::cli::{InventoryCommand, parse};
    let words = |text: &str| -> Vec<String> { text.split(' ').map(str::to_owned).collect() };
    assert_eq!(
        parse(&words("show danny")).expect("reads"),
        InventoryCommand::Show {
            owner: "danny".to_owned()
        }
    );
    assert_eq!(
        parse(&words("hide")).expect("reads"),
        InventoryCommand::Hide
    );
    assert!(parse(&words("show")).is_err());
    assert!(parse(&words("hide danny")).is_err());
}
