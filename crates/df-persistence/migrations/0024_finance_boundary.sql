-- Finance posting boundary, reconciliation tables and roles (planning/finance-ledger-research.md
-- 17.5, 17.6, 17.9, F round 1). The only write path from the service to a finance table is
-- `lock_flow` (takes the locks of one flow in rank order and returns the snapshot read under them)
-- followed by `commit_event` (checks and writes one event: entries, hash chain, balances, row
-- changes, the journal row). `df_finance` holds SELECT and EXECUTE only.
-- TODO(FIN-01): `journal.submit_pending` does not exist yet; `finance.pending_event` stands in for
-- the recovery journal's pending table and is replaced when O2 lands.
-- TODO(FIN-02): `replay_event`, `record_replay_check` and the replay job (22.2); the tables and the
-- dedicated role exist now.

CREATE TABLE finance.pending_event (
    pending_id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    event_id bytea NOT NULL UNIQUE,
    event_kind text NOT NULL,
    payload_schema integer NOT NULL,
    payload jsonb NOT NULL,
    metadata jsonb NOT NULL,
    recorded_at_ms bigint NOT NULL
);
CREATE TRIGGER pending_event_append_only BEFORE UPDATE OR DELETE ON finance.pending_event
    FOR EACH ROW EXECUTE FUNCTION finance.refuse_change();

CREATE TABLE finance.reconciliation_run (
    run_id bytea PRIMARY KEY CHECK (octet_length(run_id) = 16),
    kind text NOT NULL CHECK (kind IN ('journal_integrity', 'journal_compare', 'integrity_controls',
        'processor', 'payouts', 'bank', 'supplier_usage', 'reservations', 'meter', 'subledger',
        'outcome_feeds', 'campaign_restore', 'full')),
    scope text,
    as_of_ms bigint NOT NULL,
    watermark_journal_sequence bigint,
    stream_heads_digest bytea CHECK (octet_length(stream_heads_digest) = 32),
    started_at_ms bigint NOT NULL,
    finished_at_ms bigint,
    outcome text CHECK (outcome IN ('clean', 'differences', 'failed'))
);
CREATE TABLE finance.reconciliation_difference (
    difference_id bytea PRIMARY KEY CHECK (octet_length(difference_id) = 16),
    run_id bytea NOT NULL REFERENCES finance.reconciliation_run,
    kind text NOT NULL,
    reference text NOT NULL,
    expected_micro_dollars bigint,
    observed_micro_dollars bigint,
    state text NOT NULL CHECK (state IN ('open', 'within_tolerance', 'explained', 'corrected')),
    explanation_code text,
    corrected_by_entry bytea REFERENCES finance.journal_entry
);
CREATE TABLE finance.reconciliation_signoff (
    run_id bytea PRIMARY KEY REFERENCES finance.reconciliation_run,
    reviewer_id bytea NOT NULL,
    signed_at_ms bigint NOT NULL,
    note_code text
);
CREATE TABLE finance.period_balance (
    period_id integer NOT NULL REFERENCES finance.accounting_period,
    key_id bigint NOT NULL REFERENCES finance.account_key,
    opening_micro_dollars bigint NOT NULL,
    debits_micro_dollars bigint NOT NULL,
    credits_micro_dollars bigint NOT NULL,
    closing_micro_dollars bigint NOT NULL,
    PRIMARY KEY (period_id, key_id)
);
CREATE TRIGGER period_balance_append_only BEFORE UPDATE OR DELETE ON finance.period_balance
    FOR EACH ROW EXECUTE FUNCTION finance.refuse_change();
CREATE TABLE finance.export_manifest (
    period_id integer NOT NULL REFERENCES finance.accounting_period,
    kind text NOT NULL,
    format text NOT NULL,
    generation integer NOT NULL,
    digest bytea NOT NULL CHECK (octet_length(digest) = 32),
    watermark_journal_sequence bigint,
    stream_heads_digest bytea,
    as_of_ms bigint NOT NULL,
    generated_at_ms bigint NOT NULL,
    provisional boolean NOT NULL,
    PRIMARY KEY (period_id, kind, format, generation)
);
CREATE TABLE finance.restore_replay (
    recovery_epoch bigint NOT NULL,
    journal_sequence bigint NOT NULL,
    position integer NOT NULL,
    event_id bytea NOT NULL,
    applied_at_ms bigint NOT NULL,
    PRIMARY KEY (recovery_epoch, journal_sequence, position)
);
CREATE TABLE finance.reconstruction (
    reconstruction_id bytea PRIMARY KEY CHECK (octet_length(reconstruction_id) = 16),
    tenant_id bytea NOT NULL,
    campaign_id bytea NOT NULL,
    outcome text NOT NULL CHECK (outcome IN ('reconstructed', 'failed')),
    finished_at_ms bigint NOT NULL
);
CREATE TABLE finance.replay_state (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    status text NOT NULL CHECK (status IN ('idle', 'replaying', 'rebuilding_work', 'checking', 'complete', 'failed')),
    recovery_epoch bigint NOT NULL,
    started_at_ms bigint,
    events_total bigint,
    events_applied bigint,
    last_progress_at_ms bigint,
    failure_reason text
);
INSERT INTO finance.replay_state (status, recovery_epoch) VALUES ('idle', 0);
CREATE TABLE finance.replay_check_record (
    check_id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    recovery_epoch bigint NOT NULL,
    result text NOT NULL CHECK (result IN ('pass', 'fail')),
    stream_heads_digest bytea NOT NULL CHECK (octet_length(stream_heads_digest) = 32),
    journal_sequence bigint NOT NULL,
    checked_at_ms bigint NOT NULL,
    detail_code text
);

-- The registry of row changes `apply_change` may write (17.6). Phase 1 changes create the rows
-- account keys and entries depend on and run before the entries; phase 2 changes run after them.
-- `columns` are the columns an insert row may carry (a column that must default, such as `revision`
-- or a running total, is not among them); `updatable_cols` are the columns an update may change
-- besides its key; `journal_cols` are the only columns of a row image the journal payload keeps (22.1).
CREATE TABLE finance.change_kind (
    kind text PRIMARY KEY,
    table_name text NOT NULL,
    op text NOT NULL CHECK (op IN ('insert', 'update', 'upsert', 'delete', 'key')),
    pk_cols text[] NOT NULL,
    phase smallint NOT NULL CHECK (phase IN (1, 2)),
    unjournaled boolean NOT NULL DEFAULT false,
    columns text[] NOT NULL DEFAULT '{}',
    updatable_cols text[] NOT NULL DEFAULT '{}',
    journal_cols text[] NOT NULL DEFAULT '{}'
);
INSERT INTO finance.change_kind (kind, table_name, op, pk_cols, phase, unjournaled, columns, updatable_cols) VALUES
    ('key_create', 'account_key', 'key', '{}', 1, false,
        '{code,currency,payer,lot,allocation,provider,processor,loss_class,jurisdiction,promo_source}', '{}'),
    ('payer_insert', 'payer', 'insert', '{account_id}', 1, false,
        '{account_id,standing,standing_reason,created_at_ms}', '{}'),
    ('funding_lot_insert', 'funding_lot', 'insert', '{lot_id}', 1, false,
        '{lot_id,payer_account_id,class,promo_source,currency,original_micro_dollars,state,processor,payment_ref,charge_ref,purchase_order_ref,tax_mode,tax_jurisdiction,tax_rate_numerator,tax_rate_denominator,tax_micro_dollars,billing_country,billing_region,tax_policy_version,created_at_ms,expires_at_ms}',
        '{}'),
    ('funding_lot_update', 'funding_lot', 'update', '{lot_id}', 2, false, '{}',
        '{consumed_micro_dollars,extinguished_micro_dollars}'),
    ('processor_payment_state_insert', 'processor_payment_state', 'insert', '{processor,payment_ref}', 1, false,
        '{processor,payment_ref,lot_id,captured,captured_micro_dollars,applied_at_ms,object_revision}', '{}'),
    ('trial_grant_insert', 'trial_grant', 'insert', '{account_id}', 1, false,
        '{account_id,email_grant_key,lot_id,granted_at_ms}', '{}'),
    ('price_version_insert', 'price_version', 'insert', '{price_version_id}', 1, false,
        '{price_version_id,label,effective_from_ms,currency,platform_rate_micro_dollars_per_hour,markup_numerator,markup_denominator,tax_policy_version,presets,proposed_pilot,status,tariff_digest,published_at_ms,published_by}',
        '{}'),
    ('price_version_tariff_insert', 'price_version_tariff', 'insert',
        '{price_version_id,provider,model,category,context_tier_max_tokens}', 1, false,
        '{price_version_id,provider,model,category,context_tier_max_tokens,rate_nano_dollars,per_units,as_of,source}', '{}'),
    ('session_funding_insert', 'session_funding', 'insert', '{session_id}', 1, false,
        '{session_id,tenant_id,campaign_id,price_version_id,currency,status,hourly_ceiling_micro_dollars,session_ceiling_micro_dollars,per_request_threshold_micro_dollars,split_mode,contributions_visible,generation_policy_digest,opened_at_ms}',
        '{}'),
    ('session_funding_update', 'session_funding', 'update', '{session_id}', 2, false, '{}',
        '{current_epoch}'),
    ('session_charged_total_update', 'session_funding', 'update', '{session_id}', 2, false, '{}',
        '{charged_total_micro_dollars}'),
    ('funding_epoch_insert', 'funding_epoch', 'insert', '{session_id,epoch_seq}', 2, false,
        '{session_id,epoch_seq,reason,split_mode,created_at_ms}', '{}'),
    ('epoch_share_insert', 'epoch_share', 'insert', '{session_id,epoch_seq,allocation_id}', 2, false,
        '{session_id,epoch_seq,allocation_id,weight}', '{}'),
    ('allocation_insert', 'allocation', 'insert', '{allocation_id}', 1, false,
        '{allocation_id,session_id,payer_account_id,authorized_micro_dollars,drawn_micro_dollars,status,consent_revision,created_at_ms}',
        '{}'),
    ('allocation_update', 'allocation', 'update', '{allocation_id}', 2, false, '{}',
        '{authorized_micro_dollars,drawn_micro_dollars,returned_micro_dollars,charged_micro_dollars,consent_revision,status}'),
    ('allocation_lot_insert', 'allocation_lot', 'insert', '{allocation_id,lot_id}', 1, false,
        '{allocation_id,lot_id,draw_rank}', '{}'),
    ('allocation_consent_insert', 'allocation_consent', 'insert', '{allocation_id,consent_revision}', 2, false,
        '{allocation_id,consent_revision,price_version_id,approved_hourly_ceiling,approved_session_ceiling,approved_split_mode,approval_threshold_micro_dollars,generation_policy_digest,consent_terms_version,consented_at_ms}',
        '{}'),
    ('quote_insert', 'quote', 'insert', '{quote_id}', 2, false,
        '{quote_id,session_id,request_digest,price_version_id,work_class,retail_estimate_micro_dollars,retail_maximum_micro_dollars,supplier_maximum_micro_dollars,created_at_ms,expires_at_ms,state}',
        '{}'),
    ('quote_update', 'quote', 'update', '{quote_id}', 2, false, '{}', '{state,used_by_reservation}'),
    ('reservation_insert', 'reservation', 'insert', '{reservation_id}', 2, false,
        '{reservation_id,session_id,payer_kind,company_class,kind,operation_id,quote_id,price_version_id,retail_maximum_micro_dollars,admitted_at_ms,state}',
        '{}'),
    ('reservation_update', 'reservation', 'update', '{reservation_id}', 2, false, '{}',
        '{state,charged_micro_dollars,released_micro_dollars}'),
    ('reservation_share_insert', 'reservation_share', 'insert', '{reservation_id,allocation_id,lot_id}', 2, false,
        '{reservation_id,allocation_id,lot_id,class,held_micro_dollars}', '{}'),
    ('reservation_share_update', 'reservation_share', 'update', '{reservation_id,allocation_id,lot_id}', 2, false, '{}',
        '{charged_micro_dollars,released_micro_dollars,reversed_micro_dollars}'),
    ('provider_attempt_insert', 'provider_attempt', 'insert', '{attempt_id}', 2, false,
        '{attempt_id,reservation_id,effect_id,campaign_id,account_id,tenant_id,provider,model,route,media_asset_id,supplier_maximum_micro_dollars,supplier_exposure_micro_dollars,dispatch_state,customer_state,supplier_state,admitted_at_ms,fence_recovery_epoch,fence_claim_seq,fence_placement_id}',
        '{}'),
    ('provider_attempt_update', 'provider_attempt', 'update', '{attempt_id}', 2, false, '{}',
        '{dispatch_state,customer_state,supplier_state,dispatcher_instance_id,supplier_outcome,usage_source,supplier_cost_micro_dollars,usability,prepared_at_ms,sent_at_ms,result_at_ms,customer_released_at_ms,settled_at_ms}'),
    ('usage_event_insert', 'usage_event', 'insert', '{usage_event_id}', 2, false,
        '{usage_event_id,attempt_id,meter_session_id,meter_segment_seq,session_id,category,quantity_text,units,retail_micro_dollars,supplier_micro_dollars,supplier_purchased_micro_dollars,supplier_promotional_micro_dollars,price_version_id,entry_id,settled_at_ms}',
        '{}'),
    ('usage_event_update', 'usage_event', 'update', '{usage_event_id}', 2, false, '{}',
        '{reversed_micro_dollars}'),
    ('usage_reversal_insert', 'usage_reversal', 'insert', '{usage_event_id,operation_id}', 2, false,
        '{usage_event_id,operation_id,micro_dollars,reason,entry_id}', '{}'),
    ('operation_receipt_insert', 'operation_receipt', 'insert', '{namespace,principal,operation_id}', 2, false,
        '{namespace,principal,operation_id,fingerprint,outcome,result_ref,created_at_ms}', '{}'),
    ('settlement_fact_insert', 'settlement_fact', 'insert', '{attempt_id}', 2, false, '{}', '{}'),
    ('attempt_outcome_event_insert', 'attempt_outcome_event', 'insert', '{attempt_id}', 2, false, '{}', '{}'),
    ('company_loss_insert', 'company_loss', 'insert', '{attempt_id,loss_class}', 2, false, '{}', '{}'),
    ('forgone_revenue_insert', 'forgone_revenue', 'insert', '{reason,ref_id}', 2, false, '{}', '{}'),
    ('meter_segment_priced_insert', 'meter_segment_priced', 'insert', '{session_id,segment_seq}', 2, false, '{}', '{}'),
    ('work_item_upsert', 'work_item', 'upsert', '{kind,ref_id}', 2, true, '{kind,ref_id,due_at_ms}', '{}'),
    ('work_item_complete', 'work_item', 'delete', '{kind,ref_id}', 2, true, '{kind,ref_id}', '{}');

-- The insert columns of the tables whose rows the first rounds do not exercise are every column but
-- `revision`; a later round narrows them when it builds the flow.
UPDATE finance.change_kind k
   SET columns = ARRAY(SELECT c.column_name::text FROM information_schema.columns c
                        WHERE c.table_schema = 'finance' AND c.table_name = k.table_name
                          AND c.column_name <> 'revision' AND c.is_identity = 'NO' AND c.is_generated = 'NEVER'
                        ORDER BY c.ordinal_position)
 WHERE k.op = 'insert' AND k.columns = '{}';

-- What the journal payload keeps (22.1): the row's own columns without the personal ones, never a
-- column added later by default. The golden list in the tests pins the result.
UPDATE finance.change_kind k
   SET journal_cols = ARRAY(
        SELECT c FROM unnest(CASE WHEN k.op IN ('insert', 'key') THEN k.columns
                                  WHEN k.op = 'update' THEN k.pk_cols || k.updatable_cols
                                  ELSE '{}'::text[] END) AS c
         WHERE c <> ALL (ARRAY['billing_country', 'billing_region', 'email_grant_key', 'reason_text',
                               'provider_request_ref', 'quantity_text', 'standing_reason']))
 WHERE NOT k.unjournaled;

CREATE TRIGGER change_kind_append_only BEFORE UPDATE OR DELETE ON finance.change_kind
    FOR EACH ROW EXECUTE FUNCTION finance.refuse_change();

