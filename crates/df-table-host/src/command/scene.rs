//! The `scene` verbs: the live caption, party, choices, pictures, camera and sound of the
//! playground.
//!
//! Each verb reads its words into one `SceneChange`; whether the change is allowed (a name that is
//! in the media library, a target that is on the stage) is decided by the host when it arrives.

pub(super) mod home;
pub(super) mod lobby;
pub(super) mod pills;
mod ui;

use df_playground::fixture::Slide;
use df_playground::fixture::{Choice, PartyMember, UnavailableChoice};
use df_playground::limits::{
    MAXIMUM_CAMERA_MS, MAXIMUM_ROTATION_DEGREES, MAXIMUM_SLIDE_MS, MAXIMUM_SLIDE_PERCENT,
    MAXIMUM_VOLUME_PERCENT, MAXIMUM_ZOOM_PERCENT, MINIMUM_ZOOM_PERCENT,
};
use df_playground::scene::{ImageSlot, SceneChange, Target};

use super::usage::{
    CAPTION_USAGE, CHOICES_USAGE, IMAGE_USAGE, MUSIC_USAGE, PARTY_USAGE, SCENE_CAMERA_USAGE,
    SCENE_USAGE, SLIDE_USAGE, SOUND_USAGE,
};
use super::{Command, CommandError, bad_choice, count, keyed, pair, split_list, truth};

/// Volume of music and sounds when none is given, in percent.
pub const DEFAULT_VOLUME: u8 = 80;
/// Where a slide starts and ends, and how long it takes, when none is given: the left of the
/// screen to the right, in five seconds.
pub const DEFAULT_SLIDE: Slide = Slide {
    from_percent: 15,
    to_percent: 85,
    duration_ms: 5_000,
    serial: 0,
    began_ms: 0,
    elapsed_ms: 0,
};
/// Zoom of a camera move and how long it takes when none are given.
pub const DEFAULT_ZOOM_PERCENT: u16 = 200;
pub const DEFAULT_CAMERA_MS: u64 = 1_200;

const PICTURE_SLOTS: &str = "tv-backdrop, tv-card or phone";

fn caption(words: &[String]) -> Result<Command, CommandError> {
    let [text, rest @ ..] = words else {
        return Err(CommandError::Usage(CAPTION_USAGE));
    };
    let mut speaker = None;
    for (_, value) in keyed(rest, "speaker=NAME", &["speaker"])? {
        speaker = Some(value.to_owned());
    }
    Ok(Command::Scene(SceneChange::Caption {
        speaker,
        text: text.clone(),
    }))
}

fn party(words: &[String]) -> Result<Command, CommandError> {
    const MEMBER_KEYS: [&str; 4] = ["hp", "max", "color", "conditions"];
    match words {
        [verb] if verb == "clear" => Ok(Command::Scene(SceneChange::PartyClear {})),
        [verb, name] if verb == "remove" => Ok(Command::Scene(SceneChange::PartyRemove {
            name: name.clone(),
        })),
        [verb, name, pairs @ ..] if verb == "add" => {
            let mut member = PartyMember {
                name: name.clone(),
                hit_points: 0,
                maximum_hit_points: 0,
                conditions: Vec::new(),
                color: None,
            };
            let mut has_hit_points = false;
            let mut has_maximum = false;
            for (key, value) in keyed(
                pairs,
                "hp=N, max=N, color=#rrggbb, conditions=a,b",
                &MEMBER_KEYS,
            )? {
                match key {
                    "hp" => {
                        member.hit_points = count("hp", value)?;
                        has_hit_points = true;
                    }
                    "max" => {
                        member.maximum_hit_points = count("max", value)?;
                        has_maximum = true;
                    }
                    "color" => member.color = Some(value.to_owned()),
                    _ => member.conditions = split_list(value),
                }
            }
            if !has_hit_points {
                return Err(CommandError::Usage(PARTY_USAGE));
            }
            if !has_maximum {
                member.maximum_hit_points = member.hit_points;
            }
            Ok(Command::Scene(SceneChange::PartyAdd { member }))
        }
        [verb, name, pairs @ ..] if verb == "set" && !pairs.is_empty() => {
            let (mut hit_points, mut maximum_hit_points, mut conditions) = (None, None, None);
            for (key, value) in keyed(
                pairs,
                "hp=N, max=N, conditions=a,b",
                &["hp", "max", "conditions"],
            )? {
                match key {
                    "hp" => hit_points = Some(count("hp", value)?),
                    "max" => maximum_hit_points = Some(count("max", value)?),
                    _ => conditions = Some(split_list(value)),
                }
            }
            Ok(Command::Scene(SceneChange::PartySet {
                name: name.clone(),
                hit_points,
                maximum_hit_points,
                conditions,
            }))
        }
        _ => Err(CommandError::Usage(PARTY_USAGE)),
    }
}

