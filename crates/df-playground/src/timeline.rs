//! What a page showed and played, and when, so scene timing can be measured from the host's log.
//!
//! A page keeps a `PageLog`: every line, plate, figure, clip, scene, sound, music, camera framing
//! and rebuilding draw it starts or ends becomes a `TimelineRecord` in page milliseconds, and every
//! error it meets a `ClientError`. The report it sends the host carries the records new or closed
//! since the last report it delivered, and the host aligns them to its own clock and writes the
//! audit lines. The diff of what is shown (`Showing`) is plain Rust that native tests cover; the
//! page only measures the time and says what it drew.

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::bubble_place::BubbleSide;
use crate::clues::ClueBoard;
use crate::controls::TransitionStyle;
use crate::copy::{ReadingPace, caption_dwell_ms, result_card_dwell_ms, word_count};
use crate::fixture::{Fixture, StageElement};
use crate::image_layout::speaker_masks;
use crate::limits::{
    MAXIMUM_REPORTED_ERROR_CHARACTERS, MAXIMUM_REPORTED_ERRORS, MAXIMUM_TIMELINE_ID_CHARACTERS,
    MAXIMUM_TIMELINE_PARTS, MAXIMUM_TIMELINE_RECORDS,
};
use crate::media::{MediaKind, media_kind};

/// What a timeline record is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineKind {
    /// A caption or narration line on the shared screen.
    Line,
    /// A still picture on the shared screen: the backdrop or a card.
    Plate,
    /// A figure on the stage.
    Figure,
    /// A moving picture on the shared screen.
    Clip,
    /// The scene (the scenario file) shown.
    Scene,
    /// A sound effect.
    Sound,
    /// A music track.
    Music,
    /// A draw that rebuilt the devices or some of their pieces.
    Draw,
    /// A camera framing.
    Camera,
    /// A microphone recording, from the moment it began to the moment it stopped. Its id is
    /// `microphone`; the audio is never named or kept.
    Capture,
    /// A recording on its way to text, from the request to the answer. Its id is `speech`; the
    /// text is never named.
    Transcribe,
    /// A question put to a character, from the request to the answer. Its id is the character's.
    Ask,
    /// A speaking character's outline in the picture, from the moment their line starts to the
    /// moment it ends. Its id is the character's.
    Outline,
    /// The card of a speaker who is not in the picture, from the moment their line starts to the
    /// moment it ends. Its id is `card-<character>-<line>`.
    Card,
    /// A backdrop crossfade: the wait for the new plate's image (`fade-load-<media>`) and its fade
    /// (`fade-<media>`).
    Fade,
    /// A visible speaker's bubble given way to the lower third during a plate change
    /// (`third-<character>`).
    Third,
    /// The clue board on the shared screen, from the draw that put it up (or changed it) to the
    /// draw that changed or took it away. Its id is `clues-<notes>-<hash>`
    /// (`ClueBoard::board_identity`).
    Clues,
    /// An error the page met. The page never sends this kind as a record; the host makes one from
    /// each error a report carries, so a timeline read shows errors in their place.
    Error,
}

impl TimelineKind {
    pub const ALL: [Self; 18] = [
        Self::Line,
        Self::Plate,
        Self::Figure,
        Self::Clip,
        Self::Scene,
        Self::Sound,
        Self::Music,
        Self::Draw,
        Self::Camera,
        Self::Capture,
        Self::Transcribe,
        Self::Ask,
        Self::Outline,
        Self::Card,
        Self::Fade,
        Self::Third,
        Self::Clues,
        Self::Error,
    ];

    /// The serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Plate => "plate",
            Self::Figure => "figure",
            Self::Clip => "clip",
            Self::Scene => "scene",
            Self::Sound => "sound",
            Self::Music => "music",
            Self::Draw => "draw",
            Self::Camera => "camera",
            Self::Capture => "capture",
            Self::Transcribe => "transcribe",
            Self::Ask => "ask",
            Self::Outline => "outline",
            Self::Card => "card",
            Self::Fade => "fade",
            Self::Third => "third",
            Self::Clues => "clues",
            Self::Error => "error",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }
}

/// Why something stopped being shown or played.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    /// Another of its kind took its place.
    Replaced,
    /// The page let it go without it ending (a sound held past the page's bound).
    Expired,
    /// The scene changed under it.
    SceneChange,
    /// It was taken away and nothing took its place.
    Removed,
    /// It played to its end.
    Ended,
    /// The camera was aimed somewhere else.
    Retargeted,
    /// The browser refused it or could not load it.
    Failed,
}

