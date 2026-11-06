use super::*;

/// A box from `left` to `right` and from `top` to the bottom edge, with its head at the top middle.
fn geometry(left: u16, right: u16, top: u16) -> RegionGeometry {
    RegionGeometry {
        left_permille: left,
        top_permille: top,
        right_permille: right,
        bottom_permille: 1000,
        head_x_permille: (left + right) / 2,
        head_y_permille: top,
    }
}

fn region(label: Option<&str>, left: u16, right: u16, head_y: u16) -> SceneRegion {
    let id = region_id(&[left as u8, right as u8]);
    SceneRegion::new(id, geometry(left, right, head_y), 950)
        .unwrap()
        .with_label(label.map(str::to_owned))
}

fn layout() -> SceneLayout {
    SceneLayout::new(
        PictureBinding {
            name: "plate.webp".to_owned(),
            sha256: "ab".repeat(32),
            width_px: 1672,
            height_px: 941,
        },
        "sam-3.1",
        "person",
        vec![
            region(None, 0, 200, 300),
            region(Some("elf"), 400, 600, 200),
            region(Some("innkeeper"), 700, 1000, 300),
        ],
    )
}

#[test]
fn a_layout_is_for_its_own_picture_only() {
    let scene = layout();
    assert!(scene.is_for("plate.webp", &"AB".repeat(32)));
    assert!(!scene.is_for("plate.webp", &"cd".repeat(32)));
    assert!(!scene.is_for("other.webp", &"ab".repeat(32)));
}

#[test]
fn region_ids_are_stable_and_references_find_by_number_id_or_name() {
    assert_eq!(region_id(b"runs"), region_id(b"runs"));
    assert_ne!(region_id(b"runs"), region_id(b"other"));
    assert_eq!(RegionId::new(region_id(b"x").as_str()), Ok(region_id(b"x")));
    let scene = layout();
    let id = scene.regions()[2].id().to_string();
    assert_eq!(scene.find(RegionRef::parse("1")), Some(1));
    assert_eq!(scene.find(RegionRef::parse(&id)), Some(2));
    assert_eq!(scene.find(RegionRef::parse("elf")), Some(1));
    assert_eq!(scene.find(RegionRef::parse("9")), None);
    assert_eq!(scene.find(RegionRef::parse("bard")), None);
    assert_eq!(scene.names(), vec!["elf", "innkeeper"]);
}

#[test]
fn points_shots_and_targets_come_from_the_region() {
    let standing = region(Some("hero"), 400, 600, 200);
    assert_eq!(point_of(&standing, CharacterPoint::Head), (500, 200));
    assert_eq!(point_of(&standing, CharacterPoint::Face), (500, 288));
    assert_eq!(point_of(&standing, CharacterPoint::Centre), (500, 600));
    assert_eq!(point_of(&standing, CharacterPoint::Left), (400, 440));
    assert_eq!(shot_focus(&standing, Shot::Face), (500, 320));
    assert_eq!(shot_zoom_percent(&standing, Shot::Face), 375);
    assert_eq!(shot_zoom_percent(&standing, Shot::Full), 107);
    let targets: Vec<_> = region_targets(&layout())
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(targets[0], "region:elf");
    assert_eq!(targets.len(), 8);
    for point in CharacterPoint::ALL {
        assert_eq!(CharacterPoint::from_key(point.key()), Some(point));
    }
}

#[test]
fn the_midpoint_of_edges_past_half_of_u16_does_not_overflow() {
    assert_eq!(midpoint_permille(40_000, 60_000), 50_000);
    assert_eq!(midpoint_permille(50_000, 65_535), 57_767);
}

#[test]
fn the_cast_is_the_named_characters_left_to_right() {
    let members = cast(&layout());
    let names: Vec<_> = members.iter().map(|member| member.name.as_str()).collect();
    assert_eq!(names, vec!["elf", "innkeeper"]);
    assert_eq!(members[1].number, 2);
    assert_eq!(members[0].head, (500, 200));
}