/// A choice as `label` or `label=explanation`.
fn choice(text: &str) -> Choice {
    match text.split_once('=') {
        Some((label, explain)) => Choice {
            label: label.to_owned(),
            explain: Some(explain.to_owned()),
        },
        None => Choice {
            label: text.to_owned(),
            explain: None,
        },
    }
}

fn choices(words: &[String]) -> Result<Command, CommandError> {
    match words {
        [verb] if verb == "clear" => Ok(Command::Scene(SceneChange::ClearChoices {})),
        [verb, labels @ ..] if verb == "suggested" => Ok(Command::Scene(SceneChange::Suggested {
            choices: labels.iter().map(|text| choice(text)).collect(),
        })),
        [verb, name, labels @ ..] if verb == "group" => Ok(Command::Scene(SceneChange::Group {
            name: name.clone(),
            choices: labels.iter().map(|text| choice(text)).collect(),
        })),
        [verb, entries @ ..] if verb == "unavailable" => {
            let mut choices = Vec::new();
            for text in entries {
                let (label, reason) = pair(text)?;
                choices.push(UnavailableChoice {
                    label: label.to_owned(),
                    reason: reason.to_owned(),
                });
            }
            Ok(Command::Scene(SceneChange::Unavailable { choices }))
        }
        _ => Err(CommandError::Usage(CHOICES_USAGE)),
    }
}

fn target(value: &str) -> Result<Target, CommandError> {
    Target::from_key(value).ok_or_else(|| bad_choice("on", value, "both, tv or phone"))
}

fn volume(value: &str) -> Result<u8, CommandError> {
    value
        .parse::<u8>()
        .ok()
        .filter(|volume| *volume <= MAXIMUM_VOLUME_PERCENT)
        .ok_or_else(|| {
            bad_choice(
                "volume",
                value,
                format!("a whole number from 0 to {MAXIMUM_VOLUME_PERCENT}"),
            )
        })
}

fn rotation(key: &str, value: &str, suffix: &str) -> Result<u16, CommandError> {
    value
        .parse::<u16>()
        .ok()
        .filter(|degrees| *degrees <= MAXIMUM_ROTATION_DEGREES)
        .ok_or_else(|| {
            bad_choice(
                key,
                value,
                format!("a whole number of degrees from 0 to {MAXIMUM_ROTATION_DEGREES}{suffix}"),
            )
        })
}

/// The slot a CLI word names: `tv-backdrop`, `tv-card` or `phone` (the words `playground::audit`
/// writes back), not the slot's wire key.
fn picture_slot(text: &str, allowed: &str) -> Result<ImageSlot, CommandError> {
    match text {
        "tv-backdrop" => Ok(ImageSlot::TvBackdrop),
        "tv-card" => Ok(ImageSlot::TvCard),
        "phone" => Ok(ImageSlot::PhoneImage),
        _ => Err(bad_choice("the picture slot", text, allowed)),
    }
}

fn image(words: &[String]) -> Result<Command, CommandError> {
    let [slot, media, rest @ ..] = words else {
        return Err(CommandError::Usage(IMAGE_USAGE));
    };
    let slot = picture_slot(slot, PICTURE_SLOTS)?;
    let (mut caption, mut rotation_degrees) = (None, 0_u16);
    for (key, value) in keyed(rest, "caption=TEXT, rotate=0-359", &["caption", "rotate"])? {
        if key == "rotate" {
            rotation_degrees = rotation(key, value, " (180 is upside down)")?;
        } else {
            caption = Some(value.to_owned());
        }
    }
    Ok(Command::Scene(SceneChange::Image {
        slot,
        media: media.clone(),
        caption,
        rotation_degrees,
        slide: None,
        layout: None,
    }))
}