-- Event kinds (17.6): which row changes and which entry kinds each may carry.
INSERT INTO finance.event_kind (event_kind, carries_entries, journaled, allowed_changes, allowed_entries) VALUES
    ('payer_registered', false, true, '{payer_insert}', '{}'),
    ('price_version_published', false, true, '{price_version_insert,price_version_tariff_insert,key_create}', '{}'),
    ('session_funding_opened', false, true, '{session_funding_insert,funding_epoch_insert}', '{}'),
    ('trial_granted', true, true, '{payer_insert,funding_lot_insert,key_create,trial_grant_insert}', '{trial_grant}'),
    ('purchase_applied', true, true,
        '{payer_insert,funding_lot_insert,key_create,processor_payment_state_insert}', '{purchase}'),
    ('allocation_made', true, true,
        '{allocation_insert,allocation_update,allocation_lot_insert,allocation_consent_insert,key_create,funding_epoch_insert,epoch_share_insert,session_funding_update,operation_receipt_insert}',
        '{allocation}'),
    ('admission_admitted', true, true,
        '{reservation_insert,reservation_share_insert,provider_attempt_insert,quote_update,operation_receipt_insert,work_item_upsert}',
        '{reservation}'),
    ('settlement_applied', true, true,
        '{usage_event_insert,reservation_update,reservation_share_update,provider_attempt_update,funding_lot_update,allocation_update,settlement_fact_insert,work_item_upsert,work_item_complete,company_loss_insert,session_charged_total_update}',
        '{settlement}'),
    ('release_applied', true, true,
        '{reservation_update,reservation_share_update,provider_attempt_update,allocation_update,funding_lot_update,work_item_upsert,work_item_complete,attempt_outcome_event_insert,operation_receipt_insert}',
        '{release,allocation_release}'),
    ('credit_applied', true, true,
        '{usage_event_update,reservation_share_update,funding_lot_update,usage_reversal_insert,operation_receipt_insert}',
        '{customer_credit}'),
    ('quote_created', false, true, '{quote_insert}', '{}'),
    ('quote_decided', false, true, '{quote_update}', '{}'),
    ('attempt_prepared', false, true, '{provider_attempt_update,work_item_upsert,work_item_complete}', '{}'),
    ('attempt_sent', false, true, '{provider_attempt_update,work_item_upsert,work_item_complete}', '{}'),
    ('usage_recorded', false, true, '{provider_attempt_update,work_item_upsert,settlement_fact_insert}', '{}'),
    ('processing_fee_posted', true, true, '{key_create}', '{processing_fee,payout}'),
    ('manual_adjustment_posted', true, true, '{key_create}', '{manual_adjustment}');

-- The columns an update may set, per event kind and change kind (17.6): a settlement cannot change
-- what only a credit changes, and no event kind can change a column no flow of its own writes.
-- `change_kind.updatable_cols` is the union; `apply_change` takes the narrower list.
CREATE TABLE finance.event_update_cols (
    event_kind text NOT NULL REFERENCES finance.event_kind (event_kind),
    kind text NOT NULL REFERENCES finance.change_kind (kind),
    cols text[] NOT NULL CHECK (cardinality(cols) > 0),
    PRIMARY KEY (event_kind, kind)
);
INSERT INTO finance.event_update_cols (event_kind, kind, cols) VALUES
    ('allocation_made', 'allocation_update', '{authorized_micro_dollars,drawn_micro_dollars,consent_revision}'),
    ('allocation_made', 'session_funding_update', '{current_epoch}'),
    ('admission_admitted', 'quote_update', '{state,used_by_reservation}'),
    ('settlement_applied', 'reservation_update', '{state,charged_micro_dollars,released_micro_dollars}'),
    ('settlement_applied', 'reservation_share_update', '{charged_micro_dollars,released_micro_dollars}'),
    ('settlement_applied', 'provider_attempt_update',
        '{dispatch_state,customer_state,supplier_state,supplier_outcome,usage_source,supplier_cost_micro_dollars,usability,result_at_ms,settled_at_ms}'),
    ('settlement_applied', 'funding_lot_update', '{consumed_micro_dollars,extinguished_micro_dollars}'),
    ('settlement_applied', 'allocation_update', '{charged_micro_dollars,returned_micro_dollars}'),
    ('settlement_applied', 'session_charged_total_update', '{charged_total_micro_dollars}'),
    ('release_applied', 'reservation_update', '{state,charged_micro_dollars,released_micro_dollars}'),
    ('release_applied', 'reservation_share_update', '{charged_micro_dollars,released_micro_dollars}'),
    ('release_applied', 'provider_attempt_update',
        '{dispatch_state,customer_state,supplier_state,supplier_outcome,customer_released_at_ms}'),
    ('release_applied', 'funding_lot_update', '{consumed_micro_dollars,extinguished_micro_dollars}'),
    ('release_applied', 'allocation_update', '{authorized_micro_dollars,returned_micro_dollars,status}'),
    ('credit_applied', 'usage_event_update', '{reversed_micro_dollars}'),
    ('credit_applied', 'reservation_share_update', '{reversed_micro_dollars}'),
    ('credit_applied', 'funding_lot_update', '{consumed_micro_dollars}'),
    ('quote_decided', 'quote_update', '{state}'),
    ('attempt_prepared', 'provider_attempt_update', '{dispatch_state,prepared_at_ms}'),
    ('attempt_sent', 'provider_attempt_update', '{dispatch_state,dispatcher_instance_id,sent_at_ms}'),
    ('usage_recorded', 'provider_attempt_update', '{usage_source,supplier_cost_micro_dollars,usability,result_at_ms}');
CREATE TRIGGER event_update_cols_append_only BEFORE UPDATE OR DELETE ON finance.event_update_cols
    FOR EACH ROW EXECUTE FUNCTION finance.refuse_change();
-- The two lists agree: every allowed update has its columns, they sit inside the union, and the
-- union holds nothing no event kind uses.
DO $$
DECLARE
    v_bad text;
BEGIN
    SELECT string_agg(e.event_kind || '/' || c, ', ') INTO v_bad
      FROM finance.event_kind e CROSS JOIN LATERAL unnest(e.allowed_changes) c
      JOIN finance.change_kind k ON k.kind = c AND k.op = 'update'
     WHERE NOT EXISTS (SELECT 1 FROM finance.event_update_cols u WHERE u.event_kind = e.event_kind AND u.kind = c);
    IF v_bad IS NOT NULL THEN
        RAISE EXCEPTION 'update changes without a column list: %', v_bad;
    END IF;
    SELECT string_agg(u.event_kind || '/' || u.kind, ', ') INTO v_bad
      FROM finance.event_update_cols u JOIN finance.event_kind e USING (event_kind)
      JOIN finance.change_kind k ON k.kind = u.kind
     WHERE NOT (u.kind = ANY (e.allowed_changes)) OR NOT (u.cols <@ k.updatable_cols);
    IF v_bad IS NOT NULL THEN
        RAISE EXCEPTION 'column lists outside the registry: %', v_bad;
    END IF;
    SELECT string_agg(k.kind || '/' || c, ', ') INTO v_bad
      FROM finance.change_kind k CROSS JOIN LATERAL unnest(k.updatable_cols) c
     WHERE k.op = 'update'
       AND NOT EXISTS (SELECT 1 FROM finance.event_update_cols u WHERE u.kind = k.kind AND c = ANY (u.cols));
    IF v_bad IS NOT NULL THEN
        RAISE EXCEPTION 'updatable columns no event kind writes: %', v_bad;
    END IF;
END $$;

-- ---------------------------------------------------------------------------------------------
-- Helpers over an event document. commit_event decodes the lines of an event once and hands the
-- checks the document with two derived members: `_lines` (every line with its entry's kind and
-- reservation) and `_kinds` (the entry kinds present).
-- ---------------------------------------------------------------------------------------------
CREATE FUNCTION finance.event_changes(p jsonb, p_kind text) RETURNS SETOF jsonb
LANGUAGE sql IMMUTABLE
AS $$ SELECT c FROM jsonb_array_elements(coalesce(p -> 'changes', '[]'::jsonb)) c WHERE c ->> 'kind' = p_kind $$;

CREATE FUNCTION finance.event_has_kind(p jsonb, p_kinds text[]) RETURNS boolean
LANGUAGE sql IMMUTABLE
AS $$ SELECT coalesce(p -> '_kinds', '[]'::jsonb) ?| p_kinds $$;

CREATE FUNCTION finance.event_lines(p jsonb)
RETURNS TABLE (entry_ix integer, entry_kind text, entry_reservation bytea, code integer, currency text,
               payer bytea, lot bytea, allocation bytea, provider text, processor text, loss_class text,
               jurisdiction text, promo_source text, amount bigint)
LANGUAGE sql IMMUTABLE AS $$
    SELECT r.entry_ix, r.entry_kind, decode(r.entry_reservation, 'hex'), r.code, r.currency,
           decode(r.payer, 'hex'), decode(r.lot, 'hex'), decode(r.allocation, 'hex'), r.provider,
           r.processor, r.loss_class, r.jurisdiction, r.promo_source, r.amount
      FROM jsonb_to_recordset(coalesce(p -> '_lines', '[]'::jsonb)) AS r(entry_ix integer, entry_kind text,
           entry_reservation text, code integer, currency text, payer text, lot text, allocation text,
           provider text, processor text, loss_class text, jurisdiction text, promo_source text, amount bigint)
$$;

-- Records the rows a flow locked, in one statement.
CREATE FUNCTION finance.note_locks(p_table text, p_pks text[]) RETURNS void
LANGUAGE sql VOLATILE
AS $$ INSERT INTO finance.flow_lock (txid, table_name, pk)
      SELECT txid_current(), p_table, pk FROM unnest(p_pks) AS pk ON CONFLICT DO NOTHING $$;

-- The primary key of a change row as the lock set records it: hex ids and text, joined by '|'.
CREATE FUNCTION finance.change_pk(p_row jsonb, p_cols text[]) RETURNS text
LANGUAGE sql IMMUTABLE AS $$
    SELECT string_agg(regexp_replace(p_row ->> c, '^\\x', ''), '|' ORDER BY ord)
      FROM unnest(p_cols) WITH ORDINALITY AS u(c, ord) $$;

-- ---------------------------------------------------------------------------------------------
-- One static function per insert and update kind, generated from the registry, so a change costs a
-- cached plan and not a parse (17.6). An insert names exactly the kind's `columns`; a column with a
-- default takes it when the row leaves it out. An update sets only the columns its row carries,
-- compare-and-set on the revision it read.
-- ---------------------------------------------------------------------------------------------
DO $generate$
DECLARE
    k record;
    v_cols text;
    v_exprs text;
    v_sets text;
    v_where text;
    v_apply text := '';
    v_update text := '';
BEGIN
    FOR k IN SELECT * FROM finance.change_kind WHERE op IN ('insert', 'update') ORDER BY kind LOOP
        IF k.op = 'insert' THEN
            SELECT string_agg(format('%I', u.c), ', ' ORDER BY u.ord),
                   string_agg(CASE WHEN d.adbin IS NULL THEN format('r.%I', u.c)
                                   ELSE format('coalesce(r.%I, %s)', u.c, pg_get_expr(d.adbin, d.adrelid)) END, ', ' ORDER BY u.ord)
              INTO v_cols, v_exprs
              FROM unnest(k.columns) WITH ORDINALITY AS u(c, ord)
              JOIN pg_attribute a ON a.attrelid = format('finance.%I', k.table_name)::regclass AND a.attname = u.c
                                 AND a.attidentity = '' AND a.attgenerated = ''
              LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum;
            EXECUTE format($f$CREATE FUNCTION finance.apply_%1$I(p_row jsonb) RETURNS void
                LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $body$
                BEGIN
                    INSERT INTO finance.%2$I (%3$s) SELECT %4$s FROM jsonb_populate_record(NULL::finance.%2$I, p_row) r;
                END $body$$f$, k.kind, k.table_name, v_cols, v_exprs);
            v_apply := v_apply || format(E'        WHEN %L THEN PERFORM finance.apply_%I(p_row);\n', k.kind, k.kind);
        ELSE
            SELECT string_agg(format('%1$I = CASE WHEN p_row ? %2$L THEN r.%1$I ELSE t.%1$I END', u.c, u.c), ', ' ORDER BY u.ord)
              INTO v_sets FROM unnest(k.updatable_cols) WITH ORDINALITY AS u(c, ord);
            SELECT string_agg(format('t.%1$I = r.%1$I', c), ' AND ') INTO v_where FROM unnest(k.pk_cols) c;
            EXECUTE format($f$CREATE FUNCTION finance.apply_%1$I(p_row jsonb, p_expected bigint) RETURNS bigint
                LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $body$
                DECLARE v_count bigint;
                BEGIN
                    UPDATE finance.%2$I t SET %3$s, revision = t.revision + 1
                      FROM jsonb_populate_record(NULL::finance.%2$I, p_row) r WHERE %4$s AND t.revision = p_expected;
                    GET DIAGNOSTICS v_count = ROW_COUNT;
                    RETURN v_count;
                END $body$$f$, k.kind, k.table_name, v_sets, v_where);
            v_update := v_update || format(E'        WHEN %L THEN RETURN finance.apply_%I(p_row, p_expected);\n', k.kind, k.kind);
        END IF;
    END LOOP;
    EXECUTE format($f$CREATE FUNCTION finance.apply_insert(p_kind text, p_row jsonb) RETURNS void
        LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $body$
        BEGIN
            CASE p_kind
%s                ELSE RAISE EXCEPTION 'no insert function for change kind %%', p_kind USING ERRCODE = 'FN003';
            END CASE;
        END $body$$f$, replace(v_apply, E'        WHEN', E'                WHEN'));
    EXECUTE format($f$CREATE FUNCTION finance.apply_update(p_kind text, p_row jsonb, p_expected bigint) RETURNS bigint
        LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $body$
        BEGIN
            CASE p_kind
%s                ELSE RAISE EXCEPTION 'no update function for change kind %%', p_kind USING ERRCODE = 'FN003';
            END CASE;
        END $body$$f$, replace(v_update, E'        WHEN', E'                WHEN'));
END $generate$;

-- ---------------------------------------------------------------------------------------------
-- apply_change: a typed after-image, insert or compare-and-set update (17.6). A row may carry only
-- the columns its change kind lists: an insert's `columns`, an update's key and `updatable_cols`.
-- ---------------------------------------------------------------------------------------------
CREATE FUNCTION finance.apply_change(p_event_kind text, p_change jsonb) RETURNS void
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_ck finance.change_kind%ROWTYPE;
    v_row jsonb := p_change -> 'row';
    v_expected bigint;
    v_count bigint;
    v_pk text;
    v_extra text;
    v_allowed text[];
BEGIN
    SELECT * INTO v_ck FROM finance.change_kind WHERE kind = p_change ->> 'kind';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'unknown change kind %', p_change ->> 'kind' USING ERRCODE = 'FN003';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM finance.event_kind
                    WHERE event_kind = p_event_kind AND v_ck.kind = ANY (allowed_changes)) THEN
        RAISE EXCEPTION 'event kind % may not carry change %', p_event_kind, v_ck.kind USING ERRCODE = 'FN003';
    END IF;
    IF v_ck.op = 'update' THEN
        SELECT v_ck.pk_cols || u.cols INTO v_allowed FROM finance.event_update_cols u
         WHERE u.event_kind = p_event_kind AND u.kind = v_ck.kind;
        IF v_allowed IS NULL THEN
            RAISE EXCEPTION 'event kind % has no column list for %', p_event_kind, v_ck.kind USING ERRCODE = 'FN003';
        END IF;
    ELSE
        v_allowed := v_ck.columns;
    END IF;
    SELECT string_agg(k, ', ') INTO v_extra FROM jsonb_object_keys(v_row) k WHERE k <> ALL (v_allowed);
    IF v_extra IS NOT NULL THEN
        RAISE EXCEPTION 'change % may not set column(s) %', v_ck.kind, v_extra USING ERRCODE = 'FN003';
    END IF;

    IF v_ck.op = 'key' THEN
        INSERT INTO finance.account_key (code, currency, payer_account_id, lot_id, allocation_id,
                                         provider, processor, loss_class, jurisdiction, promo_source)
        VALUES ((v_row ->> 'code')::smallint, v_row ->> 'currency',
                finance.rowbytes(v_row ->> 'payer'), finance.rowbytes(v_row ->> 'lot'),
                finance.rowbytes(v_row ->> 'allocation'), v_row ->> 'provider', v_row ->> 'processor',
                v_row ->> 'loss_class', v_row ->> 'jurisdiction', v_row ->> 'promo_source')
        ON CONFLICT DO NOTHING;
        RETURN;
    END IF;

    IF v_ck.op = 'insert' THEN
        PERFORM finance.apply_insert(v_ck.kind, v_row);
        RETURN;
    END IF;

    IF v_ck.op = 'upsert' THEN
        -- work_item: re-deriving an item is idempotent on (kind, ref_id); the due time follows the latest write.
        INSERT INTO finance.work_item (kind, ref_id, due_at_ms)
        VALUES (v_row ->> 'kind', finance.rowbytes(v_row ->> 'ref_id'), (v_row ->> 'due_at_ms')::bigint)
        ON CONFLICT (kind, ref_id) DO UPDATE SET due_at_ms = EXCLUDED.due_at_ms;
        RETURN;
    END IF;
    IF v_ck.op = 'delete' THEN
        DELETE FROM finance.work_item WHERE kind = v_row ->> 'kind' AND ref_id = finance.rowbytes(v_row ->> 'ref_id');
        RETURN;
    END IF;

    -- update: compare-and-set on the expected revision, and the row must be in the flow's lock set.
    v_pk := finance.change_pk(v_row, v_ck.pk_cols);
    v_expected := (p_change ->> 'expected_revision')::bigint;
    IF v_expected IS NULL THEN
        RAISE EXCEPTION 'update of % carries no expected_revision', v_ck.table_name USING ERRCODE = 'FN001';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM finance.flow_lock WHERE txid = txid_current()
                    AND table_name = v_ck.table_name AND pk = v_pk) THEN
        RAISE EXCEPTION 'row %/% is not in the lock set of this transaction', v_ck.table_name, v_pk
            USING ERRCODE = 'FN002';
    END IF;
    v_count := finance.apply_update(v_ck.kind, v_row, v_expected);
    IF v_count <> 1 THEN
        RAISE EXCEPTION 'stale change: % % is no longer at revision %', v_ck.table_name, v_pk, v_expected
            USING ERRCODE = 'FN001';
    END IF;
