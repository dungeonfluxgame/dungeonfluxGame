//! The section label's title and sub-line.

use crate::kit::section_label::SectionLabel;

#[test]
fn a_label_reads_and_a_blank_title_is_not_drawn() {
    let label: SectionLabel =
        serde_json::from_str(r#"{"title":"Journal / Quests","subtitle":" ","icon":"book"}"#)
            .expect("a label reads");
    assert_eq!(label.title(), Some("Journal / Quests"));
    assert_eq!(label.subtitle(), None);
    let blank: SectionLabel = serde_json::from_str(r#"{"title":""}"#).expect("a label reads");
    assert_eq!(blank.title(), None);
}
