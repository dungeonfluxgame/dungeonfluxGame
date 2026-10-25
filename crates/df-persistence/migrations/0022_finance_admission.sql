-- Finance admission and settlement rows (planning/finance-ledger-research.md sections 9.3, 11,
-- 12, 17.3): quotes, reservations and their shares, provider attempts with their three state
-- dimensions, usage events, settlement facts, work items, and the commit-time triggers that tie
-- the subledgers to the account keys.

CREATE TABLE finance.quote (
    quote_id bytea PRIMARY KEY CHECK (octet_length(quote_id) = 16),
    session_id bytea NOT NULL REFERENCES finance.session_funding,
    request_digest bytea NOT NULL CHECK (octet_length(request_digest) = 32),
    price_version_id bytea NOT NULL REFERENCES finance.price_version,
    work_class text NOT NULL CHECK (work_class IN ('essential', 'optional', 'premium')),
    retail_estimate_micro_dollars bigint NOT NULL CHECK (retail_estimate_micro_dollars >= 0),
    retail_maximum_micro_dollars bigint NOT NULL CHECK (retail_maximum_micro_dollars >= 0),
    supplier_maximum_micro_dollars bigint NOT NULL CHECK (supplier_maximum_micro_dollars >= 0),
    created_at_ms bigint NOT NULL,
    expires_at_ms bigint NOT NULL,
    state text NOT NULL CHECK (state IN ('open', 'approved', 'declined', 'expired', 'used')),
    used_by_reservation bytea,
    revision bigint NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX quote_open_unique ON finance.quote (session_id, request_digest, price_version_id)
    WHERE state = 'open';
CREATE TABLE finance.quote_approval (
    quote_id bytea NOT NULL REFERENCES finance.quote,
    allocation_id bytea NOT NULL REFERENCES finance.allocation,
    decision text NOT NULL CHECK (decision IN ('approved', 'declined')),
    consent_terms_version integer NOT NULL,
    at_ms bigint NOT NULL,
    PRIMARY KEY (quote_id, allocation_id)
);

CREATE TABLE finance.reservation (
    reservation_id bytea PRIMARY KEY CHECK (octet_length(reservation_id) = 16),
    session_id bytea REFERENCES finance.session_funding,
    payer_kind text NOT NULL CHECK (payer_kind IN ('table', 'company')),
    company_class text,
    kind text NOT NULL CHECK (kind IN ('provider', 'time')),
    operation_id bytea NOT NULL CHECK (octet_length(operation_id) = 16),
    quote_id bytea REFERENCES finance.quote,
    price_version_id bytea NOT NULL REFERENCES finance.price_version,
    retail_maximum_micro_dollars bigint NOT NULL CHECK (retail_maximum_micro_dollars >= 0),
    admitted_at_ms bigint NOT NULL,
    state text NOT NULL CHECK (state IN ('held', 'charged', 'released')),
    charged_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (charged_micro_dollars >= 0),
    released_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (released_micro_dollars >= 0),
    revision bigint NOT NULL DEFAULT 1,
    CHECK (charged_micro_dollars + released_micro_dollars <= retail_maximum_micro_dollars),
    CHECK (state = 'held' OR charged_micro_dollars + released_micro_dollars = retail_maximum_micro_dollars),
    CHECK ((payer_kind = 'table') = (session_id IS NOT NULL)),
    CHECK ((payer_kind = 'company') = (company_class IS NOT NULL))
);
CREATE UNIQUE INDEX reservation_quote_unique ON finance.reservation (quote_id) WHERE quote_id IS NOT NULL;
CREATE INDEX reservation_session_held ON finance.reservation (session_id) WHERE state = 'held';

CREATE TABLE finance.reservation_share (
    reservation_id bytea NOT NULL REFERENCES finance.reservation,
    allocation_id bytea NOT NULL REFERENCES finance.allocation,
    lot_id bytea NOT NULL REFERENCES finance.funding_lot,
    class text NOT NULL CHECK (class IN ('purchased', 'promotional')),
    held_micro_dollars bigint NOT NULL CHECK (held_micro_dollars > 0),
    charged_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (charged_micro_dollars >= 0),
    released_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (released_micro_dollars >= 0),
    reversed_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (reversed_micro_dollars >= 0),
    revision bigint NOT NULL DEFAULT 1,
    PRIMARY KEY (reservation_id, allocation_id, lot_id),
    CHECK (charged_micro_dollars + released_micro_dollars <= held_micro_dollars),
    CHECK (reversed_micro_dollars <= charged_micro_dollars)
);
CREATE INDEX reservation_share_open ON finance.reservation_share (allocation_id, lot_id)
    WHERE charged_micro_dollars + released_micro_dollars < held_micro_dollars;
CREATE INDEX reservation_share_key ON finance.reservation_share (allocation_id, lot_id);

CREATE TABLE finance.provider_attempt (
    attempt_id bytea PRIMARY KEY CHECK (octet_length(attempt_id) = 16),
    reservation_id bytea NOT NULL UNIQUE REFERENCES finance.reservation,
    effect_id bytea NOT NULL CHECK (octet_length(effect_id) = 16),
    campaign_id bytea,
    account_id bytea,
    tenant_id bytea,
    provider text NOT NULL,
    model text NOT NULL,
    route text NOT NULL,
    media_asset_id bytea,
    supplier_maximum_micro_dollars bigint NOT NULL CHECK (supplier_maximum_micro_dollars >= 0),
    supplier_exposure_micro_dollars bigint NOT NULL CHECK (supplier_exposure_micro_dollars >= 0),
    dispatch_state text NOT NULL CHECK (dispatch_state IN ('reserved', 'dispatch_prepared', 'sent',
        'result_known', 'outcome_unknown', 'verified_unsent', 'cancelled_unsent', 'failed_unknown')),
    customer_state text NOT NULL CHECK (customer_state IN ('held', 'charged', 'released', 'credited')),
    supplier_state text NOT NULL CHECK (supplier_state IN ('exposure_open', 'cost_recorded',
        'exposure_released', 'absorbed_worst_case')),
    dispatcher_instance_id bytea,
    supplier_outcome text CHECK (supplier_outcome IN ('billed', 'not_billed', 'unknown')),
    usage_source text CHECK (usage_source IN ('reported', 'estimated')),
    supplier_cost_micro_dollars bigint,
    usability text CHECK (usability IN ('usable', 'no_usable_output')),
    usability_class text,
    admitted_at_ms bigint NOT NULL,
    prepared_at_ms bigint,
    sent_at_ms bigint,
    result_at_ms bigint,
    customer_released_at_ms bigint,
    settled_at_ms bigint,
    fence_recovery_epoch bigint,
    fence_claim_seq bigint,
    fence_placement_id bytea,
    revision bigint NOT NULL DEFAULT 1,
    CHECK ((campaign_id IS NULL) <> (account_id IS NULL))
);
CREATE INDEX provider_attempt_open ON finance.provider_attempt (dispatch_state, admitted_at_ms)
    WHERE dispatch_state IN ('reserved', 'dispatch_prepared', 'sent', 'outcome_unknown');

-- The arrows of 11.3; anything else is refused.
CREATE FUNCTION finance.attempt_arrows() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, finance, pg_temp AS $$
BEGIN
    IF NEW.dispatch_state <> OLD.dispatch_state AND NOT (
        (OLD.dispatch_state, NEW.dispatch_state) IN (
            ('reserved', 'dispatch_prepared'), ('reserved', 'cancelled_unsent'),
            ('dispatch_prepared', 'sent'), ('dispatch_prepared', 'cancelled_unsent'),
            ('sent', 'result_known'), ('sent', 'outcome_unknown'), ('sent', 'verified_unsent'),
            ('sent', 'failed_unknown'), ('outcome_unknown', 'result_known'),
            ('outcome_unknown', 'failed_unknown'))) THEN
        RAISE EXCEPTION 'attempt dispatch state may not move % -> %', OLD.dispatch_state, NEW.dispatch_state
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF NEW.customer_state <> OLD.customer_state AND NOT (
        (OLD.customer_state, NEW.customer_state) IN (
            ('held', 'charged'), ('held', 'released'), ('charged', 'credited'))) THEN
        RAISE EXCEPTION 'attempt customer state may not move % -> %', OLD.customer_state, NEW.customer_state
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF NEW.supplier_state <> OLD.supplier_state AND NOT (
        (OLD.supplier_state, NEW.supplier_state) IN (
            ('exposure_open', 'cost_recorded'), ('exposure_open', 'exposure_released'),
            ('exposure_open', 'absorbed_worst_case'))) THEN
        RAISE EXCEPTION 'attempt supplier state may not move % -> %', OLD.supplier_state, NEW.supplier_state
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER provider_attempt_arrows BEFORE UPDATE ON finance.provider_attempt
    FOR EACH ROW EXECUTE FUNCTION finance.attempt_arrows();

CREATE TABLE finance.usage_event (
    usage_event_id bytea PRIMARY KEY CHECK (octet_length(usage_event_id) = 16),
    attempt_id bytea UNIQUE REFERENCES finance.provider_attempt,
    meter_session_id bytea,
    meter_segment_seq bigint,
    session_id bytea NOT NULL REFERENCES finance.session_funding,
    category text NOT NULL,
    quantity_text text NOT NULL,
    units jsonb NOT NULL DEFAULT '{}',
    retail_micro_dollars bigint NOT NULL CHECK (retail_micro_dollars >= 0),
    supplier_micro_dollars bigint NOT NULL CHECK (supplier_micro_dollars >= 0),
    supplier_purchased_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (supplier_purchased_micro_dollars >= 0),
    supplier_promotional_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (supplier_promotional_micro_dollars >= 0),
    reversed_micro_dollars bigint NOT NULL DEFAULT 0,
    reclassified_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (reclassified_micro_dollars >= 0),
    price_version_id bytea NOT NULL REFERENCES finance.price_version,
    entry_id bytea REFERENCES finance.journal_entry,
    settled_at_ms bigint NOT NULL,
    revision bigint NOT NULL DEFAULT 1,
    CHECK (reversed_micro_dollars >= 0 AND reversed_micro_dollars <= retail_micro_dollars),
    CHECK ((attempt_id IS NULL) <> (meter_segment_seq IS NULL)),
    UNIQUE (meter_session_id, meter_segment_seq)
);
CREATE INDEX usage_event_session ON finance.usage_event (session_id, settled_at_ms);
CREATE TABLE finance.usage_reversal (
    usage_event_id bytea NOT NULL REFERENCES finance.usage_event,
    operation_id bytea NOT NULL,
    micro_dollars bigint NOT NULL CHECK (micro_dollars > 0),
    reason text NOT NULL REFERENCES finance.entry_reason,
    entry_id bytea REFERENCES finance.journal_entry,
    PRIMARY KEY (usage_event_id, operation_id)
);

CREATE TABLE finance.meter_segment_priced (
    session_id bytea NOT NULL REFERENCES finance.session_funding,
    segment_seq bigint NOT NULL,
    priced_ms bigint NOT NULL,
    credited_ms bigint NOT NULL DEFAULT 0,
    charge_micro_dollars bigint NOT NULL,
    forgone_micro_dollars bigint NOT NULL DEFAULT 0,
    entry_id bytea REFERENCES finance.journal_entry,
    PRIMARY KEY (session_id, segment_seq)
);
CREATE TABLE finance.forgone_revenue (
    reason text NOT NULL CHECK (reason IN ('meter_unfunded', 'refunded_externally')),
    ref_id bytea NOT NULL,
    session_id bytea NOT NULL,
    micro_dollars bigint NOT NULL CHECK (micro_dollars > 0),
    PRIMARY KEY (reason, ref_id)
);
CREATE TABLE finance.company_loss (
    attempt_id bytea NOT NULL REFERENCES finance.provider_attempt,
    loss_class text NOT NULL,
    micro_dollars bigint NOT NULL CHECK (micro_dollars >= 0),
    entry_id bytea REFERENCES finance.journal_entry,
    PRIMARY KEY (attempt_id, loss_class)
);
CREATE TABLE finance.settlement_fact (
    attempt_id bytea PRIMARY KEY REFERENCES finance.provider_attempt,
    source text NOT NULL CHECK (source IN ('runtime', 'o2_outbox', 'deadline')),
    outcome text NOT NULL,
    reason text,
    outbox_seq bigint,
    recorded_at_ms bigint NOT NULL
);
CREATE TABLE finance.settlement_feed_position (
    consumer text PRIMARY KEY,
    last_outbox_seq bigint NOT NULL,
    updated_at_ms bigint NOT NULL
);
CREATE TABLE finance.attempt_outcome_event (
    event_seq bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    attempt_id bytea NOT NULL UNIQUE REFERENCES finance.provider_attempt,
    media_asset_id bytea,
    outcome text NOT NULL CHECK (outcome IN ('failed_unknown', 'cancelled_unsent')),
    reason text NOT NULL CHECK (reason IN ('unknown_outcome_failed', 'not_dispatched')),
    recorded_at_ms bigint NOT NULL
);

CREATE TABLE finance.work_item (
    work_id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    kind text NOT NULL,
    ref_id bytea NOT NULL,
    due_at_ms bigint NOT NULL,
    claimed_by bytea,
    lease_until_ms bigint,
    generation bigint NOT NULL DEFAULT 0,
    UNIQUE (kind, ref_id)
);
CREATE INDEX work_item_due ON finance.work_item (due_at_ms);

CREATE TABLE finance.company_exposure_counter (
    scope text NOT NULL,
    period text NOT NULL,
    used_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (used_micro_dollars >= 0),
    PRIMARY KEY (scope, period)
);

-- Commit-time triggers tying the reservation subledger to the reserved keys (17.3).
CREATE FUNCTION finance.check_reserved_key(p_allocation bytea, p_lot bytea) RETURNS void
LANGUAGE plpgsql SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_gap bigint;
BEGIN
    IF finance.already_checked('rk_' || encode(p_allocation, 'hex') || encode(p_lot, 'hex')) THEN
        RETURN;
    END IF;
    SELECT coalesce((SELECT sum(s.held_micro_dollars - s.charged_micro_dollars - s.released_micro_dollars)
                       FROM finance.reservation_share s
                      WHERE s.allocation_id = p_allocation AND s.lot_id = p_lot), 0)
           - coalesce((SELECT sum(b.natural_micro_dollars) FROM finance.account_key k
                        JOIN finance.account_balance b ON b.key_id = k.key_id
                       WHERE k.allocation_id = p_allocation AND k.lot_id = p_lot
                         AND k.code IN (2130, 2230)), 0)
      INTO v_gap;
    IF v_gap <> 0 THEN
        RAISE EXCEPTION 'reserved key of allocation % differs from its open shares by %',
            encode(p_allocation, 'hex'), v_gap USING ERRCODE = 'integrity_constraint_violation';
    END IF;
END $$;
CREATE FUNCTION finance.reserved_key_trigger() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, finance, pg_temp AS $$
BEGIN
    IF TG_TABLE_NAME = 'reservation_share' THEN
        PERFORM finance.check_reserved_key(NEW.allocation_id, NEW.lot_id);
    ELSE
        PERFORM finance.check_reserved_key(k.allocation_id, k.lot_id) FROM finance.account_key k
            WHERE k.key_id = NEW.key_id AND k.code IN (2130, 2230);
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER reservation_share_reserved AFTER INSERT OR UPDATE ON finance.reservation_share
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance.reserved_key_trigger();
CREATE CONSTRAINT TRIGGER account_balance_reserved AFTER UPDATE ON finance.account_balance
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance.reserved_key_trigger();

-- A reservation equals the sum of its shares in all three amounts.
CREATE FUNCTION finance.check_reservation_totals(p_reservation bytea) RETURNS void
LANGUAGE plpgsql SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    r finance.reservation%ROWTYPE;
    v_held bigint; v_charged bigint; v_released bigint;
BEGIN
    IF finance.already_checked('rv_' || encode(p_reservation, 'hex')) THEN
        RETURN;
    END IF;
    SELECT * INTO r FROM finance.reservation WHERE reservation_id = p_reservation;
    IF NOT FOUND THEN RETURN; END IF;
    SELECT coalesce(sum(held_micro_dollars), 0), coalesce(sum(charged_micro_dollars), 0),
           coalesce(sum(released_micro_dollars), 0)
      INTO v_held, v_charged, v_released
      FROM finance.reservation_share WHERE reservation_id = p_reservation;
    IF r.payer_kind = 'table' AND (v_held <> r.retail_maximum_micro_dollars
        OR v_charged <> r.charged_micro_dollars OR v_released <> r.released_micro_dollars) THEN
        RAISE EXCEPTION 'reservation % differs from the sum of its shares', encode(p_reservation, 'hex')
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
END $$;
CREATE FUNCTION finance.reservation_totals_trigger() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, finance, pg_temp AS $$
BEGIN
    PERFORM finance.check_reservation_totals(NEW.reservation_id);
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER reservation_totals AFTER INSERT OR UPDATE ON finance.reservation
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance.reservation_totals_trigger();
CREATE CONSTRAINT TRIGGER reservation_share_totals AFTER INSERT OR UPDATE ON finance.reservation_share
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance.reservation_totals_trigger();

-- A lot does not close with an open share or a non-zero key (the conservation function checks the
-- keys); an allocation may be closed only with no open share.
CREATE FUNCTION finance.check_lot_open_shares() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, finance, pg_temp AS $$
BEGIN
    IF NEW.state IN ('closed', 'forfeited') AND EXISTS (
        SELECT 1 FROM finance.reservation_share s
         WHERE s.lot_id = NEW.lot_id AND s.charged_micro_dollars + s.released_micro_dollars < s.held_micro_dollars) THEN
        RAISE EXCEPTION 'a lot cannot close with an open share' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER funding_lot_open_shares AFTER INSERT OR UPDATE ON finance.funding_lot
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance.check_lot_open_shares();

CREATE TRIGGER usage_reversal_append_only BEFORE UPDATE OR DELETE ON finance.usage_reversal
    FOR EACH ROW EXECUTE FUNCTION finance.refuse_change();
CREATE TRIGGER quote_approval_append_only BEFORE UPDATE OR DELETE ON finance.quote_approval
    FOR EACH ROW EXECUTE FUNCTION finance.refuse_change();
