use super::screens::{
    CLUES_PAGE, LookCard, MAP_PAGE, NotesFilter, PageTap, PagesInput, PagesUi, TalkMic, WayCard,
    clue_viewer, panels, tab_of,
};
use super::wire::{KeptKind, MapPlace, SeatPages, Thing};
use crate::clues::{ClueBoard, ClueNote};
use crate::phone_kit::page::{Block, NodeState};
use crate::phone_kit::shell::TabKey;

// TODO(DEV-12): the Bag's item and trade cards, the Talk thread's speaker names and the send
// button's reason, the Notebook's filters and keep rows, the painting reveal (`new_painting`).

#[test]
fn every_tap_reads_back_from_its_action() {
    let taps = [
        PageTap::Say(3),
        PageTap::Address(None),
        PageTap::Address(Some("mara".to_owned())),
        PageTap::Send,
        PageTap::Filter(NotesFilter::Kind(KeptKind::Painting)),
        PageTap::Filter(NotesFilter::Recent),
        PageTap::Write,
        PageTap::Snap,
        PageTap::Keep(41),
        PageTap::Refused("plate_7".to_owned()),
        PageTap::Trade("garrick".to_owned()),
        PageTap::Mic,
        PageTap::Speaking(true),
        PageTap::Speaking(false),
        PageTap::Use("rags".to_owned()),
        PageTap::Drop("rags".to_owned()),
        PageTap::Give("painting-01".to_owned()),
        PageTap::Sell("painting-01".to_owned()),
        PageTap::Clue("seal".to_owned()),
        PageTap::Clue("close".to_owned()),
        PageTap::CloseClue,
        PageTap::ShowClue("seal".to_owned()),
        PageTap::TakeClueOff,
    ];
    for tap in taps {
        assert_eq!(PageTap::from_action(&tap.action()), Some(tap));
    }
    assert_eq!(PageTap::from_action("seat:say-1"), None);
    assert_eq!(PageTap::from_action("pages:to-"), None);
}

#[test]
fn the_map_marks_where_the_party_is_and_offers_the_asked_ways() {
    let pages = SeatPages {
        places: vec![
            MapPlace {
                id: "road".to_owned(),
                name: "The road".to_owned(),
                plate: None,
                here: false,
                refused: Some("No way back".to_owned()),
                card: None,
            },
            MapPlace {
                id: "square".to_owned(),
                name: "The square".to_owned(),
                plate: None,
                here: true,
                refused: None,
                card: None,
            },
        ],
        ..SeatPages::default()
    };
    let ways = [WayCard {
        index: 1,
        label: "The market".to_owned(),
    }];
    let input = PagesInput {
        pages: &pages,
        hero: "Rylan",
        lines: &[],
        faces: &[],
        ways: &ways,
        looks: &[],
        talk_waiting: 0,
        mic: TalkMic::default(),
        called: None,
        clues: None,
        on_table: None,
        ui: &PagesUi::default(),
    };
    let drawn = panels(MAP_PAGE, &input).expect("the map is a live page");
    let Some(Block::Map { nodes, ways }) = drawn
        .blocks
        .iter()
        .find(|block| matches!(block, Block::Map { .. }))
    else {
        panic!("the map page draws a map");
    };
    assert_eq!(nodes[1].state, NodeState::Here);
    assert_eq!(
        nodes[0].action,
        Some(PageTap::Refused("road".to_owned()).action())
    );
    assert_eq!(nodes[2].action, Some(PageTap::Say(1).action()));
    assert!(
        ways.iter()
            .any(|way| way.from == "The square" && way.to == "The market")
    );
    assert_eq!(tab_of(MAP_PAGE), Some(TabKey::Map));
    // The paintings are reached from "more": the bar lights "more", never another panel's name.
    assert_eq!(tab_of(super::screens::GALLERY_PAGE), None);
}

#[test]
fn a_thing_is_tappable_only_when_the_table_asks_to_step_up_to_it() {
    let pages = SeatPages {
        things: vec![
            Thing {
                id: "well".to_owned(),
                name: "Old well".to_owned(),
                look: None,
                picture: None,
            },
            Thing {
                id: "gate".to_owned(),
                name: "Gate".to_owned(),
                look: None,
                picture: None,
            },
        ],
        ..SeatPages::default()
    };
    let looks = [LookCard {
        index: 0,
        thing: "gate".to_owned(),
    }];
    let input = PagesInput {
        pages: &pages,
        hero: "Rylan",
        lines: &[],
        faces: &[],
        ways: &[],
        looks: &looks,
        talk_waiting: 0,
        mic: TalkMic::default(),
        called: None,
        clues: None,
        on_table: None,
        ui: &PagesUi::default(),
    };
    let drawn = panels("look", &input).expect("look closer is a live page");
    let Some(Block::Cards { cards, .. }) = drawn
        .blocks
        .iter()
        .find(|block| matches!(block, Block::Cards { .. }))
    else {
        panic!("the things are cards");
    };
    assert_eq!(cards[0].action, None);
    assert_eq!(cards[1].action, Some(PageTap::Say(0).action()));
}

