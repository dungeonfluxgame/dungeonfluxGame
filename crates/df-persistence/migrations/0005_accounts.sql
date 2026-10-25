-- Customer accounts (planning/account-service-research.md section 5.1), in schema `accounts`
-- of the hosted database; the store sets search_path to it before applying this file.
-- Times are milliseconds since the Unix epoch from the service clock. Ids are 16 bytes.
-- Tokens are stored only as SHA-256 digests.

CREATE TABLE account (
    account_id bytea PRIMARY KEY CHECK (octet_length(account_id) = 16),
    email_normalized text UNIQUE CHECK (octet_length(email_normalized) <= 254),
    email_display text CHECK (octet_length(email_display) <= 254),
    email_verified_at_ms bigint,
    adult_attested_at bigint,
    terms_version text,
    terms_accepted_at bigint,
    display_name text CHECK (char_length(display_name) <= 60),
    status text NOT NULL CHECK (status IN ('active', 'closing', 'closed')),
    credential_generation bigint NOT NULL CHECK (credential_generation >= 1),
    created_at_ms bigint NOT NULL,
    delete_after_ms bigint,
    CHECK ((status = 'closed') = (email_normalized IS NULL)),
    CHECK ((email_normalized IS NULL) = (email_display IS NULL)),
    CHECK ((adult_attested_at IS NULL) = (terms_version IS NULL)),
    CHECK ((adult_attested_at IS NULL) = (terms_accepted_at IS NULL))
);

CREATE TABLE password_credential (
    account_id bytea PRIMARY KEY REFERENCES account (account_id),
    phc_hash text NOT NULL,
    updated_at_ms bigint NOT NULL
);

-- A hosted identity provider's subject linked to an account (master launch plan 15.1
-- `IdentityLink`): unique provider plus subject.
CREATE TABLE identity_link (
    provider text NOT NULL CHECK (provider IN ('workos', 'fake')),
    subject text NOT NULL CHECK (octet_length(subject) BETWEEN 1 AND 256),
    account_id bytea NOT NULL REFERENCES account (account_id),
    linked_at_ms bigint NOT NULL,
    PRIMARY KEY (provider, subject)
);
CREATE INDEX identity_link_account ON identity_link (account_id);

CREATE TABLE web_session (
    session_id bytea PRIMARY KEY CHECK (octet_length(session_id) = 16),
    token_digest bytea NOT NULL UNIQUE CHECK (octet_length(token_digest) = 32),
    account_id bytea NOT NULL REFERENCES account (account_id),
    credential_generation bigint NOT NULL,
    created_at_ms bigint NOT NULL,
    last_seen_at_ms bigint NOT NULL,
    absolute_expires_at_ms bigint NOT NULL,
    revoked_at_ms bigint,
    revoke_reason text CHECK (revoke_reason IN ('logout', 'revoked_by_user',
        'sign_out_everywhere', 'password_changed', 'password_reset', 'email_changed', 'rotated',
        'account_closed')),
    csrf_secret bytea NOT NULL CHECK (octet_length(csrf_secret) = 32),
    device_label text NOT NULL CHECK (device_label IN ('phone', 'tablet', 'desktop', 'unknown')),
    CHECK ((revoked_at_ms IS NULL) = (revoke_reason IS NULL))
);
CREATE INDEX web_session_account ON web_session (account_id);

CREATE TABLE email_change (
    email_change_id bytea PRIMARY KEY CHECK (octet_length(email_change_id) = 16),
    account_id bytea NOT NULL REFERENCES account (account_id),
    new_email_normalized text NOT NULL,
    new_email_display text NOT NULL,
    old_confirmed boolean NOT NULL DEFAULT false,
    new_confirmed boolean NOT NULL DEFAULT false,
    is_open boolean NOT NULL DEFAULT true,
    expires_at_ms bigint NOT NULL
);
CREATE INDEX email_change_account ON email_change (account_id);

CREATE TABLE account_token (
    token_id bytea PRIMARY KEY CHECK (octet_length(token_id) = 16),
    token_digest bytea NOT NULL UNIQUE CHECK (octet_length(token_digest) = 32),
    account_id bytea NOT NULL REFERENCES account (account_id),
    purpose text NOT NULL CHECK (purpose IN ('verify_email', 'password_reset',
        'email_change_old', 'email_change_new')),
    email_change_id bytea REFERENCES email_change (email_change_id),
    credential_generation bigint NOT NULL,
    created_at_ms bigint NOT NULL,
    expires_at_ms bigint NOT NULL,
    consumed_at_ms bigint,
    attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    attempt_limit integer NOT NULL CHECK (attempt_limit >= 1)
);
CREATE INDEX account_token_account ON account_token (account_id, purpose);

CREATE TABLE operation_receipt (
    namespace text NOT NULL CHECK (namespace IN ('signup', 'email_change')),
    principal bytea NOT NULL CHECK (octet_length(principal) = 32),
    operation_id bytea NOT NULL CHECK (octet_length(operation_id) = 16),
    fingerprint bytea NOT NULL CHECK (octet_length(fingerprint) = 32),
    outcome text CHECK (outcome IN ('done')),
    created_at_ms bigint NOT NULL,
    PRIMARY KEY (namespace, principal, operation_id)
);

CREATE TABLE account_event (
    event_id bigserial PRIMARY KEY,
    account_id bytea NOT NULL REFERENCES account (account_id),
    kind text NOT NULL CHECK (kind IN ('signed_up', 'verified', 'logged_in', 'login_refused',
        'logged_out', 'password_changed', 'password_reset', 'email_change_requested',
        'email_changed', 'sessions_revoked', 'data_exported', 'closed', 'reopened')),
    at_ms bigint NOT NULL,
    device_label text NOT NULL CHECK (device_label IN ('phone', 'tablet', 'desktop', 'unknown'))
);
CREATE INDEX account_event_account ON account_event (account_id, event_id DESC);

-- Security history is append-only: rows are never edited; only the retention pass deletes the
-- rows of an account whose personal data is being deleted.
CREATE FUNCTION account_event_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'account_event is append-only';
END;
$$;
CREATE TRIGGER account_event_no_update BEFORE UPDATE ON account_event
    FOR EACH ROW EXECUTE FUNCTION account_event_append_only();
