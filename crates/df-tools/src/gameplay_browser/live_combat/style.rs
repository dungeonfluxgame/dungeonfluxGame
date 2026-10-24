//! Scoped presentation for the live combat, aftermath, rest and character-sheet screens.
//!
//! The df-ui themes style the surfaces themselves. This stylesheet fits them to the live
//! roles (a phone-first single column for players, a 16:9 ten-foot layout inside the shared
//! display's `.tv-stage`) and attaches the generated art named in [`ART_SLOTS`]. Every art
//! slot sits on top of a readable CSS fallback, so a missing file leaves plain gold and ink.
//! `@S` stands for the `.live-screen[data-role]` scope, which outranks the df-ui theme rules
//! that are installed later in the document.

use super::super::live_tv::GENERATED;
use df_render::art_contract;

/// The single place that turns a generated-art stem into its served URL, by the art contract:
/// chrome, or the reviewed photograph the stem names or an identity key answers. A painted stem,
/// a role token or a key with no photograph has no URL and its slot is `none` (the readable CSS
/// fallback under it stands).
pub(super) fn generated_asset_url(stem: &str) -> Option<String> {
    art_contract::fetch_target(stem).map(|target| format!("{GENERATED}/{target}"))
}

/// The picture behind a fight, set on the screen's host by `live_combat::present` as
/// `--asset-battle-backdrop`. Until one is ready the variable is unset and every rule that reads it
/// falls back to `none`: the readable CSS base stands, never a painting of another place.
pub(super) const BATTLE_BACKDROP: &str = "var(--asset-battle-backdrop,none)";

/// CSS custom property -> generated art stem, with the shape each slot expects.
pub(super) const ART_SLOTS: [(&str, &str); 14] = [
    ("--asset-d20", "d20"),                       // about 11:12, blank top face
    ("--asset-ribbon-success", "ribbon-success"), // about 8:1
    ("--asset-ribbon-failure", "ribbon-failure"), // about 8:1
    ("--asset-frame-panel", "panel-frame"),       // 3:2, thin gold nine-slice border
    ("--asset-frame-button", "button-frame"),     // about 5.4:1, stretched
    ("--asset-divider", "divider"),               // about 13:1
    ("--asset-icon-attack", "icon-attack"),       // 1:1
    ("--asset-icon-second-wind", "icon-second-wind"),
    ("--asset-icon-end-turn", "icon-end-turn"),
    ("--asset-icon-short-rest", "icon-short-rest"),
    ("--asset-icon-journey", "icon-journey"),
    ("--asset-icon-character", "icon-character"),
    ("--asset-portrait-bandit", "portrait-bandit"),
    ("--asset-portrait-bandit-full", "portrait-bandit-full"),
];

pub(super) fn stylesheet() -> String {
    let mut variables = String::new();
    for (property, stem) in ART_SLOTS {
        match generated_asset_url(stem) {
            Some(url) => variables.push_str(&format!("{property}:url('{url}');")),
            None => variables.push_str(&format!("{property}:none;")),
        }
    }
    let mut portraits = String::new();
    for stem in [
        "ant-bandit",
        "ant-cultist",
        "ant-guard",
        "ant-goblin-warrior",
        "ant-kobold-warrior",
        "ant-skeleton",
        "ant-zombie",
        "ant-wolf",
        "ant-giant-rat",
        "ant-giant-weasel",
        "ant-mastiff",
        "ant-scout",
    ] {
        // A creature with no reviewed photograph has no rule: its portrait circle keeps the
        // neutral fill until the server's live canonical photograph replaces it.
        let Some(url) = generated_asset_url(&format!("portrait-{stem}")) else {
            continue;
        };
        let crop = "background-size:240% auto;background-position:50% 0;";
        portraits.push_str(&format!("@S .combat-actor[data-portrait={stem}] .combat-initiative-value::before,@S .aftermath-member[data-portrait={stem}] .aftermath-portrait::before,.live-portrait[data-portrait={stem}]::before{{background-image:url('{url}');{crop}}}@S[data-role=display] .live-keyart[data-portrait={stem}]{{background-image:url('{url}')}}"));
    }
    format!(":root{{{variables}}}\n{COMMON}\n{PLAYER}\n{DISPLAY}\n{REST}\n{SHEET}\n{portraits}")
        .replace("@S", ".live-screen[data-role]")
        .replace("@BATTLE_BACKDROP", BATTLE_BACKDROP)
}

