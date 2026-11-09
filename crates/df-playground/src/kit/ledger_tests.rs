//! The loot and shop lists' rows.

use crate::kit::kit_tests::assert_css_words_are_serde_names;
use crate::kit::ledger::{Ledger, LedgerKind};
use crate::kit::{Fill, Rarity};

#[test]
fn a_loot_row_shows_its_quantity_and_skips_a_price() {
    let ledger: Ledger = serde_json::from_str(
        r#"{"title":"Chest Contents","items":[
            {"id":"p","name":"Potion of Healing","quantity":"x2","price":"50 gp","rarity":"common"},
            {"id":"b","name":" "}
        ]}"#,
    )
    .expect("a list reads");
    assert_eq!(ledger.kind, LedgerKind::Loot);
    let rows = ledger.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].figure, Some("x2"));
    assert_eq!(rows[0].item.rarity, Some(Rarity::Common));
    assert!(ledger.is_drawn());
}

#[test]
fn a_shop_row_shows_the_server_price() {
    let ledger: Ledger = serde_json::from_str(
        r#"{"kind":"shop","items":[
            {"id":"s","name":"Longsword","price":"15 gp","quantity":"x1"}
        ]}"#,
    )
    .expect("a list reads");
    assert_eq!(ledger.rows()[0].figure, Some("15 gp"));
    assert_eq!(ledger.title(), None);
}

#[test]
fn a_list_is_framed_glass_unless_it_floats() {
    let mut ledger: Ledger = serde_json::from_str(r#"{"items":[]}"#).expect("a list reads");
    assert!(ledger.framed);
    assert_eq!(ledger.fill(), Fill::Blurred);
    ledger.framed = false;
    assert_eq!(ledger.fill(), Fill::Solid);
}

#[test]
fn a_list_without_named_rows_is_not_drawn() {
    let ledger: Ledger = serde_json::from_str(r#"{"items":[]}"#).expect("a list reads");
    assert!(!ledger.is_drawn());
    assert_css_words_are_serde_names(&[LedgerKind::Loot, LedgerKind::Shop], LedgerKind::key);
}
