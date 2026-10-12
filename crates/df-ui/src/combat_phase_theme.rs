pub(crate) const STYLES: &str = r#"
.df-combat{--gold:#e0bd7f;--muted:#bfb8aa;position:relative;isolation:isolate;min-height:100svh;background:radial-gradient(ellipse at 20% 15%,#42301c,transparent 65%),#111512;color:#f4eddf;font:16px/1.5 Georgia,'Times New Roman',serif;overflow:hidden}
.df-combat *{box-sizing:border-box;min-width:0}
.df-combat [hidden]{display:none!important}
.df-combat p,.df-combat h1,.df-combat h2,.df-combat h3,.df-combat button,.df-combat dd{overflow-wrap:anywhere}
.df-combat .combat-art-host{position:absolute;inset:0;z-index:-3;pointer-events:none}
.df-combat .combat-art{position:absolute;inset:0;width:100%;height:100%;object-fit:cover;object-position:50% 46%;color:var(--muted)}
.df-combat:before{content:'';position:absolute;inset:0;z-index:-2;pointer-events:none;background:linear-gradient(180deg,#0b101453,transparent 24%,#1014111a 46%,#111512e6 76%,#111512),linear-gradient(90deg,#11151280,transparent 55%)}
.df-combat .combat-topbar{display:flex;align-items:center;gap:24px;padding:23px 4vw;background:linear-gradient(#101310b0,transparent)}
.df-combat .combat-brand{font-size:27px;letter-spacing:-1px;color:#f0ce92;text-shadow:0 2px 16px #000}
.df-combat .combat-location{flex:1;margin:0;font:11px/1.6 system-ui,sans-serif;text-transform:uppercase;letter-spacing:2.5px;color:#e9dfcc}
.df-combat .combat-notice{font:10px/1.5 system-ui,sans-serif;letter-spacing:1.3px;text-transform:uppercase;color:#e0ceb0;padding:7px 11px;border:1px solid #d9b77a55;border-radius:3px;background:#1216118c}
.df-combat .combat-stage{width:min(1680px,92%);margin:auto;display:grid;grid-template-columns:minmax(0,1fr) 292px;gap:5vw;align-items:end;min-height:calc(100svh - 370px);padding:160px 0 35px}
.df-combat .combat-story{padding:0 0 8px}
.df-combat .combat-overline{font:11px/1.7 system-ui,sans-serif;letter-spacing:2.5px;text-transform:uppercase;color:var(--gold);margin:0 0 13px}
.df-combat h1{font-weight:400;font-size:clamp(42px,4.6vw,72px);line-height:1.06;letter-spacing:-1.8px;text-shadow:0 3px 28px #000;margin:0 0 17px;max-width:650px;text-wrap:balance}
.df-combat .combat-turn-badge{display:table;margin:0 0 14px;padding:5px 10px;border:1px solid #d9b77a73;border-radius:3px;background:#182019d9;color:#f0d19c;font:11px/1.6 system-ui,sans-serif;letter-spacing:1px;text-transform:uppercase}
.df-combat .combat-narration{font-size:18px;line-height:1.65;color:#eee5d5;max-width:580px;margin:0;text-shadow:0 2px 8px #000}
.df-combat .combat-initiative{padding:19px 20px;background:linear-gradient(145deg,#22291fed,#141a16f2);border:1px solid #c7a86b66;border-radius:9px;box-shadow:0 18px 48px #0006}
.df-combat .combat-actors,.df-combat .combat-rolls{list-style:none;margin:0;padding:0}
.df-combat .combat-actor{display:flex;gap:12px;align-items:center;padding:12px 0;border-top:1px solid #d9b77a2e}
.df-combat .combat-actor:first-child{border-top:0}
.df-combat .combat-actor[data-active=true]{background:linear-gradient(90deg,#d3a25920,transparent);border-radius:4px;box-shadow:inset 3px 0 #e0b877;padding-left:10px;padding-right:6px}
.df-combat .combat-actor[data-active=true] h3{color:#ffe0a3}
.df-combat .combat-actor-body{flex:1}
.df-combat .combat-initiative-value{min-width:38px;max-width:38%;min-height:38px;padding:8px;overflow-wrap:anywhere;text-align:center;flex:0 1 auto;display:grid;place-items:center;background:#101611;border:1px solid #b5aa874d;border-radius:50%;color:#d2c7a9;font:18px/1.1 Georgia,serif}
.df-combat .combat-actor[data-active=true] .combat-initiative-value{color:#ffe0a3;border-color:#e0b877;background:#493b22;box-shadow:0 0 16px #bb853b26}
.df-combat h3{font-weight:400;margin:0;font-size:19px;line-height:1.3}
.df-combat .combat-muted{font:12px/1.5 system-ui,sans-serif;color:var(--muted);margin:4px 0 0}
.df-combat .combat-resources{display:grid;grid-template-columns:auto 1fr;column-gap:12px;row-gap:3px;margin:7px 0 0;font:11px/1.5 system-ui,sans-serif}
.df-combat .combat-resources dt{color:#b8ad96}
.df-combat .combat-resources dd{margin:0;color:#f0debb;text-align:right}
.df-combat .combat-lower{width:min(1680px,92%);margin:auto;display:grid;grid-template-columns:minmax(0,1.7fr) minmax(250px,1fr);gap:30px;align-items:start;padding:0 0 25px}
.df-combat .combat-action-rail{padding:20px 22px;background:linear-gradient(135deg,#1d241ef5,#151b16e8);border:1px solid #c8a76b59;border-radius:9px;box-shadow:0 12px 35px #0003}
.df-combat .combat-roll-panel{padding:20px 24px;border-left:2px solid #d5af70;background:linear-gradient(90deg,#171e19e8,#171e1966);border-radius:0 8px 8px 0}
.df-combat h2{font-weight:400;font-size:24px;line-height:1.3;margin:0 0 17px;color:#f2dfbb}
.df-combat .combat-offers{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:12px}
.df-combat .combat-offer{padding:0;background:none;border:0}
.df-combat button{width:100%;min-height:48px;border:1px solid #c19d6066;background:#303329;color:#f4e5ca;border-radius:6px;padding:12px 15px;font:13px/1.5 system-ui,sans-serif;cursor:pointer;touch-action:manipulation;box-shadow:inset 0 1px 0 #fff1;transition:background-color .15s,border-color .15s,box-shadow .15s}
.df-combat .combat-offer[data-kind=action] button:enabled{background:linear-gradient(135deg,#f0d5a4,#c6a05e);color:#211b11;border-color:#f0d195;font-weight:650;box-shadow:0 5px 20px #9f6b3026,inset 0 1px 0 #fff6}
.df-combat .combat-offer[data-kind=roll] button:before{content:'◇';margin-right:8px;color:#f0cb8c}
.df-combat .combat-offer[data-kind=reaction] button:enabled{border-color:#91b4af;color:#d5eeea;background:#253c37}
.df-combat button:hover:enabled{background:#465040;border-color:#f0c985}
.df-combat .combat-offer[data-kind=action] button:hover:enabled{background:#f4ddb5}
.df-combat button:active:enabled{box-shadow:inset 0 0 0 2px #ffe3ad,0 0 18px #d9b77a33}
.df-combat button:disabled{color:#b5b0a6;background:#191e19;cursor:default;border-color:#8c8c704d;box-shadow:none}
.df-combat :focus-visible{outline:3px solid #ffe0a3;outline-offset:4px}
.df-combat button[aria-busy=true]{border:1px dashed #d9b77a;color:#e5ca9c;background:repeating-linear-gradient(135deg,#2d3024 0 9px,#262b22 9px 18px);cursor:wait}
.df-combat .combat-offer p{margin:8px 2px 0;line-height:1.6}
.df-combat .combat-reaction{margin-top:17px;padding:14px 16px;border:1px solid #99beb76e;border-left:3px solid #a3cfc6;background:#28423a59;border-radius:5px}
.df-combat .combat-reaction h3{color:#dcf1ea;font-size:18px}
.df-combat .combat-timing{margin:6px 0;color:#c3e2d7;font:12px/1.5 system-ui,sans-serif}
.df-combat .combat-draft{margin-top:17px}
.df-combat .df-ui-field{display:grid;gap:5px}
.df-combat .df-ui-field label{display:grid;gap:7px;font:12px/1.6 system-ui,sans-serif;color:#dfceb0}
.df-combat input{width:100%;min-height:46px;color:#f4eddf;background:#101610;border:1px solid #a3916b73;border-radius:6px;font:14px/1.5 system-ui,sans-serif;padding:11px 12px}
.df-combat input[aria-invalid=true]{border-color:#e6ab95}
.df-combat .df-ui-feedback,.df-combat .combat-feedback{font:12px/1.6 system-ui,sans-serif;margin:9px 0 0;color:#f0d19e}
.df-combat .df-ui-feedback:empty,.df-combat .combat-feedback:empty,.df-combat .combat-muted:empty{display:none}
.df-combat .combat-feedback[data-state=refused]{color:#ffc4ad;border-left:2px solid #e6ab95;padding-left:10px}
.df-combat .combat-feedback[data-state=pending]{padding:9px 11px;background:#d5ac5617;border:1px dashed #d5ac5680;border-radius:4px}
.df-combat .combat-roll{display:grid;grid-template-columns:66px 1fr;column-gap:16px;padding:0 0 15px;margin:0 0 15px;border-bottom:1px solid #b7a48240}
.df-combat .combat-roll:last-child{border-bottom:0;margin:0;padding-bottom:0}
.df-combat .combat-roll h3{grid-column:2;font:13px/1.5 system-ui,sans-serif;color:#f0dfbd;letter-spacing:.3px}
.df-combat .combat-roll-value{grid-column:1;grid-row:1 / span 4;display:grid;place-items:center;align-self:center;aspect-ratio:1;margin:0;border:1px solid #e4c084;clip-path:polygon(25% 0,75% 0,100% 25%,100% 75%,75% 100%,25% 100%,0 75%,0 25%);background:linear-gradient(140deg,#86633373,#171c15);background-size:cover;background-position:center;background-repeat:no-repeat;font:36px/1 Georgia,serif;color:#ffe0a3;text-shadow:0 2px 5px #000}
.df-combat .combat-roll-explanation{grid-column:2;font:12px/1.6 system-ui,sans-serif;color:#e1d6bf;margin:5px 0}
.df-combat .combat-roll-source{grid-column:2;font:11px/1.6 system-ui,sans-serif;color:#b8ad97;margin:3px 0 0}
.df-combat .combat-hp{position:relative;height:18px;margin:8px 0 0;border:1px solid #d9b77a59;border-radius:9px;background:#0a0e0b;overflow:hidden}
.df-combat .combat-hp-fill{position:absolute;inset:0 auto 0 0;width:var(--hp,0%);background:linear-gradient(90deg,#3f8553,#8fd18d);transition:width .45s ease-out}
.df-combat .combat-hp[data-state=low] .combat-hp-fill{background:linear-gradient(90deg,#92392a,#e0784f)}
.df-combat .combat-hp[data-state=down] .combat-hp-fill{width:0}
.df-combat .combat-hp-text{position:relative;display:block;text-align:center;font:700 11px/18px system-ui,sans-serif;color:#fff;text-shadow:0 1px 3px #000}
.df-combat .combat-roll-verdict{grid-column:2;justify-self:start;margin:6px 0 2px;padding:3px 24px;font:700 13px/1.7 system-ui,sans-serif;letter-spacing:2px;text-transform:uppercase;color:#fff4d6;background:#6b5a35;clip-path:polygon(0 0,100% 0,calc(100% - 11px) 50%,100% 100%,0 100%,11px 50%)}
.df-combat .combat-roll-verdict[data-tone=success]{background:linear-gradient(#2f8150,#1d5a37);color:#dcffe8}
.df-combat .combat-roll-verdict[data-tone=failure]{background:linear-gradient(#963a2e,#65261f);color:#ffe3dc}
.df-combat .combat-roll:first-child{grid-template-columns:104px 1fr}
.df-combat .combat-roll:first-child .combat-roll-value{font-size:54px;box-shadow:0 0 28px #e0b87766}
/* FIGHT-001: the Latest-roll badge is the small die: a natural 20 reads bright gold, a
natural 1 ember, ordinary faces keep the parchment number. */
.df-combat .combat-roll-value[data-nat="20"]{color:#ffedbe;border-color:#ffe9b0;box-shadow:0 0 28px #ffe9b080}
.df-combat .combat-roll-value[data-nat="1"]{color:#eda37e;border-color:#d96a4b}
.df-combat .combat-roll:first-child h3{font-size:15px}
.df-combat .combat-offer[data-kind=choice] button{display:flex;justify-content:space-between;align-items:center;gap:12px;text-align:left;background:#1a211b;border-color:#c19d6055}
.df-combat .combat-offer[data-kind=choice] button:after{content:'Off';flex:none;min-width:52px;padding:2px 10px;border:1px solid #b5aa8799;border-radius:12px;font:700 11px/1.7 system-ui,sans-serif;letter-spacing:1px;text-align:center;text-transform:uppercase;color:#cfc6b0}
.df-combat .combat-offer[data-kind=choice][data-selected=true] button{background:#3a3422;border-color:#f0d195}
.df-combat .combat-offer[data-kind=choice][data-selected=true] button:after{content:'On';background:#e0bd7f;border-color:#f0d195;color:#211b11}
.df-combat .combat-connection{text-align:center;color:#b9ae96;font:11px/1.7 system-ui,sans-serif;padding:0 4vw 17px}
.df-combat .combat-fixture-tools{width:min(1680px,92%);margin:0 auto 30px;border-top:1px solid #ad925d55;padding-top:20px}
.df-combat .combat-fixture-tools summary{min-height:44px;color:#dfb974;cursor:pointer}
.df-combat .combat-fixture-tools nav{display:flex;flex-wrap:wrap;gap:8px}
.df-combat .combat-fixture-tools button{width:auto}
.df-combat .combat-fixture-status{font:12px/1.6 system-ui,sans-serif;color:#b6bec6}
[data-combat-client=shared-display] .df-combat .combat-stage{min-height:64svh;align-items:end;padding-top:180px}
[data-combat-client=shared-display] .df-combat .combat-story{max-width:660px}
[data-combat-client=shared-display] .df-combat .combat-initiative{align-self:start;margin-top:20px}
[data-combat-client=shared-display] .df-combat .combat-lower{grid-template-columns:minmax(0,1fr) minmax(280px,.65fr)}
@media(min-width:1600px){.df-combat .combat-stage{padding-top:220px}.df-combat .combat-lower{gap:42px}[data-combat-client=shared-display] .df-combat .combat-stage{padding-top:100px}[data-combat-client=shared-display] .df-combat .combat-narration{font-size:21px}}
@media(max-width:1000px){.df-combat .combat-stage{grid-template-columns:1fr 250px;gap:24px;min-height:480px;padding-top:110px}.df-combat h1{font-size:52px}.df-combat .combat-lower{grid-template-columns:1.3fr 1fr;gap:20px}.df-combat .combat-offers{grid-template-columns:1fr}.df-combat .combat-offer p{margin:5px 2px 0}[data-combat-client=shared-display] .df-combat .combat-stage{padding-top:110px}}
@media(max-width:700px){
.df-combat .combat-topbar{padding:17px 5vw 0;gap:7px 13px;flex-wrap:wrap}
.df-combat .combat-brand{font-size:22px;flex:1}
.df-combat .combat-notice{font-size:8px;letter-spacing:.8px;max-width:116px;padding:5px 7px}
.df-combat .combat-location{order:3;flex-basis:100%;font-size:9px;letter-spacing:2px}
.df-combat .combat-art{height:460px;object-position:46% center}
.df-combat:before{background:linear-gradient(180deg,#11151266,transparent 17%,#111512b3 310px,#111512 460px)}
.df-combat .combat-stage,[data-combat-client=shared-display] .df-combat .combat-stage{display:flex;flex-direction:column;align-items:stretch;gap:18px;width:90%;min-height:0;padding:155px 0 19px}
.df-combat .combat-story{padding:0}
.df-combat .combat-overline{font-size:10px;letter-spacing:2px;margin-bottom:9px}
.df-combat h1{font-size:37px;letter-spacing:-1px;margin-bottom:12px}
.df-combat .combat-narration,[data-combat-client=shared-display] .df-combat .combat-narration{font-size:14px;line-height:1.65}
.df-combat .combat-initiative{padding:14px 15px;border-radius:7px;box-shadow:0 8px 22px #0002}
.df-combat .combat-initiative .combat-overline{margin-bottom:7px}
.df-combat .combat-actor{padding:9px 0;gap:11px}
.df-combat .combat-actor:first-child{padding-top:3px}
.df-combat .combat-actor:last-child{padding-bottom:3px}
.df-combat h3{font-size:17px}
.df-combat .combat-initiative-value{min-width:35px;min-height:35px;font-size:17px}
.df-combat .combat-resources{margin-top:4px;font-size:11px}
.df-combat .combat-lower,[data-combat-client=shared-display] .df-combat .combat-lower{width:90%;grid-template-columns:1fr;gap:16px;padding-bottom:18px}
.df-combat .combat-action-rail{padding:16px 15px}
.df-combat h2{font-size:22px;margin-bottom:12px}
.df-combat .combat-offers{grid-template-columns:1fr 1fr;gap:11px}
.df-combat .combat-offer[data-kind=action]{grid-column:1 / -1}
.df-combat button{min-height:48px;padding:11px 12px;font-size:13px}
.df-combat .combat-offer[data-kind=action] button{font-size:15px}
.df-combat .combat-offer p{font-size:11px;line-height:1.55;margin-top:6px}
.df-combat .combat-draft{margin-top:14px}
.df-combat .combat-roll-panel{padding:15px 16px}
.df-combat .combat-roll-value{font-size:31px}
.df-combat .combat-roll{grid-template-columns:56px 1fr;column-gap:13px}
.df-combat .combat-roll-panel h2{font-size:19px}
.df-combat .combat-connection{font-size:10px;padding-bottom:15px}
.df-combat .combat-fixture-tools{width:90%}
.df-combat .combat-fixture-tools nav{display:grid;grid-template-columns:1fr 1fr}
.df-combat .combat-fixture-tools button{width:100%;font-size:12px}
[data-combat-client=player] .df-combat{display:flex;flex-direction:column}
[data-combat-client=player] .df-combat .combat-stage{display:contents}
[data-combat-client=player] .df-combat .combat-topbar{order:0}
[data-combat-client=player] .df-combat .combat-story{order:1;width:90%;margin:0 auto;padding:155px 0 22px}
[data-combat-client=player] .df-combat .combat-lower{order:2;margin:0 auto}
[data-combat-client=player] .df-combat .combat-initiative{order:3;width:90%;margin:0 auto 18px}
[data-combat-client=player] .df-combat .combat-connection{order:4}
[data-combat-client=player] .df-combat .combat-fixture-tools{order:5}
}
@media(max-width:360px){.df-combat .combat-brand{font-size:20px}.df-combat .combat-notice{max-width:99px;font-size:7px}.df-combat h1{font-size:33px}.df-combat .combat-narration{font-size:13px}.df-combat .combat-action-rail{padding:15px 12px}.df-combat .combat-offers{gap:9px}}
"#;
