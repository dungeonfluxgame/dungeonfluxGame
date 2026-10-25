-- Finance funding (planning/finance-ledger-research.md sections 7, 8, 17.2): payers, funding lots,
-- the trial grant, price versions, session funding, allocations and epochs. No foreign key reaches
-- another owner's schema: account, session and campaign ids are validated when a row is created.
-- Mutable rows carry `revision`; `apply_change` updates them by compare-and-set on it.

CREATE TABLE finance.payer (
    account_id bytea PRIMARY KEY CHECK (octet_length(account_id) = 16),
    standing text NOT NULL CHECK (standing IN ('active', 'restricted')),
    standing_reason text,
    revision bigint NOT NULL DEFAULT 1,
    created_at_ms bigint NOT NULL
);

CREATE TABLE finance.funding_lot (
    lot_id bytea PRIMARY KEY CHECK (octet_length(lot_id) = 16),
    payer_account_id bytea NOT NULL REFERENCES finance.payer,
    class text NOT NULL CHECK (class IN ('purchased', 'promotional')),
    promo_source text CHECK (promo_source IN ('trial', 'goodwill')),
    currency char(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    original_micro_dollars bigint NOT NULL CHECK (original_micro_dollars > 0),
    state text NOT NULL CHECK (state IN ('active', 'frozen', 'charged_back', 'closed', 'forfeited')),
    consumed_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (consumed_micro_dollars >= 0),
    refunded_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (refunded_micro_dollars >= 0),
    charged_back_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (charged_back_micro_dollars >= 0),
    extinguished_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (extinguished_micro_dollars >= 0),
    rounded_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (rounded_micro_dollars >= 0),
    expired_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (expired_micro_dollars >= 0),
    forfeited_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (forfeited_micro_dollars >= 0),
    processor text,
    payment_ref text,
    charge_ref text,
    purchase_order_ref text,
    tax_mode text NOT NULL DEFAULT 'none',
    tax_jurisdiction text,
    tax_rate_numerator bigint,
    tax_rate_denominator bigint,
    tax_micro_dollars bigint NOT NULL DEFAULT 0,
    billing_country text,
    billing_region text,
    evidence_state text NOT NULL DEFAULT 'complete' CHECK (evidence_state IN ('complete', 'pending_rebuild')),
    tax_policy_version integer NOT NULL DEFAULT 1,
    created_at_ms bigint NOT NULL,
    expires_at_ms bigint,
    revision bigint NOT NULL DEFAULT 1,
    UNIQUE (processor, payment_ref),
    CHECK ((class = 'purchased') = (payment_ref IS NOT NULL)),
    CHECK ((class = 'promotional') = (promo_source IS NOT NULL)),
    CHECK (state <> 'forfeited' OR class = 'promotional')
);
CREATE INDEX funding_lot_payer ON finance.funding_lot (payer_account_id, created_at_ms);

CREATE TABLE finance.processor_payment_state (
    processor text NOT NULL,
    payment_ref text NOT NULL,
    lot_id bytea REFERENCES finance.funding_lot,
    captured boolean NOT NULL,
    captured_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (captured_micro_dollars >= 0),
    refunded_total_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (refunded_total_micro_dollars >= 0),
    dispute_ref text,
    dispute_status text CHECK (dispute_status IN ('open', 'won', 'lost')),
    disputed_micro_dollars bigint NOT NULL DEFAULT 0,
    object_revision bigint NOT NULL DEFAULT 1,
    revision bigint NOT NULL DEFAULT 1,
    applied_at_ms bigint NOT NULL,
    PRIMARY KEY (processor, payment_ref)
);

CREATE TABLE finance.trial_grant (
    account_id bytea PRIMARY KEY CHECK (octet_length(account_id) = 16),
    email_grant_key bytea UNIQUE CHECK (octet_length(email_grant_key) = 32),
    lot_id bytea NOT NULL REFERENCES finance.funding_lot,
    evidence_state text NOT NULL DEFAULT 'complete' CHECK (evidence_state IN ('complete', 'pending_rebuild')),
    granted_at_ms bigint NOT NULL,
    CHECK (email_grant_key IS NOT NULL OR evidence_state = 'pending_rebuild')
);

CREATE TABLE finance.price_version (
    price_version_id bytea PRIMARY KEY CHECK (octet_length(price_version_id) = 16),
    label text NOT NULL UNIQUE,
    effective_from_ms bigint NOT NULL,
    currency char(3) NOT NULL,
    platform_rate_micro_dollars_per_hour bigint NOT NULL CHECK (platform_rate_micro_dollars_per_hour >= 0),
    markup_numerator bigint NOT NULL CHECK (markup_numerator > 0),
    markup_denominator bigint NOT NULL CHECK (markup_denominator > 0),
    tax_policy_version integer NOT NULL DEFAULT 1,
    presets jsonb NOT NULL DEFAULT '{}',
    proposed_pilot boolean NOT NULL DEFAULT true,
    status text NOT NULL CHECK (status IN ('published', 'retired')),
    tariff_digest bytea CHECK (octet_length(tariff_digest) = 32),
    published_at_ms bigint NOT NULL,
    published_by bytea,
    revision bigint NOT NULL DEFAULT 1
);
CREATE TABLE finance.price_version_tariff (
    price_version_id bytea NOT NULL REFERENCES finance.price_version,
    provider text NOT NULL,
    model text NOT NULL,
    category text NOT NULL,
    context_tier_max_tokens bigint NOT NULL DEFAULT 0,
    rate_nano_dollars numeric NOT NULL CHECK (rate_nano_dollars >= 0),
    per_units bigint NOT NULL CHECK (per_units > 0),
    as_of text,
    source text,
    PRIMARY KEY (price_version_id, provider, model, category, context_tier_max_tokens)
);
CREATE TRIGGER price_version_tariff_append_only BEFORE UPDATE OR DELETE ON finance.price_version_tariff
    FOR EACH ROW EXECUTE FUNCTION finance.refuse_change();

CREATE TABLE finance.session_funding (
    session_id bytea PRIMARY KEY CHECK (octet_length(session_id) = 16),
    tenant_id bytea NOT NULL CHECK (octet_length(tenant_id) = 16),
    campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16),
    price_version_id bytea NOT NULL REFERENCES finance.price_version,
    currency char(3) NOT NULL,
    status text NOT NULL CHECK (status IN ('open', 'settling', 'closed')),
    policy_revision bigint NOT NULL DEFAULT 1,
    hourly_ceiling_micro_dollars bigint NOT NULL CHECK (hourly_ceiling_micro_dollars >= 0),
    session_ceiling_micro_dollars bigint,
    per_request_threshold_micro_dollars bigint NOT NULL DEFAULT 500000,
    split_mode text NOT NULL DEFAULT 'proportional' CHECK (split_mode IN ('proportional', 'sponsored', 'equal')),
    contributions_visible boolean NOT NULL DEFAULT false,
    generation_policy_digest bytea,
    current_epoch bigint NOT NULL DEFAULT 1,
    charged_total_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (charged_total_micro_dollars >= 0),
    pace_tokens_micro_dollars bigint NOT NULL DEFAULT 0,
    pace_updated_at_ms bigint NOT NULL DEFAULT 0,
    meter_priced_ms bigint NOT NULL DEFAULT 0,
    meter_charged_micro_dollars bigint NOT NULL DEFAULT 0,
    meter_forgone_micro_dollars bigint NOT NULL DEFAULT 0,
    meter_funded_until_ms bigint NOT NULL DEFAULT 0,
    opened_at_ms bigint NOT NULL,
    settling_at_ms bigint,
    closed_at_ms bigint,
    revision bigint NOT NULL DEFAULT 1
);
CREATE INDEX session_funding_campaign ON finance.session_funding (campaign_id);

