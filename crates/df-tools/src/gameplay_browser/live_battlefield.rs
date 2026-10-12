//! Adapter between the shared display and the 3D battlefield in `df-render`.
//!
//! The battlefield only draws staging. Positions are STAGING ONLY today: the rules engine has no
//! positions, movement or range (one melee enemy, everyone in reach), so this module derives a
//! stable formation from the combat view (party members by seat on the near side, the enemy
//! opposite, on walkable cells of the scene's grid) and animates the step-beside and lunge
//! cosmetically. Nothing here decides an outcome: every hit, miss, damage figure and downing is
//! copied from the server's `CombatOutcome`. When the server later supplies cells, build the
//! tokens with those cells instead of `derive_formation`; the renderer does not change.
//!
//! `#battle-preview[=<location-id>][&splat][&reduced]` mounts the battlefield alone and loops a
//! scripted fight with no server calls, for looking at the renderer without playing a game.

use super::*;
use df_render::battle::{
    self, AttackResult, Availability, BattleScene, BattleStage, Beat, Cell, HitPoints, Side,
    StageOptions, Token, TokenId,
};
use gloo_timers::future::TimeoutFuture;
use std::cell::Cell as StdCell;

const STYLE: &str = include_str!("live_battlefield.css");
const STYLE_ATTRIBUTE: &str = "data-battlefield-style";
const PREVIEW_HASH: &str = "#battle-preview";

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

struct PreviewRequest {
    scene_id: String,
    procedural_splats: bool,
    reduced_motion: bool,
    /// A splat scene served from `/assets/splat/` to use instead of the painted ground.
    splat_file: Option<(String, battle::SplatFormat)>,
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
    let mut scene_id = battle::DEFAULT_SCENE_ID.to_owned();
    let mut request = PreviewRequest {
        scene_id: String::new(),
        procedural_splats: false,
        reduced_motion: battle::prefers_reduced_motion(),
        splat_file: None,
    };
    for part in rest.split('&') {
        match part.strip_prefix('=').unwrap_or(part) {
            "" => {}
            "splat" => request.procedural_splats = true,
            "reduced" => request.reduced_motion = true,
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
            id if battle::embedded_ids().any(|known| known == id) => scene_id = id.to_owned(),
            _ => {}
        }
    }
    request.scene_id = scene_id;
    Some(request)
}

/// Whether the page was opened as `#battle-preview` and should show only the battlefield.
pub(super) fn preview_requested() -> bool {
    preview_request().is_some()
}

fn hero(id: &str, name: &str, cell: Cell, figure: &str, portrait: &str) -> Token {
    Token {
        id: TokenId::new(id),
        side: Side::Hero,
        name: name.to_owned(),
        cell,
        height_m: 1.35,
        figure: Some(figure.to_owned()),
        portrait: Some(portrait.to_owned()),
        hit_points: Some(HitPoints {
            current: 12,
            maximum: 12,
        }),
        statuses: Vec::new(),
    }
}

fn preview_tokens(scene: &BattleScene) -> Result<Vec<Token>, JsValue> {
    let formation = battle::derive_formation(&scene.grid, 2, 1)
        .map_err(|_| JsValue::from_str("battlefield formation has no room"))?;
    Ok(vec![
        hero(
            "tobin",
            "Tobin",
            formation.heroes[0],
            "figure-hero-1",
            "portrait-hero-1",
        ),
        hero(
            "nimrael",
            "Nimrael",
            formation.heroes[1],
            "figure-hero-2",
            "portrait-hero-2",
        ),
        Token {
            id: TokenId::new("goblin"),
            side: Side::Enemy,
            name: "Goblin Warrior".to_owned(),
            cell: formation.enemies[0],
            height_m: 1.0,
            figure: Some("figure-ant-goblin-warrior".to_owned()),
            portrait: Some("portrait-ant-goblin-warrior".to_owned()),
            hit_points: Some(HitPoints {
                current: 12,
                maximum: 12,
            }),
            statuses: Vec::new(),
        },
    ])
}

/// The looping preview fight: a miss, hits both ways, a heal, then a finishing blow.
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
        (2, attack("nimrael", "goblin", hit(5, false), Some(7))),
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
        (3, attack("tobin", "goblin", hit(7, true), Some(0))),
    ]
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
        });
    }
    let tokens = match preview_tokens(&scene) {
        Ok(tokens) => tokens,
        Err(_) => return,
    };
    let stage = BattleStage::mount(
        &host,
        scene,
        StageOptions {
            reduced_motion: request.reduced_motion,
            procedural_splats: request.procedural_splats,
        },
    )
    .await;
    if let Availability::Unavailable(reason) = stage.availability() {
        show_unavailable(&host, reason.key());
        return;
    }
    PREVIEW.with(|slot| *slot.borrow_mut() = Some(stage));
    wasm_bindgen_futures::spawn_local(watch_availability(host.clone()));
    let beat_counter = StdCell::new(0u32);
    loop {
        let reset = PREVIEW.with(|slot| {
            slot.borrow().as_ref().and_then(|stage| {
                stage.with_stage(|stage| {
                    stage.set_tokens(tokens.clone());
                    stage.set_initiative(vec![
                        TokenId::new("tobin"),
                        TokenId::new("goblin"),
                        TokenId::new("nimrael"),
                    ]);
                    stage.set_round(1);
                    stage.establish();
                })
            })
        });
        if reset.is_none() {
            // The stage became unavailable (device lost) or was disposed: nothing left to drive.
            if let Some(Availability::Unavailable(reason)) =
                PREVIEW.with(|slot| slot.borrow().as_ref().map(BattleStage::availability))
            {
                show_unavailable(&host, reason.key());
            }
            return;
        }
        TimeoutFuture::new(3000).await;
        for (round, beat) in preview_script() {
            let queued = PREVIEW.with(|slot| {
                slot.borrow().as_ref().and_then(|stage| {
                    stage.with_stage(|stage| {
                        stage.set_round(round);
                        stage.stage(beat.clone()).is_ok()
                    })
                })
            });
            if queued != Some(true) {
                return;
            }
            beat_counter.set(beat_counter.get() + 1);
            // Best-effort test hook: lets a headless harness time its captures.
            let _ = host.set_attribute("data-battlefield-beat", &beat_counter.get().to_string());
            loop {
                TimeoutFuture::new(100).await;
                let busy = PREVIEW.with(|slot| {
                    slot.borrow()
                        .as_ref()
                        .and_then(|stage| stage.with_stage(|stage| stage.is_busy()))
                });
                match busy {
                    Some(true) => {}
                    Some(false) => break,
                    None => return,
                }
            }
            TimeoutFuture::new(900).await;
        }
        TimeoutFuture::new(2500).await;
    }
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
}

