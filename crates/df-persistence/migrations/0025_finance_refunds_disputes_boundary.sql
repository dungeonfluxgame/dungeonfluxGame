-- Finance refunds and disputes boundary (planning/finance-ledger-research.md 14, 16.9, 16.10,
-- 17.6; FIN-05). Extends the 0024 registry with the refund/dispute change kinds, event kinds,
-- lock flows and anchors the refund, dispute and invalid-output flows write through
-- `finance.commit_event`. The tables themselves are 0023; no row of any finance table is
-- written outside `commit_event` after this migration either.

-- ---------------------------------------------------------------------------------------------
-- Change kinds for the 0023 tables and the dispute/refund columns of existing tables.
-- ---------------------------------------------------------------------------------------------
INSERT INTO finance.change_kind
    (kind, table_name, op, pk_cols, phase, unjournaled, columns, updatable_cols, journal_cols) VALUES
    ('refund_insert', 'refund', 'insert', '{refund_id}', 1, false,
        '{refund_id,payer_account_id,operation_id,requested_micro_dollars,state,requested_at_ms,released_allocations}',
        '{}',
        '{refund_id,payer_account_id,operation_id,requested_micro_dollars,state,requested_at_ms,released_allocations}'),
    ('refund_update', 'refund', 'update', '{refund_id}', 2, false, '{}',
        '{state,released_allocations}',
        '{refund_id,state,released_allocations}'),
    ('refund_lot_insert', 'refund_lot', 'insert', '{refund_lot_id}', 1, false,
        '{refund_lot_id,refund_id,lot_id,principal_micro_dollars,tax_micro_dollars,processor_refund_ref,payout_method,state,updated_at_ms}',
        '{}',
        '{refund_lot_id,refund_id,lot_id,principal_micro_dollars,tax_micro_dollars,processor_refund_ref,payout_method,state,updated_at_ms}'),
    ('refund_lot_update', 'refund_lot', 'update', '{refund_lot_id}', 2, false, '{}',
        '{state,processor_refund_ref,object_revision,updated_at_ms}',
        '{refund_lot_id,state,processor_refund_ref,object_revision,updated_at_ms}'),
    ('refund_outbox_insert', 'refund_outbox', 'insert', '{refund_lot_id}', 1, false,
        '{refund_lot_id,next_attempt_at_ms}', '{}', '{refund_lot_id,next_attempt_at_ms}'),
    ('refund_outbox_update', 'refund_outbox', 'update', '{refund_lot_id}', 2, false, '{}',
        '{claimed_by,lease_until_ms,attempts,next_attempt_at_ms}',
        '{refund_lot_id,claimed_by,lease_until_ms,attempts,next_attempt_at_ms}'),
    ('dispute_insert', 'dispute', 'insert', '{processor,dispute_ref}', 1, false,
        '{processor,dispute_ref,lot_id,disputed_micro_dollars,status,fee_micro_dollars,opened_at_ms,closed_at_ms}',
        '{}',
        '{processor,dispute_ref,lot_id,disputed_micro_dollars,status,fee_micro_dollars,opened_at_ms,closed_at_ms}'),
    ('dispute_update', 'dispute', 'update', '{processor,dispute_ref}', 2, false, '{}',
        '{status,closed_at_ms}', '{processor,dispute_ref,status,closed_at_ms}'),
    ('processor_payment_state_update', 'processor_payment_state', 'update',
        '{processor,payment_ref}', 2, false, '{}',
        '{dispute_ref,dispute_status,disputed_micro_dollars,refunded_total_micro_dollars,object_revision,applied_at_ms}',
        '{processor,payment_ref,dispute_ref,dispute_status,disputed_micro_dollars,refunded_total_micro_dollars,object_revision,applied_at_ms}'),
    ('funding_lot_refund_update', 'funding_lot', 'update', '{lot_id}', 2, false, '{}',
        '{refunded_micro_dollars}', '{lot_id,refunded_micro_dollars}'),
    ('funding_lot_dispute_update', 'funding_lot', 'update', '{lot_id}', 2, false, '{}',
        '{state,charged_back_micro_dollars,extinguished_micro_dollars}',
        '{lot_id,state,charged_back_micro_dollars,extinguished_micro_dollars}'),
    ('provider_attempt_outcome_update', 'provider_attempt', 'update', '{attempt_id}', 2, false, '{}',
        '{dispatch_state,customer_state,supplier_state,supplier_outcome,usage_source,supplier_cost_micro_dollars,usability,usability_class,result_at_ms,customer_released_at_ms,settled_at_ms}',
        '{attempt_id,dispatch_state,customer_state,supplier_state,supplier_outcome,usage_source,supplier_cost_micro_dollars,usability,usability_class,result_at_ms,customer_released_at_ms,settled_at_ms}');

