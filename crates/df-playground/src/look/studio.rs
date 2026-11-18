//! The phone's look studio page (`planning/phone-and-creation.md` P3, board 6 of the phone kit):
//! the presentation state, the taps and the screen made of the kit's blocks.
//!
//! It holds only what the phone owns: the draft (REF-049), the start form's picks and the last
//! sentence the host refused with. Everything else is the host's latest answer, and the refine
//! machine (`editing` > `refining` > `comparing` > `kept` | `discarded`, or `failed` keeping the
//! old picture) is read from it, never decided here. A tap is an action string (`LookTap`); an
//! action a state does not take is ignored.

use crate::look::wire::{
    ControlKind, ControlValue, LockState, LookControl, LookControls, LookDetails, LookDraft,
    LookStartOptions, LookStudioView, LookVersion, StartCard, VersionState,
};
use crate::overlay::Glyph;
use crate::phone_kit::page::{
    Block, Chip, PickCard, SliderRange, StageState, StripItem, Swatch, bar_share,
};
use crate::phone_kit::shell::{ButtonKind, ButtonSpec, PillTone};

/// The prefix of every look studio action.
const PREFIX: &str = "look:";
/// The id of the details box, read when Read details is tapped.
pub const WORDS_FIELD_ID: &str = "ph-look-words";
/// The id of the new hero's name field, read when Start is tapped.
pub const NAME_FIELD_ID: &str = "ph-look-name";

/// A tap of the look studio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookTap {
    /// A swatch or a chip: the control and the option's id.
    Choose(String, String),
    /// A slider moved: the control and its new value.
    Slide(String, u32),
    RemoveDetail(usize),
    ReadDetails,
    Refine,
    /// Keep a version: the new one, or one of the strip.
    Keep(u64),
    /// Keep the old version over this new one.
    KeepOld(u64),
    StartAgain,
    /// A random look from the host, to adjust before a picture is made.
    Roll,
    LockIn,
    Species(String),
    Class(String),
    Start,
    /// Open a picture of the studio full screen.
    View(String),
    ViewNext,
    ViewPrevious,
    ViewClose,
    /// Back to the player's heroes.
    Done,
    /// Play the locked-in hero at the seat: to the lobby in one tap.
    TakeSeat(u64),
}

impl LookTap {
    /// The action string of the tap.
    pub fn action(&self) -> String {
        let rest = match self {
            Self::Choose(control, option) => format!("set:{control}:{option}"),
            Self::Slide(control, value) => format!("slider:{control}:{value}"),
            Self::RemoveDetail(index) => format!("remove:{index}"),
            Self::ReadDetails => "details".to_owned(),
            Self::Refine => "refine".to_owned(),
            Self::Keep(version) => format!("keep:{version}"),
            Self::KeepOld(version) => format!("keep-old:{version}"),
            Self::StartAgain => "again".to_owned(),
            Self::Roll => "roll".to_owned(),
            Self::LockIn => "lock".to_owned(),
            Self::Species(id) => format!("species:{id}"),
            Self::Class(id) => format!("class:{id}"),
            Self::Start => "start".to_owned(),
            Self::Done => "done".to_owned(),
            Self::TakeSeat(hero) => format!("take-seat:{hero}"),
            Self::View(picture) => format!("view:{picture}"),
            Self::ViewNext => "view-next".to_owned(),
            Self::ViewPrevious => "view-previous".to_owned(),
            Self::ViewClose => "view-close".to_owned(),
        };
        format!("{PREFIX}{rest}")
    }

