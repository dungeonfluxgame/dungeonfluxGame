//! Explicitly local-development grants for the single gameplay demonstration.
//! The caller owns a dedicated loopback database, connection driver, and actor thread.
//! No production bootstrap, arbitrary scope constructor, or hosted identity is provided.
mod adventure;
mod recovery;
mod restart;
pub use restart::{ensure_run_database, retain_run_successor, run_successor};
mod seats;
pub use seats::{
    LocalReclaimedSeat, LocalRoomSeat, LocalSeatReclaim, SEAT_AWAY_MILLIS, SeatReclaimRefusal,
};
mod table_settings;
pub use adventure::{ADVENTURE_DOCUMENT_BYTES, AdventureStartup, DemoAdventure, stored_adventure};
pub use table_settings::StoredTableSettings;
mod startup;
pub use startup::{
    DemoStartup, DemoStartupIdentity, admit_startup, admit_startup_with_adventure, release_owner,
    stored_startup_configuration,
};

use std::time::Duration;

use df_auth::membership::{
    MembershipAuthority, MembershipRecord, MembershipRequest, authorize_membership,
};
use df_model::checkpoint::{
    Checkpoint, CheckpointPins, DurableStatus, EffectKind, ExecutionMode, GameInput, JobCompletion,
};
use df_session::submission::RepositoryError;
use df_types::{MemberId, OperationId, RecoveryEpoch, SessionId};
use tokio::runtime::Handle;
use tokio_postgres::Client;

use crate::checkpoint_codec::encode_checkpoint;
use crate::native_scope::{AuthKeyMapping, DatabaseBindingVerifier};

pub enum LocalRejectedLookup {
    NotRecorded,
    Accepted,
    Committed(Vec<u8>),
    Conflict,
    Expired,
}
use crate::{NativeCodecLimits, NativeScope, NativeVerifierSource};

/// Retained bytes allowed for one operation's receipt (the accepted action, hex-encoded, plus
/// fixed overhead). A Longbow creation sheet alone needs about 2.1 KiB here, so the old 4 KiB
/// bound refused a valid archer build as a capacity failure. The session owner and the
/// repository must use this one value.
pub const RECEIPT_RETAINED_BYTES_MAX: usize = 8192;
pub const TENANT: [u8; 16] = [0x31; 16];
pub const CAMPAIGN: [u8; 16] = [0x32; 16];
pub const PLAYER: [u8; 16] = [0x33; 16];
pub const DISPLAY: [u8; 16] = [0x34; 16];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalDemoRole {
    Player,
    Display,
}
impl LocalDemoRole {
    fn bytes(self) -> Vec<u8> {
        match self {
            Self::Player => b"player".to_vec(),
            Self::Display => b"display".to_vec(),
        }
    }
}

/// How long the demo's owner lease and access grants last before they must be renewed.
static LEASE_SECONDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(300);

/// Sets the lease length for this process, before startup seeds or claims a session. The owner
/// renews it long before it lapses (see [`LocalDemoScopeIssuer::renew_owner`]).
pub fn set_lease_seconds(seconds: u64) {
    LEASE_SECONDS.store(seconds.max(1), std::sync::atomic::Ordering::Relaxed);
}

/// The lease length in seconds (300 unless set).
pub fn lease_seconds() -> u64 {
    LEASE_SECONDS.load(std::sync::atomic::Ordering::Relaxed)
}

const VERIFY: &str = "SELECT p.*, g.active AND g.expires_at > clock_timestamp() AS permission_current FROM df_local_demo.scope_proofs p JOIN df_local_demo.grants g ON g.credential = p.credential WHERE p.binding = $1::bytea AND p.session_id = $2::bytea";

pub fn verifier() -> NativeVerifierSource {
    NativeVerifierSource {
        query: VERIFY,
        maximum_binding_bytes: 32,
        maximum_authority_value_bytes: 128,
        maximum_namespace_bytes: 128,
    }
}

/// This authority reads actual configured grant rows, never request-owned role claims.
pub struct LocalDemoAuthority {
    runtime: Handle,
    client: Option<Client>,
}
impl LocalDemoAuthority {
    fn client(&self) -> Result<&Client, RepositoryError> {
        self.client.as_ref().ok_or(RepositoryError::Unavailable)
    }
    fn client_mut(&mut self) -> Result<&mut Client, RepositoryError> {
        self.client.as_mut().ok_or(RepositoryError::Unavailable)
    }
}
impl MembershipAuthority for LocalDemoAuthority {
    type Principal = [u8; 16];
    type Tenant = [u8; 16];
    type Campaign = [u8; 16];
    type Role = Vec<u8>;
    type Revision = Vec<u8>;
    type Error = RepositoryError;
    fn read_current(
        &mut self,
        request: MembershipRequest<'_, Self>,
    ) -> Result<Option<MembershipRecord<Self>>, RepositoryError> {
        self.runtime.block_on(async {
            let row = tokio::time::timeout(Duration::from_secs(2), self.client()?.query_opt(
                "SELECT principal_id, tenant_id, campaign_id, effective_role, access_revision, active, extract(epoch from expires_at)::bigint AS expires_unix FROM df_local_demo.grants WHERE tenant_id = $1::bytea AND principal_id = $2::bytea AND campaign_id = $3::bytea AND effective_role = $4::bytea AND active LIMIT 1",
                &[&request.tenant.as_slice(), &request.principal.as_slice(), &request.campaign.as_slice(), &request.role.as_slice()]))
                .await.map_err(|_| RepositoryError::Unavailable)?
                .map_err(|_| RepositoryError::Unavailable)?;
            let Some(row) = row else { return Ok(None); };
            let identity = |name| -> Result<[u8;16], RepositoryError> {
                row.try_get::<_,Vec<u8>>(name).map_err(|_| RepositoryError::Unauthorized)?
                    .try_into().map_err(|_| RepositoryError::Unauthorized)
            };
            Ok(Some(MembershipRecord { principal: identity("principal_id")?, tenant: identity("tenant_id")?,
                campaign: identity("campaign_id")?, role: row.try_get("effective_role").map_err(|_| RepositoryError::Unauthorized)?,
                revision: row.try_get("access_revision").map_err(|_| RepositoryError::Unauthorized)?,
                active: row.try_get("active").map_err(|_| RepositoryError::Unauthorized)?,
                expires_at: Some(u64::try_from(row.try_get::<_,i64>("expires_unix").map_err(|_| RepositoryError::Unauthorized)?).map_err(|_| RepositoryError::Unauthorized)?) }))
        })
    }
}

