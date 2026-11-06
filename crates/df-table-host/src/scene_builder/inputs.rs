//! What each stage request is for, kept where the worker that serves it can read it.
//!
//! A broker request carries a purpose, a key, an epoch and a correlation, and nothing about what
//! to draw or which picture to segment. The scene builder records a `StageInput` under the
//! request's key before it asks, and the worker that runs the provider call reads it by the key of
//! the call it claimed. The key is the hash of the purpose and the inputs that matter to that
//! stage, with the scene generation (H.8): the background and the description depend on the scene
//! alone, so a rebuild of the same scene shares one flight; a plate depends on what the picture
//! shows and the background it is made from; a Locate request depends on the plate it reads and
//! the prompt.
//!
//! The book is bounded, and the oldest input is forgotten first, which is safe because a request
//! reads its input when it is claimed, soon after it was recorded.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, MutexGuard, PoisonError};

use df_table::identity::SceneId;
use df_table::plate::digest::PlateDigest;
use df_table::plate::job::PlateJobId;
use df_table::ports::PurposeName;
use df_table::scene_plan::locate::LocatePrompt;
use df_table::scene_plan::{SceneStage, StoredPicture};
use df_table::table_state::SceneGeneration;

use super::limits::MAXIMUM_REMEMBERED_INPUTS;
use crate::broker::request::RequestKey;

/// What one stage request is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageInput {
    pub scene: SceneId,
    pub generation: SceneGeneration,
    pub job: PlateJobId,
    pub stage: SceneStage,
    /// What the picture shows, for the plate stage.
    pub staging_digest: u64,
    /// The picture the stage starts from: the background for a plate, the plate for Locate.
    pub source: Option<StoredPicture>,
    /// What Locate asks for, for the Locate stage.
    pub prompt: Option<LocatePrompt>,
}

impl StageInput {
    /// The key of the request for this input under `purpose` (H.8).
    pub fn key(&self, purpose: &PurposeName) -> RequestKey {
        let mut digest = PlateDigest::new();
        digest.text(purpose.as_str());
        digest.number(self.scene.get());
        digest.number(self.generation.get());
        digest.text(self.stage.key());
        match self.stage {
            SceneStage::Describe | SceneStage::Background | SceneStage::Ready => {}
            SceneStage::Plate => {
                digest.number(self.staging_digest);
                digest.option(self.source, |digest, picture| digest.number(picture.handle));
            }
            SceneStage::Locate => {
                digest.option(self.source, |digest, picture| digest.number(picture.handle));
                match &self.prompt {
                    Some(LocatePrompt::People) => digest.byte(1),
                    Some(LocatePrompt::Item {
                        item,
                        phrase,
                        attempt,
                    }) => {
                        digest.byte(2);
                        digest.text(item.as_str());
                        digest.text(phrase.as_str());
                        digest.byte(*attempt);
                    }
                    None => digest.byte(0),
                }
            }
        }
        RequestKey::new(digest.finish())
    }
}

#[derive(Default)]
struct Book {
    by_key: BTreeMap<RequestKey, StageInput>,
    order: VecDeque<RequestKey>,
}

/// The inputs of the requests the scene builder made.
#[derive(Default)]
pub struct StageInputs {
    book: Mutex<Book>,
}

impl StageInputs {
    fn book(&self) -> MutexGuard<'_, Book> {
        self.book.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn record(&self, key: RequestKey, input: StageInput) {
        let mut book = self.book();
        if book.by_key.insert(key, input).is_none() {
            book.order.push_back(key);
        }
        while book.order.len() > MAXIMUM_REMEMBERED_INPUTS {
            if let Some(oldest) = book.order.pop_front() {
                book.by_key.remove(&oldest);
            }
        }
    }

    pub fn get(&self, key: RequestKey) -> Option<StageInput> {
        self.book().by_key.get(&key).cloned()
    }

    pub fn len(&self) -> usize {
        self.book().by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
