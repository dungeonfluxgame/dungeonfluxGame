-- Campaigns, membership, invitations, room codes, transfers and the event feed
-- (planning/campaigns-and-table-research.md section 6), schema `campaigns` of the hosted
-- database. Every name is schema-qualified, so the runner's search_path does not matter.
--
-- There are no foreign keys to other owners' tables (accounts, tenants, sessions): a foreign key
-- takes FOR KEY SHARE on the referenced row, which would lock an accounts row after the campaign
-- lock and break the lock order of launch decision 29 (accounts and finance, O1 rows, then
-- campaigns.campaign, then campaign child rows). Ids are validated at creation through the ports.
-- Times are milliseconds since the Unix epoch; ids are 16-byte bytea. PostgreSQL 17 or later
-- (transaction_timeout) is required.

CREATE SCHEMA IF NOT EXISTS campaigns;

CREATE TABLE campaigns.campaign (
    campaign_id bytea PRIMARY KEY CHECK (octet_length(campaign_id) = 16),
    tenant_id bytea NOT NULL CHECK (octet_length(tenant_id) = 16),
    owner_account_id bytea NOT NULL CHECK (octet_length(owner_account_id) = 16),
    name text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
    adventure_kind text NOT NULL DEFAULT 'bundled',
    rules_edition text NOT NULL DEFAULT 'srd_5_2_1' CHECK (rules_edition IN ('srd_5_2_1')),
    content_rating text NOT NULL DEFAULT 'standard'
        CHECK (content_rating IN ('gentle', 'standard', 'mature')),
    seat_count integer NOT NULL CHECK (seat_count BETWEEN 1 AND 64),
    room_codes text NOT NULL DEFAULT 'with_approval' CHECK (room_codes IN ('off', 'with_approval')),
    default_co_host_delegations smallint NOT NULL DEFAULT 0
        CHECK (default_co_host_delegations BETWEEN 0 AND 7),
    funding_model text NOT NULL DEFAULT 'named_sponsor'
        CHECK (funding_model IN ('named_sponsor', 'shared_pot')),
    sponsor_account_id bytea CHECK (octet_length(sponsor_account_id) = 16),
    voice_default text NOT NULL DEFAULT 'on' CHECK (voice_default IN ('on', 'off')),
    picture_default text NOT NULL DEFAULT 'existing_art'
        CHECK (picture_default IN ('existing_art', 'automatic')),
    spend_preset text NOT NULL DEFAULT 'story'
        CHECK (spend_preset IN ('story', 'immersive', 'cinematic', 'custom')),
    custom_hourly_micro_dollars bigint CHECK (custom_hourly_micro_dollars >= 0),
    session_ceiling_micro_dollars bigint CHECK (session_ceiling_micro_dollars >= 0),
    display_summary_enabled boolean NOT NULL DEFAULT false,
    planned_session_at_ms bigint,
    status text NOT NULL CHECK (status IN ('draft', 'active', 'archived', 'deleting')),
    draft_step smallint CHECK (draft_step BETWEEN 1 AND 5),
    revision bigint NOT NULL DEFAULT 1 CHECK (revision >= 1),
    -- The gapless per-campaign event counter, bumped under this row's lock in the transaction of
    -- the change (5.1). A bigserial assigned at insert would let a lower number commit later.
    event_seq bigint NOT NULL DEFAULT 0 CHECK (event_seq >= 0),
    -- A random 128-bit epoch of the event feed, re-randomized by the restore step.
    feed_epoch bytea NOT NULL CHECK (octet_length(feed_epoch) = 16),
    -- The primary's clock at the last event; only a discovery hint for durable consumers.
    last_event_at_ms bigint NOT NULL DEFAULT 0,
    created_at_ms bigint NOT NULL,
    archived_at_ms bigint,
    scheduled_delete_at_ms bigint,
    draft_expires_at_ms bigint,
    CHECK ((status = 'draft') = (draft_step IS NOT NULL))
);
CREATE INDEX campaign_last_event ON campaigns.campaign (last_event_at_ms, campaign_id);
CREATE INDEX campaign_owner ON campaigns.campaign (owner_account_id, campaign_id);
CREATE INDEX campaign_scheduled_deletion ON campaigns.campaign (scheduled_delete_at_ms)
    WHERE scheduled_delete_at_ms IS NOT NULL;