impl EndReason {
    pub const ALL: [Self; 7] = [
        Self::Replaced,
        Self::Expired,
        Self::SceneChange,
        Self::Removed,
        Self::Ended,
        Self::Retargeted,
        Self::Failed,
    ];

    /// The serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Replaced => "replaced",
            Self::Expired => "expired",
            Self::SceneChange => "scene_change",
            Self::Removed => "removed",
            Self::Ended => "ended",
            Self::Retargeted => "retargeted",
            Self::Failed => "failed",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| reason.key() == key)
    }
}

/// Where an error the page met came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorSource {
    /// Drawing the page.
    Draw,
    /// Asking the host.
    Net,
    /// Moving the figures or framing the camera.
    Stage,
    /// A picture, the theme or a fixture file.
    Asset,
    /// The page's code panicked.
    Panic,
    /// Telling the host what the page shows.
    Report,
    /// Playing sound.
    Audio,
}

impl ErrorSource {
    pub const ALL: [Self; 7] = [
        Self::Draw,
        Self::Net,
        Self::Stage,
        Self::Asset,
        Self::Panic,
        Self::Report,
        Self::Audio,
    ];

    /// The serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Draw => "draw",
            Self::Net => "net",
            Self::Stage => "stage",
            Self::Asset => "asset",
            Self::Panic => "panic",
            Self::Report => "report",
            Self::Audio => "audio",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|source| source.key() == key)
    }

    /// Whether an error from here is sent to the host at once instead of with the next report: a
    /// panic may be the last thing the page does.
    pub fn forces_report(self) -> bool {
        self == Self::Panic
    }
}

/// One error the page met. The message is for a person reading `clients`; the host's audit line
/// carries only its source, its sequence and its length.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientError {
    /// The page's sequence number, shared with its timeline records.
    pub seq: u64,
    pub source: ErrorSource,
    /// When it happened, in milliseconds on the page's clock.
    pub page_ms: u64,
    pub message: String,
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

/// What more a record of some kinds says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "detail", rename_all = "snake_case", deny_unknown_fields)]
pub enum TimelineDetail {
    /// A draw that rebuilt the devices (`devices`) or some pieces of them (`parts`, their DOM
    /// markers), with the frame it drew and how many stage-only moves came since the last one.
    Draw {
        frame: Option<u64>,
        devices: bool,
        parts: Vec<String>,
        transition: Option<TransitionStyle>,
        moved: u32,
    },
    /// A camera framing: the zoom asked for, and the zoom drawn when the picture could not bear it.
    Camera {
        zoom_percent: u16,
        allowed_percent: Option<u16>,
    },
    /// An anchored bubble placed: the side of its character's head it went to, and whether it
    /// moved from a place it had (a bubble shown for the first time did not move).
    Bubble { side: BubbleSide, moved: bool },
    /// A scene shown: the scene before it and how the page left that one.
    Scene {
        from: Option<String>,
        transition: Option<TransitionStyle>,
    },
}

/// One thing the page showed or played, in milliseconds on the page's clock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineRecord {
    /// The page's sequence number; a record sent open and later closed keeps its number.
    pub seq: u64,
    pub kind: TimelineKind,
    /// A media name, a figure id, a scene file, a camera serial, or `line-<hash>` for a line (never
    /// the line's words).
    pub id: String,
    pub start_page_ms: u64,
    /// `None` while it is still shown or playing.
    pub end_page_ms: Option<u64>,
    pub end_reason: Option<EndReason>,
    /// How long it was meant to stay, when the page knows: a line's dwell, a card's dwell, a camera
    /// move's length.
    pub planned_ms: Option<u64>,
    pub detail: Option<TimelineDetail>,
}

impl TimelineRecord {
    /// Holds the record's texts to their bounds.
    pub fn bounded(mut self) -> Self {
        cut(&mut self.id, MAXIMUM_TIMELINE_ID_CHARACTERS);
        match &mut self.detail {
            Some(TimelineDetail::Draw { parts, .. }) => {
                parts.truncate(MAXIMUM_TIMELINE_PARTS);
                parts
                    .iter_mut()
                    .for_each(|part| cut(part, MAXIMUM_TIMELINE_ID_CHARACTERS));
            }
            Some(TimelineDetail::Scene {
                from: Some(from), ..
            }) => cut(from, MAXIMUM_TIMELINE_ID_CHARACTERS),
            _ => {}
        }
        self
    }

