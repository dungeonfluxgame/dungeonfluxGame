//! One buffered transcription through Forge's realtime speech route (`WS /api/asr/realtime`).
//!
//! Forge proxies NVIDIA's PCM16 protocol unchanged (`src/asr-stream.js` is its browser client):
//! wait for `session.created`, send `session.update` with the sample rate, wait for
//! `session.updated`, send the audio as binary frames, send `input_audio_buffer.commit` and read
//! `conversation.item.input_audio_transcription.completed`. Forge starts Nemotron without
//! endpointing, so the commit gives the one final transcript of the whole recording. It is the
//! route Forge documents for audio that is not a project file; the recording is never stored.

use futures_util::{SinkExt, StreamExt};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;

use super::{ForgeCause, ForgeError};

/// The route on Forge's origin.
const ROUTE: &str = "/api/asr/realtime";
/// Bytes of audio in one frame: half a second at 16 kHz, well under Forge's 1 MiB frame bound.
const FRAME_BYTES: usize = 16_000;
/// The largest event read, in bytes (a transcript of a minute of speech is a few KiB).
const MAXIMUM_EVENT_BYTES: usize = 1024 * 1024;
/// The largest session Forge accepts, in bytes (its proxy's bound).
const MAXIMUM_SESSION_BYTES: usize = 64 * 1024 * 1024;
/// The sample rates Forge accepts, in Hz (its client's bounds).
const SAMPLE_RATES_HZ: std::ops::RangeInclusive<u32> = 8_000..=96_000;

/// The fields of a protocol event this client reads.
struct Event {
    kind: String,
    transcript: Option<String>,
}

/// Transcribes `pcm` within `until`; see the module.
pub(super) async fn transcribe(
    origin: &str,
    pcm: &[u8],
    sample_rate_hz: u32,
    until: Instant,
) -> Result<String, ForgeError> {
    if pcm.is_empty()
        || !pcm.len().is_multiple_of(2)
        || pcm.len() > MAXIMUM_SESSION_BYTES
        || !SAMPLE_RATES_HZ.contains(&sample_rate_hz)
    {
        return Err(ForgeError::new(ForgeCause::InvalidRequest));
    }
    let started = Instant::now();
    let mut ready_ms = 0;
    let outcome = tokio::time::timeout_at(
        until,
        session(origin, pcm, sample_rate_hz, started, &mut ready_ms),
    )
    .await
    .unwrap_or(Err(ForgeError::new(ForgeCause::Deadline)));
    df_observe::record_audit_line(
        "FORGE",
        format!(
            "FORGE transcribed audio_bytes={} ready_ms={ready_ms} duration_ms={} outcome={}",
            pcm.len(),
            df_types::saturating_millis(started.elapsed()),
            outcome
                .as_ref()
                .map_or_else(|error| error.cause.key(), |_| "completed"),
        ),
    );
    outcome
}

async fn session(
    origin: &str,
    pcm: &[u8],
    sample_rate_hz: u32,
    started: Instant,
    ready_ms: &mut u64,
) -> Result<String, ForgeError> {
    let session_error = || ForgeError::new(ForgeCause::Session);
    let authority = origin
        .strip_prefix("http://")
        .ok_or(ForgeError::new(ForgeCause::InvalidConfiguration))?;
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{authority}{ROUTE}"))
        .await
        .map_err(|_| ForgeError::new(ForgeCause::EngineUnavailable))?;
    expect(&mut socket, "session.created").await?;
    let update = serde_json::json!({
        "type": "session.update",
        "session": { "sample_rate": sample_rate_hz },
    });
    socket
        .send(Message::Text(update.to_string().into()))
        .await
        .map_err(|_| session_error())?;
    expect(&mut socket, "session.updated").await?;
    *ready_ms = df_types::saturating_millis(started.elapsed());
    for frame in pcm.chunks(FRAME_BYTES) {
        socket
            .send(Message::Binary(frame.to_vec().into()))
            .await
            .map_err(|_| session_error())?;
    }
    let commit = serde_json::json!({ "type": "input_audio_buffer.commit" });
    socket
        .send(Message::Text(commit.to_string().into()))
        .await
        .map_err(|_| session_error())?;
    loop {
        let event = next_event(&mut socket).await?;
        if event.kind == "conversation.item.input_audio_transcription.completed" {
            let transcript = event.transcript.ok_or_else(session_error)?;
            // Closing is a courtesy: the transcript is already read.
            let _ = socket.close(None).await;
            return Ok(transcript.trim().to_owned());
        }
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Reads events until one of `kind`; a delta or another informational event is skipped.
async fn expect(socket: &mut Socket, kind: &str) -> Result<(), ForgeError> {
    loop {
        if next_event(socket).await?.kind == kind {
            return Ok(());
        }
    }
}

/// The next protocol event; an `error` event, a close or an event that does not read ends the
/// session.
async fn next_event(socket: &mut Socket) -> Result<Event, ForgeError> {
    let session_error = || ForgeError::new(ForgeCause::Session);
    loop {
        let message = socket
            .next()
            .await
            .ok_or_else(session_error)?
            .map_err(|_| session_error())?;
        let text = match message {
            Message::Text(text) => text,
            Message::Ping(_) | Message::Pong(_) => continue,
            _ => return Err(session_error()),
        };
        if text.len() > MAXIMUM_EVENT_BYTES {
            return Err(session_error());
        }
        let value: serde_json::Value = serde_json::from_str(&text).map_err(|_| session_error())?;
        let event = Event {
            kind: value["type"].as_str().ok_or_else(session_error)?.to_owned(),
            transcript: value["transcript"].as_str().map(str::to_owned),
        };
        if event.kind == "error" {
            return Err(session_error());
        }
        return Ok(event);
    }
}
