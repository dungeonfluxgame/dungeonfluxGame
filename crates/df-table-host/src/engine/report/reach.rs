//! What the read verbs that reach into the owner answer with: `inspect`, `dump table`,
//! `dump <machine>`, `broker queues` with its limits, and the event level.
//!
//! An inspect report is the `df_table::inspect` report of one module, built by the pure crate for
//! one audience (it redacts while it builds) and carried here as its JSON, because the pure
//! crate's reports serialise and are not read back. Everything else is the host's own typed
//! value. Like every report of the engine routes, nothing here holds player text.

use serde::{Deserialize, Serialize};

use crate::broker::purpose::QueueClass;
use crate::broker::report::QueuesReport;
use crate::observe::EventLevel;

use super::super::request::InspectName;
use super::TableDump;

/// `inspect <report>`: one module's report, as the audience it was built for may see it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InspectReport {
    pub report: InspectName,
    /// `operator`, `host` or `seat:N`.
    pub audience: String,
    /// The owner's clock when the report was built.
    pub now_ms: u64,
    /// The revision of the committed table the report reads.
    pub revision: u64,
    /// The report as `df_table::inspect` writes it; a hidden value is `{"visibility":"redacted",
    /// "needs":..}` and was never read.
    pub value: serde_json::Value,
}

/// One machine's place on the table and its saved state, as `dump <machine>` shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MachineDump {
    pub machine: String,
    pub state: String,
    /// The scene the machine runs in, or `None` for the table's own machine.
    pub scene: Option<u64>,
    /// Whether it waits below another machine on its scene's hand-off stack.
    pub stacked: bool,
    /// Whether it is the machine the table's input goes to now.
    pub focus: bool,
    pub schema_version: u32,
    pub slot: serde_json::Value,
}

/// What `dump table` and `dump <machine>` answer: the committed revision and the part named.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "of", rename_all = "snake_case")]
pub enum DumpReport {
    Table {
        revision: u64,
        commit: u64,
        table: TableDump,
    },
    /// Every place the machine holds, the focus first (a machine can wait on more than one
    /// scene's stack).
    Machine {
        revision: u64,
        commit: u64,
        places: Vec<MachineDump>,
    },
}

/// The most requests one class may have waiting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueBoundReport {
    pub class: QueueClass,
    pub bound: usize,
}

/// The broker's limits, as it was built with them. They are read here and never changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerLimitsReport {
    pub text_slots: usize,
    pub image_slots: usize,
    /// Text slots only the live class may take.
    pub reserved_live_text_slots: usize,
    pub queue_bounds: Vec<QueueBoundReport>,
}

/// `broker queues`: the queues and what runs, and the limits they run under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerQueuesReport {
    #[serde(flatten)]
    pub queues: QueuesReport,
    pub limits: BrokerLimitsReport,
}

/// The level of the host's event sinks: class A only (`always`) or class B detail too
/// (`verbose`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventLevelReport {
    pub level: EventLevel,
}
