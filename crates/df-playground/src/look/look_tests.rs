use super::wire::{
    BodyFrame, BodyRegion, ControlValue, DetailKind, LookDraft, LookVersion, VersionState,
};

fn serde_name<T: serde::Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

#[test]
fn every_closed_key_is_its_serde_name_and_reads_back() {
    for frame in BodyFrame::ALL {
        assert_eq!(frame.key(), serde_name(frame));
        assert_eq!(BodyFrame::from_key(frame.key()), Some(frame));
    }
    for kind in DetailKind::ALL {
        assert_eq!(kind.key(), serde_name(kind));
        assert_eq!(DetailKind::from_key(kind.key()), Some(kind));
    }
    for region in BodyRegion::ALL {
        assert_eq!(region.key(), serde_name(region));
        assert_eq!(BodyRegion::from_key(region.key()), Some(region));
    }
}

#[test]
fn a_draft_reads_numbers_and_choices_and_details_with_where() {
    let draft: LookDraft = serde_json::from_str(
        r#"{"values":{"build":3,"hair_colour":"auburn"},
            "details":[{"kind":"scar","where":"face","what":"across the left cheek"}]}"#,
    )
    .unwrap();
    assert_eq!(draft.values["build"], ControlValue::Number(3));
    assert_eq!(
        draft.values["hair_colour"],
        ControlValue::Choice("auburn".to_owned())
    );
    assert_eq!(draft.details[0].place, BodyRegion::Face);
}

#[test]
fn a_failed_version_carries_its_state_and_cause_beside_its_fields() {
    let version: LookVersion = serde_json::from_str(
        r#"{"id":2,"parent":1,"draft":{},"changes":[],"picture":null,
            "state":"failed","cause":"backend","asked_ms":5,"took_ms":9}"#,
    )
    .unwrap();
    assert!(matches!(version.state, VersionState::Failed { .. }));
    assert!(!version.state.keepable());
}

mod studio_page {
    use crate::look::studio::{LookPage, LookPhase, LookTap};
    use crate::look::wire::{
        BodyFrame, LookControls, LookStudioView, LookVersion, PictureFailure, VersionState,
    };
    use crate::phone_kit::page::{Block, StageState};

    fn version(id: u64, state: VersionState) -> LookVersion {
        LookVersion {
            id,
            parent: None,
            draft: Default::default(),
            changes: Vec::new(),
            picture: Some(format!("look-1-v{id}.png")),
            state,
            asked_ms: 0,
            took_ms: None,
        }
    }

    fn page(kept: Option<u64>, versions: Vec<LookVersion>) -> LookPage {
        LookPage {
            studio: Some(LookStudioView {
                kept,
                versions,
                lock: None,
            }),
            controls: Some(LookControls {
                hero: 1,
                frame: BodyFrame::Medium,
                controls: Vec::new(),
            }),
            ..LookPage::default()
        }
    }

    #[test]
    fn every_tap_reads_back_from_its_action() {
        for tap in [
            LookTap::Choose("hair_colour".to_owned(), "auburn".to_owned()),
            LookTap::Slide("build".to_owned(), 3),
            LookTap::RemoveDetail(2),
            LookTap::ReadDetails,
            LookTap::Refine,
            LookTap::Keep(4),
            LookTap::KeepOld(5),
            LookTap::StartAgain,
            LookTap::Roll,
            LookTap::LockIn,
            LookTap::Species("elf".to_owned()),
            LookTap::Class("wizard".to_owned()),
            LookTap::Start,
            LookTap::Done,
            LookTap::TakeSeat(7),
            LookTap::View("look-1-v2.png".to_owned()),
            LookTap::ViewNext,
            LookTap::ViewPrevious,
            LookTap::ViewClose,
        ] {
            assert_eq!(LookTap::from_action(&tap.action()), Some(tap));
        }
        assert_eq!(LookTap::from_action("seat:new-hero"), None);
    }

    #[test]
    fn the_refine_machine_is_read_from_the_hosts_versions() {
        let refining = page(
            Some(1),
            vec![
                version(1, VersionState::Ready),
                version(2, VersionState::Pending),
            ],
        );
        assert_eq!(refining.phase(), LookPhase::Refining(2));
        let comparing = page(
            Some(1),
            vec![
                version(1, VersionState::Ready),
                version(2, VersionState::Ready),
            ],
        );
        assert_eq!(comparing.phase(), LookPhase::Comparing(2));
        assert!(matches!(
            comparing.screen().blocks[0],
            Block::Compare { .. }
        ));
        let stale = page(
            Some(1),
            vec![
                version(1, VersionState::Ready),
                version(2, VersionState::Stale),
            ],
        );
        assert_eq!(stale.phase(), LookPhase::Editing);
    }

    #[test]
    fn a_failed_picture_keeps_the_kept_one_on_the_stage() {
        let failed = VersionState::Failed {
            cause: PictureFailure::Backend,
        };
        let page = page(
            Some(1),
            vec![version(1, VersionState::Ready), version(2, failed)],
        );
        assert_eq!(page.phase(), LookPhase::Editing);
        let Block::Stage { picture, state, .. } = &page.screen().blocks[0] else {
            panic!("the stage comes first");
        };
        assert_eq!(picture.as_deref(), Some("look-1-v1.png"));
        assert_eq!(*state, StageState::Failed);
    }

