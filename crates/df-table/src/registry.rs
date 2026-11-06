//! The machine registry: one line per machine, validated at startup (D24, RES28, O.5).
//!
//! It owns `RegistryBuilder` (register machines, set enable flags, build), `Registry` (the
//! validated set and the lookup of an enabled machine), `MachineFlags` (a table's per-machine
//! on or off) and `RegistryReport` (which machines are enabled and why). Building checks every
//! rule that can be checked before a table starts: ids are unique, every flag names a registered
//! machine, every hand-off target of an enabled machine is registered and enabled and passes the
//! scope rule (`Scope::may_hand_off_to`), every port an enabled machine needs is wired, and a
//! machine that sends actions to the rules owner declares that it needs the rules owner. Every
//! problem comes back at once as a typed `RegistryError`, never as a panic and never as a
//! machine silently left out. A disabled machine is registered and reported, but nothing is
//! checked about its neighbours and it is never returned by `machine`.
//!
//! A table is startable when an enabled `Table` machine can begin it and an enabled `Group`
//! machine can run a scene; without both the registry is refused (`NotStartable`). A seat lane
//! ends by releasing itself and a group machine may return to the frame below it, so neither
//! needs a target machine. Startup checks what a spec declares; `Registry::check_hand_off`
//! applies the same declared-target, enabled and scope rules to the value a machine's `exit`
//! actually returned, which is what the pipeline calls before it moves the table.
//!
//! Adding a machine is its module plus one `register` line. It never runs a machine and never
//! routes an input: the pipeline does.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use crate::listed::listed_enum;
use crate::machine::{HandOff, HandOffTarget, Machine, MachineId, MachineSpec, Scope};
use crate::ports::PortName;

/// A table's choice to run a machine or not. A machine with no flag runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagState {
    On,
    Off,
}

/// The per-machine flags of one table (O.5 `MachineFlags`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MachineFlags {
    flags: BTreeMap<MachineId, FlagState>,
}

impl MachineFlags {
    pub fn with_flag(mut self, machine: MachineId, state: FlagState) -> Self {
        self.flags.insert(machine, state);
        self
    }

    pub fn flag(&self, machine: &MachineId) -> Option<FlagState> {
        self.flags.get(machine).copied()
    }
}

/// Whether a registered machine runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Enablement {
    Enabled,
    Disabled,
}

/// Why a machine is enabled or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnablementReason {
    /// No flag names it, and a machine runs unless told not to.
    DefaultOn,
    FlaggedOn,
    FlaggedOff,
}

/// One machine's line in the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineStatus {
    pub machine: MachineId,
    pub scope: Scope,
    pub enablement: Enablement,
    pub reason: EnablementReason,
}

/// Which machines are registered, which run and why, in id order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryReport {
    pub machines: Vec<MachineStatus>,
}

impl RegistryReport {
    pub fn enabled(&self) -> impl Iterator<Item = &MachineId> {
        self.machines
            .iter()
            .filter(|status| status.enablement == Enablement::Enabled)
            .map(|status| &status.machine)
    }
}

listed_enum! {
    /// The kinds of startup error, without their payload, so a test can prove each is reachable.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub enum RegistryErrorKind {
        DuplicateMachine,
        UnknownFlagTarget,
        UnknownHandOffTarget,
        HandOffTargetDisabled,
        ScopeRuleBroken,
        MissingPort,
        RulesActionsNeedRulesOwner,
        NoEnabledMachines,
        NotStartable,
    }
}

/// A startup validation failure. Each variant names what the author fixes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    DuplicateMachine {
        machine: MachineId,
    },
    UnknownFlagTarget {
        machine: MachineId,
    },
    UnknownHandOffTarget {
        machine: MachineId,
        target: MachineId,
    },
    HandOffTargetDisabled {
        machine: MachineId,
        target: MachineId,
    },
    ScopeRuleBroken {
        machine: MachineId,
        target: MachineId,
        from: Scope,
        to: Scope,
    },
    MissingPort {
        machine: MachineId,
        port: PortName,
    },
    RulesActionsNeedRulesOwner {
        machine: MachineId,
    },
    NoEnabledMachines,
    /// No enabled machine of this scope exists, so no table can start or run a scene.
    NotStartable {
        missing: Scope,
    },
}

