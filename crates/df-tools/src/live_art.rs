//! How the display takes generated art from a view of the revision it already shows.
//!
//! A revision names game state, so an equal revision never carries new game content, and the
//! display's view store keeps the view it accepted. The server does publish generated art
//! (`GameplayScene.live_backdrop`, `SceneActor.live_sprite`) as a presentation-only change that
//! leaves the revision untouched, together with other equal-revision presentation (table lines,
//! tempo cues) that the display must keep taking at the pace of its director. So the display
//! copies only the art fields from such a view into the one it shows, and only when the scene and
//! cast it describes are the same ones and every art path is empty or a well-formed
//! `/gameplay/imagery/<key>`.

use df_protocol::common as rpc;

use crate::presentation::imagery_url;

fn well_formed(path: &str) -> bool {
    path.is_empty() || imagery_url(path).is_some()
}

/// Copies the generated-art fields of `new` into `shown` and returns whether any changed. Nothing
/// is copied unless both are display views of the same scene (title and authored painting) with
/// the same cast, in the same order, and every art path in `new` is well formed.
pub(crate) fn merge_art(shown: &mut rpc::ViewMessage, new: &rpc::ViewMessage) -> bool {
    let (
        Some(rpc::view_message::Audience::Display(shown)),
        Some(rpc::view_message::Audience::Display(new)),
    ) = (shown.audience.as_mut(), new.audience.as_ref())
    else {
        return false;
    };
    let (Some(shown), Some(new)) = (shown.scene.as_mut(), new.scene.as_ref()) else {
        return false;
    };
    let same_scene = shown.title == new.title
        && shown.scene_asset == new.scene_asset
        && shown.cast.len() == new.cast.len()
        && shown
            .cast
            .iter()
            .zip(&new.cast)
            .all(|(old, new)| old.id == new.id && old.sprite == new.sprite);
    let well_formed_art = well_formed(&new.live_backdrop)
        && new.cast.iter().all(|actor| well_formed(&actor.live_sprite));
    if !same_scene || !well_formed_art {
        return false;
    }
    let mut changed = shown.live_backdrop != new.live_backdrop;
    shown.live_backdrop.clone_from(&new.live_backdrop);
    for (old, new) in shown.cast.iter_mut().zip(&new.cast) {
        changed |= old.live_sprite != new.live_sprite;
        old.live_sprite.clone_from(&new.live_sprite);
    }
    changed
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
