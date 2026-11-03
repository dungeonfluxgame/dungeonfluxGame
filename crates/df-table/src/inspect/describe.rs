//! `describe`: the verbs, the registered machines and the fields of every report, generated from
//! the registry and the verb table (H.16, RES33).
//!
//! The host builds its command list and its `help` from this value instead of keeping a second
//! list: each machine gets the verbs that every machine must support (`Verb::machine_required`)
//! and the verbs that apply where meaningful, with the usage filled in from the machine's own id
//! and state names, so `force lobby <gathering|ready>` is read off the spec and cannot drift from
//! it. The fields of each report are a table here (`ReportName::fields`), and a test compares each
//! table with the keys of the report it names and with what an audience that lacks the clearance
//! is shown, so a report cannot gain, lose or re-gate a field unnoticed.
//!
//! It reads the registry and changes nothing.

use serde::Serialize;

use crate::identity::MachineId;
use crate::inspect::Needs;
use crate::inspect::registry::MachineSpecLine;
use crate::inspect::verbs::{MachineSupport, Verb, VerbSpec};
use crate::listed::listed_enum;
use crate::machine::Scope;
use crate::registry::{Enablement, Registry};

listed_enum! {
    /// The reports a host can ask for, one per stateful module.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ReportName {
        OwnerQueue,
        RegisteredMachines,
        Ports,
        Machines,
        Table,
        Pending,
        Rulings,
        Requests,
        Results,
        Settings,
        Pipeline,
    }
}

/// One top-level field of a report and who is told its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FieldSpec {
    pub name: &'static str,
    /// The audience the value is hidden from unless it meets this; `None` when every audience is
    /// shown the field (a list a seat may see only part of is `None` too: the entries are
    /// filtered, not the field).
    pub gate: Option<Needs>,
}

const fn open(name: &'static str) -> FieldSpec {
    FieldSpec { name, gate: None }
}

const fn gated(name: &'static str, needs: Needs) -> FieldSpec {
    FieldSpec {
        name,
        gate: Some(needs),
    }
}

const OWNER_QUEUE_FIELDS: &[FieldSpec] = &[
    gated("last_turn_seq", Needs::Host),
    gated("remembered_operations", Needs::Host),
    gated("lanes", Needs::Host),
    open("seats"),
    open("in_progress"),
];
const REGISTERED_MACHINES_FIELDS: &[FieldSpec] = &[open("machines")];
const PORTS_FIELDS: &[FieldSpec] = &[gated("ports", Needs::Host)];
const MACHINES_FIELDS: &[FieldSpec] = &[open("machines")];
const TABLE_FIELDS: &[FieldSpec] = &[
    open("revision"),
    open("status"),
    open("world_time_s"),
    open("seat_capacity"),
    open("split_policy"),
    open("focus_scene"),
    open("scenes"),
    open("seats"),
    open("cast"),
    gated("digest", Needs::Operator),
];
const PENDING_FIELDS: &[FieldSpec] = &[gated("open", Needs::Host), open("pendings")];
const RULINGS_FIELDS: &[FieldSpec] = &[gated("stored", Needs::Host), gated("entries", Needs::Host)];
const REQUESTS_FIELDS: &[FieldSpec] = &[
    gated("outstanding", Needs::Host),
    gated("unissued", Needs::Host),
    gated("entries", Needs::Host),
];
const RESULTS_FIELDS: &[FieldSpec] = &[gated("remembered", Needs::Host)];
const SETTINGS_FIELDS: &[FieldSpec] = &[open("settings")];
const PIPELINE_FIELDS: &[FieldSpec] = &[
    open("table"),
    open("host"),
    gated("state_hash", Needs::Operator),
    open("facts"),
    open("pending"),
    open("rulings"),
    open("requests"),
    open("results"),
    open("clarifications"),
    open("projections"),
];

