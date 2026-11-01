-- Creator-account portrait funding: source and disposable PG tests explicitly approved
-- by the owner on 2026-10-28 (creator-funding-approval.md). No production deployment.
-- All money posts through the existing finance.commit_event and account_balance.
INSERT INTO finance.ledger_account VALUES
 (2150,'purchased_account_reserved','general','liability',-1,true,'{payer,lot}');
INSERT INTO finance.entry_kind VALUES
 ('account_reservation','payer','account_draft','none','{system}',false),
 ('account_settlement','payer','account_draft','none','{system}',false),
 ('account_release','payer','account_draft','none','{system}',false);
INSERT INTO finance.entry_kind_rule VALUES
 ('account_reservation',2110,1),('account_reservation',9100,1),
 ('account_reservation',2150,-1),('account_reservation',9190,-1),
 ('account_settlement',2150,1),('account_settlement',5100,1),('account_settlement',5200,1),('account_settlement',9190,1),
 ('account_settlement',4110,-1),('account_settlement',2110,-1),
 ('account_settlement',2300,-1),('account_settlement',9100,-1),
 ('account_release',2150,1),('account_release',9190,1),
 ('account_release',2110,-1),('account_release',9100,-1);

CREATE TABLE finance.draft_image_consent (
 consent_id bytea PRIMARY KEY CHECK(octet_length(consent_id)=16),
 account_id bytea NOT NULL REFERENCES finance.payer,
 scope jsonb NOT NULL CHECK(jsonb_typeof(scope)='object'),
 scope_digest bytea NOT NULL CHECK(octet_length(scope_digest)=32),
 cap_micro_dollars bigint NOT NULL CHECK(cap_micro_dollars>0),
 expires_at_ms bigint NOT NULL,
 consented_at_ms bigint NOT NULL,
 revoked boolean NOT NULL DEFAULT false,
 revision bigint NOT NULL DEFAULT 1,
 CHECK(expires_at_ms>consented_at_ms)
);
ALTER TABLE finance.quote ALTER COLUMN session_id DROP NOT NULL;
ALTER TABLE finance.quote ADD COLUMN account_id bytea REFERENCES finance.payer;
ALTER TABLE finance.quote ADD COLUMN consent_id bytea REFERENCES finance.draft_image_consent;
ALTER TABLE finance.quote ADD COLUMN consent_revision bigint;
ALTER TABLE finance.quote ADD COLUMN draft_binding jsonb;
ALTER TABLE finance.quote ADD CONSTRAINT quote_owner CHECK(
 (session_id IS NOT NULL AND account_id IS NULL AND consent_id IS NULL AND draft_binding IS NULL)
 OR (session_id IS NULL AND account_id IS NOT NULL AND consent_id IS NOT NULL
  AND consent_revision IS NOT NULL AND consent_revision>0 AND draft_binding IS NOT NULL
  AND jsonb_typeof(draft_binding)='object'));
ALTER TABLE finance.reservation ADD COLUMN account_id bytea REFERENCES finance.payer;
DO $$ DECLARE r record; BEGIN
 FOR r IN SELECT conname FROM pg_constraint WHERE conrelid='finance.reservation'::regclass
  AND contype='c' AND pg_get_constraintdef(oid) LIKE '%payer_kind%' LOOP
  EXECUTE format('ALTER TABLE finance.reservation DROP CONSTRAINT %I',r.conname);
 END LOOP;
END $$;
ALTER TABLE finance.reservation ADD CONSTRAINT reservation_owner CHECK(
 (payer_kind='table' AND session_id IS NOT NULL AND account_id IS NULL AND company_class IS NULL)
 OR (payer_kind='company' AND session_id IS NULL AND account_id IS NULL AND company_class IS NOT NULL)
 OR (payer_kind='account' AND session_id IS NULL AND account_id IS NOT NULL AND company_class IS NULL));
ALTER TABLE finance.usage_event ALTER COLUMN session_id DROP NOT NULL;
ALTER TABLE finance.usage_event ADD COLUMN account_id bytea REFERENCES finance.payer;
ALTER TABLE finance.usage_event ADD COLUMN draft_usage_revision bigint CHECK(draft_usage_revision>0);
ALTER TABLE finance.usage_event ADD COLUMN draft_usage_evidence text CHECK(draft_usage_evidence IN ('reported','estimated'));
ALTER TABLE finance.usage_event ADD CONSTRAINT draft_usage_identity CHECK((draft_usage_revision IS NULL)=(draft_usage_evidence IS NULL));
ALTER TABLE finance.usage_event ADD CONSTRAINT usage_owner CHECK(
 (session_id IS NULL) <> (account_id IS NULL));
ALTER TABLE finance.operation_receipt DROP CONSTRAINT operation_receipt_namespace_check;
ALTER TABLE finance.operation_receipt ADD CONSTRAINT operation_receipt_namespace_check CHECK(
 namespace IN ('admission','allocation','withdrawal','policy_update','quote_approval',
  'refund','credit','manual_adjustment','draft_admission'));

-- Same role as reservation_share; no separate balance or ledger. The existing lot
-- conservation check sums all account keys, including account-reserved 2150.
CREATE TABLE finance.account_reservation_share (
 reservation_id bytea NOT NULL REFERENCES finance.reservation,
 lot_id bytea NOT NULL REFERENCES finance.funding_lot,
 account_id bytea NOT NULL REFERENCES finance.payer,
 held_micro_dollars bigint NOT NULL CHECK(held_micro_dollars>0),
 charged_micro_dollars bigint NOT NULL DEFAULT 0 CHECK(charged_micro_dollars>=0),
 released_micro_dollars bigint NOT NULL DEFAULT 0 CHECK(released_micro_dollars>=0),
 revision bigint NOT NULL DEFAULT 1,
 PRIMARY KEY(reservation_id,lot_id),
 CHECK(charged_micro_dollars+released_micro_dollars<=held_micro_dollars)
);
CREATE INDEX account_reservation_share_lot ON finance.account_reservation_share(account_id,lot_id);
CREATE INDEX draft_quote_consent ON finance.quote(consent_id) WHERE consent_id IS NOT NULL;

INSERT INTO finance.change_kind VALUES
 ('draft_consent_insert','draft_image_consent','insert','{consent_id}',1,false,
 '{consent_id,account_id,scope,scope_digest,cap_micro_dollars,expires_at_ms,consented_at_ms}',
 '{}','{consent_id,account_id,scope,scope_digest,cap_micro_dollars,expires_at_ms,consented_at_ms}'),
 ('draft_consent_update','draft_image_consent','update','{consent_id}',2,false,'{}',
 '{revoked}','{consent_id,revoked}'),
 ('draft_quote_insert','quote','insert','{quote_id}',1,false,
 '{quote_id,account_id,consent_id,consent_revision,draft_binding,request_digest,price_version_id,work_class,retail_estimate_micro_dollars,retail_maximum_micro_dollars,supplier_maximum_micro_dollars,created_at_ms,expires_at_ms,state}',
 '{}','{quote_id,account_id,consent_id,consent_revision,draft_binding,request_digest,price_version_id,work_class,retail_estimate_micro_dollars,retail_maximum_micro_dollars,supplier_maximum_micro_dollars,created_at_ms,expires_at_ms,state}'),
 ('draft_quote_update','quote','update','{quote_id}',2,false,'{}','{state,used_by_reservation}',
 '{quote_id,state,used_by_reservation}'),
 ('draft_share_insert','account_reservation_share','insert','{reservation_id,lot_id}',2,false,
 '{reservation_id,lot_id,account_id,held_micro_dollars}','{}',
 '{reservation_id,lot_id,account_id,held_micro_dollars}'),
 ('draft_share_update','account_reservation_share','update','{reservation_id,lot_id}',2,false,
 '{}','{charged_micro_dollars,released_micro_dollars}',
 '{reservation_id,lot_id,charged_micro_dollars,released_micro_dollars}'),
 ('draft_receipt_insert','operation_receipt','insert','{namespace,principal,operation_id}',2,false,
 '{namespace,principal,operation_id,fingerprint,outcome,result_ref,created_at_ms}','{}',
 '{namespace,principal,operation_id,fingerprint,outcome,result_ref,created_at_ms}'),
 ('draft_reservation_insert','reservation','insert','{reservation_id}',1,false,
 '{reservation_id,account_id,payer_kind,kind,operation_id,quote_id,price_version_id,retail_maximum_micro_dollars,admitted_at_ms,state}',
 '{}','{reservation_id,account_id,payer_kind,kind,operation_id,quote_id,price_version_id,retail_maximum_micro_dollars,admitted_at_ms,state}'),
 ('draft_usage_insert','usage_event','insert','{usage_event_id}',2,false,
 '{usage_event_id,attempt_id,account_id,category,quantity_text,units,draft_usage_revision,draft_usage_evidence,retail_micro_dollars,supplier_micro_dollars,supplier_purchased_micro_dollars,price_version_id,entry_id,settled_at_ms}',
 '{}','{usage_event_id,attempt_id,account_id,category,units,retail_micro_dollars,supplier_micro_dollars,supplier_purchased_micro_dollars,price_version_id,entry_id,settled_at_ms}');
INSERT INTO finance.event_kind VALUES
 ('draft_consent_recorded',false,true,'{draft_consent_insert,draft_consent_update}','{}'),
 ('draft_quote_created',false,true,'{draft_quote_insert}','{}'),
 ('draft_admission_admitted',true,true,
 '{key_create,draft_reservation_insert,draft_share_insert,provider_attempt_insert,draft_quote_update,draft_receipt_insert}',
 '{account_reservation}'),
 ('draft_settlement_applied',true,true,
 '{key_create,draft_usage_insert,reservation_update,draft_share_update,provider_attempt_update,funding_lot_update,company_loss_insert}',
 '{account_settlement}'),
 ('draft_release_applied',true,true,
 '{reservation_update,draft_share_update,provider_attempt_update}',
 '{account_release}');
