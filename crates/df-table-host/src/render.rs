//! The plain debugging view of the canvas: SVG, tables of elements and kinds, and the event log.
//!
//! The page refreshes itself with a `<meta>` tag, not a script, because the project keeps
//! application logic out of JavaScript. That reloads the whole page every second, which resets
//! scroll and text selection; the live view for design work is the playground (`df-playground`),
//! so this one stays simple (decision of 2026-11-02). It escapes every piece of text it prints and
//! never reads state: it only draws the copy it is given.

use df_table::element::{CANVAS_HEIGHT_UNITS, CANVAS_WIDTH_UNITS};
use df_table::kind::Shape;

use crate::event_log::LoggedLine;
use crate::view::{CanvasView, ElementView};

/// Seconds between automatic refreshes of the page.
pub const REFRESH_SECONDS: u32 = 1;
/// Most characters of a label drawn on the canvas; longer labels are cut with an ellipsis.
pub const MAXIMUM_LABEL_CHARACTERS: usize = 24;
/// Distance between grid lines, in canvas units.
const GRID_STEP_UNITS: f32 = 10.0;
const DEFAULT_FILL: &str = "#8a94a6";

const STYLE: &str = "\
:root { color-scheme: dark; }
body { margin: 0; background: #11151c; color: #dfe6f0; font: 14px/1.45 system-ui, sans-serif; }
header { padding: 12px 20px; border-bottom: 1px solid #2a3140; }
h1 { margin: 0; font-size: 18px; }
h2 { margin: 18px 0 6px; font-size: 14px; color: #9fb0c8; }
header p { margin: 2px 0 0; color: #9fb0c8; }
main { display: grid; grid-template-columns: minmax(0, 2fr) minmax(280px, 1fr); gap: 20px; padding: 16px 20px; }
@media (max-width: 900px) { main { grid-template-columns: 1fr; } }
svg { width: 100%; height: auto; background: #181e29; border: 1px solid #2a3140; border-radius: 6px; }
table { border-collapse: collapse; width: 100%; }
th, td { text-align: left; padding: 3px 8px 3px 0; border-bottom: 1px solid #232a38; vertical-align: top; }
code, pre { font: 12px/1.4 ui-monospace, Consolas, monospace; }
pre { margin: 0; padding: 10px; background: #0d1117; border-radius: 6px; max-height: 260px; overflow: auto; }
.problem { color: #ff8585; }
.empty { color: #9fb0c8; }
section.log { padding: 0 20px 20px; }
";

/// Escapes text for HTML and SVG, element content and attribute values alike.
pub fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Cuts a label to `MAXIMUM_LABEL_CHARACTERS`, ending with an ellipsis when it was longer.
pub fn shorten_label(label: &str) -> String {
    let mut characters = label.chars();
    let head: String = characters
        .by_ref()
        .take(MAXIMUM_LABEL_CHARACTERS.saturating_sub(1))
        .collect();
    if label.chars().count() > MAXIMUM_LABEL_CHARACTERS {
        format!("{head}\u{2026}")
    } else {
        label.to_owned()
    }
}

fn grid() -> String {
    let mut lines = String::new();
    let mut at = GRID_STEP_UNITS;
    while at < CANVAS_WIDTH_UNITS {
        lines.push_str(&format!(
            r##"<line x1="{at}" y1="0" x2="{at}" y2="{CANVAS_HEIGHT_UNITS}" stroke="#222a38" stroke-width="0.15"/>"##
        ));
        at += GRID_STEP_UNITS;
    }
    at = GRID_STEP_UNITS;
    while at < CANVAS_HEIGHT_UNITS {
        lines.push_str(&format!(
            r##"<line x1="0" y1="{at}" x2="{CANVAS_WIDTH_UNITS}" y2="{at}" stroke="#222a38" stroke-width="0.15"/>"##
        ));
        at += GRID_STEP_UNITS;
    }
    lines
}

fn element_svg(element: &ElementView) -> String {
    let fill = element.color.as_deref().unwrap_or(DEFAULT_FILL);
    let stroke = if element.problem.is_some() {
        r##"stroke="#ff5a5a" stroke-width="0.4" stroke-dasharray="1 0.6""##
    } else {
        r##"stroke="#0d1117" stroke-width="0.25""##
    };
    let tooltip = escape(&format!("{} ({})", element.id, element.kind));
    let shape = match element.shape {
        Shape::Circle => format!(
            r#"<circle cx="{:.2}" cy="{:.2}" r="{:.2}" fill="{}" {stroke}/>"#,
            element.x,
            element.y,
            element.width_units / 2.0,
            escape(fill)
        ),
        Shape::Rect => format!(
            r#"<rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" rx="0.6" fill="{}" {stroke}/>"#,
            element.x - element.width_units / 2.0,
            element.y - element.height_units / 2.0,
            element.width_units,
            element.height_units,
            escape(fill)
        ),
    };
    // The label sits under the element but is kept on the canvas near the bottom edge.
    let label_y = (element.y + element.height_units.max(element.width_units) / 2.0 + 2.4)
        .min(CANVAS_HEIGHT_UNITS - 0.5);
    format!(
        r##"<g><title>{tooltip}</title>{shape}<text x="{:.2}" y="{label_y:.2}" font-size="1.9" text-anchor="middle" fill="#dfe6f0">{}</text></g>"##,
        element.x,
        escape(&shorten_label(&element.label))
    )
}

fn canvas_svg(view: &CanvasView) -> String {
    let mut svg = format!(
        r#"<svg viewBox="0 0 {CANVAS_WIDTH_UNITS} {CANVAS_HEIGHT_UNITS}" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="The canvas with {} elements">"#,
        view.elements.len()
    );
    svg.push_str(&grid());
    for element in &view.elements {
        svg.push_str(&element_svg(element));
    }
    svg.push_str("</svg>");
    svg
}

fn element_table(view: &CanvasView) -> String {
    if view.elements.is_empty() {
        return "<p class=\"empty\">Nothing here yet. Spawn something with \
                <code>df-table dev spawn &lt;kind&gt; &lt;id&gt;</code> (see <code>df-table dev kinds</code>).</p>"
            .to_owned();
    }
    let mut table =
        String::from("<table><tr><th>Element</th><th>Kind</th><th>At</th><th>Fields</th></tr>");
    for element in &view.elements {
        let fields = element
            .fields
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join(" ");
        let problem = element
            .problem
            .as_deref()
            .map_or_else(String::new, |problem| {
                format!(r#"<div class="problem">{}</div>"#, escape(problem))
            });
        table.push_str(&format!(
            "<tr><td><code>{}</code></td><td>{}</td><td>{:.1}, {:.1}</td><td><code>{}</code>{problem}</td></tr>",
            escape(&element.id),
            escape(&element.kind),
            element.x,
            element.y,
            escape(&fields)
        ));
    }
    table.push_str("</table>");
    table
}

fn kind_table(view: &CanvasView) -> String {
    if view.kinds.is_empty() {
        return "<p class=\"empty\">No kinds are loaded. Add a file to the kind directory and run \
                <code>df-table dev reload</code>.</p>"
            .to_owned();
    }
    let mut table = String::from("<table><tr><th>Kind</th><th>Shape</th><th>Fields</th></tr>");
    for kind in &view.kinds {
        let fields = kind
            .fields
            .iter()
            .map(|field| {
                if field.required {
                    format!("{}*", field.name)
                } else {
                    field.name.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        table.push_str(&format!(
            "<tr><td><code>{}</code></td><td>{:?}</td><td>{}</td></tr>",
            escape(&kind.name),
            kind.shape,
            escape(&fields)
        ));
    }
    table.push_str("</table>");
    table
}

/// The log as text, newest first. `events` arrive oldest first.
fn log_text(events: &[LoggedLine]) -> String {
    if events.is_empty() {
        return "Nothing has happened yet.".to_owned();
    }
    events
        .iter()
        .rev()
        .map(|line| format!("{:>4}  {}", line.sequence, escape(&line.text)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The whole page for one copy of the canvas and the newest event lines (oldest first).
pub fn page(view: &CanvasView, events: &[LoggedLine]) -> String {
    let head = format!(
        "<meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta http-equiv=\"refresh\" content=\"{REFRESH_SECONDS}\">\
         <title>df-table canvas</title><style>{STYLE}</style>"
    );
    let header = format!(
        "<header><h1>Blank canvas</h1><p>revision {} with {} elements and {} kinds</p></header>",
        view.revision,
        view.elements.len(),
        view.kinds.len()
    );
    let main = format!(
        "<main><section>{}</section><aside><h2>Elements</h2>{}\
         <h2>Kinds (a star marks a required field)</h2>{}</aside></main>",
        canvas_svg(view),
        element_table(view),
        kind_table(view)
    );
    let log = format!(
        "<section class=\"log\"><h2>Newest events first</h2><pre>{}</pre></section>",
        log_text(events)
    );
    format!(
        "<!doctype html><html lang=\"en\"><head>{head}</head><body>{header}{main}{log}</body></html>"
    )
}
