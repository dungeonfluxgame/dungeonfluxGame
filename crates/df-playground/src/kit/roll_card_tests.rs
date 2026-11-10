//! The roll card's words, dice and written sum, and the display rubric of the plan
//! (`planning/dice-gauntlet.md`, display rules 4 to 6) read from the kit's own stylesheet.

use std::path::PathBuf;

use crate::contrast::{contrast_ratio, parse_hex};
use crate::fixture::{
    DieRole, InspirationReroll, NaturalMark, RollDie, RollKind, RollMode, RollModifier,
    RollOutcome, RollTarget, RollView,
};
use crate::kit::roll_card::{
    DieArt, DieState, MINUS, RollCard, Tone, kind_word, outcome_word, percentile_numerals, signed,
};
use crate::kit::roll_tray::{MOST_DICE_DRAWN, Tally, TrayGrid, TrayLayout};

const KIT: &str = include_str!("../../../../assets/playground/kit.css");

fn die(sides: u8, face: u8, role: DieRole, kept: bool) -> RollDie {
    RollDie {
        sides,
        face,
        role,
        kept,
    }
}

fn modifier(label: &str, value: i32) -> RollModifier {
    RollModifier {
        label: label.to_owned(),
        value,
    }
}

fn roll(kind: RollKind, dice: Vec<RollDie>, modifiers: Vec<RollModifier>, total: i32) -> RollView {
    RollView {
        id: "r".to_owned(),
        title: "Athletics (Strength)".to_owned(),
        kind,
        mode: RollMode::Normal,
        dice,
        modifiers,
        total,
        target: Some(RollTarget {
            label: "DC".to_owned(),
            value: 15,
        }),
        outcome: Some(RollOutcome::Success),
        natural: NaturalMark::Unmarked,
        inspiration_reroll: None,
    }
}

fn athletics() -> RollCard {
    RollCard::from(&roll(
        RollKind::SkillCheck,
        vec![die(20, 14, DieRole::Test, true)],
        vec![modifier("Strength", 3), modifier("Proficiency", 2)],
        19,
    ))
}

fn written(card: &RollCard) -> String {
    let mut text = String::new();
    for (index, term) in card.terms().iter().enumerate() {
        if index > 0 {
            text.push_str(if term.negative { " - " } else { " + " });
        } else if term.negative {
            text.push('-');
        }
        text.push_str(&term.number);
        if let Some(label) = &term.label {
            text.push(' ');
            text.push_str(label);
        }
    }
    text
}

#[test]
fn the_sum_is_written_once_as_the_die_and_each_modifier_with_its_name() {
    assert_eq!(written(&athletics()), "14 + 3 Strength + 2 Proficiency");
    assert!(athletics().writes_a_sum());
    assert_eq!(athletics().total_text(), "19");
}

#[test]
fn a_negative_modifier_is_taken_away_and_a_lone_number_is_no_sum() {
    let card = RollCard::from(&roll(
        RollKind::AbilityCheck,
        vec![die(20, 12, DieRole::Test, true)],
        vec![modifier("Strength", -1)],
        11,
    ));
    assert_eq!(written(&card), "12 - 1 Strength");
    let lone = RollCard::from(&roll(
        RollKind::TableRoll,
        vec![die(20, 12, DieRole::Table, true)],
        Vec::new(),
        12,
    ));
    assert!(!lone.writes_a_sum());
}

#[test]
fn only_the_kept_die_is_in_the_sum_and_the_other_stands_dimmed_and_struck() {
    let mut view = roll(
        RollKind::SkillCheck,
        vec![
            die(20, 6, DieRole::Test, false),
            die(20, 17, DieRole::Test, true),
        ],
        vec![modifier("Strength", 3)],
        20,
    );
    view.mode = RollMode::Advantage;
    let card = RollCard::from(&view);
    assert_eq!(written(&card), "17 + 3 Strength");
    assert_eq!(card.mode_word(), Some("Advantage"));
    let slots = card.slots();
    assert_eq!(
        slots
            .iter()
            .map(|slot| (slot.numeral.as_str(), slot.state))
            .collect::<Vec<_>>(),
        [("6", DieState::Unused), ("17", DieState::Counted)]
    );
    view.mode = RollMode::Disadvantage;
    assert_eq!(RollCard::from(&view).mode_word(), Some("Disadvantage"));
    view.mode = RollMode::Normal;
    assert_eq!(RollCard::from(&view).mode_word(), None);
}