CREATE TABLE campaigns.campaign_member (
    member_id bytea PRIMARY KEY CHECK (octet_length(member_id) = 16),
    campaign_id bytea NOT NULL REFERENCES campaigns.campaign (campaign_id),
    account_id bytea NOT NULL CHECK (octet_length(account_id) = 16),
    role text NOT NULL CHECK (role IN ('host', 'co_host', 'player', 'contributor')),
    holds_seat boolean NOT NULL,
    delegations smallint NOT NULL DEFAULT 0 CHECK (delegations BETWEEN 0 AND 7),
    status text NOT NULL CHECK (status IN ('active', 'left', 'removed')),
    may_rejoin boolean NOT NULL DEFAULT false,
    removed_by_role text CHECK (removed_by_role IN ('host', 'co_host')),
    last_invitation_id bytea CHECK (octet_length(last_invitation_id) = 16),
    joined_at_ms bigint NOT NULL,
    ended_at_ms bigint,
    revision bigint NOT NULL DEFAULT 1 CHECK (revision >= 1),
    -- One row per person per campaign for life: a returning person reactivates this row.
    UNIQUE (campaign_id, account_id),
    -- A contributor never holds a seat; an active player always does.
    CHECK (role <> 'contributor' OR NOT holds_seat),
    CHECK (status <> 'active' OR role <> 'player' OR holds_seat),
    CHECK (removed_by_role IS NULL OR status = 'removed')
);
-- One active host per campaign. The index is not deferrable, so a host swap demotes first.
CREATE UNIQUE INDEX campaign_member_one_active_host ON campaigns.campaign_member (campaign_id)
    WHERE role = 'host' AND status = 'active';
CREATE INDEX campaign_member_account ON campaigns.campaign_member (account_id, campaign_id)
    WHERE status = 'active';

CREATE TABLE campaigns.campaign_invitation (
    invitation_id bytea PRIMARY KEY CHECK (octet_length(invitation_id) = 16),
    campaign_id bytea NOT NULL REFERENCES campaigns.campaign (campaign_id),
    token_digest bytea NOT NULL UNIQUE CHECK (octet_length(token_digest) = 32),
    created_by_member bytea NOT NULL CHECK (octet_length(created_by_member) = 16),
    joins_as text NOT NULL CHECK (joins_as IN ('player', 'contributor')),
    created_at_ms bigint NOT NULL,
    expires_at_ms bigint NOT NULL,
    max_uses integer NOT NULL CHECK (max_uses >= 1),
    use_count integer NOT NULL DEFAULT 0 CHECK (use_count >= 0),
    revoked_at_ms bigint
);
CREATE INDEX campaign_invitation_creator ON campaigns.campaign_invitation (campaign_id, created_by_member);

CREATE TABLE campaigns.join_code (
    code_digest bytea PRIMARY KEY CHECK (octet_length(code_digest) = 32),
    campaign_id bytea NOT NULL REFERENCES campaigns.campaign (campaign_id),
    created_at_ms bigint NOT NULL,
    expires_at_ms bigint NOT NULL,
    revoked_at_ms bigint
);
-- One live code per campaign; rotating revokes the old one first.
CREATE UNIQUE INDEX join_code_one_live ON campaigns.join_code (campaign_id)
    WHERE revoked_at_ms IS NULL;

CREATE TABLE campaigns.join_request (
    request_id bytea PRIMARY KEY CHECK (octet_length(request_id) = 16),
    campaign_id bytea NOT NULL REFERENCES campaigns.campaign (campaign_id),
    account_id bytea NOT NULL CHECK (octet_length(account_id) = 16),
    state text NOT NULL CHECK (state IN ('pending', 'approved', 'denied', 'withdrawn')),
    approved_as text CHECK (approved_as IN ('player', 'contributor')),
    created_at_ms bigint NOT NULL,
    decided_by_member bytea CHECK (octet_length(decided_by_member) = 16),
    decided_at_ms bigint
);
CREATE UNIQUE INDEX join_request_one_pending ON campaigns.join_request (campaign_id, account_id)
    WHERE state = 'pending';

