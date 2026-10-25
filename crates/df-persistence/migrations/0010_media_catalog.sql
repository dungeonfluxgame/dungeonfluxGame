-- Media catalog (planning/asset-storage-research.md 6, 10, 11, 16; plan 13.3). Schema media, in the
-- one hosted database (D7). Times are milliseconds since the Unix epoch from the service clock.
-- Ids are 16 bytes; request keys and digests 32. The catalog is the authority for whether an asset
-- exists (13.3); object bytes are written before a row reaches 'ready'.
-- Lock order (D25): finance and O1 rows, then media.quota_usage by (scope_kind, scope_id)
-- ('account' sorts before 'campaign'), then media.asset by asset_id, then every other media row.

CREATE SCHEMA IF NOT EXISTS media;

CREATE TABLE media.quota_usage (
    scope_kind text NOT NULL CHECK (scope_kind IN ('account', 'campaign')),
    scope_id bytea NOT NULL CHECK (octet_length(scope_id) = 16),
    committed_bytes bigint NOT NULL DEFAULT 0 CHECK (committed_bytes >= 0),
    reserved_bytes bigint NOT NULL DEFAULT 0 CHECK (reserved_bytes >= 0),
    in_flight_uploads integer NOT NULL DEFAULT 0 CHECK (in_flight_uploads >= 0),
    template_count integer NOT NULL DEFAULT 0 CHECK (template_count >= 0),
    PRIMARY KEY (scope_kind, scope_id)
);

CREATE TABLE media.asset (
    asset_id bytea PRIMARY KEY CHECK (octet_length(asset_id) = 16),
    kind text NOT NULL CHECK (kind IN ('image', 'audio', 'video', 'character_json',
        'checkpoint_bundle', 'export_archive')),
    provenance text NOT NULL CHECK (provenance IN ('generated', 'uploaded', 'reviewed',
        'claimed_library')),
    -- Who carries the bytes in the quota (moves to the campaign on share).
    charge_scope_kind text NOT NULL CHECK (charge_scope_kind IN ('account', 'campaign')),
    charge_scope_id bytea NOT NULL CHECK (octet_length(charge_scope_id) = 16),
    origin_account bytea CHECK (octet_length(origin_account) = 16),
    campaign_id bytea CHECK (octet_length(campaign_id) = 16),
    template_id bytea CHECK (octet_length(template_id) = 16),
    audience_kind text NOT NULL CHECK (audience_kind IN ('public_static', 'campaign_shared',
        'members', 'host', 'owner_private', 'withheld')),
    audience_campaign bytea CHECK (octet_length(audience_campaign) = 16),
    audience_account bytea CHECK (octet_length(audience_account) = 16),
    audience_members bytea[],
    -- Dedupe: the campaign (or account, or 16 zero bytes for reviewed public photographs).
    dedupe_scope bytea NOT NULL CHECK (octet_length(dedupe_scope) = 16),
    request_key bytea CHECK (octet_length(request_key) = 32),
    state text NOT NULL CHECK (state IN ('uploading', 'quarantined', 'validated', 'ready',
        'archived', 'deleted', 'purging', 'purged', 'rejected')),
    reject_reason text CHECK (reject_reason IN ('validation_failed', 'screen_rejected',
        'abandoned', 'not_dispatched', 'supplier_failed', 'unknown_outcome_failed',
        'output_lost', 'over_maximum_bytes', 'review_expired', 'screen_unavailable',
        'staged_expired', 'staged_lost', 'quota_exceeded')),
    attempt bytea NOT NULL CHECK (octet_length(attempt) = 16),
    step text CHECK (step IN ('stage', 'validate', 'screen', 'publish', 'purge')),
    step_owner bytea CHECK (octet_length(step_owner) = 16),
    step_lease_until_ms bigint,
    staged_key text,
    staged_version text,
    staged_byte_length bigint CHECK (staged_byte_length >= 0),
    staged_sha256 bytea CHECK (octet_length(staged_sha256) = 32),
    staged_at_ms bigint,
    finalize_requested_ms bigint,
    winning_writes bytea[],
    mime text CHECK (octet_length(mime) <= 64),
    byte_length bigint CHECK (byte_length >= 0),
    width_px integer CHECK (width_px > 0),
    height_px integer CHECK (height_px > 0),
    duration_ms bigint CHECK (duration_ms >= 0),
    sha256 bytea CHECK (octet_length(sha256) = 32),
    judgement text CHECK (judgement IN ('passed', 'rejected', 'unjudged', 'not_applicable')),
    retention_class text NOT NULL CHECK (retention_class IN ('quarantine', 'intermediate',
        'personal', 'campaign_media', 'export', 'backup')),
    suppression_generation bigint NOT NULL DEFAULT 0 CHECK (suppression_generation >= 0),
    ready_at_ms bigint,
    created_at_ms bigint NOT NULL,
    updated_at_ms bigint NOT NULL,
    CHECK ((state = 'rejected') = (reject_reason IS NOT NULL)),
    CHECK ((step IS NULL) = (step_owner IS NULL) AND (step_owner IS NULL) = (step_lease_until_ms IS NULL)),
    CHECK ((staged_key IS NULL) = (staged_version IS NULL)
        AND (staged_key IS NULL) = (staged_byte_length IS NULL)
        AND (staged_key IS NULL) = (staged_sha256 IS NULL)
        AND (staged_key IS NULL) = (staged_at_ms IS NULL)),
    CHECK (state NOT IN ('ready', 'archived', 'deleted', 'purging', 'purged')
        OR ready_at_ms IS NOT NULL),
    CHECK (state NOT IN ('ready', 'archived') OR (winning_writes IS NOT NULL
        AND byte_length IS NOT NULL AND sha256 IS NOT NULL)),
    CHECK (audience_kind NOT IN ('campaign_shared', 'members', 'host')
        OR audience_campaign IS NOT NULL),
    CHECK ((audience_kind = 'owner_private') = (audience_account IS NOT NULL)),
    CHECK ((audience_kind = 'members') = (audience_members IS NOT NULL))
);

