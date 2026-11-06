//! The range checks every value passes at the edge, whether it comes from a live change or from a
//! fixture file.
//!
//! There is one copy of each rule, so a rotation of 400 is refused the same way from the command
//! line, from the overlay and from a scenario file. A check never changes a value and never draws.

use crate::fixture::{Choice, PartyMember, Slide};
use crate::limits::{
    MAXIMUM_CAMERA_MS, MAXIMUM_CHOICES_PER_LIST, MAXIMUM_NAME_CHARACTERS, MAXIMUM_ROTATION_DEGREES,
    MAXIMUM_SLIDE_MS, MAXIMUM_SLIDE_PERCENT, MAXIMUM_VOLUME_PERCENT, MAXIMUM_ZOOM_PERCENT,
    MINIMUM_ZOOM_PERCENT,
};
use crate::scene::SceneError;

/// Text that is not blank and not longer than `limit` characters.
pub fn text(what: &'static str, value: &str, limit: usize) -> Result<(), SceneError> {
    if value.trim().is_empty() {
        Err(SceneError::Empty(what))
    } else if value.chars().count() > limit {
        Err(SceneError::TooLong { what, limit })
    } else {
        Ok(())
    }
}

/// A short piece of text: a name, a label, a speaker or a reason.
pub fn labelled(what: &'static str, value: &str) -> Result<(), SceneError> {
    text(what, value, MAXIMUM_NAME_CHARACTERS)
}

pub fn volume(percent: u8) -> Result<(), SceneError> {
    if percent > MAXIMUM_VOLUME_PERCENT {
        Err(SceneError::InvalidVolume(percent))
    } else {
        Ok(())
    }
}

/// A turn is from 0 to 359 degrees; 360 is the same as 0 and is refused with the range.
pub fn rotation(degrees: u16) -> Result<(), SceneError> {
    if degrees > MAXIMUM_ROTATION_DEGREES {
        Err(SceneError::InvalidRotation(degrees))
    } else {
        Ok(())
    }
}

/// A slide goes between two shares of the screen from 0 to 100 percent, and takes 1 or more
/// milliseconds, up to the longest allowed.
pub fn slide(slide: &Slide) -> Result<(), SceneError> {
    if slide.from_percent > MAXIMUM_SLIDE_PERCENT
        || slide.to_percent > MAXIMUM_SLIDE_PERCENT
        || !(1..=MAXIMUM_SLIDE_MS).contains(&slide.duration_ms)
    {
        Err(SceneError::InvalidSlide)
    } else {
        Ok(())
    }
}

/// A camera look: a target that is a name (when there is one), a zoom from the whole stage up to
/// the largest, and a move that takes no more than the longest allowed.
pub fn camera(target: Option<&str>, zoom_percent: u16, duration_ms: u64) -> Result<(), SceneError> {
    if let Some(target) = target {
        labelled("a camera target", target)?;
    }
    if !(MINIMUM_ZOOM_PERCENT..=MAXIMUM_ZOOM_PERCENT).contains(&zoom_percent) {
        return Err(SceneError::InvalidZoom(zoom_percent));
    }
    if duration_ms > MAXIMUM_CAMERA_MS {
        return Err(SceneError::InvalidCameraTime(duration_ms));
    }
    Ok(())
}

pub fn count(count: usize) -> Result<(), SceneError> {
    if count > MAXIMUM_CHOICES_PER_LIST {
        Err(SceneError::TooManyChoices {
            limit: MAXIMUM_CHOICES_PER_LIST,
        })
    } else {
        Ok(())
    }
}

pub fn choice(choice: &Choice) -> Result<(), SceneError> {
    labelled("a choice label", &choice.label)?;
    if let Some(explain) = &choice.explain {
        labelled("a choice explanation", explain)?;
    }
    Ok(())
}

pub fn member(member: &PartyMember) -> Result<(), SceneError> {
    labelled("a party member's name", &member.name)?;
    if member.maximum_hit_points == 0 {
        return Err(SceneError::InvalidMaximum);
    }
    for condition in &member.conditions {
        labelled("a condition", condition)?;
    }
    Ok(())
}