CREATE TABLE campaigns.transfer_proposal (
    proposal_id bytea PRIMARY KEY CHECK (octet_length(proposal_id) = 16),
    campaign_id bytea NOT NULL REFERENCES campaigns.campaign (campaign_id),
    kind text NOT NULL CHECK (kind IN ('host', 'ownership')),
    from_member bytea NOT NULL CHECK (octet_length(from_member) = 16),
    from_account bytea NOT NULL CHECK (octet_length(from_account) = 16),
    to_member bytea NOT NULL CHECK (octet_length(to_member) = 16),
    -- Copied at proposal time so acceptance can take the account lock before the campaign lock.
    to_account bytea NOT NULL CHECK (octet_length(to_account) = 16),
    old_host_becomes text CHECK (old_host_becomes IN ('player', 'co_host')),
    campaign_revision bigint NOT NULL,
    proposed_at_ms bigint NOT NULL,
    expires_at_ms bigint NOT NULL,
    state text NOT NULL CHECK (state IN ('proposed', 'accepted', 'cancelled', 'expired')),
    CHECK ((kind = 'host') = (old_host_becomes IS NOT NULL))
);
CREATE INDEX transfer_proposal_from ON campaigns.transfer_proposal (from_account, campaign_id)
    WHERE state = 'proposed';

-- The append-only event feed. The CHECK constraints tie `scope` to its columns and to the kind,
-- so a malformed row cannot be written (5.1).
CREATE TABLE campaigns.campaign_event (
    campaign_id bytea NOT NULL REFERENCES campaigns.campaign (campaign_id),
    event_seq bigint NOT NULL CHECK (event_seq >= 1),
    scope text NOT NULL CHECK (scope IN ('member', 'ticket', 'campaign')),
    feed_epoch bytea NOT NULL CHECK (octet_length(feed_epoch) = 16),
    member_id bytea CHECK (octet_length(member_id) = 16),
    member_revision bigint,
    revoking boolean NOT NULL,
    at_ms bigint NOT NULL,
    kind text NOT NULL CHECK (kind IN (
        'joined', 'rejoined', 'left', 'removed', 'rejoin_allowed', 'role_changed',
        'delegations_changed', 'seat_changed', 'invitations_revoked', 'host_transferred',
        'ownership_transferred', 'archived', 'unarchived', 'delete_requested',
        'delete_scheduled', 'delete_cancelled', 'display_ticket_revoked')),
    actor_member bytea CHECK (octet_length(actor_member) = 16),
    ticket_id bytea CHECK (octet_length(ticket_id) = 16),
    writer_fence bytea,
    PRIMARY KEY (campaign_id, event_seq),
    CHECK (
        (scope = 'member' AND member_id IS NOT NULL AND member_revision IS NOT NULL
            AND ticket_id IS NULL)
        OR (scope = 'ticket' AND ticket_id IS NOT NULL AND member_id IS NULL
            AND member_revision IS NULL)
        OR (scope = 'campaign' AND member_id IS NULL AND member_revision IS NULL
            AND ticket_id IS NULL)),
    CHECK ((kind IN ('joined', 'rejoined', 'left', 'removed', 'rejoin_allowed', 'role_changed',
            'delegations_changed', 'seat_changed', 'host_transferred', 'ownership_transferred'))
        = (scope = 'member')),
    CHECK ((kind = 'display_ticket_revoked') = (scope = 'ticket')),
    CHECK ((kind IN ('invitations_revoked', 'archived', 'unarchived', 'delete_requested',
            'delete_scheduled', 'delete_cancelled')) = (scope = 'campaign')),
    -- The revoking flag is fixed by the kinds that always or never revoke.
    CHECK (
        (kind IN ('joined', 'rejoined', 'rejoin_allowed', 'invitations_revoked', 'unarchived',
            'delete_cancelled') AND NOT revoking)
        OR (kind IN ('left', 'removed', 'archived', 'delete_requested', 'delete_scheduled',
            'display_ticket_revoked') AND revoking)
        OR kind IN ('role_changed', 'delegations_changed', 'seat_changed', 'host_transferred',
            'ownership_transferred'))
);

CREATE FUNCTION campaigns.refuse_change() RETURNS trigger
    LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'campaigns.% is append-only', TG_TABLE_NAME USING ERRCODE = 'integrity_constraint_violation';
END $$;

CREATE TRIGGER campaign_event_append_only BEFORE UPDATE OR DELETE ON campaigns.campaign_event
    FOR EACH ROW EXECUTE FUNCTION campaigns.refuse_change();