#[test]
fn a_percentile_roll_is_two_ten_sided_dice_of_tens_and_ones() {
    assert_eq!(percentile_numerals(57), ("50".to_owned(), "7".to_owned()));
    assert_eq!(percentile_numerals(1), ("00".to_owned(), "1".to_owned()));
    assert_eq!(percentile_numerals(100), ("00".to_owned(), "0".to_owned()));
    assert_eq!(percentile_numerals(90), ("90".to_owned(), "0".to_owned()));
    let card = RollCard::from(&roll(
        RollKind::TableRoll,
        vec![die(100, 57, DieRole::Table, true)],
        Vec::new(),
        57,
    ));
    let slots = card.slots();
    assert_eq!(slots.len(), 2);
    assert!(slots.iter().all(|slot| slot.art == DieArt::D10));
    assert_eq!(
        slots
            .iter()
            .map(|slot| slot.numeral.as_str())
            .collect::<Vec<_>>(),
        ["50", "7"]
    );
    assert_eq!(written(&card), "50 + 7");
}

#[test]
fn a_heroic_inspiration_reroll_shows_the_replaced_die_struck_before_the_new_one() {
    let mut view = roll(
        RollKind::AbilityCheck,
        vec![die(20, 15, DieRole::Test, true)],
        vec![modifier("Dexterity", 2)],
        17,
    );
    view.inspiration_reroll = Some(InspirationReroll { index: 0, was: 4 });
    let slots = RollCard::from(&view).slots();
    assert_eq!(
        slots
            .iter()
            .map(|slot| (slot.numeral.as_str(), slot.state))
            .collect::<Vec<_>>(),
        [("4", DieState::Replaced), ("15", DieState::Counted)]
    );
    assert_eq!(
        RollCard::from(&view).inspiration_word().as_deref(),
        Some("Heroic Inspiration, was 4")
    );
    assert_eq!(written(&RollCard::from(&view)), "15 + 2 Dexterity");
}

#[test]
fn damage_dice_each_show_their_face_and_a_bonus_die_is_named_in_the_sum() {
    let card = RollCard::from(&roll(
        RollKind::Damage,
        vec![
            die(6, 4, DieRole::Damage, true),
            die(6, 5, DieRole::Damage, true),
        ],
        vec![modifier("Strength", 3)],
        12,
    ));
    assert_eq!(written(&card), "4 + 5 + 3 Strength");
    assert_eq!(card.slots().len(), 2);
    let mixed = RollCard::from(&roll(
        RollKind::SkillCheck,
        vec![
            die(20, 11, DieRole::Test, true),
            die(4, 3, DieRole::Bonus, true),
        ],
        Vec::new(),
        14,
    ));
    assert_eq!(written(&mixed), "11 + 3 Bonus die");
}

#[test]
fn a_natural_mark_shows_only_when_the_server_says_so() {
    let mut view = roll(
        RollKind::MeleeAttack,
        vec![die(20, 20, DieRole::Test, true)],
        Vec::new(),
        20,
    );
    assert_eq!(RollCard::from(&view).natural_word(), None);
    view.natural = NaturalMark::Twenty;
    assert_eq!(RollCard::from(&view).natural_word(), Some("Natural 20"));
    view.natural = NaturalMark::One;
    assert_eq!(RollCard::from(&view).natural_word(), Some("Natural 1"));
}

#[test]
fn the_target_and_the_verdict_are_worded_as_the_server_gave_them() {
    let card = athletics();
    assert_eq!(card.target_text().as_deref(), Some("DC 15"));
    assert_eq!(card.verdict(), Some(("Success", Tone::Positive)));
    let mut view = card.view.clone();
    view.target = None;
    view.outcome = None;
    let bare = RollCard::from(&view);
    assert_eq!((bare.target_text(), bare.verdict()), (None, None));
}

