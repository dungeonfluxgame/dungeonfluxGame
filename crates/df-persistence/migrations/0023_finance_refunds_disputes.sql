-- Finance refunds, disputes and manual adjustments (planning/finance-ledger-research.md 14, 17.4).
-- The tables and their constraints; the refund and dispute flows that write them are FIN-05
-- (df-commerce refund/dispute planners, df-finance refunds service, migration 0025 boundary).

CREATE TABLE finance.refund (
    refund_id bytea PRIMARY KEY CHECK (octet_length(refund_id) = 16),
    payer_account_id bytea NOT NULL REFERENCES finance.payer,
    operation_id bytea NOT NULL,
    requested_micro_dollars bigint NOT NULL CHECK (requested_micro_dollars > 0),
    state text NOT NULL CHECK (state IN ('pending', 'paid', 'partly_paid', 'review', 'returned')),
    requested_at_ms bigint NOT NULL,
    released_allocations jsonb NOT NULL DEFAULT '[]',
    revision bigint NOT NULL DEFAULT 1,
    UNIQUE (payer_account_id, operation_id)
);
CREATE TABLE finance.refund_lot (
    refund_lot_id bytea PRIMARY KEY CHECK (octet_length(refund_lot_id) = 16),
    refund_id bytea NOT NULL REFERENCES finance.refund,
    lot_id bytea NOT NULL REFERENCES finance.funding_lot,
    principal_micro_dollars bigint NOT NULL CHECK (principal_micro_dollars > 0),
    tax_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (tax_micro_dollars >= 0),
    processor_refund_ref text,
    payout_method text NOT NULL CHECK (payout_method IN ('processor', 'manual')),
    state text NOT NULL CHECK (state IN ('pending', 'unknown', 'paid', 'failed_review',
        'returned_to_credit', 'manual_pending', 'blocked_by_dispute', 'cancelled_dispute_lost')),
    object_revision bigint NOT NULL DEFAULT 1,
    updated_at_ms bigint NOT NULL,
    revision bigint NOT NULL DEFAULT 1,
    UNIQUE (refund_id, lot_id)
);
CREATE TABLE finance.refund_outbox (
    refund_lot_id bytea PRIMARY KEY REFERENCES finance.refund_lot,
    claimed_by bytea,
    lease_until_ms bigint,
    attempts integer NOT NULL DEFAULT 0,
    next_attempt_at_ms bigint NOT NULL DEFAULT 0
);
CREATE TABLE finance.dispute (
    processor text NOT NULL,
    dispute_ref text NOT NULL,
    lot_id bytea NOT NULL REFERENCES finance.funding_lot,
    disputed_micro_dollars bigint NOT NULL CHECK (disputed_micro_dollars > 0),
    status text NOT NULL CHECK (status IN ('open', 'won', 'lost')),
    fee_micro_dollars bigint NOT NULL DEFAULT 0,
    opened_at_ms bigint NOT NULL,
    closed_at_ms bigint,
    revision bigint NOT NULL DEFAULT 1,
    PRIMARY KEY (processor, dispute_ref)
);
CREATE TABLE finance.manual_adjustment (
    adjustment_id bytea PRIMARY KEY CHECK (octet_length(adjustment_id) = 16),
    entry_id bytea REFERENCES finance.journal_entry,
    actor_id bytea NOT NULL,
    approver_id bytea,
    case_ref text NOT NULL,
    reason_text text,
    created_at_ms bigint NOT NULL,
    CHECK (approver_id IS NULL OR approver_id <> actor_id)
);
CREATE TABLE finance.tax_policy (
    tax_policy_version integer PRIMARY KEY,
    mode text NOT NULL CHECK (mode IN ('none', 'at_purchase', 'at_consumption')),
    jurisdictions jsonb NOT NULL DEFAULT '[]',
    effective_from_ms bigint NOT NULL
);
INSERT INTO finance.tax_policy VALUES (1, 'none', '[]', 0);
CREATE TRIGGER tax_policy_append_only BEFORE UPDATE OR DELETE ON finance.tax_policy
    FOR EACH ROW EXECUTE FUNCTION finance.refuse_change();