END $$;

-- The journal copy keeps only the columns each change kind lists (22.1) and no unjournaled change.
CREATE FUNCTION finance.payload_for_journal(p jsonb) RETURNS jsonb
LANGUAGE sql STABLE AS $$
    SELECT jsonb_build_object(
        'event_id', p -> 'event_id', 'event_kind', p -> 'event_kind', 'payload_schema', p -> 'payload_schema',
        'entries', coalesce(p -> 'entries', '[]'::jsonb),
        'changes', coalesce((
            SELECT jsonb_agg(jsonb_build_object('kind', c.val ->> 'kind',
                    'row', (SELECT coalesce(jsonb_object_agg(e.key, e.value), '{}'::jsonb)
                              FROM jsonb_each(c.val -> 'row') e WHERE e.key = ANY (k.journal_cols)))
                    || CASE WHEN c.val ? 'expected_revision'
                            THEN jsonb_build_object('expected_revision', c.val -> 'expected_revision')
                            ELSE '{}'::jsonb END ORDER BY c.ord)
              FROM jsonb_array_elements(coalesce(p -> 'changes', '[]'::jsonb)) WITH ORDINALITY AS c(val, ord)
              JOIN finance.change_kind k ON k.kind = c.val ->> 'kind' WHERE NOT k.unjournaled), '[]'::jsonb))
$$;

-- ---------------------------------------------------------------------------------------------
-- Per-kind equations (17.6 E1-E10). Each is its own function so a mutation can replace it.
-- ---------------------------------------------------------------------------------------------
CREATE FUNCTION finance.eq_fail(p_id text, p_detail text) RETURNS void
LANGUAGE plpgsql IMMUTABLE SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    RAISE EXCEPTION 'equation % failed: %', p_id, p_detail USING ERRCODE = 'FN004';
END $$;

-- E9: the session's running charged total moves only through its own change kind, and only in an
-- event that carries a settlement or meter charge entry (E1 then ties the move to the usage event).
CREATE FUNCTION finance.eq_e9(p jsonb) RETURNS void
LANGUAGE plpgsql STABLE SET search_path = pg_catalog, finance, pg_temp AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM jsonb_array_elements(coalesce(p -> 'changes', '[]'::jsonb)) c
                WHERE c ->> 'kind' IN ('session_funding_update', 'session_charged_total_update')
                  AND c -> 'row' ? 'charged_total_micro_dollars'
                  AND (c ->> 'kind' <> 'session_charged_total_update'
                       OR NOT finance.event_has_kind(p, ARRAY['settlement', 'meter_charge']))) THEN
        PERFORM finance.eq_fail('E9', 'the session charged total moves only with a settlement or meter charge entry');
    END IF;
END $$;

-- E6: a reservation's reserved credits equal its retail maximum and its shares; the exposure lines
-- equal the attempt's supplier exposure.
CREATE FUNCTION finance.eq_e6(p jsonb) RETURNS void
LANGUAGE plpgsql STABLE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_reserved bigint; v_maximum bigint; v_held bigint; v_exposure bigint; v_attempt_exposure bigint;
BEGIN
    IF NOT finance.event_has_kind(p, ARRAY['reservation', 'meter_hold']) THEN
        RETURN;
    END IF;
    SELECT -coalesce(sum(amount), 0) INTO v_reserved FROM finance.event_lines(p)
     WHERE entry_kind IN ('reservation', 'meter_hold') AND code IN (2130, 2230);
    SELECT coalesce(sum((c -> 'row' ->> 'retail_maximum_micro_dollars')::bigint), 0) INTO v_maximum
      FROM finance.event_changes(p, 'reservation_insert') c;
    SELECT coalesce(sum((c -> 'row' ->> 'held_micro_dollars')::bigint), 0) INTO v_held
      FROM finance.event_changes(p, 'reservation_share_insert') c;
    IF v_reserved <> v_maximum OR v_held <> v_maximum THEN
        PERFORM finance.eq_fail('E6', format('reserved %s, shares %s, retail maximum %s', v_reserved, v_held, v_maximum));
    END IF;
    SELECT coalesce(sum(amount), 0) INTO v_exposure FROM finance.event_lines(p) WHERE code = 9100;
    SELECT coalesce(sum((c -> 'row' ->> 'supplier_exposure_micro_dollars')::bigint), 0) INTO v_attempt_exposure
      FROM finance.event_changes(p, 'provider_attempt_insert') c;
    IF v_exposure <> v_attempt_exposure THEN
        PERFORM finance.eq_fail('E6', format('exposure lines %s, attempt exposure %s', v_exposure, v_attempt_exposure));
    END IF;
END $$;

-- The deltas of the reservation shares an event updates, against the rows as they are before the
-- event's phase 2 changes run.
CREATE FUNCTION finance.event_share_deltas(p jsonb)
RETURNS TABLE (reservation_id bytea, allocation_id bytea, lot_id bytea, class text, payer bytea,
               d_charged bigint, d_released bigint, d_reversed bigint)
LANGUAGE sql STABLE SET search_path = pg_catalog, finance, pg_temp AS $$
    SELECT s.reservation_id, s.allocation_id, s.lot_id, s.class, a.payer_account_id,
           coalesce((c -> 'row' ->> 'charged_micro_dollars')::bigint, s.charged_micro_dollars) - s.charged_micro_dollars,
           coalesce((c -> 'row' ->> 'released_micro_dollars')::bigint, s.released_micro_dollars) - s.released_micro_dollars,
           coalesce((c -> 'row' ->> 'reversed_micro_dollars')::bigint, s.reversed_micro_dollars) - s.reversed_micro_dollars
      FROM finance.event_changes(p, 'reservation_share_update') c
      JOIN finance.reservation_share s
        ON s.reservation_id = finance.rowbytes((c -> 'row' ->> 'reservation_id'))
       AND s.allocation_id = finance.rowbytes((c -> 'row' ->> 'allocation_id'))
       AND s.lot_id = finance.rowbytes((c -> 'row' ->> 'lot_id'))
      JOIN finance.allocation a ON a.allocation_id = s.allocation_id
$$;

-- E2: the reserved debit of each share's key equals the change in its charged plus released.
CREATE FUNCTION finance.eq_e2(p jsonb) RETURNS void
LANGUAGE plpgsql STABLE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    d record; v_lines bigint; v_total_lines bigint; v_total_deltas bigint;
BEGIN
    IF NOT finance.event_has_kind(p, ARRAY['settlement', 'release', 'company_loss', 'meter_charge']) THEN
        RETURN;
    END IF;
    FOR d IN SELECT * FROM finance.event_share_deltas(p) LOOP
        SELECT coalesce(sum(amount), 0) INTO v_lines FROM finance.event_lines(p) l
         WHERE l.code = CASE d.class WHEN 'purchased' THEN 2130 ELSE 2230 END
           AND l.allocation = d.allocation_id AND l.lot = d.lot_id;
        IF v_lines <> d.d_charged + d.d_released THEN
            PERFORM finance.eq_fail('E2', format('reserved debit %s, charged+released change %s', v_lines, d.d_charged + d.d_released));
        END IF;
    END LOOP;
    SELECT coalesce(sum(amount), 0) INTO v_total_lines FROM finance.event_lines(p)
     WHERE code IN (2130, 2230) AND entry_kind IN ('settlement', 'release', 'company_loss', 'meter_charge');
    SELECT coalesce(sum(d_charged + d_released), 0) INTO v_total_deltas FROM finance.event_share_deltas(p);
    IF v_total_lines <> v_total_deltas THEN
        PERFORM finance.eq_fail('E2', format('reserved debits %s, share changes %s', v_total_lines, v_total_deltas));
    END IF;
END $$;

-- E3: membership. Every line whose key carries a payer, lot or allocation belongs to the shares
-- of the reservation the entry is anchored to (or, for an allocation entry, to the allocation).
CREATE FUNCTION finance.eq_e3(p jsonb) RETURNS void
LANGUAGE plpgsql STABLE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    l record;
BEGIN
    FOR l IN SELECT * FROM finance.event_lines(p)
              WHERE entry_kind IN ('settlement', 'release', 'company_loss', 'meter_charge', 'chargeback_recovery')
                AND (allocation IS NOT NULL OR lot IS NOT NULL OR payer IS NOT NULL) LOOP
        IF l.allocation IS NOT NULL AND NOT EXISTS (
            SELECT 1 FROM finance.reservation_share s
             WHERE s.reservation_id = l.entry_reservation AND s.allocation_id = l.allocation AND s.lot_id = l.lot) THEN
            PERFORM finance.eq_fail('E3', 'a line names an allocation key that is not a share of the reservation');
        END IF;
        IF l.allocation IS NULL AND l.lot IS NOT NULL AND NOT EXISTS (
            SELECT 1 FROM finance.reservation_share s
             WHERE s.reservation_id = l.entry_reservation AND s.lot_id = l.lot) THEN
            PERFORM finance.eq_fail('E3', 'a line names a lot that is not a lot of the reservation');
        END IF;
        IF l.payer IS NOT NULL AND NOT EXISTS (
            SELECT 1 FROM finance.reservation_share s JOIN finance.allocation a ON a.allocation_id = s.allocation_id
             WHERE s.reservation_id = l.entry_reservation AND s.lot_id = l.lot AND a.payer_account_id = l.payer) THEN
            PERFORM finance.eq_fail('E3', 'a line names a payer that is not a payer of the reservation');
        END IF;
    END LOOP;
    FOR l IN SELECT * FROM finance.event_lines(p)
              WHERE entry_kind IN ('reservation', 'meter_hold') AND allocation IS NOT NULL LOOP
        IF NOT EXISTS (SELECT 1 FROM finance.event_changes(p, 'reservation_share_insert') c
                        WHERE finance.rowbytes((c -> 'row' ->> 'allocation_id')) = l.allocation
                          AND finance.rowbytes((c -> 'row' ->> 'lot_id')) = l.lot) THEN
            PERFORM finance.eq_fail('E3', 'a reservation line names a key that has no share');
        END IF;
    END LOOP;
    FOR l IN SELECT * FROM finance.event_lines(p) WHERE entry_kind = 'customer_credit' AND lot IS NOT NULL LOOP
        IF NOT EXISTS (
            SELECT 1 FROM finance.reservation_share s
              JOIN finance.provider_attempt a ON a.reservation_id = s.reservation_id
              JOIN finance.usage_event u ON u.attempt_id = a.attempt_id
             WHERE s.lot_id = l.lot AND u.usage_event_id = finance.rowbytes((
                   (SELECT c -> 'row' ->> 'usage_event_id' FROM finance.event_changes(p, 'usage_event_update') c LIMIT 1)))) THEN
            PERFORM finance.eq_fail('E3', 'a credit line names a lot the charge was not drawn from');
        END IF;
    END LOOP;
END $$;

-- E1: the revenue and consumption lines equal the usage event; the cost lines equal its cost split.
CREATE FUNCTION finance.eq_e1(p jsonb) RETURNS void
LANGUAGE plpgsql STABLE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_retail bigint; v_supplier bigint; v_purchased bigint; v_promotional bigint;
    v_revenue bigint; v_offset bigint; v_cost_p bigint; v_cost_q bigint; v_payable bigint;
    v_charged bigint; v_counter bigint;
BEGIN
    IF NOT finance.event_has_kind(p, ARRAY['settlement', 'meter_charge']) THEN
        RETURN;
    END IF;
    SELECT (c -> 'row' ->> 'retail_micro_dollars')::bigint, (c -> 'row' ->> 'supplier_micro_dollars')::bigint,
           coalesce((c -> 'row' ->> 'supplier_purchased_micro_dollars')::bigint, 0),
           coalesce((c -> 'row' ->> 'supplier_promotional_micro_dollars')::bigint, 0)
      INTO v_retail, v_supplier, v_purchased, v_promotional
      FROM finance.event_changes(p, 'usage_event_insert') c LIMIT 1;
    IF v_retail IS NULL THEN
        PERFORM finance.eq_fail('E1', 'a settlement carries no usage event');
    END IF;
    SELECT -coalesce(sum(amount) FILTER (WHERE code IN (4110, 4100)), 0),
           -coalesce(sum(amount) FILTER (WHERE code = 3910), 0),
           coalesce(sum(amount) FILTER (WHERE code = 5100), 0),
           coalesce(sum(amount) FILTER (WHERE code = 5110), 0),
           -coalesce(sum(amount) FILTER (WHERE code IN (2300, 1200)), 0)
      INTO v_revenue, v_offset, v_cost_p, v_cost_q, v_payable
      FROM finance.event_lines(p) WHERE entry_kind IN ('settlement', 'meter_charge');
    IF v_revenue + v_offset <> v_retail THEN
        PERFORM finance.eq_fail('E1', format('revenue %s + offset %s differ from the usage event retail %s', v_revenue, v_offset, v_retail));
    END IF;
    IF v_cost_p <> v_purchased OR v_cost_q <> v_promotional OR v_cost_p + v_cost_q <> v_supplier OR v_payable <> v_supplier THEN
        PERFORM finance.eq_fail('E1', format('cost lines %s/%s, payable %s differ from the supplier cost %s', v_cost_p, v_cost_q, v_payable, v_supplier));
    END IF;
    SELECT coalesce(sum(d_charged), 0) INTO v_charged FROM finance.event_share_deltas(p);
    IF v_charged <> v_retail THEN
        PERFORM finance.eq_fail('E1', format('shares charged %s, usage event retail %s', v_charged, v_retail));
    END IF;
    -- The session's running charged total moves by exactly the usage event's retail charge.
    SELECT coalesce(sum((c -> 'row' ->> 'charged_total_micro_dollars')::bigint - s.charged_total_micro_dollars), 0)
      INTO v_counter
      FROM finance.event_changes(p, 'session_charged_total_update') c
      JOIN finance.session_funding s ON s.session_id = finance.rowbytes(c -> 'row' ->> 'session_id');
    IF v_counter <> v_retail THEN
        PERFORM finance.eq_fail('E1', format('session charged total moves by %s, usage event retail %s', v_counter, v_retail));
    END IF;
    IF EXISTS (SELECT 1 FROM finance.event_changes(p, 'session_charged_total_update') c
                WHERE finance.rowbytes(c -> 'row' ->> 'session_id') IS DISTINCT FROM
                      (SELECT finance.rowbytes(u -> 'row' ->> 'session_id')
                         FROM finance.event_changes(p, 'usage_event_insert') u LIMIT 1)) THEN
        PERFORM finance.eq_fail('E1', 'the session charged total moves for a session other than the usage event');
    END IF;
    IF v_revenue <> (SELECT coalesce(sum(d_charged), 0) FROM finance.event_share_deltas(p) WHERE class = 'purchased')
       OR v_offset <> (SELECT coalesce(sum(d_charged), 0) FROM finance.event_share_deltas(p) WHERE class = 'promotional') THEN
        PERFORM finance.eq_fail('E1', 'revenue and offset do not match the purchased and promotional charges');
    END IF;
END $$;

