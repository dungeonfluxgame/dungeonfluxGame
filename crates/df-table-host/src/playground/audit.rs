//! The audit lines of the playground: `PLAYGROUND <verb> key=value ...`, one line per change.
//!
//! The fields of a line are built from the typed change, never from its JSON, and only checked
//! values go in: a camera target, a zoom, a duration, a media name, a volume, a serial. Caption
//! text, which is free text, is never written; only its length is. Camera rule 9 of O.14 asks for
//! exactly this, so a playtest can measure when the camera moved and why.

use axum::http::HeaderMap;
use df_playground::controls::ControlsChange;
use df_playground::fixture::Slide;
use df_playground::path::CLIENT_HEADER;
use df_playground::scene::{ImageSlot, SceneChange, SceneOverlay, Target};
use df_table::event::RejectionCause;
use serde::{Deserialize, Serialize};

use super::check_client_name;

/// What kind of caller asked for a change, as its `X-Df-Client` header says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeSource {
    /// A playground page, which names itself `page-xxxxxxxx`.
    Page,
    /// The dev command line.
    Cli,
}

impl ChangeSource {
    pub const ALL: [Self; 2] = [Self::Page, Self::Cli];

    pub fn key(self) -> &'static str {
        match self {
            Self::Page => "page",
            Self::Cli => "cli",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|source| source.key() == key)
    }
}

/// The `X-Df-Client` value of the dev command line.
const CLI_CLIENT: &str = "cli";
/// How a page's name starts.
const PAGE_CLIENT_PREFIX: &str = "page-";

/// Who asked for a change: `source=page client=page-..`, `source=cli client=-`, or `-` for both
/// when the header is missing or is not a name the host accepts (it is never written as given).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ChangeOrigin {
    source: Option<ChangeSource>,
    client: Option<String>,
}

impl ChangeOrigin {
    pub(crate) fn from_headers(headers: &HeaderMap) -> Self {
        let Some(value) = headers
            .get(CLIENT_HEADER)
            .and_then(|value| value.to_str().ok())
        else {
            return Self::default();
        };
        if value == CLI_CLIENT {
            return Self {
                source: Some(ChangeSource::Cli),
                client: None,
            };
        }
        if value.starts_with(PAGE_CLIENT_PREFIX) && check_client_name(value).is_ok() {
            return Self {
                source: Some(ChangeSource::Page),
                client: Some(value.to_owned()),
            };
        }
        Self::default()
    }

    /// Whether the dev command line asked.
    pub(crate) fn is_cli(&self) -> bool {
        self.source == Some(ChangeSource::Cli)
    }

    /// The `source` and `client` pairs, which end the line.
    pub(crate) fn pairs(&self) -> [(&'static str, String); 2] {
        [
            (
                "source",
                self.source
                    .map_or_else(|| "-".to_owned(), |source| source.key().to_owned()),
            ),
            (
                "client",
                self.client.clone().unwrap_or_else(|| "-".to_owned()),
            ),
        ]
    }
}

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

/// The line of a request refused because a file could not be read or written:
/// `PLAYGROUND reject cause=file_error reason=<reason>` and any extra pairs. `reason` is a
/// `FileError` key or a closed word naming the file (`fixture_unreadable`, `media_unreadable`).
pub(crate) fn file_reject_line(reason: &'static str, extra: &[(&str, String)]) -> String {
    let mut pairs = vec![
        ("cause", "file_error".to_owned()),
        ("reason", reason.to_owned()),
    ];
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
            ControlsChange::ShowSam(shown) => ("show_sam", shown.to_string()),
            ControlsChange::Tts(on) => ("tts", on.to_string()),
            ControlsChange::Stt(on) => ("stt", on.to_string()),
            ControlsChange::TransitionStyle(style) => ("transition_style", style.key().to_owned()),
            ControlsChange::TransitionMs(ms) => ("transition_ms", ms.to_string()),
            ControlsChange::ZoomLimit(limit) => ("zoom_limit", limit.key().to_owned()),
        })
        .collect()
}