const COMMON: &str = r#"
.live-screen .combat-actor[data-personal-art] .combat-initiative-value::before,.live-portrait[data-personal-art]::before,.live-screen .aftermath-member[data-personal-art] .aftermath-portrait::before{background-image:var(--personal-portrait);background-size:240% auto;background-position:50% 0}
.live-screen[data-role=display] .live-keyart[data-personal-art]{background-image:var(--personal-portrait)}
.live-screen{position:relative;isolation:isolate;color:#f4eddf;font-family:Georgia,'Times New Roman',serif}
.live-screen *{box-sizing:border-box;min-width:0}
@S .combat-topbar,@S .combat-connection,@S .combat-story .combat-overline,@S .combat-initiative>h2{display:none}
@S .combat-initiative,@S .combat-roll-panel,@S .combat-action-rail,@S .camp-member,@S .aftermath-panel,.live-hero-head,.live-sheet-inline{background:linear-gradient(135deg,#0f1a22ed,#070d13f2);border:12px solid transparent;border-image:var(--asset-frame-panel) 3.2% / 12px stretch;border-radius:0;box-shadow:0 12px 36px #0009}
@S .combat-offer[data-kind=action] button{display:flex;align-items:center;justify-content:flex-start;gap:.7em;min-height:64px;padding:12px 28px 12px 66px;border:0;border-radius:0;background:var(--icon,none) left 28px center/1.7em no-repeat,var(--asset-frame-button) center/100% 100% no-repeat,linear-gradient(#1c1710b0,#0d0b08b0) center/calc(100% - 26px) calc(100% - 16px) no-repeat;color:#fff1d2;font:600 18px/1.3 Georgia,serif;letter-spacing:.02em;text-align:left;box-shadow:none}
@S button[data-combat-offer=strike]{--icon:var(--asset-icon-attack)}
@S button[data-combat-offer=second-wind]{--icon:var(--asset-icon-second-wind)}
@S button[data-combat-offer=end-turn]{--icon:var(--asset-icon-end-turn)}
@S .camp-actions button:first-child{--icon:var(--asset-icon-short-rest)}
@S .camp-actions button:last-child{--icon:var(--asset-icon-journey)}
@S .camp-actions button{display:flex;align-items:center;justify-content:flex-start;gap:.7em;min-height:64px;padding:12px 28px 12px 66px;max-width:none;border:0;border-radius:0;background:var(--icon,none) left 28px center/1.7em no-repeat,var(--asset-frame-button) center/100% 100% no-repeat,linear-gradient(#1c1710b0,#0d0b08b0) center/calc(100% - 26px) calc(100% - 16px) no-repeat;color:#fff1d2;font:600 18px/1.3 Georgia,serif;text-align:left;box-shadow:none}
@S .df-combat .combat-offer[data-kind=action] button:enabled,@S .df-campfire .camp-actions button:enabled,@S .df-campfire .camp-actions button:last-child:enabled,@S .df-combat .combat-offer[data-kind=action] button[aria-busy=true]{background:var(--icon,none) left 28px center/1.7em no-repeat,var(--asset-frame-button) center/100% 100% no-repeat,linear-gradient(#1c1710b0,#0d0b08b0) center/calc(100% - 26px) calc(100% - 16px) no-repeat;color:#fff1d2;border:0;box-shadow:none;font-weight:600}
@S .df-combat .combat-offer[data-kind=action] button:hover:enabled,@S .df-campfire .camp-actions button:hover:enabled{filter:brightness(1.3)}
@S .df-combat .combat-offer[data-kind=action] button:disabled,@S .df-campfire .camp-actions button:disabled{opacity:.5;background:var(--icon,none) left 28px center/1.7em no-repeat,var(--asset-frame-button) center/100% 100% no-repeat,#0d0b08;color:#cfc6b0;border:0}
@S .df-aftermath .aftermath-story{display:block;padding:0;border:0;background:none}
@S .combat-offer[data-kind=choice] button{display:flex;justify-content:space-between;align-items:center;gap:14px;min-height:52px;padding:8px 16px;border:1px solid #b5946066;border-radius:8px;background:#0f1519e0;color:#efe6d3;font:600 16px/1.3 system-ui,sans-serif;text-align:left}
@S .combat-offer[data-kind=choice] button::after{content:'';flex:none;min-width:0;width:54px;height:30px;padding:0;border:1px solid #8d8b80;border-radius:15px;background:radial-gradient(circle at 15px 50%,#f6efe0 11px,transparent 12px),#3a3f45;transition:background-color .2s}
@S .combat-offer[data-kind=choice][data-selected=true] button{border-color:#d9b77a;background:#1c1a10e6}
@S .combat-offer[data-kind=choice][data-selected=true] button::after{content:'';background:radial-gradient(circle at calc(100% - 15px) 50%,#fff7e6 11px,transparent 12px),linear-gradient(#3c9a5c,#237043);border-color:#8fe0a8}
@S .combat-actor .combat-initiative-value{position:relative;overflow:hidden;isolation:isolate;border-radius:50%;background:#162230}
@S .combat-actor .combat-initiative-value::before{content:'';position:absolute;inset:0;z-index:2;border-radius:50%;background:center/cover no-repeat}
@S .combat-actor[data-portrait=hero-1] .combat-initiative-value::before,.live-portrait[data-portrait=hero-1]::before{background-image:var(--asset-portrait-hero-1)}
@S .combat-actor[data-portrait=hero-2] .combat-initiative-value::before,.live-portrait[data-portrait=hero-2]::before{background-image:var(--asset-portrait-hero-2)}
@S .combat-actor[data-portrait=bandit] .combat-initiative-value::before{background-image:var(--asset-portrait-bandit)}
@S .aftermath-portrait::before{content:'';position:absolute;inset:0;z-index:2;border-radius:50%;background:center/cover no-repeat}
@S .aftermath-member[data-portrait=hero-1] .aftermath-portrait::before{background-image:var(--asset-portrait-hero-1)}
@S .aftermath-member[data-portrait=hero-2] .aftermath-portrait::before{background-image:var(--asset-portrait-hero-2)}
@S .live-keyart{display:none}
@S .live-initiative{position:absolute;left:0;right:0;top:0;z-index:5;padding:14px 16px;text-align:center;font:700 clamp(22px,7vw,34px)/1.2 Georgia,serif;color:#f4d98d;background:linear-gradient(#0a1018f5,#0a1018d6);border-bottom:2px solid #e0b877;pointer-events:none;animation:live-initiative 7s ease forwards}
@S[data-role=display] .live-initiative{display:none}
@keyframes live-initiative{0%{transform:translateY(-100%);opacity:0}8%{transform:none;opacity:1}88%{transform:none;opacity:1}100%{transform:none;opacity:0;visibility:hidden}}
.live-rules-badge{display:inline-block;padding:4px 12px;border:1px solid #d9b77a;border-radius:6px;font:700 12px/1.4 system-ui,sans-serif;letter-spacing:.18em;text-transform:uppercase;color:#e8cb94;background:#0a1018d9}
@S .combat-actor{align-items:center}
@S .combat-actor[data-active=true] .combat-initiative-value{border:3px solid #e0b877;box-shadow:0 0 22px #e0b87780}
@S .combat-roll-panel{padding:16px}
@S .combat-roll{display:grid;grid-template-columns:auto minmax(0,1fr);grid-template-areas:"die label" "die expl" "die verdict" "die source";align-items:center;column-gap:16px;margin:0;padding:0 0 12px;border:0}
@S .combat-roll h3{grid-area:label;margin:0;font:600 16px/1.4 system-ui,sans-serif;color:#f0dfbd;letter-spacing:.02em}
@S .combat-roll h3,@S .combat-roll-explanation,@S .combat-roll-source{font-family:var(--serif,Georgia,serif);font-variant-numeric:lining-nums}
@S .combat-roll-value{grid-area:die;margin:0;font-family:Georgia,serif}
@S .combat-roll-explanation{grid-area:expl;margin:2px 0;color:#fff1d2}
@S .combat-roll-source{grid-area:source;margin:2px 0 0;color:#d9cfb8}
@S .combat-roll-verdict{grid-area:verdict;justify-self:start;position:relative;isolation:isolate;margin:6px 0;padding:0;text-align:center;font:700 16px/1 system-ui,sans-serif;letter-spacing:.22em;text-transform:uppercase;color:#f8f1dd;background:none;clip-path:none;aspect-ratio:7.9;width:min(230px,100%);display:grid;place-items:center;text-shadow:0 1px 3px #000}
@S .combat-roll-verdict::before{content:'';position:absolute;inset:0;z-index:-1;background:linear-gradient(#6b5a35,#4a3d22);clip-path:polygon(0 50%,3% 0,97% 0,100% 50%,97% 100%,3% 100%)}
@S .combat-roll-verdict[data-tone=success]::before{background:center/100% 100% no-repeat var(--asset-ribbon-success),linear-gradient(#2f8150,#1d5a37)}
@S .combat-roll-verdict[data-tone=failure]::before{background:center/100% 100% no-repeat var(--asset-ribbon-failure),linear-gradient(#963a2e,#65261f)}
@S .combat-roll-verdict::after{content:none}
@S .combat-roll:first-child{--die:150px;grid-template-columns:var(--die) minmax(0,1fr);padding-bottom:14px}
@S .combat-roll:first-child .combat-roll-value{width:var(--die);aspect-ratio:988/1088;height:auto;display:grid;place-items:center;padding-top:10%;clip-path:none;border:0;box-shadow:none;background:var(--asset-d20) center/contain no-repeat,radial-gradient(circle,#1b2430 40%,transparent 70%);font-size:calc(var(--die)*.25);line-height:1;color:#ffd98a;text-shadow:0 2px 6px #000,0 0 14px #ffbd4a80}
@S .combat-roll:not(:first-child){grid-template-columns:auto auto minmax(0,1fr) auto;grid-template-areas:"die label expl verdict";column-gap:10px;padding:6px 0;margin:0;border-top:1px solid #d9b77a33}
@S .combat-roll:not(:first-child) .combat-roll-value{width:2em;height:2em;display:grid;place-items:center;aspect-ratio:auto;clip-path:none;border:1px solid #d9b77a99;border-radius:50%;background:#14110b;font-size:16px;color:#ffd98a}
@S .combat-roll:not(:first-child) .combat-roll-source{display:none}
@S .combat-roll:not(:first-child) .combat-roll-verdict{width:auto;aspect-ratio:auto;margin:0;font-size:inherit;letter-spacing:.1em;justify-self:end}
@S .combat-roll:not(:first-child) .combat-roll-verdict::before{content:none}
@S .combat-roll:not(:first-child) .combat-roll-verdict[data-tone=success]{color:#8fe0a8}
@S .combat-roll:not(:first-child) .combat-roll-verdict[data-tone=failure]{color:#ff9d8a}
.live-screen .live-divider{position:relative;height:1px;margin:22px 0;border:0;background:linear-gradient(90deg,transparent,#d9b77a99,transparent)}
.live-screen .live-divider::after{content:'';position:absolute;left:0;right:0;top:50%;height:28px;transform:translateY(-50%);background:var(--asset-divider) center/contain no-repeat}
@S .live-extras .dialogue-recovery,@S .live-extras .recovery-needed{border:1px solid #c8a76b66;background:linear-gradient(135deg,#0f1a22f0,#070d13f2);color:#f4eddf}
@S .live-extras .dialogue-recovery h3{color:#f2dfbb}
@S .live-extras .dialogue-controls button,@S .live-extras .tools button{min-height:48px;border:1px solid #c19d6066;background:#1d242b;color:#f4e5ca;font-size:15px}
@S .live-extras .dialogue-controls button:first-child{background:linear-gradient(135deg,#f0d5a4,#c6a05e);color:#211b11;border-color:#f0d195;font-weight:650}
@S .live-extras .feedback{font:16px/1.5 system-ui,sans-serif;color:#f0d19e;margin:12px 0 0}
@S .live-extras .feedback:empty{display:none}
@S[data-role=player] .tactical-controls .resource-strip{display:flex;flex-wrap:wrap;gap:6px;margin:0 0 8px;padding:8px;border:1px solid #d9b77a66;border-radius:8px;background:#0a1018e8;color:inherit;font:600 13px/1.35 system-ui,sans-serif}
@S[data-role=player] .tactical-controls .resource-chip{display:inline-flex;padding:5px 8px;border:1px solid #d9b77a66;border-radius:6px;background:#17222de8;color:inherit}
@S[data-role=player] .tactical-controls{display:grid;grid-template-columns:repeat(6,minmax(0,1fr));gap:8px;padding:10px 12px 12px;border:1px solid #d9b77a40;border-radius:12px;background:#0a1018e8}
@S[data-role=player] .tactical-controls>*{grid-column:1/-1}
@S[data-role=player] .tactical-controls .resource-strip{margin:0;order:-1}
@S[data-role=player] .tactical-controls>.offered-action{display:contents}
@S[data-role=player] .tactical-controls>.offered-action>*{grid-column:1/-1}
@S[data-role=player] .tactical-controls .move-map{display:grid;grid-template-columns:repeat(var(--columns),minmax(0,44px));justify-content:center;gap:2px;padding:6px;border:1px solid #d9b77a40;border-radius:8px;background:#05080ad9}
@S[data-role=player] .tactical-controls .move-map .move-cell{display:grid;place-items:center;aspect-ratio:1;min-width:0;min-height:0;margin:0;padding:0;border:0;border-radius:4px;background:#ffffff0a;color:#f4eddf;font:700 11px/1 system-ui,sans-serif}
@S[data-role=player] .tactical-controls .move-map button.move-cell{background:#d9b77a38;box-shadow:inset 0 0 0 1px #d9b77a99;cursor:pointer}
@S[data-role=player] .tactical-controls .move-map button.move-cell[aria-pressed=true]{background:#f0cf8e;box-shadow:inset 0 0 0 2px #fffaf0,0 0 0 2px #f0cf8e}
@S[data-role=player] .tactical-controls .move-map .move-cell[data-side="1"]{background:#2b6a8c}
@S[data-role=player] .tactical-controls .move-map .move-cell[data-side="2"]{background:#96392f}
@S[data-role=player] .tactical-controls .move-map .move-cell[data-own]{box-shadow:inset 0 0 0 2px #fffaf0}
@S[data-role=player] .tactical-controls .move-map button.move-cell[data-side="2"][aria-pressed=true]{background:#c8452f;color:#fffaf0}
@S[data-role=player] .tactical-controls .move-map .move-cell[data-down]{opacity:.4}
@S[data-role=player] .tactical-controls .move-chosen{margin:0;font:600 13px/1.4 system-ui,sans-serif;color:#cdb98c}
@S[data-role=player] .tactical-controls>.offered-action[data-on-map]>label{position:absolute;width:1px;height:1px;margin:-1px;overflow:hidden;clip-path:inset(50%);white-space:nowrap}
@S[data-role=player] .tactical-controls .attack-option{display:grid;grid-template-columns:minmax(0,1fr);gap:4px;margin:4px 0 0;font:600 12px/1.3 system-ui,sans-serif;letter-spacing:.05em;text-transform:uppercase;color:#cdb98c}
@S[data-role=player] .tactical-controls .attack-option select{width:100%;min-width:0;min-height:44px;padding:9px 10px;font:500 15px/1.3 system-ui,sans-serif;letter-spacing:0;text-transform:none}
@S[data-role=player] .tactical-controls>.offered-action>button.offered{margin:0;text-align:center}
@S[data-role=player] .tactical-controls>.offered-action>button:is([data-action-kind="13"],[data-action-kind="14"],[data-action-kind="15"]){grid-column:span 2;padding-left:4px;padding-right:4px}
@S[data-role=player] .tactical-controls>.offered-action>button:is([data-action-kind="9"],[data-action-kind="10"]){grid-column:span 3}
@S[data-role=player] .tactical-controls:not(:has(button[data-action-kind="9"])) button[data-action-kind="10"]{grid-column:1/-1}
@S .rules-disclosure{display:block;margin-top:8px;padding:7px 10px;border:1px solid #d9b77a66;border-radius:8px;background:#0a1018d9;color:inherit;font:13px/1.45 system-ui,sans-serif}
@S .rules-disclosure summary{cursor:pointer;color:inherit;font-weight:650}
@S .rules-disclosure .rules-note{display:block;margin-top:6px;color:inherit}
"#;

/// Phone-first single column, valid at any container width (390px phones are the target).
const PLAYER: &str = r#"
@S[data-role=player]{max-width:560px;margin:0 auto;--frame-w:12px}
@S[data-role=player] .df-combat{display:flex;flex-direction:column;min-height:0;font-size:16px}
@S[data-role=player] .combat-art{height:460px;object-position:46% center}
@S[data-role=player] .df-combat:before{background:linear-gradient(180deg,#07101866,transparent 12%,#0a1118b3 300px,#0b1218 470px)}
@S[data-role=player] .combat-stage{display:contents}
@S[data-role=player] .combat-story{order:1;padding:56px 16px 10px}
@S[data-role=player] h1{font-size:30px;letter-spacing:-.5px;margin:0 0 10px}
@S[data-role=player] .combat-turn-badge{display:block;margin:0 0 12px;padding:10px 14px;font:700 15px/1.4 system-ui,sans-serif;letter-spacing:.16em;text-transform:uppercase;color:#ffe0a3;border:0;border-left:4px solid #e0b877;border-radius:0;background:linear-gradient(90deg,#1c1508ee,#1c150833)}
@S[data-role=player] .combat-narration{font-size:16px;line-height:1.6}
@S[data-role=player] .combat-lower{order:2;display:flex;flex-direction:column;gap:16px;width:100%;margin:0;padding:0 16px 16px}
@S[data-role=player] .combat-action-rail{padding:16px}
@S[data-role=player] .df-combat:has(>.tactical-controls>.offered-action) .combat-action-rail{display:none}
@S[data-role=player] .combat-action-rail h2{font-size:21px;margin:0 0 12px}
@S[data-role=player] .combat-offers{grid-template-columns:1fr;gap:12px}
@S[data-role=player] .combat-offer[data-kind=action] button{font-size:18px}
@S[data-role=player] .combat-offer[data-kind=choice]{margin-left:14px}
@S[data-role=player] .combat-offer p{font-size:14px;line-height:1.5;margin:6px 2px 0}
@S[data-role=player] .combat-muted{font-size:14px}
@S[data-role=player] .combat-roll-panel:not(:has(.combat-roll)){display:none}
@S[data-role=player] .combat-roll-panel h2{font-size:21px;margin:0 0 12px}
@S[data-role=player] .combat-roll:first-child{--die:128px;column-gap:14px}
@S[data-role=player] .combat-roll:first-child .combat-roll-explanation{font-size:19px;line-height:1.35}
@S[data-role=player] .combat-roll-explanation,@S[data-role=player] .combat-roll-source{font-size:14px;line-height:1.45}
@S[data-role=player] .combat-roll:not(:first-child){grid-template-columns:auto minmax(0,1fr) auto;grid-template-areas:"die label verdict" "die expl verdict"}
@S[data-role=player] .combat-roll:not(:first-child) .combat-roll-explanation{font-size:14px}
@S[data-role=player] .combat-roll:nth-child(n+4){display:none}
@S[data-role=player] .combat-initiative{order:3;width:auto;margin:0 16px 16px;padding:12px}
@S[data-role=player] .combat-actors{display:grid;gap:10px}
@S[data-role=player] .combat-actor{padding:8px 4px;gap:12px;border-top:1px solid #d9b77a29}
@S[data-role=player] .combat-actor:first-child{border-top:0}
@S[data-role=player] .combat-actor .combat-initiative-value{width:56px;height:56px;min-width:56px;max-width:none;padding:0;font-size:24px}
@S[data-role=player] .combat-actor h3{font-size:19px}
@S[data-role=player] .live-extras{padding:0 16px calc(36px + env(safe-area-inset-bottom))}
@S[data-role=player] .live-rules-badge{margin:8px 16px 0}
@S[data-role=player] .df-combat button{min-height:48px}
@S[data-role=player] .tactical-controls{order:2;align-self:stretch;margin:0 16px 16px;background:linear-gradient(145deg,#101922f5,#080d13f7);box-shadow:0 12px 28px #0008}
@S[data-role=player] .combat-lower{order:3}
@S[data-role=player] .combat-initiative{order:4}
@S[data-role=player] .combat-connection{order:5}
@S[data-role=player] .tactical-controls .attack-option{width:100%;min-width:0;padding:6px 8px;border:1px solid #d9b77a33;border-radius:6px;background:#080d13e8}
@S[data-role=player] .tactical-controls .attack-option select{display:block;width:100%;min-width:0;max-width:100%;overflow:hidden;text-overflow:ellipsis}
@S[data-role=player] .tactical-controls button[data-action-kind="10"]{grid-column:1/-1;border-color:#f0d195;background:linear-gradient(135deg,#f0d5a4,#c6a05e);color:#211b11;font-weight:700}
@S[data-role=player] .combat-hp-text{z-index:1;font-size:12px;text-shadow:0 1px 3px #000,0 0 3px #000}
@S[data-role=player] .df-combat[data-own-turn=true] .combat-story{padding:8px 16px 4px}
@S[data-role=player] .df-combat[data-own-turn=true] .combat-story h1,@S[data-role=player] .df-combat[data-own-turn=true] .combat-narration{display:none}
@S[data-role=player] .df-combat[data-own-turn=true] .combat-turn-badge{margin:0;padding:8px 10px;letter-spacing:.08em}
@S[data-role=player] .df-combat[data-own-turn=true] .tactical-controls button[data-action-kind="8"]{grid-column:1/-1;min-height:56px;border-color:#f0d195;background:linear-gradient(135deg,#f0d5a4,#c6a05e);color:#211b11;font-size:19px;font-weight:700}
@S[data-role=player] .df-combat[data-own-turn=true] .tactical-controls button[data-action-kind="10"]{position:sticky;bottom:calc(8px + env(safe-area-inset-bottom));z-index:6;border-color:#d9b77a;background:#17222d;color:#fff1d2}
@S[data-role=player] .tactical-controls[data-reaction=decision],@S[data-role=player] .tactical-controls[data-reaction=waiting]{order:0;width:auto;margin:8px 12px 12px;padding:12px;border:2px solid #f0d195;border-radius:12px;background:linear-gradient(145deg,#21180cfa,#0b1118fa);box-shadow:0 10px 28px #000b}
@S[data-role=player] .reaction-prompt,@S[data-role=player] .reaction-waiting{grid-column:1/-1;margin:0;padding:8px 10px;border-radius:8px;background:#30210e;color:#fff0d1}
@S[data-role=player] .tactical-controls[data-reaction=decision] .reaction-prompt,@S[data-role=player] .tactical-controls[data-reaction=waiting] .reaction-waiting{order:-2}
@S[data-role=player] .reaction-prompt h2{margin:0 0 4px;font:700 16px/1.35 system-ui,sans-serif;letter-spacing:.12em;text-transform:uppercase;color:#f0cf8e}
@S[data-role=player] .reaction-question{margin:0;font:700 20px/1.3 system-ui,sans-serif;color:#fff7e6}
@S[data-role=player] .reaction-countdown{grid-column:1/-1;margin:4px 0 0;font:700 16px/1.3 system-ui,sans-serif;color:#f0cf8e}
@S[data-role=player] .reaction-waiting{font:700 18px/1.35 system-ui,sans-serif;text-align:center}
@S[data-role=player] .tactical-controls[data-reaction=decision] button[data-action-kind="16"]{grid-column:span 3;border-color:#f0d195;background:linear-gradient(135deg,#f0d5a4,#c6a05e);color:#211b11;font-weight:700}
@S[data-role=player] .tactical-controls[data-reaction=decision] button[data-action-kind="17"]{grid-column:span 3;border-color:#d9b77a;background:#17222d;color:#fff1d2}
"#;

/// A fixed 16:9, ten-foot interface in the display's `.tv-stage`. `--u` is 1% of the width
/// on a 16:9 canvas (19.2px at 1920x1080), so every size scales with the screen.
const DISPLAY: &str = r#"
body:has(.tv-stage[data-tv-mode=combat])>header,body:has(.tv-stage[data-tv-mode=combat])>footer{display:none}
body:has(.tv-stage[data-tv-mode=combat])>main{min-height:0}
.tv-stage[data-tv-mode=combat]>.tv-art,.tv-stage[data-tv-mode=combat]>.tv-scrim,.tv-stage[data-tv-mode=combat]>.tv-safe,.tv-stage[data-tv-mode=combat]>.tv-legal{display:none}
.tv-stage:not([data-tv-mode=combat])>.live-screen{display:none}
@S[data-role=display]{position:absolute;inset:0;z-index:1;overflow:hidden;background:#05080a;--u:min(1vw,1.7778vh);--frame-w:calc(var(--u)*.7)}
@S[data-role=display] .live-extras{position:absolute;z-index:5;top:1.2vh;right:2vw;max-width:34vw;font-size:calc(var(--u)*1.25)}
@S[data-role=display] .df-combat{display:grid;width:100%;height:100%;min-height:0;overflow:hidden;font-size:calc(var(--u)*1.3);grid-template-columns:minmax(0,.8fr) minmax(0,1.2fr);grid-template-rows:auto auto auto minmax(0,1fr) auto;grid-template-areas:"banner banner" "title roll" "narr roll" "key roll" "cast cast";gap:calc(var(--u)*1.2) calc(var(--u)*2.4);padding:4.5vh 4.5vw}
@S[data-role=display] .df-combat:before{background:linear-gradient(180deg,#05080ae0 0,#05080a40 20%,#05080a33 40%,#05080aeb 78%,#05080a),linear-gradient(90deg,#05080acc,transparent 55%)}
@S[data-role=display] .combat-stage,@S[data-role=display] .combat-lower,@S[data-role=display] .combat-story{display:contents}
@S[data-role=display] .combat-action-rail{display:none}
@S[data-role=display] .combat-turn-badge{grid-area:banner;justify-self:center;display:block;margin:0;padding:.1em 1.6em;font:600 calc(var(--u)*3.3)/1.2 Georgia,serif;letter-spacing:.14em;text-transform:uppercase;color:#f0cf8e;text-align:center;border:0;border-top:1px solid #d9b77a99;border-bottom:1px solid #d9b77a99;border-radius:0;background:linear-gradient(90deg,transparent,#120d05d9 18%,#120d05d9 82%,transparent);text-shadow:0 3px 18px #000}
@S[data-role=display] .df-combat h1{grid-area:title;align-self:end;margin:0 0 calc(var(--u)*.6);max-width:none;font-size:calc(var(--u)*2.5);letter-spacing:-.01em;line-height:1.1}
@S[data-role=display] .combat-narration{grid-area:narr;align-self:start;max-width:none;font-size:calc(var(--u)*1.4);line-height:1.3;overflow:hidden;display:-webkit-box;-webkit-line-clamp:8;-webkit-box-orient:vertical}
@S[data-role=display] .combat-roll-panel{grid-area:roll;min-height:0;overflow:hidden;padding:calc(var(--u)*1) calc(var(--u)*1.6);align-self:stretch;display:flex;flex-direction:column;justify-content:center}
@S[data-role=display] .combat-roll-panel h2{margin:0 0 calc(var(--u)*.4);font-size:calc(var(--u)*1.3);letter-spacing:.2em;text-transform:uppercase;color:#e0b877}
@S[data-role=display] .combat-roll-panel .combat-muted{font-size:calc(var(--u)*1.3)}
@S[data-role=display] .combat-rolls{min-height:0}
@S[data-role=display] .live-keyart{display:block;grid-area:key;justify-self:start;align-self:end;height:100%;max-width:100%;aspect-ratio:2/3;background:center/cover no-repeat;-webkit-mask-image:radial-gradient(ellipse 62% 56% at 50% 46%,#000 52%,transparent 100%);mask-image:radial-gradient(ellipse 62% 56% at 50% 46%,#000 52%,transparent 100%)}
@S[data-role=display] .live-keyart[data-portrait=hero-1]{background-image:var(--asset-portrait-hero-1-full)}
@S[data-role=display] .live-keyart[data-portrait=hero-2]{background-image:var(--asset-portrait-hero-2-full)}
@S[data-role=display] .live-keyart[data-portrait=bandit]{background-image:var(--asset-portrait-bandit-full)}
@S[data-role=display] .live-keyart[hidden]{display:none}
@S[data-role=display] .combat-roll-panel:not(:has(.combat-roll)){align-items:center;text-align:center}
@S[data-role=display] .combat-roll-panel:not(:has(.combat-roll))::before{content:'';width:calc(var(--u)*11);aspect-ratio:988/1088;background:var(--asset-d20) center/contain no-repeat}
@S[data-role=display] .combat-roll-panel:not(:has(.combat-roll)) h2{display:none}
@S[data-role=display] .combat-roll-panel:not(:has(.combat-roll)) .combat-muted{margin-top:calc(var(--u)*1);font:calc(var(--u)*2.4)/1.3 var(--serif,Georgia,serif);color:#f0dfbd}
@S[data-role=display] .live-rules-badge{position:absolute;z-index:6;top:calc(var(--u)*1.2);right:4.5vw;font-size:calc(var(--u)*1.3);padding:calc(var(--u)*.3) calc(var(--u)*1)}@S[data-role=display] .combat-roll:first-child{--die:calc(var(--u)*18);column-gap:calc(var(--u)*2);padding-bottom:calc(var(--u)*.8)}
@S[data-role=display] .combat-roll:first-child h3{font-size:calc(var(--u)*1.9)}
@S[data-role=display] .combat-roll:first-child .combat-roll-explanation{font-size:calc(var(--u)*2.3);margin:calc(var(--u)*.3) 0}
@S[data-role=display] .combat-roll:first-child .combat-roll-verdict{width:calc(var(--u)*22);font-size:calc(var(--u)*1.5);margin:calc(var(--u)*.4) 0}
@S[data-role=display] .combat-roll:first-child .combat-roll-source{font-size:calc(var(--u)*1.35);line-height:1.35}
@S[data-role=display] .combat-roll:not(:first-child){grid-template-columns:calc(var(--u)*4.6) auto minmax(0,1fr) auto;padding:calc(var(--u)*.4) 0;column-gap:calc(var(--u)*1)}
@S[data-role=display] .combat-roll:not(:first-child) .combat-roll-value{width:calc(var(--u)*4.2);height:calc(var(--u)*4.2);flex:none;font-size:calc(var(--u)*1.9);overflow:visible}
@S[data-role=display] .combat-roll:not(:first-child) h3{font-size:calc(var(--u)*1.7)}
@S[data-role=display] .combat-roll:not(:first-child) .combat-roll-explanation{margin:0;font-size:calc(var(--u)*1.95);line-height:1.35;color:#e6dbc3}
@S[data-role=display] .combat-roll:not(:first-child) .combat-roll-source{font-size:calc(var(--u)*1.6);line-height:1.35}
@S[data-role=display] .combat-roll:not(:first-child) .combat-roll-verdict{font-size:calc(var(--u)*1.6)}
@S[data-role=display] .combat-roll:first-child .combat-roll-explanation{line-height:1.35}
@S[data-role=display] .combat-roll-panel>.tv-dice{overflow:visible;padding:calc(var(--u)*.45) calc(var(--u)*.6);margin:0 auto calc(var(--u)*.6)}
@S[data-role=display] .combat-roll-panel>.tv-dice .tv-die{overflow:visible;flex:none}
@S[data-role=display] .combat-roll:nth-child(n+4){display:none}
@S[data-role=display] .combat-initiative{grid-area:cast;padding:calc(var(--u)*.8) calc(var(--u)*1.2)}
@S[data-role=display] .combat-actors{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:calc(var(--u)*1.2)}
@S[data-role=display] .combat-actor{min-width:0;gap:calc(var(--u)*1);padding:calc(var(--u)*.5) calc(var(--u)*.7);border:1px solid #b69b6640;border-radius:calc(var(--u)*.4);background:#ffffff08}
@S[data-role=display] .combat-actor:first-child{padding-top:calc(var(--u)*.5);border-top:1px solid #b69b6640}
@S[data-role=display] .combat-actor[data-active=true]{border-color:#f0c985;background:linear-gradient(135deg,#d3a2594d,#d3a25914);box-shadow:0 0 calc(var(--u)*1.4) #e0b87755}
@S[data-role=display] .combat-actor .combat-initiative-value{flex:none;width:calc(var(--u)*5.4);height:calc(var(--u)*5.4);min-width:0;max-width:none;padding:0;font-size:calc(var(--u)*2.4)}
@S[data-role=display] .combat-actor h3{font-size:calc(var(--u)*1.9)}
@S[data-role=display] .combat-actor .combat-muted{margin:0;font-size:calc(var(--u)*1.25)}
@S[data-role=display] .combat-hp{height:calc(var(--u)*1.7);margin-top:calc(var(--u)*.3);border-radius:calc(var(--u)*.85)}
@S[data-role=display] .combat-hp-text{font-size:calc(var(--u)*1.25);line-height:calc(var(--u)*1.7)}
@S[data-role=display] .combat-resources{display:none}
@S[data-role=display] .live-extras .recovery-needed{background:#2a2417f2;margin:0 0 .5em}
@S[data-role=display] .live-extras .recovery-needed p,@S[data-role=display] .live-extras .feedback,@S[data-role=display] .live-extras .tools button{font-size:calc(var(--u)*1.25)}
@S[data-role=display][data-screen=aftermath] .df-aftermath{height:100%;min-height:0;overflow:hidden;display:grid;grid-template-rows:minmax(0,1fr) auto;padding:4.5vh 4.5vw;font-size:calc(var(--u)*1.3);text-align:center}
@S[data-role=display] .df-aftermath .aftermath-story{max-width:min(72vw,1500px);margin:0 auto;align-self:end;display:flex;flex-direction:column;align-items:center;padding:0;border:0;background:none}
@S[data-role=display] .df-aftermath .aftermath-chapter{display:grid;place-items:center;width:calc(var(--u)*30);aspect-ratio:7.9;margin:0 0 calc(var(--u)*.8);padding:0;font:700 calc(var(--u)*1.7)/1 system-ui,sans-serif;letter-spacing:.34em;color:#f8f1dd;text-shadow:0 2px 6px #000;background:var(--asset-ribbon-success) center/100% 100% no-repeat}
@S[data-role=display][data-outcome=defeat] .df-aftermath .aftermath-chapter{background:var(--asset-ribbon-failure) center/100% 100% no-repeat}
@S[data-role=display] .df-aftermath h1{font-size:calc(var(--u)*4.4);margin:0;text-shadow:0 4px 30px #000}
@S[data-role=display] .df-aftermath h1::after{content:'';display:block;width:calc(var(--u)*34);height:calc(var(--u)*1.6);margin:calc(var(--u)*.5) auto 0;background:var(--asset-divider) center/contain no-repeat}
@S[data-role=display] .df-aftermath .aftermath-narrative{margin:calc(var(--u)*.5) 0 0;font-size:calc(var(--u)*1.62);line-height:1.35;text-shadow:0 2px 14px #000}
@S[data-role=display] .df-aftermath .aftermath-lower{display:flex;flex-direction:column-reverse;gap:calc(var(--u)*.9);max-width:none;width:100%;margin:calc(var(--u)*1.2) 0 0}
@S[data-role=display] .df-aftermath .aftermath-lower>.aftermath-panel{padding:0;background:none;border:0;border-image:none;box-shadow:none}
@S[data-role=display] .df-aftermath .aftermath-panel h2,@S[data-role=display] .df-aftermath .aftermath-next-context{display:none}
@S[data-role=display] .df-aftermath .aftermath-empty{margin:0;font:calc(var(--u)*1.7)/1.3 var(--serif,Georgia,serif);color:#f0cf8e;text-shadow:0 2px 12px #000}
@S[data-role=display] .df-aftermath .aftermath-party{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:calc(var(--u)*1.4)}
@S[data-role=display] .df-aftermath .aftermath-member{gap:calc(var(--u)*1.2);padding:calc(var(--u)*.8) calc(var(--u)*1.1);text-align:left;border:1px solid #d9b77a8c;border-radius:calc(var(--u)*.5);background:linear-gradient(135deg,#0f1a22e6,#070d13f0)}
@S[data-role=display] .df-aftermath .aftermath-member:first-child{padding-top:calc(var(--u)*.8);border-top:1px solid #d9b77a8c}
@S[data-role=display] .df-aftermath .aftermath-portrait{width:calc(var(--u)*5.4);height:calc(var(--u)*5.4);font-size:calc(var(--u)*2.4)}
@S[data-role=display] .df-aftermath h3{font-size:calc(var(--u)*1.9);margin:0}
@S[data-role=display] .df-aftermath .aftermath-member p{margin:0;font-size:calc(var(--u)*1.25)}
@S[data-role=display] .df-aftermath .aftermath-hp{height:calc(var(--u)*1.7);margin-top:calc(var(--u)*.3);border-radius:calc(var(--u)*.85)}
@S[data-role=display] .df-aftermath .aftermath-hp-text{font-size:calc(var(--u)*1.25);line-height:calc(var(--u)*1.7)}
"#;

/// Aftermath and short-rest screens: the courier road painting over the combat painting.
const REST: &str = r#"
@S[data-screen=aftermath] .df-aftermath{background:linear-gradient(180deg,#05080a70 0,transparent 22%,transparent 40%,#05080ac4 66%,#05080af2),@BATTLE_BACKDROP center 38%/cover,#0b0f0c}
@S[data-screen=rest] .df-campfire{--camp-art:@BATTLE_BACKDROP !important;min-height:0;border-radius:0;border:0}
@S[data-role=player][data-screen=rest] .df-campfire{padding:20px 16px}
@S[data-role=player][data-screen=rest] .df-campfire .camp-location{font-size:13px;letter-spacing:.3em}
@S[data-role=player][data-screen=rest] .df-campfire{display:flex;flex-direction:column}
@S[data-role=player][data-screen=rest] .df-campfire .camp-location{order:0}
@S[data-role=player][data-screen=rest] .df-campfire h1{order:1;font-size:30px;margin-bottom:12px}
@S[data-role=player][data-screen=rest] .df-campfire .camp-narration{order:2}
@S[data-role=player][data-screen=rest] .df-campfire .camp-actions{order:3;margin-bottom:26px}
@S[data-role=player][data-screen=rest] .df-campfire .camp-party-label{order:4}
@S[data-role=player][data-screen=rest] .df-campfire .camp-party{order:5}
@S[data-role=player][data-screen=rest] .df-campfire .camp-notice{order:6}
@S[data-role=player][data-screen=rest] .df-campfire .camp-narration{font-size:16px;line-height:1.6;margin-bottom:22px}
@S[data-role=player][data-screen=rest] .df-campfire .camp-party-label{font-size:21px}
@S[data-role=player][data-screen=rest] .df-campfire .camp-party{grid-template-columns:1fr 1fr;gap:10px}
@S[data-role=player][data-screen=rest] .df-campfire .camp-member{padding:12px}
@S[data-role=player][data-screen=rest] .df-campfire .camp-member-name{font-size:19px;margin:0 0 6px}
@S[data-role=player][data-screen=rest] .df-campfire .camp-detail,@S[data-role=player][data-screen=rest] .df-campfire .camp-notice{font-size:14px}
@S[data-role=player][data-screen=rest] .df-campfire .camp-notice{font-size:16px;font-style:italic;font-family:Georgia,serif}
@S[data-role=player][data-screen=rest] .df-campfire .camp-actions{display:grid;gap:14px}
@S[data-role=player][data-screen=rest] .df-campfire .camp-actions button{font-size:18px;flex-basis:auto}
"#;

/// Hero header, inline sheet (concept panel 4) and the full-sheet overlay.
const SHEET: &str = r#"
.live-portrait{position:relative;flex:none;overflow:hidden;width:72px;height:72px;border-radius:12px;border:2px solid #cba466b3;background:#162230;display:grid;place-items:center;font-size:30px;color:#e3c68f}
.live-portrait::before{content:'';position:absolute;inset:0;z-index:2;background:center/cover no-repeat}
.live-hero-head{display:flex;align-items:center;gap:14px;margin:12px 16px 0;padding:12px}
.live-hero-meta{display:grid;gap:4px;flex:1;min-width:0}
.live-hero-name{font-size:23px;color:#f4e3bd;line-height:1.2}
.live-hero-line{font:15px/1.4 system-ui,sans-serif;color:#d7cbb3}
.live-hero-head .combat-hp,.live-sheet-inline .combat-hp{position:relative;height:20px;border:1px solid #d9b77a59;border-radius:10px;background:#0a0e0b;overflow:hidden}
.live-hero-head .combat-hp-fill,.live-sheet-inline .combat-hp-fill{position:absolute;inset:0 auto 0 0;width:var(--hp,0%);background:linear-gradient(90deg,#3f8553,#8fd18d)}
.live-hero-head .combat-hp[data-state=low] .combat-hp-fill,.live-sheet-inline .combat-hp[data-state=low] .combat-hp-fill{background:linear-gradient(90deg,#92392a,#e0784f)}
.live-hero-head .combat-hp-text,.live-sheet-inline .combat-hp-text{position:relative;display:block;text-align:center;font:700 13px/20px system-ui,sans-serif;color:#fff;text-shadow:0 0 3px #000,0 0 3px #000,0 1px 3px #000}
.live-sheet-inline{margin:20px 0 0;padding:16px;display:grid;gap:16px;scroll-margin-top:12px}
.live-sheet-head{display:flex;align-items:center;gap:14px}
.live-sheet-head .live-portrait{width:96px;height:96px;border-radius:14px}
.live-abilities{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:8px}
.live-ability{display:grid;justify-items:center;padding:8px 4px;border:1px solid #d9b77a40;border-radius:8px;background:#ffffff08;text-align:center}
.live-ability-label{font:700 12px/1.4 system-ui,sans-serif;letter-spacing:.12em;color:#c9bda3}
.live-ability-score{font-size:26px;line-height:1.1;color:#fff1d2}
.live-ability-mod{font:600 14px/1.4 system-ui,sans-serif;color:#e0b877}
.live-sheet-inline h3{margin:0 0 6px;font-size:19px;font-weight:400;color:#f2dfbb}
.live-sheet-inline ul{margin:0;padding:0;list-style:none;display:grid;gap:4px}
.live-sheet-inline li{font:15px/1.5 system-ui,sans-serif;color:#e4d9c2}
.live-sheet-inline li b{font-weight:600;color:#c9bda3}
.live-sheet-open{display:flex;align-items:center;justify-content:center;gap:.6em;width:100%;min-height:52px;padding:10px 16px;border:1px solid #c19d6099;border-radius:6px;background:#162027;color:#f4e5ca;font:600 16px/1.4 system-ui,sans-serif;cursor:pointer;touch-action:manipulation}
.live-sheet-open::before{content:'';width:1.5em;height:1.5em;background:var(--asset-icon-character) center/contain no-repeat}
.live-sheet-open:hover{border-color:#f0c985;background:#1f2b33}
.live-sheet-open:focus-visible,.live-sheet-close:focus-visible{outline:3px solid #ffe0a3;outline-offset:3px}
.live-sheet-overlay{position:fixed;inset:0;z-index:80;overflow:auto;background:#0b0f0c}
.live-sheet-close{position:fixed;z-index:90;top:12px;right:12px;min-height:48px;padding:10px 18px;border:1px solid #f0d195;border-radius:6px;background:linear-gradient(135deg,#f0d5a4,#c6a05e);color:#211b11;font:700 16px/1.4 system-ui,sans-serif;cursor:pointer;touch-action:manipulation}
"#;
