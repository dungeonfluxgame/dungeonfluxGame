//! Tests of the landing page's skeleton components: the data and its bounds, the names of the
//! drawings, and the live changes to the brand mark, the title block, the bell, the navigation
//! rail and the action bar: what each does, what it refuses, that a refusal changes nothing and
//! what the audit line may carry.

use crate::fixture::Fixture;
use crate::fixtures::assert_keys_are_serde_names;
use crate::home::change::{BrandEdit, ButtonEdit, HomeEdit, NavEdit, TitleEdit, TopEdit};
use crate::home::checks::HomeError;
use crate::home::chrome::{BrandSize, bell_label, unread_text};
use crate::home::glyph::{HomeGlyph, HomeIcon};
use crate::home::model::{HomePart, HomeScene};
use crate::home::nav::{ActionVariant, NavItem, NavRail};
use crate::limits::{MAXIMUM_FOCUS_CHARACTERS, MAXIMUM_NAV_ITEMS};
use crate::overlay::{Edit, Glyph};
use crate::scene::{SceneChange, SceneError, SceneOverlay};

const FIXTURE: &str = r##"{
    "name": "Base",
    "tv": {
        "place": "", "status": {"kind": "ok"}, "elements": [], "caption": null,
        "home": {
            "title": {"title": "DungeonFlux", "tagline": "Forge a world."},
            "nav": {"items": [
                {"label": "Home", "glyph": "home", "active": true},
                {"label": "Campaigns", "glyph": "campaigns"}
            ]},
            "actions": {"buttons": [
                {"label": "Continue Campaign", "subtitle": "The Ashen Citadel", "glyph": "play", "variant": "primary"},
                {"label": "Quick Launch", "glyph": "bolt"}
            ]}
        }
    },
    "phone": {"hero": "Pip", "status": {"kind": "ok"}}
}"##;

fn base() -> Fixture {
    Fixture::parse(FIXTURE).expect("the fixture reads")
}

fn make(edit: HomeEdit) -> (SceneOverlay, Result<(), SceneError>) {
    let mut overlay = SceneOverlay::default();
    let result = overlay.apply(SceneChange::Home { edit }, &base());
    (overlay, result)
}

fn drawn(overlay: &SceneOverlay) -> HomeScene {
    let mut fixture = base();
    overlay.apply_to(&mut fixture);
    fixture.tv.home.expect("the page has a home")
}

fn nav_edit(number: usize, item: NavEdit) -> HomeEdit {
    HomeEdit::Nav { number, item }
}

fn action_edit(number: usize, button: ButtonEdit) -> HomeEdit {
    HomeEdit::Action { number, button }
}

#[test]
fn a_scene_without_a_home_reads_as_before_and_one_with_it_keeps_every_part() {
    let plain = Fixture::parse(
        r#"{"name":"x","tv":{"place":"","status":{"kind":"ok"},"elements":[],"caption":null},"phone":{"hero":"P","status":{"kind":"ok"}}}"#,
    )
    .expect("reads");
    assert_eq!(plain.tv.home, None);
    let home = base().tv.home.expect("home");
    assert_eq!(
        home.present(),
        vec![HomePart::Title, HomePart::Nav, HomePart::Actions]
    );
    assert_eq!(home.nav.expect("nav").items.len(), 2);
}

#[test]
fn every_drawing_has_a_unique_name_and_a_drawing() {
    let mut keys: Vec<&str> = HomeGlyph::ALL.iter().map(|glyph| glyph.key()).collect();
    keys.extend(Glyph::ALL.iter().map(|glyph| glyph.key()));
    let total = keys.len();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), total, "no two drawings share a name");
    for glyph in HomeGlyph::ALL {
        assert!(glyph.markup().starts_with('<'));
        assert_eq!(HomeGlyph::from_key(glyph.key()), Some(glyph));
    }
    assert_eq!(
        HomeIcon::from_key("sword"),
        Some(HomeIcon::Overlay(Glyph::Sword))
    );
    assert_eq!(HomeIcon::from_key("banana"), None);
}

