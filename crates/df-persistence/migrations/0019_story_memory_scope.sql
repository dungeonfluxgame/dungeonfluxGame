-- Story memory scope for the hosted database (planning/hosted-runtime-research.md 4.4 item 5;
-- O1). Applied after 0003 on a database whose story tables are empty: hosted rows are written
-- with their tenant and campaign, and importing a local campaign writes new rows. Not applied
-- to a local run database.

ALTER TABLE df_story.story_memory
    ADD COLUMN tenant_id bytea NOT NULL CHECK (octet_length(tenant_id) = 16),
    ADD COLUMN campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16);
ALTER TABLE df_story.memory_recalls
    ADD COLUMN tenant_id bytea NOT NULL CHECK (octet_length(tenant_id) = 16),
    ADD COLUMN campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16);
ALTER TABLE df_story.entity_dossiers
    ADD COLUMN tenant_id bytea NOT NULL CHECK (octet_length(tenant_id) = 16),
    ADD COLUMN campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16);

CREATE INDEX story_memory_campaign_recent
    ON df_story.story_memory (tenant_id, campaign_id, event_id DESC);
CREATE INDEX memory_recalls_campaign
    ON df_story.memory_recalls (tenant_id, campaign_id, recall_id DESC);
CREATE INDEX entity_dossiers_campaign
    ON df_story.entity_dossiers (tenant_id, campaign_id);

-- The append-only escape needs both the setting and membership of df_story_maintenance, so a
-- runtime that sets df_story.maintenance gains nothing (the trigger is SECURITY INVOKER).
CREATE OR REPLACE FUNCTION df_story.story_memory_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF current_setting('df_story.maintenance', true) IS NOT DISTINCT FROM 'on'
       AND pg_has_role(current_user, 'df_story_maintenance', 'member') THEN
        RETURN COALESCE(NEW, OLD);
    END IF;
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'story memory is append-only';
    END IF;
    IF NEW.event_id IS DISTINCT FROM OLD.event_id
       OR NEW.run_id IS DISTINCT FROM OLD.run_id
       OR NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
       OR NEW.campaign_id IS DISTINCT FROM OLD.campaign_id
       OR NEW.natural_key IS DISTINCT FROM OLD.natural_key
       OR NEW.source IS DISTINCT FROM OLD.source
       OR NEW.source_epoch IS DISTINCT FROM OLD.source_epoch
       OR NEW.source_seq IS DISTINCT FROM OLD.source_seq
       OR NEW.game_revision IS DISTINCT FROM OLD.game_revision
       OR NEW.created_at IS DISTINCT FROM OLD.created_at
       OR NEW.kind IS DISTINCT FROM OLD.kind
       OR NEW.visibility IS DISTINCT FROM OLD.visibility
       OR NEW.private_to IS DISTINCT FROM OLD.private_to
       OR NEW.body IS DISTINCT FROM OLD.body
       OR NEW.body_digest IS DISTINCT FROM OLD.body_digest
       OR NEW.scene IS DISTINCT FROM OLD.scene
       OR NEW.npc_ids IS DISTINCT FROM OLD.npc_ids
       OR NEW.hero_ids IS DISTINCT FROM OLD.hero_ids
       OR NEW.place_ids IS DISTINCT FROM OLD.place_ids
       OR NEW.item_ids IS DISTINCT FROM OLD.item_ids
       OR NEW.rebuildable IS DISTINCT FROM OLD.rebuildable THEN
        RAISE EXCEPTION 'story memory events are immutable';
    END IF;
    IF OLD.status = 'superseded' AND NEW.status <> 'superseded' THEN
        RAISE EXCEPTION 'a superseded story memory event stays superseded';
    END IF;
    IF OLD.supersedes_id IS NOT NULL AND NEW.supersedes_id IS DISTINCT FROM OLD.supersedes_id THEN
        RAISE EXCEPTION 'story memory supersession links are set once';
    END IF;
    IF OLD.superseded_by IS NOT NULL AND NEW.superseded_by IS DISTINCT FROM OLD.superseded_by THEN
        RAISE EXCEPTION 'story memory supersession links are set once';
    END IF;
    RETURN NEW;
END;
$$;

DO $$
DECLARE
    v_table text;
BEGIN
    FOREACH v_table IN ARRAY ARRAY['story_memory', 'memory_recalls', 'entity_dossiers']
    LOOP
        EXECUTE format('ALTER TABLE df_story.%I ENABLE ROW LEVEL SECURITY', v_table);
        EXECUTE format('ALTER TABLE df_story.%I FORCE ROW LEVEL SECURITY', v_table);
        EXECUTE format('CREATE POLICY runtime_scope ON df_story.%I FOR ALL TO df_campaign_runtime '
            'USING ((tenant_id, campaign_id) = (SELECT l.tenant_id, l.campaign_id '
            'FROM df_runtime.leased_campaign() l)) '
            'WITH CHECK ((tenant_id, campaign_id) = (SELECT l.tenant_id, l.campaign_id '
            'FROM df_runtime.leased_campaign() l))', v_table);
        EXECUTE format('CREATE POLICY maintenance_all ON df_story.%I FOR ALL '
            'TO df_story_maintenance USING (true) WITH CHECK (true)', v_table);
    END LOOP;
END $$;

GRANT USAGE ON SCHEMA df_story TO df_campaign_runtime, df_story_maintenance;
GRANT SELECT, INSERT, UPDATE ON df_story.story_memory, df_story.entity_dossiers
    TO df_campaign_runtime;
GRANT SELECT, INSERT ON df_story.memory_recalls TO df_campaign_runtime;
GRANT SELECT, UPDATE, DELETE ON df_story.story_memory, df_story.memory_recalls,
    df_story.entity_dossiers TO df_story_maintenance;

-- The importer reads and inserts the story rows of the one campaign its import job names.
GRANT USAGE ON SCHEMA df_story TO df_importer;
GRANT SELECT, INSERT ON df_story.story_memory, df_story.memory_recalls,
    df_story.entity_dossiers TO df_importer;
DO $$
DECLARE
    v_table text;
BEGIN
    FOREACH v_table IN ARRAY ARRAY['story_memory', 'memory_recalls', 'entity_dossiers']
    LOOP
        EXECUTE format('CREATE POLICY importer_read ON df_story.%I FOR SELECT TO df_importer '
            'USING ((tenant_id, campaign_id) IN '
            '(SELECT i.tenant_id, i.campaign_id FROM df_runtime.importing_campaign() i))',
            v_table);
        EXECUTE format('CREATE POLICY importer_insert ON df_story.%I FOR INSERT TO df_importer '
            'WITH CHECK ((tenant_id, campaign_id) IN '
            '(SELECT i.tenant_id, i.campaign_id FROM df_runtime.importing_campaign() i))',
            v_table);
    END LOOP;
END $$;
