//! Reads an add request while it arrives: the JSON object `{"name": ..., "data_base64": ...}` is
//! parsed byte by byte as the body comes in, and the file's base64 is decoded into a writer in
//! pieces, so the host never holds the request or the file in memory. It checks the name as soon
//! as it is read and stops as soon as the decoded size or the request passes its limit. It never
//! touches the library: the caller owns the writer.
//!
//! Only the two keys of `MediaAdd` are taken, in either order; an unknown or repeated key is
//! refused, as `deny_unknown_fields` does for the other routes. The base64 string must not use
//! JSON escapes (there is nothing to escape in base64).

use std::fmt;
use std::io::Write;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use df_playground::media::media_kind;

use crate::limits::{MAXIMUM_UPLOAD_KEY_BYTES, MAXIMUM_UPLOAD_NAME_BYTES};
use crate::media_ingest::{IngestError, MediaLimits};

const NAME_KEY: &[u8] = b"name";
const DATA_KEY: &[u8] = b"data_base64";
const BASE64_GROUP_BYTES: usize = 4;

/// What is wrong with the shape of an add request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnvelopeCause {
    UnexpectedCharacter,
    UnknownField,
    DuplicateField,
    MissingField,
    NameTooLong,
    InvalidNameText,
    EndedEarly,
}

impl fmt::Display for EnvelopeCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnexpectedCharacter => {
                "it must be a JSON object with a string name and a string data_base64"
            }
            Self::UnknownField => "it has a field other than name and data_base64",
            Self::DuplicateField => "it names a field twice",
            Self::MissingField => "it lacks the name or the data_base64",
            Self::NameTooLong => "the name is far longer than a media name can be",
            Self::InvalidNameText => "the name is not valid JSON text",
            Self::EndedEarly => "the request ended before the object was complete",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Data,
}

#[derive(Debug)]
enum State {
    Start,
    FirstKey,
    NextKey,
    Key(Vec<u8>),
    Colon(Field),
    ValueStart(Field),
    Name { raw: Vec<u8>, escaped: bool },
    Data,
    AfterValue,
    Done,
}

/// A request that has been read to its end.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Envelope {
    pub(crate) name: String,
    /// Bytes of the decoded file.
    pub(crate) file_bytes: u64,
}

/// The state of one request being read.
pub(crate) struct EnvelopeParser {
    state: State,
    name: Option<String>,
    saw_data: bool,
    /// Base64 characters not yet decoded: less than a group, or the final padded group.
    carry: Vec<u8>,
    decoded: Vec<u8>,
    file_bytes: u64,
    request_bytes: u64,
    maximum_file_bytes: u64,
    maximum_request_bytes: u64,
}

fn is_json_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

fn unexpected() -> IngestError {
    IngestError::InvalidRequest(EnvelopeCause::UnexpectedCharacter)
}

fn malformed(problem: EnvelopeCause) -> IngestError {
    IngestError::InvalidRequest(problem)
}

impl EnvelopeParser {
    pub(crate) fn new(limits: &MediaLimits) -> Self {
        Self {
            state: State::Start,
            name: None,
            saw_data: false,
            carry: Vec::new(),
            decoded: Vec::new(),
            file_bytes: 0,
            request_bytes: 0,
            maximum_file_bytes: limits.maximum_file_bytes,
            maximum_request_bytes: limits.maximum_upload_body_bytes(),
        }
    }

    /// Takes the next piece of the request, writing the file bytes it completes to `out`.
    pub(crate) fn feed(&mut self, bytes: &[u8], out: &mut impl Write) -> Result<(), IngestError> {
        let received = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        self.request_bytes = self.request_bytes.saturating_add(received);
        if self.request_bytes > self.maximum_request_bytes {
            return Err(self.too_large());
        }
        let mut rest = bytes;
        while !rest.is_empty() {
            if matches!(self.state, State::Data) {
                let end = rest
                    .iter()
                    .position(|&byte| byte == b'"')
                    .unwrap_or(rest.len());
                // `end` is a position in `rest` or its length, so the split cannot panic.
                let (piece, after) = rest.split_at(end);
                self.push_data(piece, out)?;
                rest = after;
                if let Some((_, tail)) = rest.split_first() {
                    self.finish_data(out)?;
                    self.state = State::AfterValue;
                    rest = tail;
                }
            } else if let Some((&byte, tail)) = rest.split_first() {
                self.step(byte)?;
                rest = tail;
            }
        }
        Ok(())
    }

    /// The request after its last piece: it must be a complete object with both fields.
    pub(crate) fn finish(self) -> Result<Envelope, IngestError> {
        if !matches!(self.state, State::Done) {
            return Err(malformed(EnvelopeCause::EndedEarly));
        }
        match self.name {
            Some(name) if self.saw_data => Ok(Envelope {
                name,
                file_bytes: self.file_bytes,
            }),
            _ => Err(malformed(EnvelopeCause::MissingField)),
        }
    }

    fn too_large(&self) -> IngestError {
        IngestError::TooLarge {
            maximum_bytes: self.maximum_file_bytes,
        }
    }

