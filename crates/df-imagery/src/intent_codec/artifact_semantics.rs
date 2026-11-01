//! Campaign admission semantics exclude only typed transport authority, never provider text.
use super::{CodecError, DecodedIntent, primitives::Writer};
use crate::{VersionedSnapshot, request::ExecutionMode};

const MAGIC: &[u8; 8] = b"DFARTP01";
const REVISION: u32 = 1;

pub(crate) fn encode(
    intent: &DecodedIntent,
    config: &VersionedSnapshot,
    executor: &VersionedSnapshot,
    maximum_bytes: usize,
) -> Result<VersionedSnapshot, CodecError> {
    let artwork = intent.operation.artwork().ok_or(CodecError::Malformed)?;
    let mut output = Writer::new(maximum_bytes);
    output.extend(MAGIC)?;
    output.push(u8::try_from(intent.purpose.purpose.index()).map_err(|_| CodecError::TooLong)?)?;
    output.push(super::encode::role_tag(intent.purpose.role))?;
    output.push(super::encode::operation_tag(&intent.operation))?;
    super::encode::encode_operation(
        &mut output,
        &intent.operation,
        &crate::service::PROTOCOL_LIMITS,
    )?;
    super::encode::encode_execution(&mut output, ExecutionMode::Live)?;
    super::encode::encode_freshness(&mut output, &intent.freshness)?;
    super::encode::encode_audience(&mut output, &intent.audience)?;
    output.u64_be(intent.expected_source_revision)?;
    super::encode::encode_output(&mut output, &intent.output)?;
    let pipeline = intent.operation.pipeline();
    output.boolean(pipeline.is_some())?;
    if let Some(pipeline) = pipeline {
        super::pipeline::encode(&mut output, pipeline)?;
    }
    super::artwork::encode_spec(&mut output, &artwork.spec)?;
    output.u32_be(config.revision())?;
    output.counted(config.bytes())?;
    output.u32_be(executor.revision())?;
    output.counted(executor.bytes())?;
    VersionedSnapshot::new(REVISION, output.finish(), maximum_bytes)
        .map_err(|_| CodecError::TooLong)
}
