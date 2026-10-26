-- Owner decision 2026-10-26: campaign defaults remain editable after Play now.
DO $$
DECLARE constraint_row record;
BEGIN
    FOR constraint_row IN SELECT conname FROM pg_constraint
        WHERE conrelid = 'campaigns.campaign_event'::regclass AND contype = 'c'
        AND pg_get_constraintdef(oid) ~ '\mkind\M'
    LOOP
        EXECUTE format('ALTER TABLE campaigns.campaign_event DROP CONSTRAINT %I', constraint_row.conname);
    END LOOP;
END $$;
ALTER TABLE campaigns.campaign_event ADD CONSTRAINT campaign_event_kind_check CHECK (kind IN (
    'joined', 'rejoined', 'left', 'removed', 'rejoin_allowed', 'role_changed',
    'delegations_changed', 'seat_changed', 'invitations_revoked', 'host_transferred',
    'ownership_transferred', 'archived', 'unarchived', 'delete_requested',
    'delete_scheduled', 'delete_cancelled', 'display_ticket_revoked', 'settings_changed'));
ALTER TABLE campaigns.campaign_event ADD CONSTRAINT campaign_event_member_scope_check CHECK (
    (kind IN ('joined', 'rejoined', 'left', 'removed', 'rejoin_allowed', 'role_changed',
    'delegations_changed', 'seat_changed', 'host_transferred', 'ownership_transferred')) = (scope = 'member'));
ALTER TABLE campaigns.campaign_event ADD CONSTRAINT campaign_event_ticket_scope_check CHECK (
    (kind = 'display_ticket_revoked') = (scope = 'ticket'));
ALTER TABLE campaigns.campaign_event ADD CONSTRAINT campaign_event_campaign_scope_check CHECK (
    (kind IN ('invitations_revoked', 'archived', 'unarchived', 'delete_requested',
    'delete_scheduled', 'delete_cancelled', 'settings_changed')) = (scope = 'campaign'));
ALTER TABLE campaigns.campaign_event ADD CONSTRAINT campaign_event_revoking_check CHECK (
    (kind IN ('joined', 'rejoined', 'rejoin_allowed', 'invitations_revoked', 'unarchived',
    'delete_cancelled', 'settings_changed') AND NOT revoking)
    OR (kind IN ('left', 'removed', 'archived', 'delete_requested', 'delete_scheduled',
    'display_ticket_revoked') AND revoking)
    OR kind IN ('role_changed', 'delegations_changed', 'seat_changed', 'host_transferred', 'ownership_transferred'));

CREATE TABLE campaigns.account_preferences (
    account_id bytea PRIMARY KEY REFERENCES accounts.account (account_id) ON DELETE CASCADE
        CHECK (octet_length(account_id) = 16),
    content_rating text NOT NULL CHECK (content_rating IN ('gentle', 'standard', 'mature')),
    chosen_at_ms bigint NOT NULL
);
GRANT SELECT, INSERT, UPDATE ON campaigns.account_preferences TO df_campaigns;
GRANT SELECT, DELETE ON campaigns.account_preferences TO df_accounts;