pub struct LocalDemoScopeIssuer {
    authority: LocalDemoAuthority,
    session: SessionId,
    fence: [u8; 16],
    reconnect: Option<tokio_postgres::Config>,
    driver: Option<tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>>,
}
struct LocalInputBinding {
    principal: [u8; 16],
    operation: OperationId,
    epoch: RecoveryEpoch,
    namespace: Vec<u8>,
    proof_binding: [u8; 32],
    fingerprint: [u8; 32],
    input: GameInput,
    pins: CheckpointPins,
}
impl LocalDemoScopeIssuer {
    /// Extends this owner's session lease and every active access grant of the session by the
    /// lease length. It succeeds whether or not the lease has already lapsed, because the owner
    /// fence is what names the owner: while the fence is still this process's, nobody else has
    /// claimed the session (a claim replaces the fence), so an idle table never loses its game.
    /// A different fence means ownership was taken over, which is `RevisionConflict`.
    pub fn renew_owner(&mut self) -> Result<(), RepositoryError> {
        let runtime = self.authority.runtime.clone();
        crate::native_connection::actor_block_on(&runtime, self.renew_owner_owned())?
    }

    async fn renew_owner_owned(&mut self) -> Result<(), RepositoryError> {
        self.reconnect_owned().await?;
        let seconds = i64::try_from(lease_seconds()).map_err(|_| RepositoryError::Capacity)?;
        let (session, fence) = (self.session, self.fence);
        let client = self.authority.client_mut()?;
        tokio::time::timeout(Duration::from_secs(5), async {
            let transaction = client
                .transaction()
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            let renewed = transaction
                .execute(
                    "UPDATE df_game.sessions SET lease_until=clock_timestamp()+($3::bigint * interval '1 second') WHERE tenant_id=$1::bytea AND session_id=$2::bytea AND owner_fence=$4::bytea",
                    &[&TENANT.as_slice(), &session.as_bytes().as_slice(), &seconds, &fence.as_slice()],
                )
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            if renewed != 1 {
                return Err(RepositoryError::RevisionConflict);
            }
            transaction
                .execute(
                    "UPDATE df_local_demo.grants SET expires_at=clock_timestamp()+($3::bigint * interval '1 second') WHERE tenant_id=$1::bytea AND session_id=$2::bytea AND active",
                    &[&TENANT.as_slice(), &session.as_bytes().as_slice(), &seconds],
                )
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            transaction
                .commit()
                .await
                .map_err(|_| RepositoryError::Unavailable)
        })
        .await
        .map_err(|_| RepositoryError::Unavailable)?
    }

    /// Call only after the serialization inbox has stopped admission and drained.
    pub fn release_owner(&mut self) -> Result<(), RepositoryError> {
        let runtime = self.authority.runtime.clone();
        runtime.block_on(self.release_owner_owned())
    }

    /// Native setup failure owns this issuer before it has entered an actor thread.
    pub async fn release_owner_owned(&mut self) -> Result<(), RepositoryError> {
        self.reconnect_owned().await?;
        release_owner(self.authority.client()?, self.session, self.fence).await
    }