impl ReportName {
    /// What the report is a report of.
    pub fn subject(self) -> &'static str {
        match self {
            Self::OwnerQueue => "the owner queue",
            Self::RegisteredMachines => "the registered machines",
            Self::Ports => "the ports the host wired",
            Self::Machines => "the active machines of the table",
            Self::Table => "the table state",
            Self::Pending => "the open pending resolutions",
            Self::Rulings => "the stored rulings",
            Self::Requests => "the resource requests that are out",
            Self::Results => "the remembered operations",
            Self::Settings => "the settings in force",
            Self::Pipeline => "everything the pipeline's table holds",
        }
    }

    /// The top-level fields of the report, in the order they are written.
    pub fn fields(self) -> &'static [FieldSpec] {
        match self {
            Self::OwnerQueue => OWNER_QUEUE_FIELDS,
            Self::RegisteredMachines => REGISTERED_MACHINES_FIELDS,
            Self::Ports => PORTS_FIELDS,
            Self::Machines => MACHINES_FIELDS,
            Self::Table => TABLE_FIELDS,
            Self::Pending => PENDING_FIELDS,
            Self::Rulings => RULINGS_FIELDS,
            Self::Requests => REQUESTS_FIELDS,
            Self::Results => RESULTS_FIELDS,
            Self::Settings => SETTINGS_FIELDS,
            Self::Pipeline => PIPELINE_FIELDS,
        }
    }
}

/// One report and its fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportDescription {
    pub report: ReportName,
    pub subject: &'static str,
    pub fields: Vec<FieldSpec>,
}

/// A verb a machine has, with its usage filled in from the machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MachineVerbUse {
    pub verb: Verb,
    pub required: bool,
    pub usage: String,
}

/// One registered machine as the CLI sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MachineDescription {
    pub machine: MachineId,
    pub scope: Scope,
    pub enabled: bool,
    /// `None` for a disabled machine, whose spec the registry does not hand out.
    pub spec: Option<MachineSpecLine>,
    /// Empty for a disabled machine.
    pub verbs: Vec<MachineVerbUse>,
}

/// The whole command surface of one build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Description {
    pub verbs: Vec<VerbSpec>,
    pub machines: Vec<MachineDescription>,
    pub reports: Vec<ReportDescription>,
}

fn machine_usage(verb: Verb, machine: &MachineId, spec: &MachineSpecLine) -> String {
    let states: Vec<&str> = spec.states.iter().map(|state| state.as_str()).collect();
    verb.spec()
        .usage
        .replace("<machine|table>", machine.as_str())
        .replace("<machine>", machine.as_str())
        .replace("<state>", &format!("<{}>", states.join("|")))
}

fn machine_verbs(machine: &MachineId, spec: &MachineSpecLine) -> Vec<MachineVerbUse> {
    Verb::ALL
        .iter()
        .filter(|verb| verb.spec().machines != MachineSupport::NotApplicable)
        .map(|verb| MachineVerbUse {
            verb: *verb,
            required: verb.spec().machines == MachineSupport::Required,
            usage: machine_usage(*verb, machine, spec),
        })
        .collect()
}

/// The verbs, the registered machines with the verbs each has, and the fields of every report.
pub fn describe(registry: &Registry) -> Description {
    let machines = registry
        .report()
        .machines
        .into_iter()
        .map(|status| {
            let spec = registry
                .machine(&status.machine)
                .map(|machine| MachineSpecLine::of(machine.spec()));
            let verbs = spec
                .as_ref()
                .map(|line| machine_verbs(&status.machine, line))
                .unwrap_or_default();
            MachineDescription {
                enabled: status.enablement == Enablement::Enabled,
                machine: status.machine,
                scope: status.scope,
                spec,
                verbs,
            }
        })
        .collect();
    Description {
        verbs: Verb::ALL.iter().map(|verb| *verb.spec()).collect(),
        machines,
        reports: ReportName::ALL
            .iter()
            .map(|report| ReportDescription {
                report: *report,
                subject: report.subject(),
                fields: report.fields().to_vec(),
            })
            .collect(),
    }
}
