//! What each open playground page last said it was showing and playing, and what it showed when.
//!
//! A page reports every ten seconds, when its view changes, when its link to the host goes down or
//! comes back, and when its files change. The table remembers the newest report of each page by
//! the page's own name, and a bounded timeline of what the page showed and played, put on the
//! playground's clock with the page's own clock reading (`page_clock_ms`). It writes an audit line
//! only for a change: a page seen, gone or evicted, its link, revision, fixture or sound lock
//! changed, an error it met, a draw that rebuilt pieces, and each line, plate, figure, clip,
//! scene, sound, song and camera framing as it ends (`TIMING ..`). A heartbeat that says nothing
//! new writes nothing, and a report wakes no page.

mod request;
mod routes;
#[cfg(test)]
mod routes_tests;
#[cfg(test)]
mod table_tests;

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

use df_playground::limits::MAXIMUM_KEPT_TIMELINE_RECORDS;
use df_playground::media::{ClientReport, is_plain_file_name};
use df_playground::timeline::{
    ClientError, ClientTimeline, TimelineDetail, TimelineEntry, TimelineKind, TimelineQuery,
    TimelineRecord,
};

pub use self::request::{ClientTimelineReply, ClientsError};
pub(super) use self::routes::{clients_json, receive_report, timeline_json};
use crate::limits::{FORGET_CLIENT_AFTER, MAXIMUM_CLIENT_NAME, MAXIMUM_CLIENTS};
use crate::wire::ClientView;

/// Why a page's name is not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientNameError {
    Empty,
    TooLong,
    BadCharacter(char),
}

impl fmt::Display for ClientNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "a client name is letters, digits, '_' and '-', up to {MAXIMUM_CLIENT_NAME} characters"
        )?;
        match self {
            Self::Empty => formatter.write_str(" (it is empty)"),
            Self::TooLong => formatter.write_str(" (it is longer)"),
            Self::BadCharacter(character) => write!(formatter, " ('{character}' is not allowed)"),
        }
    }
}

/// Checks the name a page gives itself.
pub fn check_client_name(name: &str) -> Result<(), ClientNameError> {
    if name.is_empty() {
        return Err(ClientNameError::Empty);
    }
    if name.len() > MAXIMUM_CLIENT_NAME {
        return Err(ClientNameError::TooLong);
    }
    match name
        .chars()
        .find(|character| !(character.is_ascii_alphanumeric() || "_-".contains(*character)))
    {
        Some(character) => Err(ClientNameError::BadCharacter(character)),
        None => Ok(()),
    }
}

/// `name` when it is a page name, or `-`: what an audit line may say of a name a request gave.
pub(crate) fn checked_client(name: Option<&str>) -> &str {
    name.filter(|name| check_client_name(name).is_ok())
        .unwrap_or("-")
}

/// `value` when it is a plain name (letters, digits, `_`, `-` and `.`), or `-`: ids, media names,
/// scene files and revisions a page sends, as an audit line may carry them.
fn checked(value: &str) -> &str {
    if is_plain_file_name(value) {
        value
    } else {
        "-"
    }
}

fn checked_or_dash(value: Option<&str>) -> &str {
    value.map_or("-", checked)
}

fn number_or_dash(value: Option<u64>) -> String {
    value.map_or_else(|| "-".to_owned(), |value| value.to_string())
}

/// One page as the table keeps it.
#[derive(Debug)]
struct Page {
    /// When it last reported, on the playground's clock.
    heard_ms: u64,
    /// Its newest report, without the timeline records (they are in `timeline`).
    report: ClientReport,
    /// The newest error it reported, so each error is written once.
    last_error_seq: u64,
    /// What it showed and played, oldest first, at most `MAXIMUM_KEPT_TIMELINE_RECORDS`.
    timeline: VecDeque<TimelineEntry>,
}

/// What recording a report found: the audit lines to write, and the revision to compare with the
/// host's files when the page's revision is new to the table.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Recorded {
    pub lines: Vec<String>,
    pub check_revision: Option<String>,
}

/// The pages heard from, by name, and the `CLIENT gone` lines of the pages forgotten since the
/// lines were last taken.
#[derive(Debug, Default)]
struct Pages {
    pages: BTreeMap<String, Page>,
    /// At most `MAXIMUM_CLIENTS`; the oldest go first.
    departed: VecDeque<String>,
}

