//! The resting shot: where the camera stays between beats, which is most of a fight (the whole
//! of a player's thinking time).
//!
//! It is solved, not authored. From the combatants and the grid it finds a wide, raised
//! three-quarter view in which
//! 1. every living combatant of both sides stands whole in the free part of the picture,
//! 2. so does every cell the acting figure can reach and every cell it can target,
//! 3. nothing important sits under a HUD card (the party column, the enemy column, the round
//!    strip and the dice card are keep-out rectangles; the free part is what lies between them),
//! 4. the place itself shows beyond the grid: the camera is never closer than the scene's own wide
//!    shot, and the picture is centred so the grid sits low and the scenery fills the upper third.
//!
//! The scene's authored `tactical` shot is only a hint for the direction, height and lens. Pure
//! geometry, tested natively on every scene for one to six heroes against one to six enemies.

use super::{
    camera::CameraPose,
    framing::{Body, hidden_pairs, project_body, project_point, wrap_deg},
    math::Vec3,
    occupancy::Occupancy,
};

/// A rectangle of the picture as `(left, top, right, bottom)` shares (y down).
pub type Rect = (f32, f32, f32, f32);

/// The party cards run down the left edge, one pitch per member, from the top.
const PARTY_COLUMN: Rect = (0.02, 0.043, 0.225, 0.043);
const PARTY_PITCH: f32 = 0.1157;
/// The enemy cards run down the right edge, starting lower (the sound button sits above them).
const ENEMY_COLUMN: Rect = (0.775, 0.118, 0.98, 0.118);
const ENEMY_PITCH: f32 = 0.118;
/// A card is a little shorter than its pitch.
const CARD_HEIGHT: f32 = 0.103;
/// The title and the round strip across the top.
const TOP_STRIP: Rect = (0.28, 0.02, 0.72, 0.225);
/// The dice card under the figures.
const DICE_CARD: Rect = (0.34, 0.80, 0.66, 0.955);

/// Where the resting shot keeps everything it holds: between the two card columns, below the round
/// strip and above the dice card.
pub const REST_AREA: Rect = (0.235, 0.31, 0.765, 0.79);
/// Where the reachable and targetable ground is kept. It may run under the cards at the sides
/// and the dice card, which the figures may not: a hero who can reach most of the field no
/// longer pushes the camera back until that whole region fits the free middle, which left the
/// figures a few dozen pixels tall. Its top is the free middle's, so the upper part of the
/// picture stays the place itself.
pub const FIELD_AREA: Rect = (0.04, 0.30, 0.96, 0.97);

/// How far the resting shot looks down, in degrees: at least this, so it reads as a three-quarter
/// view of the ground, and never above the upper limit, so figures keep their height.
const MIN_PITCH_DEG: f32 = 33.0;
const MAX_PITCH_DEG: f32 = 42.0;
/// How much higher than usual the camera may be raised to clear scan geometry near the lens
/// (a trunk, a branch, foliage), as extra pitch in degrees, tried in this order, and the highest
/// pitch that results.
const RAISE_CANDIDATES_DEG: [f32; 4] = [0.0, 7.0, 14.0, 21.0];
pub const MAX_RAISED_PITCH_DEG: f32 = 58.0;
/// A line from the lens to a figure is blocked by solid scan geometry above this height, except in
/// the last [`CLEAR_END_M`] metres of it (the figure's own surroundings).
const CLEAR_MIN_HEIGHT_M: f32 = 1.0;
const CLEAR_END_M: f32 = 1.3;
/// Only the part of a line nearer the lens counts: the last share of it lies in the field itself
/// (stones, shrubs and branches there are the scenery, not a mass between lens and field).
const CLEAR_FIELD_SHARE: f32 = 0.4;
/// The closest the resting shot comes, as a share of the scene's own wide shot: the place around
/// the grid is always shown.
const MIN_DISTANCE_SHARE: f32 = 0.9;
const MAX_DISTANCE_M: f32 = 48.0;
/// Room kept around each figure when fitting: extra height and width in metres.
const FIT_SLACK_M: f32 = 0.3;
/// Headings tried either side of the scene's own, in degrees.
const YAW_CANDIDATES_DEG: [f32; 9] = [0.0, 9.0, -9.0, 18.0, -18.0, 30.0, -30.0, 45.0, -45.0];
/// A pair of figures hiding more than this share of the smaller counts as overlapping.
const OVERLAP_SHARE: f32 = 0.45;

