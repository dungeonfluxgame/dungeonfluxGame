//! What the broker records: one typed event for every change of a request, in one place.
//!
//! The broker is the one choke point for resource requests (coding style section 8, pattern 3), so
//! every request, claim, retry, preemption, cancellation, rejection and charge is a `BrokerEvent`
//! recorded through `BrokerAudit`, and each renders as one `AREA verb key=value` line in the two
//! areas pattern 7 names: `RESOURCE` for the life of a request and `SPEND` for a charge. An event
//! holds ids, closed keys and counts only: never a prompt, a result or any player text.
//!
//! These are host events because `df_table::event::TableEvent` is a closed enum of the pure crate.
//! A failure is recorded once, here; the pipeline records the `BoundaryFailed` of the port call
//! that sees it, which is the boundary that handles it.

use std::fmt::{self, Write};

use df_table::event::FailureCause;
use df_table::identity::OperationId;
use df_table::ports::{PurposeName, ResourceHandle};

use super::purpose::{Price, QueueClass};
use super::request::{AttemptId, Epoch, RequestKey, WorkerId};

/// Why a result or a request was found stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleReason {
    /// A request for a scene epoch that had already passed was refused.
    RefusedAtRequest,
    /// The scene epoch passed while the request waited or ran.
    EpochAdvanced,
    /// A provider result arrived for an attempt that was abandoned.
    LateResult,
}

impl StaleReason {
    pub fn key(self) -> &'static str {
        match self {
            Self::RefusedAtRequest => "refused_at_request",
            Self::EpochAdvanced => "epoch_advanced",
            Self::LateResult => "late_result",
        }
    }
}

/// Why a claimed attempt was given up before it finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbandonReason {
    Preempted,
    Cancelled,
    EpochAdvanced,
    DeadlineExceeded,
    LeaseExpired,
}

impl AbandonReason {
    pub fn key(self) -> &'static str {
        match self {
            Self::Preempted => "preempted",
            Self::Cancelled => "cancelled",
            Self::EpochAdvanced => "epoch_advanced",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::LeaseExpired => "lease_expired",
        }
    }
}

/// Why a request was refused before it started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalKind {
    UnknownPurpose,
    QueueFull,
    Closed,
}

impl RefusalKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::UnknownPurpose => "unknown_purpose",
            Self::QueueFull => "queue_full",
            Self::Closed => "closed",
        }
    }
}

/// One change of the broker, with the fields a reader joins on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokerEvent {
    Requested {
        handle: ResourceHandle,
        purpose: PurposeName,
        class: QueueClass,
        epoch: Epoch,
        key: RequestKey,
        operation: OperationId,
    },
    /// A second request for a key already in flight joined it.
    Joined {
        handle: ResourceHandle,
        purpose: PurposeName,
        holders: u32,
    },
    Refused {
        purpose: PurposeName,
        kind: RefusalKind,
    },
    Claimed {
        handle: ResourceHandle,
        purpose: PurposeName,
        attempt: AttemptId,
        worker: WorkerId,
        number: u32,
    },
    /// The provider call for an attempt was sent, so its price is spent and never refunded.
    Spent {
        handle: ResourceHandle,
        purpose: PurposeName,
        attempt: AttemptId,
        price: Price,
    },
    Retrying {
        handle: ResourceHandle,
        purpose: PurposeName,
        retry: u8,
        backoff_ms: u64,
        cause: FailureCause,
    },
    Ready {
        handle: ResourceHandle,
        purpose: PurposeName,
        ms: u64,
    },
    Failed {
        handle: ResourceHandle,
        purpose: PurposeName,
        cause: FailureCause,
        ms: u64,
        /// The call may have been sent and nothing says how it ended, so its spend is never
        /// released (H.8).
        unknown_dispatch: bool,
    },
    Cancelled {
        handle: ResourceHandle,
        purpose: PurposeName,
    },
    Preempted {
        handle: ResourceHandle,
        purpose: PurposeName,
        by: ResourceHandle,
    },
    Dropped {
        handle: ResourceHandle,
        purpose: PurposeName,
        class: QueueClass,
    },
    Stale {
        handle: Option<ResourceHandle>,
        purpose: PurposeName,
        epoch: Epoch,
        current: Epoch,
        reason: StaleReason,
    },
    Abandoned {
        handle: ResourceHandle,
        attempt: AttemptId,
        reason: AbandonReason,
    },
    LeaseExpired {
        handle: ResourceHandle,
        attempt: AttemptId,
        sent: bool,
        requeued: bool,
    },
    EpochAdvanced {
        from: Epoch,
        to: Epoch,
        superseded: usize,
    },
}

