-- Durable imagery ownership bridge (task IMG-HOST-IDENTITY-20261028-A1).
-- One closed authority context per row: hosted bindings name a real customer
-- finance tuple, local rows name only the local guest domain. The two never mix.
-- No money, credentials, prompts, reference bytes or private game data here.
-- Finance retains its existing member/fence authority. The imagery run check
-- locks canonical campaign/session and gameplay lease before media/job locks.

CREATE SCHEMA IF NOT EXISTS imagery;

-- Canonical hosted run binding: one trusted (tenant, campaign, hosted session)
-- keeps its gameplay tenant/session/run immutable. A current recovery fence
-- may replace its predecessor with a monotonic epoch and retained fence history.
CREATE TABLE IF NOT EXISTS imagery.image_host_run_binding (
    tenant_id bytea NOT NULL CHECK (octet_length(tenant_id) = 16 AND tenant_id <> decode('00000000000000000000000000000000', 'hex')),
    campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16 AND campaign_id <> decode('00000000000000000000000000000000', 'hex')),
    hosted_session_id bytea NOT NULL CHECK (octet_length(hosted_session_id) = 16 AND hosted_session_id <> decode('00000000000000000000000000000000', 'hex')),
    gameplay_tenant_id bytea NOT NULL CHECK (octet_length(gameplay_tenant_id) = 16),
    gameplay_session_id bytea NOT NULL CHECK (octet_length(gameplay_session_id) = 16 AND gameplay_session_id <> decode('00000000000000000000000000000000', 'hex')),
    run_id bytea NOT NULL CHECK (octet_length(run_id) = 16 AND run_id <> decode('00000000000000000000000000000000', 'hex')),
    owner_fence bytea NOT NULL CHECK (octet_length(owner_fence) = 16 AND owner_fence <> decode('00000000000000000000000000000000', 'hex')),
    owner_epoch bigint NOT NULL DEFAULT 1 CHECK (owner_epoch > 0),
    created_at_ms bigint NOT NULL CHECK (created_at_ms >= 0),
    CONSTRAINT image_host_run_binding_pkey PRIMARY KEY (tenant_id, campaign_id, hosted_session_id),
    UNIQUE (gameplay_tenant_id, gameplay_session_id),
    UNIQUE (tenant_id, campaign_id, hosted_session_id, run_id, gameplay_session_id)
);

-- Canonical hosted seat binding: one customer account/member maps to one
-- gameplay member under one run/session and one active generation. Rotation
-- replaces the generation, reclaim clears `active`; both preserve the account.
-- Only the generation, activity and timestamps may change on UPDATE.
CREATE TABLE IF NOT EXISTS imagery.image_host_seat_binding (
    tenant_id bytea NOT NULL CHECK (octet_length(tenant_id) = 16 AND tenant_id <> decode('00000000000000000000000000000000', 'hex')),
    campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16 AND campaign_id <> decode('00000000000000000000000000000000', 'hex')),
    hosted_session_id bytea NOT NULL CHECK (octet_length(hosted_session_id) = 16 AND hosted_session_id <> decode('00000000000000000000000000000000', 'hex')),
    account_id bytea NOT NULL CHECK (octet_length(account_id) = 16 AND account_id <> decode('00000000000000000000000000000000', 'hex')),
    member_id bytea NOT NULL CHECK (octet_length(member_id) = 16 AND member_id <> decode('00000000000000000000000000000000', 'hex')),
    gameplay_member_id bytea NOT NULL CHECK (octet_length(gameplay_member_id) = 16 AND gameplay_member_id <> decode('00000000000000000000000000000000', 'hex')),
    run_id bytea NOT NULL CHECK (octet_length(run_id) = 16 AND run_id <> decode('00000000000000000000000000000000', 'hex')),
    gameplay_session_id bytea NOT NULL CHECK (octet_length(gameplay_session_id) = 16 AND gameplay_session_id <> decode('00000000000000000000000000000000', 'hex')),
    generation bytea NOT NULL CHECK (octet_length(generation) = 16 AND generation <> decode('00000000000000000000000000000000', 'hex')),
    active boolean NOT NULL DEFAULT true,
    created_at_ms bigint NOT NULL CHECK (created_at_ms >= 0),
    updated_at_ms bigint NOT NULL CHECK (updated_at_ms >= 0),
    CONSTRAINT image_host_seat_binding_pkey PRIMARY KEY (tenant_id, campaign_id, hosted_session_id, account_id, member_id),
    UNIQUE (tenant_id, campaign_id, hosted_session_id, member_id),
    UNIQUE (tenant_id, campaign_id, hosted_session_id, gameplay_member_id),
    FOREIGN KEY (tenant_id, campaign_id, hosted_session_id, run_id, gameplay_session_id)
        REFERENCES imagery.image_host_run_binding
            (tenant_id, campaign_id, hosted_session_id, run_id, gameplay_session_id)
);

