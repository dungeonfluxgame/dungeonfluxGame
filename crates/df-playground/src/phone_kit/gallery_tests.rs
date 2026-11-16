//! The phone gallery's boards read, and show every component of this batch with real media names.

use crate::media::{MediaKind, media_kind};
use crate::phone_kit::gallery::{PhoneGallery, board_number};
use crate::phone_kit::shell::TabKey;

#[test]
fn the_compiled_in_boards_read_and_number_from_one() {
    let gallery = PhoneGallery::read().expect("the boards read");
    assert_eq!(gallery.boards.len(), 10);
    assert!(gallery.board(0).is_none());
    assert!(gallery.board(1).is_some());
    assert!(gallery.board(11).is_none());
}

#[test]
fn the_query_names_a_board() {
    assert_eq!(board_number("?phone_kit=3"), Some(3));
    assert_eq!(board_number("?room=ABC&phone_kit=2"), Some(2));
    assert_eq!(board_number("?phone_kit=0"), None);
    assert_eq!(board_number("?kit=3"), None);
    assert_eq!(board_number(""), None);
}

#[test]
fn every_picture_a_board_names_is_an_image_in_the_media_library() {
    let gallery = PhoneGallery::read().expect("reads");
    for board in &gallery.boards {
        for screen in &board.screens {
            let panels = &screen.panels;
            let mut names: Vec<&str> = Vec::new();
            names.extend(panels.header.iter().filter_map(|h| h.portrait.as_deref()));
            if let Some(talk) = &panels.talk {
                names.extend(talk.cast.iter().filter_map(|c| c.portrait.as_deref()));
            }
            for name in names {
                assert_eq!(media_kind(name), Some(MediaKind::Image), "{name}");
                let path = format!(
                    "{}/../../assets/playground/media/{name}",
                    env!("CARGO_MANIFEST_DIR")
                );
                assert!(
                    std::path::Path::new(&path).exists(),
                    "{name} is not in the media library"
                );
            }
        }
    }
}

#[test]
fn the_boards_cover_the_shell_and_the_three_panels() {
    let gallery = PhoneGallery::read().expect("reads");
    let screens: Vec<_> = gallery
        .boards
        .iter()
        .flat_map(|board| &board.screens)
        .collect();
    assert!(screens.iter().any(|s| s.panels.link_down));
    assert!(screens.iter().any(|s| s.panels.toast.is_some()));
    assert!(screens.iter().any(|s| !s.panels.hp.is_empty()));
    assert!(screens.iter().any(|s| s.panels.empty.is_some()));
    assert!(screens.iter().any(|s| s.panels.loading.is_some()));
    assert!(screens.iter().any(|s| s.panels.act.is_some()));
    assert!(screens.iter().any(|s| s.panels.talk.is_some()));
    assert!(screens.iter().any(|s| s.panels.roll.is_some()));
    assert!(
        screens
            .iter()
            .any(|s| s.panels.tabs.as_ref().is_some_and(|t| t.more_open))
    );
    for screen in screens {
        if let Some(tabs) = &screen.panels.tabs {
            assert!(
                tabs.active.is_none_or(|active| tabs.tabs.contains(&active)),
                "{} lacks its open tab",
                screen.title
            );
            assert!(tabs.tabs.iter().all(|tab| TabKey::ALL.contains(tab)));
        }
    }
}

#[test]
fn every_page_key_is_unique_and_every_action_leads_to_a_page() {
    use crate::phone_kit::nav::NavAction;
    use crate::phone_kit::page::Block;

    let gallery = PhoneGallery::read().expect("reads");
    let screens: Vec<_> = gallery
        .boards
        .iter()
        .flat_map(|board| &board.screens)
        .collect();
    let keys: Vec<&str> = screens
        .iter()
        .filter_map(|screen| screen.key.as_deref())
        .collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), keys.len(), "a page key is used twice");
    let mut actions: Vec<&str> = Vec::new();
    for screen in &screens {
        if let Some(auto) = &screen.auto {
            assert!(keys.contains(&auto.page.as_str()), "{}", auto.page);
        }
        for block in &screen.panels.blocks {
            match block {
                Block::Buttons { buttons } => {
                    actions.extend(buttons.iter().filter_map(|b| b.action.as_deref()));
                }
                Block::Cards { cards, .. } => {
                    actions.extend(cards.iter().filter_map(|c| c.action.as_deref()));
                }
                Block::Chips { chips, .. } => {
                    actions.extend(chips.iter().filter_map(|c| c.action.as_deref()));
                }
                Block::Rows { rows } => {
                    actions.extend(rows.iter().filter_map(|r| r.action.as_deref()));
                }
                Block::Map { nodes, .. } => {
                    actions.extend(nodes.iter().filter_map(|n| n.action.as_deref()));
                }
                _ => {}
            }
        }
    }
    for action in actions {
        match NavAction::parse(action) {
            Some(NavAction::Go(page)) => {
                assert!(keys.contains(&page.as_str()), "{action} goes nowhere");
            }
            Some(_) => {}
            None => panic!("{action} is not a navigation"),
        }
    }
    for tab in TabKey::ALL {
        assert!(
            keys.contains(&tab.page_key()),
            "{} has no page",
            tab.label()
        );
    }
}
