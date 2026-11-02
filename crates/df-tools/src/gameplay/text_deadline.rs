//! The request deadline, enforced around a backend. `df-ai` is platform-neutral and has no timer,
//! so the composition layer bounds every backend call by the request's own deadline. A backend that
//! also enforces its deadline keeps doing so; this wrapper is the guarantee that a backend which
//! does not (or hangs) cannot hold a request open past it.

use std::future::Future;

use df_ai::text::{TextBackend, TextOutcome, TextReceipt, TextRequest};

/// Bounds `inner` by each request's deadline.
pub(super) struct DeadlineBackend<B> {
    inner: B,
}

impl<B> DeadlineBackend<B> {
    pub(super) fn new(inner: B) -> Self {
        Self { inner }
    }
}

impl<B: TextBackend> TextBackend for DeadlineBackend<B> {
    fn complete(&self, request: &TextRequest) -> impl Future<Output = TextReceipt> + Send {
        async move {
            match tokio::time::timeout(request.deadline, self.inner.complete(request)).await {
                Ok(receipt) => receipt,
                // The request left before the deadline and no answer arrived. Whether the supplier
                // received it is not known, so no usage is claimed and the attempt stays paid-for.
                Err(_) => TextOutcome::Deadline.into(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use df_ai::{
        text::{TextProfile, TextPurpose, TextUsage},
        text_fixture::ScriptedBackend,
    };
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    /// A backend that never answers, as a hung supplier would.
    struct Hangs;

    impl TextBackend for Hangs {
        fn complete(&self, _: &TextRequest) -> impl Future<Output = TextReceipt> + Send {
            std::future::pending()
        }
    }

    fn request(deadline: Duration) -> TextRequest {
        TextRequest {
            purpose: TextPurpose::Storyteller,
            profile: TextProfile::new("deadline-test").unwrap(),
            instructions: "Narrate one line.".to_owned(),
            facts: serde_json::json!({}),
            output_name: "narration".to_owned(),
            output_schema: serde_json::json!({"type": "object"}),
            deadline,
        }
    }

    fn block_on<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn a_backend_that_never_answers_is_cut_off_at_the_request_deadline() {
        let backend = DeadlineBackend::new(Hangs);
        let receipt = block_on(backend.complete(&request(Duration::from_millis(5))));
        assert_eq!(receipt.outcome, TextOutcome::Deadline);
        assert_eq!(receipt.usage, None);
    }

    #[test]
    fn an_answer_inside_the_deadline_passes_through_with_its_usage() {
        let usage = TextUsage {
            total_tokens: 12,
            input_tokens: None,
            output_tokens: Some(4),
            cached_input_tokens: None,
            cache_write_tokens: None,
            reasoning_output_tokens: None,
        };
        let answered = TextReceipt {
            outcome: TextOutcome::Answered(serde_json::json!({"line": "ok"})),
            usage: Some(usage),
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let backend = DeadlineBackend::new(ScriptedBackend::new([answered.clone()], calls.clone()));
        let receipt = block_on(backend.complete(&request(Duration::from_secs(30))));
        assert_eq!(receipt, answered);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