    /// Same actor operation key as accepted decisions. New rejected bytes are an already
    /// mapped typed RPC receipt; persistence stores them immutably without inventing facts.
    /// Any uncertain transaction stops the native owner; it must not submit another key.
    pub fn rejected_operation(
        &mut self,
        scope: &NativeScope<LocalDemoAuthority>,
        current: df_model::checkpoint::Basis,
        new_receipt: Option<&[u8]>,
        codec_limits: NativeCodecLimits,
    ) -> Result<LocalRejectedLookup, RepositoryError> {
        if new_receipt.is_some_and(|bytes| bytes.is_empty() || bytes.len() > 4096) {
            return Err(RepositoryError::Capacity);
        }
        let runtime = self.authority.runtime.clone();
        runtime.block_on(async {
            let result = tokio::time::timeout(Duration::from_secs(2), async {
                let statement = self.authority.client()?.prepare(VERIFY).await.map_err(|_| RepositoryError::Unavailable)?;
                let verifier = DatabaseBindingVerifier { statement:Some(statement), maximum_binding_bytes:32,
                    maximum_authority_value_bytes:128, maximum_namespace_bytes:128 };
                let transaction = self.authority.client_mut()?.transaction().await.map_err(|_| RepositoryError::Unavailable)?;
                verifier.verify(&transaction,scope).await?;
                let locked = crate::decision_rows::lock_session(&transaction,scope).await?;
                // A grant can expire or be revoked while this transaction waits for the row.
                verifier.verify(&transaction,scope).await?;
                let accepted = crate::decision_rows::lookup_locked(&transaction,scope,codec_limits,RECEIPT_RETAINED_BYTES_MAX).await?;
                let epoch = scope.epoch.get().to_string();
                let parameters: &[&(dyn tokio_postgres::types::ToSql + Sync)] = &[
                    &scope.tenant.as_slice(), &scope.session.as_bytes().as_slice(),
                    &scope.principal.as_slice(), &scope.namespace.as_slice(), &epoch,
                    &scope.operation.as_bytes().as_slice()];
                let rejected = transaction.query_opt("SELECT fingerprint_version, typed_receipt_version, canonical_fingerprint, typed_receipt IS NULL AS tombstone, octet_length(typed_receipt)::bigint AS receipt_bytes, CASE WHEN octet_length(typed_receipt) <= 4096 THEN typed_receipt END AS typed_receipt FROM df_local_demo.rejected_operations WHERE tenant_id=$1 AND session_id=$2 AND principal_id=$3 AND command_namespace=$4 AND recovery_epoch=$5::text::numeric AND operation_id=$6",parameters).await.map_err(|_|RepositoryError::Unavailable)?;
                use df_session::submission::OperationLookup;
                let has_accepted = matches!(accepted,OperationLookup::Committed(_) | OperationLookup::Conflict);
                if has_accepted && rejected.is_some() { return Err(RepositoryError::InvalidReceipt); }
                let outcome = if has_accepted { LocalRejectedLookup::Accepted }
                else if matches!(accepted,OperationLookup::ExpiredOrIndeterminate | OperationLookup::InProgress) { LocalRejectedLookup::Expired }
                else if let Some(row)=rejected {
                    let version:i32=row.try_get("fingerprint_version").map_err(|_|RepositoryError::InvalidReceipt)?;
                    if row.try_get::<_,i32>("typed_receipt_version").map_err(|_|RepositoryError::InvalidReceipt)? != 1 { return Err(RepositoryError::InvalidReceipt); }
                    let fingerprint:Vec<u8>=row.try_get("canonical_fingerprint").map_err(|_|RepositoryError::InvalidReceipt)?;
                    if version!=1 || fingerprint.as_slice()!=scope.fingerprint { LocalRejectedLookup::Conflict }
                    else if row.try_get::<_,bool>("tombstone").map_err(|_|RepositoryError::InvalidReceipt)? { LocalRejectedLookup::Expired }
                    else {
                        let length:i64=row.try_get("receipt_bytes").map_err(|_|RepositoryError::InvalidReceipt)?;
                        if !(1..=4096).contains(&length) { return Err(RepositoryError::InvalidReceipt); }
                        LocalRejectedLookup::Committed(row.try_get("typed_receipt").map_err(|_|RepositoryError::InvalidReceipt)?)
                    }
                } else if let Some(bytes)=new_receipt {
                    locked.validate_new_decision(scope,current)?;
                    verifier.verify(&transaction,scope).await?;
                    let current_epoch=current.revision.epoch().get().to_string();
                    let current_sequence=current.revision.sequence().to_string();
                    // Recheck the live lease at the write boundary, without advancing game state.
                    let fence = transaction.query_opt("UPDATE df_game.sessions SET lease_until=lease_until WHERE tenant_id=$1::bytea AND session_id=$2::bytea AND run_id=$3::bytea AND owner_fence=$4::bytea AND recovery_epoch=$5::text::numeric AND in_epoch_sequence=$6::text::numeric AND lease_until>clock_timestamp() RETURNING session_id", &[&scope.tenant.as_slice(),&scope.session.as_bytes().as_slice(),&current.run.as_bytes().as_slice(),&scope.owner_fence.as_slice(),&current_epoch,&current_sequence]).await.map_err(|_|RepositoryError::Unavailable)?;
                    if fence.is_none() { return Err(RepositoryError::RevisionConflict); }
                    transaction.execute("INSERT INTO df_local_demo.rejected_operations (tenant_id,session_id,principal_id,command_namespace,recovery_epoch,operation_id,fingerprint_version,canonical_fingerprint,checkpoint_epoch,checkpoint_sequence,typed_receipt_version,typed_receipt) VALUES ($1,$2,$3,$4,$5::text::numeric,$6,1,$7,$8::text::numeric,$9::text::numeric,1,$10)", &[&scope.tenant.as_slice(),&scope.session.as_bytes().as_slice(),&scope.principal.as_slice(),&scope.namespace.as_slice(),&epoch,&scope.operation.as_bytes().as_slice(),&scope.fingerprint.as_slice(),&current_epoch,&current_sequence,&bytes]).await.map_err(|_|RepositoryError::Unavailable)?;
                    LocalRejectedLookup::Committed(bytes.to_vec())
                } else { LocalRejectedLookup::NotRecorded };
                transaction.commit().await.map_err(|_|RepositoryError::UnresolvedCommit)?;
                Ok(outcome)
            }).await;
            result.map_err(|_|RepositoryError::UnresolvedCommit)?
        })
    }
    /// Construction is native configuration and must run on the dedicated actor thread.
    pub fn new(
        runtime: Handle,
        client: Client,
        session: SessionId,
        fence: [u8; 16],
    ) -> Result<Self, RepositoryError> {
        if fence == [0; 16] {
            return Err(RepositoryError::Unauthorized);
        }
        Ok(Self {
            authority: LocalDemoAuthority {
                runtime,
                client: Some(client),
            },
            session,
            fence,
            reconnect: None,
            driver: None,
        })
    }