-- E4: each released amount returns to the key the lot state and allocation status name (14.4).
CREATE FUNCTION finance.eq_e4(p jsonb) RETURNS void
LANGUAGE plpgsql STABLE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    d record; v_state text; v_status text; v_code integer; v_processor text; v_lines bigint; v_total bigint;
BEGIN
    IF NOT finance.event_has_kind(p, ARRAY['settlement', 'release', 'meter_charge', 'company_loss']) THEN
        RETURN;
    END IF;
    FOR d IN SELECT * FROM finance.event_share_deltas(p) WHERE d_released > 0 LOOP
        SELECT l.state, l.processor INTO v_state, v_processor FROM finance.funding_lot l WHERE l.lot_id = d.lot_id;
        SELECT status INTO v_status FROM finance.allocation WHERE allocation_id = d.allocation_id;
        IF d.class = 'promotional' THEN
            v_code := CASE WHEN v_status IN ('active', 'consent_incompatible') THEN 2220 ELSE 2210 END;
        ELSIF v_state = 'frozen' THEN v_code := 2140;
        ELSIF v_state = 'charged_back' THEN v_code := 5500;
        ELSIF v_status IN ('active', 'consent_incompatible') THEN v_code := 2120;
        ELSE v_code := 2110;
        END IF;
        SELECT -coalesce(sum(amount), 0) INTO v_lines FROM finance.event_lines(p) l
         WHERE l.code = v_code
           AND ((v_code = 5500 AND l.processor IS NOT DISTINCT FROM v_processor)
                OR (v_code = 2140 AND l.lot = d.lot_id)
                OR (v_code IN (2120, 2220) AND l.lot = d.lot_id AND l.allocation = d.allocation_id)
                OR (v_code IN (2110, 2210) AND l.lot = d.lot_id AND l.payer = d.payer));
        IF v_lines <> d.d_released THEN
            PERFORM finance.eq_fail('E4', format('released %s returned %s to code %s (lot %s, allocation %s)',
                d.d_released, v_lines, v_code, v_state, v_status));
        END IF;
    END LOOP;
    SELECT -coalesce(sum(amount), 0) INTO v_total FROM finance.event_lines(p)
     WHERE code IN (2120, 2110, 2140, 5500, 2220, 2210)
       AND entry_kind IN ('settlement', 'release', 'meter_charge', 'company_loss');
    IF v_total <> (SELECT coalesce(sum(d_released), 0) FROM finance.event_share_deltas(p)) THEN
        PERFORM finance.eq_fail('E4', 'return lines do not equal the released amounts');
    END IF;
END $$;

-- E5: a credit equals the change in the usage event's reversed amount and the shares' reversed.
CREATE FUNCTION finance.eq_e5(p jsonb) RETURNS void
LANGUAGE plpgsql STABLE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_new bigint; v_old bigint; v_debits bigint; v_shares bigint; v_credits bigint; d record; v_lines bigint;
    v_state text; v_id bytea;
BEGIN
    IF NOT finance.event_has_kind(p, ARRAY['customer_credit']) THEN
        RETURN;
    END IF;
    -- TODO(FIN-03): the TechnicalFailure reclassification (12.5) is not built yet.
    IF EXISTS (SELECT 1 FROM finance.event_lines(p)
                WHERE entry_kind = 'customer_credit' AND code IN (5200, 5100, 5110, 5300, 4200)) THEN
        PERFORM finance.eq_fail('E5', 'reclassification lines are not supported in round 1');
    END IF;
    SELECT (c -> 'row' ->> 'reversed_micro_dollars')::bigint,
           finance.rowbytes((c -> 'row' ->> 'usage_event_id'))
      INTO v_new, v_id FROM finance.event_changes(p, 'usage_event_update') c LIMIT 1;
    IF v_new IS NULL THEN
        PERFORM finance.eq_fail('E5', 'a credit carries no usage event update');
    END IF;
    SELECT reversed_micro_dollars INTO v_old FROM finance.usage_event WHERE usage_event_id = v_id;
    SELECT coalesce(sum(amount) FILTER (WHERE code IN (4110, 4100, 3910)), 0),
           -coalesce(sum(amount) FILTER (WHERE code IN (2110, 2140, 2210)), 0)
      INTO v_debits, v_credits FROM finance.event_lines(p) WHERE entry_kind = 'customer_credit';
    SELECT coalesce(sum(d_reversed), 0) INTO v_shares FROM finance.event_share_deltas(p);
    IF v_debits <> v_new - v_old OR v_credits <> v_new - v_old OR v_shares <> v_new - v_old THEN
        PERFORM finance.eq_fail('E5', format('credit debits %s credits %s shares %s, reversed change %s',
            v_debits, v_credits, v_shares, v_new - v_old));
    END IF;
    FOR d IN SELECT * FROM finance.event_share_deltas(p) WHERE d_reversed > 0 LOOP
        SELECT state INTO v_state FROM finance.funding_lot WHERE lot_id = d.lot_id;
        IF v_state NOT IN ('active', 'frozen') THEN
            PERFORM finance.eq_fail('E5', 'a credit may not return to a lot that is ' || v_state);
        END IF;
        SELECT -coalesce(sum(amount), 0) INTO v_lines FROM finance.event_lines(p) l
         WHERE l.lot = d.lot_id AND l.payer IS NOT DISTINCT FROM CASE WHEN v_state = 'frozen' AND d.class = 'purchased' THEN NULL ELSE d.payer END
           AND l.code = CASE WHEN d.class = 'promotional' THEN 2210 WHEN v_state = 'frozen' THEN 2140 ELSE 2110 END;
        IF v_lines <> d.d_reversed THEN
            PERFORM finance.eq_fail('E5', format('share reversed %s but %s returned to its key', d.d_reversed, v_lines));
        END IF;
    END LOOP;
END $$;

-- E7: allocation entries equal the change in the allocation's drawn or returned amount.
CREATE FUNCTION finance.eq_e7(p jsonb) RETURNS void
LANGUAGE plpgsql STABLE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_credit bigint; v_debit bigint; v_drawn bigint; v_returned bigint;
    v_status text; v_allocation bytea; v_old_authorized bigint; v_old_returned bigint;
    v_new_returned bigint;
    v_allocated bigint; v_change_count integer;
BEGIN
    IF finance.event_has_kind(p, ARRAY['allocation']) THEN
        SELECT -coalesce(sum(amount) FILTER (WHERE code IN (2120, 2220)), 0),
               coalesce(sum(amount) FILTER (WHERE code IN (2110, 2210)), 0)
          INTO v_credit, v_debit FROM finance.event_lines(p) WHERE entry_kind = 'allocation';
        -- An inserted allocation is already in place (phase 1: its keys reference it), so its old
        -- drawn amount is zero by definition; an updated one is compared with the stored row.
        SELECT coalesce((SELECT sum((c -> 'row' ->> 'drawn_micro_dollars')::bigint)
                           FROM finance.event_changes(p, 'allocation_insert') c), 0)
             + coalesce((SELECT sum((c -> 'row' ->> 'drawn_micro_dollars')::bigint - a.drawn_micro_dollars)
                           FROM finance.event_changes(p, 'allocation_update') c
                           JOIN finance.allocation a ON a.allocation_id = finance.rowbytes((c -> 'row' ->> 'allocation_id'))), 0)
          INTO v_drawn;
        IF v_credit <> v_drawn OR v_debit <> v_drawn THEN
            PERFORM finance.eq_fail('E7', format('allocated %s from available %s, drawn change %s', v_credit, v_debit, v_drawn));
        END IF;
    END IF;
    IF finance.event_has_kind(p, ARRAY['allocation_release']) THEN
        SELECT coalesce(sum(amount) FILTER (WHERE code IN (2120, 2220)), 0) INTO v_debit
          FROM finance.event_lines(p) WHERE entry_kind = 'allocation_release';
        SELECT coalesce(sum((c -> 'row' ->> 'returned_micro_dollars')::bigint
                  - (SELECT a.returned_micro_dollars FROM finance.allocation a
                      WHERE a.allocation_id = finance.rowbytes((c -> 'row' ->> 'allocation_id')))), 0)
          INTO v_returned FROM finance.event_changes(p, 'allocation_update') c;
        IF v_debit <> v_returned THEN
            PERFORM finance.eq_fail('E7', format('released %s, returned change %s', v_debit, v_returned));
        END IF;
        IF EXISTS (SELECT 1 FROM finance.event_changes(p, 'operation_receipt_insert') c
                    WHERE c -> 'row' ->> 'namespace' = 'withdrawal') THEN
            SELECT count(*) INTO v_change_count FROM finance.event_changes(p, 'allocation_update');
            IF v_change_count <> 1 THEN
                PERFORM finance.eq_fail('E7', 'a withdrawal must close exactly one allocation');
            END IF;
            SELECT c -> 'row' ->> 'status', finance.rowbytes(c -> 'row' ->> 'allocation_id'),
                   (c -> 'row' ->> 'authorized_micro_dollars')::bigint,
                   (c -> 'row' ->> 'returned_micro_dollars')::bigint
              INTO v_status, v_allocation, v_credit, v_new_returned
              FROM finance.event_changes(p, 'allocation_update') c WHERE c -> 'row' ? 'status';
            IF v_status <> 'withdrawn' THEN
                PERFORM finance.eq_fail('E7', 'a withdrawal must mark its allocation withdrawn');
            END IF;
            SELECT authorized_micro_dollars, returned_micro_dollars
              INTO v_old_authorized, v_old_returned FROM finance.allocation WHERE allocation_id = v_allocation;
            SELECT coalesce(sum(b.natural_micro_dollars), 0) INTO v_allocated
              FROM finance.account_key k JOIN finance.account_balance b USING (key_id)
             WHERE k.allocation_id = v_allocation AND k.code IN (2120, 2220);
            IF v_allocated <> 0 OR v_credit <> v_old_authorized - (v_new_returned - v_old_returned) THEN
                PERFORM finance.eq_fail('E7', 'a withdrawal must return all free funds and reduce authorization by that amount');
            END IF;
        END IF;
    END IF;
END $$;

-- E8: a purchase credits the lot's original amount; a trial grant credits the grant.
CREATE FUNCTION finance.eq_e8(p jsonb) RETURNS void
LANGUAGE plpgsql STABLE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_original bigint; v_tax bigint; v_credit bigint; v_tax_lines bigint; v_debit bigint;
BEGIN
    IF finance.event_has_kind(p, ARRAY['purchase']) THEN
        SELECT coalesce(sum((c -> 'row' ->> 'original_micro_dollars')::bigint), 0),
               coalesce(sum((c -> 'row' ->> 'tax_micro_dollars')::bigint), 0)
          INTO v_original, v_tax FROM finance.event_changes(p, 'funding_lot_insert') c;
        SELECT -coalesce(sum(amount) FILTER (WHERE code = 2110), 0), -coalesce(sum(amount) FILTER (WHERE code = 2400), 0),
               coalesce(sum(amount) FILTER (WHERE code = 1100), 0)
          INTO v_credit, v_tax_lines, v_debit FROM finance.event_lines(p) WHERE entry_kind = 'purchase';
        IF v_credit <> v_original OR v_tax_lines <> v_tax OR v_debit <> v_original + v_tax THEN
            PERFORM finance.eq_fail('E8', format('purchase credits %s (tax %s), lot original %s (tax %s)', v_credit, v_tax_lines, v_original, v_tax));
        END IF;
    END IF;
    IF finance.event_has_kind(p, ARRAY['trial_grant', 'goodwill_grant']) THEN
        SELECT coalesce(sum((c -> 'row' ->> 'original_micro_dollars')::bigint), 0) INTO v_original
          FROM finance.event_changes(p, 'funding_lot_insert') c;
        SELECT -coalesce(sum(amount) FILTER (WHERE code = 2210), 0), coalesce(sum(amount) FILTER (WHERE code = 3910), 0)
          INTO v_credit, v_debit FROM finance.event_lines(p) WHERE entry_kind IN ('trial_grant', 'goodwill_grant');
        IF v_credit <> v_original OR v_debit <> v_original THEN
            PERFORM finance.eq_fail('E8', format('grant credits %s offsets %s, lot original %s', v_credit, v_debit, v_original));
        END IF;
    END IF;
END $$;

-- The sessions whose running charged total differs from the sum of their usage events' retail.
-- The maintenance worker and the property tests read it; an empty answer is the invariant.
CREATE FUNCTION finance.session_charge_drift()
RETURNS TABLE (session_id bytea, charged_total_micro_dollars bigint, usage_retail_micro_dollars bigint)
LANGUAGE sql STABLE SET search_path = pg_catalog, finance, pg_temp AS $$
    SELECT s.session_id, s.charged_total_micro_dollars, coalesce(sum(u.retail_micro_dollars), 0)::bigint
      FROM finance.session_funding s
      LEFT JOIN finance.usage_event u ON u.session_id = s.session_id
     GROUP BY s.session_id, s.charged_total_micro_dollars
    HAVING s.charged_total_micro_dollars <> coalesce(sum(u.retail_micro_dollars), 0)
$$;

CREATE FUNCTION finance.check_equations(p jsonb) RETURNS void
LANGUAGE plpgsql STABLE SET search_path = pg_catalog, finance, pg_temp AS $$
BEGIN
    PERFORM finance.eq_e1(p);
    PERFORM finance.eq_e2(p);
    PERFORM finance.eq_e3(p);
    PERFORM finance.eq_e4(p);
    PERFORM finance.eq_e5(p);
    PERFORM finance.eq_e6(p);
    PERFORM finance.eq_e7(p);
    PERFORM finance.eq_e8(p);
    PERFORM finance.eq_e9(p);
END $$;


-- ---------------------------------------------------------------------------------------------
-- Anchors: what an entry is about, and what its stream and correlation columns must then be.
-- ---------------------------------------------------------------------------------------------
-- The stream and correlation an entry must carry, from the anchor row (in a table, or inserted by
-- the same event). NULL means the anchor does not exist. An empty object means no expectation.
CREATE FUNCTION finance.anchor_expectation(p_anchor text, p_entry jsonb, p_event jsonb) RETURNS jsonb
LANGUAGE plpgsql STABLE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_reservation bytea := finance.hexbytes(p_entry -> 'reservation_id');
    v_session bytea;
    v_price_version bytea;
    v_campaign bytea;
    v_attempt bytea;
    v_payer bytea;
    v_ref text := p_entry ->> 'external_ref';
BEGIN
    IF p_anchor = 'none' THEN
        RETURN '{}'::jsonb;
    ELSIF p_anchor = 'trial_grant' THEN
        RETURN jsonb_build_object('stream_kind', 'payer', 'stream_id', p_entry -> 'stream_id',
            'campaign_id', NULL, 'session_id', NULL, 'attempt_id', NULL, 'reservation_id', NULL, 'price_version_id', NULL);
    ELSIF p_anchor = 'processor_payment_state' THEN
        SELECT l.payer_account_id INTO v_payer
          FROM finance.processor_payment_state s JOIN finance.funding_lot l ON l.lot_id = s.lot_id
         WHERE s.processor = split_part(v_ref, '|', 1) AND s.payment_ref = substr(v_ref, strpos(v_ref, '|') + 1);
        IF v_payer IS NULL THEN RETURN NULL; END IF;
        RETURN jsonb_build_object('stream_kind', 'payer', 'stream_id', encode(v_payer, 'hex'),
            'campaign_id', NULL, 'session_id', NULL, 'attempt_id', NULL, 'reservation_id', NULL, 'price_version_id', NULL);
    ELSIF p_anchor = 'reservation' THEN
        SELECT r.session_id, r.price_version_id INTO v_session, v_price_version FROM (
            SELECT session_id, price_version_id FROM finance.reservation WHERE reservation_id = v_reservation
            UNION ALL
            SELECT finance.rowbytes(c -> 'row' ->> 'session_id'), finance.rowbytes(c -> 'row' ->> 'price_version_id')
              FROM finance.event_changes(p_event, 'reservation_insert') c
             WHERE finance.rowbytes(c -> 'row' ->> 'reservation_id') = v_reservation) r LIMIT 1;
        IF v_session IS NULL THEN RETURN NULL; END IF;
        SELECT a.attempt_id INTO v_attempt FROM (
            SELECT attempt_id FROM finance.provider_attempt WHERE reservation_id = v_reservation
            UNION ALL
            SELECT finance.rowbytes(c -> 'row' ->> 'attempt_id') FROM finance.event_changes(p_event, 'provider_attempt_insert') c
             WHERE finance.rowbytes(c -> 'row' ->> 'reservation_id') = v_reservation) a LIMIT 1;
        SELECT campaign_id INTO v_campaign FROM finance.session_funding WHERE session_id = v_session;
        RETURN jsonb_build_object('stream_kind', 'session', 'stream_id', encode(v_session, 'hex'),
            'campaign_id', encode(v_campaign, 'hex'), 'session_id', encode(v_session, 'hex'),
            'attempt_id', encode(v_attempt, 'hex'), 'reservation_id', encode(v_reservation, 'hex'),
            'price_version_id', encode(v_price_version, 'hex'));
    ELSIF p_anchor = 'allocation' THEN
        SELECT a.session_id, a.payer_account_id, s.campaign_id, s.price_version_id
          INTO v_session, v_payer, v_campaign, v_price_version
          FROM finance.allocation a JOIN finance.session_funding s ON s.session_id = a.session_id
         WHERE a.session_id = finance.hexbytes(p_entry -> 'session_id')
           AND a.payer_account_id = finance.hexbytes(p_entry -> 'stream_id');
        IF v_session IS NULL THEN RETURN NULL; END IF;
        RETURN jsonb_build_object('stream_kind', 'payer', 'stream_id', encode(v_payer, 'hex'),
            'campaign_id', encode(v_campaign, 'hex'), 'session_id', encode(v_session, 'hex'),
            'attempt_id', NULL, 'reservation_id', NULL, 'price_version_id', encode(v_price_version, 'hex'));
    ELSIF p_anchor = 'usage_event' THEN
        SELECT u.session_id, u.price_version_id, u.attempt_id, a.reservation_id, s.campaign_id
          INTO v_session, v_price_version, v_attempt, v_reservation, v_campaign
          FROM finance.usage_event u JOIN finance.provider_attempt a ON a.attempt_id = u.attempt_id
          JOIN finance.session_funding s ON s.session_id = u.session_id
         WHERE u.attempt_id = finance.hexbytes(p_entry -> 'attempt_id');
        IF v_session IS NULL THEN RETURN NULL; END IF;
        RETURN jsonb_build_object('stream_kind', 'session', 'stream_id', encode(v_session, 'hex'),
            'campaign_id', encode(v_campaign, 'hex'), 'session_id', encode(v_session, 'hex'),
            'attempt_id', encode(v_attempt, 'hex'), 'reservation_id', encode(v_reservation, 'hex'),
            'price_version_id', encode(v_price_version, 'hex'));
    END IF;
    RETURN NULL;