    /// The bytes of text the record holds.
    pub fn held_bytes(&self) -> usize {
        let detail = match &self.detail {
            Some(TimelineDetail::Draw { parts, .. }) => parts.iter().map(String::len).sum(),
            Some(TimelineDetail::Scene { from, .. }) => from.as_ref().map_or(0, String::len),
            _ => 0,
        };
        self.id.len() + detail
    }
}

/// Cuts `text` to `limit` characters, on a character.
pub(crate) fn cut(text: &mut String, limit: usize) {
    if let Some((index, _)) = text.char_indices().nth(limit) {
        text.truncate(index);
    }
}

/// The id of a line: a hash of its words, so the log can tell lines apart without carrying them.
pub fn line_id(text: &str) -> String {
    let hash = text.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    });
    format!("line-{hash:08x}")
}

/// A camera framing as the timeline keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShownCamera {
    pub serial: u64,
    pub zoom_percent: u16,
    pub duration_ms: u64,
}

/// What the shared screen shows now, as timeline ids. Only the shared screen is followed: it is
/// what the table reads across the room.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Showing {
    /// The scenario file shown.
    pub scene: Option<String>,
    /// How the page leaves the scene before, when `scene` has just changed.
    pub transition: Option<TransitionStyle>,
    /// Each line by id, with its dwell.
    pub lines: BTreeMap<String, u64>,
    /// Each still picture by media name, with its dwell when it has one (a card).
    pub plates: BTreeMap<String, Option<u64>>,
    pub clips: BTreeMap<String, Option<u64>>,
    pub figures: Vec<String>,
    /// The characters outlined as speaking, by name.
    pub outlines: Vec<String>,
    /// The off-screen speaker's card, by its key.
    pub cards: Vec<String>,
    /// The clue board, by its key.
    pub clues: Vec<String>,
    pub camera: Option<ShownCamera>,
}

impl Showing {
    /// What the shared screen shows of `fixture`, with `elements` on its stage, read at `pace`. A
    /// page that does not show the shared screen shows nothing of it.
    pub fn of(
        scene: Option<&str>,
        fixture: Option<&Fixture>,
        elements: Option<&[StageElement]>,
        pace: ReadingPace,
        shows_tv: bool,
    ) -> Self {
        let mut showing = Self {
            scene: scene.map(str::to_owned),
            ..Self::default()
        };
        let Some(fixture) = fixture.filter(|_| shows_tv) else {
            return showing;
        };
        let tv = &fixture.tv;
        let line_texts = [
            tv.caption.as_ref().map(|caption| caption.text.as_str()),
            tv.narration
                .as_ref()
                .map(|narration| narration.text.as_str()),
            tv.roleplay
                .as_ref()
                .and_then(|roleplay| roleplay.caption.as_deref()),
        ];
        for text in line_texts.into_iter().flatten() {
            showing
                .lines
                .insert(line_id(text), caption_dwell_ms(word_count(text), pace));
        }
        let pictures = [
            (tv.backdrop.as_ref(), None),
            (tv.card.as_ref(), Some(result_card_dwell_ms(pace))),
        ];
        for (picture, dwell_ms) in pictures {
            let Some(picture) = picture else { continue };
            let held = match media_kind(&picture.media) {
                Some(MediaKind::Video) => &mut showing.clips,
                _ => &mut showing.plates,
            };
            held.insert(picture.media.clone(), dwell_ms);
        }
        showing.figures = elements
            .unwrap_or_default()
            .iter()
            .map(|element| element.id.clone())
            .collect();
        if let Some(roleplay) = &tv.roleplay {
            showing.outlines = speaker_masks(&roleplay.bubbles, tv.backdrop_layout())
                .into_iter()
                .map(|(character, _)| character.to_owned())
                .collect();
        }
        showing.cards = crate::speaker_card::scene_card(tv)
            .map(|card| card.card_identity())
            .into_iter()
            .collect();
        showing.clues = tv.clues.iter().map(ClueBoard::board_identity).collect();
        showing.camera = tv.camera.as_ref().map(|camera| ShownCamera {
            serial: camera.serial,
            zoom_percent: camera.zoom_percent,
            duration_ms: camera.duration_ms,
        });
        showing
    }
}

