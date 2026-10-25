use std::fmt;

use crate::QueueError;

pub(crate) const MAX_SAMPLE_BYTES: usize = 16 * 1024 * 1024;

/// Local decoded interleaved f32 resource layout, not a wire codec or supported-device claim.
/// A qualified decoder supplies this layout after current source/rights/cue admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcmFormat {
    channels: u16,
    sample_rate: u32,
}

impl PcmFormat {
    pub fn new(channels: u16, sample_rate: u32) -> Result<Self, QueueError> {
        if channels == 0 || sample_rate == 0 {
            return Err(QueueError::InvalidFormat);
        }
        Ok(Self {
            channels,
            sample_rate,
        })
    }

    pub fn channels(self) -> u16 {
        self.channels
    }
    pub fn sample_rate(self) -> u32 {
        self.sample_rate
    }
}

/// Move-only owned decoded samples. Boxed slices have no uncounted spare capacity.
/// Construction checks a hard resource ceiling before scanning sample values.
pub struct PcmBuffer {
    format: PcmFormat,
    samples: Box<[f32]>,
}

pub struct PcmRefusal {
    pub reason: QueueError,
    pub samples: Box<[f32]>,
}

impl fmt::Debug for PcmRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PcmRefusal")
            .field("reason", &self.reason)
            .field("sample_count", &self.samples.len())
            .finish()
    }
}

impl fmt::Debug for PcmBuffer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PcmBuffer")
            .field("format", &self.format)
            .field("sample_count", &self.samples.len())
            .finish()
    }
}

impl PcmBuffer {
    pub fn new(format: PcmFormat, samples: Box<[f32]>) -> Result<Self, PcmRefusal> {
        let sample_bytes = samples.len().checked_mul(size_of::<f32>());
        let reason = if sample_bytes.is_none_or(|sample_bytes| sample_bytes > MAX_SAMPLE_BYTES) {
            Some(QueueError::SampleCapacity)
        } else if samples.is_empty()
            || !samples.len().is_multiple_of(usize::from(format.channels))
            || samples.iter().any(|sample| !sample.is_finite())
        {
            Some(QueueError::InvalidSamples)
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(PcmRefusal { reason, samples });
        }
        Ok(Self { format, samples })
    }

    pub fn format(&self) -> PcmFormat {
        self.format
    }
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }
    pub fn frames(&self) -> u64 {
        (self.samples.len() / usize::from(self.format.channels)) as u64
    }
    pub fn sample_bytes(&self) -> usize {
        self.samples.len() * size_of::<f32>()
    }
}
