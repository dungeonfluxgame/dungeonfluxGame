CREATE TABLE imagery.image_dispatch_usage (
    dispatch_id bytea NOT NULL REFERENCES imagery.image_dispatch_attempt(dispatch_id),
    owner_scope bytea NOT NULL,
    job_id bytea NOT NULL REFERENCES imagery.image_job(job_id),
    stage_id bytea NOT NULL,
    revision bigint NOT NULL CHECK (revision > 0),
    provider text NOT NULL CHECK (octet_length(provider) BETWEEN 1 AND 120),
    model text NOT NULL CHECK (octet_length(model) BETWEEN 1 AND 128),
    cause text NOT NULL CHECK (octet_length(cause) BETWEEN 1 AND 120),
    detail text NOT NULL CHECK (octet_length(detail) BETWEEN 1 AND 120),
    maxima jsonb NOT NULL CHECK (jsonb_typeof(maxima) = 'array'),
    evidence text NOT NULL CHECK (evidence IN ('reported','estimated','unknown')),
    units jsonb NOT NULL CHECK (jsonb_typeof(units) = 'array'),
    usable boolean NOT NULL,
    observed_at_ms bigint NOT NULL CHECK (observed_at_ms >= 0),
    raw_output bytea,
    raw_output_sha256 bytea GENERATED ALWAYS AS (sha256(raw_output)) STORED,
    projection_durable boolean NOT NULL DEFAULT false,
    PRIMARY KEY (dispatch_id, revision),
    FOREIGN KEY (job_id, owner_scope) REFERENCES imagery.image_job(job_id, owner_scope),
    FOREIGN KEY (job_id, stage_id, dispatch_id)
        REFERENCES imagery.image_dispatch_attempt(job_id, stage_id, dispatch_id),
    FOREIGN KEY (job_id, stage_id) REFERENCES imagery.image_execution_stage(job_id, stage_id)
);
CREATE INDEX image_dispatch_usage_stage
    ON imagery.image_dispatch_usage(job_id, stage_id);

CREATE INDEX image_dispatch_usage_pending ON imagery.image_dispatch_usage(job_id,stage_id)
    WHERE NOT projection_durable;

REVOKE ALL ON imagery.image_dispatch_usage FROM PUBLIC;
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='df_imagery_usage') THEN
        CREATE ROLE df_imagery_usage NOLOGIN;
    END IF;
    ALTER ROLE df_imagery_usage NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
END $$;
GRANT USAGE ON SCHEMA imagery TO df_imagery_usage;
GRANT SELECT ON imagery.image_job,imagery.image_execution_stage,imagery.image_dispatch_attempt
    TO df_imagery_usage;
GRANT SELECT,INSERT ON imagery.image_dispatch_usage TO df_imagery_usage;
GRANT UPDATE(projection_durable) ON imagery.image_dispatch_usage TO df_imagery_usage;

GRANT SELECT ON imagery.image_dispatch_usage TO df_imagery_publication;

CREATE TABLE imagery.image_dispatch_sources (
    dispatch_id bytea PRIMARY KEY REFERENCES imagery.image_dispatch_attempt(dispatch_id),
    owner_scope bytea NOT NULL,
    job_id bytea NOT NULL,
    stage_id bytea NOT NULL,
    receipts jsonb NOT NULL CHECK (jsonb_typeof(receipts)='array'),
    source_digest bytea NOT NULL CHECK(octet_length(source_digest)=32),
    FOREIGN KEY (job_id,owner_scope) REFERENCES imagery.image_job(job_id,owner_scope),
    FOREIGN KEY (job_id,stage_id,dispatch_id)
        REFERENCES imagery.image_dispatch_attempt(job_id,stage_id,dispatch_id)
);
REVOKE ALL ON imagery.image_dispatch_sources FROM PUBLIC;
GRANT SELECT,INSERT ON imagery.image_dispatch_sources TO df_imagery_usage;
GRANT SELECT ON imagery.image_dispatch_sources TO df_imagery_publication;
GRANT SELECT,INSERT ON imagery.image_dispatch_sources TO df_imagery_admission;

GRANT SELECT ON imagery.image_dispatch_usage TO df_imagery_admission;

DO $$ BEGIN
    IF to_regclass('imagery.image_no_delivery') IS NOT NULL THEN
        GRANT SELECT ON imagery.image_no_delivery TO df_imagery_usage;
    END IF;
END $$;
