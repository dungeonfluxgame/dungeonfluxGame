//! Row mapping for the media catalog: the closed-vocabulary columns read back into typed values,
//! the shared statement fragments and the one place a database error is recorded.

use df_assets::{
    AssetId, AssetKind, AssetManifest, AssetRecord, AssetState, AssetVariant, AttemptId, Audience,
    CatalogError, ClaimToken, DedupeScope, FinalizeOutcome, MediaRequestKey, MemberSet, ObjectKey,
    ObjectVersion, OutboxEntry, Provenance, PublishedFacts, QuotaScope, QuotaUsage, RejectReason,
    RetentionClass, StagedObjectRecord, Step, StepLease, StorageDomain, VariantRecord,
    VariantState, WriteId,
};
use df_types::{AccountId, CampaignId, MemberId};
use tokio_postgres::Row;

pub(super) const OPEN_STATES: &str = "('uploading', 'quarantined', 'validated')";
pub(super) const LIVE_STATES: &str = "('uploading', 'quarantined', 'validated', 'ready')";

pub(super) const ASSET_COLUMNS: &str = "asset_id, kind, provenance, charge_scope_kind, charge_scope_id, \
    origin_account, campaign_id, audience_kind, audience_campaign, audience_account, \
    audience_members, dedupe_scope, request_key, retention_class, state, reject_reason, attempt, \
    step, step_owner, step_lease_until_ms, staged_key, staged_version, staged_byte_length, \
    staged_sha256, staged_at_ms, winning_writes, mime, byte_length, sha256, width_px, height_px, \
    suppression_generation, ready_at_ms, created_at_ms, updated_at_ms";

pub(super) const VARIANT_COLUMNS: &str = "asset_id, attempt, write_id, variant, owner, state, object_domain, \
    object_key, object_version, byte_length, sha256, created_at_ms";

/// The one place a database error becomes `Unavailable`: the SQLSTATE is recorded so a CHECK
/// violation or a deadlock is not mistaken for an outage. No statement text or value is logged.
pub(crate) fn unavailable(error: tokio_postgres::Error) -> CatalogError {
    record_failure(&error);
    CatalogError::Unavailable
}

pub(crate) fn record_failure(error: &tokio_postgres::Error) {
    let sqlstate = error.code().map_or("none", |code| code.code());
    df_observe::record_audit_line("ASSET", format!("ASSET db_failed sqlstate={sqlstate}"));
}

pub(super) fn to_i64(value: u64) -> Result<i64, CatalogError> {
    i64::try_from(value).map_err(|_| CatalogError::InvalidRecord)
}

pub(super) fn to_u64(value: i64) -> Result<u64, CatalogError> {
    u64::try_from(value).map_err(|_| CatalogError::InvalidRecord)
}

pub(super) fn invalid<T, E>(result: Result<T, E>) -> Result<T, CatalogError> {
    result.map_err(|_| {
        df_observe::record_audit_line("ASSET", "ASSET record_invalid".to_owned());
        CatalogError::InvalidRecord
    })
}

pub(super) fn asset_of(bytes: &[u8]) -> Result<AssetId, CatalogError> {
    invalid(AssetId::from_bytes(bytes))
}

pub(super) fn attempt_of(bytes: &[u8]) -> Result<AttemptId, CatalogError> {
    invalid(AttemptId::from_bytes(bytes))
}

pub(super) fn claim_of(bytes: &[u8]) -> Result<ClaimToken, CatalogError> {
    invalid(ClaimToken::from_bytes(bytes))
}

pub(super) fn write_of(bytes: &[u8]) -> Result<WriteId, CatalogError> {
    invalid(WriteId::from_bytes(bytes))
}

pub(super) fn scope_columns(scope: QuotaScope) -> (&'static str, Vec<u8>) {
    (scope.kind_label(), scope.id_bytes().to_vec())
}

pub(super) fn scope_of(kind: &str, id: &[u8]) -> Result<QuotaScope, CatalogError> {
    match kind {
        "account" => Ok(QuotaScope::Account(invalid(AccountId::from_bytes(id))?)),
        "campaign" => Ok(QuotaScope::Campaign(invalid(CampaignId::from_bytes(id))?)),
        _ => Err(CatalogError::InvalidRecord),
    }
}

pub(super) struct AudienceColumns {
    pub(super) kind: &'static str,
    pub(super) campaign: Option<Vec<u8>>,
    pub(super) account: Option<Vec<u8>>,
    pub(super) members: Option<Vec<Vec<u8>>>,
}

pub(super) fn audience_columns(audience: &Audience) -> AudienceColumns {
    let (campaign, account, members) = match audience {
        Audience::PublicStatic | Audience::Withheld => (None, None, None),
        Audience::CampaignShared(campaign) | Audience::Host(campaign) => {
            (Some(campaign.as_bytes().to_vec()), None, None)
        }
        Audience::Members(campaign, members) => (
            Some(campaign.as_bytes().to_vec()),
            None,
            Some(
                members
                    .members()
                    .iter()
                    .map(|member| member.as_bytes().to_vec())
                    .collect(),
            ),
        ),
        Audience::OwnerPrivate(account) => (None, Some(account.as_bytes().to_vec()), None),
    };
    AudienceColumns {
        kind: audience.label(),
        campaign,
        account,
        members,
    }
}