    /// Validate a server-issued credential against its actual current grant row.
    pub fn authenticate(&self, credential: &[u8; 32]) -> Result<LocalDemoRole, RepositoryError> {
        self.authority.runtime.block_on(async {
            let row = tokio::time::timeout(Duration::from_secs(2), self.authority.client()?.query_opt(
                "SELECT principal_id, effective_role FROM df_local_demo.grants WHERE credential = $1::bytea AND session_id = $2::bytea AND tenant_id = $3::bytea AND campaign_id = $4::bytea AND active AND expires_at > clock_timestamp()",
                &[&credential.as_slice(), &self.session.as_bytes().as_slice(), &TENANT.as_slice(), &CAMPAIGN.as_slice()]))
                .await.map_err(|_| RepositoryError::Unavailable)?.map_err(|_| RepositoryError::Unavailable)?
                .ok_or(RepositoryError::Unauthorized)?;
            let principal: Vec<u8> = row.try_get("principal_id").map_err(|_| RepositoryError::Unauthorized)?;
            let role: Vec<u8> = row.try_get("effective_role").map_err(|_| RepositoryError::Unauthorized)?;
            match (principal.as_slice(), role.as_slice()) {
                (p, b"player") if p.len() == 16 && p != DISPLAY => Ok(LocalDemoRole::Player),
                (p, b"display") if p == DISPLAY => Ok(LocalDemoRole::Display),
                _ => Err(RepositoryError::Unauthorized),
            }
        })
    }

    /// Resolve the principal from an actual live native grant, never an RPC role or member.
    pub fn principal(&self, credential: &[u8; 32]) -> Result<[u8; 16], RepositoryError> {
        self.authority.runtime.block_on(async {
            let row = tokio::time::timeout(Duration::from_secs(2), self.authority.client()?.query_opt(
                "SELECT principal_id FROM df_local_demo.grants WHERE credential=$1::bytea AND session_id=$2::bytea AND tenant_id=$3::bytea AND campaign_id=$4::bytea AND active AND expires_at>clock_timestamp()",
                &[&credential.as_slice(),&self.session.as_bytes().as_slice(),&TENANT.as_slice(),&CAMPAIGN.as_slice()]
            )).await.map_err(|_|RepositoryError::Unavailable)?.map_err(|_|RepositoryError::Unavailable)?
                .ok_or(RepositoryError::Unauthorized)?;
            row.try_get::<_,Vec<u8>>("principal_id").map_err(|_|RepositoryError::Unauthorized)?
                .try_into().map_err(|_|RepositoryError::Unauthorized)
        })
    }

