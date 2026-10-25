-- Account and host exports (planning/asset-storage-research.md 12; plan 13.11, J14; D14). The
-- archive is encrypted under a per-export data key stored only wrapped under S2's key-encryption
-- key; expiry deletes the archive and this wrapped key. The Exports bucket is never versioned.

CREATE TABLE media.export_job (
    export_id bytea PRIMARY KEY CHECK (octet_length(export_id) = 16),
    account_id bytea NOT NULL CHECK (octet_length(account_id) = 16),
    kind text NOT NULL CHECK (kind IN ('account', 'campaign_host')),
    campaign_id bytea CHECK (octet_length(campaign_id) = 16),
    state text NOT NULL CHECK (state IN ('requested', 'building', 'ready', 'expired', 'failed')),
    asset_id bytea REFERENCES media.asset (asset_id),
    wrapped_data_key bytea CHECK (octet_length(wrapped_data_key) BETWEEN 32 AND 128),
    requested_at_ms bigint NOT NULL,
    ready_at_ms bigint,
    expires_at_ms bigint,
    CHECK ((kind = 'campaign_host') = (campaign_id IS NOT NULL)),
    CHECK (state <> 'ready' OR (asset_id IS NOT NULL AND wrapped_data_key IS NOT NULL
        AND expires_at_ms IS NOT NULL)),
    CHECK (state <> 'expired' OR wrapped_data_key IS NULL)
);
CREATE INDEX export_job_account ON media.export_job (account_id, requested_at_ms);

CREATE TABLE media.export_access (
    access_seq bigserial PRIMARY KEY,
    export_id bytea NOT NULL REFERENCES media.export_job (export_id),
    account_id bytea NOT NULL CHECK (octet_length(account_id) = 16),
    byte_length bigint NOT NULL CHECK (byte_length >= 0),
    at_ms bigint NOT NULL
);

CREATE TRIGGER export_access_append_only BEFORE UPDATE OR DELETE
    ON media.export_access FOR EACH ROW EXECUTE FUNCTION media.refuse_change();
