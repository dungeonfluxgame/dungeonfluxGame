//! A hook through which a caller meters the billed calls of a client that builds and sends its
//! own request: the choice (Jev), plate-judge and head-locator clients.
//!
//! The client calls [`MeterSlot::sent`] right before it sends, and settles the returned
//! [`CallTicket`] with what happened: [`CallTicket::answered`] when the supplier answered (the
//! call was billed), [`CallTicket::refused`] when it refused before doing billable work (a rate
//! limit, a rejected request). A ticket dropped unsettled is a call of unknown outcome: it may
//! have been billed. The caller decides what a ticket records; this crate knows no prices and no
//! ledger, and reports sizes and the supplier's own token counts only (units, never dollars).

use std::sync::Arc;

use serde_json::Value;

use crate::NativeHttpRefusal;

/// Which client sent the call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeteredClient {
    /// The Jev choice client.
    Choice,
    /// The vision judge over a few labelled pictures.
    PlateJudge,
    /// The vision model that finds heads in a plate.
    HeadLocator,
}

/// A call about to be sent, as its request says.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SentCall {
    pub client: MeteredClient,
    pub model: &'static str,
    /// Bytes of everything in the request except the pictures (instruction, schema, state).
    pub text_bytes: usize,
    pub pictures: usize,
    /// The most output tokens the request lets the model write; zero when it sets no limit.
    pub maximum_output_tokens: u64,
}

/// What the supplier reported. A field it did not report is `None`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AnsweredUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// One call in flight. See the module note for what an unsettled drop means.
pub trait CallTicket: Send {
    /// The supplier answered: the call was billed.
    fn answered(self: Box<Self>, usage: AnsweredUsage);
    /// The supplier answered and billed the call, but its answer was of no use (it did not parse,
    /// or named nothing that was asked): the cost stands and the output does not.
    fn unusable(self: Box<Self>, usage: AnsweredUsage);
    /// The supplier refused before doing billable work: nothing was billed.
    fn refused(self: Box<Self>);
}

/// Starts the metering of one call.
pub trait CallMeter: Send + Sync {
    fn sent(&self, call: SentCall) -> Box<dyn CallTicket>;
}

/// The meter a client was given, if any. A client without one meters nothing.
#[derive(Clone)]
pub struct MeterSlot(Option<Arc<dyn CallMeter>>);

impl MeterSlot {
    pub fn new(meter: Arc<dyn CallMeter>) -> Self {
        Self(Some(meter))
    }

    pub fn sent(&self, call: SentCall) -> Option<Box<dyn CallTicket>> {
        self.0.as_ref().map(|meter| meter.sent(call))
    }
}

impl std::fmt::Debug for MeterSlot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MeterSlot")
            .field("metered", &self.0.is_some())
            .finish()
    }
}

/// Whether a call answered with this HTTP status may have been billed: a rejected request (a
/// client error, a rate limit) was not, and any other status may have been, even a server error.
pub const fn status_may_be_billed(status: u16) -> bool {
    !matches!(status, 400 | 401 | 403 | 404 | 413 | 422 | 429)
}

/// Whether a call that failed in transport with this refusal may have been billed: a request
/// refused before it was sent was not; a deadline, a dropped connection or a cancel may have been.
pub const fn refusal_may_be_billed(refusal: NativeHttpRefusal) -> bool {
    !matches!(
        refusal,
        NativeHttpRefusal::InvalidLimits
            | NativeHttpRefusal::InvalidEndpoint
            | NativeHttpRefusal::InvalidCredential
            | NativeHttpRefusal::RequestTooLarge
    )
}

/// Settles `ticket`, if any, for an answer that was not a success: a call that cannot have been
/// billed is refused, any other is left unsettled (of unknown outcome).
pub fn settle_status(ticket: Option<Box<dyn CallTicket>>, status: u16) {
    if let Some(ticket) = ticket.filter(|_| !status_may_be_billed(status)) {
        ticket.refused();
    }
}

/// Settles `ticket`, if any, for a transport failure: a request refused before it was sent is
/// refused, any other failure is left unsettled (of unknown outcome).
pub fn settle_refusal(ticket: Option<Box<dyn CallTicket>>, refusal: NativeHttpRefusal) {
    if let Some(ticket) = ticket.filter(|_| !refusal_may_be_billed(refusal)) {
        ticket.refused();
    }
}

/// The token counts of a chat-completions answer (`usage.prompt_tokens`, `usage.completion_tokens`).
pub(crate) fn chat_usage(response: &Value) -> AnsweredUsage {
    let count = |name: &str| response.get("usage")?.get(name)?.as_u64();
    AnsweredUsage {
        input_tokens: count("prompt_tokens"),
        output_tokens: count("completion_tokens"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct Record(Mutex<Vec<&'static str>>);

    struct Ticket(Arc<Record>);

    impl CallTicket for Ticket {
        fn answered(self: Box<Self>, _usage: AnsweredUsage) {
            self.0.0.lock().unwrap().push("answered");
        }

        fn unusable(self: Box<Self>, _usage: AnsweredUsage) {
            self.0.0.lock().unwrap().push("unusable");
        }

        fn refused(self: Box<Self>) {
            self.0.0.lock().unwrap().push("refused");
        }
    }

    impl Drop for Ticket {
        fn drop(&mut self) {
            self.0.0.lock().unwrap().push("dropped");
        }
    }

    struct Meter(Arc<Record>);

    impl CallMeter for Meter {
        fn sent(&self, _call: SentCall) -> Box<dyn CallTicket> {
            Box::new(Ticket(Arc::clone(&self.0)))
        }
    }

    fn call() -> SentCall {
        SentCall {
            client: MeteredClient::Choice,
            model: "jev-latest",
            text_bytes: 10,
            pictures: 0,
            maximum_output_tokens: 0,
        }
    }

    #[test]
    fn a_chat_answer_reports_its_prompt_and_completion_tokens() {
        assert_eq!(
            chat_usage(
                &serde_json::json!({"usage": {"prompt_tokens": 90, "completion_tokens": 4}})
            ),
            AnsweredUsage {
                input_tokens: Some(90),
                output_tokens: Some(4)
            }
        );
        assert_eq!(
            chat_usage(&serde_json::json!({"choices": []})),
            AnsweredUsage::default()
        );
    }

    #[test]
    fn a_rejected_status_or_unsent_request_is_refused_and_any_other_outcome_is_left_unknown() {
        for (status, refused) in [
            (400, true),
            (401, true),
            (429, true),
            (500, false),
            (503, false),
            (408, false),
        ] {
            let record = Arc::new(Record::default());
            let slot = MeterSlot::new(Arc::new(Meter(Arc::clone(&record))));
            settle_status(slot.sent(call()), status);
            let seen = record.0.lock().unwrap().clone();
            assert_eq!(
                seen.contains(&"refused"),
                refused,
                "status {status} gave {seen:?}"
            );
            assert!(!seen.contains(&"answered"));
        }
        for (refusal, refused) in [
            (NativeHttpRefusal::InvalidEndpoint, true),
            (NativeHttpRefusal::RequestTooLarge, true),
            (NativeHttpRefusal::Deadline, false),
            (NativeHttpRefusal::Unknown, false),
            (NativeHttpRefusal::Cancelled, false),
        ] {
            let record = Arc::new(Record::default());
            let slot = MeterSlot::new(Arc::new(Meter(Arc::clone(&record))));
            settle_refusal(slot.sent(call()), refusal);
            assert_eq!(
                record.0.lock().unwrap().contains(&"refused"),
                refused,
                "{refusal:?}"
            );
        }
    }
}