    /// Complete only an already committed local-room join on the same serialized actor.
    /// The join secret itself is never stored: its exact request fingerprint is immutable.
    pub fn complete_room_join(
        &mut self,
        scope: &NativeScope<LocalDemoAuthority>,
        current: &Checkpoint,
        member: [u8; 16],
        entity: [u8; 16],
        new_credential: [u8; 32],
        codec: NativeCodecLimits,
    ) -> Result<[u8; 32], RepositoryError> {
        if scope.principal != PLAYER
            || scope.namespace != b"local-room-join-v1"
            || member == PLAYER
            || member == DISPLAY
            || new_credential == [0; 32]
            || !current.state().members.iter().any(|link| {
                link.member.as_bytes() == &member
                    && link.character.is_some_and(|id| id.as_bytes() == &entity)
            })
        {
            return Err(RepositoryError::Unauthorized);
        }
        let hex = |bytes: &[u8]| {
            bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        };
        let expected = format!("join:{}:{}", hex(&member), hex(&entity));
        let encoded =
            encode_checkpoint(current, codec).map_err(|_| RepositoryError::InvalidCandidate)?;
        let runtime = self.authority.runtime.clone();
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(2),async {
                let statement=self.authority.client()?.prepare(VERIFY).await.map_err(|_|RepositoryError::Unavailable)?;
                let verifier=DatabaseBindingVerifier {statement:Some(statement),maximum_binding_bytes:32,
                    maximum_authority_value_bytes:128,maximum_namespace_bytes:128};
                let transaction=self.authority.client_mut()?.transaction().await.map_err(|_|RepositoryError::Unavailable)?;
                verifier.verify(&transaction,scope).await?;
                let locked=crate::decision_rows::lock_session(&transaction,scope).await?;
                verifier.verify(&transaction,scope).await?;
                locked.validate_new_decision(scope,current.basis())?;
                let committed=crate::decision_rows::lookup_locked(&transaction,scope,codec,4096).await?;
                let df_session::submission::OperationLookup::Committed(receipt)=committed else {
                    return Err(RepositoryError::OperationConflict);
                };
                if receipt.decision().source_policy.as_str()!="local-room-join-1"
                    || receipt.decision().semantic_output.as_deref()!=Some(expected.as_str())
                { return Err(RepositoryError::InvalidReceipt); }
                let row=transaction.query_one(crate::sql::LOAD_CURRENT,&[
                    &scope.tenant.as_slice(),&scope.session.as_bytes().as_slice(),
                    &(codec.maximum_document_bytes as i64)
                ]).await.map_err(|_|RepositoryError::Unavailable)?;
                let actual:Vec<u8>=row.try_get("complete_envelope").map_err(|_|RepositoryError::InvalidCandidate)?;
                if actual!=encoded {return Err(RepositoryError::RevisionConflict);}
                let prior=transaction.query_opt(
                    "SELECT j.canonical_fingerprint,j.principal_id,g.credential,g.active AND g.expires_at>clock_timestamp() AS permission_current FROM df_local_demo.room_grants j JOIN df_local_demo.grants g ON g.credential=j.credential WHERE j.operation_id=$1::bytea AND g.session_id=$2::bytea",
                    &[&scope.operation.as_bytes().as_slice(),&scope.session.as_bytes().as_slice()]
                ).await.map_err(|_|RepositoryError::Unavailable)?;
                let credential=if let Some(prior)=prior {
                    if !prior.try_get::<_,bool>("permission_current").map_err(|_|RepositoryError::InvalidReceipt)? {return Err(RepositoryError::Unauthorized);}
                    let fingerprint:Vec<u8>=prior.try_get("canonical_fingerprint").map_err(|_|RepositoryError::InvalidReceipt)?;
                    let principal:Vec<u8>=prior.try_get("principal_id").map_err(|_|RepositoryError::InvalidReceipt)?;
                    if fingerprint!=scope.fingerprint || principal!=member {return Err(RepositoryError::OperationConflict);}
                    prior.try_get::<_,Vec<u8>>("credential").map_err(|_|RepositoryError::InvalidReceipt)?
                        .try_into().map_err(|_|RepositoryError::InvalidReceipt)?
                } else {
                    // Current lease is checked at the actual write, not only before waiting.
                    let written=transaction.execute(
                        "INSERT INTO df_local_demo.grants SELECT $1::bytea,$2::bytea,$3::bytea,$4::bytea,$5::bytea,$6::bytea,$7::bytea,true,lease_until FROM df_game.sessions WHERE tenant_id=$2::bytea AND session_id=$7::bytea AND run_id=$8::bytea AND owner_fence=$9::bytea AND recovery_epoch=$10::text::numeric AND in_epoch_sequence=$11::text::numeric AND lease_until>clock_timestamp()",
                        &[&new_credential.as_slice(),&TENANT.as_slice(),&member.as_slice(),
                          &CAMPAIGN.as_slice(),&b"player".as_slice(),&b"local-demo-access-1".as_slice(),
                          &scope.session.as_bytes().as_slice(),&current.basis().run.as_bytes().as_slice(),
                          &scope.owner_fence.as_slice(),&current.basis().revision.epoch().get().to_string(),
                          &current.basis().revision.sequence().to_string()]
                    ).await.map_err(|_|RepositoryError::Unavailable)?;
                    if written!=1 {return Err(RepositoryError::ExpiredOwner);}
                    transaction.execute(
                        "INSERT INTO df_local_demo.room_grants VALUES ($1::bytea,$2::bytea,$3::bytea,$4::bytea)",
                        &[&scope.operation.as_bytes().as_slice(),&scope.fingerprint.as_slice(),
                          &member.as_slice(),&new_credential.as_slice()]
                    ).await.map_err(|_|RepositoryError::Unavailable)?;
                    new_credential
                };
                verifier.verify(&transaction,scope).await?;
                transaction.commit().await.map_err(|_|RepositoryError::UnresolvedCommit)?;
                Ok(credential)
            }).await.map_err(|_|RepositoryError::UnresolvedCommit)?
        })
    }

    /// Issue an immutable exact input proof after current player authentication.
    /// `proof_binding` and fingerprint come from this native owner, never RPC fields.
    pub fn issue(
        &mut self,
        credential: &[u8; 32],
        proof_binding: [u8; 32],
        fingerprint: [u8; 32],
        input: GameInput,
        pins: CheckpointPins,
    ) -> Result<NativeScope<LocalDemoAuthority>, RepositoryError> {
        if self.authenticate(credential)? != LocalDemoRole::Player {
            return Err(RepositoryError::Unauthorized);
        }
        let GameInput::Game(command) = &input else {
            return Err(RepositoryError::InputBinding);
        };
        let principal = self.principal(credential)?;
        if command.basis.session != self.session
            || command.member.as_bytes() != &principal
            || proof_binding == [0; 32]
        {
            return Err(RepositoryError::InputBinding);
        }
        let namespace = if principal == PLAYER {
            match &command.command {
                df_model::checkpoint::GameCommand::ProposeAction { action, .. }
                    if action.entry.as_str() == "join-room" =>
                {
                    b"local-room-join-v1".to_vec()
                }
                _ => return Err(RepositoryError::Unauthorized),
            }
        } else {
            b"local-room-journey-v1".to_vec()
        };
        let binding = LocalInputBinding {
            principal,
            operation: command.operation,
            epoch: command.basis.revision.epoch(),
            namespace,
            proof_binding,
            fingerprint,
            input,
            pins,
        };
        self.issue_bound(credential, binding)
    }

    /// Native-only completion admission. The executor supplies no client credential or role.
    /// A matching durable effect and current recipient grant are required independently;
    /// the engine still checks the source, recording and generation against current state.
    pub fn issue_completion(
        &mut self,
        current: &Checkpoint,
        completion: JobCompletion,
        member: MemberId,
        operation: OperationId,
        proof: ([u8; 32], [u8; 32]),
    ) -> Result<NativeScope<LocalDemoAuthority>, RepositoryError> {
        if current.state().mode != ExecutionMode::PreparedOnly
            || current.basis().session != self.session
            || completion.basis.session != self.session
            || completion.basis.run != current.basis().run
            || completion.basis.revision.epoch() != current.basis().revision.epoch()
            || operation == completion.operation
            || proof.0 == [0; 32]
            || member.as_bytes() == &PLAYER
            || member.as_bytes() == &DISPLAY
        {
            return Err(RepositoryError::InputBinding);
        }
        let intent = current
            .state()
            .intents
            .iter()
            .find(|intent| {
                intent.kind == EffectKind::RunAi
                    && intent.status == DurableStatus::Pending
                    && intent.job == Some(completion.job)
                    && intent.basis == completion.basis
                    && intent.operation == completion.operation
                    && intent.generation == completion.generation
            })
            .ok_or(RepositoryError::InputBinding)?;
        let principal = *member.as_bytes();
        // Read a server-owned active grant and original durable declaration. Neither an
        // inbox item nor a caller-provided checkpoint can fabricate this causal association.
        let credential: [u8; 32] = self.authority.runtime.block_on(async {
            let row = tokio::time::timeout(Duration::from_secs(2), self.authority.client()?.query_opt(
                "SELECT g.credential FROM df_local_demo.grants AS g JOIN df_game.intents AS i ON i.tenant_id=g.tenant_id AND i.session_id=g.session_id AND i.principal_id=g.principal_id WHERE g.tenant_id=$1::bytea AND g.session_id=$2::bytea AND g.principal_id=$3::bytea AND g.campaign_id=$4::bytea AND g.effective_role=$5::bytea AND g.active AND g.expires_at>clock_timestamp() AND i.effect_id=$6::bytea AND i.job_id=$7::bytea AND i.operation_id=$8::bytea AND i.run_id=$9::bytea AND i.process_generation=$10::text::numeric AND i.execution_mode=2 AND i.intent_kind=1 AND i.recovery_epoch=$11::text::numeric AND i.committed_epoch=$11::text::numeric AND i.committed_sequence=$12::text::numeric",
                &[&TENANT.as_slice(), &self.session.as_bytes().as_slice(), &principal.as_slice(), &CAMPAIGN.as_slice(), &b"player".as_slice(), &intent.id.as_bytes().as_slice(), &completion.job.as_bytes().as_slice(), &completion.operation.as_bytes().as_slice(), &completion.basis.run.as_bytes().as_slice(), &completion.generation.to_string(), &completion.basis.revision.epoch().get().to_string(), &completion.basis.revision.sequence().to_string()]
            )).await.map_err(|_| RepositoryError::Unavailable)?.map_err(|_| RepositoryError::Unavailable)?.ok_or(RepositoryError::Unauthorized)?;
            row.try_get::<_,Vec<u8>>("credential").map_err(|_| RepositoryError::Unauthorized)?.try_into().map_err(|_| RepositoryError::Unauthorized)
        })?;
        self.issue_bound(
            &credential,
            LocalInputBinding {
                principal,
                operation,
                epoch: completion.basis.revision.epoch(),
                namespace: b"local-courier-completion-v1".to_vec(),
                proof_binding: proof.0,
                fingerprint: proof.1,
                input: GameInput::Job(completion),
                pins: current.pins().clone(),
            },
        )
    }

    fn issue_bound(
        &mut self,
        credential: &[u8; 32],
        binding: LocalInputBinding,
    ) -> Result<NativeScope<LocalDemoAuthority>, RepositoryError> {
        let LocalInputBinding {
            principal,
            operation,
            epoch,
            namespace,
            proof_binding,
            fingerprint,
            input,
            pins,
        } = binding;
        if self.authenticate(credential)? != LocalDemoRole::Player
            || self.principal(credential)? != principal
            || input.retained_bytes().is_none_or(|bytes| bytes > 8192)
        {
            return Err(RepositoryError::InputBinding);
        }
        let role = LocalDemoRole::Player.bytes();
        let now = self.authority.runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(2),
                self.authority.client()?.query_one(
                    "SELECT extract(epoch from clock_timestamp())::bigint AS now",
                    &[],
                ),
            )
            .await
            .map_err(|_| RepositoryError::Unavailable)?
            .map_err(|_| RepositoryError::Unavailable)?
            .try_get::<_, i64>("now")
            .map_err(|_| RepositoryError::Unavailable)
        })?;
        let capability = authorize_membership(
            &mut self.authority,
            MembershipRequest {
                principal: &principal,
                tenant: &TENANT,
                campaign: &CAMPAIGN,
                role: &role,
            },
            u64::try_from(now).map_err(|_| RepositoryError::Unavailable)?,
        )
        .map_err(|error| match error {
            df_auth::membership::MembershipError::Source(error) => error,
            _ => RepositoryError::Unauthorized,
        })?;
        let revision = capability.revision().clone();
        let epoch_text = epoch.get().to_string();
        self.authority.runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), self.authority.client()?.execute(
                "INSERT INTO df_local_demo.scope_proofs (binding, credential, tenant_id, principal_id, campaign_id, effective_role, access_revision, session_id, operation_id, command_namespace, recovery_epoch, fingerprint_version, canonical_fingerprint, owner_fence, execution_mode, lookup_only) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,1,$12,$13,2,false)",
                &[&proof_binding.as_slice(), &credential.as_slice(), &TENANT.as_slice(), &principal.as_slice(), &CAMPAIGN.as_slice(), &role.as_slice(), &revision.as_slice(), &self.session.as_bytes().as_slice(), &operation.as_bytes().as_slice(), &namespace.as_slice(), &epoch_text, &fingerprint.as_slice(), &self.fence.as_slice()]))
                .await.map_err(|_| RepositoryError::Unavailable)?.map_err(|_| RepositoryError::Unavailable)?;
            Ok::<_,RepositoryError>(())
        })?;
        Ok(NativeScope {
            capability,
            mapping: AuthKeyMapping {
                tenant_bytes: |v| Ok(*v),
                principal_bytes: |v| Ok(*v),
                campaign_bytes: |v| Ok(v.to_vec()),
                role_bytes: |v| Ok(v.clone()),
                revision_bytes: |v| Ok(v.clone()),
                retained_capability_heap_bytes: |c| {
                    c.request()
                        .role
                        .capacity()
                        .checked_add(c.revision().capacity())
                },
            },
            binding: proof_binding.to_vec(),
            tenant: TENANT,
            principal,
            session: self.session,
            operation,
            namespace,
            epoch,
            fingerprint,
            owner_fence: self.fence,
            lookup_only: false,
            bound_input: input,
            pins,
            mode: ExecutionMode::PreparedOnly,
        })
    }
}