#[test]
fn the_talk_page_says_how_many_lines_wait_for_the_table() {
    let (pages, ui) = (SeatPages::default(), PagesUi::default());
    let said = |talk_waiting| {
        let input = PagesInput {
            pages: &pages,
            hero: "Wren",
            lines: &[],
            faces: &[],
            ways: &[],
            looks: &[],
            talk_waiting,
            mic: TalkMic::default(),
            called: None,
            clues: None,
            on_table: None,
            ui: &ui,
        };
        format!("{:?}", panels(super::screens::TALK_PAGE, &input))
    };
    assert!(said(1).contains("1 line waiting for the next moment"));
    assert!(said(3).contains("3 lines waiting for the next moment"));
    assert!(!said(0).contains("waiting for the next moment"));
}

#[test]
fn the_talk_page_shows_the_voice_switch_and_microphone_as_the_seat_can_use_them() {
    let (pages, ui) = (SeatPages::default(), PagesUi::default());
    let drawn = |mic| {
        let input = PagesInput {
            pages: &pages,
            hero: "Wren",
            lines: &[],
            faces: &[],
            ways: &[],
            looks: &[],
            talk_waiting: 0,
            mic,
            called: None,
            clues: None,
            on_table: None,
            ui: &ui,
        };
        format!("{:?}", panels(super::screens::TALK_PAGE, &input))
    };
    let unavailable = drawn(TalkMic::Unavailable);
    assert!(!unavailable.contains("pages:mic") && !unavailable.contains("pages:speaking"));
    let off = drawn(TalkMic::Off);
    assert!(off.contains("Turn voice on") && off.contains("pages:speaking-on"));
    assert!(!off.contains("pages:mic"));
    let ready = drawn(TalkMic::Ready);
    assert!(ready.contains("\"Speak\"") && ready.contains("pages:speaking-off"));
    let recording = drawn(TalkMic::Recording);
    assert!(recording.contains("\"Stop\"") && recording.contains("Listening."));
    assert!(drawn(TalkMic::Busy).contains("enabled: false"));
}

#[test]
fn a_place_a_card_goes_back_to_sends_that_card_and_is_not_drawn_twice() {
    let pages = SeatPages {
        places: vec![
            MapPlace {
                id: "roam-03".to_owned(),
                name: "The square".to_owned(),
                plate: None,
                here: false,
                refused: None,
                card: Some(0),
            },
            MapPlace {
                id: "roam-04".to_owned(),
                name: "The mill".to_owned(),
                plate: None,
                here: true,
                refused: None,
                card: None,
            },
        ],
        ..SeatPages::default()
    };
    let ways = [
        WayCard {
            index: 0,
            label: "Back to the square".to_owned(),
        },
        WayCard {
            index: 1,
            label: "The ford".to_owned(),
        },
    ];
    let input = PagesInput {
        pages: &pages,
        hero: "Rylan",
        lines: &[],
        faces: &[],
        ways: &ways,
        looks: &[],
        talk_waiting: 0,
        mic: TalkMic::default(),
        called: None,
        clues: None,
        on_table: None,
        ui: &PagesUi::default(),
    };
    let drawn = panels(MAP_PAGE, &input).expect("the map is a live page");
    let Some(Block::Map { nodes, .. }) = drawn
        .blocks
        .iter()
        .find(|block| matches!(block, Block::Map { .. }))
    else {
        panic!("the map page draws a map");
    };
    assert_eq!(nodes.len(), 3, "{nodes:?}");
    assert_eq!(nodes[0].action, Some(PageTap::Say(0).action()));
    assert_eq!(nodes[2].action, Some(PageTap::Say(1).action()));
}

fn board() -> ClueBoard {
    let note = |id: &str, picture: Option<&str>| ClueNote {
        id: id.to_owned(),
        name: format!("The {id}"),
        detail: format!("What the {id} shows."),
        worked_out: false,
        picture: picture.map(str::to_owned),
    };
    ClueBoard {
        notes: vec![
            note("seal", Some("seal.png")),
            note("map", None),
            note("ring", None),
        ],
    }
}

