-- The lobby and display pairing (planning/campaigns-and-table-research.md section 6), schema
-- `campaigns`. Applied after 0008. `session_id` columns carry no foreign key: O1's session tables
-- come later (0015-0019) and the session directory port validates them.
--
-- Row-level security on `display_ticket` with O1's `df_runtime.leased_table_sessions()` is a
-- follow-up migration that must run after O1's 0015: it cannot be created before that function
-- exists. Until then the scope of the runtime's revocation writes is enforced in the service
-- (`RevocationCaller`); the row security is the second layer (TODO(S1-05)).

CREATE TABLE campaigns.lobby_member (
    session_id bytea NOT NULL CHECK (octet_length(session_id) = 16),
    member_id bytea NOT NULL CHECK (octet_length(member_id) = 16),
    campaign_id bytea NOT NULL REFERENCES campaigns.campaign (campaign_id),
    character_choice text NOT NULL DEFAULT 'undecided'
        CHECK (character_choice IN ('undecided', 'template', 'existing')),
    character_ref bytea CHECK (octet_length(character_ref) = 16),
    ready boolean NOT NULL DEFAULT false,
    microphone_preference text NOT NULL DEFAULT 'ask'
        CHECK (microphone_preference IN ('ask', 'typed_only')),
    microphone_report text NOT NULL DEFAULT 'unknown'
        CHECK (microphone_report IN ('unknown', 'granted', 'declined', 'unsupported')),
    updated_at_ms bigint NOT NULL,
    revision bigint NOT NULL DEFAULT 1 CHECK (revision >= 1),
    -- Set for every seated member's row when the session owner reports the session started
    -- (`session_started`); a frozen row, and any new row of a session that has frozen rows, is
    -- refused every change.
    frozen_at_ms bigint,
    PRIMARY KEY (session_id, member_id)
);
CREATE INDEX lobby_member_campaign ON campaigns.lobby_member (campaign_id, member_id);

CREATE TABLE campaigns.display_pairing (
    pairing_id bytea PRIMARY KEY CHECK (octet_length(pairing_id) = 16),
    -- Set together with `session_id` at approval.
    campaign_id bytea REFERENCES campaigns.campaign (campaign_id),
    -- Readable while pending so the status response can show it again; not a secret by itself
    -- (approval needs an authenticated delegated member, redemption needs the device secret).
    user_code text CHECK (char_length(user_code) = 8),
    device_secret_digest bytea NOT NULL CHECK (octet_length(device_secret_digest) = 32),
    device_label text NOT NULL CHECK (device_label IN ('phone', 'tablet', 'desktop', 'unknown')),
    ip_key bytea NOT NULL CHECK (octet_length(ip_key) = 32),
    -- The wider network prefix (IPv6 /48, IPv4 /24): the loose pending cap counts by it.
    ip_key_loose bytea NOT NULL CHECK (octet_length(ip_key_loose) = 32),
    created_at_ms bigint NOT NULL,
    expires_at_ms bigint NOT NULL,
    state text NOT NULL CHECK (state IN ('pending', 'approved', 'redeemed', 'cancelled', 'expired')),
    session_id bytea CHECK (octet_length(session_id) = 16),
    approved_by_member bytea CHECK (octet_length(approved_by_member) = 16),
    approved_at_ms bigint,
    redeemed_at_ms bigint,
    reissue_count smallint NOT NULL DEFAULT 0 CHECK (reissue_count IN (0, 1)),
    renewable_until_ms bigint,
    CHECK ((campaign_id IS NULL) = (session_id IS NULL)),
    CHECK (state IN ('pending', 'cancelled', 'expired') OR campaign_id IS NOT NULL)
);
CREATE UNIQUE INDEX display_pairing_pending_code ON campaigns.display_pairing (user_code)
    WHERE state = 'pending';
CREATE UNIQUE INDEX display_pairing_device ON campaigns.display_pairing (device_secret_digest);
CREATE INDEX display_pairing_pending_ip ON campaigns.display_pairing (ip_key)
    WHERE state = 'pending';
CREATE INDEX display_pairing_pending_ip_loose ON campaigns.display_pairing (ip_key_loose)
    WHERE state = 'pending';

CREATE TABLE campaigns.display_ticket (
    ticket_id bytea PRIMARY KEY CHECK (octet_length(ticket_id) = 16),
    ticket_digest bytea NOT NULL UNIQUE CHECK (octet_length(ticket_digest) = 32),
    pairing_id bytea NOT NULL REFERENCES campaigns.display_pairing (pairing_id),
    campaign_id bytea NOT NULL REFERENCES campaigns.campaign (campaign_id),
    session_id bytea NOT NULL CHECK (octet_length(session_id) = 16),
    device_secret_digest bytea NOT NULL CHECK (octet_length(device_secret_digest) = 32),
    device_label text NOT NULL CHECK (device_label IN ('phone', 'tablet', 'desktop', 'unknown')),
    approved_by_member bytea NOT NULL CHECK (octet_length(approved_by_member) = 16),
    issued_at_ms bigint NOT NULL,
    expires_at_ms bigint NOT NULL,
    revoked_at_ms bigint,
    revoke_reason text CHECK (revoke_reason IN ('unpaired', 'host_transferred', 'archived',
        'session_ended', 'approver_authority_lost', 'reissued', 'expired_cleanup')),
    CHECK ((revoked_at_ms IS NULL) = (revoke_reason IS NULL))
    -- No capability column exists: a display ticket can only view the public projection.
);
CREATE INDEX display_ticket_session ON campaigns.display_ticket (session_id)
    WHERE revoked_at_ms IS NULL;
CREATE INDEX display_ticket_campaign ON campaigns.display_ticket (campaign_id)
    WHERE revoked_at_ms IS NULL;
CREATE INDEX display_ticket_approver ON campaigns.display_ticket (campaign_id, approved_by_member)
    WHERE revoked_at_ms IS NULL;

-- Column-level UPDATE, as in 0008. The closure methods that run in B's transaction also touch
-- these; 0008 already failed the migration if `df_accounts` is missing.
GRANT SELECT, INSERT, DELETE ON campaigns.lobby_member TO df_campaigns;
GRANT UPDATE (character_choice, character_ref, ready, microphone_preference,
    microphone_report, updated_at_ms, revision, frozen_at_ms)
    ON campaigns.lobby_member TO df_campaigns;
GRANT SELECT, INSERT ON campaigns.display_pairing, campaigns.display_ticket TO df_campaigns;
GRANT UPDATE (campaign_id, session_id, state, approved_by_member, approved_at_ms,
    redeemed_at_ms, reissue_count, renewable_until_ms, user_code)
    ON campaigns.display_pairing TO df_campaigns;
GRANT UPDATE (expires_at_ms, revoked_at_ms, revoke_reason)
    ON campaigns.display_ticket TO df_campaigns;

GRANT SELECT ON campaigns.display_pairing, campaigns.display_ticket TO df_accounts;
GRANT SELECT, DELETE ON campaigns.lobby_member TO df_accounts;
GRANT UPDATE (state, user_code) ON campaigns.display_pairing TO df_accounts;
GRANT UPDATE (revoked_at_ms, revoke_reason) ON campaigns.display_ticket TO df_accounts;
