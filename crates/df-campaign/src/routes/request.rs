//! What a handler works with: the request context, path parameters and body parsing, and the
//! conversions between wire types and domain types.

use df_campaign_wire as wire;
use df_types::{AccountId, CampaignId, MemberId, OperationId, SessionId};
use http::{StatusCode, request::Parts};
use serde::de::DeserializeOwned;

use super::{
    App,
    guard::{Caller, Guard},
    responses::{Failed, refused},
};
use crate::{
    join_invitation::JoinRole,
    roles::{CampaignRole, Delegation, Delegations},
    secrets::hex,
    service::{CampaignService, Ports},
};

pub(super) struct Ctx<'a, P: Ports, G: Guard> {
    pub(super) app: &'a App<P, G>,
    pub(super) params: Vec<(String, String)>,
    pub(super) parts: &'a Parts,
    pub(super) caller: Caller,
    pub(super) body: Vec<u8>,
}

impl<P: Ports, G: Guard> Ctx<'_, P, G> {
    pub(super) fn service(&self) -> &CampaignService<P> {
        &self.app.service
    }

    /// The signed-in account. A session route always has one; a missing one is a guard bug and
    /// answers as unauthenticated.
    pub(super) fn account(&self) -> Result<AccountId, Failed> {
        self.caller
            .account
            .ok_or_else(|| refused(StatusCode::UNAUTHORIZED, wire::Reason::Unauthenticated))
    }

    pub(super) fn json<T: DeserializeOwned>(&self) -> Result<T, Failed> {
        serde_json::from_slice(&self.body).map_err(|_| invalid())
    }

    pub(super) fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub(super) fn campaign(&self) -> Result<CampaignId, Failed> {
        self.param("campaign")
            .and_then(|text| CampaignId::from_hex(text).ok())
            .ok_or_else(not_found)
    }

    pub(super) fn member(&self) -> Result<MemberId, Failed> {
        self.param("member")
            .and_then(|text| MemberId::from_hex(text).ok())
            .ok_or_else(not_found)
    }

    pub(super) fn session(&self) -> Result<SessionId, Failed> {
        self.param("session")
            .and_then(|text| SessionId::from_hex(text).ok())
            .ok_or_else(not_found)
    }
}

/// A path id that is missing or malformed answers like an id that does not exist.
pub(super) fn not_found() -> Failed {
    refused(StatusCode::NOT_FOUND, wire::Reason::NotFound)
}

pub(super) fn invalid() -> Failed {
    refused(StatusCode::BAD_REQUEST, wire::Reason::InvalidInput)
}

pub(super) fn operation_of(text: &str) -> Result<OperationId, Failed> {
    OperationId::from_hex(text).map_err(|_| invalid())
}

pub(super) fn join_role(role: wire::JoinsAs) -> JoinRole {
    match role {
        wire::JoinsAs::Player => JoinRole::Player,
        wire::JoinsAs::Contributor => JoinRole::Contributor,
    }
}

pub(super) fn delegations_of(body: &wire::CoHostBody) -> Delegations {
    let mut delegations = Delegations::NONE;
    if body.invite_players {
        delegations = delegations.with(Delegation::InvitePlayers);
    }
    if body.start_or_pause_session {
        delegations = delegations.with(Delegation::StartOrPauseSession);
    }
    if body.export_campaign {
        delegations = delegations.with(Delegation::ExportCampaign);
    }
    delegations
}

pub(super) fn co_host_body(delegations: Delegations) -> wire::CoHostBody {
    wire::CoHostBody {
        invite_players: delegations.contains(Delegation::InvitePlayers),
        start_or_pause_session: delegations.contains(Delegation::StartOrPauseSession),
        export_campaign: delegations.contains(Delegation::ExportCampaign),
    }
}

pub(super) fn role_name(role: CampaignRole) -> wire::RoleName {
    match role {
        CampaignRole::Host => wire::RoleName::Host,
        CampaignRole::CoHost => wire::RoleName::CoHost,
        CampaignRole::Player => wire::RoleName::Player,
        CampaignRole::Contributor | CampaignRole::PairedDisplay => wire::RoleName::Contributor,
    }
}

pub(super) fn hex_of(bytes: &[u8; 16]) -> String {
    hex(bytes)
}