CREATE TABLE finance.allocation (
    allocation_id bytea PRIMARY KEY CHECK (octet_length(allocation_id) = 16),
    session_id bytea NOT NULL REFERENCES finance.session_funding,
    payer_account_id bytea NOT NULL REFERENCES finance.payer,
    authorized_micro_dollars bigint NOT NULL CHECK (authorized_micro_dollars >= 0),
    drawn_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (drawn_micro_dollars >= 0),
    returned_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (returned_micro_dollars >= 0),
    charged_micro_dollars bigint NOT NULL DEFAULT 0 CHECK (charged_micro_dollars >= 0),
    status text NOT NULL CHECK (status IN ('active', 'paused_departed', 'consent_incompatible',
                                           'withdrawn', 'closing', 'closed')),
    consent_revision bigint NOT NULL DEFAULT 1,
    created_at_ms bigint NOT NULL,
    ended_at_ms bigint,
    revision bigint NOT NULL DEFAULT 1,
    UNIQUE (session_id, payer_account_id)
);

CREATE TABLE finance.allocation_consent (
    allocation_id bytea NOT NULL REFERENCES finance.allocation,
    consent_revision bigint NOT NULL,
    price_version_id bytea NOT NULL REFERENCES finance.price_version,
    approved_hourly_ceiling bigint NOT NULL,
    approved_session_ceiling bigint,
    approved_split_mode text NOT NULL,
    approval_threshold_micro_dollars bigint NOT NULL,
    generation_policy_digest bytea,
    consent_terms_version integer NOT NULL,
    consented_at_ms bigint NOT NULL,
    PRIMARY KEY (allocation_id, consent_revision)
);
CREATE TRIGGER allocation_consent_append_only BEFORE UPDATE OR DELETE ON finance.allocation_consent
    FOR EACH ROW EXECUTE FUNCTION finance.refuse_change();