/// The page's own account of what it showed and what went wrong, held to its bounds.
#[derive(Debug, Default)]
pub struct PageLog {
    next_seq: u64,
    errors: VecDeque<ClientError>,
    /// Records opened or closed since the last delivered report, each with the push number that
    /// says whether a report carried it. The oldest go first past `MAXIMUM_TIMELINE_RECORDS`.
    unreported: VecDeque<(u64, TimelineRecord)>,
    pushes: u64,
    /// Records still open, by sequence number.
    open: BTreeMap<u64, TimelineRecord>,
    /// The open record of each shown thing, by kind and id.
    keyed: BTreeMap<(TimelineKind, String), u64>,
    /// What was shown at the last look.
    showing: Showing,
    /// Stage-only moves since the last rebuilding draw.
    moved: u32,
}

impl PageLog {
    fn next_seq(&mut self) -> u64 {
        self.next_seq += 1;
        self.next_seq
    }

    fn push(&mut self, record: TimelineRecord) {
        self.pushes += 1;
        self.unreported.push_back((self.pushes, record.bounded()));
        if self.unreported.len() > MAXIMUM_TIMELINE_RECORDS {
            self.unreported.pop_front();
        }
    }

    /// Records an error. A repeat of the newest error is dropped. Returns whether the error should
    /// be sent to the host now (`ErrorSource::forces_report`).
    pub fn note_error(&mut self, source: ErrorSource, message: &str, now_ms: u64) -> bool {
        let repeat = self
            .errors
            .back()
            .is_some_and(|newest| newest.source == source && newest.message == message);
        if !repeat {
            let mut message = message.to_owned();
            cut(&mut message, MAXIMUM_REPORTED_ERROR_CHARACTERS);
            let seq = self.next_seq();
            self.errors.push_back(ClientError {
                seq,
                source,
                page_ms: now_ms,
                message,
            });
            if self.errors.len() > MAXIMUM_REPORTED_ERRORS {
                self.errors.pop_front();
            }
        }
        source.forces_report()
    }

    /// The errors kept, oldest first.
    pub fn errors(&self) -> Vec<ClientError> {
        self.errors.iter().cloned().collect()
    }

    /// Starts a record and returns its sequence number.
    pub fn open(
        &mut self,
        kind: TimelineKind,
        id: &str,
        now_ms: u64,
        planned_ms: Option<u64>,
        detail: Option<TimelineDetail>,
    ) -> u64 {
        let seq = self.next_seq();
        let record = TimelineRecord {
            seq,
            kind,
            id: id.to_owned(),
            start_page_ms: now_ms,
            end_page_ms: None,
            end_reason: None,
            planned_ms,
            detail,
        };
        self.push(record.clone());
        self.open.insert(seq, record);
        seq
    }

    /// Ends the open record `seq`. A record already ended, or let go, is left.
    pub fn close(&mut self, seq: u64, now_ms: u64, reason: EndReason) {
        if let Some(mut record) = self.open.remove(&seq) {
            record.end_page_ms = Some(now_ms.max(record.start_page_ms));
            record.end_reason = Some(reason);
            self.keyed.remove(&(record.kind, record.id.clone()));
            self.push(record);
        }
    }

    fn open_keyed(
        &mut self,
        kind: TimelineKind,
        id: &str,
        now_ms: u64,
        planned_ms: Option<u64>,
        detail: Option<TimelineDetail>,
    ) {
        let seq = self.open(kind, id, now_ms, planned_ms, detail);
        self.keyed.insert((kind, id.to_owned()), seq);
    }

    fn close_keyed(&mut self, kind: TimelineKind, id: &str, now_ms: u64, reason: EndReason) {
        if let Some(seq) = self.keyed.get(&(kind, id.to_owned())).copied() {
            self.close(seq, now_ms, reason);
        }
    }