impl Pages {
    /// Forgets the pages silent for `FORGET_CLIENT_AFTER` as of `now_ms`, keeping a `CLIENT gone`
    /// line for each.
    fn forget_silent(&mut self, now_ms: u64) {
        let forget_after_ms = forget_after_ms();
        let departed = &mut self.departed;
        self.pages.retain(|client, page| {
            let silent_ms = now_ms.saturating_sub(page.heard_ms);
            let keep = silent_ms < forget_after_ms;
            if !keep {
                departed.push_back(format!("CLIENT gone client={client} silent_ms={silent_ms}"));
                if departed.len() > MAXIMUM_CLIENTS {
                    departed.pop_front();
                }
            }
            keep
        });
    }

    /// Forgets the silent pages and takes every `CLIENT gone` line kept.
    fn sweep(&mut self, now_ms: u64) -> Vec<String> {
        self.forget_silent(now_ms);
        self.departed.drain(..).collect()
    }
}

/// The pages heard from, by name.
#[derive(Debug, Default)]
pub struct ClientTable {
    pages: Mutex<Pages>,
}

fn forget_after_ms() -> u64 {
    u64::try_from(FORGET_CLIENT_AFTER.as_millis()).unwrap_or(u64::MAX)
}

/// A page millisecond on the playground's clock, by the offset of the report that carried it.
fn host_ms(page_ms: u64, offset_ms: i128) -> u64 {
    u64::try_from((i128::from(page_ms) + offset_ms).max(0)).unwrap_or(u64::MAX)
}

/// Whether something shown for `shown_ms` came down before its planned time.
fn is_short(shown_ms: Option<u64>, planned_ms: Option<u64>) -> bool {
    matches!((shown_ms, planned_ms), (Some(shown), Some(planned)) if shown < planned)
}

/// The host's entry for a page's record, on the playground's clock. A record already kept keeps
/// the start it was first given (`known_start_ms`), whatever the offset of a later report.
fn entry_of(record: TimelineRecord, offset_ms: i128, known_start_ms: Option<u64>) -> TimelineEntry {
    let start_ms = known_start_ms.unwrap_or_else(|| host_ms(record.start_page_ms, offset_ms));
    let end_ms = record
        .end_page_ms
        .map(|end| host_ms(end, offset_ms).max(start_ms));
    let shown_ms = end_ms.map(|end| end - start_ms);
    TimelineEntry {
        seq: record.seq,
        kind: record.kind,
        id: record.id,
        start_ms,
        end_ms,
        shown_ms,
        planned_ms: record.planned_ms,
        end_reason: record.end_reason,
        short: is_short(shown_ms, record.planned_ms),
        detail: record.detail,
        message: None,
    }
}

fn error_entry(error: &ClientError, offset_ms: i128) -> TimelineEntry {
    let at_ms = host_ms(error.page_ms, offset_ms);
    TimelineEntry {
        seq: error.seq,
        kind: TimelineKind::Error,
        id: error.source.key().to_owned(),
        start_ms: at_ms,
        end_ms: Some(at_ms),
        shown_ms: None,
        planned_ms: None,
        end_reason: None,
        short: false,
        detail: None,
        message: Some(error.message.clone()),
    }
}

/// The line a record writes when it first arrives: a sound or a song started, a draw.
fn arrival_line(client: &str, entry: &TimelineEntry) -> Option<String> {
    match (entry.kind, &entry.detail) {
        (TimelineKind::Sound | TimelineKind::Music, _) => Some(format!(
            "CLIENT {} client={client} media={} action=started at_ms={}",
            entry.kind.key(),
            checked(&entry.id),
            entry.start_ms
        )),
        (
            TimelineKind::Draw,
            Some(TimelineDetail::Draw {
                frame,
                devices,
                parts,
                transition,
                moved,
            }),
        ) => {
            let parts: Vec<&str> = parts.iter().map(|part| checked(part)).collect();
            Some(format!(
                "CLIENT draw client={client} seq={} frame={} at_ms={} devices={devices} \
                 parts={} transition={} moved={moved}",
                entry.seq,
                number_or_dash(*frame),
                entry.start_ms,
                if parts.is_empty() {
                    "-".to_owned()
                } else {
                    parts.join(",")
                },
                transition.map_or("none", |style| style.key()),
            ))
        }
        _ => None,
    }
}

