//! Pure argument parsing for the development control surface: cell names,
//! action names and hero references. No I/O, no game state; unit-tested here.

use df_protocol::common as rpc;

/// A tactical grid cell named `c<col>r<row>`, both 1-based as shown on the
/// phone (`Column <col>, row <row>`), converted to the 0-based option id
/// `<col-1>-<row-1>` the engine offers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Cell {
    /// 0-based column used in the offered option id.
    pub col0: u32,
    /// 0-based row used in the offered option id.
    pub row0: u32,
}

impl Cell {
    /// The option id the engine offers for this cell (`<col0>-<row0>`).
    pub(super) fn option_id(self) -> String {
        format!("{}-{}", self.col0, self.row0)
    }
}

/// Parses `c5r6` (case-insensitive, 1-based) into a [`Cell`]. Refuses anything
/// else, including 0-based or out-of-grid values.
pub(super) fn parse_cell(text: &str) -> Result<Cell, String> {
    let lower = text.to_ascii_lowercase();
    let (col, row) = lower
        .strip_prefix('c')
        .and_then(|rest| rest.split_once('r'))
        .ok_or_else(|| format!("cell {text:?} must look like c5r6"))?;
    let col: u32 = col
        .parse()
        .map_err(|_| format!("cell {text:?} must look like c5r6"))?;
    let row: u32 = row
        .parse()
        .map_err(|_| format!("cell {text:?} must look like c5r6"))?;
    if !(1..=32).contains(&col) || !(1..=32).contains(&row) {
        return Err(format!("cell {text:?} is outside the 1..=32 grid"));
    }
    Ok(Cell {
        col0: col - 1,
        row0: row - 1,
    })
}

/// The explicit game actions the control surface plays through the phone path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PlayAction {
    Create,
    BeginStory,
    AskCourier,
    EscortCourier,
    DefendCourier,
    Move,
    Attack,
    SecondWind,
    StabiliseWithHealersKit,
    StabiliseMedicine,
    HelpAttack,
    EndTurn,
    OpportunityAttack,
    DeclineReaction,
    Dash,
    Dodge,
    Disengage,
    Stand,
    PickUpWeapon,
    ShortRest,
    ChooseInn,
    ResolveErrand,
    NewAdventure,
    Search,
    TableSetting,
    Choose,
}

/// Maps one explicit action word to its engine kind. There is no free-text
/// action: an unknown word is refused rather than guessed.
pub(super) fn parse_action(text: &str) -> Result<PlayAction, String> {
    Ok(match text.to_ascii_lowercase().as_str() {
        "create" => PlayAction::Create,
        "begin" | "begin-story" => PlayAction::BeginStory,
        "ask" | "ask-courier" => PlayAction::AskCourier,
        "escort" | "escort-courier" => PlayAction::EscortCourier,
        "defend" | "defend-courier" => PlayAction::DefendCourier,
        "move" => PlayAction::Move,
        "attack" => PlayAction::Attack,
        "second-wind" => PlayAction::SecondWind,
        "stabilise-kit" | "stabilise-with-healers-kit" => PlayAction::StabiliseWithHealersKit,
        "stabilise-medicine" => PlayAction::StabiliseMedicine,
        "help-attack" => PlayAction::HelpAttack,
        "end-turn" => PlayAction::EndTurn,
        "opportunity-attack" => PlayAction::OpportunityAttack,
        "decline-reaction" => PlayAction::DeclineReaction,
        "dash" => PlayAction::Dash,
        "dodge" => PlayAction::Dodge,
        "disengage" => PlayAction::Disengage,
        "stand" => PlayAction::Stand,
        "pick-up" | "pick-up-weapon" => PlayAction::PickUpWeapon,
        "short-rest" => PlayAction::ShortRest,
        "inn" | "choose-inn" => PlayAction::ChooseInn,
        "resolve" | "resolve-errand" => PlayAction::ResolveErrand,
        "new-adventure" => PlayAction::NewAdventure,
        "search" => PlayAction::Search,
        "choose" => PlayAction::Choose,
        "table-setting" => PlayAction::TableSetting,
        _ => return Err(format!("unknown action {text:?}; see dev describe")),
    })
}

impl PlayAction {
    /// The engine kind submitted for this action.
    pub(super) fn kind(self) -> rpc::GameplayActionKind {
        match self {
            Self::Create => rpc::GameplayActionKind::CreateCharacter,
            Self::BeginStory => rpc::GameplayActionKind::BeginStory,
            Self::AskCourier => rpc::GameplayActionKind::AskCourier,
            Self::EscortCourier => rpc::GameplayActionKind::EscortCourier,
            Self::DefendCourier => rpc::GameplayActionKind::DefendCourier,
            Self::Move => rpc::GameplayActionKind::Move,
            Self::Attack => rpc::GameplayActionKind::GreatswordAttack,
            Self::SecondWind => rpc::GameplayActionKind::SecondWind,
            Self::StabiliseWithHealersKit => rpc::GameplayActionKind::StabiliseWithHealersKit,
            Self::StabiliseMedicine => rpc::GameplayActionKind::StabiliseMedicine,
            Self::HelpAttack => rpc::GameplayActionKind::HelpAttack,
            Self::EndTurn => rpc::GameplayActionKind::EndTurn,
            Self::OpportunityAttack => rpc::GameplayActionKind::OpportunityAttack,
            Self::DeclineReaction => rpc::GameplayActionKind::DeclineReaction,
            Self::Dash => rpc::GameplayActionKind::Dash,
            Self::Dodge => rpc::GameplayActionKind::Dodge,
            Self::Disengage => rpc::GameplayActionKind::Disengage,
            Self::Stand => rpc::GameplayActionKind::Stand,
            Self::PickUpWeapon => rpc::GameplayActionKind::PickUpWeapon,
            Self::ShortRest => rpc::GameplayActionKind::ShortRest,
            Self::ChooseInn => rpc::GameplayActionKind::ChooseHarborScene,
            Self::ResolveErrand => rpc::GameplayActionKind::ResolveErrand,
            Self::NewAdventure => rpc::GameplayActionKind::StartNewAdventure,
            Self::Search => rpc::GameplayActionKind::ExamineHarborSeal,
            Self::Choose => rpc::GameplayActionKind::Unspecified,
            Self::TableSetting => rpc::GameplayActionKind::SetTableSetting,
        }
    }

