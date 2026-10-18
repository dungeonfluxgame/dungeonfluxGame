//! Adapter between the shared display and the 3D battlefield in `df-render`.
//!
//! The live battlefield renders server cells, layout, attack styles and voice sets. Legacy views
//! and the isolated renderer preview retain a cosmetic formation fallback. No client rule decides
//! reachable cells, legal targets, cover, damage or outcomes.
//!
//! `#battle-preview[=<location-id>[:<layout-id>]][&splat][&restrained][&statuses][&sheet[=n]]`
//! mounts the battlefield alone and loops a scripted fight (it opens with the initiative roll and
//! roll-call) with no server calls, for looking at the renderer without playing a game.
//! `&statuses` puts every status on a figure in turn; `&sheet=n` shows six figures at once.
//!
//! Server tactical data supplies layout, cells, attack mode and voice. The retained legacy
//! SWITCH POINT fallbacks cover old checkpoints and the standalone preview: the layout choice
//! ([`chosen_layout_id`]), the attack style ([`attack_style_for`]), the voice set
//! (`df_render::battle::voice_for`, called by the stage) and the content rating
//! ([`content_rating`]).

use super::*;
use df_render::battle::{
    self, AttackResult, AttackStyle, Availability, BattleScene, BattleStage, Beat, Cell, HitPoints,
    Side, StageOptions, Status, Token, TokenId,
};
use gloo_timers::future::TimeoutFuture;
use std::{cell::Cell as StdCell, time::Duration};

const STYLE: &str = include_str!("live_battlefield.css");
const STYLE_ATTRIBUTE: &str = "data-battlefield-style";
const PREVIEW_HASH: &str = "#battle-preview";
/// The default tier of a streamed scan: the second largest, not the largest.
const DEFAULT_SPLAT_LOD: u32 = 1;
const DEFAULT_PREVIEW_HEROES: usize = 3;
const DEFAULT_PREVIEW_ENEMIES: usize = 1;
/// The most figures per side the preview stands without a layout to seat them.
const PREVIEW_MAX_PER_SIDE: usize = 6;
/// Standing height of a person-sized hero as staged for the TV, so a person reads
/// across a room; small creatures keep their own smaller heights.
const PERSON_HEIGHT_M: f32 = 1.72;
const BATTLEFIELD_BEAT_LOG_CAP: usize = 32;
const CARD_LOG_CAP: usize = 40;

thread_local! {
    static PREVIEW: RefCell<Option<BattleStage>> = const { RefCell::new(None) };
    static CAMERA_OVERRIDE: RefCell<Option<rpc::BattleCameraOverride>> = const { RefCell::new(None) };
    static CAMERA_APPLIED: RefCell<Option<(u64, String)>> = const { RefCell::new(None) };
}

/// Adds the battlefield rules once per page.
pub(super) fn install_style(document: &Document) -> Result<(), JsValue> {
    if document
        .query_selector(&format!("style[{STYLE_ATTRIBUTE}]"))?
        .is_some()
    {
        return Ok(());
    }
    let style = document.create_element("style")?;
    style.set_attribute(STYLE_ATTRIBUTE, "")?;
    style.set_text_content(Some(STYLE));
    let host = match document.query_selector("head")? {
        Some(head) => head,
        None => document
            .document_element()
            .ok_or_else(|| JsValue::from_str("document element absent"))?,
    };
    host.append_child(&style)?;
    Ok(())
}

/// What the preview shows: the scripted fight, one status after another on a figure, or a
/// contact sheet of six figures each with different statuses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreviewMode {
    Fight,
    Techniques,
    Statuses,
    Sheet(usize),
    /// The heroes and the bandit standing on the layout under the wide shot, for layout review.
    Still,
}

struct PreviewRequest {
    scene_id: String,
    layout_id: Option<String>,
    procedural_splats: bool,
    restrained: bool,
    /// `&place=<token>:<col>:<row>`: stands a token on a cell, to review one situation.
    places: Vec<(String, Cell)>,
    /// Frame the still closer (`&close`) to read figures against obstacles.
    close: bool,
    mode: PreviewMode,
    /// A splat scene served from `/assets/splat/` to use instead of the painted ground.
    splat_file: Option<(String, battle::SplatFormat)>,
    /// `&hold`: stage the figures and stop (no scripted fight), for stills and fitting.
    hold: bool,
    /// `&fixed[&budget=N][&minc=X][&rscale=S]`: hold the frame ladder for benchmarks.
    fixed: Option<battle::FixedQuality>,
    /// `&orbit` (with `hold`): turn the camera slowly about its target, for checking sort stability.
    orbit: bool,
    /// `&shot=<camera name>`: cut to one of the scene's named cameras.
    shot: Option<battle::ShotName>,
    /// `&cam=tx,ty,tz,distance,yaw,pitch,fov`: cut to a free camera.
    free_camera: Option<battle::CameraPose>,
    home_camera: Option<battle::CameraPose>,
    /// `&fps=24|30`: the presentation rate (30 when absent or invalid).
    fps: battle::BattleFps,
    /// `&beat=graze|heavy|critical|miss|finish`: stage one attack of that kind after the camera
    /// has idled a moment, then hold (review frames of the feel).
    demo: Option<String>,
    /// `&slow=0.1`: the stage clock at that share of real time, for slow-motion review strips.
    slow: Option<f32>,
    /// `&feel=off`: battle feel (shake, hit-stop, sway, blur, particles) off, for measurements.
    feel_off: bool,
    /// `&tempo=0..1000`: the tempo intensity the camera sway follows (the live game reads the
    /// server's tempo cue).
    tempo: Option<u16>,
    /// `&lod=0..3`: the tier of a streamed scan drawn as the finest (1 when absent, as in the
    /// live game).
    splat_lod: u32,
    /// `&heroes=n` and `&enemies=n`: how many of each stand on the field (3 and 1 when absent),
    /// up to what the layout seats.
    heroes: usize,
    enemies: usize,
    /// `&reach`: the first hero's full reach (every walkable cell within six, 30 ft) and every foe
    /// are shown as the server's reachable and target cells, as on a hero's turn.
    reach: bool,
    /// `&grid=1`: draw the cell grid (off by default, as on the TV).
    grid: bool,
}

fn preview_home_camera(text: &str) -> Option<battle::CameraPose> {
    let values: Vec<f32> = text
        .split(',')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let [pitch_deg, yaw_deg, distance, fov_deg] = values[..] else {
        return None;
    };
    let pose = battle::CameraPose {
        target: battle::Vec3::ZERO,
        pitch_deg,
        yaw_deg,
        distance,
        fov_deg,
    };
    pose.validate().ok().map(|()| pose)
}

fn free_camera(text: &str) -> Option<battle::CameraPose> {
    let values: Vec<f32> = text
        .split(',')
        .map(|part| part.parse::<f32>().ok())
        .collect::<Option<_>>()?;
    let [tx, ty, tz, distance, yaw_deg, pitch_deg, fov_deg] = values[..] else {
        return None;
    };
    let pose = battle::CameraPose {
        target: battle::Vec3::new(tx, ty, tz),
        distance,
        yaw_deg,
        pitch_deg,
        fov_deg,
    };
    pose.validate().ok().map(|()| pose)
}

fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 48
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn preview_request() -> Option<PreviewRequest> {
    let hash = web_sys::window()?.location().hash().ok()?;
    let rest = hash.strip_prefix(PREVIEW_HASH)?;
    let mut request = PreviewRequest {
        scene_id: battle::DEFAULT_SCENE_ID.to_owned(),
        layout_id: None,
        procedural_splats: false,
        restrained: false,
        places: Vec::new(),
        close: false,
        mode: PreviewMode::Fight,
        splat_file: None,
        hold: false,
        fixed: None,
        orbit: false,
        shot: None,
        free_camera: None,
        home_camera: None,
        fps: battle::BattleFps::DEFAULT,
        feel_off: false,
        tempo: None,
        splat_lod: DEFAULT_SPLAT_LOD,
        heroes: DEFAULT_PREVIEW_HEROES,
        enemies: DEFAULT_PREVIEW_ENEMIES,
        reach: false,
        grid: false,
        demo: None,
        slow: None,
    };
    for part in rest.split('&') {
        match part.strip_prefix('=').unwrap_or(part) {
            "" => {}
            "splat" => request.procedural_splats = true,
            "hold" => request.hold = true,
            "orbit" => request.orbit = true,
            "fixed" => {
                request.fixed.get_or_insert(battle::FixedQuality {
                    splat_budget: None,
                    min_contribution: None,
                    render_scale: None,
                    splat_scale: None,
                    debug_switches: 0,
                });
            }
            other if other.starts_with("budget=") => {
                let fixed = request.fixed.get_or_insert(battle::FixedQuality {
                    splat_budget: None,
                    min_contribution: None,
                    render_scale: None,
                    splat_scale: None,
                    debug_switches: 0,
                });
                fixed.splat_budget = other[7..].parse().ok();
            }
            other if other.starts_with("minc=") => {
                let fixed = request.fixed.get_or_insert(battle::FixedQuality {
                    splat_budget: None,
                    min_contribution: None,
                    render_scale: None,
                    splat_scale: None,
                    debug_switches: 0,
                });
                fixed.min_contribution = other[5..].parse().ok();
            }
            other if other.starts_with("dbg=") => {
                let fixed = request.fixed.get_or_insert(battle::FixedQuality {
                    splat_budget: None,
                    min_contribution: None,
                    render_scale: None,
                    splat_scale: None,
                    debug_switches: 0,
                });
                fixed.debug_switches = other[4..].parse().unwrap_or(0);
            }
            other if other.starts_with("sscale=") => {
                let fixed = request.fixed.get_or_insert(battle::FixedQuality {
                    splat_budget: None,
                    min_contribution: None,
                    render_scale: None,
                    splat_scale: None,
                    debug_switches: 0,
                });
                fixed.splat_scale = other[7..].parse().ok();
            }
            other if other.starts_with("rscale=") => {
                let fixed = request.fixed.get_or_insert(battle::FixedQuality {
                    splat_budget: None,
                    min_contribution: None,
                    render_scale: None,
                    splat_scale: None,
                    debug_switches: 0,
                });
                fixed.render_scale = other[7..].parse().ok();
            }
            other if other.starts_with("shot=") => {
                request.shot = battle::ShotName::from_key(&other[5..]);
            }
            other if other.starts_with("cam=") => {
                request.home_camera = preview_home_camera(&other[4..]);
                request.free_camera = free_camera(&other[4..]);
            }
            "feel=off" => request.feel_off = true,
            other if other.starts_with("beat=") && plain_name(&other[5..]) => {
                request.demo = Some(other[5..].to_owned());
            }
            other if other.starts_with("slow=") => request.slow = other[5..].parse().ok(),
            other if other.starts_with("tempo=") => request.tempo = other[6..].parse().ok(),
            other if other.starts_with("heroes=") => {
                request.heroes = other[7..].parse().unwrap_or(DEFAULT_PREVIEW_HEROES);
            }
            other if other.starts_with("enemies=") => {
                request.enemies = other[8..].parse().unwrap_or(DEFAULT_PREVIEW_ENEMIES);
            }
            other if other.starts_with("lod=") => {
                request.splat_lod = other[4..].parse().unwrap_or(DEFAULT_SPLAT_LOD);
            }
            other if other.starts_with("fps=") => {
                request.fps = other[4..]
                    .parse::<u32>()
                    .ok()
                    .and_then(battle::BattleFps::from_hz)
                    .unwrap_or_default();
            }
            "restrained" => request.restrained = true,
            "reach" => request.reach = true,
            "grid=1" => request.grid = true,
            "techniques" => request.mode = PreviewMode::Techniques,
            "statuses" => request.mode = PreviewMode::Statuses,
            "still" => request.mode = PreviewMode::Still,
            "close" => request.close = true,
            other if other.starts_with("place=") => {
                let fields: Vec<&str> = other[6..].split(':').collect();
                if let [id, col, row] = fields[..]
                    && let (Ok(col), Ok(row)) = (col.parse::<i32>(), row.parse::<i32>())
                    && plain_name(id)
                {
                    request.places.push((id.to_owned(), Cell::new(col, row)));
                }
            }
            "sheet" => request.mode = PreviewMode::Sheet(0),
            other if other.starts_with("sheet=") => {
                if let Ok(page) = other[6..].parse::<usize>() {
                    request.mode = PreviewMode::Sheet(page);
                }
            }
            other if other.starts_with("sog=") && plain_name(&other[4..]) => {
                request.splat_file = Some((
                    format!("/assets/splat/{}/meta.json", &other[4..]),
                    battle::SplatFormat::Sog,
                ));
            }
            other if other.starts_with("splatfile=") && plain_name(&other[10..]) => {
                request.splat_file = Some((
                    format!("/assets/splat/{}.splat", &other[10..]),
                    battle::SplatFormat::Splat,
                ));
            }
            // `<location-id>` or `<location-id>:<layout-id>`.
            other => {
                let (scene, layout) = match other.split_once(':') {
                    Some((scene, layout)) => (scene, Some(layout)),
                    None => (other, None),
                };
                if battle::embedded_ids().any(|known| known == scene) {
                    request.scene_id = scene.to_owned();
                    request.layout_id = layout.filter(|id| plain_name(id)).map(str::to_owned);
                }
            }
        }
    }
    Some(request)
}

/// Whether the page was opened as `#battle-preview` and should show only the battlefield.
pub(super) fn preview_requested() -> bool {
    preview_request().is_some()
}

fn combatant(
    id: &str,
    name: &str,
    side: Side,
    cell: Cell,
    figure: &str,
    portrait: &str,
    height_m: f32,
) -> Token {
    Token {
        id: TokenId::new(id),
        side,
        name: name.to_owned(),
        cell,
        height_m,
        figure: Some(figure.to_owned()),
        portrait: Some(portrait.to_owned()),
        hit_points: Some(HitPoints {
            current: 12,
            maximum: 12,
        }),
        statuses: Vec::new(),
        attack: attack_style_for(figure),
        voice: None,
    }
}

/// Legacy/preview attack style fallback; tactical figures use the server attack_style field.
/// The preview cast is Tobin the javelin thrower, Sylvie the archer, Nimrael in melee.
fn attack_style_for(figure: &str) -> AttackStyle {
    match figure {
        "figure-hero-3" => AttackStyle::Ranged,
        "figure-hero-1" => AttackStyle::Thrown,
        _ => AttackStyle::Melee,
    }
}

const PREVIEW_FOE: (&str, &str, &str) = (
    "bandit",
    "sprite-bandit-documentary-photo",
    "sprite-bandit-documentary-photo",
);

/// How many figures per side a preview can stand on `layout`: what it seats, never fewer than one.
fn preview_limit(layout: Option<&battle::Layout>) -> usize {
    layout
        .map_or(PREVIEW_MAX_PER_SIDE, |layout| {
            layout.heroes.len().min(layout.enemies.len())
        })
        .clamp(1, PREVIEW_MAX_PER_SIDE.max(4))
}

/// The preview's heroes (Tobin the fighter, Sylvie the archer, Nimrael, then more of the same
/// figures) and bandits on a layout's spawn cells (or the derived formation when the scene has no
/// layouts). `heroes` and `enemies` are clamped to what the layout seats.
fn preview_tokens(
    grid: &battle::Grid,
    layout: Option<&battle::Layout>,
    heroes: usize,
    enemies: usize,
) -> Result<Vec<Token>, JsValue> {
    let limit = preview_limit(layout);
    let (heroes, enemies) = (heroes.clamp(1, limit), enemies.clamp(1, limit));
    let seats = match layout {
        Some(layout) => battle::from_layout(layout, grid, heroes, enemies),
        None => battle::derive_formation(grid, heroes, enemies),
    }
    .map_err(|_| JsValue::from_str("battlefield formation has no room"))?;
    const CAST: [(&str, &str, &str, &str); 3] = [
        ("tobin", "Tobin", "figure-hero-1", "portrait-hero-1"),
        ("sylvie", "Sylvie", "figure-hero-3", "portrait-hero-3"),
        ("nimrael", "Nimrael", "figure-hero-2", "portrait-hero-2"),
    ];
    let mut tokens = Vec::new();
    for (index, cell) in seats.heroes.iter().enumerate() {
        let (id, name, figure, portrait) = CAST[index % CAST.len()];
        let (id, name) = if index < CAST.len() {
            (id.to_owned(), name.to_owned())
        } else {
            (
                format!("{id}-{}", index / CAST.len() + 1),
                format!("{name} {}", index / CAST.len() + 1),
            )
        };
        tokens.push(combatant(
            &id,
            &name,
            Side::Hero,
            *cell,
            figure,
            portrait,
            PERSON_HEIGHT_M,
        ));
    }
    for (index, cell) in seats.enemies.iter().enumerate() {
        let (id, name) = if index == 0 {
            (PREVIEW_FOE.0.to_owned(), "Bandit".to_owned())
        } else {
            (
                format!("{}-{}", PREVIEW_FOE.0, index + 1),
                format!("Bandit {}", index + 1),
            )
        };
        tokens.push(combatant(
            &id,
            &name,
            Side::Enemy,
            *cell,
            PREVIEW_FOE.1,
            PREVIEW_FOE.2,
            PERSON_HEIGHT_M,
        ));
    }
    Ok(tokens)
}

/// Whether the request uses the preview's default cast, which the recorded fight is written for.
fn default_cast(request: &PreviewRequest) -> bool {
    request.heroes == DEFAULT_PREVIEW_HEROES && request.enemies == DEFAULT_PREVIEW_ENEMIES
}

fn initiative_for(request: &PreviewRequest, tokens: Vec<Token>) -> Vec<battle::InitiativeRoll> {
    if default_cast(request) {
        preview_initiative()
    } else {
        generated_initiative(&tokens)
    }
}