impl RegistryError {
    pub fn kind(&self) -> RegistryErrorKind {
        match self {
            Self::DuplicateMachine { .. } => RegistryErrorKind::DuplicateMachine,
            Self::UnknownFlagTarget { .. } => RegistryErrorKind::UnknownFlagTarget,
            Self::UnknownHandOffTarget { .. } => RegistryErrorKind::UnknownHandOffTarget,
            Self::HandOffTargetDisabled { .. } => RegistryErrorKind::HandOffTargetDisabled,
            Self::ScopeRuleBroken { .. } => RegistryErrorKind::ScopeRuleBroken,
            Self::MissingPort { .. } => RegistryErrorKind::MissingPort,
            Self::RulesActionsNeedRulesOwner { .. } => {
                RegistryErrorKind::RulesActionsNeedRulesOwner
            }
            Self::NoEnabledMachines => RegistryErrorKind::NoEnabledMachines,
            Self::NotStartable { .. } => RegistryErrorKind::NotStartable,
        }
    }

    /// The machine the error is about, when it is about one.
    pub fn machine(&self) -> Option<&MachineId> {
        match self {
            Self::DuplicateMachine { machine }
            | Self::UnknownFlagTarget { machine }
            | Self::UnknownHandOffTarget { machine, .. }
            | Self::HandOffTargetDisabled { machine, .. }
            | Self::ScopeRuleBroken { machine, .. }
            | Self::MissingPort { machine, .. }
            | Self::RulesActionsNeedRulesOwner { machine } => Some(machine),
            Self::NoEnabledMachines | Self::NotStartable { .. } => None,
        }
    }
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateMachine { machine } => {
                write!(formatter, "the machine {machine} is registered twice")
            }
            Self::UnknownFlagTarget { machine } => {
                write!(formatter, "a flag names {machine}, which is not registered")
            }
            Self::UnknownHandOffTarget { machine, target } => {
                write!(
                    formatter,
                    "{machine} hands off to {target}, which is not registered"
                )
            }
            Self::HandOffTargetDisabled { machine, target } => {
                write!(
                    formatter,
                    "{machine} hands off to {target}, which is disabled"
                )
            }
            Self::ScopeRuleBroken {
                machine,
                target,
                from,
                to,
            } => write!(
                formatter,
                "{machine} ({}) may not hand off to {target} ({})",
                from.key(),
                to.key()
            ),
            Self::MissingPort { machine, port } => {
                write!(
                    formatter,
                    "{machine} needs the {} port, which is not wired",
                    port.key()
                )
            }
            Self::RulesActionsNeedRulesOwner { machine } => write!(
                formatter,
                "{machine} sends actions to the rules owner but does not require that port"
            ),
            Self::NoEnabledMachines => formatter.write_str("no machine is enabled"),
            Self::NotStartable { missing } => write!(
                formatter,
                "no enabled machine has {} scope, so the table cannot start",
                missing.key()
            ),
        }
    }
}

impl std::error::Error for RegistryError {}

struct Registered {
    machine: Arc<dyn Machine>,
    enablement: Enablement,
    reason: EnablementReason,
}

/// Collects machines and flags, then validates them together.
#[derive(Default)]
pub struct RegistryBuilder {
    machines: Vec<Arc<dyn Machine>>,
    flags: MachineFlags,
}

