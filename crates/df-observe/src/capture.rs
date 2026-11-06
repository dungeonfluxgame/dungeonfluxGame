//! Shared capture identities; these values grant no authority.
use df_types::BuildRevision;
use std::fmt;

/// Resource schema URL of every native and browser capture.
pub(crate) const RESOURCE_SCHEMA_URL: &str = "https://dungeonflux.local/telemetry/native-v1";
/// Instrumentation scope schema URL of every native and browser capture.
pub(crate) const SCOPE_SCHEMA_URL: &str = "https://dungeonflux.local/telemetry/scope-v1";
/// Version of the instrumentation scopes this crate names.
pub(crate) const SCOPE_VERSION: &str = "0.1.0";
/// Resource attribute name for each build revision, in the order captures write them.
pub(crate) const BUILD_REVISION_ATTRIBUTES: [(&str, BuildRevision); 5] = [
    ("df.build", BuildRevision::Source),
    ("df.native", BuildRevision::Native),
    ("df.wasm", BuildRevision::Wasm),
    ("df.configuration", BuildRevision::Configuration),
    ("df.content", BuildRevision::Content),
];
/// Capture identity attributes the shared path writes itself; callers may never supply them.
pub(crate) const PRODUCER_ID_ATTRIBUTE: &str = "df.producer_id";
pub(crate) const RECORD_ID_ATTRIBUTE: &str = "df.record_id";
pub(crate) const SOURCE_SEQUENCE_ATTRIBUTE: &str = "df.source_sequence";

/// True for an attribute name reserved for the capture identity.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn is_capture_identity_attribute(name: &str) -> bool {
    [
        PRODUCER_ID_ATTRIBUTE,
        RECORD_ID_ATTRIBUTE,
        SOURCE_SEQUENCE_ATTRIBUTE,
    ]
    .contains(&name)
}

/// Caller-owned source lifecycle identity; uniqueness across lifetimes is a caller duty.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ProducerId([u8; 16]);
/// Stable record identity captured before SDK dispatch and retained for retries.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RecordId([u8; 16]);
macro_rules! identity {
    ($name:ident) => {
        impl $name {
            /// Refuse the all-zero identity, which no producer may use.
            pub fn new(bytes: [u8; 16]) -> Result<Self, TelemetryError> {
                if bytes == [0; 16] {
                    return Err(TelemetryError::InvalidIdentity);
                }
                Ok(Self(bytes))
            }
            pub fn bytes(self) -> [u8; 16] {
                self.0
            }
            pub fn hex(self) -> String {
                lower_hex(&self.0)
            }
            /// Parse exactly 32 lower-case hex digits, the spelling `hex` writes.
            pub fn from_hex(text: &str) -> Result<Self, TelemetryError> {
                if text.len() != 32 || !is_lower_hex(text) {
                    return Err(TelemetryError::InvalidIdentity);
                }
                let mut bytes = [0; 16];
                for (out, pair) in bytes.iter_mut().zip(text.as_bytes().chunks_exact(2)) {
                    let value =
                        std::str::from_utf8(pair).map_err(|_| TelemetryError::InvalidIdentity)?;
                    *out = u8::from_str_radix(value, 16)
                        .map_err(|_| TelemetryError::InvalidIdentity)?;
                }
                Self::new(bytes)
            }
        }
    };
}
identity!(ProducerId);
identity!(RecordId);

/// Lower-case hexadecimal text of `bytes`, the one spelling every diagnostic identity uses.
pub(crate) fn lower_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // Writing into a `String` cannot fail.
        let _infallible = write!(text, "{byte:02x}");
    }
    text
}

/// True when `text` is only lower-case hexadecimal digits, as identities are written.
pub(crate) fn is_lower_hex(text: &str) -> bool {
    text.bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Positive per-producer record sequence; a missing value is a visible source gap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceSequence(i64);
impl SourceSequence {
    /// Refuse zero and negative sequences.
    pub fn new(value: i64) -> Result<Self, TelemetryError> {
        if value <= 0 {
            return Err(TelemetryError::InvalidIdentity);
        }
        Ok(Self(value))
    }
    pub fn get(self) -> i64 {
        self.0
    }
}
/// Full identity of one captured record: retry identity plus ordering sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureKey {
    pub producer: ProducerId,
    pub record: RecordId,
    pub sequence: SourceSequence,
}
/// The OTLP signal a captured batch carries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Signal {
    Logs,
    Spans,
}

/// Typed capture, ingress and storage outcomes; none carries private payload text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TelemetryError {
    InvalidIdentity,
    SequenceExhausted,
    InvalidLimits,
    Malformed,
    Oversized,
    Capacity,
    Closed,
    Deadline,
    Conflict,
    Corrupt,
    ForeignRoot,
    Ownership,
    Io,
    SinkUnavailable,
    Internal,
}
impl fmt::Display for TelemetryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}
impl std::error::Error for TelemetryError {}
