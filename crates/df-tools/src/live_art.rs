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

// The party mural (HS49) is generated art too: a ready mural at the same revision reaches the TV.
#[path = "gameplay/living_scenes/mural_view.rs"]
pub(crate) mod mural_view;

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

/// PLAY-V10: copies the home loop's state and served path (`JourneyView.home_video_*`) of `new`
/// into `shown`. A path is taken only in the server's own route form.
fn merge_loop(shown: &mut Option<rpc::JourneyView>, new: Option<&rpc::JourneyView>) -> bool {
    let (Some(shown), Some(new)) = (shown.as_mut(), new) else {
        return false;
    };
    let route_ok = new.home_video_path.is_empty()
        || (new.home_video_path.starts_with("/gameplay/home-video/")
            && new.home_video_path.len() <= 96);
    if !route_ok
        || (shown.home_video_state == new.home_video_state
            && shown.home_video_path == new.home_video_path)
    {
        return false;
    }
    shown.home_video_state = new.home_video_state;
    shown.home_video_path.clone_from(&new.home_video_path);
    true
}

/// Content-free condition names for refused art; an unchanged valid snapshot is not a refusal.
pub(crate) fn refusal_reason(
    shown: &rpc::ViewMessage,
    new: &rpc::ViewMessage,
) -> Option<&'static str> {
    let (shown_scene, new_scene, journey) = match (&shown.audience, &new.audience) {
        (
            Some(rpc::view_message::Audience::Display(shown)),
            Some(rpc::view_message::Audience::Display(new)),
        ) => (
            shown.scene.as_ref(),
            new.scene.as_ref(),
            new.journey.as_ref(),
        ),
        (
            Some(rpc::view_message::Audience::Player(shown)),
            Some(rpc::view_message::Audience::Player(new)),
        ) => {
            if shown.scene.as_ref().map(|scene| &scene.title)
                != new.scene.as_ref().map(|scene| &scene.title)
            {
                return Some("scene-title");
            }
            if new
                .scene
                .as_ref()
                .is_some_and(|scene| !well_formed(&scene.live_plate))
            {
                return Some("plate-path");
            }
            return (!faces_well_formed(new.journey.as_ref())).then_some("face-path");
        }
        _ => return Some("audience"),
    };
    let (Some(shown), Some(new)) = (shown_scene, new_scene) else {
        return (shown_scene.is_some() != new_scene.is_some()).then_some("scene-presence");
    };
    if shown.title != new.title {
        Some("scene-title")
    } else if shown.scene_asset != new.scene_asset {
        Some("scene-asset")
    } else if shown.cast.len() != new.cast.len() {
        Some("cast-count")
    } else if !shown
        .cast
        .iter()
        .zip(&new.cast)
        .all(|(old, new)| old.id == new.id)
    {
        Some("cast-identity")
    } else if !well_formed(&new.live_backdrop) {
        Some("backdrop-path")
    } else if !well_formed(&new.live_plate) {
        Some("plate-path")
    } else if !new.cast.iter().all(|actor| well_formed(&actor.live_sprite)) {
        Some("sprite-path")
    } else if !faces_well_formed(journey) {
        Some("face-path")
    } else {
        None
    }
}

