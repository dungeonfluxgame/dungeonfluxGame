-- Atomic manual triage receipts for the support queue. Model classifications stay immutable.
BEGIN;
CREATE TABLE support.triage_operation (
    operation_id text PRIMARY KEY,
    ticket_id text NOT NULL REFERENCES support.ticket(ticket_id) ON DELETE CASCADE,
    operator_id text NOT NULL,
    expected_revision bigint NOT NULL CHECK (expected_revision >= 0),
    source_revision bigint NOT NULL CHECK (source_revision >= 0 AND source_revision <= expected_revision),
    requested_status text CHECK (requested_status IN
        ('new','open','waiting_for_customer','waiting_for_internal','resolved','spam')),
    requested_category text CHECK (requested_category IN
        ('billing_refund','payment_dispute','bug_report','account_access','privacy_data_request',
         'legal_notice_takedown_ip','abuse_safety','feedback','sales_press','spam','undetermined')),
    reason text NOT NULL CHECK (length(btrim(reason)) BETWEEN 1 AND 500),
    previous_revision bigint NOT NULL CHECK (previous_revision = expected_revision),
    new_revision bigint NOT NULL CHECK (new_revision = previous_revision + 1),
    resulting_status text NOT NULL CHECK (resulting_status IN
        ('new','open','waiting_for_customer','waiting_for_internal','resolved','spam')),
    resulting_category text CHECK (resulting_category IN
        ('billing_refund','payment_dispute','bug_report','account_access','privacy_data_request',
         'legal_notice_takedown_ip','abuse_safety','feedback','sales_press','spam','undetermined')),
    audit_id text NOT NULL,
    created_at_unix_s bigint NOT NULL,
    CHECK (requested_status IS NOT NULL OR requested_category IS NOT NULL),
    FOREIGN KEY (ticket_id, audit_id) REFERENCES support.audit(ticket_id, audit_id) ON DELETE CASCADE
);
CREATE INDEX support_classification_ticket_version ON support.classification
    (ticket_id, source_revision DESC, created_at_unix_s DESC, classification_id DESC);
CREATE INDEX support_ticket_queue ON support.ticket (last_inbound_at_unix_s DESC, ticket_id);
GRANT SELECT, INSERT, UPDATE, DELETE ON support.triage_operation TO df_control;
COMMIT;
