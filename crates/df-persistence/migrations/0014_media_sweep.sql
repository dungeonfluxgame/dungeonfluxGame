-- Sweep support for the media catalog (planning/asset-storage-research.md 6.2): staged objects
-- that no row names any more, and the indexes the sweeper reads. A staged object is recorded on
-- its asset row; when an attempt is superseded the row moves on to the new attempt, so the old
-- object is kept here until the sweep deletes it.

ALTER TABLE media.asset ADD COLUMN staged_deleted_at_ms bigint;

CREATE TABLE media.staged_orphan (
    asset_id bytea NOT NULL REFERENCES media.asset (asset_id),
    attempt bytea NOT NULL CHECK (octet_length(attempt) = 16),
    object_key text NOT NULL,
    object_version text NOT NULL,
    superseded_at_ms bigint NOT NULL,
    PRIMARY KEY (asset_id, attempt)
);

CREATE INDEX asset_due ON media.asset (updated_at_ms, asset_id)
    WHERE state IN ('uploading', 'quarantined', 'validated');
CREATE INDEX asset_staged_cleanup ON media.asset (updated_at_ms)
    WHERE staged_key IS NOT NULL AND staged_deleted_at_ms IS NULL
        AND (state = 'rejected' OR ready_at_ms IS NOT NULL);
CREATE INDEX asset_variant_sweep ON media.asset_variant (created_at_ms);

-- The hub looks a request key up whatever the row's state (ready, archived, tombstoned or
-- absent); the unique index above covers live rows only, so this one serves the lookup.
CREATE INDEX asset_request_key_lookup ON media.asset (dedupe_scope, request_key)
    WHERE request_key IS NOT NULL;

-- Access (owner ruling, 2026-10-25): only the control-plane role reaches these schemas. No
-- runtime role holds any privilege on them, so row-level security is not needed; the catalog-walk
-- test in df-persistence fails if any role other than the owner and df_control gains one.
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'df_control') THEN
        CREATE ROLE df_control NOLOGIN;
    END IF;
END $$;

REVOKE ALL ON SCHEMA media, library, journal FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA media, library, journal FROM PUBLIC;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA media, library, journal FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA media, library, journal FROM PUBLIC;
ALTER DEFAULT PRIVILEGES IN SCHEMA media, library, journal REVOKE ALL ON FUNCTIONS FROM PUBLIC;

GRANT USAGE ON SCHEMA media, library, journal TO df_control;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA media, library, journal
    TO df_control;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA media, library, journal TO df_control;
