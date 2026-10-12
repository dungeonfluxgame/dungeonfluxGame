//! The shot for an attack: both fighters in frame for the whole beat, from a side of the line of
//! action the camera already holds, reached by the shortest move, or by a cut when the move would
//! be a swing.
//!
//! The rules, in the order they apply:
//! 1. Both fighters (and every spot either will stand on during the beat) are inside the safe
//!    area of the picture, always. The camera backs off until they are.
//! 2. The camera stays on the side of the line between the fighters it is already on, so left and
//!    right do not flip between a hero's turn and the enemy's answer. Of the four over-the-
//!    shoulder angles it takes the one nearest its current heading, preferring the target's
//!    shoulder to the attacker's by [`ATTACKER_SHOULDER_PENALTY_DEG`].
//! 3. A move larger than [`MAX_SWING_DEG`] of heading is a cut (the caller pulls focus), never an
//!    orbit.
//!
//! Pure geometry: tested natively for every attacker and target cell on every scene and layout.

use super::{camera::CameraPose, math::Vec3};

/// The part of the picture figures are kept inside, as `(left, top, right, bottom)` shares of the
/// frame (y down). It clears the party and enemy cards at the sides, the round strip at the top
/// and the floating numbers and roll card below.
pub const SAFE_AREA: (f32, f32, f32, f32) = (0.16, 0.28, 0.84, 0.92);
/// Aspect ratio the framing is solved for (the shared display is 16:9).
pub const FRAMED_ASPECT: f32 = 16.0 / 9.0;
/// Camera angle from the line of action, over the target's shoulder and over the attacker's.
pub const OVER_TARGET_DEG: f32 = 55.0;
pub const OVER_ATTACKER_DEG: f32 = 125.0;
/// How much more a camera angle over the attacker's shoulder costs than one over the target's.
pub const ATTACKER_SHOULDER_PENALTY_DEG: f32 = 20.0;
/// The most the camera turns (degrees of heading) for one turn before it cuts instead.
pub const MAX_SWING_DEG: f32 = 60.0;
/// How far the camera's heading may stray from the scene's own tactical heading, in degrees: a
/// scene is built to be seen from its front (a scan has a fence, a hut or a slope behind the
/// arena), so the action shots stay on that side and never swing round to the back of the set.
pub const MAX_HOME_DEVIATION_DEG: f32 = 75.0;
/// The closest and farthest the action shot may be, in metres.
pub const MIN_DISTANCE_M: f32 = 3.6;
pub const MAX_DISTANCE_M: f32 = 28.0;
/// Half the width of a standing figure as a share of its height (the art is about that wide).
const HALF_WIDTH_SHARE: f32 = 0.32;
/// Extra room kept around the fighters when fitting, in metres, for a recoil or a sway.
const FIT_SLACK_M: f32 = 0.45;

/// A figure the shot must contain: where its feet are and how tall it stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub feet: Vec3,
    pub height_m: f32,
}

/// The shot chosen for an attack.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActionShot {
    pub pose: CameraPose,
    /// Cut to the pose (with a focus pull) instead of panning, because the move is a swing.
    pub cut: bool,
    /// How far the heading turns, in degrees (0 to 180).
    pub swing_deg: f32,
}

/// Wraps an angle in degrees to -180..180.
pub fn wrap_deg(angle: f32) -> f32 {
    let wrapped = (angle + 180.0).rem_euclid(360.0) - 180.0;
    if wrapped == -180.0 { 180.0 } else { wrapped }
}

/// The heading (yaw, degrees) of the camera that looks along `from -> to` over the far end.
fn heading_deg(from: Vec3, to: Vec3) -> f32 {
    (to.x - from.x).atan2(to.z - from.z).to_degrees()
}

/// A camera's projection, built once for many bodies.
struct View {
    matrix: super::math::Mat4,
    right: Vec3,
}

impl View {
    fn new(pose: &CameraPose, aspect: f32) -> Self {
        let look = pose.target - pose.eye();
        Self {
            matrix: pose.projection(aspect, 0.1, 300.0).mul(&pose.view()),
            right: Vec3::new(-look.z, 0.0, look.x).normalized(),
        }
    }

