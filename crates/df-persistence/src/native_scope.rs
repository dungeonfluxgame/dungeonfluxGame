//! Native binding data is never permission. A configured database verifier rechecks actual
//! df-auth capability identities/revision in the active transaction; absent configuration refuses.
use df_auth::membership::{MembershipAuthority, MembershipCapability};
use df_model::checkpoint::{CheckpointPins, DurableIntent, EffectKind, ExecutionMode, GameInput};
use df_session::inbox::ActorInput;
use df_session::submission::{OperationScope, RepositoryError};
use df_types::{OperationId, RecoveryEpoch, SessionId};
use tokio_postgres::{Statement, Transaction};

/// Native producer-specific mappings preserve the actual df-auth associated domain identities.
/// These callbacks belong to trusted runtime configuration, never the request or OperationScope.
/// Their real producer implementation and current-grant issuer remain qualification gates.
pub(crate) struct AuthKeyMapping<A: MembershipAuthority> {
    pub(crate) tenant_bytes: fn(&A::Tenant) -> Result<[u8; 16], RepositoryError>,
    pub(crate) principal_bytes: fn(&A::Principal) -> Result<[u8; 16], RepositoryError>,
    pub(crate) campaign_bytes: fn(&A::Campaign) -> Result<Vec<u8>, RepositoryError>,
    pub(crate) role_bytes: fn(&A::Role) -> Result<Vec<u8>, RepositoryError>,
    pub(crate) revision_bytes: fn(&A::Revision) -> Result<Vec<u8>, RepositoryError>,
    pub(crate) retained_capability_heap_bytes: fn(&MembershipCapability<A>) -> Option<usize>,
}

/// Only a root-owned prepared statement for the approved DB verifier can configure this adapter.
/// It accepts the issuer's binding and session and returns bounded exact verified authority
/// and operation fields. Its SQL must project only bounded rows/columns before driver allocation;
/// campaign/role/access revision and causal completion associations are issuer-owned.
/// It must implement D03 lifetime/revocation/unforgeability; query text is never caller-controlled.
pub(crate) struct DatabaseBindingVerifier {
    pub(crate) statement: Option<Statement>,
    pub(crate) maximum_binding_bytes: usize,
    pub(crate) maximum_authority_value_bytes: usize,
    pub(crate) maximum_namespace_bytes: usize,
}
impl DatabaseBindingVerifier {
    pub(crate) fn unconfigured() -> Self {
        Self {
            statement: None,
            maximum_binding_bytes: 0,
            maximum_authority_value_bytes: 0,
            maximum_namespace_bytes: 0,
        }
    }

