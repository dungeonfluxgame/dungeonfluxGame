-- Immutable campaign image admission bindings (task IMG-ADMISSION-BINDING-A1).
-- Composition migration: apply only after media 0010, finance 0020/0022 and
-- imagery 0038 are all migrated. Never register in the standalone journal
-- migrations: the standalone journal must stay usable without finance/media.
--
-- What it owns: the original owner/job/stage allocation of one publication
-- attempt, and the exact finance receipt replay of its first admission.
-- What it never does: no FK from the original publication_attempt to the
-- mutable media.asset.attempt (replay reads history after catalog supersede);
-- no UPDATE or DELETE (narrow immutable trigger); no cascading deletes;
-- no public grants (only owner/superuser until the later definer binding).
-- Lock order: finance rows, then media quota/asset, then the imagery job,
-- then these rows. A withdraw transaction holding the job never takes
-- media/finance locks, so this order adds no inverse edge.

CREATE SCHEMA IF NOT EXISTS imagery;

CREATE TABLE IF NOT EXISTS imagery.image_publication_allocation (
    publication_attempt bytea NOT NULL CHECK (octet_length(publication_attempt) = 16 AND publication_attempt <> decode('00000000000000000000000000000000', 'hex')),
    asset_id bytea NOT NULL CHECK (octet_length(asset_id) = 16 AND asset_id <> decode('00000000000000000000000000000000', 'hex')) REFERENCES media.asset(asset_id),
    job_id bytea NOT NULL CHECK (octet_length(job_id) = 16 AND job_id <> decode('00000000000000000000000000000000', 'hex')),
    stage_id bytea NOT NULL CHECK (octet_length(stage_id) = 16 AND stage_id <> decode('00000000000000000000000000000000', 'hex')),
    owner_scope bytea NOT NULL CHECK (octet_length(owner_scope) IN (18, 34)),
    original_recipe_key bytea NOT NULL CHECK (octet_length(original_recipe_key) = 32),
    original_recipe_revision bigint NOT NULL CHECK (original_recipe_revision >= 1 AND original_recipe_revision <= 4294967295),
    original_recipe_bytes bytea NOT NULL CHECK (octet_length(original_recipe_bytes) >= 1 AND octet_length(original_recipe_bytes) <= 1048576),
    original_config_revision bigint NOT NULL CHECK (original_config_revision >= 1 AND original_config_revision <= 4294967295),
    original_config_bytes bytea NOT NULL CHECK (octet_length(original_config_bytes) >= 1 AND octet_length(original_config_bytes) <= 1048576),
    created_at_ms bigint NOT NULL CHECK (created_at_ms >= 0),
    CONSTRAINT image_publication_allocation_pkey PRIMARY KEY (publication_attempt),
    CONSTRAINT image_publication_allocation_asset_unique UNIQUE (asset_id),
    CONSTRAINT image_publication_allocation_job_stage UNIQUE (job_id, stage_id),
    CONSTRAINT image_publication_allocation_full_tuple UNIQUE (publication_attempt, job_id, stage_id, asset_id, owner_scope),
    CONSTRAINT image_publication_allocation_job_fkey FOREIGN KEY (job_id, owner_scope)
        REFERENCES imagery.image_job (job_id, owner_scope)
        DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE IF NOT EXISTS imagery.image_finance_admission_binding (
    finance_attempt bytea NOT NULL CHECK (octet_length(finance_attempt) = 16 AND finance_attempt <> decode('00000000000000000000000000000000', 'hex')),
    publication_attempt bytea NOT NULL CHECK (octet_length(publication_attempt) = 16 AND publication_attempt <> decode('00000000000000000000000000000000', 'hex')),
    job_id bytea NOT NULL CHECK (octet_length(job_id) = 16 AND job_id <> decode('00000000000000000000000000000000', 'hex')),
    stage_id bytea NOT NULL CHECK (octet_length(stage_id) = 16 AND stage_id <> decode('00000000000000000000000000000000', 'hex')),
    asset_id bytea NOT NULL CHECK (octet_length(asset_id) = 16 AND asset_id <> decode('00000000000000000000000000000000', 'hex')),
    owner_scope bytea NOT NULL CHECK (octet_length(owner_scope) IN (18, 34)),
    tenant_id bytea NOT NULL CHECK (octet_length(tenant_id) = 16),
    campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16),
    session_id bytea NOT NULL CHECK (octet_length(session_id) = 16),
    operation_id bytea NOT NULL CHECK (octet_length(operation_id) = 16),
    effect_id bytea NOT NULL CHECK (octet_length(effect_id) = 16),
    receipt_namespace text NOT NULL CHECK (receipt_namespace = 'admission'),
    receipt_fingerprint bytea NOT NULL CHECK (octet_length(receipt_fingerprint) = 32),
    reservation_id bytea NOT NULL CHECK (octet_length(reservation_id) = 16),
    created_at_ms bigint NOT NULL CHECK (created_at_ms >= 0),
    CONSTRAINT image_finance_admission_binding_pkey PRIMARY KEY (finance_attempt),
    CONSTRAINT image_finance_admission_binding_attempt_fkey FOREIGN KEY (finance_attempt)
        REFERENCES finance.provider_attempt(attempt_id)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT image_finance_admission_binding_allocation_fkey FOREIGN KEY (publication_attempt, job_id, stage_id, asset_id, owner_scope)
        REFERENCES imagery.image_publication_allocation (publication_attempt, job_id, stage_id, asset_id, owner_scope)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT image_finance_admission_binding_scoped_operation UNIQUE (receipt_namespace, campaign_id, operation_id),
    CONSTRAINT image_finance_admission_binding_receipt_fkey FOREIGN KEY (receipt_namespace, campaign_id, operation_id)
        REFERENCES finance.operation_receipt(namespace, principal, operation_id)
        DEFERRABLE INITIALLY DEFERRED
);

