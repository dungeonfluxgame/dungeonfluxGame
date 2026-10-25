//! Display pairing in the fake campaign directory (TODO(S1)): the TV asks for a code, the host
//! looks it up and confirms it in the lobby, and the TV redeems the approval once for a ticket.

use df_account_app::wire_campaign::{
    DisplayChallenge, DisplayRedeemed, DisplayRenewed, DisplayState, LobbyView, PairingLookup,
    Reason,
};
use df_customer::DeviceLabel;
use df_types::SessionId;

use super::{
    campaigns::{PairState, Pairing, State, Ticket, device_kind, fresh_code},
    campaigns_rules::because,
    fresh_hex,
};
use crate::app_api::{
    ids::hex,
    ports::{
        Actor, DeviceSecret, IssuedChallenge, PortError, PortResult, RedeemedTicket, RenewedTicket,
    },
};

const CHALLENGE_LIFETIME_MS: u64 = 10 * 60 * 1000;
const POLL_SECONDS: u32 = 2;
const TICKET_LIFETIME_MS: u64 = 12 * 60 * 60 * 1000;
const MAXIMUM_DISPLAYS_PER_SESSION: usize = 4;
const MAXIMUM_PAIRINGS: usize = 64;

fn secret(max_age_ms: u64, value: String) -> DeviceSecret {
    DeviceSecret {
        secret: value,
        max_age_s: max_age_ms / 1000,
    }
}

impl State {
    pub(super) fn issue_challenge(&mut self, device: DeviceLabel, now_ms: u64) -> IssuedChallenge {
        if self.pairings.len() >= MAXIMUM_PAIRINGS {
            self.pairings.remove(0);
        }
        let pairing = Pairing {
            id: format!("pair-{}", fresh_hex()),
            user_code: fresh_code(),
            device_secret: fresh_hex() + &fresh_hex(),
            device: device_kind(device),
            created_at_ms: now_ms,
            expires_at_ms: now_ms + CHALLENGE_LIFETIME_MS,
            state: PairState::Pending,
            ticket_id: None,
        };
        let issued = IssuedChallenge {
            challenge: DisplayChallenge {
                user_code: pairing.user_code.clone(),
                expires_at_ms: pairing.expires_at_ms,
                poll_seconds: POLL_SECONDS,
            },
            device: secret(TICKET_LIFETIME_MS, pairing.device_secret.clone()),
        };
        self.pairings.push(pairing);
        issued
    }

    fn pairing_by_secret(&mut self, device_secret: &str) -> PortResult<&mut Pairing> {
        self.pairings
            .iter_mut()
            .find(|pairing| pairing.device_secret == device_secret)
            .ok_or(PortError::NotFound)
    }

    pub(super) fn pairing_status(
        &mut self,
        device_secret: &str,
        now_ms: u64,
    ) -> PortResult<DisplayState> {
        let pairing = self.pairing_by_secret(device_secret)?;
        Ok(match pairing.state {
            PairState::Pending if pairing.expires_at_ms <= now_ms => DisplayState::Expired,
            PairState::Pending => DisplayState::Pending,
            PairState::Approved { .. } | PairState::Redeemed { .. } => DisplayState::Approved,
            PairState::Cancelled => DisplayState::Cancelled,
        })
    }

    /// Issues the ticket once; repeating after a lost answer re-issues the same ticket.
    pub(super) fn redeem_pairing(
        &mut self,
        device_secret: &str,
        now_ms: u64,
    ) -> PortResult<RedeemedTicket> {
        let pairing = self.pairing_by_secret(device_secret)?;
        let (session, ticket, expires_at_ms) = match &pairing.state {
            PairState::Approved { session } => (
                *session,
                fresh_hex() + &fresh_hex(),
                now_ms + TICKET_LIFETIME_MS,
            ),
            PairState::Redeemed {
                session,
                ticket,
                expires_at_ms,
            } if *expires_at_ms > now_ms => (*session, ticket.clone(), *expires_at_ms),
            _ => return Err(because(Reason::NoLongerValid)),
        };
        pairing.state = PairState::Redeemed {
            session,
            ticket: ticket.clone(),
            expires_at_ms,
        };
        Ok(RedeemedTicket {
            redeemed: DisplayRedeemed {
                session_id: hex(session.as_bytes()),
                ticket_expires_at_ms: expires_at_ms,
            },
            ticket: secret(expires_at_ms.saturating_sub(now_ms), ticket),
        })
    }

