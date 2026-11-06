use crate::{
    limits::{
        MAXIMUM_ROOT_RECORDS, MAXIMUM_SPOOL_FILES, MAXIMUM_SQLITE_GROWTH_RESERVE_BYTES,
        SQLITE_BUSY_TIMEOUT,
    },
    ownership::{OwnedRoot, open_file, open_or_create_file, safe_path, sync_directory},
    wire::{Batch, Record, signal_code, signal_from_code},
};
use df_observe::{ProducerId, RecordId, Signal, TelemetryError, TelemetryLimits};
use rusqlite::{Connection, OpenFlags, params};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    io::{Read, Write},
    path::Path,
};
/// Spool frame magic; the frame is magic, signal code, big-endian length, SHA-256, payload.
const FRAME_MAGIC: &[u8; 8] = b"DFOTLP01";
const FRAME_HEADER_BYTES: usize = FRAME_MAGIC.len() + 1 + 8 + 32;
/// SQLite's default page size, used to turn `sqlite_bytes` into `max_page_count`.
const SQLITE_PAGE_BYTES: u64 = 4096;
/// Schema version stamped in `user_version`; any other value is refused as corrupt.
pub(crate) const SCHEMA_VERSION: i64 = 2;
const CREATE_SCHEMA: &str = concat!(
    "CREATE TABLE IF NOT EXISTS records (position INTEGER PRIMARY KEY AUTOINCREMENT, ",
    "producer BLOB NOT NULL CHECK(length(producer)=16), ",
    "record BLOB NOT NULL CHECK(length(record)=16), ",
    "sequence INTEGER NOT NULL CHECK(sequence>0), signal INTEGER NOT NULL, ",
    "source_time BLOB NOT NULL CHECK(length(source_time)=8), ",
    "observed_time BLOB CHECK(observed_time IS NULL OR length(observed_time)=8), ",
    "severity INTEGER NOT NULL, trace BLOB NOT NULL, span BLOB NOT NULL, session TEXT, ",
    "operation TEXT, build TEXT, digest BLOB NOT NULL, otlp BLOB NOT NULL, ",
    "UNIQUE(producer,record), UNIQUE(producer,sequence)); ",
    "CREATE INDEX IF NOT EXISTS records_trace ON records(trace,position); ",
    "CREATE INDEX IF NOT EXISTS records_session ON records(session,position); ",
    "CREATE INDEX IF NOT EXISTS records_operation ON records(operation,position); ",
    "CREATE INDEX IF NOT EXISTS records_time ON records(source_time,position); ",
    "CREATE INDEX IF NOT EXISTS records_build ON records(build,position);",
);
const CHECK_SCHEMA: &str = concat!(
    "SELECT position, producer, record, sequence, signal, source_time, observed_time, ",
    "severity, trace, span, session, operation, build, digest, otlp FROM records LIMIT 0",
);
const INSERT_RECORD: &str = concat!(
    "INSERT OR IGNORE INTO records (producer,record,sequence,signal,source_time,",
    "observed_time,severity,trace,span,session,operation,build,digest,otlp) ",
    "VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
);
/// Retry identity `(producer, record)` mapped to its source sequence and content digest.
type IdentityIndex = HashMap<(ProducerId, RecordId), (i64, Vec<u8>)>;
/// Ordering identity `(producer, sequence)` mapped to the record that owns it.
type SequenceIndex = HashMap<(ProducerId, i64), RecordId>;
pub(crate) struct Persistence {
    pub root: OwnedRoot,
    pub connection: Connection,
    pub limits: TelemetryLimits,
    pub backlog: VecDeque<u64>,
    pub next: u64,
    pub spool_bytes: u64,
    pub watermark: u64,
    pub checkpoint: u64,
    identities: IdentityIndex,
    sequences: SequenceIndex,
}
/// What one pass over the spool directory found at open.
struct SpoolScan {
    positions: Vec<u64>,
    spool_bytes: u64,
    identities: IdentityIndex,
    sequences: SequenceIndex,
    /// Identities in frames the checkpoint claims are committed; each must exist in SQLite.
    checkpointed: HashSet<(ProducerId, RecordId)>,
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
        let connection = open_writer(&db, limits)?;
        let watermark = committed_watermark(&connection)?;
        let checkpoint = read_checkpoint(&root.path)?;
        let mut scan = scan_spool(&root.path.join("spool"), limits, checkpoint)?;
        verify_committed_rows(
            &connection,
            limits,
            &scan.identities,
            &mut scan.checkpointed,
        )?;
        // A numeric checkpoint is only a claim. Every identity/content it covers must
        // exist in the committed corpus, including when its value is in spool range.
        if !scan.checkpointed.is_empty() {
            return Err(TelemetryError::Corrupt);
        }
        let mut positions = scan.positions;
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
            spool_bytes: scan.spool_bytes,
            watermark,
            checkpoint,
            identities: scan.identities,
            sequences: scan.sequences,
        })
    }
    pub fn spool(&mut self, batch: &Batch) -> Result<u64, TelemetryError> {
        self.check_conflicts(&batch.records)?;
        if self.identities.len().saturating_add(batch.records.len()) > MAXIMUM_ROOT_RECORDS {
            return Err(TelemetryError::Capacity);
        }
        let frame_bytes = (FRAME_HEADER_BYTES + batch.bytes.len()) as u64;
        if self.spool_bytes.saturating_add(frame_bytes) > self.limits.spool_bytes
            || self.next > MAXIMUM_SPOOL_FILES as u64
        {
            return Err(TelemetryError::Capacity);
        }
        let position = self.next;
        let path = self.frame_path(position);
        let mut file = open_file(&path, true, true)?;
        let mut frame = Vec::with_capacity(FRAME_HEADER_BYTES + batch.bytes.len());
        frame.extend_from_slice(FRAME_MAGIC);
        frame.push(signal_code(batch.signal));
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
        let digests: Vec<Vec<u8>> = records.iter().map(digest).collect();
        for (record, digest) in records.iter().zip(&digests) {
            for (prior, prior_digest) in records.iter().zip(&digests) {
                if record.key.producer == prior.key.producer
                    && (record.key.record == prior.key.record
                        || record.key.sequence == prior.key.sequence)
                    && (record.key != prior.key || digest != prior_digest)
                {
                    return Err(TelemetryError::Conflict);
                }
            }
            // The bounded durable index includes records still waiting for SQLite.
            if let Some((sequence, prior_digest)) = self
                .identities
                .get(&(record.key.producer, record.key.record))
                && (*sequence != record.key.sequence.get() || prior_digest != digest)
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
        let sqlite_growth_reserve_bytes =
            (self.limits.sqlite_bytes / 2).min(MAXIMUM_SQLITE_GROWTH_RESERVE_BYTES);
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
            let mut statement = transaction.prepare(INSERT_RECORD).map_err(sink)?;
            for record in &batch.records {
                inserted += statement
                    .execute(params![
                        record.key.producer.bytes().as_slice(),
                        record.key.record.bytes().as_slice(),
                        record.key.sequence.get(),
                        signal_code(record.signal),
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
        self.watermark = committed_watermark(&self.connection)?;
        Ok((position, inserted, batch.records.len() - inserted))
    }
    pub fn checkpoint(&mut self, position: u64) -> Result<(), TelemetryError> {
        let path = self.root.path.join("checkpoint-next");
        let mut file = open_or_create_file(&path)?;
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
/// Open (creating when absent) the single-writer SQLite corpus and check its schema.
fn open_writer(db: &Path, limits: TelemetryLimits) -> Result<Connection, TelemetryError> {
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
        if version != SCHEMA_VERSION {
            return Err(TelemetryError::Corrupt);
        }
    }
    connection.busy_timeout(SQLITE_BUSY_TIMEOUT).map_err(sink)?;
    connection
        .execute_batch(concat!(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; ",
            "PRAGMA wal_autocheckpoint=16; PRAGMA foreign_keys=ON;",
        ))
        .map_err(sink)?;
    if existed {
        connection
            .prepare(CHECK_SCHEMA)
            .map_err(|_| TelemetryError::Corrupt)?;
    } else {
        connection.execute_batch(CREATE_SCHEMA).map_err(sink)?;
        connection
            .pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(sink)?;
    }
    // Half the byte budget in pages; the rest is headroom for the WAL and shared memory.
    let maximum_pages = i64::try_from(limits.sqlite_bytes / SQLITE_PAGE_BYTES / 2)
        .map_err(|_| TelemetryError::InvalidLimits)?;
    connection
        .pragma_update(None, "max_page_count", maximum_pages)
        .map_err(sink)?;
    Ok(connection)
}
fn committed_watermark(connection: &Connection) -> Result<u64, TelemetryError> {
    connection
        .query_row("SELECT COALESCE(MAX(position),0) FROM records", [], |row| {
            sql_u64(row, 0)
        })
        .map_err(sink)
}
/// Read every retained spool frame, rebuilding the identity indexes and refusing any frame,
/// name or identity reuse that is inconsistent.
fn scan_spool(
    spool: &Path,
    limits: TelemetryLimits,
    checkpoint: u64,
) -> Result<SpoolScan, TelemetryError> {
    let mut scan = SpoolScan {
        positions: Vec::new(),
        spool_bytes: 0,
        identities: HashMap::new(),
        sequences: HashMap::new(),
        checkpointed: HashSet::new(),
    };
    for entry in fs::read_dir(spool).map_err(|_| TelemetryError::Io)? {
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
        if !metadata.is_file() || position == 0 || scan.positions.len() >= MAXIMUM_SPOOL_FILES {
            return Err(TelemetryError::Corrupt);
        }
        scan.spool_bytes = scan
            .spool_bytes
            .checked_add(metadata.len())
            .ok_or(TelemetryError::Capacity)?;
        if scan.spool_bytes > limits.spool_bytes {
            return Err(TelemetryError::Capacity);
        }
        let batch = read_frame(&entry.path(), limits)?;
        for record in batch.records {
            let pair = (record.key.producer, record.key.record);
            let value = (record.key.sequence.get(), digest(&record));
            if position <= checkpoint {
                scan.checkpointed.insert(pair);
            }
            if let Some(prior) = scan.identities.insert(pair, value.clone())
                && prior != value
            {
                return Err(TelemetryError::Corrupt);
            }
            if let Some(prior) = scan.sequences.insert(
                (record.key.producer, record.key.sequence.get()),
                record.key.record,
            ) && prior != record.key.record
            {
                return Err(TelemetryError::Corrupt);
            }
            if scan.identities.len() > MAXIMUM_ROOT_RECORDS {
                return Err(TelemetryError::Capacity);
            }
        }
        scan.positions.push(position);
    }
    Ok(scan)
}
/// All fixture spool frames are retained. Refuse foreign or inconsistent SQLite records before
/// using the durable identity index instead of per-record SQL lookups, and strike each
/// committed identity from `checkpointed`.
fn verify_committed_rows(
    connection: &Connection,
    limits: TelemetryLimits,
    identities: &IdentityIndex,
    checkpointed: &mut HashSet<(ProducerId, RecordId)>,
) -> Result<(), TelemetryError> {
    let corrupt = |_| TelemetryError::Corrupt;
    let maximum_record_bytes = i64::try_from(limits.record_bytes).unwrap_or(i64::MAX);
    let mut statement = connection
        .prepare("SELECT producer,record,sequence,digest,signal,length(otlp),otlp FROM records")
        .map_err(corrupt)?;
    let mut rows = statement.query([]).map_err(corrupt)?;
    while let Some(row) = rows.next().map_err(corrupt)? {
        let producer: Vec<u8> = row.get(0).map_err(corrupt)?;
        let record: Vec<u8> = row.get(1).map_err(corrupt)?;
        let producer = ProducerId::new(producer.try_into().map_err(|_| TelemetryError::Corrupt)?)
            .map_err(|_| TelemetryError::Corrupt)?;
        let record = RecordId::new(record.try_into().map_err(|_| TelemetryError::Corrupt)?)
            .map_err(|_| TelemetryError::Corrupt)?;
        let sequence: i64 = row.get(2).map_err(corrupt)?;
        let stored_digest = row
            .get_ref(3)
            .map_err(corrupt)?
            .as_blob()
            .map_err(|_| TelemetryError::Corrupt)?;
        if stored_digest.len() != 32 {
            return Err(TelemetryError::Corrupt);
        }
        let signal: i64 = row.get(4).map_err(corrupt)?;
        let signal = u8::try_from(signal)
            .ok()
            .and_then(signal_from_code)
            .ok_or(TelemetryError::Corrupt)?;
        let length: i64 = row.get(5).map_err(corrupt)?;
        if length <= 0 || length > maximum_record_bytes {
            return Err(TelemetryError::Corrupt);
        }
        let bytes: Vec<u8> = row.get(6).map_err(corrupt)?;
        if content_digest(signal, &bytes) != stored_digest
            || identities.get(&(producer, record)) != Some(&(sequence, stored_digest.to_vec()))
        {
            return Err(TelemetryError::Corrupt);
        }
        checkpointed.remove(&(producer, record));
    }
    Ok(())
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
    content_digest(record.signal, &record.bytes)
}
/// Signal-qualified SHA-256 of one single-record OTLP request, as stored in `digest`.
fn content_digest(signal: Signal, bytes: &[u8]) -> Vec<u8> {
    let mut digest = Sha256::new();
    digest.update([signal_code(signal)]);
    digest.update(bytes);
    digest.finalize().to_vec()
}
pub(crate) fn read_frame(path: &Path, limits: TelemetryLimits) -> Result<Batch, TelemetryError> {
    let mut file = open_file(path, false, false)?;
    let size = file.metadata().map_err(|_| TelemetryError::Io)?.len();
    let header_bytes = FRAME_HEADER_BYTES as u64;
    if size < header_bytes || size > header_bytes + limits.batch_bytes as u64 {
        return Err(TelemetryError::Corrupt);
    }
    let mut header = [0; FRAME_HEADER_BYTES];
    file.read_exact(&mut header)
        .map_err(|_| TelemetryError::Corrupt)?;
    let (magic, rest) = header.split_at(FRAME_MAGIC.len());
    let (signal, rest) = rest.split_at(1);
    let (length, payload_digest) = rest.split_at(8);
    if magic != FRAME_MAGIC {
        return Err(TelemetryError::Corrupt);
    }
    let signal = signal
        .first()
        .copied()
        .and_then(signal_from_code)
        .ok_or(TelemetryError::Corrupt)?;
    let length = u64::from_be_bytes(length.try_into().map_err(|_| TelemetryError::Corrupt)?);
    if length > limits.batch_bytes as u64 || length + header_bytes != size {
        return Err(TelemetryError::Corrupt);
    }
    let mut bytes = vec![0; usize::try_from(length).map_err(|_| TelemetryError::Corrupt)?];
    file.read_exact(&mut bytes)
        .map_err(|_| TelemetryError::Corrupt)?;
    if Sha256::digest(&bytes).as_slice() != payload_digest {
        return Err(TelemetryError::Corrupt);
    }
    Batch::decode(signal, &bytes, limits).map_err(|_| TelemetryError::Corrupt)
}

pub(crate) fn sql_u64(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value = row.get::<_, i64>(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}
