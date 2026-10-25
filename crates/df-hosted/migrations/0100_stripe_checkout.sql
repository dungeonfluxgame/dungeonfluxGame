CREATE TABLE hosted.stripe_checkout (
    operation_id bytea PRIMARY KEY CHECK (octet_length(operation_id) = 16),
    account_id bytea NOT NULL CHECK (octet_length(account_id) = 16),
    request_id bytea NOT NULL CHECK (octet_length(request_id) = 16),
    amount_cents integer NOT NULL CHECK (amount_cents BETWEEN 500 AND 50000),
    currency text NOT NULL CHECK (currency = 'usd'),
    is_test_mode boolean NOT NULL,
    stripe_session_id text UNIQUE,
    redirect_url text,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    paid boolean NOT NULL DEFAULT false,
    UNIQUE (account_id, request_id)
);

CREATE TABLE hosted.stripe_event (
    event_id text PRIMARY KEY CHECK (octet_length(event_id) BETWEEN 1 AND 255),
    operation_id bytea NOT NULL REFERENCES hosted.stripe_checkout(operation_id),
    received_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    credited boolean NOT NULL DEFAULT false
);