END $$;

-- The database's own business key for the kinds whose key is a function of the account alone, so a
-- caller cannot mint a second grant by choosing another key. NULL means the caller's key stands.
CREATE FUNCTION finance.derived_business_key(p_kind text, p_entry jsonb) RETURNS bytea
LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE WHEN p_kind = 'trial_grant'
                THEN sha256(finance.enc_bytes('trial_grant'::bytea) || finance.enc_bytes(finance.hexbytes(p_entry -> 'stream_id'))) END
$$;

-- A trial grant is the one trial of its account: the same event inserts the account's trial_grant
-- row for the very lot the entry credits, and the account has no other trial lot.
CREATE FUNCTION finance.check_trial_entry(p_entry jsonb, p_event jsonb) RETURNS void
LANGUAGE plpgsql STABLE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_account bytea := finance.hexbytes(p_entry -> 'stream_id');
    v_lot bytea;
BEGIN
    SELECT finance.hexbytes(l -> 'lot') INTO v_lot FROM jsonb_array_elements(p_entry -> 'lines') l
     WHERE (l ->> 'code')::integer = 2210 LIMIT 1;
    IF v_lot IS NULL OR NOT EXISTS (
        SELECT 1 FROM finance.event_changes(p_event, 'trial_grant_insert') c
         WHERE finance.rowbytes(c -> 'row' ->> 'account_id') = v_account
           AND finance.rowbytes(c -> 'row' ->> 'lot_id') = v_lot) THEN
        RAISE EXCEPTION 'a trial grant entry needs the trial_grant row of its account for the lot it credits'
            USING ERRCODE = 'FN003';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM finance.funding_lot WHERE lot_id = v_lot AND payer_account_id = v_account
                    AND class = 'promotional' AND promo_source = 'trial') THEN
        RAISE EXCEPTION 'the lot of a trial grant is the account''s promotional trial lot' USING ERRCODE = 'FN003';
    END IF;
    IF EXISTS (SELECT 1 FROM finance.funding_lot WHERE payer_account_id = v_account
                AND promo_source = 'trial' AND lot_id <> v_lot) THEN
        RAISE EXCEPTION 'the account already has a trial lot' USING ERRCODE = 'FN003';
    END IF;
END $$;

-- ---------------------------------------------------------------------------------------------
-- One entry against its kind: allowlist of kinds, reason, actor, reference, stream, anchor,
-- correlation and posting date (17.6, 17.8).
-- ---------------------------------------------------------------------------------------------
CREATE FUNCTION finance.check_entry(p_entry jsonb, p_event jsonb, p_allowed text[], p_now bigint,
                                    p_period integer) RETURNS finance.entry_kind
LANGUAGE plpgsql STABLE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_rule finance.entry_kind%ROWTYPE;
    v_expect jsonb;
    v_field text;
    v_effective bigint := coalesce((p_entry ->> 'effective_at_ms')::bigint, p_now);
    v_effective_period integer;
    v_derived bytea;
BEGIN
    SELECT * INTO v_rule FROM finance.entry_kind WHERE kind = p_entry ->> 'kind';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'unknown entry kind %', p_entry ->> 'kind' USING ERRCODE = 'FN003';
    END IF;
    IF NOT (v_rule.kind = ANY (p_allowed)) THEN
        RAISE EXCEPTION 'event kind % may not carry an entry of kind %', p_event ->> 'event_kind', v_rule.kind
            USING ERRCODE = 'FN003';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM finance.entry_reason WHERE code = p_entry ->> 'reason') THEN
        RAISE EXCEPTION 'entry kind % needs a reason from the closed list', v_rule.kind USING ERRCODE = 'FN003';
    END IF;
    IF NOT ((p_entry ->> 'actor_kind') = ANY (v_rule.actors)) THEN
        RAISE EXCEPTION 'entry kind % may not be posted by %', v_rule.kind, p_entry ->> 'actor_kind' USING ERRCODE = 'FN003';
    END IF;
    IF (p_entry ->> 'actor_kind') = 'system' AND (p_entry -> 'actor_id' <> 'null'::jsonb
        OR p_entry -> 'approver_id' <> 'null'::jsonb OR p_entry -> 'case_ref' <> 'null'::jsonb) THEN
        RAISE EXCEPTION 'a system entry names no actor, approver or case' USING ERRCODE = 'FN003';
    END IF;
    IF (p_entry ->> 'external_ref_kind') <> v_rule.external_ref_kind
       OR (v_rule.external_ref_kind <> 'none' AND (p_entry ->> 'external_ref') IS NULL) THEN
        RAISE EXCEPTION 'entry kind % needs external reference kind %', v_rule.kind, v_rule.external_ref_kind USING ERRCODE = 'FN003';
    END IF;
    IF v_rule.stream_rule = 'session' THEN
        IF (p_entry ->> 'stream_kind') NOT IN ('session', 'campaign', 'account') THEN
            RAISE EXCEPTION 'entry kind % posts to a session, campaign or account stream', v_rule.kind USING ERRCODE = 'FN003';
        END IF;
    ELSIF v_rule.stream_rule <> 'original' AND (p_entry ->> 'stream_kind') <> v_rule.stream_rule THEN
        RAISE EXCEPTION 'entry kind % posts to its % stream', v_rule.kind, v_rule.stream_rule USING ERRCODE = 'FN003';
    END IF;

    v_expect := finance.anchor_expectation(v_rule.anchor, p_entry, p_event);
    IF v_expect IS NULL THEN
        RAISE EXCEPTION 'entry kind % has no % anchor', v_rule.kind, v_rule.anchor USING ERRCODE = 'FN003';
    END IF;
    IF v_expect ? 'stream_id' THEN
        IF (p_entry ->> 'stream_kind') <> (v_expect ->> 'stream_kind') OR (p_entry ->> 'stream_id') <> (v_expect ->> 'stream_id') THEN
            RAISE EXCEPTION 'entry kind % posts to a stream other than its anchor''s', v_rule.kind USING ERRCODE = 'FN003';
        END IF;
        FOREACH v_field IN ARRAY ARRAY['campaign_id', 'session_id', 'attempt_id', 'reservation_id', 'price_version_id'] LOOP
            IF (p_entry ->> v_field) IS DISTINCT FROM (v_expect ->> v_field) THEN
                RAISE EXCEPTION 'entry % differs from its anchor in %', v_rule.kind, v_field USING ERRCODE = 'FN003';
            END IF;
        END LOOP;
    END IF;
    IF v_rule.anchor = 'trial_grant' THEN
        PERFORM finance.check_trial_entry(p_entry, p_event);
    END IF;
    v_derived := finance.derived_business_key(v_rule.kind, p_entry);
    IF v_derived IS NOT NULL AND v_derived IS DISTINCT FROM finance.hexbytes(p_entry -> 'business_key') THEN
        RAISE EXCEPTION 'entry kind % has a business key the database derives', v_rule.kind USING ERRCODE = 'FN003';
    END IF;

    -- An entry effective in another period than its posting period is a correction of a closed one;
    -- the service clock may lead or trail the database clock by up to a minute without being one.
    IF abs(v_effective - p_now) > 60000 THEN
        v_effective_period := finance.period_for(v_effective);
        IF v_effective_period <> p_period AND (
            (p_entry ->> 'corrects_period_id') IS DISTINCT FROM v_effective_period::text
            OR NOT EXISTS (SELECT 1 FROM finance.accounting_period WHERE period_id = v_effective_period AND status = 'closed')) THEN
            RAISE EXCEPTION 'an entry effective in period % posts there only as a correction of a closed period', v_effective_period
                USING ERRCODE = 'FN003';
        END IF;
    END IF;
    RETURN v_rule;
END $$;

-- The lines of an event: a line is a debit or credit the entry kind allows, never zero.
CREATE FUNCTION finance.check_event_lines(p_lines jsonb) RETURNS void
LANGUAGE plpgsql STABLE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_bad record;
BEGIN
    SELECT r.entry_kind, r.code, r.amount INTO v_bad
      FROM jsonb_to_recordset(p_lines) AS r(entry_kind text, code integer, amount bigint)
     WHERE r.amount = 0 OR NOT EXISTS (SELECT 1 FROM finance.entry_kind_rule k
                                        WHERE k.kind = r.entry_kind AND k.code = r.code
                                          AND k.side = CASE WHEN r.amount > 0 THEN 1 ELSE -1 END)
     LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'entry kind % may not post % on account % ', v_bad.entry_kind,
            CASE WHEN v_bad.amount > 0 THEN 'a debit' ELSE 'a credit' END, v_bad.code USING ERRCODE = 'FN003';
    END IF;
END $$;

-- ---------------------------------------------------------------------------------------------
-- The event document: lines decoded once, keys resolved by dimension key.
-- ---------------------------------------------------------------------------------------------
CREATE FUNCTION finance.event_lines_doc(p_entries jsonb) RETURNS jsonb
LANGUAGE sql STABLE AS $$
    SELECT coalesce(jsonb_agg(jsonb_build_object(
        'entry_ix', e.ord - 1, 'line_no', l.ord, 'entry_kind', e.val ->> 'kind',
        'entry_reservation', e.val ->> 'reservation_id',
        'code', (l.val ->> 'code')::integer, 'currency', l.val ->> 'currency',
        'payer', l.val -> 'payer', 'lot', l.val -> 'lot', 'allocation', l.val -> 'allocation',
        'provider', l.val -> 'provider', 'processor', l.val -> 'processor', 'loss_class', l.val -> 'loss_class',
        'jurisdiction', l.val -> 'jurisdiction', 'promo_source', l.val -> 'promo_source',
        'amount', (l.val ->> 'amount')::bigint,
        'dk', encode(finance.dimension_key((l.val ->> 'code')::integer, l.val ->> 'currency',
            finance.hexbytes(l.val -> 'payer'), finance.hexbytes(l.val -> 'lot'), finance.hexbytes(l.val -> 'allocation'),
            l.val ->> 'provider', l.val ->> 'processor', l.val ->> 'loss_class', l.val ->> 'jurisdiction',
            l.val ->> 'promo_source'), 'hex')) ORDER BY e.ord, l.ord), '[]'::jsonb)
      FROM jsonb_array_elements(p_entries) WITH ORDINALITY AS e(val, ord),
           jsonb_array_elements(e.val -> 'lines') WITH ORDINALITY AS l(val, ord)
$$;

-- key_id by dimension key (hex) for every key the lines name that exists.
CREATE FUNCTION finance.key_map(p_lines jsonb) RETURNS jsonb
LANGUAGE sql STABLE AS $$
    SELECT coalesce(jsonb_object_agg(encode(k.dimension_key, 'hex'), k.key_id), '{}'::jsonb)
      FROM finance.account_key k
     WHERE k.dimension_key = ANY (SELECT decode(r.dk, 'hex') FROM jsonb_to_recordset(p_lines) AS r(dk text))
$$;

-- Locks the chain head of every stream the event posts to, then the balance rows of the keys that
-- already exist, each in sorted order, before anything else is written (19.1).
CREATE FUNCTION finance.lock_event_heads(p_entries jsonb, p_keys jsonb) RETURNS void
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $$
BEGIN
    INSERT INTO finance.ledger_stream (stream_kind, stream_id, last_seq, last_hash)
    SELECT DISTINCT e.val ->> 'stream_kind', finance.hexbytes(e.val -> 'stream_id'), 0, decode(repeat('00', 32), 'hex')
      FROM jsonb_array_elements(p_entries) AS e(val)
    ON CONFLICT DO NOTHING;
    PERFORM 1 FROM finance.ledger_stream s
     WHERE (s.stream_kind, s.stream_id) IN (SELECT e.val ->> 'stream_kind', finance.hexbytes(e.val -> 'stream_id')
                                              FROM jsonb_array_elements(p_entries) AS e(val))
     ORDER BY s.stream_kind, s.stream_id FOR UPDATE;
    PERFORM 1 FROM finance.account_balance b
     WHERE b.key_id IN (SELECT (v.value)::bigint FROM jsonb_each_text(p_keys) v)
     ORDER BY b.key_id FOR UPDATE;
END $$;

-- Phase 1 or 2 of the row changes. Phase 1 runs the key creations last and in sorted order, so two
-- events creating the same keys cannot wait on each other.
CREATE FUNCTION finance.apply_phase(p_event_kind text, p_event jsonb, p_phase smallint) RETURNS void
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_change jsonb;
BEGIN
    FOR v_change IN
        SELECT c.val FROM jsonb_array_elements(coalesce(p_event -> 'changes', '[]'::jsonb)) WITH ORDINALITY AS c(val, ord)
          JOIN finance.change_kind k ON k.kind = c.val ->> 'kind'
         WHERE k.phase = p_phase
         ORDER BY (k.op = 'key'),
                  CASE WHEN k.op = 'key' THEN encode(finance.dimension_key((c.val -> 'row' ->> 'code')::integer,
                        c.val -> 'row' ->> 'currency', finance.rowbytes(c.val -> 'row' ->> 'payer'),
                        finance.rowbytes(c.val -> 'row' ->> 'lot'), finance.rowbytes(c.val -> 'row' ->> 'allocation'),
                        c.val -> 'row' ->> 'provider', c.val -> 'row' ->> 'processor', c.val -> 'row' ->> 'loss_class',
                        c.val -> 'row' ->> 'jurisdiction', c.val -> 'row' ->> 'promo_source'), 'hex') END,
                  c.ord
    LOOP
        PERFORM finance.apply_change(p_event_kind, v_change);
    END LOOP;
    IF p_phase = 1 AND EXISTS (SELECT 1 FROM jsonb_array_elements(coalesce(p_event -> 'changes', '[]'::jsonb)) c
                                WHERE NOT EXISTS (SELECT 1 FROM finance.change_kind k WHERE k.kind = c ->> 'kind')) THEN
        RAISE EXCEPTION 'unknown change kind' USING ERRCODE = 'FN003';
    END IF;
END $$;

-- ---------------------------------------------------------------------------------------------
-- Posting: entries with their hash chain, then all lines and all balance deltas in one statement each.
-- ---------------------------------------------------------------------------------------------
CREATE FUNCTION finance.post_entries(p_event_id bytea, p_entries jsonb, p_lines jsonb, p_keys jsonb,
                                     p_now bigint, p_period integer) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_entry jsonb;
    v_ix integer := 0;
    v_head finance.ledger_stream%ROWTYPE;
    v_ej jsonb;
    v_hash bytea;
    v_entry_id bytea;
    v_seq bigint;
    v_bk bytea;
    v_out jsonb := '[]'::jsonb;