#[test]
fn a_clue_opens_full_screen_with_its_steps_text_and_table_buttons() {
    let board = board();
    // The board's order is the newest first: ring, map, seal.
    let open = clue_viewer(&board, "map", None, None).expect("the clue is on the board");
    assert_eq!(open.viewer.index, 1);
    assert_eq!(open.viewer.count, 3);
    assert_eq!(
        open.viewer.previous,
        Some(PageTap::Clue("ring".to_owned()).action())
    );
    assert_eq!(
        open.viewer.next,
        Some(PageTap::Clue("seal".to_owned()).action())
    );
    assert_eq!(open.viewer.close, PageTap::CloseClue.action());
    assert_eq!(open.text, "What the map shows.");
    let actions: Vec<_> = open
        .viewer
        .item
        .buttons
        .iter()
        .filter_map(|button| button.action.clone())
        .collect();
    assert_eq!(actions, vec![PageTap::ShowClue("map".to_owned()).action()]);

    // While another clue is on the table, any phone may take it off; the one shown is not offered
    // again.
    let shown = clue_viewer(
        &board,
        "seal",
        Some("seal"),
        Some("Nothing is on the table."),
    )
    .expect("the clue is on the board");
    assert_eq!(shown.viewer.item.picture, "seal.png");
    assert_eq!(shown.problem.as_deref(), Some("Nothing is on the table."));
    let actions: Vec<_> = shown
        .viewer
        .item
        .buttons
        .iter()
        .filter_map(|button| button.action.clone())
        .collect();
    assert_eq!(actions, vec![PageTap::TakeClueOff.action()]);
    let other = clue_viewer(&board, "ring", Some("seal"), None).expect("on the board");
    assert_eq!(other.viewer.item.buttons.len(), 2);
    assert!(clue_viewer(&board, "gone", None, None).is_none());
}

#[test]
fn the_clues_page_offers_to_take_the_shown_clue_off_the_table() {
    let pages = SeatPages::default();
    let board = board();
    let ui = PagesUi::default();
    let input = |on_table| PagesInput {
        pages: &pages,
        hero: "Rylan",
        lines: &[],
        faces: &[],
        ways: &[],
        looks: &[],
        talk_waiting: 0,
        mic: TalkMic::default(),
        called: None,
        clues: Some(&board),
        on_table,
        ui: &ui,
    };
    let buttons = |drawn: crate::phone_kit::PhonePanels| {
        drawn
            .blocks
            .into_iter()
            .filter_map(|block| match block {
                Block::Buttons { buttons } => Some(buttons),
                _ => None,
            })
            .flatten()
            .filter_map(|button| button.action)
            .collect::<Vec<_>>()
    };
    let idle = panels(CLUES_PAGE, &input(None)).expect("a live page");
    assert!(buttons(idle).is_empty());
    let shown = panels(CLUES_PAGE, &input(Some("seal"))).expect("a live page");
    assert_eq!(buttons(shown), vec![PageTap::TakeClueOff.action()]);
}

