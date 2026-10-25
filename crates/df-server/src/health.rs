//! The five separate health signals of plan 16.3: process liveness, readiness, database,
//! provider availability and owner progress. They are reported independently, so a failing
//! database never hides a live process and a stalled owner is visible on its own. A signal that
//! does not apply to a role says so rather than reporting success.

use std::sync::Mutex;

use crate::metrics::{GaugeVec, MetricsError, MetricsRegistry};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Signal {
    /// The process and its event loop answer.
    Live,
    /// Dependencies, schema range, lease at my fence, failover check since my last reconnect,
    /// and not draining.
    Ready,
    Database,
    /// Circuit state per provider; the dispatcher's.
    Providers,
    /// Owner progress age, longest reduce in progress and inbox depth; the runtime's.
    Progress,
}

impl Signal {
    pub const ALL: [Self; 5] = [
        Self::Live,
        Self::Ready,
        Self::Database,
        Self::Providers,
        Self::Progress,
    ];

    /// The path segment under `/healthz/`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Ready => "ready",
            Self::Database => "database",
            Self::Providers => "providers",
            Self::Progress => "progress",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|signal| signal.name() == name)
    }

    const fn index(self) -> usize {
        match self {
            Self::Live => 0,
            Self::Ready => 1,
            Self::Database => 2,
            Self::Providers => 3,
            Self::Progress => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalState {
    Healthy,
    /// A closed reason word, never an error text or an id.
    Unhealthy(&'static str),
    NotApplicable,
}

/// The current state of every signal of one process.
pub struct HealthBoard {
    states: Mutex<[SignalState; 5]>,
    gauge: GaugeVec,
}

impl HealthBoard {
    /// A starting process: alive, not yet ready, database unchecked; providers and progress
    /// do not apply until the role says they do.
    pub fn starting(registry: &MetricsRegistry) -> Result<Self, MetricsError> {
        let gauge = registry.gauge_vec(
            "df_health_signal",
            "Health signal state: 1 healthy, 0 unhealthy; absent when not applicable.",
            &["signal"],
        )?;
        let board = Self {
            states: Mutex::new([
                SignalState::Healthy,
                SignalState::Unhealthy("starting"),
                SignalState::Unhealthy("not_checked"),
                SignalState::NotApplicable,
                SignalState::NotApplicable,
            ]),
            gauge,
        };
        for signal in Signal::ALL {
            board.publish(signal, board.state(signal));
        }
        Ok(board)
    }

    pub fn state(&self, signal: Signal) -> SignalState {
        self.states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())[signal.index()]
    }

    pub fn set(&self, signal: Signal, state: SignalState) {
        self.states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())[signal.index()] = state;
        self.publish(signal, state);
    }

    fn publish(&self, signal: Signal, state: SignalState) {
        let value = match state {
            SignalState::Healthy => 1.0,
            SignalState::Unhealthy(_) => 0.0,
            SignalState::NotApplicable => return,
        };
        if let Some(gauge) = self.gauge.with(&[signal.name()]) {
            gauge.set(value);
        }
    }
}
