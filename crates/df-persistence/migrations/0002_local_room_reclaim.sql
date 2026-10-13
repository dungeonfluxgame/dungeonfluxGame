-- SES-04: additive local demonstration authority records, version 1.
-- Applied in the seed/restore transaction; canonical gameplay checkpoints are unchanged.
CREATE TABLE IF NOT EXISTS df_local_demo.room_seats (
    session_id bytea NOT NULL CHECK(octet_length(session_id)=16),
    principal_id bytea NOT NULL CHECK(octet_length(principal_id)=16),
    credential bytea NOT NULL UNIQUE REFERENCES df_local_demo.grants,
    seat_token bytea NOT NULL UNIQUE CHECK(octet_length(seat_token)=32),
    last_seen_millis bigint NOT NULL CHECK(last_seen_millis>=0),
    away_announced boolean NOT NULL DEFAULT false,
    version integer NOT NULL DEFAULT 1 CHECK(version=1),
    PRIMARY KEY(session_id,principal_id)
);
CREATE TABLE IF NOT EXISTS df_local_demo.seat_reclaims (
    session_id bytea NOT NULL CHECK(octet_length(session_id)=16),
    operation_id bytea NOT NULL CHECK(octet_length(operation_id)=16),
    canonical_fingerprint bytea NOT NULL CHECK(octet_length(canonical_fingerprint)=32),
    principal_id bytea NOT NULL CHECK(octet_length(principal_id)=16),
    credential bytea NOT NULL REFERENCES df_local_demo.grants,
    run_id bytea NOT NULL CHECK(octet_length(run_id)=16),
    recovery_epoch numeric(20,0) NOT NULL,
    in_epoch_sequence numeric(20,0) NOT NULL,
    version integer NOT NULL DEFAULT 1 CHECK(version=1),
    PRIMARY KEY(session_id,operation_id)
);
