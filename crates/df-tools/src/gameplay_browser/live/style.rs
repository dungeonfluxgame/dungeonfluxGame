//! Scoped presentation for the live combat, aftermath, rest and character-sheet screens.
//!
//! The df-ui themes style the surfaces themselves. This stylesheet only (a) fits them to
//! the live roles — a phone-first single column for players and a fixed 16:9, 10-foot
//! layout for the shared display — and (b) attaches optional generated art through the
//! slots named in [`ART_SLOTS`]. Every art slot has a CSS-only fallback: a missing or
//! failed image leaves the plain gold-and-ink rendering visible.

/// Single place that turns a generated-art file name into its served URL.
pub(super) fn generated_asset_url(name: &str) -> String {
    format!("/assets/generated/{name}")
}

/// Combat scene art for both roles. It is served by the gameplay public-asset route.
pub(super) const COMBAT_ART_URL: &str =
    "/assets/concept-art/combat-harbor-docks-wide-lanterns.webp";

/// CSS custom property -> generated file name (and the aspect ratio the art should have).
/// The portraits are shown as circles, so square head-and-shoulders crops are expected.
pub(super) const ART_SLOTS: [(&str, &str); 14] = [
    ("--asset-d20", "d20-blank.png"), // 1:1, transparent, blank top face
    ("--asset-ribbon-success", "ribbon-success.png"), // about 4:1
    ("--asset-ribbon-failure", "ribbon-failure.png"), // about 4:1
    ("--asset-frame-panel", "frame-panel.png"), // square, nine-slice friendly
    ("--asset-frame-button", "frame-button.png"), // about 4:1, nine-slice friendly
    ("--asset-divider", "divider-ornament.png"), // about 8:1
    ("--asset-icon-attack", "icon-attack.png"), // 1:1
    ("--asset-icon-second-wind", "icon-second-wind.png"), // 1:1
    ("--asset-icon-end-turn", "icon-end-turn.png"), // 1:1
    ("--asset-icon-short-rest", "icon-short-rest.png"), // 1:1
    ("--asset-icon-journey", "icon-journey.png"), // 1:1
    ("--asset-icon-character", "icon-character.png"), // 1:1
    ("--asset-portrait-hero-1", "portrait-hero-1.png"), // 1:1
    ("--asset-portrait-hero-2", "portrait-hero-2.png"), // 1:1
];
/// Not a CSS slot of its own: the bandit portrait shares the portrait rules.
pub(super) const BANDIT_PORTRAIT: (&str, &str) = ("--asset-portrait-bandit", "portrait-bandit.png");

pub(super) fn stylesheet() -> String {
    let mut variables = String::new();
    for (property, file) in ART_SLOTS.iter().chain(std::iter::once(&BANDIT_PORTRAIT)) {
        variables.push_str(&format!("{property}:url('{}');", generated_asset_url(file)));
    }
    format!(
        ":root{{{variables}--frame-slice:30%;--frame-width:clamp(10px,1vw,18px)}}\n{COMMON}\n{PLAYER}\n{DISPLAY}\n{REST}\n{SHEET}"
    )
    .replace("{COMBAT_ART}", COMBAT_ART_URL)
}