INSERT INTO finance.event_update_cols VALUES
 ('draft_consent_recorded','draft_consent_update','{revoked}'),
 ('draft_admission_admitted','draft_quote_update','{state,used_by_reservation}'),
 ('draft_settlement_applied','reservation_update','{state,charged_micro_dollars,released_micro_dollars}'),
 ('draft_settlement_applied','draft_share_update','{charged_micro_dollars,released_micro_dollars}'),
 ('draft_settlement_applied','provider_attempt_update',
 '{dispatch_state,customer_state,supplier_state,supplier_outcome,usage_source,supplier_cost_micro_dollars,usability,result_at_ms,settled_at_ms}'),
 ('draft_settlement_applied','funding_lot_update','{consumed_micro_dollars}'),
 ('draft_release_applied','reservation_update','{state,charged_micro_dollars,released_micro_dollars}'),
 ('draft_release_applied','draft_share_update','{charged_micro_dollars,released_micro_dollars}'),
 ('draft_release_applied','provider_attempt_update',
 '{dispatch_state,customer_state,supplier_state,supplier_outcome,customer_released_at_ms}');

-- Add only new registry branches; preserve existing dispatcher OIDs and behavior.
DO $generate$
DECLARE k record; cols text; exprs text; sets text; criteria text; body text;
BEGIN
 FOR k IN SELECT * FROM finance.change_kind WHERE kind LIKE 'draft_%' ORDER BY kind LOOP
  IF k.op='insert' THEN
   SELECT string_agg(format('%I',u.c),',' ORDER BY u.ord),
    string_agg(CASE WHEN d.adbin IS NULL THEN format('r.%I',u.c)
     ELSE format('coalesce(r.%I,%s)',u.c,pg_get_expr(d.adbin,d.adrelid)) END,',' ORDER BY u.ord)
    INTO cols,exprs FROM unnest(k.columns) WITH ORDINALITY u(c,ord)
    JOIN pg_attribute a ON a.attrelid=format('finance.%I',k.table_name)::regclass AND a.attname=u.c
    LEFT JOIN pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum;
   EXECUTE format('CREATE FUNCTION finance.apply_%I(p_row jsonb) RETURNS void LANGUAGE sql VOLATILE SET search_path=pg_catalog,finance,pg_temp AS %L',
    k.kind,format('INSERT INTO finance.%I(%s) SELECT %s FROM jsonb_populate_record(NULL::finance.%I,p_row) r',k.table_name,cols,exprs,k.table_name));
   body:=pg_get_functiondef('finance.apply_insert(text,jsonb)'::regprocedure);
   EXECUTE replace(body,'CASE p_kind',format(E'CASE p_kind\n WHEN %L THEN PERFORM finance.apply_%I(p_row);',k.kind,k.kind));
  ELSE
   SELECT string_agg(format('%1$I=CASE WHEN p_row ? %2$L THEN r.%1$I ELSE t.%1$I END',c,c),',')
    INTO sets FROM unnest(k.updatable_cols)c;
   SELECT string_agg(format('t.%1$I=r.%1$I',c),' AND ') INTO criteria FROM unnest(k.pk_cols)c;
   EXECUTE format('CREATE FUNCTION finance.apply_%I(p_row jsonb,p_expected bigint) RETURNS bigint LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog,finance,pg_temp AS %L',
    k.kind,format('DECLARE n bigint; BEGIN UPDATE finance.%I t SET %s,revision=t.revision+1 FROM jsonb_populate_record(NULL::finance.%I,p_row) r WHERE %s AND t.revision=p_expected; GET DIAGNOSTICS n=ROW_COUNT; RETURN n; END',k.table_name,sets,k.table_name,criteria));
   body:=pg_get_functiondef('finance.apply_update(text,jsonb,bigint)'::regprocedure);
   EXECUTE replace(body,'CASE p_kind',format(E'CASE p_kind\n WHEN %L THEN RETURN finance.apply_%I(p_row,p_expected);',k.kind,k.kind));
  END IF;
 END LOOP;
END $generate$;

CREATE FUNCTION finance.account_draft_anchor(p_entry jsonb) RETURNS jsonb
LANGUAGE sql STABLE SET search_path=pg_catalog,finance,pg_temp AS $$
 SELECT jsonb_build_object('stream_kind','payer','stream_id',encode(r.account_id,'hex'),
  'campaign_id',NULL,'session_id',NULL,'attempt_id',p_entry->>'attempt_id',
  'reservation_id',encode(r.reservation_id,'hex'),'price_version_id',encode(r.price_version_id,'hex'))
 FROM finance.reservation r WHERE r.reservation_id=finance.hexbytes(p_entry->'reservation_id')
  AND r.payer_kind='account'
$$;
DO $$ DECLARE body text; BEGIN
 body:=pg_get_functiondef('finance.anchor_expectation(text,jsonb,jsonb)'::regprocedure);
 EXECUTE replace(body,'IF p_anchor = ''none'' THEN',
  'IF p_anchor = ''account_draft'' THEN RETURN finance.account_draft_anchor(p_entry); ELSIF p_anchor = ''none'' THEN');
END $$;

CREATE FUNCTION finance.check_account_draft() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,finance,pg_temp AS $$
DECLARE v_reservation bytea; v_account bytea; v_lot bytea;
 r finance.reservation%ROWTYPE; held bigint; charged bigint; released bigint; balance bigint;
BEGIN
 IF TG_TABLE_NAME='account_reservation_share' THEN
  v_reservation:=NEW.reservation_id; v_account:=NEW.account_id; v_lot:=NEW.lot_id;
  IF NOT EXISTS(SELECT 1 FROM finance.funding_lot l JOIN finance.reservation vr
   ON vr.reservation_id=v_reservation WHERE l.lot_id=v_lot
    AND l.payer_account_id=v_account AND l.class='purchased' AND vr.account_id=v_account) THEN
   RAISE EXCEPTION 'account share ownership differs' USING ERRCODE='23514';
  END IF;
 ELSIF TG_TABLE_NAME='reservation' THEN
  IF NEW.payer_kind<>'account' THEN RETURN NULL; END IF;
  v_reservation:=NEW.reservation_id;
 ELSE
  SELECT k.payer_account_id,k.lot_id INTO v_account,v_lot
   FROM finance.account_key k WHERE k.key_id=NEW.key_id AND k.code=2150;
  IF NOT FOUND THEN RETURN NULL; END IF;
 END IF;
 IF v_reservation IS NOT NULL THEN
  SELECT * INTO STRICT r FROM finance.reservation WHERE reservation_id=v_reservation;
  IF NOT EXISTS(SELECT 1 FROM finance.quote q WHERE q.quote_id=r.quote_id
   AND q.account_id=r.account_id AND q.session_id IS NULL AND q.used_by_reservation=r.reservation_id
   AND q.retail_maximum_micro_dollars=r.retail_maximum_micro_dollars
   AND q.price_version_id=r.price_version_id AND q.state='used') THEN
   RAISE EXCEPTION 'account reservation has no matching pinned quote' USING ERRCODE='23514';
  END IF;
  PERFORM 1 FROM finance.draft_image_consent c JOIN finance.quote q ON q.consent_id=c.consent_id
   WHERE q.quote_id=r.quote_id FOR UPDATE OF c;
  IF EXISTS(SELECT 1 FROM finance.quote q JOIN finance.draft_image_consent c USING(consent_id)
   WHERE q.quote_id=r.quote_id AND (SELECT coalesce(sum(v.charged_micro_dollars
    +CASE WHEN v.state='held' THEN v.retail_maximum_micro_dollars-v.charged_micro_dollars-v.released_micro_dollars ELSE 0 END),0)
    FROM finance.reservation v JOIN finance.quote x USING(quote_id) WHERE x.consent_id=c.consent_id)>c.cap_micro_dollars) THEN
   RAISE EXCEPTION 'account consent aggregate exceeds cap' USING ERRCODE='23514';
  END IF;
  IF NOT EXISTS(SELECT 1 FROM finance.provider_attempt a WHERE a.reservation_id=r.reservation_id
   AND a.account_id=r.account_id AND a.campaign_id IS NULL
   AND ((r.state='held' AND a.customer_state='held' AND a.supplier_state='exposure_open')
    OR (r.state='charged' AND a.customer_state='charged' AND a.supplier_state='cost_recorded' AND a.dispatch_state='result_known')
    OR (r.state='released' AND a.customer_state='released' AND a.supplier_state='exposure_released'
     AND a.dispatch_state IN('cancelled_unsent','verified_unsent')))) THEN
   RAISE EXCEPTION 'account reservation attempt state differs' USING ERRCODE='23514';
  END IF;
  SELECT coalesce(sum(s.held_micro_dollars),0),coalesce(sum(s.charged_micro_dollars),0),
   coalesce(sum(s.released_micro_dollars),0) INTO held,charged,released
   FROM finance.account_reservation_share s WHERE s.reservation_id=v_reservation AND s.account_id=r.account_id;
  IF held<>r.retail_maximum_micro_dollars OR charged<>r.charged_micro_dollars
   OR released<>r.released_micro_dollars THEN
   RAISE EXCEPTION 'account reservation shares differ' USING ERRCODE='23514';
  END IF;
 END IF;
 IF v_lot IS NOT NULL THEN
  SELECT coalesce(sum(s.held_micro_dollars-s.charged_micro_dollars-s.released_micro_dollars),0)
   INTO held FROM finance.account_reservation_share s WHERE s.account_id=v_account AND s.lot_id=v_lot;
  SELECT coalesce(sum(b.natural_micro_dollars),0) INTO balance FROM finance.account_key k
   JOIN finance.account_balance b USING(key_id)
   WHERE k.code=2150 AND k.payer_account_id=v_account AND k.lot_id=v_lot;
  IF held<>balance THEN RAISE EXCEPTION 'account reserved balance differs' USING ERRCODE='23514'; END IF;
 END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER account_draft_share_check AFTER INSERT OR UPDATE ON finance.account_reservation_share
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance.check_account_draft();
CREATE CONSTRAINT TRIGGER account_draft_reservation_check AFTER INSERT OR UPDATE ON finance.reservation
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance.check_account_draft();
CREATE CONSTRAINT TRIGGER account_draft_balance_check AFTER INSERT OR UPDATE ON finance.account_balance
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance.check_account_draft();

CREATE FUNCTION finance.draft_change(p_kind text,p_row jsonb,p_revision bigint DEFAULT NULL)
RETURNS jsonb LANGUAGE sql IMMUTABLE AS $$
 SELECT jsonb_build_object('kind',p_kind,'row',p_row)
  || CASE WHEN p_revision IS NULL THEN '{}'::jsonb
   ELSE jsonb_build_object('expected_revision',p_revision) END
