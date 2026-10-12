//! How the display takes generated art from a view of the revision it already shows.
//!
//! A revision names game state, so an equal revision never carries new game content, and the
//! display's view store keeps the view it accepted. The server does publish generated art
//! (`GameplayScene.live_backdrop`, `GameplayScene.live_plate`, `SceneActor.live_sprite`) as a presentation-only change that
//! leaves the revision untouched, together with other equal-revision presentation (table lines,
//! tempo cues) that the display must keep taking at the pace of its director. So the display
//! copies only the art fields from such a view into the one it shows, and only when the scene and
//! cast it describes are the same ones and every art path is empty or a well-formed
//! `/gameplay/imagery/<key>`. The heroes' health faces (`JourneyView.live_faces`) travel the same
//! way, to the display and to every phone.

use df_protocol::common as rpc;

use crate::presentation::imagery_url;

fn well_formed(path: &str) -> bool {
    path.is_empty() || imagery_url(path).is_some()
}

fn faces_well_formed(journey: Option<&rpc::JourneyView>) -> bool {
    journey.is_none_or(|journey| {
        journey
            .live_faces
            .iter()
            .all(|face| well_formed(&face.sprite) && well_formed(&face.battle_sprite))
    })
}

/// Copies the heroes' health faces (`JourneyView.live_faces`) of `new` into `shown` and returns
/// whether any changed. Both views must list the same party (by name, in order).
fn merge_faces(shown: &mut Option<rpc::JourneyView>, new: Option<&rpc::JourneyView>) -> bool {
    let (Some(shown), Some(new)) = (shown.as_mut(), new) else {
        return false;
    };
    let same_party = shown.party.len() == new.party.len()
        && shown
            .party
            .iter()
            .zip(&new.party)
            .all(|(old, new)| old.name == new.name);
    if !same_party || shown.live_faces == new.live_faces {
        return false;
    }
    shown.live_faces.clone_from(&new.live_faces);
    true
}

