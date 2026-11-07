//! Why a file operation of a host feature failed, as a closed cause.

use std::fmt;
use std::io::ErrorKind;

use serde::{Deserialize, Serialize};

/// Why a file of the locate, persona, speech or imagegen feature could not be read or written, as
/// the reply carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IoCause {
    PermissionDenied,
    StorageFull,
    /// The bytes are not what they must be (a file that is a directory included).
    InvalidData,
    Other,
}

impl IoCause {
    pub const ALL: [Self; 4] = [
        Self::PermissionDenied,
        Self::StorageFull,
        Self::InvalidData,
        Self::Other,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::PermissionDenied => "permission_denied",
            Self::StorageFull => "storage_full",
            Self::InvalidData => "invalid_data",
            Self::Other => "other",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }

    pub fn from_kind(kind: ErrorKind) -> Self {
        match kind {
            ErrorKind::PermissionDenied => Self::PermissionDenied,
            ErrorKind::StorageFull => Self::StorageFull,
            ErrorKind::InvalidData | ErrorKind::IsADirectory => Self::InvalidData,
            _ => Self::Other,
        }
    }
}

impl fmt::Display for IoCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PermissionDenied => "permission denied",
            Self::StorageFull => "the disk is full",
            Self::InvalidData => "it is not a readable file",
            Self::Other => "the file system refused it",
        })
    }
}
