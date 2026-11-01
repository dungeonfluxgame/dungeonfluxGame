//! Read-only import of bounded historical manifest members; never writes or invokes a provider.
use super::{ImageFormat, ImageKey};
use bytes::Bytes;
use serde::Deserialize;
use std::{io, path::Path};
const MAXIMUM_IMAGE_BYTES: u64 = 6 * 1024 * 1024;
const MAXIMUM_MANIFEST_RECORD_BYTES: u64 = 256 * 1024;
const MAXIMUM_MANIFEST_TAIL_BYTES: u64 = 8 * 1024 * 1024;
#[derive(Deserialize)]
struct ManifestLine {
    key: String,
    #[serde(rename = "kind")]
    _kind: String,
    #[serde(rename = "model")]
    _model: String,
    #[serde(rename = "prompt")]
    _prompt: String,
    #[serde(rename = "references")]
    _references: Vec<String>,
    #[serde(rename = "created_unix")]
    _created_unix: u64,
    #[serde(rename = "bytes")]
    _bytes: u64,
    #[serde(rename = "width")]
    _width: u32,
    #[serde(rename = "height")]
    _height: u32,
    transparent: bool,
    #[serde(rename = "micro_dollars")]
    _micro_dollars: Option<u64>,
    #[serde(default, rename = "supplier_credits")]
    _supplier_credits: Option<String>,
    #[serde(default, rename = "elapsed_ms")]
    _elapsed_ms: u64,
}
pub(super) fn manifest_tail(path: &Path) -> Option<std::io::BufReader<std::fs::File>> {
    use std::io::{BufRead, Seek, SeekFrom};
    let file = open_manifest(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() {
        return None;
    }
    let mut reader = std::io::BufReader::new(file);
    if metadata.len() > MAXIMUM_MANIFEST_TAIL_BYTES {
        // The line the tail starts in is partial, so it is dropped with the bytes before it.
        let start = metadata.len() - MAXIMUM_MANIFEST_TAIL_BYTES;
        reader.seek(SeekFrom::Start(start)).ok()?;
        reader.skip_until(b'\n').ok()?;
    }
    Some(reader)
}

/// Each record's key and format, oldest first. Lines are read one at a time, so a torn, foreign or
/// overlong line costs only that record.
pub(super) fn manifest_records(
    mut reader: impl std::io::BufRead,
) -> impl Iterator<Item = (ImageKey, ImageFormat)> {
    use std::io::{BufRead, Read};
    let mut line = Vec::new();
    std::iter::from_fn(move || {
        loop {
            line.clear();
            let read = reader
                .by_ref()
                .take(MAXIMUM_MANIFEST_RECORD_BYTES)
                .read_until(b'\n', &mut line)
                .ok()?;
            if read == 0 {
                return None;
            }
            let overlong = !line.ends_with(b"\n")
                && u64::try_from(line.len())
                    .is_ok_and(|length| length == MAXIMUM_MANIFEST_RECORD_BYTES);
            if overlong {
                // The rest of this overlong line is skipped without being buffered.
                reader.skip_until(b'\n').ok()?;
                continue;
            }
            let Ok(entry) = serde_json::from_slice::<ManifestLine>(&line) else {
                continue;
            };
            let Some(key) = ImageKey::parse(&entry.key) else {
                continue;
            };
            let format = if entry.transparent {
                ImageFormat::Webp
            } else {
                ImageFormat::Jpeg
            };
            return Some((key, format));
        }
    })
}

pub(super) fn read_cache_file(path: &Path) -> Option<Bytes> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAXIMUM_IMAGE_BYTES {
        return None;
    }
    std::fs::read(path).ok().map(Bytes::from)
}

fn open_manifest(path: &Path) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        #[cfg(all(target_os = "linux", any(target_arch = "x86", target_arch = "x86_64")))]
        const FLAGS: i32 = 0x20000 | 0x800;
        #[cfg(all(target_os = "linux", any(target_arch = "aarch64", target_arch = "arm")))]
        const FLAGS: i32 = 0x8000 | 0x800;
        #[cfg(any(target_os = "macos", target_os = "freebsd"))]
        const FLAGS: i32 = 0x100 | 0x4;
        #[cfg(not(any(
            all(target_os = "linux", any(target_arch = "x86", target_arch = "x86_64")),
            all(target_os = "linux", any(target_arch = "aarch64", target_arch = "arm")),
            target_os = "macos",
            target_os = "freebsd"
        )))]
        const FLAGS: i32 = 0;
        options.custom_flags(FLAGS);
    }
    options.open(path)
}

pub(super) fn legacy_file_name(key: &ImageKey, format: ImageFormat) -> String {
    let extension = match format {
        ImageFormat::Png => "png",
        ImageFormat::Webp => "webp",
        ImageFormat::Jpeg => "jpg",
    };
    format!("{}.{extension}", key.as_str())
}

/// The formats an image of `format` may be found in: a cut-out cached before WebP output
/// is a PNG of the same request.
pub(super) fn legacy_stored_formats(format: ImageFormat) -> &'static [ImageFormat] {
    match format {
        ImageFormat::Webp => &[ImageFormat::Webp, ImageFormat::Png],
        ImageFormat::Png => &[ImageFormat::Png],
        ImageFormat::Jpeg => &[ImageFormat::Jpeg],
    }
}