$$;
CREATE FUNCTION finance.draft_line(p_code integer,p_currency text,p_amount bigint,
 p_account bytea DEFAULT NULL,p_lot bytea DEFAULT NULL,p_provider text DEFAULT NULL,
 p_loss_class text DEFAULT NULL)
RETURNS jsonb LANGUAGE sql IMMUTABLE AS $$
 SELECT jsonb_build_object('code',p_code,'currency',p_currency,'amount',p_amount,
  'payer',encode(p_account,'hex'),'lot',encode(p_lot,'hex'),'allocation',NULL,
  'provider',p_provider,'processor',NULL,'loss_class',p_loss_class,'jurisdiction',NULL,'promo_source',NULL)
$$;
CREATE FUNCTION finance.draft_entry(p_kind text,p_document jsonb,p_account bytea,
 p_reservation bytea,p_attempt bytea,p_price bytea,p_lines jsonb) RETURNS jsonb
LANGUAGE sql IMMUTABLE AS $$
 SELECT jsonb_build_object('entry_id',p_document->>'entry','kind',p_kind,
  'business_key',encode(sha256(finance.enc_text(p_kind)||finance.enc_bytes(p_attempt)),'hex'),
  'external_ref_kind','none','external_ref',NULL,'stream_kind','payer',
  'stream_id',encode(p_account,'hex'),'effective_at_ms',(p_document->>'now_ms')::bigint,
  'reason',CASE p_kind WHEN 'account_reservation' THEN 'admitted'
   WHEN 'account_settlement' THEN 'settled' ELSE 'released_not_dispatched' END,
  'actor_kind','system','actor_id',NULL,'approver_id',NULL,'case_ref',NULL,
  'campaign_id',NULL,'session_id',NULL,'attempt_id',encode(p_attempt,'hex'),
  'reservation_id',encode(p_reservation,'hex'),'price_version_id',encode(p_price,'hex'),
  'lines',p_lines)
$$;
CREATE FUNCTION finance.draft_event(p_kind text,p_document jsonb,p_changes jsonb,
 p_entries jsonb DEFAULT '[]') RETURNS void
LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog,finance,pg_temp AS $$
BEGIN
 PERFORM finance.commit_event(jsonb_build_object('event_id',p_document->>'event',
  'event_kind',p_kind,'payload_schema',1,'entries',p_entries,'changes',p_changes));
END $$;

-- Numeric arithmetic is exact; categories must occur once. The quote uses the
-- highest covering tariff and upward liability rounding; known usage the actual tier.
CREATE FUNCTION finance.draft_price(p_version bytea,p_provider text,p_model text,
 p_units jsonb,p_maximum boolean) RETURNS jsonb
LANGUAGE plpgsql STABLE SET search_path=pg_catalog,finance,pg_temp AS $$
DECLARE u jsonb; quantity bigint; tariff finance.price_version_tariff%ROWTYPE;
 v finance.price_version%ROWTYPE; retail bigint; supplier bigint;
 chosen_rate numeric; chosen_per numeric; common numeric; factor numeric;
 numerator numeric:=0; denominator numeric:=1;
 term_numerator numeric; term_denominator numeric;
 retail_numerator numeric; retail_denominator numeric;
BEGIN
 SELECT * INTO STRICT v FROM finance.price_version WHERE price_version_id=p_version;
 IF jsonb_typeof(p_units)<>'array' OR jsonb_array_length(p_units) NOT BETWEEN 1 AND 32
  OR (SELECT count(*)<>count(DISTINCT x->>0) FROM jsonb_array_elements(p_units)x) THEN
  RAISE EXCEPTION 'invalid units' USING ERRCODE='FD010';
 END IF;
 FOR u IN SELECT value FROM jsonb_array_elements(p_units) LOOP
  quantity:=(u->>1)::bigint;
  IF quantity<0 OR (p_maximum AND quantity=0) THEN
   RAISE EXCEPTION 'invalid units' USING ERRCODE='FD010';
  END IF;
  chosen_rate:=NULL; chosen_per:=NULL;
  FOR tariff IN SELECT t.* FROM finance.price_version_tariff t
   WHERE t.price_version_id=p_version AND t.provider=p_provider
    AND t.model=p_model AND t.category=u->>0
    AND (t.context_tier_max_tokens=0 OR quantity<=t.context_tier_max_tokens)
   ORDER BY nullif(t.context_tier_max_tokens,0) ASC NULLS LAST LOOP
   IF chosen_per IS NULL OR tariff.rate_nano_dollars::numeric*chosen_per
    >chosen_rate*tariff.per_units THEN
    chosen_rate:=tariff.rate_nano_dollars; chosen_per:=tariff.per_units;
   END IF;
   IF NOT p_maximum THEN EXIT; END IF;
  END LOOP;
  IF chosen_per IS NULL THEN RAISE EXCEPTION 'unpriced units' USING ERRCODE='FD011'; END IF;
  -- All numeric values remain integers. Even a tiny early division could change
  -- a final ceiling, floor or half-up result at a monetary boundary.
  term_numerator:=quantity::numeric*chosen_rate;
  term_denominator:=chosen_per*1000;
  common:=gcd(denominator,term_denominator);
  factor:=div(term_denominator,common);
  numerator:=numerator*factor+term_numerator*div(denominator,common);
  denominator:=denominator*factor;
  common:=gcd(numerator,denominator);
  numerator:=div(numerator,common); denominator:=div(denominator,common);
 END LOOP;
 retail_numerator:=numerator*v.markup_numerator;
 retail_denominator:=denominator*v.markup_denominator;
 IF p_maximum THEN
  retail:=div(retail_numerator+retail_denominator-1,retail_denominator);
  supplier:=div(numerator+denominator-1,denominator);
 ELSE
  retail:=div(retail_numerator,retail_denominator);
  supplier:=div(numerator*2+denominator,denominator*2);
 END IF;
 RETURN jsonb_build_object('retail',retail,'supplier',supplier,'currency',v.currency);
END $$;

-- One account scope lock serializes consent capacity with every account admission.
-- Campaign allocations of the same purchased lot contend on the existing payer/lot
-- and balance locks; supplier exposure posts in the same existing journal event.
-- One aggregate cap has one unit of currency; no conversion is inferred from a route.
CREATE FUNCTION finance.draft_job_currency(p_scope jsonb) RETURNS text
LANGUAGE plpgsql STABLE SET search_path=pg_catalog,finance,pg_temp AS $$
DECLARE route jsonb; identifier bytea; selected text; currency text;
BEGIN
 IF jsonb_typeof(p_scope->'routes') IS DISTINCT FROM 'array'
  OR jsonb_array_length(p_scope->'routes') NOT BETWEEN 1 AND 16 THEN
  RAISE EXCEPTION 'job route versions unavailable' USING ERRCODE='FD011'; END IF;
 FOR route IN SELECT value FROM jsonb_array_elements(p_scope->'routes') LOOP
  IF jsonb_typeof(route->'price_version') IS DISTINCT FROM 'array'
   OR jsonb_array_length(route->'price_version')<>16 THEN
   RAISE EXCEPTION 'job route version unavailable' USING ERRCODE='FD011'; END IF;
  SELECT decode(string_agg(lpad(to_hex(v::integer),2,'0'),'' ORDER BY ord),'hex') INTO identifier
   FROM jsonb_array_elements_text(route->'price_version') WITH ORDINALITY x(v,ord);
  SELECT v.currency INTO selected FROM finance.price_version v WHERE v.price_version_id=identifier;
  IF NOT FOUND THEN RAISE EXCEPTION 'job route version unpublished' USING ERRCODE='FD011'; END IF;
  IF currency IS NOT NULL AND currency<>selected THEN
   RAISE EXCEPTION 'job cap currencies differ' USING ERRCODE='FD018'; END IF;
  currency:=selected;
 END LOOP;
 RETURN currency;
END $$;

-- A job grant selects a pinned route; it does not mint another spending balance.
CREATE FUNCTION finance.draft_scope_matches(grant_scope jsonb,binding jsonb) RETURNS boolean
LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog,finance,pg_temp AS $$
DECLARE approved jsonb; selected jsonb:=binding->'stage_route'; call jsonb:=binding->'call';
 actual jsonb:=binding->'scope'; maximum jsonb; units jsonb; n integer;
BEGIN
 IF grant_scope->>'scope_kind' IS DISTINCT FROM 'job' THEN
  RETURN grant_scope=actual AND NOT binding ? 'call';
 END IF;
 IF selected IS NULL OR call IS NULL
  OR grant_scope-ARRAY['scope_kind','routes','config_revision'] IS DISTINCT FROM
     actual-ARRAY['provider','model','route','price_version','maximum_units']
  OR call->'job' IS DISTINCT FROM actual->'job'
  OR call->'stage' IS DISTINCT FROM binding->'stage'
  OR call->'asset' IS DISTINCT FROM binding->'asset'
  OR call->'publication' IS DISTINCT FROM binding->'publication'
  OR call->'kind' IS DISTINCT FROM selected->'kind'
  OR call->'config_revision' IS DISTINCT FROM grant_scope->'config_revision'
  OR call->'config_digest' IS DISTINCT FROM grant_scope->'config_digest'
  OR selected->'provider' IS DISTINCT FROM actual->'provider'
  OR selected->'model' IS DISTINCT FROM actual->'model'
  OR selected->'route' IS DISTINCT FROM actual->'route'
  OR selected->'price_version' IS DISTINCT FROM actual->'price_version'
  OR selected->'maximum_units' IS DISTINCT FROM actual->'maximum_units'
  OR jsonb_array_length(grant_scope->'routes') NOT BETWEEN 1 AND 16 THEN RETURN false; END IF;
 IF call->>'kind' IN ('photo_inspection','head_locations','segmentation','staging_identity','staging_facing') THEN
  IF call->'asset' IS DISTINCT FROM 'null'::jsonb OR call->'publication' IS DISTINCT FROM 'null'::jsonb THEN RETURN false; END IF;
 ELSE
  IF jsonb_typeof(call->'asset') IS DISTINCT FROM 'array' OR jsonb_typeof(call->'publication') IS DISTINCT FROM 'array' THEN RETURN false; END IF;
  IF jsonb_array_length(call->'asset')<>16 OR jsonb_array_length(call->'publication')<>16 THEN RETURN false; END IF;
 END IF;
 FOR approved IN SELECT value FROM jsonb_array_elements(grant_scope->'routes') LOOP
  IF approved-'maximum_units' IS DISTINCT FROM selected-'maximum_units' THEN CONTINUE; END IF;
  IF jsonb_array_length(approved->'maximum_units') IS DISTINCT FROM jsonb_array_length(selected->'maximum_units')
   OR jsonb_array_length(selected->'maximum_units') NOT BETWEEN 1 AND 32 THEN RETURN false; END IF;
  n:=0;
  FOR maximum IN SELECT value FROM jsonb_array_elements(approved->'maximum_units') LOOP
   units:=selected->'maximum_units'->n;
   IF maximum->0 IS DISTINCT FROM units->0 OR (units->>1)::numeric<=0
    OR (units->>1)::numeric>(maximum->>1)::numeric THEN RETURN false; END IF;
   n:=n+1;
  END LOOP;
  RETURN true;
 END LOOP;
 RETURN false;
