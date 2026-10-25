-- Test of the feed-epoch restore step. STATUS: UNVERIFIED (never run: PostgreSQL is not started on the authoring machine). Run after the
-- migrations in the staging mirror (as the migrator or the administrator), non-destructively: everything happens in one transaction
-- that is rolled back.
--   psql -v ON_ERROR_STOP=1 -f deploy/db/test-feed-epochs.sql
-- Covers planning/campaigns-and-table-research.md test `two_restores_from_one_backup_produce_two_different_feed_epochs` at the database
-- level, together with the shape of the value: 16 bytes, random (not a counter), never reused.
\set ON_ERROR_STOP on
BEGIN;

DO $shape$
DECLARE
  v_distinct bigint;
  v_bad bigint;
BEGIN
  -- 1000 draws: all 16 bytes, all distinct, none the all-zero value, no fixed leading bytes (not a counter).
  SELECT count(DISTINCT e), count(*) FILTER (WHERE octet_length(e) <> 16 OR e = '\x00000000000000000000000000000000'::bytea)
    INTO v_distinct, v_bad
    FROM (SELECT campaigns.fresh_feed_epoch() AS e FROM generate_series(1, 1000)) AS draws;
  IF v_bad > 0 THEN RAISE EXCEPTION 'fresh_feed_epoch returned a value that is not 16 bytes or is all zero'; END IF;
  IF v_distinct <> 1000 THEN RAISE EXCEPTION 'fresh_feed_epoch repeated a value: % distinct of 1000', v_distinct; END IF;
  IF (SELECT count(DISTINCT substring(campaigns.fresh_feed_epoch() FROM 1 FOR 2)) FROM generate_series(1, 200)) < 20 THEN
    RAISE EXCEPTION 'fresh_feed_epoch leading bytes look like a counter, not random';
  END IF;
END
$shape$;

-- Two restores "from one backup": snapshot the epochs, restore, roll the epochs back to the snapshot (the backup), restore again.
CREATE TEMP TABLE backup_epochs ON COMMIT DROP AS SELECT campaign_id, feed_epoch FROM campaigns.campaign;
SELECT campaigns.restore_feed_epochs() AS first_count \gset
CREATE TEMP TABLE first_restore ON COMMIT DROP AS SELECT campaign_id, feed_epoch FROM campaigns.campaign;
UPDATE campaigns.campaign c SET feed_epoch = b.feed_epoch FROM backup_epochs b WHERE c.campaign_id = b.campaign_id;
SELECT campaigns.restore_feed_epochs() AS second_count \gset

DO $compare$
DECLARE
  v_same_as_backup bigint;
  v_same_as_first bigint;
BEGIN
  SELECT count(*) INTO v_same_as_backup
    FROM campaigns.campaign c JOIN backup_epochs b USING (campaign_id) WHERE c.feed_epoch = b.feed_epoch;
  SELECT count(*) INTO v_same_as_first
    FROM campaigns.campaign c JOIN first_restore f USING (campaign_id) WHERE c.feed_epoch = f.feed_epoch;
  IF v_same_as_backup > 0 THEN RAISE EXCEPTION 'a restore kept the backup''s feed_epoch for % campaign(s)', v_same_as_backup; END IF;
  IF v_same_as_first > 0 THEN RAISE EXCEPTION 'two restores from one backup gave the same feed_epoch for % campaign(s)', v_same_as_first; END IF;
END
$compare$;

\echo feed epoch tests ok (:first_count and :second_count campaign(s) re-randomized, then rolled back)
ROLLBACK;
