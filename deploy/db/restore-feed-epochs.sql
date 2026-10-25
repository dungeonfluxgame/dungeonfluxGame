-- Restore and failover runbook step: give EVERY campaign a fresh random 128-bit feed_epoch (planning/campaigns-and-table-research.md 5.1, D29;
-- planning/service-operations.md, "Restore and failover"). STATUS: UNVERIFIED (never run against PostgreSQL; the function it calls is
-- migration 0008's campaigns.restore_feed_epochs()).
--
-- WHY. A consumer of the campaign feed (the table's players, F's payer standing, O2's media gateway) stores a position (event_seq) with the
-- epoch it was read in. After a restore from backup, or a failover that lost committed transactions, event_seq values are reused. A counter
-- ("plus one") would give two restores from the SAME backup the SAME epoch, and consumers already on that epoch would see nothing change.
-- So the epoch is a fresh RANDOM value, and every consumer that sees a different one drops its cached authority and re-snapshots.
--
-- WHEN (all of it before service resumes; never after):
--   restore: the database is restored, every application process is stopped, the recovery epoch is raised through the journal head
--            compare-and-set (O1/O2's operator step; TODO(O1): the command), THEN this script, THEN services start.
--   failover that lost commits: O1's detection has set df_meta.service_state.serving = 'halted_regression'; this script runs while the
--            service is halted, in the same operator step that raises the recovery epoch and sets serving back to 'serving'.
-- It refuses to run unless the service is explicitly halted (service_state.serving = halted_regression) or no application role is
-- connected; a missing service_state row counts as not halted.
--
-- RUN AS the cluster administrator (the migrator also works), through the pinned psql path:
--   deploy/db/run-admin-sql.sh --ssh-host dfdeploy@APP_HOST --admin-password-file FILE --host CLUSTER_PRIVATE_HOST --port 25060 \
--       --dbname dungeonflux --user doadmin  deploy/db/restore-feed-epochs.sql
-- It prints the number of campaigns re-randomized; record it and the time in the incident log. Idempotent in effect (running it twice
-- gives a third epoch, which is harmless), one transaction.
\set ON_ERROR_STOP on
BEGIN;

DO $guard$
DECLARE
  v_serving text;
  v_connected bigint;
BEGIN
  SELECT serving INTO v_serving FROM df_meta.service_state;
  SELECT count(*) INTO v_connected FROM pg_stat_activity
   WHERE usename LIKE 'df\_%' AND usename <> 'df_migrator' AND pid <> pg_backend_pid();
  -- Fail CLOSED: proceed only when the service is explicitly halted (the only non-serving value df_meta.service_state allows) or
  -- no application connection is open. A missing service_state row gives a NULL state, which counts as NOT halted.
  IF NOT (v_serving IS NOT DISTINCT FROM 'halted_regression' OR v_connected = 0) THEN
    RAISE EXCEPTION 'refusing: the service state is % and % application connection(s) are open; halt the service or stop every application process first', coalesce(v_serving, 'missing'), v_connected;
  END IF;
END
$guard$;

CREATE TEMP TABLE old_feed_epochs ON COMMIT DROP AS
  SELECT campaign_id, feed_epoch FROM campaigns.campaign;

SELECT campaigns.restore_feed_epochs() AS re_randomized \gset

DO $verify$
DECLARE
  v_total bigint;
  v_unchanged bigint;
  v_distinct bigint;
BEGIN
  SELECT count(*) INTO v_total FROM campaigns.campaign;
  SELECT count(*) INTO v_unchanged
    FROM campaigns.campaign c JOIN old_feed_epochs o USING (campaign_id) WHERE c.feed_epoch = o.feed_epoch;
  SELECT count(DISTINCT feed_epoch) INTO v_distinct FROM campaigns.campaign;
  IF v_unchanged > 0 THEN
    RAISE EXCEPTION 'abort: % campaign(s) kept their old feed_epoch', v_unchanged;
  END IF;
  IF v_distinct <> v_total THEN
    RAISE EXCEPTION 'abort: feed epochs are not all distinct (% distinct of %)', v_distinct, v_total;
  END IF;
END
$verify$;

COMMIT;
\echo feed epochs re-randomized for :re_randomized campaign(s)