CREATE TABLE campaigns.campaign_operation (
    namespace text NOT NULL CHECK (namespace IN ('create_campaign', 'accept_invitation')),
    account_id bytea NOT NULL CHECK (octet_length(account_id) = 16),
    operation_id bytea NOT NULL CHECK (octet_length(operation_id) = 16),
    fingerprint bytea NOT NULL CHECK (octet_length(fingerprint) = 32),
    result_ref bytea NOT NULL,
    created_at_ms bigint NOT NULL,
    PRIMARY KEY (namespace, account_id, operation_id)
);

-- The commit-time bound on an event transaction (5.1, reliability bound). A role default of
-- `transaction_timeout` is only a default: a session can SET it to 0. The poll-overlap argument
-- needs the gap between the event stamp (taken inside the transaction) and the commit to stay
-- below the bound, so every transaction that bumps `event_seq` is also checked when it commits:
-- a deferred constraint trigger raises when the transaction is older than the bound. This holds
-- against a misconfigured or runaway session; it is not a defence against a session that
-- changes the trigger's timing with SET CONSTRAINTS. The bound lives in a one-row table the
-- service role cannot write, not in a session setting.
CREATE TABLE campaigns.feed_bound (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    max_transaction_ms integer NOT NULL CHECK (max_transaction_ms BETWEEN 100 AND 60000)
);
INSERT INTO campaigns.feed_bound VALUES (true, 10000);

-- Checked when the transaction commits: the transaction is younger than the bound, the
-- campaign's counter equals the highest event number written (no phantom number), and every
-- number this update moved the counter across has its event row (no gap below the head). The
-- trigger fires once per counter update, so a transaction that bumps twice and writes only the
-- second event is caught by the first update's range.
CREATE FUNCTION campaigns.check_event_head_at_commit() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = pg_catalog, campaigns, pg_temp AS $$
DECLARE
    v_bound integer;
    v_counter bigint;
    v_written bigint;
    v_between bigint;
BEGIN
    SELECT max_transaction_ms INTO v_bound FROM campaigns.feed_bound;
    IF clock_timestamp() - now() > make_interval(secs => v_bound / 1000.0) THEN
        RAISE EXCEPTION 'event transaction older than % ms', v_bound USING ERRCODE = 'DF003';
    END IF;
    SELECT event_seq INTO v_counter FROM campaigns.campaign WHERE campaign_id = NEW.campaign_id;
    SELECT COALESCE(max(event_seq), 0) INTO v_written
        FROM campaigns.campaign_event WHERE campaign_id = NEW.campaign_id;
    IF v_counter IS DISTINCT FROM v_written THEN
        RAISE EXCEPTION 'event_seq % has no matching event (highest written %)', v_counter, v_written
            USING ERRCODE = 'DF004';
    END IF;
    SELECT count(*) INTO v_between FROM campaigns.campaign_event
        WHERE campaign_id = NEW.campaign_id AND event_seq > OLD.event_seq
          AND event_seq <= NEW.event_seq;
    IF v_between <> NEW.event_seq - OLD.event_seq THEN
        RAISE EXCEPTION 'events % to % are not all written (found %)',
            OLD.event_seq + 1, NEW.event_seq, v_between USING ERRCODE = 'DF004';
    END IF;
    RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION campaigns.check_event_head_at_commit() FROM PUBLIC;

CREATE CONSTRAINT TRIGGER campaign_event_head AFTER UPDATE OF event_seq ON campaigns.campaign
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
    WHEN (OLD.event_seq IS DISTINCT FROM NEW.event_seq)
    EXECUTE FUNCTION campaigns.check_event_head_at_commit();

-- An event row carries the campaign's current counter and epoch: the counter was bumped in this
-- transaction under the row lock, so a row with any other number is a gap or a phantom.
CREATE FUNCTION campaigns.check_event_matches_campaign() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = pg_catalog, campaigns, pg_temp AS $$
DECLARE
    v_counter bigint;
    v_epoch bytea;
BEGIN
    SELECT event_seq, feed_epoch INTO v_counter, v_epoch
        FROM campaigns.campaign WHERE campaign_id = NEW.campaign_id;
    IF NOT FOUND OR NEW.event_seq <> v_counter OR NEW.feed_epoch <> v_epoch THEN
        RAISE EXCEPTION 'event does not match the campaign head' USING ERRCODE = 'DF004';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION campaigns.check_event_matches_campaign() FROM PUBLIC;

CREATE TRIGGER campaign_event_matches BEFORE INSERT ON campaigns.campaign_event
    FOR EACH ROW EXECUTE FUNCTION campaigns.check_event_matches_campaign();

