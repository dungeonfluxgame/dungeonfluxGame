//! Byte primitives for the intent codec: the bounded writer, the bounded
//! reader, the prompt length bound and the minimum wire widths that let the
//! decoder prove a claimed vector count fits the remaining input before it
//! reserves anything.

use super::CodecError;
use crate::request::RequestLimits;

/// Minimum encoded width of one composite layer: a full source reference, two
/// coordinates, one opacity and one blend tag.
pub(super) const MIN_LAYER_BYTES: usize = 60;
/// Minimum encoded width of one member id.
pub(super) const MIN_MEMBER_BYTES: usize = 16;
/// Minimum encoded width of one source reference: asset, digest and one
/// audience tag.
pub(super) const MIN_SOURCE_REF_BYTES: usize = 49;
/// Minimum encoded width of one transform step: a tag plus one payload byte,
/// reached by a flip or a grayscale effect.
pub(super) const MIN_STEP_BYTES: usize = 2;
/// Minimum encoded width of one named output profile: a one-character name
/// plus the smallest provider output (tag, generated dims, delivered dims,
/// alpha tag and encoding tag).
pub(super) const MIN_PROFILE_BYTES: usize = 24;

/// Byte bound for one prompt under the injected limits: four UTF-8 bytes per
/// admitted char.
pub(crate) fn prompt_byte_bound(limits: &RequestLimits) -> usize {
    limits.max_prompt_chars.saturating_mul(4)
}

pub(crate) struct Writer {
    out: Vec<u8>,
    max_bytes: usize,
}

impl Writer {
    pub(crate) fn new(max_bytes: usize) -> Self {
        Self {
            out: Vec::new(),
            max_bytes,
        }
    }

    pub(crate) fn remaining(&self) -> usize {
        // Every mutation passes through reserve, so the length never passes the bound.
        self.max_bytes - self.out.len()
    }

    fn reserve(&self, extra: usize) -> Result<(), CodecError> {
        let need = self
            .out
            .len()
            .checked_add(extra)
            .ok_or(CodecError::TooLong)?;
        if need > self.max_bytes {
            return Err(CodecError::TooLong);
        }
        Ok(())
    }

    pub(crate) fn push(&mut self, byte: u8) -> Result<(), CodecError> {
        self.reserve(1)?;
        self.out.push(byte);
        Ok(())
    }

    pub(crate) fn extend(&mut self, bytes: &[u8]) -> Result<(), CodecError> {
        self.reserve(bytes.len())?;
        self.out.extend_from_slice(bytes);
        Ok(())
    }

    pub(crate) fn u16_be(&mut self, value: u16) -> Result<(), CodecError> {
        self.extend(&value.to_be_bytes())
    }

    pub(crate) fn u32_be(&mut self, value: u32) -> Result<(), CodecError> {
        self.extend(&value.to_be_bytes())
    }

    pub(crate) fn i32_be(&mut self, value: i32) -> Result<(), CodecError> {
        self.extend(&value.to_be_bytes())
    }

    pub(crate) fn u64_be(&mut self, value: u64) -> Result<(), CodecError> {
        self.extend(&value.to_be_bytes())
    }

    pub(crate) fn counted(&mut self, bytes: &[u8]) -> Result<(), CodecError> {
        let len = u32::try_from(bytes.len()).map_err(|_| CodecError::TooLong)?;
        self.u32_be(len)?;
        self.extend(bytes)
    }

    pub(crate) fn float(&mut self, value: f32, field: &'static str) -> Result<(), CodecError> {
        if !value.is_finite() {
            return Err(CodecError::NonFinite { field });
        }
        let bits = if value == 0.0 { 0_u32 } else { value.to_bits() };
        self.u32_be(bits)
    }

    pub(crate) fn boolean(&mut self, value: bool) -> Result<(), CodecError> {
        self.push(u8::from(value))
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.out
    }
}

pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    pub(crate) fn take(&mut self, len: usize) -> Result<&'a [u8], CodecError> {
        let end = self.pos.checked_add(len).ok_or(CodecError::Truncated)?;
        if end > self.bytes.len() {
            return Err(CodecError::Truncated);
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    pub(crate) fn byte(&mut self) -> Result<u8, CodecError> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn u16_be(&mut self) -> Result<u16, CodecError> {
        Ok(u16::from_be_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| CodecError::Truncated)?,
        ))
    }

    pub(crate) fn u32_be(&mut self) -> Result<u32, CodecError> {
        Ok(u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| CodecError::Truncated)?,
        ))
    }

    pub(crate) fn i32_be(&mut self) -> Result<i32, CodecError> {
        Ok(i32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| CodecError::Truncated)?,
        ))
    }

    pub(crate) fn u64_be(&mut self) -> Result<u64, CodecError> {
        Ok(u64::from_be_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| CodecError::Truncated)?,
        ))
    }

    pub(crate) fn counted(&mut self, bound_bytes: usize) -> Result<&'a [u8], CodecError> {
        let len = usize::try_from(self.u32_be()?).map_err(|_| CodecError::TooLong)?;
        if len > bound_bytes {
            return Err(CodecError::TooLong);
        }
        self.take(len)
    }

    pub(crate) fn float(&mut self, field: &'static str) -> Result<f32, CodecError> {
        let bits = self.u32_be()?;
        if bits == 0x8000_0000 {
            return Ok(0.0);
        }
        let value = f32::from_bits(bits);
        if !value.is_finite() {
            return Err(CodecError::NonFinite { field });
        }
        Ok(value)
    }

    pub(crate) fn boolean(&mut self) -> Result<bool, CodecError> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(CodecError::Malformed),
        }
    }

    pub(crate) fn finish(&self) -> Result<(), CodecError> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err(CodecError::Trailing {
                extra_bytes: self.bytes.len() - self.pos,
            })
        }
    }
}
