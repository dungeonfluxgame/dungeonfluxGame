/// Rejection of a supplied canonical identity. Identities are not credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityError {
    InvalidLength { actual: usize },
    Zero,
}

/// Rejection of a supplied canonical lowercase hexadecimal identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextIdentityError {
    InvalidLength { actual: usize },
    Malformed,
    Zero,
}

fn parse_canonical_hex(text: &str) -> Result<[u8; 16], TextIdentityError> {
    let encoded = text.as_bytes();
    if encoded.len() != 32 {
        return Err(TextIdentityError::InvalidLength {
            actual: encoded.len(),
        });
    }

    let mut bytes = [0_u8; 16];
    for index in 0..16 {
        let high = hex_nibble(encoded[index * 2]).ok_or(TextIdentityError::Malformed)?;
        let low = hex_nibble(encoded[index * 2 + 1]).ok_or(TextIdentityError::Malformed)?;
        bytes[index] = (high << 4) | low;
    }
    if bytes == [0; 16] {
        return Err(TextIdentityError::Zero);
    }
    Ok(bytes)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

// The distinct contracts share canonical byte validation, not interchangeability.
macro_rules! identity {
    ($name:ident) => {
        /// An opaque, caller-supplied nonzero 128-bit identity in canonical byte order.
        /// This type neither generates values nor establishes access or uniqueness.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; 16]);

        impl $name {
            /// Parses exactly 32 lowercase ASCII hexadecimal digits in canonical byte order.
            pub fn from_hex(text: &str) -> Result<Self, TextIdentityError> {
                parse_canonical_hex(text).map(Self)
            }

            /// Accepts exactly 16 bytes with at least one nonzero byte.
            pub fn from_bytes(bytes: &[u8]) -> Result<Self, IdentityError> {
                let value: [u8; 16] =
                    bytes.try_into().map_err(|_| IdentityError::InvalidLength {
                        actual: bytes.len(),
                    })?;
                if value == [0; 16] {
                    return Err(IdentityError::Zero);
                }
                Ok(Self(value))
            }

            /// Returns the supplied canonical bytes without reordering.
            pub fn as_bytes(&self) -> &[u8; 16] {
                &self.0
            }
        }
    };
}
identity!(SessionId);
identity!(MemberId);
identity!(ClientBindingId);
identity!(RunId);
identity!(OperationId);
identity!(SubscriptionId);
identity!(PaidInvoiceId);
identity!(PriceVersion);
identity!(AccountId);
identity!(TenantId);
identity!(CampaignId);
identity!(WebSessionId);
identity!(CheckoutId);
identity!(OperatorId);