END $$;
CREATE UNIQUE INDEX draft_quote_dispatch ON finance.quote(account_id,
 (draft_binding->'call'->'job'),(draft_binding->'call'->'stage'),(draft_binding->'call'->'dispatch'))
 WHERE draft_binding ? 'call';

CREATE FUNCTION finance.draft_image_command(p_account bytea,p jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path=pg_catalog,finance,pg_temp SET lock_timeout='2s' AS $$
#variable_conflict use_column
DECLARE action text:=p->>'action'; now_ms bigint:=(p->>'now_ms')::bigint;
 c finance.draft_image_consent%ROWTYPE; q finance.quote%ROWTYPE;
 r finance.reservation%ROWTYPE; a finance.provider_attempt%ROWTYPE;
 s finance.account_reservation_share%ROWTYPE; lot finance.funding_lot%ROWTYPE;
 receipt finance.operation_receipt%ROWTYPE; changes jsonb:='[]'; lines jsonb:='[]';
 price jsonb; result jsonb; bound jsonb:=p->'binding'; scope jsonb;
 account_text text:=encode(p_account,'hex'); consent bytea:=finance.hexbytes(p->'consent');
 v_quote bytea:=finance.hexbytes(p->'quote'); attempt bytea:=finance.hexbytes(p->'attempt');
 v_reservation bytea:=finance.hexbytes(p->'reservation'); fingerprint bytea;
 maximum bigint; supplier bigint; v_currency text; remaining bigint; available bigint;
 used numeric; amount bigint; charged bigint; released bigint; retail bigint; cost bigint;
 ordinary bigint; absorbed bigint; loss_class text; dispatch text; n integer:=0;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' OR now_ms<0
  OR octet_length(p_account)<>16 THEN
  RAISE EXCEPTION 'invalid draft transaction' USING ERRCODE='FD001';
 END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.draft_supplier:'||coalesce(
  p->'scope'->>'provider',p->'binding'->'scope'->>'provider',
  (SELECT pa.provider FROM finance.provider_attempt pa WHERE pa.attempt_id=attempt AND pa.account_id=p_account),
  (SELECT dc.scope->>'provider' FROM finance.draft_image_consent dc WHERE dc.consent_id=consent AND dc.account_id=p_account),''),0));
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.account:'||account_text,0));
 PERFORM finance.note_locks('payer',ARRAY(SELECT encode(account_id,'hex') FROM finance.payer
  WHERE account_id=p_account FOR UPDATE));
 IF NOT EXISTS(SELECT 1 FROM finance.payer WHERE account_id=p_account AND standing='active') THEN
  RAISE EXCEPTION 'account unavailable' USING ERRCODE='FD002';
 END IF;
 IF action='consent' THEN
  IF p->'scope'->>'scope_kind'='job' THEN PERFORM finance.draft_job_currency(p->'scope'); END IF;
  SELECT * INTO c FROM finance.draft_image_consent WHERE consent_id=consent FOR UPDATE;
  IF FOUND THEN
   IF c.account_id<>p_account OR c.scope<>p->'scope'
    OR c.scope_digest<>finance.hexbytes(p->'scope_digest')
    OR c.cap_micro_dollars<>(p->>'cap_micro_dollars')::bigint
    OR c.expires_at_ms<>(p->>'expires_at_ms')::bigint THEN
    RAISE EXCEPTION 'consent conflict' USING ERRCODE='FD003';
   END IF;
   RETURN jsonb_build_object('revision',c.revision,'revoked',c.revoked);
  END IF;
  PERFORM finance.draft_event('draft_consent_recorded',p,jsonb_build_array(
   finance.draft_change('draft_consent_insert',jsonb_build_object('consent_id',consent,
    'account_id',p_account,'scope',p->'scope','scope_digest',finance.hexbytes(p->'scope_digest'),
    'cap_micro_dollars',(p->>'cap_micro_dollars')::bigint,
    'expires_at_ms',(p->>'expires_at_ms')::bigint,'consented_at_ms',now_ms))));
  RETURN jsonb_build_object('revision',1,'revoked',false);
 END IF;
 IF action IN ('quote','admit','revoke') THEN
  PERFORM finance.note_locks('draft_image_consent',ARRAY(SELECT encode(consent_id,'hex')
   FROM finance.draft_image_consent WHERE consent_id=consent FOR UPDATE));
  SELECT * INTO c FROM finance.draft_image_consent WHERE consent_id=consent AND account_id=p_account;
  IF NOT FOUND THEN RAISE EXCEPTION 'consent unavailable' USING ERRCODE='FD004'; END IF;
  IF action='revoke' THEN
   IF NOT c.revoked THEN
    PERFORM finance.draft_event('draft_consent_recorded',p,jsonb_build_array(
     finance.draft_change('draft_consent_update',jsonb_build_object('consent_id',consent,'revoked',true),c.revision)));
   END IF;
   RETURN jsonb_build_object('revoked',true);
  END IF;
  IF action='admit' THEN
   SELECT * INTO receipt FROM finance.operation_receipt WHERE namespace='draft_admission'
    AND principal=p_account AND operation_id=finance.hexbytes(p->'operation');
   IF FOUND THEN
    IF receipt.fingerprint<>finance.hexbytes(p->'fingerprint') OR NOT EXISTS(
     SELECT 1 FROM finance.reservation vr JOIN finance.quote vq USING(quote_id)
     WHERE vr.reservation_id=receipt.result_ref AND vq.quote_id=v_quote
      AND vq.account_id=p_account AND vq.consent_id=consent AND vq.draft_binding=bound) THEN
     RAISE EXCEPTION 'admission conflict' USING ERRCODE='FD003';
    END IF;
    RETURN receipt.outcome||jsonb_build_object('replay',true);
   END IF;
  END IF;
  IF c.scope->>'scope_kind'='job' THEN PERFORM finance.draft_job_currency(c.scope); END IF;
  IF c.revoked THEN RAISE EXCEPTION 'consent revoked' USING ERRCODE='FD005'; END IF;
  IF c.expires_at_ms<=now_ms THEN RAISE EXCEPTION 'consent expired' USING ERRCODE='FD006'; END IF;
  IF c.revision<>(p->>'consent_revision')::bigint OR NOT finance.draft_scope_matches(c.scope,bound) THEN
   RAISE EXCEPTION 'consent binding changed' USING ERRCODE='FD007';
  END IF;
  PERFORM finance.note_locks('quote',ARRAY(SELECT encode(quote_id,'hex') FROM finance.quote
   WHERE quote_id=v_quote FOR UPDATE));
  SELECT * INTO q FROM finance.quote WHERE quote_id=v_quote;
  IF action='quote' THEN
   IF FOUND THEN
    IF q.account_id IS DISTINCT FROM p_account OR q.consent_id IS DISTINCT FROM consent
     OR q.consent_revision IS DISTINCT FROM c.revision OR q.draft_binding IS DISTINCT FROM bound
     OR q.request_digest IS DISTINCT FROM finance.hexbytes(p->'fingerprint')
     OR q.expires_at_ms<>(p->>'expires_at_ms')::bigint THEN
     RAISE EXCEPTION 'quote conflict' USING ERRCODE='FD003';
    END IF;
    RETURN jsonb_build_object('retail',q.retail_maximum_micro_dollars,
     'supplier',q.supplier_maximum_micro_dollars,'expires_at_ms',q.expires_at_ms);
   END IF;
   price:=finance.draft_price(finance.hexbytes(p->'price_version'),bound->'scope'->>'provider',
    bound->'scope'->>'model',bound->'scope'->'maximum_units',true);
   IF (p->>'expires_at_ms')::bigint<=now_ms OR (p->>'expires_at_ms')::bigint>c.expires_at_ms THEN
    RAISE EXCEPTION 'quote expired' USING ERRCODE='FD008';
   END IF;
   IF (price->>'retail')::bigint<=0 OR (price->>'retail')::bigint>c.cap_micro_dollars THEN
    RAISE EXCEPTION 'consent cap exceeded' USING ERRCODE='FD009';
   END IF;
   PERFORM finance.draft_event('draft_quote_created',p,jsonb_build_array(
    finance.draft_change('draft_quote_insert',jsonb_build_object('quote_id',v_quote,
     'account_id',p_account,'consent_id',consent,'consent_revision',c.revision,
     'draft_binding',bound,'request_digest',finance.hexbytes(p->'fingerprint'),
     'price_version_id',finance.hexbytes(p->'price_version'),'work_class','optional',
     'retail_estimate_micro_dollars',(price->>'retail')::bigint,
     'retail_maximum_micro_dollars',(price->>'retail')::bigint,
     'supplier_maximum_micro_dollars',(price->>'supplier')::bigint,
     'created_at_ms',now_ms,'expires_at_ms',(p->>'expires_at_ms')::bigint,'state','approved'))));
   RETURN price||jsonb_build_object('expires_at_ms',(p->>'expires_at_ms')::bigint);
  END IF;
  IF q.quote_id IS NULL OR q.account_id IS DISTINCT FROM p_account OR q.consent_id IS DISTINCT FROM consent
   OR q.consent_revision IS DISTINCT FROM c.revision OR q.draft_binding IS DISTINCT FROM bound
   OR q.request_digest IS DISTINCT FROM finance.hexbytes(p->'fingerprint') OR q.state<>'approved' THEN
   RAISE EXCEPTION 'quote binding changed' USING ERRCODE='FD007';
  END IF;
  IF q.expires_at_ms<=now_ms THEN RAISE EXCEPTION 'quote expired' USING ERRCODE='FD008'; END IF;
  SELECT coalesce(sum(r.charged_micro_dollars+CASE WHEN r.state='held'
    THEN r.retail_maximum_micro_dollars-r.charged_micro_dollars-r.released_micro_dollars ELSE 0 END),0)
   INTO used FROM finance.reservation r JOIN finance.quote q USING(quote_id)
   WHERE q.consent_id=consent;
  maximum:=q.retail_maximum_micro_dollars; supplier:=q.supplier_maximum_micro_dollars;
  IF used+maximum>c.cap_micro_dollars THEN
   RAISE EXCEPTION 'consent cap exceeded' USING ERRCODE='FD009';
  END IF;
  SELECT v.currency INTO v_currency FROM finance.price_version v WHERE v.price_version_id=q.price_version_id;
  -- Take the same purchased-lot locks as campaign allocation before reading balances.
  PERFORM finance.note_locks('funding_lot',ARRAY(SELECT encode(lot_id,'hex') FROM finance.funding_lot
   WHERE payer_account_id=p_account AND class='purchased' AND finance.funding_lot.currency=v_currency
   ORDER BY lot_id FOR UPDATE));
  remaining:=maximum;
  FOR lot IN SELECT * FROM finance.funding_lot WHERE payer_account_id=p_account
   AND class='purchased' AND state='active' ORDER BY created_at_ms,lot_id LOOP
   IF lot.currency<>v_currency THEN CONTINUE; END IF;
   SELECT coalesce(sum(b.natural_micro_dollars),0) INTO available FROM finance.account_key k
    JOIN finance.account_balance b USING(key_id)
    WHERE k.code=2110 AND k.payer_account_id=p_account AND k.lot_id=lot.lot_id;
   amount:=least(remaining,available);
   IF amount=0 THEN CONTINUE; END IF;
   n:=n+1;
   IF n>24 THEN RAISE EXCEPTION 'too many funding lots' USING ERRCODE='FD012'; END IF;
   changes:=changes||jsonb_build_array(
    finance.draft_change('key_create',jsonb_build_object('code',2150,'currency',v_currency,
     'payer',p_account,'lot',lot.lot_id)),
    finance.draft_change('draft_share_insert',jsonb_build_object('reservation_id',v_reservation,
     'lot_id',lot.lot_id,'account_id',p_account,'held_micro_dollars',amount)));
   lines:=lines||jsonb_build_array(finance.draft_line(2110,v_currency,amount,p_account,lot.lot_id),
    finance.draft_line(2150,v_currency,-amount,p_account,lot.lot_id));
   remaining:=remaining-amount;
   EXIT WHEN remaining=0;
  END LOOP;
  IF remaining<>0 THEN RAISE EXCEPTION 'insufficient account funds' USING ERRCODE='FD013'; END IF;
  IF supplier>0 THEN
   lines:=lines||jsonb_build_array(finance.draft_line(9100,v_currency,supplier,NULL,NULL,bound->'scope'->>'provider'),
    finance.draft_line(9190,v_currency,-supplier,NULL,NULL,bound->'scope'->>'provider'));
  END IF;
  result:=jsonb_build_object('reservation',encode(v_reservation,'hex'),'attempt',p->>'attempt',
   'retail_maximum',maximum,'supplier_maximum',supplier,'price_version',encode(q.price_version_id,'hex'),
   'admitted_at_ms',now_ms);
  changes:=changes||jsonb_build_array(
   finance.draft_change('draft_reservation_insert',jsonb_build_object('reservation_id',v_reservation,
    'account_id',p_account,'payer_kind','account','kind','provider',
    'operation_id',finance.hexbytes(p->'operation'),'quote_id',v_quote,'price_version_id',q.price_version_id,
    'retail_maximum_micro_dollars',maximum,'admitted_at_ms',now_ms,'state','held')),
   finance.draft_change('provider_attempt_insert',jsonb_build_object('attempt_id',attempt,
    'reservation_id',v_reservation,'effect_id',finance.hexbytes(p->'effect'),'account_id',p_account,
    'provider',bound->'scope'->>'provider','model',bound->'scope'->>'model','route',bound->'scope'->>'route',
    'media_asset_id',finance.hexbytes(p->'asset'),'supplier_maximum_micro_dollars',supplier,
    'supplier_exposure_micro_dollars',supplier,'dispatch_state','reserved',
    'customer_state','held','supplier_state','exposure_open','admitted_at_ms',now_ms)),
   finance.draft_change('draft_quote_update',jsonb_build_object('quote_id',v_quote,'state','used',
    'used_by_reservation',v_reservation),q.revision),
   finance.draft_change('draft_receipt_insert',jsonb_build_object('namespace','draft_admission',
    'principal',p_account,'operation_id',finance.hexbytes(p->'operation'),
    'fingerprint',finance.hexbytes(p->'fingerprint'),'outcome',result,'result_ref',v_reservation,
    'created_at_ms',now_ms)));
  PERFORM finance.draft_event('draft_admission_admitted',p,changes,jsonb_build_array(
   finance.draft_entry('account_reservation',p,p_account,v_reservation,attempt,q.price_version_id,lines)));
  RETURN result||jsonb_build_object('replay',false);
 END IF;
 -- Every later command requires the stored authenticated account/attempt mapping.
 SELECT * INTO a FROM finance.provider_attempt WHERE attempt_id=attempt AND account_id=p_account;
 IF NOT FOUND THEN RAISE EXCEPTION 'attempt unavailable' USING ERRCODE='FD004'; END IF;
 v_reservation:=a.reservation_id;
 PERFORM finance.note_locks('funding_lot',ARRAY(SELECT encode(l.lot_id,'hex') FROM finance.funding_lot l
  JOIN finance.account_reservation_share s USING(lot_id) WHERE s.reservation_id=v_reservation
  ORDER BY l.lot_id FOR UPDATE OF l));
 PERFORM finance.note_locks('reservation',ARRAY(SELECT encode(reservation_id,'hex')
  FROM finance.reservation WHERE reservation_id=v_reservation FOR UPDATE));
 PERFORM finance.note_locks('provider_attempt',ARRAY(SELECT encode(attempt_id,'hex')
  FROM finance.provider_attempt WHERE attempt_id=attempt FOR UPDATE));
 PERFORM finance.note_locks('account_reservation_share',ARRAY(SELECT encode(reservation_id,'hex')||'|'||encode(lot_id,'hex')
  FROM finance.account_reservation_share WHERE reservation_id=v_reservation ORDER BY lot_id FOR UPDATE));
 SELECT * INTO a FROM finance.provider_attempt WHERE attempt_id=attempt;
 SELECT * INTO r FROM finance.reservation WHERE reservation_id=v_reservation;
 SELECT * INTO q FROM finance.quote WHERE quote_id=r.quote_id;
 SELECT v.currency INTO v_currency FROM finance.price_version v WHERE v.price_version_id=r.price_version_id;
 IF action='lock' THEN RETURN jsonb_build_object('locked',true); END IF;
 IF action IN ('prepare','sent','release','unknown') AND q.draft_binding ? 'call' AND q.draft_binding IS DISTINCT FROM p->'binding' THEN
  RAISE EXCEPTION 'stage send binding missing' USING ERRCODE='FD007'; END IF;
 IF action IN ('prepare','sent','unknown') THEN
  IF r.state<>'held' THEN RAISE EXCEPTION 'attempt closed' USING ERRCODE='FD014'; END IF;
  dispatch:=CASE action WHEN 'prepare' THEN 'dispatch_prepared' WHEN 'sent' THEN 'sent' ELSE 'outcome_unknown' END;
  IF a.dispatch_state=dispatch THEN
   IF action='sent' AND a.dispatcher_instance_id IS DISTINCT FROM finance.hexbytes(p->'dispatcher') THEN
    RAISE EXCEPTION 'dispatch identity changed' USING ERRCODE='FD003';
   END IF;
   RETURN jsonb_build_object('replay',true,'dispatch',dispatch);
  END IF;
  IF action IN('prepare','sent') THEN
   SELECT * INTO c FROM finance.draft_image_consent WHERE consent_id=q.consent_id;
   IF c.revoked THEN RAISE EXCEPTION 'consent revoked before send' USING ERRCODE='FD005'; END IF;
   IF c.expires_at_ms<=now_ms OR q.expires_at_ms<=now_ms THEN
    RAISE EXCEPTION 'consent or quote expired before send' USING ERRCODE='FD006';
   END IF;
   IF c.revision<>q.consent_revision THEN
    RAISE EXCEPTION 'consent changed before send' USING ERRCODE='FD007';
   END IF;
  END IF;
  IF (action='prepare' AND a.dispatch_state<>'reserved')
   OR (action='sent' AND a.dispatch_state<>'dispatch_prepared')
   OR (action='unknown' AND a.dispatch_state<>'sent') THEN
   RAISE EXCEPTION 'dispatch transition refused' USING ERRCODE='FD014';
  END IF;
  result:=jsonb_build_object('attempt_id',attempt,'dispatch_state',dispatch);
  IF action='prepare' THEN result:=result||jsonb_build_object('prepared_at_ms',now_ms); END IF;
  IF action='sent' THEN result:=result||jsonb_build_object('sent_at_ms',now_ms,
   'dispatcher_instance_id',finance.hexbytes(p->'dispatcher')); END IF;
  PERFORM finance.draft_event(CASE action WHEN 'prepare' THEN 'attempt_prepared'
   WHEN 'sent' THEN 'attempt_sent' ELSE 'draft_attempt_unknown' END,p,jsonb_build_array(
   finance.draft_change('provider_attempt_update',result,a.revision)));
  RETURN jsonb_build_object('replay',false,'dispatch',dispatch);
 END IF;
 IF action NOT IN ('settle','release') THEN
  RAISE EXCEPTION 'unsupported draft command' USING ERRCODE='FD001';
 END IF;
 IF action='settle' AND q.draft_binding ? 'call' AND ((p->>'usage_revision')::bigint IS NULL
  OR (p->>'usage_revision')::bigint<=0 OR p->>'evidence' NOT IN ('reported','estimated')
  OR p->>'evidence' IS NULL) THEN RAISE EXCEPTION 'stage usage identity missing' USING ERRCODE='FD010'; END IF;
 IF r.state<>'held' THEN
  IF action='settle' THEN
   SELECT jsonb_build_object('units',u.units,'usability',a.usability,
    'usage_revision',u.draft_usage_revision,'evidence',u.draft_usage_evidence) INTO result
    FROM finance.usage_event u WHERE u.attempt_id=attempt;
   IF result IS NULL OR result->'units' IS DISTINCT FROM p->'actual_units'
    OR (result->>'usability') IS DISTINCT FROM p->>'usability'
    OR (q.draft_binding ? 'call' AND (result->'usage_revision' IS DISTINCT FROM p->'usage_revision'
     OR result->'evidence' IS DISTINCT FROM p->'evidence')) THEN
    RAISE EXCEPTION 'settlement conflict' USING ERRCODE='FD003';
   END IF;
  ELSIF r.state<>'released' THEN
   RAISE EXCEPTION 'release conflict' USING ERRCODE='FD003';
  END IF;
  RETURN jsonb_build_object('replay',true,'charged',r.charged_micro_dollars,'released',r.released_micro_dollars);
 END IF;
 IF action='release' THEN
  IF a.dispatch_state NOT IN ('reserved','dispatch_prepared','verified_unsent','cancelled_unsent') THEN
   RAISE EXCEPTION 'cannot release a potentially sent attempt' USING ERRCODE='FD015';
  END IF;
  retail:=0; cost:=0;
 ELSE
  IF a.dispatch_state NOT IN ('sent','outcome_unknown') THEN
   RAISE EXCEPTION 'cannot settle an unsent attempt' USING ERRCODE='FD014';
  END IF;
  IF p->>'usability' NOT IN ('usable','no_usable_output') THEN
   RAISE EXCEPTION 'unknown usability' USING ERRCODE='FD010';
  END IF;
  IF (SELECT array_agg(u->>0 ORDER BY u->>0) FROM jsonb_array_elements(p->'actual_units')u)
   IS DISTINCT FROM (SELECT array_agg(u->>0 ORDER BY u->>0)
    FROM jsonb_array_elements(q.draft_binding->'scope'->'maximum_units')u) THEN
   RAISE EXCEPTION 'usage categories differ from pinned maximum' USING ERRCODE='FD010';
  END IF;
  price:=finance.draft_price(r.price_version_id,a.provider,a.model,p->'actual_units',false);
  retail:=CASE WHEN p->>'usability'='usable'
   THEN least((price->>'retail')::bigint,r.retail_maximum_micro_dollars) ELSE 0 END;
  cost:=(price->>'supplier')::bigint;
  IF p->>'usability'='no_usable_output' AND cost>0 THEN
   changes:=changes||jsonb_build_array(finance.draft_change('company_loss_insert',jsonb_build_object(
    'attempt_id',attempt,'loss_class','draft_billed_failure','micro_dollars',cost,
    'entry_id',finance.hexbytes(p->'entry'))));
  ELSIF cost>a.supplier_maximum_micro_dollars THEN
   changes:=changes||jsonb_build_array(finance.draft_change('company_loss_insert',jsonb_build_object(
    'attempt_id',attempt,'loss_class','draft_supplier_overrun',
    'micro_dollars',cost-a.supplier_maximum_micro_dollars,'entry_id',finance.hexbytes(p->'entry'))));
  END IF;
  changes:=changes||jsonb_build_array(finance.draft_change('draft_usage_insert',jsonb_build_object(
   'usage_event_id',finance.hexbytes(p->'usage'),'attempt_id',attempt,'account_id',p_account,
   'category','draft_image','quantity_text','pinned_units','units',p->'actual_units',
   'draft_usage_revision',(p->>'usage_revision')::bigint,'draft_usage_evidence',p->>'evidence',
   'retail_micro_dollars',retail,'supplier_micro_dollars',cost,
   'supplier_purchased_micro_dollars',cost,'price_version_id',r.price_version_id,
   'entry_id',finance.hexbytes(p->'entry'),'settled_at_ms',now_ms)));
 END IF;
 remaining:=retail;
 FOR s IN SELECT * FROM finance.account_reservation_share WHERE reservation_id=v_reservation ORDER BY lot_id LOOP
  SELECT * INTO lot FROM finance.funding_lot WHERE lot_id=s.lot_id;
  -- A disputed/frozen lot needs the existing dispute resolution owner. Never turn
  -- known returned funds into spendable credit while the lot is restricted.
  IF lot.state<>'active' THEN RAISE EXCEPTION 'restricted funding lot' USING ERRCODE='FD016'; END IF;
  charged:=least(remaining,s.held_micro_dollars); released:=s.held_micro_dollars-charged;
  lines:=lines||jsonb_build_array(finance.draft_line(2150,v_currency,s.held_micro_dollars,p_account,s.lot_id));
  IF released>0 THEN lines:=lines||jsonb_build_array(finance.draft_line(2110,v_currency,-released,p_account,s.lot_id)); END IF;
  IF charged>0 THEN
   changes:=changes||jsonb_build_array(finance.draft_change('funding_lot_update',jsonb_build_object(
    'lot_id',s.lot_id,'consumed_micro_dollars',lot.consumed_micro_dollars+charged),lot.revision));
  END IF;
  changes:=changes||jsonb_build_array(finance.draft_change('draft_share_update',jsonb_build_object(
   'reservation_id',v_reservation,'lot_id',s.lot_id,'charged_micro_dollars',charged,
   'released_micro_dollars',released),s.revision));
  remaining:=remaining-charged;
 END LOOP;
 IF retail>0 THEN lines:=lines||jsonb_build_array(finance.draft_line(4110,v_currency,-retail)); END IF;
 IF cost>0 THEN
  ordinary:=CASE WHEN p->>'usability'='no_usable_output' THEN 0 ELSE least(cost,a.supplier_maximum_micro_dollars) END;
  absorbed:=cost-ordinary;
  loss_class:=CASE WHEN p->>'usability'='no_usable_output' THEN 'draft_billed_failure' ELSE 'draft_supplier_overrun' END;
  IF ordinary>0 THEN lines:=lines||jsonb_build_array(finance.draft_line(5100,v_currency,ordinary,NULL,NULL,a.provider)); END IF;
  IF absorbed>0 THEN
   changes:=changes||jsonb_build_array(finance.draft_change('key_create',jsonb_build_object(
    'code',5200,'currency',v_currency,'provider',a.provider,'loss_class',loss_class)));
   lines:=lines||jsonb_build_array(finance.draft_line(5200,v_currency,absorbed,NULL,NULL,a.provider,loss_class));
  END IF;
  lines:=lines||jsonb_build_array(finance.draft_line(2300,v_currency,-cost,NULL,NULL,a.provider));
 END IF;
 IF a.supplier_exposure_micro_dollars>0 THEN
  lines:=lines||jsonb_build_array(finance.draft_line(9190,v_currency,a.supplier_exposure_micro_dollars,NULL,NULL,a.provider),
   finance.draft_line(9100,v_currency,-a.supplier_exposure_micro_dollars,NULL,NULL,a.provider));
 END IF;
 changes:=changes||jsonb_build_array(finance.draft_change('reservation_update',jsonb_build_object(
  'reservation_id',v_reservation,'state',CASE WHEN action='release' THEN 'released' ELSE 'charged' END,
  'charged_micro_dollars',retail,'released_micro_dollars',r.retail_maximum_micro_dollars-retail),r.revision));
 result:=jsonb_build_object('attempt_id',attempt,
  'dispatch_state',CASE WHEN action='release' THEN 'cancelled_unsent' ELSE 'result_known' END,
  'customer_state',CASE WHEN action='release' THEN 'released' ELSE 'charged' END,
  'supplier_state',CASE WHEN action='release' THEN 'exposure_released' ELSE 'cost_recorded' END,
  'supplier_outcome',CASE WHEN cost>0 THEN 'billed' ELSE 'not_billed' END);
 IF action='release' THEN result:=result||jsonb_build_object('customer_released_at_ms',now_ms);
 ELSE result:=result||jsonb_build_object('usage_source',coalesce(p->>'evidence','reported'),'supplier_cost_micro_dollars',cost,
  'usability',p->>'usability','result_at_ms',now_ms,'settled_at_ms',now_ms); END IF;
 changes:=changes||jsonb_build_array(finance.draft_change('provider_attempt_update',result,a.revision));
 PERFORM finance.draft_event(CASE WHEN action='release' THEN 'draft_release_applied' ELSE 'draft_settlement_applied' END,
  p,changes,jsonb_build_array(finance.draft_entry(CASE WHEN action='release' THEN 'account_release'
   ELSE 'account_settlement' END,p,p_account,v_reservation,attempt,r.price_version_id,lines)));
 RETURN jsonb_build_object('replay',false,'charged',retail,'released',r.retail_maximum_micro_dollars-retail);
