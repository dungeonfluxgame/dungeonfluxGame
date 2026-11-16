//! The phone's page navigation: what an action asks for and how a tap moves between pages.

use crate::phone_kit::nav::{NavAction, PhoneNav, page_action};
use crate::phone_kit::shell::TabKey;

#[test]
fn an_action_reads_as_a_page_the_way_back_or_the_more_sheet() {
    assert_eq!(
        NavAction::parse("page:bag"),
        Some(NavAction::Go("bag".to_owned()))
    );
    assert_eq!(NavAction::parse("page:back"), Some(NavAction::Back));
    assert_eq!(NavAction::parse("more"), Some(NavAction::ToggleMore));
    assert_eq!(NavAction::parse("more-close"), Some(NavAction::CloseMore));
    assert_eq!(NavAction::parse("page:"), None);
    assert_eq!(NavAction::parse("mic"), None);
    assert_eq!(page_action("map"), "page:map");
}

#[test]
fn every_tab_opens_its_own_page() {
    let keys: Vec<&str> = TabKey::ALL.iter().map(|tab| tab.page_key()).collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), keys.len());
    for tab in TabKey::ALL {
        assert_eq!(
            NavAction::parse(&page_action(tab.page_key())),
            Some(NavAction::Go(tab.page_key().to_owned()))
        );
    }
}

#[test]
fn going_and_coming_back_follow_the_pages_visited() {
    let mut nav = PhoneNav::new("story");
    nav.apply(NavAction::Go("bag".to_owned()));
    nav.apply(NavAction::Go("bag-item".to_owned()));
    nav.apply(NavAction::Go("bag-item".to_owned()));
    assert_eq!(nav.page, "bag-item");
    nav.apply(NavAction::Back);
    assert_eq!(nav.page, "bag");
    nav.apply(NavAction::Back);
    nav.apply(NavAction::Back);
    assert_eq!(nav.page, "story");
}

#[test]
fn the_more_sheet_opens_closes_and_closes_on_a_choice() {
    let mut nav = PhoneNav::new("story");
    nav.apply(NavAction::ToggleMore);
    assert!(nav.more_open);
    nav.apply(NavAction::CloseMore);
    assert!(!nav.more_open);
    nav.apply(NavAction::ToggleMore);
    nav.apply(NavAction::Go("map".to_owned()));
    assert!(!nav.more_open);
    assert_eq!(nav.page, "map");
}
