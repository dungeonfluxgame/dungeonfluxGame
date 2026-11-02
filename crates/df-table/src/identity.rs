//! The ids and the correlation that every event and every port call carries.
//!
//! It owns the small typed ids (`TableId`, `SceneId`, `SeatId`, `MachineId`, `OperationId`,
//! `FactId`, `EvidenceRef`, `TurnSeq`, `SessionRevision`) and `Correlation`, the three values that
//! join a broker request, a narration job and a projection back to the input that caused them
//! (O.6, H.10). It never issues an id or reads a clock: whoever owns the counter hands the value
//! in. Building one fails with a typed error only where a value can be invalid.
//!
//! `OperationId`, `FactId`, `RecoveryEpoch` and `SessionRevision` mirror their `df-types` and
//! `df-model` namesakes, so the type is swapped for the real one when `df-table` takes the edge at
//! C6: a nonzero 128-bit id (`from_bytes`, `from_hex`, `as_bytes`) written as 32 lowercase
//! hexadecimal digits, where zero, an uppercase digit and a wrong length are refused with the same
//! error shapes; a non-zero epoch; and `next_sequence` with `SequenceOverflow`. `EvidenceRef`
//! uses the same shape. `TableId`, `SceneId`, `SeatId` and `TurnSeq` are local counters with no
//! namesake yet (a seat will become a `MemberId` at C6), and `MachineId` is a registered name.

use std::fmt;
use std::num::{NonZeroU64, NonZeroU128};

use serde::{Deserialize, Serialize};

use crate::element::name_type;

name_type!(
    /// The registered name of a machine, such as `lobby`.
    MachineId
);

/// Why 16 bytes were refused as an identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityError {
    InvalidLength { actual: usize },
    Zero,
}

impl fmt::Display for IdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { actual } => {
                write!(formatter, "an id is 16 bytes, not {actual}")
            }
            Self::Zero => formatter.write_str("an id cannot be all zero"),
        }
    }
}

impl std::error::Error for IdentityError {}

/// Why text was refused as an identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextIdentityError {
    InvalidLength {
        actual: usize,
    },
    /// A character that is not a lowercase hexadecimal digit.
    Malformed,
    Zero,
}

impl fmt::Display for TextIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { actual } => {
                write!(formatter, "an id is 32 hexadecimal digits, not {actual}")
            }
            Self::Malformed => formatter.write_str("an id uses only 0 to 9 and a to f"),
            Self::Zero => formatter.write_str("an id cannot be all zero"),
        }
    }
}

impl std::error::Error for TextIdentityError {}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn parse_canonical_hex(text: &str) -> Result<[u8; 16], TextIdentityError> {
    let encoded = text.as_bytes();
    if encoded.len() != 32 {
        return Err(TextIdentityError::InvalidLength {
            actual: encoded.len(),
        });
    }
    let mut bytes = [0_u8; 16];
    let (pairs, _) = encoded.as_chunks::<2>();
    for (byte, [high, low]) in bytes.iter_mut().zip(pairs) {
        let high = hex_nibble(*high).ok_or(TextIdentityError::Malformed)?;
        let low = hex_nibble(*low).ok_or(TextIdentityError::Malformed)?;
        *byte = (high << 4) | low;
    }
    if bytes == [0; 16] {
        return Err(TextIdentityError::Zero);
    }
    Ok(bytes)
}

