use super::{MaskPixels, instance};

fn mask(width: u32, rows: &[&str]) -> MaskPixels {
    let on: Vec<bool> = rows
        .iter()
        .flat_map(|row| row.bytes().map(|cell| cell == b'#'))
        .collect();
    MaskPixels {
        width,
        height: u32::try_from(rows.len()).unwrap(),
        on,
    }
}

fn runs(mask: &super::AnalysisMask) -> Vec<(u32, u32)> {
    mask.runs
        .iter()
        .map(|run| (run.start, run.length))
        .collect()
}

#[test]
fn the_best_match_is_the_whole_first_mask() {
    let top1 = mask(4, &["##..", "##.."]);
    let first = instance(0, &[0.9, 0.8], None, &top1).unwrap();
    assert_eq!(first.index, 0);
    assert!((first.score.get() - 0.9).abs() < 1e-6);
    assert_eq!(runs(&first), vec![(0, 2), (4, 2)]);
}

#[test]
fn a_later_match_is_what_the_next_mask_adds_and_an_overlap_stays_with_the_better() {
    let top1 = mask(4, &["##..", "##.."]);
    // The second match overlaps the first in column 1 of the second row.
    let top2 = mask(4, &["##.#", "####"]);
    let second = instance(1, &[0.9, 0.8], Some(&top1), &top2).unwrap();
    assert_eq!(second.index, 1);
    assert!((second.score.get() - 0.8).abs() < 1e-6);
    assert_eq!(runs(&second), vec![(3, 1), (6, 2)]);
}

#[test]
fn a_match_without_a_reported_score_is_not_made() {
    let top1 = mask(2, &["#."]);
    assert!(instance(1, &[0.9], None, &top1).is_none());
}
