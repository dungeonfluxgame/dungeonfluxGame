//! The roll card's tray: how the dice of a roll are laid on it. A few dice stand in a grid of
//! cells sized to the tray, each drawn whole with its face; a crowd of dice (more than
//! `MOST_DICE_DRAWN`) is a small cluster of dice that tumble and a tally of the faces, grouped by
//! value with their counts, so the faces stay large enough to read across a room.
//!
//! The layout is worked out here as plain functions native tests cover; `kit::draw::roll_card`
//! makes the elements. It never changes a face and never drops one: the sum under the tray
//! carries every die.

use crate::kit::roll_card::{DieArt, DieSlot, DieState};

/// The tray's inner width the dice are laid in, in tenths of a cqw.
const TRAY_INNER_WIDTH_TENTHS: u32 = 264;
/// The tray's inner height for one die, which has the tray to itself: squarer than a tray for
/// a handful, so its numeral reads across a room.
const TRAY_INNER_HEIGHT_ONE_TENTHS: u32 = 140;
/// The tray's inner height for a pair.
const TRAY_INNER_HEIGHT_TWO_TENTHS: u32 = 115;
/// The tray's inner height for three dice or more.
const TRAY_INNER_HEIGHT_MANY_TENTHS: u32 = 95;
/// What the tray is taller than the dice's inner height: the rim and its padding, in tenths.
const TRAY_RIM_TENTHS: u32 = 20;
/// The most the cells of a row are set apart, in tenths of a cqw: the slack of a pair is shared
/// out as a gap, so a die that is left out and drawn smaller does not lie under the one that
/// counts, and the pair still sits inside the tray's rim.
const CELL_GAP_MOST_TENTHS: u32 = 30;
/// The room under the dice for a caption ("tens", "ones"), in tenths of a cqw.
const CAPTION_ROOM_TENTHS: u32 = 22;
/// How much wider a die's picture is drawn than the cell it stands in, in percent: the art leaves
/// a wide margin round the die.
const PICTURE_OVER_CELL_PERCENT: u32 = 125;
/// The same for one die or two, whose cells are as tall as the tray: the margin is clipped by
/// the tray and the die stands as large as the tray allows.
const PICTURE_OVER_CELL_FEW_PERCENT: u32 = 140;
/// How large a numeral is drawn against its die's picture, in percent, for one die or two. It is
/// the same for every kind of die, so a d10 reads as large as a d20.
const NUMERAL_OF_PICTURE_PERCENT: u32 = 27;
/// The same for three dice or more, whose pictures are small: the halo keeps a numeral that runs
/// onto the enamel readable.
const NUMERAL_OF_SMALL_PICTURE_PERCENT: u32 = 38;
/// How much of its size a numeral of two digits keeps, in percent, for three dice or more, whose
/// pictures are small: one die or two have the room for two digits at the size of one.
const WIDE_NUMERAL_FIT_SMALL_PERCENT: u32 = 85;
/// The most dice that are drawn larger.
const DICE_DRAWN_LARGER: usize = 2;

/// More dice than this are not drawn one by one: they are a cluster and a tally.
pub const MOST_DICE_DRAWN: usize = 4;
/// The dice of the cluster that tumbles beside a tally.
const CLUSTER_DICE: usize = 3;
/// The width of the cluster beside a tally, in tenths of a cqw.
const CLUSTER_WIDTH_TENTHS: u32 = 78;
/// The most rows of chips a tally has.
const TALLY_ROWS: u32 = 3;
/// The most chips a tally shows; the rest are counted in one more chip.
const MOST_TALLY_CHIPS: usize = 17;

/// How the dice are laid on the tray: the rows and columns that give the largest cells, so every
/// die is drawn whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrayGrid {
    pub columns: u32,
    pub rows: u32,
    /// The side of a cell, in tenths of a cqw.
    pub cell_tenths: u32,
    /// The height the cells are laid in, in tenths of a cqw.
    pub inner_height_tenths: u32,
    /// How much wider the die's picture is than its cell, in percent.
    pub picture_percent: u32,
    /// How large a numeral is against the picture, in percent.
    pub numeral_percent: u32,
    /// How much of its size a numeral of two digits keeps, in percent.
    pub wide_fit_percent: u32,
    /// The room under the dice for captions, in tenths of a cqw; none when no die has one.
    pub caption_tenths: u32,
    /// How much of its random resting offset a die keeps, in percent: a pair or a handful
    /// scatters less, so no die rests on another or on the rim.
    pub scatter_percent: u32,
}