-- The counter moves only by one; the epoch, the tenant and the identity never change through a
-- service role. The restore step (a role outside the two service roles) is the only writer of the
-- epoch. Not a security definer: it looks at the role that runs the statement.
CREATE FUNCTION campaigns.guard_campaign_update() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = pg_catalog, campaigns, pg_temp AS $$
BEGIN
    IF NEW.event_seq <> OLD.event_seq AND NEW.event_seq <> OLD.event_seq + 1 THEN
        RAISE EXCEPTION 'event_seq moves only by one' USING ERRCODE = 'DF004';
    END IF;
    IF NEW.campaign_id <> OLD.campaign_id THEN
        RAISE EXCEPTION 'campaign_id is immutable' USING ERRCODE = 'DF004';
    END IF;
    IF (NEW.feed_epoch <> OLD.feed_epoch OR NEW.tenant_id <> OLD.tenant_id
        OR NEW.created_at_ms <> OLD.created_at_ms)
       AND EXISTS (
           SELECT 1 FROM pg_auth_members m
             JOIN pg_roles member_role ON member_role.oid = m.member
             JOIN pg_roles granted ON granted.oid = m.roleid
            WHERE member_role.rolname = current_user
              AND granted.rolname IN ('df_campaigns', 'df_accounts')) THEN
        RAISE EXCEPTION 'feed_epoch, tenant_id and created_at_ms are immutable for service roles'
            USING ERRCODE = 'DF004';
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER campaign_guard BEFORE UPDATE ON campaigns.campaign
    FOR EACH ROW EXECUTE FUNCTION campaigns.guard_campaign_update();

-- A fresh random 128-bit feed epoch from two UUIDs, skipping the version and variant bits
-- (bytes 6, 7 and 8 of the first; the second contributes three free bytes).
CREATE FUNCTION campaigns.fresh_feed_epoch() RETURNS bytea
    LANGUAGE sql VOLATILE AS $$
    SELECT substring(a FROM 1 FOR 6) || substring(a FROM 10 FOR 7) || substring(b FROM 1 FOR 3)
    FROM (SELECT uuid_send(gen_random_uuid()) AS a, uuid_send(gen_random_uuid()) AS b) AS source
$$;

-- The restore step of the runbook (S2) calls this once, in the same step that raises O1's
-- recovery_epoch, and after a failover that lost commits before service resumes: every campaign
-- gets a fresh random epoch so a consumer never compares sequence numbers across epochs.
CREATE FUNCTION campaigns.restore_feed_epochs() RETURNS bigint
    LANGUAGE plpgsql VOLATILE AS $$
DECLARE
    v_count bigint;
BEGIN
    UPDATE campaigns.campaign SET feed_epoch = campaigns.fresh_feed_epoch();
    GET DIAGNOSTICS v_count = ROW_COUNT;
    RETURN v_count;
END $$;
REVOKE ALL ON FUNCTION campaigns.restore_feed_epochs() FROM PUBLIC;
REVOKE ALL ON FUNCTION campaigns.fresh_feed_epoch() FROM PUBLIC;

-- The campaign service's runtime role. `df_campaigns` is also created by the accounts migration;
-- both create it NOLOGIN when absent (the deployment adds LOGIN and a password). The bound on a
-- transaction is the role's default: a per-transaction SET LOCAL does not shorten a timer that is
-- already running, and the service refuses to start unless the default holds (5.1).
-- Roles are cluster-wide, so concurrent migrations (one per database) can collide on them: every
-- role statement runs only when it would change something, and is retried a few times.
DO $$
DECLARE
    v_attempt integer;
BEGIN
    FOR v_attempt IN 1..5 LOOP
        BEGIN
            IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'df_campaigns') THEN
                CREATE ROLE df_campaigns NOLOGIN;
            END IF;
            IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'df_campaigns' AND (rolsuper
                    OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls)) THEN
                ALTER ROLE df_campaigns NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION
                    NOBYPASSRLS;
            END IF;
            IF NOT EXISTS (
                SELECT 1 FROM pg_db_role_setting s JOIN pg_roles r ON r.oid = s.setrole
                WHERE r.rolname = 'df_campaigns' AND s.setdatabase = 0
                  AND 'transaction_timeout=10s' = ANY (s.setconfig)
            ) THEN
                ALTER ROLE df_campaigns SET transaction_timeout = '10s';
            END IF;
            EXIT;
        EXCEPTION WHEN OTHERS THEN
            IF v_attempt = 5 THEN
                RAISE;
            END IF;
            PERFORM pg_sleep(0.1 * v_attempt);
        END;
    END LOOP;