fn script_for(request: &PreviewRequest, tokens: &[Token]) -> Vec<(u32, Beat)> {
    if default_cast(request) {
        preview_script()
    } else {
        generated_script(tokens)
    }
}

/// Initiative for any head count: the heroes and enemies alternate, highest first.
fn generated_initiative(tokens: &[Token]) -> Vec<battle::InitiativeRoll> {
    let heroes: Vec<&Token> = tokens.iter().filter(|t| t.side == Side::Hero).collect();
    let enemies: Vec<&Token> = tokens.iter().filter(|t| t.side == Side::Enemy).collect();
    let mut order: Vec<&Token> = Vec::new();
    for index in 0..heroes.len().max(enemies.len()) {
        order.extend(heroes.get(index));
        order.extend(enemies.get(index));
    }
    order
        .iter()
        .enumerate()
        .map(|(index, token)| {
            let total = 20 - index as i32;
            battle::InitiativeRoll {
                token: token.id.clone(),
                die: (total - 2).clamp(1, 20) as u32,
                modifier: 2,
                total,
            }
        })
        .collect()
}

/// A fight for any head count: every enemy strikes a hero, every hero strikes an enemy, then each
/// enemy in turn is brought down.
/// Fixed display fixtures, independent of live rules. The cast never drops, so a complete
/// technique cycle can be watched on either a collider-backed world or an external scan.
fn technique_script(tokens: &[Token]) -> Vec<(&'static str, AttackStyle, Beat, RollCard)> {
    let Some(caster) = tokens.first() else {
        return Vec::new();
    };
    let Some(target) = tokens.iter().find(|token| token.side != caster.side) else {
        return Vec::new();
    };
    let card = |value: &str, label: String, explanation: &str, verdict: &str| RollCard {
        value: value.to_owned(),
        label,
        explanation: explanation.to_owned(),
        verdict: verdict.to_owned(),
        tone: "hit".to_owned(),
    };
    let resolved = |target: &Token,
                    damage,
                    face,
                    damage_faces: Vec<u32>,
                    attack_card: RollCard,
                    final_card: RollCard| battle::StrikeDisplay {
        target: target.id.clone(),
        result: AttackResult::Hit {
            damage,
            critical: false,
            downed: false,
        },
        target_hit_points_after: None,
        card: Some(final_card),
        roll: Some(battle::ResolvedRoll {
            d20: face,
            damage_faces,
            attack_card: face.map(|_| attack_card),
        }),
    };
    let swing = |index| {
        resolved(
            target,
            8,
            Some(12),
            vec![5],
            card(
                "19",
                format!("{} | Longsword {index}", caster.name),
                "Melee | d20 12 + ability 4 + proficiency 3 = 19 vs AC 15",
                "HIT",
            ),
            card(
                "19",
                format!("{} | Longsword {index}", caster.name),
                "d20 12 + ability 4 + proficiency 3 = 19 vs AC 15 | damage 5 + 3 = 8 slashing",
                "HIT",
            ),
        )
    };
    let flurry = |count, surge| Beat::Flurry {
        attacker: caster.id.clone(),
        strikes: (1..=count).map(swing).collect(),
        action_surge_at: surge,
    };
    let heading = card("", String::new(), "", "");
    let bolt = resolved(
        target,
        13,
        Some(14),
        vec![6, 7],
        card(
            "21",
            format!("{} | Fire Bolt", caster.name),
            "Ranged spell | d20 14 + ability 4 + proficiency 3 = 21 vs AC 15",
            "HIT",
        ),
        card(
            "21",
            format!("{} | Fire Bolt", caster.name),
            "d20 14 + 7 = 21 vs AC 15 | damage 6 + 7 = 13 fire",
            "HIT",
        ),
    );
    let mut script = vec![
        (
            "Extra Attack",
            AttackStyle::Melee,
            flurry(2, None),
            heading.clone(),
        ),
        (
            "Action Surge",
            AttackStyle::Melee,
            flurry(4, Some(2)),
            heading.clone(),
        ),
        (
            "Fire Bolt",
            AttackStyle::Bolt,
            Beat::Flurry {
                attacker: caster.id.clone(),
                strikes: vec![bolt],
                action_surge_at: None,
            },
            heading.clone(),
        ),
    ];
    let marks: Vec<_> = tokens
        .iter()
        .filter(|token| token.id != caster.id)
        .collect();
    let darts = (0..3)
        .map(|index| {
            let mark = marks[index % marks.len()];
            let final_card = card(
                "4",
                format!(
                    "{} to {} | Magic Missile dart {}",
                    caster.name,
                    mark.name,
                    index + 1
                ),
                "damage 3 + 1 = 4 force",
                "AUTOMATIC HIT",
            );
            resolved(mark, 4, None, vec![3], heading.clone(), final_card)
        })
        .collect();
    script.push((
        "Magic Missile",
        AttackStyle::Missiles,
        Beat::Missiles {
            caster: caster.id.clone(),
            darts,
        },
        heading.clone(),
    ));
    let targets = marks
        .iter()
        .enumerate()
        .map(|(index, mark)| {
            let saved = index % 2 == 0;
            resolved(
                mark,
                if saved { 14 } else { 28 },
                Some(if saved { 15 } else { 6 }),
                vec![3, 4, 5, 2, 6, 1, 3, 4],
                card(
                    if saved { "18" } else { "9" },
                    format!("{} | Dexterity save", mark.name),
                    "d20 + Dexterity 3 vs DC 15",
                    if saved { "SAVED" } else { "FAILED" },
                ),
                card(
                    if saved { "14" } else { "28" },
                    format!("{} | Fireball", mark.name),
                    "3 + 4 + 5 + 2 + 6 + 1 + 3 + 4 = 28 fire",
                    if saved {
                        "SAVED | HALF DAMAGE 14"
                    } else {
                        "FAILED | DAMAGE 28"
                    },
                ),
            )
        })
        .collect();
    script.push((
        "Fireball",
        AttackStyle::Burst,
        Beat::AreaEffect {
            caster: caster.id.clone(),
            centre: target.cell,
            radius_m: 6.1,
            targets,
        },
        heading.clone(),
    ));
    let shield = battle::StrikeDisplay {
        target: target.id.clone(),
        result: AttackResult::Miss,
        target_hit_points_after: None,
        card: Some(card(
            "19",
            format!("{} | Shield", target.name),
            "d20 12 + 7 = 19 vs Shield AC 20",
            "MISS | TURNED ASIDE",
        )),
        roll: Some(battle::ResolvedRoll {
            d20: Some(12),
            damage_faces: Vec::new(),
            attack_card: None,
        }),
    };
    script.push((
        "Shield",
        AttackStyle::Melee,
        Beat::Shield {
            attacker: caster.id.clone(),
            defender: target.id.clone(),
            display: Some(Box::new(shield)),
        },
        heading,
    ));
    script
}

fn generated_script(tokens: &[Token]) -> Vec<(u32, Beat)> {
    let heroes: Vec<&Token> = tokens.iter().filter(|t| t.side == Side::Hero).collect();
    let enemies: Vec<&Token> = tokens.iter().filter(|t| t.side == Side::Enemy).collect();
    let attack =
        |attacker: &Token, target: &Token, damage: u32, downed: bool, after: u32| Beat::Attack {
            attacker: attacker.id.clone(),
            target: target.id.clone(),
            result: AttackResult::Hit {
                damage,
                critical: false,
                downed,
            },
            target_hit_points_after: Some(after),
        };
    let mut script = Vec::new();
    for (index, enemy) in enemies.iter().enumerate() {
        script.push((1, attack(enemy, heroes[index % heroes.len()], 2, false, 10)));
    }
    for (index, hero) in heroes.iter().enumerate() {
        script.push((2, attack(hero, enemies[index % enemies.len()], 3, false, 9)));
    }
    for (index, enemy) in enemies.iter().enumerate() {
        script.push((3, attack(heroes[index % heroes.len()], enemy, 12, true, 0)));
    }
    script
}

/// The recorded rolls the preview shows, first to act first: Sylvie 20, Tobin 15, then the goblin
/// and Nimrael tied at 13 with the goblin ahead (a tie the "server" resolved).
fn preview_initiative() -> Vec<battle::InitiativeRoll> {
    let roll = |id: &str, die, modifier| battle::InitiativeRoll {
        token: TokenId::new(id),
        die,
        modifier,
        total: die as i32 + modifier,
    };
    vec![
        roll("sylvie", 17, 3),
        roll("tobin", 14, 1),
        roll("bandit", 11, 2),
        roll("nimrael", 10, 3),
    ]
}

/// The looping preview fight, staged at range so distance reads: Tobin throws a
/// javelin (miss, then a hit, then the finishing throw), Sylvie looses bow shots
/// (a hit and a miss that flies past), Nimrael and the bandit trade melee blows,
/// and a heal. Thrown and bow flights scale with the gap between the figures.
fn preview_script() -> Vec<(u32, Beat)> {
    let attack = |attacker: &str, target: &str, result, after| Beat::Attack {
        attacker: TokenId::new(attacker),
        target: TokenId::new(target),
        result,
        target_hit_points_after: after,
    };
    let hit = |damage, downed| AttackResult::Hit {
        damage,
        critical: false,
        downed,
    };
    vec![
        (1, attack("tobin", "bandit", AttackResult::Miss, None)),
        (1, attack("bandit", "nimrael", hit(3, false), Some(9))),
        (2, attack("sylvie", "bandit", hit(4, false), Some(8))),
        (2, attack("nimrael", "bandit", hit(5, false), Some(3))),
        (
            2,
            Beat::Heal {
                source: TokenId::new("tobin"),
                target: TokenId::new("nimrael"),
                amount: 3,
                target_hit_points_after: Some(12),
            },
        ),
        (3, attack("bandit", "tobin", hit(2, false), Some(10))),
        (3, attack("sylvie", "bandit", AttackResult::Miss, None)),
        (3, attack("tobin", "bandit", hit(3, true), Some(0))),
    ]
}

/// Single statuses first, then a few combinations that exercise the combination rules.
fn status_cycle() -> Vec<Vec<Status>> {
    let mut cycle: Vec<Vec<Status>> = Status::ALL.iter().map(|status| vec![*status]).collect();
    cycle.extend([
        vec![Status::Petrified, Status::Poisoned],
        vec![Status::Invisible, Status::Burning],
        vec![Status::Prone, Status::Poisoned],
        vec![Status::Frightened, Status::Blessed],
        vec![Status::Restrained, Status::Bloodied],
    ]);
    cycle
}

fn status_name(set: &[Status]) -> String {
    set.iter()
        .map(|status| status.id())
        .collect::<Vec<_>>()
        .join("+")
}

fn pick_layout(scene: &BattleScene, requested: Option<&str>, cycle: usize) -> Option<String> {
    if let Some(id) = requested
        && scene.layouts.iter().any(|layout| layout.id == id)
    {
        return Some(id.to_owned());
    }
    // No layout asked for: cycle through them, one per loop of the fight.
    (!scene.layouts.is_empty()).then(|| scene.layouts[cycle % scene.layouts.len()].id.clone())
}

fn with_preview<R>(operation: impl FnOnce(&mut battle::Stage) -> R) -> Option<R> {
    PREVIEW.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(|stage| stage.with_stage(operation))
    })
}

fn mark(host: &Element, name: &str, value: &str) {
    // Best-effort test hook: a harness reads these to time its captures.
    let _ = host.set_attribute(name, value);
}

/// Waits until the stage has finished everything queued; `false` when it went away.
async fn wait_idle() -> bool {
    loop {
        TimeoutFuture::new(100).await;
        match with_preview(|stage| stage.is_busy()) {
            Some(true) => {}
            Some(false) => return true,
            None => return false,
        }
    }
}

/// Bounded same-origin asset load; timeout aborts the browser request as well as its waiter.
async fn world_bytes(url: &str, maximum: u32) -> Option<Vec<u8>> {
    use wasm_bindgen_futures::JsFuture;
    let controller = web_sys::AbortController::new().ok()?;
    let options = web_sys::RequestInit::new();
    options.set_signal(Some(&controller.signal()));
    let request = web_sys::Request::new_with_str_and_init(url, &options).ok()?;
    let fetch = async {
        let response: web_sys::Response =
            JsFuture::from(web_sys::window()?.fetch_with_request(&request))
                .await
                .ok()?
                .dyn_into()
                .ok()?;
        if !response.ok() {
            return None;
        }
        let length = response
            .headers()
            .get("content-length")
            .ok()??
            .parse::<u32>()
            .ok()?;
        if length == 0 || length > maximum {
            return None;
        }
        let buffer = JsFuture::from(response.array_buffer().ok()?).await.ok()?;
        let bytes = js_sys::Uint8Array::new(&buffer);
        (bytes.length() > 0 && bytes.length() <= maximum).then(|| bytes.to_vec())
    };
    let result = match futures::future::select(
        Box::pin(fetch),
        Box::pin(TimeoutFuture::new(10_000)),
    )
    .await
    {
        futures::future::Either::Left((bytes, _)) => bytes,
        futures::future::Either::Right(_) => None,
    };
    controller.abort();
    result
}

async fn preview_world() -> Option<rpc::BattleWorld> {
    let bytes = world_bytes("/assets/splat/battle-preview.json", 4096).await?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    Some(rpc::BattleWorld {
        ply_url: value.get("ply_url")?.as_str()?.to_owned(),
        collider_url: value.get("collider_url")?.as_str()?.to_owned(),
        metric_scale_factor: value.get("metric_scale_factor")?.as_f64()?,
        ground_plane_offset: value.get("ground_plane_offset")?.as_f64()?,
    })
}

/// An open square nearest the centre of the largest edge-connected open area. Ties are stable.
fn open_floor_centre(grid: &battle::WorldGrid) -> Option<[f32; 2]> {
    let mut seen = std::collections::HashSet::new();
    let mut largest = Vec::new();
    for row in 0..grid.rows() {
        for col in 0..grid.cols() {
            if grid.cell(col, row) != Some(battle::WorldCell::Open) || !seen.insert((col, row)) {
                continue;
            }
            let mut area = Vec::new();
            let mut stack = vec![(col, row)];
            while let Some((col, row)) = stack.pop() {
                area.push((col, row));
                for next in [
                    col.checked_sub(1).map(|c| (c, row)),
                    row.checked_sub(1).map(|r| (col, r)),
                    col.checked_add(1)
                        .filter(|c| *c < grid.cols())
                        .map(|c| (c, row)),
                    row.checked_add(1)
                        .filter(|r| *r < grid.rows())
                        .map(|r| (col, r)),
                ]
                .into_iter()
                .flatten()
                {
                    if grid.cell(next.0, next.1) == Some(battle::WorldCell::Open)
                        && seen.insert(next)
                    {
                        stack.push(next);
                    }
                }
            }
            if area.len() > largest.len() {
                largest = area;
            }
        }
    }
    let sum = largest.iter().fold([0.0, 0.0], |sum, &(col, row)| {
        [sum[0] + col as f32, sum[1] + row as f32]
    });
    let count = largest.len() as f32;
    let (col, row) = largest.into_iter().min_by(|a, b| {
        let distance = |&(col, row): &(u32, u32)| {
            (col as f32 - sum[0] / count).powi(2) + (row as f32 - sum[1] / count).powi(2)
        };
        distance(a)
            .total_cmp(&distance(b))
            .then_with(|| (a.1, a.0).cmp(&(b.1, b.0)))
    })?;
    grid.cell_centre(col, row)
}

fn world_mount_is_current(mount: Option<&Rc<()>>) -> bool {
    mount.is_none_or(|mount| {
        LIVE.with(|live| {
            let live = live.borrow();
            matches!(live.phase, Phase::Mounting)
                && live
                    .world_mount
                    .as_ref()
                    .is_some_and(|current| Rc::ptr_eq(current, mount))
        })
    })
}