impl TrayGrid {
    /// The grid for `slots` dice: of every number of rows, the one whose cells are largest (the
    /// fewest rows among equals).
    pub fn of(slots: usize) -> Self {
        let count = u32::try_from(slots.max(1)).unwrap_or(u32::MAX);
        let inner_height_tenths = match slots {
            0..=1 => TRAY_INNER_HEIGHT_ONE_TENTHS,
            2 => TRAY_INNER_HEIGHT_TWO_TENTHS,
            _ => TRAY_INNER_HEIGHT_MANY_TENTHS,
        };
        let (mut best_rows, mut best_columns, mut best_cell) = (1, count, 0);
        for rows in 1..=count {
            let columns = count.div_ceil(rows);
            let cell = (TRAY_INNER_WIDTH_TENTHS / columns).min(inner_height_tenths / rows);
            if cell > best_cell {
                (best_rows, best_columns, best_cell) = (rows, columns, cell);
            }
        }
        Self {
            columns: best_columns,
            rows: best_rows,
            cell_tenths: best_cell.max(1),
            inner_height_tenths,
            picture_percent: if slots <= DICE_DRAWN_LARGER {
                PICTURE_OVER_CELL_FEW_PERCENT
            } else {
                PICTURE_OVER_CELL_PERCENT
            },
            numeral_percent: if slots <= DICE_DRAWN_LARGER {
                NUMERAL_OF_PICTURE_PERCENT
            } else {
                NUMERAL_OF_SMALL_PICTURE_PERCENT
            },
            wide_fit_percent: if slots <= DICE_DRAWN_LARGER {
                100
            } else {
                WIDE_NUMERAL_FIT_SMALL_PERCENT
            },
            caption_tenths: 0,
            scatter_percent: match slots {
                0..=2 => 45,
                _ => 30,
            },
        }
    }

    /// The same grid with room under the dice for their captions.
    fn with_captions(self) -> Self {
        Self {
            caption_tenths: CAPTION_ROOM_TENTHS,
            ..self
        }
    }

    /// The gap between the cells of a row, in tenths of a cqw: what the row leaves of the tray's
    /// width, up to `CELL_GAP_MOST_TENTHS`. Three dice or more fill the row and have none.
    pub fn cell_gap_tenths(self) -> u32 {
        if self.columns < 2 {
            return 0;
        }
        let slack = TRAY_INNER_WIDTH_TENTHS.saturating_sub(self.columns * self.cell_tenths);
        (slack / (self.columns - 1)).min(CELL_GAP_MOST_TENTHS)
    }

    /// The side of a die's picture, in tenths of a cqw.
    pub fn picture_tenths(self) -> u32 {
        self.cell_tenths * self.picture_percent / 100
    }

    /// The size of a numeral, in tenths of a cqw.
    pub fn numeral_tenths(self) -> u32 {
        self.picture_tenths() * self.numeral_percent / 100
    }

    /// The tray's own height, in tenths of a cqw.
    pub fn tray_height_tenths(self) -> u32 {
        self.inner_height_tenths + TRAY_RIM_TENTHS + self.caption_tenths
    }

    /// The custom property of the tray itself.
    pub fn tray_vars(self) -> String {
        format!(
            "--tray-h:{}cqw;",
            f64::from(self.tray_height_tenths()) / 10.0
        )
    }

    /// The custom properties of the dice's container.
    pub fn vars(self) -> String {
        format!(
            concat!(
                "--cols:{};--cell:{}cqw;--cell-gap:{}cqw;--roll-die:{}cqw;--numeral:{}cqw;",
                "--wide-fit:{};",
                "--scatter:{};--caption-room:{}cqw;",
            ),
            self.columns,
            f64::from(self.cell_tenths) / 10.0,
            f64::from(self.cell_gap_tenths()) / 10.0,
            f64::from(self.picture_tenths()) / 10.0,
            f64::from(self.numeral_tenths()) / 10.0,
            f64::from(self.wide_fit_percent) / 100.0,
            f64::from(self.scatter_percent) / 100.0,
            f64::from(self.caption_tenths) / 10.0,
        )
    }
}