#[test]
fn cues_read_and_write_as_plain_json() {
    let thinking: CharacterCue =
        serde_json::from_str(r#"{"character":"elf","kind":"thinking"}"#).unwrap();
    assert_eq!(thinking.kind, CueKind::Thinking);
    let speaking: CharacterCue =
        serde_json::from_str(r#"{"character":"elf","kind":"speaking","line":"Hello."}"#).unwrap();
    assert_eq!(
        speaking.kind,
        CueKind::Speaking {
            line: "Hello.".to_owned()
        }
    );
    assert!(
        serde_json::from_str::<CharacterCue>(r#"{"character":"elf","kind":"shouting"}"#).is_err()
    );
    assert!(is_character_name("braided_woman") && !is_character_name("two words"));
}

#[test]
fn a_layout_reads_from_json_and_refuses_unknown_fields() {
    let text = serde_json::to_string(&layout()).unwrap();
    let back: SceneLayout = serde_json::from_str(&text).unwrap();
    assert_eq!(back, layout());
    assert!(serde_json::from_str::<SceneLayout>(&text.replace("\"prompt\"", "\"other\"")).is_err());
    let unknown_region_field = text.replacen("\"mask\"", "\"outline\"", 1);
    assert!(serde_json::from_str::<SceneLayout>(&unknown_region_field).is_err());
}

#[test]
fn a_region_without_a_label_or_mask_field_reads_as_unnamed_and_maskless() {
    let text = r#"{"id":"r00000000","left_permille":100,"top_permille":200,"right_permille":300,
        "bottom_permille":1000,"head_x_permille":200,"head_y_permille":200,"score_permille":900}"#;
    let read: SceneRegion = serde_json::from_str(text).unwrap();
    assert_eq!((read.label(), read.mask()), (None, None));
}

/// The committed playground layout, as the host serves it and the page draws it.
const COMMITTED_LAYOUT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/playground/layouts/innkeeper-table-plate-clean.webp.layout.json"
);

#[test]
fn the_committed_playground_layout_still_reads_and_writes_back_the_same_json() {
    let text = std::fs::read_to_string(COMMITTED_LAYOUT).unwrap();
    let read: SceneLayout = serde_json::from_str(&text).unwrap();
    assert!(read.is_for(
        "innkeeper-table-plate-clean.webp",
        "41ee9287026cb0a55a8f57919f6124eba1950e51bc275168ffaa04f59038336c"
    ));
    assert_eq!(read.regions().len(), 16);
    assert_eq!(
        read.names(),
        vec!["warrior", "elf", "braided_woman", "wizard", "innkeeper"]
    );
    let written = serde_json::to_value(&read).unwrap();
    let committed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(written, committed);
}

/// `layout()` as written, with its second region's `field` written as `value`.
fn with_field(field: &str, value: u16) -> serde_json::Value {
    let mut wire = serde_json::to_value(layout()).unwrap();
    wire["regions"][1][field] = serde_json::Value::from(value);
    wire
}

/// What reading the second region of `wire` through its record gives: the typed outcome of the
/// read, which a whole layout's read reports as its own failure.
fn read_second_region(wire: &serde_json::Value) -> Result<SceneRegion, SceneRegionError> {
    let record: SceneRegionRecord = serde_json::from_value(wire["regions"][1].clone()).unwrap();
    SceneRegion::try_from(record)
}

/// The box of the second region of `layout()`, with `field` set to `value`.
fn second_box_with(field: &str, value: u16) -> SceneRegionError {
    let mut edges = [400, 200, 600, 1000];
    let at = [
        "left_permille",
        "top_permille",
        "right_permille",
        "bottom_permille",
    ]
    .iter()
    .position(|name| *name == field)
    .unwrap();
    edges[at] = value;
    SceneRegionError::InvalidBox {
        left_permille: edges[0],
        top_permille: edges[1],
        right_permille: edges[2],
        bottom_permille: edges[3],
    }
}

#[test]
fn a_layout_in_another_schema_does_not_read() {
    let mut wire = serde_json::to_value(layout()).unwrap();
    wire["schema"] = serde_json::Value::from(SCHEMA_VERSION + 1);
    assert!(serde_json::from_value::<SceneLayout>(wire.clone()).is_err());
    let record: SceneLayoutRecord = serde_json::from_value(wire).unwrap();
    assert_eq!(
        SceneLayout::try_from(record),
        Err(SceneLayoutError::InvalidSchema {
            schema: SCHEMA_VERSION + 1,
        })
    );
}

#[test]
fn a_region_id_that_is_not_r_and_8_hex_digits_does_not_read() {
    for text in ["r1234567", "x12345678", "r1234567g", "r123456789"] {
        assert_eq!(
            RegionId::new(text),
            Err(RegionIdError::InvalidText {
                text: text.to_owned()
            })
        );
        let mut wire = serde_json::to_value(layout()).unwrap();
        wire["regions"][1]["id"] = serde_json::Value::from(text);
        assert!(
            serde_json::from_value::<SceneLayout>(wire).is_err(),
            "{text}"
        );
    }
    assert!(RegionId::new("rABCDEF01").is_ok());
}

#[test]
fn a_region_box_outside_the_picture_or_inside_out_does_not_read() {
    // The second region is left 400, top 200, right 600, bottom 1000, head (500, 200).
    for (field, value) in [
        ("right_permille", 400),
        ("right_permille", 1001),
        ("left_permille", 700),
        ("bottom_permille", 200),
        ("bottom_permille", 1001),
        ("top_permille", 1000),
    ] {
        let wire = with_field(field, value);
        assert!(serde_json::from_value::<SceneLayout>(wire.clone()).is_err());
        assert_eq!(
            read_second_region(&wire),
            Err(second_box_with(field, value)),
            "{field} {value}"
        );
    }
}

#[test]
fn a_head_point_outside_its_box_does_not_read_and_one_on_its_edge_does() {
    for (field, value, head) in [
        ("head_x_permille", 399, (399, 200)),
        ("head_x_permille", 601, (601, 200)),
        ("head_y_permille", 199, (500, 199)),
    ] {
        let wire = with_field(field, value);
        assert!(serde_json::from_value::<SceneLayout>(wire.clone()).is_err());
        assert_eq!(
            read_second_region(&wire),
            Err(SceneRegionError::InvalidHead {
                head_x_permille: head.0,
                head_y_permille: head.1,
            }),
            "{field} {value}"
        );
    }
    for (field, value) in [
        ("head_x_permille", 400),
        ("head_x_permille", 600),
        ("head_y_permille", 1000),
    ] {
        assert!(serde_json::from_value::<SceneLayout>(with_field(field, value)).is_ok());
    }
}

#[test]
fn a_score_over_the_whole_does_not_read_and_the_whole_does() {
    let wire = with_field("score_permille", 1001);
    assert!(serde_json::from_value::<SceneLayout>(wire.clone()).is_err());
    assert_eq!(
        read_second_region(&wire),
        Err(SceneRegionError::InvalidScore {
            score_permille: 1001
        })
    );
    assert!(serde_json::from_value::<SceneLayout>(with_field("score_permille", 1000)).is_ok());
}

#[test]
fn a_region_is_made_only_when_valid_and_its_error_names_what_is_wrong() {
    let id = || region_id(b"elf");
    assert!(SceneRegion::new(id(), geometry(400, 600, 200), 950).is_ok());
    assert_eq!(
        SceneRegion::new(id(), geometry(400, 400, 200), 950),
        Err(SceneRegionError::InvalidBox {
            left_permille: 400,
            top_permille: 200,
            right_permille: 400,
            bottom_permille: 1000,
        })
    );
    let floating = RegionGeometry {
        head_y_permille: 100,
        ..geometry(400, 600, 200)
    };
    assert_eq!(
        SceneRegion::new(id(), floating, 950),
        Err(SceneRegionError::InvalidHead {
            head_x_permille: 500,
            head_y_permille: 100,
        })
    );
    assert_eq!(
        SceneRegion::new(id(), geometry(400, 600, 200), 1001),
        Err(SceneRegionError::InvalidScore {
            score_permille: 1001
        })
    );
}

#[test]
fn every_key_is_its_serde_name_and_reads_back() {
    for point in CharacterPoint::ALL {
        let wire = serde_json::to_value(point).expect("a point serializes");
        assert_eq!(wire, serde_json::Value::from(point.key()));
        assert_eq!(CharacterPoint::from_key(point.key()), Some(point));
    }
    for shot in Shot::ALL {
        let wire = serde_json::to_value(shot).expect("a shot serializes");
        assert_eq!(wire, serde_json::Value::from(shot.key()));
        assert_eq!(Shot::from_key(shot.key()), Some(shot));
    }
}

#[test]
fn a_face_box_runs_from_the_head_to_the_chin_inside_the_region() {
    let layout = layout();
    let region = layout.region("elf").expect("named");
    let face = face_box(region, 1672, 941);
    assert_eq!(face.top_permille, region.head_y_permille());
    let (_, face_y) = point_of(region, CharacterPoint::Face);
    assert!(face.bottom_permille > face_y, "{face:?}");
    assert!(face.left_permille >= region.left_permille());
    assert!(face.right_permille <= region.right_permille());
    assert!(face.left_permille < region.head_x_permille());
    assert!(face.right_permille > region.head_x_permille());
    // TODO(DEV-12): cover a region whose box is narrower than its face and a cut-off figure.
}
