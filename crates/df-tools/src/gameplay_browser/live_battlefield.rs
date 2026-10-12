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
use std::cell::Cell as StdCell;

const STYLE: &str = include_str!("live_battlefield.css");
const STYLE_ATTRIBUTE: &str = "data-battlefield-style";
const PREVIEW_HASH: &str = "#battle-preview";
/// The default tier of a streamed scan: the second largest, not the largest.
const DEFAULT_SPLAT_LOD: u32 = 1;
const DEFAULT_PREVIEW_HEROES: usize = 3;
const DEFAULT_PREVIEW_ENEMIES: usize = 1;
/// The most figures per side the preview stands without a layout to seat them.
const PREVIEW_MAX_PER_SIDE: usize = 6;

thread_local! {
    static PREVIEW: RefCell<Option<BattleStage>> = const { RefCell::new(None) };
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
    Statuses,
    Sheet(usize),
    /// The heroes and the goblin standing on the layout under the wide shot, for layout review.
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
        fps: battle::BattleFps::DEFAULT,
        feel_off: false,
        tempo: None,
        splat_lod: DEFAULT_SPLAT_LOD,
        heroes: DEFAULT_PREVIEW_HEROES,
        enemies: DEFAULT_PREVIEW_ENEMIES,
        reach: false,
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
            other if other.starts_with("cam=") => request.free_camera = free_camera(&other[4..]),
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
fn attack_style_for(figure: &str) -> AttackStyle {
    match figure {
        "figure-hero-3" => AttackStyle::Ranged,
        _ => AttackStyle::Melee,
    }
}

const GOBLIN: (&str, &str, &str) = (
    "goblin",
    "figure-ant-goblin-warrior",
    "portrait-ant-goblin-warrior",
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
/// figures) and goblins on a layout's spawn cells (or the derived formation when the scene has no
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
            1.35,
        ));
    }
    for (index, cell) in seats.enemies.iter().enumerate() {
        let (id, name) = if index == 0 {
            (GOBLIN.0.to_owned(), "Goblin Warrior".to_owned())
        } else {
            (
                format!("{}-{}", GOBLIN.0, index + 1),
                format!("Goblin Warrior {}", index + 1),
            )
        };
        tokens.push(combatant(
            &id,
            &name,
            Side::Enemy,
            *cell,
            GOBLIN.1,
            GOBLIN.2,
            1.0,
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
        roll("goblin", 11, 2),
        roll("nimrael", 10, 3),
    ]
}

/// The looping preview fight: a miss, hits both ways, a ranged hit and miss, a heal, then a
/// finishing blow.
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
        (1, attack("tobin", "goblin", AttackResult::Miss, None)),
        (1, attack("goblin", "nimrael", hit(3, false), Some(9))),
        (2, attack("sylvie", "goblin", hit(4, false), Some(8))),
        (2, attack("nimrael", "goblin", hit(5, false), Some(3))),
        (
            2,
            Beat::Heal {
                source: TokenId::new("tobin"),
                target: TokenId::new("nimrael"),
                amount: 3,
                target_hit_points_after: Some(12),
            },
        ),
        (3, attack("goblin", "tobin", hit(2, false), Some(10))),
        (3, attack("sylvie", "goblin", AttackResult::Miss, None)),
        (3, attack("tobin", "goblin", hit(3, true), Some(0))),
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
        });
    }
    let scene_for_choice = scene.clone();
    mark(&host, "data-battlefield-scene", &request.scene_id);
    let stage = BattleStage::mount(
        &host,
        scene,
        StageOptions {
            procedural_splats: request.procedural_splats,
            fixed_quality: request.fixed,
            fps: request.fps,
            feel_off: request.feel_off,
            slow: request.slow,
            splat_lod: request.splat_lod,
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
                PreviewMode::Fight | PreviewMode::Statuses | PreviewMode::Still => preview_tokens(
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
                        target: TokenId::new("goblin"),
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
                        attacker: TokenId::new("goblin"),
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
                    target: TokenId::new("goblin"),
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
                    let queued = with_preview(|stage| {
                        stage.set_round(round);
                        stage.stage(beat.clone()).is_ok()
                    });
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
        stage.set_attack_label(&attacker, &label);
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
            1.35,
        ),
        combatant(
            "s1",
            "",
            Side::Hero,
            seats.heroes[1],
            "figure-hero-3",
            "portrait-hero-3",
            1.35,
        ),
        combatant(
            "s2",
            "",
            Side::Hero,
            seats.heroes[2],
            "figure-hero-2",
            "portrait-hero-2",
            1.35,
        ),
        combatant(
            "s3",
            "",
            Side::Enemy,
            seats.enemies[0],
            "figure-ant-goblin-warrior",
            "portrait-ant-goblin-warrior",
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
use df_render::battle::{CombatFeed, CombatSnapshot, FeedMode, Member, Outcome, RollCard};

/// What the cue director knows about a roll, copied from its own detail.
pub(super) struct RollSummary {
    pub(super) attacker: String,
    pub(super) target: String,
    pub(super) hit: bool,
    pub(super) critical: bool,
    pub(super) grazed: bool,
    pub(super) damage: u32,
    pub(super) target_down: bool,
    pub(super) card: Option<RollCard>,
}

enum Phase {
    Idle,
    Mounting,
    Active,
    /// The fight ended: the stage finishes what it is showing, fades out, then is dropped.
    Closing,
    Unavailable,
}

struct Live {
    phase: Phase,
    host: Option<Element>,
    stage: Option<BattleStage>,
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
}

thread_local! {
    static LIVE: RefCell<Live> = RefCell::new(Live {
        phase: Phase::Idle,
        host: None,
        stage: None,
        feed: CombatFeed::new(FeedMode::HoldForCues),
        latest: None,
        begun_live: false,
        expects_splat: false,
        initiative_pending: false,
        initiative_seen: false,
        reachable: Vec::new(),
        targets: Vec::new(),
    });
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The stem part of an asset path: `/assets/ui/generated/portrait-ant-guard.webp` -> `portrait-ant-guard`.
fn stem_of(path: &str) -> Option<&str> {
    let name = path.rsplit('/').next()?;
    let stem = name.split('.').next()?;
    (!stem.is_empty()
        && stem
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
    .then_some(stem)
}

/// `portrait-ant-x`, `sprite-ant-x` and `figure-ant-x` all name the same creature `ant-x`.
fn creature_of(stem: &str) -> &str {
    let base = ["portrait-", "sprite-", "figure-"]
        .iter()
        .find_map(|prefix| stem.strip_prefix(prefix))
        .unwrap_or(stem);
    base.strip_suffix("-full").unwrap_or(base)
}

fn enemy_height_m(creature: &str) -> f32 {
    match creature {
        c if c.contains("goblin") || c.contains("kobold") => 1.0,
        c if c.contains("giant-rat") => 0.55,
        c if c.contains("weasel") => 0.6,
        c if c.contains("wolf") => 0.9,
        c if c.contains("mastiff") => 0.85,
        _ => 1.75,
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

/// The fight's own painting, used as the backdrop behind the battlefield (the confrontation
/// setting's art), when it names a story scene picture the display already serves.
fn backdrop_stem(view: &LiveView<'_>) -> Option<String> {
    stem_of(&view.scene.scene_asset)
        .filter(|stem| stem.starts_with("scene-"))
        .map(str::to_owned)
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
fn content_rating() -> battle::ContentRating {
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
fn voice_set(key: &str) -> Option<battle::VoiceSet> {
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

fn snapshot_of(view: &LiveView<'_>) -> Option<CombatSnapshot> {
    let combat = view.journey.combat.as_ref()?;
    let party = view.journey.party.len();
    let creature = stem_of(&view.scene.speaker_portrait)
        .map(creature_of)
        .unwrap_or("bandit");
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
            let portrait = if authoritative {
                member.portrait.clone()
            } else if hero {
                format!("portrait-hero-{hero_art}")
            } else {
                format!("portrait-{creature}")
            };
            let figure = if authoritative {
                portrait.replacen("portrait-", "figure-", 1)
            } else if hero {
                format!("figure-hero-{hero_art}")
            } else {
                format!("figure-{creature}")
            };
            Member {
                id: id_of(index, member),
                name: member.name.clone(),
                side: if hero { Side::Hero } else { Side::Enemy },
                hit_points: member.hit_points,
                maximum_hit_points: member.maximum_hit_points,
                height_m: if hero {
                    1.35
                } else {
                    enemy_height_m(creature_of(&portrait))
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
    let Some(snapshot) = snapshot_of(view) else {
        return;
    };
    let choice = scene_choice(view);
    let backdrop = backdrop_stem(view);
    let key = layout_key(view, &snapshot);
    let phase_idle = LIVE.with(|live| {
        let mut live = live.borrow_mut();
        if let Some(combat) = &view.journey.combat {
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
                        .find(|member| member.participant_id == target.participant_id)?
                        .cell
                        .as_ref()
                })
                .filter_map(|cell| {
                    Some(Cell::new(
                        i32::try_from(cell.col).ok()?,
                        i32::try_from(cell.row).ok()?,
                    ))
                })
                .collect();
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
        );
    } else {
        sync();
    }
}

fn start_mount(
    client: &Rc<RefCell<Client>>,
    choice: &battle::SceneChoice,
    backdrop: Option<String>,
    key: &str,
    server_layout: Option<String>,
    story_location: &str,
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
    // The fight's title is the story place, never the scan's source name (that belongs in the
    // credits with its attribution).
    let place_title = story_scene
        .as_ref()
        .map_or_else(|| location_scene.title.clone(), |story| story.title.clone());
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
    scene.title = place_title;
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
    let layout = server_layout.or_else(|| chosen_layout_id(&scene, key));
    let _ = host.set_attribute(
        "data-battlefield-layout",
        layout.as_deref().unwrap_or("none"),
    );
    if install_style(&document).is_err() {
        LIVE.with(|live| live.borrow_mut().phase = Phase::Unavailable);
        return;
    }
    LIVE.with(|live| {
        let mut live = live.borrow_mut();
        live.phase = Phase::Mounting;
        live.host = Some(host.clone());
        live.expects_splat = expects_splat;
    });
    wasm_bindgen_futures::spawn_local(async move {
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
            },
        )
        .await;
        let available = matches!(stage.availability(), Availability::Available);
        if available {
            stage.set_sound_sink(Rc::new(|event| {
                live_sfx::play_event(&event, content_rating());
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
            }
            available
        });
        if keep {
            sync();
        }
    });
}

/// Applies the latest snapshot; notices a stage that went unavailable (device lost).
fn sync() {
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
        let fits = stage
            .with_stage(|stage| {
                stage.set_tactical_highlights(live.reachable.clone(), live.targets.clone());
                feed.apply(stage, &shown)
            })
            .flatten();
        if live.begun_live && fits.is_some() && !shown.initiative.is_empty() {
            live.initiative_pending = true;
            live.initiative_seen = false;
        }
        live.begun_live = false;
        if fits.is_none() {
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

/// The cue director entered a state: stage the matching half of the attack.
pub(super) fn on_cue(state: &str, roll: Option<&RollSummary>) {
    let Some(roll) = roll else {
        return;
    };
    LIVE.with(|live| {
        let guard = live.borrow();
        let Some(stage) = guard
            .stage
            .as_ref()
            .filter(|_| matches!(guard.phase, Phase::Active))
        else {
            return;
        };
        let after = guard.latest.as_ref().and_then(|snapshot| {
            snapshot
                .members
                .iter()
                .find(|member| member.name == roll.target)
                .map(|member| member.hit_points)
        });
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
            match state {
                "rolling" => {
                    // Dropped, not fatal: the queue is bounded and the view sync still lands.
                    let _ = stage.stage(Beat::Approach { attacker, target });
                    // The card waits for the camera to arrive on the two fighters.
                    if let Some(card) = roll.card.clone() {
                        stage.show_roll_card(card);
                    }
                }
                "impact" => {
                    // The view's latest outcome for this pair names the attack.
                    let label = guard.latest.as_ref().and_then(|snapshot| {
                        snapshot
                            .outcomes
                            .iter()
                            .rev()
                            .find(|outcome| {
                                outcome.actor == attacker.0 && outcome.target == target.0
                            })
                            .and_then(Outcome::attack_label)
                    });
                    if let Some(label) = label {
                        stage.set_attack_label(&attacker, &label);
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
                    let _ = stage.stage(Beat::Strike {
                        attacker,
                        target,
                        result,
                        target_hit_points_after: after,
                    });
                }
                _ => {}
            }
        });
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
        sync();
    }
    LIVE.with(|live| {
        let mut live = live.borrow_mut();
        match live.phase {
            Phase::Idle | Phase::Mounting => Battlefield::Pending,
            Phase::Unavailable | Phase::Closing => Battlefield::Fallback,
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

/// Applies the latest accepted snapshot again: called when a turn's beats are over so the turn
/// label and marker follow the server's active combatant.
pub(super) fn resync() {
    sync();
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
// Called by the display's line sequencer (the stage-machines work), which lands separately.
#[allow(dead_code)]
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
// Called by the display's line sequencer (the stage-machines work), which lands separately.
#[allow(dead_code)]
pub(super) fn end_speech(line_id: u64) {
    LIVE.with(|live| {
        if let Some(stage) = &live.borrow().stage {
            stage.with_stage(|stage| stage.end_speech(line_id));
        }
    });
}

fn reset_fight(live: &mut Live) {
    live.initiative_pending = false;
    live.initiative_seen = false;
    live.phase = Phase::Idle;
    live.latest = None;
    live.host = None;
    live.feed = CombatFeed::new(FeedMode::HoldForCues);
}

/// The fight ended or the view scope changed. A stage that is still playing (the finishing blow)
/// is allowed to finish, then fades out; one that never mounted or is unavailable is just reset.
pub(super) fn release() {
    // A view that is not combat means the next fight begins in front of the display.
    SEEN_NON_COMBAT.with(|seen| seen.set(true));
    let closing = LIVE.with(|live| {
        let mut live = live.borrow_mut();
        match live.phase {
            Phase::Idle | Phase::Closing => false,
            Phase::Mounting | Phase::Unavailable => {
                live.stage = None;
                reset_fight(&mut live);
                false
            }
            Phase::Active => {
                live.phase = Phase::Closing;
                live.latest = None;
                true
            }
        }
    });
    if closing {
        wasm_bindgen_futures::spawn_local(close_when_idle());
    }
}

const CLOSE_POLL_MS: u32 = 200;
const CLOSE_PATIENCE_POLLS: u32 = 40;
const FADE_OUT_MS: u32 = 700;

async fn close_when_idle() {
    for _ in 0..CLOSE_PATIENCE_POLLS {
        let busy = LIVE.with(|live| {
            live.borrow()
                .stage
                .as_ref()
                .and_then(|stage| stage.with_stage(|stage| stage.is_busy()))
                .unwrap_or(false)
        });
        if !busy {
            break;
        }
        TimeoutFuture::new(CLOSE_POLL_MS).await;
    }
    let host = LIVE.with(|live| live.borrow().host.clone());
    if let Some(container) =
        host.and_then(|host| host.query_selector(".battlefield").ok().flatten())
    {
        // The fade is cosmetic; the stage is dropped either way.
        let _ = container.set_attribute("data-closing", "");
    }
    TimeoutFuture::new(FADE_OUT_MS).await;
    let dropped = LIVE.with(|live| {
        let mut live = live.borrow_mut();
        let stage = live.stage.take();
        reset_fight(&mut live);
        stage
    });
    drop(dropped);
    live_sfx::set_battlefield_owns_combat(false);
}
