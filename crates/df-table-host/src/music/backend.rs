//! The composer: one instrumental from a prompt. The ElevenLabs adapter is the only one the host
//! runs; the trait is there so the reuse-or-compose flow is tested over a fake that counts calls.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use df_playground::voice_wire::MissingSetting;
use df_providers::{
    ElevenMusicConfig, ElevenMusicError, MUSIC_MODEL, NativeElevenMusicProvider, NativeHttpCause,
};

use super::limits::COMPOSE_DEADLINE;
use super::request::{MusicError, MusicProviderCause};
use crate::speech::limits::KEY_VARIABLES;

/// The supplier's name in audit lines.
pub(super) const ELEVENLABS: &str = "elevenlabs";

/// The answer of one composition: MP3 bytes, or why there are none.
pub(super) type ComposeFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<u8>, MusicError>> + Send + 'a>>;

/// Makes one instrumental of `length_ms` from `prompt` within `deadline`; never retried.
pub(super) trait Composer: Send + Sync {
    /// The supplier, as audit lines name it.
    fn provider(&self) -> &'static str;
    /// The supplier's model, as audit lines name it.
    fn model(&self) -> &'static str;
    fn compose<'a>(
        &'a self,
        prompt: &'a str,
        length_ms: u32,
        deadline: Duration,
    ) -> ComposeFuture<'a>;
}

/// ElevenLabs `/v1/music` over the speech key (`DF_ELEVENLABS_API_KEY`), or a composer that
/// refuses every call when the key is not set or not usable.
pub(super) struct ElevenLabsComposer {
    provider: Result<NativeElevenMusicProvider, MusicError>,
}

impl ElevenLabsComposer {
    pub(super) fn from_environment() -> Self {
        let provider = match crate::environment::first_text(&KEY_VARIABLES) {
            None => Err(MusicError::NotConfigured {
                missing: MissingSetting::SpeechKey,
            }),
            Some(key) => {
                NativeElevenMusicProvider::new(ElevenMusicConfig::live(), &key, COMPOSE_DEADLINE)
                    .map_err(provider_error)
            }
        };
        Self { provider }
    }
}

impl Composer for ElevenLabsComposer {
    fn provider(&self) -> &'static str {
        ELEVENLABS
    }

    fn model(&self) -> &'static str {
        MUSIC_MODEL
    }

    fn compose<'a>(
        &'a self,
        prompt: &'a str,
        length_ms: u32,
        deadline: Duration,
    ) -> ComposeFuture<'a> {
        Box::pin(async move {
            let provider = self.provider.as_ref().map_err(Clone::clone)?;
            provider
                .compose(prompt, length_ms, deadline)
                .await
                .map(|audio| audio.bytes)
                .map_err(provider_error)
        })
    }
}

fn provider_error(error: ElevenMusicError) -> MusicError {
    let cause = match error {
        ElevenMusicError::InvalidRequest => MusicProviderCause::Rejected,
        ElevenMusicError::Unauthorized => MusicProviderCause::Denied,
        ElevenMusicError::Provider(429) => MusicProviderCause::Capacity,
        ElevenMusicError::Provider(_) => MusicProviderCause::Supplier,
        ElevenMusicError::Transport(NativeHttpCause::Deadline) => MusicProviderCause::Deadline,
        ElevenMusicError::Transport(_) => MusicProviderCause::Transport,
        ElevenMusicError::Output => MusicProviderCause::Unusable,
    };
    MusicError::Provider { cause }
}
