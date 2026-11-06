//! The registry's report and the ports' report: which machines are registered and why they run,
//! and which ports the host wired and who needs them.
//!
//! Both are the table's configuration, so nothing here is hidden from an administrator. A seat
//! sees which machines exist and whether they run, and nothing of their specs or of the wiring. A
//! disabled machine is listed, but the registry hands out the spec of an enabled machine only, so
//! a disabled one has no spec line. `WiredPorts` pairs the registry with the ports the host wired,
//! because the registry does not know which ports exist outside it.
//!
//! The closed keys of the registry's own enums (`enablement`, `reason`) are written as their keys.
//! `Clock` and `EventSink` cannot fail and have no `PortName`, so they are not listed.

use serde::Serialize;

use crate::identity::MachineId;
use crate::inspect::{Inspect, InspectView, Needs, Visible};
use crate::machine::spec::{MachineSpec, StateName};
use crate::machine::{InputClaim, Scope, SuspendPolicy};
use crate::ports::{ActionId, PortName};
use crate::registry::{Enablement, EnablementReason, Registry};

/// What a machine's spec declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MachineSpecLine {
    pub schema_version: u32,
    pub states: Vec<StateName>,
    pub hand_off_targets: Vec<MachineId>,
    pub required_ports: Vec<PortName>,
    pub rules_actions: Vec<ActionId>,
    pub unseated_actions: Vec<ActionId>,
    pub claimed_inputs: Vec<InputClaim>,
    pub on_suspend: SuspendPolicy,
}

impl MachineSpecLine {
    pub(super) fn of(spec: &MachineSpec) -> Self {
        Self {
            schema_version: spec.schema_version(),
            states: spec.states().to_vec(),
            hand_off_targets: spec.hand_off_targets().to_vec(),
            required_ports: spec.required_ports().to_vec(),
            rules_actions: spec.rules_actions().to_vec(),
            unseated_actions: spec.unseated_actions().to_vec(),
            claimed_inputs: spec.claimed_inputs().to_vec(),
            on_suspend: spec.on_suspend(),
        }
    }
}

/// One registered machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RegisteredMachine {
    pub machine: MachineId,
    pub scope: Scope,
    /// `enabled` or `disabled`.
    pub enablement: &'static str,
    /// `default_on`, `flagged_on` or `flagged_off`.
    pub reason: &'static str,
    /// `None` for a disabled machine, whose spec the registry does not hand out.
    pub spec: Visible<Option<MachineSpecLine>>,
}

/// The registered machines, in id order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RegisteredMachinesReport {
    pub machines: Vec<RegisteredMachine>,
}

fn enablement_key(enablement: Enablement) -> &'static str {
    match enablement {
        Enablement::Enabled => "enabled",
        Enablement::Disabled => "disabled",
    }
}

fn reason_key(reason: EnablementReason) -> &'static str {
    match reason {
        EnablementReason::DefaultOn => "default_on",
        EnablementReason::FlaggedOn => "flagged_on",
        EnablementReason::FlaggedOff => "flagged_off",
    }
}

impl Inspect for Registry {
    type Report = RegisteredMachinesReport;

    fn inspect(&self, view: InspectView) -> RegisteredMachinesReport {
        let machines = self
            .report()
            .machines
            .into_iter()
            .map(|status| {
                let spec = Visible::when(view, Needs::Host, || {
                    self.machine(&status.machine)
                        .map(|machine| MachineSpecLine::of(machine.spec()))
                });
                RegisteredMachine {
                    machine: status.machine,
                    scope: status.scope,
                    enablement: enablement_key(status.enablement),
                    reason: reason_key(status.reason),
                    spec,
                }
            })
            .collect();
        RegisteredMachinesReport { machines }
    }
}

/// One port: whether the host wired it and which enabled machines need it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PortLine {
    pub port: PortName,
    pub wired: bool,
    pub required_by: Vec<MachineId>,
}

/// Every port that can fail, in declaration order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PortsReport {
    pub ports: Visible<Vec<PortLine>>,
}

/// The registry together with the ports the host wired.
#[derive(Clone, Copy)]
pub struct WiredPorts<'a> {
    pub registry: &'a Registry,
    pub wired: &'a [PortName],
}

impl Inspect for WiredPorts<'_> {
    type Report = PortsReport;

    fn inspect(&self, view: InspectView) -> PortsReport {
        let enabled: Vec<&MachineSpec> = self
            .registry
            .report()
            .enabled()
            .filter_map(|id| self.registry.machine(id))
            .map(|machine| machine.spec())
            .collect();
        PortsReport {
            ports: Visible::when(view, Needs::Host, || {
                PortName::ALL
                    .iter()
                    .map(|port| PortLine {
                        port: *port,
                        wired: self.wired.contains(port),
                        required_by: enabled
                            .iter()
                            .filter(|spec| spec.required_ports().contains(port))
                            .map(|spec| spec.id().clone())
                            .collect(),
                    })
                    .collect()
            }),
        }
    }
}
