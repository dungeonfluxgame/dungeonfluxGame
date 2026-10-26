-- Campaign-owned curated public projections, never game memory, transcripts or media URLs.
CREATE TABLE campaigns.share_preview (
    session_id bytea PRIMARY KEY CHECK (octet_length(session_id) = 16),
    campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16),
    creator_account_id bytea NOT NULL CHECK (octet_length(creator_account_id) = 16),
    revision text NOT NULL CHECK (length(revision) = 64),
    public_title text NOT NULL CHECK (char_length(public_title) BETWEEN 1 AND 120),
    highlights text[] NOT NULL CHECK (cardinality(highlights) <= 5),
    public_date text NOT NULL CHECK (length(public_date) = 10),
    duration_minutes bigint NOT NULL CHECK (duration_minutes >= 0),
    updated_at_ms bigint NOT NULL
);
CREATE TABLE campaigns.share_consent (
    session_id bytea NOT NULL REFERENCES campaigns.share_preview(session_id) ON DELETE CASCADE,
    account_id bytea NOT NULL CHECK (octet_length(account_id) = 16),
    public_name text NOT NULL CHECK (char_length(public_name) BETWEEN 1 AND 80),
    revision text NOT NULL CHECK (length(revision) = 64),
    consented_at_ms bigint,
    PRIMARY KEY(session_id, account_id)
);
CREATE TABLE campaigns.share_link (
    link_id bytea PRIMARY KEY CHECK (octet_length(link_id) = 16),
    token_digest bytea NOT NULL UNIQUE CHECK (octet_length(token_digest) = 32),
    session_id bytea NOT NULL REFERENCES campaigns.share_preview(session_id) ON DELETE CASCADE,
    creator_account_id bytea NOT NULL CHECK (octet_length(creator_account_id) = 16),
    revision text NOT NULL CHECK (length(revision) = 64),
    created_at_ms bigint NOT NULL,
    expires_at_ms bigint NOT NULL CHECK (expires_at_ms > created_at_ms AND expires_at_ms - created_at_ms IN (604800000,2592000000,7776000000)),
    revoked_at_ms bigint,
    allow_indexing boolean NOT NULL DEFAULT false,
    allow_embeds boolean NOT NULL DEFAULT false,
    views bigint NOT NULL DEFAULT 0 CHECK (views >= 0)
);
CREATE INDEX share_link_session ON campaigns.share_link(session_id,expires_at_ms);
CREATE INDEX share_link_expiry ON campaigns.share_link(expires_at_ms);
CREATE INDEX share_link_retirement ON campaigns.share_link(COALESCE(revoked_at_ms,expires_at_ms),link_id);
GRANT SELECT,INSERT,UPDATE,DELETE ON campaigns.share_preview,campaigns.share_consent,campaigns.share_link TO df_campaigns;