    pub(crate) async fn verify<A: MembershipAuthority>(
        &self,
        transaction: &Transaction<'_>,
        scope: &NativeScope<A>,
    ) -> Result<(), RepositoryError> {
        let statement = self
            .statement
            .as_ref()
            .ok_or(RepositoryError::Unauthorized)?;
        if self.maximum_binding_bytes == 0
            || self.maximum_authority_value_bytes == 0
            || self.maximum_namespace_bytes == 0
            || scope.binding.is_empty()
            || scope.binding.len() > self.maximum_binding_bytes
            || scope.namespace.is_empty()
            || scope.namespace.len() > self.maximum_namespace_bytes
        {
            return Err(RepositoryError::Capacity);
        }
        let session = scope.session;
        let row = transaction
            .query_opt(
                statement,
                &[&scope.binding.as_slice(), &session.as_bytes().as_slice()],
            )
            .await
            .map_err(|_| RepositoryError::Unavailable)?
            .ok_or(RepositoryError::Unauthorized)?;
        let tenant: Vec<u8> = row
            .try_get("tenant_id")
            .map_err(|_| RepositoryError::Unauthorized)?;
        let principal: Vec<u8> = row
            .try_get("principal_id")
            .map_err(|_| RepositoryError::Unauthorized)?;
        let campaign: Vec<u8> = row
            .try_get("campaign_id")
            .map_err(|_| RepositoryError::Unauthorized)?;
        let role: Vec<u8> = row
            .try_get("effective_role")
            .map_err(|_| RepositoryError::Unauthorized)?;
        let revision: Vec<u8> = row
            .try_get("access_revision")
            .map_err(|_| RepositoryError::Unauthorized)?;
        let permission_current: bool = row
            .try_get("permission_current")
            .map_err(|_| RepositoryError::Unauthorized)?;
        let request = scope.capability.request();
        let expected_tenant = (scope.mapping.tenant_bytes)(request.tenant)?;
        let expected_principal = (scope.mapping.principal_bytes)(request.principal)?;
        let expected_campaign = (scope.mapping.campaign_bytes)(request.campaign)?;
        let expected_role = (scope.mapping.role_bytes)(request.role)?;
        let expected_revision = (scope.mapping.revision_bytes)(scope.capability.revision())?;
        for authority_bytes in [
            &campaign,
            &role,
            &revision,
            &expected_campaign,
            &expected_role,
            &expected_revision,
        ] {
            if authority_bytes.is_empty()
                || authority_bytes.len() > self.maximum_authority_value_bytes
            {
                return Err(RepositoryError::Unauthorized);
            }
        }
        let operation: Vec<u8> = row
            .try_get("operation_id")
            .map_err(|_| RepositoryError::Unauthorized)?;
        let namespace: Vec<u8> = row
            .try_get("command_namespace")
            .map_err(|_| RepositoryError::Unauthorized)?;
        let epoch: String = row
            .try_get("recovery_epoch")
            .map_err(|_| RepositoryError::Unauthorized)?;
        let fingerprint_version: i32 = row
            .try_get("fingerprint_version")
            .map_err(|_| RepositoryError::Unauthorized)?;
        let fingerprint: Vec<u8> = row
            .try_get("canonical_fingerprint")
            .map_err(|_| RepositoryError::Unauthorized)?;
        let fence: Vec<u8> = row
            .try_get("owner_fence")
            .map_err(|_| RepositoryError::Unauthorized)?;
        let mode: i16 = row
            .try_get("execution_mode")
            .map_err(|_| RepositoryError::Unauthorized)?;
        let lookup_only: bool = row
            .try_get("lookup_only")
            .map_err(|_| RepositoryError::Unauthorized)?;
        let expected_mode = match scope.mode {
            ExecutionMode::Live => 1,
            ExecutionMode::PreparedOnly => 2,
            ExecutionMode::Replay => 3,
        };
        if operation.as_slice() != scope.operation.as_bytes()
            || namespace != scope.namespace
            || crate::revision_codec::decode_unsigned_number(&epoch)
                .map_err(|_| RepositoryError::Unauthorized)?
                != scope.epoch.get()
            || fingerprint_version != scope.fingerprint_version()
            || fingerprint.as_slice() != scope.fingerprint
            || fence.as_slice() != scope.owner_fence
            || mode != expected_mode
            || lookup_only != scope.lookup_only
        {
            return Err(RepositoryError::Unauthorized);
        }
        if !permission_current
            || tenant.as_slice() != expected_tenant
            || principal.as_slice() != expected_principal
            || campaign != expected_campaign
            || role != expected_role
            || revision != expected_revision
            || tenant.as_slice() != scope.tenant
            || principal.as_slice() != scope.principal
        {
            return Err(RepositoryError::Unauthorized);
        }
        Ok(())
    }
}