impl RegistryBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a machine: the one line a new machine adds.
    pub fn register(self, machine: impl Machine + 'static) -> Self {
        self.register_shared(Arc::new(machine))
    }

    pub fn register_shared(mut self, machine: Arc<dyn Machine>) -> Self {
        self.machines.push(machine);
        self
    }

    pub fn with_flags(mut self, flags: MachineFlags) -> Self {
        self.flags = flags;
        self
    }

    /// Validates everything and returns the registry, or every problem found, in a fixed order.
    /// `wired` lists the ports the host provides.
    pub fn build(self, wired: &[PortName]) -> Result<Registry, Vec<RegistryError>> {
        let mut errors = Vec::new();
        let mut registered: BTreeMap<MachineId, Registered> = BTreeMap::new();
        for machine in self.machines {
            let id = machine.spec().id().clone();
            if registered.contains_key(&id) {
                errors.push(RegistryError::DuplicateMachine { machine: id });
                continue;
            }
            let flag = self.flags.flag(&id);
            let (enablement, reason) = match flag {
                None => (Enablement::Enabled, EnablementReason::DefaultOn),
                Some(FlagState::On) => (Enablement::Enabled, EnablementReason::FlaggedOn),
                Some(FlagState::Off) => (Enablement::Disabled, EnablementReason::FlaggedOff),
            };
            registered.insert(
                id,
                Registered {
                    machine,
                    enablement,
                    reason,
                },
            );
        }
        for flagged in self.flags.flags.keys() {
            if !registered.contains_key(flagged) {
                errors.push(RegistryError::UnknownFlagTarget {
                    machine: flagged.clone(),
                });
            }
        }
        for entry in registered.values() {
            if entry.enablement == Enablement::Enabled {
                check_enabled_machine(entry.machine.as_ref(), &registered, wired, &mut errors);
            }
        }
        let any_enabled = registered
            .values()
            .any(|entry| entry.enablement == Enablement::Enabled);
        if !any_enabled {
            errors.push(RegistryError::NoEnabledMachines);
        } else {
            for needed in [Scope::Table, Scope::Group] {
                let present = registered.values().any(|entry| {
                    entry.enablement == Enablement::Enabled
                        && entry.machine.spec().scope() == needed
                });
                if !present {
                    errors.push(RegistryError::NotStartable { missing: needed });
                }
            }
        }
        if errors.is_empty() {
            Ok(Registry { registered })
        } else {
            Err(errors)
        }
    }
}

fn check_enabled_machine(
    machine: &dyn Machine,
    registered: &BTreeMap<MachineId, Registered>,
    wired: &[PortName],
    errors: &mut Vec<RegistryError>,
) {
    let spec = machine.spec();
    let id = spec.id();
    for port in spec.required_ports() {
        if !wired.contains(port) {
            errors.push(RegistryError::MissingPort {
                machine: id.clone(),
                port: *port,
            });
        }
    }
    if !spec.rules_actions().is_empty() && !spec.required_ports().contains(&PortName::RulesOwner) {
        errors.push(RegistryError::RulesActionsNeedRulesOwner {
            machine: id.clone(),
        });
    }
    for target in spec.hand_off_targets() {
        let Some(neighbour) = registered.get(target) else {
            errors.push(RegistryError::UnknownHandOffTarget {
                machine: id.clone(),
                target: target.clone(),
            });
            continue;
        };
        if neighbour.enablement == Enablement::Disabled {
            errors.push(RegistryError::HandOffTargetDisabled {
                machine: id.clone(),
                target: target.clone(),
            });
        }
        let to = neighbour.machine.spec().scope();
        if !spec.scope().may_hand_off_to(to) {
            errors.push(RegistryError::ScopeRuleBroken {
                machine: id.clone(),
                target: target.clone(),
                from: spec.scope(),
                to,
            });
        }
    }
}

/// Why the value a machine's `exit` returned cannot be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandOffTargetError {
    /// The machine that is leaving is not registered and enabled.
    UnknownSource {
        machine: MachineId,
    },
    UndeclaredTarget {
        target: MachineId,
    },
    UnknownTarget {
        target: MachineId,
    },
    TargetDisabled {
        target: MachineId,
    },
    ScopeRuleBroken {
        from: Scope,
        to: Scope,
    },
    /// Only a seat lane can release itself.
    LaneNotReleasable {
        scope: Scope,
    },
    /// Only a group machine has a frame below it to return to.
    NothingBelow {
        scope: Scope,
    },
}

impl fmt::Display for HandOffTargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSource { machine } => {
                write!(formatter, "{machine} is not registered and enabled")
            }
            Self::UndeclaredTarget { target } => {
                write!(
                    formatter,
                    "the spec does not declare a hand-off to {target}"
                )
            }
            Self::UnknownTarget { target } => write!(formatter, "{target} is not registered"),
            Self::TargetDisabled { target } => write!(formatter, "{target} is disabled"),
            Self::ScopeRuleBroken { from, to } => write!(
                formatter,
                "a {} machine may not hand off to a {} machine",
                from.key(),
                to.key()
            ),
            Self::LaneNotReleasable { scope } => {
                write!(
                    formatter,
                    "a {} machine has no seat lane to release",
                    scope.key()
                )
            }
            Self::NothingBelow { scope } => {
                write!(formatter, "a {} machine has no frame below it", scope.key())
            }
        }
    }
}

