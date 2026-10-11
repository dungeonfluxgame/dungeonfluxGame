/// Closed shared palette inherited from the existing DungeonFlux visual foundation.
/// Values are client-owned; server content cannot inject style declarations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThemeToken {
    Background,
    Surface,
    Text,
    SecondaryText,
    StoryAccent,
    EngineAccent,
}

impl ThemeToken {
    pub const fn css_value(self) -> &'static str {
        match self {
            Self::Background => "#08111f",
            Self::Surface => "#0d1a2e",
            Self::Text => "#f1e7d0",
            Self::SecondaryText => "#b7c0cf",
            Self::StoryAccent => "#e6b45e",
            Self::EngineAccent => "#7fd0f2",
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn stylesheet() -> String {
    format!(
        r#".df-ui-root {{
--df-background: {}; --df-surface: {}; --df-text: {};
--df-secondary: {}; --df-story: {}; --df-engine: {};
--df-gap: 16px; --df-radius: 4px; --df-target: 44px;
box-sizing: border-box; color: var(--df-text); background: var(--df-background);
font: 1rem/1.5 system-ui, sans-serif; padding: clamp(16px, 4vw, 48px);
width: 100%; min-width: 0;
}}
.df-ui-root *, .df-ui-root *::before, .df-ui-root *::after {{ box-sizing: border-box; }}
.df-ui-root .df-ui-content, .df-ui-root .df-ui-stack {{
display: grid; gap: var(--df-gap); min-width: 0; grid-template-columns: minmax(0, 1fr);
}}
.df-ui-root .df-ui-panel {{
background: var(--df-surface); border: 1px solid var(--df-secondary);
border-radius: var(--df-radius); padding: var(--df-gap); min-width: 0;
}}
.df-ui-root h2 {{ margin: 0 0 var(--df-gap); font: 600 1.5rem/1.2 Georgia, serif; }}
.df-ui-root h2, .df-ui-root p, .df-ui-root label, .df-ui-root button {{ overflow-wrap: anywhere; }}
.df-ui-root .df-ui-action, .df-ui-root .df-ui-input {{
font: inherit; color: var(--df-text); background: var(--df-background);
border: 1px solid var(--df-engine); border-radius: var(--df-radius);
min-height: var(--df-target); min-width: 0; max-width: 100%; padding: 10px 16px;
}}
.df-ui-root .df-ui-action {{ min-width: var(--df-target); cursor: pointer; touch-action: manipulation; }}
.df-ui-root .df-ui-action:disabled {{ cursor: default; color: var(--df-secondary); }}
.df-ui-root .df-ui-field {{ display: grid; gap: 8px; min-width: 0; }}
.df-ui-root :focus-visible {{ outline: 3px solid var(--df-story); outline-offset: 3px; }}
@media (min-width: 60rem) {{
.df-ui-root[data-df-layout="shared-display"] .df-ui-content {{
grid-template-columns: repeat(2, minmax(0, 1fr)); align-items: start;
}}
}}
/* Explicit reduction adds to the OS preference; it never forces motion on. */
.df-ui-root[data-df-reduced-motion="true"],
.df-ui-root[data-df-reduced-motion="true"]::before,
.df-ui-root[data-df-reduced-motion="true"]::after,
.df-ui-root[data-df-reduced-motion="true"] *,
.df-ui-root[data-df-reduced-motion="true"] *::before,
.df-ui-root[data-df-reduced-motion="true"] *::after {{
animation: none !important; transition: none !important; scroll-behavior: auto !important;
}}
@media (prefers-reduced-motion: reduce) {{
.df-ui-root, .df-ui-root::before, .df-ui-root::after,
.df-ui-root *, .df-ui-root *::before, .df-ui-root *::after {{
animation: none !important; transition: none !important; scroll-behavior: auto !important;
}}
}}
"#,
        ThemeToken::Background.css_value(),
        ThemeToken::Surface.css_value(),
        ThemeToken::Text.css_value(),
        ThemeToken::SecondaryText.css_value(),
        ThemeToken::StoryAccent.css_value(),
        ThemeToken::EngineAccent.css_value(),
    )
}