async fn load_battle_world(
    host: &Element,
    scene: &mut BattleScene,
    world: &rpc::BattleWorld,
    layout_id: Option<&str>,
    mount: Option<&Rc<()>>,
) {
    mark(host, "data-battle-world", "loading");
    let result = async {
        if !world.ply_url.starts_with("/assets/splat/worlds/")
            || !world.ply_url.ends_with(".ply")
            || !world.collider_url.starts_with("/assets/splat/worlds/")
            || !world.collider_url.ends_with(".glb")
        {
            return Err("asset-path");
        }
        let mut transform = battle::marble_world_transform(
            world.metric_scale_factor as f32,
            world.ground_plane_offset as f32,
        )
        .map_err(|_| "coordinate-frame")?;
        let bytes = world_bytes(&world.collider_url, 32 * 1024 * 1024)
            .await
            .ok_or("collider-load")?;
        if !world_mount_is_current(mount) {
            return Err("cancelled");
        }
        let mesh = battle::ColliderMesh::from_glb(&bytes).map_err(|_| "collider-decode")?;
        let grid = battle::world_grid(
            &mesh,
            &transform,
            &battle::WorldGridSettings {
                cell_m: 0.5,
                ..Default::default()
            },
            None,
        )
        .map_err(|_| "collider-grid")?;
        let centre = open_floor_centre(&grid).ok_or("no-open-floor")?;
        let stage_centre = scene.grid.centre();
        transform.translate =
            battle::Vec3::new(stage_centre.x - centre[0], 0.0, stage_centre.z - centre[1]);
        let origin = scene.grid.origin();
        let comparison_grid = battle::world_grid(
            &mesh,
            &transform,
            &battle::WorldGridSettings {
                cell_m: scene.grid.cell_m(),
                footprint: Some(battle::WorldBounds {
                    min: origin,
                    max: [
                        origin[0] + scene.grid.cols() as f32 * scene.grid.cell_m(),
                        origin[1] + scene.grid.rows() as f32 * scene.grid.cell_m(),
                    ],
                }),
                ..Default::default()
            },
            None,
        )
        .map_err(|_| "comparison-grid")?;
        let authored = scene
            .layouts
            .iter()
            .find(|layout| Some(layout.id.as_str()) == layout_id);
        let mut disagreement = 0;
        let mut compared = 0;
        let mut no_floor = 0;
        for row in 0..scene.grid.rows() {
            for col in 0..scene.grid.cols() {
                let cell = Cell::new(col as i32, row as i32);
                let scanned = comparison_grid.cell(col, row);
                let blocked = !scene.grid.is_walkable(cell)
                    || authored.is_some_and(|layout| {
                        layout.obstacles.iter().any(|obstacle| {
                            obstacle.blocks_movement && obstacle.cells.contains(&cell)
                        })
                    });
                match scanned {
                    Some(battle::WorldCell::Open | battle::WorldCell::Blocked) => {
                        compared += 1;
                        disagreement +=
                            u32::from(blocked != (scanned == Some(battle::WorldCell::Blocked)));
                    }
                    _ => no_floor += 1,
                }
            }
        }
        mark(
            host,
            "data-battle-world-layout",
            &format!("disagree={disagreement} compared={compared} no_floor={no_floor}"),
        );
        mark(
            host,
            "data-battle-world-placement",
            &format!(
                "x={} z={} scale={} floor={}",
                transform.translate.x, transform.translate.z, transform.scale, transform.offset_y
            ),
        );
        Ok(battle::SplatLayer {
            url: world.ply_url.clone(),
            format: battle::SplatFormat::Ply,
            scale: transform.scale,
            offset_y: transform.offset_y,
            rot_y_deg: transform.rot_y_deg,
            translate: transform.translate,
            orient: transform.orient,
            max_splats: None,
            levels: None,
            detail_radius_m: None,
            grade: battle::SplatGrade::NEUTRAL,
            depth_bias_m: 0.15,
            clip: None,
            rim_m: None,
        })
    }
    .await;
    if !world_mount_is_current(mount) {
        return;
    }
    match result {
        Ok(layer) => {
            scene.splat = Some(layer);
            mark(host, "data-battle-world", "placed");
        }
        Err(reason) => {
            scene.splat = None;
            mark(host, "data-battle-world", reason);
        }
    }
}

