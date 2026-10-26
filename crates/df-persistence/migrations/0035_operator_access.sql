-- Account authority is rechecked under its existing row lock; no cross-owner foreign keys.
CREATE TABLE accounts.operator_mfa (
    account_id bytea PRIMARY KEY CHECK (octet_length(account_id) = 16),
    generation bigint NOT NULL CHECK (generation > 0),
    credential_generation bigint NOT NULL CHECK (credential_generation > 0),
    state text NOT NULL CHECK (state IN ('pending', 'active', 'revoked')),
    seal_key_id text NOT NULL CHECK (octet_length(seal_key_id) BETWEEN 1 AND 128),
    sealed_secret bytea NOT NULL CHECK (octet_length(sealed_secret) BETWEEN 1 AND 4096),
    created_at_ms bigint NOT NULL CHECK (created_at_ms >= 0),
    confirmed_at_ms bigint,
    last_used_step bigint CHECK (last_used_step >= 0),
    CHECK (state <> 'active' OR (confirmed_at_ms IS NOT NULL
        AND confirmed_at_ms >= created_at_ms AND last_used_step IS NOT NULL)),
    CHECK (state <> 'pending' OR confirmed_at_ms IS NULL)
);

CREATE TABLE accounts.operator_enrolment_ticket (
    digest bytea PRIMARY KEY CHECK (octet_length(digest) = 32),
    account_id bytea NOT NULL CHECK (octet_length(account_id) = 16),
    enrolment_generation bigint NOT NULL CHECK (enrolment_generation > 0),
    credential_generation bigint NOT NULL CHECK (credential_generation > 0),
    issued_at_ms bigint NOT NULL CHECK (issued_at_ms >= 0),
    expires_at_ms bigint NOT NULL,
    consumed_at_ms bigint,
    CHECK (expires_at_ms > issued_at_ms AND expires_at_ms - issued_at_ms <= 900000),
    CHECK (consumed_at_ms IS NULL OR consumed_at_ms BETWEEN issued_at_ms AND expires_at_ms - 1)
);
CREATE INDEX operator_ticket_account ON accounts.operator_enrolment_ticket(account_id);

CREATE TABLE accounts.operator_grant (
    grant_id bytea PRIMARY KEY CHECK (octet_length(grant_id) = 16),
    account_id bytea NOT NULL CHECK (octet_length(account_id) = 16),
    capabilities smallint[] NOT NULL CHECK (cardinality(capabilities) BETWEEN 1 AND 11
        AND capabilities <@ ARRAY[1,2,3,4,5,6,7,8,9,10,11]::smallint[]
        AND array_position(capabilities, NULL) IS NULL),
    scope_kind smallint NOT NULL CHECK (scope_kind BETWEEN 0 AND 2),
    scope_id bytea,
    version bigint NOT NULL CHECK (version > 0),
    issued_at_ms bigint NOT NULL CHECK (issued_at_ms >= 0),
    expires_at_ms bigint NOT NULL,
    revoked_at_ms bigint,
    enrolment_generation bigint NOT NULL CHECK (enrolment_generation > 0),
    reason text NOT NULL CHECK (char_length(btrim(reason)) BETWEEN 1 AND 500),
    CHECK ((scope_kind = 0 AND scope_id IS NULL)
        OR (scope_kind IN (1,2) AND scope_id IS NOT NULL AND octet_length(scope_id) = 16)),
    CHECK (expires_at_ms > issued_at_ms AND expires_at_ms - issued_at_ms <= 2592000000),
    CHECK (revoked_at_ms IS NULL OR revoked_at_ms >= issued_at_ms)
);
CREATE INDEX operator_grant_account ON accounts.operator_grant(account_id, grant_id);

