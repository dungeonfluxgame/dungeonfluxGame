//! Couch-room credential transfer. All calls run on the existing session owner.
use super::*;
use df_persistence::local_demo_scope::{
    LocalDemoScopeIssuer, LocalReclaimedSeat, LocalRoomSeat, LocalSeatReclaim, SEAT_AWAY_MILLIS,
    SeatReclaimRefusal,
};
use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Anchor durable activity timestamps once, then use the owner's monotonic elapsed clock.
/// Tests supply an explicit Instant and timestamp; gameplay/logical time is never advanced.
pub(in crate::gameplay) struct SeatClock {
    origin: Instant,
    unix_millis: Result<i64, RepositoryError>,
    pub(in crate::gameplay) next_poll: Instant,
    activity: HashMap<[u8; 32], SeatActivity>,
}

struct SeatActivity {
    last_seen: Instant,
    last_write: Instant,
}
impl Default for SeatClock {
    fn default() -> Self {
        let origin = Instant::now();
        Self {
            origin,
            unix_millis: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| RepositoryError::Unavailable)
                .and_then(|time| {
                    i64::try_from(time.as_millis()).map_err(|_| RepositoryError::Capacity)
                }),
            next_poll: origin,
            activity: HashMap::new(),
        }
    }
}
impl SeatClock {
    fn activity_write_due(&mut self, credential: &[u8; 32], now: Instant) -> bool {
        let Some(activity) = self.activity.get_mut(credential) else {
            self.activity.insert(
                *credential,
                SeatActivity {
                    last_seen: now,
                    last_write: now,
                },
            );
            return true;
        };
        let reactivated = now.saturating_duration_since(activity.last_seen)
            >= Duration::from_millis(SEAT_AWAY_MILLIS as u64);
        let debounce_elapsed =
            now.saturating_duration_since(activity.last_write) >= Duration::from_secs(5);
        if reactivated {
            actor::record_audit_line(
                "seat.returned",
                format!(
                    "SEAT returned seat={} away_s={}",
                    df_observe::seat_hash(credential),
                    now.saturating_duration_since(activity.last_seen).as_secs()
                ),
            );
        }
        activity.last_seen = now;
        if reactivated || debounce_elapsed {
            activity.last_write = now;
            true
        } else {
            false
        }
    }

    pub(in crate::gameplay) fn millis(&self, now: Instant) -> Result<i64, RepositoryError> {
        let elapsed = now
            .checked_duration_since(self.origin)
            .ok_or(RepositoryError::InputBinding)?;
        self.unix_millis?
            .checked_add(i64::try_from(elapsed.as_millis()).map_err(|_| RepositoryError::Capacity)?)
            .ok_or(RepositoryError::Capacity)
    }
}

pub(in crate::gameplay) struct AwayHero {
    pub(in crate::gameplay) member: [u8; 16],
    pub(in crate::gameplay) name: String,
    pub(in crate::gameplay) token: [u8; 32],
}

const RECOVERY_PROOF_PREFIX: &[u8; 8] = b"DFREJOIN";

fn recovery_proof(hero: &AwayHero) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"local-room-tv-recovery-v1");
    hash.update(hero.member);
    hash.update(hero.token);
    let digest = hash.finalize();
    let mut proof = [0; 32];
    proof[..8].copy_from_slice(RECOVERY_PROOF_PREFIX);
    proof[8..24].copy_from_slice(&hero.member);
    proof[24..].copy_from_slice(&digest[..8]);
    proof
}

pub(super) fn listing(away: Vec<AwayHero>, table_display: bool) -> Vec<rpc::AwayHero> {
    away.into_iter()
        .map(|hero| rpc::AwayHero {
            seat_token: if table_display {
                recovery_proof(&hero).to_vec()
            } else {
                // Public seat identifiers carry no authority; #1273 exposed the raw token here.
                hero.member.to_vec()
            },
            name: hero.name,
        })
        .collect()
}

struct RecoveryAuthority<'a, A> {
    issuer: &'a mut A,
    away: &'a [AwayHero],
}
impl<A: SeatAuthority> SeatAuthority for RecoveryAuthority<'_, A> {
    fn reclaim(
        &mut self,
        mut request: LocalSeatReclaim,
    ) -> Result<Result<LocalReclaimedSeat, SeatReclaimRefusal>, RepositoryError> {
        if request.token.starts_with(RECOVERY_PROOF_PREFIX)
            && let Some(hero) = self
                .away
                .iter()
                .find(|hero| recovery_proof(hero) == request.token)
        {
            request.token = hero.token;
        }
        // The fingerprint remains the submitted proof, so a lost reply replays even after
        // rotation removes this hero from the away list. The durable authority checks replay first.
        self.issuer.reclaim(request)
    }
}

