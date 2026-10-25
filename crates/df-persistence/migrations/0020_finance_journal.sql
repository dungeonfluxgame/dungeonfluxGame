-- Finance journal (planning/finance-ledger-research.md sections 5 and 17.1, F round 1).
-- Schema `finance` in the one hosted database. Ids are 16-byte bytea, amounts are signed bigint
-- micro-dollars (+ debit, - credit), times are milliseconds since the Unix epoch. Posting reaches
-- these tables only through the definer functions of 0024 (`lock_flow`, `commit_event`); every
-- constraint here stays in force even if a posting function had a defect.

CREATE SCHEMA IF NOT EXISTS finance;
REVOKE ALL ON SCHEMA finance FROM PUBLIC;

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'df_finance_definer') THEN
        CREATE ROLE df_finance_definer NOLOGIN;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'df_finance') THEN
        CREATE ROLE df_finance NOLOGIN;
    END IF;
END $$;

CREATE SEQUENCE finance.global_seq;

-- The database clock in milliseconds. There is no override: a test that needs another time
-- replaces this function in its own scratch database, so a service role cannot move the clock.
CREATE FUNCTION finance.now_ms() RETURNS bigint
LANGUAGE sql VOLATILE
AS $$ SELECT (extract(epoch FROM clock_timestamp()) * 1000)::bigint $$;

-- The pure helpers below name only pg_catalog objects and carry no search_path, so the planner
-- can inline them (a SET clause would make each call a function-manager round trip).
-- Canonical encoders (17.7 item 2): length-prefixed, big-endian; an optional value has a
-- presence byte, so an absent id and an all-zero id hash differently.
CREATE FUNCTION finance.enc_bytes(b bytea) RETURNS bytea
LANGUAGE sql IMMUTABLE
AS $$ SELECT int4send(octet_length(b)) || b $$;
CREATE FUNCTION finance.enc_text(t text) RETURNS bytea
LANGUAGE sql IMMUTABLE
AS $$ SELECT int4send(octet_length(convert_to(t, 'UTF8'))) || convert_to(t, 'UTF8') $$;
CREATE FUNCTION finance.enc_i64(n bigint) RETURNS bytea
LANGUAGE sql IMMUTABLE
AS $$ SELECT int8send(n) $$;
CREATE FUNCTION finance.enc_opt_bytes(b bytea) RETURNS bytea
LANGUAGE sql IMMUTABLE
AS $$ SELECT CASE WHEN b IS NULL THEN '\x00'::bytea ELSE '\x01'::bytea || int4send(octet_length(b)) || b END $$;
CREATE FUNCTION finance.enc_opt_text(t text) RETURNS bytea
LANGUAGE sql IMMUTABLE
AS $$ SELECT CASE WHEN t IS NULL THEN '\x00'::bytea
                  ELSE '\x01'::bytea || int4send(octet_length(convert_to(t, 'UTF8'))) || convert_to(t, 'UTF8') END $$;
CREATE FUNCTION finance.enc_opt_i64(n bigint) RETURNS bytea
LANGUAGE sql IMMUTABLE
AS $$ SELECT CASE WHEN n IS NULL THEN '\x00'::bytea ELSE '\x01'::bytea || int8send(n) END $$;

