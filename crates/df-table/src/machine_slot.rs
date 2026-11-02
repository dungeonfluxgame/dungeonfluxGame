//! The saved form of a machine's state: one versioned record (RES32, H.12).
//!
//! It owns `MachineSlot` (the machine's id, the schema version its state was written under, and
//! the state itself as data), the encode and decode helpers a machine uses to move between its
//! closed state enum and the slot, the tag read that names a state without decoding the rest,
//! and `LoadError`, the typed reason a record cannot be used. Machines keep their states as closed
//! enums and turn them into slots at the edge of `step`, so the table can hold slots of every
//! machine in one shape, hash them, save them and refuse to load a record it cannot migrate
//! (`load.refused`, never a silent best effort). `RecordKind` and `LoadFault` are the vocabulary
//! of that refusal for every persisted record type, not only machine slots.
//!
//! A decode reads the whole value tree into the typed state, so it costs an allocation per field;
//! the calls that only need the state's name read the tag instead. A failure keeps its serde
//! error class (`ErrorClass`) and never the record's contents, which may be private.
//! It never migrates a record itself (the machine's `migrate` hook does) and never decides which
//! version is current (the machine's spec does).

use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json::error::Category;

use crate::element::NameError;
use crate::event::TableEvent;
use crate::identity::{MachineId, TableId};
use crate::listed::listed_enum;
use crate::machine_spec::{MachineSpec, StateName};

#[cfg(test)]
thread_local! {
    static DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many full decodes this thread has made, for the test that proves a call decodes once.
#[cfg(test)]
pub(crate) fn decodes_on_this_thread() -> usize {
    DECODES.with(std::cell::Cell::get)
}

listed_enum! {
    /// Which kind of serde failure it was, and nothing about the data that caused it.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ErrorClass {
        Io,
        Syntax,
        /// The value is well formed but not the shape asked for: a missing or wrongly typed field.
        Data,
        Eof,
    }
}

impl ErrorClass {
    pub fn key(self) -> &'static str {
        match self {
            Self::Io => "io",
            Self::Syntax => "syntax",
            Self::Data => "data",
            Self::Eof => "eof",
        }
    }
}

impl From<Category> for ErrorClass {
    fn from(category: Category) -> Self {
        match category {
            Category::Io => Self::Io,
            Category::Syntax => Self::Syntax,
            Category::Data => Self::Data,
            Category::Eof => Self::Eof,
        }
    }
}

/// A machine's state as a saved record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineSlot {
    machine: MachineId,
    schema_version: u32,
    state: Value,
}

impl MachineSlot {
    /// Writes a typed state under the machine's current schema version.
    pub fn encode<T: Serialize>(spec: &MachineSpec, state: &T) -> Result<Self, SlotError> {
        let state = serde_json::to_value(state).map_err(|error| SlotError::NotEncodable {
            class: error.classify().into(),
        })?;
        Ok(Self {
            machine: spec.id().clone(),
            schema_version: spec.schema_version(),
            state,
        })
    }

    /// A slot from its parts, for a migration step and for records read back from storage.
    pub fn from_parts(machine: MachineId, schema_version: u32, state: Value) -> Self {
        Self {
            machine,
            schema_version,
            state,
        }
    }

    fn check_owner(&self, spec: &MachineSpec) -> Result<(), LoadError> {
        if &self.machine != spec.id() {
            return Err(LoadError::WrongMachine {
                expected: spec.id().clone(),
                found: self.machine.clone(),
            });
        }
        if self.schema_version != spec.schema_version() {
            return Err(LoadError::UnsupportedVersion {
                found: self.schema_version,
                expected: spec.schema_version(),
            });
        }
        Ok(())
    }

    /// Reads the typed state. The slot must belong to the machine and carry its current schema
    /// version: an older record goes through `Machine::load` first.
    pub fn decode<T: DeserializeOwned>(&self, spec: &MachineSpec) -> Result<T, LoadError> {
        self.check_owner(spec)?;
        #[cfg(test)]
        DECODES.with(|count| count.set(count.get() + 1));
        T::deserialize(&self.state).map_err(|error| LoadError::Malformed {
            version: self.schema_version,
            class: error.classify().into(),
        })
    }