END $$;

-- A campaign row can be deleted only while it is a draft (an unfinished wizard): the trigger
-- refuses every other status for every role, and the foreign keys refuse a row with members.
-- Drafts are deleted by the retention pass when they expire and by account closure.
CREATE FUNCTION campaigns.refuse_delete_unless_draft() RETURNS trigger
    LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.status <> 'draft' THEN
        RAISE EXCEPTION 'only a draft campaign can be deleted' USING ERRCODE = '42501';
    END IF;
    RETURN OLD;
END $$;

CREATE TRIGGER campaign_delete_only_drafts BEFORE DELETE ON campaigns.campaign
    FOR EACH ROW EXECUTE FUNCTION campaigns.refuse_delete_unless_draft();

-- Grants are column-level for UPDATE: no service role can rewrite an identity, the tenant, the
-- epoch or a creation time. B's close and completion transactions run the closure methods on
-- B's own role (`df_accounts`), so that role gets exactly the columns those methods write; the
-- migration fails if the role is missing instead of silently granting nothing.
GRANT USAGE ON SCHEMA campaigns TO df_campaigns;
GRANT SELECT, INSERT ON campaigns.campaign, campaigns.campaign_member,
    campaigns.campaign_invitation, campaigns.join_code, campaigns.join_request,
    campaigns.transfer_proposal TO df_campaigns;
GRANT SELECT, INSERT ON campaigns.campaign_event, campaigns.campaign_operation TO df_campaigns;
GRANT UPDATE (owner_account_id, name, seat_count, content_rating, room_codes,
    default_co_host_delegations, display_summary_enabled, planned_session_at_ms, status,
    revision, event_seq, last_event_at_ms, archived_at_ms, scheduled_delete_at_ms,
    adventure_kind, voice_default, picture_default, spend_preset, custom_hourly_micro_dollars,
    session_ceiling_micro_dollars, funding_model, sponsor_account_id, draft_step,
    draft_expires_at_ms)
    ON campaigns.campaign TO df_campaigns;
GRANT UPDATE (role, holds_seat, delegations, status, may_rejoin, removed_by_role,
    last_invitation_id, joined_at_ms, ended_at_ms, revision)
    ON campaigns.campaign_member TO df_campaigns;
GRANT UPDATE (use_count, revoked_at_ms) ON campaigns.campaign_invitation TO df_campaigns;
GRANT UPDATE (revoked_at_ms) ON campaigns.join_code TO df_campaigns;
GRANT UPDATE (state, approved_as, decided_by_member, decided_at_ms)
    ON campaigns.join_request TO df_campaigns;
GRANT UPDATE (state) ON campaigns.transfer_proposal TO df_campaigns;
GRANT DELETE ON campaigns.campaign TO df_campaigns;
GRANT SELECT ON campaigns.feed_bound TO df_campaigns;

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'df_accounts') THEN
        RAISE EXCEPTION 'role df_accounts is missing: apply the accounts migrations first';
    END IF;
END $$;
GRANT USAGE ON SCHEMA campaigns TO df_accounts;
GRANT SELECT ON campaigns.campaign, campaigns.campaign_member, campaigns.campaign_invitation,
    campaigns.join_code, campaigns.transfer_proposal TO df_accounts;
GRANT SELECT, INSERT ON campaigns.campaign_event TO df_accounts;
GRANT UPDATE (revision, event_seq, last_event_at_ms, scheduled_delete_at_ms)
    ON campaigns.campaign TO df_accounts;
GRANT DELETE ON campaigns.campaign TO df_accounts;
GRANT UPDATE (role, holds_seat, delegations, status, may_rejoin, removed_by_role,
    last_invitation_id, joined_at_ms, ended_at_ms, revision)
    ON campaigns.campaign_member TO df_accounts;
GRANT UPDATE (revoked_at_ms) ON campaigns.campaign_invitation TO df_accounts;
GRANT UPDATE (revoked_at_ms) ON campaigns.join_code TO df_accounts;
GRANT UPDATE (state) ON campaigns.transfer_proposal TO df_accounts;