-- A bytea carried in JSON as lowercase hex (no prefix); null stays null.
CREATE FUNCTION finance.hexbytes(j jsonb) RETURNS bytea
LANGUAGE sql IMMUTABLE
AS $$ SELECT CASE WHEN j IS NULL OR jsonb_typeof(j) = 'null' THEN NULL ELSE decode(j #>> '{}', 'hex') END $$;

-- A bytea inside a row image, written as `\x` plus hex because jsonb_populate_record reads that.
CREATE FUNCTION finance.rowbytes(p text) RETURNS bytea
LANGUAGE sql IMMUTABLE
AS $$ SELECT CASE WHEN p IS NULL THEN NULL ELSE decode(regexp_replace(p, '^\\x', ''), 'hex') END $$;

-- A check that commit_event has already run for an object during the current event need not run
-- again at commit: the deferred triggers fire once per changed row, and most rows of one event
-- share a lot, an allocation or an entry. The marks live in a table only the definer role writes
-- (commit_event writes them after its own set-based check passed, and clears them when it starts),
-- never in a session setting, which any session can set. With no mark the trigger runs the full
-- check, so a row written any other way is still checked.
CREATE UNLOGGED TABLE finance.check_mark (
    txid bigint NOT NULL,
    tag text NOT NULL,
    PRIMARY KEY (txid, tag)
);
CREATE FUNCTION finance.already_checked(p_tag text) RETURNS boolean
LANGUAGE sql STABLE
AS $$ SELECT EXISTS (SELECT 1 FROM finance.check_mark WHERE txid = txid_current() AND tag = p_tag) $$;

CREATE TABLE finance.ledger_account (
    code smallint PRIMARY KEY,
    name text NOT NULL UNIQUE,
    book text NOT NULL CHECK (book IN ('general', 'promo', 'exposure')),
    kind text NOT NULL CHECK (kind IN ('asset', 'liability', 'equity', 'revenue',
                                       'contra_revenue', 'expense', 'memo')),
    normal_side smallint NOT NULL CHECK (normal_side IN (1, -1)),
    balance_enforced boolean NOT NULL,
    dimensions text[] NOT NULL
);

INSERT INTO finance.ledger_account (code, name, book, kind, normal_side, balance_enforced, dimensions) VALUES
    (1000, 'bank_operating', 'general', 'asset', 1, false, '{}'),
    (1100, 'processor_clearing', 'general', 'asset', 1, false, '{processor}'),
    (1200, 'supplier_prepaid', 'general', 'asset', 1, false, '{provider}'),
    (1300, 'dispute_funds_withheld', 'general', 'asset', 1, false, '{processor}'),
    (2110, 'purchased_available', 'general', 'liability', -1, true, '{payer,lot}'),
    (2120, 'purchased_allocated', 'general', 'liability', -1, true, '{allocation,lot}'),
    (2130, 'purchased_reserved', 'general', 'liability', -1, true, '{allocation,lot}'),
    (2140, 'purchased_frozen', 'general', 'liability', -1, true, '{lot}'),
    (2300, 'supplier_payable', 'general', 'liability', -1, false, '{provider}'),
    (2310, 'supplier_accrued_unconfirmed', 'general', 'liability', -1, false, '{provider}'),
    (2400, 'tax_payable', 'general', 'liability', -1, false, '{jurisdiction}'),
    (2500, 'refund_payable', 'general', 'liability', -1, true, '{lot}'),
    (3000, 'owner_equity', 'general', 'equity', -1, false, '{}'),
    (4100, 'revenue_table_time', 'general', 'revenue', -1, false, '{}'),
    (4110, 'revenue_ai_usage', 'general', 'revenue', -1, false, '{}'),
    (4190, 'refunds_of_consumed_service', 'general', 'contra_revenue', 1, false, '{}'),
    (4200, 'promotional_revenue', 'general', 'revenue', -1, false, '{promo_source}'),
    (5100, 'cost_of_services', 'general', 'expense', 1, false, '{provider}'),
    (5110, 'cost_of_services_promotional', 'general', 'expense', 1, false, '{provider}'),
    (5200, 'absorbed_supplier_cost', 'general', 'expense', 1, false, '{provider,loss_class}'),
    (5300, 'promotional_credit_expense', 'general', 'expense', 1, false, '{promo_source}'),
    (5400, 'processing_fees', 'general', 'expense', 1, false, '{processor}'),
    (5500, 'chargeback_loss', 'general', 'expense', 1, false, '{processor}'),
    (5700, 'rounding', 'general', 'expense', 1, false, '{}'),
    (2210, 'promotional_available', 'promo', 'memo', -1, true, '{payer,lot}'),
    (2220, 'promotional_allocated', 'promo', 'memo', -1, true, '{allocation,lot}'),
    (2230, 'promotional_reserved', 'promo', 'memo', -1, true, '{allocation,lot}'),
    (3910, 'promotional_offset', 'promo', 'memo', 1, false, '{promo_source}'),
    (9100, 'supplier_exposure_open', 'exposure', 'memo', 1, false, '{provider}'),
    (9190, 'supplier_exposure_offset', 'exposure', 'memo', -1, false, '{provider}');

-- Accounting periods (17.8): pre-created, derived from the database clock, never reopened.
CREATE TABLE finance.accounting_period (
    period_id integer PRIMARY KEY CHECK (period_id BETWEEN 200001 AND 999912),
    starts_at_ms bigint NOT NULL,
    ends_at_ms bigint NOT NULL CHECK (ends_at_ms > starts_at_ms),
    status text NOT NULL CHECK (status IN ('open', 'closed')),
    closed_at_ms bigint,
    closed_by bytea,
    reviewed_by bytea,
    digest bytea CHECK (digest IS NULL OR octet_length(digest) = 32),
    CHECK ((status = 'closed') = (closed_at_ms IS NOT NULL AND digest IS NOT NULL)),
    CHECK (closed_by IS NULL OR reviewed_by IS NULL OR closed_by <> reviewed_by)
);

CREATE TABLE finance.accounting_time_zone_history (
    effective_from_ms bigint PRIMARY KEY,
    time_zone text NOT NULL,
    recorded_at_ms bigint NOT NULL
);
INSERT INTO finance.accounting_time_zone_history VALUES (0, 'UTC', 0);

CREATE FUNCTION finance.time_zone_at(p_ms bigint) RETURNS text
LANGUAGE sql STABLE SET search_path = pg_catalog, finance, pg_temp
AS $$ SELECT time_zone FROM finance.accounting_time_zone_history
       WHERE effective_from_ms <= p_ms ORDER BY effective_from_ms DESC LIMIT 1 $$;

-- The period containing an instant, in the zone in force at that instant.
CREATE FUNCTION finance.period_for(p_ms bigint) RETURNS integer
LANGUAGE sql STABLE SET search_path = pg_catalog, finance, pg_temp
AS $$ SELECT (extract(year FROM to_timestamp(p_ms / 1000.0) AT TIME ZONE finance.time_zone_at(p_ms))::int * 100
            + extract(month FROM to_timestamp(p_ms / 1000.0) AT TIME ZONE finance.time_zone_at(p_ms))::int) $$;

-- Pre-creates periods from the month before `p_from_ms` for `p_months` months; idempotent.
CREATE FUNCTION finance.precreate_periods(p_from_ms bigint, p_months integer) RETURNS integer
LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_zone text := finance.time_zone_at(p_from_ms);
    v_start timestamp := date_trunc('month', to_timestamp(p_from_ms / 1000.0) AT TIME ZONE v_zone) - interval '1 month';
    v_made integer := 0;
    i integer;
    v_next timestamp;
BEGIN
    FOR i IN 0 .. p_months - 1 LOOP
        v_next := v_start + interval '1 month';
        INSERT INTO finance.accounting_period (period_id, starts_at_ms, ends_at_ms, status)
        VALUES (extract(year FROM v_start)::int * 100 + extract(month FROM v_start)::int,
                (extract(epoch FROM v_start AT TIME ZONE v_zone) * 1000)::bigint,
                (extract(epoch FROM v_next AT TIME ZONE v_zone) * 1000)::bigint, 'open')
        ON CONFLICT (period_id) DO NOTHING;
        IF FOUND THEN v_made := v_made + 1; END IF;
        v_start := v_next;
    END LOOP;
    RETURN v_made;
END $$;

SELECT finance.precreate_periods(finance.now_ms(), 36);

-- Extends the pre-created periods so that the last one ends at least `p_months` months after now.
-- The maintenance worker calls it on its tick, so posting never reaches an uncreated month.
CREATE FUNCTION finance.extend_period_horizon(p_months integer) RETURNS integer
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_last timestamp := to_timestamp((SELECT max(ends_at_ms) FROM finance.accounting_period) / 1000.0) AT TIME ZONE 'UTC';
    v_target timestamp := to_timestamp(finance.now_ms() / 1000.0) AT TIME ZONE 'UTC' + make_interval(months => p_months);
    v_missing integer;
BEGIN
    -- A caller cannot ask for an unbounded run of periods: five years is the most one call creates.
    IF p_months IS NULL OR p_months < 0 OR p_months > 60 THEN
        RAISE EXCEPTION 'the period horizon is between 0 and 60 months, not %', p_months
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    v_missing := ((extract(year FROM v_target) - extract(year FROM v_last)) * 12
                  + extract(month FROM v_target) - extract(month FROM v_last))::integer;
    IF v_missing <= 0 THEN
        RETURN 0;
    END IF;
    RETURN finance.precreate_periods((SELECT max(ends_at_ms) FROM finance.accounting_period) + 1, v_missing + 1);
END $$;

CREATE FUNCTION finance.period_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, finance, pg_temp AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'accounting periods are never deleted' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF OLD.status = 'closed' THEN
        RAISE EXCEPTION 'a closed accounting period is final' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF NEW.status <> 'closed' OR NEW.period_id <> OLD.period_id
       OR NEW.starts_at_ms <> OLD.starts_at_ms OR NEW.ends_at_ms <> OLD.ends_at_ms THEN
        RAISE EXCEPTION 'an accounting period may only move from open to closed'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER accounting_period_guard BEFORE UPDATE OR DELETE ON finance.accounting_period
    FOR EACH ROW EXECUTE FUNCTION finance.period_guard();

-- The allowlist of 17.6.
CREATE TABLE finance.entry_kind (
    kind text PRIMARY KEY,
    stream_rule text NOT NULL CHECK (stream_rule IN ('payer', 'session', 'campaign', 'account',
                                                      'platform', 'original')),
    anchor text NOT NULL,
    external_ref_kind text NOT NULL,
    actors text[] NOT NULL,
    reversible boolean NOT NULL,
    needs_approver_above_micro_dollars bigint
);
CREATE TABLE finance.entry_kind_rule (
    kind text NOT NULL REFERENCES finance.entry_kind,
    code smallint NOT NULL REFERENCES finance.ledger_account,
    side smallint NOT NULL CHECK (side IN (1, -1)),
    PRIMARY KEY (kind, code, side)
);

-- Seed: (kind, stream, anchor, external reference kind, actors, reversible, debit codes, credit codes).
CREATE TEMP TABLE entry_kind_seed (
    kind text, stream_rule text, anchor text, external_ref_kind text, actors text[], reversible boolean,
    debits int[], credits int[]
);
INSERT INTO entry_kind_seed VALUES
 ('trial_grant', 'payer', 'trial_grant', 'trial_grant', '{system}', false, '{3910}', '{2210}'),
 ('goodwill_grant', 'payer', 'manual_adjustment', 'support_case', '{operator}', false, '{3910}', '{2210}'),
 ('purchase', 'payer', 'processor_payment_state', 'processor_payment', '{system}', false, '{1100}', '{2110,2400}'),
 ('processing_fee', 'platform', 'none', 'processor_balance_txn', '{system}', true, '{5400}', '{1100}'),
 ('payout', 'platform', 'none', 'processor_payout', '{system}', true, '{1000}', '{1100}'),
 ('allocation', 'payer', 'allocation', 'none', '{system}', false, '{2110,2210}', '{2120,2220}'),
 ('allocation_release', 'payer', 'allocation', 'none', '{system}', false, '{2120,2220}', '{2110,2210}'),
 ('reservation', 'session', 'reservation', 'none', '{system}', false, '{2120,2220,9100}', '{2130,2230,9190}'),
 ('meter_hold', 'session', 'reservation', 'none', '{system}', false, '{2120,2220}', '{2130,2230}'),
 ('release', 'session', 'reservation', 'none', '{system}', false, '{2130,2230,9190}', '{2120,2110,2140,5500,2220,2210,9100}'),
 ('settlement', 'session', 'reservation', 'none', '{system}', false, '{2130,2230,5300,5100,5110,9190}', '{4110,4100,2400,2120,2110,2140,5500,2220,2210,3910,4200,2300,1200,9100}'),
 ('meter_charge', 'session', 'reservation', 'none', '{system}', false, '{2130,2230,5300,5100,5110}', '{4110,4100,2400,2120,2110,2140,5500,2220,2210,3910,4200,2300,1200}'),
 ('company_loss', 'session', 'reservation', 'none', '{system}', false, '{2130,2230,5200,9190}', '{2120,2110,2140,5500,2220,2210,2300,1200,9100}'),
 ('customer_credit', 'session', 'usage_event', 'none', '{operator,system}', false, '{4110,4100,5200,3910,4200}', '{2110,2140,5100,5110,2210,5300}'),
 ('refund_requested', 'payer', 'refund_lot', 'none', '{system}', false, '{2110}', '{2500}'),
 ('refund_paid', 'payer', 'refund_lot', 'processor_refund', '{system}', false, '{2500,2400}', '{1100}'),
 ('manual_refund_payout', 'payer', 'refund_lot', 'bank_transaction', '{operator}', false, '{2500,2400}', '{1000}'),
 ('refund_returned_to_credit', 'payer', 'refund_lot', 'none', '{operator}', false, '{2500}', '{2110,2140}'),
 ('external_refund', 'payer', 'processor_payment_state', 'processor_refund', '{system}', false, '{2110,2120,2130,2140,4190}', '{1100}'),
 ('dispute_freeze', 'payer', 'dispute', 'processor_dispute', '{system}', false, '{2110,2120}', '{2140}'),
 ('dispute_withheld', 'platform', 'dispute', 'processor_dispute', '{system}', false, '{1300,5400}', '{1100}'),
 ('dispute_won', 'payer', 'dispute', 'processor_dispute', '{system}', false, '{1100,2140}', '{1300,2110,5400}'),
 ('dispute_lost', 'payer', 'dispute', 'processor_dispute', '{system}', false, '{2140,2500,5500}', '{1300,2110}'),
 ('chargeback_recovery', 'session', 'reservation', 'none', '{system}', false, '{2130}', '{5500}'),
 ('promotional_expiry', 'payer', 'funding_lot', 'none', '{system}', false, '{2210}', '{3910}'),
 ('promotional_forfeit', 'payer', 'funding_lot', 'none', '{system}', false, '{2210}', '{3910}'),
 ('rounding', 'payer', 'funding_lot', 'none', '{system}', true, '{2110}', '{5700}'),
 ('supplier_exposure_write_off', 'platform', 'provider_attempt', 'none', '{system}', true, '{5200,9190}', '{2310,9100}'),
 ('supplier_invoice_match', 'platform', 'none', 'supplier_invoice', '{system}', true, '{2310,2300,5700,9190}', '{5200,5700,9100}'),
 ('supplier_payment', 'platform', 'none', 'bank_transaction', '{operator}', true, '{2300}', '{1000}'),
 ('supplier_prepayment', 'platform', 'none', 'bank_transaction', '{operator}', true, '{1200}', '{1000}'),
 ('manual_adjustment', 'platform', 'none', 'support_case', '{operator}', true,
     '{2300,2310,2400,3000,4100,4110,4190,5100,5110,5200,5400,5500,5700}',
     '{2300,2310,2400,3000,4100,4110,4190,5100,5110,5200,5400,5500,5700}'),
 ('reversal', 'original', 'none', 'none', '{operator}', false, '{}', '{}');

INSERT INTO finance.entry_kind (kind, stream_rule, anchor, external_ref_kind, actors, reversible)
    SELECT kind, stream_rule, anchor, external_ref_kind, actors, reversible FROM entry_kind_seed;
INSERT INTO finance.entry_kind_rule (kind, code, side)
    SELECT kind, c, 1 FROM entry_kind_seed, unnest(debits) AS c
    UNION ALL
    SELECT kind, c, -1 FROM entry_kind_seed, unnest(credits) AS c;
DROP TABLE entry_kind_seed;

-- The identity of an account key: a hash of its code, currency and dimensions, so a line finds its
-- key by one equality lookup instead of ten null-safe comparisons.
CREATE FUNCTION finance.dimension_key(p_code integer, p_currency text, p_payer bytea, p_lot bytea,
    p_allocation bytea, p_provider text, p_processor text, p_loss_class text, p_jurisdiction text,
    p_promo_source text) RETURNS bytea
LANGUAGE sql IMMUTABLE
AS $$ SELECT sha256(finance.enc_i64(p_code) || finance.enc_text(p_currency)
        || finance.enc_opt_bytes(p_payer) || finance.enc_opt_bytes(p_lot) || finance.enc_opt_bytes(p_allocation)
        || finance.enc_opt_text(p_provider) || finance.enc_opt_text(p_processor)
        || finance.enc_opt_text(p_loss_class) || finance.enc_opt_text(p_jurisdiction)
        || finance.enc_opt_text(p_promo_source)) $$;

-- Account keys: one per account code and dimension tuple; created only by the definers that
-- create the lot, allocation or price version a key belongs to, never by posting. A key is
-- immutable: its dimensions feed the entry hash.
CREATE TABLE finance.account_key (
    key_id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    code smallint NOT NULL REFERENCES finance.ledger_account,
    currency char(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    payer_account_id bytea CHECK (octet_length(payer_account_id) = 16),
    lot_id bytea CHECK (octet_length(lot_id) = 16),
    allocation_id bytea CHECK (octet_length(allocation_id) = 16),
    provider text,
    processor text,
    loss_class text,
    jurisdiction text,
    promo_source text,
    dimension_key bytea NOT NULL UNIQUE CHECK (octet_length(dimension_key) = 32)
);
CREATE INDEX account_key_lot ON finance.account_key (lot_id) WHERE lot_id IS NOT NULL;
CREATE INDEX account_key_allocation ON finance.account_key (allocation_id) WHERE allocation_id IS NOT NULL;
CREATE INDEX account_key_payer ON finance.account_key (payer_account_id) WHERE payer_account_id IS NOT NULL;

CREATE FUNCTION finance.account_key_shape() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_required text[];
    v_present text[] := ARRAY[]::text[];
BEGIN
    SELECT dimensions INTO v_required FROM finance.ledger_account WHERE code = NEW.code;
    IF NEW.payer_account_id IS NOT NULL THEN v_present := v_present || 'payer'::text; END IF;
    IF NEW.lot_id IS NOT NULL THEN v_present := v_present || 'lot'::text; END IF;
    IF NEW.allocation_id IS NOT NULL THEN v_present := v_present || 'allocation'::text; END IF;
    IF NEW.provider IS NOT NULL THEN v_present := v_present || 'provider'::text; END IF;
    IF NEW.processor IS NOT NULL THEN v_present := v_present || 'processor'::text; END IF;
    IF NEW.loss_class IS NOT NULL THEN v_present := v_present || 'loss_class'::text; END IF;
    IF NEW.jurisdiction IS NOT NULL THEN v_present := v_present || 'jurisdiction'::text; END IF;
    IF NEW.promo_source IS NOT NULL THEN v_present := v_present || 'promo_source'::text; END IF;
    IF (SELECT array_agg(x ORDER BY x) FROM unnest(v_present) x)
       IS DISTINCT FROM (SELECT array_agg(x ORDER BY x) FROM unnest(v_required) x)
       AND NOT (cardinality(v_present) = 0 AND cardinality(v_required) = 0) THEN
        RAISE EXCEPTION 'account key for code % has dimensions %, requires %', NEW.code, v_present, v_required
            USING ERRCODE = 'check_violation';
    END IF;
    NEW.dimension_key := finance.dimension_key(NEW.code, NEW.currency, NEW.payer_account_id, NEW.lot_id,
        NEW.allocation_id, NEW.provider, NEW.processor, NEW.loss_class, NEW.jurisdiction, NEW.promo_source);
    RETURN NEW;
END $$;
CREATE TRIGGER account_key_shape BEFORE INSERT ON finance.account_key
    FOR EACH ROW EXECUTE FUNCTION finance.account_key_shape();

CREATE TABLE finance.account_balance (
    key_id bigint PRIMARY KEY REFERENCES finance.account_key,
    natural_micro_dollars bigint NOT NULL CHECK (natural_micro_dollars >= 0)
);

-- An enforced code's balance row exists from the moment its key does.
CREATE FUNCTION finance.account_key_balance_row() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, finance, pg_temp AS $$
BEGIN
    IF (SELECT balance_enforced FROM finance.ledger_account WHERE code = NEW.code) THEN
        INSERT INTO finance.account_balance (key_id, natural_micro_dollars) VALUES (NEW.key_id, 0);
    END IF;
    RETURN NULL;
END $$;
CREATE TRIGGER account_key_balance_row AFTER INSERT ON finance.account_key
    FOR EACH ROW EXECUTE FUNCTION finance.account_key_balance_row();

-- One chain head per lock anchor (17.7).
CREATE TABLE finance.ledger_stream (
    stream_kind text NOT NULL CHECK (stream_kind IN ('payer', 'session', 'campaign', 'account', 'platform')),
    stream_id bytea NOT NULL CHECK (octet_length(stream_id) = 16),
    last_seq bigint NOT NULL,
    last_hash bytea NOT NULL CHECK (octet_length(last_hash) = 32),
    PRIMARY KEY (stream_kind, stream_id)
);

CREATE TABLE finance.event_kind (
    event_kind text PRIMARY KEY,
    carries_entries boolean NOT NULL,
    journaled boolean NOT NULL CHECK (journaled),
    allowed_changes text[] NOT NULL DEFAULT '{}',
    allowed_entries text[] NOT NULL DEFAULT '{}'
);
CREATE TABLE finance.event_marker (
    txid bigint NOT NULL,
    event_id bytea NOT NULL,
    event_kind text NOT NULL REFERENCES finance.event_kind,
    PRIMARY KEY (txid, event_id)
);
CREATE UNLOGGED TABLE finance.flow_lock (
    txid bigint NOT NULL,
    table_name text NOT NULL,
    pk text NOT NULL,
    PRIMARY KEY (txid, table_name, pk)
);

-- The closed set of reasons an entry may carry (22.1): a code, never free text, because the reason
-- is sealed into the journal with the entry.
CREATE TABLE finance.entry_reason (
    code text PRIMARY KEY CHECK (code ~ '^[a-z_]{3,40}$')
);
INSERT INTO finance.entry_reason (code) VALUES
    ('trial_granted'), ('goodwill_granted'), ('purchase_confirmed'), ('allocated'), ('allocation_released'),
    ('admitted'), ('settled'), ('released_not_dispatched'), ('released_hold_deadline'), ('company_loss'),
    ('processor_fee'), ('processor_payout'), ('refund_requested'), ('refund_paid'), ('dispute_opened'),
    ('dispute_resolved'), ('chargeback_recovered'), ('promotional_expired'), ('promotional_forfeited'),
    ('rounding'), ('supplier_exposure_written_off'), ('supplier_invoice_matched'), ('supplier_paid'),
    ('supplier_prepaid'), ('manual_adjustment'), ('correction'), ('credit_technical_failure'),
    ('credit_content_failure'), ('credit_billing_error'), ('credit_duplicate_charge'), ('credit_goodwill');

CREATE TABLE finance.journal_entry (
    entry_id bytea PRIMARY KEY CHECK (octet_length(entry_id) = 16),
    kind text NOT NULL REFERENCES finance.entry_kind,
    business_key bytea NOT NULL CHECK (octet_length(business_key) = 32),
    external_ref_kind text NOT NULL CHECK (external_ref_kind IN ('none', 'processor_payment',
        'processor_refund', 'processor_dispute', 'processor_balance_txn', 'processor_payout',
        'supplier_invoice', 'bank_transaction', 'support_case', 'attempt', 'trial_grant')),
    external_ref text CHECK (external_ref IS NULL OR octet_length(external_ref) <= 128),
    stream_kind text NOT NULL,
    stream_id bytea NOT NULL,
    stream_seq bigint NOT NULL CHECK (stream_seq >= 1),
    global_seq bigint NOT NULL UNIQUE DEFAULT nextval('finance.global_seq'),
    prev_hash bytea NOT NULL CHECK (octet_length(prev_hash) = 32),
    entry_hash bytea NOT NULL CHECK (octet_length(entry_hash) = 32),
    entry_hash_version smallint NOT NULL CHECK (entry_hash_version >= 1),
    effective_at_ms bigint NOT NULL,
    recorded_at_ms bigint NOT NULL,
    period_id integer NOT NULL REFERENCES finance.accounting_period,
    line_count smallint NOT NULL CHECK (line_count BETWEEN 2 AND 64),
    reverses_entry_id bytea REFERENCES finance.journal_entry,
    actor_kind text NOT NULL CHECK (actor_kind IN ('system', 'operator')),
    actor_id bytea,
    approver_id bytea,
    reason text NOT NULL REFERENCES finance.entry_reason,
    case_ref text CHECK (octet_length(case_ref) <= 64),
    event_id bytea NOT NULL,
    campaign_id bytea,
    session_id bytea,
    attempt_id bytea,
    reservation_id bytea,
    price_version_id bytea,
    corrects_period_id integer REFERENCES finance.accounting_period,
    UNIQUE (kind, business_key),
    UNIQUE (stream_kind, stream_id, stream_seq),
    UNIQUE (reverses_entry_id),
    CHECK ((kind = 'reversal') = (reverses_entry_id IS NOT NULL)),
    CHECK (actor_kind = 'system' OR (actor_id IS NOT NULL AND case_ref IS NOT NULL)),
    CHECK (actor_kind <> 'system' OR (actor_id IS NULL AND approver_id IS NULL AND case_ref IS NULL)),
    CHECK (approver_id IS NULL OR actor_id IS NULL OR approver_id <> actor_id)
);

CREATE TABLE finance.journal_line (
    entry_id bytea NOT NULL REFERENCES finance.journal_entry,
    line_no smallint NOT NULL CHECK (line_no >= 1),
    key_id bigint NOT NULL REFERENCES finance.account_key,
    amount_micro_dollars bigint NOT NULL CHECK (amount_micro_dollars <> 0),
    PRIMARY KEY (entry_id, line_no),
    UNIQUE (entry_id, key_id)
);
CREATE INDEX journal_line_key ON finance.journal_line (key_id);

-- Only an admitted operation has a receipt (11.2 step 2).
CREATE TABLE finance.operation_receipt (
    namespace text NOT NULL CHECK (namespace IN ('admission', 'allocation', 'withdrawal',
        'policy_update', 'quote_approval', 'refund', 'credit', 'manual_adjustment')),
    principal bytea NOT NULL CHECK (octet_length(principal) = 16),
    operation_id bytea NOT NULL CHECK (octet_length(operation_id) = 16),
    fingerprint bytea NOT NULL CHECK (octet_length(fingerprint) = 32),
    outcome jsonb NOT NULL,
    result_ref bytea,
    created_at_ms bigint NOT NULL,
    PRIMARY KEY (namespace, principal, operation_id)
);

-- The full entry as JSON, from rows: the one input of the canonical hash (17.7), so posting,
-- replay and the daily compare all hash the same bytes.
CREATE FUNCTION finance.entry_json(p_entry_id bytea) RETURNS jsonb
LANGUAGE sql STABLE SET search_path = pg_catalog, finance, pg_temp AS $$
    SELECT jsonb_build_object(
        'entry_id', encode(e.entry_id, 'hex'), 'event_id', encode(e.event_id, 'hex'), 'kind', e.kind,
        'business_key', encode(e.business_key, 'hex'),
        'external_ref_kind', e.external_ref_kind, 'external_ref', e.external_ref,
        'stream_kind', e.stream_kind, 'stream_id', encode(e.stream_id, 'hex'), 'stream_seq', e.stream_seq,
        'recorded_at_ms', e.recorded_at_ms, 'effective_at_ms', e.effective_at_ms,
        'period_id', e.period_id, 'corrects_period_id', e.corrects_period_id,
        'reverses_entry_id', encode(e.reverses_entry_id, 'hex'),
        'campaign_id', encode(e.campaign_id, 'hex'), 'session_id', encode(e.session_id, 'hex'),
        'attempt_id', encode(e.attempt_id, 'hex'), 'reservation_id', encode(e.reservation_id, 'hex'),
        'price_version_id', encode(e.price_version_id, 'hex'),
        'reason', e.reason, 'case_ref', e.case_ref, 'actor_kind', e.actor_kind,
        'actor_id', encode(e.actor_id, 'hex'), 'approver_id', encode(e.approver_id, 'hex'),
        'lines', (SELECT jsonb_agg(jsonb_build_object(
                'code', k.code, 'currency', k.currency,
                'payer', encode(k.payer_account_id, 'hex'), 'lot', encode(k.lot_id, 'hex'),
                'allocation', encode(k.allocation_id, 'hex'), 'provider', k.provider,
                'processor', k.processor, 'loss_class', k.loss_class,
                'jurisdiction', k.jurisdiction, 'promo_source', k.promo_source,
                'amount', l.amount_micro_dollars) ORDER BY l.line_no)
            FROM finance.journal_line l JOIN finance.account_key k USING (key_id)
            WHERE l.entry_id = e.entry_id))
    FROM finance.journal_entry e WHERE e.entry_id = p_entry_id
$$;

-- Version 1 canonical bytes (frozen; a new field is a version 2 beside this function).
CREATE FUNCTION finance.entry_canonical_v1(j jsonb) RETURNS bytea
LANGUAGE plpgsql IMMUTABLE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_out bytea;
    v_line jsonb;
BEGIN
    v_out := finance.enc_bytes(finance.hexbytes(j -> 'entry_id'))
        || finance.enc_bytes(finance.hexbytes(j -> 'event_id'))
        || finance.enc_text(j ->> 'kind')
        || finance.enc_bytes(finance.hexbytes(j -> 'business_key'))
        || finance.enc_text(j ->> 'external_ref_kind')
        || finance.enc_opt_text(j ->> 'external_ref')
        || finance.enc_text(j ->> 'stream_kind')
        || finance.enc_bytes(finance.hexbytes(j -> 'stream_id'))
        || finance.enc_i64((j ->> 'stream_seq')::bigint)
        || finance.enc_i64((j ->> 'recorded_at_ms')::bigint)
        || finance.enc_i64((j ->> 'effective_at_ms')::bigint)
        || finance.enc_i64((j ->> 'period_id')::bigint)
        || finance.enc_opt_i64((j ->> 'corrects_period_id')::bigint)
        || finance.enc_opt_bytes(finance.hexbytes(j -> 'reverses_entry_id'))
        || finance.enc_opt_bytes(finance.hexbytes(j -> 'campaign_id'))
        || finance.enc_opt_bytes(finance.hexbytes(j -> 'session_id'))
        || finance.enc_opt_bytes(finance.hexbytes(j -> 'attempt_id'))
        || finance.enc_opt_bytes(finance.hexbytes(j -> 'reservation_id'))
        || finance.enc_opt_bytes(finance.hexbytes(j -> 'price_version_id'))
        || finance.enc_text(j ->> 'reason')
        || finance.enc_opt_text(j ->> 'case_ref')
        || finance.enc_text(j ->> 'actor_kind')
        || finance.enc_opt_bytes(finance.hexbytes(j -> 'actor_id'))
        || finance.enc_opt_bytes(finance.hexbytes(j -> 'approver_id'))
        || finance.enc_i64(jsonb_array_length(j -> 'lines'));
    FOR v_line IN SELECT value FROM jsonb_array_elements(j -> 'lines') LOOP
        v_out := v_out
            || finance.enc_i64((v_line ->> 'code')::bigint)
            || finance.enc_text(v_line ->> 'currency')
            || finance.enc_opt_bytes(finance.hexbytes(v_line -> 'payer'))
            || finance.enc_opt_bytes(finance.hexbytes(v_line -> 'lot'))
            || finance.enc_opt_bytes(finance.hexbytes(v_line -> 'allocation'))
            || finance.enc_opt_text(v_line ->> 'provider')
            || finance.enc_opt_text(v_line ->> 'processor')
            || finance.enc_opt_text(v_line ->> 'loss_class')
            || finance.enc_opt_text(v_line ->> 'jurisdiction')
            || finance.enc_opt_text(v_line ->> 'promo_source')
            || finance.enc_i64((v_line ->> 'amount')::bigint);
    END LOOP;
    RETURN v_out;
END $$;

CREATE FUNCTION finance.entry_hash(p_version smallint, p_prev bytea, j jsonb) RETURNS bytea
LANGUAGE plpgsql IMMUTABLE SET search_path = pg_catalog, finance, pg_temp AS $$
BEGIN
    IF p_version = 1 THEN
        RETURN sha256(convert_to('df-ledger/1', 'UTF8') || p_prev || finance.entry_canonical_v1(j));
    END IF;
    RAISE EXCEPTION 'unknown entry hash version %', p_version USING ERRCODE = 'invalid_parameter_value';
END $$;

-- History rows refuse UPDATE and DELETE by trigger. TRUNCATE is refused by privilege (no finance role
-- holds it) and, on the six central tables, by a second trigger.
CREATE FUNCTION finance.refuse_change() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    RAISE EXCEPTION 'finance history rows are append-only' USING ERRCODE = 'integrity_constraint_violation';
END $$;
CREATE FUNCTION finance.refuse_truncate() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    RAISE EXCEPTION 'finance tables are never truncated' USING ERRCODE = 'integrity_constraint_violation';
END $$;
CREATE TRIGGER ledger_account_append_only BEFORE UPDATE OR DELETE ON finance.ledger_account
    FOR EACH ROW EXECUTE FUNCTION finance.refuse_change();
CREATE TRIGGER journal_entry_append_only BEFORE UPDATE OR DELETE ON finance.journal_entry
    FOR EACH ROW EXECUTE FUNCTION finance.refuse_change();
CREATE TRIGGER journal_line_append_only BEFORE UPDATE OR DELETE ON finance.journal_line
    FOR EACH ROW EXECUTE FUNCTION finance.refuse_change();
CREATE TRIGGER entry_kind_append_only BEFORE UPDATE OR DELETE ON finance.entry_kind
    FOR EACH ROW EXECUTE FUNCTION finance.refuse_change();
CREATE TRIGGER entry_kind_rule_append_only BEFORE UPDATE OR DELETE ON finance.entry_kind_rule
    FOR EACH ROW EXECUTE FUNCTION finance.refuse_change();
CREATE TRIGGER event_kind_append_only BEFORE UPDATE OR DELETE ON finance.event_kind
    FOR EACH ROW EXECUTE FUNCTION finance.refuse_change();
CREATE TRIGGER entry_reason_append_only BEFORE UPDATE OR DELETE ON finance.entry_reason
    FOR EACH ROW EXECUTE FUNCTION finance.refuse_change();
CREATE TRIGGER account_key_append_only BEFORE UPDATE OR DELETE ON finance.account_key
    FOR EACH ROW EXECUTE FUNCTION finance.refuse_change();
CREATE TRIGGER accounting_time_zone_history_append_only BEFORE UPDATE OR DELETE ON finance.accounting_time_zone_history
    FOR EACH ROW EXECUTE FUNCTION finance.refuse_change();
CREATE TRIGGER ledger_account_no_truncate BEFORE TRUNCATE ON finance.ledger_account
    FOR EACH STATEMENT EXECUTE FUNCTION finance.refuse_truncate();
CREATE TRIGGER journal_entry_no_truncate BEFORE TRUNCATE ON finance.journal_entry
    FOR EACH STATEMENT EXECUTE FUNCTION finance.refuse_truncate();
CREATE TRIGGER journal_line_no_truncate BEFORE TRUNCATE ON finance.journal_line
    FOR EACH STATEMENT EXECUTE FUNCTION finance.refuse_truncate();
CREATE TRIGGER account_key_no_truncate BEFORE TRUNCATE ON finance.account_key
    FOR EACH STATEMENT EXECUTE FUNCTION finance.refuse_truncate();
CREATE TRIGGER account_balance_no_truncate BEFORE TRUNCATE ON finance.account_balance
    FOR EACH STATEMENT EXECUTE FUNCTION finance.refuse_truncate();
CREATE TRIGGER ledger_stream_no_truncate BEFORE TRUNCATE ON finance.ledger_stream
    FOR EACH STATEMENT EXECUTE FUNCTION finance.refuse_truncate();

-- A closed period takes no entry (17.8).
CREATE FUNCTION finance.entry_period_open() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, finance, pg_temp AS $$
BEGIN
    IF (SELECT status FROM finance.accounting_period WHERE period_id = NEW.period_id) <> 'open' THEN
        RAISE EXCEPTION 'accounting period % is closed', NEW.period_id
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER journal_entry_period_open BEFORE INSERT ON finance.journal_entry
    FOR EACH ROW EXECUTE FUNCTION finance.entry_period_open();

-- Balanced per book and currency, with the declared line count, checked at commit (17.1 item 2).
CREATE FUNCTION finance.check_entry_balanced() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_declared integer;
    v_count integer;
BEGIN
    IF finance.already_checked('e_' || encode(NEW.entry_id, 'hex')) THEN
        RETURN NULL;
    END IF;
    SELECT line_count INTO v_declared FROM finance.journal_entry WHERE entry_id = NEW.entry_id;
    SELECT count(*) INTO v_count FROM finance.journal_line WHERE entry_id = NEW.entry_id;
    IF v_declared IS DISTINCT FROM v_count THEN
        RAISE EXCEPTION 'entry declares % lines but has %', v_declared, v_count
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF EXISTS (
        SELECT 1 FROM finance.journal_line l
            JOIN finance.account_key k ON k.key_id = l.key_id
            JOIN finance.ledger_account a ON a.code = k.code
         WHERE l.entry_id = NEW.entry_id
         GROUP BY a.book, k.currency
        HAVING sum(l.amount_micro_dollars) <> 0) THEN
        RAISE EXCEPTION 'entry is not balanced in every book and currency'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER journal_entry_balanced AFTER INSERT ON finance.journal_entry
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance.check_entry_balanced();
CREATE CONSTRAINT TRIGGER journal_line_balanced AFTER INSERT ON finance.journal_line
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance.check_entry_balanced();

-- Every enforced line's key has its balance row (17.1 item 4), checked once per entry.
CREATE FUNCTION finance.check_line_balance_row() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, finance, pg_temp AS $$
BEGIN
    IF finance.already_checked('lb_' || encode(NEW.entry_id, 'hex')) THEN
        RETURN NULL;
    END IF;
    IF EXISTS (SELECT 1 FROM finance.journal_line l
                 JOIN finance.account_key k ON k.key_id = l.key_id
                 JOIN finance.ledger_account a ON a.code = k.code
                 LEFT JOIN finance.account_balance b ON b.key_id = k.key_id
                WHERE l.entry_id = NEW.entry_id AND a.balance_enforced AND b.key_id IS NULL) THEN
        RAISE EXCEPTION 'an enforced key of entry % has no balance row', encode(NEW.entry_id, 'hex')
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER journal_line_balance_row AFTER INSERT ON finance.journal_line
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance.check_line_balance_row();

-- The chain link: prev_hash is the previous entry's hash of the same stream (17.1 item 10).
CREATE FUNCTION finance.check_chain_link() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_expected bytea;
BEGIN
    IF NEW.stream_seq = 1 THEN
        v_expected := decode(repeat('00', 32), 'hex');
    ELSE
        SELECT entry_hash INTO v_expected FROM finance.journal_entry
         WHERE stream_kind = NEW.stream_kind AND stream_id = NEW.stream_id AND stream_seq = NEW.stream_seq - 1;
    END IF;
    IF v_expected IS DISTINCT FROM NEW.prev_hash THEN
        RAISE EXCEPTION 'entry % does not extend its stream chain', encode(NEW.entry_id, 'hex')
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER journal_entry_chain AFTER INSERT ON finance.journal_entry
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance.check_chain_link();

-- A reversal is the exact negation of its original, line for line (17.1 item 8).
CREATE FUNCTION finance.check_reversal() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, finance, pg_temp AS $$
BEGIN
    IF NEW.reverses_entry_id IS NULL THEN RETURN NULL; END IF;
    IF NOT (SELECT reversible FROM finance.entry_kind k
              JOIN finance.journal_entry o ON o.kind = k.kind WHERE o.entry_id = NEW.reverses_entry_id) THEN
        RAISE EXCEPTION 'the original entry is of a kind that is not reversible'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF EXISTS (
        (SELECT key_id, amount_micro_dollars FROM finance.journal_line WHERE entry_id = NEW.reverses_entry_id
         EXCEPT SELECT key_id, -amount_micro_dollars FROM finance.journal_line WHERE entry_id = NEW.entry_id)
        UNION ALL
        (SELECT key_id, -amount_micro_dollars FROM finance.journal_line WHERE entry_id = NEW.entry_id
         EXCEPT SELECT key_id, amount_micro_dollars FROM finance.journal_line WHERE entry_id = NEW.reverses_entry_id)) THEN
        RAISE EXCEPTION 'a reversal must negate its original exactly'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER journal_entry_reversal AFTER INSERT ON finance.journal_entry
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance.check_reversal();