CREATE INDEX IF NOT EXISTS image_host_seat_binding_run
    ON imagery.image_host_seat_binding (tenant_id, campaign_id, hosted_session_id)
    WHERE active;

-- Local identities are independent per authenticated session member. The
-- random account survives credential replacement; current grants remain the
-- authorization authority. No local row supplies hosted finance authority.
CREATE TABLE IF NOT EXISTS imagery.image_local_guest (
    member_id bytea NOT NULL CHECK (octet_length(member_id) = 16),
    session_id bytea NOT NULL CHECK (octet_length(session_id) = 16),
    PRIMARY KEY (session_id, member_id),
    guest_account_id bytea NOT NULL UNIQUE CHECK (octet_length(guest_account_id) = 16 AND guest_account_id <> decode('00000000000000000000000000000000', 'hex')),
    operation_id bytea NOT NULL CHECK (octet_length(operation_id) = 16 AND operation_id <> decode('00000000000000000000000000000000', 'hex')),
    fingerprint bytea NOT NULL CHECK (octet_length(fingerprint) = 32),
    generation bytea NOT NULL CHECK (octet_length(generation) = 16 AND generation <> decode('00000000000000000000000000000000', 'hex')),
    created_at_ms bigint NOT NULL CHECK (created_at_ms >= 0),
    updated_at_ms bigint NOT NULL CHECK (updated_at_ms >= 0)
);

CREATE OR REPLACE FUNCTION imagery.forbid_host_run_mutation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'UPDATE' AND NEW.tenant_id = OLD.tenant_id
        AND NEW.campaign_id = OLD.campaign_id
        AND NEW.hosted_session_id = OLD.hosted_session_id
        AND NEW.gameplay_tenant_id = OLD.gameplay_tenant_id
        AND NEW.gameplay_session_id = OLD.gameplay_session_id
        AND NEW.run_id = OLD.run_id AND NEW.created_at_ms = OLD.created_at_ms
        AND NEW.owner_fence <> OLD.owner_fence
        AND NEW.owner_epoch = OLD.owner_epoch + 1 THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'imagery host run identity is immutable';
END;
$$;

CREATE OR REPLACE FUNCTION imagery.forbid_local_guest_mutation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'imagery local guest bootstrap is append-only';
END;
$$;

-- A seat UPDATE may only rotate the generation or reclaim activity: the
-- tenant, campaign, sessions, account, member, gameplay member and run that
-- name the binding never change, so rotation preserves the account.
CREATE OR REPLACE FUNCTION imagery.constrain_host_seat_update() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
        OR NEW.campaign_id IS DISTINCT FROM OLD.campaign_id
        OR NEW.hosted_session_id IS DISTINCT FROM OLD.hosted_session_id
        OR NEW.account_id IS DISTINCT FROM OLD.account_id
        OR NEW.member_id IS DISTINCT FROM OLD.member_id
        OR NEW.gameplay_member_id IS DISTINCT FROM OLD.gameplay_member_id
        OR NEW.run_id IS DISTINCT FROM OLD.run_id
        OR NEW.gameplay_session_id IS DISTINCT FROM OLD.gameplay_session_id
        OR NEW.created_at_ms IS DISTINCT FROM OLD.created_at_ms THEN
        RAISE EXCEPTION 'imagery host seat identity columns are immutable';
    END IF;
    IF NOT OLD.active AND NEW.active THEN
        RAISE EXCEPTION 'revoked imagery seat cannot be resurrected';
    END IF;
    IF NEW.generation = decode('00000000000000000000000000000000', 'hex') THEN
        RAISE EXCEPTION 'imagery host seat generation stays nonzero';
    END IF;
    RETURN NEW;
END;
$$;

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_trigger WHERE tgname = 'image_host_run_binding_immutable') THEN
        CREATE TRIGGER image_host_run_binding_immutable
            BEFORE UPDATE OR DELETE ON imagery.image_host_run_binding
            FOR EACH ROW EXECUTE FUNCTION imagery.forbid_host_run_mutation();
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_trigger WHERE tgname = 'image_local_guest_immutable') THEN
        CREATE TRIGGER image_local_guest_immutable
            BEFORE UPDATE OR DELETE ON imagery.image_local_guest
            FOR EACH ROW EXECUTE FUNCTION imagery.forbid_local_guest_mutation();
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_trigger WHERE tgname = 'image_host_seat_binding_constrained') THEN
        CREATE TRIGGER image_host_seat_binding_constrained
            BEFORE UPDATE ON imagery.image_host_seat_binding
            FOR EACH ROW EXECUTE FUNCTION imagery.constrain_host_seat_update();
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_trigger WHERE tgname = 'image_host_seat_binding_no_delete') THEN
        CREATE TRIGGER image_host_seat_binding_no_delete
            BEFORE DELETE ON imagery.image_host_seat_binding
            FOR EACH ROW EXECUTE FUNCTION imagery.forbid_host_run_mutation();
    END IF;
