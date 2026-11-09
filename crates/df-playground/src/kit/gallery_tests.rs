//! The kit gallery's boards and the stage fixture under them.

use crate::fixture::Fixture;
use crate::kit::choices::ChoiceLayout;
use crate::kit::gallery::{KitGallery, board_number};
use crate::kit::party::{KitCard, PartyLayout};
use crate::kit::{BLURRED_PANES_AT_MOST, FINE_PANES_AT_MOST, KitPanels, PanelTone};
use crate::media::{MediaKind, media_kind};

const GALLERY_FIXTURE: &str =
    include_str!("../../../../assets/playground/fixtures/ui_kit_gallery.json");

fn gallery() -> KitGallery {
    KitGallery::read().expect("the gallery reads")
}

fn some_board(has: impl Fn(&KitPanels) -> bool) -> bool {
    gallery().boards.iter().any(|board| has(&board.panels))
}

#[test]
fn the_boards_show_every_component_and_its_variants() {
    assert!(some_board(|panels| panels.brand.is_some()));
    assert!(some_board(|panels| panels
        .chips
        .iter()
        .any(|chip| !chip.track().is_empty())));
    assert!(some_board(|panels| panels.status.is_some()));
    assert!(some_board(|panels| panels.progress.is_some()));
    for tone in [PanelTone::Gold, PanelTone::Danger] {
        assert!(some_board(|panels| panels
            .narration
            .as_ref()
            .is_some_and(|narration| narration.tone == tone)));
        assert!(some_board(|panels| panels
            .banners
            .iter()
            .any(|banner| banner.tone == tone)));
    }
    assert!(some_board(|panels| panels.party.is_some()));
    assert!(some_board(|panels| panels
        .bubbles
        .iter()
        .any(|bubble| !bubble.is_thinking())));
    assert!(some_board(|panels| panels
        .bubbles
        .iter()
        .any(|bubble| bubble.is_thinking())));
    assert!(some_board(|panels| panels.nameplate.is_some()));
    assert!(some_board(|panels| panels.caption.is_some()));
    assert!(some_board(|panels| panels.choices.is_some()));
    assert!(some_board(|panels| panels.journal.is_some()));
    assert!(some_board(|panels| !panels.title_cards.is_empty()));
    assert!(some_board(|panels| panels.speaker.is_some()));
    assert!(some_board(|panels| panels.section.is_some()));
    assert!(some_board(|panels| panels.ledgers.len() >= 2));
    assert!(some_board(|panels| !panels.places.is_empty()));
    assert!(some_board(|panels| panels.clues.is_some()));
}

#[test]
fn no_board_blurs_more_panes_than_the_budgets() {
    for board in &gallery().boards {
        let panes = board.panels.blurred_panes();
        assert!(
            panes <= BLURRED_PANES_AT_MOST,
            "{}: {panes} blurred panes",
            board.name
        );
        let fine = board.panels.fine_panes();
        assert!(
            fine <= FINE_PANES_AT_MOST,
            "{}: {fine} fine panes",
            board.name
        );
    }
    let counts: Vec<(usize, usize)> = gallery()
        .boards
        .iter()
        .map(|board| (board.panels.blurred_panes(), board.panels.fine_panes()))
        .collect();
    assert_eq!(
        counts,
        [
            (2, 6),
            (4, 0),
            (3, 8),
            (4, 3),
            (3, 1),
            (0, 0),
            (0, 0),
            (4, 1),
            (1, 0),
            (3, 8),
            (3, 3),
            (2, 1),
            (1, 0),
            (2, 4),
            (1, 6),
            (0, 0),
            (2, 3)
        ]
    );
}

