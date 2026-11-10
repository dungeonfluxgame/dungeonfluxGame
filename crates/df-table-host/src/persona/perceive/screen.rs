//! The picture a perception looks at: the one on screen now (the shown backdrop, with the layout
//! the page draws it with) or one the request names, its scan, and its bytes when the model is to
//! be given the picture itself.
//!
//! A close-up insert, or a room picture shown with the cast of the room before it, has no scan
//! that fits what it shows (`PlateKind`): its layout is left out, so no region is offered that is
//! not where it says.

use std::path::Path;

use df_ai::text::TextImageType;
use df_playground::fixture::PlateKind;
use df_scene_layout::SceneLayout;

use super::request::PerceiveCause;
use crate::blocking::run_blocking;
use crate::locate::request::LocateError;
use crate::locate::{LAYOUT_DIRECTORY, current_layout, shown_backdrop};
use crate::media_library::media_directory;
use crate::shown_scene::ShownSceneError;
use crate::state::AppState;

/// What a perception looks at.
pub(super) struct Screen {
    /// The picture looked at: a library picture that was read. None when it could not be.
    pub(super) picture: Option<String>,
    /// The scan of the picture, when it has one that fits it.
    pub(super) layout: Option<SceneLayout>,
    /// Whether the picture is an insert filling the screen with one thing.
    pub(super) close_up: bool,
    /// The picture itself, when it was asked for and could be read as an image.
    pub(super) image: Option<(TextImageType, Vec<u8>)>,
}

/// The image type a picture of the library is sent as, by its extension.
fn image_type(picture: &str) -> Option<TextImageType> {
    match picture
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some(TextImageType::Png),
        Some("jpg" | "jpeg") => Some(TextImageType::Jpeg),
        Some("webp") => Some(TextImageType::Webp),
        _ => None,
    }
}

/// Whether `picture` is a picture of the library a model can be given, and its bytes and the type
/// to send them as when `vision` asks for them.
fn read_image(
    media: &Path,
    picture: &str,
    vision: bool,
) -> Option<Option<(TextImageType, Vec<u8>)>> {
    let kind = image_type(picture)?;
    let path = media.join(picture);
    if !vision {
        return path.is_file().then_some(None);
    }
    let bytes = std::fs::read(path).ok().filter(|bytes| !bytes.is_empty())?;
    Some(Some((kind, bytes)))
}

/// The picture of the request, or the one on screen now, with its scan and (when `vision`) its
/// bytes.
pub(super) async fn read_screen(
    app: &AppState,
    named: Option<&str>,
    vision: bool,
) -> Result<Screen, PerceiveCause> {
    let (picture, layout, close_up) = match named {
        Some(name) => {
            crate::locate::check_picture_name(name)
                .map_err(|cause| PerceiveCause::Picture { cause })?;
            let (media, layouts) = (
                media_directory(app.playground().files()),
                app.playground().files().assets().join(LAYOUT_DIRECTORY),
            );
            let picture = name.to_owned();
            let layout = run_blocking(
                move || current_layout(&media, &layouts, &picture),
                LocateError::Stopped,
            )
            .await
            .map_err(|cause| PerceiveCause::Picture { cause })?;
            (name.to_owned(), layout, false)
        }
        None => {
            let (fixture, _) = crate::playground::shown_fixture(app)
                .await
                .map_err(|error| PerceiveCause::Shown {
                    cause: ShownSceneError::from(error),
                })?;
            let backdrop = shown_backdrop(app, &fixture)
                .await
                .map_err(|error| PerceiveCause::Shown {
                    cause: ShownSceneError::from(error),
                })?
                .ok_or(PerceiveCause::Shown {
                    cause: ShownSceneError::NoBackdrop,
                })?;
            let close_up = backdrop.plate == PlateKind::CloseUp;
            let fits = backdrop.plate == PlateKind::Room;
            (backdrop.picture, backdrop.layout.filter(|_| fits), close_up)
        }
    };
    let media = media_directory(app.playground().files());
    let name = picture.clone();
    let read = run_blocking(move || Ok::<_, ()>(read_image(&media, &name, vision)), ())
        .await
        .unwrap_or(None);
    Ok(Screen {
        // A picture that is not in the library cannot be named by a memory, so it is not named.
        picture: read.is_some().then_some(picture),
        layout,
        close_up,
        image: read.flatten(),
    })
}