END
$$;

REVOKE ALL ON imagery.image_host_run_binding FROM PUBLIC;
REVOKE ALL ON imagery.image_host_seat_binding FROM PUBLIC;
REVOKE ALL ON imagery.image_local_guest FROM PUBLIC;

CREATE TABLE IF NOT EXISTS imagery.image_host_fence_history (
    tenant_id bytea NOT NULL,
    campaign_id bytea NOT NULL,
    hosted_session_id bytea NOT NULL,
    owner_fence bytea NOT NULL,
    PRIMARY KEY (tenant_id, campaign_id, hosted_session_id, owner_fence),
    FOREIGN KEY (tenant_id, campaign_id, hosted_session_id)
        REFERENCES imagery.image_host_run_binding
);
REVOKE ALL ON imagery.image_host_fence_history FROM PUBLIC;

-- Canonical rows are locked until the admission commits. The definer already
-- owns the O1 tables and their RLS policy; callers cannot mutate authority rows.
CREATE OR REPLACE FUNCTION imagery.current_run(
    p_tenant bytea, p_campaign bytea, p_hosted bytea, p_game_tenant bytea,
    p_session bytea, p_run bytea, p_fence bytea) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    PERFORM 1 FROM campaigns.campaign
      WHERE tenant_id=p_tenant AND campaign_id=p_campaign AND status='active' FOR SHARE;
    IF NOT FOUND THEN RETURN false; END IF;
    PERFORM 1 FROM df_runtime.table_sessions
      WHERE tenant_id=p_tenant AND campaign_id=p_campaign AND session_id=p_hosted
        AND phase IN ('preparing','active','paused') FOR SHARE;
    IF NOT FOUND THEN RETURN false; END IF;
    PERFORM 1 FROM df_game.sessions
      WHERE tenant_id=p_game_tenant AND session_id=p_session AND run_id=p_run
        AND owner_fence=p_fence AND lease_until>clock_timestamp() FOR SHARE;
    RETURN FOUND;
END $$;
REVOKE ALL ON FUNCTION imagery.current_run(bytea,bytea,bytea,bytea,bytea,bytea,bytea) FROM PUBLIC;
-- Local RecordOnly databases have local scope/session authority but no hosted
-- account/campaign/O1 installation. Hosted startup must separately require it.
DO $$
BEGIN
    IF to_regclass('campaigns.campaign') IS NULL
        AND to_regclass('df_runtime.table_sessions') IS NULL THEN
        RETURN;
    END IF;
    IF to_regclass('campaigns.campaign') IS NULL
        OR to_regclass('df_runtime.table_sessions') IS NULL
        OR NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='df_runtime_definer')
        OR NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='df_imagery_admission') THEN
        RAISE EXCEPTION 'imagery hosted identity prerequisites are incomplete';
    END IF;
    GRANT USAGE ON SCHEMA imagery, campaigns TO df_runtime_definer;
    GRANT SELECT, UPDATE (revision) ON campaigns.campaign TO df_runtime_definer;
    ALTER FUNCTION imagery.current_run(bytea,bytea,bytea,bytea,bytea,bytea,bytea)
        OWNER TO df_runtime_definer;
    GRANT USAGE ON SCHEMA imagery TO df_imagery_admission;
    GRANT SELECT ON imagery.image_host_run_binding, imagery.image_host_seat_binding,
        imagery.image_local_guest TO df_imagery_admission;
    GRANT EXECUTE ON FUNCTION imagery.current_run(bytea,bytea,bytea,bytea,bytea,bytea,bytea)
        TO df_imagery_admission;
END $$;

CREATE TABLE IF NOT EXISTS imagery.image_host_seat_rotation (
    tenant_id bytea NOT NULL, campaign_id bytea NOT NULL, hosted_session_id bytea NOT NULL,
    account_id bytea NOT NULL, member_id bytea NOT NULL,
    old_generation bytea NOT NULL CHECK(octet_length(old_generation)=16),
    new_generation bytea NOT NULL CHECK(octet_length(new_generation)=16 AND new_generation<>old_generation),
    PRIMARY KEY(tenant_id,campaign_id,hosted_session_id,account_id,member_id,old_generation),
    FOREIGN KEY(tenant_id,campaign_id,hosted_session_id,account_id,member_id)
        REFERENCES imagery.image_host_seat_binding
);
REVOKE ALL ON imagery.image_host_seat_rotation FROM PUBLIC;