#[test]
fn an_unknown_icon_in_a_scene_file_names_the_icons() {
    let text = FIXTURE.replace("\"glyph\": \"bolt\"", "\"glyph\": \"banana\"");
    let error = Fixture::parse(&text).expect_err("refused");
    assert!(error.message.contains("banana"), "{}", error.message);
    assert!(error.message.contains("hourglass"), "{}", error.message);
}

#[test]
fn a_field_the_page_does_not_know_is_a_mistake() {
    let text = FIXTURE.replace(
        "\"tagline\": \"Forge a world.\"",
        "\"tagline\": \"Forge a world.\", \"rool\": true",
    );
    assert!(Fixture::parse(&text).is_err());
}

#[test]
fn the_brand_mark_is_made_and_changed_field_by_field() {
    let (overlay, result) = make(HomeEdit::Brand {
        brand: BrandEdit {
            size: Some(BrandSize::Display),
            ..BrandEdit::default()
        },
    });
    result.expect("applies");
    let brand = drawn(&overlay).brand.expect("brand");
    assert_eq!(
        (brand.wordmark.as_str(), brand.size, brand.star),
        ("DungeonFlux", BrandSize::Display, true)
    );
    let mut overlay = overlay;
    overlay
        .apply(
            SceneChange::Home {
                edit: HomeEdit::Brand {
                    brand: BrandEdit {
                        wordmark: Some("Flux".to_owned()),
                        star: Some(false),
                        ..BrandEdit::default()
                    },
                },
            },
            &base(),
        )
        .expect("applies");
    let brand = drawn(&overlay).brand.expect("brand");
    assert_eq!(
        (brand.wordmark.as_str(), brand.size, brand.star),
        ("Flux", BrandSize::Display, false)
    );
}

#[test]
fn a_blank_wordmark_or_title_is_refused_and_changes_nothing() {
    for edit in [
        HomeEdit::Brand {
            brand: BrandEdit {
                wordmark: Some("  ".to_owned()),
                ..BrandEdit::default()
            },
        },
        HomeEdit::Title {
            title: TitleEdit {
                title: Some(String::new()),
                ..TitleEdit::default()
            },
        },
    ] {
        let (overlay, result) = make(edit);
        assert!(matches!(result, Err(SceneError::Empty(_))), "{result:?}");
        assert!(overlay.is_empty(), "a refusal leaves the overlay as it was");
    }
}

#[test]
fn the_title_block_changes_its_tagline_and_rule() {
    let (mut overlay, result) = make(HomeEdit::Title {
        title: TitleEdit {
            tagline: Edit::Set("Begin the legend.".to_owned()),
            rule: Some(false),
            ..TitleEdit::default()
        },
    });
    result.expect("applies");
    let title = drawn(&overlay).title.expect("title");
    assert_eq!(title.tagline.as_deref(), Some("Begin the legend."));
    assert!(!title.rule);
    assert_eq!(title.title, "DungeonFlux");
    overlay
        .apply(
            SceneChange::Home {
                edit: HomeEdit::Title {
                    title: TitleEdit {
                        tagline: Edit::Clear,
                        ..TitleEdit::default()
                    },
                },
            },
            &base(),
        )
        .expect("applies");
    assert_eq!(drawn(&overlay).title.expect("title").tagline, None);
}

#[test]
fn the_bell_counts_unread_notices_and_says_so_to_a_screen_reader() {
    assert_eq!(unread_text(None), None);
    assert_eq!(unread_text(Some(0)), None);
    assert_eq!(unread_text(Some(7)).as_deref(), Some("7"));
    assert_eq!(unread_text(Some(100)).as_deref(), Some("99+"));
    assert_eq!(bell_label(None), "Notifications");
    assert_eq!(bell_label(Some(1)), "Notifications, 1 unread");
    assert_eq!(bell_label(Some(3)), "Notifications, 3 unread");
    let (overlay, result) = make(HomeEdit::Top {
        top: TopEdit {
            unread: Edit::Set(3),
            ..TopEdit::default()
        },
    });
    result.expect("applies");
    let top = drawn(&overlay).top.expect("top");
    assert_eq!((top.bell, top.unread), (true, Some(3)));
}