/// Copies the generated-art fields of `new` into `shown` and returns whether any changed. Nothing
/// is copied unless both are views for the same audience of the same scene (title and authored
/// painting) with the same cast, in the same order, and every art path in `new` is well formed.
/// A display view carries the backdrop, the cast's cut-outs and the heroes' health faces; a
/// player view only the heroes' health faces.
pub(crate) fn merge_art(shown: &mut rpc::ViewMessage, new: &rpc::ViewMessage) -> bool {
    match (shown.audience.as_mut(), new.audience.as_ref()) {
        (
            Some(rpc::view_message::Audience::Display(shown)),
            Some(rpc::view_message::Audience::Display(new)),
        ) => {
            let (Some(shown_scene), Some(new_scene)) = (shown.scene.as_mut(), new.scene.as_ref())
            else {
                return false;
            };
            let same_scene = shown_scene.title == new_scene.title
                && shown_scene.scene_asset == new_scene.scene_asset
                && shown_scene.cast.len() == new_scene.cast.len()
                && shown_scene
                    .cast
                    .iter()
                    .zip(&new_scene.cast)
                    .all(|(old, new)| old.id == new.id && old.sprite == new.sprite);
            let well_formed_art = well_formed(&new_scene.live_backdrop)
                && well_formed(&new_scene.live_plate)
                && new_scene
                    .cast
                    .iter()
                    .all(|actor| well_formed(&actor.live_sprite))
                && faces_well_formed(new.journey.as_ref());
            if !same_scene || !well_formed_art {
                return false;
            }
            let mut changed = shown_scene.live_backdrop != new_scene.live_backdrop;
            shown_scene
                .live_backdrop
                .clone_from(&new_scene.live_backdrop);
            // PLAY-V09: the scene plate, its layout and its state are generated art too.
            changed |= shown_scene.live_plate != new_scene.live_plate
                || shown_scene.plate_layout != new_scene.plate_layout
                || shown_scene.plate_state != new_scene.plate_state;
            shown_scene.live_plate.clone_from(&new_scene.live_plate);
            shown_scene.plate_layout.clone_from(&new_scene.plate_layout);
            shown_scene.plate_state = new_scene.plate_state;
            for (old, new) in shown_scene.cast.iter_mut().zip(&new_scene.cast) {
                changed |= old.live_sprite != new.live_sprite;
                old.live_sprite.clone_from(&new.live_sprite);
            }
            changed | merge_faces(&mut shown.journey, new.journey.as_ref())
        }
        (
            Some(rpc::view_message::Audience::Player(shown)),
            Some(rpc::view_message::Audience::Player(new)),
        ) => {
            let same_scene = shown.scene.as_ref().map(|scene| &scene.title)
                == new.scene.as_ref().map(|scene| &scene.title);
            let plate_well_formed = new
                .scene
                .as_ref()
                .is_none_or(|scene| well_formed(&scene.live_plate));
            if !same_scene || !plate_well_formed || !faces_well_formed(new.journey.as_ref()) {
                return false;
            }
            // PLAY-V09: the scene plate goes to phones too (their scene picture).
            let mut changed = false;
            if let (Some(shown_scene), Some(new_scene)) = (shown.scene.as_mut(), new.scene.as_ref())
            {
                changed |= shown_scene.live_plate != new_scene.live_plate
                    || shown_scene.plate_layout != new_scene.plate_layout
                    || shown_scene.plate_state != new_scene.plate_state;
                shown_scene.live_plate.clone_from(&new_scene.live_plate);
                shown_scene.plate_layout.clone_from(&new_scene.plate_layout);
                shown_scene.plate_state = new_scene.plate_state;
            }
            changed | merge_faces(&mut shown.journey, new.journey.as_ref())
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: char) -> String {
        format!("/gameplay/imagery/{}", byte.to_string().repeat(64))
    }

    fn view(backdrop: &str, sprite: &str, narration: &str, title: &str) -> rpc::ViewMessage {
        rpc::ViewMessage {
            revision: Some(rpc::SessionRevision {
                epoch: Some(rpc::RecoveryEpoch { value: Some(1) }),
                sequence: Some(4),
            }),
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView {
                    narration: narration.to_owned(),
                    scene: Some(rpc::GameplayScene {
                        title: title.to_owned(),
                        live_backdrop: backdrop.to_owned(),
                        cast: vec![rpc::SceneActor {
                            id: "merchant".to_owned(),
                            sprite: "sprite-patron-merchant".to_owned(),
                            live_sprite: sprite.to_owned(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
        }
    }

    fn player_view(faces: &[(&str, &str)], party: &[&str]) -> rpc::ViewMessage {
        rpc::ViewMessage {
            revision: Some(rpc::SessionRevision {
                epoch: Some(rpc::RecoveryEpoch { value: Some(1) }),
                sequence: Some(4),
            }),
            audience: Some(rpc::view_message::Audience::Player(
                rpc::PlayerGameplayView {
                    scene: Some(rpc::GameplayScene {
                        title: "The Heron Shrine".to_owned(),
                        ..Default::default()
                    }),
                    journey: Some(rpc::JourneyView {
                        party: party
                            .iter()
                            .map(|name| rpc::PartyMemberView {
                                name: (*name).to_owned(),
                                ..Default::default()
                            })
                            .collect(),
                        live_faces: faces
                            .iter()
                            .map(|(name, sprite)| rpc::LiveFace {
                                name: (*name).to_owned(),
                                sprite: (*sprite).to_owned(),
                                ..Default::default()
                            })
                            .collect(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
        }
    }

    #[test]
    fn a_phones_scene_plate_is_copied_for_the_same_scene_and_a_bad_path_is_never_taken() {
        let mut shown = player_view(&[], &["Rowan"]);
        let mut newer = player_view(&[], &["Rowan"]);
        let set_plate = |view: &mut rpc::ViewMessage, path: &str| {
            if let Some(rpc::view_message::Audience::Player(player)) = view.audience.as_mut()
                && let Some(scene) = player.scene.as_mut()
            {
                scene.live_plate = path.to_owned();
                scene.plate_state = rpc::PlateState::Ready as i32;
            }
        };
        set_plate(&mut newer, &key('b'));
        assert!(merge_art(&mut shown, &newer));
        assert_eq!(shown, newer);
        assert!(!merge_art(&mut shown, &newer), "nothing more to copy");
        let before = shown.clone();
        let mut bad = player_view(&[], &["Rowan"]);
        set_plate(&mut bad, "/gameplay/imagery/short");
        assert!(!merge_art(&mut shown, &bad));
        assert_eq!(shown, before);
    }

    #[test]
    fn a_phones_hero_faces_are_copied_for_the_same_party_and_nothing_else_changes() {
        let mut shown = player_view(&[], &["Rowan", "Mira"]);
        let newer = player_view(&[("Rowan", &key('a'))], &["Rowan", "Mira"]);
        assert!(merge_art(&mut shown, &newer));
        assert_eq!(shown, newer);
        assert!(!merge_art(&mut shown, &newer), "nothing more to copy");
        // Healing clears the face again.
        assert!(merge_art(&mut shown, &player_view(&[], &["Rowan", "Mira"])));
        // A different party, a bad path or a display view is never taken.
        let original = shown.clone();
        assert!(!merge_art(
            &mut shown,
            &player_view(&[("Rowan", &key('a'))], &["Rowan", "Someone"])
        ));
        assert!(!merge_art(
            &mut shown,
            &player_view(&[("Rowan", "/gameplay/imagery/short")], &["Rowan", "Mira"])
        ));
        assert!(!merge_art(
            &mut shown,
            &view(&key('a'), "", "x", "The Heron Shrine")
        ));
        assert_eq!(shown, original);
    }

    #[test]
    fn a_displays_hero_faces_travel_with_its_art() {
        let mut shown = view("", "", "The shrine is quiet.", "The Heron Shrine");
        let mut newer = view("", "", "Newer narration.", "The Heron Shrine");
        for message in [&mut shown, &mut newer] {
            if let Some(rpc::view_message::Audience::Display(display)) = message.audience.as_mut() {
                display.journey = Some(rpc::JourneyView {
                    party: vec![rpc::PartyMemberView {
                        name: "Rowan".to_owned(),
                        ..Default::default()
                    }],
                    ..Default::default()
                });
            }
        }
        if let Some(rpc::view_message::Audience::Display(display)) = newer.audience.as_mut()
            && let Some(journey) = display.journey.as_mut()
        {
            journey.live_faces = vec![rpc::LiveFace {
                name: "Rowan".to_owned(),
                sprite: key('c'),
                ..Default::default()
            }];
        }
        assert!(merge_art(&mut shown, &newer));
        let Some(rpc::view_message::Audience::Display(display)) = &shown.audience else {
            panic!("display view");
        };
        assert_eq!(display.journey.as_ref().unwrap().live_faces.len(), 1);
        assert_eq!(display.narration, "The shrine is quiet.");
    }

    #[test]
    fn only_the_art_fields_are_copied_into_the_shown_view() {
        let mut shown = view("", "", "The shrine is quiet.", "The Heron Shrine");
        // The newer view also has other presentation the display takes at its own pace.
        let newer = view(&key('a'), &key('b'), "Newer narration.", "The Heron Shrine");
        assert!(merge_art(&mut shown, &newer));
        assert_eq!(
            shown,
            view(
                &key('a'),
                &key('b'),
                "The shrine is quiet.",
                "The Heron Shrine"
            )
        );
        assert!(!merge_art(&mut shown, &newer), "nothing more to copy");
        let cleared = view("", "", "Another.", "The Heron Shrine");
        assert!(merge_art(&mut shown, &cleared));
        assert_eq!(
            shown,
            view("", "", "The shrine is quiet.", "The Heron Shrine")
        );
    }

    #[test]
    fn the_scene_plate_its_layout_and_its_state_are_copied_with_the_other_art() {
        let mut shown = view("", "", "The shrine is quiet.", "The Heron Shrine");
        let mut newer = view("", "", "Newer narration.", "The Heron Shrine");
        if let Some(rpc::view_message::Audience::Display(display)) = newer.audience.as_mut()
            && let Some(scene) = display.scene.as_mut()
        {
            scene.live_plate = key('c');
            scene.plate_state = rpc::PlateState::Ready as i32;
            scene.plate_layout = Some(rpc::PlateLayout {
                bubble_side: rpc::PlateSide::Left as i32,
                head_x_permille: 640,
                head_y_permille: 320,
                head_width_permille: 120,
                head_height_permille: 180,
                ..Default::default()
            });
        }
        assert!(merge_art(&mut shown, &newer));
        let Some(rpc::view_message::Audience::Display(display)) = shown.audience.as_ref() else {
            panic!("a display view");
        };
        let scene = display.scene.as_ref().unwrap();
        assert_eq!(scene.live_plate, key('c'));
        assert_eq!(scene.plate_state, rpc::PlateState::Ready as i32);
        assert_eq!(scene.plate_layout.as_ref().unwrap().head_x_permille, 640);
        assert_eq!(display.narration, "The shrine is quiet.");
        assert!(!merge_art(&mut shown, &newer), "nothing more to copy");
        assert!(!merge_art(
            &mut shown,
            &view("/gameplay/imagery/x", "", "x", "The Heron Shrine")
        ));
    }

    #[test]
    fn art_of_another_scene_or_cast_or_with_a_bad_path_is_never_copied() {
        let original = view("", "", "The shrine is quiet.", "The Heron Shrine");
        let mut shown = original.clone();
        assert!(!merge_art(
            &mut shown,
            &view(&key('a'), "", "x", "Another Place")
        ));
        let mut other_cast = view(&key('a'), "", "x", "The Heron Shrine");
        if let Some(rpc::view_message::Audience::Display(display)) = other_cast.audience.as_mut()
            && let Some(scene) = display.scene.as_mut()
        {
            scene.cast[0].id = "someone-else".to_owned();
        }
        assert!(!merge_art(&mut shown, &other_cast));
        for bad in [
            "/assets/ui/generated/scene-x",
            "/gameplay/imagery/short",
            "/gameplay/imagery/../../etc/passwd",
            "https://elsewhere.example/a.png",
        ] {
            assert!(
                !merge_art(&mut shown, &view(bad, "", "x", "The Heron Shrine")),
                "{bad}"
            );
            assert!(
                !merge_art(&mut shown, &view("", bad, "x", "The Heron Shrine")),
                "{bad}"
            );
        }
        assert_eq!(shown, original);
        let player = rpc::ViewMessage {
            revision: shown.revision,
            audience: Some(rpc::view_message::Audience::Player(Default::default())),
        };
        assert!(!merge_art(&mut shown, &player));
        assert!(!merge_art(&mut player.clone(), &original));
    }
}
