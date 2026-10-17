//! Text embeddings for the story memory, and what they cost.
//!
//! [`Embedder`] is the one seam to a provider: the game uses [`OpenAiEmbedder`]
//! (`text-embedding-3-small`, 1536 dimensions, through `df_providers`' shared native transport),
//! and tests use a deterministic fake. Every call is charged to the generation hub's spend
//! ledger as [`Unit::EmbeddingToken`] by [`embed_charged`]: it reserves an estimate, makes the
//! call, then commits the supplier-reported tokens (or the estimate when none were reported).
//! A call that was refused locally, or never left this machine, refunds its reservation.

use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use df_providers::{EMBEDDING_MAXIMUM_INPUT_CHARACTERS, EmbeddingConfig, NativeEmbeddingProvider};

use crate::gameplay::resources::{SpendLedger, Unit};

pub(in crate::gameplay) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What a provider answered.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::gameplay) struct Embedded {
    /// One vector per text, in order.
    pub(in crate::gameplay) vectors: Vec<Vec<f32>>,
    /// The supplier-reported input tokens, when it reported them.
    pub(in crate::gameplay) tokens: Option<u64>,
}

/// Why no vectors came back.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum EmbedFailure {
    /// Refused before it left this machine: nothing was sent or billed.
    NotSent(&'static str),
    /// The call was sent and failed or answered badly; it may have been billed.
    Failed(&'static str),
}

impl EmbedFailure {
    pub(in crate::gameplay) fn label(self) -> &'static str {
        match self {
            Self::NotSent(label) | Self::Failed(label) => label,
        }
    }
}

/// A source of embeddings. Implementations never log the texts.
pub(in crate::gameplay) trait Embedder: Send + Sync {
    /// The model identifier stored beside every vector it makes.
    fn model(&self) -> &str;

    /// The vectors of `texts`, at most [`BATCH`] of them.
    fn embed<'a>(&'a self, texts: &'a [String]) -> BoxFuture<'a, Result<Embedded, EmbedFailure>>;
}

/// The most texts of one provider call.
pub(in crate::gameplay) const BATCH: usize = 32;
/// One call's whole deadline.
const CALL_DEADLINE: Duration = Duration::from_secs(20);

/// OpenAI's embeddings endpoint.
pub(in crate::gameplay) struct OpenAiEmbedder {
    provider: NativeEmbeddingProvider,
    model: String,
}

impl OpenAiEmbedder {
    pub(in crate::gameplay) fn new(credential: &str, model: &str) -> Result<Self, &'static str> {
        let provider =
            NativeEmbeddingProvider::new(EmbeddingConfig::live(), credential, model, CALL_DEADLINE)
                .map_err(|_| "embedding provider refused its configuration")?;
        Ok(Self {
            provider,
            model: model.to_owned(),
        })
    }
}

impl Embedder for OpenAiEmbedder {
    fn model(&self) -> &str {
        &self.model
    }

    fn embed<'a>(&'a self, texts: &'a [String]) -> BoxFuture<'a, Result<Embedded, EmbedFailure>> {
        Box::pin(async move {
            // The provider refuses an over-long text; cut it here so one long line cannot
            // starve a whole batch.
            let cut: Vec<String> = texts
                .iter()
                .map(|text| {
                    text.chars()
                        .take(EMBEDDING_MAXIMUM_INPUT_CHARACTERS)
                        .collect()
                })
                .collect();
            match self.provider.embed(&cut, CALL_DEADLINE).await {
                Ok(batch) => Ok(Embedded {
                    vectors: batch.vectors,
                    tokens: batch.prompt_tokens,
                }),
                Err(error) if !error.was_sent() => Err(EmbedFailure::NotSent(error.label())),
                Err(error) => Err(EmbedFailure::Failed(error.label())),
            }
        })
    }
}

