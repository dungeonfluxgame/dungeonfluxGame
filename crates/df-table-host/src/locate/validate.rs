//! Input checks of the locate requests: picture names, mask counts and labels.

use df_imagery::oneshot_segment;
use df_playground::media::is_plain_file_name;

use super::limits::MAXIMUM_PICTURE_NAME_BYTES;
use super::request::LocateError;

/// How many instances a scan asks for when the caller does not say.
pub const DEFAULT_MASKS: u32 = 16;
const PICTURE_EXTENSIONS: [&str; 4] = ["png", "webp", "jpg", "jpeg"];

/// A picture of the library is named by its file name only, as the library names files (letters,
/// digits, `_`, `-` and `.`, no leading dot), with a picture extension. Such a name is also safe to
/// write in an audit line.
pub fn check_picture_name(name: &str) -> Result<(), LocateError> {
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase());
    let fine = is_plain_file_name(name)
        && name.len() <= MAXIMUM_PICTURE_NAME_BYTES
        && extension.is_some_and(|extension| PICTURE_EXTENSIONS.contains(&extension.as_str()));
    if fine {
        Ok(())
    } else {
        Err(LocateError::InvalidPictureName {
            name: name.to_owned(),
        })
    }
}

/// How many instances to ask for: 1 to the supplier's bound, `DEFAULT_MASKS` when not given.
pub fn check_masks(masks: Option<u32>) -> Result<u32, LocateError> {
    let masks = masks.unwrap_or(DEFAULT_MASKS);
    if (1..=oneshot_segment::MAXIMUM_MASKS).contains(&masks) {
        Ok(masks)
    } else {
        Err(LocateError::InvalidMaskCount { masks })
    }
}

/// A region label: a character name (`df_scene_layout::is_character_name`).
pub fn check_label(label: &str) -> Result<(), LocateError> {
    if df_scene_layout::is_character_name(label) {
        Ok(())
    } else {
        Err(LocateError::InvalidLabel {
            label: label.to_owned(),
        })
    }
}
