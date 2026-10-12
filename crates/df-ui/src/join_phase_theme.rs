pub(crate) const STYLES: &str = r#"
.df-campaign[data-join-phase]{background:#070d14}
.df-campaign[data-join-phase] .stage{grid-template-columns:minmax(0,1fr) 410px;gap:6vw;align-items:center;padding:85px 0 65px;min-height:640px}
.df-campaign[data-join-phase] .scene-panel,.df-campaign[data-join-phase] .party-section{display:none}
.df-campaign[data-join-phase] .lower{grid-template-columns:minmax(0,1fr);max-width:1520px}
.df-campaign[data-join-phase] h1{max-width:770px;font-size:clamp(50px,6.6vw,102px);letter-spacing:-2.5px}
.df-campaign[data-join-phase] .description{max-width:550px;color:#eee3d1}
.df-campaign[data-join-phase] .scene-art{object-position:46% center}
.df-campaign[data-join-phase] .join-panel{position:relative;min-width:0;padding:34px;background:linear-gradient(145deg,#142031f5,#080f19f5);border:1px solid #c3a26b85;border-radius:5px;box-shadow:0 24px 70px #0009,inset 0 1px #ead1a126}
.df-campaign[data-join-phase] .join-panel:before{content:'✦';position:absolute;top:-13px;left:calc(50% - 21px);width:42px;text-align:center;color:#e4c38b;background:#111b29;font-size:22px}
.df-campaign[data-join-phase] .join-heading{margin:0 0 12px;color:#f1d6a5;font-size:31px;font-weight:400;line-height:1.15}
.df-campaign[data-join-phase] .join-description{font:13px/1.8 system-ui,sans-serif;color:#b8c5d2;margin:0 0 24px}
.df-campaign[data-join-phase] .join-fields{display:grid;gap:18px}
.df-campaign[data-join-phase] .join-fields .df-ui-field{margin:0;display:block;min-width:0}
.df-campaign[data-join-phase] .join-fields label>span{display:block;margin-bottom:9px;font:11px/1.5 system-ui,sans-serif;letter-spacing:1.1px;color:#dcc69f}
.df-campaign[data-join-phase] input{display:block;width:100%;min-width:0;min-height:48px;border:1px solid #7995ac66;border-radius:3px;padding:12px 13px;font:16px/1.5 system-ui,sans-serif;background:#050c16;color:#eef0ef;box-shadow:inset 0 1px 5px #0004}
.df-campaign[data-join-phase] input:focus-visible{outline:2px solid #a3d9fb;outline-offset:3px;border-color:#a3d9fb}
.df-campaign[data-join-phase] input[readonly]{color:#b9c6d1;background:#0f1822}
.df-campaign[data-join-phase] input[aria-invalid=true]{border-color:#e2a891}
.df-campaign[data-join-phase] .df-ui-feedback{font:11px/1.6 system-ui,sans-serif;color:#e4b497;margin:6px 0 0}
.df-campaign[data-join-phase] .df-ui-feedback:empty{display:none}
.df-campaign[data-join-phase] .join-panel button{width:100%;min-height:49px;padding:12px 18px;border:1px solid #e0c18b;background:linear-gradient(120deg,#e2c48e,#b59056);color:#171c25;font:600 13px/1.5 system-ui,sans-serif;letter-spacing:.6px;box-shadow:0 4px 14px #0003}
.df-campaign[data-join-phase] .join-panel button:hover{background:#f1d6a4;color:#171c25}
.df-campaign[data-join-phase] .join-panel button:disabled{opacity:.58;cursor:default}
.df-campaign[data-join-phase] [hidden]{display:none!important}
.df-campaign[data-join-phase] .join-room{border:1px solid #bea27050;border-radius:3px;padding:20px;text-align:center;background:#050c1666;margin:20px 0 26px}
.df-campaign[data-join-phase] .join-room-code{font:28px/1.6 Georgia,serif;letter-spacing:4px;color:#f3d89f;overflow-wrap:anywhere}
.df-campaign[data-join-phase] .join-room-detail{font:11px/1.7 system-ui,sans-serif;color:#acbdca;margin:4px 0 0}
.df-campaign[data-join-phase] .join-roster-heading{font:10px/1.8 system-ui,sans-serif;text-transform:uppercase;letter-spacing:2px;color:#c5af84;margin:0 0 12px}
.df-campaign[data-join-phase] .join-roster{list-style:none;padding:0;margin:0 0 24px;display:grid;gap:10px}
.df-campaign[data-join-phase] .join-participant{display:flex;align-items:center;gap:12px;padding:12px 0;border-top:1px solid #bacbd526}
.df-campaign[data-join-phase] .join-sigil{width:36px;height:36px;display:grid;place-items:center;flex-shrink:0;border:1px solid #bd996e80;border-radius:50%;color:#e8c890;background:#223143;font-size:14px}
.df-campaign[data-join-phase] .join-participant-content{min-width:0;flex:1}.df-campaign[data-join-phase] .join-participant-name{font-size:17px;overflow-wrap:anywhere}
.df-campaign[data-join-phase] .join-presence{font:10px/1.8 system-ui,sans-serif;color:#9eb5c5;overflow-wrap:anywhere}
.df-campaign[data-join-phase] .join-readiness{font:10px/1.7 system-ui,sans-serif;color:#cde2c6;max-width:95px;text-align:right;overflow-wrap:anywhere}
.df-campaign[data-join-phase] .join-empty{font:13px/1.8 system-ui,sans-serif;color:#b0beca;padding:15px 0}
.df-campaign[data-join-phase] .join-status{font:12px/1.8 system-ui,sans-serif;color:#c6d9e6;margin:20px 0 0;overflow-wrap:anywhere}.df-campaign[data-join-phase] .join-status:empty{display:none}
.df-campaign[data-join-phase] .join-status[data-feedback=rejected]{color:#f0bba3}
.df-campaign[data-join-phase] .join-pairing{border-top:1px solid #a59b8138;padding-top:18px;margin-top:24px}.df-campaign[data-join-phase] .join-pairing-fallback{font:10px/1.8 system-ui,sans-serif;color:#a6b6c5;margin:0}
.df-campaign[data-join-phase] .join-heading,.df-campaign[data-join-phase] .join-description{overflow-wrap:anywhere}
.df-campaign[data-join-phase] .narration>div{min-width:0}.df-campaign[data-join-phase] .narration blockquote{overflow-wrap:anywhere}
@media(max-width:1050px){.df-campaign[data-join-phase] .stage{grid-template-columns:minmax(0,1fr) 360px;gap:28px}.df-campaign[data-join-phase] .join-panel{padding:26px}.df-campaign[data-join-phase] h1{font-size:66px}}
@media(max-width:760px){.df-campaign[data-join-phase] .stage{display:flex;flex-direction:column;align-items:stretch;width:90vw;padding:76px 0 32px;min-height:0;gap:35px}.df-campaign[data-join-phase] h1{font-size:clamp(44px,11vw,74px);letter-spacing:-1px}.df-campaign[data-join-phase] .description{font-size:17px}.df-campaign[data-join-phase] .join-panel{padding:27px 22px}.df-campaign[data-join-phase] .lower{width:90vw}.df-campaign[data-join-phase] .scene-art{height:780px}.df-campaign[data-join-phase] .join-heading{font-size:29px}.df-campaign[data-join-phase] .topbar{gap:14px}}
"#;
