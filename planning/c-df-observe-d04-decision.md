# D04: Browser buffer and bounded export lifetime

Date: 2026-10-02
Status: Source-backed decision; browser buffer/upload implementation remains pending
Repair input: `95d7c0cad8af34b60c9062b002fd5c2790eb474c`
Task/attempt: `B-C-df-observe-D04` / `B-C-df-observe-D04-a3`

## Decision

The browser buffer is owned by one live client binding generation. It accepts only
already allowlisted, typed OTEL single-record bytes from the D02 catalog. It is
best-effort memory owned by that page/binding; it does not claim browser crash
persistence. The buffer retains records until a terminal upload receipt says they
were durably accepted. A send, half-close, enqueued/pending receipt, or timeout alone never
releases them. Existing native `Durability::Spooled` and `Committed` are explicit
durable stages; the generated browser adapter must prove which terminal stage
identifies this exact batch before calling the release seam. Retries reuse the same record identities and bytes.

Use at most 128 records / 256 KiB of retained encoded payload, including the
one in-flight prefix, and a single upload lease of at most 32 records / 64 KiB.

> Amended 2026-11-06 (owner: limits stay out of the way of dev work): the page-side retained buffer
> is 100,000 records / 64 MiB in `df-observe`'s `limits.rs`. The upload lease (32 records / 64 KiB) and the
> 8192-byte single-record request are unchanged.

Each encoded single-record request must be nonempty and at most 8192 bytes,
matching `TelemetryLimits::default().record_bytes` and the native decoder.
The 256 KiB limit is a payload bound, not a total process-memory claim. Normalize
accepted bytes to boxed slices with no spare capacity. Fixed 128 inline slots
independently bound metadata: retained requested storage is at most 256 KiB plus
`size_of::<BrowserBuffer>()` and one fixed `Rc<u8>` owner allocation (one marker
byte plus the platform reference-count header). Lease handles share this allocation;
cloning a handle never copies payload. Old callback tokens retain their original
owner allocation until dropped; callbacks and token counts must be bounded by
their I03 lifecycle owner separately. Allocator
bookkeeping, caller-owned input and transient serialization/upload allocations
are separate; I03 must bound those before claiming a whole-path memory ceiling.
The literal never copies an in-flight batch or allocates queue metadata. A record that cannot fit, or a new record arriving
at capacity, is refused without evicting older records; increment a saturating refusal
count and expose it as a gap (maximum means at least that many refusals). Do not retain refused field names or values. The
buffer owns encoded record bytes, not generic event bodies or unfiltered
attributes.

At most one upload is active per binding generation. Its owner is the binding's
buffer/export state, with a finite 10-second attempt deadline. On deadline, close,
or transport failure, preserve the batch in that state and report the classified
outcome; do not infer that the server rejected it or cancel server-owned work. A
terminal durability receipt releases only records it identifies as accepted. An
explicit partial/rejected receipt remains visible and leaves unaccepted records
owned for a policy-correct retry or operator-visible stop. A callback may change
state only when its private owner marker, captured generation, checked attempt
identity, and retained prefix identity all match the current owner. Every constructor
allocates a fresh `Rc` marker; lease tokens retain that allocation, so a queued old
callback prevents address reuse even after its buffer is destroyed. Equality uses
`Rc::ptr_eq`, not marker value or a caller-supplied unique generation. Moving the
buffer preserves its owner; reconstruction creates a distinct owner even with the
same generation, counters and clock. Marker/token fields are private and are never
serialized or reconstructed from RPC values. Attempt exhaustion refuses dispatch;
rebinding within one owner never resets its counters. Record ordinals in the literal are private
queue ownership tokens, not replacements for canonical capture identities. The
D02 producer/record/sequence identities stay embedded in unchanged bytes. Rebinding invalidates the old lease; page destruction may lose the
best-effort in-memory remainder, which is reported as a gap when the lifecycle
allows reporting.

This is an internal ownership seam, not a new public Rust API, wire envelope,
OTEL SDK, or uploader implementation. The future `df-api` adapter authorizes the
client independently and connects the approved OTLP records to
`TelemetryIngress`; `df-telemetry` remains the sole native spool/SQLite owner.
The exact generated `UploadTelemetry` request/receipt types and browser transport
feasibility are not present in this source revision, so their mapping remains a
consumer implementation gate. Operation/trace context is provenance only.

