//! Durable, local-room seat transfer; no checkpoint or hero is changed.
use super::*;
use df_model::checkpoint::Basis;

pub const SEAT_AWAY_MILLIS: i64 = 30_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SeatReclaimRefusal {
    NotAway,
    UnknownToken,
    OperationConflict,
}

/// Native-only projection. The public room projection sends only hero name and token.
pub struct LocalRoomSeat {
    pub member: [u8; 16],
    pub token: [u8; 32],
    pub last_seen_millis: i64,
    pub newly_away: bool,
}

/// An immutable successful receipt; deliberately excludes Debug for credential secrecy.
#[derive(Clone, Eq, PartialEq)]
pub struct LocalReclaimedSeat {
    pub member: [u8; 16],
    pub credential: [u8; 32],
    pub basis: Basis,
    pub replayed: bool,
}

pub struct LocalSeatReclaim {
    pub operation: OperationId,
    pub fingerprint: [u8; 32],
    pub token: [u8; 32],
    pub new_credential: [u8; 32],
    pub next_token: [u8; 32],
    pub now_millis: i64,
    pub basis: Basis,
}

fn require_away(last_seen_millis: i64, now_millis: i64) -> Result<(), SeatReclaimRefusal> {
    if now_millis.saturating_sub(last_seen_millis) < SEAT_AWAY_MILLIS {
        Err(SeatReclaimRefusal::NotAway)
    } else {
        Ok(())
    }
}