END $$;

INSERT INTO finance.event_kind VALUES
 ('draft_attempt_unknown',false,true,'{provider_attempt_update}','{}');
INSERT INTO finance.event_update_cols VALUES
 ('draft_attempt_unknown','provider_attempt_update','{dispatch_state}');

-- Consent terms never change in place, even for privileged accidental DML.
CREATE FUNCTION finance.draft_consent_immutable() RETURNS trigger
LANGUAGE plpgsql AS $$ BEGIN
 IF to_jsonb(NEW)-ARRAY['revoked','revision'] IS DISTINCT FROM to_jsonb(OLD)-ARRAY['revoked','revision']
  OR NOT NEW.revoked OR OLD.revoked THEN
  RAISE EXCEPTION 'consent terms are immutable' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER draft_consent_immutable BEFORE UPDATE ON finance.draft_image_consent
 FOR EACH ROW EXECUTE FUNCTION finance.draft_consent_immutable();

-- New admission stops after a confirmed supplier overrun until the incident owner
-- supplies a revised policy; historical receipts and settlement remain usable.
DO $$ DECLARE body text; BEGIN
 body:=pg_get_functiondef('finance.draft_image_command(bytea,jsonb)'::regprocedure);
 body:=replace(body,'  maximum:=q.retail_maximum_micro_dollars;',
  '  IF EXISTS(SELECT 1 FROM finance.provider_attempt pa WHERE pa.supplier_cost_micro_dollars>pa.supplier_maximum_micro_dollars AND pa.provider=q.draft_binding->''scope''->>''provider'') THEN RAISE EXCEPTION ''supplier admissions blocked'' USING ERRCODE=''FD017''; END IF; maximum:=q.retail_maximum_micro_dollars;');
 EXECUTE body;