/// This scope owns the actual checked df-auth capability. It has no public raw-data constructor.
/// Root-owned native producer builds it from authenticated membership, immutable input binding,
/// current issuer proof and native owner lease. It is independently verified on every repository call.
pub struct NativeScope<A: MembershipAuthority> {
    pub(crate) capability: MembershipCapability<A>,
    pub(crate) mapping: AuthKeyMapping<A>,
    pub(crate) binding: Vec<u8>,
    pub(crate) tenant: [u8; 16],
    pub(crate) principal: [u8; 16],
    pub(crate) session: SessionId,
    pub(crate) operation: OperationId,
    pub(crate) namespace: Vec<u8>,
    pub(crate) epoch: RecoveryEpoch,
    pub(crate) fingerprint: [u8; 32],
    pub(crate) owner_fence: [u8; 16],
    pub(crate) lookup_only: bool,
    pub(crate) bound_input: GameInput,
    pub(crate) pins: CheckpointPins,
    pub(crate) mode: ExecutionMode,
}
impl<A: MembershipAuthority> NativeScope<A> {
    pub(crate) fn tenant_bytes(&self) -> &[u8] {
        &self.tenant
    }
    pub(crate) fn principal_bytes(&self) -> &[u8] {
        &self.principal
    }
    pub(crate) fn namespace_bytes(&self) -> &[u8] {
        &self.namespace
    }
    pub(crate) fn epoch(&self) -> RecoveryEpoch {
        self.epoch
    }
    pub(crate) fn owner_fence_bytes(&self) -> &[u8] {
        &self.owner_fence
    }
    pub(crate) fn fingerprint_version(&self) -> i32 {
        1
    }
    pub(crate) fn fingerprint_bytes(&self) -> &[u8] {
        &self.fingerprint
    }
    pub(crate) fn admitted_mode(&self) -> ExecutionMode {
        self.mode
    }
    pub(crate) fn admitted_pins(&self) -> &CheckpointPins {
        &self.pins
    }
    pub(crate) fn session(&self) -> SessionId {
        self.session
    }
    pub(crate) fn operation(&self) -> OperationId {
        self.operation
    }
    pub(crate) fn is_lookup_only(&self) -> bool {
        self.lookup_only
    }
    pub(crate) fn validate_unpaid_intent(
        &self,
        intent: &DurableIntent,
    ) -> Result<(), RepositoryError> {
        if self.mode == ExecutionMode::Live
            && matches!(intent.kind, EffectKind::RunAi | EffectKind::RunMedia)
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        Ok(())
    }
}
impl<A: MembershipAuthority> ActorInput for NativeScope<A>
where
    A::Principal: Send,
    A::Tenant: Send,
    A::Campaign: Send,
    A::Role: Send,
    A::Revision: Send,
{
    fn retained_heap_bytes(&self) -> Option<usize> {
        (self.mapping.retained_capability_heap_bytes)(&self.capability)?
            .checked_add(self.binding.capacity())?
            .checked_add(self.namespace.capacity())?
            .checked_add(self.bound_input.retained_heap_bytes()?)?
            .checked_add(crate::checkpoint_codec::pins_retained_heap_bytes(
                &self.pins,
            )?)
    }
}
impl<A: MembershipAuthority> OperationScope for NativeScope<A>
where
    A::Principal: Send,
    A::Tenant: Send,
    A::Campaign: Send,
    A::Role: Send,
    A::Revision: Send,
{
    type UncertaintyKey = NativeUncertaintyKey;

    fn capture_uncertainty_key(
        &self,
        maximum_retained_bytes: usize,
    ) -> Result<Self::UncertaintyKey, RepositoryError> {
        let namespace = copy_key_namespace(&self.namespace, maximum_retained_bytes)?;
        Ok(NativeUncertaintyKey {
            tenant: self.tenant,
            session: self.session,
            principal: self.principal,
            namespace,
            epoch: self.epoch,
            operation: self.operation,
            fingerprint_version: self.fingerprint_version(),
            fingerprint: self.fingerprint,
        })
    }

    fn session(&self) -> SessionId {
        self.session
    }
    fn operation(&self) -> OperationId {
        self.operation
    }
    fn validate_input(&self, input: &GameInput) -> Result<(), RepositoryError> {
        if input != &self.bound_input {
            return Err(RepositoryError::InputBinding);
        }
        Ok(())
    }
    fn is_lookup_only(&self) -> bool {
        self.lookup_only
    }
}

