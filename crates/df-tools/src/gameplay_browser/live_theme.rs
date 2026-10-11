//! Presentation rules for the live lobby and exploration screens, installed once per page.
//!
//! The shared display (`.tv-stage`) is a fixed 16:9 canvas sized from `--u`, one hundredth
//! of the width of the largest 16:9 rectangle that fits the window, so type and panels
//! scale together on a 1080p or 4K television and never scroll. Optional generated art is
//! layered above its authored fallback in CSS, so an absent file leaves the fallback.
use super::*;

const STYLES: &str = r#"
body:has(.tv-stage){overflow:hidden}
body:has(.tv-stage) .client.display>.connection{position:fixed;z-index:30;top:.7vh;right:4.5vw;padding:0;background:none;border:0;color:#a8c9ba;font:clamp(14px,calc(min(1vw,1.7778vh)*1.1),48px)/1.2 system-ui,sans-serif}
.tv-stage{--u:min(1vw,1.7778vh);--gold:#e3c68f;--fs-title:clamp(40px,calc(var(--u)*4.6),220px);--fs-line:clamp(22px,calc(var(--u)*2.05),110px);--fs-name:clamp(24px,calc(var(--u)*2),100px);--fs-label:clamp(16px,calc(var(--u)*1.3),64px);--fs-code:clamp(48px,calc(var(--u)*5.6),280px);position:fixed;inset:0;z-index:20;overflow:hidden;background:#05090f;color:#f6efdf;font-family:Georgia,'Times New Roman',serif;text-shadow:0 2px 12px #000c}
.tv-stage *{box-sizing:border-box;min-width:0}
.tv-stage [hidden]{display:none!important}
.tv-art{position:absolute;inset:0;background-color:#0a1118;background-size:cover;background-position:center 42%}
.tv-scrim{position:absolute;inset:0;background:linear-gradient(180deg,#03070cc7 0%,#03070c00 22%,#03070c14 38%,#03070cd1 66%,#03070cf5 100%),linear-gradient(90deg,#03070cb3,#03070c00 62%)}
.tv-safe{position:absolute;inset:4.5vh 4.5vw;display:grid;grid-template-rows:auto minmax(0,1fr) auto auto;gap:calc(var(--u)*1.4)}
.tv-top{display:flex;justify-content:space-between;align-items:center;gap:calc(var(--u)*2)}
.tv-brand{display:flex;align-items:center;min-height:calc(var(--u)*6.5)}
.tv-wordmark{display:block;height:calc(var(--u)*6.5);width:auto;max-width:40vw}
.tv-wordmark-text{font-size:calc(var(--u)*4);letter-spacing:-.04em;color:#f3d59d}
.tv-wordmark-text b{font-weight:400;color:#b9d8e9}
.tv-eyebrow{font:var(--fs-label)/1.3 system-ui,sans-serif;letter-spacing:.18em;text-transform:uppercase;color:var(--gold)}
.tv-headline{display:grid;grid-template-columns:minmax(0,1.1fr) minmax(0,.9fr);gap:calc(var(--u)*3);align-items:end;min-height:0}
.tv-stage[data-tv-mode=scene] .tv-headline{grid-template-columns:minmax(0,1fr);align-items:start}
.tv-title{margin:0;font-size:var(--fs-title);font-weight:400;line-height:1.04;letter-spacing:-.02em;max-width:22ch;text-shadow:0 4px 40px #000}
.tv-divider{height:calc(var(--u)*1.4);margin:calc(var(--u)*1) 0;max-width:calc(var(--u)*34);background:url(/assets/ui/generated/divider) left center/contain no-repeat}
.tv-lede{margin:0;max-width:46ch;font-size:var(--fs-line);line-height:1.3;color:#eadfc6}
.tv-panel{position:relative;padding:calc(var(--u)*1.5) calc(var(--u)*2);background:linear-gradient(135deg,#0e1824e6,#060c14ea);border:max(2px,calc(var(--u)*.12)) solid #d6b16a8c;border-radius:calc(var(--u)*.5);box-shadow:0 calc(var(--u)*1) calc(var(--u)*3) #000a;border-image:url(/assets/ui/generated/panel-frame) 14% fill/calc(var(--u)*1.6)/0 stretch}
.tv-join{justify-self:end;width:min(100%,calc(var(--u)*40));text-align:center}
.tv-code{font-size:var(--fs-code);letter-spacing:.14em;line-height:1.1;color:#f4d9a0;margin:calc(var(--u)*.8) 0;padding-left:.14em;overflow-wrap:anywhere}
.tv-url{display:block;font:var(--fs-label)/1.35 system-ui,sans-serif;color:#d9e6be;overflow-wrap:anywhere;text-decoration:none;text-shadow:none}
.tv-hint{margin:calc(var(--u)*.8) 0 0;font:var(--fs-label)/1.35 system-ui,sans-serif;color:#b8c5d2}
.tv-band{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1.1fr);gap:calc(var(--u)*1.6);align-items:stretch}
.tv-stage[data-tv-mode=lobby] .tv-band{display:none}
.tv-stage[data-tv-mode=scene] .tv-lede{display:none}
.tv-narration{margin:0;font-size:var(--fs-line);line-height:1.3;max-width:46ch;display:-webkit-box;-webkit-line-clamp:4;-webkit-box-orient:vertical;overflow:hidden}
.tv-speaker{display:grid;grid-template-columns:auto minmax(0,1fr);gap:calc(var(--u)*1.6);align-items:center}
.tv-portrait{position:relative;flex:none;width:calc(var(--u)*7);height:calc(var(--u)*7);border-radius:12%;overflow:hidden;border:max(2px,calc(var(--u)*.1)) solid #cba466b3;background:#162230}
.tv-portrait-glyph{position:absolute;inset:0;display:grid;place-items:center;font-size:calc(var(--u)*3.2);color:var(--gold)}
.tv-portrait-img{position:absolute;inset:0;background-size:cover;background-position:center 20%}
.tv-speaker-name{margin:0 0 calc(var(--u)*.3);font-size:var(--fs-name);font-weight:400;line-height:1.2;color:var(--gold)}
.tv-quote{margin:0;max-width:46ch;font-size:var(--fs-line);line-height:1.3;font-style:italic;display:-webkit-box;-webkit-line-clamp:3;-webkit-box-orient:vertical;overflow:hidden}
.tv-party{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:calc(var(--u)*1.6)}
.tv-card{display:grid;grid-template-columns:auto minmax(0,1fr);gap:calc(var(--u)*1.4);align-items:center;padding:calc(var(--u)*1) calc(var(--u)*1.4)}
.tv-card[data-empty]{opacity:.62;border-style:dashed}
.tv-card .tv-portrait{width:calc(var(--u)*5.6);height:calc(var(--u)*5.6)}
.tv-card .tv-portrait-glyph{font-size:calc(var(--u)*2.6)}
.tv-member-name{font-size:var(--fs-name);line-height:1.2;overflow-wrap:anywhere}
.tv-member-status{font:var(--fs-label)/1.3 system-ui,sans-serif;color:#cde2c6;text-shadow:none}
.tv-card[data-ready=false] .tv-member-status{color:#e4c38b}
.tv-dock{position:absolute;z-index:3;right:4.5vw;top:calc(4.5vh + var(--u)*8);width:min(34vw,calc(var(--u)*44));font:var(--fs-label)/1.4 system-ui,sans-serif;text-shadow:none}
.tv-dock:empty{display:none}
.tv-dock p,.tv-dock summary{font-size:var(--fs-label)}
.tv-dock button{font-size:var(--fs-label);min-height:44px}
.tv-dock .feedback:empty{display:none}
.tv-legal{position:absolute;z-index:2;left:4.5vw;right:4.5vw;bottom:.8vh;margin:0;font:clamp(11px,calc(var(--u)*.6),24px)/1.3 system-ui,sans-serif;color:#9aa8a4;opacity:.7;text-shadow:none}
.tv-legal a{color:inherit}

body:has(.df-campaign)>header{display:none}
main.player-only:has(.df-campaign){display:block;max-width:none;padding:0;min-height:0}
main.player-only:has(.df-campaign) .client.player{border:0;border-radius:0;background:transparent;box-shadow:none;overflow:visible}
.client.player .live-dock{width:min(1100px,92%);margin:0 auto;padding:0 0 28px;position:relative;z-index:1}
.client.player .live-dock .feedback{font-size:16px}
.client.player .df-exploration .exploration-choice-detail{display:none}
.client.player .df-exploration .exploration-actions .df-ui-field,.client.player .df-exploration .exploration-submit{display:none}
.client.player .df-exploration .exploration-empty{display:none}
.client.player .df-exploration .party-section{display:none}
@media(max-width:700px){
.client.player .df-exploration .exploration-choices{grid-template-columns:1fr}
.client.player .df-exploration .exploration-choice button{font-size:17px;min-height:48px}
.client.player .df-exploration .description,.client.player .df-exploration .exploration-dialogue,.client.player .df-exploration .narration blockquote{font-size:16px;line-height:1.5}
.client.player .df-campaign[data-join-phase] .join-description,.client.player .df-campaign[data-join-phase] .join-presence,.client.player .df-campaign[data-join-phase] .join-readiness,.client.player .df-campaign[data-join-phase] .join-status{font-size:16px}
.client.player .df-campaign[data-join-phase] .join-panel button{min-height:48px;font-size:16px}
.client.player .live-dock{width:90%}
.client.player .live-dock .tools button,.client.player .live-dock .dialogue-controls button{min-height:48px;font-size:16px}
}
"#;

/// Adds the live screen rules once; a second call on the same page is a no-op.
pub(super) fn install(document: &Document) -> Result<(), JsValue> {
    if document.query_selector("style[data-live-theme]")?.is_some() {
        return Ok(());
    }
    let style = document.create_element("style")?;
    style.set_attribute("data-live-theme", "")?;
    style.set_text_content(Some(STYLES));
    let host = match document.query_selector("head")? {
        Some(head) => head,
        None => document
            .document_element()
            .ok_or_else(|| JsValue::from_str("document element absent"))?,
    };
    host.append_child(&style)?;
    Ok(())
}
