//! Identifiers as the API carries them: lowercase hexadecimal text. A malformed id is read as
//! "no such thing" (`not_found`), the same answer as a missing or foreign one; a malformed
//! operation id is `invalid`.

use df_account_app::wire::ErrorCode;
use df_types::{AccountId, CampaignId, CheckoutId, OperationId, SessionId};

use super::guard::ApiFailure;

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn operation(text: &str) -> Result<OperationId, ApiFailure> {
    OperationId::from_hex(text).map_err(|_| ApiFailure::of(ErrorCode::Invalid))
}

pub fn campaign(text: &str) -> Result<CampaignId, ApiFailure> {
    CampaignId::from_hex(text).map_err(|_| ApiFailure::of(ErrorCode::NotFound))
}

pub fn session(text: &str) -> Result<SessionId, ApiFailure> {
    SessionId::from_hex(text).map_err(|_| ApiFailure::of(ErrorCode::NotFound))
}

pub fn checkout(text: &str) -> Result<CheckoutId, ApiFailure> {
    CheckoutId::from_hex(text).map_err(|_| ApiFailure::of(ErrorCode::NotFound))
}

pub fn account(text: &str) -> Result<AccountId, ApiFailure> {
    AccountId::from_hex(text).map_err(|_| ApiFailure::of(ErrorCode::NotFound))
}

pub fn account_text(account: AccountId) -> String {
    hex(account.as_bytes())
}