pub(super) fn audience_of(row: &Row) -> Result<Audience, CatalogError> {
    let kind: String = row.get("audience_kind");
    let campaign: Option<Vec<u8>> = row.get("audience_campaign");
    let account: Option<Vec<u8>> = row.get("audience_account");
    let members: Option<Vec<Vec<u8>>> = row.get("audience_members");
    let campaign = campaign
        .map(|bytes| invalid(CampaignId::from_bytes(&bytes)))
        .transpose()?;
    match (kind.as_str(), campaign) {
        ("public_static", _) => Ok(Audience::PublicStatic),
        ("withheld", _) => Ok(Audience::Withheld),
        ("campaign_shared", Some(campaign)) => Ok(Audience::CampaignShared(campaign)),
        ("host", Some(campaign)) => Ok(Audience::Host(campaign)),
        ("members", Some(campaign)) => {
            let members = members
                .unwrap_or_default()
                .iter()
                .map(|bytes| invalid(MemberId::from_bytes(bytes)))
                .collect::<Result<Vec<_>, _>>()?;
            let set = MemberSet::new(members).ok_or(CatalogError::InvalidRecord)?;
            Ok(Audience::Members(campaign, set))
        }
        ("owner_private", _) => {
            let account = account.ok_or(CatalogError::InvalidRecord)?;
            Ok(Audience::OwnerPrivate(invalid(AccountId::from_bytes(
                &account,
            ))?))
        }
        _ => Err(CatalogError::InvalidRecord),
    }
}

pub(super) fn label<T>(
    row: &Row,
    column: &str,
    parse: fn(&str) -> Option<T>,
) -> Result<T, CatalogError> {
    let text: String = row.get(column);
    parse(&text).ok_or(CatalogError::InvalidRecord)
}