#[test]
fn a_navigation_item_is_changed_by_its_number_and_one_item_stays_active() {
    let (overlay, result) = make(nav_edit(
        2,
        NavEdit {
            active: Some(true),
            badge: Edit::Set("2".to_owned()),
            glyph: Some(HomeIcon::Page(HomeGlyph::Globe)),
            ..NavEdit::default()
        },
    ));
    result.expect("applies");
    let rail = drawn(&overlay).nav.expect("nav");
    assert_eq!(
        rail.items
            .iter()
            .map(|item| item.active)
            .collect::<Vec<_>>(),
        vec![false, true]
    );
    assert_eq!(rail.items[1].badge.as_deref(), Some("2"));
    assert_eq!(rail.items[1].glyph, HomeIcon::Page(HomeGlyph::Globe));
}

#[test]
fn the_next_free_number_adds_an_item_and_a_new_item_needs_a_label() {
    let (overlay, result) = make(nav_edit(
        3,
        NavEdit {
            label: Some("Assets".to_owned()),
            ..NavEdit::default()
        },
    ));
    result.expect("applies");
    let rail = drawn(&overlay).nav.expect("nav");
    assert_eq!(rail.items.len(), 3);
    assert_eq!(rail.items[2].label, "Assets");
    let (overlay, result) = make(nav_edit(3, NavEdit::default()));
    assert!(matches!(result, Err(SceneError::Empty(_))), "{result:?}");
    assert!(overlay.is_empty());
}

#[test]
fn a_number_past_the_next_free_one_says_which_is_next() {
    let (overlay, result) = make(nav_edit(
        9,
        NavEdit {
            label: Some("Far".to_owned()),
            ..NavEdit::default()
        },
    ));
    assert_eq!(
        result,
        Err(SceneError::Home(HomeError::Number {
            what: "navigation item",
            number: 9,
            next: 3
        }))
    );
    assert!(overlay.is_empty());
    let message = result.expect_err("refused").to_string();
    assert!(message.contains("next free number is 3"), "{message}");
    let (_, result) = make(HomeEdit::NavRemove { number: 0 });
    assert!(matches!(
        result,
        Err(SceneError::Home(HomeError::Number { number: 0, .. }))
    ));
}

#[test]
fn removing_an_item_takes_it_off_the_rail() {
    let (overlay, result) = make(HomeEdit::NavRemove { number: 1 });
    result.expect("applies");
    let rail = drawn(&overlay).nav.expect("nav");
    assert_eq!(rail.items.len(), 1);
    assert_eq!(rail.items[0].label, "Campaigns");
}

#[test]
fn two_active_items_in_a_scene_file_are_refused() {
    let rail = NavRail {
        items: vec![
            NavItem {
                label: "A".to_owned(),
                glyph: HomeIcon::Page(HomeGlyph::Home),
                active: true,
                badge: None,
            },
            NavItem {
                label: "B".to_owned(),
                glyph: HomeIcon::Page(HomeGlyph::Home),
                active: true,
                badge: None,
            },
        ],
    };
    assert_eq!(
        crate::home::checks::nav(&rail),
        Err(SceneError::Home(HomeError::ManyActive { count: 2 }))
    );
}

#[test]
fn the_navigation_rail_holds_a_very_large_number_of_items() {
    let rail = NavRail {
        items: (0..=MAXIMUM_NAV_ITEMS)
            .map(|index| NavItem {
                label: format!("Item {index}"),
                glyph: HomeIcon::Page(HomeGlyph::Home),
                active: false,
                badge: None,
            })
            .collect(),
    };
    assert!(matches!(
        crate::home::checks::nav(&rail),
        Err(SceneError::Home(HomeError::TooMany { .. }))
    ));
}

