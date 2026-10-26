-- Durable hosted-Checkout order, processor object and signed webhook inbox.
-- Applied by the owning persistence composition; raw bodies are retained for audit/replay.
CREATE SCHEMA IF NOT EXISTS payments;

CREATE TABLE payments.purchase_order (
    purchase_id bytea PRIMARY KEY CHECK (octet_length(purchase_id) = 16),
    account_id bytea NOT NULL CHECK (octet_length(account_id) = 16),
    operation_id bytea NOT NULL CHECK (octet_length(operation_id) = 16),
    amount_cents integer NOT NULL CHECK (amount_cents > 0),
    currency text NOT NULL CHECK (currency = 'usd'),
    state text NOT NULL CHECK (state IN ('created', 'processing', 'succeeded', 'refunded', 'disputed', 'failed')),
    created_at_ms bigint NOT NULL CHECK (created_at_ms >= 0),
    updated_at_ms bigint NOT NULL CHECK (updated_at_ms >= created_at_ms),
    UNIQUE (account_id, operation_id)
);

CREATE TABLE payments.processor_payment (
    processor_payment_id text PRIMARY KEY,
    purchase_id bytea NOT NULL UNIQUE REFERENCES payments.purchase_order(purchase_id),
    processor_account_id text NOT NULL,
    checkout_session_id text NOT NULL UNIQUE,
    status text NOT NULL,
    updated_at_ms bigint NOT NULL CHECK (updated_at_ms >= 0)
);

CREATE TABLE payments.webhook_inbox (
    event_id text PRIMARY KEY,
    processor_account_id text NOT NULL,
    processor_payment_id text NOT NULL,
    event_type text NOT NULL,
    raw_body bytea NOT NULL,
    signature text NOT NULL,
    received_at_ms bigint NOT NULL CHECK (received_at_ms >= 0),
    processed_at_ms bigint,
    processing_error text,
    CHECK ((processed_at_ms IS NULL) OR processed_at_ms >= received_at_ms)
);

CREATE INDEX webhook_inbox_pending ON payments.webhook_inbox (received_at_ms, event_id)
    WHERE processed_at_ms IS NULL;
