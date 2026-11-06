//! Layout storage: the layout files in the playground's `layouts` folder, the check that a stored
//! layout is bound to its picture as it is now, and attaching it to a fixture's backdrop.

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use df_imagery::oneshot_segment;
use df_playground::fixture::Fixture;
use df_scene_layout::{SCHEMA_VERSION, SceneLayout};
use serde::Deserialize;

use super::request::{IoCause, LocateError};
use super::validate::check_picture_name;
use crate::media_library::media_directory;
use crate::state::AppState;

/// Held around every read-modify-write of a layout file (scan, label, identify), so one never
/// overwrites names or regions another stored meanwhile. Taken only inside blocking work, never
/// across an `.await`; one lock for the folder, since layout writes are rare and short.
static LAYOUT_WRITES: Mutex<()> = Mutex::new(());

/// The folder of stored layouts under the playground's assets, apart from the media library.
pub const LAYOUT_DIRECTORY: &str = "layouts";
/// What a layout file's name adds to its picture's name.
pub const LAYOUT_SUFFIX: &str = ".layout.json";

/// The layout file of `picture`.
pub fn layout_file_name(picture: &str) -> String {
    format!("{picture}{LAYOUT_SUFFIX}")
}

/// Takes the layout write lock ([`LAYOUT_WRITES`]); a panic of an earlier holder left no file half
/// written (`write_layout` is whole or not at all), so a poisoned lock is still good.
pub(super) fn lock_layout_writes() -> MutexGuard<'static, ()> {
    LAYOUT_WRITES.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The bytes of `picture` in the media library: a missing file is `PictureMissing`, any other
/// failure to read it is `PictureUnreadable` with its cause.
pub(super) fn read_picture(media: &Path, picture: &str) -> Result<Vec<u8>, LocateError> {
    std::fs::read(media.join(picture)).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            LocateError::PictureMissing {
                picture: picture.to_owned(),
            }
        } else {
            LocateError::picture_unreadable(picture, &error)
        }
    })
}

/// The schema version a stored layout file says it is in, read apart from the rest of the file
/// (whose other fields are not looked at), to tell an outdated layout from a damaged one.
#[derive(Deserialize)]
struct WrittenSchema {
    schema: u32,
}

/// The stored layout of `picture` in the `layouts` folder, or `None` when it has none. A layout in
/// another schema version does not read and is refused as stale (scan again); any other file that
/// does not read is `LayoutInvalid`.
pub fn read_layout(layouts: &Path, picture: &str) -> Result<Option<SceneLayout>, LocateError> {
    let path = layouts.join(layout_file_name(picture));
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(LocateError::LayoutUnreadable {
                picture: picture.to_owned(),
                cause: IoCause::from_kind(error.kind()),
            });
        }
    };
    serde_json::from_str(&text).map(Some).map_err(|error| {
        match serde_json::from_str::<WrittenSchema>(&text) {
            Ok(written) if written.schema != SCHEMA_VERSION => LocateError::StaleLayout {
                picture: picture.to_owned(),
            },
            _ => LocateError::LayoutInvalid {
                picture: picture.to_owned(),
                detail: error.to_string(),
            },
        }
    })
}

/// Writes `layout` as the layout file of `picture` in the `layouts` folder (made if missing),
/// whole or not at all.
pub fn write_layout(
    layouts: &Path,
    picture: &str,
    layout: &SceneLayout,
) -> Result<(), LocateError> {
    let unwritable = |cause| LocateError::LayoutUnwritable {
        picture: picture.to_owned(),
        cause,
    };
    // A layout always writes as JSON; the mapping only keeps the path panic-free.
    let text =
        serde_json::to_string_pretty(layout).map_err(|_| unwritable(IoCause::InvalidData))?;
    crate::atomic_file::write_atomically(layouts, &layout_file_name(picture), text.as_bytes())
        .map_err(|error| unwritable(IoCause::from_kind(error.kind())))
}

/// A fixture's text with its backdrop's current layout attached, when the backdrop has a picture
/// with a layout bound to it as it is now and does not carry a layout of its own. A stale layout
/// is not attached. Anything that does not read is served as it was: the page then reports the
/// fixture's own problem, not this one.
pub fn attach_layout(bytes: Vec<u8>, media: &Path, layouts: &Path) -> Vec<u8> {
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return bytes;
    };
    let Some(backdrop) = value
        .get_mut("tv")
        .and_then(|tv| tv.get_mut("backdrop"))
        .and_then(serde_json::Value::as_object_mut)
    else {
        return bytes;
    };
    if backdrop.contains_key("layout") {
        return bytes;
    }
    let Some(picture) = backdrop
        .get("media")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
    else {
        return bytes;
    };
    if check_picture_name(&picture).is_err() {
        return bytes;
    }
    let Ok(Some(layout)) = current_layout(media, layouts, &picture) else {
        return bytes;
    };
    let Ok(layout) = serde_json::to_value(layout) else {
        return bytes;
    };
    backdrop.insert("layout".to_owned(), layout);
    serde_json::to_vec_pretty(&value).unwrap_or(bytes)
}

/// The stored layout of `picture`, only if it is bound to the picture as it is now: same name,
/// same digest, this schema. `Ok(None)` when there is none; a layout of another picture (by its
/// binding) or another schema version (on read, [`read_layout`]) is refused as stale (scan again),
/// never used.
pub fn current_layout(
    media: &Path,
    layouts: &Path,
    picture: &str,
) -> Result<Option<SceneLayout>, LocateError> {
    let Some(layout) = read_layout(layouts, picture)? else {
        return Ok(None);
    };
    let bytes = read_picture(media, picture)?;
    if layout.is_for(picture, &oneshot_segment::picture_sha256(&bytes)) {
        Ok(Some(layout))
    } else {
        Err(LocateError::StaleLayout {
            picture: picture.to_owned(),
        })
    }
}

/// The backdrop picture the live scene shows and its layout, as the page draws them.
pub(crate) struct ShownBackdrop {
    pub(crate) picture: String,
    pub(crate) layout: Option<SceneLayout>,
}

/// The backdrop the page draws: the live scene's own when it changed it (with the layout attached
/// at that change), otherwise `fixture`'s (with its own layout, or else its picture's current
/// stored one, as `attach_layout` serves it). `Ok(None)` when no backdrop is shown; a stored layout
/// that does not read or is stale is that refusal. The picture is read and hashed off the workers.
pub(crate) async fn shown_backdrop(
    state: &AppState,
    fixture: &Fixture,
) -> Result<Option<ShownBackdrop>, LocateError> {
    let backdrop = state.playground().scene_backdrop();
    let from_scene = backdrop.set_value().is_some();
    let Some(image) = backdrop.drawn(fixture.tv.backdrop.as_ref()) else {
        return Ok(None);
    };
    // The page reads no stored layout for a scene's own backdrop or one that carries a layout, and
    // a name that is not a library picture's never has one.
    if from_scene || image.layout.is_some() || check_picture_name(&image.media).is_err() {
        return Ok(Some(ShownBackdrop {
            picture: image.media,
            layout: image.layout,
        }));
    }
    let files = state.playground().files();
    let (media, layouts) = (
        media_directory(files),
        files.assets().join(LAYOUT_DIRECTORY),
    );
    let picture = image.media.clone();
    let layout = tokio::task::spawn_blocking(move || current_layout(&media, &layouts, &picture))
        .await
        .map_err(|_| LocateError::Stopped)??;
    Ok(Some(ShownBackdrop {
        picture: image.media,
        layout,
    }))
}
