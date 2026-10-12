//! Level-of-detail index for streamed splat scans (`lod-meta.json`, written by `splat-transform`)
//! and the choice of which chunk ranges to draw for a battle area.
//!
//! The index lists chunk files (`<level>_<chunk>/meta.json`, level 0 finest) and an octree whose
//! leaves each own one contiguous range per level: `{file, offset, count}`. This is the same
//! structure PlayCanvas's `gsplat-octree` reads; only the selection rule is ours. Selection is
//! pure so it is tested natively: the finest level is used inside the play radius, one level
//! coarser per doubling of distance beyond it, and the whole choice is pushed coarser until it
//! fits the splat budget. A selection can be restricted to the chunks already loaded, so a scene
//! refines as chunks arrive and always has something to draw.

use serde::Deserialize;

use super::math::Vec3;

/// Table slots for contiguous ranges across all chunks in one decode plan.
pub const MAX_RANGES: usize = 1 << 15;
const MAX_LEAVES: usize = 200_000;
const MAX_DEPTH: usize = 40;
const MAX_FILES: usize = 4096;
const MAX_LEVELS: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LodError {
    Json,
    Levels,
    FileName,
    FileIndex,
    Tree,
    TooLarge,
}

/// One chunk file of the index: which level it belongs to and its folder name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkFile {
    pub level: u32,
    /// `<level>_<chunk>`, the folder holding `meta.json` and the WebP planes.
    pub folder: String,
}

/// A contiguous run of splats of one level inside one chunk file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Range {
    pub file: u32,
    pub offset: u32,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq)]
struct Leaf {
    min: Vec3,
    max: Vec3,
    /// Indexed by level; `None` where the leaf has no splats at that level.
    ranges: Vec<Option<Range>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LodIndex {
    counts: Vec<u32>,
    files: Vec<ChunkFile>,
    leaves: Vec<Leaf>,
    bound_min: Vec3,
    bound_max: Vec3,
}

#[derive(Deserialize)]
struct FileIndex {
    #[serde(rename = "lodLevels")]
    levels: usize,
    counts: Vec<u32>,
    filenames: Vec<String>,
    tree: FileNode,
}

#[derive(Deserialize)]
struct FileBound {
    min: [f32; 3],
    max: [f32; 3],
}

#[derive(Deserialize)]
struct FileRange {
    file: u32,
    offset: u32,
    count: u32,
}

#[derive(Deserialize)]
struct FileNode {
    bound: FileBound,
    #[serde(default)]
    children: Vec<FileNode>,
    #[serde(default)]
    lods: std::collections::BTreeMap<String, FileRange>,
}

fn parse_file_name(name: &str) -> Option<ChunkFile> {
    let folder = name.strip_suffix("/meta.json")?;
    let (level, chunk) = folder.split_once('_')?;
    let digits = |text: &str| {
        !text.is_empty() && text.len() <= 6 && text.bytes().all(|b| b.is_ascii_digit())
    };
    if !digits(level) || !digits(chunk) {
        return None;
    }
    Some(ChunkFile {
        level: level.parse().ok()?,
        folder: folder.to_owned(),
    })
}

impl LodIndex {
    pub fn parse(json: &str) -> Result<Self, LodError> {
        let file: FileIndex = serde_json::from_str(json).map_err(|_| LodError::Json)?;
        if file.levels == 0 || file.levels > MAX_LEVELS || file.counts.len() != file.levels {
            return Err(LodError::Levels);
        }
        if file.filenames.len() > MAX_FILES {
            return Err(LodError::TooLarge);
        }
        let files = file
            .filenames
            .iter()
            .map(|name| parse_file_name(name).ok_or(LodError::FileName))
            .collect::<Result<Vec<_>, _>>()?;
        if files.iter().any(|f| f.level as usize >= file.levels) {
            return Err(LodError::Levels);
        }
        let mut leaves = Vec::new();
        let mut stack = vec![(&file.tree, 0usize)];
        while let Some((node, depth)) = stack.pop() {
            if depth > MAX_DEPTH {
                return Err(LodError::Tree);
            }
            if node.children.is_empty() {
                if leaves.len() >= MAX_LEAVES {
                    return Err(LodError::TooLarge);
                }
                let mut ranges = vec![None; file.levels];
                for (key, range) in &node.lods {
                    let level: usize = key.parse().map_err(|_| LodError::Tree)?;
                    let slot = ranges.get_mut(level).ok_or(LodError::Levels)?;
                    let chunk = files.get(range.file as usize).ok_or(LodError::FileIndex)?;
                    if chunk.level as usize != level {
                        return Err(LodError::FileIndex);
                    }
                    if range.count > 0 {
                        *slot = Some(Range {
                            file: range.file,
                            offset: range.offset,
                            count: range.count,
                        });
                    }
                }
                let bound = &node.bound;
                leaves.push(Leaf {
                    min: Vec3::new(bound.min[0], bound.min[1], bound.min[2]),
                    max: Vec3::new(bound.max[0], bound.max[1], bound.max[2]),
                    ranges,
                });
            } else {
                // Reversed so leaves come out in file order.
                stack.extend(node.children.iter().rev().map(|child| (child, depth + 1)));
            }
        }
        let root = &file.tree.bound;
        Ok(Self {
            counts: file.counts,
            files,
            leaves,
            bound_min: Vec3::new(root.min[0], root.min[1], root.min[2]),
            bound_max: Vec3::new(root.max[0], root.max[1], root.max[2]),
        })
    }

