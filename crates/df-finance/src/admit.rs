//! Paid admission (planning/finance-ledger-research.md 11.2): one transaction that takes the
//! admission locks, decides, posts the reservation entry, and only then asks the fence and the media
//! catalog, so a refusal there rolls the entry back (lock order finance, then O1, then media).

use std::time::Duration;

use df_commerce::admission::{
    AdmissionDecision, AdmissionRequest, Metering, Route, WorkClass, decide_admission,
};
use df_commerce::finance_ids::{
    AttemptId, EffectId, EntryId, EventId, OperationKey, PayerId, PriceVersionId, QuoteId,
    ReservationId, SessionKey,
};
use df_commerce::views::customer::{AdmissionView, MoneyView, price_version_label};
use serde_json::json;
use tokio_postgres::Transaction;

use crate::ports::{
    FencePort, FenceRequest, FenceVerdict, MediaPort, MediaRequest, MediaVerdict, MemberPort,
};
use crate::service::{FinanceError, FinanceService};
use crate::wire::parse_admission_snapshot;

/// An admission as the runtime's finance endpoint receives it.
#[derive(Clone, Debug)]
pub struct AdmitCommand {
    pub operation: OperationKey,
    pub attempt: AttemptId,
    pub effect: EffectId,
    pub fence: FenceRequest,
    pub session: SessionKey,
    pub route: Route,
    pub work_class: WorkClass,
    pub maximum_units: Vec<(String, u64)>,
    pub estimate_units: Vec<(String, u64)>,
    pub quote: Option<QuoteId>,
    pub request_digest: [u8; 32],
    pub media: Option<MediaRequest>,
}

/// What an admission answered. It holds what the table is charged for the hold and no supplier
/// figure: the supplier side stays in the attempt row, for operators.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdmissionOutcome {
    Admitted {
        reservation: ReservationId,
        attempt: AttemptId,
        retail_maximum_micro_dollars: u64,
        price_version: PriceVersionId,
    },
    /// The same operation was admitted before: the stored outcome (SAVE-02).
    AlreadyAdmitted {
        reservation: ReservationId,
        attempt: AttemptId,
        retail_maximum_micro_dollars: u64,
        price_version: PriceVersionId,
    },
}

impl AdmissionOutcome {
    /// The customer-safe form of the outcome.
    pub fn view(&self) -> AdmissionView {
        let (Self::Admitted {
            reservation,
            attempt,
            retail_maximum_micro_dollars,
            price_version,
        }
        | Self::AlreadyAdmitted {
            reservation,
            attempt,
            retail_maximum_micro_dollars,
            price_version,
        }) = self;
        AdmissionView {
            reservation: reservation.hex(),
            attempt: attempt.hex(),
            held: MoneyView::new(*retail_maximum_micro_dollars),
            price_version_label: price_version_label(*price_version),
        }
    }
}

/// The route of a metered, priced provider call, for callers that build admissions.
pub fn priced_route(provider: &str, model: &str, name: &str, categories: &[&str]) -> Route {
    Route {
        provider: provider.to_owned(),
        model: model.to_owned(),
        name: name.to_owned(),
        declared_categories: categories
            .iter()
            .map(|category| (*category).to_owned())
            .collect(),
        metering: Metering::Priced,
    }
}

impl<F: FencePort, M: MediaPort, S: MemberPort> FinanceService<F, M, S> {
    /// 11.2: one paid admission. Identical requests serialize on the session lock, so the second sees
    /// the first one's receipt and answers `AlreadyAdmitted`; a refused one writes nothing.
    pub async fn admit(
        &mut self,
        command: &AdmitCommand,
    ) -> Result<AdmissionOutcome, FinanceError> {
        let now_ms = self.clock.now_ms();
        let event_id = EventId::from_array(self.id());
        let entry_id = EntryId::from_array(self.id());
        let reservation = ReservationId::from_array(self.id());
        let reserved_expiry_ms = self.config.reserved_expiry_ms;
        let deadline = Duration::from_millis(self.config.port_deadline_ms);
        let transaction = Self::start(&mut self.client).await?;
        let mut args = json!({
            "session": command.session.hex(),
            "principal": command.fence.campaign.hex(),
            "operation": command.operation.hex(),
            "now_ms": now_ms,
        });
        if let Some(quote) = command.quote {
            args["quote"] = json!(quote.hex());
        }
        let document = Self::lock(&transaction, "admission", args).await?;
        let snapshot = parse_admission_snapshot(&document)?;
        let payers: Vec<PayerId> = snapshot
            .allocations
            .iter()
            .map(|(allocation, _)| allocation.payer)
            .collect();
        let authorized = bounded(
            deadline,
            self.members
                .authorized_payers(command.fence.campaign, &payers),
        )
        .await?;
        let request = AdmissionRequest {
            event_id,
            entry_id,
            reservation,
            attempt: command.attempt,
            effect: command.effect,
            operation: command.operation,
            tenant: command.fence.tenant,
            campaign: command.fence.campaign,
            session: command.session,
            route: command.route.clone(),
            work_class: command.work_class,
            maximum_units: command.maximum_units.clone(),
            estimate_units: command.estimate_units.clone(),
            quote: command.quote,
            request_digest: command.request_digest,
            authorized_payers: authorized,
            now_ms,
            reserved_expiry_ms,
        };
        let admitted =
            match decide_admission(&snapshot, &request).map_err(FinanceError::Admission)? {
                AdmissionDecision::AlreadyAdmitted(receipt) => {
                    transaction.rollback().await?;
                    return Ok(AdmissionOutcome::AlreadyAdmitted {
                        reservation: receipt.reservation,
                        attempt: receipt.attempt,
                        retail_maximum_micro_dollars: receipt.retail_maximum_micro_dollars,
                        price_version: receipt.price_version,
                    });
                }
                AdmissionDecision::Admitted(admitted) => admitted,
            };
        Self::commit_plan(&transaction, &admitted.plan).await?;
        ask_fence_and_media(&self.fence, &self.media, &transaction, command, deadline).await?;
        transaction.commit().await?;
        let receipt = admitted.receipt;
        Ok(AdmissionOutcome::Admitted {
            reservation: receipt.reservation,
            attempt: receipt.attempt,
            retail_maximum_micro_dollars: receipt.retail_maximum_micro_dollars,
            price_version: receipt.price_version,
        })
    }
}

/// Fence, then media: after the finance rows, in D25 order. A refusal returns the error and the
/// caller's transaction rolls back when it is dropped.
async fn ask_fence_and_media<F: FencePort, M: MediaPort>(
    fence: &F,
    media: &M,
    transaction: &Transaction<'_>,
    command: &AdmitCommand,
    deadline: Duration,
) -> Result<(), FinanceError> {
    let verdict = bounded(deadline, fence.fence_current(transaction, &command.fence)).await?;
    if verdict != FenceVerdict::Current {
        return Err(FinanceError::Fence(verdict));
    }
    if let Some(request) = &command.media {
        let verdict = bounded(deadline, media.admit_generation(transaction, request)).await?;
        if verdict != MediaVerdict::Admitted {
            return Err(FinanceError::Media(verdict));
        }
    }
    Ok(())
}

/// A cross-owner call under a deadline; a port that fails or does not answer in time leaves the
/// admission unwritten.
async fn bounded<T>(
    deadline: Duration,
    call: impl std::future::Future<Output = Result<T, crate::ports::PortUnavailable>>,
) -> Result<T, FinanceError> {
    match tokio::time::timeout(deadline, call).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(_)) | Err(_) => Err(FinanceError::PortUnavailable),
    }
}