## Source fit

- D01 keeps correlation separate from authorization and requires owned work to
  retain captured context across request-span completion. The page binding owns
  only its best-effort browser buffer; server-accepted work has its own lifetime.
- D02 is the browser capture allowlist: preserve standard typed OTEL records,
  reserved producer/record/sequence identity, and reject unlisted/private data
  before encoding. Its native limits do not establish browser memory bounds.
- D03 assigns serialization and native ingress to the caller-side adapter. Pure
  results remain facts; the browser reporter does not install a second logger,
  queue owner, or durable store.
- `crates/df-observe/src/ingress.rs` defines 32 records/64 KiB native batches,
  stable capture identities, and a `PendingReceipt` whose timeout cancels only
  the wait. `crates/df-observe/src/producer.rs` is native-only. Neither file
  supplies a browser queue or browser upload receipt type.
- `planning/rpc-api.md` specifies bounded client-streaming uploads, terminal
  durability receipts, and retry by stable record IDs. It gives no numeric
  browser buffer or RPC deadline. The 10-second attempt deadline and browser
  retention bounds above are this decision's local buffer policy, not a claim
  about server processing time or durability.

## Bounded contract example

The literal below models only the page-owned retained buffer and owner/generation/lease
checks. `Record` stands for one already D02-filtered encoded OTEL record, preserving
canonical capture identity. The local release seam models full-prefix acceptance;
partial/rejected or unmatched receipts must never call it and remain retained
with a visible classified outcome pending the concrete adapter mapping. It does
not define or simulate an RPC receipt or an implemented browser uploader.