/// A conservative token estimate for budgeting before the call (the supplier's count settles it).
pub(in crate::gameplay) fn estimate_tokens(texts: &[String]) -> u64 {
    texts
        .iter()
        .map(|text| (text.chars().count() as u64).div_ceil(3).max(1) + 4)
        .sum()
}

/// One embedding call under the hub's spend contract: reserve, call, commit or refund.
pub(in crate::gameplay) async fn embed_charged(
    embedder: &dyn Embedder,
    ledger: &Arc<SpendLedger>,
    texts: &[String],
) -> Result<Embedded, EmbedFailure> {
    if texts.is_empty() {
        return Err(EmbedFailure::NotSent("no_input"));
    }
    let estimate = estimate_tokens(texts);
    // The ledger never refuses (owner decision: no spend caps); it records and audits the spend.
    let reservation = ledger.reserve(Unit::EmbeddingToken, estimate);
    match embedder.embed(texts).await {
        Ok(answer) if answer.vectors.len() == texts.len() => {
            reservation.commit(answer.tokens.unwrap_or(estimate));
            Ok(answer)
        }
        Ok(_) => {
            // The call was made and billed; its answer is unusable.
            reservation.commit(estimate);
            Err(EmbedFailure::Failed("bad_output"))
        }
        Err(EmbedFailure::NotSent(label)) => {
            reservation.refund();
            Err(EmbedFailure::NotSent(label))
        }
        Err(other) => {
            reservation.commit(estimate);
            Err(other)
        }
    }
}

/// A deterministic fake for tests: a hashed bag of words, normalised, so texts that share words
/// are near each other and the same text always gets the same vector. Never calls anything.
#[cfg(test)]
pub(in crate::gameplay) struct FakeEmbedder {
    pub(in crate::gameplay) model: String,
    /// Calls made, to prove what was and was not embedded.
    pub(in crate::gameplay) calls: std::sync::atomic::AtomicUsize,
    pub(in crate::gameplay) texts_seen: std::sync::atomic::AtomicUsize,
    /// When set, every call fails as a sent-and-failed call.
    pub(in crate::gameplay) failing: std::sync::atomic::AtomicBool,
}

#[cfg(test)]
impl FakeEmbedder {
    pub(in crate::gameplay) fn new(model: &str) -> Arc<Self> {
        Arc::new(Self {
            model: model.to_owned(),
            calls: std::sync::atomic::AtomicUsize::new(0),
            texts_seen: std::sync::atomic::AtomicUsize::new(0),
            failing: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub(in crate::gameplay) fn vector_of(text: &str) -> Vec<f32> {
        use sha2::{Digest, Sha256};
        let mut vector = vec![0.0_f32; df_persistence::story_memory::EMBEDDING_DIMENSIONS];
        for word in text.split(|c: char| !c.is_alphanumeric()) {
            if word.chars().count() < 2 {
                continue;
            }
            let digest = Sha256::digest(word.to_lowercase().as_bytes());
            let slot = usize::from(u16::from_le_bytes([digest[0], digest[1]])) % vector.len();
            vector[slot] += if digest[2] & 1 == 0 { 1.0 } else { -1.0 };
        }
        let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            vector.iter_mut().for_each(|v| *v /= norm);
        } else {
            vector[0] = 1.0;
        }
        vector
    }
}

#[cfg(test)]
impl Embedder for FakeEmbedder {
    fn model(&self) -> &str {
        &self.model
    }

    fn embed<'a>(&'a self, texts: &'a [String]) -> BoxFuture<'a, Result<Embedded, EmbedFailure>> {
        use std::sync::atomic::Ordering;
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.texts_seen.fetch_add(texts.len(), Ordering::SeqCst);
            if self.failing.load(Ordering::SeqCst) {
                return Err(EmbedFailure::Failed("provider_error"));
            }
            Ok(Embedded {
                vectors: texts.iter().map(|text| Self::vector_of(text)).collect(),
                tokens: Some(
                    texts
                        .iter()
                        .map(|t| t.split_whitespace().count() as u64)
                        .sum(),
                ),
            })
        })
    }
}