#[test]
fn the_shop_page_buys_by_the_bundle_haggles_off_catalog_and_shows_the_load() {
    use super::screens::shop_page;
    use super::wire::{BagLoad, ContainerShown, ShopShown, StockShown};
    let line = |item: &str, per: u32, in_stock: u64, buyable: bool| StockShown {
        item: item.to_owned(),
        name: item.to_owned(),
        in_stock,
        price: "1 gp".to_owned(),
        per,
        weight: Some("1 lb".to_owned()),
        buyable,
    };
    let pages = SeatPages {
        shops: vec![ShopShown {
            id: "garrick".to_owned(),
            keeper: "Garrick".to_owned(),
            lines: vec![
                line("arrow", 20, 40, true),
                line("rope", 1, 0, true),
                line("charm", 1, 3, false),
            ],
        }],
        load: Some(BagLoad {
            weight: "62 lb".to_owned(),
            capacity: Some("150 lb".to_owned()),
            over: false,
            containers: vec![ContainerShown {
                id: "backpack".to_owned(),
                name: "Backpack".to_owned(),
                count: 1,
                load: "30 lb".to_owned(),
                capacity: Some("30 lb".to_owned()),
                full: true,
                shares: None,
            }],
            unweighed: Vec::new(),
        }),
        ..SeatPages::default()
    };
    let ui = PagesUi::default();
    let input = PagesInput {
        pages: &pages,
        hero: "Rylan",
        lines: &[],
        faces: &[],
        ways: &[],
        looks: &[],
        talk_waiting: 0,
        mic: TalkMic::default(),
        called: None,
        clues: None,
        on_table: None,
        ui: &ui,
    };
    assert_eq!(tab_of(&shop_page("garrick")), Some(TabKey::Act));
    let drawn = panels(&shop_page("garrick"), &input).expect("a shop is a live page");
    let rows: Vec<_> = drawn
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Rows { rows } => Some(rows.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    let buy = PageTap::Buy {
        shop: "garrick".to_owned(),
        item: "arrow".to_owned(),
        qty: 20,
    };
    assert_eq!(rows[0].action, Some(buy.action()));
    assert_eq!(PageTap::from_action(&buy.action()), Some(buy));
    assert!(rows[1].action.is_none(), "sold out");
    assert_eq!(
        rows[2].action,
        Some(PageTap::Trade("garrick".to_owned()).action())
    );
    assert!(drawn.blocks.iter().any(|block| matches!(
        block,
        Block::Stats { stats } if stats.iter().any(|stat| stat.value == "62 lb of 150 lb")
    )));
    // The Bag shows the load and marks a full container.
    let bag = panels(super::screens::BAG_PAGE, &input).expect("the bag is a live page");
    assert!(bag.blocks.iter().any(|block| matches!(
        block,
        Block::Rows { rows } if rows.iter().any(|row| row.label == "Backpack"
            && row.value.as_deref() == Some("30 lb of 30 lb")
            && row.mark.as_ref().is_some_and(|mark| mark.text == "Full"))
    )));
}

#[test]
fn a_waiting_question_marks_act_and_a_called_roll_marks_roll_on_the_other_pages() {
    use super::screens::TableWaits;
    let waits = TableWaits {
        ask: Some("Which road?".to_owned()),
        roll: Some("Perception check".to_owned()),
    };
    let marked: Vec<_> = waits
        .badges()
        .into_iter()
        .filter(|badge| badge.waiting)
        .map(|badge| badge.tab)
        .collect();
    assert_eq!(marked, vec![TabKey::Act, TabKey::Roll]);
    let banner = waits.banner("story").expect("Story names the question");
    assert_eq!(banner.text, "The table asks: Which road? — tap to answer");
    assert_eq!(
        banner.action,
        crate::phone_kit::nav::page_action(TabKey::Act.page_key())
    );
    assert!(waits.banner("act").is_none());
    let roll_only = TableWaits {
        ask: None,
        roll: Some("Perception check".to_owned()),
    };
    assert!(roll_only.banner("roll").is_none());
    assert_eq!(
        roll_only.banner("talk").map(|banner| banner.text),
        Some("The table calls for Perception check — tap to roll".to_owned())
    );
    assert!(TableWaits::default().badges().is_empty());
    assert!(TableWaits::default().banner("story").is_none());
}

#[test]
fn the_clues_page_lists_each_note_of_the_board_as_a_card_that_opens_it() {
    let pages = SeatPages::default();
    let board = ClueBoard {
        notes: vec![ClueNote {
            id: "shrine_verse".to_owned(),
            name: "The shrine's verse".to_owned(),
            detail: "Ring thrice where the river bends.".to_owned(),
            worked_out: false,
            picture: Some("shrine-verse.png".to_owned()),
        }],
    };
    let ui = PagesUi::default();
    let input = |clues| PagesInput {
        pages: &pages,
        hero: "Rylan",
        lines: &[],
        faces: &[],
        ways: &[],
        looks: &[],
        talk_waiting: 0,
        mic: TalkMic::default(),
        called: None,
        clues,
        on_table: None,
        ui: &ui,
    };
    let drawn = panels(CLUES_PAGE, &input(Some(&board))).expect("a live page");
    let cards: Vec<_> = drawn
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Cards { cards, .. } => Some(cards.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].title, "The shrine's verse");
    assert_eq!(
        cards[0].sub.as_deref(),
        Some("Ring thrice where the river bends.")
    );
    assert_eq!(cards[0].picture.as_deref(), Some("shrine-verse.png"));
    assert_eq!(
        cards[0].action,
        Some(PageTap::Clue("shrine_verse".to_owned()).action())
    );
    assert!(drawn.empty.is_none());
    let none = panels(CLUES_PAGE, &input(None)).expect("a live page");
    assert!(none.empty.is_some(), "no board says so");
}