CREATE TABLE finance.allocation_lot (
    allocation_id bytea NOT NULL REFERENCES finance.allocation,
    lot_id bytea NOT NULL REFERENCES finance.funding_lot,
    draw_rank integer NOT NULL,
    PRIMARY KEY (allocation_id, lot_id)
);

CREATE TABLE finance.funding_epoch (
    session_id bytea NOT NULL REFERENCES finance.session_funding,
    epoch_seq bigint NOT NULL,
    reason text NOT NULL,
    split_mode text NOT NULL,
    created_at_ms bigint NOT NULL,
    PRIMARY KEY (session_id, epoch_seq)
);
CREATE TABLE finance.epoch_share (
    session_id bytea NOT NULL,
    epoch_seq bigint NOT NULL,
    allocation_id bytea NOT NULL REFERENCES finance.allocation,
    weight bigint NOT NULL CHECK (weight > 0),
    PRIMARY KEY (session_id, epoch_seq, allocation_id),
    FOREIGN KEY (session_id, epoch_seq) REFERENCES finance.funding_epoch
);

CREATE TABLE finance.funding_feed_position (
    campaign_id bytea PRIMARY KEY CHECK (octet_length(campaign_id) = 16),
    last_event_seq bigint NOT NULL,
    feed_epoch bytea NOT NULL CHECK (octet_length(feed_epoch) = 16),
    updated_at_ms bigint NOT NULL
);
CREATE TABLE finance.funding_effect (
    campaign_id bytea NOT NULL,
    event_seq bigint NOT NULL,
    effect text NOT NULL CHECK (effect IN ('paused_departed', 'released_unreserved', 'none')),
    allocation_id bytea,
    applied_at_ms bigint NOT NULL,
    PRIMARY KEY (campaign_id, event_seq)
);

-- The real foreign keys of the account keys (17.1 item 5) and the lot-class and currency guard.
ALTER TABLE finance.account_key
    ADD CONSTRAINT account_key_payer FOREIGN KEY (payer_account_id) REFERENCES finance.payer,
    ADD CONSTRAINT account_key_lot FOREIGN KEY (lot_id) REFERENCES finance.funding_lot,
    ADD CONSTRAINT account_key_allocation FOREIGN KEY (allocation_id) REFERENCES finance.allocation;

CREATE FUNCTION finance.account_key_class_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_lot finance.funding_lot%ROWTYPE;
    v_allocation_payer bytea;
BEGIN
    IF NEW.lot_id IS NULL THEN RETURN NEW; END IF;
    SELECT * INTO v_lot FROM finance.funding_lot WHERE lot_id = NEW.lot_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'account key names a lot that does not exist' USING ERRCODE = 'foreign_key_violation';
    END IF;
    IF NEW.code IN (2110, 2120, 2130, 2140, 2500) AND v_lot.class <> 'purchased' THEN
        RAISE EXCEPTION 'code % needs a purchased lot', NEW.code USING ERRCODE = 'check_violation';
    END IF;
    IF NEW.code IN (2210, 2220, 2230) AND v_lot.class <> 'promotional' THEN
        RAISE EXCEPTION 'code % needs a promotional lot', NEW.code USING ERRCODE = 'check_violation';
    END IF;
    IF NEW.currency <> v_lot.currency THEN
        RAISE EXCEPTION 'key currency % differs from the lot currency %', NEW.currency, v_lot.currency
            USING ERRCODE = 'check_violation';
    END IF;
    IF NEW.payer_account_id IS NOT NULL AND NEW.payer_account_id <> v_lot.payer_account_id THEN
        RAISE EXCEPTION 'key payer differs from the lot payer' USING ERRCODE = 'check_violation';
    END IF;
    IF NEW.allocation_id IS NOT NULL THEN
        SELECT payer_account_id INTO v_allocation_payer FROM finance.allocation WHERE allocation_id = NEW.allocation_id;
        IF v_allocation_payer IS DISTINCT FROM v_lot.payer_account_id THEN
            RAISE EXCEPTION 'allocation payer differs from the lot payer' USING ERRCODE = 'check_violation';
        END IF;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER account_key_class_guard BEFORE INSERT ON finance.account_key
    FOR EACH ROW EXECUTE FUNCTION finance.account_key_class_guard();

