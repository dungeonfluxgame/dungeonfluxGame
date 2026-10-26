//! The preview's PAT store mirrors the account lock, expiry and metadata-only PostgreSQL port.
use super::*;
use crate::integration::{CreateTokenOutcome,MAXIMUM_ACTIVE_TOKENS,PersonalTokenStore,SignInSession,StoredToken,TokenId,TokenMetadata};
impl PersonalTokenStore for MemoryAccountStore{
    async fn integration_sessions(&self,account:AccountId,after:Option<TokenId>,limit:u32)->StoreResult<Vec<SignInSession>>{
        let state=self.state();
        let mut rows=state.sessions.values().filter(|r|r.record.account==account).map(|r|SignInSession{id:TokenId(*r.record.id.as_bytes()),device_label:r.record.device.label(),created_at_ms:r.record.created_at_ms,last_seen_at_ms:r.record.last_seen_at_ms,expires_at_ms:r.absolute_expires_at_ms,revoked:r.record.is_revoked}).filter(|r|after.is_none_or(|after|r.id.0>after.0)).collect::<Vec<_>>();
        rows.sort_by_key(|r|r.id.0);rows.truncate(limit.min(101) as usize);Ok(rows)
    }
    async fn create_personal_token(&self,token:StoredToken)->StoreResult<CreateTokenOutcome>{
        let mut state=self.state();let account=token.metadata.account;let now=token.metadata.created_at_ms;
        if !state.accounts.get(&account).is_some_and(|a|a.status==AccountStatus::Active&&a.is_verified){return Ok(CreateTokenOutcome::InactiveAccount);}
        if state.personal_tokens.values().filter(|t|t.metadata.account==account&&t.metadata.revoked_at_ms.is_none()&&t.metadata.expires_at_ms>now).count()>=MAXIMUM_ACTIVE_TOKENS{return Ok(CreateTokenOutcome::LimitReached);}
        if state.personal_tokens.contains_key(&token.metadata.id.0){return Err(crate::StoreError::InvalidRecord);}
        state.personal_tokens.insert(token.metadata.id.0,token);
        let mut history=state.personal_tokens.values().filter(|t|t.metadata.account==account&&(t.metadata.revoked_at_ms.is_some()||t.metadata.expires_at_ms<=now)).map(|t|(t.metadata.created_at_ms,t.metadata.id.0)).collect::<Vec<_>>();
        history.sort_unstable_by(|a,b|b.cmp(a));for(_,id)in history.into_iter().skip(90){state.personal_tokens.remove(&id);}Ok(CreateTokenOutcome::Created)
    }
    async fn personal_tokens(&self,account:AccountId)->StoreResult<Vec<TokenMetadata>>{
        let mut rows=self.state().personal_tokens.values().filter(|t|t.metadata.account==account).map(|t|t.metadata.clone()).collect::<Vec<_>>();rows.sort_by(|a,b|(b.created_at_ms,b.id.0).cmp(&(a.created_at_ms,a.id.0)));rows.truncate(100);Ok(rows)
    }
    async fn personal_token(&self,id:TokenId)->StoreResult<Option<StoredToken>>{
        Ok(self.state().personal_tokens.get(&id.0).map(|t|StoredToken{metadata:t.metadata.clone(),pepper_id:t.pepper_id,digest:t.digest}))
    }
    async fn revoke_personal_token(&self,account:AccountId,id:TokenId,now_ms:u64)->StoreResult<bool>{
        let mut state=self.state();let Some(token)=state.personal_tokens.get_mut(&id.0).filter(|t|t.metadata.account==account)else{return Ok(false);};token.metadata.revoked_at_ms.get_or_insert(now_ms);Ok(true)
    }
    async fn used_personal_token(&self,id:TokenId,old_digest:[u8;32],pepper_id:u32,digest:[u8;32],now_ms:u64)->StoreResult<bool>{
        let mut state=self.state();let Some(account)=state.personal_tokens.get(&id.0).map(|t|t.metadata.account)else{return Ok(false);};
        if !state.accounts.get(&account).is_some_and(|a|a.status==AccountStatus::Active&&a.is_verified){return Ok(false);}
        let Some(token)=state.personal_tokens.get_mut(&id.0).filter(|t|t.digest==old_digest&&t.metadata.revoked_at_ms.is_none()&&t.metadata.expires_at_ms>now_ms)else{return Ok(false);};
        token.digest=digest;token.pepper_id=pepper_id;token.metadata.last_used_at_ms=Some(token.metadata.last_used_at_ms.unwrap_or(0).max(now_ms));Ok(true)
    }
}