    /// Opens and closes the records of what changed between the last look and `showing`: what
    /// left is closed first, so a thing replaced ends when its successor starts.
    pub fn observe(&mut self, showing: Showing, now_ms: u64) {
        let old = std::mem::take(&mut self.showing);
        let scene_changed = showing.scene != old.scene;
        let before = shown_items(&old);
        let after = shown_items(&showing);
        for ((kind, gone), (_, kept)) in before.iter().zip(&after) {
            let reason = if scene_changed {
                EndReason::SceneChange
            } else if !kept.is_empty() && *kind != TimelineKind::Figure {
                EndReason::Replaced
            } else {
                EndReason::Removed
            };
            for (id, _) in gone {
                if scene_changed || !kept.iter().any(|(kept_id, _)| kept_id == id) {
                    self.close_keyed(*kind, id, now_ms, reason);
                }
            }
        }
        let camera_end = match (old.camera, showing.camera) {
            (Some(was), Some(now)) if was.serial == now.serial && !scene_changed => None,
            (Some(was), Some(_)) if !scene_changed => Some((was, EndReason::Retargeted)),
            (Some(was), _) if scene_changed => Some((was, EndReason::SceneChange)),
            (Some(was), _) => Some((was, EndReason::Removed)),
            (None, _) => None,
        };
        if let Some((was, reason)) = camera_end {
            self.close_keyed(
                TimelineKind::Camera,
                &was.serial.to_string(),
                now_ms,
                reason,
            );
        }
        if scene_changed {
            if let Some(was) = &old.scene {
                let reason = if showing.scene.is_some() {
                    EndReason::Replaced
                } else {
                    EndReason::Removed
                };
                self.close_keyed(TimelineKind::Scene, was, now_ms, reason);
            }
            if let Some(now) = &showing.scene {
                let detail = TimelineDetail::Scene {
                    from: old.scene.clone(),
                    transition: showing.transition,
                };
                self.open_keyed(TimelineKind::Scene, now, now_ms, None, Some(detail));
            }
        }
        for ((kind, kept), (_, gone)) in after.iter().zip(&before) {
            for (id, planned_ms) in kept {
                if scene_changed || !gone.iter().any(|(gone_id, _)| gone_id == id) {
                    self.open_keyed(*kind, id, now_ms, *planned_ms, None);
                }
            }
        }
        if let Some(now) = showing.camera
            && (camera_end.is_some() || old.camera.is_none())
        {
            let detail = TimelineDetail::Camera {
                zoom_percent: now.zoom_percent,
                allowed_percent: None,
            };
            let id = now.serial.to_string();
            self.open_keyed(
                TimelineKind::Camera,
                &id,
                now_ms,
                Some(now.duration_ms),
                Some(detail),
            );
        }
        self.showing = showing;
    }

    /// The camera framing `serial` asked for `requested` and the picture bears only `allowed`:
    /// the zoom drawn is noted on its record, which is opened if the page has none yet.
    pub fn note_camera_clamp(&mut self, serial: u64, requested: u16, allowed: u16, now_ms: u64) {
        let id = serial.to_string();
        let detail = TimelineDetail::Camera {
            zoom_percent: requested,
            allowed_percent: Some(allowed),
        };
        match self.keyed.get(&(TimelineKind::Camera, id.clone())).copied() {
            Some(seq) => {
                if let Some(record) = self.open.get_mut(&seq) {
                    record.detail = Some(detail);
                    let updated = record.clone();
                    self.push(updated);
                }
            }
            None => self.open_keyed(TimelineKind::Camera, &id, now_ms, None, Some(detail)),
        }
    }

    /// A draw that only moved figures or framed the camera; it is counted into the next
    /// rebuilding draw's record instead of making one of its own.
    pub fn note_stage_move(&mut self) {
        self.moved = self.moved.saturating_add(1);
    }

    /// A draw that rebuilt the devices or some pieces of them.
    pub fn note_draw(
        &mut self,
        frame: Option<u64>,
        devices: bool,
        parts: Vec<String>,
        transition: Option<TransitionStyle>,
        now_ms: u64,
    ) {
        let detail = TimelineDetail::Draw {
            frame,
            devices,
            parts,
            transition,
            moved: std::mem::take(&mut self.moved),
        };
        let id = if devices { "devices" } else { "parts" };
        let seq = self.next_seq();
        self.push(TimelineRecord {
            seq,
            kind: TimelineKind::Draw,
            id: id.to_owned(),
            start_page_ms: now_ms,
            end_page_ms: Some(now_ms),
            end_reason: None,
            planned_ms: None,
            detail: Some(detail),
        });
    }

