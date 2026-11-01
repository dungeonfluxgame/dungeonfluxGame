-- Canonical content lookup is an index hint; full retained snapshots and authority still decide reuse.
ALTER TABLE imagery.image_job
    ADD COLUMN IF NOT EXISTS compatible_key bytea
    CHECK (compatible_key IS NULL OR octet_length(compatible_key)=32);
CREATE INDEX IF NOT EXISTS image_job_compatible
    ON imagery.image_job(owner_scope,compatible_key,created_at_ms DESC,job_id DESC);
