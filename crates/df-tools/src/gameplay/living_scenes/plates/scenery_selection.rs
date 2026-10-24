//! Exact restoration of a saved public portrait setting; no semantic classification.
pub(crate) fn saved_option<'a>(
    options: &'a [df_protocol::common::JourneyOption],
    saved: &str,
) -> Option<&'a str> {
    options
        .iter()
        .find(|option| option.option_id != "custom" && option.description == saved)
        .map(|option| option.option_id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_scenery_restores_exact_presets_and_preserves_unmatched_custom_text() {
        let options = vec![
            df_protocol::common::JourneyOption {
                option_id: "studio".to_owned(),
                label: "Studio backdrop".to_owned(),
                description: "A plain studio backdrop.".to_owned(),
                ..Default::default()
            },
            df_protocol::common::JourneyOption {
                option_id: "custom".to_owned(),
                ..Default::default()
            },
        ];
        assert_eq!(
            saved_option(&options, "A plain studio backdrop."),
            Some("studio")
        );
        assert_eq!(saved_option(&options, "A plain studio backdrop. "), None);
        assert_eq!(saved_option(&options, "A moonlit garden."), None);
        assert_eq!(saved_option(&options, ""), None);
    }
}
