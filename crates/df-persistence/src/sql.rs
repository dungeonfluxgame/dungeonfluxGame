// Explicit base types and immutable committed-run receipt binding; statements from SQL09.
pub(crate) const LOCK_SESSION: &str = r#"SELECT run_id::bytea AS run_id, recovery_epoch::text, in_epoch_sequence::text, owner_fence::bytea AS owner_fence,
       GREATEST(0, CEIL(EXTRACT(EPOCH FROM lease_until - clock_timestamp())))::bigint AS lease_remaining_seconds
FROM df_game.sessions
WHERE tenant_id = $1::bytea AND session_id = $2::bytea
FOR UPDATE;"#;

pub(crate) const LOOKUP_OPERATION: &str = r#"SELECT o.fingerprint_version, o.canonical_fingerprint, o.committed_epoch::text,
       o.committed_sequence::text, c.run_id::bytea AS committed_run_id, o.receipt_version,
       octet_length(o.receipt)::bigint AS receipt_bytes,
       CASE WHEN octet_length(o.receipt) <= $7::bigint THEN o.receipt END AS receipt
FROM df_game.operations AS o
JOIN df_game.checkpoints AS c ON c.tenant_id = o.tenant_id AND c.session_id = o.session_id
  AND c.recovery_epoch = o.committed_epoch AND c.in_epoch_sequence = o.committed_sequence
WHERE o.tenant_id = $1::bytea AND o.session_id = $2::bytea AND o.principal_id = $3::bytea
  AND o.command_namespace = $4 AND o.recovery_epoch = $5::text::numeric
  AND o.operation_id = $6::bytea;"#;

pub(crate) const NAMESPACE_RETIRED: &str = r#"SELECT EXISTS (
    SELECT 1 FROM df_game.retired_namespaces
    WHERE tenant_id = $1::bytea AND session_id = $2::bytea AND principal_id = $3::bytea
      AND command_namespace = $4 AND recovery_epoch = $5::text::numeric
);"#;

pub(crate) const INSERT_CHECKPOINT: &str = r#"INSERT INTO df_game.checkpoints
    (tenant_id, session_id, recovery_epoch, in_epoch_sequence, run_id,
     schema_version, codec_version, complete_envelope)
VALUES ($1::bytea, $2::bytea, $3::text::numeric, $4::text::numeric, $5::bytea, $6, $7, $8::bytea);"#;

pub(crate) const INSERT_FACT: &str = r#"INSERT INTO df_game.facts
    (tenant_id, session_id, committed_epoch, committed_sequence, ordinal, fact_version, fact)
VALUES ($1::bytea, $2::bytea, $3::text::numeric, $4::text::numeric, $5, $6, $7::bytea);"#;

pub(crate) const INSERT_OPERATION: &str = r#"INSERT INTO df_game.operations
    (tenant_id, session_id, principal_id, command_namespace, recovery_epoch,
     operation_id, fingerprint_version, canonical_fingerprint, committed_epoch,
     committed_sequence, receipt_version, receipt)
VALUES ($1::bytea, $2::bytea, $3::bytea, $4::bytea, $5::text::numeric, $6::bytea, $7, $8::bytea, $9::text::numeric,
        $10::text::numeric, $11, $12::bytea);"#;

pub(crate) const INSERT_INTENT: &str = r#"INSERT INTO df_game.intents
    (tenant_id, session_id, principal_id, command_namespace, recovery_epoch,
     operation_id, slot, effect_id, job_id, timer_id, run_id, committed_epoch,
     committed_sequence, process_generation, execution_mode, intent_kind, intent_version, intent)
VALUES ($1::bytea, $2::bytea, $3::bytea, $4::bytea, $5::text::numeric, $6::bytea, $7, $8::bytea, $9::bytea, $10::bytea, $11::bytea,
        $12::text::numeric, $13::text::numeric, $14::text::numeric, $15, $16, $17, $18::bytea);"#;

pub(crate) const FINAL_REVISION_CAS: &str = r#"
UPDATE df_game.sessions
SET in_epoch_sequence = in_epoch_sequence + 1
WHERE tenant_id = $1::bytea AND session_id = $2::bytea AND run_id = $3::bytea AND owner_fence = $4::bytea
  AND recovery_epoch = $5::text::numeric AND in_epoch_sequence = $6::text::numeric
  AND in_epoch_sequence < 18446744073709551615 AND lease_until > clock_timestamp()
RETURNING recovery_epoch::text, in_epoch_sequence::text;"#;

pub(crate) const LOAD_CURRENT: &str = r#"SELECT s.recovery_epoch::text, s.in_epoch_sequence::text, s.run_id::bytea AS run_id,
       c.schema_version, c.codec_version, c.complete_envelope
FROM df_game.sessions AS s
JOIN df_game.checkpoints AS c
  ON c.tenant_id = s.tenant_id AND c.session_id = s.session_id
 AND c.recovery_epoch = s.recovery_epoch AND c.in_epoch_sequence = s.in_epoch_sequence
 AND c.run_id = s.run_id
WHERE s.tenant_id = $1::bytea AND s.session_id = $2::bytea
  AND octet_length(c.complete_envelope) <= $3::bigint;"#;
