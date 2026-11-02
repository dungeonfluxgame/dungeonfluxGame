//! The audit lines of the playground: `PLAYGROUND <verb> key=value ...`, one line per change.
//!
//! The fields of a line are built from the typed change, never from its JSON, and only checked
//! values go in: a camera target, a zoom, a duration, a media name, a volume, a serial. Caption
//! text, which is free text, is never written; only its length is. Camera rule 9 of O.14 asks for
//! exactly this, so a playtest can measure when the camera moved and why.

use df_playground::controls::ControlsChange;
use df_playground::fixture::Slide;
use df_playground::scene::{ImageSlot, SceneChange, SceneOverlay, Target};
use df_table::event::RejectionCause;

/// A text as one value of a line: as it is when it is a plain word, otherwise quoted.
fn value(text: &str) -> String {
    let plain = !text.is_empty()
        && !text
            .chars()
            .any(|character| character.is_whitespace() || character == '=' || character == '"');
    if plain {
        text.to_owned()
    } else {
        format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// A line for the audit log: the area, the verb and `key=value` pairs, like the canvas's lines.
pub(crate) fn playground_line(verb: &str, pairs: &[(&str, String)]) -> String {
    let mut line = format!("PLAYGROUND {verb}");
    for (key, value) in pairs {
        line.push_str(&format!(" {key}={value}"));
    }
    line
}

/// The line of a refused request: `PLAYGROUND reject cause=<cause>` and any extra pairs.
pub(crate) fn reject_line(cause: RejectionCause, extra: &[(&str, String)]) -> String {
    let mut pairs = vec![("cause", cause.key().to_owned())];
    pairs.extend(extra.iter().cloned());
    playground_line("reject", &pairs)
}

fn slot_key(slot: ImageSlot) -> &'static str {
    match slot {
        ImageSlot::TvBackdrop => "tv-backdrop",
        ImageSlot::TvCard => "tv-card",
        ImageSlot::PhoneImage => "phone",
    }
}

fn target_key(target: Target) -> &'static str {
    match target {
        Target::Both => "both",
        Target::Tv => "tv",
        Target::Phone => "phone",
    }
}

fn slide_text(slide: &Slide) -> String {
    format!(
        "{}-{}/{}",
        slide.from_percent, slide.to_percent, slide.duration_ms
    )
}

/// The pairs of a change of the controls, in the order given. The keys are the names of the
/// changes on the wire.
pub(crate) fn controls_pairs(changes: &[ControlsChange]) -> Vec<(&'static str, String)> {
    changes
        .iter()
        .map(|change| match change {
            ControlsChange::Fixture(name) => ("fixture", value(name)),
            ControlsChange::Device(device) => ("device", device.key().to_owned()),
            ControlsChange::Pace(pace) => ("pace", pace.key().to_owned()),
            ControlsChange::Status(key) => ("status", value(key)),
            ControlsChange::ShowUnavailable(shown) => ("show_unavailable", shown.to_string()),
            ControlsChange::ShowGrid(shown) => ("show_grid", shown.to_string()),
            ControlsChange::TransitionStyle(style) => ("transition_style", style.key().to_owned()),
            ControlsChange::TransitionMs(ms) => ("transition_ms", ms.to_string()),
        })
        .collect()
}

/// The verb and pairs of an applied scene change. `scene` is the overlay after the change, which
/// holds the serial the host gave a camera move, a slide or a sound.
fn scene_fields(
    change: &SceneChange,
    scene: &SceneOverlay,
) -> (&'static str, Vec<(&'static str, String)>) {
    match change {
        SceneChange::Caption { speaker, text } => {
            let mut pairs = Vec::new();
            if let Some(speaker) = speaker {
                pairs.push(("speaker", value(speaker)));
            }
            pairs.push(("chars", text.chars().count().to_string()));
            ("caption", pairs)
        }
        SceneChange::ClearCaption {} => ("caption_clear", Vec::new()),
        SceneChange::PartyAdd { member } => (
            "party_add",
            vec![
                ("name", value(&member.name)),
                ("hp", member.hit_points.to_string()),
                ("max", member.maximum_hit_points.to_string()),
            ],
        ),
        SceneChange::PartySet {
            name,
            hit_points,
            maximum_hit_points,
            conditions,
        } => {
            let mut pairs = vec![("name", value(name))];
            if let Some(hit_points) = hit_points {
                pairs.push(("hp", hit_points.to_string()));
            }
            if let Some(maximum) = maximum_hit_points {
                pairs.push(("max", maximum.to_string()));
            }
            if let Some(conditions) = conditions {
                pairs.push(("conditions", conditions.len().to_string()));
            }
            ("party_set", pairs)
        }
        SceneChange::PartyRemove { name } => ("party_remove", vec![("name", value(name))]),
        SceneChange::PartyClear {} => ("party_clear", Vec::new()),
        SceneChange::Suggested { choices } => {
            ("suggested", vec![("count", choices.len().to_string())])
        }
        SceneChange::Group { name, choices } => (
            "group",
            vec![("name", value(name)), ("count", choices.len().to_string())],
        ),
        SceneChange::Unavailable { choices } => {
            ("unavailable", vec![("count", choices.len().to_string())])
        }
        SceneChange::ClearChoices {} => ("choices_clear", Vec::new()),
        SceneChange::Image {
            slot,
            media,
            rotation_degrees,
            slide,
            ..
        } => {
            let mut pairs = vec![
                ("slot", slot_key(*slot).to_owned()),
                ("media", value(media)),
                ("rotate", rotation_degrees.to_string()),
            ];
            if let Some(slide) = slide {
                pairs.push(("slide", slide_text(slide)));
                pairs.push(("serial", scene.slide_count.to_string()));
            }
            ("image", pairs)
        }
        SceneChange::ClearImage { slot } => {
            ("image_clear", vec![("slot", slot_key(*slot).to_owned())])
        }
        SceneChange::Music {
            media,
            target,
            volume,
            looping,
        } => (
            "music",
            vec![
                ("media", value(media)),
                ("volume", volume.to_string()),
                ("on", target_key(*target).to_owned()),
                ("loop", looping.to_string()),
            ],
        ),
        SceneChange::StopMusic {} => ("music_stop", Vec::new()),
        SceneChange::Sound {
            media,
            target,
            volume,
        } => (
            "sound",
            vec![
                ("media", value(media)),
                ("volume", volume.to_string()),
                ("on", target_key(*target).to_owned()),
                ("cue", scene.cue_count.to_string()),
            ],
        ),
        SceneChange::Camera {
            target,
            zoom_percent,
            duration_ms,
        } => match target {
            Some(element) => (
                "camera",
                vec![
                    ("element", value(element)),
                    ("zoom", zoom_percent.to_string()),
                    ("over", duration_ms.to_string()),
                    ("serial", scene.camera_count.to_string()),
                ],
            ),
            None => (
                "camera_wide",
                vec![
                    ("over", duration_ms.to_string()),
                    ("serial", scene.camera_count.to_string()),
                ],
            ),
        },
        SceneChange::Reset {} => ("reset", Vec::new()),
    }
}

/// The audit line of a scene change that was applied, ending with the revision it made.
pub(crate) fn scene_line(change: &SceneChange, scene: &SceneOverlay, revision: u64) -> String {
    let (verb, mut pairs) = scene_fields(change, scene);
    pairs.push(("revision", revision.to_string()));
    playground_line(verb, &pairs)
}