    fn project(&self, body: &Body) -> Option<(f32, f32, f32, f32)> {
        let half = body.height_m * HALF_WIDTH_SHARE;
        let mut bounds = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for side in [-1.0, 1.0] {
            for height in [0.0, body.height_m] {
                let point = body.feet + self.right * (half * side) + Vec3::new(0.0, height, 0.0);
                let (x, y, _) = self.matrix.project(point)?;
                let (screen_x, screen_y) = (x * 0.5 + 0.5, 0.5 - y * 0.5);
                bounds.0 = bounds.0.min(screen_x);
                bounds.1 = bounds.1.min(screen_y);
                bounds.2 = bounds.2.max(screen_x);
                bounds.3 = bounds.3.max(screen_y);
            }
        }
        Some(bounds)
    }

    fn contains(&self, body: &Body) -> bool {
        self.project(body)
            .is_some_and(|(left, top, right, bottom)| {
                left >= SAFE_AREA.0
                    && top >= SAFE_AREA.1
                    && right <= SAFE_AREA.2
                    && bottom <= SAFE_AREA.3
            })
    }
}

/// A body's projected box on the screen as `(left, top, right, bottom)` shares (y down), or
/// `None` when any corner is behind the camera.
pub fn project_body(pose: &CameraPose, aspect: f32, body: &Body) -> Option<(f32, f32, f32, f32)> {
    View::new(pose, aspect).project(body)
}

/// Whether the body's box is inside the safe area.
pub fn in_safe_area(pose: &CameraPose, aspect: f32, body: &Body) -> bool {
    View::new(pose, aspect).contains(body)
}

fn all_in_safe_area(pose: &CameraPose, aspect: f32, bodies: &[Body]) -> bool {
    let view = View::new(pose, aspect);
    bodies.iter().all(|body| view.contains(body))
}

/// How far a bystander must be nearer the camera than a fighter, in metres, to count as in front.
const IN_FRONT_M: f32 = 0.2;