CREATE OR REPLACE FUNCTION imagery.forbid_admission_mutation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'imagery admission bindings are immutable';
END
$$;

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_trigger WHERE tgname = 'image_publication_allocation_immutable') THEN
        CREATE TRIGGER image_publication_allocation_immutable
            BEFORE UPDATE OR DELETE ON imagery.image_publication_allocation
            FOR EACH ROW EXECUTE FUNCTION imagery.forbid_admission_mutation();
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_trigger WHERE tgname = 'image_finance_admission_binding_immutable') THEN
        CREATE TRIGGER image_finance_admission_binding_immutable
            BEFORE UPDATE OR DELETE ON imagery.image_finance_admission_binding
            FOR EACH ROW EXECUTE FUNCTION imagery.forbid_admission_mutation();
    END IF;
END
$$;

REVOKE ALL ON imagery.image_publication_allocation FROM PUBLIC;
REVOKE ALL ON imagery.image_finance_admission_binding FROM PUBLIC;

-- Imagery admission composition role (tasks IMG-ADMISSION-BINDING-A5/A6).
--
-- Real-PostgreSQL A4 evidence: the finance service connection runs `SET ROLE df_finance`,
-- and every actual admission then fails with ImageStageUnavailable SQLSTATE 42501, because
-- df_finance holds finance SELECT/EXECUTE only and has no privilege on the media or imagery
-- rows the port touches. This block gives those privileges to one dedicated NOLOGIN
-- composition role instead of widening df_finance, so ordinary finance authority is unchanged.
--
-- Role contract:
-- - df_imagery_admission is created NOLOGIN, NOSUPERUSER, NOCREATEDB, NOCREATEROLE,
--   NOREPLICATION and NOBYPASSRLS, and owns nothing. An existing role keeps its LOGIN setting:
--   the migration never forces NOLOGIN, so deployment-added LOGIN and a password per
--   environment survive, exactly like the 0018 runtime roles; the migrator needs the same
--   role-management capability that 0018 and 0020 already assume. The fresh role defaults to
--   INHERIT, so the df_finance membership below stays effective; deployment must preserve it.
-- - GRANT df_finance TO df_imagery_admission inherits the ordinary finance service permission
--   (USAGE on schema finance, SELECT on its tables, EXECUTE on lock_flow/commit_event and the
--   other finance entry points). The reverse is never granted: df_finance gains nothing here
--   and still cannot read or write any media or imagery row.
-- - Nothing is granted to PUBLIC, no ALL PRIVILEGES anywhere, and no direct finance DML: the
--   composition role inherits finance SELECT only, so an INSERT, UPDATE or DELETE on any finance
--   table stays 42501. Finance rows change only through the SECURITY DEFINER lock_flow and
--   commit_event functions.
-- - The service runs the whole admission transaction as df_imagery_admission (connect, or SET
--   ROLE as the real-PostgreSQL tests do). Never connect as the table owner or a superuser to
--   work around the grants.
--
-- Per-statement grants (the exact call chain of the port; see imagery_admission/rows.rs and
-- media_catalog_sql/admission.rs and locks.rs):
-- - media.quota_usage SELECT, INSERT, UPDATE: lock_quota inserts the scope row if absent, locks
--   it FOR UPDATE, and adjust_usage moves the reservation on admit.
-- - media.asset SELECT, INSERT, DELETE plus UPDATE(updated_at_ms): admit inserts the asset
--   row, the key-conflict read looks the live row up, check_catalog_identity re-reads it
--   FOR UPDATE, and a quota refusal deletes the just-inserted uncommitted row. PostgreSQL
--   requires UPDATE on at least one column for FOR UPDATE, so the grant is narrow to
--   updated_at_ms only: the role can touch that timestamp but state, attempt, charge_scope,
--   owner, audience and snapshot columns stay ungranted (42501). This path never updates one.
-- - media.quota_reservation INSERT: admit records the reservation. No SELECT or DELETE: this
--   path never reads or releases a reservation (release and the settlement outbox belong to the
--   finalize and reject paths owned by other roles).
-- - imagery.image_job SELECT plus UPDATE(updated_at_ms): check_frozen_job reads the frozen job
--   row FOR UPDATE. PostgreSQL requires UPDATE on at least one column for FOR UPDATE, so the
--   grant is narrow to updated_at_ms only: the role can touch that timestamp but state,
--   owner_scope, recipe, config and revision columns stay ungranted (42501).
-- - imagery.image_interest SELECT: check_frozen_job counts attached interests.
-- - imagery.image_publication_allocation SELECT, INSERT: the pre-write existence check and the
--   immutable insert.
-- - imagery.image_finance_admission_binding SELECT, INSERT: the replay read and the exact
--   receipt-replay insert.
-- - USAGE on schemas media and imagery so the role resolves those tables. Finance schema USAGE
--   arrives through the df_finance membership.

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'df_imagery_admission') THEN
        CREATE ROLE df_imagery_admission NOLOGIN;
    END IF;
    EXECUTE 'ALTER ROLE df_imagery_admission NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS';