    #[test]
    fn a_control_change_and_a_removed_detail_stay_on_the_phone() {
        let mut page = page(None, Vec::new());
        page.draft.details.push(crate::look::wire::LookDetail {
            kind: crate::look::wire::DetailKind::Scar,
            place: crate::look::wire::BodyRegion::Face,
            what: "a scar".to_owned(),
        });
        assert!(page.apply_local(&LookTap::Slide("build".to_owned(), 4)));
        assert!(page.apply_local(&LookTap::RemoveDetail(0)));
        assert!(page.draft.details.is_empty());
        assert!(!page.apply_local(&LookTap::Refine));
    }

    #[test]
    fn the_viewer_opens_a_new_picture_with_keep_and_steps_through_the_versions() {
        let mut page = page(
            Some(1),
            vec![
                version(1, VersionState::Ready),
                version(2, VersionState::Ready),
            ],
        );
        assert!(page.apply_local(&LookTap::View("look-1-v2.png".to_owned())));
        let viewer = page.screen().viewer.unwrap();
        assert_eq!(viewer.item.label, "New");
        assert_eq!(viewer.item.buttons.len(), 2);
        assert_eq!(viewer.next, None);
        assert!(page.apply_local(&LookTap::ViewPrevious));
        let viewer = page.screen().viewer.unwrap();
        assert_eq!(viewer.item.label, "Version 1, kept");
        assert!(!page.apply_local(&LookTap::ViewPrevious));
        assert!(page.apply_local(&LookTap::ViewClose));
        assert_eq!(page.screen().viewer, None);
    }
}

#[test]
fn every_refusal_kind_reads_back_from_its_tag_and_a_seat_refusal_reads_on_every_route() {
    use super::wire::LookErrorKind;
    use crate::dev_reply::ErrorTag;
    use crate::pages::wire::PagesErrorKind;
    use crate::seat::wire::SeatErrorKind;
    for kind in SeatErrorKind::ALL {
        assert_eq!(kind.key(), serde_name(kind));
        assert_eq!(SeatErrorKind::from_tag(kind.key()), Some(kind));
        // A tag both lists have (`invalid_body`, `storage`) reads as the look route's own.
        let look_own = LookErrorKind::ALL
            .into_iter()
            .any(|own| own.key() == kind.key());
        if !look_own {
            assert_eq!(
                LookErrorKind::from_tag(kind.key()),
                Some(LookErrorKind::Seat(kind))
            );
        }
        assert_eq!(
            PagesErrorKind::from_tag(kind.key()),
            Some(PagesErrorKind::Seat(kind))
        );
    }
    for kind in LookErrorKind::ALL
        .into_iter()
        .filter(|kind| *kind != LookErrorKind::StudioUnavailable)
    {
        assert_eq!(LookErrorKind::from_tag(kind.key()), Some(kind));
    }
    for tag in ["studio_unreadable", "studio_invalid", "studio_rejected"] {
        assert_eq!(
            LookErrorKind::from_tag(tag),
            Some(LookErrorKind::StudioUnavailable)
        );
    }
    for kind in [
        PagesErrorKind::Memory,
        PagesErrorKind::UnknownLine,
        PagesErrorKind::NothingShown,
    ] {
        assert_eq!(PagesErrorKind::from_tag(kind.key()), Some(kind));
    }
}

#[test]
fn the_words_reading_moves_the_controls_it_set_and_adds_its_chips_once() {
    use super::studio::LookPage;
    use super::wire::{
        ControlKind, LookControl, LookControls, LookDetail, LookDetails, LookOption,
    };
    let chip = |id: &str, options: &[&str]| LookControl {
        id: id.to_owned(),
        label: id.to_owned(),
        kind: ControlKind::Chip {
            options: options
                .iter()
                .map(|option| LookOption {
                    id: (*option).to_owned(),
                    label: (*option).to_owned(),
                    colour: None,
                })
                .collect(),
        },
        default: ControlValue::Choice(options[0].to_owned()),
    };
    let spectacles = LookDetail {
        kind: DetailKind::Other,
        place: BodyRegion::Face,
        what: "round wire-rimmed spectacles".to_owned(),
    };
    let mut page = LookPage {
        controls: Some(LookControls {
            hero: 8,
            frame: BodyFrame::Medium,
            controls: vec![
                chip("presentation", &["woman", "man"]),
                chip("hair_colour", &["brown", "white"]),
            ],
        }),
        ..LookPage::default()
    };
    page.draft.details.push(spectacles.clone());
    let read = LookDetails {
        values: [
            ("presentation", ControlValue::Choice("woman".to_owned())),
            ("hair_colour", ControlValue::Choice("white".to_owned())),
            // A control this hero is not shown stays out of the draft.
            ("horns", ControlValue::Choice("curled".to_owned())),
        ]
        .into_iter()
        .map(|(id, value)| (id.to_owned(), value))
        .collect(),
        details: vec![spectacles.clone()],
    };
    page.apply_details(read);
    assert_eq!(
        page.draft.values.get("presentation"),
        Some(&ControlValue::Choice("woman".to_owned()))
    );
    assert_eq!(
        page.draft.values.get("hair_colour"),
        Some(&ControlValue::Choice("white".to_owned()))
    );
    assert!(!page.draft.values.contains_key("horns"));
    assert_eq!(page.draft.details, vec![spectacles]);
}