END $$;

GRANT SELECT ON finance.draft_image_consent,finance.account_reservation_share TO df_finance;
GRANT SELECT,INSERT ON finance.draft_image_consent,finance.account_reservation_share TO df_finance_definer;
GRANT UPDATE(revoked,revision) ON finance.draft_image_consent TO df_finance_definer;
GRANT UPDATE(charged_micro_dollars,released_micro_dollars,revision) ON finance.account_reservation_share TO df_finance_definer;
-- New account columns are insert-only through their registered changes.
DO $$ DECLARE f record; BEGIN
 FOR f IN SELECT p.oid::regprocedure signature FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
  WHERE n.nspname='finance' AND (p.proname LIKE 'draft_%' OR p.proname LIKE 'apply_draft_%'
   OR p.proname IN ('account_draft_anchor','check_account_draft')) LOOP
  EXECUTE format('REVOKE ALL ON FUNCTION %s FROM PUBLIC',f.signature);
  EXECUTE format('GRANT EXECUTE ON FUNCTION %s TO df_finance_definer',f.signature);
 END LOOP;
END $$;
ALTER FUNCTION finance.draft_image_command(bytea,jsonb) OWNER TO df_finance_definer;
GRANT EXECUTE ON FUNCTION finance.draft_image_command(bytea,jsonb) TO df_finance;

