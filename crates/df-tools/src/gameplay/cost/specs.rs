//! The call specs and settlements of the routes metered outside the storyteller, voice and
//! picture paths: transcription, embeddings, segmentation, music, video and generated worlds.
//! Each spec states its maxima in the route's own categories, so a call of unknown outcome is
//! booked at what it could have used and a prepaid admission holds a figure for every category.

use df_providers::{
    ELEVEN_STT_MODEL, ElevenMusicError, FalSegmentError, MUSIC_MODEL, SOUND_MODEL, WorldLabsError,
    refusal_may_be_billed, status_may_be_billed,
};

use super::{BilledCall, CallSpec, Category, Cause, Provider, Usage};

/// The spec of one transcription of `seconds` of audio, the whole seconds the supplier bills.
pub(in crate::gameplay) fn stt_call(seconds: u64) -> CallSpec {
    CallSpec::new(
        Provider::ElevenLabs,
        ELEVEN_STT_MODEL,
        Cause::Transcription,
        "clip",
    )
    .with_maxima(vec![(Category::AudioSeconds, seconds)])
}

/// The spec of one embedding call of the story memory, with the token estimate of its texts.
pub(in crate::gameplay) fn embedding_call(model: &str, tokens: u64) -> CallSpec {
    CallSpec::new(Provider::OpenAi, model, Cause::Embedding, "story_memory")
        .with_maxima(vec![(Category::TextInput, tokens)])
}

/// The spec of one segmentation request to the fal endpoint `endpoint`, named `detail`.
pub(in crate::gameplay) fn segment_call(endpoint: &str, detail: &str) -> CallSpec {
    CallSpec::new(Provider::Fal, endpoint, Cause::Segmentation, detail)
        .with_maxima(vec![(Category::Calls, 1)])
}

/// The spec of one music layer (`layer` names it) of `seconds` of source audio.
pub(in crate::gameplay) fn music_layer_call(layer: &str, seconds: u64) -> CallSpec {
    CallSpec::new(Provider::ElevenLabs, MUSIC_MODEL, Cause::PlaceMusic, layer)
        .with_maxima(vec![(Category::AudioSeconds, seconds)])
}

/// The spec of one ambience bed of `seconds`.
pub(in crate::gameplay) fn ambience_call(detail: &str, seconds: u64) -> CallSpec {
    CallSpec::new(
        Provider::ElevenLabs,
        SOUND_MODEL,
        Cause::PlaceAmbience,
        detail,
    )
    .with_maxima(vec![(Category::AudioSeconds, seconds)])
}

/// The spec of one generated video clip of `seconds` by the fal model `model`.
pub(in crate::gameplay) fn video_call(
    model: &str,
    cause: Cause,
    detail: &str,
    seconds: u64,
) -> CallSpec {
    CallSpec::new(Provider::Fal, model, cause, detail)
        .with_maxima(vec![(Category::VideoSeconds, seconds)])
}

/// The spec of one generated world by the World Labs model `model`, with the credits it costs.
pub(in crate::gameplay) fn world_call(model: &str, credits: u64) -> CallSpec {
    CallSpec::new(Provider::WorldLabs, model, Cause::SplatWorld, "world")
        .with_maxima(vec![(Category::Credits, credits)])
}

/// Whether a failed music or ambience request may have been billed.
pub(in crate::gameplay) const fn music_error_may_be_billed(error: ElevenMusicError) -> bool {
    match error {
        ElevenMusicError::InvalidRequest | ElevenMusicError::Unauthorized => false,
        ElevenMusicError::Provider(status) => status_may_be_billed(status),
        ElevenMusicError::Transport(refusal) => refusal_may_be_billed(refusal),
        ElevenMusicError::Output => true,
    }
}

/// Whether a failed segmentation request may have been billed.
pub(in crate::gameplay) const fn segment_error_may_be_billed(error: FalSegmentError) -> bool {
    match error {
        FalSegmentError::InvalidRequest | FalSegmentError::Unauthorized => false,
        FalSegmentError::Provider(status) => status_may_be_billed(status),
        FalSegmentError::Transport(refusal) => refusal_may_be_billed(refusal),
        FalSegmentError::Queue | FalSegmentError::Failed | FalSegmentError::Output => true,
    }
}

/// Whether a failed World Labs request may have been billed: a request the supplier refused
/// (invalid, unauthorised, out of credits, rate limited, rejected by its content policy) was not.
pub(in crate::gameplay) const fn world_error_may_be_billed(error: WorldLabsError) -> bool {
    match error {
        WorldLabsError::InvalidRequest
        | WorldLabsError::Unauthorized
        | WorldLabsError::InsufficientCredits
        | WorldLabsError::RateLimited
        | WorldLabsError::ContentPolicy => false,
        WorldLabsError::Http(status) => status_may_be_billed(status),
        WorldLabsError::Timeout | WorldLabsError::Output | WorldLabsError::Transport => true,
    }
}

/// Settles a call that failed: not billed when the failure cannot have been billed, else of
/// unknown outcome (dropped, so it is booked at its maxima).
pub(in crate::gameplay) fn settle_failure(call: BilledCall, may_have_been_billed: bool) {
    if may_have_been_billed {
        drop(call);
    } else {
        call.not_billed();
    }
}

/// Settles a call that succeeded with units the server counted rather than the supplier
/// reported (seconds of audio sent, requests made).
pub(in crate::gameplay) fn settle_counted(call: BilledCall, quantities: Vec<(Category, u64)>) {
    call.settle(Usage::estimated(quantities));
}
