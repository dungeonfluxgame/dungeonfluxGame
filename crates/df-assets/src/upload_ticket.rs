//! Typed claims for the one-object upload capability.

use crate::media_ids::{AssetId, AttemptId, UploadId};
use crate::object_store::ObjectKey;
use df_types::{AccountId, CampaignId};
use sha2::{Digest, Sha256};

pub const MAX_UPLOAD_TICKET_LIFETIME_MS: i64 = 5 * 60 * 1000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadPurpose {
    Portrait,
    CharacterJson,
}

impl UploadPurpose {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Portrait => "portrait",
            Self::CharacterJson => "character_json",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadTicket {
    pub upload: UploadId,
    pub asset: AssetId,
    pub attempt: AttemptId,
    pub account: AccountId,
    pub campaign: CampaignId,
    pub purpose: UploadPurpose,
    pub object_key: ObjectKey,
    /// Exact body size recorded when the ticket was issued.
    pub declared_bytes: u64,
    pub expected_sha256: [u8; 32],
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadTicketRejection {
    WrongAccount,
    WrongCampaign,
    WrongPurpose,
    WrongObjectKey,
    Expired,
    InvalidLifetime,
    DigestMismatch,
    AlreadyUsed,
}

/// The durable result of trying to consume an upload ticket.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadTicketClaim {
    Claimed,
    Refused(UploadTicketRejection),
}

impl UploadTicketRejection {
    pub const fn label(self) -> &'static str {
        match self {
            Self::WrongAccount => "wrong_account",
            Self::WrongCampaign => "wrong_campaign",
            Self::WrongPurpose => "wrong_purpose",
            Self::WrongObjectKey => "wrong_object_key",
            Self::Expired => "expired",
            Self::InvalidLifetime => "invalid_lifetime",
            Self::DigestMismatch => "digest_mismatch",
            Self::AlreadyUsed => "already_used",
        }
    }
}

/// Validates immutable ticket bindings after the catalog has durably claimed it.
pub fn validate_upload_ticket(
    ticket: &UploadTicket,
    account: AccountId,
    campaign: CampaignId,
    purpose: UploadPurpose,
    object_key: &ObjectKey,
    bytes: &[u8],
    now_ms: i64,
) -> Result<(), UploadTicketRejection> {
    validate_upload_ticket_binding(ticket, account, campaign, purpose, object_key, now_ms)?;
    if Sha256::digest(bytes).as_slice() != ticket.expected_sha256 {
        return Err(UploadTicketRejection::DigestMismatch);
    }
    if u64::try_from(bytes.len()).ok() != Some(ticket.declared_bytes) {
        return Err(UploadTicketRejection::DigestMismatch);
    }
    Ok(())
}