-- Account finance linkage uses the common immutable publication allocation. The
-- standalone finance harness has no imagery schema; full composition applies this
-- migration after 0039 and receives these scope extensions.
DO $$ BEGIN
 IF to_regclass('imagery.image_finance_admission_binding') IS NOT NULL THEN
  ALTER TABLE imagery.image_finance_admission_binding ADD COLUMN account_id bytea;
  ALTER TABLE imagery.image_finance_admission_binding ALTER COLUMN tenant_id DROP NOT NULL;
  ALTER TABLE imagery.image_finance_admission_binding ALTER COLUMN campaign_id DROP NOT NULL;
  ALTER TABLE imagery.image_finance_admission_binding ALTER COLUMN session_id DROP NOT NULL;
  ALTER TABLE imagery.image_finance_admission_binding DROP CONSTRAINT image_finance_admission_binding_receipt_namespace_check;
  ALTER TABLE imagery.image_finance_admission_binding ADD CONSTRAINT image_finance_binding_scope CHECK(
   (account_id IS NULL AND tenant_id IS NOT NULL AND campaign_id IS NOT NULL
    AND session_id IS NOT NULL AND receipt_namespace='admission')
   OR (account_id IS NOT NULL AND octet_length(account_id)=16 AND tenant_id IS NULL AND campaign_id IS NULL
    AND session_id IS NULL AND receipt_namespace='draft_admission'));
  ALTER TABLE imagery.image_finance_admission_binding ADD COLUMN receipt_principal bytea
   GENERATED ALWAYS AS(coalesce(campaign_id,account_id)) STORED;
  ALTER TABLE imagery.image_finance_admission_binding DROP CONSTRAINT image_finance_admission_binding_receipt_fkey;
  ALTER TABLE imagery.image_finance_admission_binding DROP CONSTRAINT image_finance_admission_binding_scoped_operation;
  ALTER TABLE imagery.image_finance_admission_binding ADD CONSTRAINT image_finance_admission_binding_scoped_operation
   UNIQUE(receipt_namespace,receipt_principal,operation_id);
  ALTER TABLE imagery.image_finance_admission_binding ADD CONSTRAINT image_finance_admission_binding_receipt_fkey
   FOREIGN KEY(receipt_namespace,receipt_principal,operation_id)
   REFERENCES finance.operation_receipt(namespace,principal,operation_id) DEFERRABLE INITIALLY DEFERRED;
 END IF;
END $$;

-- The registered posting boundary verifies the account event's money equations,
-- independently of the command facade that constructed the plan.
CREATE FUNCTION finance.draft_equations(p jsonb) RETURNS void
LANGUAGE plpgsql STABLE SET search_path=pg_catalog,finance,pg_temp AS $$
DECLARE held bigint; maximum bigint; charged bigint; released bigint; cost bigint;
 l record; delta bigint; exposure bigint; attempted bigint;
BEGIN
 IF finance.event_has_kind(p,ARRAY['account_reservation']) THEN
  SELECT coalesce(sum((c->'row'->>'held_micro_dollars')::bigint),0) INTO held
   FROM finance.event_changes(p,'draft_share_insert') c;
  SELECT coalesce(sum((c->'row'->>'retail_maximum_micro_dollars')::bigint),0) INTO maximum
   FROM finance.event_changes(p,'draft_reservation_insert') c;
  IF held<>maximum OR held<>(SELECT -coalesce(sum(amount),0) FROM finance.event_lines(p) WHERE code=2150)
   OR held<>(SELECT coalesce(sum(amount),0) FROM finance.event_lines(p) WHERE code=2110) THEN
   PERFORM finance.eq_fail('DA1','account hold differs from reservation and shares');
  END IF;
  SELECT coalesce(sum((c->'row'->>'supplier_exposure_micro_dollars')::bigint),0) INTO attempted
   FROM finance.event_changes(p,'provider_attempt_insert') c;
  IF attempted<>(SELECT coalesce(sum(amount),0) FROM finance.event_lines(p) WHERE code=9100) THEN
   PERFORM finance.eq_fail('DA2','account supplier exposure differs');
  END IF;
  FOR l IN SELECT * FROM finance.event_lines(p) WHERE code IN(2110,2150) LOOP
   IF NOT EXISTS(SELECT 1 FROM finance.event_changes(p,'draft_share_insert') c
    WHERE finance.rowbytes(c->'row'->>'lot_id')=l.lot
     AND finance.rowbytes(c->'row'->>'account_id')=l.payer
     AND finance.rowbytes(c->'row'->>'reservation_id')=l.entry_reservation
     AND (c->'row'->>'held_micro_dollars')::bigint=abs(l.amount)) THEN
    PERFORM finance.eq_fail('DA3','account reservation line has no exact owned share');
   END IF;
  END LOOP;
 END IF;
 IF finance.event_has_kind(p,ARRAY['account_settlement','account_release']) THEN
  SELECT coalesce(sum((c->'row'->>'charged_micro_dollars')::bigint-s.charged_micro_dollars),0),
   coalesce(sum((c->'row'->>'released_micro_dollars')::bigint-s.released_micro_dollars),0)
   INTO charged,released FROM finance.event_changes(p,'draft_share_update') c
   JOIN finance.account_reservation_share s
    ON s.reservation_id=finance.rowbytes(c->'row'->>'reservation_id')
    AND s.lot_id=finance.rowbytes(c->'row'->>'lot_id');
  IF charged<>(SELECT -coalesce(sum(amount),0) FROM finance.event_lines(p) WHERE code=4110)
   OR released<>(SELECT -coalesce(sum(amount),0) FROM finance.event_lines(p) WHERE code=2110)
   OR charged+released<>(SELECT coalesce(sum(amount),0) FROM finance.event_lines(p) WHERE code=2150) THEN
   PERFORM finance.eq_fail('DA4','account settlement lines differ from charged and released shares');
  END IF;
  SELECT coalesce(sum((c->'row'->>'supplier_micro_dollars')::bigint),0) INTO cost
   FROM finance.event_changes(p,'draft_usage_insert') c;
  IF cost<>(SELECT coalesce(sum(amount),0) FROM finance.event_lines(p) WHERE code IN(5100,5200))
   OR cost<>(SELECT -coalesce(sum(amount),0) FROM finance.event_lines(p) WHERE code=2300)
   OR charged<>(SELECT coalesce(sum((c->'row'->>'retail_micro_dollars')::bigint),0)
    FROM finance.event_changes(p,'draft_usage_insert') c) THEN
   PERFORM finance.eq_fail('DA5','account usage differs from supplier cost and retail revenue');
  END IF;
  SELECT coalesce(sum(a.supplier_exposure_micro_dollars),0) INTO attempted
   FROM finance.event_changes(p,'provider_attempt_update') c JOIN finance.provider_attempt a
    ON a.attempt_id=finance.rowbytes(c->'row'->>'attempt_id')
   WHERE a.account_id IS NOT NULL AND a.supplier_state='exposure_open';
  IF attempted<>(SELECT -coalesce(sum(amount),0) FROM finance.event_lines(p) WHERE code=9100) THEN
   PERFORM finance.eq_fail('DA6','account settled supplier exposure differs');
  END IF;
  FOR l IN SELECT * FROM finance.event_lines(p) WHERE code IN(2110,2150) LOOP
   IF NOT EXISTS(SELECT 1 FROM finance.account_reservation_share s WHERE s.reservation_id=l.entry_reservation
    AND s.lot_id=l.lot AND s.account_id=l.payer) THEN
    PERFORM finance.eq_fail('DA7','account settlement line has no owned reservation share');
   END IF;
  END LOOP;
 END IF;
END $$;
DO $$ DECLARE body text; BEGIN
 body:=pg_get_functiondef('finance.check_equations(jsonb)'::regprocedure);
 EXECUTE replace(body,'PERFORM finance.eq_e1(p);','PERFORM finance.draft_equations(p); PERFORM finance.eq_e1(p);');
END $$;
REVOKE ALL ON FUNCTION finance.draft_equations(jsonb) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.draft_equations(jsonb) TO df_finance_definer;

ALTER FUNCTION finance.check_account_draft() OWNER TO df_finance_definer;