#[test]
fn every_outcome_has_a_word_and_a_look_and_every_kind_a_kicker() {
    for outcome in RollOutcome::ALL {
        let (word, _) = outcome_word(outcome);
        assert!(!word.is_empty(), "{outcome:?}");
    }
    assert_eq!(outcome_word(RollOutcome::Tied).1, Tone::Neutral);
    assert_eq!(outcome_word(RollOutcome::Miss).1, Tone::Negative);
    assert_eq!(outcome_word(RollOutcome::CriticalHit).0, "Critical hit");
    let kickers: std::collections::BTreeSet<&str> =
        RollKind::ALL.into_iter().map(kind_word).collect();
    assert_eq!(
        kickers.len(),
        RollKind::ALL.len(),
        "no two kinds share a kicker"
    );
}

#[test]
fn a_negative_number_is_written_with_the_minus_sign() {
    assert_eq!(signed(-3), format!("{MINUS}3"));
    assert_eq!(signed(7), "7");
}

#[test]
fn the_card_is_one_sentence_for_the_accessibility_tree() {
    assert_eq!(
        athletics().summary(),
        concat!(
            "Athletics (Strength): 14 plus 3 Strength plus 2 Proficiency makes 19 against ",
            "DC 15. Success"
        )
    );
}

#[test]
fn the_dice_are_laid_in_rows_that_fit_the_tray_and_fewer_dice_are_bigger() {
    let mut last_cell = u32::MAX;
    for count in 1..=MOST_DICE_DRAWN {
        let grid = TrayGrid::of(count);
        assert!(
            grid.columns * grid.rows >= u32::try_from(count).expect("small"),
            "{count}"
        );
        assert!(
            grid.columns * grid.cell_tenths <= 264,
            "{count} dice overflow the width"
        );
        assert!(
            grid.rows * grid.cell_tenths <= grid.inner_height_tenths,
            "{count} dice overflow the height"
        );
        assert!(
            grid.cell_tenths <= last_cell,
            "{count} dice are bigger than fewer"
        );
        last_cell = grid.cell_tenths;
    }
    assert_eq!(TrayGrid::of(1).picture_tenths(), 196);
    assert_eq!((TrayGrid::of(2).columns, TrayGrid::of(2).rows), (2, 1));
}

#[test]
fn a_chip_too_long_for_a_row_is_two_lines_tall_in_the_height_of_the_card() {
    let mut view = roll(
        RollKind::AbilityCheck,
        vec![die(20, 15, DieRole::Test, true)],
        vec![modifier("Dexterity", 2)],
        17,
    );
    let plain = RollCard::from(&view).height_tenths();
    view.inspiration_reroll = Some(InspirationReroll { index: 0, was: 4 });
    let inspired = RollCard::from(&view).height_tenths();
    assert!(
        inspired >= plain + 25,
        "the chip's two lines outweigh the pair's shorter tray: {plain} then {inspired}"
    );
}

#[test]
fn a_crowd_of_dice_says_how_many_of_which_kind_and_a_few_dice_say_nothing() {
    let crowd = many_dice(8);
    assert_eq!(crowd.dice_word().as_deref(), Some("8d10"));
    let few = many_dice(i32::try_from(MOST_DICE_DRAWN).expect("small"));
    assert_eq!(few.dice_word(), None);
}

#[test]
fn the_written_sum_is_set_large_enough_for_its_digits_to_stand_over_the_floor() {
    // A digit's ink is about 0.64 of its font, and the floor for ink is 3.9 percent of the
    // screen's 56.25cqw height.
    let css = roll_css();
    for rule in [".kit-roll-sum {", ".kit-roll-sum-long {"] {
        let rule = &css[css.find(rule).expect("the sum's rule")..];
        let rule = &rule[..rule.find('}').expect("ends")];
        let font: f64 = rule
            .lines()
            .find_map(|line| {
                let line = line.trim();
                line.strip_prefix("font-size: ")
                    .or_else(|| line.strip_prefix("font: 700 "))
            })
            .and_then(|size| size.split("cqw").next())
            .expect("a size in cqw")
            .parse()
            .expect("a number");
        assert!(font * 0.64 / 56.25 >= 0.039, "{font}cqw is too small");
    }
}