/// The line a record writes when it ends.
fn ending_line(client: &str, entry: &TimelineEntry) -> Option<String> {
    let end_ms = entry.end_ms?;
    let reason = entry.end_reason.map_or("-", |reason| reason.key());
    let id = checked(&entry.id);
    match (entry.kind, &entry.detail) {
        (
            TimelineKind::Line | TimelineKind::Plate | TimelineKind::Figure | TimelineKind::Clip,
            _,
        ) => Some(format!(
            "TIMING {} client={client} id={id} start_ms={} end_ms={end_ms} shown_ms={} \
                 planned_ms={} reason={reason}{}",
            entry.kind.key(),
            entry.start_ms,
            end_ms - entry.start_ms,
            number_or_dash(entry.planned_ms),
            if entry.short { " short=true" } else { "" },
        )),
        (TimelineKind::Scene, detail) => {
            let (from, transition) = match detail {
                Some(TimelineDetail::Scene { from, transition }) => (
                    checked_or_dash(from.as_deref()),
                    transition.map_or("none", |style| style.key()),
                ),
                _ => ("-", "none"),
            };
            Some(format!(
                "TIMING scene client={client} from={from} to={id} transition={transition} \
                 start_ms={} end_ms={end_ms} reason={reason}",
                entry.start_ms
            ))
        }
        (TimelineKind::Sound | TimelineKind::Music, _) => Some(format!(
            "CLIENT {} client={client} media={id} action={reason} at_ms={end_ms}",
            entry.kind.key()
        )),
        (TimelineKind::Camera, detail) => {
            let (zoom, allowed) = match detail {
                Some(TimelineDetail::Camera {
                    zoom_percent,
                    allowed_percent,
                }) => (
                    zoom_percent.to_string(),
                    number_or_dash(allowed_percent.map(u64::from)),
                ),
                _ => ("-".to_owned(), "-".to_owned()),
            };
            Some(format!(
                "CLIENT camera client={client} serial={id} zoom={zoom} allowed={allowed} \
                 start_ms={} end_ms={end_ms} reason={reason}",
                entry.start_ms
            ))
        }
        (TimelineKind::Draw | TimelineKind::Error, _) => None,
    }
}

/// The lines a change between the page's last report and this one writes.
fn change_lines(
    client: &str,
    before: &ClientReport,
    after: &ClientReport,
    now_ms: u64,
) -> Vec<String> {
    let mut lines = Vec::new();
    if before.link != after.link {
        lines.push(format!(
            "CLIENT link client={client} state={} failures={}",
            after.link.key(),
            after.failures
        ));
    }
    if before.asset_revision != after.asset_revision {
        lines.push(format!(
            "CLIENT revision client={client} from={} to={}",
            checked(&before.asset_revision),
            checked(&after.asset_revision)
        ));
    }
    if (&before.fixture, before.fixture_error) != (&after.fixture, after.fixture_error)
        && after.fixture.is_some()
    {
        lines.push(format!(
            "CLIENT fixture client={client} name={} result={}",
            checked_or_dash(after.fixture.as_deref()),
            after.fixture_error.map_or("ok", |cause| cause.key())
        ));
    }
    if before.audio != after.audio {
        lines.push(format!(
            "CLIENT audio client={client} state={} at_ms={now_ms}",
            after.audio.key()
        ));
    }
    lines
}