    fn step(&mut self, byte: u8) -> Result<(), IngestError> {
        let state = std::mem::replace(&mut self.state, State::Done);
        self.state = match state {
            State::Start if is_json_space(byte) => State::Start,
            State::Start if byte == b'{' => State::FirstKey,
            State::FirstKey if is_json_space(byte) => State::FirstKey,
            State::FirstKey if byte == b'}' => State::Done,
            State::FirstKey | State::NextKey if byte == b'"' => State::Key(Vec::new()),
            State::NextKey if is_json_space(byte) => State::NextKey,
            State::Key(key) => self.read_key(key, byte)?,
            State::Colon(field) if is_json_space(byte) => State::Colon(field),
            State::Colon(field) if byte == b':' => State::ValueStart(field),
            State::ValueStart(field) if is_json_space(byte) => State::ValueStart(field),
            State::ValueStart(Field::Name) if byte == b'"' => State::Name {
                raw: Vec::new(),
                escaped: false,
            },
            State::ValueStart(Field::Data) if byte == b'"' => {
                self.saw_data = true;
                State::Data
            }
            State::Name { raw, escaped } => self.read_name(raw, escaped, byte)?,
            State::AfterValue if is_json_space(byte) => State::AfterValue,
            State::AfterValue if byte == b',' => State::NextKey,
            State::AfterValue if byte == b'}' => State::Done,
            State::Done if is_json_space(byte) => State::Done,
            _ => return Err(unexpected()),
        };
        Ok(())
    }

    fn read_key(&mut self, mut key: Vec<u8>, byte: u8) -> Result<State, IngestError> {
        if byte != b'"' {
            if key.len() >= MAXIMUM_UPLOAD_KEY_BYTES {
                return Err(malformed(EnvelopeCause::UnknownField));
            }
            key.push(byte);
            return Ok(State::Key(key));
        }
        let field = if key == NAME_KEY {
            Field::Name
        } else if key == DATA_KEY {
            Field::Data
        } else {
            return Err(malformed(EnvelopeCause::UnknownField));
        };
        let repeated = match field {
            Field::Name => self.name.is_some(),
            Field::Data => self.saw_data,
        };
        if repeated {
            return Err(malformed(EnvelopeCause::DuplicateField));
        }
        Ok(State::Colon(field))
    }

    fn read_name(
        &mut self,
        mut raw: Vec<u8>,
        escaped: bool,
        byte: u8,
    ) -> Result<State, IngestError> {
        if !escaped && byte == b'"' {
            self.name = Some(parse_name(&raw)?);
            return Ok(State::AfterValue);
        }
        if raw.len() >= MAXIMUM_UPLOAD_NAME_BYTES {
            return Err(malformed(EnvelopeCause::NameTooLong));
        }
        raw.push(byte);
        Ok(State::Name {
            raw,
            escaped: !escaped && byte == b'\\',
        })
    }

    /// Decodes the whole groups in the carry, up to a group with padding (which must be the last).
    fn push_data(&mut self, piece: &[u8], out: &mut impl Write) -> Result<(), IngestError> {
        self.carry.extend_from_slice(piece);
        let padding = self.carry.iter().position(|&byte| byte == b'=');
        let usable = padding.unwrap_or(self.carry.len());
        let usable = usable - usable % BASE64_GROUP_BYTES;
        if padding.is_some() && self.carry.len() - usable > BASE64_GROUP_BYTES {
            return Err(IngestError::InvalidBase64);
        }
        self.decode_front(usable, out)
    }

    /// The closing quote of the data: what is left must be one complete group.
    fn finish_data(&mut self, out: &mut impl Write) -> Result<(), IngestError> {
        if !self.carry.len().is_multiple_of(BASE64_GROUP_BYTES) {
            return Err(IngestError::InvalidBase64);
        }
        self.decode_front(self.carry.len(), out)
    }

    fn decode_front(&mut self, count: usize, out: &mut impl Write) -> Result<(), IngestError> {
        if count == 0 {
            return Ok(());
        }
        self.decoded.clear();
        let front = self.carry.get(..count).ok_or(IngestError::InvalidBase64)?;
        STANDARD
            .decode_vec(front, &mut self.decoded)
            .map_err(|_| IngestError::InvalidBase64)?;
        self.carry.drain(..count);
        let written = u64::try_from(self.decoded.len()).unwrap_or(u64::MAX);
        self.file_bytes = self.file_bytes.saturating_add(written);
        if self.file_bytes > self.maximum_file_bytes {
            return Err(self.too_large());
        }
        out.write_all(&self.decoded)
            .map_err(|error| IngestError::FileFailed(error.kind()))
    }
}

/// The name from its JSON string text (escapes included), checked to be a media name.
fn parse_name(raw: &[u8]) -> Result<String, IngestError> {
    let text = std::str::from_utf8(raw).map_err(|_| malformed(EnvelopeCause::InvalidNameText))?;
    let name: String = serde_json::from_str(&format!("\"{text}\""))
        .map_err(|_| malformed(EnvelopeCause::InvalidNameText))?;
    if media_kind(&name).is_none() {
        return Err(IngestError::InvalidMediaName { name });
    }
    Ok(name)
}