END $$;

GRANT df_finance TO df_imagery_admission;

GRANT USAGE ON SCHEMA media TO df_imagery_admission;
GRANT USAGE ON SCHEMA imagery TO df_imagery_admission;
GRANT SELECT, INSERT, UPDATE ON media.quota_usage TO df_imagery_admission;
GRANT SELECT, INSERT, DELETE ON media.asset TO df_imagery_admission;
GRANT UPDATE (updated_at_ms) ON media.asset TO df_imagery_admission;
GRANT INSERT ON media.quota_reservation TO df_imagery_admission;
GRANT SELECT ON imagery.image_job TO df_imagery_admission;
GRANT UPDATE (updated_at_ms) ON imagery.image_job TO df_imagery_admission;
GRANT SELECT ON imagery.image_interest TO df_imagery_admission;
GRANT SELECT, INSERT ON imagery.image_publication_allocation TO df_imagery_admission;
GRANT SELECT, INSERT ON imagery.image_finance_admission_binding TO df_imagery_admission;

CREATE INDEX IF NOT EXISTS image_publication_allocation_job
    ON imagery.image_publication_allocation (job_id);
CREATE INDEX IF NOT EXISTS image_finance_admission_binding_publication
    ON imagery.image_finance_admission_binding (publication_attempt);