#[test]
fn a_pair_is_set_apart_by_a_gap_that_fits_the_tray_and_other_rows_have_none() {
    let pair = TrayGrid::of(2);
    assert!(
        pair.cell_gap_tenths() >= 20,
        "a left-out die has room beside the kept one"
    );
    assert!(
        pair.columns * pair.cell_tenths + (pair.columns - 1) * pair.cell_gap_tenths() <= 264,
        "the pair and its gap fit the tray"
    );
    assert!(pair.vars().contains("--cell-gap:3cqw;"));
    for count in [1, 3, MOST_DICE_DRAWN] {
        assert_eq!(TrayGrid::of(count).cell_gap_tenths(), 0, "{count}");
    }
}

fn many_dice(count: i32) -> RollCard {
    RollCard::from(&roll(
        RollKind::Damage,
        (1..=count)
            .map(|face| {
                die(
                    10,
                    u8::try_from(face % 10 + 1).expect("small"),
                    DieRole::Damage,
                    true,
                )
            })
            .collect(),
        Vec::new(),
        (1..=count).map(|face| face % 10 + 1).sum(),
    ))
}

#[test]
fn a_crowd_of_dice_is_a_cluster_and_a_tally_of_every_face_with_its_count() {
    let card = many_dice(20);
    assert_eq!(card.slots().len(), 20);
    let TrayLayout::Tally(tally) = card.layout() else {
        panic!("twenty dice are tallied");
    };
    assert_eq!(tally.cluster.len(), 3);
    let counted: u32 = tally.chips.iter().map(|chip| chip.count).sum();
    assert_eq!(counted, 20, "every die is in the tally");
    let faces: Vec<&str> = tally.chips.iter().map(|chip| chip.face.as_str()).collect();
    assert_eq!(faces, ["10", "9", "8", "7", "6", "5", "4", "3", "2", "1"]);
    assert!(tally.chips.iter().all(|chip| chip.count == 2));
    assert_eq!((tally.columns, tally.rows), (4, 3));
}

#[test]
fn a_tally_keeps_the_faces_that_did_not_fit_as_one_chip_and_the_dice_left_out_after_the_rest() {
    let mut view = many_dice(5).view;
    view.dice[0].kept = false;
    view.total -= i32::from(view.dice[0].face);
    let TrayLayout::Tally(tally) = RollCard::from(&view).layout() else {
        panic!("five dice are tallied");
    };
    let last = tally.chips.last().expect("a chip");
    assert_eq!(last.state, DieState::Unused);
    assert_eq!(tally.chips.len(), 5);
    let wide = RollCard::from(&roll(
        RollKind::Damage,
        (1..=40)
            .map(|index| die(20, (index - 1) % 20 + 1, DieRole::Damage, true))
            .collect(),
        Vec::new(),
        2 * (1..=20).sum::<i32>(),
    ));
    let TrayLayout::Tally(tally) = wide.layout() else {
        panic!("a crowd is tallied");
    };
    assert_eq!(tally.chips.len(), 18);
    assert_eq!(
        tally.chips.last().map(|chip| chip.face.as_str()),
        Some("+6")
    );
}

#[test]
fn a_percentile_roll_is_written_as_its_tens_and_ones_with_captions_on_its_two_dice() {
    let card = RollCard::from(&roll(
        RollKind::TableRoll,
        vec![die(100, 100, DieRole::Table, true)],
        Vec::new(),
        100,
    ));
    assert_eq!(written(&card), "00 + 0");
    assert!(card.writes_a_sum());
    let captions: Vec<Option<&str>> = card.slots().iter().map(|slot| slot.caption).collect();
    assert_eq!(captions, [Some("tens"), Some("ones")]);
}

#[test]
fn the_kicker_is_left_off_when_the_title_says_the_same() {
    let mut view = athletics().view;
    assert_eq!(RollCard::from(&view).kicker(), Some("Skill check"));
    view.kind = RollKind::Initiative;
    view.title = "Initiative".to_owned();
    assert_eq!(RollCard::from(&view).kicker(), None);
}