impl ClientTable {
    fn lock(&self) -> MutexGuard<'_, Pages> {
        self.pages.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Remembers what a page said at `now_ms` and says which changes it brought. The report is
    /// bounded; pages silent too long are forgotten first, and past `MAXIMUM_CLIENTS` a new page
    /// replaces the page heard from longest ago.
    pub(crate) fn record(&self, client: String, report: ClientReport, now_ms: u64) -> Recorded {
        let mut recorded = Recorded::default();
        let mut table = self.lock();
        recorded.lines = table.sweep(now_ms);
        let lines = &mut recorded.lines;
        let pages = &mut table.pages;
        let mut report = report.bounded();
        let records = std::mem::take(&mut report.timeline);
        let offset_ms = i128::from(now_ms) - i128::from(report.page_clock_ms);
        if !pages.contains_key(&client) {
            if pages.len() >= MAXIMUM_CLIENTS
                && let Some(oldest) = pages
                    .iter()
                    .min_by_key(|(_, page)| page.heard_ms)
                    .map(|(name, _)| name.clone())
            {
                pages.remove(&oldest);
                lines.push(format!("CLIENT evicted client={oldest}"));
            }
            lines.push(format!(
                "CLIENT seen client={client} device={} viewport={}",
                report.device.key(),
                report.viewport_px.map_or_else(
                    || "-".to_owned(),
                    |viewport| format!("{}x{}", viewport.width, viewport.height)
                )
            ));
            recorded.check_revision = Some(report.asset_revision.clone());
            let baseline = ClientReport {
                asset_revision: report.asset_revision.clone(),
                ..ClientReport::default()
            };
            pages.insert(
                client.clone(),
                Page {
                    heard_ms: now_ms,
                    report: baseline,
                    last_error_seq: 0,
                    timeline: VecDeque::new(),
                },
            );
        }
        let Some(page) = pages.get_mut(&client) else {
            return recorded;
        };
        lines.extend(change_lines(&client, &page.report, &report, now_ms));
        if page.report.asset_revision != report.asset_revision {
            recorded.check_revision = Some(report.asset_revision.clone());
        }
        for error in &report.errors {
            if error.seq > page.last_error_seq {
                lines.push(format!(
                    "CLIENT error client={client} seq={} source={} at_ms={} chars={}",
                    error.seq,
                    error.source.key(),
                    host_ms(error.page_ms, offset_ms),
                    error.message.chars().count()
                ));
                page.last_error_seq = error.seq;
                push_entry(&mut page.timeline, error_entry(error, offset_ms));
            }
        }
        let mut records = records;
        records.sort_by_key(|record| record.seq);
        for record in records {
            if record.kind == TimelineKind::Error {
                continue;
            }
            let known = page
                .timeline
                .iter_mut()
                .rev()
                .find(|entry| entry.seq == record.seq && entry.kind == record.kind);
            match known {
                Some(entry) => {
                    let was_open = entry.end_ms.is_none();
                    *entry = entry_of(record, offset_ms, Some(entry.start_ms));
                    if was_open && let Some(line) = ending_line(&client, entry) {
                        lines.push(line);
                    }
                }
                None => {
                    let entry = entry_of(record, offset_ms, None);
                    lines.extend(arrival_line(&client, &entry));
                    lines.extend(ending_line(&client, &entry));
                    push_entry(&mut page.timeline, entry);
                }
            }
        }
        page.heard_ms = now_ms;
        page.report = report;
        recorded
    }

    /// Forgets the pages silent for `FORGET_CLIENT_AFTER` as of `now_ms`, and gives their
    /// `CLIENT gone` lines.
    pub(crate) fn sweep(&self, now_ms: u64) -> Vec<String> {
        self.lock().sweep(now_ms)
    }

    /// The pages heard from as of `now_ms`, with how long ago each last spoke. A page silent for
    /// `FORGET_CLIENT_AFTER` is dropped from the answer and from memory; its `CLIENT gone` line is
    /// kept for the next `sweep` or `record`.
    pub fn views_at(&self, now_ms: u64) -> Vec<ClientView> {
        let mut table = self.lock();
        table.forget_silent(now_ms);
        table
            .pages
            .iter()
            .map(|(client, page)| ClientView {
                client: client.clone(),
                age_ms: now_ms.saturating_sub(page.heard_ms),
                report: page.report.clone(),
            })
            .collect()
    }

    /// The timeline of `client`, oldest first, held to `query`, or `None` when no such page is
    /// open.
    pub(crate) fn timeline(&self, client: &str, query: TimelineQuery) -> Option<ClientTimeline> {
        let table = self.lock();
        let page = table.pages.get(client)?;
        let mut entries: Vec<TimelineEntry> = page
            .timeline
            .iter()
            .filter(|entry| query.kind.is_none_or(|kind| entry.kind == kind))
            .filter(|entry| query.after.is_none_or(|after| entry.seq > after))
            .cloned()
            .collect();
        entries.sort_by_key(|entry| entry.seq);
        Some(ClientTimeline {
            client: client.to_owned(),
            entries,
        })
    }
}

fn push_entry(timeline: &mut VecDeque<TimelineEntry>, entry: TimelineEntry) {
    timeline.push_back(entry);
    if timeline.len() > MAXIMUM_KEPT_TIMELINE_RECORDS {
        timeline.pop_front();
    }
}
