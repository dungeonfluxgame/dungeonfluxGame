use super::*;

fn region(label: Option<&str>, left: u16, right: u16, head_y: u16) -> SceneRegion {
    SceneRegion {
        id: region_id(&[left as u8, right as u8]),
        label: label.map(str::to_owned),
        left_permille: left,
        top_permille: head_y,
        right_permille: right,
        bottom_permille: 1000,
        head_x_permille: (left + right) / 2,
        head_y_permille: head_y,
        score_permille: 950,
        mask: None,
    }
}

fn layout() -> SceneLayout {
    SceneLayout {
        schema: SCHEMA_VERSION,
        picture: PictureBinding {
            name: "plate.webp".to_owned(),
            sha256: "ab".repeat(32),
            width_px: 1672,
            height_px: 941,
        },
        model: "sam-3.1".to_owned(),
        prompt: "person".to_owned(),
        regions: vec![
            region(None, 0, 200, 300),
            region(Some("elf"), 400, 600, 200),
            region(Some("innkeeper"), 700, 1000, 300),
        ],
    }
}

#[test]
fn a_layout_is_for_its_own_picture_and_version_only() {
    let scene = layout();
    assert!(scene.is_for("plate.webp", &"AB".repeat(32)));
    assert!(!scene.is_for("plate.webp", &"cd".repeat(32)));
    assert!(!scene.is_for("other.webp", &"ab".repeat(32)));
    let mut old = scene;
    old.schema = 0;
    assert!(!old.is_for("plate.webp", &"ab".repeat(32)));
}

#[test]
fn region_ids_are_stable_and_references_find_by_number_id_or_name() {
    assert_eq!(region_id(b"runs"), region_id(b"runs"));
    assert_ne!(region_id(b"runs"), region_id(b"other"));
    assert!(region_id(b"x").starts_with('r') && region_id(b"x").len() == 9);
    let scene = layout();
    let id = scene.regions[2].id.clone();
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
fn the_centre_of_a_box_with_edges_past_half_of_u16_does_not_overflow() {
    let mut oversized = region(Some("giant"), 0, 0, 200);
    oversized.left_permille = 40_000;
    oversized.right_permille = 60_000;
    oversized.top_permille = 50_000;
    oversized.bottom_permille = 65_535;
    assert_eq!(
        point_of(&oversized, CharacterPoint::Centre),
        (50_000, 57_767)
    );
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
