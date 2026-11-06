//! Size, count and time bounds of native capture, browser buffering and presentation timing.
use std::time::Duration;

// Per-record structure bounds of native capture. They keep caller input inside the ingress
// decoder's field and depth budgets (df-telemetry `limits.rs`), so they are part of the wire
// contract, not tunables.
/// Most attributes on one log record, span or span link.
pub(crate) const MAXIMUM_ATTRIBUTES: usize = 64;
/// Most events on one completed span.
pub(crate) const MAXIMUM_SPAN_EVENTS: usize = 128;
/// Most links on one span.
pub(crate) const MAXIMUM_SPAN_LINKS: usize = 16;
/// Longest span name.
pub(crate) const MAXIMUM_SPAN_NAME_BYTES: usize = 256;
/// Longest service name and log event name.
pub(crate) const MAXIMUM_NAME_BYTES: usize = 128;
/// Deepest nesting of list and map log values.
pub(crate) const MAXIMUM_VALUE_DEPTH: usize = 8;
/// Most elements of one list, map or array value.
pub(crate) const MAXIMUM_VALUE_ELEMENTS: usize = 64;
/// Most values one caller input may hold; bounds traversal before SDK conversion.
pub(crate) const MAXIMUM_INPUT_NODES: usize = 512;

/// Smallest `TelemetryLimits::sqlite_bytes` a store accepts.
pub(crate) const MINIMUM_SQLITE_BYTES: u64 = 256 * 1024;
/// Spans the fixture exporter retains; later spans are counted as dropped.
pub(crate) const MAXIMUM_FIXTURE_SPANS: usize = 512;

/// Records the browser buffer retains before refusing new ones.
pub(crate) const MAXIMUM_BROWSER_RECORDS: usize = 128;
/// Encoded bytes the browser buffer retains across all records.
pub(crate) const MAXIMUM_BROWSER_BYTES: usize = 256 * 1024;
/// Largest encoded browser record.
pub(crate) const MAXIMUM_BROWSER_RECORD_BYTES: usize = 8192;
/// Longest browser event name or status; both come from static instrumentation.
pub(crate) const MAXIMUM_BROWSER_TEXT_BYTES: usize = 256;
/// Most records one browser upload lease carries.
pub(crate) const MAXIMUM_UPLOAD_RECORDS: usize = 32;
/// Most encoded bytes one browser upload lease carries.
pub(crate) const MAXIMUM_UPLOAD_BYTES: usize = 64 * 1024;
/// How long a browser upload lease lasts; an expired lease's records stay retained.
pub(crate) const UPLOAD_DEADLINE: Duration = Duration::from_secs(10);

/// Most timing events one page retains before refusing new ones.
pub const MAXIMUM_TIMING_EVENTS: usize = 128;
/// Most timing events in one upload batch.
pub const MAXIMUM_TIMING_BATCH_EVENTS: usize = 32;
/// Largest encoded timing upload batch: room for authenticated generated-RPC scope/identity
/// fields under the 64 KiB bridge cap.
pub const MAXIMUM_TIMING_BATCH_BYTES: usize = 60 * 1024;
/// Longest page-relative time or held span a timing event may carry: one week.
pub(crate) const MAXIMUM_PAGE_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