/// Copies the generated-art fields of `new` into `shown` and returns whether any changed. Nothing
/// is copied unless both are views for the same audience of the same scene (title and authored
/// painting) with the same cast, in the same order, and every art path in `new` is well formed.
/// A display view carries the backdrop, the cast's cut-outs and the heroes' health faces; a
/// player view only the heroes' health faces.
pub(crate) fn merge_art(shown: &mut rpc::ViewMessage, new: &rpc::ViewMessage) -> bool {
    if refusal_reason(shown, new).is_some() {
        return false;
    }
    match (shown.audience.as_mut(), new.audience.as_ref()) {
        (
            Some(rpc::view_message::Audience::Display(shown)),
            Some(rpc::view_message::Audience::Display(new)),
        ) => {
            // PLAY-V10: the lobby's loop belongs to the lobby's painting, which the audience names
            // directly, so it is merged even when the view carries no scene.
            let loop_changed = shown.scene_asset == new.scene_asset
                && merge_loop(&mut shown.journey, new.journey.as_ref());
            let (Some(shown_scene), Some(new_scene)) = (shown.scene.as_mut(), new.scene.as_ref())
            else {
                return loop_changed;
            };
            let mut changed = shown_scene.live_backdrop != new_scene.live_backdrop;
            shown_scene
                .live_backdrop
                .clone_from(&new_scene.live_backdrop);
            // PLAY-V09: the scene plate, its layout and its state are generated art too.
            changed |= shown_scene.live_plate != new_scene.live_plate
                || shown_scene.plate_layout != new_scene.plate_layout
                || shown_scene.plate_state != new_scene.plate_state
                || shown_scene.plate_update_pending != new_scene.plate_update_pending;
            shown_scene.live_plate.clone_from(&new_scene.live_plate);
            shown_scene.plate_layout.clone_from(&new_scene.plate_layout);
            shown_scene.plate_state = new_scene.plate_state;
            shown_scene.plate_update_pending = new_scene.plate_update_pending;
            changed |= mural_view::merge_scene_contents(
                &mut shown_scene.scene_contents_json,
                &new_scene.scene_contents_json,
            );
            for (old, new) in shown_scene.cast.iter_mut().zip(&new_scene.cast) {
                changed |= old.live_sprite != new.live_sprite;
                old.live_sprite.clone_from(&new.live_sprite);
            }
            changed | merge_faces(&mut shown.journey, new.journey.as_ref()) | loop_changed
        }
        (
            Some(rpc::view_message::Audience::Player(shown)),
            Some(rpc::view_message::Audience::Player(new)),
        ) => {
            // PLAY-V09: the scene plate goes to phones too (their scene picture).
            let mut changed = false;
            if let (Some(shown_scene), Some(new_scene)) = (shown.scene.as_mut(), new.scene.as_ref())
            {
                changed |= shown_scene.live_plate != new_scene.live_plate
                    || shown_scene.plate_layout != new_scene.plate_layout
                    || shown_scene.plate_state != new_scene.plate_state
                    || shown_scene.plate_update_pending != new_scene.plate_update_pending;
                shown_scene.live_plate.clone_from(&new_scene.live_plate);
                shown_scene.plate_layout.clone_from(&new_scene.plate_layout);
                shown_scene.plate_state = new_scene.plate_state;
                shown_scene.plate_update_pending = new_scene.plate_update_pending;
                changed |= mural_view::merge_scene_contents(
                    &mut shown_scene.scene_contents_json,
                    &new_scene.scene_contents_json,
                );
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

    fn set_plate(
        view: &mut rpc::ViewMessage,
        path: &str,
        state: rpc::PlateState,
        update_pending: bool,
    ) {
        let scene = match view.audience.as_mut() {
            Some(rpc::view_message::Audience::Display(display)) => display.scene.as_mut(),
            Some(rpc::view_message::Audience::Player(player)) => player.scene.as_mut(),
            None => None,
        };
        if let Some(scene) = scene {
            scene.live_plate = path.to_owned();
            scene.plate_state = state as i32;
            scene.plate_update_pending = update_pending;
        }
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
                figure_masks: Vec::new(),
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
    fn same_revision_plate_transitions_reach_displays_and_phones_for_each_party_size() {
        for hero_count in [1, 4, 6] {
            let party = (0..hero_count)
                .map(|index| format!("Hero {index}"))
                .collect::<Vec<_>>();
            let party_names = party.iter().map(String::as_str).collect::<Vec<_>>();
            for mut shown in [
                view("", "", "Current line.", "The Heron Shrine"),
                player_view(&[], &party_names),
            ] {
                if let Some(rpc::view_message::Audience::Display(display)) = shown.audience.as_mut()
                {
                    display.journey = Some(rpc::JourneyView {
                        party: party
                            .iter()
                            .map(|name| rpc::PartyMemberView {
                                name: name.clone(),
                                ..Default::default()
                            })
                            .collect(),
                        ..Default::default()
                    });
                }
                let mut ready = shown.clone();
                set_plate(&mut ready, &key('a'), rpc::PlateState::Ready, false);
                assert!(merge_art(&mut shown, &ready), "ready for {hero_count}");
                assert_eq!(shown, ready);

                let mut failed = shown.clone();
                set_plate(&mut failed, "", rpc::PlateState::Failed, false);
                assert!(merge_art(&mut shown, &failed), "failed for {hero_count}");
                assert_eq!(shown, failed);

                let mut pending = shown.clone();
                set_plate(&mut pending, "", rpc::PlateState::Pending, true);
                assert!(merge_art(&mut shown, &pending), "pending for {hero_count}");
                assert_eq!(shown, pending);

                let mut replaced = shown.clone();
                set_plate(&mut replaced, &key('b'), rpc::PlateState::Ready, false);
                assert!(
                    merge_art(&mut shown, &replaced),
                    "replacement for {hero_count}"
                );
                assert_eq!(shown, replaced);
            }
        }
    }

    #[test]
    fn a_cast_sprite_change_does_not_block_same_revision_plate_and_live_sprite_art() {
        let mut shown = view("", "", "Current line.", "The Heron Shrine");
        let mut newer = view("", &key('d'), "Newer line.", "The Heron Shrine");
        if let Some(rpc::view_message::Audience::Display(display)) = newer.audience.as_mut()
            && let Some(scene) = display.scene.as_mut()
        {
            scene.cast[0].sprite = "sprite-patron-merchant-alt".to_owned();
            scene.live_plate = key('c');
            scene.plate_state = rpc::PlateState::Ready as i32;
            scene.plate_layout = Some(rpc::PlateLayout {
                figure_masks: Vec::new(),
                actors: vec![rpc::PlateActorHead {
                    actor_id: "merchant".to_owned(),
                    head_x_permille: 640,
                    ..Default::default()
                }],
                ..Default::default()
            });
            scene.plate_update_pending = false;
        }
        assert!(merge_art(&mut shown, &newer));
        let Some(rpc::view_message::Audience::Display(display)) = shown.audience.as_ref() else {
            panic!("display view");
        };
        let scene = display.scene.as_ref().unwrap();
        assert_eq!(scene.live_plate, key('c'));
        assert_eq!(scene.plate_state, rpc::PlateState::Ready as i32);
        assert!(!scene.plate_update_pending);
        assert_eq!(
            scene
                .plate_layout
                .as_ref()
                .and_then(|layout| layout.actors.first())
                .map(|actor| actor.head_x_permille),
            Some(640)
        );
        assert_eq!(scene.cast[0].sprite, "sprite-patron-merchant");
        assert_eq!(scene.cast[0].live_sprite, key('d'));
        assert_eq!(display.narration, "Current line.");
    }

    #[test]
    fn refused_art_names_the_guard_and_does_not_change_the_retained_scene() {
        let original = view("", "", "Current narration.", "Salt quay");
        for condition in [
            "scene-title",
            "scene-asset",
            "cast-count",
            "cast-identity",
            "backdrop-path",
            "plate-path",
            "sprite-path",
            "face-path",
        ] {
            let mut incoming = original.clone();
            let Some(rpc::view_message::Audience::Display(display)) = &mut incoming.audience else {
                unreachable!()
            };
            let scene = display.scene.as_mut().unwrap();
            match condition {
                "scene-title" => scene.title = "Another place".into(),
                "scene-asset" => scene.scene_asset = "another-painting".into(),
                "cast-count" => scene.cast.clear(),
                "cast-identity" => scene.cast[0].id = "another-actor".into(),
                "backdrop-path" => scene.live_backdrop = "/gameplay/imagery/short".into(),
                "plate-path" => scene.live_plate = "https://elsewhere.test/plate".into(),
                "sprite-path" => scene.cast[0].live_sprite = "/gameplay/imagery/../bad".into(),
                "face-path" => {
                    display.journey = Some(rpc::JourneyView {
                        live_faces: vec![rpc::LiveFace {
                            sprite: "/gameplay/imagery/short".into(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    })
                }
                _ => unreachable!(),
            }
            assert_eq!(refusal_reason(&original, &incoming), Some(condition));
            let mut retained = original.clone();
            assert!(!merge_art(&mut retained, &incoming));
            assert_eq!(retained, original);
        }
        assert_eq!(refusal_reason(&original, &original), None);
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
