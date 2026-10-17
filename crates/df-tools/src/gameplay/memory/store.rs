//! Where the story memory keeps its events: the run's PostgreSQL database in the game, an
//! in-process double in tests. One enum, so the engine is written once and the same recall,
//! consistency and erasure code runs against both.

use df_persistence::story_memory::{
    Counts, DossierRow, Inserted, NewEvent, PendingEmbedding, RecallRecord, Relevance, Removed,
    Scope, Scored, Status, StoredEvent, StoryMemoryDb, StoryMemoryError,
};

#[cfg(test)]
use super::test_store::TestStore;

pub(in crate::gameplay) type StoreResult<T> = Result<T, StoryMemoryError>;

pub(in crate::gameplay) enum Backend {
    Postgres(StoryMemoryDb),
    #[cfg(test)]
    Test(TestStore),
}

macro_rules! dispatch {
    ($self:ident . $method:ident ( $($argument:expr),* )) => {
        match $self {
            Backend::Postgres(store) => store.$method($($argument),*).await,
            #[cfg(test)]
            Backend::Test(store) => store.$method($($argument),*).await,
        }
    };
}

impl Backend {
    pub(in crate::gameplay) fn name(&self) -> &'static str {
        match self {
            Self::Postgres(_) => "postgres",
            #[cfg(test)]
            Self::Test(_) => "in-process",
        }
    }

    pub(in crate::gameplay) fn vector_enabled(&self) -> bool {
        match self {
            Self::Postgres(store) => store.vector_enabled(),
            #[cfg(test)]
            Self::Test(store) => store.vector_enabled(),
        }
    }

    pub(in crate::gameplay) fn vector_reason(&self) -> Option<String> {
        match self {
            Self::Postgres(store) => store.vector_reason().map(str::to_owned),
            #[cfg(test)]
            Self::Test(_) => None,
        }
    }

    pub(in crate::gameplay) async fn insert_events(
        &self,
        events: &[NewEvent],
    ) -> StoreResult<Vec<Inserted>> {
        dispatch!(self.insert_events(events))
    }

    pub(in crate::gameplay) async fn pending_embeddings(
        &self,
        run: &str,
        model: &str,
        limit: usize,
    ) -> StoreResult<Vec<PendingEmbedding>> {
        dispatch!(self.pending_embeddings(run, model, limit))
    }

    pub(in crate::gameplay) async fn store_embeddings(
        &self,
        run: &str,
        model: &str,
        items: &[(i64, Vec<f32>)],
    ) -> StoreResult<u64> {
        dispatch!(self.store_embeddings(run, model, items))
    }

    pub(in crate::gameplay) async fn vector_search(
        &self,
        scope: &Scope,
        model: &str,
        query: &[f32],
        limit: usize,
    ) -> StoreResult<Vec<Scored>> {
        dispatch!(self.vector_search(scope, model, query, limit))
    }

    pub(in crate::gameplay) async fn text_search(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> StoreResult<Vec<Scored>> {
        dispatch!(self.text_search(scope, query, limit))
    }

    pub(in crate::gameplay) async fn select_events(
        &self,
        scope: &Scope,
        relevance: &Relevance,
        limit: usize,
    ) -> StoreResult<Vec<StoredEvent>> {
        dispatch!(self.select_events(scope, relevance, limit))
    }

    pub(in crate::gameplay) async fn events_by_ids(
        &self,
        scope: &Scope,
        ids: &[i64],
    ) -> StoreResult<Vec<StoredEvent>> {
        dispatch!(self.events_by_ids(scope, ids))
    }

    pub(in crate::gameplay) async fn operator_event(
        &self,
        run: &str,
        id: i64,
    ) -> StoreResult<Option<StoredEvent>> {
        dispatch!(self.operator_event(run, id))
    }

    pub(in crate::gameplay) async fn entity_event_ids(
        &self,
        scope: &Scope,
        entity: &str,
    ) -> StoreResult<Option<Vec<(i64, Status)>>> {
        dispatch!(self.entity_event_ids(scope, entity))
    }

    pub(in crate::gameplay) async fn entity_events(
        &self,
        scope: &Scope,
        entity: &str,
        after_id: i64,
        limit: usize,
    ) -> StoreResult<Vec<StoredEvent>> {
        dispatch!(self.entity_events(scope, entity, after_id, limit))
    }

    pub(in crate::gameplay) async fn supersede(
        &self,
        run: &str,
        old: i64,
        new: i64,
    ) -> StoreResult<bool> {
        dispatch!(self.supersede(run, old, new))
    }

    pub(in crate::gameplay) async fn retire_source_since(
        &self,
        run: &str,
        source: &str,
        epoch: &str,
        from_seq: i64,
        reason: &str,
    ) -> StoreResult<u64> {
        dispatch!(self.retire_source_since(run, source, epoch, from_seq, reason))
    }

    pub(in crate::gameplay) async fn retire_revisions_after(
        &self,
        run: &str,
        revision: i64,
        reason: &str,
    ) -> StoreResult<u64> {
        dispatch!(self.retire_revisions_after(run, revision, reason))
    }

    pub(in crate::gameplay) async fn counts(&self, run: &str, model: &str) -> StoreResult<Counts> {
        dispatch!(self.counts(run, model))
    }

    pub(in crate::gameplay) async fn delete_run(
        &self,
        run: &str,
        rebuildable_only: bool,
    ) -> StoreResult<Removed> {
        dispatch!(self.delete_run(run, rebuildable_only))
    }

    pub(in crate::gameplay) async fn erase_hero(
        &self,
        run: &str,
        hero: &str,
    ) -> StoreResult<Removed> {
        dispatch!(self.erase_hero(run, hero))
    }

    pub(in crate::gameplay) async fn log_recall(&self, record: &RecallRecord) -> StoreResult<i64> {
        dispatch!(self.log_recall(record))
    }

    pub(in crate::gameplay) async fn recall(
        &self,
        run: &str,
        id: i64,
    ) -> StoreResult<Option<RecallRecord>> {
        dispatch!(self.recall(run, id))
    }

    pub(in crate::gameplay) async fn dossier(
        &self,
        run: &str,
        entity: &str,
        audience: &str,
    ) -> StoreResult<Option<DossierRow>> {
        dispatch!(self.dossier(run, entity, audience))
    }

    pub(in crate::gameplay) async fn put_dossier(
        &self,
        run: &str,
        dossier: &DossierRow,
    ) -> StoreResult<()> {
        dispatch!(self.put_dossier(run, dossier))
    }
}
