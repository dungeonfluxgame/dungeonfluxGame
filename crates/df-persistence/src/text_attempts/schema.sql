CREATE SCHEMA IF NOT EXISTS df_text;

-- One row per scoped operation key. `phase` only moves along the transitions in
-- df_ai::text_attempts: accepted -> dispatched -> candidate -> settled | rejected,
-- dispatched -> settled | unknown, and accepted -> cancelled; a settled answer can be withdrawn to
-- rejected. Every transition is an UPDATE guarded by the phase it expects, so an unknown, cancelled
-- or rejected record never changes again.
--
-- This table replaces `df_text.attempt` from the first slice, which held only probe rows and is
-- never read again. It is left in place rather than dropped.
CREATE TABLE IF NOT EXISTS df_text.text_attempt (
    scope text NOT NULL
        CHECK (char_length(scope) BETWEEN 1 AND 128),
    operation_key text NOT NULL
        CHECK (char_length(operation_key) BETWEEN 1 AND 128),
    fingerprint bytea NOT NULL
        CHECK (octet_length(fingerprint) = 32),
    purpose text NOT NULL
        CHECK (char_length(purpose) BETWEEN 1 AND 64),
    profile text NOT NULL
        CHECK (char_length(profile) BETWEEN 1 AND 128),
    output_name text NOT NULL
        CHECK (char_length(output_name) BETWEEN 1 AND 128),
    phase text NOT NULL
        CHECK (phase IN ('accepted', 'dispatched', 'candidate', 'settled', 'rejected', 'unknown', 'cancelled')),
    owner bytea
        CHECK (owner IS NULL OR octet_length(owner) = 16),
    basis bytea
        CHECK (basis IS NULL OR octet_length(basis) = 32),
    outcome_kind text
        CHECK (outcome_kind IS NULL OR outcome_kind IN ('answered', 'unavailable', 'deadline', 'invalid', 'stale')),
    outcome_reason text
        CHECK (outcome_reason IS NULL OR char_length(outcome_reason) BETWEEN 1 AND 64),
    answer text
        CHECK (answer IS NULL OR left(answer, 1) = '{'),
    usage text
        CHECK (usage IS NULL OR left(usage, 1) = '{'),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (scope, operation_key),
    CHECK (phase NOT IN ('dispatched', 'candidate', 'settled', 'rejected', 'unknown')
           OR (owner IS NOT NULL AND basis IS NOT NULL)),
    CHECK ((phase = 'settled') = (outcome_kind IS NOT NULL)),
    CHECK (phase <> 'candidate' OR answer IS NOT NULL),
    CHECK (outcome_kind IS DISTINCT FROM 'answered' OR answer IS NOT NULL),
    CHECK (outcome_kind IS DISTINCT FROM 'unavailable' OR outcome_reason IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS text_attempt_recent ON df_text.text_attempt (updated_at DESC);