    /// Whether this action needs a tactical option chosen from the offers.
    pub(super) fn needs_option(self) -> bool {
        matches!(
            self,
            Self::Move
                | Self::Attack
                | Self::OpportunityAttack
                | Self::StabiliseWithHealersKit
                | Self::StabiliseMedicine
                | Self::HelpAttack
                | Self::PickUpWeapon
                | Self::Search
                | Self::Choose
        )
    }
}

#[cfg(test)]
mod action_name_tests {
    use super::*;

    #[test]
    fn listed_reaction_action_names_parse_to_the_same_wire_kind() {
        for (name, kind) in [
            (
                "opportunity-attack",
                rpc::GameplayActionKind::OpportunityAttack,
            ),
            ("decline-reaction", rpc::GameplayActionKind::DeclineReaction),
            ("search", rpc::GameplayActionKind::ExamineHarborSeal),
        ] {
            assert_eq!(parse_action(name).unwrap().kind(), kind);
        }
        assert!(parse_action("opportunity-attack").unwrap().needs_option());
        assert!(!parse_action("decline-reaction").unwrap().needs_option());
    }
}

/// Whether `query` names `candidate`: an exact case-insensitive match, or a
/// unique case-insensitive substring. Returns false on ambiguity (callers
/// refuse rather than guess); use [`match_index`] to find the single hit.
pub(super) fn names_match(candidate: &str, query: &str) -> bool {
    candidate.eq_ignore_ascii_case(query)
        || (!query.is_empty()
            && candidate
                .to_ascii_lowercase()
                .contains(&query.to_ascii_lowercase()))
}

/// The index of the single candidate named by `query`, or `None` when zero or
/// more than one candidate matches.
pub(super) fn match_index(candidates: &[&str], query: &str) -> Option<usize> {
    let exact = candidates
        .iter()
        .position(|candidate| candidate.eq_ignore_ascii_case(query));
    if exact.is_some() {
        return exact;
    }
    let mut found = None;
    for (index, candidate) in candidates.iter().enumerate() {
        if names_match(candidate, query) {
            if found.is_some() {
                return None;
            }
            found = Some(index);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_parsing_accepts_1_based_cells() {
        assert_eq!(parse_cell("c5r6"), Ok(Cell { col0: 4, row0: 5 }));
        assert_eq!(parse_cell("C1R1"), Ok(Cell { col0: 0, row0: 0 }));
        assert_eq!(parse_cell("c5r6").expect("cell").option_id(), "4-5");
    }

    #[test]
    fn cell_parsing_refuses_malformed_cells() {
        for bad in ["", "c5", "5r6", "c0r1", "c1r0", "c33r1", "e5", "c5r6x"] {
            assert!(parse_cell(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn action_parsing_maps_known_words_and_refuses_the_rest() {
        // The two stabilisation actions map to SRD 5.2.1 p18 Help and p97 Healer's Kit use.
        assert_eq!(parse_action("move"), Ok(PlayAction::Move));
        assert_eq!(parse_action("Attack"), Ok(PlayAction::Attack));
        assert_eq!(parse_action("begin-story"), Ok(PlayAction::BeginStory));
        assert_eq!(parse_action("choose"), Ok(PlayAction::Choose));
        assert!(PlayAction::Choose.needs_option());
        let kit = parse_action("stabilise-kit").unwrap();
        assert_eq!(kit.kind(), rpc::GameplayActionKind::StabiliseWithHealersKit);
        assert!(kit.needs_option());
        let medicine = parse_action("stabilise-medicine").unwrap();
        assert_eq!(medicine.kind(), rpc::GameplayActionKind::StabiliseMedicine);
        assert!(medicine.needs_option());
        // SRD 2024 Help (Attack): the ally and the foe are both chosen from the offered pairs.
        let help = parse_action("help-attack").unwrap();
        assert_eq!(help.kind(), rpc::GameplayActionKind::HelpAttack);
        assert!(help.needs_option());
        assert!(parse_action("do something brave").is_err());
        assert!(parse_action("").is_err());
    }

    #[test]
    fn search_action_maps_to_server_observation_srd_521_p187() {
        // SRD 5.2.1 p. 187: Search is a Wisdom check, including Perception for concealed objects.
        let search = parse_action("search").unwrap();
        assert_eq!(search.kind(), rpc::GameplayActionKind::ExamineHarborSeal);
        assert!(search.needs_option());
    }

    #[test]
    fn name_matching_is_exact_first_then_unique_substring() {
        let names = ["Brynn", "Neris"];
        assert_eq!(match_index(&names, "brynn"), Some(0));
        assert_eq!(match_index(&names, "ner"), Some(1));
        assert_eq!(match_index(&names, "r"), None);
        assert_eq!(match_index(&names, "tobin"), None);
    }
}
