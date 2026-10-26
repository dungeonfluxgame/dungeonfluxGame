-- Customer PATs; integration-api-and-embeds-research 2.2. Account-owned secrets are
-- HMAC digests only. The pepper key is held exclusively in server configuration.
CREATE TABLE personal_token (
    token_id bytea PRIMARY KEY CHECK (octet_length(token_id) = 16),
    account_id bytea NOT NULL REFERENCES account(account_id),
    label text NOT NULL CHECK (char_length(label) BETWEEN 1 AND 60),
    scopes text[] NOT NULL CHECK (cardinality(scopes) BETWEEN 1 AND 8 AND scopes <@ ARRAY[
        'account:read','account:email:read','auth_sessions:read','play_sessions:read',
        'assets:read','funding:read','statements:read','stats:read']::text[]),
    pepper_id bigint NOT NULL CHECK (pepper_id BETWEEN 0 AND 4294967295),
    secret_digest bytea NOT NULL CHECK (octet_length(secret_digest) = 32),
    created_at_ms bigint NOT NULL CHECK (created_at_ms >= 0),
    expires_at_ms bigint NOT NULL CHECK (expires_at_ms > created_at_ms AND
        expires_at_ms - created_at_ms IN (604800000,2592000000,7776000000,15552000000,31536000000)),
    revoked_at_ms bigint,
    last_used_at_ms bigint,
    CHECK (NOT ('account:email:read' = ANY(scopes)) OR 'account:read' = ANY(scopes))
);
CREATE INDEX personal_token_account ON personal_token(account_id, created_at_ms DESC, token_id);
CREATE INDEX personal_token_expiry ON personal_token(expires_at_ms);
GRANT SELECT, INSERT, UPDATE, DELETE ON personal_token TO df_accounts;

-- Closure and all other transitions out of active standing revoke irreversibly,
-- inside the account transaction, including when a closing account later reopens.
CREATE FUNCTION revoke_personal_tokens_on_closure() RETURNS trigger LANGUAGE plpgsql
SET search_path = pg_catalog, accounts, pg_temp AS $$
BEGIN
    IF NEW.status <> 'active' AND OLD.status = 'active' THEN
        UPDATE accounts.personal_token SET revoked_at_ms = (extract(epoch FROM statement_timestamp()) * 1000)::bigint
        WHERE account_id = NEW.account_id AND revoked_at_ms IS NULL;
    END IF;
    IF NEW.status = 'closed' THEN
        DELETE FROM accounts.personal_token WHERE account_id = NEW.account_id;
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER account_revoke_personal_tokens AFTER UPDATE OF status ON account
FOR EACH ROW EXECUTE FUNCTION revoke_personal_tokens_on_closure();