thread_local! {
    static LIVE: RefCell<Live> = RefCell::new(Live {
        phase: Phase::Idle,
        host: None,
        stage: None,
        feed: CombatFeed::new(FeedMode::HoldForCues),
        latest: None,
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

/// The battle scene for the display's scene art: `scene-<location>-arrival` -> `<location>`.
fn scene_id_for(scene_asset: &str) -> String {
    let location = stem_of(scene_asset)
        .and_then(|stem| stem.strip_prefix("scene-"))
        .map(|rest| {
            rest.strip_suffix("-arrival")
                .or_else(|| rest.strip_suffix("-interior"))
                .unwrap_or(rest)
        })
        .unwrap_or("");
    if battle::embedded_ids().any(|id| id == location) {
        location.to_owned()
    } else {
        battle::DEFAULT_SCENE_ID.to_owned()
    }
}

/// Copies the server's combat projection into plain data. No outcome is decided here.
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
            let hero = index < party;
            Member {
                id: id_of(index, member),
                name: member.name.clone(),
                side: if hero { Side::Hero } else { Side::Enemy },
                hit_points: member.hit_points,
                maximum_hit_points: member.maximum_hit_points,
                height_m: if hero { 1.35 } else { enemy_height_m(creature) },
                figure: Some(if hero {
                    format!("figure-hero-{}", (index % 2) + 1)
                } else {
                    format!("figure-{creature}")
                }),
                portrait: Some(if hero {
                    format!("portrait-hero-{}", (index % 2) + 1)
                } else {
                    format!("portrait-{creature}")
                }),
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
    Some(CombatSnapshot {
        members,
        active,
        round: combat.round,
        outcomes,
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
    let scene_id = scene_id_for(&view.scene.scene_asset);
    let phase_idle = LIVE.with(|live| {
        let mut live = live.borrow_mut();
        live.latest = Some(snapshot);
        matches!(live.phase, Phase::Idle)
    });
    if phase_idle {
        start_mount(client, &scene_id);
    } else {
        sync();
    }
}

fn start_mount(client: &Rc<RefCell<Client>>, scene_id: &str) {
    let (document, host) = {
        let state = client.borrow();
        (state.document.clone(), state.root.clone())
    };
    let Some(Ok(scene)) = battle::load_embedded(scene_id) else {
        LIVE.with(|live| live.borrow_mut().phase = Phase::Unavailable);
        return;
    };
    if install_style(&document).is_err() {
        LIVE.with(|live| live.borrow_mut().phase = Phase::Unavailable);
        return;
    }
    LIVE.with(|live| {
        let mut live = live.borrow_mut();
        live.phase = Phase::Mounting;
        live.host = Some(host.clone());
    });
    wasm_bindgen_futures::spawn_local(async move {
        let stage = BattleStage::mount(
            &host,
            scene,
            StageOptions {
                reduced_motion: battle::prefers_reduced_motion(),
                procedural_splats: false,
            },
        )
        .await;
        let available = matches!(stage.availability(), Availability::Available);
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
        if matches!(stage.availability(), Availability::Unavailable(_)) {
            live.phase = Phase::Unavailable;
            live.stage = None;
            return;
        }
        let feed = &mut live.feed;
        let fits = stage
            .with_stage(|stage| feed.apply(stage, snapshot))
            .flatten();
        if fits.is_none() {
            // No room on the grid for this roster: leave the flat view.
            live.phase = Phase::Unavailable;
            live.stage = None;
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
                    if let Some(card) = roll.card.clone() {
                        stage.show_roll_card(card);
                    }
                    // Dropped, not fatal: the queue is bounded and the view sync still lands.
                    let _ = stage.stage(Beat::Approach { attacker, target });
                }
                "impact" => {
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

fn reset_fight(live: &mut Live) {
    live.phase = Phase::Idle;
    live.latest = None;
    live.host = None;
    live.feed = CombatFeed::new(FeedMode::HoldForCues);
}

/// The fight ended or the view scope changed. A stage that is still playing (the finishing blow)
/// is allowed to finish, then fades out; one that never mounted or is unavailable is just reset.
pub(super) fn release() {
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
}