    /// The tap an action string names, or `None` when it is not the look studio's.
    pub fn from_action(action: &str) -> Option<Self> {
        let rest = action.strip_prefix(PREFIX)?;
        let (verb, tail) = rest.split_once(':').unwrap_or((rest, ""));
        match (verb, tail) {
            ("set", tail) => {
                let (control, option) = tail.split_once(':')?;
                Some(Self::Choose(control.to_owned(), option.to_owned()))
            }
            ("slider", tail) => {
                let (control, value) = tail.rsplit_once(':')?;
                Some(Self::Slide(control.to_owned(), value.parse().ok()?))
            }
            ("remove", index) => index.parse().ok().map(Self::RemoveDetail),
            ("details", "") => Some(Self::ReadDetails),
            ("refine", "") => Some(Self::Refine),
            ("keep", version) => version.parse().ok().map(Self::Keep),
            ("keep-old", version) => version.parse().ok().map(Self::KeepOld),
            ("again", "") => Some(Self::StartAgain),
            ("roll", "") => Some(Self::Roll),
            ("lock", "") => Some(Self::LockIn),
            ("species", id) if !id.is_empty() => Some(Self::Species(id.to_owned())),
            ("class", id) if !id.is_empty() => Some(Self::Class(id.to_owned())),
            ("start", "") => Some(Self::Start),
            ("done", "") => Some(Self::Done),
            ("take-seat", hero) => hero.parse().ok().map(Self::TakeSeat),
            ("view", picture) if !picture.is_empty() => Some(Self::View(picture.to_owned())),
            ("view-next", "") => Some(Self::ViewNext),
            ("view-previous", "") => Some(Self::ViewPrevious),
            ("view-close", "") => Some(Self::ViewClose),
            _ => None,
        }
    }
}

/// Where the look studio stands, read from the host's latest answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookPhase {
    /// Waiting for the host's first answers.
    Loading,
    /// A new hero is started: name, species, class.
    Start,
    Editing,
    /// A picture is being made (version `0`).
    Refining(u64),
    /// A new version shows beside the kept one.
    Comparing(u64),
    /// The look is being locked in.
    Locking,
    Locked,
}

/// The phone's look studio page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LookPage {
    /// The host's studio of the seat.
    pub studio: Option<LookStudioView>,
    /// The controls of the seat's hero, once it has one with a look in the making.
    pub controls: Option<LookControls>,
    /// What a new hero may be started as, when one is being started.
    pub options: Option<LookStartOptions>,
    pub draft: LookDraft,
    pub species: Option<String>,
    pub class: Option<String>,
    /// The host's last refusal, or why an ask failed.
    pub notice: Option<String>,
    /// A refine, details reading or lock-in sent and not answered yet.
    pub sending: bool,
    /// The new hero's name as typed, so Start knows whether one is given.
    pub name: String,
    /// A new roll over a kept look: the next picture is a new person from the base pose.
    pub fresh: bool,
    /// The picture open full screen, by its name in the media library.
    pub viewing: Option<String>,
}

/// The number the player knows version `id` by: its place among this hero's versions, from 1.
/// The host's ids run on across a seat's heroes.
fn number_of(studio: &LookStudioView, id: u64) -> usize {
    studio
        .versions
        .iter()
        .position(|version| version.id == id)
        .map_or(0, |index| index + 1)
}

fn newest(studio: &LookStudioView) -> Option<&LookVersion> {
    studio.versions.iter().max_by_key(|version| version.id)
}

fn version(studio: &LookStudioView, id: Option<u64>) -> Option<&LookVersion> {
    let id = id?;
    studio.versions.iter().find(|version| version.id == id)
}

/// The control value `draft` holds, or the control's default.
fn value_of<'page>(draft: &'page LookDraft, control: &'page LookControl) -> &'page ControlValue {
    draft.values.get(&control.id).unwrap_or(&control.default)
}

fn button(label: &str, kind: ButtonKind, tap: Option<&LookTap>) -> ButtonSpec {
    ButtonSpec {
        label: label.to_owned(),
        kind,
        enabled: tap.is_some(),
        reason: None,
        action: tap.map(LookTap::action),
    }
}