/// One chip of a tally: a face and how many dice showed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TallyChip {
    /// The face, or "+N" for the dice of the faces that did not fit.
    pub face: String,
    /// How many dice showed it; one is not written.
    pub count: u32,
    /// Whether the dice were left out of the sum.
    pub state: DieState,
}

/// The faces of a crowd of dice, grouped by value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tally {
    /// The chips, the largest face first.
    pub chips: Vec<TallyChip>,
    pub columns: u32,
    pub rows: u32,
    /// The pictures of the dice that tumble beside it.
    pub cluster: Vec<DieArt>,
}

impl Tally {
    /// The tally of `slots`: the dice that count, grouped by face with the largest face first,
    /// then the dice that were left out, each by face. A face past `MOST_TALLY_CHIPS` chips is
    /// counted into the last chip as "+N" dice.
    pub fn of(slots: &[DieSlot]) -> Self {
        let mut counted: Vec<(String, u32)> = Vec::new();
        let mut left_out: Vec<(String, u32)> = Vec::new();
        for slot in slots {
            let group = match slot.state {
                DieState::Counted => &mut counted,
                DieState::Unused | DieState::Replaced => &mut left_out,
            };
            match group.iter_mut().find(|(face, _)| *face == slot.numeral) {
                Some((_, count)) => *count += 1,
                None => group.push((slot.numeral.clone(), 1)),
            }
        }
        for group in [&mut counted, &mut left_out] {
            group.sort_by_key(|(face, _)| std::cmp::Reverse(face.parse::<u32>().unwrap_or(0)));
        }
        let mut chips: Vec<TallyChip> = counted
            .into_iter()
            .map(|(face, count)| TallyChip {
                face,
                count,
                state: DieState::Counted,
            })
            .chain(left_out.into_iter().map(|(face, count)| TallyChip {
                face,
                count,
                state: DieState::Unused,
            }))
            .collect();
        if chips.len() > MOST_TALLY_CHIPS {
            let hidden: u32 = chips[MOST_TALLY_CHIPS..]
                .iter()
                .map(|chip| chip.count)
                .sum();
            chips.truncate(MOST_TALLY_CHIPS);
            chips.push(TallyChip {
                face: format!("+{hidden}"),
                count: 1,
                state: DieState::Counted,
            });
        }
        let count = u32::try_from(chips.len()).unwrap_or(u32::MAX);
        let columns = count.div_ceil(TALLY_ROWS).clamp(3, 6);
        let rows = count.div_ceil(columns).max(1);
        let mut cluster = Vec::new();
        for slot in slots {
            if cluster.len() < CLUSTER_DICE && !cluster.contains(&slot.art) {
                cluster.push(slot.art);
            }
        }
        while cluster.len() < CLUSTER_DICE.min(slots.len()) {
            cluster.push(slots[cluster.len() % slots.len()].art);
        }
        Self {
            chips,
            columns,
            rows,
            cluster,
        }
    }

    /// The custom properties of the chips' container.
    pub fn vars(&self) -> String {
        format!(
            "--chip-cols:{};--chip-rows:{};--cluster:{}cqw;--scatter:0.4;",
            self.columns,
            self.rows,
            f64::from(CLUSTER_WIDTH_TENTHS) / 10.0,
        )
    }
}

/// How the dice lie on the tray.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayLayout {
    /// Each die, whole, with its face.
    Dice(TrayGrid),
    /// A cluster of dice and the faces counted.
    Tally(Tally),
}

impl TrayLayout {
    /// The layout for `slots`.
    pub fn of(slots: &[DieSlot]) -> Self {
        if slots.len() > MOST_DICE_DRAWN {
            Self::Tally(Tally::of(slots))
        } else if slots.iter().any(|slot| slot.caption.is_some()) {
            Self::Dice(TrayGrid::of(slots.len()).with_captions())
        } else {
            Self::Dice(TrayGrid::of(slots.len()))
        }
    }

    /// The side of a cluster die's picture, in tenths of a cqw.
    pub fn cluster_picture_tenths() -> u32 {
        CLUSTER_WIDTH_TENTHS * 9 / 10
    }
}
