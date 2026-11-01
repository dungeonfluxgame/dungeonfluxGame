use df_commerce::draft_image::{Binding, ConsentId};
use df_commerce::finance_ids::{AttemptId, QuoteId};

#[derive(Clone, Debug)]
pub struct AdmissionCommand {
    pub consent: ConsentId,
    pub consent_revision: u64,
    pub quote: QuoteId,
    pub binding: Binding,
}

#[derive(Clone, Debug)]
pub struct QuoteCommand {
    pub admission: AdmissionCommand,
    pub expires_at_ms: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Usability {
    Usable,
    NoUsableOutput,
}

impl Usability {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Usable => "usable",
            Self::NoUsableOutput => "no_usable_output",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SettlementCommand {
    pub attempt: AttemptId,
    pub actual_units: Vec<(String, u64)>,
    pub usability: Usability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Settlement {
    pub charged_micro_dollars: u64,
    pub released_micro_dollars: u64,
    pub replay: bool,
}