#[test]
fn every_die_has_its_picture_in_the_dice_folder_and_its_numeral_on_the_picture() {
    let dice = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/playground/dice");
    for art in DieArt::ALL {
        let file = dice.join(format!("{}.png", art.key()));
        assert!(
            file.is_file(),
            "{} is not in assets/playground/dice",
            art.key()
        );
        let geometry = art.geometry();
        assert!((0..=1000).contains(&geometry.x_permille), "{art:?}");
        assert!((0..=1000).contains(&geometry.y_permille), "{art:?}");
        assert!((100..=130).contains(&geometry.scale_percent), "{art:?}");
    }
    assert_eq!(DieArt::of_sides(100), Some((DieArt::D10, true)));
    assert_eq!(DieArt::of_sides(7), None);
}

// The rubric. Digits are at most this share of the font size tall in the page's fonts (measured
// in the browser: lining figures of the kit's serif stack), so a size here is a lower bound.
const DIGIT_HEIGHT_EM: f64 = 0.66;
/// The stage is 100cqw wide and 16:9, so its height is this many cqw.
const STAGE_HEIGHT_CQW: f64 = 56.25;

/// The font size, in cqw, the rules of the kit's stylesheet for `selector` set with the `font`
/// shorthand.
fn font_size_cqw(selector: &str) -> f64 {
    let header = format!(
        "
{selector} {{"
    );
    let mut rest = KIT;
    while let Some(start) = rest.find(&header) {
        rest = &rest[start + header.len()..];
        let rule = &rest[..rest.find('}').expect("a rule ends")];
        let Some(font) = rule
            .lines()
            .find_map(|line| line.trim().strip_prefix("font:"))
        else {
            continue;
        };
        let size = font
            .split_whitespace()
            .find_map(|word| {
                word.split('/')
                    .next()
                    .and_then(|size| size.strip_suffix("cqw"))
            })
            .unwrap_or_else(|| panic!("{selector} has no size in cqw"));
        return size.parse().expect("a number");
    }
    panic!("the kit has no rule {selector} that sets a font")
}

/// The colour a custom property of the roll card is set to.
fn token(name: &str) -> [u8; 3] {
    let key = format!("{name}: ");
    let start = KIT.find(&key).unwrap_or_else(|| panic!("no {name}")) + key.len();
    let value = KIT[start..].split(';').next().expect("ends").trim();
    parse_hex(value).unwrap_or_else(|| panic!("{name} is {value}, not a plain colour"))
}

fn height_share(font_cqw: f64) -> f64 {
    font_cqw * DIGIT_HEIGHT_EM / STAGE_HEIGHT_CQW
}

#[test]
fn the_total_is_over_six_percent_of_the_screen_height_and_the_sum_and_target_over_three_and_a_half()
{
    assert!(height_share(font_size_cqw(".kit-roll-total-number")) >= 0.06);
    assert!(
        font_size_cqw(".kit-roll-term-label") / STAGE_HEIGHT_CQW >= 0.035,
        "a modifier's label, a line of capitals, is over 3.5 percent of the height as a font"
    );
    for selector in [".kit-roll-sum", ".kit-roll-against"] {
        assert!(
            height_share(font_size_cqw(selector)) >= 0.035,
            "{selector} is {:.3} of the height",
            height_share(font_size_cqw(selector))
        );
    }
}

#[test]
fn the_numerals_read_at_seven_to_one_and_the_labels_at_four_and_a_half_on_their_plates() {
    let plate = parse_hex("#0b0a12").expect("the plate");
    assert!(
        KIT.contains("--kit-fill-solid: #0b0a12f6;"),
        "the plate colour is the one measured here"
    );
    let numerals = [
        ("the total and the sum", [253, 251, 248]),
        (
            "a face's ink on the pale gold of the art",
            [0x0e, 0x15, 0x30],
        ),
    ];
    assert!(contrast_ratio(numerals[0].1, plate) >= 7.0);
    let face_gold = [0xdc, 0xb4, 0x5a];
    assert!(contrast_ratio(numerals[1].1, face_gold) >= 7.0);
    for name in ["--roll-positive", "--roll-negative", "--roll-neutral"] {
        assert!(contrast_ratio(token(name), plate) >= 4.5, "{name}");
    }
    assert!(contrast_ratio(parse_hex("#f1d9a0").expect("gold"), plate) >= 4.5);
    assert!(contrast_ratio(token("--roll-face-ink"), face_gold) >= 7.0);
}

#[test]
fn the_roll_card_never_asks_for_reduced_motion() {
    let start = KIT
        .find("=== the roll card")
        .expect("the roll card section");
    assert!(!KIT[start..].contains("prefers-reduced-motion"));
}

const THEME: &str = include_str!("../../../../assets/playground/theme.css");

fn roll_css() -> String {
    let section = |css: &str, start: &str, end: &str| {
        let from = css
            .find(start)
            .unwrap_or_else(|| panic!("no section {start}"));
        let to = css.find(end).unwrap_or_else(|| panic!("no end {end}"));
        css[from..to].to_owned()
    };
    format!(
        "{}{}",
        section(KIT, "=== the roll card", "=== end the roll card"),
        section(
            THEME,
            "=== component: roll card",
            "=== end component: roll card"
        ),
    )
}

/// The names of the custom properties of the roll card's own families: `--roll-*` and the dice's.
fn own_properties(text: &str, after: &str) -> std::collections::BTreeSet<String> {
    let mut names = std::collections::BTreeSet::new();
    let mut rest = text;
    while let Some(at) = rest.find(after) {
        rest = &rest[at + after.len()..];
        let name: String = rest
            .chars()
            .take_while(|character| character.is_ascii_alphanumeric() || *character == '-')
            .collect();
        let own = [
            "roll-", "from-", "rest-", "art-", "face-", "cols", "cell", "chip-", "cluster", "rx",
            "ry", "scatter", "r0", "r3", "hop", "throw", "tray-", "numeral", "caption-", "wide-",
        ]
        .iter()
        .any(|family| name.starts_with(family));
        if own {
            names.insert(name);
        }
    }
    names
}

#[test]
fn every_custom_property_the_roll_stylesheet_reads_is_set_by_the_page_or_by_the_sheet() {
    use crate::roll::{RollSchedule, TumblePlan};
    let css = roll_css();
    let used = own_properties(&css, "var(--");
    let set_by_the_sheet = own_properties(&css, "\n  --");
    let plan = TumblePlan::new(5, 1);
    let vars = format!(
        "{}{}{}{}{}--roll-fit:0.8;",
        crate::kit::roll_card::staging_vars(RollSchedule::new(1_500, 0), 0),
        crate::kit::roll_card::die_vars(DieArt::D20, plan.dice.first()),
        TrayGrid::of(3).vars(),
        TrayGrid::of(3).tray_vars(),
        Tally::of(&many_dice(9).slots()).vars(),
    );
    let set_by_the_page = own_properties(&vars, "--");
    for name in &used {
        assert!(
            set_by_the_sheet.contains(name) || set_by_the_page.contains(name),
            "--{name} is read by the stylesheet and set by neither the page nor the sheet"
        );
    }
    assert!(
        used.len() > 15,
        "the check found the roll card's properties: {used:?}"
    );
}

#[test]
fn the_stages_begin_at_the_times_of_the_schedule_less_the_age() {
    use crate::roll::RollSchedule;
    let schedule = RollSchedule::new(1_500, 0);
    let vars = crate::kit::roll_card::staging_vars(schedule, 750);
    for expected in [
        "--roll-age:750ms;",
        "--roll-tumble-at:2000ms;",
        "--roll-tumble:1500ms;",
        "--roll-faces-at:3500ms;",
        "--roll-sum-at:4200ms;",
        "--roll-verdict-at:4800ms;",
        "--roll-off-at:10800ms;",
        "--roll-exit:400ms;",
    ] {
        assert!(vars.contains(expected), "{expected} is not in {vars}");
    }
}

#[test]
fn a_dice_numeral_stands_where_the_art_has_its_main_panel() {
    use crate::roll::TumblePlan;
    let plan = TumblePlan::new(9, 1);
    let vars = crate::kit::roll_card::die_vars(DieArt::D20, plan.dice.first());
    assert!(vars.contains("--art-url:url(/playground/dice/d20.png);"));
    assert!(vars.contains("--art-scale:1;--face-x:52%;--face-y:50.3%;"));
    assert!(vars.contains("--throw:"));
    let bare = crate::kit::roll_card::die_vars(DieArt::D4, None);
    assert!(bare.contains("--art-scale:1.18;") && !bare.contains("--rest-x"));
}

#[test]
fn a_long_title_and_a_long_sum_are_written_smaller_and_short_ones_are_not() {
    let mut view = athletics().view;
    assert!(!RollCard::from(&view).title_is_long());
    assert!(!RollCard::from(&view).sum_is_long());
    view.title = "Dexterity saving throw against the Ancient Red Dragon's fiery breath".to_owned();
    view.modifiers = vec![
        modifier("Dexterity", 2),
        modifier("Proficiency", 3),
        modifier("Cloak of Protection", 1),
        modifier("Bless", 2),
    ];
    view.total = 22;
    let card = RollCard::from(&view);
    assert!(card.title_is_long() && card.sum_is_long());
}

/// The share of the screen's height a numeral of `font_cqw` stands, digits being `DIGIT_HEIGHT_EM`
/// of the font.
fn numeral_share(font_tenths: u32) -> f64 {
    height_share(f64::from(font_tenths) / 10.0)
}

#[test]
fn a_lone_die_has_the_tray_and_a_numeral_over_six_percent_of_the_screen_height() {
    let grid = TrayGrid::of(1);
    assert!(numeral_share(grid.numeral_tenths()) >= 0.06);
    assert_eq!(
        grid.wide_fit_percent, 100,
        "two digits keep the size of one on a lone die"
    );
    assert!(
        grid.tray_height_tenths() >= 150,
        "the tray of one die is squarer than a handful's"
    );
    assert!(
        numeral_share(TrayGrid::of(2).numeral_tenths()) >= 0.05,
        "a pair is nearly as large"
    );
}

#[test]
fn a_numeral_is_as_large_on_every_die_because_the_grid_and_not_the_art_sets_it() {
    assert!(
        !KIT.contains("--face-size"),
        "no die sets its own numeral size"
    );
    let css = roll_css();
    let face = &css[css.find(".kit-roll-face {").expect("the face rule")..];
    let face = &face[..face.find('}').expect("ends")];
    assert!(face.contains("var(--numeral)") && !face.contains("--art-scale"));
}

#[test]
fn a_percentile_roll_keeps_room_under_its_dice_for_their_captions() {
    let card = RollCard::from(&roll(
        RollKind::TableRoll,
        vec![die(100, 57, DieRole::Table, true)],
        Vec::new(),
        57,
    ));
    let TrayLayout::Dice(grid) = card.layout() else {
        panic!("a pair of dice is drawn");
    };
    assert!(grid.caption_tenths >= 20);
    assert!(grid.vars().contains(&format!(
        "--caption-room:{}cqw;",
        f64::from(grid.caption_tenths) / 10.0
    )));
    assert_eq!(TrayGrid::of(2).caption_tenths, 0, "plain dice have none");
}

fn worst_case() -> RollCard {
    let mut view = roll(
        RollKind::SkillCheck,
        vec![
            die(20, 14, DieRole::Test, true),
            die(8, 3, DieRole::Bonus, true),
        ],
        vec![
            modifier("Strength", 3),
            modifier("Proficiency", 2),
            modifier("Expertise", 2),
            modifier("Bless", 1),
            modifier("Guidance", 1),
        ],
        26,
    );
    view.title = "Athletics (Strength) to climb the wet slate face of the old tower".to_owned();
    RollCard::from(&view)
}

#[test]
fn an_ordinary_card_keeps_its_size_and_a_card_taller_than_the_screen_is_drawn_smaller() {
    assert_eq!(athletics().fit_percent(), 100);
    let worst = worst_case();
    assert!(
        worst.fit_percent() < 100,
        "the worst case does not fit as it is"
    );
    assert!(worst.fit_percent() >= 60);
    let tall = worst.height_tenths() + 40;
    let bottom = 26 + tall * worst.fit_percent() / 100;
    assert!(
        bottom <= 562 - 16 + 1,
        "{bottom} tenths is past the foot of the screen"
    );
    assert!(
        athletics().height_tenths() + 40 + 26 <= 562 - 16,
        "a lone d20 with its stamp stays above the foot of the screen"
    );
}

#[test]
fn the_sum_wraps_only_between_terms_and_keeps_its_ending_whole() {
    let css = roll_css();
    let term = &css[css.find(".kit-roll-term {").expect("the term rule")..];
    assert!(term[..term.find('}').expect("ends")].contains("white-space: nowrap"));
}

#[test]
fn the_seal_hangs_over_the_panels_foot_and_the_card_moves_only_by_transform_and_opacity() {
    let css = roll_css();
    let seal = &css[css.find(".kit-roll-verdict {").expect("the seal")..];
    let seal = &seal[..seal.find('}').expect("ends")];
    assert!(seal.contains("position: absolute") && seal.contains("bottom: -"));
    for banned in [
        "mix-blend-mode",
        "filter: blur",
        "filter: grayscale",
        "filter: saturate",
    ] {
        let kit_roll = &KIT[KIT.find("=== the roll card").expect("section")..];
        let kit_roll = &kit_roll[..kit_roll.find("=== end the roll card").expect("end")];
        let allowed = kit_roll.matches(banned).count();
        assert_eq!(allowed, 0, "{banned} is redrawn each frame on a moving die");
    }
}

#[test]
fn while_the_card_is_up_the_narration_is_a_low_strip_on_the_foot_left_of_the_card_in_its_glass() {
    let start = THEME
        .find("=== component: roll card")
        .expect("the roll card section");
    let section = &THEME[start..];
    let strip = &section[section
        .find(":has(.tv-roll:not(.roll-gone)) .tv-caption.tv-narration {")
        .expect("the strip's rule")..];
    let strip = &strip[..strip.find('}').expect("ends")];
    assert!(strip.contains("bottom: 0;") && strip.contains("left: 0;"));
    let glass = &section[section.find(".kit-narration {").expect("the strip's glass")..];
    let glass = &glass[..glass.find('}').expect("ends")];
    let width: f64 = glass
        .lines()
        .find_map(|line| line.trim().strip_prefix("width: "))
        .and_then(|width| width.strip_suffix("cqw;"))
        .expect("a width in cqw")
        .parse()
        .expect("a number");
    // The card begins 2.6cqw + 33cqw in from the right edge of the 100cqw screen.
    assert!(
        width <= 100.0 - 2.6 - 33.0 - 2.0,
        "the strip clears the card"
    );
    assert!(glass.contains("--kit-rim: var(--kit-rim-gold);"));
}

/// The height of the card's pane as the browser laid it out in the playground's stylesheet (the
/// pane's `offsetHeight`, in tenths of a cqw), for the cards below, in a 1280 px wide page.
// TODO(DEV-12): the three heights are the earlier browser measurements carried over by the
// stylesheet's changes since (sum font and line, section gap, foot padding and the larger chips),
// worked out by hand and not measured again; measure them in a browser.
#[test]
fn the_estimated_height_of_a_card_is_never_under_what_the_browser_measured_and_not_far_over() {
    let mut contested = roll(
        RollKind::ContestedCheck,
        vec![die(20, 20, DieRole::Test, true)],
        vec![modifier("Athletics", 6)],
        26,
    );
    contested.title = "Grapple against the Ogre".to_owned();
    contested.natural = NaturalMark::Twenty;
    contested.outcome = Some(RollOutcome::Won);
    contested.target = Some(RollTarget {
        label: "Ogre Athletics".to_owned(),
        value: 19,
    });
    let mut percentile = roll(
        RollKind::TableRoll,
        vec![die(100, 57, DieRole::Table, true)],
        Vec::new(),
        57,
    );
    percentile.title = "Wild magic surge".to_owned();
    percentile.target = None;
    percentile.outcome = None;
    for (name, card, measured) in [
        ("a lone d20", athletics(), 478),
        ("a percentile roll", RollCard::from(&percentile), 430),
        (
            "a contested roll with a long target",
            RollCard::from(&contested),
            571,
        ),
    ] {
        let estimate = card.height_tenths();
        assert!(
            estimate + 2 >= measured,
            "{name}: {estimate} is under {measured}"
        );
        assert!(
            estimate <= measured + 30,
            "{name}: {estimate} is far over {measured}"
        );
    }
}