    /// An anchored bubble of `character` was placed on `side` of its head. `moved` is true when
    /// it had another place before. Only a change is noted, never a redraw that kept the place.
    pub fn note_bubble(&mut self, character: &str, side: BubbleSide, moved: bool, now_ms: u64) {
        let seq = self.next_seq();
        self.push(TimelineRecord {
            seq,
            kind: TimelineKind::Draw,
            id: character.to_owned(),
            start_page_ms: now_ms,
            end_page_ms: Some(now_ms),
            end_reason: None,
            planned_ms: None,
            detail: Some(TimelineDetail::Bubble { side, moved }),
        });
    }

    /// The records new or changed since the last delivered report, one per sequence number (its
    /// newest state), and the push number to acknowledge once a report carrying them is delivered.
    pub fn unreported(&self) -> (Vec<TimelineRecord>, u64) {
        let mut newest: BTreeMap<u64, &TimelineRecord> = BTreeMap::new();
        for (_, record) in &self.unreported {
            newest.insert(record.seq, record);
        }
        (newest.into_values().cloned().collect(), self.pushes)
    }

    /// Forgets the records a delivered report carried: those pushed up to `through`.
    pub fn acknowledge(&mut self, through: u64) {
        self.unreported.retain(|(push, _)| *push > through);
    }

    /// How many records wait to be reported.
    pub fn unreported_len(&self) -> usize {
        self.unreported.len()
    }
}

/// Each kind of thing followed by id, with what was shown of it: its id and its dwell.
type ShownItems = [(TimelineKind, Vec<(String, Option<u64>)>); 7];

fn shown_items(showing: &Showing) -> ShownItems {
    let with_dwell = |items: &BTreeMap<String, Option<u64>>| {
        items
            .iter()
            .map(|(id, planned_ms)| (id.clone(), *planned_ms))
            .collect()
    };
    [
        (
            TimelineKind::Line,
            showing
                .lines
                .iter()
                .map(|(id, planned_ms)| (id.clone(), Some(*planned_ms)))
                .collect(),
        ),
        (TimelineKind::Plate, with_dwell(&showing.plates)),
        (TimelineKind::Clip, with_dwell(&showing.clips)),
        (
            TimelineKind::Figure,
            showing
                .figures
                .iter()
                .map(|id| (id.clone(), None))
                .collect(),
        ),
        (
            TimelineKind::Outline,
            showing
                .outlines
                .iter()
                .map(|character| (character.clone(), None))
                .collect(),
        ),
        (
            TimelineKind::Card,
            showing
                .cards
                .iter()
                .map(|key| (key.clone(), None))
                .collect(),
        ),
        (
            TimelineKind::Clues,
            showing
                .clues
                .iter()
                .map(|key| (key.clone(), None))
                .collect(),
        ),
    ]
}

/// One record as the host keeps it: on the host's clock, and with how long it was shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineEntry {
    pub seq: u64,
    pub kind: TimelineKind,
    /// As the page sent it; for an error, the error's source.
    pub id: String,
    /// Milliseconds on the host's playground clock.
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    /// `end_ms - start_ms`, once it ended.
    pub shown_ms: Option<u64>,
    pub planned_ms: Option<u64>,
    pub end_reason: Option<EndReason>,
    /// It ended before its planned time.
    pub short: bool,
    pub detail: Option<TimelineDetail>,
    /// For an error, its message.
    pub message: Option<String>,
}

/// One page's timeline, oldest first, as `GET /playground/clients/{client}/timeline.json` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientTimeline {
    pub client: String,
    pub entries: Vec<TimelineEntry>,
}

/// The query of a timeline read: one kind, and entries after a sequence number.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineQuery {
    pub kind: Option<TimelineKind>,
    pub after: Option<u64>,
}

/// What a page reads as its device's size, in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewportPx {
    pub width: u32,
    pub height: u32,
}

/// Whether the page can reach the host.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    #[default]
    Up,
    Down,
}

impl LinkState {
    pub const ALL: [Self; 2] = [Self::Up, Self::Down];

    /// The serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.key() == key)
    }
}

/// Why the fixture the controls name could not be shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureLoadCause {
    /// The file could not be fetched.
    FetchFailed,
    /// The file was read and is not a valid scenario.
    Invalid,
}

impl FixtureLoadCause {
    pub const ALL: [Self; 2] = [Self::FetchFailed, Self::Invalid];

    /// The serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::FetchFailed => "fetch_failed",
            Self::Invalid => "invalid",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }
}