pub(super) fn record_of(row: &Row) -> Result<AssetRecord, CatalogError> {
    let scope_kind: String = row.get("charge_scope_kind");
    let scope_id: Vec<u8> = row.get("charge_scope_id");
    let origin: Option<Vec<u8>> = row.get("origin_account");
    let campaign: Option<Vec<u8>> = row.get("campaign_id");
    let dedupe: Vec<u8> = row.get("dedupe_scope");
    let request_key: Option<Vec<u8>> = row.get("request_key");
    let reject: Option<String> = row.get("reject_reason");
    let attempt: Vec<u8> = row.get("attempt");
    Ok(AssetRecord {
        asset: asset_of(&row.get::<_, Vec<u8>>("asset_id"))?,
        kind: label(row, "kind", AssetKind::from_label)?,
        provenance: label(row, "provenance", Provenance::from_label)?,
        charge_scope: scope_of(&scope_kind, &scope_id)?,
        origin_account: origin
            .map(|bytes| invalid(AccountId::from_bytes(&bytes)))
            .transpose()?,
        campaign: campaign
            .map(|bytes| invalid(CampaignId::from_bytes(&bytes)))
            .transpose()?,
        audience: audience_of(row)?,
        dedupe_scope: DedupeScope::from_bytes(
            dedupe.try_into().map_err(|_| CatalogError::InvalidRecord)?,
        ),
        request_key: request_key
            .map(|bytes| bytes.try_into().map(MediaRequestKey::from_bytes))
            .transpose()
            .map_err(|_| CatalogError::InvalidRecord)?,
        retention: label(row, "retention_class", RetentionClass::from_label)?,
        state: label(row, "state", AssetState::from_label)?,
        reject_reason: reject
            .map(|text| RejectReason::from_label(&text).ok_or(CatalogError::InvalidRecord))
            .transpose()?,
        attempt: attempt_of(&attempt)?,
        lease: lease_of(row)?,
        staged: staged_of(row)?,
        winning_writes: row
            .get::<_, Option<Vec<Vec<u8>>>>("winning_writes")
            .unwrap_or_default()
            .iter()
            .map(|bytes| write_of(bytes))
            .collect::<Result<_, _>>()?,
        published: published_of(row)?,
        suppression_generation: to_u64(row.get("suppression_generation"))?,
        ready_at_ms: row.get("ready_at_ms"),
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

pub(super) fn lease_of(row: &Row) -> Result<Option<StepLease>, CatalogError> {
    let step: Option<String> = row.get("step");
    let owner: Option<Vec<u8>> = row.get("step_owner");
    let until: Option<i64> = row.get("step_lease_until_ms");
    match (step, owner, until) {
        (Some(step), Some(owner), Some(until_ms)) => Ok(Some(StepLease {
            step: Step::from_label(&step).ok_or(CatalogError::InvalidRecord)?,
            owner: claim_of(&owner)?,
            until_ms,
        })),
        (None, None, None) => Ok(None),
        _ => Err(CatalogError::InvalidRecord),
    }
}

pub(super) fn staged_of(row: &Row) -> Result<Option<StagedObjectRecord>, CatalogError> {
    let key: Option<String> = row.get("staged_key");
    let Some(key) = key else {
        return Ok(None);
    };
    let version: String = row.get("staged_version");
    let length: i64 = row.get("staged_byte_length");
    let sha256: Vec<u8> = row.get("staged_sha256");
    Ok(Some(StagedObjectRecord {
        key: ObjectKey::from_stored(StorageDomain::Quarantine, &key)
            .ok_or(CatalogError::InvalidRecord)?,
        version: ObjectVersion::new(version).ok_or(CatalogError::InvalidRecord)?,
        manifest: AssetManifest {
            byte_len: to_u64(length)?,
            sha256: sha256.try_into().map_err(|_| CatalogError::InvalidRecord)?,
        },
        staged_at_ms: row.get("staged_at_ms"),
    }))
}

pub(super) fn published_of(row: &Row) -> Result<Option<PublishedFacts>, CatalogError> {
    let Some(length) = row.get::<_, Option<i64>>("byte_length") else {
        return Ok(None);
    };
    let sha256: Vec<u8> = row.get("sha256");
    let dimension = |column: &str| -> Result<Option<u32>, CatalogError> {
        row.get::<_, Option<i32>>(column)
            .map(|value| u32::try_from(value).map_err(|_| CatalogError::InvalidRecord))
            .transpose()
    };
    Ok(Some(PublishedFacts {
        mime: row.get("mime"),
        byte_len: to_u64(length)?,
        sha256: sha256.try_into().map_err(|_| CatalogError::InvalidRecord)?,
        width_px: dimension("width_px")?,
        height_px: dimension("height_px")?,
    }))
}

pub(super) fn variant_of(row: &Row) -> Result<VariantRecord, CatalogError> {
    let domain: String = row.get("object_domain");
    let key: String = row.get("object_key");
    let version: Option<String> = row.get("object_version");
    let length: Option<i64> = row.get("byte_length");
    let sha256: Option<Vec<u8>> = row.get("sha256");
    let manifest = match (length, sha256) {
        (Some(length), Some(sha256)) => Some(AssetManifest {
            byte_len: to_u64(length)?,
            sha256: sha256.try_into().map_err(|_| CatalogError::InvalidRecord)?,
        }),
        _ => None,
    };
    let variant: String = row.get("variant");
    Ok(VariantRecord {
        asset: asset_of(&row.get::<_, Vec<u8>>("asset_id"))?,
        attempt: attempt_of(&row.get::<_, Vec<u8>>("attempt"))?,
        write: write_of(&row.get::<_, Vec<u8>>("write_id"))?,
        variant: variant_from_label(&variant)?,
        owner: claim_of(&row.get::<_, Vec<u8>>("owner"))?,
        state: label(row, "state", VariantState::from_label)?,
        key: StorageDomain::from_label(&domain)
            .and_then(|domain| ObjectKey::from_stored(domain, &key))
            .ok_or(CatalogError::InvalidRecord)?,
        version: version
            .map(|text| ObjectVersion::new(text).ok_or(CatalogError::InvalidRecord))
            .transpose()?,
        manifest,
        created_at_ms: row.get("created_at_ms"),
    })
}

pub(super) fn variant_from_label(text: &str) -> Result<AssetVariant, CatalogError> {
    match text {
        "master" => Ok(AssetVariant::Master),
        "display" => Ok(AssetVariant::Display),
        _ => Err(CatalogError::InvalidRecord),
    }
}

pub(super) fn outbox_of(row: &Row) -> Result<OutboxEntry, CatalogError> {
    let outcome: String = row.get("outcome");
    let reason: Option<String> = row.get("reason");
    let reason = reason
        .map(|text| RejectReason::from_label(&text).ok_or(CatalogError::InvalidRecord))
        .transpose()?;
    let outcome = match (outcome.as_str(), reason) {
        ("ready", None) => FinalizeOutcome::Ready,
        ("superseded", None) => FinalizeOutcome::Superseded,
        ("rejected", Some(reason)) => FinalizeOutcome::Rejected(reason),
        ("invalid", Some(reason)) => FinalizeOutcome::Invalid(reason),
        _ => return Err(CatalogError::InvalidRecord),
    };
    Ok(OutboxEntry {
        sequence: row.get("outbox_seq"),
        attempt: attempt_of(&row.get::<_, Vec<u8>>("attempt"))?,
        asset: asset_of(&row.get::<_, Vec<u8>>("asset_id"))?,
        outcome,
        at_ms: row.get("at_ms"),
        is_consumed: row.get::<_, Option<i64>>("consumed_at_ms").is_some(),
    })
}

pub(super) fn usage_of(row: &Row) -> Result<QuotaUsage, CatalogError> {
    Ok(QuotaUsage {
        committed_bytes: to_u64(row.get("committed_bytes"))?,
        reserved_bytes: to_u64(row.get("reserved_bytes"))?,
        in_flight_uploads: u32::try_from(row.get::<_, i32>("in_flight_uploads"))
            .map_err(|_| CatalogError::InvalidRecord)?,
        template_count: u32::try_from(row.get::<_, i32>("template_count"))
            .map_err(|_| CatalogError::InvalidRecord)?,
    })
}