fn boxes_overlap(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

/// The share of the smaller box that the other covers.
fn covered_share(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> f32 {
    let width = (a.2.min(b.2) - a.0.max(b.0)).max(0.0);
    let height = (a.3.min(b.3) - a.1.max(b.1)).max(0.0);
    let smaller = ((a.2 - a.0) * (a.3 - a.1)).min((b.2 - b.0) * (b.3 - b.1));
    if smaller <= 0.0 {
        0.0
    } else {
        width * height / smaller
    }
}

/// How much of the farther fighter the nearer one may hide before the angle is rejected: the two
/// silhouettes have to be separate on the screen (a camera along the line of action puts the
/// attacker's body over the target).
pub const MAX_PAIR_COVER: f32 = 0.3;

/// Whether the two fighters hide each other too much from `pose`.
fn pair_hidden(pose: &CameraPose, aspect: f32, a: &Body, b: &Body) -> bool {
    if (a.feet - b.feet).length() < 1e-3 {
        return false;
    }
    let view = View::new(pose, aspect);
    let (Some(first), Some(second)) = (view.project(a), view.project(b)) else {
        return true;
    };
    covered_share(first, second) > MAX_PAIR_COVER
}

/// Whether panning from `from` to `to` keeps every body on the screen the whole way (the middle of
/// the move is where an orbit around a far subject loses it). A move that does not is a cut.
pub fn pan_keeps_in_view(from: &CameraPose, to: &CameraPose, bodies: &[Body], aspect: f32) -> bool {
    for step in 1..=4 {
        let t = super::math::ease_in_out(step as f32 / 5.0);
        let view = View::new(&from.lerp(to, t), aspect);
        for body in bodies {
            let on_screen = view
                .project(body)
                .is_some_and(|(left, top, right, bottom)| {
                    let (x, y) = ((left + right) * 0.5, (top + bottom) * 0.5);
                    (0.03..=0.97).contains(&x) && (0.03..=0.97).contains(&y)
                });
            if !on_screen {
                return false;
            }
        }
    }
    true
}

/// How many bystanders stand in front of an involved fighter on the screen (overlapping its box
/// and nearer the camera).
fn blocked_by_bystanders(
    pose: &CameraPose,
    aspect: f32,
    involved: &[Body],
    bystanders: &[Body],
) -> usize {
    let view = View::new(pose, aspect);
    let eye = pose.eye();
    let mut count = 0;
    for bystander in bystanders {
        let Some(hidden) = view.project(bystander) else {
            continue;
        };
        let nearer =
            |body: &Body| (bystander.feet - eye).length() + IN_FRONT_M < (body.feet - eye).length();
        let blocks = involved.iter().any(|body| {
            nearer(body)
                && view
                    .project(body)
                    .is_some_and(|shown| boxes_overlap(hidden, shown))
        });
        count += usize::from(blocks);
    }
    count
}

/// The shot for `actor`, striking `target` (or acting on itself when `target` is `None`).
///
/// `style` carries the scene's authored pitch and lens for a close shot; `current` is where the
/// camera is now (its heading decides the side); `also` lists every other place either fighter
/// stands during the beat (the attacker's spot at contact, the target's recoil), so the frame holds
/// them all without moving. `bystanders` are everyone else on the field: of the camera angles that
/// keep the swing small, the first with none of them standing in front of an involved fighter is
/// taken, and a bystander is never part of what the frame is fitted to.
pub fn action_shot(
    style: &CameraPose,
    current: &CameraPose,
    actor: &Body,
    target: Option<&Body>,
    also: &[Body],
    bystanders: &[Body],
    aspect: f32,
) -> ActionShot {
    action_shot_from(
        style, current, actor, target, also, bystanders, aspect, None,
    )
}

/// As [`action_shot`], keeping the camera within [`MAX_HOME_DEVIATION_DEG`] of `home_yaw` (the
/// scene's tactical heading) when given.
#[allow(clippy::too_many_arguments)]
pub fn action_shot_from(
    style: &CameraPose,
    current: &CameraPose,
    actor: &Body,
    target: Option<&Body>,
    also: &[Body],
    bystanders: &[Body],
    aspect: f32,
    home_yaw: Option<f32>,
) -> ActionShot {
    let chest = |body: &Body| body.feet + Vec3::new(0.0, body.height_m * 0.5, 0.0);
    let other = target.copied().unwrap_or(*actor);
    // A little toward the target, so the frame leans over its shoulder.
    let focus = chest(actor).lerp(chest(&other), 0.55);
    let has_line = target.is_some() && (other.feet - actor.feet).length() >= 1e-3;
    let line_heading = heading_deg(actor.feet, other.feet);
    // Candidate swings from the current heading with what each costs beyond the swing itself.
    let mut candidates: Vec<(f32, f32)> = if has_line {
        [
            (OVER_TARGET_DEG, 0.0),
            (-OVER_TARGET_DEG, 0.0),
            (OVER_ATTACKER_DEG, ATTACKER_SHOULDER_PENALTY_DEG),
            (-OVER_ATTACKER_DEG, ATTACKER_SHOULDER_PENALTY_DEG),
            (90.0, 6.0),
            (-90.0, 6.0),
        ]
        .into_iter()
        .map(|(offset, penalty)| (wrap_deg(line_heading + offset - current.yaw_deg), penalty))
        .collect()
    } else {
        // A self action has no line: stay put, or turn to the side that clears the bystanders.
        [
            0.0, 35.0, -35.0, 70.0, -70.0, 110.0, -110.0, 150.0, -150.0, 180.0,
        ]
        .into_iter()
        .map(|swing| (swing, 0.0))
        .collect()
    };
    // An angle behind the set is a last resort: it costs more than any swing.
    if let Some(home) = home_yaw {
        for (swing, penalty) in &mut candidates {
            let deviation = wrap_deg(current.yaw_deg + *swing - home).abs();
            if deviation > MAX_HOME_DEVIATION_DEG {
                *penalty += 1000.0 + deviation;
            }
        }
    }
    candidates.sort_by(|a, b| (a.0.abs() + a.1).total_cmp(&(b.0.abs() + b.1)));
    let mut bodies = vec![*actor, other];
    bodies.extend_from_slice(also);
    // Fit with a little room to spare: each body a touch taller as well as exactly as it is.
    let mut slack_bodies: Vec<Body> = bodies
        .iter()
        .map(|body| Body {
            feet: body.feet,
            height_m: body.height_m + FIT_SLACK_M,
        })
        .collect();
    slack_bodies.extend_from_slice(&bodies);
    let fit = |swing: f32| {
        let mut pose = CameraPose {
            target: focus,
            distance: MIN_DISTANCE_M.max(style.distance * 0.6),
            yaw_deg: current.yaw_deg + swing,
            pitch_deg: style.pitch_deg,
            fov_deg: style.fov_deg,
        };
        while pose.distance < MAX_DISTANCE_M && !all_in_safe_area(&pose, aspect, &slack_bodies) {
            pose.distance *= 1.05;
        }
        pose.distance = pose.distance.min(MAX_DISTANCE_M);
        pose
    };
    let involved = [*actor, other];
    let mut chosen: Option<(usize, f32, CameraPose)> = None;
    for (swing, _) in &candidates {
        let pose = fit(*swing);
        let mut blocked = if bystanders.is_empty() {
            0
        } else {
            blocked_by_bystanders(&pose, aspect, &involved, bystanders)
        };
        if has_line && pair_hidden(&pose, aspect, actor, &other) {
            blocked += 1;
        }
        if chosen.is_none_or(|(least, _, _)| blocked < least) {
            chosen = Some((blocked, *swing, pose));
        }
        if blocked == 0 {
            break;
        }
    }
    let (_, swing, pose) = chosen.unwrap_or_else(|| (0, 0.0, fit(0.0)));
    ActionShot {
        pose,
        cut: swing.abs() > MAX_SWING_DEG,
        swing_deg: swing.abs(),
    }
}

/// Whether any bystander stands in front of an involved fighter on the screen from `pose`.
pub fn bystander_in_front(
    pose: &CameraPose,
    aspect: f32,
    involved: &[Body],
    bystanders: &[Body],
) -> bool {
    blocked_by_bystanders(pose, aspect, involved, bystanders) > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> CameraPose {
        CameraPose {
            target: Vec3::ZERO,
            distance: 8.0,
            yaw_deg: 32.0,
            pitch_deg: 13.0,
            fov_deg: 38.0,
        }
    }

    fn body(x: f32, z: f32) -> Body {
        Body {
            feet: Vec3::new(x, 0.0, z),
            height_m: 1.35,
        }
    }

    fn current(yaw: f32) -> CameraPose {
        CameraPose {
            yaw_deg: yaw,
            ..style()
        }
    }

    #[test]
    fn wrapping_takes_the_short_way_round() {
        assert_eq!(wrap_deg(190.0), -170.0);
        assert_eq!(wrap_deg(-190.0), 170.0);
        assert_eq!(wrap_deg(0.0), 0.0);
        assert_eq!(wrap_deg(540.0), 180.0);
        assert_eq!(wrap_deg(720.0), 0.0);
    }

    #[test]
    fn both_fighters_are_in_the_safe_area_whatever_their_distance_apart() {
        for gap in [1.4, 2.8, 5.0, 9.0, 14.0] {
            let (attacker, target) = (body(-gap / 2.0, 0.0), body(gap / 2.0, 0.0));
            let shot = action_shot(
                &style(),
                &current(0.0),
                &attacker,
                Some(&target),
                &[],
                &[],
                FRAMED_ASPECT,
            );
            assert!(
                in_safe_area(&shot.pose, FRAMED_ASPECT, &attacker),
                "gap {gap}"
            );
            assert!(
                in_safe_area(&shot.pose, FRAMED_ASPECT, &target),
                "gap {gap}"
            );
        }
    }

    #[test]
    fn the_frame_also_holds_the_places_they_will_stand_later_in_the_beat() {
        let (attacker, target) = (body(-4.0, 0.0), body(1.0, 0.0));
        let later = [body(0.0, 0.8), body(1.6, -0.3)];
        let shot = action_shot(
            &style(),
            &current(0.0),
            &attacker,
            Some(&target),
            &later,
            &[],
            FRAMED_ASPECT,
        );
        for body in [attacker, target, later[0], later[1]] {
            assert!(in_safe_area(&shot.pose, FRAMED_ASPECT, &body));
        }
    }

    #[test]
    fn the_camera_stays_on_its_side_of_the_line_when_the_enemy_answers() {
        // A hero strikes along +x; the camera ends up somewhere on one side of that line.
        let hero = body(-2.0, 0.0);
        let goblin = body(0.0, 0.0);
        let first = action_shot(
            &style(),
            &current(32.0),
            &hero,
            Some(&goblin),
            &[],
            &[],
            FRAMED_ASPECT,
        );
        let side = |pose: &CameraPose| (pose.eye() - pose.target).z.signum();
        // The goblin answers: the line is reversed, the camera must not cross to the other side.
        let answer = action_shot(
            &style(),
            &first.pose,
            &goblin,
            Some(&hero),
            &[],
            &[],
            FRAMED_ASPECT,
        );
        assert_eq!(
            side(&first.pose),
            side(&answer.pose),
            "{first:?} {answer:?}"
        );
        assert!(answer.swing_deg <= MAX_SWING_DEG, "{}", answer.swing_deg);
        assert!(!answer.cut);
    }

    #[test]
    fn a_big_change_of_subject_cuts_and_a_small_one_pans_the_short_way() {
        let (a, b) = (body(-3.0, 0.0), body(0.0, 0.0));
        let near = action_shot(
            &style(),
            &current(60.0),
            &a,
            Some(&b),
            &[],
            &[],
            FRAMED_ASPECT,
        );
        assert!(!near.cut, "{}", near.swing_deg);
        // However the line points, the choice of four angles keeps the swing at most 90 degrees
        // from the nearest, so a swing that needs more than the cap is a cut.
        let mut cuts = 0;
        for yaw in (-180..180).step_by(10) {
            let shot = action_shot(
                &style(),
                &current(yaw as f32),
                &a,
                Some(&b),
                &[],
                &[],
                FRAMED_ASPECT,
            );
            assert_eq!(shot.cut, shot.swing_deg > MAX_SWING_DEG);
            assert!(shot.swing_deg <= 90.0 + 1e-3, "{yaw}: {}", shot.swing_deg);
            // The pose is reached by the short way: its yaw is within the swing of the current.
            assert!((shot.pose.yaw_deg - yaw as f32).abs() <= shot.swing_deg + 1e-3);
            cuts += usize::from(shot.cut);
        }
        // The four angles are at most 70 degrees apart from the camera only for some headings.
        let _ = cuts;
    }

    #[test]
    fn it_prefers_the_target_shoulder_but_stays_put_rather_than_swing_to_it() {
        let (attacker, target) = (body(-3.0, 0.0), body(0.0, 0.0));
        // The line heads along +x, heading 90: over the target is 145 or 35.
        let on_target_side = action_shot(
            &style(),
            &current(35.0),
            &attacker,
            Some(&target),
            &[],
            &[],
            FRAMED_ASPECT,
        );
        assert!(on_target_side.swing_deg < 1e-3);
        // Camera already over the attacker's shoulder on that side (heading 90 + 125 = 215 = -145):
        let on_attacker_side = action_shot(
            &style(),
            &current(-145.0),
            &attacker,
            Some(&target),
            &[],
            &[],
            FRAMED_ASPECT,
        );
        assert!(
            on_attacker_side.swing_deg < 1e-3,
            "no swing to reach the other shoulder"
        );
    }

    #[test]
    fn coincident_fighters_do_not_break_the_framing() {
        let spot = body(1.0, 1.0);
        let shot = action_shot(
            &style(),
            &current(10.0),
            &spot,
            Some(&spot),
            &[],
            &[],
            FRAMED_ASPECT,
        );
        assert!(shot.pose.validate().is_ok());
        assert!(in_safe_area(&shot.pose, FRAMED_ASPECT, &spot));
    }

    #[test]
    fn a_body_behind_the_camera_is_not_in_the_safe_area() {
        let pose = CameraPose {
            target: Vec3::ZERO,
            distance: 5.0,
            yaw_deg: 0.0,
            pitch_deg: 10.0,
            fov_deg: 38.0,
        };
        assert!(!in_safe_area(&pose, FRAMED_ASPECT, &body(0.0, 50.0)));
    }

    /// The test the rules exist for: on every scene and layout, for every attacker and target cell
    /// pair, both fighters stay inside the safe area at the start of the beat, at contact and at
    /// the end, from the one pose the camera holds throughout.
    #[test]
    fn both_fighters_are_in_frame_for_every_cell_pair_on_every_scene_and_layout() {
        use crate::battle::{Cell, embedded_ids, load_embedded};
        let mut checked = 0u64;
        let mut worst_distance = 0.0f32;
        for id in embedded_ids() {
            let scene = load_embedded(id).unwrap().unwrap();
            let style = scene.cameras[&crate::battle::ShotName::TurnFocus];
            let mut grids = vec![scene.grid.clone()];
            grids.extend(
                scene
                    .layouts
                    .iter()
                    .map(|layout| scene.grid.with_layout(layout)),
            );
            for grid in grids {
                let cells: Vec<Cell> = grid.walkable_cells().collect();
                for (index, attacker_cell) in cells.iter().enumerate() {
                    for target_cell in cells.iter().skip(index % 12).step_by(12) {
                        if attacker_cell == target_cell {
                            continue;
                        }
                        let stand = grid
                            .free_cell_beside(*target_cell, *attacker_cell, &[])
                            .unwrap_or(*attacker_cell);
                        let body = |cell: Cell, height: f32| Body {
                            feet: grid.cell_centre(cell),
                            height_m: height,
                        };
                        let (attacker, target) =
                            (body(*attacker_cell, 1.35), body(*target_cell, 2.1));
                        // Where each stands during the beat: the attacker at contact, the target
                        // pushed back a little by the blow.
                        let at_contact = body(stand, 1.35);
                        let line = (target.feet - attacker.feet).normalized();
                        let recoiled = Body {
                            feet: target.feet + line * 0.5,
                            ..target
                        };
                        let also = [at_contact, recoiled];
                        for yaw in [-120.0, 160.0] {
                            let shot = action_shot(
                                &style,
                                &CameraPose {
                                    yaw_deg: yaw,
                                    ..style
                                },
                                &attacker,
                                Some(&target),
                                &also,
                                &[],
                                FRAMED_ASPECT,
                            );
                            worst_distance = worst_distance.max(shot.pose.distance);
                            for moment in [attacker, at_contact, target, recoiled] {
                                assert!(
                                    in_safe_area(&shot.pose, FRAMED_ASPECT, &moment),
                                    "{id}: {attacker_cell:?} -> {target_cell:?} from yaw {yaw}: {shot:?}"
                                );
                            }
                            assert!(
                                shot.swing_deg <= MAX_SWING_DEG + 1e-3,
                                "{id}: {}",
                                shot.swing_deg
                            );
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert!(checked > 1_500, "{checked} shots checked");
        assert!(
            worst_distance < MAX_DISTANCE_M,
            "the farthest pair still fits: {worst_distance}"
        );
    }

    /// The live defect: a self action with a bystander on the camera line must frame the actor
    /// clear of every other figure, fully inside the safe area.
    #[test]
    fn a_self_action_is_framed_clear_of_a_bystander_standing_on_the_camera_line() {
        let actor = body(-2.1, -0.7);
        let goblin = body(-0.7, -2.1);
        let bystander = body(0.7, 2.1);
        // The camera arrives from the +z side, exactly where the bystander stands.
        let current = CameraPose {
            target: Vec3::new(-0.8, 0.7, -0.9),
            distance: 11.5,
            yaw_deg: 32.0,
            pitch_deg: 29.0,
            fov_deg: 40.0,
        };
        let bystanders = [goblin, bystander];
        let shot = action_shot(
            &style(),
            &current,
            &actor,
            None,
            &[],
            &bystanders,
            FRAMED_ASPECT,
        );
        assert!(in_safe_area(&shot.pose, FRAMED_ASPECT, &actor), "{shot:?}");
        assert!(
            !bystander_in_front(&shot.pose, FRAMED_ASPECT, &[actor], &bystanders),
            "no figure stands in front of the actor: {shot:?}"
        );
        // And the shot taken without the check would have been blocked, so the test has teeth.
        let blind = action_shot(&style(), &current, &actor, None, &[], &[], FRAMED_ASPECT);
        let _ = blind;
        let in_the_way = CameraPose {
            yaw_deg: heading_deg(actor.feet, bystander.feet),
            ..shot.pose
        };
        assert!(bystander_in_front(
            &in_the_way,
            FRAMED_ASPECT,
            &[actor],
            &[bystander]
        ));
    }

    /// The other live defect: an attack with a third figure nearby keeps the attacker and the
    /// target fully in the safe area with nobody standing in front of either.
    #[test]
    fn an_attack_with_a_third_figure_nearby_keeps_both_fighters_clear_and_in_frame() {
        let attacker = body(-2.1, -0.7);
        let target = body(-0.7, -2.1);
        let bystander = body(0.7, 2.1);
        for yaw in (-180..180).step_by(15) {
            let current = CameraPose {
                yaw_deg: yaw as f32,
                ..style()
            };
            let shot = action_shot(
                &style(),
                &current,
                &attacker,
                Some(&target),
                &[],
                &[bystander],
                FRAMED_ASPECT,
            );
            for fighter in [attacker, target] {
                assert!(
                    in_safe_area(&shot.pose, FRAMED_ASPECT, &fighter),
                    "yaw {yaw}: {shot:?}"
                );
            }
            assert!(
                !bystander_in_front(&shot.pose, FRAMED_ASPECT, &[attacker, target], &[bystander]),
                "yaw {yaw}: {shot:?}"
            );
            // The bystander is never what the camera is aimed at.
            let aim = shot.pose.target;
            let midpoint = (attacker.feet + target.feet) * 0.5;
            assert!(
                ((aim.x - midpoint.x).hypot(aim.z - midpoint.z)) < 1.2,
                "yaw {yaw}"
            );
        }
    }

    #[test]
    fn a_bystander_right_beside_a_fighter_never_ends_up_centred_or_hiding_it() {
        let actor = body(0.0, 0.0);
        let others = [body(0.4, 0.5), body(-0.5, 0.7), body(0.1, -0.9)];
        for yaw in (-180..180).step_by(30) {
            let current = CameraPose {
                yaw_deg: yaw as f32,
                ..style()
            };
            let shot = action_shot(
                &style(),
                &current,
                &actor,
                None,
                &[],
                &others,
                FRAMED_ASPECT,
            );
            assert!(in_safe_area(&shot.pose, FRAMED_ASPECT, &actor), "yaw {yaw}");
        }
    }

    /// Tonight's defect: attacker and target stand on one line (here along z) and the camera sat
    /// on that line, so the attacker's body covered a foe a third of his height. The chosen angle
    /// is off the line and the two silhouettes are separate.
    #[test]
    fn a_small_foe_on_the_attackers_line_is_not_covered_by_him() {
        let brakka = body(-0.7, 2.1);
        let weasel = Body {
            feet: Vec3::new(-0.7, 0.0, -2.1),
            height_m: 0.6,
        };
        // Camera on the line, behind the weasel looking at Brakka: the worst case.
        for current_yaw in [180.0, 0.0, 90.0, -90.0] {
            let current = CameraPose {
                yaw_deg: current_yaw,
                ..style()
            };
            let shot = action_shot(
                &style(),
                &current,
                &brakka,
                Some(&weasel),
                &[],
                &[],
                FRAMED_ASPECT,
            );
            assert!(
                in_safe_area(&shot.pose, FRAMED_ASPECT, &brakka),
                "{current_yaw}"
            );
            assert!(
                in_safe_area(&shot.pose, FRAMED_ASPECT, &weasel),
                "{current_yaw}"
            );
            let a = project_body(&shot.pose, FRAMED_ASPECT, &brakka).unwrap();
            let b = project_body(&shot.pose, FRAMED_ASPECT, &weasel).unwrap();
            assert!(
                covered_share(a, b) <= MAX_PAIR_COVER + 1e-4,
                "{current_yaw}: {shot:?}"
            );
        }
    }

    #[test]
    fn a_pan_that_would_lose_the_fighters_midway_is_reported() {
        let near = CameraPose {
            target: Vec3::new(0.0, 1.0, 0.0),
            distance: 10.0,
            yaw_deg: 0.0,
            pitch_deg: 20.0,
            fov_deg: 38.0,
        };
        let far = CameraPose {
            target: Vec3::new(40.0, 1.0, 0.0),
            ..near
        };
        // From a shot on the origin to one 40 m aside, the middle looks at empty ground.
        assert!(!pan_keeps_in_view(
            &near,
            &far,
            &[body(0.0, 0.0), body(40.0, 0.0)],
            FRAMED_ASPECT
        ));
        // A pan that stays on the fighters is a pan.
        assert!(pan_keeps_in_view(
            &near,
            &near,
            &[body(0.0, 0.0)],
            FRAMED_ASPECT
        ));
    }

    /// The scan's fence stands behind the foe: a line along z must be shot from the front side,
    /// never from the far side of the set.
    #[test]
    fn the_camera_stays_in_front_of_the_set_even_when_the_line_points_away() {
        let attacker = body(-0.7, 2.1);
        let target = body(-0.7, -2.1);
        for current_yaw in [-170.0, -100.0, 0.0, 60.0, 170.0] {
            let current = CameraPose {
                yaw_deg: current_yaw,
                ..style()
            };
            let shot = action_shot_from(
                &style(),
                &current,
                &attacker,
                Some(&target),
                &[],
                &[],
                FRAMED_ASPECT,
                Some(0.0),
            );
            let heading = wrap_deg(shot.pose.yaw_deg);
            assert!(
                heading.abs() <= MAX_HOME_DEVIATION_DEG + 1e-3,
                "from {current_yaw}: {heading}"
            );
        }
    }
}