/// Seed one owned, empty local-development database before serving any client.
/// Existing sessions are refused: restarting must not overwrite committed gameplay.
/// Bounded initializer diagnostics contain fixed stage/class tags and no bound values.
#[derive(Debug)]
pub struct DemoInitializationError {
    pub stage: &'static str,
    pub class: RepositoryError,
}
pub async fn initialize(
    client: &mut Client,
    checkpoint: &Checkpoint,
    fence: [u8; 16],
    player_credential: [u8; 32],
    display_credential: [u8; 32],
    codec_limits: NativeCodecLimits,
) -> Result<(), DemoInitializationError> {
    initialize_with_adventure(
        client,
        checkpoint,
        fence,
        player_credential,
        display_credential,
        codec_limits,
        None,
    )
    .await
}
async fn initialize_with_adventure(
    client: &mut Client,
    checkpoint: &Checkpoint,
    fence: [u8; 16],
    player_credential: [u8; 32],
    display_credential: [u8; 32],
    codec_limits: NativeCodecLimits,
    adventure: Option<&DemoAdventure>,
) -> Result<(), DemoInitializationError> {
    if let Some(adventure) = adventure {
        adventure.validate_bounds()?;
    }
    if fence == [0; 16]
        || player_credential == [0; 32]
        || display_credential == [0; 32]
        || player_credential == display_credential
    {
        return Err(DemoInitializationError {
            stage: "configured_identity",
            class: RepositoryError::Unauthorized,
        });
    }
    let database: String = client
        .query_one("SELECT current_database()", &[])
        .await
        .map_err(|_| DemoInitializationError {
            stage: "database_identity_or_transaction",
            class: RepositoryError::Unavailable,
        })?
        .try_get(0)
        .map_err(|_| DemoInitializationError {
            stage: "database_identity_or_transaction",
            class: RepositoryError::Unavailable,
        })?;
    if !database.starts_with("df_gameplay_demo_") {
        return Err(DemoInitializationError {
            stage: "configured_identity",
            class: RepositoryError::Unauthorized,
        });
    }
    client
        .batch_execute(include_str!("../migrations/0001_game_decisions.sql"))
        .await
        .map_err(|_| DemoInitializationError {
            stage: "base_migration",
            class: RepositoryError::Unavailable,
        })?;
    let transaction = client
        .transaction()
        .await
        .map_err(|_| DemoInitializationError {
            stage: "local_transaction",
            class: RepositoryError::Unavailable,
        })?;
    transaction.batch_execute("CREATE SCHEMA df_local_demo; REVOKE ALL ON SCHEMA df_local_demo FROM PUBLIC;
        CREATE TABLE df_local_demo.grants (credential bytea PRIMARY KEY CHECK(octet_length(credential)=32), tenant_id bytea NOT NULL, principal_id bytea NOT NULL, campaign_id bytea NOT NULL, effective_role bytea NOT NULL, access_revision bytea NOT NULL, session_id bytea NOT NULL, active boolean NOT NULL, expires_at timestamptz NOT NULL);
        CREATE TABLE df_local_demo.room_grants (operation_id bytea PRIMARY KEY CHECK(octet_length(operation_id)=16), canonical_fingerprint bytea NOT NULL CHECK(octet_length(canonical_fingerprint)=32), principal_id bytea NOT NULL UNIQUE CHECK(octet_length(principal_id)=16), credential bytea NOT NULL UNIQUE REFERENCES df_local_demo.grants);
        CREATE TABLE df_local_demo.scope_proofs (binding bytea PRIMARY KEY CHECK(octet_length(binding)=32), credential bytea REFERENCES df_local_demo.grants, tenant_id bytea NOT NULL, principal_id bytea NOT NULL, campaign_id bytea NOT NULL, effective_role bytea NOT NULL, access_revision bytea NOT NULL, session_id bytea NOT NULL, operation_id bytea NOT NULL, command_namespace bytea NOT NULL, recovery_epoch text NOT NULL, fingerprint_version integer NOT NULL, canonical_fingerprint bytea NOT NULL, owner_fence bytea NOT NULL, execution_mode smallint NOT NULL, lookup_only boolean NOT NULL);").await.map_err(|_| DemoInitializationError { stage:"local_schema",class:RepositoryError::Unavailable })?;
    transaction.batch_execute("CREATE TABLE df_local_demo.rejected_operations (tenant_id bytea NOT NULL, session_id bytea NOT NULL, principal_id bytea NOT NULL, command_namespace bytea NOT NULL, recovery_epoch numeric(20,0) NOT NULL, operation_id bytea NOT NULL, fingerprint_version integer NOT NULL CHECK(fingerprint_version=1), canonical_fingerprint bytea NOT NULL CHECK(octet_length(canonical_fingerprint)=32), checkpoint_epoch numeric(20,0) NOT NULL, checkpoint_sequence numeric(20,0) NOT NULL, typed_receipt_version integer NOT NULL CHECK(typed_receipt_version=1), typed_receipt bytea CHECK(octet_length(typed_receipt)<=4096), PRIMARY KEY(tenant_id,session_id,principal_id,command_namespace,recovery_epoch,operation_id), FOREIGN KEY(tenant_id,session_id,checkpoint_epoch,checkpoint_sequence) REFERENCES df_game.checkpoints(tenant_id,session_id,recovery_epoch,in_epoch_sequence));").await.map_err(|_|DemoInitializationError { stage:"rejected_schema",class:RepositoryError::Unavailable })?;
    transaction
        .batch_execute(include_str!("../migrations/0002_local_room_reclaim.sql"))
        .await
        .map_err(|_| DemoInitializationError {
            stage: "seat_schema",
            class: RepositoryError::Unavailable,
        })?;
    if let Some(adventure) = adventure {
        adventure::insert(&transaction, adventure).await?;
    }
    let basis = checkpoint.basis();
    let epoch = basis.revision.epoch().get().to_string();
    let sequence = basis.revision.sequence().to_string();
    transaction.execute(&format!("INSERT INTO df_game.sessions VALUES ($1::bytea,$2::bytea,$3::bytea,$4::text::numeric,$5::text::numeric,$6::bytea,clock_timestamp()+interval '{} seconds')", lease_seconds()), &[&TENANT.as_slice(), &basis.session.as_bytes().as_slice(), &basis.run.as_bytes().as_slice(), &epoch, &sequence, &fence.as_slice()]).await.map_err(|_| DemoInitializationError { stage:"session_seed",class:RepositoryError::InputBinding })?;
    let bytes =
        encode_checkpoint(checkpoint, codec_limits).map_err(|_| DemoInitializationError {
            stage: "checkpoint_encoding",
            class: RepositoryError::InvalidCandidate,
        })?;
    let schema = i32::from(checkpoint.schema());
    let checkpoint_codec =
        crate::checkpoint_codec::checkpoint_storage_version(&bytes).map_err(|_| {
            DemoInitializationError {
                stage: "checkpoint_encoding",
                class: RepositoryError::InvalidCandidate,
            }
        })?;
    transaction
        .execute(
            crate::sql::INSERT_CHECKPOINT,
            &[
                &TENANT.as_slice(),
                &basis.session.as_bytes().as_slice(),
                &epoch,
                &sequence,
                &basis.run.as_bytes().as_slice(),
                &schema,
                &checkpoint_codec,
                &bytes,
            ],
        )
        .await
        .map_err(|_| DemoInitializationError {
            stage: "checkpoint_seed",
            class: RepositoryError::Unavailable,
        })?;
    for (credential, principal, role) in [
        (player_credential, PLAYER, LocalDemoRole::Player),
        (display_credential, DISPLAY, LocalDemoRole::Display),
    ] {
        transaction.execute(&format!("INSERT INTO df_local_demo.grants VALUES ($1,$2,$3,$4,$5,$6,$7,true,clock_timestamp()+interval '{} seconds')", lease_seconds()), &[&credential.as_slice(), &TENANT.as_slice(), &principal.as_slice(), &CAMPAIGN.as_slice(), &role.bytes(), &b"local-demo-access-1".as_slice(), &basis.session.as_bytes().as_slice()]).await.map_err(|_| DemoInitializationError { stage:"grant_seed",class:RepositoryError::Unavailable })?;
    }
    transaction
        .commit()
        .await
        .map_err(|_| DemoInitializationError {
            stage: "seed_commit",
            class: RepositoryError::UnresolvedCommit,
        })?;
    Ok(())
}

/// Encode a checkpoint owned by the finite native demo for its private acceptance audit.
/// This is never a public RPC view or an authorization constructor.
pub fn encode_owned_demo_checkpoint(
    checkpoint: &Checkpoint,
    limits: NativeCodecLimits,
) -> Result<Vec<u8>, RepositoryError> {
    encode_checkpoint(checkpoint, limits).map_err(|_| RepositoryError::InvalidCandidate)
}

/// Decode an owned demo checkpoint for an offline recovery audit. The same bounded
/// canonical decoder enforces the expected basis, every pin and the admitted inventory.
/// This creates no database/session authority and performs no I/O.
pub fn decode_owned_demo_checkpoint(
    document: &[u8],
    expected: df_model::checkpoint::Basis,
    pins: &CheckpointPins,
    recovery: &crate::NativeRecoverySource,
    limits: NativeCodecLimits,
) -> Result<Checkpoint, RepositoryError> {
    crate::checkpoint_codec::decode_checkpoint(
        document,
        expected,
        pins,
        df_model::checkpoint::ReferenceInventory {
            rules: &recovery.rules,
            content: &recovery.content,
            resources: &recovery.resources,
            assets: &recovery.assets,
        },
        recovery.limits,
        limits,
    )
    .map_err(|_| RepositoryError::InvalidCandidate)
}

/// Decode a stored receipt for the owned native gameplay qualification only.
/// The canonical codec validates its version and expected session and operation.
pub fn decode_owned_demo_receipt(
    document: &[u8],
    session: SessionId,
    operation: OperationId,
    limits: NativeCodecLimits,
) -> Result<df_session::submission::DecisionReceipt, RepositoryError> {
    if document.len() > 4096 {
        return Err(RepositoryError::InvalidReceipt);
    }
    crate::checkpoint_codec::decode_receipt(document, session, operation, 4096, limits)
        .map_err(|_| RepositoryError::InvalidReceipt)
}

/// Operator-only loopback qualification on already owned sockets. No bind/connect,
/// scope authority, transaction replay, or normal gameplay fault control is supplied.
pub use crate::owned_pg_commit_proxy_fixture::{
    CommitAckObservation, ProxyBounds, ProxyError, drop_commit_acknowledgement,
};
