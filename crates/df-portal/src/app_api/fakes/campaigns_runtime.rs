//! Disposable local preview placements; each game retains its own gameplay authority.
use super::{campaigns::FakeCampaigns, lock};
use crate::app_api::ports::{Actor, PortError, PortResult};
use df_account_app::wire_campaign::GameLinks;
use df_campaign::{
    lobby_state::SeatedMember,
    session_link::{HttpRuntime, Placement, RuntimeError, runtime_protocol::LocalLinks},
};
use df_types::{MemberId, SessionId};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;

const MAXIMUM_PLACEMENTS: usize = 8;
struct Slot {
    placement: Placement,
    allocation: Mutex<()>,
}
pub(super) struct LocalRuntime {
    client: HttpRuntime,
    slots: Mutex<HashMap<SessionId, Arc<Slot>>>,
}

impl LocalRuntime {
    fn new(client: HttpRuntime) -> Self {
        Self {
            client,
            slots: Mutex::new(HashMap::new()),
        }
    }

    async fn allocate(
        &self,
        placement: Placement,
    ) -> Result<Option<(GameLinks, String)>, RuntimeError> {
        let slot = {
            let mut slots = self.slots.lock().await;
            if let Some(slot) = slots.get(&placement.session) {
                if slot.placement.campaign != placement.campaign {
                    return Err(RuntimeError::Occupied);
                }
                Arc::clone(slot)
            } else {
                // Like the runtime, never recycle a room into a different session.
                if slots.len() >= MAXIMUM_PLACEMENTS {
                    return Err(RuntimeError::Occupied);
                }
                let slot = Arc::new(Slot {
                    placement,
                    allocation: Mutex::new(()),
                });
                slots.insert(slot.placement.session, Arc::clone(&slot));
                slot
            }
        };
        // Cancellation retains the frozen reservation; concurrent requests reconcile one allocation.
        let _allocation = slot.allocation.lock().await;
        let (room, allocated) = self.client.allocate_with_links(&slot.placement).await?;
        let Some(LocalLinks {
            launcher_url,
            display_url,
            player_url,
        }) = allocated
        else {
            return Ok(None);
        };
        // The launcher is the host's screen on this PC; the table display and phones may join over the
        // host's private network (#1459).
        if !valid_local_url(&launcher_url, false)
            || [&display_url, &player_url]
                .iter()
                .any(|url| !valid_local_url(url, true))
        {
            return Err(RuntimeError::InvalidReply);
        }
        let allocated = (
            GameLinks {
                display_url,
                player_url,
                room_code: Some(room.code),
            },
            launcher_url,
        );
        Ok(Some(allocated))
    }
}

fn valid_local_url(url: &str, allow_private_network: bool) -> bool {
    url.parse::<axum::http::Uri>().ok().is_some_and(|uri| {
        uri.scheme_str() == Some("http")
            && uri.authority().is_some_and(|authority| {
                authority
                .as_str()
                .parse::<std::net::SocketAddr>()
                .is_ok_and(|address| {
                    let local = address.ip().is_loopback()
                        || (allow_private_network
                            && matches!(address.ip(), std::net::IpAddr::V4(ip) if ip.is_private()));
                    local && address.port() != 0
                })
            })
    })
}

impl FakeCampaigns {
    /// Configures the trusted loopback runtime before accepting preview requests.
    pub fn use_runtime(&self, client: HttpRuntime) {
        *lock(&self.runtime) = Some(Arc::new(LocalRuntime::new(client)));
    }

    pub(super) async fn allocate_runtime(&self, who: &Actor, session: SessionId) -> PortResult<()> {
        let runtime = lock(&self.runtime).clone();
        let Some(runtime) = runtime else {
            return Ok(());
        };
        let placement = self.run(|state, _, _| {
            let (lobby, campaign, _) = state.lobby_for(who, session)?;
            if !lobby.has_started {
                return Ok(None);
            }
            let roster = campaign
                .members
                .iter()
                .filter(|member| member.has_seat())
                .map(|member| {
                    // The fake directory uses a stable synthetic member identity, never player input.
                    Ok(SeatedMember {
                        account: member.account,
                        member: MemberId::from_bytes(member.account.as_bytes())
                            .map_err(|_| PortError::Unavailable)?,
                    })
                })
                .collect::<PortResult<Vec<_>>>()?;
            Ok(Some(Placement {
                campaign: campaign.id,
                session,
                roster,
            }))
        })?;
        let Some(placement) = placement else {
            return Ok(());
        };
        let result = runtime.allocate(placement).await;
        let session_text = df_campaign::secrets::hex(session.as_bytes());
        df_observe::record_audit_line(
            "session.preview_placement",
            format!(
                "SESSION preview_placement session={session_text} reason={}",
                match &result {
                    Ok(Some(_)) => "ready",
                    Ok(None) => "launcher_missing",
                    Err(RuntimeError::Unavailable) => "unavailable",
                    Err(RuntimeError::Occupied) => "occupied",
                    Err(RuntimeError::InvalidReply) => "invalid_reply",
                    Err(RuntimeError::Deadline) => "deadline",
                }
            ),
        );
        self.run(|state, _, _| {
            if let Ok(Some((links, launcher))) = result {
                state.game_links.insert(session, links);
                state.launcher_urls.insert(session, launcher);
            } else {
                state.game_links.remove(&session);
                state.launcher_urls.remove(&session);
            }
            Ok(())
        })?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "campaigns_runtime_tests.rs"]
mod tests;
