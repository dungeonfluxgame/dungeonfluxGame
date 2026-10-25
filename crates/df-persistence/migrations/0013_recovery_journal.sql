-- The recovery journal's PostgreSQL side (planning/asset-storage-research.md 13; plan 13.10; D22).
-- Submitters insert pending entries in their own transaction; the single writer stamps a batch
-- with its sequence in one commit before writing the segment, so the batch for a sequence never
-- changes after a crash; entries at or below the confirmed object-store head are confirmed.

CREATE SCHEMA IF NOT EXISTS journal;

CREATE TABLE journal.pending_entry (
    entry_id bytea PRIMARY KEY CHECK (octet_length(entry_id) = 16),
    kind text NOT NULL CHECK (octet_length(kind) <= 64),
    stable_id bytea NOT NULL CHECK (octet_length(stable_id) BETWEEN 16 AND 64),
    metadata bytea NOT NULL CHECK (octet_length(metadata) <= 4096),
    submitted_at_ms bigint NOT NULL,
    recovery_epoch bigint,
    sequence bigint CHECK (sequence >= 1),
    confirmed_at_ms bigint,
    CHECK ((recovery_epoch IS NULL) = (sequence IS NULL)),
    CHECK (confirmed_at_ms IS NULL OR sequence IS NOT NULL)
);
CREATE INDEX pending_entry_unstamped ON journal.pending_entry (submitted_at_ms)
    WHERE sequence IS NULL;
CREATE INDEX pending_entry_batch ON journal.pending_entry (recovery_epoch, sequence);

-- The single writer's lease, fenced like a step lease.
CREATE TABLE journal.writer_lease (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    owner bytea NOT NULL CHECK (octet_length(owner) = 16),
    lease_until_ms bigint NOT NULL
);

-- The last head the writer confirmed in the object store (informational: start-up trusts the head
-- read from the object store, verified against the off-host watermark).
CREATE TABLE journal.confirmed_head (
    recovery_epoch bigint PRIMARY KEY,
    sequence bigint NOT NULL CHECK (sequence >= 1),
    chain_sha256 bytea NOT NULL CHECK (octet_length(chain_sha256) = 32),
    confirmed_at_ms bigint NOT NULL
);