/// Checks authenticated identity, purpose, quarantine key and the ticket lifetime before a body is read.
pub fn validate_upload_ticket_binding(
    ticket: &UploadTicket,
    account: AccountId,
    campaign: CampaignId,
    purpose: UploadPurpose,
    object_key: &ObjectKey,
    now_ms: i64,
) -> Result<(), UploadTicketRejection> {
    if account != ticket.account {
        return Err(UploadTicketRejection::WrongAccount);
    }
    if campaign != ticket.campaign {
        return Err(UploadTicketRejection::WrongCampaign);
    }
    if purpose != ticket.purpose {
        return Err(UploadTicketRejection::WrongPurpose);
    }
    if object_key != &ticket.object_key {
        return Err(UploadTicketRejection::WrongObjectKey);
    }
    let lifetime = ticket
        .expires_at_ms
        .checked_sub(ticket.issued_at_ms)
        .ok_or(UploadTicketRejection::InvalidLifetime)?;
    if lifetime <= 0 || lifetime > MAX_UPLOAD_TICKET_LIFETIME_MS {
        return Err(UploadTicketRejection::InvalidLifetime);
    }
    if now_ms < ticket.issued_at_ms || now_ms >= ticket.expires_at_ms {
        return Err(UploadTicketRejection::Expired);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_ids::WriteId;
    use crate::object_store::{AssetVariant, KeyLayout, ObjectEnvironment};

    fn ids<T>(constructor: fn(&[u8]) -> Result<T, crate::media_ids::MediaIdError>, value: u8) -> T {
        constructor(&[value; 16]).expect("nonzero id")
    }

    fn fixture() -> (UploadTicket, ObjectKey, AccountId, CampaignId) {
        let account = AccountId::from_bytes(&[3; 16]).expect("account id");
        let campaign = CampaignId::from_bytes(&[4; 16]).expect("campaign id");
        let layout = KeyLayout::new(ObjectEnvironment::new("test").expect("environment"));
        let key = layout.campaign_asset(
            &campaign,
            ids(AssetId::from_bytes, 5),
            ids(AttemptId::from_bytes, 6),
            ids(WriteId::from_bytes, 7),
            AssetVariant::Master,
        );
        let bytes = b"expected upload";
        let ticket = UploadTicket {
            upload: ids(UploadId::from_bytes, 8),
            asset: ids(AssetId::from_bytes, 5),
            attempt: ids(AttemptId::from_bytes, 6),
            account,
            campaign,
            purpose: UploadPurpose::Portrait,
            object_key: key.clone(),
            declared_bytes: bytes.len() as u64,
            expected_sha256: Sha256::digest(bytes).into(),
            issued_at_ms: 10,
            expires_at_ms: 10 + MAX_UPLOAD_TICKET_LIFETIME_MS,
        };
        (ticket, key, account, campaign)
    }

    #[test]
    fn upload_ticket_accepts_its_exact_binding_and_digest_once_validated() {
        let (ticket, key, account, campaign) = fixture();
        assert_eq!(
            validate_upload_ticket(
                &ticket,
                account,
                campaign,
                UploadPurpose::Portrait,
                &key,
                b"expected upload",
                11,
            ),
            Ok(())
        );
    }

    #[test]
    fn upload_ticket_refuses_another_account() {
        let (ticket, key, _, campaign) = fixture();
        let other = AccountId::from_bytes(&[9; 16]).expect("account id");
        assert_eq!(
            validate_upload_ticket(
                &ticket,
                other,
                campaign,
                UploadPurpose::Portrait,
                &key,
                b"expected upload",
                11,
            ),
            Err(UploadTicketRejection::WrongAccount)
        );
    }

    #[test]
    fn upload_ticket_refuses_another_campaign_or_purpose() {
        let (ticket, key, account, campaign) = fixture();
        let other_campaign = CampaignId::from_bytes(&[10; 16]).expect("campaign id");
        assert_eq!(
            validate_upload_ticket(
                &ticket,
                account,
                other_campaign,
                UploadPurpose::Portrait,
                &key,
                b"expected upload",
                11,
            ),
            Err(UploadTicketRejection::WrongCampaign)
        );
        assert_eq!(
            validate_upload_ticket(
                &ticket,
                account,
                campaign,
                UploadPurpose::CharacterJson,
                &key,
                b"expected upload",
                11,
            ),
            Err(UploadTicketRejection::WrongPurpose)
        );
    }

    #[test]
    fn upload_ticket_refuses_another_object_key_and_digest() {
        let (ticket, key, account, campaign) = fixture();
        let layout = KeyLayout::new(ObjectEnvironment::new("test").expect("environment"));
        let other_key = layout.campaign_asset(
            &campaign,
            ids(AssetId::from_bytes, 10),
            ids(AttemptId::from_bytes, 6),
            ids(WriteId::from_bytes, 7),
            AssetVariant::Master,
        );
        assert_eq!(
            validate_upload_ticket(
                &ticket,
                account,
                campaign,
                UploadPurpose::Portrait,
                &other_key,
                b"expected upload",
                11,
            ),
            Err(UploadTicketRejection::WrongObjectKey)
        );
        assert_eq!(
            validate_upload_ticket(
                &ticket,
                account,
                campaign,
                UploadPurpose::Portrait,
                &key,
                b"different bytes",
                11,
            ),
            Err(UploadTicketRejection::DigestMismatch)
        );
    }

    #[test]
    fn upload_ticket_refuses_expiration_and_overlong_lifetime() {
        let (mut ticket, key, account, campaign) = fixture();
        assert_eq!(
            validate_upload_ticket(
                &ticket,
                account,
                campaign,
                UploadPurpose::Portrait,
                &key,
                b"expected upload",
                ticket.expires_at_ms,
            ),
            Err(UploadTicketRejection::Expired)
        );
        ticket.expires_at_ms += 1;
        assert_eq!(
            validate_upload_ticket(
                &ticket,
                account,
                campaign,
                UploadPurpose::Portrait,
                &key,
                b"expected upload",
                11,
            ),
            Err(UploadTicketRejection::InvalidLifetime)
        );
    }
}