/// Owned exact identity of the first unresolved operation. Equality includes its canonical
/// input binding; refreshing access/owner/issuer proof never changes this identity.
/// Private fields and no public constructor ensure this is data, never an authority grant.
#[derive(Eq, PartialEq)]
pub struct NativeUncertaintyKey {
    tenant: [u8; 16],
    session: SessionId,
    principal: [u8; 16],
    namespace: Vec<u8>,
    epoch: RecoveryEpoch,
    operation: OperationId,
    fingerprint_version: i32,
    fingerprint: [u8; 32],
}
impl ActorInput for NativeUncertaintyKey {
    fn retained_heap_bytes(&self) -> Option<usize> {
        Some(self.namespace.capacity())
    }
}
fn copy_key_namespace(
    namespace: &[u8],
    maximum_retained_bytes: usize,
) -> Result<Vec<u8>, RepositoryError> {
    let maximum_heap_bytes = maximum_retained_bytes
        .checked_sub(std::mem::size_of::<NativeUncertaintyKey>())
        .ok_or(RepositoryError::Capacity)?;
    if maximum_retained_bytes == 0 || namespace.len() > maximum_heap_bytes {
        return Err(RepositoryError::Capacity);
    }
    let mut namespace_copy = Vec::new();
    namespace_copy
        .try_reserve_exact(namespace.len())
        .map_err(|_| RepositoryError::Capacity)?;
    if namespace_copy.capacity() > maximum_heap_bytes {
        return Err(RepositoryError::Capacity);
    }
    namespace_copy.extend_from_slice(namespace);
    Ok(namespace_copy)
}

#[cfg(test)]
mod uncertainty_key_tests {
    use super::*;
    fn key() -> NativeUncertaintyKey {
        NativeUncertaintyKey {
            tenant: [1; 16],
            session: SessionId::from_bytes(&[2; 16]).unwrap(),
            principal: [3; 16],
            namespace: b"command/v1".to_vec(),
            epoch: RecoveryEpoch::new(4).unwrap(),
            operation: OperationId::from_bytes(&[5; 16]).unwrap(),
            fingerprint_version: 1,
            fingerprint: [6; 32],
        }
    }
    #[test]
    fn every_physical_scope_and_semantic_binding_dimension_changes_the_key() {
        let original = key();
        let mut other = key();
        other.tenant = [9; 16];
        assert!(original != other);
        let mut other = key();
        other.session = SessionId::from_bytes(&[9; 16]).unwrap();
        assert!(original != other);
        let mut other = key();
        other.principal = [9; 16];
        assert!(original != other);
        let mut other = key();
        other.namespace = b"native/job".to_vec();
        assert!(original != other);
        let mut other = key();
        other.epoch = RecoveryEpoch::new(9).unwrap();
        assert!(original != other);
        let mut other = key();
        other.operation = OperationId::from_bytes(&[9; 16]).unwrap();
        assert!(original != other);
        let mut other = key();
        other.fingerprint_version = 2;
        assert!(original != other);
        let mut other = key();
        other.fingerprint = [9; 32];
        assert!(original != other);
        assert!(original == key());
    }
    #[test]
    fn key_copy_refuses_zero_and_requested_overflow_before_allocating() {
        assert!(matches!(
            copy_key_namespace(b"namespace", 0),
            Err(RepositoryError::Capacity)
        ));
        let key_inline_size_bytes = std::mem::size_of::<NativeUncertaintyKey>();
        assert!(matches!(
            copy_key_namespace(b"namespace", key_inline_size_bytes + 8),
            Err(RepositoryError::Capacity)
        ));
        let copied_namespace =
            copy_key_namespace(b"namespace", key_inline_size_bytes + 128).unwrap();
        assert_eq!(copied_namespace, b"namespace");
        assert!(copied_namespace.capacity() <= 128);
    }
    #[test]
    fn retained_key_accounting_reports_actual_capacity() {
        let mut uncertainty_key = key();
        uncertainty_key.namespace.reserve_exact(64);
        assert_eq!(
            uncertainty_key.retained_heap_bytes(),
            Some(uncertainty_key.namespace.capacity())
        );
    }
}
