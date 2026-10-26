-- Hosted save points (master launch plan 13.6-13.7, 15.4 SAVE-01..03; LAUNCH decisions 24, 31; O1).
-- Forward-only links to finance admission receipts and paid generation attempts are by id only:
-- no cross-owner foreign keys (decision 31). Finance rows are never written here, so a branch or
-- restore can never roll money or paid generations back.
CREATE SCHEMA IF NOT EXISTS df_save;
REVOKE ALL ON SCHEMA df_save FROM PUBLIC;

CREATE TABLE IF NOT EXISTS df_save.schema_migration (
    version integer PRIMARY KEY,
    applied_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE df_save.save_points (
    save_point_id bytea PRIMARY KEY CHECK (octet_length(save_point_id) = 16),
    tenant_id bytea NOT NULL CHECK (octet_length(tenant_id) = 16),
    campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16),
    branch_id bytea NOT NULL CHECK (octet_length(branch_id) = 16),
    parent_save_point_id bytea,
    codec_version integer NOT NULL CHECK (codec_version >= 1),
    schema_version integer NOT NULL CHECK (schema_version >= 1),
    content_version integer NOT NULL CHECK (content_version >= 1),
    checkpoint bytea NOT NULL CHECK (octet_length(checkpoint) > 0),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (save_point_id, tenant_id, campaign_id),
    FOREIGN KEY (parent_save_point_id, tenant_id, campaign_id)
        REFERENCES df_save.save_points (save_point_id, tenant_id, campaign_id)
);
CREATE INDEX save_points_campaign_branch
    ON df_save.save_points (tenant_id, campaign_id, branch_id, created_at);

CREATE TABLE df_save.branch_heads (
    tenant_id bytea NOT NULL CHECK (octet_length(tenant_id) = 16),
    campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16),
    branch_id bytea NOT NULL CHECK (octet_length(branch_id) = 16),
    head_save_point_id bytea NOT NULL,
    PRIMARY KEY (tenant_id, campaign_id, branch_id),
    FOREIGN KEY (head_save_point_id, tenant_id, campaign_id)
        REFERENCES df_save.save_points (save_point_id, tenant_id, campaign_id)
);

-- Commit-before-reply (SAVE-02): the operation result commits with the state in one
-- transaction; a retry returns the stored result, a changed fingerprint conflicts.
CREATE TABLE df_save.save_operations (
    tenant_id bytea NOT NULL CHECK (octet_length(tenant_id) = 16),
    campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16),
    operation_id bytea NOT NULL CHECK (octet_length(operation_id) = 16),
    fingerprint bytea NOT NULL CHECK (octet_length(fingerprint) = 32),
    result_save_point_id bytea NOT NULL,
    result_bytes bytea NOT NULL,
    PRIMARY KEY (tenant_id, campaign_id, operation_id),
    FOREIGN KEY (result_save_point_id, tenant_id, campaign_id)
        REFERENCES df_save.save_points (save_point_id, tenant_id, campaign_id)
);

CREATE TABLE df_save.save_links (
    save_point_id bytea NOT NULL REFERENCES df_save.save_points ON DELETE CASCADE,
    link_kind text NOT NULL CHECK (link_kind IN ('finance_admission', 'paid_generation')),
    linked_id bytea NOT NULL CHECK (octet_length(linked_id) = 16),
    PRIMARY KEY (save_point_id, link_kind, linked_id)
);

-- The newest owner fence claimed by a resume (decision 24); claim_seq only ever increases.
CREATE TABLE df_save.resume_fences (
    tenant_id bytea NOT NULL CHECK (octet_length(tenant_id) = 16),
    campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16),
    recovery_epoch bigint NOT NULL CHECK (recovery_epoch >= 1),
    claim_seq bigint NOT NULL CHECK (claim_seq >= 1),
    placement_id bytea NOT NULL CHECK (octet_length(placement_id) = 16),
    claimed_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, campaign_id, claim_seq)
);

DO $$
DECLARE
    v_table text;
BEGIN
    FOREACH v_table IN ARRAY ARRAY['schema_migration', 'save_points', 'branch_heads',
        'save_operations', 'save_links', 'resume_fences']
    LOOP
        EXECUTE format('ALTER TABLE df_save.%I ENABLE ROW LEVEL SECURITY', v_table);
        EXECUTE format('ALTER TABLE df_save.%I FORCE ROW LEVEL SECURITY', v_table);
    END LOOP;
    -- FORCE also applies to a non-superuser migrator; only that role manages its ledger.
    EXECUTE format('CREATE POLICY migration_owner ON df_save.schema_migration '
        'FOR ALL TO %I USING (true) WITH CHECK (true)', current_user);
    FOREACH v_table IN ARRAY ARRAY['save_points', 'branch_heads', 'save_operations',
        'resume_fences']
    LOOP
        EXECUTE format('CREATE POLICY runtime_scope ON df_save.%I '
            'FOR ALL TO df_campaign_runtime '
            'USING ((tenant_id, campaign_id) = '
            '(SELECT l.tenant_id, l.campaign_id FROM df_runtime.leased_campaign() l)) '
            'WITH CHECK ((tenant_id, campaign_id) = '
            '(SELECT l.tenant_id, l.campaign_id FROM df_runtime.leased_campaign() l))', v_table);
    END LOOP;
    FOREACH v_table IN ARRAY ARRAY['save_points', 'branch_heads', 'save_operations',
        'save_links', 'resume_fences']
    LOOP
        EXECUTE format('CREATE POLICY control_read ON df_save.%I '
            'FOR SELECT TO df_control USING (true)', v_table);
    END LOOP;
END $$;

-- Links inherit campaign scope from their immutable save point, including on INSERT.
CREATE POLICY runtime_scope ON df_save.save_links FOR ALL TO df_campaign_runtime
    USING (EXISTS (SELECT 1 FROM df_save.save_points p
        WHERE p.save_point_id = save_links.save_point_id))
    WITH CHECK (EXISTS (SELECT 1 FROM df_save.save_points p
        WHERE p.save_point_id = save_links.save_point_id));

GRANT USAGE ON SCHEMA df_save TO df_control, df_campaign_runtime;
GRANT SELECT ON df_save.save_points, df_save.branch_heads, df_save.save_operations,
    df_save.save_links, df_save.resume_fences TO df_control, df_campaign_runtime;
GRANT INSERT ON df_save.save_points, df_save.branch_heads, df_save.save_operations,
    df_save.save_links, df_save.resume_fences TO df_campaign_runtime;
GRANT UPDATE (head_save_point_id) ON df_save.branch_heads TO df_campaign_runtime;
GRANT EXECUTE ON FUNCTION df_runtime.current_recovery_epoch() TO df_control, df_campaign_runtime;
