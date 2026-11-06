use df_model::checkpoint::{
    Basis, Checkpoint, CommandInput, GameCommand, GameInput, ReferenceInventory,
};
use df_model::commands::{CommandError, CommandLimits, validate_client_command};
use df_protocol::common as wire;
use df_types::{
    BuildIdentity, BuildIdentityError, ClientBindingId, IdentityError, MemberId, OperationId,
    RecoveryEpoch, RevisionError, RunId, SessionId, SessionRevision,
};

/// Names absent or invalid fields without retaining request bytes or text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestField {
    Session,
    Member,
    ClientBinding,
    Run,
    Operation,
    Revision,
    Epoch,
    Sequence,
    Build,
}

/// Malformed common fields remain distinct from structural domain refusals.
/// Neither outcome is a durable game decision or an RPC receipt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestError {
    /// A required field is absent.
    Missing(RequestField),
    /// An identity field is present but its bytes are not a valid identity.
    Identity {
        field: RequestField,
        cause: IdentityError,
    },
    /// The recovery epoch is present but not a valid epoch.
    Revision(RevisionError),
    /// A build provenance label is present but malformed.
    Build(BuildIdentityError),
    /// The fields are well formed and the closed command is structurally refused.
    Domain(CommandError),
}

/// Parses the bytes of one identity field, naming `field` when they are absent or malformed.
fn parse_identity<T>(
    value: Option<&[u8]>,
    field: RequestField,
    from_bytes: fn(&[u8]) -> Result<T, IdentityError>,
) -> Result<T, RequestError> {
    let bytes = value.ok_or(RequestError::Missing(field))?;
    from_bytes(bytes).map_err(|cause| RequestError::Identity { field, cause })
}

/// Parses a supplied identity; a well-formed value grants no session access.
pub fn session_id(value: Option<&wire::SessionId>) -> Result<SessionId, RequestError> {
    let bytes = value.and_then(|id| id.value.as_deref());
    parse_identity(bytes, RequestField::Session, SessionId::from_bytes)
}

/// Parses a supplied identity; the caller's member must still come from authentication.
pub fn member_id(value: Option<&wire::MemberId>) -> Result<MemberId, RequestError> {
    let bytes = value.and_then(|id| id.value.as_deref());
    parse_identity(bytes, RequestField::Member, MemberId::from_bytes)
}

/// Parses an identity without granting or validating an input lease.
pub fn client_binding_id(
    value: Option<&wire::ClientBindingId>,
) -> Result<ClientBindingId, RequestError> {
    let bytes = value.and_then(|id| id.value.as_deref());
    parse_identity(
        bytes,
        RequestField::ClientBinding,
        ClientBindingId::from_bytes,
    )
}

/// Parses a supplied run identity; it does not check that the run is the session's current one.
pub fn run_id(value: Option<&wire::RunId>) -> Result<RunId, RequestError> {
    let bytes = value.and_then(|id| id.value.as_deref());
    parse_identity(bytes, RequestField::Run, RunId::from_bytes)
}

/// Parses the supplied retry identity; it does not perform operation lookup or deduplication.
pub fn operation_id(value: Option<&wire::OperationId>) -> Result<OperationId, RequestError> {
    let bytes = value.and_then(|id| id.value.as_deref());
    parse_identity(bytes, RequestField::Operation, OperationId::from_bytes)
}

/// Requires explicit epoch and sequence presence. Sequence zero remains a valid value.
pub fn session_revision(
    value: Option<&wire::SessionRevision>,
) -> Result<SessionRevision, RequestError> {
    let value = value.ok_or(RequestError::Missing(RequestField::Revision))?;
    let epoch = value
        .epoch
        .as_ref()
        .and_then(|epoch| epoch.value)
        .ok_or(RequestError::Missing(RequestField::Epoch))?;
    let sequence = value
        .sequence
        .ok_or(RequestError::Missing(RequestField::Sequence))?;
    let epoch = RecoveryEpoch::new(epoch).map_err(RequestError::Revision)?;
    Ok(SessionRevision::new(epoch, sequence))
}