pub(in crate::gameplay) struct ReclaimRequest {
    pub(in crate::gameplay) room_code: String,
    pub(in crate::gameplay) operation: df_types::OperationId,
    pub(in crate::gameplay) token: [u8; 32],
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(in crate::gameplay) enum ReclaimRefusal {
    WrongRoomCode,
    Seat(SeatReclaimRefusal),
}

// The test port exercises the same admission and response mapping without starting PostgreSQL.
trait SeatAuthority {
    fn reclaim(
        &mut self,
        request: LocalSeatReclaim,
    ) -> Result<Result<LocalReclaimedSeat, SeatReclaimRefusal>, RepositoryError>;
}
impl SeatAuthority for LocalDemoScopeIssuer {
    fn reclaim(
        &mut self,
        request: LocalSeatReclaim,
    ) -> Result<Result<LocalReclaimedSeat, SeatReclaimRefusal>, RepositoryError> {
        self.reclaim_room_seat(request)
    }
}

fn fingerprint(request: &ReclaimRequest) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"local-room-reclaim-v1");
    hash.update((request.room_code.len() as u64).to_le_bytes());
    hash.update(request.room_code.as_bytes());
    hash.update(request.operation.as_bytes());
    hash.update(request.token);
    hash.finalize().into()
}

fn admit_reclaim(
    issuer: &mut impl SeatAuthority,
    request: ReclaimRequest,
    correct_room: bool,
    current: &df_model::checkpoint::Checkpoint,
    now_millis: i64,
    new_credential: [u8; 32],
    next_token: [u8; 32],
) -> Result<Result<rpc::JoinedRoom, ReclaimRefusal>, RepositoryError> {
    if !correct_room {
        return Ok(Err(ReclaimRefusal::WrongRoomCode));
    }
    let grant = issuer.reclaim(LocalSeatReclaim {
        fingerprint: fingerprint(&request),
        operation: request.operation,
        token: request.token,
        new_credential,
        next_token,
        now_millis,
        basis: current.basis(),
    })?;
    let grant = match grant {
        Ok(grant) => grant,
        Err(reason) => return Ok(Err(ReclaimRefusal::Seat(reason))),
    };
    Ok(Ok(rpc::JoinedRoom {
        receipt: Some(wire::receipt(
            grant.basis,
            request.operation,
            rpc::decision_receipt::Outcome::Accepted(Box::new(rpc::AcceptedAction {
                // The grant receipt records identity admission, not a new game decision.
                phase: rpc::JourneyPhase::Room as i32,
                ..Default::default()
            })),
            grant.replayed,
        )),
        local_binding: journey::hex(&grant.credential),
        client_binding_id: Some(rpc::ClientBindingId {
            value: Some(grant.member.to_vec()),
        }),
        session_id: Some(rpc::SessionId {
            value: Some(grant.basis.session.as_bytes().to_vec()),
        }),
        run_id: Some(rpc::RunId {
            value: Some(grant.basis.run.as_bytes().to_vec()),
        }),
    }))
}

fn hero_name(
    current: &df_model::checkpoint::Checkpoint,
    member: &[u8; 16],
) -> Result<String, RepositoryError> {
    let link = journey::participants(current)
        .into_iter()
        .find(|link| link.member.as_bytes() == member)
        .ok_or(RepositoryError::Unauthorized)?;
    let entity = link.character.ok_or_else(|| {
        crate::gameplay::refusal_detail::note("reclaim-hero-missing");
        RepositoryError::InvalidCandidate
    })?;
    if current
        .state()
        .characters
        .iter()
        .any(|character| character.entity == entity)
    {
        journey::name(current.state(), entity)
    } else {
        Ok("Creating a hero…".to_owned())
    }
}

fn seat_event(line: &str, event: &'static str) {
    actor::record_audit_line(event, line.to_owned());
}

impl actor::Actor {
    pub(in crate::gameplay) fn seat_activity(
        &mut self,
        credential: &[u8; 32],
    ) -> Result<(), Status> {
        let instant = Instant::now();
        if !self
            .session
            .seat_clock
            .activity_write_due(credential, instant)
        {
            return Ok(());
        }
        let now = self
            .session
            .seat_clock
            .millis(instant)
            .map_err(actor::unavailable)?;
        if let Err(error) = self.issuer.touch_room_seat(credential, now) {
            seat_event(
                &format!("SEAT activity write failed reason={error:?}"),
                "seat.activity_write_failed",
            );
        }
        Ok(())
    }