-- At most one live row per request key (6.2): a rejected row never blocks regeneration.
CREATE UNIQUE INDEX asset_live_request_key ON media.asset (dedupe_scope, request_key)
    WHERE request_key IS NOT NULL AND state IN ('uploading', 'quarantined', 'validated', 'ready');
CREATE INDEX asset_campaign_state ON media.asset (campaign_id, state);
CREATE INDEX asset_origin_state ON media.asset (origin_account, state);
CREATE INDEX asset_sweep ON media.asset (state, updated_at_ms);

-- The reservation one admission holds against one scope until its terminal transition.
-- expires_at_ms is null while it must not expire (a dispatch in flight).
CREATE TABLE media.quota_reservation (
    asset_id bytea PRIMARY KEY REFERENCES media.asset (asset_id),
    scope_kind text NOT NULL CHECK (scope_kind IN ('account', 'campaign')),
    scope_id bytea NOT NULL CHECK (octet_length(scope_id) = 16),
    reserved_bytes bigint NOT NULL CHECK (reserved_bytes >= 0),
    counts_upload boolean NOT NULL,
    expires_at_ms bigint
);

CREATE TABLE media.asset_variant (
    asset_id bytea NOT NULL REFERENCES media.asset (asset_id),
    attempt bytea NOT NULL CHECK (octet_length(attempt) = 16),
    write_id bytea NOT NULL CHECK (octet_length(write_id) = 16),
    variant text NOT NULL CHECK (variant IN ('master', 'display')),
    owner bytea NOT NULL CHECK (octet_length(owner) = 16),
    state text NOT NULL CHECK (state IN ('intended', 'written', 'deleting')),
    object_domain text NOT NULL CHECK (object_domain IN ('private_media', 'quarantine')),
    object_key text NOT NULL,
    object_version text,
    byte_length bigint CHECK (byte_length >= 0),
    sha256 bytea CHECK (octet_length(sha256) = 32),
    created_at_ms bigint NOT NULL,
    PRIMARY KEY (asset_id, attempt, write_id, variant),
    CHECK (state <> 'written' OR (object_version IS NOT NULL AND byte_length IS NOT NULL
        AND sha256 IS NOT NULL))
);

CREATE TABLE media.provider_lineage (
    asset_id bytea PRIMARY KEY REFERENCES media.asset (asset_id),
    attempt bytea NOT NULL CHECK (octet_length(attempt) = 16),
    provider text NOT NULL CHECK (octet_length(provider) <= 64),
    model text NOT NULL CHECK (octet_length(model) <= 128),
    request_schema text NOT NULL CHECK (octet_length(request_schema) <= 32),
    -- The prompt's digest only: withheld prompts can hold hidden facts.
    prompt_sha256 bytea NOT NULL CHECK (octet_length(prompt_sha256) = 32),
    reference_sha256 bytea[] NOT NULL,
    supplier_job_id text CHECK (octet_length(supplier_job_id) <= 256),
    cost_micro_dollars bigint CHECK (cost_micro_dollars >= 0),
    elapsed_ms bigint CHECK (elapsed_ms >= 0)
);

