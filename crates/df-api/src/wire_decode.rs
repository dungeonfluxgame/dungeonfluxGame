//! Bounded decoding of the currently generated common messages; no action schema is invented.
use crate::request::{self, RequestError, RequestField};
use df_model::checkpoint::{Basis, GameCommand, GameInput};
use df_protocol::common as wire;
use df_types::{
    BuildIdentity, BuildRevision, ClientBindingId, MemberId, OperationId, RecoveryEpoch, RunId,
    SessionId, SessionRevision,
};
use prost::Message;
use std::mem::size_of;

/// Caller-selected bounds, without implicit production values.
/// `maximum_wire_bytes` limits each encoded common message (or the aggregate command fields).
/// `maximum_owned_bytes` limits accepted simultaneous generated and domain ownership, including
/// inline representations and actual retained capacities. Input is borrowed. Generated decoding
/// can allocate before this retained-capacity check; the wire-byte check precedes decoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WireDecodeLimits {
    pub maximum_wire_bytes: usize,
    pub maximum_owned_bytes: usize,
}

impl WireDecodeLimits {
    /// A zero bound would refuse every message, so it is a caller error and not a limit.
    const fn has_zero_bound(self) -> bool {
        self.maximum_wire_bytes == 0 || self.maximum_owned_bytes == 0
    }
}

/// Safe classifications: no error retains malformed bytes, private text, or codec diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireDecodeError {
    /// A caller-supplied bound is zero.
    InvalidLimits,
    /// The encoded bytes exceed `maximum_wire_bytes`.
    WireCapacity,
    /// The decoded value would retain more than `maximum_owned_bytes`, or the final command more
    /// than the caller's `maximum_command_retained_bytes`.
    OwnedCapacity,
    /// The bytes are not a valid protobuf message of the named field.
    Malformed(RequestField),
    /// The message decoded but its fields are absent or invalid.
    Field(RequestError),
}

/// Heap bytes a generated identity message retains: the capacity of its byte vector.
fn identity_heap_bytes(value: Option<&Vec<u8>>) -> Option<usize> {
    Some(value.map_or(0, Vec::capacity))
}

/// For a message or domain value that owns no heap storage.
fn no_heap_bytes<T>(_: &T) -> Option<usize> {
    Some(0)
}

fn decode_field<M: Message + Default, T>(
    field: RequestField,
    encoded: &[u8],
    limits: WireDecodeLimits,
    wire_heap: fn(&M) -> Option<usize>,
    map: fn(&M) -> Result<T, RequestError>,
    domain_heap: fn(&T) -> Option<usize>,
) -> Result<T, WireDecodeError> {
    if limits.has_zero_bound() {
        return Err(WireDecodeError::InvalidLimits);
    }
    if encoded.len() > limits.maximum_wire_bytes {
        return Err(WireDecodeError::WireCapacity);
    }
    let inline = size_of::<M>()
        .checked_add(size_of::<T>())
        .ok_or(WireDecodeError::OwnedCapacity)?;
    if inline > limits.maximum_owned_bytes {
        return Err(WireDecodeError::OwnedCapacity);
    }
    // The encoded byte bound applies before prost allocates its generated field storage.
    // Protobuf syntax/presence and canonical domain validation remain separate refusals.
    let message = M::decode(encoded).map_err(|_| WireDecodeError::Malformed(field))?;
    let wire_retained = inline
        .checked_add(wire_heap(&message).ok_or(WireDecodeError::OwnedCapacity)?)
        .ok_or(WireDecodeError::OwnedCapacity)?;
    if wire_retained > limits.maximum_owned_bytes {
        return Err(WireDecodeError::OwnedCapacity);
    }
    let value = map(&message).map_err(WireDecodeError::Field)?;
    let retained = wire_retained
        .checked_add(domain_heap(&value).ok_or(WireDecodeError::OwnedCapacity)?)
        .ok_or(WireDecodeError::OwnedCapacity)?;
    if retained > limits.maximum_owned_bytes {
        return Err(WireDecodeError::OwnedCapacity);
    }
    Ok(value)
}

/// Decodes an actual generated SessionId message; parsing grants no access.
pub fn decode_session_id(
    encoded: &[u8],
    limits: WireDecodeLimits,
) -> Result<SessionId, WireDecodeError> {
    decode_field(
        RequestField::Session,
        encoded,
        limits,
        |message: &wire::SessionId| identity_heap_bytes(message.value.as_ref()),
        |message| request::session_id(Some(message)),
        no_heap_bytes,
    )
}

/// Decodes an actual generated MemberId message; the caller's member still comes from
/// authentication.
pub fn decode_member_id(
    encoded: &[u8],
    limits: WireDecodeLimits,
) -> Result<MemberId, WireDecodeError> {
    decode_field(
        RequestField::Member,
        encoded,
        limits,
        |message: &wire::MemberId| identity_heap_bytes(message.value.as_ref()),
        |message| request::member_id(Some(message)),
        no_heap_bytes,
    )
}

/// Decodes an actual generated ClientBindingId message without granting an input lease.
pub fn decode_client_binding_id(
    encoded: &[u8],
    limits: WireDecodeLimits,
) -> Result<ClientBindingId, WireDecodeError> {
    decode_field(
        RequestField::ClientBinding,
        encoded,
        limits,
        |message: &wire::ClientBindingId| identity_heap_bytes(message.value.as_ref()),
        |message| request::client_binding_id(Some(message)),
        no_heap_bytes,
    )
}

/// Decodes an actual generated RunId message.
pub fn decode_run_id(encoded: &[u8], limits: WireDecodeLimits) -> Result<RunId, WireDecodeError> {
    decode_field(
        RequestField::Run,
        encoded,
        limits,
        |message: &wire::RunId| identity_heap_bytes(message.value.as_ref()),
        |message| request::run_id(Some(message)),
        no_heap_bytes,
    )
}

