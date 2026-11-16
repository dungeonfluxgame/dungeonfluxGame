//! The phone frame's decisions: the health colours, the tab bar and its "more" slot.

use crate::overlay::rules::HealthTier;
use crate::phone_kit::shell::{HpBar, TabBadge, TabBar, TabKey};

fn bar(active: TabKey, tabs: &[TabKey]) -> TabBar {
    TabBar {
        active,
        tabs: tabs.to_vec(),
        badges: Vec::new(),
        more_open: false,
        working: Vec::new(),
    }
}

#[test]
fn the_health_bar_is_green_amber_below_half_and_red_below_a_quarter() {
    let tier = |current| {
        HpBar {
            current,
            maximum: 100,
            temporary: 0,
        }
        .tier()
    };
    assert_eq!(tier(100), HealthTier::High);
    assert_eq!(tier(50), HealthTier::High);
    assert_eq!(tier(49), HealthTier::Amber);
    assert_eq!(tier(25), HealthTier::Amber);
    assert_eq!(tier(24), HealthTier::Red);
    assert_eq!(tier(0), HealthTier::Red);
}

#[test]
fn the_health_words_name_the_temporary_points_only_when_there_are_some() {
    let plain = HpBar {
        current: 24,
        maximum: 27,
        temporary: 0,
    };
    assert_eq!(plain.text(), "24 / 27 HP");
    let padded = HpBar {
        temporary: 5,
        ..plain
    };
    assert_eq!(padded.text(), "24 / 27 HP (+5 temporary)");
}

#[test]
fn the_temporary_points_stay_inside_the_track() {
    let bar = |current, temporary| HpBar {
        current,
        maximum: 20,
        temporary,
    };
    assert!((bar(10, 4).temporary_percent() - 20.0).abs() < 0.1);
    assert!((bar(18, 9).temporary_percent() - 10.0).abs() < 0.1);
    assert_eq!(bar(10, 0).temporary_percent(), 0.0);
    let empty = HpBar {
        current: 0,
        maximum: 0,
        temporary: 3,
    };
    assert_eq!(empty.temporary_percent(), 0.0);
}

#[test]
fn five_tabs_or_fewer_are_all_on_the_bar() {
    let layout = bar(TabKey::Act, &TabKey::ALL[..5]).layout();
    assert_eq!(layout.bar, TabKey::ALL[..5].to_vec());
    assert!(layout.more.is_empty());
}

#[test]
fn more_tabs_keep_four_places_and_a_more_slot() {
    let layout = bar(TabKey::Act, &TabKey::ALL).layout();
    assert_eq!(layout.bar, TabKey::ALL[..4].to_vec());
    assert_eq!(layout.more, TabKey::ALL[4..].to_vec());
}

#[test]
fn an_open_panel_behind_more_takes_the_last_bar_place() {
    let layout = bar(TabKey::Hero, &TabKey::ALL).layout();
    assert_eq!(
        layout.bar,
        vec![TabKey::Story, TabKey::Act, TabKey::Talk, TabKey::Hero]
    );
    assert_eq!(
        layout.more,
        vec![
            TabKey::Roll,
            TabKey::Inventory,
            TabKey::Clues,
            TabKey::Map,
            TabKey::Notebook
        ]
    );
}

#[test]
fn a_badge_of_none_is_not_shown_and_dimmed_tabs_are_those_not_working() {
    let mut tabs = bar(TabKey::Act, &TabKey::ALL);
    tabs.badges = vec![
        TabBadge {
            tab: TabKey::Roll,
            count: 2,
        },
        TabBadge {
            tab: TabKey::Map,
            count: 0,
        },
    ];
    assert_eq!(tabs.badge(TabKey::Roll), Some(2));
    assert_eq!(tabs.badge(TabKey::Map), None);
    assert!(tabs.works(TabKey::Map));
    tabs.working = vec![TabKey::Act];
    assert!(tabs.works(TabKey::Act));
    assert!(!tabs.works(TabKey::Map));
}