impl BrokerEvent {
    pub fn area(&self) -> &'static str {
        match self {
            Self::Spent { .. } => "SPEND",
            _ => "RESOURCE",
        }
    }

    /// The whole line, `AREA verb key=value ...`.
    pub fn render_audit_line(&self) -> String {
        let mut line = String::with_capacity(160);
        // Writing into a String cannot fail, so the result carries no information.
        let _ = self.write_audit_line(&mut line);
        line
    }

    fn write_audit_line(&self, out: &mut String) -> fmt::Result {
        write!(out, "{} ", self.area())?;
        match self {
            Self::Requested {
                handle,
                purpose,
                class,
                epoch,
                key,
                operation,
            } => write!(
                out,
                "request purpose={purpose} class={} handle={} epoch={} key={:016x} operation_id={operation}",
                class.key(),
                handle.get(),
                epoch.get(),
                key.get()
            ),
            Self::Joined {
                handle,
                purpose,
                holders,
            } => write!(
                out,
                "joined purpose={purpose} handle={} holders={holders}",
                handle.get()
            ),
            Self::Refused { purpose, kind } => {
                write!(out, "refused purpose={purpose} cause={}", kind.key())
            }
            Self::Claimed {
                handle,
                purpose,
                attempt,
                worker,
                number,
            } => write!(
                out,
                "claimed purpose={purpose} handle={} attempt={} worker={} number={number}",
                handle.get(),
                attempt.get(),
                worker.get()
            ),
            Self::Spent {
                handle,
                purpose,
                attempt,
                price,
            } => {
                write!(
                    out,
                    "commit purpose={purpose} handle={} attempt={} ",
                    handle.get(),
                    attempt.get()
                )?;
                match price {
                    Price::Priced { micro_dollars } => write!(out, "micro_dollars={micro_dollars}"),
                    Price::Unpriced => out.write_str("price=unpriced"),
                }
            }
            Self::Retrying {
                handle,
                purpose,
                retry,
                backoff_ms,
                cause,
            } => write!(
                out,
                "retry purpose={purpose} handle={} retry={retry} backoff_ms={backoff_ms} cause={}",
                handle.get(),
                cause.key()
            ),
            Self::Ready {
                handle,
                purpose,
                ms,
            } => write!(
                out,
                "ready purpose={purpose} handle={} ms={ms}",
                handle.get()
            ),
            Self::Failed {
                handle,
                purpose,
                cause,
                ms,
                unknown_dispatch,
            } => {
                write!(
                    out,
                    "failed purpose={purpose} handle={} cause={} ms={ms}",
                    handle.get(),
                    cause.key()
                )?;
                if *unknown_dispatch {
                    out.write_str(" unknown_dispatch=true")?;
                }
                Ok(())
            }
            Self::Cancelled { handle, purpose } => {
                write!(out, "cancelled purpose={purpose} handle={}", handle.get())
            }
            Self::Preempted {
                handle,
                purpose,
                by,
            } => write!(
                out,
                "preempted purpose={purpose} handle={} by={}",
                handle.get(),
                by.get()
            ),
            Self::Dropped {
                handle,
                purpose,
                class,
            } => write!(
                out,
                "dropped purpose={purpose} handle={} class={}",
                handle.get(),
                class.key()
            ),
            Self::Stale {
                handle,
                purpose,
                epoch,
                current,
                reason,
            } => {
                write!(out, "stale purpose={purpose} ")?;
                if let Some(handle) = handle {
                    write!(out, "handle={} ", handle.get())?;
                }
                write!(
                    out,
                    "epoch={} current={} reason={}",
                    epoch.get(),
                    current.get(),
                    reason.key()
                )
            }
            Self::Abandoned {
                handle,
                attempt,
                reason,
            } => write!(
                out,
                "abandoned handle={} attempt={} reason={}",
                handle.get(),
                attempt.get(),
                reason.key()
            ),
            Self::LeaseExpired {
                handle,
                attempt,
                sent,
                requeued,
            } => write!(
                out,
                "lease_expired handle={} attempt={} sent={sent} requeued={requeued}",
                handle.get(),
                attempt.get()
            ),
            Self::EpochAdvanced {
                from,
                to,
                superseded,
            } => write!(
                out,
                "epoch_advanced from={} to={} superseded={superseded}",
                from.get(),
                to.get()
            ),
        }
    }
}

/// Receives every broker event. An implementation must not block and must not fail the caller.
pub trait BrokerAudit: Send + Sync {
    fn record(&self, event: &BrokerEvent);
}
