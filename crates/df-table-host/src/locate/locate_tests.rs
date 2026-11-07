use df_imagery::oneshot_segment::{
    AnalysisMask, AnalysisMaskRun, AnalysisMaskSet, AnalysisUnit, picture_sha256,
};
use df_scene_layout::{
    PictureBinding, RegionGeometry, RegionRef, SCHEMA_VERSION, SceneLayout, SceneRegion,
};

use super::*;
use crate::dev_http::RouteError;

/// A 100 by 50 picture with two people: a block at columns 60..80, rows 10..50 (found first), and
/// one at columns 10..30, rows 20..50.
fn two_people() -> AnalysisMaskSet {
    let block = |left: u32, right: u32, top: u32, bottom: u32| {
        (top..bottom)
            .map(|row| AnalysisMaskRun {
                start: row * 100 + left,
                length: right - left,
            })
            .collect::<Vec<_>>()
    };
    AnalysisMaskSet {
        prompt: df_imagery::oneshot_segment::SegmentationPrompt::Person,
        width_px: 100,
        height_px: 50,
        instances: vec![
            AnalysisMask {
                index: 0,
                score: AnalysisUnit::new(0.93).unwrap(),
                runs: block(60, 80, 10, 50),
            },
            AnalysisMask {
                index: 1,
                score: AnalysisUnit::new(0.81).unwrap(),
                runs: block(10, 30, 20, 50),
            },
        ],
    }
}

fn binding(name: &str, sha256: &str) -> PictureBinding {
    PictureBinding {
        name: name.to_owned(),
        sha256: sha256.to_owned(),
        width_px: 100,
        height_px: 50,
    }
}

fn region(label: Option<&str>, left: u16, right: u16) -> SceneRegion {
    let geometry = RegionGeometry {
        left_permille: left,
        top_permille: 200,
        right_permille: right,
        bottom_permille: 1000,
        head_x_permille: (left + right) / 2,
        head_y_permille: 200,
    };
    let id = df_scene_layout::region_id(&[left as u8, right as u8]);
    SceneRegion::new(id, geometry, 900)
        .unwrap()
        .with_label(label.map(str::to_owned))
}

/// A layout of `regions` bound to `picture`.
fn bound(picture: PictureBinding, regions: Vec<SceneRegion>) -> SceneLayout {
    SceneLayout::new(picture, "sam-3.1", "person", regions)
}