async fn run_preview(host: Element, request: PreviewRequest) {
    let mut scene = match battle::load_embedded(&request.scene_id) {
        Some(Ok(scene)) => scene,
        _ => return,
    };
    if let Some((url, format)) = request.splat_file.clone() {
        scene.splat = Some(battle::SplatLayer {
            url,
            format,
            scale: 1.0,
            offset_y: 0.0,
            rot_y_deg: 0.0,
            translate: battle::Vec3::ZERO,
            max_splats: None,
            orient: battle::Quat::IDENTITY,
            levels: None,
            detail_radius_m: None,
            grade: battle::SplatGrade::NEUTRAL,
            depth_bias_m: 0.15,
            clip: None,
            rim_m: None,
        });
    }
    if request.procedural_splats {
        match preview_world().await {
            Some(world) => {
                let layout = pick_layout(&scene, request.layout_id.as_deref(), 0);
                load_battle_world(&host, &mut scene, &world, layout.as_deref(), None).await;
            }
            None => mark(&host, "data-battle-world", "unavailable"),
        }
    }
    let scene_for_choice = scene.clone();
    mark(&host, "data-battlefield-scene", &request.scene_id);
    let stage = BattleStage::mount(
        &host,
        scene,
        StageOptions {
            procedural_splats: false,
            fixed_quality: request.fixed,
            fps: request.fps,
            feel_off: request.feel_off,
            slow: request.slow,
            splat_lod: request.splat_lod,
            show_grid: request.grid,
            ..StageOptions::default()
        },
    )
    .await;
    if let Availability::Unavailable(reason) = stage.availability() {
        show_unavailable(&host, reason.key());
        return;
    }
    stage.set_sound_sink(Rc::new(|event| {
        live_sfx::play_event(&event, content_rating());
    }));
    // A harness reads when each blow lands, from the same impact event the HUD work subscribes to.
    let impact_host = host.clone();
    stage.set_impact_sink(Rc::new(move |event| {
        mark(
            &impact_host,
            "data-battlefield-impact",
            &format!("{:?}:{:.2}", event.class, event.intensity),
        );
    }));
    PREVIEW.with(|slot| *slot.borrow_mut() = Some(stage));
    with_preview(|stage| {
        if let Some(pose) = request.home_camera {
            let pose = pose.aimed_at(stage.scene().home.target);
            if let Err(error) = stage.set_home_override(Some(pose)) {
                web_sys::console::error_1(&JsValue::from_str(&format!(
                    "invalid preview home: {error:?}"
                )));
            }
        }
        stage.set_restrained(
            request.restrained || content_rating() == battle::ContentRating::Restrained,
        )
    });
    if let Some(tempo) = request.tempo {
        with_preview(|stage| stage.set_tempo_intensity(tempo));
    }
    live_sfx::set_battlefield_owns_combat(true);
    // The shared sound control lets a harness (or a viewer) unlock audio with one click.
    if let Some(document) = host.owner_document() {
        let _ = live_voice::attach(&document, &host);
    }
    wasm_bindgen_futures::spawn_local(watch_availability(host.clone()));
    let beat_counter = StdCell::new(0u32);
    let mut cycle = 0usize;
    loop {
        let layout_id = pick_layout(&scene_for_choice, request.layout_id.as_deref(), cycle);
        cycle += 1;
        mark(
            &host,
            "data-battlefield-layout",
            layout_id.as_deref().unwrap_or("none"),
        );
        let prepared = with_preview(|stage| {
            if let Some(id) = &layout_id {
                stage.set_layout(id);
            }
            let layout = stage.layout().cloned();
            let tokens = match request.mode {
                PreviewMode::Fight
                | PreviewMode::Techniques
                | PreviewMode::Statuses
                | PreviewMode::Still => preview_tokens(
                    stage.grid(),
                    layout.as_ref(),
                    request.heroes,
                    request.enemies,
                ),
                PreviewMode::Sheet(page) => sheet_tokens(stage.grid(), layout.as_ref(), page),
            };
            tokens.map(|tokens| {
                stage.set_tokens(tokens);
                let order: Vec<TokenId> = match request.mode {
                    PreviewMode::Fight => {
                        initiative_for(&request, stage.tokens().cloned().collect())
                            .into_iter()
                            .map(|roll| roll.token)
                            .collect()
                    }
                    _ => stage.tokens().map(|token| token.id.clone()).collect(),
                };
                stage.set_initiative(order);
                stage.set_round(1);
                for (id, cell) in &request.places {
                    stage.place_token(&TokenId::new(id.clone()), *cell);
                }
                if request.reach {
                    let heroes: Vec<_> = stage
                        .tokens()
                        .filter(|token| token.side == Side::Hero)
                        .map(|token| token.cell)
                        .collect();
                    let foes: Vec<_> = stage
                        .tokens()
                        .filter(|token| token.side == Side::Enemy)
                        .map(|token| token.cell)
                        .collect();
                    if let Some(actor) = heroes.first() {
                        let reachable: Vec<_> = stage
                            .grid()
                            .walkable_cells()
                            .filter(|cell| {
                                (cell.col - actor.col)
                                    .abs()
                                    .max((cell.row - actor.row).abs())
                                    <= 6
                            })
                            .collect();
                        stage.set_tactical_highlights(reachable, foes);
                    }
                }
            })
        });
        match prepared {
            Some(Ok(())) => {}
            Some(Err(_)) => return,
            None => {
                // The stage became unavailable (device lost) or was disposed: nothing left to drive.
                if let Some(Availability::Unavailable(reason)) =
                    PREVIEW.with(|slot| slot.borrow().as_ref().map(BattleStage::availability))
                {
                    show_unavailable(&host, reason.key());
                }
                return;
            }
        }
        if let Some(kind) = request.demo.clone() {
            // One attack of the asked kind on a camera that has idled a moment, then hold.
            with_preview(|stage| stage.establish());
            TimeoutFuture::new(2500).await;
            if matches!(
                kind.as_str(),
                "speak" | "named" | "namedmiss" | "namedcrit" | "chain" | "victory" | "defeat"
            ) {
                mark(&host, "data-battlefield-demo", &kind);
                run_extra_demo(&host, &kind).await;
                return;
            }
            if kind == "rest" {
                // A hero's turn begins: the lean toward it, then the resting shot.
                let actor = preview_ids().0.first().cloned();
                with_preview(|stage| {
                    if let Some(actor) = actor {
                        let _ = stage.stage(Beat::TurnStart { actor });
                    }
                });
                mark(&host, "data-battlefield-demo", &kind);
                hold_preview().await;
                return;
            }
            if kind == "selfheal" {
                // A self action (Second Wind): the actor alone, clear of everyone else.
                with_preview(|stage| {
                    let _ = stage.stage(Beat::Heal {
                        source: TokenId::new("tobin"),
                        target: TokenId::new("tobin"),
                        amount: 5,
                        target_hit_points_after: Some(12),
                    });
                });
                mark(&host, "data-battlefield-demo", &kind);
                loop {
                    TimeoutFuture::new(1000).await;
                    if PREVIEW.with(|slot| slot.borrow().is_none()) {
                        return;
                    }
                }
            }
            if kind == "shoot" {
                // A ranged attack across the field: the archer shoots the goblin.
                with_preview(|stage| {
                    let _ = stage.stage(Beat::Attack {
                        attacker: TokenId::new("sylvie"),
                        target: TokenId::new("bandit"),
                        result: AttackResult::Hit {
                            damage: 4,
                            critical: false,
                            downed: false,
                        },
                        target_hit_points_after: Some(8),
                    });
                });
                mark(&host, "data-battlefield-demo", &kind);
                loop {
                    TimeoutFuture::new(1000).await;
                    if PREVIEW.with(|slot| slot.borrow().is_none()) {
                        return;
                    }
                }
            }
            if kind == "enemyhit" {
                // The enemy's turn: the goblin hits the hero.
                with_preview(|stage| {
                    let _ = stage.stage(Beat::Attack {
                        attacker: TokenId::new("bandit"),
                        target: TokenId::new("tobin"),
                        result: AttackResult::Hit {
                            damage: 3,
                            critical: false,
                            downed: false,
                        },
                        target_hit_points_after: Some(9),
                    });
                });
                mark(&host, "data-battlefield-demo", &kind);
                loop {
                    TimeoutFuture::new(1000).await;
                    if PREVIEW.with(|slot| slot.borrow().is_none()) {
                        return;
                    }
                }
            }
            let (result, after) = match kind.as_str() {
                "graze" => (
                    AttackResult::Hit {
                        damage: 1,
                        critical: false,
                        downed: false,
                    },
                    Some(6),
                ),
                "heavy" => (
                    AttackResult::Hit {
                        damage: 7,
                        critical: false,
                        downed: false,
                    },
                    Some(2),
                ),
                "critical" => (
                    AttackResult::Hit {
                        damage: 5,
                        critical: true,
                        downed: false,
                    },
                    Some(3),
                ),
                "finish" => (
                    AttackResult::Hit {
                        damage: 9,
                        critical: false,
                        downed: true,
                    },
                    Some(0),
                ),
                _ => (AttackResult::Miss, None),
            };
            with_preview(|stage| {
                let _ = stage.stage(Beat::Attack {
                    attacker: TokenId::new("tobin"),
                    target: TokenId::new("bandit"),
                    result,
                    target_hit_points_after: after,
                });
            });
            mark(&host, "data-battlefield-demo", &kind);
            loop {
                TimeoutFuture::new(1000).await;
                if PREVIEW.with(|slot| slot.borrow().is_none()) {
                    return;
                }
            }
        }
        if request.hold {
            // Stills: the figures stand and the stage stays as it is (or orbits, when asked).
            with_preview(|stage| {
                stage.establish();
                let named = request
                    .shot
                    .and_then(|shot| stage.scene().cameras.get(&shot).copied());
                if let Some(pose) = request.free_camera.or(named) {
                    stage.set_camera_pose(pose);
                }
            });
            TimeoutFuture::new(3000).await;
            let base = with_preview(|stage| stage.camera_pose());
            let mut turned_deg = 0.0;
            loop {
                TimeoutFuture::new(if request.orbit { 16 } else { 1000 }).await;
                if PREVIEW.with(|slot| slot.borrow().is_none()) {
                    return;
                }
                if let (true, Some(base)) = (request.orbit, base) {
                    turned_deg += 0.35;
                    let pose = battle::CameraPose {
                        yaw_deg: base.yaw_deg + turned_deg,
                        ..base
                    };
                    with_preview(|stage| stage.set_camera_pose(pose));
                }
            }
        }
        match request.mode {
            PreviewMode::Techniques => {
                with_preview(|stage| stage.establish());
                TimeoutFuture::new(1500).await;
                let tokens = with_preview(|stage| stage.tokens().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();
                for (name, style, beat, card) in technique_script(&tokens) {
                    mark(&host, "data-battlefield-phase", "techniques");
                    mark(&host, "data-battlefield-technique", name);
                    let id = super::presentation_timing::next_component_id();
                    battle_component(
                        Some(&host),
                        df_observe::presentation::PresentationComponent::Battlefield,
                        id,
                        &request.scene_id,
                        df_observe::presentation::ComponentPhase::Opened,
                        "technique_preview",
                    );
                    let queued = with_preview(|stage| {
                        if let Some(actor) = tokens.first() {
                            stage.set_attack_style(&actor.id, style);
                        }
                        // The director owns the contact timing. Its timeline inserts the committed
                        // card after the effect lands; opening it here would spoil the result.
                        let _ = card;
                        stage.stage(beat).is_ok()
                    });
                    if queued != Some(true) || !wait_idle().await {
                        return;
                    }
                    battle_component(
                        Some(&host),
                        df_observe::presentation::PresentationComponent::Battlefield,
                        id,
                        &request.scene_id,
                        df_observe::presentation::ComponentPhase::Hidden,
                        "technique_preview_complete",
                    );
                    TimeoutFuture::new(1200).await;
                }
            }
            PreviewMode::Fight => {
                with_preview(|stage| stage.establish());
                TimeoutFuture::new(1500).await;
                mark(&host, "data-battlefield-phase", "initiative");
                let rolls = with_preview(|stage| {
                    initiative_for(&request, stage.tokens().cloned().collect())
                })
                .unwrap_or_default();
                let queued = with_preview(|stage| stage.stage(Beat::Initiative { rolls }).is_ok());
                if queued != Some(true) || !wait_idle().await {
                    return;
                }
                mark(&host, "data-battlefield-phase", "fight");
                TimeoutFuture::new(700).await;
                let script = with_preview(|stage| {
                    script_for(&request, &stage.tokens().cloned().collect::<Vec<_>>())
                })
                .unwrap_or_default();
                for (round, beat) in script {
                    let queued =
                        with_preview(|stage| stage.stage_in_round(beat.clone(), round).is_ok());
                    if queued != Some(true) {
                        return;
                    }
                    beat_counter.set(beat_counter.get() + 1);
                    mark(
                        &host,
                        "data-battlefield-beat",
                        &beat_counter.get().to_string(),
                    );
                    if !wait_idle().await {
                        return;
                    }
                    TimeoutFuture::new(900).await;
                }
                TimeoutFuture::new(2500).await;
            }
            PreviewMode::Statuses => {
                let queued = with_preview(|stage| {
                    stage.set_active(Some(TokenId::new("tobin")));
                    stage
                        .stage(Beat::TurnStart {
                            actor: TokenId::new("tobin"),
                        })
                        .is_ok()
                });
                if queued != Some(true) || !wait_idle().await {
                    return;
                }
                for (index, set) in status_cycle().into_iter().enumerate() {
                    mark(&host, "data-battlefield-status", &status_name(&set));
                    mark(&host, "data-battlefield-status-index", &index.to_string());
                    let applied = with_preview(|stage| {
                        if let Ok(mut tokens) = preview_tokens(
                            stage.grid(),
                            stage.layout().cloned().as_ref(),
                            request.heroes,
                            request.enemies,
                        ) {
                            tokens[0].statuses = set.clone();
                            // The goblin keeps the frightened figure's attention honest.
                            stage.set_tokens(tokens);
                        }
                    });
                    if applied.is_none() {
                        return;
                    }
                    TimeoutFuture::new(3500).await;
                }
            }
            PreviewMode::Still => {
                with_preview(|stage| {
                    if request.close {
                        stage.show_close();
                    } else {
                        stage.show_tactical();
                    }
                });
                mark(&host, "data-battlefield-phase", "still");
                TimeoutFuture::new(60_000).await;
            }
            PreviewMode::Sheet(_) => {
                with_preview(|stage| stage.show_close());
                mark(&host, "data-battlefield-phase", "sheet");
                TimeoutFuture::new(60_000).await;
            }
        }
    }
}

/// The ids of the figures on the preview field: heroes, then enemies.
fn preview_ids() -> (Vec<TokenId>, Vec<TokenId>) {
    with_preview(|stage| {
        let ids = |side| {
            stage
                .tokens()
                .filter(|token| token.side == side)
                .map(|token| token.id.clone())
                .collect::<Vec<_>>()
        };
        (ids(Side::Hero), ids(Side::Enemy))
    })
    .unwrap_or_default()
}

/// A hit or miss for the demos below.
fn demo_blow(attacker: &TokenId, target: &TokenId, label: &str, result: AttackResult) {
    let (attacker, target) = (attacker.clone(), target.clone());
    let label = label.to_owned();
    with_preview(move |stage| {
        stage.set_attack_label(&attacker, &label, "");
        let after = match result {
            AttackResult::Miss => None,
            AttackResult::Hit { downed: true, .. } => Some(0),
            AttackResult::Hit { damage, .. } => Some(12u32.saturating_sub(damage)),
        };
        let _ = stage.stage(Beat::Attack {
            attacker,
            target,
            result,
            target_hit_points_after: after,
        });
    });
}

/// Stays until the preview is torn down.
async fn hold_preview() {
    loop {
        TimeoutFuture::new(1000).await;
        if PREVIEW.with(|slot| slot.borrow().is_none()) {
            return;
        }
    }
}

/// The review demos for speech, damage text, chained enemy turns and the victory and defeat beats
/// (`&beat=speak|named|namedmiss|namedcrit|chain|victory|defeat`).
async fn run_extra_demo(host: &Element, kind: &str) {
    let (heroes, enemies) = preview_ids();
    let (Some(hero), Some(enemy)) = (heroes.first().cloned(), enemies.first().cloned()) else {
        return;
    };
    let hit = |damage, critical, downed| AttackResult::Hit {
        damage,
        critical,
        downed,
    };
    match kind {
        "speak" => {
            let name_of = |id: &TokenId| {
                with_preview(|stage| {
                    stage
                        .tokens()
                        .find(|token| token.id == *id)
                        .map(|token| token.name.clone())
                })
                .flatten()
                .unwrap_or_default()
            };
            let lines = [
                (
                    name_of(&enemy),
                    "Hand over the packet and nobody has to bleed tonight.",
                ),
                (
                    name_of(&hero),
                    "You will have to take it from me, friend. I am not going anywhere.",
                ),
            ];
            for (index, (speaker, text)) in lines.iter().enumerate() {
                let line_id = index as u64 + 1;
                let opened = with_preview(|stage| stage.speak(speaker, text, line_id));
                mark(host, "data-battlefield-line", &line_id.to_string());
                mark(host, "data-battlefield-speaker", speaker);
                if opened != Some(true) {
                    mark(host, "data-battlefield-speaker", "not-on-field");
                }
                TimeoutFuture::new(6000).await;
                with_preview(|stage| stage.end_speech(line_id));
                mark(host, "data-battlefield-line", "ended");
                TimeoutFuture::new(2500).await;
            }
        }
        "named" => demo_blow(&hero, &enemy, "Greatsword", hit(9, false, false)),
        "namedmiss" => {
            let archer = heroes.get(1).cloned().unwrap_or(hero);
            demo_blow(&archer, &enemy, "Longbow", AttackResult::Miss);
        }
        "namedcrit" => demo_blow(&enemy, &hero, "Bite", hit(7, true, false)),
        "chain" => {
            // Every enemy acts in a row on a hero: the results stack, none over another.
            for (index, attacker) in enemies.iter().enumerate() {
                let target = &heroes[index % heroes.len()];
                demo_blow(
                    attacker,
                    target,
                    "Scimitar",
                    hit(3 + index as u32, false, false),
                );
            }
        }
        "victory" => {
            // The party brings every foe down, one after another: the last blow decides the fight.
            for (index, target) in enemies.iter().enumerate() {
                let attacker = &heroes[index % heroes.len()];
                demo_blow(attacker, target, "Greatsword", hit(12, false, true));
            }
        }
        "defeat" => {
            for (index, target) in heroes.iter().enumerate() {
                let attacker = &enemies[index % enemies.len()];
                demo_blow(attacker, target, "Scimitar", hit(14, false, true));
            }
        }
        _ => {}
    }
    mark(host, "data-battlefield-demo-done", kind);
    hold_preview().await;
}

/// Six figures, three heroes and three enemies, each carrying different statuses, for review.
fn sheet_tokens(
    grid: &battle::Grid,
    layout: Option<&battle::Layout>,
    page: usize,
) -> Result<Vec<Token>, JsValue> {
    let seats = match layout {
        Some(layout) => battle::from_layout(layout, grid, 3, 3),
        None => battle::derive_formation(grid, 3, 3),
    }
    .map_err(|_| JsValue::from_str("battlefield formation has no room"))?;
    let cycle = status_cycle();
    let set_for = |slot: usize| {
        cycle
            .get((page * 6 + slot) % cycle.len())
            .cloned()
            .unwrap_or_default()
    };
    let mut tokens = vec![
        combatant(
            "s0",
            "",
            Side::Hero,
            seats.heroes[0],
            "figure-hero-1",
            "portrait-hero-1",
            PERSON_HEIGHT_M,
        ),
        combatant(
            "s1",
            "",
            Side::Hero,
            seats.heroes[1],
            "figure-hero-3",
            "portrait-hero-3",
            PERSON_HEIGHT_M,
        ),
        combatant(
            "s2",
            "",
            Side::Hero,
            seats.heroes[2],
            "figure-hero-2",
            "portrait-hero-2",
            PERSON_HEIGHT_M,
        ),
        combatant(
            "s3",
            "",
            Side::Enemy,
            seats.enemies[0],
            "sprite-bandit-documentary-photo",
            "sprite-bandit-documentary-photo",
            1.0,
        ),
        combatant(
            "s4",
            "",
            Side::Enemy,
            seats.enemies[1],
            "figure-ant-skeleton",
            "portrait-ant-skeleton",
            1.75,
        ),
        combatant(
            "s5",
            "",
            Side::Enemy,
            seats.enemies[2],
            "figure-ant-wolf",
            "portrait-ant-wolf",
            0.9,
        ),
    ];
    for (slot, token) in tokens.iter_mut().enumerate() {
        token.statuses = set_for(slot);
        token.name = token
            .statuses
            .first()
            .map_or_else(String::new, |status| status.label().to_owned());
    }
    Ok(tokens)
}

/// Shows the notice once if the stage later becomes unavailable (a lost device).
async fn watch_availability(host: Element) {
    loop {
        TimeoutFuture::new(500).await;
        let state = PREVIEW.with(|slot| slot.borrow().as_ref().map(BattleStage::availability));
        match state {
            Some(Availability::Available) => {}
            Some(Availability::Unavailable(reason)) => {
                show_unavailable(&host, reason.key());
                return;
            }
            None => return,
        }
    }
}

fn show_unavailable(host: &Element, reason: &str) {
    if host
        .query_selector(".bf-unavailable")
        .ok()
        .flatten()
        .is_some()
    {
        return;
    }
    let Some(document) = host.owner_document() else {
        return;
    };
    let Ok(card) = document.create_element("div") else {
        return;
    };
    card.set_class_name("bf-unavailable");
    card.set_text_content(Some(&format!(
        "The 3D battlefield is not available on this device ({reason}). The flat combat view is used instead."
    )));
    // The notice is advisory; a failed append leaves the data attribute as the record.
    let _ = host.append_child(&card);
}

/// Mounts the preview into the display root and starts the scripted fight.
pub(super) fn start_preview(document: &Document) -> Result<(), JsValue> {
    let request =
        preview_request().ok_or_else(|| JsValue::from_str("battle preview not requested"))?;
    install_style(document)?;
    let host = match document.get_element_by_id("display") {
        Some(root) => root,
        None => document
            .body()
            .ok_or_else(|| JsValue::from_str("document body absent"))?
            .into(),
    };
    wasm_bindgen_futures::spawn_local(run_preview(host, request));
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Live combat on the shared display.
//
// The flat combat view stays mounted underneath: the battlefield is an overlay, so when it is not
// available (no WebGPU, device lost, still mounting) the display simply shows today's view.
// The display's cue director drives the attack staging: `on_cue` is called from
// `live_motion::enter`, so the dice sound, the 3D approach/strike and the damage number stay in
// step. Views only keep the tokens, hit points, round, initiative and turn in line.
// ---------------------------------------------------------------------------------------------

use super::live_combat::LiveView;
use crate::presentation::GridCell;
use df_render::art_contract::Role;
use df_render::battle::{CombatFeed, CombatSnapshot, FeedMode, Member, Outcome, RollCard};
use std::collections::VecDeque;

/// What the cue director knows about a roll, copied from its own detail.
#[derive(Clone)]
pub(super) struct RollSummary {
    pub(super) sequence: u64,
    pub(super) attacker: String,
    pub(super) target: String,
    pub(super) hit: bool,
    pub(super) critical: bool,
    pub(super) grazed: bool,
    pub(super) damage: u32,
    pub(super) target_down: bool,
    pub(super) fight_over: bool,
    /// The attack's name and the rules' damage type for the floating text.
    pub(super) attack_label: Option<String>,
    pub(super) damage_type: String,
    /// The server-reported weapon determines whether this was a bow, throw or melee blow.
    pub(super) attack_style: AttackStyle,
    /// The attacker's turn begins with this blow.
    pub(super) turn_start: bool,
    /// The cells the attacker walks from and to before this blow, when the turn includes a move.
    pub(super) walk: Option<(GridCell, GridCell)>,
    /// The target's hit points after this blow, from the outcome itself: the strike shows
    /// exactly this when it lands, never the newest view's later state.
    pub(super) target_hp_after: u32,
    /// A roll that is not an attack (a death save, a stabilise check): the roll card shows, with
    /// no approach and no strike.
    pub(super) check: bool,
    /// The first part of a move an opportunity attack interrupts: the mover is held on this
    /// square while the reaction plays, and the second part walks it on.
    pub(super) then_hold: Option<GridCell>,
    pub(super) card: Option<RollCard>,
}

enum Phase {
    Idle,
    Mounting,
    Active,
    Unavailable,
}

struct Live {
    phase: Phase,
    /// Identity of the current world/mount request; release invalidates stale asynchronous loads.
    world_mount: Option<Rc<()>>,
    host: Option<Element>,
    stage: Option<BattleStage>,
    battlefield_component_id: Option<u64>,
    banner_component_id: Option<u64>,
    turn_component_id: Option<u64>,
    last_turn_key: Option<String>,
    techniques: std::collections::BTreeMap<u64, rpc::BattleTechnique>,
    feed: CombatFeed,
    latest: Option<CombatSnapshot>,
    /// The fight began in front of the display and its first feed has not run yet.
    begun_live: bool,
    /// The scene is a scan: its splat streams in, and the entrance waits for it.
    expects_splat: bool,
    /// The initiative roll and roll-call were staged and are not yet over: the entrance waits for
    /// the battlefield's own HUD to show them and then to finish them.
    initiative_pending: bool,
    initiative_seen: bool,
    reachable: Vec<Cell>,
    targets: Vec<Cell>,
    /// Movers whose server cell is not shown yet: they stand where they did until the cue
    /// stream reaches their turn and walks them (name, cell they stand on, when held).
    held_moves: Vec<(String, Cell, f64)>,
    /// Outcomes of accepted views whose cues have not yet been staged: while any wait, the label
    /// stays with the attacker being shown.
    pending_cues: usize,
    /// Where the newest accepted view puts everyone, which the display's view lags behind while
    /// the blows of earlier views are shown: unheld figures stand there, not where an older
    /// view had them.
    accepted_cells: std::collections::HashMap<String, GridCell>,
    /// A reaction the server waits on: who moves and who decides, as the latest view says.
    reaction: Option<(String, String)>,
    reaction_seconds_remaining: Option<u32>,
    /// The prompt now shown for it.
    shown_prompt: Option<String>,
    shown_reaction: Option<(String, String)>,
    /// When `pending_cues` last grew, so a cue stream that never arrives cannot hold the label.
    pending_at: f64,
    /// The line the conductor is playing (label, text), kept so a line that opens before the
    /// stage mounts still lands on the loading splash once it does.
    pending_caption: Option<(String, String)>,
    /// The last beat sequence the display finished presenting: the feed is truncated to it,
    /// so health cards and downed poses never show a blow that has not landed on screen.
    presented_seq: u64,
    /// The newest beat sequence in the latest accepted view.
    latest_seq: u64,
    /// Per HP change `(sequence, target name, before, after)`, oldest first: the values used to
    /// project the newest view's hit points to the last presented beat.
    beat_hp: Vec<(u64, String, u32, u32)>,
    /// `(sequence, name, down)` for each beat that set or cleared a creature's down mark.
    beat_down: Vec<(u64, String, bool)>,
    /// The hit currently in its strike timeline, matched against the renderer's landed event.
    active_strike: Option<(u64, String, String)>,
    /// Nominal stages and budgets for the recent battlefield cues, oldest first.
    battlefield_beat_log: VecDeque<String>,
    card_values: std::collections::HashMap<String, u32>,
    card_log: VecDeque<String>,
    outcomes: std::collections::HashMap<u64, rpc::CombatOutcome>,
    presenting_sequence: Option<u64>,
    damage_readable_sequence: Option<u64>,
    pending_impact: Option<RollSummary>,
}

thread_local! {
    static LIVE: RefCell<Live> = RefCell::new(Live {
        phase: Phase::Idle,
        host: None,
        stage: None,
        battlefield_component_id: None,
        banner_component_id: None,
        turn_component_id: None,
        last_turn_key: None,
        techniques: std::collections::BTreeMap::new(),
        feed: CombatFeed::new(FeedMode::HoldForCues),
        latest: None,
        begun_live: false,
        expects_splat: false,
        world_mount: None,
        initiative_pending: false,
        initiative_seen: false,
        reachable: Vec::new(),
        targets: Vec::new(),
        held_moves: Vec::new(),
        pending_cues: 0,
        accepted_cells: std::collections::HashMap::new(),
        reaction: None,
        reaction_seconds_remaining: None,
        shown_prompt: None,
        shown_reaction: None,
        pending_at: 0.0,
        pending_caption: None,
        presented_seq: 0,
        latest_seq: 0,
        beat_hp: Vec::new(),
        beat_down: Vec::new(),
        active_strike: None,
        battlefield_beat_log: VecDeque::new(),
        card_values: std::collections::HashMap::new(),
        card_log: VecDeque::new(),
        outcomes: std::collections::HashMap::new(),
        presenting_sequence: None,
        damage_readable_sequence: None,
        pending_impact: None,
    });
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Display the occupied squares from the supplied size and north-west anchor.
fn participant_cells(member: &rpc::PartyMemberView) -> Vec<Cell> {
    let Some(anchor) = member.cell.as_ref().and_then(|cell| {
        Some(Cell::new(
            i32::try_from(cell.col).ok()?,
            i32::try_from(cell.row).ok()?,
        ))
    }) else {
        return Vec::new();
    };
    let extent = match member.size() {
        rpc::CreatureSize::Large => 2,
        rpc::CreatureSize::Huge => 3,
        _ => 1,
    };
    (0..extent)
        .flat_map(|row| (0..extent).map(move |col| Cell::new(anchor.col + col, anchor.row + row)))
        .collect()
}

/// The figure's standing height from the server-supplied SRD size, without reading its name.
fn enemy_height_m(size: rpc::CreatureSize) -> f32 {
    match size {
        rpc::CreatureSize::Small => 1.0,
        rpc::CreatureSize::Large => 2.6,
        rpc::CreatureSize::Huge => 3.0,
        rpc::CreatureSize::Medium | rpc::CreatureSize::Unspecified => 1.75,
    }
}

/// The battle scene for the fight, chosen from the LOCATION the server names
/// (`GameplayScene.location_id`), never from the painting: story art for a fight can be any
/// `scene-place-*` or `scene-dest-*` picture and no longer names the place. The reason for any
/// fallback is returned so it can be recorded on the page.
fn scene_choice(view: &LiveView<'_>) -> battle::SceneChoice {
    battle::resolve_scene(
        view.journey
            .combat
            .as_ref()
            .map(|combat| combat.location_id.as_str())
            .filter(|id| !id.is_empty())
            .unwrap_or(&view.scene.location_id),
    )
}

/// The picture behind the battlefield, by the art contract's backdrop order: the place's live
/// room photograph, the current scene plate, then a reviewed photograph the scene names. `None`
/// keeps the battle scene file's own backdrop (itself a role token or a reviewed photograph) and
/// the dark stage. The display never asks for a picture to be made; the server does.
fn backdrop_stem(view: &LiveView<'_>) -> Option<String> {
    crate::presentation::place_picture(
        Role::BattleBackdrop,
        &view.scene.scene_asset,
        &view.scene.live_backdrop,
        &view.scene.live_plate,
    )
}

/// Legacy/preview layout fallback; mounted tactical fights use the server layout_id.
fn chosen_layout_id(scene: &BattleScene, key: &str) -> Option<String> {
    battle::choose_layout(&scene.layouts, key).map(|layout| layout.id.clone())
}

fn layout_key(view: &LiveView<'_>, snapshot: &CombatSnapshot) -> String {
    let mut ids: Vec<&str> = snapshot
        .members
        .iter()
        .map(|member| member.id.as_str())
        .collect();
    ids.sort_unstable();
    format!("{}|{}", view.scene.location_id, ids.join(","))
}

/// SWITCH POINT (content rating). `mature` unless the page says `restrained` (the server's
/// `DF_GAMEPLAY_CONTENT_RATING` setting will write `data-content-rating` on the document element).
pub(super) fn content_rating() -> battle::ContentRating {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
        .and_then(|element| element.get_attribute("data-content-rating"))
        .and_then(|key| battle::ContentRating::from_key(&key))
        .unwrap_or_default()
}

thread_local! {
    /// A non-combat view was seen since the last fight: the next fight began in front of the
    /// display, so its initiative roll is staged. A display that loads straight into a fight does
    /// not stage it.
    static SEEN_NON_COMBAT: StdCell<bool> = const { StdCell::new(false) };
}

/// Copies the server's combat projection into plain data. No outcome is decided here.
pub(super) fn voice_set(key: &str) -> Option<battle::VoiceSet> {
    use battle::VoiceSet as V;
    match key {
        "hero-male" => Some(V::HeroMale),
        "hero-female" => Some(V::HeroFemale),
        "small-creature" => Some(V::SmallCreature),
        "beast" => Some(V::Beast),
        "undead" => Some(V::Undead),
        "humanoid" => Some(V::Humanoid),
        _ => None,
    }
}

fn member_maximum_hit_points(member: &rpc::PartyMemberView, party: &[rpc::PartyMemberView]) -> u32 {
    let party_member = if member.participant_id.is_empty() {
        party.iter().find(|candidate| candidate.name == member.name)
    } else {
        party
            .iter()
            .find(|candidate| candidate.participant_id == member.participant_id)
            .or_else(|| party.iter().find(|candidate| candidate.name == member.name))
    };
    party_member.map_or(member.maximum_hit_points, |candidate| {
        candidate.maximum_hit_points
    })
}

fn reaction_prompt(mover: &str, reactor: &str, seconds_remaining: Option<u32>) -> String {
    let countdown =
        seconds_remaining.map_or_else(String::new, |seconds| format!(" ({seconds}s remaining)"));
    format!("{reactor} decides: Opportunity attack as {mover} moves{countdown}")
}

fn participant_live_art(
    member: &rpc::PartyMemberView,
    hero: bool,
    scene: &rpc::GameplayScene,
    live_faces: &[rpc::LiveFace],
) -> Option<String> {
    if hero {
        live_faces
            .iter()
            .find(|face| face.name == member.name)
            .and_then(|face| crate::presentation::imagery_url(&face.battle_sprite))
    } else {
        live_faces
            .iter()
            .find(|face| face.name == member.name)
            .and_then(|face| crate::presentation::imagery_url(&face.battle_sprite))
            .or_else(|| {
                scene
                    .cast
                    .iter()
                    .find(|actor| actor.name == member.name && actor.portrait == member.portrait)
                    .and_then(|actor| crate::presentation::imagery_url(&actor.live_sprite))
            })
    }
}

fn snapshot_of(view: &LiveView<'_>) -> Option<CombatSnapshot> {
    let combat = view.journey.combat.as_ref()?;
    let party = view.journey.party.len();
    let id_of = |index: usize, member: &rpc::PartyMemberView| {
        if member.participant_id.is_empty() {
            format!("participant-{index}")
        } else {
            hex(&member.participant_id)
        }
    };
    let members: Vec<Member> = combat
        .participants
        .iter()
        .enumerate()
        .map(|(index, member)| {
            let authoritative = member.cell.is_some();
            let hero = if authoritative {
                member.side == 1
            } else {
                index < party
            };
            // Seats wear the hero art in this order: the dwarf fighter, then the archer (the
            // owner asked for a female long-range player in the party), then the second fighter.
            let hero_art = [1, 3, 2][index % 3];
            let creature = Some(member.creature_id.as_str())
                .filter(|id| !id.is_empty())
                .unwrap_or("bandit");
            let portrait = if authoritative && !member.portrait.is_empty() {
                member.portrait.clone()
            } else if hero {
                format!("portrait-hero-{hero_art}")
            } else {
                // The preview path has no authoritative portrait: the creature id the server
                // sent names the family, the bandit when it sent none.
                format!("portrait-ant-{creature}")
            };
            let figure = if authoritative && !portrait.is_empty() {
                portrait.replacen("portrait-", "figure-", 1)
            } else if hero {
                format!("figure-hero-{hero_art}")
            } else {
                portrait.replacen("portrait-", "figure-", 1)
            };
            // A hero wears their own picture: the live battle face the server publishes (their
            // canonical cut-out, or its health variant when hurt); the stock seat art only until
            // one exists (B-08). Scene fighters likewise receive their own canonical picture on
            // every view, including art that became ready after the stage mounted (B-17).
            let live_face =
                participant_live_art(member, hero, view.scene, &view.journey.live_faces);
            let (figure, portrait) = match live_face {
                Some(path) => (path.clone(), path),
                None => (figure, portrait),
            };
            Member {
                id: id_of(index, member),
                name: member.name.clone(),
                side: if hero { Side::Hero } else { Side::Enemy },
                hit_points: member.hit_points,
                maximum_hit_points: member_maximum_hit_points(member, &view.journey.party),
                height_m: if hero {
                    PERSON_HEIGHT_M
                } else {
                    enemy_height_m(member.size())
                },
                attack: if authoritative {
                    if member.attack_style == 2 {
                        AttackStyle::Ranged
                    } else {
                        AttackStyle::Melee
                    }
                } else {
                    attack_style_for(&figure)
                },
                voice: voice_set(&member.voice_set),
                // Server-supplied cells replace the spawn cells here (none carried today).
                cell: member.cell.as_ref().and_then(|cell| {
                    Some(Cell::new(
                        i32::try_from(cell.col).ok()?,
                        i32::try_from(cell.row).ok()?,
                    ))
                }),
                position_sequence: combat
                    .move_outcomes
                    .iter()
                    .filter(|outcome| outcome.actor_name == member.name)
                    .map(|outcome| outcome.sequence)
                    .max()
                    .unwrap_or_default(),
                // Only what the view already says: bloodied from the hit points, unconscious
                // from the server's own knockout flag. The conditions wait for the server.
                statuses: if authoritative {
                    member
                        .conditions
                        .iter()
                        .filter_map(|condition| Status::from_id(condition))
                        .collect()
                } else {
                    battle::derived_from_view(
                        member.hit_points,
                        member.maximum_hit_points,
                        member.unconscious,
                    )
                },
                figure: Some(figure),
                portrait: Some(portrait),
            }
        })
        .collect();
    let by_name = |name: &str| {
        members
            .iter()
            .find(|member| member.name == name)
            .map(|m| m.id.clone())
    };
    let outcomes = combat
        .outcomes
        .iter()
        .filter_map(|outcome| {
            let actor = if outcome.actor_id.is_empty() {
                by_name(&outcome.actor_name)?
            } else {
                hex(&outcome.actor_id)
            };
            let target = if outcome.target_id.is_empty() {
                by_name(&outcome.target_name)?
            } else {
                hex(&outcome.target_id)
            };
            Some(Outcome {
                actor,
                target,
                hit: outcome.hit,
                critical: outcome.critical,
                damage: outcome.damage,
                target_hit_points_after: outcome.target_hit_points,
                downed: outcome.knocked_out || outcome.target_hit_points == 0,
                weapon: outcome.weapon.clone(),
                grazed: outcome.grazed,
                damage_type: outcome.damage_type.clone(),
            })
        })
        .collect();
    let active = (!combat.finished)
        .then(|| {
            if combat.active_participant_id.is_empty() {
                by_name(&combat.active_actor)
            } else {
                Some(hex(&combat.active_participant_id))
            }
        })
        .flatten();
    let mut initiative: Vec<(u32, battle::InitiativeEntry)> = combat
        .initiative
        .iter()
        .map(|roll| {
            (
                roll.order,
                battle::InitiativeEntry {
                    id: hex(&roll.participant_id),
                    die: roll.die,
                    modifier: roll.modifier,
                    total: roll.total,
                },
            )
        })
        .collect();
    initiative.sort_by_key(|(order, _)| *order);
    Some(CombatSnapshot {
        members,
        active,
        round: combat.round,
        latest_sequence: combat.latest_sequence,
        presented_sequence: 0,
        outcomes,
        initiative: initiative.into_iter().map(|(_, entry)| entry).collect(),
        begun_live: false,
    })
}

/// Feeds the accepted combat view to the battlefield. Call it after the flat view is presented;
/// it mounts the battlefield on first use and never blocks or fails the flat view.
pub(super) fn present(client: &Rc<RefCell<Client>>, view: &LiveView<'_>) {
    if client.borrow().role == "player" {
        return;
    }
    // The current view initializes the command on mount. The receive hook also delivers
    // same-game-revision camera edits, before an older retained view can be redrawn.
    if CAMERA_OVERRIDE.with(|slot| slot.borrow().is_none()) {
        let current = client
            .borrow()
            .views
            .as_ref()
            .and_then(|views| views.current().cloned());
        if let Some(current) = current {
            camera_view(client, &current);
        }
    }
    apply_live_camera();
    let Some(snapshot) = snapshot_of(view) else {
        return;
    };
    let stale = view.journey.combat.as_ref().is_some_and(|combat| {
        LIVE.with(|live| {
            let live = live.borrow();
            live.latest.is_some()
                && crate::presentation::is_stale_combat_sequence(
                    live.latest_seq,
                    combat.latest_sequence,
                )
        })
    });
    if stale {
        return;
    }
    let choice = scene_choice(view);
    let backdrop = backdrop_stem(view);
    let key = layout_key(view, &snapshot);
    let phase_idle = LIVE.with(|live| {
        let mut live = live.borrow_mut();
        if let Some(combat) = &view.journey.combat {
            let presenting = live
                .presenting_sequence
                .and_then(|sequence| live.outcomes.get(&sequence).cloned());
            live.outcomes = combat
                .outcomes
                .iter()
                .rev()
                .take(BATTLEFIELD_BEAT_LOG_CAP)
                .map(|outcome| (outcome.sequence, outcome.clone()))
                .collect();
            if let Some(outcome) = presenting {
                live.outcomes.entry(outcome.sequence).or_insert(outcome);
            }
            live.techniques = combat
                .outcomes
                .iter()
                .rev()
                .take(BATTLEFIELD_BEAT_LOG_CAP)
                .filter_map(|outcome| {
                    outcome
                        .presentation
                        .clone()
                        .map(|effect| (outcome.sequence, effect))
                })
                .collect();
            live.latest_seq = combat.latest_sequence;
            live.reaction_seconds_remaining = Some(combat.reaction_seconds_remaining);
            live.reaction = (!combat.reaction_participant_id.is_empty())
                .then(|| {
                    let reactor = combat
                        .participants
                        .iter()
                        .find(|member| member.participant_id == combat.reaction_participant_id)?;
                    Some((combat.active_actor.clone(), reactor.name.clone()))
                })
                .flatten();
            live.reachable = combat
                .reachable_cells
                .iter()
                .filter_map(|cell| {
                    Some(Cell::new(
                        i32::try_from(cell.col).ok()?,
                        i32::try_from(cell.row).ok()?,
                    ))
                })
                .collect();
            live.targets = combat
                .legal_targets
                .iter()
                .filter_map(|target| {
                    combat
                        .participants
                        .iter()
                        .find(|member| member.participant_id == target.participant_id)
                })
                .flat_map(participant_cells)
                .collect();
        }
        if let Some(combat) = &view.journey.combat {
            for outcome in &combat.outcomes {
                if live.beat_hp.iter().any(|(sequence, target, ..)| {
                    *sequence == outcome.sequence && *target == outcome.target_name
                }) {
                    continue;
                }
                let maximum = snapshot
                    .members
                    .iter()
                    .find(|member| member.name == outcome.target_name)
                    .map_or(u32::MAX, |member| member.maximum_hit_points);
                let before = live
                    .beat_hp
                    .iter()
                    .rev()
                    .find(|(_, target, ..)| *target == outcome.target_name)
                    .map_or_else(
                        || {
                            outcome
                                .target_hit_points
                                .saturating_add(outcome.damage)
                                .min(maximum)
                        },
                        |(_, _, _, after)| *after,
                    );
                live.beat_hp.push((
                    outcome.sequence,
                    outcome.target_name.clone(),
                    before,
                    outcome.target_hit_points,
                ));
                live.beat_hp.sort_by_key(|(sequence, ..)| *sequence);
            }
            let beats = crate::combat_beats::in_resolution_order(
                combat
                    .outcomes
                    .iter()
                    .chain(combat.death_save_outcomes.iter()),
            );
            for mark in crate::combat_beats::down_marks(&beats) {
                if !live
                    .beat_down
                    .iter()
                    .any(|(sequence, name, _)| *sequence == mark.0 && *name == mark.1)
                {
                    live.beat_down.push(mark);
                }
            }
            live.beat_down.sort_by_key(|(sequence, ..)| *sequence);
            let mut heals = Vec::new();
            if let Some(previous) = &live.latest {
                for member in &snapshot.members {
                    let Some(before) = previous
                        .members
                        .iter()
                        .find(|previous| previous.id == member.id)
                        .map(|previous| previous.hit_points)
                    else {
                        continue;
                    };
                    if member.hit_points <= before {
                        continue;
                    }
                    let change_before = live
                        .beat_hp
                        .iter()
                        .rev()
                        .find(|(_, target, ..)| *target == member.name)
                        .map_or(before, |(_, _, _, after)| *after);
                    if member.hit_points > change_before {
                        heals.push((
                            combat.latest_sequence,
                            member.name.clone(),
                            change_before,
                            member.hit_points,
                        ));
                    }
                }
            }
            live.beat_hp.extend(heals);
            live.beat_hp.sort_by_key(|(sequence, ..)| *sequence);
        }
        live.latest = Some(snapshot);
        let idle = matches!(live.phase, Phase::Idle);
        if idle {
            // A fight beginning in front of the display stages its initiative roll once.
            live.begun_live = SEEN_NON_COMBAT.with(|seen| seen.replace(false));
        }
        idle
    });
    if phase_idle {
        start_mount(
            client,
            &choice,
            backdrop,
            &key,
            view.journey
                .combat
                .as_ref()
                .map(|combat| combat.layout_id.clone())
                .filter(|id| !id.is_empty()),
            &view.scene.location_id,
            &view.scene.title,
            view.journey
                .combat
                .as_ref()
                .and_then(|combat| combat.battle_world.clone()),
        );
    } else {
        sync("view");
    }
}

fn start_mount(
    client: &Rc<RefCell<Client>>,
    choice: &battle::SceneChoice,
    backdrop: Option<String>,
    key: &str,
    server_layout: Option<String>,
    story_location: &str,
    place_title: &str,
    world: Option<rpc::BattleWorld>,
) {
    let (document, host) = {
        let state = client.borrow();
        (state.document.clone(), state.root.clone())
    };
    let Some(Ok(location_scene)) = battle::load_embedded(choice.scene_id) else {
        LIVE.with(|live| live.borrow_mut().phase = Phase::Unavailable);
        return;
    };
    // The story place's own scene: its title names the fight, and its painted layers are what a
    // scan falls back to when it cannot be drawn. (The server stages a fight on a forced scan's
    // own grid, so the scan is the scene chosen above.)
    let story_scene = battle::load_embedded(story_location).and_then(Result::ok);
    // Which story place the fight is told in (a diagnostic for the page).
    let _ = host.set_attribute("data-battlefield-story", story_location);
    // The record of which scene was chosen, and why if it is not the fight's own location.
    let marker = choice
        .fallback_reason
        .clone()
        .unwrap_or_else(|| choice.scene_id.to_owned());
    // `DF_GAMEPLAY_BATTLE_SCENE` reaches the page as `data-battle-scene`: every fight is staged
    // on that scene (a scan, say), and the location's own painted layers are its fallback.
    let forced = host
        .get_attribute("data-battle-scene")
        .and_then(|id| battle::load_embedded(&id))
        .and_then(Result::ok);
    // Even a retained/older projection cannot replace a forced scan with a generated world.
    let world = world.filter(|_| forced.is_none());
    let painted = |scene: &BattleScene| Some((scene.ground.clone(), scene.backdrop.clone()));
    let (mut scene, mut fallback_layers) = match forced {
        Some(forced) => (forced, painted(&location_scene)),
        None if location_scene.splat.is_some() => {
            let fallback = story_scene
                .as_ref()
                .filter(|story| story.splat.is_none())
                .and_then(painted);
            (location_scene, fallback)
        }
        None => (location_scene, None),
    };
    // A scan's splat streams in; the entrance waits for it (see `splat_streaming`).
    let expects_splat = scene.splat.is_some();
    scene.title = place_title.to_owned();
    // Best-effort diagnostic attribute; a failure here changes nothing about the fight.
    let _ = host.set_attribute("data-battlefield-scene", &marker);
    if let Some(stem) = backdrop {
        match &mut fallback_layers {
            Some((_, fallback_backdrop)) => fallback_backdrop.stem = stem,
            None => scene.backdrop.stem = stem,
        }
    }
    // `DF_GAMEPLAY_BATTLE_FPS` reaches the page as `data-battle-fps` (the server refuses anything
    // but 24 or 30 at startup; a missing or odd value here means the default).
    let fps = host
        .get_attribute("data-battle-fps")
        .and_then(|value| value.parse::<u32>().ok())
        .and_then(battle::BattleFps::from_hz)
        .unwrap_or_default();
    // `DF_GAMEPLAY_SPLAT_LOD` reaches the page as `data-battle-splat-lod` (validated at startup).
    let splat_lod = host
        .get_attribute("data-battle-splat-lod")
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(DEFAULT_SPLAT_LOD);
    // `DF_GAMEPLAY_BATTLE_GRID=on` reaches the page as `data-battle-grid` (validated at startup).
    let show_grid = host.get_attribute("data-battle-grid").as_deref() == Some("on");
    let layout = server_layout.or_else(|| chosen_layout_id(&scene, key));
    let _ = host.set_attribute(
        "data-battlefield-layout",
        layout.as_deref().unwrap_or("none"),
    );
    if install_style(&document).is_err() {
        LIVE.with(|live| live.borrow_mut().phase = Phase::Unavailable);
        return;
    }
    let mount = Rc::new(());
    CAMERA_APPLIED.with(|slot| *slot.borrow_mut() = None);
    let camera_client = Rc::downgrade(client);
    LIVE.with(|live| {
        let mut live = live.borrow_mut();
        live.phase = Phase::Mounting;
        live.world_mount = Some(mount.clone());
        live.host = Some(host.clone());
        live.expects_splat = expects_splat;
    });
    wasm_bindgen_futures::spawn_local(async move {
        if let Some(world) = world {
            if fallback_layers.is_none() {
                fallback_layers = Some((scene.ground.clone(), scene.backdrop.clone()));
            }
            load_battle_world(&host, &mut scene, &world, layout.as_deref(), Some(&mount)).await;
            let current = LIVE.with(|live| {
                let mut live = live.borrow_mut();
                let current = matches!(live.phase, Phase::Mounting)
                    && live
                        .world_mount
                        .as_ref()
                        .is_some_and(|current| Rc::ptr_eq(current, &mount));
                if current {
                    live.expects_splat = scene.splat.is_some();
                }
                current
            });
            if !current {
                return;
            }
        }
        let stage = BattleStage::mount(
            &host,
            scene,
            StageOptions {
                procedural_splats: false,
                fallback_layers,
                fixed_quality: None,
                fps,
                feel_off: false,
                slow: None,
                splat_lod,
                show_grid,
            },
        )
        .await;
        if !world_mount_is_current(Some(&mount)) {
            return;
        }
        let available = matches!(stage.availability(), Availability::Available);
        if available {
            stage.set_sound_sink(Rc::new(|event| {
                live_sfx::play_event(&event, content_rating());
            }));
            let impact_host = host.clone();
            stage.set_impact_sink(Rc::new(move |event| {
                mark(
                    &impact_host,
                    "data-battlefield-impact",
                    &format!("{:?}:{:.2}", event.class, event.intensity),
                );
                let sequence = LIVE.with(|live| {
                    let mut live = live.borrow_mut();
                    let Some((sequence, attacker, target)) = &live.active_strike else {
                        return None;
                    };
                    if event.attacker.0.as_str() != attacker || event.target.0.as_str() != target {
                        return None;
                    }
                    let sequence = *sequence;
                    live.active_strike = None;
                    Some(sequence)
                });
                if let Some(sequence) = sequence {
                    super::live_motion::strike_landed(sequence);
                }
            }));
            if let Some(id) = &layout {
                stage.with_stage(|stage| {
                    stage.set_layout(id);
                    stage.set_restrained(content_rating() == battle::ContentRating::Restrained);
                });
            }
            live_sfx::set_battlefield_owns_combat(true);
        }
        let keep = LIVE.with(|live| {
            let mut live = live.borrow_mut();
            // Released while mounting (the fight ended): throw the new stage away.
            if !matches!(live.phase, Phase::Mounting) {
                return false;
            }
            live.phase = if available {
                Phase::Active
            } else {
                Phase::Unavailable
            };
            if available {
                live.stage = Some(stage);
                live.battlefield_component_id =
                    Some(super::presentation_timing::next_component_id());
            }
            // A line that opened while the stage mounted lands on the splash now.
            if let (Some(stage), Some((label, text))) = (&live.stage, &live.pending_caption) {
                stage.set_splash_caption(Some((label, text)));
            }
            available
        });
        if keep {
            LIVE.with(|live| {
                let live = live.borrow();
                if let (Some(id), Some(host)) = (live.battlefield_component_id, live.host.as_ref())
                {
                    let scene = host
                        .get_attribute("data-battlefield-scene")
                        .unwrap_or_default();
                    let element = host.query_selector(".battlefield").ok().flatten();
                    battle_component(
                        element.as_ref(),
                        df_observe::presentation::PresentationComponent::Battlefield,
                        id,
                        &scene,
                        df_observe::presentation::ComponentPhase::Opened,
                        "ready",
                    );
                    battle_component(
                        element.as_ref(),
                        df_observe::presentation::PresentationComponent::Battlefield,
                        id,
                        &scene,
                        df_observe::presentation::ComponentPhase::Readable,
                        "ready",
                    );
                }
            });
            sync("view");
            apply_live_camera();
            camera_readbacks(camera_client, mount).await;
        }
    });
}

/// Accepts an authenticated display view, including a duplicate gameplay revision.
/// The caller must first validate its connection generation and session scope.
pub(super) fn camera_view(_client: &Rc<RefCell<Client>>, view: &rpc::ViewMessage) {
    let Some(rpc::view_message::Audience::Display(display)) = &view.audience else {
        return;
    };
    CAMERA_OVERRIDE.with(|slot| {
        let mut stored = slot.borrow_mut();
        if display.camera_override.as_ref().is_some_and(|new| {
            stored
                .as_ref()
                .is_none_or(|old| new.revision >= old.revision)
        }) {
            stored.clone_from(&display.camera_override);
        }
    });
    apply_live_camera();
}

fn apply_live_camera() {
    let command = CAMERA_OVERRIDE.with(|slot| slot.borrow().clone());
    let Some(command) = command else {
        return;
    };
    LIVE.with(|live| {
        let live = live.borrow();
        let Some(stage) = live.stage.as_ref() else {
            return;
        };
        stage.with_stage(|stage| {
            let key = (command.revision, stage.scene().location_id.clone());
            if CAMERA_APPLIED.with(|slot| slot.borrow().as_ref() == Some(&key)) {
                return;
            }
            let pose = command.home.as_ref().map(|home| battle::CameraPose {
                target: stage.scene().home.target,
                pitch_deg: home.pitch_deg,
                yaw_deg: home.yaw_deg,
                distance: home.distance,
                fov_deg: home.fov_deg,
            });
            if let Err(error) = stage.set_home_override(pose) {
                web_sys::console::error_1(&JsValue::from_str(&format!(
                    "camera override rejected: {error:?}"
                )));
                return;
            }
            if let Some(shot) = battle::ShotName::from_key(&command.shot) {
                stage.set_camera_shot(shot);
            }
            CAMERA_APPLIED.with(|slot| *slot.borrow_mut() = Some(key));
        });
    });
}

fn camera_pose_json(pose: battle::CameraPose) -> serde_json::Value {
    serde_json::json!({"target":[pose.target.x,pose.target.y,pose.target.z],
        "distance":pose.distance,"yaw_deg":pose.yaw_deg,"pitch_deg":pose.pitch_deg,"fov_deg":pose.fov_deg})
}

fn camera_report(stage: &mut battle::Stage) -> Option<String> {
    let revision =
        CAMERA_APPLIED.with(|slot| slot.borrow().as_ref().map(|(revision, _)| *revision))?;
    let live = stage.frame().camera;
    let home = stage.home_pose();
    let report = serde_json::json!({"battle_camera": {
        "revision":revision,"scene":stage.scene().location_id,
        "live":camera_pose_json(live),"home":camera_pose_json(home),
        "authored_home":camera_pose_json(stage.scene().home),
        "at_home":live.is_at_home(&home),"sample_time_ms":now_ms()
    }});
    Some(report.to_string())
}

/// One bounded report at a time, owned by the current battlefield mount; retirement stops it.
async fn camera_readbacks(client: std::rc::Weak<RefCell<Client>>, mount: Rc<()>) {
    loop {
        TimeoutFuture::new(1000).await;
        if !world_mount_is_current(Some(&mount)) {
            return;
        }
        let Some(client) = client.upgrade() else {
            return;
        };
        if !CAMERA_OVERRIDE.with(|slot| slot.borrow().is_some()) {
            continue;
        }
        let report = LIVE.with(|live| {
            let live = live.borrow();
            let stage = live.stage.as_ref()?;
            stage.with_stage(camera_report).flatten()
        });
        let Some(text) = report else {
            continue;
        };
        let (sessions, credential) = {
            let state = client.borrow();
            (state.sessions.clone(), state.credential.clone())
        };
        drop(client);
        let Some(mut sessions) = sessions else {
            continue;
        };
        let message = rpc::PresentedRequest {
            sequence: 0,
            display_instance: super::live_motion::display_instance(),
            layout_readback_json: text,
        };
        let outcome = match super::request(message, &credential) {
            Ok(mut call) => {
                call.set_timeout(Duration::from_secs(2));
                sessions
                    .report_presented(call)
                    .await
                    .map(|_| ())
                    .map_err(|_| "camera report refused")
            }
            Err(_) => Err("camera report encoding failed"),
        };
        if let Err(error) = outcome {
            web_sys::console::error_1(&JsValue::from_str(error));
        }
    }
}

/// Applies the latest snapshot; notices a stage that went unavailable (device lost).
fn sync(writer: &str) {
    LIVE.with(|live| {
        let mut guard = live.borrow_mut();
        let live = &mut *guard;
        if !matches!(live.phase, Phase::Active) {
            return;
        }
        let (Some(stage), Some(snapshot)) = (&live.stage, &live.latest) else {
            return;
        };
        if live.begun_live && splat_streaming(live) {
            return;
        }
        if matches!(stage.availability(), Availability::Unavailable(_)) {
            live.phase = Phase::Unavailable;
            live.stage = None;
            return;
        }
        let feed = &mut live.feed;
        let mut shown = snapshot.clone();
        shown.begun_live = live.begun_live;
        shown.presented_sequence = live.presented_seq;
        let now = now_ms();
        live.held_moves
            .retain(|(.., at)| now - at < HOLD_PATIENCE_MS);
        if now - live.pending_at > HOLD_PATIENCE_MS {
            live.pending_cues = 0;
        }
        // A diagnostic for harnesses: who is held where, and how many outcomes still wait.
        if let Some(host) = &live.host {
            let held: Vec<String> = live
                .held_moves
                .iter()
                .map(|(name, cell, _)| format!("{name}@{},{}", cell.col, cell.row))
                .collect();
            let _ = host.set_attribute("data-battlefield-held", &held.join(";"));
            let _ = host.set_attribute("data-battlefield-pending", &live.pending_cues.to_string());
        }
        // While the blows of accepted views are being shown, everyone stands where the newest
        // accepted view puts them, except a mover that has not had its turn yet.
        let outstanding = live.pending_cues > 0 || !live.held_moves.is_empty();
        for member in &mut shown.members {
            if let Some((_, from, _)) = live
                .held_moves
                .iter()
                .find(|(name, ..)| *name == member.name)
            {
                member.cell = Some(*from);
            } else if outstanding
                && let Some(cell) = live
                    .accepted_cells
                    .get(&member.name)
                    .copied()
                    .and_then(cell_of)
            {
                member.cell = Some(cell);
            }
        }
        if !live.begun_live {
            // One clock: the feed is truncated to the presented sequence. A combatant shows
            // the hit points of its last presented outcome; with none presented yet, the
            // damage of its unpresented outcomes is added back. The strike beat itself
            // carries the exact figure for the moment it lands.
            let presented_seq = live.presented_seq;
            for member in &mut shown.members {
                let changes: Vec<battle::HitPointChange> = live
                    .beat_hp
                    .iter()
                    .filter(|(_, target, ..)| *target == member.name)
                    .map(|(sequence, _, before, after)| battle::HitPointChange {
                        sequence: *sequence,
                        before: *before,
                        after: *after,
                    })
                    .collect();
                let (newest_hit_points, newest_down) = (
                    member.hit_points,
                    member.hit_points == 0
                        || member.statuses.contains(&battle::Status::Unconscious),
                );
                member.hit_points =
                    battle::hit_points_at_sequence(member.hit_points, presented_seq, &changes);
                // The down marker, the blood and the stance follow the beat on screen with the
                // hit points: a creature the newest view lays low is shown down only with the
                // hit that dropped it.
                let down = crate::combat_beats::down_at(
                    &live.beat_down,
                    &member.name,
                    presented_seq,
                    newest_down,
                );
                if member.hit_points != newest_hit_points || down != newest_down {
                    member.statuses = crate::combat_beats::statuses_at_beat(
                        &member.statuses,
                        member.hit_points,
                        member.maximum_hit_points,
                        down,
                    );
                }
            }
        }
        if live.begun_live {
            // The fight is opening in front of the audience: show it as it stood at the roll for
            // initiative, before any combatant's turn. A view that already carries the enemy's
            // resolved first attack (it won initiative) is wound back to that moment; the
            // attack then plays as ordinary beats after the roll-call.
            for outcome in &shown.outcomes {
                if let Some(member) = shown.members.iter_mut().find(|m| m.name == outcome.target) {
                    member.hit_points = member
                        .hit_points
                        .saturating_add(outcome.damage)
                        .min(member.maximum_hit_points);
                }
            }
            shown.outcomes.clear();
            shown.round = 1;
            shown.active = shown.initiative.first().map(|entry| entry.id.clone());
        }
        let applied = stage
            .with_stage(|stage| {
                stage.observe_presented_sequence(live.presented_seq);
                stage.hold_active(battle::hold_server_turn(
                    live.pending_cues > 0,
                    live.latest_seq,
                    live.presented_seq,
                ));
                // A suspended turn is shown once the attacks before it have been: who decides and
                // what, with the camera on the two figures.
                let reaction = live
                    .reaction
                    .as_ref()
                    .filter(|_| live.pending_cues == 0)
                    .cloned();
                let prompt = reaction.as_ref().map(|(mover, reactor)| {
                    reaction_prompt(mover, reactor, live.reaction_seconds_remaining)
                });
                if prompt != live.shown_prompt {
                    if let (Some(prompt), Some((mover, reactor))) = (&prompt, &reaction) {
                        let same_reaction = live.shown_reaction.as_ref().is_some_and(
                            |(shown_mover, shown_reactor)| {
                                shown_mover == mover && shown_reactor == reactor
                            },
                        );
                        if same_reaction {
                            stage.set_prompt_after_camera_settles(Some(prompt.clone()));
                            live.shown_prompt = Some(prompt.clone());
                        } else {
                            let id_of = |name: &str| {
                                stage
                                    .tokens()
                                    .find(|token| token.name == name)
                                    .map(|token| token.id.clone())
                            };
                            if let (Some(attacker), Some(target)) = (id_of(mover), id_of(reactor)) {
                                if stage.stage(Beat::Approach { attacker, target }).is_ok() {
                                    stage.set_prompt_after_camera_settles(Some(prompt.clone()));
                                    live.shown_prompt = Some(prompt.clone());
                                    live.shown_reaction = Some((mover.clone(), reactor.clone()));
                                } else {
                                    stage.set_prompt(None);
                                    live.shown_prompt = None;
                                    live.shown_reaction = None;
                                }
                            } else {
                                stage.set_prompt(None);
                                live.shown_prompt = None;
                                live.shown_reaction = None;
                            }
                        }
                    } else {
                        stage.set_prompt(None);
                        live.shown_prompt = None;
                        live.shown_reaction = None;
                    }
                }
                stage.set_tactical_highlights(live.reachable.clone(), live.targets.clone());
                let report = feed.apply(stage, &shown)?;
                let cards = stage
                    .frame()
                    .figures
                    .iter()
                    .filter_map(|figure| {
                        figure
                            .hit_points
                            .map(|hit_points| (figure.name.clone(), hit_points.current))
                    })
                    .collect::<Vec<_>>();
                Some((report, cards))
            })
            .flatten();
        if let Some((_, cards)) = &applied {
            record_card_values(live, cards.iter().cloned(), writer);
        }
        if live.begun_live && applied.is_some() && !shown.initiative.is_empty() {
            live.initiative_pending = true;
            live.initiative_seen = false;
        }
        if applied.is_some() {
            record_battle_overlays(live, &shown);
        }
        live.begun_live = false;
        if applied.is_none() {
            // No room on the grid for this roster: leave the flat view.
            live.phase = Phase::Unavailable;
            live.stage = None;
        }
    });
}

/// The tempo cue's intensity (0 to 1000) for the camera's sway: calm is slow and small, peak
/// quicker and tighter. Called with every accepted display view, from the same tempo the music
/// follows, so there is one intensity signal.
pub(super) fn set_tempo(intensity: u16) {
    LIVE.with(|live| {
        if let Some(stage) = &live.borrow().stage {
            stage.set_tempo_intensity(intensity);
        }
    });
}

/// Converts only the server's display envelope. No inferred area membership, saves, attack
/// counts or damage; unsupported variants use the existing ordinary-outcome presentation.
fn technique_from_wire(
    display: &rpc::BattleTechnique,
    actor: &TokenId,
    target: &TokenId,
) -> Option<(AttackStyle, Beat)> {
    let strikes: Vec<_> = display
        .strikes
        .iter()
        .map(|strike| battle::StrikeDisplay {
            target: TokenId::new(hex(&strike.target_id)),
            result: if strike.hit {
                AttackResult::Hit {
                    damage: strike.damage,
                    critical: strike.critical,
                    downed: strike.downed,
                }
            } else {
                AttackResult::Miss
            },
            target_hit_points_after: strike.hit_points_after,
            roll: strike.roll.as_ref().map(|roll| battle::ResolvedRoll {
                d20: roll.d20,
                damage_faces: roll.damage_faces.clone(),
                attack_card: roll.attack_card.as_ref().map(|card| RollCard {
                    value: card.value.clone(),
                    label: card.label.clone(),
                    explanation: card.explanation.clone(),
                    verdict: card.verdict.clone(),
                    tone: card.tone.clone(),
                }),
            }),
            card: strike.card.as_ref().map(|card| RollCard {
                value: card.value.clone(),
                label: card.label.clone(),
                explanation: card.explanation.clone(),
                verdict: card.verdict.clone(),
                tone: card.tone.clone(),
            }),
        })
        .collect();
    match rpc::BattleTechniqueKind::try_from(display.kind).ok()? {
        rpc::BattleTechniqueKind::BattleTechniqueFireBolt => {
            let [strike] = strikes.as_slice() else {
                return None;
            };
            Some((
                AttackStyle::Bolt,
                Beat::Flurry {
                    attacker: actor.clone(),
                    strikes: vec![strike.clone()],
                    action_surge_at: None,
                },
            ))
        }
        rpc::BattleTechniqueKind::BattleTechniqueMagicMissile => Some((
            AttackStyle::Missiles,
            Beat::Missiles {
                caster: actor.clone(),
                darts: strikes,
            },
        )),
        rpc::BattleTechniqueKind::BattleTechniqueFireball => {
            let centre = display.centre.as_ref()?;
            Some((
                AttackStyle::Burst,
                Beat::AreaEffect {
                    caster: actor.clone(),
                    centre: Cell::new(
                        i32::try_from(centre.col).ok()?,
                        i32::try_from(centre.row).ok()?,
                    ),
                    radius_m: display.radius_m?,
                    targets: strikes,
                },
            ))
        }
        rpc::BattleTechniqueKind::BattleTechniqueShield => Some((
            AttackStyle::Melee,
            Beat::Shield {
                attacker: actor.clone(),
                defender: target.clone(),
                display: strikes.first().cloned().map(Box::new),
            },
        )),
        rpc::BattleTechniqueKind::BattleTechniqueFlurry => Some((
            AttackStyle::Melee,
            Beat::Flurry {
                attacker: actor.clone(),
                strikes,
                action_surge_at: display.action_surge_at,
            },
        )),
        rpc::BattleTechniqueKind::BattleTechniqueUnspecified => None,
    }
}

/// The cue director entered a state: stage the matching half of the attack.
pub(super) fn on_cue(state: &str, roll: Option<&RollSummary>) {
    let Some(roll) = roll else {
        return;
    };
    // The mover's hold ends as its turn is staged; an outcome is no longer waiting once its blow
    // is. The label is released when none is left.
    let mut drained = false;
    if state == "impact" && !roll.check && ordinary_presentation_active(roll.sequence) {
        let waiting = LIVE.with(|live| {
            let mut live = live.borrow_mut();
            if live.damage_readable_sequence == Some(roll.sequence) {
                return false;
            }
            live.pending_impact = Some(roll.clone());
            true
        });
        if waiting {
            return;
        }
    }
    let state = if state == "present-impact" {
        "impact"
    } else {
        state
    };
    LIVE.with(|live| {
        let mut live = live.borrow_mut();
        match state {
            "impact" if roll.sequence != 0 && (roll.hit || roll.grazed) => {
                live.active_strike =
                    Some((roll.sequence, roll.attacker.clone(), roll.target.clone()));
            }
            "impact" => live.active_strike = None,
            "rolling" | "turn" | "skipped" => live.active_strike = None,
            _ => {}
        }
        if state == "impact"
            && roll.sequence != 0
            && let Some(stage) = &live.stage
        {
            let _protected =
                stage.with_stage(|stage| stage.protect_hit_points_through(roll.sequence));
        }
        if matches!(state, "rolling" | "turn" | "skipped") {
            live.held_moves.retain(|(name, ..)| *name != roll.attacker);
        }
        if state == "turn"
            && let Some(cell) = roll.then_hold.and_then(cell_of)
        {
            live.held_moves
                .push((roll.attacker.clone(), cell, now_ms()));
        }
        if matches!(state, "impact" | "turn" | "skipped") {
            live.pending_cues = live.pending_cues.saturating_sub(1);
            if live.pending_cues == 0 {
                // Every recorded move and attack has reached the cue stream.
                live.held_moves.clear();
                drained = true;
            }
        }
    });

    if state == "impact" && !roll.hit && !roll.grazed && roll.sequence != 0 {
        // The miss verdict is already on the roll card; no damage event follows to acknowledge it.
        super::live_motion::strike_landed(roll.sequence);
    }

    LIVE.with(|live| {
        let guard = live.borrow();
        let Some(stage) = guard
            .stage
            .as_ref()
            .filter(|_| matches!(guard.phase, Phase::Active))
        else {
            return;
        };
        // The outcome's own figure, not the newest snapshot's: later blows may already be
        // resolved there, and the card must change only when this hit lands.
        let after = Some(roll.target_hp_after);
        stage.with_stage(|stage| {
            let id_of = |name: &str| {
                stage
                    .tokens()
                    .find(|token| token.name == name)
                    .map(|token| token.id.clone())
            };
            let (Some(attacker), Some(target)) = (id_of(&roll.attacker), id_of(&roll.target))
            else {
                return;
            };
            stage.hold_active(battle::hold_server_turn(
                guard.pending_cues > 0,
                guard.latest_seq,
                guard.presented_seq,
            ));
            match state {
                "skipped" if roll.check => {
                    // A skipped death save or stabilise check changes no token.
                    stage.skip_to_idle();
                }
                "skipped" => {
                    // The director emits skips in sequence, after the previous impact drains.
                    // Land every layer on this blow's result before the next roll starts.
                    stage.skip_to_idle();
                    let tokens: Vec<_> = stage.tokens().cloned().collect();
                    stage.set_tokens(crate::presentation::skipped_blow_tokens(
                        &tokens,
                        &roll.attacker,
                        &roll.target,
                        roll.walk.and_then(|(_, to)| cell_of(to)),
                        roll.target_hp_after,
                    ));
                    // Token replacement can stage a walk; a skipped blow lands that walk too.
                    stage.skip_to_idle();
                    stage.show_turn_now(Some(attacker));
                }
                "rolling" | "turn" => {
                    // This attacker's turn, strictly in order: its label, the walk the server
                    // gave it, then the attack. Dropped, not fatal: the queue is bounded and the
                    // view sync still lands.
                    if roll.turn_start {
                        // The label names this attacker now, with its card: it must not wait
                        // behind the previous blow's draining beats.
                        stage.show_turn_now(Some(attacker.clone()));
                        let _ = stage.stage(Beat::TurnStart {
                            actor: attacker.clone(),
                        });
                    }
                    if let Some((_, to)) = roll.walk
                        && let Some(to) = cell_of(to)
                    {
                        let _ = stage.stage_move(&attacker, to);
                    }
                    if state == "turn" {
                        return;
                    }
                    if roll.check {
                        // The die settles before its committed card is opened.
                        return;
                    }
                    if guard.techniques.contains_key(&roll.sequence) {
                        // Technique timelines own their card timing after contact.
                        return;
                    }
                    stage.set_attack_style(&attacker, roll.attack_style);
                    let _ = stage.stage(Beat::Approach { attacker, target });
                }
                "impact" if roll.check => {
                    if let Some(card) = &roll.card {
                        stage.show_roll_card(card.clone());
                    }
                }
                "impact" => {
                    if let Some(presentation) = guard.techniques.get(&roll.sequence) {
                        match technique_from_wire(presentation, &attacker, &target) {
                            Some((style, beat)) => {
                                stage.set_attack_style(&attacker, style);
                                if let Err(error) = stage.stage(beat)
                                    && let Some(host) = &guard.host
                                {
                                    mark(
                                        host,
                                        "data-battlefield-technique-error",
                                        &format!("{error:?}"),
                                    );
                                }
                                return;
                            }
                            None => {
                                if let Some(host) = &guard.host {
                                    mark(
                                        host,
                                        "data-battlefield-technique-error",
                                        "invalid_display_envelope",
                                    );
                                }
                                // An unknown optional display variant retains the ordinary outcome.
                            }
                        }
                    }
                    if let Some(label) = &roll.attack_label {
                        stage.set_attack_label(&attacker, label, &roll.damage_type);
                    }
                    let result = if roll.hit || roll.grazed {
                        AttackResult::Hit {
                            damage: roll.damage,
                            critical: roll.critical,
                            downed: roll.target_down,
                        }
                    } else {
                        AttackResult::Miss
                    };
                    let strike = if roll.fight_over {
                        Beat::FightEndingStrike {
                            attacker,
                            target,
                            result,
                            target_hit_points_after: after,
                        }
                    } else {
                        Beat::Strike {
                            attacker,
                            target,
                            result,
                            target_hit_points_after: after,
                        }
                    };
                    let _ = stage.stage(strike);
                }
                _ => {}
            }
        });
    });
    if state == "rolling" && roll.sequence != 0 {
        let phase_log = LIVE.with(|live| {
            let guard = live.borrow();
            let stage = guard
                .stage
                .as_ref()
                .filter(|_| matches!(guard.phase, Phase::Active))?;
            stage.with_stage(|stage| {
                let squares = roll.walk.map_or(0, |(from, to)| {
                    let Some((from, to)) = cell_of(from).zip(cell_of(to)) else {
                        return 1;
                    };
                    let squares = stage
                        .grid()
                        .path(from, to)
                        .map_or(1, |path| path.len().saturating_sub(1).max(1));
                    u32::try_from(squares).unwrap_or(u32::MAX)
                });
                let heading_ms = if roll.turn_start {
                    battle::beat_timing::turn_ms()
                } else {
                    0
                };
                let walk_ms = if squares == 0 {
                    0
                } else {
                    battle::beat_timing::move_ms(squares)
                };
                let (approach_ms, strike_ms, settle_ms) = battle::beat_timing::blow_parts_ms(
                    roll.attack_style == AttackStyle::Ranged,
                    roll.hit || roll.grazed,
                    roll.target_down,
                );
                format!(
                    "{}:{}:heading={heading_ms}:walk={walk_ms}:roll={}:approach={approach_ms}:strike={strike_ms}:settle={settle_ms}",
                    roll.sequence,
                    roll.attacker,
                    crate::stage_director::ROLLING_MS
                )
            })
        });
        if let Some(phase_log) = phase_log {
            LIVE.with(|live| {
                let mut live = live.borrow_mut();
                if live.battlefield_beat_log.len() >= BATTLEFIELD_BEAT_LOG_CAP {
                    live.battlefield_beat_log.pop_front();
                }
                live.battlefield_beat_log.push_back(phase_log);
            });
        }
    }

    if drained {
        sync("director");
    }
}

/// A queued cue can outlive the accepted view's recent-outcome window.
pub(super) fn attack_outcome(sequence: u64) -> Option<rpc::CombatOutcome> {
    LIVE.with(|live| live.borrow().outcomes.get(&sequence).cloned())
}

/// Opens or refreshes the card from the committed outcome. It is called by the delayed dice
/// presenter, so the card cannot appear at the same instant as the d20 tumble.
pub(super) fn show_attack_card(sequence: u64, card: RollCard) -> bool {
    LIVE.with(|live| {
        let live = live.borrow();
        if live.presenting_sequence != Some(sequence) {
            return false;
        }
        let Some(stage) = &live.stage else {
            return false;
        };
        stage.with_stage(|stage| {
            stage.open_attack_card(card);
        });
        true
    })
}

pub(super) fn begin_attack_presentation(outcome: rpc::CombatOutcome) {
    let sequence = outcome.sequence;
    LIVE.with(|live| {
        let mut live = live.borrow_mut();
        live.presenting_sequence = Some(sequence);
        live.damage_readable_sequence = None;
        live.pending_impact = None;
        live.outcomes.insert(sequence, outcome);
        if let Some(stage) = &live.stage {
            stage.with_stage(|stage| {
                // The live attack owns the dice seat even before the next feed redraw.
                stage.skip_initiative();
                stage.close_attack_card();
            });
        }
    });
}

/// Releases impact/HP/floating damage only after the actual damage dice have settled.
/// This is a presentation barrier, independent of the server cue's arrival time.
pub(super) fn damage_readable(sequence: u64) {
    let pending = LIVE.with(|live| {
        let mut live = live.borrow_mut();
        if live.presenting_sequence != Some(sequence) {
            return None;
        }
        live.damage_readable_sequence = Some(sequence);
        live.pending_impact
            .take()
            .filter(|roll| roll.sequence == sequence)
    });
    if let Some(roll) = pending {
        on_cue("present-impact", Some(&roll));
    }
}

pub(super) fn finish_attack_presentation(sequence: u64) {
    LIVE.with(|live| {
        let mut live = live.borrow_mut();
        if live.presenting_sequence != Some(sequence) {
            return;
        }
        if let Some(stage) = &live.stage {
            stage.with_stage(|stage| stage.close_attack_card());
        }
        live.presenting_sequence = None;
        live.damage_readable_sequence = None;
        live.pending_impact = None;
    });
}

pub(super) fn ordinary_presentation_active(sequence: u64) -> bool {
    LIVE.with(|live| live.borrow().presenting_sequence == Some(sequence))
}

pub(super) fn close_attack_card(sequence: u64) {
    LIVE.with(|live| {
        let live = live.borrow();
        if live.presenting_sequence == Some(sequence)
            && let Some(stage) = &live.stage
        {
            stage.with_stage(|stage| stage.close_attack_card());
        }
    });
}

/// How far the battlefield is with a fight's entrance, read by the display's battle machine:
/// pending while it mounts or plays its banner, initiative roll and roll-call; a fallback when it
/// will not appear.
pub(super) fn entrance() -> crate::stage_conductor::Battlefield {
    use crate::stage_conductor::Battlefield;
    // The initiative roll waits for a scan's splat to finish streaming in: when it has, stage it.
    let stage_initiative = LIVE.with(|live| {
        let live = live.borrow();
        matches!(live.phase, Phase::Active) && live.begun_live && !splat_streaming(&live)
    });
    if stage_initiative {
        sync("director");
    }
    LIVE.with(|live| {
        let mut live = live.borrow_mut();
        match live.phase {
            Phase::Idle | Phase::Mounting => Battlefield::Pending,
            Phase::Unavailable => Battlefield::Fallback,
            Phase::Active => {
                // The initiative roll and roll-call are on the battlefield's own HUD: the
                // entrance is over only once they have shown and finished.
                let showing = live.host.as_ref().is_some_and(|host| {
                    host.query_selector(".bf-hud[data-initiative]")
                        .ok()
                        .flatten()
                        .is_some()
                });
                if live.initiative_pending {
                    if showing {
                        live.initiative_seen = true;
                    } else if live.initiative_seen {
                        live.initiative_pending = false;
                    }
                }
                // Nothing is ready while the loading splash still holds the scene.
                let resolved = live.stage.as_ref().is_some_and(|stage| stage.is_resolved());
                let playing = !resolved
                    || splat_streaming(&live)
                    || live.begun_live
                    || live.initiative_pending
                    || live
                        .stage
                        .as_ref()
                        .and_then(|stage| stage.with_stage(|stage| stage.is_busy()))
                        .unwrap_or(false);
                if playing {
                    Battlefield::Pending
                } else {
                    Battlefield::Ready
                }
            }
        }
    })
}

/// Progress shown on the loading card, its source values, and whether initiative roll-call is
/// running. The adapter reports changes to the conductor so a live load does not time out.
pub(super) fn entrance_progress() -> (Option<u32>, Option<(u32, u32, bool)>, bool) {
    LIVE.with(|live| {
        let live = live.borrow();
        let progress = live.stage.as_ref().map(|stage| stage.scan_progress());
        let percent = progress.map(|(loaded, wanted, complete)| {
            if complete {
                100
            } else if wanted == 0 {
                0
            } else {
                ((loaded as f32 / wanted as f32).min(0.99) * 100.0).round() as u32
            }
        });
        (percent, progress, live.initiative_pending)
    })
}

/// Drops a battlefield whose entrance stopped progressing, removing its loading card before
/// the conductor releases the held opening blows on the flat fallback.
pub(super) fn fail_entrance() {
    let stage = LIVE.with(|live| {
        let mut live = live.borrow_mut();
        live.phase = Phase::Unavailable;
        live.stage.take()
    });
    drop(stage);
}

/// A scan's splat is still streaming in: part of the entrance, so nothing of the fight (not even
/// the initiative roll) starts until it is complete or has failed and the fallback shows.
fn splat_streaming(live: &Live) -> bool {
    live.host.as_ref().is_some_and(|host| {
        match host.get_attribute("data-battlefield-splat").as_deref() {
            Some("idle" | "loading" | "showing") => true,
            Some(_) => false,
            // Before the renderer first reports, a scan scene is still on its way in.
            None => live.expects_splat,
        }
    })
}

/// The strike result for `sequence` landed on screen: the feed may show its health change.
/// A change re-syncs once (at most once per beat), so cards land with the strike.
pub(super) fn set_presented(sequence: u64, writer: &str) {
    let changed = LIVE.with(|live| {
        let mut live = live.borrow_mut();
        let presented = battle::advance_presented_sequence(live.presented_seq, sequence);
        if presented == live.presented_seq {
            return false;
        }
        live.presented_seq = presented;
        true
    });
    if changed {
        sync(writer);
    }
}

/// The renderer's nominal phase budgets for recent battlefield turns, oldest first.
pub(super) fn beat_log() -> String {
    LIVE.with(|live| {
        live.borrow()
            .battlefield_beat_log
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(",")
    })
}

/// Whether accepted blows or their held movement are still waiting on the battlefield.
pub(super) fn has_pending() -> bool {
    LIVE.with(|live| {
        let live = live.borrow();
        live.pending_cues > 0 || !live.held_moves.is_empty()
    })
}

/// The coherent catch-up jump: cut every playing and queued stage beat and land on the
/// newest token state at once, then feed it. Nothing is replayed on any layer.
pub(super) fn jump_to_latest() {
    LIVE.with(|live| {
        let mut live = live.borrow_mut();
        live.pending_cues = 0;
        live.held_moves.clear();
        if let Some(stage) = &live.stage {
            let _cut = stage.with_stage(|stage| stage.skip_to_idle());
        }
    });
    sync("skip");
}

/// Whether the battlefield is still showing beats (or its effects): the next blow's dice wait.
pub(super) fn is_busy() -> bool {
    LIVE.with(|live| {
        let live = live.borrow();
        matches!(live.phase, Phase::Active)
            && (live.presenting_sequence.is_some()
                || live
                    .stage
                    .as_ref()
                    .and_then(|stage| stage.with_stage(|stage| stage.is_busy()))
                    .unwrap_or(false))
    })
}

/// A mover of an accepted view that has outcomes still to be shown stays on `from` until the cue
/// stream reaches its turn. The first hold of a mover stands (a newer view carries the same walk).
pub(super) fn hold_move(name: &str, from: GridCell) {
    let Some(from) = cell_of(from) else {
        return;
    };
    LIVE.with(|live| {
        let mut live = live.borrow_mut();
        if !live.held_moves.iter().any(|(held, ..)| held == name) {
            live.held_moves.push((name.to_owned(), from, now_ms()));
        }
    });
}

/// Where the newest accepted view stands everyone (see `Live::accepted_cells`).
pub(super) fn accept_cells(cells: &std::collections::HashMap<String, GridCell>) {
    LIVE.with(|live| live.borrow_mut().accepted_cells.clone_from(cells));
}

/// All accepted rolls, including those waiting in the director rather than the renderer.
pub(super) fn accept_cues(count: usize) {
    LIVE.with(|live| {
        let mut live = live.borrow_mut();
        live.pending_cues += count;
        live.pending_at = now_ms();
    });
}

/// Nothing is waiting any more (a restart, or the fight is over): holds are released.
pub(super) fn drop_pending() {
    LIVE.with(|live| {
        let mut live = live.borrow_mut();
        live.held_moves.clear();
        live.pending_cues = 0;
    });
    sync("director");
}

/// How long a hold or an expectation may outlive the cues that should release it.
const HOLD_PATIENCE_MS: f64 = 90_000.0;

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map_or(0.0, |performance| performance.now())
}

fn cell_of((col, row): GridCell) -> Option<Cell> {
    Some(Cell::new(
        i32::try_from(col).ok()?,
        i32::try_from(row).ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::{Cell, LIVE, has_pending, now_ms};

    #[test]
    fn large_and_huge_target_highlights_cover_the_supplied_north_west_footprint() {
        for (size, extent) in [
            (rpc::CreatureSize::Medium, 1),
            (rpc::CreatureSize::Large, 2),
            (rpc::CreatureSize::Huge, 3),
        ] {
            let member = rpc::PartyMemberView {
                size: size as i32,
                cell: Some(rpc::GridCell { col: 4, row: 2 }),
                ..Default::default()
            };
            let cells = super::participant_cells(&member);
            assert_eq!(cells.len(), (extent * extent) as usize);
            assert!(cells.contains(&Cell::new(4, 2)));
            assert!(cells.contains(&Cell::new(4 + extent - 1, 2 + extent - 1)));
        }
    }

    use df_protocol::common as rpc;

    #[test]
    fn a_late_scene_fighter_depiction_reaches_the_next_snapshot() {
        let member = rpc::PartyMemberView {
            name: "Jessa".to_owned(),
            portrait: "portrait-shop-soldier".to_owned(),
            ..Default::default()
        };
        let mut scene = rpc::GameplayScene {
            cast: vec![rpc::SceneActor {
                name: member.name.clone(),
                portrait: member.portrait.clone(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            super::participant_live_art(&member, false, &scene, &[]),
            None
        );

        let ready = format!("/gameplay/imagery/{}", "a".repeat(64));
        scene.cast[0].live_sprite = ready.clone();
        assert_eq!(
            super::participant_live_art(&member, false, &scene, &[]),
            Some(ready)
        );
        let mut stage = df_render::battle::Stage::new(
            df_render::battle::load_embedded("salt-quay")
                .unwrap()
                .unwrap(),
        );
        let fighter = df_render::battle::Member {
            id: "jessa".to_owned(),
            name: member.name.clone(),
            side: super::Side::Enemy,
            hit_points: 11,
            maximum_hit_points: 11,
            height_m: super::PERSON_HEIGHT_M,
            figure: None,
            portrait: None,
            attack: super::AttackStyle::Melee,
            voice: None,
            cell: None,
            position_sequence: 0,
            statuses: Vec::new(),
        };
        let mut hero = fighter.clone();
        hero.id = "hero".to_owned();
        hero.name = "Brakka".to_owned();
        hero.side = super::Side::Hero;
        let mut snapshot = df_render::battle::CombatSnapshot {
            members: vec![hero, fighter],
            ..Default::default()
        };
        let mut feed = df_render::battle::CombatFeed::default();
        assert!(feed.apply(&mut stage, &snapshot).is_some());
        stage.skip_to_idle();
        let cell = stage
            .tokens()
            .find(|token| token.name == "Jessa")
            .unwrap()
            .cell;
        let ready = super::participant_live_art(&member, false, &scene, &[]).unwrap();
        snapshot.members[1].figure = Some(ready.clone());
        snapshot.members[1].portrait = Some(ready.clone());
        assert!(feed.apply(&mut stage, &snapshot).is_some());
        let token = stage.tokens().find(|token| token.name == "Jessa").unwrap();
        assert_eq!(token.figure.as_deref(), Some(ready.as_str()));
        assert_eq!(token.portrait.as_deref(), Some(ready.as_str()));
        assert_eq!(token.cell, cell);
        assert_eq!(stage.tokens().count(), 2);
        // A different person's art cannot answer this participant.
        scene.cast[0].portrait = "portrait-shop-adventurer".to_owned();
        assert_eq!(
            super::participant_live_art(&member, false, &scene, &[]),
            None
        );
    }

    #[test]
    fn world_floor_centre_is_open_in_the_largest_connected_area() {
        use df_render::battle::{self, Vec3};
        let mut vertices = Vec::new();
        let mut triangles = Vec::new();
        for (col, row) in (0..3)
            .flat_map(|row| (0..3).map(move |col| (col, row)))
            .filter(|cell| *cell != (1, 1))
            .chain([(10, 10)])
        {
            let offset = vertices.len() as u32;
            let (x, z) = (col as f32, row as f32);
            vertices.extend([
                Vec3::new(x, 0.0, z),
                Vec3::new(x, 0.0, z + 1.0),
                Vec3::new(x + 1.0, 0.0, z),
                Vec3::new(x + 1.0, 0.0, z + 1.0),
            ]);
            triangles.extend([
                [offset, offset + 1, offset + 2],
                [offset + 2, offset + 1, offset + 3],
            ]);
        }
        let mesh = battle::ColliderMesh::from_triangles(vertices, triangles).unwrap();
        let grid = battle::world_grid(
            &mesh,
            &battle::SplatTransform::IDENTITY,
            &battle::WorldGridSettings {
                cell_m: 1.0,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(grid.largest_open_area().unwrap().squares, 8);
        // The centre of this ring has no floor. Choose the nearest open square, with a stable tie.
        assert_eq!(super::open_floor_centre(&grid), Some([1.5, 0.5]));
    }

    #[test]
    fn battlefield_uses_the_party_maximum_for_a_hero_health_bar() {
        let party = [rpc::PartyMemberView {
            name: "Brakka".to_owned(),
            maximum_hit_points: 13,
            ..Default::default()
        }];
        let combat = rpc::PartyMemberView {
            name: "Brakka".to_owned(),
            maximum_hit_points: 11,
            ..Default::default()
        };

        assert_eq!(super::member_maximum_hit_points(&combat, &party), 13);
        assert_eq!(
            crate::gameplay_browser::live_health::display_tier(Some((13, 13)), false),
            Some(crate::health_tier::HealthTier::Healthy)
        );
    }

    #[test]
    fn reaction_prompt_names_the_mover_reactor_and_server_countdown() {
        assert_eq!(
            super::reaction_prompt("Brakka", "Sigrun", Some(7)),
            "Sigrun decides: Opportunity attack as Brakka moves (7s remaining)"
        );
    }

    #[test]
    fn tv_status_badges_spell_out_unconscious_and_bloodied() {
        let css = include_str!("live_battlefield.css");

        assert!(css.contains("content:\"Unconscious\""));
        assert!(css.contains("content:\"Bloodied\""));
    }

    #[test]
    fn attack_card_and_float_layout_fit_compact_and_tv_viewports() {
        let css = include_str!("live_battlefield.css");
        assert!(css.contains(
            "[data-attack-card=open] .combat-roll-panel .combat-roll{visibility:visible!important}"
        ));
        assert!(css.contains("width:min(92vw,calc(var(--u)*76))"));
        assert!(css.contains("left:50%;right:auto"));
        assert!(css.contains("max-width:min(51%,calc(100vw - 2 * clamp(12px,3vw,40px)))"));

        for (viewport_width, viewport_height) in [(800.0_f32, 574.0_f32), (1920.0, 1080.0)] {
            let unit = (viewport_width * 0.01).min(viewport_height * 0.017_778);
            let card_width = (viewport_width * 0.92).min(unit * 76.0);
            let card_left = (viewport_width - card_width) * 0.5;
            let safe_gutter = (viewport_width * 0.03).clamp(12.0, 40.0);
            let float_max_width = (viewport_width * 0.51).min(viewport_width - safe_gutter * 2.0);

            assert!(card_left >= safe_gutter);
            assert!(card_left + card_width <= viewport_width - safe_gutter);
            assert!(float_max_width <= viewport_width - safe_gutter * 2.0);
        }
    }

    #[test]
    fn tv_roll_hides_stale_dm_caption_while_the_card_is_visible() {
        let css = include_str!("live_tv.css");

        assert!(css.contains(".live-screen[data-role=display]:has(.tv-roll) .tv-dm{display:none}"));
    }

    /// SRD 5.2.1 p. 177 (Attack [Action]); this test covers display ordering only.
    #[test]
    fn held_enemy_move_keeps_the_beat_unpresented_until_its_attack() {
        let previous = LIVE.with(|live| {
            let mut live = live.borrow_mut();
            let pending_cues = std::mem::replace(&mut live.pending_cues, 0);
            let held_moves = std::mem::replace(
                &mut live.held_moves,
                vec![("Weasel 1".to_owned(), Cell::new(1, 1), now_ms())],
            );
            (pending_cues, held_moves)
        });

        let has_pending_move = has_pending();

        LIVE.with(|live| {
            let mut live = live.borrow_mut();
            live.pending_cues = previous.0;
            live.held_moves = previous.1;
        });

        assert!(has_pending_move);
    }
}

/// Applies the latest accepted snapshot again: called when a turn's beats are over so the turn
/// label and marker follow the server's active combatant.
pub(super) fn resync() {
    sync("director");
}

fn record_card_values(
    live: &mut Live,
    cards: impl IntoIterator<Item = (String, u32)>,
    writer: &str,
) {
    for (name, value) in cards {
        if live.card_values.get(&name) == Some(&value) {
            continue;
        }
        let sequence = live
            .beat_hp
            .iter()
            .filter(|(sequence, target, ..)| target == &name && *sequence <= live.presented_seq)
            .map(|(sequence, ..)| *sequence)
            .max()
            .unwrap_or(0);
        if live.card_log.len() >= CARD_LOG_CAP {
            live.card_log.pop_front();
        }
        live.card_log.push_back(format!(
            "{name}:{value}:{sequence}:{writer}:{:.0}",
            now_ms()
        ));
        live.card_values.insert(name, value);
    }
}

/// Records and returns the most recent actual hit-point card writes.
pub(super) fn card_log() -> String {
    LIVE.with(|live| {
        let mut live = live.borrow_mut();
        let cards = live
            .stage
            .as_ref()
            .and_then(|stage| {
                stage.with_stage(|stage| {
                    stage
                        .frame()
                        .figures
                        .iter()
                        .filter_map(|figure| {
                            figure
                                .hit_points
                                .map(|hit_points| (figure.name.clone(), hit_points.current))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .unwrap_or_default();
        record_card_values(&mut live, cards, "stage");
        live.card_log.iter().cloned().collect::<Vec<_>>().join(",")
    })
}

/// The fight's exit has begun (its finishing blow has been shown out): the battlefield fades out
/// now, under the scene transition, instead of being cut when the next scene is drawn.
pub(super) fn fade_out() {
    let host = LIVE.with(|live| live.borrow().host.clone());
    if let Some(container) =
        host.and_then(|host| host.query_selector(".battlefield").ok().flatten())
    {
        // The fade is cosmetic; the stage is dropped by `release` either way.
        let _ = container.set_attribute("data-closing", "");
    }
}

/// A line is spoken by the figure called `speaker_name`: a speech bubble opens over it, its tail to
/// the speaker's head, and the camera eases in on it. Call it when the line's voice starts, and
/// [`end_speech`] when it ends. Returns false (and shows nothing) when that speaker is not on the
/// field, so the caller can fall back to its own presentation.
pub(super) fn speak(speaker_name: &str, text: &str, line_id: u64) -> bool {
    LIVE.with(|live| {
        let live = live.borrow();
        live.stage
            .as_ref()
            .filter(|_| matches!(live.phase, Phase::Active))
            .and_then(|stage| stage.with_stage(|stage| stage.speak(speaker_name, text, line_id)))
            .unwrap_or(false)
    })
}

/// The line `line_id` ended: its bubble closes and the camera eases back. Any other line's end
/// is ignored.
pub(super) fn end_speech(line_id: u64) {
    LIVE.with(|live| {
        if let Some(stage) = &live.borrow().stage {
            stage.with_stage(|stage| stage.end_speech(line_id));
        }
    });
}

/// The line the conductor is playing, for the fight's loading splash: the newest opened line
/// (the Dungeon Master label or the speaker's name with the words) shows below the progress
/// bar until the line closes or the splash goes. A no-op without a stage; the battlefield's
/// own bubbles open only once figures stand, so nothing ever doubles.
pub(super) fn splash_caption(label: &str, text: &str, showing: bool) {
    LIVE.with(|live| {
        let mut live = live.borrow_mut();
        live.pending_caption = showing.then(|| (label.to_owned(), text.to_owned()));
        if let Some(stage) = &live.stage {
            stage.set_splash_caption(if showing { Some((label, text)) } else { None });
        }
    });
}

fn reset_fight(live: &mut Live) {
    live.battlefield_component_id = None;
    live.banner_component_id = None;
    live.turn_component_id = None;
    live.last_turn_key = None;
    live.world_mount = None;
    live.accepted_cells.clear();
    live.reaction = None;
    live.reaction_seconds_remaining = None;
    live.shown_prompt = None;
    live.shown_reaction = None;
    live.pending_caption = None;
    live.held_moves.clear();
    live.pending_cues = 0;
    live.reachable.clear();
    live.targets.clear();
    live.initiative_pending = false;
    live.initiative_seen = false;
    live.phase = Phase::Idle;
    live.latest = None;
    live.host = None;
    live.presented_seq = 0;
    live.latest_seq = 0;
    live.beat_hp.clear();
    live.beat_down.clear();
    live.active_strike = None;
    live.battlefield_beat_log.clear();
    live.card_values.clear();
    live.card_log.clear();
    live.outcomes.clear();
    live.presenting_sequence = None;
    live.damage_readable_sequence = None;
    live.pending_impact = None;
    live.feed = CombatFeed::new(FeedMode::HoldForCues);
}

/// The fight ended or the view scope changed. A stage that is still playing (the finishing blow)
/// is allowed to finish, then fades out; one that never mounted or is unavailable is just reset.
pub(super) fn release() {
    // A view that is not combat means the next fight begins in front of the display.
    SEEN_NON_COMBAT.with(|seen| seen.set(true));
    let (field_id, scene) = LIVE.with(|live| {
        let mut live = live.borrow_mut();
        let scene = live
            .host
            .as_ref()
            .and_then(|host| host.get_attribute("data-battlefield-scene"))
            .unwrap_or_default();
        let field_id = live.battlefield_component_id.take();
        if let Some(id) = live.banner_component_id.take() {
            battle_component(
                None,
                df_observe::presentation::PresentationComponent::CombatBanner,
                id,
                &scene,
                df_observe::presentation::ComponentPhase::Hidden,
                "ended",
            );
        }
        if let Some(id) = live.turn_component_id.take() {
            battle_component(
                None,
                df_observe::presentation::PresentationComponent::TurnChange,
                id,
                &scene,
                df_observe::presentation::ComponentPhase::Hidden,
                "ended",
            );
        }
        live.stage = None;
        reset_fight(&mut live);
        (field_id, scene)
    });
    if let Some(id) = field_id {
        battle_component(
            None,
            df_observe::presentation::PresentationComponent::Battlefield,
            id,
            &scene,
            df_observe::presentation::ComponentPhase::Hidden,
            "ended",
        );
    }
    live_sfx::set_battlefield_owns_combat(false);
}

fn record_battle_overlays(live: &mut Live, snapshot: &CombatSnapshot) {
    use df_observe::presentation::{ComponentPhase, PresentationComponent};

    let Some(host) = live.host.as_ref() else {
        return;
    };
    let scene = host
        .get_attribute("data-battlefield-scene")
        .unwrap_or_default();
    let banner = host.query_selector(".bf-banner[data-open]").ok().flatten();
    match (live.banner_component_id, banner.as_ref()) {
        (None, Some(element)) => {
            let id = super::presentation_timing::next_component_id();
            live.banner_component_id = Some(id);
            battle_component(
                Some(element),
                PresentationComponent::CombatBanner,
                id,
                &scene,
                ComponentPhase::Opened,
                "ready",
            );
            battle_component(
                Some(element),
                PresentationComponent::CombatBanner,
                id,
                &scene,
                ComponentPhase::Readable,
                "ready",
            );
        }
        (Some(id), None) => {
            live.banner_component_id = None;
            battle_component(
                None,
                PresentationComponent::CombatBanner,
                id,
                &scene,
                ComponentPhase::Hidden,
                "ended",
            );
        }
        _ => {}
    }

    let turn_key = snapshot
        .active
        .as_ref()
        .map(|active| format!("{}:{active}", snapshot.round));
    if turn_key != live.last_turn_key {
        if let Some(id) = live.turn_component_id.take() {
            battle_component(
                None,
                PresentationComponent::TurnChange,
                id,
                &scene,
                ComponentPhase::Hidden,
                "changed",
            );
        }
        live.last_turn_key = turn_key.clone();
        if let Some(turn_key) = turn_key {
            let id = super::presentation_timing::identity(&turn_key);
            live.turn_component_id = Some(id);
            let turn_element = host.query_selector(".bf-turn").ok().flatten();
            battle_component(
                turn_element.as_ref(),
                PresentationComponent::TurnChange,
                id,
                &scene,
                ComponentPhase::Opened,
                "changed",
            );
            battle_component(
                turn_element.as_ref(),
                PresentationComponent::TurnChange,
                id,
                &scene,
                ComponentPhase::Readable,
                "changed",
            );
        }
    }
}

fn battle_component(
    element: Option<&Element>,
    component: df_observe::presentation::PresentationComponent,
    id: u64,
    scene: &str,
    phase: df_observe::presentation::ComponentPhase,
    reason: &'static str,
) {
    super::presentation_timing::component(element, component, id, scene, phase, reason, 0);
}