```rust
use std::rc::Rc;

const MAX_ITEMS: usize = 128;
const MAX_BYTES: usize = 256 * 1024;
const MAX_RECORD_BYTES: usize = 8192;
const MAX_BATCH_ITEMS: usize = 32;
const MAX_BATCH_BYTES: usize = 64 * 1024;
const UPLOAD_DEADLINE_MS: u64 = 10_000;

struct Record(Vec<u8>);

struct RetainedRecord {
    ordinal: u64,
    bytes: Box<[u8]>,
}

#[derive(Clone, Debug)]
struct OwnerId(Rc<u8>);

impl PartialEq for OwnerId {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for OwnerId {}

#[derive(Clone, Debug, PartialEq, Eq)]
struct UploadLease {
    owner: OwnerId,
    generation: u64,
    attempt: u64,
    first_record: u64,
    records: usize,
    deadline_ms: u64,
}

#[derive(Debug, PartialEq, Eq)]
enum BufferError {
    Empty,
    Oversized,
    Capacity,
    IdentityExhausted,
    StaleLease,
    Deadline,
    NotExpired,
}

struct BrowserBuffer {
    owner: OwnerId,
    generation: u64,
    slots: [Option<RetainedRecord>; MAX_ITEMS],
    head: usize,
    count: usize,
    bytes: usize,
    refused: usize,
    next_record: u64,
    next_attempt: u64,
    lease: Option<UploadLease>,
}

impl BrowserBuffer {
    fn new(generation: u64) -> Self {
        Self {
            owner: OwnerId(Rc::new(0)),
            generation,
            slots: std::array::from_fn(|_| None),
            head: 0,
            count: 0,
            bytes: 0,
            refused: 0,
            next_record: 1,
            next_attempt: 1,
            lease: None,
        }
    }

    fn retain(&mut self, record: Record) -> Result<(), BufferError> {
        let length = record.0.len();
        let error = if length == 0 {
            Some(BufferError::Empty)
        } else if length > MAX_RECORD_BYTES {
            Some(BufferError::Oversized)
        } else if self.count == MAX_ITEMS
            || self
                .bytes
                .checked_add(length)
                .is_none_or(|total| total > MAX_BYTES)
        {
            Some(BufferError::Capacity)
        } else if self.next_record == u64::MAX {
            Some(BufferError::IdentityExhausted)
        } else {
            None
        };
        if let Some(error) = error {
            self.refused = self.refused.saturating_add(1);
            return Err(error);
        }
        let index = (self.head + self.count) % MAX_ITEMS;
        self.slots[index] = Some(RetainedRecord {
            ordinal: self.next_record,
            bytes: record.0.into_boxed_slice(),
        });
        self.next_record += 1;
        self.count += 1;
        self.bytes += length;
        Ok(())
    }

    fn begin_upload(&mut self, now_ms: u64) -> Result<UploadLease, BufferError> {
        if self.lease.is_some() {
            return Err(BufferError::Capacity);
        }
        if self.next_attempt == u64::MAX {
            return Err(BufferError::IdentityExhausted);
        }
        let first_record = self.slots[self.head]
            .as_ref()
            .ok_or(BufferError::Empty)?
            .ordinal;
        let mut bytes = 0;
        let mut records = 0;
        for offset in 0..self.count {
            let record = self.slots[(self.head + offset) % MAX_ITEMS]
                .as_ref()
                .ok_or(BufferError::Empty)?;
            if records == MAX_BATCH_ITEMS || bytes + record.bytes.len() > MAX_BATCH_BYTES {
                break;
            }
            bytes += record.bytes.len();
            records += 1;
        }
        let lease = UploadLease {
            owner: self.owner.clone(),
            generation: self.generation,
            attempt: self.next_attempt,
            first_record,
            records,
            deadline_ms: now_ms.saturating_add(UPLOAD_DEADLINE_MS),
        };
        self.next_attempt += 1;
        self.lease = Some(lease.clone());
        Ok(lease)
    }

    // Timeout ends only this attempt; record identities and bytes remain owned.
    fn expire_upload(&mut self, now_ms: u64) -> Result<(), BufferError> {
        let lease = self.lease.as_ref().ok_or(BufferError::Empty)?;
        if now_ms < lease.deadline_ms {
            return Err(BufferError::NotExpired);
        }
        self.lease = None;
        Ok(())
    }

    // Keep counters monotonic even if a caller mistakenly reuses a generation.
    fn invalidate(&mut self, generation: u64) -> usize {
        let lost = self.count;
        self.slots = std::array::from_fn(|_| None);
        self.head = 0;
        self.count = 0;
        self.bytes = 0;
        self.lease = None;
        self.generation = generation;
        lost
    }

    // This local seam is called only after a matched, full terminal durable receipt.
    fn acknowledge_durable(&mut self, lease: &UploadLease, now_ms: u64) -> Result<(), BufferError> {
        if lease.owner != self.owner
            || lease.generation != self.generation
            || self.lease.as_ref() != Some(lease)
            || self.slots[self.head].as_ref().map(|record| record.ordinal)
                != Some(lease.first_record)
        {
            return Err(BufferError::StaleLease);
        }
        if now_ms >= lease.deadline_ms {
            return Err(BufferError::Deadline);
        }
        for _ in 0..lease.records {
            if let Some(record) = self.slots[self.head].take() {
                self.bytes -= record.bytes.len();
                self.count -= 1;
                self.head = (self.head + 1) % MAX_ITEMS;
            }
        }
        self.lease = None;
        Ok(())
    }
}

fn main() {
    let mut buffer = BrowserBuffer::new(7);
    buffer.retain(Record(vec![1, 2, 3])).unwrap();
    let lease = buffer.begin_upload(1_000).unwrap();
    assert_eq!(lease.deadline_ms, 11_000);
    assert_eq!(buffer.bytes, 3);
    assert_eq!(buffer.expire_upload(10_999), Err(BufferError::NotExpired));
    assert_eq!(
        buffer.acknowledge_durable(&lease, 11_000),
        Err(BufferError::Deadline)
    );
    buffer.expire_upload(11_000).unwrap();
    let retry = buffer.begin_upload(11_000).unwrap();
    assert_eq!(retry.first_record, lease.first_record);
    assert_ne!(retry.attempt, lease.attempt);
    assert_eq!(
        buffer.acknowledge_durable(&lease, 11_001),
        Err(BufferError::StaleLease)
    );
    assert_eq!(buffer.bytes, 3);
    buffer.acknowledge_durable(&retry, 12_000).unwrap();
    assert_eq!(buffer.bytes, 0);

    let mut refused = BrowserBuffer::new(8);
    assert_eq!(refused.retain(Record(Vec::new())), Err(BufferError::Empty));
    assert_eq!(
        refused.retain(Record(vec![0; MAX_RECORD_BYTES + 1])),
        Err(BufferError::Oversized)
    );
    refused.retain(Record(vec![4])).unwrap();
    let stale = refused.begin_upload(1_000).unwrap();
    assert_eq!(refused.invalidate(9), 1);
    refused.retain(Record(vec![5])).unwrap();
    assert_eq!(
        refused.acknowledge_durable(&stale, 2_000),
        Err(BufferError::StaleLease)
    );
    assert_eq!(refused.bytes, 1);
    assert_eq!(refused.refused, 2);

    let mut previous = BrowserBuffer::new(30);
    previous.retain(Record(vec![1])).unwrap();
    let old = previous.begin_upload(0).unwrap();
    drop(previous);
    let mut current = BrowserBuffer::new(30);
    current.retain(Record(vec![2])).unwrap();
    let live = current.begin_upload(0).unwrap();
    assert_eq!(old.attempt, live.attempt);
    assert_eq!(old.first_record, live.first_record);
    assert_eq!(old.deadline_ms, live.deadline_ms);
    assert_ne!(old.owner, live.owner);
    assert_eq!(
        current.acknowledge_durable(&old, 0),
        Err(BufferError::StaleLease)
    );
    assert_eq!(current.bytes, 1);
    current.acknowledge_durable(&live, 0).unwrap();
}
```