impl LocalDemoScopeIssuer {
    /// Register an existing committed join once. Restore retains the current replacement
    /// credential, token and activity instead of reviving the original grant.
    pub fn register_room_seat(
        &mut self,
        member: [u8; 16],
        token: [u8; 32],
        now_millis: i64,
    ) -> Result<(), RepositoryError> {
        if member == PLAYER || member == DISPLAY || token == [0; 32] || now_millis < 0 {
            return Err(RepositoryError::InputBinding);
        }
        let runtime = self.authority.runtime.clone();
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), self.authority.client()?.execute(
                "INSERT INTO df_local_demo.room_seats (session_id,principal_id,credential,seat_token,last_seen_millis) SELECT g.session_id,g.principal_id,g.credential,$3::bytea,$4::bigint FROM df_local_demo.room_grants j JOIN df_local_demo.grants g ON g.credential=j.credential WHERE g.session_id=$1::bytea AND g.principal_id=$2::bytea AND g.active ON CONFLICT (session_id,principal_id) DO NOTHING",
                &[&self.session.as_bytes().as_slice(), &member.as_slice(), &token.as_slice(), &now_millis],
            )).await.map_err(|_| RepositoryError::Unavailable)?
                .map_err(|_| RepositoryError::Unavailable)?;
            Ok(())
        })
    }

    /// Record only an authenticated current binding. Revoked phones cannot keep a seat live.
    pub fn touch_room_seat(
        &mut self,
        credential: &[u8; 32],
        now_millis: i64,
    ) -> Result<(), RepositoryError> {
        let runtime = self.authority.runtime.clone();
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), self.authority.client()?.execute(
                "UPDATE df_local_demo.room_seats s SET last_seen_millis=GREATEST(s.last_seen_millis,$3::bigint),away_announced=false FROM df_local_demo.grants g WHERE s.session_id=$1::bytea AND s.credential=$2::bytea AND g.credential=s.credential AND g.active AND g.expires_at>clock_timestamp()",
                &[&self.session.as_bytes().as_slice(), &credential.as_slice(), &now_millis],
            )).await.map_err(|_| RepositoryError::Unavailable)?
                .map_err(|_| RepositoryError::Unavailable)?;
            Ok(())
        })
    }

    /// Mark each away transition once, returning native-only seat identities for projection.
    pub fn away_room_seats(
        &mut self,
        now_millis: i64,
    ) -> Result<Vec<LocalRoomSeat>, RepositoryError> {
        let runtime = self.authority.runtime.clone();
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), async {
                let tx = self.authority.client_mut()?.transaction().await
                    .map_err(|_| RepositoryError::Unavailable)?;
                let rows = tx.query(
                    "SELECT principal_id,seat_token,last_seen_millis,away_announced FROM df_local_demo.room_seats WHERE session_id=$1::bytea AND last_seen_millis<=$2::bigint FOR UPDATE",
                    &[&self.session.as_bytes().as_slice(), &now_millis.saturating_sub(SEAT_AWAY_MILLIS)],
                ).await.map_err(|_| RepositoryError::Unavailable)?;
                let mut seats = Vec::new();
                for row in rows {
                    seats.push(LocalRoomSeat {
                        member: bytes(&row, "principal_id")?,
                        token: bytes(&row, "seat_token")?,
                        last_seen_millis: row.try_get("last_seen_millis").map_err(|_| RepositoryError::InvalidReceipt)?,
                        newly_away: !row.try_get::<_, bool>("away_announced").map_err(|_| RepositoryError::InvalidReceipt)?,
                    });
                }
                tx.execute("UPDATE df_local_demo.room_seats SET away_announced=true WHERE session_id=$1::bytea AND last_seen_millis<=$2::bigint AND NOT away_announced",
                    &[&self.session.as_bytes().as_slice(), &now_millis.saturating_sub(SEAT_AWAY_MILLIS)],
                ).await.map_err(|_| RepositoryError::Unavailable)?;
                tx.commit().await.map_err(|_| RepositoryError::UnresolvedCommit)?;
                Ok(seats)
            }).await.map_err(|_| RepositoryError::UnresolvedCommit)?
        })
    }

    /// Same session-row serialization/fence as Join. A successful receipt, replacement
    /// grant, revocation and token rotation commit together. Exact retries are lookup first.
    pub fn reclaim_room_seat(
        &mut self,
        request: LocalSeatReclaim,
    ) -> Result<Result<LocalReclaimedSeat, SeatReclaimRefusal>, RepositoryError> {
        if request.basis.session != self.session
            || request.new_credential == [0; 32]
            || request.next_token == [0; 32]
            || request.next_token == request.token
            || request.now_millis < 0
        {
            return Err(RepositoryError::InputBinding);
        }
        let runtime = self.authority.runtime.clone();
        let session = self.session;
        let fence = self.fence;
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), async {
                let tx = self.authority.client_mut()?.transaction().await
                    .map_err(|_| RepositoryError::Unavailable)?;
                tx.batch_execute("SET LOCAL statement_timeout='2000ms'; SET LOCAL lock_timeout='1000ms'")
                    .await.map_err(|_| RepositoryError::Unavailable)?;
                let locked = tx.query_opt(crate::sql::LOCK_SESSION,
                    &[&TENANT.as_slice(), &session.as_bytes().as_slice()])
                    .await.map_err(|_| RepositoryError::Unavailable)?
                    .ok_or(RepositoryError::Unauthorized)?;
                if bytes::<16>(&locked, "owner_fence")? != fence {
                    return Err(RepositoryError::StaleFence);
                }
                if !locked.try_get::<_, bool>("lease_current").map_err(|_| RepositoryError::InvalidReceipt)? {
                    return Err(RepositoryError::ExpiredOwner);
                }
                if crate::decision_rows::current_row_basis(&locked, session)? != request.basis {
                    return Err(RepositoryError::RevisionConflict);
                }
                let key: &[&(dyn tokio_postgres::types::ToSql + Sync)] = &[
                    &session.as_bytes().as_slice(), &request.operation.as_bytes().as_slice(),
                ];
                if let Some(row) = tx.query_opt(
                    "SELECT canonical_fingerprint,principal_id,credential,run_id,recovery_epoch::text,in_epoch_sequence::text FROM df_local_demo.seat_reclaims WHERE session_id=$1::bytea AND operation_id=$2::bytea",
                    key,
                ).await.map_err(|_| RepositoryError::Unavailable)? {
                    if bytes::<32>(&row, "canonical_fingerprint")? != request.fingerprint {
                        return Ok(Err(SeatReclaimRefusal::OperationConflict));
                    }
                    let receipt = LocalReclaimedSeat {
                        member: bytes(&row, "principal_id")?,
                        credential: bytes(&row, "credential")?,
                        basis: crate::decision_rows::current_row_basis(&row, session)?,
                        replayed: true,
                    };
                    tx.commit().await.map_err(|_| RepositoryError::UnresolvedCommit)?;
                    return Ok(Ok(receipt));
                }
                let Some(seat) = tx.query_opt(
                    "SELECT principal_id,credential,last_seen_millis FROM df_local_demo.room_seats WHERE session_id=$1::bytea AND seat_token=$2::bytea FOR UPDATE",
                    &[&session.as_bytes().as_slice(), &request.token.as_slice()],
                ).await.map_err(|_| RepositoryError::Unavailable)? else {
                    return Ok(Err(SeatReclaimRefusal::UnknownToken));
                };
                let last_seen = seat.try_get("last_seen_millis").map_err(|_| RepositoryError::InvalidReceipt)?;
                if let Err(reason) = require_away(last_seen, request.now_millis) {
                    return Ok(Err(reason));
                }
                let member: [u8; 16] = bytes(&seat, "principal_id")?;
                let old_credential: [u8; 32] = bytes(&seat, "credential")?;
                if request.new_credential == old_credential {
                    return Err(RepositoryError::InputBinding);
                }
                let written = tx.execute(
                    "INSERT INTO df_local_demo.grants SELECT $1::bytea,$2::bytea,$3::bytea,$4::bytea,$5::bytea,$6::bytea,$7::bytea,true,lease_until FROM df_game.sessions WHERE tenant_id=$2::bytea AND session_id=$7::bytea AND owner_fence=$8::bytea AND lease_until>clock_timestamp()",
                    &[&request.new_credential.as_slice(), &TENANT.as_slice(), &member.as_slice(), &CAMPAIGN.as_slice(), &b"player".as_slice(), &b"local-demo-access-1".as_slice(), &session.as_bytes().as_slice(), &fence.as_slice()],
                ).await.map_err(|_| RepositoryError::Unavailable)?;
                if written != 1 { return Err(RepositoryError::ExpiredOwner); }
                tx.execute("UPDATE df_local_demo.grants SET active=false WHERE credential=$1::bytea",
                    &[&old_credential.as_slice()]).await.map_err(|_| RepositoryError::Unavailable)?;
                tx.execute("UPDATE df_local_demo.room_seats SET credential=$3::bytea,seat_token=$4::bytea,last_seen_millis=$5::bigint,away_announced=false WHERE session_id=$1::bytea AND principal_id=$2::bytea",
                    &[&session.as_bytes().as_slice(), &member.as_slice(), &request.new_credential.as_slice(), &request.next_token.as_slice(), &request.now_millis],
                ).await.map_err(|_| RepositoryError::Unavailable)?;
                tx.execute("INSERT INTO df_local_demo.seat_reclaims (session_id,operation_id,canonical_fingerprint,principal_id,credential,run_id,recovery_epoch,in_epoch_sequence) VALUES ($1::bytea,$2::bytea,$3::bytea,$4::bytea,$5::bytea,$6::bytea,$7::text::numeric,$8::text::numeric)",
                    &[&session.as_bytes().as_slice(), &request.operation.as_bytes().as_slice(), &request.fingerprint.as_slice(), &member.as_slice(), &request.new_credential.as_slice(), &request.basis.run.as_bytes().as_slice(), &request.basis.revision.epoch().get().to_string(), &request.basis.revision.sequence().to_string()],
                ).await.map_err(|_| RepositoryError::Unavailable)?;
                tx.commit().await.map_err(|_| RepositoryError::UnresolvedCommit)?;
                Ok(Ok(LocalReclaimedSeat { member, credential: request.new_credential, basis: request.basis, replayed: false }))
            }).await.map_err(|_| RepositoryError::UnresolvedCommit)?
        })
    }
}

fn bytes<const N: usize>(
    row: &tokio_postgres::Row,
    column: &str,
) -> Result<[u8; N], RepositoryError> {
    row.try_get::<_, Vec<u8>>(column)
        .map_err(|_| RepositoryError::InvalidReceipt)?
        .try_into()
        .map_err(|_| RepositoryError::InvalidReceipt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seat_reclaim_requires_thirty_seconds_of_silence() {
        assert_eq!(require_away(1000, 30_999), Err(SeatReclaimRefusal::NotAway));
        assert_eq!(require_away(1000, 31_000), Ok(()));
        assert_eq!(require_away(1000, 31_001), Ok(()));
        assert_eq!(
            require_away(31_000, 31_001),
            Err(SeatReclaimRefusal::NotAway)
        );
        assert_eq!(require_away(31_000, 1000), Err(SeatReclaimRefusal::NotAway));
    }
}