fn percent(key: &str, value: &str) -> Result<u8, CommandError> {
    value
        .parse::<u8>()
        .ok()
        .filter(|share| *share <= MAXIMUM_SLIDE_PERCENT)
        .ok_or_else(|| {
            bad_choice(
                key,
                value,
                format!(
                    "a whole number from 0 to {} (a share of the screen's width)",
                    MAXIMUM_SLIDE_PERCENT
                ),
            )
        })
}

fn slide(words: &[String]) -> Result<Command, CommandError> {
    let [slot, media, rest @ ..] = words else {
        return Err(CommandError::Usage(SLIDE_USAGE));
    };
    let slot = picture_slot(slot, "tv-card (only the card can slide)")?;
    let mut moving = DEFAULT_SLIDE;
    let (mut caption, mut rotation_degrees) = (None, 0_u16);
    for (key, value) in keyed(
        rest,
        "from=N, to=N, over=MS, caption=TEXT, rotate=0-359",
        &["from", "to", "over", "caption", "rotate"],
    )? {
        match key {
            "from" => moving.from_percent = percent(key, value)?,
            "to" => moving.to_percent = percent(key, value)?,
            "over" => {
                moving.duration_ms = value
                    .parse::<u64>()
                    .ok()
                    .filter(|ms| (1..=MAXIMUM_SLIDE_MS).contains(ms))
                    .ok_or_else(|| {
                        bad_choice(
                            key,
                            value,
                            format!("a whole number of milliseconds from 1 to {MAXIMUM_SLIDE_MS}"),
                        )
                    })?;
            }
            "rotate" => rotation_degrees = rotation(key, value, "")?,
            _ => caption = Some(value.to_owned()),
        }
    }
    Ok(Command::Scene(SceneChange::Image {
        slot,
        media: media.clone(),
        caption,
        rotation_degrees,
        slide: Some(moving),
        layout: None,
    }))
}

fn camera(words: &[String], looking_at: Option<&str>) -> Result<Command, CommandError> {
    let mut zoom_percent = if looking_at.is_some() {
        DEFAULT_ZOOM_PERCENT
    } else {
        MINIMUM_ZOOM_PERCENT
    };
    let mut duration_ms = DEFAULT_CAMERA_MS;
    let (keys, allowed) = if looking_at.is_some() {
        (&["zoom", "over"][..], "zoom=N, over=MS")
    } else {
        (&["over"][..], "over=MS")
    };
    for (key, value) in keyed(words, allowed, keys)? {
        if key == "zoom" {
            zoom_percent = value
                .parse::<u16>()
                .ok()
                .filter(|percent| (MINIMUM_ZOOM_PERCENT..=MAXIMUM_ZOOM_PERCENT).contains(percent))
                .ok_or_else(|| {
                    bad_choice(
                        key,
                        value,
                        format!(
                            "a whole number of percent from {} to {} (200 is twice as large)",
                            MINIMUM_ZOOM_PERCENT, MAXIMUM_ZOOM_PERCENT
                        ),
                    )
                })?;
        } else {
            duration_ms = value
                .parse::<u64>()
                .ok()
                .filter(|ms| *ms <= MAXIMUM_CAMERA_MS)
                .ok_or_else(|| {
                    bad_choice(
                        key,
                        value,
                        format!("a whole number of milliseconds from 0 to {MAXIMUM_CAMERA_MS}"),
                    )
                })?;
        }
    }
    Ok(Command::Scene(SceneChange::Camera {
        target: looking_at.map(str::to_owned),
        zoom_percent,
        duration_ms,
    }))
}