    /// Extends the redeemed ticket; the device secret and the ticket must both match.
    pub(super) fn renew_ticket(
        &mut self,
        device_secret: &str,
        presented: &str,
        now_ms: u64,
    ) -> PortResult<RenewedTicket> {
        let pairing = self.pairing_by_secret(device_secret)?;
        let PairState::Redeemed {
            ticket,
            expires_at_ms,
            ..
        } = &mut pairing.state
        else {
            return Err(because(Reason::NoLongerValid));
        };
        if ticket != presented || *expires_at_ms <= now_ms {
            return Err(because(Reason::NoLongerValid));
        }
        *expires_at_ms = now_ms + TICKET_LIFETIME_MS;
        Ok(RenewedTicket {
            renewed: DisplayRenewed {
                ticket_expires_at_ms: *expires_at_ms,
            },
            ticket: secret(TICKET_LIFETIME_MS, ticket.clone()),
        })
    }

    fn managed_lobby(&self, who: &Actor, session: SessionId) -> PortResult<()> {
        let (lobby, _, me) = self.lobby_for(who, session)?;
        if !me.can_manage() {
            return Err(PortError::Forbidden);
        }
        if lobby.has_started {
            return Err(because(Reason::StaleRevision));
        }
        Ok(())
    }

    pub(super) fn lookup_display(
        &self,
        who: &Actor,
        session: SessionId,
        code: &str,
        now_ms: u64,
    ) -> PortResult<PairingLookup> {
        self.managed_lobby(who, session)?;
        let wanted = code.trim().to_ascii_uppercase();
        let found = self.pairings.iter().find(|pairing| {
            pairing.user_code == wanted
                && matches!(pairing.state, PairState::Pending)
                && pairing.expires_at_ms > now_ms
        });
        Ok(
            found.map_or(PairingLookup::NotFound, |pairing| PairingLookup::Found {
                pairing_id: pairing.id.clone(),
                device: pairing.device,
                created_at_ms: pairing.created_at_ms,
                waiting_seconds: now_ms.saturating_sub(pairing.created_at_ms) / 1000,
            }),
        )
    }

    pub(super) fn approve_display(
        &mut self,
        who: &Actor,
        session: SessionId,
        pairing_id: &str,
        now_ms: u64,
    ) -> PortResult<LobbyView> {
        self.managed_lobby(who, session)?;
        let displays = self
            .lobbies
            .get(&session)
            .map_or(0, |lobby| lobby.tickets.len());
        if displays >= MAXIMUM_DISPLAYS_PER_SESSION {
            return Err(because(Reason::DisplayLimit));
        }
        let pairing = self
            .pairings
            .iter_mut()
            .find(|pairing| {
                pairing.id == pairing_id
                    && matches!(pairing.state, PairState::Pending)
                    && pairing.expires_at_ms > now_ms
            })
            .ok_or(because(Reason::NoLongerValid))?;
        let ticket_id = format!("display-{}", fresh_hex());
        pairing.state = PairState::Approved { session };
        pairing.ticket_id = Some(ticket_id.clone());
        let device = pairing.device;
        if let Some(lobby) = self.lobbies.get_mut(&session) {
            lobby.tickets.push(Ticket {
                ticket_id,
                device,
                paired_at_ms: now_ms,
            });
        }
        self.lobby_view(who, session)
    }

    /// Unpairs one display, or all of them when `ticket_id` is `None`; the TV's pairing ends.
    pub(super) fn unpair_display(
        &mut self,
        who: &Actor,
        session: SessionId,
        ticket_id: Option<&str>,
    ) -> PortResult<LobbyView> {
        self.managed_lobby(who, session)?;
        let lobby = self.lobbies.get_mut(&session).ok_or(PortError::NotFound)?;
        if let Some(ticket_id) = ticket_id
            && !lobby
                .tickets
                .iter()
                .any(|ticket| ticket.ticket_id == ticket_id)
        {
            return Err(PortError::NotFound);
        }
        let ended: Vec<String> = lobby
            .tickets
            .iter()
            .filter(|ticket| ticket_id.is_none_or(|wanted| wanted == ticket.ticket_id))
            .map(|ticket| ticket.ticket_id.clone())
            .collect();
        lobby
            .tickets
            .retain(|ticket| !ended.contains(&ticket.ticket_id));
        for pairing in &mut self.pairings {
            if pairing
                .ticket_id
                .as_ref()
                .is_some_and(|ticket| ended.contains(ticket))
            {
                pairing.state = PairState::Cancelled;
            }
        }
        self.lobby_view(who, session)
    }
}