/// The HUD rectangles for a fight of `party` heroes and `enemies` foes (defeated ones keep their
/// card, so the counts are of everyone on each side).
pub fn hud_keep_outs(party: usize, enemies: usize) -> Vec<Rect> {
    let column = |column: Rect, pitch: f32, count: usize| -> Option<Rect> {
        if count == 0 {
            return None;
        }
        Some((
            column.0,
            column.1,
            column.2,
            column.1 + pitch * (count as f32 - 1.0) + CARD_HEIGHT,
        ))
    };
    [
        column(PARTY_COLUMN, PARTY_PITCH, party),
        column(ENEMY_COLUMN, ENEMY_PITCH, enemies),
        Some(TOP_STRIP),
        Some(DICE_CARD),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Whether two rectangles overlap.
pub fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

/// What the resting shot must hold.
pub struct RestInput<'a> {
    /// Every living combatant.
    pub bodies: &'a [Body],
    /// Points on the ground that must be in frame: the corners of every reachable and targetable
    /// cell.
    pub ground: &'a [Vec3],
    /// The scene's authored wide shot: a hint for the direction, height and lens.
    pub hint: &'a CameraPose,
    /// The heading the camera has now, to stay near it.
    pub previous_yaw: Option<f32>,
    /// Solid scan geometry around the arena, when the scene has it: the shot keeps a clear line to
    /// every figure and to the field, raising or turning the camera rather than looking through it.
    pub occupancy: Option<&'a Occupancy>,
    pub aspect: f32,
}

/// The screen rectangle holding everything in `input` from `pose`, or `None` when part of it is
/// behind the camera.
pub fn extent(pose: &CameraPose, input: &RestInput<'_>) -> Option<Rect> {
    let mut bounds = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    let mut take = |x: f32, y: f32| {
        bounds.0 = bounds.0.min(x);
        bounds.1 = bounds.1.min(y);
        bounds.2 = bounds.2.max(x);
        bounds.3 = bounds.3.max(y);
    };
    for body in input.bodies {
        let slack = Body {
            feet: body.feet,
            height_m: body.height_m + FIT_SLACK_M,
        };
        let (left, top, right, bottom) = project_body(pose, input.aspect, &slack)?;
        take(left, top);
        take(right, bottom);
    }
    for point in input.ground {
        let (x, y) = project_point(pose, input.aspect, *point)?;
        take(x, y);
    }
    (bounds.0 <= bounds.2).then_some(bounds)
}

/// The screen rectangle holding the figures alone (the whole of `input` when it has none).
fn figures_extent(pose: &CameraPose, input: &RestInput<'_>) -> Option<Rect> {
    if input.bodies.is_empty() {
        return extent(pose, input);
    }
    extent(
        pose,
        &RestInput {
            bodies: input.bodies,
            ground: &[],
            hint: input.hint,
            previous_yaw: input.previous_yaw,
            occupancy: input.occupancy,
            aspect: input.aspect,
        },
    )
}

/// Whether `pose` holds the figures in the free middle and the ground in the field area.
pub fn holds(pose: &CameraPose, input: &RestInput<'_>) -> bool {
    figures_extent(pose, input).is_some_and(|found| inside(found, REST_AREA))
        && extent(pose, input).is_some_and(|found| inside(found, FIELD_AREA))
}

fn centre(rect: Rect) -> (f32, f32) {
    ((rect.0 + rect.2) * 0.5, (rect.1 + rect.3) * 0.5)
}

fn inside(rect: Rect, area: Rect) -> bool {
    rect.0 >= area.0 && rect.1 >= area.1 && rect.2 <= area.2 && rect.3 <= area.3
}

/// The pose with heading `yaw_deg` that holds the figures of `input` inside [`REST_AREA`] and its
/// ground inside [`FIELD_AREA`] from as
/// near as the scene's wide shot allows.
fn fit(input: &RestInput<'_>, yaw_deg: f32, raise_deg: f32) -> CameraPose {
    // A scene may ask for a steeper wide shot than the usual upper limit (a wood whose trunks
    // stand in the field is read from higher up); the raised limit still bounds it.
    let own_limit = MAX_PITCH_DEG.max(input.hint.pitch_deg.min(MAX_RAISED_PITCH_DEG));
    let pitch_deg = (input.hint.pitch_deg.clamp(MIN_PITCH_DEG, own_limit) + raise_deg)
        .min(MAX_RAISED_PITCH_DEG);
    let yaw = yaw_deg.to_radians();
    let right = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
    let forward = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
    let mut sum = Vec3::ZERO;
    let mut count = 0.0;
    for point in input
        .bodies
        .iter()
        .map(|body| body.feet)
        .chain(input.ground.iter().copied())
    {
        sum = sum + point;
        count += 1.0;
    }
    let middle = if count > 0.0 {
        Vec3::new(sum.x / count, 0.0, sum.z / count)
    } else {
        Vec3::new(input.hint.target.x, 0.0, input.hint.target.z)
    };
    let mut pose = CameraPose {
        target: middle + Vec3::new(0.0, 0.9, 0.0),
        distance: input.hint.distance,
        yaw_deg,
        pitch_deg,
        fov_deg: input.hint.fov_deg,
    };
    let minimum = input.hint.distance * MIN_DISTANCE_SHARE;
    let want = centre(REST_AREA);
    let size = |rect: Rect| (rect.2 - rect.0, rect.3 - rect.1);
    let (area_width, area_height) = size(REST_AREA);
    let (field_width, field_height) = size(FIELD_AREA);
    for _ in 0..48 {
        let (Some(found), Some(all)) = (figures_extent(&pose, input), extent(&pose, input)) else {
            pose.distance = (pose.distance * 1.3).min(MAX_DISTANCE_M);
            continue;
        };
        // The figures fill the free middle, or the ground fills the field, whichever is tighter.
        let (figures, ground) = (size(found), size(all));
        let need = (figures.0 / area_width)
            .max(figures.1 / area_height)
            .max(ground.0 / field_width)
            .max(ground.1 / field_height);
        let have = centre(found);
        let (miss_x, miss_y) = (want.0 - have.0, want.1 - have.1);
        if miss_x.abs() < 0.003 && miss_y.abs() < 0.003 && (0.985..=1.0).contains(&need) {
            break;
        }
        // Slide the target on the ground until the picture is centred: how far the middle of the
        // held area moves for half a metre along each ground axis gives the two directions.
        let probe = |step: Vec3| {
            let moved = CameraPose {
                target: pose.target + step,
                ..pose
            };
            figures_extent(&moved, input).map(centre)
        };
        if let (Some(along_right), Some(along_forward)) = (probe(right * 0.5), probe(forward * 0.5))
        {
            let (a, b) = (
                (along_right.0 - have.0, along_right.1 - have.1),
                (along_forward.0 - have.0, along_forward.1 - have.1),
            );
            let determinant = a.0 * b.1 - a.1 * b.0;
            if determinant.abs() > 1e-9 {
                let to_right = (miss_x * b.1 - miss_y * b.0) / determinant;
                let to_forward = (a.0 * miss_y - a.1 * miss_x) / determinant;
                pose.target = pose.target
                    + right * (to_right.clamp(-8.0, 8.0) * 0.5)
                    + forward * (to_forward.clamp(-8.0, 8.0) * 0.5);
            }
        }
        pose.distance =
            (pose.distance * need.clamp(0.7, 1.4).powf(0.9)).clamp(minimum, MAX_DISTANCE_M);
    }
    // Whatever the iteration reached, back off until it is inside.
    while pose.distance < MAX_DISTANCE_M && !holds(&pose, input) {
        pose.distance *= 1.03;
    }
    pose.distance = pose.distance.min(MAX_DISTANCE_M);
    pose
}

/// How many of the lines from `pose`'s lens to the figures and to the field pass through solid scan
/// geometry: `(figures, field points)`. Zero when the scene carries no occupancy.
pub fn blocked_lines(pose: &CameraPose, input: &RestInput<'_>) -> (usize, usize) {
    let Some(occupancy) = input.occupancy else {
        return (0, 0);
    };
    let eye = pose.eye();
    let figures = input
        .bodies
        .iter()
        .filter(|body| {
            let chest = body.feet + Vec3::new(0.0, body.height_m * 0.7, 0.0);
            let end = ((chest - eye).length() * CLEAR_FIELD_SHARE).max(CLEAR_END_M);
            !occupancy.segment_clear(eye, chest, end, CLEAR_MIN_HEIGHT_M)
        })
        .count();
    // Up to about sixty field points, spread evenly.
    let stride = (input.ground.len() / 60).max(1);
    let field = input
        .ground
        .iter()
        .step_by(stride)
        .filter(|point| {
            let above = **point + Vec3::new(0.0, 0.9, 0.0);
            let end = ((above - eye).length() * CLEAR_FIELD_SHARE).max(CLEAR_END_M);
            !occupancy.segment_clear(eye, above, end, CLEAR_MIN_HEIGHT_M)
        })
        .count();
    (figures, field)
}

/// The resting shot: the best of a few headings near the scene's own, each fitted to hold
/// everything, raised when needed to see over scan geometry near the lens. The choice is the one
/// with the fewest blocked lines to figures and the field, then the fewest figures hiding one
/// another, preferring the scene's own heading and low camera, then the heading the camera
/// already has.
pub fn rest_pose(input: &RestInput<'_>) -> CameraPose {
    let mut best: Option<(f32, CameraPose)> = None;
    for raise in RAISE_CANDIDATES_DEG {
        for offset in YAW_CANDIDATES_DEG {
            let yaw = input.hint.yaw_deg + offset;
            let pose = fit(input, yaw, raise);
            let hidden = hidden_pairs(&pose, input.aspect, input.bodies, OVERLAP_SHARE) as f32;
            let drift = input
                .previous_yaw
                .map_or(0.0, |previous| wrap_deg(yaw - previous).abs() * 0.02);
            let (blocked_figures, blocked_field) = blocked_lines(&pose, input);
            let cost = hidden * 10.0
                + offset.abs() * 0.05
                + raise * 0.3
                + drift
                + blocked_figures as f32 * 40.0
                + blocked_field as f32 * 2.0;
            if best.is_none_or(|(least, _)| cost < least) {
                best = Some((cost, pose));
            }
        }
        // A heading that sees everything at this height is taken; only a blocked view raises the
        // camera further.
        if best.is_some_and(|(_, pose)| blocked_lines(&pose, input) == (0, 0)) {
            break;
        }
    }
    best.map_or(*input.hint, |(_, pose)| pose)
}

/// The corners of a cell's square on the ground, for the cells that must be framed.
pub fn cell_corners(centre: Vec3, cell_m: f32) -> [Vec3; 4] {
    let half = cell_m * 0.5;
    [
        Vec3::new(centre.x - half, centre.y, centre.z - half),
        Vec3::new(centre.x + half, centre.y, centre.z - half),
        Vec3::new(centre.x - half, centre.y, centre.z + half),
        Vec3::new(centre.x + half, centre.y, centre.z + half),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle::{
        Cell, Grid, derive_formation, embedded_ids, framing::FRAMED_ASPECT, load_embedded,
    };

    /// Everything a fight of `heroes` against `enemies` asks the resting shot to hold on `grid`:
    /// the first hero's reachable region (every walkable cell within six cells) and every foe.
    struct Fight {
        bodies: Vec<Body>,
        ground: Vec<Vec3>,
        party: usize,
        enemies: usize,
    }

    fn fight(grid: &Grid, heroes: usize, enemies: usize) -> Option<Fight> {
        let formation = derive_formation(grid, heroes, enemies).ok()?;
        let body = |cell: &Cell, height_m: f32| Body {
            feet: grid.cell_centre(*cell),
            height_m,
        };
        let mut bodies: Vec<Body> = formation
            .heroes
            .iter()
            .map(|cell| body(cell, 1.35))
            .collect();
        bodies.extend(formation.enemies.iter().map(|cell| body(cell, 1.0)));
        let actor = formation.heroes[0];
        let mut ground = Vec::new();
        let mut take = |cell: Cell| {
            ground.extend(cell_corners(grid.cell_centre(cell), grid.cell_m()));
        };
        for cell in grid.walkable_cells() {
            let near = (cell.col - actor.col)
                .abs()
                .max((cell.row - actor.row).abs())
                <= 6;
            if near {
                take(cell);
            }
        }
        for cell in &formation.enemies {
            take(*cell);
        }
        Some(Fight {
            bodies,
            ground,
            party: heroes,
            enemies,
        })
    }

    fn input<'a>(fight: &'a Fight, hint: &'a CameraPose) -> RestInput<'a> {
        RestInput {
            bodies: &fight.bodies,
            ground: &fight.ground,
            hint,
            previous_yaw: None,
            occupancy: None,
            aspect: FRAMED_ASPECT,
        }
    }

    #[test]
    fn the_cards_leave_a_free_middle_for_the_resting_shot() {
        for (party, enemies) in [(1, 1), (3, 3), (6, 6)] {
            for keep_out in hud_keep_outs(party, enemies) {
                assert!(
                    !rects_overlap(keep_out, REST_AREA),
                    "{keep_out:?} vs {REST_AREA:?} at {party}v{enemies}"
                );
            }
        }
    }

    #[test]
    fn every_scene_holds_two_seven_and_twelve_figures_in_the_free_picture() {
        let mut checked = 0;
        for id in embedded_ids() {
            let scene = load_embedded(id).unwrap().unwrap();
            let hint = scene.cameras[&crate::battle::ShotName::Tactical];
            for (heroes, enemies) in [(1, 1), (4, 3), (6, 6)] {
                let Some(fight) = fight(&scene.grid, heroes, enemies) else {
                    continue;
                };
                let input = input(&fight, &hint);
                let pose = rest_pose(&input);
                let keep_outs = hud_keep_outs(fight.party, fight.enemies);
                for (index, body) in fight.bodies.iter().enumerate() {
                    let rect = project_body(&pose, FRAMED_ASPECT, body).unwrap_or_else(|| {
                        panic!("{id} {heroes}v{enemies}: figure {index} behind")
                    });
                    assert!(
                        inside(rect, crate::battle::framing::SAFE_AREA),
                        "{id} {heroes}v{enemies}: figure {index} outside the safe area {rect:?}"
                    );
                    assert!(
                        inside(rect, REST_AREA),
                        "{id} {heroes}v{enemies}: figure {index} outside the free area {rect:?}"
                    );
                    for keep_out in &keep_outs {
                        assert!(
                            !rects_overlap(rect, *keep_out),
                            "{id} {heroes}v{enemies}: figure {index} {rect:?} under {keep_out:?}"
                        );
                    }
                }
                let held = extent(&pose, &input).expect("everything is in front of the camera");
                assert!(
                    inside(held, FIELD_AREA),
                    "{id} {heroes}v{enemies}: the reachable region leaves the frame {held:?}"
                );
                assert!(
                    pose.distance >= hint.distance * MIN_DISTANCE_SHARE - 1e-3,
                    "{id}: closer than the wide shot ({})",
                    pose.distance
                );
                assert!(
                    (MIN_PITCH_DEG..=MAX_RAISED_PITCH_DEG).contains(&pose.pitch_deg),
                    "{id}: pitch {}",
                    pose.pitch_deg
                );
                eprintln!(
                    "{id} {heroes}v{enemies}: distance {:.1} m, pitch {:.0}, yaw {:.0} (scene {:.0}), held {:.2?}",
                    pose.distance, pose.pitch_deg, pose.yaw_deg, hint.yaw_deg, held
                );
                checked += 1;
            }
        }
        assert!(checked >= 20, "only {checked} cases ran");
    }

    #[test]
    fn a_scene_with_scan_occupancy_keeps_a_clear_line_to_every_figure_and_the_field() {
        let mut checked = 0;
        for id in embedded_ids() {
            let scene = load_embedded(id).unwrap().unwrap();
            let Some(occupancy) = scene.occupancy.as_ref() else {
                continue;
            };
            let hint = scene.cameras[&crate::battle::ShotName::Tactical];
            for (heroes, enemies) in [(1, 1), (4, 3), (6, 6)] {
                let Some(fight) = fight(&scene.grid, heroes, enemies) else {
                    continue;
                };
                let plain = input(&fight, &hint);
                let aware = RestInput {
                    occupancy: Some(occupancy),
                    ..input(&fight, &hint)
                };
                // The same solve without the occupancy, judged against it: what the camera did before.
                let before = blocked_lines(&rest_pose(&plain), &aware);
                let pose = rest_pose(&aware);
                let (figures, field) = blocked_lines(&pose, &aware);
                let sampled = fight
                    .ground
                    .len()
                    .div_ceil((fight.ground.len() / 60).max(1));
                eprintln!(
                    "{id} {heroes}v{enemies}: blocked lines before {before:?}, now ({figures}, {field}) of {} figures and {sampled} field points; pitch {:.0}, yaw {:.0}",
                    fight.bodies.len(),
                    pose.pitch_deg,
                    pose.yaw_deg
                );
                assert_eq!(
                    figures, 0,
                    "{id} {heroes}v{enemies}: a figure is seen through scan geometry"
                );
                assert!(
                    field * 20 <= sampled,
                    "{id} {heroes}v{enemies}: {field} of {sampled} field points are seen through scan geometry"
                );
                assert!(
                    figures + field <= before.0 + before.1,
                    "{id}: no better than before"
                );
                // Raising the camera never costs the HUD clearance.
                let held = extent(&pose, &aware).unwrap();
                assert!(
                    inside(held, FIELD_AREA),
                    "{id} {heroes}v{enemies}: {held:?}"
                );
                checked += 1;
            }
        }
        assert!(checked >= 3, "only {checked} cases ran");
    }

    #[test]
    fn the_scenery_shows_beyond_the_grid() {
        // The far edge of the held region sits well below the top of the picture on every scene,
        // so the upper part is the place itself.
        for id in embedded_ids() {
            let scene = load_embedded(id).unwrap().unwrap();
            let hint = scene.cameras[&crate::battle::ShotName::Tactical];
            let Some(fight) = fight(&scene.grid, 3, 3) else {
                continue;
            };
            let pose = rest_pose(&input(&fight, &hint));
            let held = extent(&pose, &input(&fight, &hint)).unwrap();
            assert!(held.1 >= 0.30, "{id}: held region reaches {}", held.1);
            assert!(
                held.3 <= FIELD_AREA.3,
                "{id}: held region reaches {}",
                held.3
            );
            // The figures themselves stay above the dice card.
            let figures = figures_extent(&pose, &input(&fight, &hint)).unwrap();
            assert!(figures.3 <= 0.80, "{id}: figures reach {}", figures.3);
        }
    }

    #[test]
    fn a_heading_is_never_further_from_the_scenes_own_than_the_candidates_allow() {
        for id in embedded_ids() {
            let scene = load_embedded(id).unwrap().unwrap();
            let hint = scene.cameras[&crate::battle::ShotName::Tactical];
            let Some(fight) = fight(&scene.grid, 2, 2) else {
                continue;
            };
            let pose = rest_pose(&input(&fight, &hint));
            assert!(wrap_deg(pose.yaw_deg - hint.yaw_deg).abs() <= 45.01, "{id}");
        }
    }
}
