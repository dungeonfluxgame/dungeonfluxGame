SET search_path TO accounts;

ALTER TABLE account ADD COLUMN IF NOT EXISTS adult_attested_at bigint;
ALTER TABLE account ADD COLUMN IF NOT EXISTS terms_version text;
ALTER TABLE account ADD COLUMN IF NOT EXISTS terms_accepted_at bigint;

CREATE TABLE google_identity (
    subject text PRIMARY KEY CHECK (length(subject) BETWEEN 1 AND 256),
    account_id bytea NOT NULL UNIQUE REFERENCES account(account_id),
    linked_at_ms bigint NOT NULL
);

CREATE TABLE oauth_attempt (
    redirect_uri text NOT NULL CHECK (length(redirect_uri) BETWEEN 1 AND 2048),
    state_digest bytea PRIMARY KEY CHECK (octet_length(state_digest) = 32),
    cookie_digest bytea NOT NULL UNIQUE CHECK (octet_length(cookie_digest) = 32),
    client_digest bytea NOT NULL CHECK (octet_length(client_digest) = 32),
    nonce text NOT NULL CHECK (length(nonce) = 64),
    verifier text NOT NULL CHECK (length(verifier) = 64),
    intent text NOT NULL CHECK (intent IN ('sign_up', 'sign_in', 'link', 'reauthenticate')),
    account_id bytea REFERENCES account(account_id),
    session_id bytea REFERENCES web_session(session_id) ON DELETE CASCADE,
    credential_generation bigint,
    terms_version text,
    created_at_ms bigint NOT NULL,
    expires_at_ms bigint NOT NULL,
    CHECK (expires_at_ms = created_at_ms + 600000),
    CHECK ((intent IN ('link', 'reauthenticate')) = (account_id IS NOT NULL)),
    CHECK ((account_id IS NOT NULL) = (session_id IS NOT NULL)),
    CHECK ((account_id IS NOT NULL) = (credential_generation IS NOT NULL)),
    CHECK ((intent = 'sign_up') = (terms_version IS NOT NULL))
);
CREATE INDEX oauth_attempt_client ON oauth_attempt(client_digest);
CREATE INDEX oauth_attempt_expiry ON oauth_attempt(expires_at_ms);

CREATE TABLE google_session_proof (
    session_id bytea PRIMARY KEY REFERENCES web_session(session_id) ON DELETE CASCADE,
    reauthentication_expires_at_ms bigint
);

-- Google owns its account. Completed local closure only removes our subject link.
CREATE FUNCTION purge_google_identity() RETURNS trigger LANGUAGE plpgsql
    SET search_path = pg_catalog, accounts, pg_temp AS $$
BEGIN
    IF NEW.status = 'closed' AND OLD.status <> 'closed' THEN
        DELETE FROM google_identity WHERE account_id = NEW.account_id;
        DELETE FROM oauth_attempt WHERE account_id = NEW.account_id;
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER account_purge_google AFTER UPDATE OF status ON account
    FOR EACH ROW EXECUTE FUNCTION purge_google_identity();

GRANT SELECT, INSERT, DELETE ON google_identity, oauth_attempt, google_session_proof TO df_accounts;
