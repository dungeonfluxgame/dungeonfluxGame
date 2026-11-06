/// Length in bytes of every canonical identity.
const IDENTITY_BYTES: usize = 16;

/// Length in ASCII digits of the canonical lowercase hexadecimal spelling.
const IDENTITY_HEX_DIGITS: usize = IDENTITY_BYTES * 2;

/// Rejection of a supplied canonical identity. Identities are not credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityError {
    /// The input was not exactly 16 bytes; `actual` is its length in bytes.
    InvalidLength { actual: usize },
    /// Every byte was zero, which no identity may be.
    Zero,
}

/// Rejection of a supplied canonical lowercase hexadecimal identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextIdentityError {
    /// The input was not exactly 32 bytes; `actual` is its length in UTF-8 bytes.
    InvalidLength { actual: usize },
    /// A byte was not a lowercase ASCII hexadecimal digit.
    Malformed,
    /// Every decoded byte was zero, which no identity may be.
    Zero,
}

fn parse_canonical_hex(text: &str) -> Result<[u8; IDENTITY_BYTES], TextIdentityError> {
    let encoded = text.as_bytes();
    if encoded.len() != IDENTITY_HEX_DIGITS {
        return Err(TextIdentityError::InvalidLength {
            actual: encoded.len(),
        });
    }

    let mut bytes = [0_u8; IDENTITY_BYTES];
    let (digit_pairs, _) = encoded.as_chunks::<2>();
    for (byte, [high, low]) in bytes.iter_mut().zip(digit_pairs) {
        let high = hex_nibble(*high).ok_or(TextIdentityError::Malformed)?;
        let low = hex_nibble(*low).ok_or(TextIdentityError::Malformed)?;
        *byte = (high << 4) | low;
    }
    if bytes == [0; IDENTITY_BYTES] {
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
        pub struct $name([u8; IDENTITY_BYTES]);

        impl $name {
            /// Parses exactly 32 lowercase ASCII hexadecimal digits in canonical byte order.
            pub fn from_hex(text: &str) -> Result<Self, TextIdentityError> {
                parse_canonical_hex(text).map(Self)
            }

            /// Accepts exactly 16 bytes with at least one nonzero byte.
            pub fn from_bytes(bytes: &[u8]) -> Result<Self, IdentityError> {
                let value: [u8; IDENTITY_BYTES] =
                    bytes.try_into().map_err(|_| IdentityError::InvalidLength {
                        actual: bytes.len(),
                    })?;
                if value == [0; IDENTITY_BYTES] {
                    return Err(IdentityError::Zero);
                }
                Ok(Self(value))
            }

            /// Returns the supplied canonical bytes without reordering.
            pub fn as_bytes(&self) -> &[u8; IDENTITY_BYTES] {
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
