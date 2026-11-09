//! `scene music generate` against a host: send the request, read the typed answer, print it. Its
//! grammar is with `scene music`'s in `command::scene`.

use super::request::{MusicError, MusicPlayed, MusicRequest, MusicSource};
use super::routes::path;
use crate::dev::DevError;
use crate::dev_client;
use crate::dev_http::DevReply;

/// What the command line prints for music now playing.
pub(crate) fn format_played(played: &MusicPlayed) -> String {
    let origin = match played.source {
        MusicSource::Library => "from the library, nothing composed".to_owned(),
        MusicSource::Composed => format!(
            "composed: {} ms asked, {} bytes, took {} ms",
            played.length_ms.unwrap_or_default(),
            played.bytes,
            played.compose_ms.unwrap_or_default()
        ),
    };
    format!(
        "music {} ({origin}), looping on {} at {}% (revision {})",
        played.file,
        played.target.key(),
        played.volume_percent,
        played.revision
    )
}

/// Runs one `scene music generate` against the host on `port`.
pub async fn run(port: u16, request: MusicRequest) -> Result<String, DevError> {
    let answer: DevReply<MusicPlayed, MusicError> =
        dev_client::post_json(port, path::SCENE_MUSIC, &request).await?;
    dev_client::reply_value(answer).map(|played| format_played(&played))
}