    pub fn level_count(&self) -> usize {
        self.counts.len()
    }

    /// Splats in each whole level, finest first.
    pub fn counts(&self) -> &[u32] {
        &self.counts
    }

    pub fn files(&self) -> &[ChunkFile] {
        &self.files
    }

    pub fn leaf_count(&self) -> usize {
        self.leaves.len()
    }

    pub fn bounds(&self) -> (Vec3, Vec3) {
        (self.bound_min, self.bound_max)
    }
}

/// Where the detail should be: a point in scan space and the radius around it that gets the
/// finest level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Focus {
    pub centre: Vec3,
    pub radius: f32,
}

/// Which levels exist on disk (inclusive) and how many splats may be drawn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub finest: u32,
    pub coarsest: u32,
    pub budget: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub level: u32,
    pub range: Range,
    /// Distance from the focus to the leaf's box, in scan units: nearer chunks load first.
    pub distance: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub picks: Vec<Pick>,
    pub total: u32,
    /// How many levels coarser than the distance rule the budget forced everything.
    pub bias: u32,
}

impl Selection {
    /// The chunk files the selection reads: coarse levels first (so something recognisable draws
    /// at once), and within a level the file nearest the focus first.
    pub fn files_coarse_first(&self) -> Vec<u32> {
        let mut seen: Vec<(u32, u32, f32)> = Vec::new();
        for pick in &self.picks {
            match seen
                .iter_mut()
                .find(|(_, file, _)| *file == pick.range.file)
            {
                Some(entry) => entry.2 = entry.2.min(pick.distance),
                None => seen.push((pick.level, pick.range.file, pick.distance)),
            }
        }
        seen.sort_by(|a, b| b.0.cmp(&a.0).then(a.2.total_cmp(&b.2)).then(a.1.cmp(&b.1)));
        seen.into_iter().map(|(_, file, _)| file).collect()
    }
}

/// One chunk's share of a [`DecodePlan`]: decode `total` splats described by
/// `ranges[range_base..range_base + range_count]`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeJob {
    pub file: u32,
    pub range_base: u32,
    pub range_count: u32,
    pub total: u32,
}

/// What the GPU decodes into the record buffer: contiguous source ranges of resident chunks laid
/// out back to back. A range is `[first source splat, count, output offset, first thread of the
/// job]`, the layout the decode shader binary-searches.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DecodePlan {
    pub jobs: Vec<DecodeJob>,
    pub ranges: Vec<[u32; 4]>,
    pub total: u32,
    /// Ranges beyond the table size were left out (never happens for a real index).
    pub truncated: bool,
}