impl std::error::Error for HandOffTargetError {}

/// Applies the declared-target, enabled and scope rules to a hand-off `from` returned. `lookup`
/// answers for a machine id: its scope and whether it is enabled, or `None` when unknown.
pub(crate) fn check_hand_off_route(
    from: &MachineSpec,
    hand_off: &HandOff,
    lookup: impl Fn(&MachineId) -> Option<(Scope, Enablement)>,
) -> Result<(), HandOffTargetError> {
    match hand_off.target() {
        HandOffTarget::Machine(target) => {
            if !from.hand_off_targets().contains(target) {
                return Err(HandOffTargetError::UndeclaredTarget {
                    target: target.clone(),
                });
            }
            let (to, enablement) =
                lookup(target).ok_or_else(|| HandOffTargetError::UnknownTarget {
                    target: target.clone(),
                })?;
            if enablement == Enablement::Disabled {
                return Err(HandOffTargetError::TargetDisabled {
                    target: target.clone(),
                });
            }
            if !from.scope().may_hand_off_to(to) {
                return Err(HandOffTargetError::ScopeRuleBroken {
                    from: from.scope(),
                    to,
                });
            }
            Ok(())
        }
        HandOffTarget::ReleaseLane if from.scope() == Scope::Seat => Ok(()),
        HandOffTarget::ReleaseLane => Err(HandOffTargetError::LaneNotReleasable {
            scope: from.scope(),
        }),
        HandOffTarget::ReturnToFrameBelow if from.scope() == Scope::Group => Ok(()),
        HandOffTarget::ReturnToFrameBelow => Err(HandOffTargetError::NothingBelow {
            scope: from.scope(),
        }),
    }
}

/// Where a checked hand-off sends the table.
#[derive(Clone)]
pub enum HandOffRoute<'a> {
    To(&'a Arc<dyn Machine>),
    ReleasesLane,
    ReturnsBelow,
}

/// The validated set of machines.
pub struct Registry {
    registered: BTreeMap<MachineId, Registered>,
}

impl Registry {
    /// The machine, if it is registered and enabled.
    pub fn machine(&self, id: &MachineId) -> Option<&Arc<dyn Machine>> {
        self.registered
            .get(id)
            .filter(|entry| entry.enablement == Enablement::Enabled)
            .map(|entry| &entry.machine)
    }

    /// Checks the hand-off the machine `from` returned from `exit`, and says where it goes. The
    /// stack rules (depth, pending resolutions) are checked by the table (C4, C6), not here.
    pub fn check_hand_off(
        &self,
        from: &MachineId,
        hand_off: &HandOff,
    ) -> Result<HandOffRoute<'_>, HandOffTargetError> {
        let source = self
            .machine(from)
            .ok_or_else(|| HandOffTargetError::UnknownSource {
                machine: from.clone(),
            })?;
        check_hand_off_route(source.spec(), hand_off, |id| {
            self.registered
                .get(id)
                .map(|entry| (entry.machine.spec().scope(), entry.enablement))
        })?;
        Ok(match hand_off.target() {
            HandOffTarget::Machine(target) => match self.machine(target) {
                Some(machine) => HandOffRoute::To(machine),
                None => {
                    return Err(HandOffTargetError::TargetDisabled {
                        target: target.clone(),
                    });
                }
            },
            HandOffTarget::ReleaseLane => HandOffRoute::ReleasesLane,
            HandOffTarget::ReturnToFrameBelow => HandOffRoute::ReturnsBelow,
        })
    }

    pub fn report(&self) -> RegistryReport {
        RegistryReport {
            machines: self
                .registered
                .iter()
                .map(|(id, entry)| MachineStatus {
                    machine: id.clone(),
                    scope: entry.machine.spec().scope(),
                    enablement: entry.enablement,
                    reason: entry.reason,
                })
                .collect(),
        }
    }
}