-- Conservation per lot, one statement (17.3): original = keys + totals. Also: no key of a closed
-- or forfeited lot holds a balance.
CREATE FUNCTION finance.check_lot_conservation(p_lot bytea) RETURNS void
LANGUAGE plpgsql SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_gap bigint;
    v_state text;
    v_keys bigint;
BEGIN
    IF finance.already_checked('lot_' || encode(p_lot, 'hex')) THEN
        RETURN;
    END IF;
    SELECT l.original_micro_dollars
             - coalesce((SELECT sum(b.natural_micro_dollars) FROM finance.account_key k
                          JOIN finance.account_balance b ON b.key_id = k.key_id
                         WHERE k.lot_id = l.lot_id), 0)
             - l.consumed_micro_dollars - l.refunded_micro_dollars - l.extinguished_micro_dollars
             - l.rounded_micro_dollars - l.expired_micro_dollars - l.forfeited_micro_dollars,
           l.state,
           coalesce((SELECT sum(b.natural_micro_dollars) FROM finance.account_key k
                      JOIN finance.account_balance b ON b.key_id = k.key_id
                     WHERE k.lot_id = l.lot_id), 0)
      INTO v_gap, v_state, v_keys
      FROM finance.funding_lot l WHERE l.lot_id = p_lot;
    IF NOT FOUND THEN RETURN; END IF;
    IF v_gap <> 0 THEN
        RAISE EXCEPTION 'lot % does not conserve (off by %)', encode(p_lot, 'hex'), v_gap
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF v_state IN ('closed', 'forfeited') AND v_keys <> 0 THEN
        RAISE EXCEPTION 'a closed lot holds a balance' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
END $$;

CREATE FUNCTION finance.lot_conservation_trigger() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, finance, pg_temp AS $$
BEGIN
    IF TG_TABLE_NAME = 'funding_lot' THEN
        PERFORM finance.check_lot_conservation(NEW.lot_id);
    ELSE
        PERFORM finance.check_lot_conservation(k.lot_id) FROM finance.account_key k
            WHERE k.key_id = NEW.key_id AND k.lot_id IS NOT NULL;
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER funding_lot_conserves AFTER INSERT OR UPDATE ON finance.funding_lot
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance.lot_conservation_trigger();
CREATE CONSTRAINT TRIGGER account_balance_lot_conserves AFTER UPDATE ON finance.account_balance
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance.lot_conservation_trigger();

-- Allocation totals (17.3): drawn - returned - charged = allocated + reserved over all its keys;
-- a closed allocation holds nothing.
CREATE FUNCTION finance.check_allocation_totals(p_allocation bytea) RETURNS void
LANGUAGE plpgsql SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE
    v_gap bigint;
    v_status text;
    v_held bigint;
BEGIN
    IF finance.already_checked('al_' || encode(p_allocation, 'hex')) THEN
        RETURN;
    END IF;
    SELECT a.drawn_micro_dollars - a.returned_micro_dollars - a.charged_micro_dollars
             - coalesce((SELECT sum(b.natural_micro_dollars) FROM finance.account_key k
                          JOIN finance.account_balance b ON b.key_id = k.key_id
                         WHERE k.allocation_id = a.allocation_id), 0),
           a.status,
           coalesce((SELECT sum(b.natural_micro_dollars) FROM finance.account_key k
                      JOIN finance.account_balance b ON b.key_id = k.key_id
                     WHERE k.allocation_id = a.allocation_id), 0)
      INTO v_gap, v_status, v_held
      FROM finance.allocation a WHERE a.allocation_id = p_allocation;
    IF NOT FOUND THEN RETURN; END IF;
    IF v_gap <> 0 THEN
        RAISE EXCEPTION 'allocation % totals differ from its keys by %', encode(p_allocation, 'hex'), v_gap
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF v_status = 'closed' AND v_held <> 0 THEN
        RAISE EXCEPTION 'a closed allocation holds allocated or reserved credit'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
END $$;
CREATE FUNCTION finance.allocation_totals_trigger() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, finance, pg_temp AS $$
BEGIN
    IF TG_TABLE_NAME = 'allocation' THEN
        PERFORM finance.check_allocation_totals(NEW.allocation_id);
    ELSE
        PERFORM finance.check_allocation_totals(k.allocation_id) FROM finance.account_key k
            WHERE k.key_id = NEW.key_id AND k.allocation_id IS NOT NULL;
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER allocation_totals AFTER INSERT OR UPDATE ON finance.allocation
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance.allocation_totals_trigger();
CREATE CONSTRAINT TRIGGER account_balance_allocation_totals AFTER UPDATE ON finance.account_balance
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance.allocation_totals_trigger();
