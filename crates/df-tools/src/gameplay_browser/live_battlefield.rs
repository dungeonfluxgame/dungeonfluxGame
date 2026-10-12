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
}

fn preview_request() -> Option<PreviewRequest> {
    let hash = web_sys::window()?.location().hash().ok()?;
    let rest = hash.strip_prefix(PREVIEW_HASH)?;
    let mut scene_id = battle::DEFAULT_SCENE_ID.to_owned();
    let mut request = PreviewRequest {
        scene_id: String::new(),
        procedural_splats: false,
        reduced_motion: battle::prefers_reduced_motion(),
    };
    for part in rest.split('&') {
        match part.strip_prefix('=').unwrap_or(part) {
            "" => {}
            "splat" => request.procedural_splats = true,
            "reduced" => request.reduced_motion = true,
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
        height_m: 1.8,
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
            height_m: 1.25,
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
    let scene = match battle::load_embedded(&request.scene_id) {
        Some(Ok(scene)) => scene,
        _ => return,
    };
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

fn show_unavailable(host: &Element, reason: &str) {
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
