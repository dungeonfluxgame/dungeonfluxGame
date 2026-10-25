//! Sixteen-byte identifiers of the money core (planning/finance-ledger-research.md 29, D30).
//!
//! TODO(FIN-09): `df-types` is held by B; these move to `df_types::finance_ids` (F owns the file) when
//! its lock is released, and this module re-exports them so no caller changes.

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    text
}

/// The bytes a hex text stands for; `None` for a bad length or a non-hex digit.
pub fn unhex(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    bytes
        .chunks(2)
        .map(|pair| {
            let high = char::from(pair[0]).to_digit(16)?;
            let low = char::from(pair[1]).to_digit(16)?;
            u8::try_from(high * 16 + low).ok()
        })
        .collect()
}

macro_rules! finance_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; 16]);

        impl $name {
            pub const fn from_array(bytes: [u8; 16]) -> Self {
                Self(bytes)
            }

            /// `None` unless `bytes` is exactly sixteen bytes.
            pub fn from_slice(bytes: &[u8]) -> Option<Self> {
                bytes.try_into().ok().map(Self)
            }

            pub fn from_hex(text: &str) -> Option<Self> {
                Self::from_slice(&unhex(text)?)
            }

            pub fn as_bytes(&self) -> &[u8; 16] {
                &self.0
            }

            pub fn hex(&self) -> String {
                hex(&self.0)
            }
        }
    };
}

finance_id!(
    /// One source of credit for one payer: a purchase or a promotional grant.
    LotId
);
finance_id!(
    /// One payer's revocable authorization for one session.
    AllocationId
);
finance_id!(
    /// One durable per-operation reservation.
    ReservationId
);
finance_id!(
    /// One quote, usable once.
    QuoteId
);
finance_id!(
    /// One journal entry.
    EntryId
);
finance_id!(
    /// One committed finance event (entries plus row changes).
    EventId
);
finance_id!(
    /// The attempt id shared by the catalog, the hub and the money core (D22).
    AttemptId
);
finance_id!(
    /// One immutable published price version.
    PriceVersionId
);
finance_id!(
    /// One charge: a settled attempt or a priced meter segment.
    UsageEventId
);
finance_id!(
    /// An effect id of the game runtime.
    EffectId
);
finance_id!(
    /// A payer's account, as the money core keys it (B's account id, validated at creation).
    PayerId
);
finance_id!(
    /// A table session (O1's id, validated at creation).
    SessionKey
);
finance_id!(
    /// A campaign (S1's id, validated at creation).
    CampaignKey
);
finance_id!(
    /// A tenant (validated at creation).
    TenantKey
);
finance_id!(
    /// A caller-chosen operation id: a retry of the same operation is the same operation.
    OperationKey
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_refuses_bad_text() {
        let id = LotId::from_array([0xab; 16]);
        assert_eq!(id.hex(), "ab".repeat(16));
        assert_eq!(LotId::from_hex(&id.hex()), Some(id));
        assert_eq!(LotId::from_hex("abc"), None);
        assert_eq!(LotId::from_hex(&"zz".repeat(16)), None);
        assert_eq!(LotId::from_slice(&[1; 15]), None);
    }
}
