//! Identities local to the campaign service. They are opaque, nonzero and distinct types, like
//! the shared ones in `df-types`; they grant nothing.

use crate::secrets::{RandomUnavailable, hex, random_identity};

macro_rules! local_identity {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; 16]);

        impl $name {
            pub fn generate() -> Result<Self, RandomUnavailable> {
                random_identity().map(Self)
            }

            /// Accepts exactly 16 bytes with at least one nonzero byte.
            pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
                let value: [u8; 16] = bytes.try_into().ok()?;
                (value != [0; 16]).then_some(Self(value))
            }

            pub fn as_bytes(&self) -> &[u8; 16] {
                &self.0
            }

            pub fn text(&self) -> String {
                hex(&self.0)
            }

            /// Parses 32 lowercase hexadecimal digits.
            pub fn parse(text: &str) -> Option<Self> {
                let bytes = crate::secrets::unhex(text)?;
                Self::from_bytes(&bytes)
            }
        }
    };
}

local_identity!(InvitationId);
local_identity!(RequestId);
local_identity!(ProposalId);
local_identity!(PairingId);
local_identity!(TicketId);
// The 128-bit epoch of a campaign event feed; random, never a counter.
local_identity!(FeedEpoch);

/// Operation receipts and the ids that refer to them are good for ninety days.
pub const OPERATION_RETENTION_MS: u64 = 90 * 24 * 3_600_000;

/// The issue time an operation id carries: the first eight bytes, big-endian milliseconds, in
/// the layout B's form ids use. Receipts are purged after the retention window, so the age of an
/// id is read from the id and never from a receipt row.
pub fn operation_issued_at_ms(operation: &df_types::OperationId) -> u64 {
    let mut time = [0_u8; 8];
    time.copy_from_slice(&operation.as_bytes()[..8]);
    u64::from_be_bytes(time)
}

/// A fresh operation id carrying `now_ms`, in the same layout (for tests and for servers that
/// issue ids themselves).
pub fn new_operation_id(now_ms: u64) -> Result<df_types::OperationId, RandomUnavailable> {
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&now_ms.to_be_bytes());
    bytes[8..].copy_from_slice(&crate::secrets::random_bytes::<8>()?);
    bytes[15] |= 1;
    df_types::OperationId::from_bytes(&bytes).map_err(|_| RandomUnavailable)
}

/// Lowercase hexadecimal text of an id or digest (for audit lines and wire bodies).
pub use crate::secrets::hex as hex_of;
