//! The meter handed to the choice (Jev), plate-judge and head-locator clients of `df_providers`.
//! Those clients build and send their own request, so they meter themselves: each call they
//! send becomes a [`BilledCall`] on the hub's ledger, settled with what the supplier answered.
//!
//! Each client requires its meter at construction, so the type cannot represent an unmetered
//! production client.

use std::sync::Arc;

use df_providers::{AnsweredUsage, CallMeter, CallTicket, MeteredClient, SentCall};

use super::{
    BilledCall, CallSpec, Category, Cause, IMAGE_INPUT_TOKENS_PER_REFERENCE, Provider, Usage,
};
use crate::gameplay::resources::SpendLedger;

/// Meters the calls of one client on `ledger`; `detail` names the user of the client in the
/// events (`approach`, `creator`, `staging`).
pub(in crate::gameplay) struct LedgerMeter {
    ledger: Arc<SpendLedger>,
    detail: &'static str,
}

impl LedgerMeter {
    pub(in crate::gameplay) fn shared(
        ledger: &Arc<SpendLedger>,
        detail: &'static str,
    ) -> Arc<dyn CallMeter> {
        Arc::new(Self {
            ledger: Arc::clone(ledger),
            detail,
        })
    }
}

/// The spec of a sent call: its maxima are the request's size at two bytes per token, every
/// picture at the cap, and the output the request allows.
pub(super) fn spec_of(call: &SentCall, detail: &str) -> CallSpec {
    let text_tokens = u64::try_from(call.text_bytes.div_ceil(2)).unwrap_or(u64::MAX);
    match call.client {
        MeteredClient::Choice => {
            CallSpec::new(Provider::TypeSafe, call.model, Cause::Judgement, detail)
                .with_maxima(vec![(Category::TextInput, text_tokens)])
        }
        MeteredClient::PlateJudge | MeteredClient::HeadLocator => {
            let cause = if call.client == MeteredClient::PlateJudge {
                Cause::PictureJudgement
            } else {
                Cause::HeadLocation
            };
            let pictures = u64::try_from(call.pictures).unwrap_or(u64::MAX);
            CallSpec::new(Provider::OpenAi, call.model, cause, detail).with_maxima(vec![
                (Category::TextInput, text_tokens),
                (
                    Category::ImageInput,
                    pictures.saturating_mul(IMAGE_INPUT_TOKENS_PER_REFERENCE),
                ),
                (Category::TextOutput, call.maximum_output_tokens),
            ])
        }
    }
}

impl CallMeter for LedgerMeter {
    fn sent(&self, call: SentCall) -> Box<dyn CallTicket> {
        let spec = spec_of(&call, self.detail);
        let maxima = spec.maxima.clone();
        Box::new(Ticket {
            client: call.client,
            maxima,
            billed: self.ledger.begin_call(spec, &[]),
        })
    }
}

#[cfg(test)]
pub(in crate::gameplay) fn test_meter() -> Arc<dyn CallMeter> {
    Arc::new(TestMeter)
}

#[cfg(test)]
struct TestMeter;

#[cfg(test)]
struct TestTicket;

#[cfg(test)]
impl CallMeter for TestMeter {
    fn sent(&self, _call: SentCall) -> Box<dyn CallTicket> {
        Box::new(TestTicket)
    }
}

#[cfg(test)]
impl CallTicket for TestTicket {
    fn answered(self: Box<Self>, _usage: AnsweredUsage) {}

    fn unusable(self: Box<Self>, _usage: AnsweredUsage) {}

    fn refused(self: Box<Self>) {}
}

struct Ticket {
    client: MeteredClient,
    maxima: Vec<(Category, u64)>,
    billed: BilledCall,
}

impl CallTicket for Ticket {
    fn answered(self: Box<Self>, usage: AnsweredUsage) {
        let Self {
            client,
            maxima,
            billed,
        } = *self;
        billed.settle(answered_usage(client, &maxima, usage));
    }

    fn unusable(self: Box<Self>, usage: AnsweredUsage) {
        let Self {
            client,
            maxima,
            billed,
        } = *self;
        billed.settle(answered_usage(client, &maxima, usage).unusable());
    }

    fn refused(self: Box<Self>) {
        self.billed.not_billed();
    }
}

/// What an answer used. A supplier that reported its input tokens is believed (the pictures of a
/// judgement are inside them), and its output tokens too; a field it left out is booked at the
/// call's maximum and the usage is flagged estimated; one that reported nothing is the maxima.
pub(super) fn answered_usage(
    client: MeteredClient,
    maxima: &[(Category, u64)],
    answered: AnsweredUsage,
) -> Usage {
    let maximum = |category: Category| {
        maxima
            .iter()
            .find_map(|(candidate, quantity)| (*candidate == category).then_some(*quantity))
            .unwrap_or(0)
    };
    let Some(input) = answered.input_tokens else {
        let estimate = match client {
            MeteredClient::Choice => vec![(Category::TextInput, maximum(Category::TextInput))],
            MeteredClient::PlateJudge | MeteredClient::HeadLocator => maxima.to_vec(),
        };
        return Usage::estimated(estimate);
    };
    match (client, answered.output_tokens) {
        (MeteredClient::Choice, _) => Usage::reported(vec![(Category::TextInput, input)]),
        (_, Some(output)) => Usage::reported(vec![
            (Category::TextInput, input),
            (Category::TextOutput, output),
        ]),
        (_, None) => Usage::estimated(vec![
            (Category::TextInput, input),
            (Category::TextOutput, maximum(Category::TextOutput)),
        ]),
    }
}