    fn collect_away(&mut self, now: i64) -> Result<Vec<AwayHero>, Status> {
        let members: Vec<_> = journey::participants(self.owner.checkpoint())
            .into_iter()
            .map(|link| *link.member.as_bytes())
            .collect();
        for member in members {
            self.issuer
                .register_room_seat(member, model::random().map_err(actor::unavailable)?, now)
                .map_err(actor::unavailable)?;
        }
        self.issuer
            .away_room_seats(now)
            .map_err(actor::unavailable)?
            .into_iter()
            .map(|seat: LocalRoomSeat| {
                let name =
                    hero_name(self.owner.checkpoint(), &seat.member).map_err(actor::unavailable)?;
                if seat.newly_away {
                    seat_event(
                        &format!(
                            "SEAT away member_hash={} silent_s={}",
                            df_observe::seat_hash(&seat.member),
                            (now - seat.last_seen_millis) / 1000
                        ),
                        "seat.away",
                    );
                }
                Ok(AwayHero {
                    member: seat.member,
                    name,
                    token: seat.token,
                })
            })
            .collect()
    }

    pub(in crate::gameplay) fn poll_away_seats(&mut self, now: Instant) {
        if now < self.session.seat_clock.next_poll {
            return;
        }
        self.session.seat_clock.next_poll = now + Duration::from_secs(5);
        if self.fenced || self.session_lost() {
            return;
        }
        let outcome = self
            .session
            .seat_clock
            .millis(now)
            .map_err(actor::unavailable)
            .and_then(|now| self.collect_away(now));
        if let Err(error) = outcome {
            seat_event(
                &format!("SEAT activity unavailable reason={:?}", error.code()),
                "seat.activity_unavailable",
            );
        }
    }

    pub(in crate::gameplay) fn away_seats(
        &mut self,
        room_code: &str,
    ) -> Result<Vec<AwayHero>, Status> {
        actor::require_readable(self.fenced)?;
        if !matches_room_code(room_code) {
            return Err(Status::permission_denied("room code denied"));
        }
        let now = self
            .session
            .seat_clock
            .millis(Instant::now())
            .map_err(actor::unavailable)?;
        self.collect_away(now)
    }

    pub(in crate::gameplay) fn reclaim_seat(
        &mut self,
        request: ReclaimRequest,
    ) -> Result<Result<rpc::JoinedRoom, ReclaimRefusal>, Status> {
        actor::require_readable(self.fenced)?;
        if self.session_lost() {
            return Err(actor::session_lost_status());
        }
        let correct_room = matches_room_code(&request.room_code);
        let now = self
            .session
            .seat_clock
            .millis(Instant::now())
            .map_err(actor::unavailable)?;
        let away = if correct_room {
            self.collect_away(now)?
        } else {
            Vec::new()
        };
        let mut authority = RecoveryAuthority {
            issuer: &mut self.issuer,
            away: &away,
        };
        let answer = admit_reclaim(
            &mut authority,
            request,
            correct_room,
            self.owner.checkpoint(),
            now,
            model::random().map_err(actor::unavailable)?,
            model::random().map_err(actor::unavailable)?,
        )
        .map_err(actor::unavailable)?;
        match &answer {
            Ok(joined) => {
                let member: [u8; 16] = joined
                    .client_binding_id
                    .as_ref()
                    .and_then(|id| id.value.as_ref())
                    .ok_or_else(|| Status::internal("reclaim member missing"))?
                    .as_slice()
                    .try_into()
                    .map_err(|_| Status::internal("reclaim member invalid"))?;
                if !joined
                    .receipt
                    .as_ref()
                    .is_some_and(|receipt| receipt.replayed)
                {
                    let _name =
                        hero_name(self.owner.checkpoint(), &member).map_err(actor::unavailable)?;
                    seat_event(
                        &format!(
                            "SEAT reclaimed seat={}",
                            crate::gameplay::parse_binding(&joined.local_binding).map_or_else(
                                || "none".to_owned(),
                                |credential| df_observe::seat_hash(&credential).to_string()
                            )
                        ),
                        "seat.reclaimed",
                    );
                    self.recovery_wakeup
                        .send_replace(self.owner.checkpoint().clone());
                }
            }
            Err(reason) => seat_event(
                &format!("SEAT reclaim refused reason={reason:?}"),
                "seat.reclaim_refused",
            ),
        }
        Ok(answer)
    }
}

#[cfg(test)]
mod tests;
