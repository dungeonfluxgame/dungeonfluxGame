//! The H.16 verb set as `describe` shows it: generated from C7's verb table, with what this host
//! has built of each.
//!
//! C7 (`df_table::inspect`) owns the closed list of verbs the engine fixes (`Verb`) and what each
//! one is (`VerbSpec`: words, usage, summary, whether it reads or drives, who may run it). The host
//! does not keep a second list: `describe` reads that table and adds one fact per verb that only
//! the host knows, `HostSupport`, from an exhaustive `match`, so a verb C7 adds does not compile
//! here until someone says whether the host has it. The family verbs that machines register with
//! their owners (combat, sheet, camera) are not in C7's set; they are listed beside it as waiting.
//!
//! Verbs the host adds beyond C7's set (`history`, `config`, `rules check`) are listed in the
//! usage lines of `describe` and are not in this table: they are the host's, not the engine's.

use df_table::inspect::{Verb, VerbAccess, VerbEffect, VerbSpec};
use serde::{Deserialize, Serialize};

use super::replay::REPLAY_NEEDS;
use super::report::observe::NotBuiltVerb;

/// What the host has of one verb of the set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostSupport {
    /// The verb runs.
    Built,
    /// The verb, its route and its refusal are built; what stands behind it is not.
    Seam,
    /// The host has no such verb yet.
    NotBuilt,
}

impl HostSupport {
    pub const ALL: [Self; 3] = [Self::Built, Self::Seam, Self::NotBuilt];

    /// The serde name of the support.
    pub fn key(self) -> &'static str {
        match self {
            Self::Built => "built",
            Self::Seam => "seam",
            Self::NotBuilt => "not_built",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|support| support.key() == key)
    }

    /// The support as `describe` prints it.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Built => "built",
            Self::Seam => "seam only",
            Self::NotBuilt => "not built",
        }
    }
}

/// One verb of C7's set and what the host has of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerbLine {
    pub words: String,
    pub usage: String,
    pub summary: String,
    /// `reads` or `drives`.
    pub effect: String,
    /// `operator`, or `host` for a verb a hosted build offers.
    pub access: String,
    pub host: HostSupport,
    /// What an unbuilt verb or a seam waits for.
    pub needs: Option<String>,
}

const PENDING_BOOKS_NEED: &str = "the pipeline's pending books and rulings served by a host route \
     (C7's PendingReport and RulingsReport exist)";

/// What the host has of `verb`. Exhaustive: a verb added to C7's set must be placed here.
fn host_support(verb: Verb) -> (HostSupport, Option<&'static str>) {
    match verb {
        Verb::Describe
        | Verb::Dump
        | Verb::Force
        | Verb::Input
        | Verb::Viable
        | Verb::Dice
        | Verb::Clock
        | Verb::Chaos
        | Verb::Health
        | Verb::Cost
        | Verb::Trace
        | Verb::ProjectionShow
        | Verb::Queue
        | Verb::Broker => (HostSupport::Built, None),
        Verb::Replay => (HostSupport::Seam, Some(REPLAY_NEEDS)),
        Verb::FixtureLoad => (
            HostSupport::NotBuilt,
            Some("table export and import (H.12) and a recorded broker script"),
        ),
        Verb::SimRun => (HostSupport::NotBuilt, Some("the sim runner (H.17)")),
        Verb::Soak => (HostSupport::NotBuilt, Some("the load test of H.17")),
        Verb::Fuzz => (
            HostSupport::NotBuilt,
            Some("the property-test driver of H.17"),
        ),
        Verb::PendingList | Verb::PendingShow | Verb::RulingsList | Verb::RulingsShow => {
            (HostSupport::NotBuilt, Some(PENDING_BOOKS_NEED))
        }
    }
}

fn effect_label(effect: VerbEffect) -> &'static str {
    match effect {
        VerbEffect::Reads => "reads",
        VerbEffect::Drives => "drives",
    }
}

fn access_label(access: VerbAccess) -> &'static str {
    match access {
        VerbAccess::Operator => "operator",
        VerbAccess::Host => "host",
    }
}

/// The set, one line per verb, in C7's order.
pub fn verb_lines(specs: &[VerbSpec]) -> Vec<VerbLine> {
    specs
        .iter()
        .map(|spec| {
            let (host, needs) = host_support(spec.verb);
            VerbLine {
                words: spec.words.to_owned(),
                usage: spec.usage.to_owned(),
                summary: spec.summary.to_owned(),
                effect: effect_label(spec.effect).to_owned(),
                access: access_label(spec.access).to_owned(),
                host,
                needs: needs.map(str::to_owned),
            }
        })
        .collect()
}

/// The verbs machines register with their owners, which H.16 names and no machine has yet.
const FAMILY_VERBS_WAITING: [(&str, &str); 4] = [
    (
        "combat choices | combat anchors show | spatial query | effects list | check show",
        "the combat and exploration machines and their rules (M2, rules readiness)",
    ),
    (
        "sheet build | sheet validate | advancement pending | character award",
        "the creation and advancement machines (O.16 row M)",
    ),
    (
        "camera frame|follow | director auto slide|cue | scene state",
        "camera frame is O.13 T4 (G4) and camera follow needs the travelling elements of G2; \
         the slide and cue classes of `director auto` come with their stages (A1, V4); scene \
         state reads what the machines put on the screen (G2). `camera focus|wide` and \
         `director show|pause|resume|auto camera|advance|end|start|view` are built (G1)",
    ),
    (
        "dump <machine|table> naming a machine or the table",
        "per-machine dumps: `dump` today takes no name and prints the active machine and the \
         table, as JSON",
    ),
];

/// What `describe` says is not built: the verbs of C7's set the host lacks or only has a seam for,
/// then the family verbs that wait for their machines.
pub fn not_built(lines: &[VerbLine]) -> Vec<NotBuiltVerb> {
    lines
        .iter()
        .filter(|line| line.host != HostSupport::Built)
        .map(|line| NotBuiltVerb {
            verb: if line.host == HostSupport::Seam {
                format!("{} (seam only)", line.words)
            } else {
                line.words.clone()
            },
            needs: line.needs.clone().unwrap_or_default(),
        })
        .chain(
            FAMILY_VERBS_WAITING
                .iter()
                .map(|(verb, needs)| NotBuiltVerb {
                    verb: (*verb).to_owned(),
                    needs: (*needs).to_owned(),
                }),
        )
        .collect()
}