#[test]
fn the_stress_boards_carry_long_words_a_full_table_and_empty_lists() {
    assert!(some_board(|panels| panels
        .party
        .as_ref()
        .is_some_and(|party| party.cards().len() >= 8)));
    assert!(some_board(|panels| panels
        .ledgers
        .iter()
        .any(|ledger| ledger.rows().len() >= 8)));
    assert!(some_board(|panels| panels
        .ledgers
        .iter()
        .any(|ledger| !ledger.is_drawn())));
    assert!(some_board(|panels| panels
        .journal
        .as_ref()
        .is_some_and(|journal| journal.entries().is_empty())));
    assert!(some_board(|panels| panels.chips.iter().any(|chip| chip
        .title()
        .is_some_and(|title| title.len() >= 70))));
    assert!(some_board(|panels| panels.choices.as_ref().is_some_and(
        |choices| choices
            .drawn()
            .iter()
            .any(|choice| choice.label.len() >= 70)
    )));
}

#[test]
fn the_stress_boards_carry_the_cases_that_overflow() {
    // Long bubble lines and long speakers' names in free bubbles.
    assert!(some_board(|panels| panels.bubbles.iter().any(|bubble| {
        bubble.line().is_some_and(|line| line.len() >= 150)
            && bubble.speaker().len() >= 30
            && bubble.place.is_some()
    })));
    // A compact strip of six cards or fewer, not dense, with a long name and a long class.
    assert!(some_board(|panels| panels.party.as_ref().is_some_and(
        |strip| {
            strip.layout() == PartyLayout::Compact
                && !strip.is_dense()
                && strip.cards().iter().any(|card| match card {
                    KitCard::Hero(hero) => {
                        hero.name.len() >= 30
                            && hero
                                .class_name
                                .as_deref()
                                .is_some_and(|class| class.len() >= 20)
                    }
                    KitCard::Open => false,
                })
        }
    )));
    // A journal of six entries or more with a long title and a long body, over its label.
    assert!(some_board(|panels| {
        panels.section_under_journal()
            && panels.journal.as_ref().is_some_and(|journal| {
                journal.entries().len() >= 6
                    && journal
                        .entries()
                        .iter()
                        .any(|entry| entry.title.len() >= 60)
                    && journal
                        .detail()
                        .is_some_and(|detail| detail.body.len() >= 400)
            })
    }));
    // A tall list in the right column beside the narration and choice cards.
    assert!(some_board(|panels| {
        panels.ledgers.iter().any(|ledger| ledger.rows().len() >= 9)
            && panels.narration.is_some()
            && panels
                .choices
                .as_ref()
                .is_some_and(|choices| choices.layout == ChoiceLayout::Cards)
    }));
}

#[test]
fn every_plate_is_an_image_of_the_media_library() {
    for board in &gallery().boards {
        let plate = board.plate.as_deref().expect("every board has a plate");
        assert_eq!(media_kind(plate), Some(MediaKind::Image), "{plate}");
    }
}

#[test]
fn a_board_is_named_by_its_query_counted_from_one() {
    assert_eq!(board_number("?kit=3"), Some(3));
    assert_eq!(board_number("?a=b&kit=1"), Some(1));
    assert_eq!(board_number("?kit=0"), None);
    assert_eq!(board_number("?kit=x"), None);
    assert_eq!(board_number(""), None);
    let gallery = gallery();
    assert!(gallery.board(0).is_none());
    assert_eq!(
        gallery.board(1).map(|board| board.name.as_str()),
        Some("Narrator opening")
    );
}

#[test]
fn a_misspelt_panel_key_is_refused() {
    let error = KitGallery::parse(r#"{"boards":[{"name":"x","panels":{"bubble":[]}}]}"#)
        .expect_err("a misspelt key is refused");
    assert!(error.to_string().contains("bubble"));
}

#[test]
fn the_gallery_fixture_is_a_quiet_stage() {
    let fixture = Fixture::parse(GALLERY_FIXTURE).expect("the gallery fixture reads");
    assert!(fixture.tv.party.is_empty());
    assert!(fixture.tv.roleplay.is_none());
    assert!(fixture.tv.narration.is_none());
}
