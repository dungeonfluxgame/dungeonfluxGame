use crate::{
    ownership::{OwnedRoot, open_file, safe_path, sync_directory},
    wire::{Batch, Record},
};
use df_observe::{Signal, TelemetryError, TelemetryLimits};
use rusqlite::{Connection, OpenFlags, params};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    io::{Read, Write},
    path::Path,
    time::Duration,
};
const HEADER: usize = 49;
const MAX_SPOOL_FILES: usize = 32768;
pub(crate) struct Persistence {
    pub root: OwnedRoot,
    pub connection: Connection,
    pub limits: TelemetryLimits,
    pub backlog: VecDeque<u64>,
    pub next: u64,
    pub spool_bytes: u64,
    pub watermark: u64,
    pub checkpoint: u64,
    identities: HashMap<(df_observe::ProducerId, df_observe::RecordId), (i64, Vec<u8>)>,
    sequences: HashMap<(df_observe::ProducerId, i64), df_observe::RecordId>,
}
fn sink(error: rusqlite::Error) -> TelemetryError {
    match error {
        rusqlite::Error::SqliteFailure(error, _) if error.code == rusqlite::ErrorCode::DiskFull => {
            TelemetryError::Capacity
        }
        _ => TelemetryError::SinkUnavailable,
    }
}
impl Persistence {
    pub fn open(root: OwnedRoot, limits: TelemetryLimits) -> Result<Self, TelemetryError> {
        let db = root.path.join("telemetry.sqlite3");
        safe_path(&db)?;
        for name in [
            "telemetry.sqlite3-wal",
            "telemetry.sqlite3-shm",
            "spool",
            "checkpoint",
            "checkpoint-next",
            "emergency",
        ] {
            safe_path(&root.path.join(name))?;
        }
        let existed = db.exists();
        let connection = Connection::open_with_flags(
            db,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(sink)?;
        if existed {
            let version: i64 = connection
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .map_err(|_| TelemetryError::Corrupt)?;
            if version != 2 {
                return Err(TelemetryError::Corrupt);
            }
        }
        connection
            .busy_timeout(Duration::from_millis(50))
            .map_err(sink)?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA wal_autocheckpoint=16; PRAGMA foreign_keys=ON;").map_err(sink)?;
        if !existed {
            connection.execute_batch("
            CREATE TABLE IF NOT EXISTS records (position INTEGER PRIMARY KEY AUTOINCREMENT, producer BLOB NOT NULL CHECK(length(producer)=16), record BLOB NOT NULL CHECK(length(record)=16), sequence INTEGER NOT NULL CHECK(sequence>0), signal INTEGER NOT NULL, source_time BLOB NOT NULL CHECK(length(source_time)=8), observed_time BLOB CHECK(observed_time IS NULL OR length(observed_time)=8), severity INTEGER NOT NULL, trace BLOB NOT NULL, span BLOB NOT NULL, session TEXT, operation TEXT, build TEXT, digest BLOB NOT NULL, otlp BLOB NOT NULL, UNIQUE(producer,record), UNIQUE(producer,sequence));
            CREATE INDEX IF NOT EXISTS records_trace ON records(trace,position);
            CREATE INDEX IF NOT EXISTS records_session ON records(session,position);
            CREATE INDEX IF NOT EXISTS records_operation ON records(operation,position);
            CREATE INDEX IF NOT EXISTS records_time ON records(source_time,position);
            CREATE INDEX IF NOT EXISTS records_build ON records(build,position);
            PRAGMA user_version=2;").map_err(sink)?;
        } else {
            connection.prepare("SELECT position, producer, record, sequence, signal, source_time, observed_time, severity, trace, span, session, operation, build, digest, otlp FROM records LIMIT 0").map_err(|_| TelemetryError::Corrupt)?;
        }
        connection
            .pragma_update(
                None,
                "max_page_count",
                (limits.sqlite_bytes / 4096 / 2) as i64,
            )
            .map_err(sink)?;
        let watermark = connection
            .query_row("SELECT COALESCE(MAX(position),0) FROM records", [], |row| {
                sql_u64(row, 0)
            })
            .map_err(sink)?;
        let checkpoint = read_checkpoint(&root.path)?;
        let mut positions = Vec::new();
        let mut spool_bytes = 0u64;
        let mut identities = HashMap::new();
        let mut sequences = HashMap::new();
        let mut checkpointed = HashSet::new();
        for entry in fs::read_dir(root.path.join("spool")).map_err(|_| TelemetryError::Io)? {
            let entry = entry.map_err(|_| TelemetryError::Io)?;
            safe_path(&entry.path())?;
            let metadata = entry.metadata().map_err(|_| TelemetryError::Io)?;
            let name = entry.file_name();
            let name = name.to_str().ok_or(TelemetryError::Corrupt)?;
            let position = name
                .strip_suffix(".spool")
                .filter(|value| value.len() == 20)
                .ok_or(TelemetryError::Corrupt)?
                .parse::<u64>()
                .map_err(|_| TelemetryError::Corrupt)?;
            if !metadata.is_file() || position == 0 || positions.len() >= MAX_SPOOL_FILES {
                return Err(TelemetryError::Corrupt);
            }
            spool_bytes = spool_bytes
                .checked_add(metadata.len())
                .ok_or(TelemetryError::Capacity)?;
            if spool_bytes > limits.spool_bytes {
                return Err(TelemetryError::Capacity);
            }
            let batch = read_frame(&entry.path(), limits)?;
            for record in batch.records {
                let pair = (record.key.producer, record.key.record);
                let value = (record.key.sequence.get(), digest(&record));
                if position <= checkpoint {
                    checkpointed.insert(pair);
                }
                if let Some(prior) = identities.insert(pair, value.clone())
                    && prior != value
                {
                    return Err(TelemetryError::Corrupt);
                }
                if let Some(prior) = sequences.insert(
                    (record.key.producer, record.key.sequence.get()),
                    record.key.record,
                ) && prior != record.key.record
                {
                    return Err(TelemetryError::Corrupt);
                }
                if identities.len() > 32768 {
                    return Err(TelemetryError::Capacity);
                }
            }
            positions.push(position);
        }
        // All fixture spool frames are retained. Refuse foreign or inconsistent SQLite
        // records before using the durable identity index instead of per-record SQL lookups.
        {
            let mut statement = connection
                .prepare(
                    "SELECT producer,record,sequence,digest,signal,length(otlp),otlp FROM records",
                )
                .map_err(|_| TelemetryError::Corrupt)?;
            let mut rows = statement.query([]).map_err(|_| TelemetryError::Corrupt)?;
            while let Some(row) = rows.next().map_err(|_| TelemetryError::Corrupt)? {
                let producer: Vec<u8> = row.get(0).map_err(|_| TelemetryError::Corrupt)?;
                let record: Vec<u8> = row.get(1).map_err(|_| TelemetryError::Corrupt)?;
                let producer = df_observe::ProducerId::new(
                    producer.try_into().map_err(|_| TelemetryError::Corrupt)?,
                )
                .map_err(|_| TelemetryError::Corrupt)?;
                let record = df_observe::RecordId::new(
                    record.try_into().map_err(|_| TelemetryError::Corrupt)?,
                )
                .map_err(|_| TelemetryError::Corrupt)?;
                let sequence: i64 = row.get(2).map_err(|_| TelemetryError::Corrupt)?;
                let stored_digest = row
                    .get_ref(3)
                    .map_err(|_| TelemetryError::Corrupt)?
                    .as_blob()
                    .map_err(|_| TelemetryError::Corrupt)?;
                if stored_digest.len() != 32 {
                    return Err(TelemetryError::Corrupt);
                }
                let signal: i64 = row.get(4).map_err(|_| TelemetryError::Corrupt)?;
                let length: i64 = row.get(5).map_err(|_| TelemetryError::Corrupt)?;
                if !matches!(signal, 1 | 2) || length <= 0 || length > limits.record_bytes as i64 {
                    return Err(TelemetryError::Corrupt);
                }
                let bytes: Vec<u8> = row.get(6).map_err(|_| TelemetryError::Corrupt)?;
                let mut content_digest = Sha256::new();
                content_digest.update([signal as u8]);
                content_digest.update(&bytes);
                if content_digest.finalize().as_slice() != stored_digest
                    || identities.get(&(producer, record))
                        != Some(&(sequence, stored_digest.to_vec()))
                {
                    return Err(TelemetryError::Corrupt);
                }
                checkpointed.remove(&(producer, record));
            }
        }
        // A numeric checkpoint is only a claim. Every identity/content it covers must
        // exist in the committed corpus, including when its value is in spool range.
        if !checkpointed.is_empty() {
            return Err(TelemetryError::Corrupt);
        }
        positions.sort_unstable();
        for (index, position) in positions.iter().enumerate() {
            if *position != index as u64 + 1 {
                return Err(TelemetryError::Corrupt);
            }
        }
        let last = positions.last().copied().unwrap_or(0);
        if checkpoint > last {
            return Err(TelemetryError::Corrupt);
        }
        let backlog = positions
            .into_iter()
            .filter(|position| *position > checkpoint)
            .collect();
        Ok(Self {
            root,
            connection,
            limits,
            backlog,
            next: last.checked_add(1).ok_or(TelemetryError::Capacity)?,
            spool_bytes,
            watermark,
            checkpoint,
            identities,
            sequences,
        })
    }
    pub fn spool(&mut self, batch: &Batch) -> Result<u64, TelemetryError> {
        self.check_conflicts(&batch.records)?;
        if self.identities.len().saturating_add(batch.records.len()) > 32768 {
            return Err(TelemetryError::Capacity);
        }
        let frame_bytes = (HEADER + batch.bytes.len()) as u64;
        if self.spool_bytes.saturating_add(frame_bytes) > self.limits.spool_bytes
            || self.next > MAX_SPOOL_FILES as u64
        {
            return Err(TelemetryError::Capacity);
        }
        let position = self.next;
        let path = self.frame_path(position);
        let mut file = open_file(&path, true, true)?;
        let mut frame = Vec::with_capacity(frame_bytes as usize);
        frame.extend_from_slice(b"DFOTLP01");
        frame.push(if batch.signal == Signal::Logs { 1 } else { 2 });
        frame.extend_from_slice(&(batch.bytes.len() as u64).to_be_bytes());
        frame.extend_from_slice(&Sha256::digest(&batch.bytes));
        frame.extend_from_slice(&batch.bytes);
        file.write_all(&frame).map_err(|_| TelemetryError::Io)?;
        file.sync_all().map_err(|_| TelemetryError::Io)?;
        sync_directory(&self.root.path.join("spool"))?;
        for record in &batch.records {
            self.identities.insert(
                (record.key.producer, record.key.record),
                (record.key.sequence.get(), digest(record)),
            );
            self.sequences.insert(
                (record.key.producer, record.key.sequence.get()),
                record.key.record,
            );
        }
        self.spool_bytes += frame_bytes;
        self.next += 1;
        self.backlog.push_back(position);
        Ok(position)
    }
    pub fn frame_path(&self, position: u64) -> std::path::PathBuf {
        self.root
            .path
            .join("spool")
            .join(format!("{position:020}.spool"))
    }
    pub fn check_conflicts(&self, records: &[Record]) -> Result<(), TelemetryError> {
        for record in records {
            let digest = digest(record);
            for prior in records {
                if record.key.producer == prior.key.producer
                    && (record.key.record == prior.key.record
                        || record.key.sequence == prior.key.sequence)
                    && (record.key != prior.key || digest != crate::persistence::digest(prior))
                {
                    return Err(TelemetryError::Conflict);
                }
            }
            // The bounded durable index includes records still waiting for SQLite.
            if let Some((sequence, prior_digest)) = self
                .identities
                .get(&(record.key.producer, record.key.record))
                && (*sequence != record.key.sequence.get() || *prior_digest != digest)
            {
                return Err(TelemetryError::Conflict);
            }
            if let Some(prior_record) = self
                .sequences
                .get(&(record.key.producer, record.key.sequence.get()))
                && *prior_record != record.key.record
            {
                return Err(TelemetryError::Conflict);
            }
        }
        Ok(())
    }
    /// One transaction owns dedupe and ingestion positions. Caller checkpoints separately.
    pub fn commit_front(&mut self) -> Result<(u64, usize, usize), TelemetryError> {
        let position = *self.backlog.front().ok_or(TelemetryError::Internal)?;
        let batch = read_frame(&self.frame_path(position), self.limits)?;
        self.check_conflicts(&batch.records)?;
        let sqlite_physical_bytes = [
            "telemetry.sqlite3",
            "telemetry.sqlite3-wal",
            "telemetry.sqlite3-shm",
        ]
        .iter()
        .try_fold(0u64, |bytes, name| {
            match fs::metadata(self.root.path.join(name)) {
                Ok(metadata) => Ok(bytes.saturating_add(metadata.len())),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(bytes),
                Err(_) => Err(TelemetryError::Io),
            }
        })?;
        let sqlite_growth_reserve_bytes = (self.limits.sqlite_bytes / 2).min(4194304);
        if sqlite_physical_bytes
            .saturating_add(sqlite_growth_reserve_bytes)
            .saturating_add(batch.bytes.len() as u64)
            > self.limits.sqlite_bytes
        {
            return Err(TelemetryError::Capacity);
        }
        let transaction = self.connection.transaction().map_err(sink)?;
        let mut inserted = 0;
        {
            let mut statement = transaction.prepare("INSERT OR IGNORE INTO records (producer,record,sequence,signal,source_time,observed_time,severity,trace,span,session,operation,build,digest,otlp) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)").map_err(sink)?;
            for record in &batch.records {
                inserted += statement
                    .execute(params![
                        record.key.producer.bytes().as_slice(),
                        record.key.record.bytes().as_slice(),
                        record.key.sequence.get(),
                        if record.signal == Signal::Logs { 1 } else { 2 },
                        record.timestamp.to_be_bytes().as_slice(),
                        record.observed.map(|value| value.to_be_bytes().to_vec()),
                        record.severity,
                        &record.trace,
                        &record.span,
                        &record.session,
                        &record.operation,
                        &record.build,
                        digest(record).as_slice(),
                        &record.bytes,
                    ])
                    .map_err(sink)?;
            }
        }
        transaction.commit().map_err(sink)?;
        self.watermark = self
            .connection
            .query_row("SELECT COALESCE(MAX(position),0) FROM records", [], |row| {
                sql_u64(row, 0)
            })
            .map_err(sink)?;
        Ok((position, inserted, batch.records.len() - inserted))
    }
    pub fn checkpoint(&mut self, position: u64) -> Result<(), TelemetryError> {
        let path = self.root.path.join("checkpoint-next");
        let mut file = if path.exists() {
            open_file(&path, true, false)?
        } else {
            open_file(&path, true, true)?
        };
        file.set_len(0).map_err(|_| TelemetryError::Io)?;
        file.write_all(&position.to_be_bytes())
            .map_err(|_| TelemetryError::Io)?;
        file.sync_all().map_err(|_| TelemetryError::Io)?;
        fs::rename(path, self.root.path.join("checkpoint")).map_err(|_| TelemetryError::Io)?;
        sync_directory(&self.root.path)?;
        self.checkpoint = position;
        self.backlog.pop_front();
        Ok(())
    }
}
pub(crate) fn read_checkpoint(root: &Path) -> Result<u64, TelemetryError> {
    let path = root.join("checkpoint");
    if !path.exists() {
        return Ok(0);
    }
    let mut file = open_file(&path, false, false)?;
    if file.metadata().map_err(|_| TelemetryError::Io)?.len() != 8 {
        return Err(TelemetryError::Corrupt);
    }
    let mut bytes = [0; 8];
    file.read_exact(&mut bytes)
        .map_err(|_| TelemetryError::Corrupt)?;
    Ok(u64::from_be_bytes(bytes))
}
fn digest(record: &Record) -> Vec<u8> {
    let mut digest = Sha256::new();
    digest.update([if record.signal == Signal::Logs { 1 } else { 2 }]);
    digest.update(&record.bytes);
    digest.finalize().to_vec()
}
pub(crate) fn read_frame(path: &Path, limits: TelemetryLimits) -> Result<Batch, TelemetryError> {
    let mut file = open_file(path, false, false)?;
    let size = file.metadata().map_err(|_| TelemetryError::Io)?.len();
    if size < HEADER as u64 || size > (HEADER + limits.batch_bytes) as u64 {
        return Err(TelemetryError::Corrupt);
    }
    let mut header = [0; HEADER];
    file.read_exact(&mut header)
        .map_err(|_| TelemetryError::Corrupt)?;
    if header.get(..8) != Some(b"DFOTLP01") {
        return Err(TelemetryError::Corrupt);
    }
    let signal = match header.get(8) {
        Some(1) => Signal::Logs,
        Some(2) => Signal::Spans,
        _ => return Err(TelemetryError::Corrupt),
    };
    let length = u64::from_be_bytes(
        header
            .get(9..17)
            .ok_or(TelemetryError::Corrupt)?
            .try_into()
            .map_err(|_| TelemetryError::Corrupt)?,
    );
    if length > limits.batch_bytes as u64 || length + HEADER as u64 != size {
        return Err(TelemetryError::Corrupt);
    }
    let mut bytes = vec![0; length as usize];
    file.read_exact(&mut bytes)
        .map_err(|_| TelemetryError::Corrupt)?;
    if Sha256::digest(&bytes).as_slice() != header.get(17..49).ok_or(TelemetryError::Corrupt)? {
        return Err(TelemetryError::Corrupt);
    }
    Batch::decode(signal, &bytes, limits).map_err(|_| TelemetryError::Corrupt)
}

fn sql_u64(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value = row.get::<_, i64>(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}