-- Event kinds: which changes and entries each refund/dispute/invalid-output event may carry.
INSERT INTO finance.event_kind (event_kind, carries_entries, journaled, allowed_changes, allowed_entries) VALUES
    ('refund_requested', true, true,
        '{refund_insert,refund_lot_insert,refund_outbox_insert,key_create,operation_receipt_insert}',
        '{refund_requested}'),
    ('refund_paid', true, true,
        '{refund_lot_update,refund_update,funding_lot_refund_update,processor_payment_state_update,key_create}',
        '{refund_paid}'),
    ('refund_unknown', false, true,
        '{refund_lot_update,refund_outbox_update}', '{}'),
    ('refund_failed', false, true,
        '{refund_lot_update,refund_outbox_update}', '{}'),
    ('refund_returned', true, true,
        '{refund_lot_update,refund_update}', '{refund_returned_to_credit}'),
    ('dispute_opened', true, true,
        '{dispute_insert,funding_lot_dispute_update,allocation_update,refund_lot_update,processor_payment_state_update,key_create}',
        '{dispute_freeze,dispute_withheld}'),
    ('dispute_closed', true, true,
        '{dispute_update,funding_lot_dispute_update,refund_lot_update,processor_payment_state_update,key_create}',
        '{dispute_won,dispute_lost}'),
    ('invalid_output_settled', true, true,
        '{reservation_update,reservation_share_update,provider_attempt_outcome_update,funding_lot_update,allocation_update,settlement_fact_insert,company_loss_insert,work_item_upsert,work_item_complete,key_create}',
        '{company_loss}');

INSERT INTO finance.event_update_cols (event_kind, kind, cols) VALUES
    ('refund_paid', 'refund_lot_update', '{state,processor_refund_ref,object_revision,updated_at_ms}'),
    ('refund_paid', 'refund_update', '{state}'),
    ('refund_paid', 'funding_lot_refund_update', '{refunded_micro_dollars}'),
    ('refund_paid', 'processor_payment_state_update',
        '{refunded_total_micro_dollars,object_revision,applied_at_ms}'),
    ('refund_unknown', 'refund_lot_update', '{state,object_revision,updated_at_ms}'),
    ('refund_unknown', 'refund_outbox_update', '{attempts,next_attempt_at_ms}'),
    ('refund_failed', 'refund_lot_update', '{state,object_revision,updated_at_ms}'),
    ('refund_failed', 'refund_outbox_update', '{attempts,next_attempt_at_ms}'),
    ('refund_returned', 'refund_lot_update', '{state,object_revision,updated_at_ms}'),
    ('refund_returned', 'refund_update', '{state}'),
    ('dispute_opened', 'funding_lot_dispute_update', '{state}'),
    ('dispute_opened', 'allocation_update', '{returned_micro_dollars}'),
    ('dispute_opened', 'refund_lot_update', '{state,object_revision,updated_at_ms}'),
    ('dispute_opened', 'processor_payment_state_update',
        '{dispute_ref,dispute_status,disputed_micro_dollars,object_revision,applied_at_ms}'),
    ('dispute_closed', 'dispute_update', '{status,closed_at_ms}'),
    ('dispute_closed', 'funding_lot_dispute_update',
        '{state,charged_back_micro_dollars,extinguished_micro_dollars}'),
    ('dispute_closed', 'refund_lot_update', '{state,object_revision,updated_at_ms}'),
    ('dispute_closed', 'processor_payment_state_update',
        '{dispute_status,object_revision,applied_at_ms}'),
    ('invalid_output_settled', 'reservation_update', '{state,charged_micro_dollars,released_micro_dollars}'),
    ('invalid_output_settled', 'reservation_share_update', '{charged_micro_dollars,released_micro_dollars}'),
    ('invalid_output_settled', 'provider_attempt_outcome_update',
        '{dispatch_state,customer_state,supplier_state,supplier_outcome,usage_source,supplier_cost_micro_dollars,usability,usability_class,result_at_ms,customer_released_at_ms,settled_at_ms}'),
    ('invalid_output_settled', 'funding_lot_update', '{consumed_micro_dollars,extinguished_micro_dollars}'),
    ('invalid_output_settled', 'allocation_update', '{charged_micro_dollars,returned_micro_dollars}');