-- Resolve once by canonical call identity, then delegate every monetary operation to
-- the same registered account command. Candidate ids are discarded on exact replay.
CREATE FUNCTION finance.draft_stage_command(p_account bytea,p jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path=pg_catalog,finance,pg_temp SET lock_timeout='2s' AS $$
DECLARE q finance.quote%ROWTYPE; c finance.draft_image_consent%ROWTYPE;
 bound jsonb:=p->'binding'; document jsonb:=p; result jsonb; expiry bigint;
BEGIN
 IF p->>'action'='stage_quote' THEN
  PERFORM pg_advisory_xact_lock(hashtextextended('finance.draft_supplier:'||coalesce(bound->'scope'->>'provider',''),0));
  PERFORM pg_advisory_xact_lock(hashtextextended('finance.account:'||encode(p_account,'hex'),0));
  PERFORM finance.note_locks('payer',ARRAY(SELECT encode(account_id,'hex') FROM finance.payer WHERE account_id=p_account FOR UPDATE));
  SELECT * INTO c FROM finance.draft_image_consent WHERE consent_id=finance.hexbytes(p->'consent')
   AND account_id=p_account FOR UPDATE;
  IF NOT FOUND THEN RAISE EXCEPTION 'grant unavailable' USING ERRCODE='FD004'; END IF;
  IF NOT finance.draft_scope_matches(c.scope,bound) THEN
   RAISE EXCEPTION 'stage route differs' USING ERRCODE='FD007'; END IF;
  SELECT * INTO q FROM finance.quote WHERE account_id=p_account
   AND draft_binding->'call'->'job'=bound->'call'->'job'
   AND draft_binding->'call'->'stage'=bound->'call'->'stage'
   AND draft_binding->'call'->'dispatch'=bound->'call'->'dispatch';
  IF FOUND THEN
   IF q.consent_id IS DISTINCT FROM c.consent_id OR q.consent_revision IS DISTINCT FROM (p->>'consent_revision')::bigint
    OR q.request_digest IS DISTINCT FROM finance.hexbytes(p->'stage_fingerprint')
    OR q.draft_binding-ARRAY['operation','attempt','effect'] IS DISTINCT FROM bound-ARRAY['operation','attempt','effect'] THEN
    RAISE EXCEPTION 'stage quote conflict' USING ERRCODE='FD003'; END IF;
  ELSE
   IF (p->>'quote_lifetime_ms')::numeric<=0 THEN
    RAISE EXCEPTION 'quote lifetime invalid' USING ERRCODE='FD008'; END IF;
   expiry:=least((p->>'now_ms')::numeric+(p->>'quote_lifetime_ms')::numeric,c.expires_at_ms)::bigint;
   SELECT document||jsonb_build_object('action','quote','expires_at_ms',expiry,
    'fingerprint',p->'stage_fingerprint',
    'price_version',encode(decode(string_agg(lpad(to_hex(v::integer),2,'0'),''),'hex'),'hex')) INTO document
    FROM jsonb_array_elements_text(bound->'scope'->'price_version')x(v);
   result:=finance.draft_image_command(p_account,document);
   SELECT * INTO STRICT q FROM finance.quote WHERE quote_id=finance.hexbytes(p->'quote');
  END IF;
  RETURN jsonb_build_object('quote',encode(q.quote_id,'hex'),'consent',encode(q.consent_id,'hex'),
   'consent_revision',q.consent_revision,'binding',q.draft_binding,'fingerprint',encode(q.request_digest,'hex'),
   'retail',q.retail_maximum_micro_dollars,'supplier',q.supplier_maximum_micro_dollars,'expires_at_ms',q.expires_at_ms);
 END IF;
 SELECT * INTO q FROM finance.quote WHERE quote_id=finance.hexbytes(p->'quote') AND account_id=p_account;
 IF NOT FOUND OR q.draft_binding IS DISTINCT FROM bound OR q.request_digest IS DISTINCT FROM finance.hexbytes(p->'fingerprint')
  OR q.draft_binding->'call' IS NULL THEN RAISE EXCEPTION 'stage settlement differs' USING ERRCODE='FD007'; END IF;
 RETURN finance.draft_image_command(p_account,p);
END $$;
REVOKE ALL ON FUNCTION finance.draft_stage_command(bytea,jsonb),finance.draft_scope_matches(jsonb,jsonb) FROM PUBLIC;
ALTER FUNCTION finance.draft_stage_command(bytea,jsonb) OWNER TO df_finance_definer;
GRANT EXECUTE ON FUNCTION finance.draft_scope_matches(jsonb,jsonb) TO df_finance_definer;
GRANT EXECUTE ON FUNCTION finance.draft_stage_command(bytea,jsonb) TO df_finance;


-- The existing immutable linkage also records no-media decisions and paid children.
DO $$ BEGIN
 IF to_regclass('imagery.image_finance_admission_binding') IS NOT NULL THEN
  ALTER TABLE imagery.image_finance_admission_binding ALTER COLUMN publication_attempt DROP NOT NULL;
  ALTER TABLE imagery.image_finance_admission_binding ALTER COLUMN stage_id DROP NOT NULL;
  ALTER TABLE imagery.image_finance_admission_binding ALTER COLUMN asset_id DROP NOT NULL;
  ALTER TABLE imagery.image_finance_admission_binding ADD COLUMN execution_stage_id bytea;
  ALTER TABLE imagery.image_finance_admission_binding ADD COLUMN dispatch_id bytea;
  ALTER TABLE imagery.image_finance_admission_binding ADD COLUMN parent_dispatch bytea;
  ALTER TABLE imagery.image_finance_admission_binding ADD COLUMN stage_kind text;
  ALTER TABLE imagery.image_finance_admission_binding ADD COLUMN stage_call jsonb;
  ALTER TABLE imagery.image_finance_admission_binding ADD COLUMN route_key bytea;
  ALTER TABLE imagery.image_finance_admission_binding ADD COLUMN cost_route jsonb;
  ALTER TABLE imagery.image_finance_admission_binding ADD COLUMN planned_asset bytea;
  ALTER TABLE imagery.image_finance_admission_binding ADD COLUMN planned_publication bytea;
  ALTER TABLE imagery.image_finance_admission_binding ADD CONSTRAINT stage_media_presence CHECK((
   (publication_attempt IS NULL AND stage_id IS NULL AND asset_id IS NULL AND stage_kind IN ('classification','photo_inspection','head_locations','segmentation','staging_identity','staging_facing'))
   OR (publication_attempt IS NOT NULL AND stage_id IS NOT NULL AND asset_id IS NOT NULL)) IS TRUE);
  ALTER TABLE imagery.image_finance_admission_binding ADD CONSTRAINT stage_binding_shape CHECK((
   (execution_stage_id IS NULL AND dispatch_id IS NULL AND parent_dispatch IS NULL AND stage_kind IS NULL
    AND stage_call IS NULL AND route_key IS NULL AND cost_route IS NULL AND planned_asset IS NULL AND planned_publication IS NULL)
   OR (octet_length(execution_stage_id)=16 AND execution_stage_id IS NOT NULL
    AND octet_length(dispatch_id)=16 AND dispatch_id IS NOT NULL
    AND (parent_dispatch IS NULL OR octet_length(parent_dispatch)=16)
    AND stage_kind IN ('classification','generation','edit','inspection','correction','extraction','photo_inspection','head_locations','segmentation','staging_identity','staging_facing')
    AND jsonb_typeof(stage_call)='object' AND stage_call IS NOT NULL
    AND octet_length(route_key)=32 AND route_key IS NOT NULL AND cost_route IS NOT NULL AND jsonb_typeof(cost_route)='object'
    AND ((stage_kind IN ('photo_inspection','head_locations','segmentation','staging_identity','staging_facing') AND planned_asset IS NULL AND planned_publication IS NULL)
     OR (stage_kind NOT IN ('photo_inspection','head_locations','segmentation','staging_identity','staging_facing')
      AND octet_length(planned_asset)=16 AND planned_asset IS NOT NULL
      AND octet_length(planned_publication)=16 AND planned_publication IS NOT NULL)))) IS TRUE);
  CREATE INDEX image_finance_binding_execution_stage ON imagery.image_finance_admission_binding(job_id,coalesce(execution_stage_id,stage_id));
  CREATE UNIQUE INDEX image_finance_binding_dispatch ON imagery.image_finance_admission_binding(dispatch_id)
   WHERE dispatch_id IS NOT NULL;
  IF to_regclass('imagery.image_dispatch_attempt') IS NOT NULL THEN
   ALTER TABLE imagery.image_finance_admission_binding ADD CONSTRAINT image_finance_binding_dispatch_fk
    FOREIGN KEY(job_id,execution_stage_id,dispatch_id)
    REFERENCES imagery.image_dispatch_attempt(job_id,stage_id,dispatch_id) DEFERRABLE INITIALLY DEFERRED;
  END IF;
 END IF;
END $$;

DO $$ BEGIN
 IF to_regclass('imagery.image_dispatch_attempt') IS NOT NULL THEN
  GRANT SELECT ON imagery.image_execution_stage,imagery.image_dispatch_attempt,imagery.image_stage_cursor,
   imagery.image_execution_capacity,imagery.image_execution_gate TO df_imagery_admission;
  GRANT INSERT ON imagery.image_dispatch_attempt,imagery.image_stage_cursor TO df_imagery_admission;
  GRANT UPDATE(capacity_reserved,updated_at_ms) ON imagery.image_execution_stage TO df_imagery_admission;
  GRANT UPDATE(settlement,updated_at_ms) ON imagery.image_dispatch_attempt TO df_imagery_admission;
  GRANT UPDATE(current_dispatch,updated_at_ms) ON imagery.image_stage_cursor TO df_imagery_admission;
  GRANT UPDATE(in_flight,updated_at_ms) ON imagery.image_execution_capacity TO df_imagery_admission;
  GRANT UPDATE(updated_at_ms) ON imagery.image_execution_gate TO df_imagery_admission;
  GRANT UPDATE(revision) ON imagery.image_job TO df_imagery_admission;
 END IF;
END $$;

-- Hosted egress state uses the execution owner's same-transaction finance proof.
DO $$ BEGIN
 IF to_regprocedure('imagery.mark_hosted_dispatch_sent(bytea,bytea,bytea,bytea,bytea,bigint,bigint,bytea,bytea,bytea)') IS NOT NULL THEN
  GRANT EXECUTE ON FUNCTION imagery.mark_hosted_dispatch_sent(bytea,bytea,bytea,bytea,bytea,bigint,bigint,bytea,bytea,bytea)
   TO df_imagery_admission;
 END IF;
END $$;