CREATE TABLE media.upload (
    upload_id bytea PRIMARY KEY CHECK (octet_length(upload_id) = 16),
    asset_id bytea NOT NULL UNIQUE REFERENCES media.asset (asset_id),
    account_id bytea NOT NULL CHECK (octet_length(account_id) = 16),
    purpose text NOT NULL CHECK (purpose IN ('portrait', 'character_json')),
    declared_mime text NOT NULL CHECK (declared_mime IN ('image/jpeg', 'image/png',
        'image/webp', 'application/json')),
    declared_bytes bigint NOT NULL CHECK (declared_bytes > 0),
    declared_sha256 bytea NOT NULL CHECK (octet_length(declared_sha256) = 32),
    attempt bytea NOT NULL CHECK (octet_length(attempt) = 16),
    object_key text NOT NULL CHECK (octet_length(object_key) BETWEEN 1 AND 512),
    ticket_expires_ms bigint NOT NULL,
    used_at_ms bigint,
    attests_rights boolean NOT NULL,
    depicts_real_person boolean NOT NULL,
    screen text CHECK (screen IN ('pending', 'passed', 'flagged', 'rejected', 'unjudged')),
    screen_attempt bytea CHECK (octet_length(screen_attempt) = 16),
    review_deadline_ms bigint
);

CREATE TABLE media.asset_reference (
    asset_id bytea NOT NULL REFERENCES media.asset (asset_id),
    holder_kind text NOT NULL CHECK (holder_kind IN ('save_point', 'template_revision',
        'campaign_character', 'campaign_share', 'export_job')),
    holder_id bytea NOT NULL CHECK (octet_length(holder_id) BETWEEN 16 AND 64),
    presentation text NOT NULL CHECK (presentation IN ('live', 'placeholder')),
    created_at_ms bigint NOT NULL,
    PRIMARY KEY (asset_id, holder_kind, holder_id)
);
CREATE INDEX asset_reference_holder ON media.asset_reference (holder_kind, holder_id);

CREATE TABLE media.asset_grant (
    grant_sha256 bytea PRIMARY KEY CHECK (octet_length(grant_sha256) = 32),
    asset_id bytea NOT NULL REFERENCES media.asset (asset_id),
    audience_kind text NOT NULL,
    principal_kind text NOT NULL CHECK (principal_kind IN ('member', 'display', 'owner')),
    principal_account bytea CHECK (octet_length(principal_account) = 16),
    member_id bytea CHECK (octet_length(member_id) = 16),
    member_revision bigint,
    ticket_id bytea CHECK (octet_length(ticket_id) = 16),
    room_epoch bigint,
    issued_at_ms bigint NOT NULL,
    expires_at_ms bigint NOT NULL,
    revoked_at_ms bigint
);
CREATE INDEX asset_grant_asset ON media.asset_grant (asset_id);

CREATE TABLE media.asset_audience_event (
    event_seq bigserial PRIMARY KEY,
    asset_id bytea NOT NULL REFERENCES media.asset (asset_id),
    audience_kind text NOT NULL,
    suppression_generation bigint NOT NULL,
    at_ms bigint NOT NULL
);

CREATE TABLE media.tombstone (
    tombstone_seq bigserial PRIMARY KEY,
    asset_id bytea NOT NULL REFERENCES media.asset (asset_id),
    suppression_generation bigint NOT NULL,
    reason text NOT NULL CHECK (reason IN ('user_delete', 'account_deletion', 'retention',
        'erasure', 'restore_replay')),
    at_ms bigint NOT NULL
);

-- Exactly one terminal report per attempt, consumed by F's settlement (D22, D27).
CREATE TABLE media.settlement_outbox (
    outbox_seq bigserial PRIMARY KEY,
    attempt bytea NOT NULL UNIQUE CHECK (octet_length(attempt) = 16),
    asset_id bytea NOT NULL REFERENCES media.asset (asset_id),
    outcome text NOT NULL CHECK (outcome IN ('ready', 'superseded', 'rejected', 'invalid')),
    reason text,
    at_ms bigint NOT NULL,
    consumed_at_ms bigint,
    CHECK ((outcome IN ('rejected', 'invalid')) = (reason IS NOT NULL))
);

CREATE FUNCTION media.refuse_change() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'media history rows are append-only';
END $$;

CREATE TRIGGER asset_audience_event_append_only BEFORE UPDATE OR DELETE
    ON media.asset_audience_event FOR EACH ROW EXECUTE FUNCTION media.refuse_change();
CREATE TRIGGER tombstone_append_only BEFORE UPDATE OR DELETE
    ON media.tombstone FOR EACH ROW EXECUTE FUNCTION media.refuse_change();