/// One control as the kit's block: a slider, swatches or chips.
fn control_block(draft: &LookDraft, control: &LookControl) -> Block {
    let value = value_of(draft, control);
    match &control.kind {
        ControlKind::Slider {
            min,
            max,
            unit,
            words,
        } => {
            let at = match value {
                ControlValue::Number(number) => (*number).clamp(*min, *max),
                ControlValue::Choice(_) => *min,
            };
            let span = max.saturating_sub(*min).max(1);
            let percent = u8::try_from(at.saturating_sub(*min) * 100 / span).unwrap_or(100);
            let word = |number: u32| -> String {
                usize::try_from(number.saturating_sub(*min))
                    .ok()
                    .and_then(|index| words.get(index))
                    .cloned()
                    .unwrap_or_else(|| match unit.as_deref() {
                        Some("in") => format!("{} ft {} in", number / 12, number % 12),
                        Some(unit) => format!("{number} {unit}"),
                        None => number.to_string(),
                    })
            };
            Block::Slider {
                label: format!("{}: {}", control.label, word(at)),
                percent: bar_share(percent),
                low: word(*min),
                high: word(*max),
                range: Some(SliderRange {
                    action: format!("{PREFIX}slider:{}", control.id),
                    min: *min,
                    max: *max,
                    value: at,
                }),
            }
        }
        ControlKind::Swatch { options } => Block::Swatches {
            label: control.label.clone(),
            swatches: options
                .iter()
                .map(|option| Swatch {
                    name: option.label.clone(),
                    color: option.colour.clone().unwrap_or_default(),
                    selected: *value == ControlValue::Choice(option.id.clone()),
                    action: Some(LookTap::Choose(control.id.clone(), option.id.clone()).action()),
                })
                .collect(),
        },
        ControlKind::Chip { options } => Block::Chips {
            label: control.label.clone(),
            chips: options
                .iter()
                .map(|option| Chip {
                    text: option.label.clone(),
                    selected: *value == ControlValue::Choice(option.id.clone()),
                    removable: false,
                    action: Some(LookTap::Choose(control.id.clone(), option.id.clone()).action()),
                })
                .collect(),
        },
    }
}

/// What the look studio page shows: its blocks, in order, and the buttons of its dock.
#[derive(Debug, Clone, PartialEq)]
pub struct LookScreen {
    pub heading: String,
    pub blocks: Vec<Block>,
    pub buttons: Vec<ButtonSpec>,
    /// The picture open full screen over the page, when one is.
    pub viewer: Option<LookViewer>,
}

/// One picture the full-screen viewer steps through: what it is, and what can be done with it.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewItem {
    pub picture: String,
    pub label: String,
    pub buttons: Vec<ButtonSpec>,
}

/// The full-screen viewer: the picture open, where it is among the pictures, and its taps.
#[derive(Debug, Clone, PartialEq)]
pub struct LookViewer {
    pub item: ViewItem,
    pub index: usize,
    pub count: usize,
    /// The taps that step to the picture before and after it; none at either end.
    pub previous: Option<String>,
    pub next: Option<String>,
    /// The tap that closes the viewer.
    pub close: String,
}

impl LookPage {
    /// Where the studio stands.
    pub fn phase(&self) -> LookPhase {
        let Some(studio) = &self.studio else {
            return LookPhase::Loading;
        };
        if self.options.is_some() && self.controls.is_none() {
            return LookPhase::Start;
        }
        match studio.lock.as_ref().map(|lock| &lock.state) {
            Some(LockState::Locked { .. }) => return LookPhase::Locked,
            Some(LockState::Portrait | LockState::Sheet { .. }) => return LookPhase::Locking,
            None => {}
        }
        if self.controls.is_none() {
            return LookPhase::Loading;
        }
        match newest(studio) {
            Some(fresh) if fresh.state == VersionState::Pending => LookPhase::Refining(fresh.id),
            Some(fresh) if fresh.state == VersionState::Ready && studio.kept != Some(fresh.id) => {
                LookPhase::Comparing(fresh.id)
            }
            _ => LookPhase::Editing,
        }
    }

    /// Takes the host's reading of the player's words into the draft: each control the words set
    /// moves to its value on screen, as a roll moves them (only a control this hero is shown), and
    /// each detail not already there becomes a chip.
    pub fn apply_details(&mut self, read: LookDetails) {
        let shown = |id: &str| {
            self.controls
                .as_ref()
                .is_some_and(|controls| controls.controls.iter().any(|control| control.id == id))
        };
        let values: Vec<(String, ControlValue)> = read
            .values
            .into_iter()
            .filter(|(id, _)| shown(id))
            .collect();
        self.draft.values.extend(values);
        for detail in read.details {
            if !self.draft.details.contains(&detail) {
                self.draft.details.push(detail);
            }
        }
    }