-- Registry correction (FIN-05): a release into a frozen lot writes the allocation's
-- charged and returned totals in one update, as settlement already could; the release
-- event's column list named only the returned side, so that release always refused.
DROP TRIGGER event_update_cols_append_only ON finance.event_update_cols;
UPDATE finance.event_update_cols
   SET cols = cols || ARRAY['charged_micro_dollars']
 WHERE event_kind = 'release_applied' AND kind = 'allocation_update'
   AND NOT ('charged_micro_dollars' = ANY (cols));
CREATE TRIGGER event_update_cols_append_only BEFORE UPDATE OR DELETE ON finance.event_update_cols
    FOR EACH ROW EXECUTE FUNCTION finance.refuse_change();

-- `apply_change` compare-and-set needs a revision on every updated table.
ALTER TABLE finance.refund_outbox ADD COLUMN revision bigint NOT NULL DEFAULT 1;

-- Generated apply functions for the new kinds, and rebuilt insert/update dispatchers.
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
            EXECUTE format($f$CREATE OR REPLACE FUNCTION finance.apply_%1$I(p_row jsonb) RETURNS void
                LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $body$
                BEGIN
                    INSERT INTO finance.%2$I (%3$s) SELECT %4$s FROM jsonb_populate_record(NULL::finance.%2$I, p_row) r;
                END $body$$f$, k.kind, k.table_name, v_cols, v_exprs);
            v_apply := v_apply || format(E'        WHEN %L THEN PERFORM finance.apply_%I(p_row);\n', k.kind, k.kind);
        ELSE
            SELECT string_agg(format('%1$I = CASE WHEN p_row ? %2$L THEN r.%1$I ELSE t.%1$I END', u.c, u.c), ', ' ORDER BY u.ord)
              INTO v_sets FROM unnest(k.updatable_cols) WITH ORDINALITY AS u(c, ord);
            SELECT string_agg(format('t.%1$I = r.%1$I', c), ' AND ') INTO v_where FROM unnest(k.pk_cols) c;
            EXECUTE format($f$CREATE OR REPLACE FUNCTION finance.apply_%1$I(p_row jsonb, p_expected bigint) RETURNS bigint
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
    EXECUTE format($f$CREATE OR REPLACE FUNCTION finance.apply_insert(p_kind text, p_row jsonb) RETURNS void
        LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $body$
        BEGIN
            CASE p_kind
%s                ELSE RAISE EXCEPTION 'no insert function for change kind %%', p_kind USING ERRCODE = 'FN003';
            END CASE;
        END $body$$f$, replace(v_apply, E'        WHEN', E'                WHEN'));
    EXECUTE format($f$CREATE OR REPLACE FUNCTION finance.apply_update(p_kind text, p_row jsonb, p_expected bigint) RETURNS bigint
        LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $body$
        BEGIN
            CASE p_kind
%s                ELSE RAISE EXCEPTION 'no update function for change kind %%', p_kind USING ERRCODE = 'FN003';
            END CASE;
        END $body$$f$, replace(v_update, E'        WHEN', E'                WHEN'));
END $generate$;

-- ---------------------------------------------------------------------------------------------
-- Anchors for refund_lot and dispute entries: the payer stream of the lot the entry's lines name.
-- ---------------------------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION finance.anchor_expectation(p_anchor text, p_entry jsonb, p_event jsonb) RETURNS jsonb
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
    ELSIF p_anchor IN ('refund_lot', 'dispute') THEN
        IF p_entry ->> 'kind' = 'dispute_withheld' THEN
            RETURN '{}'::jsonb;
        END IF;
        SELECT l.payer_account_id INTO v_payer FROM finance.funding_lot l
         WHERE l.lot_id = (SELECT finance.hexbytes(line -> 'lot')
                             FROM jsonb_array_elements(p_entry -> 'lines') line
                            WHERE line -> 'lot' <> 'null'::jsonb LIMIT 1);
        IF v_payer IS NULL THEN RETURN NULL; END IF;
        RETURN jsonb_build_object('stream_kind', 'payer', 'stream_id', encode(v_payer, 'hex'),
            'campaign_id', NULL, 'session_id', NULL, 'attempt_id', NULL, 'reservation_id', NULL, 'price_version_id', NULL);
    END IF;
    RETURN NULL;
END $$;

-- E10: refund and dispute entries equal the refund-lot rows they create or settle.
CREATE FUNCTION finance.eq_e10(p jsonb) RETURNS void
LANGUAGE plpgsql STABLE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_debit bigint; v_credit bigint; v_rows bigint;
BEGIN
    IF finance.event_has_kind(p, ARRAY['refund_requested']) THEN
        SELECT coalesce(sum(amount) FILTER (WHERE code = 2110), 0),
               -coalesce(sum(amount) FILTER (WHERE code = 2500), 0)
          INTO v_debit, v_credit FROM finance.event_lines(p) WHERE entry_kind = 'refund_requested';
        SELECT coalesce(sum((c -> 'row' ->> 'principal_micro_dollars')::bigint), 0) INTO v_rows
          FROM finance.event_changes(p, 'refund_lot_insert') c;
        IF v_debit <> v_credit OR v_debit <> v_rows THEN
            PERFORM finance.eq_fail('E10', format('refund requested debits %s credits %s lots %s', v_debit, v_credit, v_rows));
        END IF;
    END IF;
    IF finance.event_has_kind(p, ARRAY['refund_paid']) THEN
        SELECT coalesce(sum(amount) FILTER (WHERE code = 2500), 0),
               -coalesce(sum(amount) FILTER (WHERE code = 1100), 0)
          INTO v_debit, v_credit FROM finance.event_lines(p) WHERE entry_kind = 'refund_paid';
        IF v_debit = 0 OR v_credit < v_debit THEN
            PERFORM finance.eq_fail('E10', format('refund paid debits %s clearing %s', v_debit, v_credit));
        END IF;
    END IF;
    IF finance.event_has_kind(p, ARRAY['dispute_freeze']) THEN
        SELECT coalesce(sum(amount) FILTER (WHERE code IN (2110, 2120)), 0),
               -coalesce(sum(amount) FILTER (WHERE code = 2140), 0)
          INTO v_debit, v_credit FROM finance.event_lines(p) WHERE entry_kind = 'dispute_freeze';
        IF v_debit <> v_credit THEN
            PERFORM finance.eq_fail('E10', format('dispute freeze debits %s frozen %s', v_debit, v_credit));
        END IF;
    END IF;
END $$;

CREATE OR REPLACE FUNCTION finance.check_equations(p jsonb) RETURNS void
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
    PERFORM finance.eq_e10(p);
END $$;

-- ---------------------------------------------------------------------------------------------
-- Lock flows (ranks of 19.1: payer, payment state, session, allocation, lot, then refund rows).
-- ---------------------------------------------------------------------------------------------
CREATE FUNCTION finance.lock_flow_refund_request(p_args jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_payer bytea := finance.hexbytes(p_args -> 'payer');
BEGIN
    PERFORM pg_advisory_xact_lock(hashtextextended('finance.payer:' || encode(v_payer, 'hex'), 0));
    PERFORM finance.note_locks('payer', ARRAY(SELECT encode(account_id, 'hex') FROM finance.payer
                                               WHERE account_id = v_payer FOR UPDATE));
    PERFORM finance.note_locks('funding_lot', ARRAY(SELECT encode(lot_id, 'hex') FROM finance.funding_lot
                                                     WHERE payer_account_id = v_payer ORDER BY lot_id FOR UPDATE));
    RETURN jsonb_build_object(
        'payer', (SELECT to_jsonb(p) FROM finance.payer p WHERE account_id = v_payer),
        'lots', coalesce((SELECT jsonb_agg(jsonb_build_object('lot', to_jsonb(l) - 'billing_country' - 'billing_region',
                    'available', coalesce((SELECT b.natural_micro_dollars FROM finance.account_key k
                                   JOIN finance.account_balance b ON b.key_id = k.key_id
                                  WHERE k.lot_id = l.lot_id AND k.code = 2110), 0))
                    ORDER BY l.created_at_ms DESC, l.lot_id DESC)
                   FROM finance.funding_lot l WHERE l.payer_account_id = v_payer), '[]'::jsonb),
        'existing_refund', (SELECT to_jsonb(r) FROM finance.refund r
                             WHERE r.payer_account_id = v_payer
                               AND r.operation_id = finance.hexbytes(p_args -> 'operation')),
        'receipt', (SELECT to_jsonb(r) FROM finance.operation_receipt r WHERE r.namespace = 'refund'
                     AND r.principal = v_payer AND r.operation_id = finance.hexbytes(p_args -> 'operation')));
END $$;

CREATE FUNCTION finance.lock_flow_refund_lot(p_args jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_refund_lot bytea := finance.hexbytes(p_args -> 'refund_lot');
    v_payer bytea;
    v_lot bytea;
BEGIN
    SELECT r.payer_account_id, rl.lot_id INTO v_payer, v_lot
      FROM finance.refund_lot rl JOIN finance.refund r ON r.refund_id = rl.refund_id
     WHERE rl.refund_lot_id = v_refund_lot;
    IF v_payer IS NULL THEN
        RETURN jsonb_build_object('missing', true);
    END IF;
    PERFORM finance.note_locks('payer', ARRAY(SELECT encode(account_id, 'hex') FROM finance.payer
                                               WHERE account_id = v_payer FOR UPDATE));
    PERFORM finance.note_locks('funding_lot', ARRAY(SELECT encode(lot_id, 'hex') FROM finance.funding_lot
                                                     WHERE lot_id = v_lot FOR UPDATE));
    PERFORM finance.note_locks('refund', ARRAY(SELECT encode(r.refund_id, 'hex') FROM finance.refund r
                  JOIN finance.refund_lot rl ON rl.refund_id = r.refund_id
                 WHERE rl.refund_lot_id = v_refund_lot FOR UPDATE));
    PERFORM finance.note_locks('refund_lot', ARRAY(SELECT encode(refund_lot_id, 'hex') FROM finance.refund_lot
                                                    WHERE refund_lot_id = v_refund_lot FOR UPDATE));
    PERFORM finance.note_locks('refund_outbox', ARRAY(SELECT encode(refund_lot_id, 'hex') FROM finance.refund_outbox
                                                    WHERE refund_lot_id = v_refund_lot FOR UPDATE));
    PERFORM finance.note_locks('processor_payment_state', ARRAY(SELECT s.processor || '|' || s.payment_ref
                  FROM finance.processor_payment_state s JOIN finance.funding_lot l ON l.lot_id = s.lot_id
                 WHERE l.lot_id = v_lot FOR UPDATE));
    RETURN jsonb_build_object(
        'refund', (SELECT to_jsonb(r) FROM finance.refund r
                    JOIN finance.refund_lot rl ON rl.refund_id = r.refund_id
                   WHERE rl.refund_lot_id = v_refund_lot),
        'refund_lot', (SELECT to_jsonb(rl) FROM finance.refund_lot rl WHERE rl.refund_lot_id = v_refund_lot),
        'lot', (SELECT to_jsonb(l) - 'billing_country' - 'billing_region' FROM finance.funding_lot l WHERE l.lot_id = v_lot),
        'siblings', coalesce((SELECT jsonb_agg(to_jsonb(s) ORDER BY s.refund_lot_id) FROM finance.refund_lot s
                               WHERE s.refund_id = (SELECT refund_id FROM finance.refund_lot WHERE refund_lot_id = v_refund_lot)), '[]'::jsonb),
        'payment_state', (SELECT to_jsonb(s) FROM finance.processor_payment_state s
                           JOIN finance.funding_lot l ON l.lot_id = s.lot_id WHERE l.lot_id = v_lot),
        'outbox', (SELECT to_jsonb(o) FROM finance.refund_outbox o WHERE o.refund_lot_id = v_refund_lot));
END $$;

CREATE FUNCTION finance.lock_flow_dispute(p_args jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_lot bytea := finance.hexbytes(p_args -> 'lot');
    v_payer bytea;
BEGIN
    SELECT payer_account_id INTO v_payer FROM finance.funding_lot WHERE lot_id = v_lot;
    IF v_payer IS NULL THEN
        RETURN jsonb_build_object('missing', true);
    END IF;
    PERFORM finance.note_locks('payer', ARRAY(SELECT encode(account_id, 'hex') FROM finance.payer
                                               WHERE account_id = v_payer FOR UPDATE));
    PERFORM finance.note_locks('funding_lot', ARRAY(SELECT encode(lot_id, 'hex') FROM finance.funding_lot
                                                     WHERE lot_id = v_lot FOR UPDATE));
    PERFORM finance.note_locks('allocation', ARRAY(SELECT encode(a.allocation_id, 'hex') FROM finance.allocation a
                  JOIN finance.allocation_lot al ON al.allocation_id = a.allocation_id
                 WHERE al.lot_id = v_lot ORDER BY a.allocation_id FOR UPDATE OF a));
    PERFORM finance.note_locks('refund_lot', ARRAY(SELECT encode(refund_lot_id, 'hex') FROM finance.refund_lot
                                                    WHERE lot_id = v_lot ORDER BY refund_lot_id FOR UPDATE));
    PERFORM finance.note_locks('dispute', ARRAY(SELECT processor || '|' || dispute_ref FROM finance.dispute
                 WHERE processor = p_args ->> 'processor' AND dispute_ref = p_args ->> 'dispute_ref' FOR UPDATE));
    PERFORM finance.note_locks('processor_payment_state', ARRAY(SELECT s.processor || '|' || s.payment_ref
                  FROM finance.processor_payment_state s JOIN finance.funding_lot l ON l.lot_id = s.lot_id
                 WHERE l.lot_id = v_lot FOR UPDATE));
    RETURN jsonb_build_object(
        'lot', (SELECT to_jsonb(l) - 'billing_country' - 'billing_region' FROM finance.funding_lot l WHERE l.lot_id = v_lot),
        'available', coalesce((SELECT b.natural_micro_dollars FROM finance.account_key k
                        JOIN finance.account_balance b ON b.key_id = k.key_id
                       WHERE k.lot_id = v_lot AND k.code = 2110), 0),
        'frozen', coalesce((SELECT b.natural_micro_dollars FROM finance.account_key k
                        JOIN finance.account_balance b ON b.key_id = k.key_id
                       WHERE k.lot_id = v_lot AND k.code = 2140), 0),
        'allocations', coalesce((SELECT jsonb_agg(jsonb_build_object('allocation', to_jsonb(a),
                    'allocated', coalesce((SELECT b.natural_micro_dollars FROM finance.account_key k
                                   JOIN finance.account_balance b ON b.key_id = k.key_id
                                  WHERE k.allocation_id = a.allocation_id AND k.lot_id = v_lot AND k.code = 2120), 0))
                    ORDER BY a.allocation_id)
                   FROM finance.allocation a JOIN finance.allocation_lot al ON al.allocation_id = a.allocation_id
                  WHERE al.lot_id = v_lot), '[]'::jsonb),
        'refund_lots', coalesce((SELECT jsonb_agg(to_jsonb(rl) ORDER BY rl.updated_at_ms DESC, rl.refund_lot_id DESC)
                   FROM finance.refund_lot rl WHERE rl.lot_id = v_lot), '[]'::jsonb),
        'dispute', (SELECT to_jsonb(d) FROM finance.dispute d
                     WHERE d.processor = p_args ->> 'processor' AND d.dispute_ref = p_args ->> 'dispute_ref'),
        'payment_state', (SELECT to_jsonb(s) FROM finance.processor_payment_state s
                           JOIN finance.funding_lot l ON l.lot_id = s.lot_id WHERE l.lot_id = v_lot));
END $$;

CREATE OR REPLACE FUNCTION finance.lock_flow(p_flow text, p_args jsonb) RETURNS jsonb
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
    ELSIF p_flow = 'refund_request' THEN
        RETURN finance.lock_flow_refund_request(p_args);
    ELSIF p_flow IN ('refund_lot', 'refund_pay', 'refund_observe') THEN
        RETURN finance.lock_flow_refund_lot(p_args);
    ELSIF p_flow IN ('dispute_open', 'dispute_close') THEN
        RETURN finance.lock_flow_dispute(p_args);
    ELSIF p_flow = 'attempt' THEN
        PERFORM finance.note_locks('provider_attempt', ARRAY(SELECT encode(attempt_id, 'hex') FROM finance.provider_attempt
                                                              WHERE attempt_id = v_attempt FOR UPDATE));
        RETURN jsonb_build_object('attempt', (SELECT to_jsonb(a) - 'provider_request_ref' FROM finance.provider_attempt a WHERE attempt_id = v_attempt));
    ELSIF p_flow = 'quote' THEN
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

-- The 0023 tables join the write guard: no row is written outside commit_event.
DO $$
DECLARE
    t text;
BEGIN
    FOREACH t IN ARRAY ARRAY['refund', 'refund_lot', 'refund_outbox', 'dispute'] LOOP
        EXECUTE format('CREATE TRIGGER %I BEFORE INSERT OR UPDATE OR DELETE ON finance.%I FOR EACH STATEMENT EXECUTE FUNCTION finance.write_guard()',
                       t || '_write_guard', t);
    END LOOP;
END $$;

GRANT INSERT ON finance.refund, finance.refund_lot, finance.refund_outbox, finance.dispute TO df_finance_definer;
GRANT UPDATE (state, released_allocations, revision) ON finance.refund TO df_finance_definer;
GRANT UPDATE (state, processor_refund_ref, object_revision, updated_at_ms, revision) ON finance.refund_lot TO df_finance_definer;
GRANT UPDATE (claimed_by, lease_until_ms, attempts, next_attempt_at_ms, revision) ON finance.refund_outbox TO df_finance_definer;
GRANT UPDATE (status, closed_at_ms, revision) ON finance.dispute TO df_finance_definer;
GRANT UPDATE (dispute_ref, dispute_status, disputed_micro_dollars, refunded_total_micro_dollars, object_revision, applied_at_ms, revision)
    ON finance.processor_payment_state TO df_finance_definer;
GRANT UPDATE (refunded_micro_dollars, state, charged_back_micro_dollars, extinguished_micro_dollars, revision)
    ON finance.funding_lot TO df_finance_definer;
GRANT UPDATE (dispatch_state, customer_state, supplier_state, supplier_outcome, usage_source,
    supplier_cost_micro_dollars, usability, usability_class, result_at_ms, customer_released_at_ms,
    settled_at_ms, revision) ON finance.provider_attempt TO df_finance_definer;
ALTER FUNCTION finance.lock_flow(text, jsonb) OWNER TO df_finance_definer;
ALTER FUNCTION finance.lock_flow_refund_request(jsonb) OWNER TO df_finance_definer;
ALTER FUNCTION finance.lock_flow_refund_lot(jsonb) OWNER TO df_finance_definer;
ALTER FUNCTION finance.lock_flow_dispute(jsonb) OWNER TO df_finance_definer;
REVOKE EXECUTE ON ALL FUNCTIONS IN SCHEMA finance FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.lock_flow(text, jsonb), finance.commit_event(jsonb),
    finance.sweep_flow_state(), finance.extend_period_horizon(integer), finance.session_charge_drift()
    TO df_finance;
GRANT EXECUTE ON FUNCTION finance.now_ms(), finance.period_for(bigint), finance.entry_json(bytea),
    finance.entry_canonical_v1(jsonb), finance.entry_hash(smallint, bytea, jsonb), finance.hexbytes(jsonb)
    TO df_finance;
GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA finance TO df_finance_definer;