#[test]
fn an_action_button_changes_its_look_and_can_be_turned_off() {
    let (overlay, result) = make(action_edit(
        2,
        ButtonEdit {
            variant: Some(ActionVariant::Primary),
            subtitle: Edit::Set("Right now".to_owned()),
            enabled: Some(false),
            glyph: Edit::Clear,
            ..ButtonEdit::default()
        },
    ));
    result.expect("applies");
    let bar = drawn(&overlay).actions.expect("actions");
    let button = &bar.buttons[1];
    assert_eq!(button.variant, ActionVariant::Primary);
    assert_eq!(button.subtitle.as_deref(), Some("Right now"));
    assert!(!button.enabled);
    assert_eq!(button.glyph, None);
    assert_eq!(bar.buttons[0].label, "Continue Campaign");
}

#[test]
fn a_button_is_added_and_removed_by_number() {
    let (overlay, result) = make(action_edit(
        3,
        ButtonEdit {
            label: Some("Join Session".to_owned()),
            glyph: Edit::Set(HomeIcon::Page(HomeGlyph::Players)),
            ..ButtonEdit::default()
        },
    ));
    result.expect("applies");
    assert_eq!(drawn(&overlay).actions.expect("actions").buttons.len(), 3);
    let (overlay, result) = make(HomeEdit::ActionRemove { number: 2 });
    result.expect("applies");
    let bar = drawn(&overlay).actions.expect("actions");
    assert_eq!(bar.buttons.len(), 1);
    let (_, result) = make(HomeEdit::ActionRemove { number: 5 });
    assert!(matches!(
        result,
        Err(SceneError::Home(HomeError::Number { number: 5, .. }))
    ));
}

#[test]
fn removing_a_part_and_clearing_the_page_work_and_a_missing_part_is_said() {
    let (overlay, result) = make(HomeEdit::Remove {
        part: HomePart::Nav,
    });
    result.expect("applies");
    assert!(drawn(&overlay).nav.is_none());
    assert!(drawn(&overlay).actions.is_some());
    let (overlay, result) = make(HomeEdit::Clear {});
    result.expect("applies");
    let mut fixture = base();
    overlay.apply_to(&mut fixture);
    assert_eq!(fixture.tv.home, None);
    let empty = Fixture::parse(
        r#"{"name":"x","tv":{"place":"","status":{"kind":"ok"},"elements":[],"caption":null},"phone":{"hero":"P","status":{"kind":"ok"}}}"#,
    )
    .expect("reads");
    let mut overlay = SceneOverlay::default();
    let result = overlay.apply(
        SceneChange::Home {
            edit: HomeEdit::NavRemove { number: 1 },
        },
        &empty,
    );
    assert!(matches!(
        result,
        Err(SceneError::Home(HomeError::Missing { .. }))
    ));
}

#[test]
fn reset_drops_the_pages_changes() {
    let (mut overlay, result) = make(HomeEdit::Remove {
        part: HomePart::Title,
    });
    result.expect("applies");
    overlay
        .apply(SceneChange::Reset {}, &base())
        .expect("applies");
    assert!(drawn(&overlay).title.is_some());
}

#[test]
fn the_audit_line_names_the_component_and_the_field_and_never_the_text() {
    let edit = nav_edit(
        2,
        NavEdit {
            label: Some("Secret words".to_owned()),
            active: Some(true),
            ..NavEdit::default()
        },
    );
    let (verb, pairs) = edit.audit_fields();
    assert_eq!(verb, "home");
    assert_eq!(
        pairs[..2],
        [
            ("component", "nav".to_owned()),
            ("field", "item".to_owned())
        ]
    );
    assert!(pairs.contains(&("item", "2".to_owned())));
    assert!(pairs.contains(&("active", "true".to_owned())));
    assert!(pairs.iter().all(|(_, value)| !value.contains("Secret")));
    let (_, pairs) = HomeEdit::Remove {
        part: HomePart::Campaigns,
    }
    .audit_fields();
    assert!(pairs.contains(&("part", "campaigns".to_owned())));
}