/// Optional art attached to the surfaces' stable hooks. Each rule is a decoration on top of a
/// fully readable fallback.
const COMMON: &str = r#"
.live-screen{position:relative;isolation:isolate;color:#f4eddf;font-family:Georgia,'Times New Roman',serif}
.live-screen *{box-sizing:border-box;min-width:0}
.live-screen[data-screen=combat] .df-combat .combat-art{object-position:50% 46%}
.live-screen .combat-initiative,.live-screen .combat-roll-panel,.live-screen .combat-action-rail,.live-screen .df-campfire .camp-member,.live-screen .df-aftermath .aftermath-panel{position:relative}
.live-screen .combat-initiative::after,.live-screen .combat-roll-panel::after,.live-screen .combat-action-rail::after,.live-screen .df-campfire .camp-member::after,.live-screen .df-aftermath .aftermath-panel::after{content:'';position:absolute;inset:-3px;pointer-events:none;border:var(--frame-width) solid transparent;border-image:var(--asset-frame-panel) var(--frame-slice) / var(--frame-width) stretch}
.live-screen .combat-offer button{position:relative}
.live-screen .combat-offer[data-kind=action] button::after{content:'';position:absolute;inset:-3px;pointer-events:none;border:var(--frame-width) solid transparent;border-image:var(--asset-frame-button) var(--frame-slice) / var(--frame-width) stretch}
.live-screen button[data-combat-offer=strike]::before,.live-screen button[data-combat-offer=second-wind]::before,.live-screen button[data-combat-offer=end-turn]::before{content:'';flex:none;display:inline-block;width:1.5em;height:1.5em;margin-right:.6em;vertical-align:-.35em;background:center/contain no-repeat}
.live-screen button[data-combat-offer=strike]::before{background-image:var(--asset-icon-attack)}
.live-screen button[data-combat-offer=second-wind]::before{background-image:var(--asset-icon-second-wind)}
.live-screen button[data-combat-offer=end-turn]::before{background-image:var(--asset-icon-end-turn)}
.live-screen .camp-actions button{position:relative}
.live-screen .camp-actions button:first-child::before{content:'';display:inline-block;width:1.4em;height:1.4em;margin-right:.55em;vertical-align:-.3em;background:var(--asset-icon-short-rest) center/contain no-repeat}
.live-screen .camp-actions button:last-child::before{content:'';display:inline-block;width:1.4em;height:1.4em;margin-right:.55em;vertical-align:-.3em;background:var(--asset-icon-journey) center/contain no-repeat}
.live-screen .combat-actor .combat-initiative-value{position:relative;overflow:hidden;isolation:isolate}
.live-screen .combat-actor .combat-initiative-value::before{content:'';position:absolute;inset:0;z-index:2;border-radius:50%;background:center/cover no-repeat}
.live-screen .combat-actor[data-portrait=hero-1] .combat-initiative-value::before{background-image:var(--asset-portrait-hero-1)}
.live-screen .combat-actor[data-portrait=hero-2] .combat-initiative-value::before{background-image:var(--asset-portrait-hero-2)}
.live-screen .combat-actor[data-portrait=bandit] .combat-initiative-value::before{background-image:var(--asset-portrait-bandit)}
.live-screen .combat-actor{align-items:center}
.live-screen .combat-roll:first-child .combat-roll-value{position:relative;isolation:isolate;clip-path:none;background:none;border:0;overflow:visible;text-shadow:0 2px 6px #000,0 0 18px #000}
.live-screen .combat-roll:first-child .combat-roll-value::before{content:'';position:absolute;inset:7%;z-index:-2;clip-path:polygon(25% 0,75% 0,100% 25%,100% 75%,75% 100%,25% 100%,0 75%,0 25%);background:linear-gradient(140deg,#9a7138,#171c15 85%)}
.live-screen .combat-roll:first-child .combat-roll-value::after{content:'';position:absolute;inset:-4%;z-index:-1;background:var(--asset-d20) center/contain no-repeat}
.live-screen .combat-roll-verdict{position:relative;isolation:isolate;background:none;clip-path:none;padding:.2em 2.4em;text-align:center}
.live-screen .combat-roll-verdict::before{content:'';position:absolute;inset:0;z-index:-2;clip-path:polygon(0 0,100% 0,calc(100% - .8em) 50%,100% 100%,0 100%,.8em 50%);background:#6b5a35}
.live-screen .combat-roll-verdict[data-tone=success]::before{background:linear-gradient(#2f8150,#1d5a37)}
.live-screen .combat-roll-verdict[data-tone=failure]::before{background:linear-gradient(#963a2e,#65261f)}
.live-screen .combat-roll-verdict::after{content:'';position:absolute;inset:-20% -6%;z-index:-1;background:center/100% 100% no-repeat}
.live-screen .combat-roll-verdict[data-tone=success]::after{background-image:var(--asset-ribbon-success)}
.live-screen .combat-roll-verdict[data-tone=failure]::after{background-image:var(--asset-ribbon-failure)}
.live-screen .live-divider{position:relative;height:1px;margin:22px 0;border:0;background:linear-gradient(90deg,transparent,#d9b77a99,transparent)}
.live-screen .live-divider::after{content:'';position:absolute;left:0;right:0;top:50%;height:28px;transform:translateY(-50%);background:var(--asset-divider) center/contain no-repeat}
.live-screen .live-extras .dialogue-recovery,.live-screen .live-extras .recovery-needed{border-color:#c8a76b66;background:linear-gradient(135deg,#1d241ef5,#151b16e8);color:#f4eddf}
.live-screen .live-extras .dialogue-recovery h3{color:#f2dfbb}
.live-screen .live-extras .dialogue-controls button,.live-screen .live-extras .tools button{min-height:48px;border-color:#c19d6066;background:#303329;color:#f4e5ca;font-size:15px}
.live-screen .live-extras .dialogue-controls button:first-child{background:linear-gradient(135deg,#f0d5a4,#c6a05e);color:#211b11;border-color:#f0d195;font-weight:650}
.live-screen .live-extras .feedback{font:16px/1.5 system-ui,sans-serif;color:#f0d19e;margin:12px 0 0}
.live-screen .live-extras .feedback:empty{display:none}
.live-screen .live-extras .trouble{color:#c9c2b2}
.live-sheet-open{display:flex;align-items:center;justify-content:center;gap:.6em;width:100%;min-height:56px;margin:18px 0 0;padding:12px 16px;border:1px solid #c19d6099;border-radius:6px;background:linear-gradient(135deg,#252c24f2,#161d17f2);color:#f4e5ca;font:600 17px/1.4 system-ui,sans-serif;cursor:pointer;touch-action:manipulation}
.live-sheet-open::before{content:'';width:1.5em;height:1.5em;background:var(--asset-icon-character) center/contain no-repeat}
.live-sheet-open:hover{border-color:#f0c985;background:#303829}
.live-sheet-open:focus-visible,.live-sheet-close:focus-visible{outline:3px solid #ffe0a3;outline-offset:3px}
"#;

/// Phone-first single column, valid at any container width (375-430px phones are the target).
const PLAYER: &str = r#"
.live-screen[data-role=player]{max-width:560px;margin:0 auto}
.live-screen[data-role=player] .df-combat{display:flex;flex-direction:column;min-height:0;font-size:16px}
.live-screen[data-role=player] .combat-art{height:420px;object-position:46% center}
.live-screen[data-role=player] .df-combat:before{background:linear-gradient(180deg,#11151266,transparent 17%,#111512b3 330px,#111512 470px)}
.live-screen[data-role=player] .combat-topbar{order:0;flex-wrap:wrap;gap:6px 12px;padding:16px 16px 0}
.live-screen[data-role=player] .combat-brand{font-size:24px;flex:1}
.live-screen[data-role=player] .combat-notice{font-size:11px;max-width:150px;padding:5px 8px}
.live-screen[data-role=player] .combat-location{order:3;flex-basis:100%;font-size:11px}
.live-screen[data-role=player] .combat-stage{display:contents}
.live-screen[data-role=player] .combat-story{order:1;padding:112px 16px 14px}
.live-screen[data-role=player] .combat-story .combat-overline{display:none}
.live-screen[data-role=player] h1{font-size:34px;letter-spacing:-.5px;margin:0 0 12px}
.live-screen[data-role=player] .combat-turn-badge{display:block;margin:0 0 12px;padding:10px 14px;font:700 16px/1.4 system-ui,sans-serif;letter-spacing:1px;color:#ffe0a3;border-color:#e0b877;border-left-width:4px}
.live-screen[data-role=player] .combat-narration{font-size:16px;line-height:1.6}
.live-screen[data-role=player] .combat-lower{order:2;display:flex;flex-direction:column;gap:16px;width:100%;margin:0;padding:0 16px 16px}
.live-screen[data-role=player] .combat-action-rail{padding:16px}
.live-screen[data-role=player] .combat-offers{grid-template-columns:1fr;gap:10px}
.live-screen[data-role=player] .combat-offer button{display:flex;align-items:center;justify-content:flex-start;min-height:56px;padding:12px 16px;font-size:17px;text-align:left}
.live-screen[data-role=player] .combat-offer[data-kind=choice] button{min-height:48px;margin-left:14px;width:calc(100% - 14px);font-size:16px}
.live-screen[data-role=player] .combat-offer p{font-size:14px;line-height:1.5;margin:6px 2px 0}
.live-screen[data-role=player] .combat-offer[data-kind=choice] p{margin-left:16px}
.live-screen[data-role=player] .combat-muted{font-size:14px}
.live-screen[data-role=player] .combat-roll-panel{padding:16px}
.live-screen[data-role=player] .combat-roll:first-child{grid-template-columns:116px 1fr;column-gap:16px}
.live-screen[data-role=player] .combat-roll:first-child .combat-roll-value{font-size:62px}
.live-screen[data-role=player] .combat-roll h3{font-size:16px}
.live-screen[data-role=player] .combat-roll:first-child .combat-roll-explanation{font-size:18px;color:#fff1d2}
.live-screen[data-role=player] .combat-roll-explanation,.live-screen[data-role=player] .combat-roll-source{font-size:14px}
.live-screen[data-role=player] .combat-roll-verdict{font-size:15px}
.live-screen[data-role=player] .combat-initiative{order:3;width:auto;margin:0 16px 16px;padding:16px}
.live-screen[data-role=player] .combat-initiative>h2{display:none}
.live-screen[data-role=player] .combat-actor h3{font-size:19px}
.live-screen[data-role=player] .combat-connection{order:4;font-size:12px;padding:0 16px 8px}
.live-screen[data-role=player] .live-extras{padding:0 16px 24px}
.live-screen[data-role=player] .df-combat button{min-height:48px}
"#;

/// A fixed 16:9, 10-foot interface. `--u` is 1% of the width on a 16:9 canvas and tracks the
/// smaller viewport dimension otherwise (19.2px on 1920x1080), so every size scales together.
const DISPLAY: &str = r#"
body:has(.live-screen[data-role=display][data-fit=viewport]){overflow:hidden}
.live-screen[data-role=display][data-fit=viewport]{position:fixed;inset:0;z-index:40;overflow:hidden;background:#05080a;--u:min(1vw,1.7778vh)}
.live-screen[data-role=display] .df-combat{display:grid;width:100%;height:100%;min-height:0;overflow:hidden;font-size:max(24px,calc(var(--u)*1.3));grid-template-columns:minmax(0,1.08fr) minmax(0,1fr);grid-template-rows:auto auto auto minmax(0,1fr) auto;grid-template-areas:"top top" "banner banner" "title roll" "narr roll" "cast cast";gap:calc(var(--u)*1.1) calc(var(--u)*2.6);padding:4.2vh 4.5vw}
.live-screen[data-role=display] .df-combat:before{background:linear-gradient(180deg,#05080ad9 0,#05080a40 20%,#05080a33 38%,#05080aeb 76%,#05080a),linear-gradient(90deg,#05080acc,transparent 62%)}
.live-screen[data-role=display] .combat-topbar{grid-area:top;padding:0;gap:calc(var(--u)*2);background:none}
.live-screen[data-role=display] .combat-brand{font-size:max(28px,calc(var(--u)*1.9))}
.live-screen[data-role=display] .combat-location{font-size:max(24px,calc(var(--u)*1.2));letter-spacing:.16em}
.live-screen[data-role=display] .combat-notice{display:none}
.live-screen[data-role=display] .combat-stage,.live-screen[data-role=display] .combat-lower,.live-screen[data-role=display] .combat-story{display:contents}
.live-screen[data-role=display] .combat-story .combat-overline,.live-screen[data-role=display] .combat-action-rail,.live-screen[data-role=display] .combat-connection{display:none}
.live-screen[data-role=display] .combat-turn-badge{grid-area:banner;display:block;margin:0;padding:.12em .6em;font:600 clamp(56px,calc(var(--u)*3.6),72px)/1.15 Georgia,serif;letter-spacing:0;text-transform:none;color:#ffe3a8;border:0;border-left:.12em solid #e0b877;border-radius:0;background:linear-gradient(90deg,#1c1508f2,#1c150880 70%,transparent);text-shadow:0 3px 18px #000}
.live-screen[data-role=display] h1{grid-area:title;align-self:end;margin:0;max-width:none;font-size:max(40px,calc(var(--u)*2.7));letter-spacing:-.02em;line-height:1.08}
.live-screen[data-role=display] .combat-narration{grid-area:narr;align-self:start;max-width:none;overflow:hidden;font-size:max(24px,calc(var(--u)*1.55));line-height:1.4;display:-webkit-box;-webkit-line-clamp:4;-webkit-box-orient:vertical}
.live-screen[data-role=display] .combat-roll-panel{grid-area:roll;display:flex;flex-direction:column;min-height:0;overflow:hidden;padding:calc(var(--u)*1.2) calc(var(--u)*1.6);border:1px solid #d5af7080;border-left:calc(var(--u)*.3) solid #d5af70;border-radius:calc(var(--u)*.6);background:linear-gradient(90deg,#090c0af2,#0d110ed9)}
.live-screen[data-role=display] .combat-roll-panel h2{margin:0 0 calc(var(--u)*.6);font-size:max(32px,calc(var(--u)*1.7))}
.live-screen[data-role=display] .combat-roll-panel .combat-muted{font-size:max(24px,calc(var(--u)*1.3))}
.live-screen[data-role=display] .combat-rolls{display:flex;flex-direction:column;gap:calc(var(--u)*.5);min-height:0}
.live-screen[data-role=display] .combat-roll{padding:0;margin:0;border:0;grid-template-columns:calc(var(--u)*3.6) 1fr;column-gap:calc(var(--u)*1.3);align-items:center}
.live-screen[data-role=display] .combat-roll:nth-child(n+5){display:none}
.live-screen[data-role=display] .combat-roll .combat-roll-value{grid-row:1 / span 2;font-size:max(28px,calc(var(--u)*2));width:calc(var(--u)*3.6);height:calc(var(--u)*3.6)}
.live-screen[data-role=display] .combat-roll h3{font-size:max(24px,calc(var(--u)*1.3));margin:0}
.live-screen[data-role=display] .combat-roll .combat-roll-explanation{margin:0;font-size:max(24px,calc(var(--u)*1.25));color:#e6dbc3}
.live-screen[data-role=display] .combat-roll .combat-roll-verdict,.live-screen[data-role=display] .combat-roll .combat-roll-source{display:none}
.live-screen[data-role=display] .combat-roll:first-child{grid-template-columns:calc(var(--u)*12.5) 1fr;column-gap:calc(var(--u)*2);padding-bottom:calc(var(--u)*.9);border-bottom:1px solid #d5af7059;margin-bottom:calc(var(--u)*.3)}
.live-screen[data-role=display] .combat-roll:first-child .combat-roll-value{grid-row:1 / span 4;width:calc(var(--u)*12.5);height:calc(var(--u)*12.5);font-size:max(120px,calc(var(--u)*7.2))}
.live-screen[data-role=display] .combat-roll:first-child h3{font-size:max(32px,calc(var(--u)*1.9));color:#fff1d2}
.live-screen[data-role=display] .combat-roll:first-child .combat-roll-explanation{font-size:max(32px,calc(var(--u)*2.2));color:#fff;margin:calc(var(--u)*.2) 0}
.live-screen[data-role=display] .combat-roll:first-child .combat-roll-verdict{display:block;font-size:max(32px,calc(var(--u)*2));margin:calc(var(--u)*.2) 0;letter-spacing:.14em;justify-self:start}
.live-screen[data-role=display] .combat-roll:first-child .combat-roll-source{display:block;font-size:max(24px,calc(var(--u)*1.3));color:#efe3c8;margin:0}
.live-screen[data-role=display] .combat-initiative{grid-area:cast;padding:calc(var(--u)*1.1) calc(var(--u)*1.4);border-radius:calc(var(--u)*.7);background:linear-gradient(145deg,#161c15f2,#0b0f0cf5)}
.live-screen[data-role=display] .combat-initiative>h2{display:none}
.live-screen[data-role=display] .combat-actors{display:flex;gap:calc(var(--u)*1.4)}
.live-screen[data-role=display] .combat-actor{flex:1 1 0;min-width:0;gap:calc(var(--u)*1.1);padding:calc(var(--u)*.7) calc(var(--u)*.9);border:1px solid #b69b6659;border-radius:calc(var(--u)*.5);background:#ffffff08}
.live-screen[data-role=display] .combat-actor:first-child{border-top:1px solid #b69b6659;padding-top:calc(var(--u)*.7)}
.live-screen[data-role=display] .combat-actor[data-active=true]{border-color:#f0c985;background:linear-gradient(135deg,#d3a2593d,#d3a25912);box-shadow:0 0 calc(var(--u)*1.6) #e0b87747,inset 0 -.2em #e0b877;padding-left:calc(var(--u)*.9)}
.live-screen[data-role=display] .combat-actor .combat-initiative-value{flex:none;width:calc(var(--u)*5.6);height:calc(var(--u)*5.6);min-width:0;max-width:none;padding:0;font-size:max(32px,calc(var(--u)*2.6))}
.live-screen[data-role=display] .combat-actor h3{font-size:max(32px,calc(var(--u)*2))}
.live-screen[data-role=display] .combat-actor .combat-muted{margin:0;font-size:max(24px,calc(var(--u)*1.25))}
.live-screen[data-role=display] .combat-hp{height:calc(var(--u)*1.8);margin-top:calc(var(--u)*.4);border-radius:calc(var(--u)*.9)}
.live-screen[data-role=display] .combat-hp-text{font-size:max(24px,calc(var(--u)*1.2));line-height:calc(var(--u)*1.8)}
.live-screen[data-role=display] .combat-resources{display:none}
.live-screen[data-role=display] .live-extras{position:fixed;z-index:60;top:1.4vh;right:2vw;max-width:34vw;font-size:max(24px,calc(var(--u)*1.2))}
.live-screen[data-role=display] .live-extras .trouble{margin:0;font-size:max(24px,calc(var(--u)*1.2));background:#05080acc;border-radius:8px;padding:0 .8em}
.live-screen[data-role=display] .live-extras .trouble summary{padding:.4em 0}
.live-screen[data-role=display] .live-extras .tools button{font-size:max(24px,calc(var(--u)*1.2));min-height:max(48px,calc(var(--u)*2.6))}
.live-screen[data-role=display] .live-extras .recovery-needed{background:#2a2417f2;margin:0 0 .5em}
.live-screen[data-role=display] .live-extras .recovery-needed p,.live-screen[data-role=display] .live-extras .feedback{font-size:max(24px,calc(var(--u)*1.2))}
.live-screen[data-role=display][data-screen=aftermath] .df-aftermath{height:100%;min-height:0;overflow:hidden;display:grid;grid-template-rows:minmax(0,1fr) auto;padding:4.2vh 4.5vw;font-size:max(24px,calc(var(--u)*1.3))}
.live-screen[data-role=display] .df-aftermath .aftermath-story{max-width:min(62vw,1400px);margin:0;align-self:end}
.live-screen[data-role=display] .df-aftermath .aftermath-chapter{font-size:max(24px,calc(var(--u)*1.2))}
.live-screen[data-role=display] .df-aftermath h1{font-size:max(48px,calc(var(--u)*3.8));margin-bottom:calc(var(--u)*1)}
.live-screen[data-role=display] .df-aftermath .aftermath-narrative{font-size:max(26px,calc(var(--u)*1.6));line-height:1.45}
.live-screen[data-role=display] .df-aftermath .aftermath-lower{max-width:none;width:100%;margin:calc(var(--u)*1.4) 0 0;gap:calc(var(--u)*1.6)}
.live-screen[data-role=display] .df-aftermath .aftermath-panel{padding:calc(var(--u)*1.2) calc(var(--u)*1.6)}
.live-screen[data-role=display] .df-aftermath h2{font-size:max(32px,calc(var(--u)*1.8))}
.live-screen[data-role=display] .df-aftermath h3{font-size:max(32px,calc(var(--u)*1.7))}
.live-screen[data-role=display] .df-aftermath .aftermath-member p,.live-screen[data-role=display] .df-aftermath .aftermath-next-context,.live-screen[data-role=display] .df-aftermath .aftermath-empty{font-size:max(24px,calc(var(--u)*1.25))}
"#;

/// Aftermath and short-rest screens share the harbor art and the combat colors.
const REST: &str = r#"
.live-screen[data-screen=aftermath] .df-aftermath{background:linear-gradient(180deg,#05080a80,#05080aeb 62%,#05080a),url('{COMBAT_ART}') center 38%/cover,#0b0f0c}
.live-screen[data-screen=rest] .df-campfire{--camp-art:url('{COMBAT_ART}') !important;min-height:0;border-radius:0;border:0}
.live-screen[data-role=player][data-screen=rest] .df-campfire{padding:20px 16px}
.live-screen[data-role=player][data-screen=rest] .df-campfire h1{font-size:34px}
.live-screen[data-role=player][data-screen=rest] .df-campfire .camp-narration{font-size:16px;line-height:1.6}
.live-screen[data-role=player][data-screen=rest] .df-campfire .camp-actions button{display:flex;align-items:center;justify-content:center;min-height:56px;font-size:17px;flex-basis:100%;max-width:none}
.live-screen[data-role=player][data-screen=rest] .df-campfire .camp-detail,.live-screen[data-role=player][data-screen=rest] .df-campfire .camp-notice{font-size:14px}
"#;

/// The full sheet opens above everything else and is closed by its own control.
const SHEET: &str = r#"
.live-sheet-overlay{position:fixed;inset:0;z-index:80;overflow:auto;background:#0b0f0c}
.live-sheet-close{position:fixed;z-index:90;top:12px;right:12px;min-height:48px;padding:10px 18px;border:1px solid #f0d195;border-radius:6px;background:linear-gradient(135deg,#f0d5a4,#c6a05e);color:#211b11;font:700 16px/1.4 system-ui,sans-serif;cursor:pointer;touch-action:manipulation}
.live-hero{display:grid;gap:10px;padding:16px;margin:18px 0 0;border:1px solid #c8a76b66;border-radius:9px;background:linear-gradient(135deg,#242b22f2,#151d17ef);box-shadow:0 12px 35px #0003}
.live-hero-name{font-size:24px;color:#f2dfbb}
.live-hero-line{font:15px/1.5 system-ui,sans-serif;color:#d7cbb3}
.live-hero .combat-hp{position:relative;height:20px;border:1px solid #d9b77a59;border-radius:10px;background:#0a0e0b;overflow:hidden}
.live-hero .combat-hp-fill{position:absolute;inset:0 auto 0 0;width:var(--hp,0%);background:linear-gradient(90deg,#3f8553,#8fd18d)}
.live-hero .combat-hp[data-state=low] .combat-hp-fill{background:linear-gradient(90deg,#92392a,#e0784f)}
.live-hero .combat-hp-text{position:relative;display:block;text-align:center;font:700 13px/20px system-ui,sans-serif;color:#fff;text-shadow:0 1px 3px #000}
"#;