/// Decodes an actual generated OperationId message; it performs no operation lookup.
pub fn decode_operation_id(
    encoded: &[u8],
    limits: WireDecodeLimits,
) -> Result<OperationId, WireDecodeError> {
    decode_field(
        RequestField::Operation,
        encoded,
        limits,
        |message: &wire::OperationId| identity_heap_bytes(message.value.as_ref()),
        |message| request::operation_id(Some(message)),
        no_heap_bytes,
    )
}

/// Decodes an actual generated RecoveryEpoch message; the value must be present and valid.
pub fn decode_recovery_epoch(
    encoded: &[u8],
    limits: WireDecodeLimits,
) -> Result<RecoveryEpoch, WireDecodeError> {
    decode_field(
        RequestField::Epoch,
        encoded,
        limits,
        no_heap_bytes::<wire::RecoveryEpoch>,
        |message| {
            let value = message
                .value
                .ok_or(RequestError::Missing(RequestField::Epoch))?;
            RecoveryEpoch::new(value).map_err(RequestError::Revision)
        },
        no_heap_bytes,
    )
}

/// Decodes an actual generated SessionRevision message with explicit epoch and sequence.
pub fn decode_session_revision(
    encoded: &[u8],
    limits: WireDecodeLimits,
) -> Result<SessionRevision, WireDecodeError> {
    decode_field(
        RequestField::Revision,
        encoded,
        limits,
        no_heap_bytes::<wire::SessionRevision>,
        |message| request::session_revision(Some(message)),
        no_heap_bytes,
    )
}

/// Decodes an actual generated BuildIdentity message; parsing does not approve that build.
pub fn decode_build_identity(
    encoded: &[u8],
    limits: WireDecodeLimits,
) -> Result<BuildIdentity, WireDecodeError> {
    decode_field(
        RequestField::Build,
        encoded,
        limits,
        build_identity_wire_heap_bytes,
        |message| request::build_identity(Some(message)),
        build_identity_heap_bytes,
    )
}

/// Heap bytes the generated provenance labels retain, or `None` on overflow.
fn build_identity_wire_heap_bytes(message: &wire::BuildIdentity) -> Option<usize> {
    [
        message.source_revision.as_ref(),
        message.native_revision.as_ref(),
        message.wasm_revision.as_ref(),
        message.configuration_revision.as_ref(),
        message.content_revision.as_ref(),
    ]
    .into_iter()
    .try_fold(0usize, |bytes, label| {
        bytes.checked_add(label.map_or(0, String::capacity))
    })
}

/// Heap bytes the parsed provenance labels retain, or `None` on overflow.
fn build_identity_heap_bytes(identity: &BuildIdentity) -> Option<usize> {
    [
        BuildRevision::Source,
        BuildRevision::Native,
        BuildRevision::Wasm,
        BuildRevision::Configuration,
        BuildRevision::Content,
    ]
    .into_iter()
    .try_fold(0usize, |bytes, label| {
        bytes.checked_add(identity.revision(label).retained_heap_bytes())
    })
}

/// Four separate currently generated common message bodies, collected by a caller.
/// This is not a protobuf request/envelope; no production action request exists in the schema.
pub struct EncodedCommandFields<'a> {
    pub session: Option<&'a [u8]>,
    pub run: Option<&'a [u8]>,
    pub operation: Option<&'a [u8]>,
    pub observed_revision: Option<&'a [u8]>,
}

fn required_field(value: Option<&[u8]>, field: RequestField) -> Result<&[u8], WireDecodeError> {
    value.ok_or(WireDecodeError::Field(RequestError::Missing(field)))
}

/// Decodes actual common message bytes into the canonical owned client command.
///
/// The caller supplies authenticated membership and an already-mapped closed GameCommand;
/// the unavailable production action DTO is not synthesized here. Native retry callers perform
/// operation lookup before state admission. This function performs no authentication, lease
/// issuance, mechanics, operation lookup, admission, or receipt commit. Internal inputs are
/// excluded by the GameCommand parameter. `maximum_command_retained_bytes` bounds the final
/// command's retained bytes, including unused owned storage.
pub fn map_encoded_game_command(
    fields: EncodedCommandFields<'_>,
    member: MemberId,
    command: GameCommand,
    limits: WireDecodeLimits,
    maximum_command_retained_bytes: usize,
) -> Result<GameInput, WireDecodeError> {
    if limits.has_zero_bound() || maximum_command_retained_bytes == 0 {
        return Err(WireDecodeError::InvalidLimits);
    }
    let session = required_field(fields.session, RequestField::Session)?;
    let run = required_field(fields.run, RequestField::Run)?;
    let operation = required_field(fields.operation, RequestField::Operation)?;
    let revision = required_field(fields.observed_revision, RequestField::Revision)?;
    let wire_bytes = [session.len(), run.len(), operation.len(), revision.len()]
        .into_iter()
        .try_fold(0usize, usize::checked_add)
        .ok_or(WireDecodeError::WireCapacity)?;
    if wire_bytes > limits.maximum_wire_bytes {
        return Err(WireDecodeError::WireCapacity);
    }
    let revision = decode_session_revision(revision, limits)?;
    let basis = Basis {
        session: decode_session_id(session, limits)?,
        run: decode_run_id(run, limits)?,
        revision,
    };
    let operation = decode_operation_id(operation, limits)?;
    let input = request::client_command_input(basis, operation, member, command);
    if input
        .retained_bytes()
        .ok_or(WireDecodeError::OwnedCapacity)?
        > maximum_command_retained_bytes
    {
        return Err(WireDecodeError::OwnedCapacity);
    }
    Ok(input)
}
