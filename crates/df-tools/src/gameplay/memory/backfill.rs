//! Backfill: writing the events that canonical sources already hold, from their existing
//! read-only accessors, so a run that started before the memory (or a rebuilt index) is whole.
//!
//! * `LocalLore::development_facts`: the canon each scene holds.
//! * `living_scene::scene_change_journal`: the committed scene changes of the decision journal.
//! * `SocialLedger::grievance_lines`: what each NPC holds against each hero.
//!
//! Each event is built with the same builder the live capture call uses, so a source event
//! captured live and backfilled later has one natural key and adds one row. Every backfilled
//! event is `rebuildable`: it can be dropped and written again from its source.
//!
//! The table log, rolls, narration and private clues are not journaled anywhere else today; they
//! exist only as the rows captured live and are never backfilled or rebuilt.

use std::sync::Arc;

use df_model::checkpoint::Checkpoint;
use serde_json::Value;
use tokio::sync::watch;

use super::{StoryMemory, capture, locked};
use crate::gameplay::{
    adventure::dossier::Dossier, journey, journey::living_scene, npc_reply::local_lore::LocalLore,
    social::SocialLedger,
};

/// What the backfill reads. Held by the memory from start-up so `reindex` can run it again.
#[derive(Clone)]
pub(in crate::gameplay) struct Sources {
    pub(in crate::gameplay) lore: Arc<LocalLore>,
    pub(in crate::gameplay) social: Arc<SocialLedger>,
    pub(in crate::gameplay) dossiers: Vec<Dossier>,
    pub(in crate::gameplay) updates: watch::Receiver<Checkpoint>,
}

/// How many source events each reader offered (an offered event that is already stored adds
/// nothing).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::gameplay) struct BackfillReport {
    pub(in crate::gameplay) canon: usize,
    pub(in crate::gameplay) journal: usize,
    pub(in crate::gameplay) grievances: usize,
    pub(in crate::gameplay) unreadable: usize,
}

/// The ids a scene change's kind object names besides its subject.
fn involved(kind: &Value) -> Vec<String> {
    [
        "holder_id",
        "seller_id",
        "buyer_id",
        "actor_id",
        "noticed_by_id",
        "owner_id",
    ]
    .iter()
    .filter_map(|field| kind.get(field).and_then(Value::as_str))
    .map(str::to_owned)
    .collect()
}

impl StoryMemory {
    pub(in crate::gameplay) fn set_sources(&self, sources: Sources) {
        *locked(&self.sources) = Some(sources);
    }

    /// Offers every source event to the queue. Nothing waits; call [`StoryMemory::settle`] to
    /// wait for the writes.
    pub(in crate::gameplay) fn backfill(&self) -> BackfillReport {
        let mut report = BackfillReport::default();
        let Some(sources) = locked(&self.sources).clone() else {
            return report;
        };
        for (scene, facts) in sources.lore.development_facts(None) {
            for fact in facts {
                let draft = capture::canon_fact_draft(
                    &scene,
                    fact.kind.label(),
                    &fact.name,
                    &fact.content,
                    &fact.established_by,
                );
                report.canon += usize::from(self.record(draft));
            }
        }
        let checkpoint = sources.updates.borrow().clone();
        match living_scene::scene_change_journal(&checkpoint) {
            Ok(changes) => {
                for change in changes {
                    let kind = serde_json::to_value(&change.change.kind).unwrap_or(Value::Null);
                    let tag = kind
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("changed");
                    let sequence = change.revision.sequence();
                    let draft = capture::scene_change_draft(
                        &format!("{}:{sequence}", change.revision.epoch().get()),
                        i64::try_from(sequence).ok(),
                        &change.place,
                        &change.hero,
                        &change.change.entity_id,
                        &change.change.description,
                        tag,
                        &involved(&kind),
                    );
                    report.journal += usize::from(self.record(draft));
                }
            }
            Err(_) => report.unreadable += 1,
        }
        let game = checkpoint.state();
        for character in &game.characters {
            let Ok(hero) = journey::name(game, character.entity) else {
                report.unreadable += 1;
                continue;
            };
            for dossier in &sources.dossiers {
                for line in sources.social.grievance_lines(character.owner, dossier) {
                    let draft =
                        capture::grievance_draft(&hero, &dossier.npc_id, &dossier.name, &line);
                    report.grievances += usize::from(self.record(draft));
                }
            }
        }
        self.audit(format!(
            "MEMORY backfill canon={} journal={} grievances={} unreadable={}",
            report.canon, report.journal, report.grievances, report.unreadable
        ));
        report
    }
}