impl Selection {
    /// Groups the picks by chunk, merges neighbouring ranges and assigns output offsets.
    pub fn plan(&self, max_ranges: usize) -> DecodePlan {
        let mut by_file: std::collections::BTreeMap<u32, Vec<(u32, u32)>> = Default::default();
        for pick in &self.picks {
            by_file
                .entry(pick.range.file)
                .or_default()
                .push((pick.range.offset, pick.range.count));
        }
        let mut plan = DecodePlan::default();
        for (file, mut spans) in by_file {
            spans.sort_unstable();
            let mut merged: Vec<(u32, u32)> = Vec::with_capacity(spans.len());
            for (offset, count) in spans {
                match merged.last_mut() {
                    Some((start, length)) if *start + *length == offset => *length += count,
                    _ => merged.push((offset, count)),
                }
            }
            let range_base = plan.ranges.len() as u32;
            let mut job_total = 0u32;
            for (offset, count) in merged {
                if plan.ranges.len() >= max_ranges {
                    plan.truncated = true;
                    break;
                }
                plan.ranges
                    .push([offset, count, plan.total + job_total, job_total]);
                job_total += count;
            }
            if job_total > 0 {
                plan.jobs.push(DecodeJob {
                    file,
                    range_base,
                    range_count: plan.ranges.len() as u32 - range_base,
                    total: job_total,
                });
                plan.total += job_total;
            }
        }
        plan
    }
}

