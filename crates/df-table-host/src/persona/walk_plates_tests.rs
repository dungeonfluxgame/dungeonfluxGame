//! The naming of someone a walk's plate was painted with whom its scan left unsure.

use df_scene_layout::{PictureBinding, RegionGeometry, SceneLayout, SceneRegion};

use super::placed_to_name;
use crate::locate::request::{IdentityOutcome, IdentityRow};

fn region(seed: &[u8], left: u16, label: Option<&str>) -> SceneRegion {
    let geometry = RegionGeometry {
        left_permille: left,
        top_permille: 200,
        right_permille: left + 200,
        bottom_permille: 1000,
        head_x_permille: left + 100,
        head_y_permille: 200,
    };
    SceneRegion::new(df_scene_layout::region_id(seed), geometry, 900)
        .expect("a region")
        .with_label(label.map(str::to_owned))
}

fn layout(regions: Vec<SceneRegion>) -> SceneLayout {
    let binding = PictureBinding {
        name: "explore-21.png".to_owned(),
        sha256: df_imagery::oneshot_segment::picture_sha256(b"plate"),
        width_px: 100,
        height_px: 50,
    };
    SceneLayout::new(binding, "sam-3.1", "person", regions)
}

fn row(name: &str, region: Option<usize>, confidence: u16) -> IdentityRow {
    IdentityRow {
        name: name.to_owned(),
        region,
        confidence_permille: confidence,
        outcome: IdentityOutcome::Unsure,
    }
}

#[test]
fn someone_painted_in_is_named_where_the_judge_picked_them_when_that_region_is_free() {
    // Host 40: the hero named, the apple seller picked for region 1 at 500 and left unnamed.
    let shown = layout(vec![
        region(b"hero", 100, Some("hero-11")),
        region(b"seller", 600, None),
    ]);
    let rows = [
        row("hero-11", Some(0), 990),
        row("fruit_vendor", Some(1), 500),
    ];
    assert_eq!(
        placed_to_name(&rows, &shown),
        [(1, "fruit_vendor".to_owned())]
    );
    // Too unsure, a region someone holds, or a region two were picked for: nobody is named.
    assert!(placed_to_name(&[row("fruit_vendor", Some(1), 200)], &shown).is_empty());
    assert!(placed_to_name(&[row("fruit_vendor", Some(0), 800)], &shown).is_empty());
    let twice = [
        row("fruit_vendor", Some(1), 500),
        row("guard", Some(1), 450),
    ];
    assert!(placed_to_name(&twice, &shown).is_empty());
    assert!(placed_to_name(&[row("fruit_vendor", None, 900)], &shown).is_empty());
}