    /// Applies a tap that changes only the phone's own state (the draft, the start picks).
    /// Answers whether it did; a tap that asks the host is the caller's.
    pub fn apply_local(&mut self, tap: &LookTap) -> bool {
        match tap {
            LookTap::Choose(control, option) => {
                self.draft
                    .values
                    .insert(control.clone(), ControlValue::Choice(option.clone()));
            }
            LookTap::Slide(control, value) => {
                self.draft
                    .values
                    .insert(control.clone(), ControlValue::Number(*value));
            }
            LookTap::RemoveDetail(index) => {
                if *index >= self.draft.details.len() {
                    return false;
                }
                self.draft.details.remove(*index);
            }
            LookTap::Species(id) => self.species = Some(id.clone()),
            LookTap::View(picture) => self.viewing = Some(picture.clone()),
            LookTap::ViewNext | LookTap::ViewPrevious => {
                let step = if *tap == LookTap::ViewNext { 1 } else { -1 };
                let Some(picture) = self.stepped(step) else {
                    return false;
                };
                self.viewing = Some(picture);
            }
            LookTap::ViewClose => self.viewing = None,
            LookTap::Class(id) => self.class = Some(id.clone()),
            _ => return false,
        }
        true
    }

    /// The cards of a start form row: each example picture (a glyph without one), its label and
    /// its line; the picked one lit.
    fn start_cards(
        cards: &[StartCard],
        picked: Option<&str>,
        glyph: Glyph,
        tap: fn(String) -> LookTap,
    ) -> Block {
        Block::Cards {
            cards: cards
                .iter()
                .map(|card| PickCard {
                    title: card.label.clone(),
                    sub: Some(card.line.clone()),
                    picture: card.picture.clone(),
                    glyph: Some(glyph),
                    mark: None,
                    rarity: None,
                    selected: picked == Some(card.id.as_str()),
                    off: false,
                    action: Some(tap(card.id.clone()).action()),
                    crop: None,
                })
                .collect(),
            small: false,
        }
    }