fn music(words: &[String]) -> Result<Command, CommandError> {
    let [media, rest @ ..] = words else {
        return Err(CommandError::Usage(MUSIC_USAGE));
    };
    let (mut on, mut level, mut looping) = (Target::Both, DEFAULT_VOLUME, true);
    for (key, value) in keyed(rest, "on=, volume=, loop=", &["on", "volume", "loop"])? {
        match key {
            "on" => on = target(value)?,
            "volume" => level = volume(value)?,
            _ => looping = truth(key, value)?,
        }
    }
    Ok(Command::Scene(SceneChange::Music {
        media: media.clone(),
        target: on,
        volume_percent: level,
        looping,
    }))
}

fn sound(words: &[String]) -> Result<Command, CommandError> {
    let [media, rest @ ..] = words else {
        return Err(CommandError::Usage(SOUND_USAGE));
    };
    let (mut on, mut level) = (Target::Both, DEFAULT_VOLUME);
    for (key, value) in keyed(rest, "on=, volume=", &["on", "volume"])? {
        match key {
            "on" => on = target(value)?,
            _ => level = volume(value)?,
        }
    }
    Ok(Command::Scene(SceneChange::Sound {
        media: media.clone(),
        target: on,
        volume_percent: level,
    }))
}

/// Reads the words after `scene` into one live change.
pub(super) fn scene(words: &[String]) -> Result<Command, CommandError> {
    match words.split_first() {
        Some((verb, rest)) if verb == "caption" => caption(rest),
        Some((verb, rest)) if verb == "caption-clear" && rest.is_empty() => {
            Ok(Command::Scene(SceneChange::ClearCaption {}))
        }
        Some((verb, rest)) if verb == "party" => party(rest),
        Some((verb, rest)) if verb == "choices" => choices(rest),
        Some((verb, rest)) if verb == "image" => image(rest),
        Some((verb, rest)) if verb == "slide" => slide(rest),
        Some((verb, rest)) if verb == "camera" => match rest.split_first() {
            Some((target, rest)) => camera(rest, Some(target.as_str())),
            None => Err(CommandError::Usage(SCENE_CAMERA_USAGE)),
        },
        Some((verb, rest)) if verb == "camera-reset" => camera(rest, None),
        Some((verb, rest)) if verb == "camera-on" => {
            crate::cast::cli::parse_camera_on(rest).map(Command::Cast)
        }
        Some((verb, rest)) if verb == "cue" => crate::cast::cli::parse_cue(rest).map(Command::Cast),
        Some((verb, [])) if verb == "cast" => Ok(Command::Cast(crate::cast::CastCommand::List)),
        Some((verb, rest)) if verb == "image-clear" => match rest {
            [slot] => picture_slot(slot, PICTURE_SLOTS)
                .map(|slot| Command::Scene(SceneChange::ClearImage { slot })),
            _ => Err(CommandError::Usage(IMAGE_USAGE)),
        },
        Some((verb, rest)) if verb == "music" => music(rest),
        Some((verb, rest)) if verb == "music-stop" && rest.is_empty() => {
            Ok(Command::Scene(SceneChange::StopMusic {}))
        }
        Some((verb, rest)) if verb == "sound" => sound(rest),
        Some((verb, rest)) if verb == "reset" && rest.is_empty() => {
            Ok(Command::Scene(SceneChange::Reset {}))
        }
        Some((verb, rest)) if matches!(verb.as_str(), "lobby" | "loading" | "roleplay") => {
            lobby::parse(verb, rest)
        }
        Some((verb, rest)) if verb == "pill" => pills::parse(rest),
        Some((verb, rest)) if verb == "home" => home::parse(rest),
        Some((verb, rest)) if verb == "start" => crate::engine::parse_scene_start(rest)
            .map(|command| Command::Engine(crate::engine::EngineCommand::Play(command))),
        Some((verb, rest))
            if matches!(verb.as_str(), "build" | "show" | "lookahead" | "timing") =>
        {
            crate::scene_builder::parse_scene(verb, rest).map(Command::SceneBuilder)
        }
        Some((verb, rest)) if matches!(verb.as_str(), "narration" | "seat" | "seats" | "logo") => {
            ui::parse(verb, rest)
        }
        _ => Err(CommandError::Usage(SCENE_USAGE)),
    }
}