    /// The name of the state, read from the tag field of the state enum without decoding the rest
    /// of it. A name the spec does not list is a malformed record.
    pub fn named_state(&self, spec: &MachineSpec, tag_key: &str) -> Result<StateName, LoadError> {
        self.check_owner(spec)?;
        let malformed = LoadError::Malformed {
            version: self.schema_version,
            class: ErrorClass::Data,
        };
        let tag = self
            .state
            .get(tag_key)
            .and_then(Value::as_str)
            .ok_or_else(|| malformed.clone())?;
        let name = StateName::new(tag).map_err(|_| malformed.clone())?;
        if spec.states().contains(&name) {
            Ok(name)
        } else {
            Err(malformed)
        }
    }

    pub fn machine(&self) -> &MachineId {
        &self.machine
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn payload(&self) -> &Value {
        &self.state
    }
}

/// Why a state or its name could not be written as a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotError {
    NotEncodable { class: ErrorClass },
    InvalidStateName(NameError),
}

impl fmt::Display for SlotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotEncodable { class } => {
                write!(
                    formatter,
                    "a state could not be written as data ({})",
                    class.key()
                )
            }
            Self::InvalidStateName(error) => write!(formatter, "a state name is invalid: {error}"),
        }
    }
}

impl std::error::Error for SlotError {}

listed_enum! {
    /// The kinds of persisted record that `load.refused` can name (H.12).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum RecordKind {
        MachineSlot,
        EffectIntent,
        PendingBook,
    }
}

impl RecordKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::MachineSlot => "machine_slot",
            Self::EffectIntent => "effect_intent",
            Self::PendingBook => "pending_book",
        }
    }
}

listed_enum! {
    /// The closed reason of a refused load, as the `load.refused` event records it.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum LoadFault {
        WrongMachine,
        /// The record is newer than the engine, older with no migration, or has no valid version.
        UnsupportedVersion,
        Malformed,
    }
}

impl LoadFault {
    pub fn key(self) -> &'static str {
        match self {
            Self::WrongMachine => "wrong_machine",
            Self::UnsupportedVersion => "unsupported_version",
            Self::Malformed => "malformed",
        }
    }
}

/// Why a saved record cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    WrongMachine {
        expected: MachineId,
        found: MachineId,
    },
    UnsupportedVersion {
        found: u32,
        expected: u32,
    },
    Malformed {
        version: u32,
        class: ErrorClass,
    },
}

impl LoadError {
    pub fn fault(&self) -> LoadFault {
        match self {
            Self::WrongMachine { .. } => LoadFault::WrongMachine,
            Self::UnsupportedVersion { .. } => LoadFault::UnsupportedVersion,
            Self::Malformed { .. } => LoadFault::Malformed,
        }
    }

    /// The `load.refused` event of refusing `slot` for the machine `spec` describes, when the
    /// host loads `table` (H.12).
    pub fn refusal_event(
        &self,
        table: TableId,
        spec: &MachineSpec,
        slot: &MachineSlot,
    ) -> TableEvent {
        TableEvent::LoadRefused {
            table,
            record: RecordKind::MachineSlot,
            machine: Some(spec.id().clone()),
            version: u64::from(slot.schema_version()),
            expected: u64::from(spec.schema_version()),
            cause: self.fault(),
        }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongMachine { expected, found } => {
                write!(formatter, "the record is for {found}, not {expected}")
            }
            Self::UnsupportedVersion { found, expected } => write!(
                formatter,
                "the record has schema version {found} and cannot be migrated to {expected}"
            ),
            Self::Malformed { version, class } => write!(
                formatter,
                "the record at version {version} is not readable ({})",
                class.key()
            ),
        }
    }
}

impl std::error::Error for LoadError {}