macro_rules! identity {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(into = "String", try_from = "String")]
        pub struct $name([u8; 16]);

        impl $name {
            /// Parses exactly 32 lowercase hexadecimal digits in canonical byte order.
            pub fn from_hex(text: &str) -> Result<Self, TextIdentityError> {
                parse_canonical_hex(text).map(Self)
            }

            /// Accepts exactly 16 bytes with at least one nonzero byte.
            pub fn from_bytes(bytes: &[u8]) -> Result<Self, IdentityError> {
                let value: [u8; 16] = bytes.try_into().map_err(|_| IdentityError::InvalidLength {
                    actual: bytes.len(),
                })?;
                if value == [0; 16] {
                    return Err(IdentityError::Zero);
                }
                Ok(Self(value))
            }

            /// An id from a number that is already known to be nonzero, in canonical byte order.
            pub fn from_nonzero_u128(value: NonZeroU128) -> Self {
                Self(value.get().to_be_bytes())
            }

            pub fn as_bytes(&self) -> &[u8; 16] {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.iter().try_for_each(|byte| write!(formatter, "{byte:02x}"))
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> Self {
                id.to_string()
            }
        }

        impl TryFrom<String> for $name {
            type Error = TextIdentityError;

            fn try_from(text: String) -> Result<Self, Self::Error> {
                Self::from_hex(&text)
            }
        }
    };
}

identity!(
    /// The identity of one input as the client sent it; the same id means the same operation.
    OperationId
);
identity!(
    /// The identity of one fact in the knowledge store.
    FactId
);
identity!(
    /// An opaque reference to a payload specimen in the authorised diagnostic evidence store. It
    /// names a specimen and holds none of its text (RES36).
    EvidenceRef
);

macro_rules! number_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(u64);

        impl $name {
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}", self.0)
            }
        }
    };
}

number_id!(
    /// One running game: the shared state that input is routed through (D5).
    TableId
);
number_id!(
    /// One scene of a table: a place and the seats in it.
    SceneId
);
number_id!(
    /// A seat: one member of the session, as the authenticated session names it.
    SeatId
);
number_id!(
    /// The commit sequence of a table: input number N is the Nth the owner served.
    TurnSeq
);

/// Why a revision was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionError {
    ZeroEpoch,
    SequenceOverflow,
}

impl fmt::Display for RevisionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroEpoch => formatter.write_str("a recovery epoch is at least 1"),
            Self::SequenceOverflow => formatter.write_str("the revision sequence is exhausted"),
        }
    }
}

impl std::error::Error for RevisionError {}

/// The count of recoveries a session has been through; a result from an older epoch is late.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecoveryEpoch(NonZeroU64);

impl RecoveryEpoch {
    /// The epoch of a session that has never recovered.
    pub const FIRST: Self = Self(NonZeroU64::MIN);

    pub fn new(value: u64) -> Result<Self, RevisionError> {
        NonZeroU64::new(value)
            .map(Self)
            .ok_or(RevisionError::ZeroEpoch)
    }

    pub fn get(self) -> u64 {
        self.0.get()
    }
}

/// The revision of a session: ordered by epoch, then by the sequence within it (RES42).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SessionRevision {
    epoch: RecoveryEpoch,
    sequence: u64,
}

impl SessionRevision {
    pub fn new(epoch: RecoveryEpoch, sequence: u64) -> Self {
        Self { epoch, sequence }
    }

    pub fn epoch(self) -> RecoveryEpoch {
        self.epoch
    }

    pub fn sequence(self) -> u64 {
        self.sequence
    }

    /// Advances only within this epoch, refusing overflow rather than issuing an epoch.
    pub fn next_sequence(self) -> Result<Self, RevisionError> {
        let sequence = self
            .sequence
            .checked_add(1)
            .ok_or(RevisionError::SequenceOverflow)?;
        Ok(Self::new(self.epoch, sequence))
    }
}

/// What joins every piece of work back to the input that caused it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Correlation {
    pub operation_id: OperationId,
    pub turn_seq: TurnSeq,
    pub revision: SessionRevision,
}

/// Where an event happened: the table, and the scene and machine when there is one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableScope {
    pub table: TableId,
    pub scene: Option<SceneId>,
    pub machine: Option<MachineId>,
}

impl TableScope {
    /// A scope that names only the table, for input that is not yet routed.
    pub fn of_table(table: TableId) -> Self {
        Self {
            table,
            scene: None,
            machine: None,
        }
    }

    pub fn in_scene(mut self, scene: SceneId) -> Self {
        self.scene = Some(scene);
        self
    }

    pub fn in_machine(mut self, machine: MachineId) -> Self {
        self.machine = Some(machine);
        self
    }
}
