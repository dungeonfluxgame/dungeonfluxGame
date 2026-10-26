CREATE TABLE hosted.checkout_provider (
    account_id bytea NOT NULL CHECK (octet_length(account_id) = 16),
    request_id bytea NOT NULL CHECK (octet_length(request_id) = 16),
    provider text NOT NULL CHECK (provider IN ('stripe', 'paypal')),
    amount_cents integer NOT NULL CHECK (amount_cents BETWEEN 500 AND 50000),
    PRIMARY KEY (account_id, request_id)
);

CREATE TABLE hosted.paypal_checkout (
    operation_id bytea PRIMARY KEY CHECK (octet_length(operation_id) = 16),
    account_id bytea NOT NULL CHECK (octet_length(account_id) = 16),
    request_id bytea NOT NULL CHECK (octet_length(request_id) = 16),
    amount_cents integer NOT NULL CHECK (amount_cents BETWEEN 500 AND 50000),
    currency text NOT NULL CHECK (currency = 'USD'),
    is_test_mode boolean NOT NULL,
    paypal_order_id text UNIQUE,
    capture_id text UNIQUE,
    redirect_url text,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    capture_started_at timestamptz,
    paid boolean NOT NULL DEFAULT false,
    cancelled boolean NOT NULL DEFAULT false,
    capture_marker text CHECK (capture_marker IN ('DENIED', 'REVERSED')),
    UNIQUE (account_id, request_id)
);

CREATE TABLE hosted.paypal_event (
    event_id text PRIMARY KEY CHECK (octet_length(event_id) BETWEEN 1 AND 255),
    operation_id bytea NOT NULL REFERENCES hosted.paypal_checkout(operation_id),
    capture_id text NOT NULL,
    event_kind text NOT NULL CHECK (event_kind IN ('COMPLETED', 'DENIED', 'REVERSED')),
    received_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    credited boolean NOT NULL DEFAULT false
);