fn layout(regions: Vec<SceneRegion>) -> SceneLayout {
    bound(binding("plate.webp", &"ab".repeat(32)), regions)
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

#[test]
fn a_mask_set_becomes_a_bound_versioned_layout_with_stable_ids() {
    let (built, left_out) = layout_from(
        &two_people(),
        "sam-3.1",
        ScanPrompt::Person,
        binding("plate.webp", &"cd".repeat(32)),
    );
    assert_eq!(left_out, 0);
    assert!(built.is_for("plate.webp", &"cd".repeat(32)));
    assert_eq!(
        (built.picture().width_px, built.picture().height_px),
        (100, 50)
    );
    let boxes: Vec<_> = built
        .regions()
        .iter()
        .map(|r| {
            (
                r.left_permille(),
                r.top_permille(),
                r.right_permille(),
                r.bottom_permille(),
            )
        })
        .collect();
    assert_eq!(boxes, vec![(100, 400, 300, 1000), (600, 200, 800, 1000)]);
    let heads: Vec<_> = built
        .regions()
        .iter()
        .map(|r| (r.head_x_permille(), r.head_y_permille(), r.score_permille()))
        .collect();
    assert_eq!(heads, vec![(200, 400, 810), (700, 200, 930)]);
    // The same masks give the same ids on another scan, and different masks different ids.
    let (again, _) = layout_from(
        &two_people(),
        "sam-3.1",
        ScanPrompt::Person,
        binding("plate.webp", &"cd".repeat(32)),
    );
    assert_eq!(built.regions()[0].id(), again.regions()[0].id());
    assert_ne!(built.regions()[0].id(), built.regions()[1].id());
    assert!(
        built
            .regions()
            .iter()
            .all(|r| r.mask().is_some() && r.label().is_none())
    );
}

#[test]
fn an_instance_too_thin_for_a_box_in_thousandths_is_left_out_so_the_layout_reads_back() {
    // A 3000 by 10 picture: column 2 rounds to left 1 and right 1 in thousandths, no width.
    let column = |left: u32, right: u32| {
        (0..10)
            .map(|row| AnalysisMaskRun {
                start: row * 3000 + left,
                length: right - left,
            })
            .collect::<Vec<_>>()
    };
    let set = AnalysisMaskSet {
        prompt: df_imagery::oneshot_segment::SegmentationPrompt::Person,
        width_px: 3000,
        height_px: 10,
        instances: vec![
            AnalysisMask {
                index: 0,
                score: AnalysisUnit::new(0.9).unwrap(),
                runs: column(2, 3),
            },
            AnalysisMask {
                index: 1,
                score: AnalysisUnit::new(0.9).unwrap(),
                runs: column(600, 900),
            },
        ],
    };
    let (built, left_out) = layout_from(
        &set,
        "sam-3.1",
        ScanPrompt::Person,
        binding("plate.webp", &"cd".repeat(32)),
    );
    let boxes: Vec<_> = built
        .regions()
        .iter()
        .map(|r| (r.left_permille(), r.right_permille()))
        .collect();
    assert_eq!(boxes, vec![(200, 300)]);
    assert_eq!(left_out, 1);
    let text = serde_json::to_string(&built).unwrap();
    assert_eq!(serde_json::from_str::<SceneLayout>(&text).unwrap(), built);
}

#[test]
fn a_new_scan_keeps_the_names_of_regions_it_finds_again() {
    let old = layout(vec![
        region(Some("bard"), 100, 300),
        region(Some("innkeeper"), 600, 800),
    ]);
    let mut new = layout(vec![region(None, 400, 500), region(None, 620, 820)]);
    assert_eq!(keep_labels(&old, &mut new), 1);
    assert_eq!(new.regions()[0].label(), None);
    assert_eq!(new.regions()[1].label(), Some("innkeeper"));
}

#[test]
fn a_label_reaches_its_region_by_number_id_or_name_and_moves_off_others() {
    let mut named = layout(vec![
        region(Some("innkeeper"), 100, 300),
        region(None, 600, 800),
    ]);
    let id = named.regions()[1].id().to_string();
    relabel(&mut named, RegionRef::parse(&id), Some("innkeeper"), None).unwrap();
    assert_eq!(named.regions()[0].label(), None);
    assert_eq!(named.regions()[1].label(), Some("innkeeper"));
    relabel(
        &mut named,
        RegionRef::parse("innkeeper"),
        Some("host"),
        None,
    )
    .unwrap();
    assert_eq!(named.regions()[1].label(), Some("host"));
    let shown = Some("Lena".to_owned());
    relabel(&mut named, RegionRef::parse("host"), Some("host"), shown).unwrap();
    assert_eq!(named.regions()[1].display_name(), Some("Lena"));
    relabel(&mut named, RegionRef::parse("1"), None, None).unwrap();
    assert_eq!(named.regions()[1].display_name(), None);
    assert_eq!(named.regions()[1].label(), None);
    for reference in ["2", "bard"] {
        let refused =
            relabel(&mut named, RegionRef::parse(reference), Some("x"), None).unwrap_err();
        assert_eq!(
            refused,
            LocateError::UnknownRegion {
                region: reference.to_owned(),
                region_count: 2,
                names: Vec::new(),
            }
        );
    }
    assert_eq!(
        relabel(&mut named, RegionRef::parse("0"), Some("not ok"), None).unwrap_err(),
        LocateError::InvalidLabel {
            label: "not ok".to_owned()
        }
    );
}

#[test]
fn picture_names_prompts_and_counts_are_checked() {
    assert!(check_picture_name("innkeeper-table-plate-clean.webp").is_ok());
    for bad in [
        "",
        "../secret.png",
        "a/b.png",
        ".hidden.png",
        "notes.txt",
        "c:x.png",
    ] {
        assert!(check_picture_name(bad).is_err(), "{bad} should be refused");
    }
    assert_eq!(ScanPrompt::default(), ScanPrompt::Person);
    assert_eq!(ScanPrompt::from_key("head"), Some(ScanPrompt::Head));
    assert_eq!(ScanPrompt::from_key("table"), None);
    assert_eq!(check_masks(None).unwrap(), DEFAULT_MASKS);
    assert_eq!(
        check_masks(Some(0)),
        Err(LocateError::InvalidMaskCount { masks: 0 })
    );
    assert!(check_masks(Some(33)).is_err());
}

#[test]
fn only_a_layout_bound_to_the_picture_as_it_is_now_is_read_or_attached() {
    let folder = tempfile::tempdir().unwrap();
    let (media, layouts) = (folder.path().join("media"), folder.path().join("layouts"));
    std::fs::create_dir_all(&media).unwrap();
    std::fs::write(media.join("plate.webp"), b"picture one").unwrap();
    let stored = bound(
        binding("plate.webp", &picture_sha256(b"picture one")),
        vec![region(Some("innkeeper"), 600, 800)],
    );
    write_layout(&layouts, "plate.webp", &stored).unwrap();
    assert_eq!(
        current_layout(&media, &layouts, "plate.webp").unwrap(),
        Some(stored.clone())
    );
    assert_eq!(
        current_layout(&media, &layouts, "other.webp").unwrap(),
        None
    );

    let fixture = br#"{"tv":{"backdrop":{"media":"plate.webp","caption":null}}}"#.to_vec();
    let served: serde_json::Value =
        serde_json::from_slice(&attach_layout(fixture.clone(), &media, &layouts)).unwrap();
    let attached: SceneLayout =
        serde_json::from_value(served["tv"]["backdrop"]["layout"].clone()).unwrap();
    assert_eq!(attached, stored);

    // The picture changes: the layout is stale, refused on read and never attached.
    std::fs::write(media.join("plate.webp"), b"picture two").unwrap();
    assert_eq!(
        current_layout(&media, &layouts, "plate.webp").unwrap_err(),
        LocateError::StaleLayout {
            picture: "plate.webp".to_owned()
        }
    );
    assert_eq!(attach_layout(fixture.clone(), &media, &layouts), fixture);
    for untouched in [
        br#"{"tv":{"backdrop":{"media":"plate.webp","layout":null}}}"#.to_vec(),
        br#"{"tv":{"backdrop":null}}"#.to_vec(),
        b"not json".to_vec(),
    ] {
        assert_eq!(
            attach_layout(untouched.clone(), &media, &layouts),
            untouched
        );
    }
}

#[test]
fn a_layout_in_another_schema_version_is_stale_and_a_damaged_one_is_invalid() {
    let folder = tempfile::tempdir().unwrap();
    let (media, layouts) = (folder.path().join("media"), folder.path().join("layouts"));
    std::fs::create_dir_all(&media).unwrap();
    std::fs::write(media.join("plate.webp"), b"picture one").unwrap();
    let stored = bound(
        binding("plate.webp", &picture_sha256(b"picture one")),
        vec![region(Some("innkeeper"), 600, 800)],
    );
    let mut wire = serde_json::to_value(&stored).unwrap();
    wire["schema"] = serde_json::Value::from(SCHEMA_VERSION + 1);
    std::fs::create_dir_all(&layouts).unwrap();
    let file = layouts.join(layout_file_name("plate.webp"));
    std::fs::write(&file, wire.to_string()).unwrap();
    let stale = LocateError::StaleLayout {
        picture: "plate.webp".to_owned(),
    };
    assert_eq!(read_layout(&layouts, "plate.webp").unwrap_err(), stale);
    assert_eq!(
        current_layout(&media, &layouts, "plate.webp").unwrap_err(),
        stale
    );
    assert_eq!(stale.status(), axum::http::StatusCode::CONFLICT);
    // In this version but with a region that does not read: damaged, not out of date.
    wire["schema"] = serde_json::Value::from(SCHEMA_VERSION);
    wire["regions"][0]["right_permille"] = serde_json::Value::from(1001);
    std::fs::write(&file, wire.to_string()).unwrap();
    let damaged = read_layout(&layouts, "plate.webp").unwrap_err();
    assert!(
        matches!(&damaged, LocateError::LayoutInvalid { picture, .. } if picture == "plate.webp"),
        "{damaged:?}"
    );
}

#[tokio::test]
async fn a_scan_without_a_picture_or_with_a_bad_mask_count_is_refused_before_any_call() {
    let folder = tempfile::tempdir().unwrap();
    let request = |masks: Option<u32>| LocateRequest {
        picture: "absent.webp".to_owned(),
        prompt: None,
        masks,
    };
    let missing = scan_picture(folder.path(), folder.path(), &request(None))
        .await
        .unwrap_err();
    assert_eq!(
        missing,
        LocateError::PictureMissing {
            picture: "absent.webp".to_owned()
        }
    );
    let bad = scan_picture(folder.path(), folder.path(), &request(Some(0)))
        .await
        .unwrap_err();
    assert_eq!(bad, LocateError::InvalidMaskCount { masks: 0 });
    let unknown_prompt =
        serde_json::from_str::<LocateRequest>(r#"{"picture":"a.png","prompt":"table"}"#);
    assert!(unknown_prompt.is_err(), "{unknown_prompt:?}");
}

#[test]
fn an_identification_names_only_sure_unshared_matches_and_keeps_hand_names() {
    use df_imagery::identity_question::Identified;
    let mut named = layout(vec![
        region(Some("bard"), 100, 300),
        region(None, 400, 500),
        region(None, 600, 800),
    ]);
    let answer =
        |name: &str, candidate: Option<usize>, confidence: u16, conflict: bool| Identified {
            name: name.to_owned(),
            candidate,
            confidence_permille: confidence,
            conflict,
        };
    let rows = apply_identification(
        &mut named,
        &[
            answer("innkeeper", Some(3), 910, false),
            answer("guard", Some(2), 400, false),
            answer("bard", None, 0, false),
            answer("cook", None, 0, true),
        ],
        &[],
        DEFAULT_MIN_CONFIDENCE_PERMILLE,
    )
    .unwrap();
    let outcomes: Vec<_> = rows
        .iter()
        .map(|row| (row.name.as_str(), row.region, row.outcome.key()))
        .collect();
    assert_eq!(
        outcomes,
        vec![
            ("innkeeper", Some(2), "named"),
            ("guard", Some(1), "unsure"),
            ("bard", None, "not_found"),
            ("cook", None, "conflict"),
        ]
    );
    let labels: Vec<_> = named.regions().iter().map(SceneRegion::label).collect();
    assert_eq!(labels, vec![Some("bard"), None, Some("innkeeper")]);
}

#[test]
fn the_media_locate_grammar() {
    use crate::command::{Command, MediaLocate, parse};
    assert_eq!(
        parse(&words("media locate plate.webp prompt=head masks=8")).unwrap(),
        Command::MediaLocate(MediaLocate::Scan {
            picture: "plate.webp".to_owned(),
            prompt: Some(ScanPrompt::Head),
            masks: Some(8),
        })
    );
    assert_eq!(
        parse(&words("media locate label plate.webp r1a2b3c4d innkeeper")).unwrap(),
        Command::MediaLocate(MediaLocate::Label {
            picture: "plate.webp".to_owned(),
            region: "r1a2b3c4d".to_owned(),
            label: Some("innkeeper".to_owned()),
            display_name: None,
        })
    );
    assert_eq!(
        parse(&words("media locate label plate.webp 3 tavern_server Lena Hart")).unwrap(),
        Command::MediaLocate(MediaLocate::Label {
            picture: "plate.webp".to_owned(),
            region: "3".to_owned(),
            label: Some("tavern_server".to_owned()),
            display_name: Some("Lena Hart".to_owned()),
        })
    );
    assert_eq!(
        parse(&words("media locate label plate.webp elf -")).unwrap(),
        Command::MediaLocate(MediaLocate::Label {
            picture: "plate.webp".to_owned(),
            region: "elf".to_owned(),
            label: None,
            display_name: None,
        })
    );
    assert_eq!(
        parse(&words("media locate points plate.webp")).unwrap(),
        Command::MediaLocate(MediaLocate::Points {
            picture: "plate.webp".to_owned()
        })
    );
    let identify: Vec<String> = [
        "media",
        "locate",
        "identify",
        "plate.webp",
        "innkeeper=bearded man in a leather vest",
        "lyra",
        "portrait.lyra=sigrun.webp",
        "min=700",
    ]
    .iter()
    .map(|word| (*word).to_owned())
    .collect();
    let Command::MediaLocate(MediaLocate::Identify {
        characters,
        min_confidence_permille,
        ..
    }) = parse(&identify).unwrap()
    else {
        panic!("not identify");
    };
    assert_eq!(min_confidence_permille, Some(700));
    assert_eq!(
        characters[0].description.as_deref(),
        Some("bearded man in a leather vest")
    );
    assert_eq!(characters[1].portrait.as_deref(), Some("sigrun.webp"));
    for bad in [
        "media locate",
        "media locate plate.webp colour=red",
        "media locate plate.webp prompt=table",
        "media locate plate.webp masks=x",
        "media locate plate.webp masks=2 masks=3",
        "media locate identify plate.webp",
        "media locate identify plate.webp innkeeper innkeeper",
        "media locate identify plate.webp innkeeper portrait.bard=x.webp",
    ] {
        assert!(parse(&words(bad)).is_err(), "{bad} should be refused");
    }
}

#[test]
fn playground_set_sam_toggles_the_layer() {
    use crate::command::{Command, parse};
    use df_playground::controls::ControlsChange;
    let parsed = parse(&words("playground set sam=false")).unwrap();
    let hides_sam = match &parsed {
        Command::PlaygroundSet(changes) => changes.contains(&ControlsChange::ShowSam(false)),
        _ => false,
    };
    assert!(hides_sam, "{parsed:?}");
}

#[test]
fn the_longest_accepted_picture_name_still_stores_its_layout() {
    let longest = format!("{}.png", "a".repeat(MAXIMUM_PICTURE_NAME_BYTES - 4));
    assert!(check_picture_name(&longest).is_ok());
    assert!(check_picture_name(&format!("a{longest}")).is_err());
    let folder = tempfile::tempdir().unwrap();
    let stored = bound(binding(&longest, &"ab".repeat(32)), Vec::new());
    write_layout(folder.path(), &longest, &stored).unwrap();
    assert_eq!(read_layout(folder.path(), &longest).unwrap(), Some(stored));
}

#[test]
fn a_picture_name_that_could_forge_an_audit_line_is_refused() {
    for bad in [
        "two words.png",
        "plate.png\nLOCATE stored picture=x.png",
        "tab\t.png",
    ] {
        assert_eq!(
            check_picture_name(bad).unwrap_err(),
            LocateError::InvalidPictureName {
                name: bad.to_owned()
            },
            "{bad:?} should be refused"
        );
    }
}

#[test]
fn a_picture_that_is_there_but_does_not_read_is_a_storage_refusal() {
    let folder = tempfile::tempdir().unwrap();
    let (media, layouts) = (folder.path().join("media"), folder.path().join("layouts"));
    std::fs::create_dir_all(media.join("plate.webp")).unwrap();
    let stored = bound(
        binding("plate.webp", &picture_sha256(b"picture one")),
        vec![region(Some("innkeeper"), 600, 800)],
    );
    write_layout(&layouts, "plate.webp", &stored).unwrap();
    let refused = current_layout(&media, &layouts, "plate.webp").unwrap_err();
    let LocateError::PictureUnreadable { picture, .. } = &refused else {
        panic!("a picture that does not read is unreadable: {refused:?}");
    };
    assert_eq!(picture, "plate.webp");
    assert_eq!(
        refused.status(),
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[test]
fn an_identification_is_stored_on_the_layout_as_it_is_now() {
    use df_imagery::identity_question::Identified;
    let folder = tempfile::tempdir().unwrap();
    let (media, layouts) = (folder.path().join("media"), folder.path().join("layouts"));
    std::fs::create_dir_all(&media).unwrap();
    std::fs::write(media.join("plate.webp"), b"picture one").unwrap();
    let picture = binding("plate.webp", &picture_sha256(b"picture one"));
    let asked = bound(
        picture.clone(),
        vec![region(None, 100, 300), region(None, 600, 800)],
    );
    // A label stored while the question was asked is kept beside the answer.
    let mut labelled = asked.clone();
    labelled.regions_mut()[0].set_label(Some("bard".to_owned()));
    write_layout(&layouts, "plate.webp", &labelled).unwrap();
    let answers = [Identified {
        name: "innkeeper".to_owned(),
        candidate: Some(2),
        confidence_permille: 900,
        conflict: false,
    }];
    let characters = [CharacterRequest {
        name: "innkeeper".to_owned(),
        display_name: Some("Bram".to_owned()),
        description: None,
        portrait: None,
    }];
    let (stored, rows) =
        store_identification(&media, &layouts, "plate.webp", &asked, &answers, &characters, 600)
            .unwrap();
    assert_eq!(rows[0].outcome, IdentityOutcome::Named);
    let innkeeper = stored.region("innkeeper").unwrap();
    assert_eq!(innkeeper.display_name(), Some("Bram"));
    assert_eq!(stored.names(), vec!["bard", "innkeeper"]);
    assert_eq!(
        read_layout(&layouts, "plate.webp").unwrap(),
        Some(stored.clone())
    );

    // A rescan stored meanwhile has other regions: the answer is refused, the scan is kept.
    let rescanned = bound(picture, vec![region(None, 200, 400)]);
    write_layout(&layouts, "plate.webp", &rescanned).unwrap();
    let refused =
        store_identification(&media, &layouts, "plate.webp", &asked, &answers, &characters, 600)
            .unwrap_err();
    assert_eq!(
        refused,
        LocateError::LayoutChanged {
            picture: "plate.webp".to_owned()
        }
    );
    assert_eq!(
        read_layout(&layouts, "plate.webp").unwrap(),
        Some(rescanned)
    );
}