The exact literal executes valid timeout/retry, stale-generation refusals, and
cross-reconstruction refusal after the original buffer is dropped.
The retained adversarial harness additionally covers duplicate same-tick receipts,
capacity normalization, the 8192-byte record ceiling, saturating refusal, item/byte/
batch ceilings, prefix identity, counter exhaustion and ring reuse. These local
checks do not implement receipts, filtering, RPC, timers, or a browser uploader.

## Alternatives

- Evicting the oldest record on overflow would silently lose accepted work; refuse
  the newest item and expose the gap instead.
- Clearing a batch when send starts or when the caller's wait times out would lose
  ownership before terminal durability is known; retain the lease through the
  receipt outcome.
- A second browser JSON schema, SDK exporter, or durable store would duplicate
  existing OTEL/ingress ownership without source support.

## Evidence and remaining checks

Repair input: `95d7c0cad8af34b60c9062b002fd5c2790eb474c`; integration base:
`61829e8b43cb6338f9188efbb406b68b52662b7c`. The preserved a1 fixes remain.
Independent a2 review passed 15 cases but reproduced an old lease acknowledging
a reconstructed owner with equal generation/counters/clock. The private shared
owner marker repairs that gap without a public identity allocator or RPC change.
The exact literal is extracted, formatted using the pinned root configuration,
then reinserted unchanged. Pinned Rust 2024 `rustc -D warnings` and the unchanged
v6 40%/256 MiB/60-second shared-lock guard govern finite checks. Actual a3
commands, source/tool/config/build/output hashes and results are retained in its
handoff/manifest; preserved a1/a2 failures remain explicit. Guarded pinned
formatter write/check, both warning-denied compiles, exact valid/refusal execution
and all 18 adversarial cases passed. The exact 16 reviewer cases are retained
with borrow/clone-only token access changes; the same-generation replacement
returns `StaleLease` and retains its newer byte. Two extra cases prove old tokens
keep a destroyed owner alive across 256 replacements, release the marker when
dropped, and preserve identity when the buffer moves. No assertions were weakened.

Next consumer `B-C-df-observe-I03` implements only the actual page/binding-owned
buffer at the existing browser capture seam (`crates/df-tools/src/browser.rs`,
`crates/df-observe/src/lib.rs`), preserving D01-D03 and stable capture IDs. The
planned generated UploadTelemetry adapter must map matched full/partial durable
receipts through existing ingress semantics (`crates/df-observe/src/ingress.rs`,
`crates/df-telemetry/src/wire.rs`); no df-api implementation exists here.
Cargo/native/WASM, real-browser, RPC, authorization, upload, durability, telemetry
pipeline, frontier review and coordinator integration remain unperformed/pending.
Browser crash persistence, concrete browser producer allocation, generated receipt
identity mapping and whole-path allocation bounds remain consumer gates.