#[test]
fn a_page_change_redraws_the_panels_and_only_them() {
    let before = base();
    let mut after = base();
    after.tv.home.as_mut().expect("home").top = Some(crate::home::chrome::HomeTopBar {
        bell: true,
        unread: Some(1),
    });
    assert!(crate::overlay::changes::panels_differ(
        &before.tv, &after.tv
    ));
    assert!(!crate::overlay::changes::panels_differ(
        &before.tv, &before.tv
    ));
    assert!(!crate::overlay::changes::pills_differ(
        &before.tv, &after.tv
    ));
}

#[test]
fn the_focus_defaults_to_the_first_primary_action_and_follows_an_explicit_id() {
    let home = base().tv.home.expect("home");
    assert_eq!(home.focus_id().as_deref(), Some("actions:1"));
    let (overlay, result) = make(HomeEdit::Focus {
        id: Edit::Set("nav:2".to_owned()),
    });
    result.expect("applies");
    assert_eq!(drawn(&overlay).focus_id().as_deref(), Some("nav:2"));
    let (mut overlay, _) = make(HomeEdit::Focus {
        id: Edit::Set("nav:2".to_owned()),
    });
    overlay
        .apply(
            SceneChange::Home {
                edit: HomeEdit::Focus { id: Edit::Clear },
            },
            &base(),
        )
        .expect("applies");
    assert_eq!(drawn(&overlay).focus_id().as_deref(), Some("actions:1"));
    let mut bare = HomeScene::default();
    assert_eq!(bare.focus_id(), None, "no primary action, no default focus");
    bare.focus = Some("adventure:launch".to_owned());
    assert_eq!(bare.focus_id().as_deref(), Some("adventure:launch"));
}

#[test]
fn a_focus_id_that_needs_quoting_is_refused_and_changes_nothing() {
    let too_long = "x".repeat(MAXIMUM_FOCUS_CHARACTERS + 1);
    for id in ["", "Actions 1", "a\"b", "nav:2]", &too_long] {
        let (overlay, result) = make(HomeEdit::Focus {
            id: Edit::Set(id.to_owned()),
        });
        assert!(
            matches!(result, Err(SceneError::Home(HomeError::InvalidFocus(_)))),
            "{id:?}: {result:?}"
        );
        assert!(overlay.is_empty());
    }
    let message = make(HomeEdit::Focus {
        id: Edit::Set("Bad Id".to_owned()),
    })
    .1
    .expect_err("refused")
    .to_string();
    assert!(message.contains("actions:1"), "{message}");
    let (_, pairs) = HomeEdit::Focus {
        id: Edit::Set("nav:2".to_owned()),
    }
    .audit_fields();
    assert_eq!(
        pairs[..2],
        [
            ("component", "page".to_owned()),
            ("field", "focus".to_owned())
        ]
    );
    assert!(pairs.contains(&("id", "nav:2".to_owned())));
}

#[test]
fn the_keys_of_the_landing_page_are_their_serde_names() {
    assert_keys_are_serde_names(&BrandSize::ALL, BrandSize::key, BrandSize::from_key);
    assert_keys_are_serde_names(
        &ActionVariant::ALL,
        ActionVariant::key,
        ActionVariant::from_key,
    );
    assert_keys_are_serde_names(&HomePart::ALL, HomePart::key, HomePart::from_key);
}

#[test]
fn a_page_icon_is_written_by_its_snake_case_key() {
    for glyph in HomeGlyph::ALL {
        let written = serde_json::to_value(HomeIcon::Page(glyph)).expect("an icon serializes");
        assert_eq!(written, serde_json::Value::from(glyph.key()));
        assert!(
            glyph
                .key()
                .chars()
                .all(|character| character.is_ascii_lowercase() || character == '_'),
            "{glyph:?}"
        );
        assert_eq!(HomeGlyph::from_key(glyph.key()), Some(glyph));
    }
    assert_eq!(HomeGlyph::from_key("chevron-down"), None);
}