BEGIN
    FOR v_entry IN SELECT value FROM jsonb_array_elements(p_entries) LOOP
        v_ix := v_ix + 1;
        SELECT * INTO v_head FROM finance.ledger_stream
         WHERE stream_kind = v_entry ->> 'stream_kind' AND stream_id = finance.hexbytes(v_entry -> 'stream_id');
        v_seq := v_head.last_seq + 1;
        v_entry_id := finance.hexbytes(v_entry -> 'entry_id');
        v_bk := coalesce(finance.derived_business_key(v_entry ->> 'kind', v_entry), finance.hexbytes(v_entry -> 'business_key'));
        v_ej := jsonb_build_object(
            'entry_id', encode(v_entry_id, 'hex'), 'event_id', encode(p_event_id, 'hex'), 'kind', v_entry ->> 'kind',
            'business_key', encode(v_bk, 'hex'),
            'external_ref_kind', v_entry ->> 'external_ref_kind', 'external_ref', v_entry -> 'external_ref',
            'stream_kind', v_entry ->> 'stream_kind', 'stream_id', v_entry ->> 'stream_id', 'stream_seq', v_seq,
            'recorded_at_ms', p_now, 'effective_at_ms', coalesce((v_entry ->> 'effective_at_ms')::bigint, p_now),
            'period_id', p_period, 'corrects_period_id', v_entry -> 'corrects_period_id',
            'reverses_entry_id', v_entry -> 'reverses_entry_id',
            'campaign_id', v_entry -> 'campaign_id', 'session_id', v_entry -> 'session_id',
            'attempt_id', v_entry -> 'attempt_id', 'reservation_id', v_entry -> 'reservation_id',
            'price_version_id', v_entry -> 'price_version_id',
            'reason', v_entry ->> 'reason', 'case_ref', v_entry -> 'case_ref',
            'actor_kind', v_entry ->> 'actor_kind', 'actor_id', v_entry -> 'actor_id',
            'approver_id', v_entry -> 'approver_id',
            'lines', (SELECT jsonb_agg(jsonb_build_object('code', r.code, 'currency', r.currency, 'payer', r.payer,
                    'lot', r.lot, 'allocation', r.allocation, 'provider', r.provider, 'processor', r.processor,
                    'loss_class', r.loss_class, 'jurisdiction', r.jurisdiction, 'promo_source', r.promo_source,
                    'amount', r.amount) ORDER BY r.line_no)
                FROM jsonb_to_recordset(p_lines) AS r(entry_ix integer, line_no integer, code integer, currency text,
                    payer jsonb, lot jsonb, allocation jsonb, provider text, processor text, loss_class text,
                    jurisdiction text, promo_source text, amount bigint)
               WHERE r.entry_ix = v_ix - 1));
        v_hash := finance.entry_hash(1::smallint, v_head.last_hash, v_ej);

        INSERT INTO finance.journal_entry (entry_id, kind, business_key, external_ref_kind, external_ref,
            stream_kind, stream_id, stream_seq, prev_hash, entry_hash, entry_hash_version, effective_at_ms,
            recorded_at_ms, period_id, line_count, reverses_entry_id, actor_kind, actor_id, approver_id,
            reason, case_ref, event_id, campaign_id, session_id, attempt_id, reservation_id,
            price_version_id, corrects_period_id)
        VALUES (v_entry_id, v_entry ->> 'kind', v_bk,
            v_entry ->> 'external_ref_kind', v_entry ->> 'external_ref', v_entry ->> 'stream_kind',
            finance.hexbytes(v_entry -> 'stream_id'), v_seq, v_head.last_hash, v_hash, 1,
            coalesce((v_entry ->> 'effective_at_ms')::bigint, p_now), p_now, p_period,
            jsonb_array_length(v_entry -> 'lines'), finance.hexbytes(v_entry -> 'reverses_entry_id'),
            v_entry ->> 'actor_kind', finance.hexbytes(v_entry -> 'actor_id'),
            finance.hexbytes(v_entry -> 'approver_id'), v_entry ->> 'reason', v_entry ->> 'case_ref',
            p_event_id, finance.hexbytes(v_entry -> 'campaign_id'), finance.hexbytes(v_entry -> 'session_id'),
            finance.hexbytes(v_entry -> 'attempt_id'), finance.hexbytes(v_entry -> 'reservation_id'),
            finance.hexbytes(v_entry -> 'price_version_id'), (v_entry ->> 'corrects_period_id')::integer);
        UPDATE finance.ledger_stream SET last_seq = v_seq, last_hash = v_hash
         WHERE stream_kind = v_entry ->> 'stream_kind' AND stream_id = finance.hexbytes(v_entry -> 'stream_id');
        v_out := v_out || jsonb_build_array(jsonb_build_object(
            'entry_id', encode(v_entry_id, 'hex'), 'stream_seq', v_seq, 'entry_hash', encode(v_hash, 'hex')));
    END LOOP;

    INSERT INTO finance.journal_line (entry_id, line_no, key_id, amount_micro_dollars)
    SELECT finance.hexbytes(e.val -> 'entry_id'), r.line_no, (p_keys ->> r.dk)::bigint, r.amount
      FROM jsonb_to_recordset(p_lines) AS r(entry_ix integer, line_no integer, dk text, amount bigint)
      JOIN jsonb_array_elements(p_entries) WITH ORDINALITY AS e(val, ord) ON e.ord = r.entry_ix + 1;

    UPDATE finance.account_balance b SET natural_micro_dollars = b.natural_micro_dollars + d.delta
      FROM (SELECT (p_keys ->> r.dk)::bigint AS key_id, sum(r.amount * a.normal_side) AS delta
              FROM jsonb_to_recordset(p_lines) AS r(code integer, dk text, amount bigint)
              JOIN finance.ledger_account a ON a.code = r.code GROUP BY 1) d
     WHERE b.key_id = d.key_id;
    RETURN v_out;
END $$;

-- The journal's own balance rules for the entries of one event, set-based: every entry balances in
-- every book and currency, and every enforced key has its balance row.
CREATE FUNCTION finance.check_event_balance(p_entry_ids bytea[], p_keys jsonb) RETURNS void
LANGUAGE plpgsql STABLE SET search_path = pg_catalog, finance, pg_temp AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM finance.journal_line l
                 JOIN finance.account_key k ON k.key_id = l.key_id
                 JOIN finance.ledger_account a ON a.code = k.code
                WHERE l.entry_id = ANY (p_entry_ids)
                GROUP BY l.entry_id, a.book, k.currency HAVING sum(l.amount_micro_dollars) <> 0) THEN
        RAISE EXCEPTION 'entry is not balanced in every book and currency'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF EXISTS (SELECT 1 FROM finance.account_key k JOIN finance.ledger_account a ON a.code = k.code
                WHERE k.key_id IN (SELECT (v.value)::bigint FROM jsonb_each_text(p_keys) v) AND a.balance_enforced
                  AND NOT EXISTS (SELECT 1 FROM finance.account_balance b WHERE b.key_id = k.key_id)) THEN
        RAISE EXCEPTION 'an enforced key has no balance row' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
END $$;

-- The subledger checks of the objects an event touched, once each, then their marks (see
-- finance.already_checked): the per-row deferred triggers stay as the backstop for any other write.
CREATE FUNCTION finance.check_touched(p_event jsonb, p_keys jsonb, p_entry_ids bytea[]) RETURNS void
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_key_ids bigint[] := ARRAY(SELECT (v.value)::bigint FROM jsonb_each_text(p_keys) v);
    v_lots bytea[];
    v_allocations bytea[];
    v_pairs record;
    v_reservations bytea[];
    v_id bytea;
    v_tags text[] := '{}';
BEGIN
    SELECT array_agg(DISTINCT lot) INTO v_lots FROM (
        SELECT lot_id AS lot FROM finance.account_key WHERE key_id = ANY (v_key_ids) AND lot_id IS NOT NULL
        UNION SELECT finance.rowbytes(c.val -> 'row' ->> 'lot_id')
                FROM jsonb_array_elements(coalesce(p_event -> 'changes', '[]'::jsonb)) AS c(val)
               WHERE c.val ->> 'kind' IN ('funding_lot_insert', 'funding_lot_update')) u WHERE lot IS NOT NULL;
    SELECT array_agg(DISTINCT allocation) INTO v_allocations FROM (
        SELECT allocation_id AS allocation FROM finance.account_key WHERE key_id = ANY (v_key_ids) AND allocation_id IS NOT NULL
        UNION SELECT finance.rowbytes(c.val -> 'row' ->> 'allocation_id')
                FROM jsonb_array_elements(coalesce(p_event -> 'changes', '[]'::jsonb)) AS c(val)
               WHERE c.val ->> 'kind' IN ('allocation_insert', 'allocation_update')) u WHERE allocation IS NOT NULL;
    SELECT array_agg(DISTINCT reservation) INTO v_reservations FROM (
        SELECT finance.rowbytes(c.val -> 'row' ->> 'reservation_id') AS reservation
          FROM jsonb_array_elements(coalesce(p_event -> 'changes', '[]'::jsonb)) AS c(val)
         WHERE c.val ->> 'kind' IN ('reservation_insert', 'reservation_update', 'reservation_share_insert', 'reservation_share_update')) u
     WHERE reservation IS NOT NULL;

    FOREACH v_id IN ARRAY coalesce(v_lots, '{}') LOOP
        PERFORM finance.check_lot_conservation(v_id);
        v_tags := v_tags || ('lot_' || encode(v_id, 'hex'));
    END LOOP;
    FOREACH v_id IN ARRAY coalesce(v_allocations, '{}') LOOP
        PERFORM finance.check_allocation_totals(v_id);
        v_tags := v_tags || ('al_' || encode(v_id, 'hex'));
    END LOOP;
    FOR v_pairs IN
        SELECT DISTINCT allocation, lot FROM (
            SELECT allocation_id AS allocation, lot_id AS lot FROM finance.account_key
             WHERE key_id = ANY (v_key_ids) AND code IN (2130, 2230)
            UNION SELECT finance.rowbytes(c.val -> 'row' ->> 'allocation_id'), finance.rowbytes(c.val -> 'row' ->> 'lot_id')
                    FROM jsonb_array_elements(coalesce(p_event -> 'changes', '[]'::jsonb)) AS c(val)
                   WHERE c.val ->> 'kind' IN ('reservation_share_insert', 'reservation_share_update')) u
         WHERE allocation IS NOT NULL AND lot IS NOT NULL LOOP
        PERFORM finance.check_reserved_key(v_pairs.allocation, v_pairs.lot);
        v_tags := v_tags || ('rk_' || encode(v_pairs.allocation, 'hex') || encode(v_pairs.lot, 'hex'));
    END LOOP;
    FOREACH v_id IN ARRAY coalesce(v_reservations, '{}') LOOP
        PERFORM finance.check_reservation_totals(v_id);
        v_tags := v_tags || ('rv_' || encode(v_id, 'hex'));
    END LOOP;
    FOREACH v_id IN ARRAY p_entry_ids LOOP
        v_tags := v_tags || ('e_' || encode(v_id, 'hex')) || ('lb_' || encode(v_id, 'hex'));
    END LOOP;
    INSERT INTO finance.check_mark (txid, tag) SELECT txid_current(), t FROM unnest(v_tags) AS t ON CONFLICT DO NOTHING;
END $$;