fn distance_to_box(point: Vec3, min: Vec3, max: Vec3) -> f32 {
    let dx = (min.x - point.x).max(point.x - max.x).max(0.0);
    let dy = (min.y - point.y).max(point.y - max.y).max(0.0);
    let dz = (min.z - point.z).max(point.z - max.z).max(0.0);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

impl LodIndex {
    /// Splats at the coarsest level, in all (the first thing worth showing).
    pub fn coarsest_total(&self, limits: Limits) -> u32 {
        self.counts
            .get(limits.coarsest as usize)
            .copied()
            .unwrap_or(0)
    }

    /// Chooses one range per leaf. `loaded` restricts the choice to chunk files already resident
    /// (`None`: everything counts as available). A leaf takes its wanted level, or the nearest
    /// coarser resident one, or failing that the nearest finer resident one.
    pub fn select(
        &self,
        focus: Focus,
        limits: Limits,
        loaded: Option<&dyn Fn(u32) -> bool>,
    ) -> Selection {
        let radius = focus.radius.max(1e-3);
        let span = limits.coarsest.saturating_sub(limits.finest);
        let distances: Vec<f32> = self
            .leaves
            .iter()
            .map(|leaf| distance_to_box(focus.centre, leaf.min, leaf.max))
            .collect();
        let wanted: Vec<u32> = distances
            .iter()
            .map(|distance| {
                if *distance <= radius {
                    0
                } else {
                    (distance / radius).log2().ceil().max(0.0) as u32
                }
            })
            .collect();
        let mut best = Selection::default();
        for bias in 0..=span {
            let mut picks = Vec::new();
            let mut total = 0u64;
            for (position, (leaf, step)) in self.leaves.iter().zip(&wanted).enumerate() {
                let want = (limits.finest + step + bias).min(limits.coarsest);
                if let Some(mut pick) = choose(leaf, want, limits, loaded) {
                    pick.distance = distances[position];
                    total += u64::from(pick.range.count);
                    picks.push(pick);
                }
            }
            let total = total.min(u64::from(u32::MAX)) as u32;
            best = Selection { picks, total, bias };
            if total <= limits.budget {
                break;
            }
        }
        best
    }
}

fn choose(
    leaf: &Leaf,
    want: u32,
    limits: Limits,
    loaded: Option<&dyn Fn(u32) -> bool>,
) -> Option<Pick> {
    let resident = |level: u32| {
        leaf.ranges
            .get(level as usize)
            .copied()
            .flatten()
            .filter(|range| loaded.is_none_or(|loaded| loaded(range.file)))
            .map(|range| Pick {
                level,
                range,
                distance: 0.0,
            })
    };
    (want..=limits.coarsest)
        .find_map(resident)
        .or_else(|| (limits.finest..want).rev().find_map(resident))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three levels and three leaves along +x: two dense ones 1000 units apart and a sparse far one.
    fn synthetic() -> String {
        r#"{"version":1,"count":3500,"counts":[2000,1000,500],"lodLevels":3,
            "filenames":["0_0/meta.json","1_0/meta.json","2_0/meta.json","0_1/meta.json"],
            "tree":{"bound":{"min":[0,0,0],"max":[3001,1,1]},"children":[
              {"bound":{"min":[0,0,0],"max":[1,1,1]},"lods":{"0":{"file":0,"offset":0,"count":1000},"1":{"file":1,"offset":0,"count":500},"2":{"file":2,"offset":0,"count":250}}},
              {"bound":{"min":[1000,0,0],"max":[1001,1,1]},"lods":{"0":{"file":3,"offset":0,"count":1000},"1":{"file":1,"offset":500,"count":500},"2":{"file":2,"offset":250,"count":250}}},
              {"bound":{"min":[3000,0,0],"max":[3001,1,1]},"lods":{"1":{"file":1,"offset":0,"count":10},"2":{"file":2,"offset":500,"count":5}}}
            ]}}"#
        .to_owned()
    }

    fn limits(budget: u32) -> Limits {
        Limits {
            finest: 0,
            coarsest: 2,
            budget,
        }
    }

    fn focus_at(x: f32, radius: f32) -> Focus {
        Focus {
            centre: Vec3::new(x, 0.5, 0.5),
            radius,
        }
    }

    #[test]
    fn the_index_and_its_tree_parse_into_leaves_with_per_level_ranges() {
        let index = LodIndex::parse(&synthetic()).unwrap();
        assert_eq!(index.level_count(), 3);
        assert_eq!(index.leaf_count(), 3);
        assert_eq!(index.counts(), &[2000, 1000, 500]);
        assert_eq!(index.files()[3].folder, "0_1");
        assert_eq!(index.files()[3].level, 0);
        assert_eq!(index.bounds().1.x, 3001.0);
    }

    #[test]
    fn indexes_with_bad_files_levels_or_ranges_are_refused() {
        assert_eq!(LodIndex::parse("nope").unwrap_err(), LodError::Json);
        let bad_name = synthetic().replace("2_0/meta.json", "../2_0/meta.json");
        assert_eq!(LodIndex::parse(&bad_name).unwrap_err(), LodError::FileName);
        let bad_levels = synthetic().replace("\"lodLevels\":3", "\"lodLevels\":2");
        assert_eq!(LodIndex::parse(&bad_levels).unwrap_err(), LodError::Levels);
        let bad_file = synthetic().replace("\"file\":3", "\"file\":9");
        assert_eq!(LodIndex::parse(&bad_file).unwrap_err(), LodError::FileIndex);
        // A range whose file belongs to a different level than its key is inconsistent.
        let crossed = synthetic().replace(
            "\"1\":{\"file\":1,\"offset\":0,\"count\":500}",
            "\"1\":{\"file\":2,\"offset\":0,\"count\":500}",
        );
        assert_eq!(LodIndex::parse(&crossed).unwrap_err(), LodError::FileIndex);
    }

    #[test]
    fn the_finest_level_is_used_inside_the_radius_and_coarser_levels_with_distance() {
        let index = LodIndex::parse(&synthetic()).unwrap();
        let selection = index.select(focus_at(0.5, 100.0), limits(u32::MAX), None);
        let level_of = |x: usize| selection.picks[x].level;
        // Leaf 0 is inside the radius, leaf 1 is 1000 away (log2(10) -> 4 levels, clamped to 2),
        // leaf 2 further still.
        assert_eq!(level_of(0), 0);
        assert_eq!(level_of(1), 2);
        assert_eq!(level_of(2), 2);
    }

    #[test]
    fn a_radius_that_covers_two_leaves_gives_both_the_finest_level() {
        let index = LodIndex::parse(&synthetic()).unwrap();
        let selection = index.select(focus_at(500.0, 600.0), limits(u32::MAX), None);
        assert_eq!(selection.picks[0].level, 0);
        assert_eq!(selection.picks[1].level, 0);
        assert_eq!(selection.picks[2].level, 2);
        assert_eq!(selection.total, 1000 + 1000 + 5);
    }

    #[test]
    fn the_budget_pushes_every_leaf_coarser_until_the_total_fits() {
        let index = LodIndex::parse(&synthetic()).unwrap();
        let free = index.select(focus_at(500.0, 600.0), limits(u32::MAX), None);
        assert_eq!(free.bias, 0);
        let capped = index.select(focus_at(500.0, 600.0), limits(1100), None);
        assert!(capped.total <= 1100, "total {}", capped.total);
        assert_eq!(capped.bias, 1);
        let tight = index.select(focus_at(500.0, 600.0), limits(10), None);
        // Nothing can fit 10 splats; the coarsest choice is returned rather than nothing.
        assert_eq!(tight.bias, 2);
        assert_eq!(tight.total, 250 + 250 + 5);
    }

    #[test]
    fn selection_limited_to_resident_chunks_falls_back_to_coarser_levels_then_refines() {
        let index = LodIndex::parse(&synthetic()).unwrap();
        let only_coarsest = |file: u32| file == 2;
        let first = index.select(focus_at(0.5, 100.0), limits(u32::MAX), Some(&only_coarsest));
        assert!(first.picks.iter().all(|pick| pick.level == 2));
        assert_eq!(first.total, 250 + 250 + 5);
        let with_fine = |file: u32| file == 2 || file == 0;
        let second = index.select(focus_at(0.5, 100.0), limits(u32::MAX), Some(&with_fine));
        assert_eq!(second.picks[0].level, 0);
        assert_eq!(second.picks[1].level, 2);
        let nothing = |_file: u32| false;
        assert!(
            index
                .select(focus_at(0.5, 100.0), limits(u32::MAX), Some(&nothing))
                .picks
                .is_empty()
        );
    }

    #[test]
    fn files_to_fetch_are_ordered_coarse_first_without_repeats() {
        let index = LodIndex::parse(&synthetic()).unwrap();
        let selection = index.select(focus_at(500.0, 600.0), limits(u32::MAX), None);
        let order = selection.files_coarse_first();
        assert_eq!(order, vec![2, 0, 3]);
    }

    #[test]
    fn the_total_never_exceeds_the_budget_when_a_coarser_choice_fits() {
        let index = LodIndex::parse(&synthetic()).unwrap();
        for budget in [300, 600, 1500, 2100, 5000] {
            let selection = index.select(focus_at(500.0, 600.0), limits(budget), None);
            let coarsest_possible = 250 + 250 + 5;
            if budget >= coarsest_possible {
                assert!(
                    selection.total <= budget,
                    "budget {budget} total {}",
                    selection.total
                );
            }
        }
    }

    #[test]
    fn a_selection_becomes_a_decode_plan_with_merged_ranges_and_back_to_back_offsets() {
        let index = LodIndex::parse(&synthetic()).unwrap();
        let selection = index.select(focus_at(500.0, 600.0), limits(u32::MAX), None);
        let plan = selection.plan(16);
        assert_eq!(plan.total, selection.total);
        assert_eq!(plan.jobs.len(), 3);
        let job = plan.jobs[0];
        assert_eq!((job.file, job.total), (0, 1000));
        let first = plan.ranges[job.range_base as usize];
        assert_eq!(first, [0, 1000, 0, 0]);
        // Output offsets never overlap and fill the total exactly.
        let mut spans: Vec<(u32, u32)> = plan.ranges.iter().map(|r| (r[2], r[1])).collect();
        spans.sort_unstable();
        let mut at = 0;
        for (offset, count) in spans {
            assert_eq!(offset, at);
            at += count;
        }
        assert_eq!(at, plan.total);
        assert!(!plan.truncated);
        assert!(selection.plan(1).truncated);
    }

    #[test]
    fn neighbouring_leaf_ranges_in_one_chunk_merge_into_one_decode_range() {
        let index = LodIndex::parse(&synthetic()).unwrap();
        // With level 1 as the finest, leaves 0 and 1 both use file 1 at offsets 0..500 and 500..1000.
        let level_one = Limits {
            finest: 1,
            coarsest: 2,
            budget: u32::MAX,
        };
        let selection = index.select(focus_at(500.0, 600.0), level_one, None);
        let plan = selection.plan(16);
        let job = plan.jobs.iter().find(|job| job.file == 1).unwrap();
        assert_eq!(job.range_count, 1);
        assert_eq!(plan.ranges[job.range_base as usize][..2], [0, 1000]);
    }
}
