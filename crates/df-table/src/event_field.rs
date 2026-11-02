//! The sealed list of types an event may carry, and the writer that renders them.
//!
//! It owns `EventField`, the one trait an audit-line value must implement, and the crate-private
//! `LineWriter` that writes `verb key=value` into any `fmt::Write` without allocating. Only ids,
//! counts and closed keys implement it, and the trait is sealed, so a type outside this file
//! cannot join the list. `PlayerText`, `Narration` and `Secret<T>` are deliberately absent: a
//! rendering function that needs a value to be an `EventField` cannot be handed one (see the
//! examples in `private_data.rs`, and the test that pins the absence). It never decides what an
//! event contains: `event.rs` does.

use std::fmt;

use crate::config::{SettingFault, SettingName};
use crate::effects::{EffectRefusal, TableEffectKind};
use crate::element::{Degrees, ElementId, FieldName, KindName, Percent};
use crate::event::{EffectKind, FailureCause, PipelineStep, RejectionCause, TransformKind};
use crate::identity::{EvidenceRef, MachineId, OperationId, SceneId, TableId, TurnSeq};
use crate::machine::{MachineFailureCause, StateName};
use crate::machine_slot::LoadFault;
use crate::outcome::{DeclineCode, OutcomeKind};
use crate::owner_queue::{Lane, NotDeclinedCause};
use crate::ports::PortName;
use crate::private_data::LengthClass;

mod sealed {
    pub trait Sealed {}
}

/// A value that may be written into an audit line: an id, a count or a closed key.
pub trait EventField: sealed::Sealed {
    fn write_value<W: fmt::Write>(&self, out: &mut W) -> fmt::Result;
}

macro_rules! event_field_by_display {
    ($($field_type:ty),+ $(,)?) => {
        $(
            impl sealed::Sealed for $field_type {}

            impl EventField for $field_type {
                fn write_value<W: fmt::Write>(&self, out: &mut W) -> fmt::Result {
                    write!(out, "{self}")
                }
            }
        )+
    };
}

macro_rules! event_field_by_key {
    ($($field_type:ty),+ $(,)?) => {
        $(
            impl sealed::Sealed for $field_type {}

            impl EventField for $field_type {
                fn write_value<W: fmt::Write>(&self, out: &mut W) -> fmt::Result {
                    out.write_str(self.key())
                }
            }
        )+
    };
}

event_field_by_display!(
    u64,
    usize,
    Degrees,
    Percent,
    ElementId,
    KindName,
    FieldName,
    SettingName,
    MachineId,
    TableId,
    SceneId,
    TurnSeq,
    OperationId,
    EvidenceRef,
    StateName,
);

event_field_by_key!(
    EffectKind,
    RejectionCause,
    DeclineCode,
    OutcomeKind,
    PipelineStep,
    PortName,
    FailureCause,
    SettingFault,
    LengthClass,
    TransformKind,
    Lane,
    NotDeclinedCause,
    TableEffectKind,
    EffectRefusal,
    MachineFailureCause,
    LoadFault,
);

/// Writes one audit line piece by piece: the verb, then ` key=value` pairs.
pub(crate) struct LineWriter<'a, W: fmt::Write> {
    out: &'a mut W,
}

impl<'a, W: fmt::Write> LineWriter<'a, W> {
    pub(crate) fn start(out: &'a mut W, verb: &str) -> Result<Self, fmt::Error> {
        out.write_str(verb)?;
        Ok(Self { out })
    }

    pub(crate) fn field<T: EventField>(&mut self, key: &str, value: &T) -> fmt::Result {
        self.out.write_char(' ')?;
        self.out.write_str(key)?;
        self.out.write_char('=')?;
        value.write_value(self.out)
    }

    pub(crate) fn field_if_present<T: EventField>(
        &mut self,
        key: &str,
        value: &Option<T>,
    ) -> fmt::Result {
        match value {
            Some(value) => self.field(key, value),
            None => Ok(()),
        }
    }
}