-- ---------------------------------------------------------------------------------------------
-- commit_event: the one write path. Order: envelope, marker, period, line allowlist, stream and
-- balance locks, phase 1 changes, entries, lines and balances, equations, phase 2 changes, the
-- subledger checks, the journal copy.
-- ---------------------------------------------------------------------------------------------
CREATE FUNCTION finance.commit_event(p_event jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, finance, pg_temp
SET lock_timeout = '2s' AS $$
DECLARE
    v_txid bigint := txid_current();
    v_event_id bytea := finance.hexbytes(p_event -> 'event_id');
    v_event_kind text := p_event ->> 'event_kind';
    v_ek finance.event_kind%ROWTYPE;
    v_now bigint := finance.now_ms();
    v_period integer := finance.period_for(v_now);
    v_entries jsonb := coalesce(p_event -> 'entries', '[]'::jsonb);
    v_entry jsonb;
    v_lines jsonb;
    v_keys jsonb;
    v_all_keys jsonb;
    v_entry_ids bytea[];
    v_doc jsonb;
    v_out jsonb;
    v_missing text;
BEGIN
    SELECT * INTO v_ek FROM finance.event_kind WHERE event_kind = v_event_kind;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'unknown event kind %', v_event_kind USING ERRCODE = 'FN003';
    END IF;
    IF coalesce((p_event ->> 'payload_schema')::integer, 0) <> 1 THEN
        RAISE EXCEPTION 'unsupported payload schema' USING ERRCODE = 'FN003';
    END IF;
    IF v_ek.carries_entries <> (jsonb_array_length(v_entries) > 0) THEN
        RAISE EXCEPTION 'event kind % % entries', v_event_kind, CASE WHEN v_ek.carries_entries THEN 'needs' ELSE 'may not carry' END
            USING ERRCODE = 'FN003';
    END IF;

    -- A caller cannot make the deferred checks fire early and break a multi-row insert.
    SET CONSTRAINTS journal_entry_balanced, journal_line_balanced, journal_line_balance_row,
        journal_entry_chain, journal_entry_reversal, funding_lot_conserves, account_balance_lot_conserves,
        allocation_totals, account_balance_allocation_totals, reservation_share_reserved,
        account_balance_reserved, reservation_totals, reservation_share_totals, funding_lot_open_shares DEFERRED;
    DELETE FROM finance.check_mark WHERE txid = v_txid;
    INSERT INTO finance.event_marker (txid, event_id, event_kind) VALUES (v_txid, v_event_id, v_event_kind);

    -- Posting period: the database clock decides, never the caller (17.8).
    PERFORM pg_advisory_xact_lock_shared(hashtextextended('finance.period:' || v_period, 0));
    IF (SELECT status FROM finance.accounting_period WHERE period_id = v_period) IS DISTINCT FROM 'open' THEN
        RAISE EXCEPTION 'no open accounting period for %', v_period USING ERRCODE = 'FN006';
    END IF;

    v_lines := finance.event_lines_doc(v_entries);
    PERFORM finance.check_event_lines(v_lines);
    v_keys := finance.key_map(v_lines);
    PERFORM finance.lock_event_heads(v_entries, v_keys);

    PERFORM finance.apply_phase(v_event_kind, p_event, 1::smallint);

    v_all_keys := finance.key_map(v_lines);
    SELECT r.dk INTO v_missing FROM jsonb_to_recordset(v_lines) AS r(dk text) WHERE NOT (v_all_keys ? r.dk) LIMIT 1;
    IF v_missing IS NOT NULL THEN
        RAISE EXCEPTION 'no account key for a line of the event (dimension key %)', v_missing USING ERRCODE = 'FN005';
    END IF;
    PERFORM 1 FROM finance.account_balance b
     WHERE b.key_id IN (SELECT (v.value)::bigint FROM jsonb_each_text(v_all_keys) v)
       AND NOT (b.key_id IN (SELECT (v.value)::bigint FROM jsonb_each_text(v_keys) v))
     ORDER BY b.key_id FOR UPDATE;

    FOR v_entry IN SELECT value FROM jsonb_array_elements(v_entries) LOOP
        PERFORM finance.check_entry(v_entry, p_event, v_ek.allowed_entries, v_now, v_period);
    END LOOP;
    v_out := finance.post_entries(v_event_id, v_entries, v_lines, v_all_keys, v_now, v_period);
    v_entry_ids := ARRAY(SELECT finance.hexbytes(e -> 'entry_id') FROM jsonb_array_elements(v_entries) e);
    PERFORM finance.check_event_balance(v_entry_ids, v_all_keys);

    -- Equations are checked against the rows as they are before the phase 2 changes.
    v_doc := p_event || jsonb_build_object('_lines', v_lines,
        '_kinds', coalesce((SELECT jsonb_agg(DISTINCT e ->> 'kind') FROM jsonb_array_elements(v_entries) e), '[]'::jsonb));
    PERFORM finance.check_equations(v_doc);

    PERFORM finance.apply_phase(v_event_kind, p_event, 2::smallint);
    PERFORM finance.check_touched(p_event, v_all_keys, v_entry_ids);

    -- TODO(FIN-01): journal.submit_pending replaces this row when the recovery journal lands.
    INSERT INTO finance.pending_event (event_id, event_kind, payload_schema, payload, metadata, recorded_at_ms)
    VALUES (v_event_id, v_event_kind, 1,
            jsonb_build_object('event', finance.payload_for_journal(p_event), 'entries', v_out),
            jsonb_build_object('event_id', encode(v_event_id, 'hex'), 'event_kind', v_event_kind,
                'streams', coalesce((SELECT jsonb_agg(DISTINCT jsonb_build_object('kind', e ->> 'stream_kind', 'id', e ->> 'stream_id'))
                                       FROM jsonb_array_elements(v_entries) e), '[]'::jsonb),
                'campaign_id', (SELECT e -> 'campaign_id' FROM jsonb_array_elements(v_entries) e LIMIT 1)),
            v_now);

    DELETE FROM finance.flow_lock WHERE txid = v_txid;
    RETURN jsonb_build_object('event_id', encode(v_event_id, 'hex'), 'entries', v_out);
END $$;

-- ---------------------------------------------------------------------------------------------
-- lock_flow: the locks of one flow in rank order (19.1), and the snapshot read under them. Each
-- flow is its own function so its lock order reads on one screen; lock_flow only dispatches.
-- Ranks: 1 payer, 2 payment state, 3 session, 4 quote, 5 allocation, 6 lot, 7 attempt-level rows.
-- ---------------------------------------------------------------------------------------------
CREATE FUNCTION finance.lock_flow_payer(p_flow text, p_args jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_payer bytea := finance.hexbytes(p_args -> 'payer');
    v_snapshot jsonb;
BEGIN
    -- Two first grants for an account that does not exist yet have no row to wait on: serialize them.
    PERFORM pg_advisory_xact_lock(hashtextextended('finance.payer:' || encode(v_payer, 'hex'), 0));
    PERFORM finance.note_locks('payer', ARRAY(SELECT encode(account_id, 'hex') FROM finance.payer
                                               WHERE account_id = v_payer FOR UPDATE));
    v_snapshot := jsonb_build_object('payer', (SELECT to_jsonb(p) FROM finance.payer p WHERE account_id = v_payer));
    IF p_flow = 'purchase' THEN
        PERFORM finance.note_locks('processor_payment_state', ARRAY(SELECT processor || '|' || payment_ref
                FROM finance.processor_payment_state
               WHERE processor = p_args ->> 'processor' AND payment_ref = p_args ->> 'payment_ref' FOR UPDATE));
        v_snapshot := v_snapshot || jsonb_build_object('payment_state',
            (SELECT to_jsonb(s) FROM finance.processor_payment_state s
              WHERE processor = p_args ->> 'processor' AND payment_ref = p_args ->> 'payment_ref'));
    ELSIF p_flow = 'trial' THEN
        v_snapshot := v_snapshot || jsonb_build_object(
            'trial_by_account', (SELECT to_jsonb(t) - 'email_grant_key' FROM finance.trial_grant t WHERE account_id = v_payer),
            'trial_by_email', (SELECT jsonb_build_object('account_id', t.account_id) FROM finance.trial_grant t
                                WHERE email_grant_key = finance.hexbytes(p_args -> 'email_grant_key')));
    END IF;
    RETURN v_snapshot;
END $$;

-- Moves credit into the session allocation: payer, session, the payer's allocation, the payer's lots
-- FOR UPDATE (the draw moves available keys, and a lot's state decides what may be drawn).
CREATE FUNCTION finance.lock_flow_allocate(p_args jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_payer bytea := finance.hexbytes(p_args -> 'payer');
    v_session bytea := finance.hexbytes(p_args -> 'session');
BEGIN
    PERFORM finance.note_locks('payer', ARRAY(SELECT encode(account_id, 'hex') FROM finance.payer
                                               WHERE account_id = v_payer FOR UPDATE));
    PERFORM finance.note_locks('session_funding', ARRAY(SELECT encode(session_id, 'hex') FROM finance.session_funding
                                                         WHERE session_id = v_session FOR UPDATE));
    PERFORM finance.note_locks('allocation', ARRAY(SELECT encode(allocation_id, 'hex') FROM finance.allocation
                                                    WHERE session_id = v_session AND payer_account_id = v_payer
                                                    ORDER BY allocation_id FOR UPDATE));
    PERFORM finance.note_locks('funding_lot', ARRAY(SELECT encode(lot_id, 'hex') FROM finance.funding_lot
                                                     WHERE payer_account_id = v_payer ORDER BY lot_id FOR UPDATE));
    RETURN jsonb_build_object(
        'payer', (SELECT to_jsonb(p) FROM finance.payer p WHERE account_id = v_payer),
        'session', (SELECT to_jsonb(s) FROM finance.session_funding s WHERE session_id = v_session),
        'allocation', (SELECT to_jsonb(a) FROM finance.allocation a WHERE session_id = v_session AND payer_account_id = v_payer),
        'allocation_lots', coalesce((SELECT jsonb_agg(al.lot_id) FROM finance.allocation_lot al
                                      JOIN finance.allocation a ON a.allocation_id = al.allocation_id
                                     WHERE a.session_id = v_session AND a.payer_account_id = v_payer), '[]'::jsonb),
        'session_allocations', coalesce((SELECT jsonb_agg(to_jsonb(a)) FROM finance.allocation a WHERE a.session_id = v_session), '[]'::jsonb),
        'lots', coalesce((SELECT jsonb_agg(jsonb_build_object('lot', to_jsonb(l) - 'billing_country' - 'billing_region',
                    'available', coalesce((SELECT b.natural_micro_dollars FROM finance.account_key k
                                   JOIN finance.account_balance b ON b.key_id = k.key_id
                                  WHERE k.lot_id = l.lot_id AND k.code IN (2110, 2210)), 0),
                    'allocated', coalesce((SELECT b.natural_micro_dollars FROM finance.account_key k
                                   JOIN finance.account_balance b ON b.key_id = k.key_id
                                   JOIN finance.allocation a ON a.allocation_id = k.allocation_id
                                  WHERE k.lot_id = l.lot_id AND k.code IN (2120, 2220)
                                    AND a.session_id = v_session), 0))
                    ORDER BY l.class DESC, l.created_at_ms, l.lot_id)
                   FROM finance.funding_lot l WHERE l.payer_account_id = v_payer), '[]'::jsonb));
END $$;

CREATE FUNCTION finance.lock_flow_withdrawal(p_args jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_payer bytea := finance.hexbytes(p_args -> 'payer');
    v_session bytea := finance.hexbytes(p_args -> 'session');
    v_allocation bytea;
BEGIN
    PERFORM finance.note_locks('payer', ARRAY(SELECT encode(account_id, 'hex') FROM finance.payer
                                               WHERE account_id = v_payer FOR UPDATE));
    PERFORM finance.note_locks('session_funding', ARRAY(SELECT encode(session_id, 'hex') FROM finance.session_funding
                                                         WHERE session_id = v_session FOR UPDATE));
    SELECT allocation_id INTO v_allocation FROM finance.allocation
     WHERE session_id = v_session AND payer_account_id = v_payer FOR UPDATE;
    PERFORM finance.note_locks('allocation', ARRAY(SELECT encode(allocation_id, 'hex') FROM finance.allocation
                                                    WHERE allocation_id = v_allocation FOR UPDATE));
    PERFORM finance.note_locks('funding_lot', ARRAY(SELECT encode(lot_id, 'hex') FROM finance.funding_lot
                                                     WHERE lot_id IN (SELECT lot_id FROM finance.allocation_lot
                                                                       WHERE allocation_id = v_allocation)
                                                     ORDER BY lot_id FOR UPDATE));
    RETURN jsonb_build_object(
        'session', (SELECT to_jsonb(s) FROM finance.session_funding s WHERE session_id = v_session),
        'allocation', (SELECT to_jsonb(a) FROM finance.allocation a WHERE allocation_id = v_allocation),
        'lots', coalesce((SELECT jsonb_agg(jsonb_build_object('lot_id', l.lot_id, 'class', l.class,
                    'allocated', coalesce((SELECT b.natural_micro_dollars FROM finance.account_key k
                                     JOIN finance.account_balance b ON b.key_id = k.key_id
                                    WHERE k.allocation_id = v_allocation AND k.lot_id = l.lot_id
                                      AND k.code IN (2120, 2220)), 0)) ORDER BY al.draw_rank)
                   FROM finance.allocation_lot al JOIN finance.funding_lot l USING (lot_id)
                  WHERE al.allocation_id = v_allocation), '[]'::jsonb),
        'receipt_exists', EXISTS (SELECT 1 FROM finance.operation_receipt
            WHERE namespace = 'withdrawal' AND principal = (SELECT campaign_id FROM finance.session_funding WHERE session_id = v_session)
              AND operation_id = finance.hexbytes(p_args -> 'operation')));
END $$;

-- One paid admission: session, quote, the epoch's allocations, their lots FOR SHARE. The hourly
-- window is measured on the caller's clock, the one `settled_at_ms` is written from.
CREATE FUNCTION finance.lock_flow_admission(p_args jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_session bytea := finance.hexbytes(p_args -> 'session');
    v_quote bytea := finance.hexbytes(p_args -> 'quote');
    v_now bigint := (p_args ->> 'now_ms')::bigint;
    v_epoch bigint;
    v_allocations bytea[];
    v_lots bytea[];
BEGIN
    IF v_now IS NULL THEN
        RAISE EXCEPTION 'an admission lock carries the caller''s clock' USING ERRCODE = 'FN003';
    END IF;
    PERFORM finance.note_locks('session_funding', ARRAY(SELECT encode(session_id, 'hex') FROM finance.session_funding
                                                         WHERE session_id = v_session FOR UPDATE));
    SELECT current_epoch INTO v_epoch FROM finance.session_funding WHERE session_id = v_session;
    IF v_quote IS NOT NULL THEN
        PERFORM finance.note_locks('quote', ARRAY(SELECT encode(quote_id, 'hex') FROM finance.quote
                                                   WHERE quote_id = v_quote FOR UPDATE));
    END IF;
    SELECT array_agg(allocation_id ORDER BY allocation_id) INTO v_allocations
      FROM finance.epoch_share WHERE session_id = v_session AND epoch_seq = v_epoch;
    PERFORM finance.note_locks('allocation', ARRAY(SELECT encode(allocation_id, 'hex') FROM finance.allocation
                                                    WHERE allocation_id = ANY (coalesce(v_allocations, '{}'))
                                                    ORDER BY allocation_id FOR UPDATE));
    SELECT array_agg(DISTINCT lot_id ORDER BY lot_id) INTO v_lots
      FROM finance.allocation_lot WHERE allocation_id = ANY (coalesce(v_allocations, '{}'));
    PERFORM 1 FROM finance.funding_lot WHERE lot_id = ANY (coalesce(v_lots, '{}')) ORDER BY lot_id FOR SHARE;
    RETURN jsonb_build_object(
        'session', (SELECT to_jsonb(s) FROM finance.session_funding s WHERE session_id = v_session),
        'price_version', (SELECT to_jsonb(v) FROM finance.price_version v
                           JOIN finance.session_funding s ON s.price_version_id = v.price_version_id WHERE s.session_id = v_session),
        'tariff', coalesce((SELECT jsonb_agg(to_jsonb(t)) FROM finance.price_version_tariff t
                             JOIN finance.session_funding s ON s.price_version_id = t.price_version_id WHERE s.session_id = v_session), '[]'::jsonb),
        'quote', (SELECT to_jsonb(q) FROM finance.quote q WHERE quote_id = v_quote),
        'epoch_shares', coalesce((SELECT jsonb_agg(to_jsonb(e) ORDER BY e.allocation_id) FROM finance.epoch_share e
                                   WHERE e.session_id = v_session AND e.epoch_seq = v_epoch), '[]'::jsonb),
        'allocations', coalesce((SELECT jsonb_agg(jsonb_build_object('allocation', to_jsonb(a),
                'standing', (SELECT standing FROM finance.payer p WHERE p.account_id = a.payer_account_id),
                'approved_hourly_ceiling', c.approved_hourly_ceiling,
                'approved_session_ceiling', c.approved_session_ceiling,
                'charged_last_hour', (SELECT coalesce(sum(s.charged_micro_dollars), 0)
                    FROM finance.reservation_share s JOIN finance.reservation r USING (reservation_id)
                    JOIN finance.usage_event u ON u.attempt_id = (SELECT pa.attempt_id FROM finance.provider_attempt pa
                                                                    WHERE pa.reservation_id = r.reservation_id)
                    WHERE s.allocation_id = a.allocation_id AND u.settled_at_ms > v_now - 3600000),
                'open_holds', (SELECT coalesce(sum(s.held_micro_dollars - s.charged_micro_dollars - s.released_micro_dollars), 0)
                    FROM finance.reservation_share s JOIN finance.reservation r USING (reservation_id)
                    WHERE s.allocation_id = a.allocation_id AND r.state = 'held'),
                'lots', (SELECT coalesce(jsonb_agg(jsonb_build_object('lot_id', l.lot_id, 'class', l.class, 'state', l.state,
                                'created_at_ms', l.created_at_ms, 'draw_rank', al.draw_rank,
                                'allocated', coalesce((SELECT b.natural_micro_dollars FROM finance.account_key k
                                               JOIN finance.account_balance b ON b.key_id = k.key_id
                                              WHERE k.allocation_id = a.allocation_id AND k.lot_id = l.lot_id
                                                AND k.code IN (2120, 2220)), 0))
                                ORDER BY al.draw_rank, l.lot_id), '[]'::jsonb)
                           FROM finance.allocation_lot al JOIN finance.funding_lot l ON l.lot_id = al.lot_id
                          WHERE al.allocation_id = a.allocation_id))
                ORDER BY a.allocation_id) FROM finance.allocation a
                JOIN finance.allocation_consent c ON c.allocation_id = a.allocation_id
                    AND c.consent_revision = a.consent_revision
                WHERE a.allocation_id = ANY (coalesce(v_allocations, '{}'))), '[]'::jsonb),
        'open_holds', (SELECT coalesce(sum(retail_maximum_micro_dollars - charged_micro_dollars - released_micro_dollars), 0)
                         FROM finance.reservation WHERE session_id = v_session AND state = 'held'),
        'charged_total', (SELECT charged_total_micro_dollars FROM finance.session_funding WHERE session_id = v_session),
        'charged_last_hour', (SELECT coalesce(sum(retail_micro_dollars), 0) FROM finance.usage_event
                               WHERE session_id = v_session AND settled_at_ms > v_now - 3600000),
        'receipt', (SELECT to_jsonb(r) FROM finance.operation_receipt r WHERE r.namespace = 'admission'
                     AND r.principal = finance.hexbytes(p_args -> 'principal') AND r.operation_id = finance.hexbytes(p_args -> 'operation')));
END $$;

-- Settles or releases a held reservation: session, the shares' allocations, their lots FOR UPDATE,
-- then the attempt, reservation and shares (rank 7). A state that moved meanwhile answers `stale`.
CREATE FUNCTION finance.lock_flow_held(p_args jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_attempt bytea := finance.hexbytes(p_args -> 'attempt');
    v_reservation bytea;
    v_session bytea;
    v_allocations bytea[];
    v_lots bytea[];
BEGIN
    SELECT reservation_id INTO v_reservation FROM finance.provider_attempt WHERE attempt_id = v_attempt;
    SELECT session_id INTO v_session FROM finance.reservation WHERE reservation_id = v_reservation;
    IF v_session IS NULL THEN
        RETURN jsonb_build_object('stale', true, 'reason', 'unknown');
    END IF;
    PERFORM finance.note_locks('session_funding', ARRAY(SELECT encode(session_id, 'hex') FROM finance.session_funding
                                                         WHERE session_id = v_session FOR UPDATE));
    SELECT array_agg(DISTINCT allocation_id ORDER BY allocation_id), array_agg(DISTINCT lot_id ORDER BY lot_id)
      INTO v_allocations, v_lots FROM finance.reservation_share WHERE reservation_id = v_reservation;
    PERFORM finance.note_locks('allocation', ARRAY(SELECT encode(allocation_id, 'hex') FROM finance.allocation
                                                    WHERE allocation_id = ANY (coalesce(v_allocations, '{}'))
                                                    ORDER BY allocation_id FOR UPDATE));
    PERFORM finance.note_locks('funding_lot', ARRAY(SELECT encode(lot_id, 'hex') FROM finance.funding_lot
                                                     WHERE lot_id = ANY (coalesce(v_lots, '{}')) ORDER BY lot_id FOR UPDATE));
    PERFORM finance.note_locks('provider_attempt', ARRAY(SELECT encode(attempt_id, 'hex') FROM finance.provider_attempt
                                                          WHERE reservation_id = v_reservation FOR UPDATE));
    PERFORM finance.note_locks('reservation', ARRAY(SELECT encode(reservation_id, 'hex') FROM finance.reservation
                                                     WHERE reservation_id = v_reservation FOR UPDATE));
    PERFORM finance.note_locks('reservation_share', ARRAY(
        SELECT encode(reservation_id, 'hex') || '|' || encode(allocation_id, 'hex') || '|' || encode(lot_id, 'hex')
          FROM finance.reservation_share WHERE reservation_id = v_reservation ORDER BY allocation_id, lot_id FOR UPDATE));
    IF (SELECT state FROM finance.reservation WHERE reservation_id = v_reservation) <> 'held' THEN
        RETURN jsonb_build_object('stale', true, 'reason', 'not_held');
    END IF;
    RETURN jsonb_build_object(
        'session', (SELECT to_jsonb(s) FROM finance.session_funding s WHERE session_id = v_session),
        'reservation', (SELECT to_jsonb(r) FROM finance.reservation r WHERE reservation_id = v_reservation),
        'attempt', (SELECT to_jsonb(a) - 'provider_request_ref' FROM finance.provider_attempt a WHERE reservation_id = v_reservation),
        'shares', coalesce((SELECT jsonb_agg(to_jsonb(s) ORDER BY s.allocation_id, s.lot_id) FROM finance.reservation_share s
                             WHERE reservation_id = v_reservation), '[]'::jsonb),
        'allocations', coalesce((SELECT jsonb_agg(to_jsonb(a) ORDER BY a.allocation_id) FROM finance.allocation a
                                  WHERE allocation_id = ANY (coalesce(v_allocations, '{}'))), '[]'::jsonb),
        'lots', coalesce((SELECT jsonb_agg(to_jsonb(l) - 'billing_country' - 'billing_region' ORDER BY l.lot_id)
                           FROM finance.funding_lot l WHERE lot_id = ANY (coalesce(v_lots, '{}'))), '[]'::jsonb),
        'price_version', (SELECT to_jsonb(v) FROM finance.price_version v
                           JOIN finance.reservation r ON r.price_version_id = v.price_version_id WHERE r.reservation_id = v_reservation),
        'tariff', coalesce((SELECT jsonb_agg(to_jsonb(t)) FROM finance.price_version_tariff t
                             JOIN finance.reservation r ON r.price_version_id = t.price_version_id WHERE r.reservation_id = v_reservation), '[]'::jsonb));
END $$;

-- An operator credit of a charge: the payers of the charge's shares, the lots they hold FOR UPDATE,
-- then the usage event and the shares.
CREATE FUNCTION finance.lock_flow_credit(p_args jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_usage bytea := finance.hexbytes(p_args -> 'usage_event');
    v_attempt bytea;
    v_reservation bytea;
    v_payers bytea[];
    v_lots bytea[];
BEGIN
    SELECT attempt_id INTO v_attempt FROM finance.usage_event WHERE usage_event_id = v_usage;
    SELECT reservation_id INTO v_reservation FROM finance.provider_attempt WHERE attempt_id = v_attempt;
    SELECT array_agg(DISTINCT a.payer_account_id ORDER BY a.payer_account_id), array_agg(DISTINCT s.lot_id ORDER BY s.lot_id)
      INTO v_payers, v_lots FROM finance.reservation_share s JOIN finance.allocation a ON a.allocation_id = s.allocation_id
     WHERE s.reservation_id = v_reservation;
    PERFORM finance.note_locks('payer', ARRAY(SELECT encode(account_id, 'hex') FROM finance.payer
                                               WHERE account_id = ANY (coalesce(v_payers, '{}')) ORDER BY account_id FOR UPDATE));
    PERFORM finance.note_locks('funding_lot', ARRAY(SELECT encode(lot_id, 'hex') FROM finance.funding_lot
                                                     WHERE lot_id = ANY (coalesce(v_lots, '{}')) ORDER BY lot_id FOR UPDATE));
    PERFORM finance.note_locks('usage_event', ARRAY(SELECT encode(usage_event_id, 'hex') FROM finance.usage_event
                                                     WHERE usage_event_id = v_usage FOR UPDATE));
    PERFORM finance.note_locks('reservation_share', ARRAY(
        SELECT encode(reservation_id, 'hex') || '|' || encode(allocation_id, 'hex') || '|' || encode(lot_id, 'hex')
          FROM finance.reservation_share WHERE reservation_id = v_reservation ORDER BY allocation_id, lot_id FOR UPDATE));
    RETURN jsonb_build_object(
        'session', (SELECT to_jsonb(s) FROM finance.session_funding s JOIN finance.usage_event u ON u.session_id = s.session_id WHERE u.usage_event_id = v_usage),
        'usage_event', (SELECT to_jsonb(u) FROM finance.usage_event u WHERE usage_event_id = v_usage),
        'attempt', (SELECT to_jsonb(a) - 'provider_request_ref' FROM finance.provider_attempt a WHERE attempt_id = v_attempt),
        'shares', coalesce((SELECT jsonb_agg(jsonb_build_object('share', to_jsonb(s), 'payer', a.payer_account_id,
                    'allocation_status', a.status) ORDER BY s.allocation_id, s.lot_id)
                    FROM finance.reservation_share s JOIN finance.allocation a ON a.allocation_id = s.allocation_id
                   WHERE s.reservation_id = v_reservation), '[]'::jsonb),
        'lots', coalesce((SELECT jsonb_agg(to_jsonb(l) - 'billing_country' - 'billing_region' ORDER BY l.lot_id)
                           FROM finance.funding_lot l WHERE lot_id = ANY (coalesce(v_lots, '{}'))), '[]'::jsonb));
END $$;

CREATE FUNCTION finance.lock_flow(p_flow text, p_args jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, finance, pg_temp
SET lock_timeout = '2s' AS $$
DECLARE
    v_attempt bytea := finance.hexbytes(p_args -> 'attempt');
    v_session bytea := finance.hexbytes(p_args -> 'session');
    v_quote bytea := finance.hexbytes(p_args -> 'quote');
BEGIN
    IF p_flow IN ('trial', 'purchase', 'payer') THEN
        RETURN finance.lock_flow_payer(p_flow, p_args);
    ELSIF p_flow = 'allocate' THEN
        RETURN finance.lock_flow_allocate(p_args);
    ELSIF p_flow = 'withdrawal' THEN
        RETURN finance.lock_flow_withdrawal(p_args);
    ELSIF p_flow = 'admission' THEN
        RETURN finance.lock_flow_admission(p_args);
    ELSIF p_flow IN ('settlement', 'release') THEN
        RETURN finance.lock_flow_held(p_args);
    ELSIF p_flow = 'credit' THEN
        RETURN finance.lock_flow_credit(p_args);
    ELSIF p_flow = 'attempt' THEN
        -- rank 7: the attempt alone (prepare, send and unsent take no row above it).
        PERFORM finance.note_locks('provider_attempt', ARRAY(SELECT encode(attempt_id, 'hex') FROM finance.provider_attempt
                                                              WHERE attempt_id = v_attempt FOR UPDATE));
        RETURN jsonb_build_object('attempt', (SELECT to_jsonb(a) - 'provider_request_ref' FROM finance.provider_attempt a WHERE attempt_id = v_attempt));
    ELSIF p_flow = 'quote' THEN
        -- rank 3 session, rank 4 quote; a quote of another session answers as absent.
        PERFORM finance.note_locks('session_funding', ARRAY(SELECT encode(session_id, 'hex') FROM finance.session_funding
                                                             WHERE session_id = v_session FOR UPDATE));
        PERFORM finance.note_locks('quote', ARRAY(SELECT encode(quote_id, 'hex') FROM finance.quote
                                                   WHERE quote_id = v_quote AND session_id = v_session FOR UPDATE));
        RETURN jsonb_build_object(
            'session', (SELECT to_jsonb(s) FROM finance.session_funding s WHERE session_id = v_session),
            'quote', (SELECT to_jsonb(q) FROM finance.quote q WHERE quote_id = v_quote AND session_id = v_session));
    END IF;
    RAISE EXCEPTION 'unknown flow %', p_flow USING ERRCODE = 'FN003';
END $$;

-- Rows that are only bookkeeping about a finished transaction can be swept. A transaction is
-- finished when the commit log no longer says it is in progress, whatever else runs in the cluster.
CREATE FUNCTION finance.sweep_flow_state() RETURNS bigint
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_removed bigint;
BEGIN
    DELETE FROM finance.flow_lock
     WHERE txid <> txid_current() AND pg_xact_status(txid::text::xid8) IS DISTINCT FROM 'in progress';
    GET DIAGNOSTICS v_removed = ROW_COUNT;
    DELETE FROM finance.event_marker
     WHERE txid <> txid_current() AND pg_xact_status(txid::text::xid8) IS DISTINCT FROM 'in progress';
    DELETE FROM finance.check_mark
     WHERE txid <> txid_current() AND pg_xact_status(txid::text::xid8) IS DISTINCT FROM 'in progress';
    RETURN v_removed;
END $$;

-- ---------------------------------------------------------------------------------------------
-- The write guard: a statement that writes a journaled table in a transaction that carries no
-- event marker fails at once (17.6), so a definer that bypassed commit_event cannot succeed. One
-- call per statement, not per row.
-- ---------------------------------------------------------------------------------------------
CREATE FUNCTION finance.write_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, finance, pg_temp AS $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM finance.event_marker WHERE txid = txid_current()) THEN
        RAISE EXCEPTION 'a finance row was written outside commit_event (%)', TG_TABLE_NAME
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NULL;
END $$;
DO $$
DECLARE
    t text;
BEGIN
    FOREACH t IN ARRAY ARRAY['payer', 'funding_lot', 'processor_payment_state', 'trial_grant', 'price_version',
        'price_version_tariff', 'session_funding', 'allocation', 'allocation_consent', 'allocation_lot',
        'funding_epoch', 'epoch_share', 'quote', 'reservation', 'reservation_share', 'provider_attempt',
        'usage_event', 'usage_reversal', 'operation_receipt', 'settlement_fact', 'attempt_outcome_event',
        'company_loss', 'forgone_revenue', 'meter_segment_priced', 'journal_entry', 'journal_line',
        'account_key', 'account_balance', 'ledger_stream'] LOOP
        EXECUTE format('CREATE TRIGGER %I BEFORE INSERT OR UPDATE OR DELETE ON finance.%I FOR EACH STATEMENT EXECUTE FUNCTION finance.write_guard()',
                       t || '_write_guard', t);
    END LOOP;
END $$;

-- The entry-bearing tables' foreign keys to entries are checked at commit, so a row change may
-- precede its entry within an event.
ALTER TABLE finance.usage_event ALTER CONSTRAINT usage_event_entry_id_fkey DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE finance.usage_reversal ALTER CONSTRAINT usage_reversal_entry_id_fkey DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE finance.company_loss ALTER CONSTRAINT company_loss_entry_id_fkey DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE finance.meter_segment_priced ALTER CONSTRAINT meter_segment_priced_entry_id_fkey DEFERRABLE INITIALLY DEFERRED;

-- ---------------------------------------------------------------------------------------------
-- Roles and grants (17.9).
-- ---------------------------------------------------------------------------------------------
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'df_finance_service') THEN
        -- The login the service connects as, once S2 gives it a credential and LOGIN (TODO(FIN-11)). It
        -- is only a member of df_finance and carries the timeouts below as its own defaults: a role's
        -- settings apply to that role's logins, not to the roles it is a member of.
        CREATE ROLE df_finance_service NOLOGIN IN ROLE df_finance;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'df_finance_replay') THEN
        -- NOLOGIN until S2 provisions its credential and enables the login (TODO(FIN-11)); it is not a
        -- member of df_finance and df_finance is not a member of it (22.2).
        CREATE ROLE df_finance_replay NOLOGIN CONNECTION LIMIT 2;
    END IF;
END $$;

REVOKE ALL ON ALL TABLES IN SCHEMA finance FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA finance FROM PUBLIC;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA finance FROM PUBLIC;
-- PostgreSQL grants EXECUTE on a new function to PUBLIC, and a schema-scoped default privilege cannot
-- take that away (only a database-wide one can, which would reach other owners' functions). A later
-- migration therefore revokes it for its own functions, and `roles.rs` fails the build of any
-- function in this schema that PUBLIC can still execute.
GRANT USAGE ON SCHEMA finance TO df_finance, df_finance_definer, df_finance_replay;
GRANT SELECT ON ALL TABLES IN SCHEMA finance TO df_finance, df_finance_definer;

-- The definer writes exactly what the registry and commit_event name: inserts into the insert
-- kinds' tables, updates on the update kinds' tables, and nothing on the immutable journal tables
-- beyond the insert.
DO $$
DECLARE
    r record;
BEGIN
    FOR r IN SELECT DISTINCT table_name, op FROM finance.change_kind WHERE op IN ('insert', 'update', 'upsert', 'delete') LOOP
        IF r.op = 'insert' THEN
            EXECUTE format('GRANT INSERT ON finance.%I TO df_finance_definer', r.table_name);
        ELSIF r.op = 'update' THEN
            -- only the columns the kinds of this table update, and the revision
            EXECUTE format('GRANT UPDATE (%s) ON finance.%I TO df_finance_definer',
                           (SELECT string_agg(DISTINCT format('%I', c), ', ')
                              FROM finance.change_kind k, unnest(k.updatable_cols || ARRAY['revision']) c
                             WHERE k.table_name = r.table_name AND k.op = 'update'), r.table_name);
        ELSE
            EXECUTE format('GRANT INSERT, UPDATE, DELETE ON finance.%I TO df_finance_definer', r.table_name);
        END IF;
    END LOOP;
END $$;
-- Rows a flow locks FOR UPDATE and no change kind updates: the lock needs one UPDATE column, and
-- `revision` is the least that does (a lock-only grant; no update kind writes these tables).
GRANT UPDATE (revision) ON finance.payer, finance.processor_payment_state TO df_finance_definer;
GRANT INSERT ON finance.journal_entry, finance.journal_line, finance.account_key, finance.pending_event,
    finance.event_marker, finance.flow_lock, finance.check_mark, finance.accounting_period,
    finance.account_balance, finance.ledger_stream TO df_finance_definer;
GRANT UPDATE ON finance.account_balance, finance.ledger_stream TO df_finance_definer;
GRANT DELETE ON finance.flow_lock, finance.event_marker, finance.check_mark TO df_finance_definer;
GRANT USAGE ON SEQUENCE finance.global_seq TO df_finance_definer;
GRANT USAGE ON ALL SEQUENCES IN SCHEMA finance TO df_finance_definer;
GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA finance TO df_finance_definer;

ALTER FUNCTION finance.precreate_periods(bigint, integer) OWNER TO df_finance_definer;
ALTER FUNCTION finance.extend_period_horizon(integer) OWNER TO df_finance_definer;
ALTER FUNCTION finance.account_key_balance_row() OWNER TO df_finance_definer;
ALTER FUNCTION finance.apply_change(text, jsonb) OWNER TO df_finance_definer;
ALTER FUNCTION finance.commit_event(jsonb) OWNER TO df_finance_definer;
ALTER FUNCTION finance.lock_flow(text, jsonb) OWNER TO df_finance_definer;
ALTER FUNCTION finance.sweep_flow_state() OWNER TO df_finance_definer;

GRANT EXECUTE ON FUNCTION finance.lock_flow(text, jsonb), finance.commit_event(jsonb),
    finance.sweep_flow_state(), finance.extend_period_horizon(integer), finance.session_charge_drift()
    TO df_finance;
-- The pure helpers (hashing and period arithmetic) the service role may call to verify what it reads.
GRANT EXECUTE ON FUNCTION finance.now_ms(), finance.period_for(bigint), finance.entry_json(bytea),
    finance.entry_canonical_v1(jsonb), finance.entry_hash(smallint, bytea, jsonb), finance.hexbytes(jsonb)
    TO df_finance;

-- Timeouts the service role carries from login, so no flow pays a round trip to set them (19.1).
ALTER ROLE df_finance_service SET lock_timeout = '2s';
ALTER ROLE df_finance_service SET statement_timeout = '10s';
ALTER ROLE df_finance_service SET idle_in_transaction_session_timeout = '15s';
ALTER ROLE df_finance_replay SET lock_timeout = '5s';
ALTER ROLE df_finance_replay SET statement_timeout = '120s';
ALTER ROLE df_finance_replay SET idle_in_transaction_session_timeout = '60s';
DO $$
BEGIN
    IF current_setting('server_version_num')::integer >= 170000 THEN
        EXECUTE 'ALTER ROLE df_finance_service SET transaction_timeout = ''20s''';
        EXECUTE 'ALTER ROLE df_finance_replay SET transaction_timeout = ''300s''';
    END IF;
END $$;