CREATE TABLE accounts.operator_session_proof (
    proof_id bytea PRIMARY KEY CHECK (octet_length(proof_id) = 16),
    session_id bytea NOT NULL CHECK (octet_length(session_id) = 16),
    account_id bytea NOT NULL CHECK (octet_length(account_id) = 16),
    credential_generation bigint NOT NULL CHECK (credential_generation > 0),
    enrolment_generation bigint NOT NULL CHECK (enrolment_generation > 0),
    grant_id bytea NOT NULL CHECK (octet_length(grant_id) = 16),
    grant_version bigint NOT NULL CHECK (grant_version > 0),
    verified_at_ms bigint NOT NULL CHECK (verified_at_ms >= 0),
    last_seen_at_ms bigint NOT NULL CHECK (last_seen_at_ms >= verified_at_ms),
    expires_at_ms bigint NOT NULL CHECK (expires_at_ms > verified_at_ms),
    revoked_at_ms bigint
);
CREATE INDEX operator_proof_session ON accounts.operator_session_proof(session_id);
CREATE INDEX operator_proof_account ON accounts.operator_session_proof(account_id);

CREATE TABLE accounts.operator_action_proof (
    proof_id bytea PRIMARY KEY CHECK (octet_length(proof_id) = 16),
    session_proof_id bytea NOT NULL CHECK (octet_length(session_proof_id) = 16),
    operation_id bytea NOT NULL CHECK (octet_length(operation_id) = 16),
    action smallint NOT NULL CHECK (action BETWEEN 1 AND 17),
    target jsonb NOT NULL,
    fingerprint bytea NOT NULL CHECK (octet_length(fingerprint) = 32),
    verified_at_ms bigint NOT NULL CHECK (verified_at_ms >= 0),
    expires_at_ms bigint NOT NULL CHECK (expires_at_ms > verified_at_ms),
    consumed_at_ms bigint,
    CHECK (consumed_at_ms IS NULL OR consumed_at_ms BETWEEN verified_at_ms AND expires_at_ms - 1)
);
CREATE INDEX operator_action_operation ON accounts.operator_action_proof(operation_id);

CREATE TABLE accounts.operator_audit (
    audit_id bigserial PRIMARY KEY,
    account_id bytea NOT NULL CHECK (octet_length(account_id) = 16),
    basis jsonb NOT NULL,
    action smallint NOT NULL CHECK (action BETWEEN 1 AND 17),
    target jsonb NOT NULL,
    reason text NOT NULL CHECK (char_length(btrim(reason)) BETWEEN 1 AND 500),
    operation_id bytea NOT NULL CHECK (octet_length(operation_id) = 16),
    fingerprint bytea NOT NULL CHECK (octet_length(fingerprint) = 32),
    outcome smallint NOT NULL CHECK (outcome BETWEEN 1 AND 4),
    correlation_id bytea NOT NULL CHECK (octet_length(correlation_id) = 16),
    at_ms bigint NOT NULL CHECK (at_ms >= 0)
);
CREATE INDEX operator_audit_account ON accounts.operator_audit(account_id, audit_id);
CREATE INDEX operator_audit_operation ON accounts.operator_audit(operation_id);

CREATE TABLE accounts.operator_receipt (
    operation_id bytea PRIMARY KEY CHECK (octet_length(operation_id) = 16),
    fingerprint bytea NOT NULL CHECK (octet_length(fingerprint) = 32),
    grant_id bytea NOT NULL CHECK (octet_length(grant_id) = 16),
    version bigint NOT NULL CHECK (version > 0),
    audit_id bigint NOT NULL REFERENCES accounts.operator_audit(audit_id)
);

CREATE FUNCTION accounts.operator_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'operator history is append-only' USING ERRCODE = '42501';
END;
$$;
CREATE TRIGGER operator_audit_immutable BEFORE UPDATE OR DELETE OR TRUNCATE
    ON accounts.operator_audit FOR EACH STATEMENT EXECUTE FUNCTION accounts.operator_append_only();
CREATE TRIGGER operator_receipt_immutable BEFORE UPDATE OR DELETE OR TRUNCATE
    ON accounts.operator_receipt FOR EACH STATEMENT EXECUTE FUNCTION accounts.operator_append_only();
REVOKE ALL ON accounts.operator_audit, accounts.operator_receipt FROM PUBLIC, df_accounts;
GRANT SELECT, INSERT ON accounts.operator_audit, accounts.operator_receipt TO df_accounts;
GRANT USAGE, SELECT ON SEQUENCE accounts.operator_audit_audit_id_seq TO df_accounts;
GRANT SELECT, INSERT, UPDATE ON accounts.operator_mfa, accounts.operator_enrolment_ticket,
    accounts.operator_grant, accounts.operator_session_proof, accounts.operator_action_proof TO df_accounts;