    /// What the start form still needs, said plainly; none when it can start.
    pub fn start_missing(&self) -> Option<String> {
        let missing: Vec<&str> = [
            (self.name.trim().is_empty(), "a name"),
            (self.species.is_none(), "a species"),
            (self.class.is_none(), "a class"),
        ]
        .into_iter()
        .filter_map(|(lacking, what)| lacking.then_some(what))
        .collect();
        let list = match missing.as_slice() {
            [] => return None,
            [one] => (*one).to_owned(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        };
        Some(format!("Still to choose: {list}."))
    }

    fn start_screen(&self, options: &LookStartOptions) -> LookScreen {
        let missing = self.start_missing();
        let ready = missing.is_none() && !self.sending;
        let mut blocks = vec![Block::Field {
            label: "Name".to_owned(),
            value: String::new(),
            placeholder: "Your hero's name".to_owned(),
            message: None,
            tall: false,
            id: Some(NAME_FIELD_ID.to_owned()),
        }];
        blocks.extend(self.notice_block());
        blocks.extend([
            Block::Caption {
                text: "Species".to_owned(),
            },
            Self::start_cards(
                &options.species,
                self.species.as_deref(),
                Glyph::Crown,
                LookTap::Species,
            ),
            Block::Caption {
                text: "Class".to_owned(),
            },
            Self::start_cards(
                &options.classes,
                self.class.as_deref(),
                Glyph::Sword,
                LookTap::Class,
            ),
        ]);
        let mut start = button(
            "Start my look",
            ButtonKind::Primary,
            ready.then_some(&LookTap::Start),
        );
        start.reason = missing;
        LookScreen {
            viewer: None,
            heading: "A new hero".to_owned(),
            blocks,
            buttons: vec![
                start,
                button("Back", ButtonKind::Secondary, Some(&LookTap::Done)),
            ],
        }
    }

    /// The host's last refusal, or why an ask failed, as a warning.
    fn notice_block(&self) -> Option<Block> {
        self.notice.as_ref().map(|text| Block::Notice {
            tone: PillTone::Warn,
            text: text.clone(),
        })
    }

    /// The stage, or the new version beside the kept one.
    fn stage(&self, studio: &LookStudioView, phase: LookPhase) -> Block {
        let kept = version(studio, studio.kept).and_then(|kept| kept.picture.clone());
        if let LookPhase::Comparing(fresh) = phase {
            let fresh = version(studio, Some(fresh)).and_then(|fresh| fresh.picture.clone());
            if let (Some(kept), Some(fresh)) = (kept.clone(), fresh.clone()) {
                let actions = [
                    LookTap::View(kept.clone()).action(),
                    LookTap::View(fresh.clone()).action(),
                ];
                return Block::Compare {
                    kept,
                    fresh,
                    caption: "Keep the new look, or the one you had? Tap one to see it whole."
                        .to_owned(),
                    actions: Some(actions),
                };
            }
            return Block::Stage {
                action: fresh.clone().map(|fresh| LookTap::View(fresh).action()),
                picture: fresh,
                state: StageState::Ready,
                caption: "Your first look. Keep it, or try again?".to_owned(),
            };
        }
        let failed =
            newest(studio).is_some_and(|fresh| matches!(fresh.state, VersionState::Failed { .. }));
        let (state, caption) = match phase {
            LookPhase::Refining(_) => (StageState::Generating, "Making your look..."),
            _ if failed => (
                StageState::Failed,
                "That picture could not be made; your look is as it was.",
            ),
            _ if kept.is_none() => (
                StageState::Ready,
                "Set anything you like, or roll a hero; what you leave is rolled for you.",
            ),
            _ if self.fresh => (
                StageState::Ready,
                "A new roll: See my hero makes a new person.",
            ),
            _ => (StageState::Ready, "Change anything, then Show the changes."),
        };
        Block::Stage {
            action: kept.clone().map(|kept| LookTap::View(kept).action()),
            picture: kept,
            state,
            caption: caption.to_owned(),
        }
    }

    fn strip(studio: &LookStudioView) -> Option<Block> {
        let items: Vec<StripItem> = studio
            .versions
            .iter()
            .filter(|version| version.picture.is_some())
            .map(|version| StripItem {
                label: if version.state == VersionState::Stale {
                    format!("{} (old)", number_of(studio, version.id))
                } else {
                    number_of(studio, version.id).to_string()
                },
                picture: version.picture.clone(),
                selected: studio.kept == Some(version.id),
                action: version
                    .picture
                    .clone()
                    .map(|picture| LookTap::View(picture).action()),
            })
            .collect();
        (items.len() > 1).then(|| Block::Strip {
            label: "Earlier looks".to_owned(),
            items,
        })
    }

    fn details_blocks(&self) -> Vec<Block> {
        let mut blocks = vec![Block::Field {
            label: "Details in your words".to_owned(),
            value: String::new(),
            placeholder: "A scar across the left cheek, three silver earrings".to_owned(),
            message: None,
            tall: true,
            id: Some(WORDS_FIELD_ID.to_owned()),
        }];
        if !self.draft.details.is_empty() {
            blocks.push(Block::Chips {
                label: "Details".to_owned(),
                chips: self
                    .draft
                    .details
                    .iter()
                    .enumerate()
                    .map(|(index, detail)| Chip {
                        text: detail.what.clone(),
                        selected: true,
                        removable: true,
                        action: Some(LookTap::RemoveDetail(index).action()),
                    })
                    .collect(),
            });
        }
        blocks.push(Block::Buttons {
            buttons: vec![button(
                "Read details",
                ButtonKind::Secondary,
                (!self.sending).then_some(&LookTap::ReadDetails),
            )],
        });
        blocks
    }

    fn studio_screen(
        &self,
        studio: &LookStudioView,
        controls: &LookControls,
        phase: LookPhase,
    ) -> LookScreen {
        let mut blocks = vec![self.stage(studio, phase)];
        blocks.extend(self.notice_block());
        blocks.extend(Self::strip(studio));
        blocks.extend(
            controls
                .controls
                .iter()
                .map(|control| control_block(&self.draft, control)),
        );
        blocks.extend(self.details_blocks());
        let free = !self.sending;
        let buttons = match phase {
            LookPhase::Comparing(fresh) => vec![
                button(
                    "Keep it",
                    ButtonKind::Primary,
                    free.then_some(&LookTap::Keep(fresh)),
                ),
                button(
                    if studio.kept.is_some() {
                        "Keep the old one"
                    } else {
                        "Try again"
                    },
                    ButtonKind::Secondary,
                    free.then_some(&LookTap::KeepOld(fresh)),
                ),
            ],
            LookPhase::Refining(_) => {
                vec![button("Making your look...", ButtonKind::Primary, None)]
            }
            _ => {
                let anew = studio.kept.is_none() || self.fresh;
                let mut lock = button(
                    "Lock in",
                    ButtonKind::Secondary,
                    (free && studio.kept.is_some()).then_some(&LookTap::LockIn),
                );
                lock.reason = studio
                    .kept
                    .is_none()
                    .then(|| "Keep a picture of your hero first.".to_owned());
                vec![
                    button(
                        if anew {
                            "See my hero"
                        } else {
                            "Show the changes"
                        },
                        ButtonKind::Primary,
                        free.then_some(&LookTap::Refine),
                    ),
                    button(
                        if studio.kept.is_none() {
                            "Roll a hero"
                        } else {
                            "Roll new"
                        },
                        ButtonKind::Secondary,
                        free.then_some(&LookTap::Roll),
                    ),
                    lock,
                ]
            }
        };
        LookScreen {
            viewer: None,
            heading: "Your look".to_owned(),
            blocks,
            buttons,
        }
    }

    fn locked_screen(&self, studio: &LookStudioView, phase: LookPhase) -> LookScreen {
        let (picture, caption, ready) = match studio.lock.as_ref().map(|lock| &lock.state) {
            Some(LockState::Locked { portrait, sheet }) => (
                portrait.clone(),
                if sheet.is_some() {
                    "Ready. Your hero is locked in."
                } else {
                    "Ready. Your hero is locked in; the reference sheet comes later."
                },
                true,
            ),
            Some(LockState::Sheet { portrait }) => {
                (portrait.clone(), "Locking in your hero...", false)
            }
            _ => (None, "Making your portrait...", false),
        };
        let sheet = match studio.lock.as_ref().map(|lock| &lock.state) {
            Some(LockState::Locked {
                sheet: Some(sheet), ..
            }) => Some(sheet.clone()),
            _ => None,
        };
        let pair = sheet.map(|sheet| Block::Strip {
            label: "Your hero, to open full screen".to_owned(),
            items: [
                (picture.clone(), "Portrait"),
                (Some(sheet), "Reference sheet"),
            ]
            .into_iter()
            .filter_map(|(picture, label)| {
                let picture = picture?;
                Some(StripItem {
                    label: label.to_owned(),
                    action: Some(LookTap::View(picture.clone()).action()),
                    picture: Some(picture),
                    selected: false,
                })
            })
            .collect(),
        });
        LookScreen {
            viewer: None,
            heading: "Your hero".to_owned(),
            blocks: [Block::Stage {
                action: picture
                    .clone()
                    .map(|picture| LookTap::View(picture).action()),
                picture,
                state: if phase == LookPhase::Locked {
                    StageState::Ready
                } else {
                    StageState::Generating
                },
                caption: caption.to_owned(),
            }]
            .into_iter()
            .chain(pair)
            .chain(self.notice_block())
            .collect(),
            buttons: match studio.lock.as_ref() {
                Some(lock) if ready => vec![
                    button(
                        "Take my seat",
                        ButtonKind::Primary,
                        Some(&LookTap::TakeSeat(lock.hero)),
                    ),
                    button(
                        "Back to my heroes",
                        ButtonKind::Secondary,
                        Some(&LookTap::Done),
                    ),
                ],
                _ => vec![button("Back to my heroes", ButtonKind::Primary, None)],
            },
        }
    }

    /// Every picture of the studio the viewer steps through, oldest first: each version with a
    /// picture, then the hero's reference sheet once locked in.
    pub fn gallery(&self) -> Vec<ViewItem> {
        let Some(studio) = &self.studio else {
            return Vec::new();
        };
        let phase = self.phase();
        let locked = matches!(phase, LookPhase::Locking | LookPhase::Locked);
        let free = !self.sending;
        let mut items: Vec<ViewItem> = studio
            .versions
            .iter()
            .filter_map(|version| {
                let picture = version.picture.clone()?;
                let kept = studio.kept == Some(version.id);
                let fresh = phase == LookPhase::Comparing(version.id);
                let label = match () {
                    () if kept && locked => "Your portrait".to_owned(),
                    () if kept => format!("Version {}, kept", number_of(studio, version.id)),
                    () if fresh => "New".to_owned(),
                    () if version.state == VersionState::Stale => {
                        format!("Version {}, old", number_of(studio, version.id))
                    }
                    () => format!("Version {}", number_of(studio, version.id)),
                };
                let buttons = if fresh {
                    vec![
                        button(
                            "Keep it",
                            ButtonKind::Primary,
                            free.then_some(&LookTap::Keep(version.id)),
                        ),
                        button(
                            if studio.kept.is_some() {
                                "Keep the old one"
                            } else {
                                "Try again"
                            },
                            ButtonKind::Secondary,
                            free.then_some(&LookTap::KeepOld(version.id)),
                        ),
                    ]
                } else if !kept && !locked && version.state.keepable() {
                    vec![button(
                        "Go back to this look",
                        ButtonKind::Secondary,
                        free.then_some(&LookTap::Keep(version.id)),
                    )]
                } else {
                    Vec::new()
                };
                Some(ViewItem {
                    picture,
                    label,
                    buttons,
                })
            })
            .collect();
        if let Some(LockState::Locked {
            sheet: Some(sheet), ..
        }) = studio.lock.as_ref().map(|lock| &lock.state)
        {
            items.push(ViewItem {
                picture: sheet.clone(),
                label: "Reference sheet".to_owned(),
                buttons: Vec::new(),
            });
        }
        items
    }

    /// The viewer over the page, when a picture of the gallery is open.
    fn viewer(&self) -> Option<LookViewer> {
        let picture = self.viewing.as_deref()?;
        let mut gallery = self.gallery();
        let index = gallery.iter().position(|item| item.picture == picture)?;
        let count = gallery.len();
        let step = |at: Option<usize>| {
            at.and_then(|at| gallery.get(at))
                .map(|item| LookTap::View(item.picture.clone()).action())
        };
        let previous = step(index.checked_sub(1));
        let next = step(index.checked_add(1));
        Some(LookViewer {
            close: LookTap::ViewClose.action(),
            item: gallery.swap_remove(index),
            index,
            count,
            previous,
            next,
        })
    }

    /// The picture `step` places from the one open (`-1` before, `1` after), if there is one.
    fn stepped(&self, step: isize) -> Option<String> {
        let gallery = self.gallery();
        let open = self.viewing.as_deref()?;
        let index = gallery.iter().position(|item| item.picture == open)?;
        gallery
            .get(index.checked_add_signed(step)?)
            .map(|item| item.picture.clone())
    }

    /// The screen of the page as it stands, with the viewer over it when a picture is open.
    pub fn screen(&self) -> LookScreen {
        let mut screen = self.page_screen();
        screen.viewer = self.viewer();
        screen
    }

    fn page_screen(&self) -> LookScreen {
        let phase = self.phase();
        match (phase, &self.studio, &self.controls, &self.options) {
            (LookPhase::Start, _, _, Some(options)) => self.start_screen(options),
            (LookPhase::Locking | LookPhase::Locked, Some(studio), _, _) => {
                self.locked_screen(studio, phase)
            }
            (
                LookPhase::Editing | LookPhase::Refining(_) | LookPhase::Comparing(_),
                Some(studio),
                Some(controls),
                _,
            ) => self.studio_screen(studio, controls, phase),
            _ => LookScreen {
                viewer: None,
                heading: "Your look".to_owned(),
                blocks: vec![Block::Stage {
                    picture: None,
                    state: StageState::Generating,
                    caption: "Opening the look studio...".to_owned(),
                    action: None,
                }]
                .into_iter()
                .chain(self.notice_block())
                .collect(),
                buttons: vec![button("Back", ButtonKind::Secondary, Some(&LookTap::Done))],
            },
        }
    }
}
