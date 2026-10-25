-- Support inbox records are private operator data. Runtime service roles receive no grants.
BEGIN;
CREATE SCHEMA support;
REVOKE ALL ON SCHEMA support FROM PUBLIC;

CREATE TABLE support.mailbox (
    mailbox_id text PRIMARY KEY,
    address text NOT NULL,
    watch_expires_at timestamptz,
    committed_history_id text,
    bootstrap_boundary text,
    sync_lease_until timestamptz,
    sync_fence bigint NOT NULL DEFAULT 0 CHECK (sync_fence >= 0),
    sync_health text NOT NULL DEFAULT 'uninitialized',
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE support.ticket (
    ticket_id text PRIMARY KEY,
    mailbox_id text NOT NULL REFERENCES support.mailbox(mailbox_id),
    gmail_thread_id text NOT NULL,
    status text NOT NULL CHECK (status IN ('new','open','waiting_for_customer','waiting_for_internal','resolved','spam')),
    revision bigint NOT NULL CHECK (revision >= 0),
    created_at_unix_s bigint NOT NULL,
    first_received_at_unix_s bigint NOT NULL,
    last_inbound_at_unix_s bigint NOT NULL,
    assignee_operator_id text,
    classification_head text,
    sensitivity text NOT NULL DEFAULT 'ordinary',
    response_target_unix_s bigint,
    retention_class text NOT NULL DEFAULT 'default',
    hold_reference text,
    retained_until timestamptz,
    UNIQUE (mailbox_id, gmail_thread_id),
    UNIQUE (ticket_id, mailbox_id)
);
CREATE INDEX support_ticket_retention ON support.ticket (retained_until, ticket_id)
    WHERE retained_until IS NOT NULL;

CREATE TABLE support.party (
    party_id text NOT NULL,
    ticket_id text NOT NULL REFERENCES support.ticket(ticket_id) ON DELETE CASCADE,
    address_ref text NOT NULL,
    display_name text,
    is_verified boolean NOT NULL DEFAULT false,
    PRIMARY KEY (ticket_id, party_id),
    UNIQUE (ticket_id, address_ref)
);

CREATE TABLE support.message (
    message_id text PRIMARY KEY,
    ticket_id text NOT NULL REFERENCES support.ticket(ticket_id) ON DELETE CASCADE,
    mailbox_id text NOT NULL REFERENCES support.mailbox(mailbox_id),
    gmail_message_id text NOT NULL,
    direction text NOT NULL CHECK (direction IN ('inbound','outbound')),
    received_at_unix_s bigint NOT NULL,
    header_date_unix_s bigint,
    rfc_message_id text,
    in_reply_to text,
    gmail_labels jsonb NOT NULL DEFAULT '[]'::jsonb,
    quote_spans jsonb NOT NULL DEFAULT '[]'::jsonb,
    source_unavailable boolean NOT NULL DEFAULT false,
    truncated boolean NOT NULL DEFAULT false,
    sender_party_id text NOT NULL,
    recipient_party_ids jsonb NOT NULL,
    body_text text NOT NULL,
    UNIQUE (mailbox_id, gmail_message_id),
    FOREIGN KEY (ticket_id, mailbox_id) REFERENCES support.ticket(ticket_id, mailbox_id) ON DELETE CASCADE,
    FOREIGN KEY (ticket_id, sender_party_id) REFERENCES support.party(ticket_id, party_id)
        ON DELETE CASCADE
);
CREATE INDEX support_message_page ON support.message (ticket_id, received_at_unix_s, message_id);

CREATE TABLE support.classification (
    classification_id text PRIMARY KEY,
    ticket_id text NOT NULL REFERENCES support.ticket(ticket_id) ON DELETE CASCADE,
    source_revision bigint NOT NULL,
    data jsonb NOT NULL,
    created_at_unix_s bigint NOT NULL
);
CREATE TABLE support.proposal (
    proposal_id text PRIMARY KEY,
    ticket_id text NOT NULL REFERENCES support.ticket(ticket_id) ON DELETE CASCADE,
    operation_id text NOT NULL UNIQUE,
    source_revision bigint NOT NULL,
    status text NOT NULL CHECK (status IN ('draft','approved','queued','running','succeeded','failed','unknown','rejected','superseded','expired')),
    data jsonb NOT NULL
);
CREATE TABLE support.approval (
    approval_id text PRIMARY KEY,
    proposal_id text NOT NULL REFERENCES support.proposal(proposal_id) ON DELETE CASCADE,
    operator_id text NOT NULL,
    proof_id text NOT NULL,
    approved_payload_digest text NOT NULL,
    decided_at_unix_s bigint NOT NULL,
    decision text NOT NULL CHECK (decision IN ('approved','rejected'))
);
CREATE TABLE support.deadline (
    ticket_id text NOT NULL REFERENCES support.ticket(ticket_id) ON DELETE CASCADE,
    kind text NOT NULL,
    due_at_unix_s bigint NOT NULL,
    based_on_received_at_unix_s bigint NOT NULL,
    PRIMARY KEY (ticket_id, kind)
);
CREATE TABLE support.audit (
    audit_id text NOT NULL,
    operation_id text NOT NULL,
    ticket_id text NOT NULL REFERENCES support.ticket(ticket_id) ON DELETE CASCADE,
    data jsonb NOT NULL,
    created_at_unix_s bigint NOT NULL,
    PRIMARY KEY (ticket_id, audit_id),
    UNIQUE (operation_id, audit_id)
);
CREATE TABLE support.retired_source (
    mailbox_id text NOT NULL REFERENCES support.mailbox(mailbox_id),
    gmail_message_id text NOT NULL,
    retired_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (mailbox_id, gmail_message_id)
);

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'df_control') THEN
        CREATE ROLE df_control NOLOGIN;
    END IF;
END $$;
REVOKE ALL ON ALL TABLES IN SCHEMA support FROM PUBLIC;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA support FROM PUBLIC;
GRANT USAGE ON SCHEMA support TO df_control;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA support TO df_control;
COMMIT;
