//! The TV's hand-tuned presentation values (camera close-ups, panel margins, speaker focus, the
//! thinking dots, idle fades), persisted in a stylesheet beside the play data and inlined into every
//! display page, so changing one needs no rebuild and survives restarts: edit the file and reload
//! the TV. The compiled stylesheets and the client's layout code only READ `--tune-*` variables,
//! each with its built-in default; this file is the one place that sets them.
//!
//! `DF_GAMEPLAY_DISPLAY_TUNING` names the file (the launcher points it at
//! `<play data>/display-tuning.css`). A missing file is created from [`TEMPLATE`], which documents
//! every knob at its default; an unset variable or an unreadable file leaves the defaults.

/// Every knob at its built-in default, documented. Deleting a line restores its default.
pub(super) const TEMPLATE: &str = r#"/* DungeonFlux TV tuning. Edit a value and reload the TV page; no rebuild or restart needed.
   Delete a line (or this file) to return to the built-in default. Numbers without units are plain
   factors; `u` is the stage's own unit (1% of the stage's short side). */
:root{
  /* Speaker close-up: the speaking character's head fills this share of the screen height... */
  --tune-speaker-head-share:.15;
  /* ...within this push-in range (1 = no zoom). */
  --tune-speaker-scale-min:1.2;
  --tune-speaker-scale-max:2.1;
  /* Where their head is brought to, in percent of the screen (50 = centre; smaller y = higher). */
  --tune-speaker-frame-x:50;
  --tune-speaker-frame-y:38;
  /* How long the push-in takes. */
  --tune-speaker-zoom-duration:4.5s;
  /* The narrator's slow pull-back: scale (below 1 = wider than rest) and its duration. */
  --tune-narrator-scale:.98;
  --tune-narrator-zoom-duration:8s;
  /* Handheld camera sway: 1 = the designed amount, 0 = still; and one sway cycle. */
  --tune-handheld-amount:1;
  --tune-handheld-period:9.7s;
  /* Speech bubbles and panels keep this share of the screen's short side clear of every edge. */
  --tune-panel-margin:.045;
  /* A hero's named panel is at least this share of the screen wide. */
  --tune-hero-panel-min-width:.24;
  /* Speaker focus: background blur (in u) and brightness behind the speaker, the warm rim's
     opacity, blur (in u) and colour, and how fast the focus fades in and out. */
  --tune-focus-blur:.18;
  --tune-focus-dim:.84;
  --tune-focus-rim-opacity:.16;
  --tune-focus-rim-blur:.55;
  --tune-focus-rim-color:#ffd9a3;
  --tune-focus-fade:1.6s;
  /* One cycle of an NPC's thinking dots. */
  --tune-thinking-dots:1.6s;
  /* The party strip fades this long after a scene settles. */
  --tune-party-fade-after:60s;
  /* The sound button stays this long after the mouse stops, then fades. */
  --tune-sound-button-idle:15.8s;
}
"#;

/// The display page's `<style>` element holding the tuning file, creating the file the first time.
/// Empty when no tuning file is configured or it cannot be read; the defaults then apply.
pub(super) async fn style_element() -> String {
    let Ok(path) = std::env::var("DF_GAMEPLAY_DISPLAY_TUNING") else {
        return String::new();
    };
    let path = std::path::PathBuf::from(path);
    if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        if let Err(error) = tokio::fs::write(&path, TEMPLATE).await {
            super::actor::record_audit_line(
                "gameplay.display_tuning",
                format!("DISPLAY_TUNING status=create_failed error={}", error.kind()),
            );
        }
    }
    match tokio::fs::read_to_string(&path).await {
        Ok(css) => style_for(&css),
        Err(error) => {
            super::actor::record_audit_line(
                "gameplay.display_tuning",
                format!("DISPLAY_TUNING status=unreadable error={}", error.kind()),
            );
            String::new()
        }
    }
}

/// The tuning rules as a `<style>` element; a `</` inside them cannot close it early.
fn style_for(css: &str) -> String {
    format!(
        "<style id=\"df-display-tuning\">{}</style>",
        css.replace("</", "<\\/")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names the client's layout code reads (`live_tv::tuning_number`), beside the stylesheets'.
    const CODE_KNOBS: &[&str] = &[
        "speaker-head-share",
        "speaker-scale-min",
        "speaker-scale-max",
        "speaker-frame-x",
        "speaker-frame-y",
        "panel-margin",
        "hero-panel-min-width",
    ];

    #[test]
    fn the_template_documents_every_tuning_knob_the_display_reads() {
        let sheets = [
            include_str!("../gameplay_browser/live_plate.css"),
            include_str!("../gameplay_browser/live_stage.css"),
            include_str!("../gameplay_browser/live_tv.css"),
        ]
        .concat();
        let mut read: Vec<String> = sheets
            .match_indices("var(--tune-")
            .map(|(at, _)| {
                sheets[at + "var(--tune-".len()..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect()
            })
            .collect();
        read.extend(CODE_KNOBS.iter().map(|name| (*name).to_owned()));
        read.sort();
        read.dedup();
        assert!(read.len() >= 20, "{read:?}");
        for name in read {
            assert!(
                TEMPLATE.contains(&format!("--tune-{name}:")),
                "{name} is read by the display but missing from the tuning template"
            );
        }
    }

    #[test]
    fn tuning_rules_cannot_close_their_style_element() {
        let style = style_for(":root{--tune-focus-dim:.8}</style><script>");
        assert!(style.starts_with("<style id=\"df-display-tuning\">"));
        assert_eq!(style.matches("</style>").count(), 1);
    }
}