/// Enumerates the current generated provenance fields, preserving their required presence.
/// Supplied labels describe a build; parsing does not approve that build.
pub fn build_identity(value: Option<&wire::BuildIdentity>) -> Result<BuildIdentity, RequestError> {
    let value = value.ok_or(RequestError::Missing(RequestField::Build))?;
    BuildIdentity::new(
        value.source_revision.as_deref(),
        value.native_revision.as_deref(),
        value.wasm_revision.as_deref(),
        value.configuration_revision.as_deref(),
        value.content_revision.as_deref(),
    )
    .map_err(RequestError::Build)
}

/// Borrowed common fields for a caller's mapping, not a protobuf request/envelope or service.
/// The schema owner must supply actual production command DTOs before a handler can call this.
pub struct CommandFields<'a> {
    pub session: Option<&'a wire::SessionId>,
    pub run: Option<&'a wire::RunId>,
    pub operation: Option<&'a wire::OperationId>,
    pub observed_revision: Option<&'a wire::SessionRevision>,
}

/// Maps well-formed common fields into the closed client command input without state checks.
/// Native retry ingress uses this before the session's operation lookup; the engine checks
/// current state only after the repository has confirmed a fresh operation is not recorded.
/// A prior committed receipt must remain discoverable even if its original basis is stale.
/// `member` and `command` have the same trusted-caller and reviewed-DTO obligations as admission.
/// No host/internal event can enter this mapper; only `GameInput::Game` can be returned.
/// ```compile_fail
/// use df_api::{CommandFields, map_game_command};
/// use df_model::checkpoint::GameInput;
/// use df_types::MemberId;
/// fn inject(fields: CommandFields<'_>, member: MemberId, event: GameInput) {
///     let _ = map_game_command(fields, member, event);
/// }
/// ```
pub fn map_game_command(
    fields: CommandFields<'_>,
    member: MemberId,
    command: GameCommand,
) -> Result<GameInput, RequestError> {
    let revision = session_revision(fields.observed_revision)?;
    let basis = Basis {
        session: session_id(fields.session)?,
        run: run_id(fields.run)?,
        revision,
    };
    let operation = operation_id(fields.operation)?;
    Ok(client_command_input(basis, operation, member, command))
}

/// Builds the one client input shape both mappers return: the observed revision is the basis
/// revision, and only `GameInput::Game` can come out.
pub(crate) fn client_command_input(
    basis: Basis,
    operation: OperationId,
    member: MemberId,
    command: GameCommand,
) -> GameInput {
    GameInput::Game(CommandInput {
        observed_revision: basis.revision,
        basis,
        operation,
        member,
        command,
    })
}

/// Maps actual common wire fields and structurally validates a closed game command.
///
/// `member` is a caller-owned authenticated member, never read from wire fields here. The caller
/// must authorize the current binding and actor, look up a prior operation result before this
/// fresh-command path, and perform rules validation and fenced durable commit afterwards.
/// `command` must come from an explicit reviewed DTO mapper; that DTO is currently absent.
/// Host/debug commands, native completions, timers and presentation reports cannot enter here.
/// Only `GameInput::Game` can be returned. No checkpoint or input is mutated on refusal.
///
/// An older observed sequence in the current epoch is context, not a global revision lock;
/// the model checks surviving windows/offers. Wrong sessions/runs, retired or future epochs,
/// future sequences, invalid selections and exhausted limits return typed domain refusals.
///
/// Arbitrary engine events cannot be used as client commands:
/// ```compile_fail
/// use df_api::{CommandFields, admit_game_command};
/// use df_model::checkpoint::{Checkpoint, GameInput, ReferenceInventory};
/// use df_model::commands::CommandLimits;
/// use df_types::MemberId;
/// fn inject(fields: CommandFields<'_>, member: MemberId, event: GameInput,
///     current: &Checkpoint, inventory: ReferenceInventory<'_>, limits: CommandLimits) {
///     let _ = admit_game_command(fields, member, event, current, inventory, limits);
/// }
/// ```
pub fn admit_game_command(
    fields: CommandFields<'_>,
    member: MemberId,
    command: GameCommand,
    checkpoint: &Checkpoint,
    inventory: ReferenceInventory<'_>,
    limits: CommandLimits,
) -> Result<GameInput, RequestError> {
    let input = map_game_command(fields, member, command)?;
    validate_client_command(&input, checkpoint, inventory, limits).map_err(RequestError::Domain)?;
    Ok(input)
}