/// The audit pair of one field of an edit: its value when it is set, `clear` when it is taken
/// away, and nothing when it is left alone.
fn edit_pair<T>(
    key: &'static str,
    edit: &df_playground::overlay::Edit<T>,
    show: impl FnOnce(&T) -> String,
) -> Option<(&'static str, String)> {
    use df_playground::overlay::Edit;
    match edit {
        Edit::Keep => None,
        Edit::Clear => Some((key, "clear".to_owned())),
        Edit::Set(set) => Some((key, show(set))),
    }
}

/// The verb and pairs of an applied scene change. It is one match with an arm for each kind of
/// change, kept whole on purpose: every verb's typed fields are read in one place, and the compiler
/// refuses a new kind of change until it has an arm. `scene` is the overlay after the change, which
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
            volume_percent,
            looping,
        } => (
            "music",
            vec![
                ("media", value(media)),
                ("volume", volume_percent.to_string()),
                ("on", target_key(*target).to_owned()),
                ("loop", looping.to_string()),
            ],
        ),
        SceneChange::StopMusic {} => ("music_stop", Vec::new()),
        SceneChange::Sound {
            media,
            target,
            volume_percent,
        } => (
            "sound",
            vec![
                ("media", value(media)),
                ("volume", volume_percent.to_string()),
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
        SceneChange::Narration {
            text,
            speaker,
            badge,
            continue_label,
            voice,
            placement,
            progress,
        } => {
            let mut pairs = Vec::new();
            if let Some(text) = text {
                pairs.push(("text_chars", text.chars().count().to_string()));
            }
            pairs.extend(edit_pair("speaker", speaker, |text| value(text)));
            pairs.extend(edit_pair("badge", badge, |glyph| glyph.key().to_owned()));
            pairs.extend(edit_pair("continue", continue_label, |text| value(text)));
            if let Some(voice) = voice {
                pairs.push(("voice", voice.to_string()));
            }
            if let Some(placement) = placement {
                pairs.push(("placement", placement.key().to_owned()));
            }
            pairs.extend(edit_pair("scene", progress, |progress| {
                format!("{}/{}", progress.index, progress.total)
            }));
            ("narration", pairs)
        }
        SceneChange::ClearNarration {} => ("narration_clear", Vec::new()),
        SceneChange::SeatSet {
            name,
            class,
            level,
            portrait,
            glyph,
            accent,
            hit_points,
            maximum_hit_points,
            conditions,
        } => {
            let mut pairs = vec![("name", value(name))];
            pairs.extend(edit_pair("class", class, |text| value(text)));
            pairs.extend(edit_pair("level", level, |level| level.to_string()));
            pairs.extend(edit_pair("portrait", portrait, |text| value(text)));
            pairs.extend(edit_pair("glyph", glyph, |glyph| glyph.key().to_owned()));
            pairs.extend(edit_pair("accent", accent, |text| value(text)));
            if let Some(hit_points) = hit_points {
                pairs.push(("hp", hit_points.to_string()));
            }
            if let Some(maximum) = maximum_hit_points {
                pairs.push(("max", maximum.to_string()));
            }
            if let Some(conditions) = conditions {
                pairs.push(("conditions", conditions.len().to_string()));
            }
            ("seat_set", pairs)
        }
        SceneChange::SeatRemove { name } => ("seat_remove", vec![("name", value(name))]),
        SceneChange::SeatsOpen { open } => ("seats_open", vec![("open", open.to_string())]),
        SceneChange::SeatsClear {} => ("seats_clear", Vec::new()),
        SceneChange::Brand { on } => ("logo", vec![("on", on.to_string())]),
        SceneChange::Screen { edit } => edit.audit_fields(),
        SceneChange::Home { edit } => edit.audit_fields(),
        SceneChange::Reset {} => ("reset", Vec::new()),
    }
}

/// The audit line of a scene change that was applied, ending with the revision it made.
pub(crate) fn scene_line(change: &SceneChange, scene: &SceneOverlay, revision: u64) -> String {
    let (verb, mut pairs) = scene_fields(change, scene);
    pairs.push(("revision", revision.to_string()));
    playground_line(verb, &pairs)
}
