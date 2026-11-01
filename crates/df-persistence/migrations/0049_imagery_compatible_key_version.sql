-- An indexed digest is only a hint; older or unknown key schemas retain bounded full comparison.
ALTER TABLE imagery.image_job
    ADD COLUMN IF NOT EXISTS compatible_key_version smallint
    CHECK (
        compatible_key_version IS NULL
        OR (compatible_key IS NOT NULL AND compatible_key_version > 0)
    );
