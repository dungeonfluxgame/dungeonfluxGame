use crate::ownership::{open_file, safe_path};
use df_observe::{ProducerId, RecordId, Signal, TelemetryError};
use rusqlite::{Connection, OpenFlags, params};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

/// Fixed typed local filters; correlation and build labels confer no authorization.
#[derive(Clone, Debug, Default)]
pub struct QueryFilter {
    pub producer: Option<ProducerId>,
    pub session: Option<String>,
    pub operation: Option<String>,
    pub minimum_severity: Option<i32>,
    pub build: Option<String>,
    pub trace: Option<[u8; 16]>,
    pub from_unix_nanos: Option<u64>,
    pub through_unix_nanos: Option<u64>,
}
#[derive(Debug, Clone)]
pub struct RecordView {
    pub position: u64,
    pub producer: ProducerId,
    pub record: RecordId,
    pub source_sequence: i64,
    pub signal: Signal,
    pub source_time_unix_nanos: u64,
    pub observed_time_unix_nanos: Option<u64>,
    /// A complete single-record standard OTLP request; all supported typed values and
    /// resource/scope/schema/causal fields survive without SQL scalar coercion.
    pub otlp: Vec<u8>,
}
#[derive(Debug)]
pub struct SourceWatermark {
    pub producer: ProducerId,
    pub accepted_records: u64,
    pub highest_sequence: i64,
    pub first_observed_sequence: i64,
    pub missing_through_highest: u64,
}
#[derive(Debug)]
pub struct QueryPage {
    pub records: Vec<RecordView>,
    pub next_cursor: u64,
    pub committed_watermark: u64,
    pub spool_checkpoint: u64,
    pub source_watermarks: Vec<SourceWatermark>,
    pub unconfirmed_capture_gaps: bool,
    pub retention: &'static str,
    pub emergency_source: Option<PathBuf>,
    pub emergency_bytes: Vec<u8>,
}
/// Actual SQLite read-only connection, bounded pages, no arbitrary SQL surface.
pub struct DiagnosticReader {
    connection: Connection,
    root: PathBuf,
}
impl DiagnosticReader {
    pub fn open(root: &Path) -> Result<Self, TelemetryError> {
        safe_path(root)?;
        for name in [
            "OWNER",
            "telemetry.sqlite3",
            "telemetry.sqlite3-wal",
            "telemetry.sqlite3-shm",
            "emergency",
        ] {
            safe_path(&root.join(name))?;
        }
        let mut owner = open_file(&root.join("OWNER"), false, false)?;
        let mut marker = [0; 47];
        let expected = b"df-telemetry native synthetic owned root v1\n";
        if owner.metadata().map_err(|_| TelemetryError::Io)?.len() != expected.len() as u64 {
            return Err(TelemetryError::ForeignRoot);
        }
        owner
            .read_exact(
                marker
                    .get_mut(..expected.len())
                    .ok_or(TelemetryError::Internal)?,
            )
            .map_err(|_| TelemetryError::ForeignRoot)?;
        if marker.get(..expected.len()) != Some(expected.as_slice()) {
            return Err(TelemetryError::ForeignRoot);
        }
        let connection = Connection::open_with_flags(
            root.join("telemetry.sqlite3"),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|_| TelemetryError::SinkUnavailable)?;
        connection
            .busy_timeout(Duration::from_millis(50))
            .map_err(|_| TelemetryError::SinkUnavailable)?;
        connection
            .execute_batch("PRAGMA query_only=ON;")
            .map_err(|_| TelemetryError::SinkUnavailable)?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|_| TelemetryError::Corrupt)?;
        if version != 2 {
            return Err(TelemetryError::ForeignRoot);
        }
        Ok(Self {
            connection,
            root: root.to_owned(),
        })
    }
    /// Exercise the fixed denial probe; no caller-controlled mutation or SQL is accepted.
    pub fn verify_write_denied(&self) -> Result<bool, TelemetryError> {
        match self.connection.execute("DELETE FROM records WHERE 0", []) {
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ReadOnly =>
            {
                Ok(true)
            }
            Err(_) => Err(TelemetryError::SinkUnavailable),
            Ok(_) => Ok(false),
        }
    }
    /// Cursor is an immutable ingestion position, not a timestamp; a new call gets a new
    /// short snapshot. Pagination does not pin a read transaction across user think time.
    pub fn query(
        &mut self,
        filter: &QueryFilter,
        after: u64,
        limit: usize,
    ) -> Result<QueryPage, TelemetryError> {
        if limit == 0
            || limit > 100
            || after > i64::MAX as u64
            || [
                filter.session.as_ref(),
                filter.operation.as_ref(),
                filter.build.as_ref(),
            ]
            .iter()
            .flatten()
            .any(|text| text.len() > 128)
            || filter
                .minimum_severity
                .is_some_and(|severity| !(0..=24).contains(&severity))
            || matches!((filter.from_unix_nanos, filter.through_unix_nanos), (Some(from), Some(through)) if from > through)
        {
            return Err(TelemetryError::InvalidLimits);
        }
        let producer = filter.producer.map(|producer| producer.bytes().to_vec());
        let trace = filter.trace.map(|trace| trace.to_vec());
        let from = filter
            .from_unix_nanos
            .map(|nanos| nanos.to_be_bytes().to_vec());
        let through = filter
            .through_unix_nanos
            .map(|nanos| nanos.to_be_bytes().to_vec());
        let transaction = self
            .connection
            .transaction()
            .map_err(|_| TelemetryError::SinkUnavailable)?;
        let watermark = transaction
            .query_row("SELECT COALESCE(MAX(position),0) FROM records", [], |row| {
                sql_u64(row, 0)
            })
            .map_err(|_| TelemetryError::SinkUnavailable)?;
        let mut statement = transaction.prepare("SELECT position,producer,record,sequence,signal,source_time,observed_time,otlp FROM records WHERE position>?1 AND (?2 IS NULL OR producer=?2) AND (?3 IS NULL OR session=?3) AND (?4 IS NULL OR operation=?4) AND (?5 IS NULL OR severity>=?5) AND (?6 IS NULL OR build=?6) AND (?7 IS NULL OR trace=?7) AND (?8 IS NULL OR source_time>=?8) AND (?9 IS NULL OR source_time<=?9) ORDER BY position LIMIT ?10").map_err(|_| TelemetryError::SinkUnavailable)?;
        let mut rows = statement
            .query(params![
                after as i64,
                producer,
                &filter.session,
                &filter.operation,
                filter.minimum_severity,
                &filter.build,
                trace,
                from,
                through,
                limit as i64
            ])
            .map_err(|_| TelemetryError::SinkUnavailable)?;
        let mut records = Vec::new();
        let mut response_bytes = 32768usize;
        while let Some(row) = rows.next().map_err(|_| TelemetryError::SinkUnavailable)? {
            let get_blob = |index| {
                row.get::<_, Vec<u8>>(index)
                    .map_err(|_| TelemetryError::Corrupt)
            };
            let otlp = get_blob(7)?;
            response_bytes += otlp.len() * 2 + 256;
            if response_bytes > 1048576 {
                break;
            }
            let producer = ProducerId::new(
                get_blob(1)?
                    .try_into()
                    .map_err(|_| TelemetryError::Corrupt)?,
            )?;
            let record = RecordId::new(
                get_blob(2)?
                    .try_into()
                    .map_err(|_| TelemetryError::Corrupt)?,
            )?;
            records.push(RecordView {
                position: sql_u64(row, 0).map_err(|_| TelemetryError::Corrupt)?,
                producer,
                record,
                source_sequence: row.get(3).map_err(|_| TelemetryError::Corrupt)?,
                signal: match row.get::<_, i32>(4).map_err(|_| TelemetryError::Corrupt)? {
                    1 => Signal::Logs,
                    2 => Signal::Spans,
                    _ => return Err(TelemetryError::Corrupt),
                },
                source_time_unix_nanos: u64::from_be_bytes(
                    get_blob(5)?
                        .try_into()
                        .map_err(|_| TelemetryError::Corrupt)?,
                ),
                observed_time_unix_nanos: row
                    .get::<_, Option<Vec<u8>>>(6)
                    .map_err(|_| TelemetryError::Corrupt)?
                    .map(|bytes| {
                        bytes
                            .try_into()
                            .map(u64::from_be_bytes)
                            .map_err(|_| TelemetryError::Corrupt)
                    })
                    .transpose()?,
                otlp,
            });
        }
        drop(rows);
        drop(statement);
        let mut sources = Vec::new();
        for record in &records {
            if sources
                .iter()
                .any(|source: &SourceWatermark| source.producer == record.producer)
            {
                continue;
            }
            let (count, sequence, first) = transaction
                .query_row(
                    "SELECT COUNT(*),MAX(sequence),MIN(sequence) FROM records WHERE producer=?1",
                    [record.producer.bytes().as_slice()],
                    |row| {
                        Ok((
                            sql_u64(row, 0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )
                .map_err(|_| TelemetryError::SinkUnavailable)?;
            sources.push(SourceWatermark {
                producer: record.producer,
                accepted_records: count,
                highest_sequence: sequence,
                first_observed_sequence: first,
                missing_through_highest: u64::try_from(sequence - first + 1)
                    .map_err(|_| TelemetryError::Corrupt)?
                    .saturating_sub(count),
            });
        }
        transaction
            .commit()
            .map_err(|_| TelemetryError::SinkUnavailable)?;
        let emergency_path = self.root.join("emergency");
        let emergency_bytes = if emergency_path.exists() {
            let mut file = open_file(&emergency_path, false, false)?;
            if file.metadata().map_err(|_| TelemetryError::Io)?.len() > 8192 {
                return Err(TelemetryError::Corrupt);
            }
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)
                .map_err(|_| TelemetryError::Io)?;
            bytes
        } else {
            Vec::new()
        };
        let next_cursor = records
            .last()
            .map(|record| record.position)
            .unwrap_or(after);
        Ok(QueryPage {
            records,
            next_cursor,
            committed_watermark: watermark,
            spool_checkpoint: crate::persistence::read_checkpoint(&self.root)?,
            source_watermarks: sources,
            unconfirmed_capture_gaps: true,
            retention: "fixture: all durable spool frames retained; finite quota; no automatic deletion; producer lifetime completeness unknown",
            emergency_source: if emergency_bytes.is_empty() {
                None
            } else {
                Some(emergency_path)
            },
            emergency_bytes,
        })
    }
}

fn sql_u64(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value = row.get::<_, i64>(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}
