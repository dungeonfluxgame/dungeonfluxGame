//! Fixtures for tests and fixture profiles: a scripted backend, a scripted owner and an in-memory
//! attempt store. None of them makes a network or process call, so a fixture profile never
//! reaches a provider.

use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
};

use serde_json::Value;

use crate::text::{
    TextBackend, TextOutcome, TextReceipt, TextRequest, TextUsage, UnavailableReason,
};
use crate::text_attempts::{
    Admission, AttemptMeta, AttemptOwner, AttemptState, AttemptStoreError, Basis, Fingerprint,
    OperationKey, QualifyResult, StoredAttempt, TextAttemptStore, TextAuthority, Verdict,
};

/// Returns scripted outcomes in order. Once the script is empty it refuses with a configuration
/// reason, so a test that makes an unexpected call fails visibly instead of hanging.
pub struct ScriptedBackend {
    receipts: Mutex<VecDeque<TextReceipt>>,
    calls: Arc<AtomicUsize>,
}

impl ScriptedBackend {
    /// `calls` is shared with the caller, which reads the count after the run.
    pub fn new(
        receipts: impl IntoIterator<Item = impl Into<TextReceipt>>,
        calls: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            receipts: Mutex::new(receipts.into_iter().map(Into::into).collect()),
            calls,
        }
    }
}

impl TextBackend for ScriptedBackend {
    fn complete(&self, _: &TextRequest) -> impl Future<Output = TextReceipt> + Send {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let receipt = self
            .receipts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| TextOutcome::Unavailable(UnavailableReason::Configuration).into());
        std::future::ready(receipt)
    }
}

/// A scripted owner for tests: each `current_basis` call returns the next scripted value, and the
/// last value sticks. An empty script means the owner holds nothing.
pub struct FixedAuthority {
    bases: Mutex<VecDeque<Option<Basis>>>,
}

impl FixedAuthority {
    /// An owner that holds `basis` for every call.
    pub fn holding(basis: Basis) -> Self {
        Self::sequence([Some(basis)])
    }

    /// An owner whose basis changes in the scripted order; the last value sticks.
    pub fn sequence(bases: impl IntoIterator<Item = Option<Basis>>) -> Self {
        Self {
            bases: Mutex::new(bases.into_iter().collect()),
        }
    }
}

impl TextAuthority for FixedAuthority {
    fn current_basis(&self, _: &OperationKey) -> impl Future<Output = Option<Basis>> + Send {
        let mut bases = self.bases.lock().unwrap_or_else(PoisonError::into_inner);
        let value = if bases.len() > 1 {
            bases.pop_front().flatten()
        } else {
            bases.front().copied().flatten()
        };
        std::future::ready(value)
    }
}

struct Record {
    fingerprint: Fingerprint,
    state: AttemptState,
    basis: Option<Basis>,
}

/// An in-memory attempt store for fixture profiles and the development probe. It keeps the same
/// transitions as the PostgreSQL store and never leaves the process. A failing store answers
/// every call with its error, to exercise the paths that stop before a send.
#[derive(Default)]
pub struct MemoryAttemptStore {
    records: Mutex<HashMap<(String, String), Record>>,
    failure: Option<AttemptStoreError>,
}

type Records<'a> = MutexGuard<'a, HashMap<(String, String), Record>>;

fn slot(key: &OperationKey) -> (String, String) {
    (key.scope().as_str().to_owned(), key.operation().to_owned())
}

impl MemoryAttemptStore {
    /// A store whose every call fails with `error`.
    pub fn failing(error: AttemptStoreError) -> Self {
        Self {
            failure: Some(error),
            ..Self::default()
        }
    }

    /// A store that already holds one attempt, as a crashed process would have left it.
    pub fn seeded(
        key: &OperationKey,
        fingerprint: Fingerprint,
        state: AttemptState,
        basis: Option<Basis>,
    ) -> Self {
        let store = Self::default();
        store.lock().ok().map(|mut records| {
            records.insert(
                slot(key),
                Record {
                    fingerprint,
                    state,
                    basis,
                },
            )
        });
        store
    }

    /// The stored state under `key`, for assertions.
    pub fn state(&self, key: &OperationKey) -> Option<AttemptState> {
        let records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        records.get(&slot(key)).map(|record| record.state.clone())
    }

    /// How many attempts are stored.
    pub fn len(&self) -> usize {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> Result<Records<'_>, AttemptStoreError> {
        match self.failure {
            Some(error) => Err(error),
            None => Ok(self.records.lock().unwrap_or_else(PoisonError::into_inner)),
        }
    }

    fn transition(
        &self,
        key: &OperationKey,
        expected: impl Fn(&AttemptState) -> bool,
        next: AttemptState,
    ) -> Result<bool, AttemptStoreError> {
        let mut records = self.lock()?;
        match records.get_mut(&slot(key)) {
            Some(record) if expected(&record.state) => {
                record.state = next;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

impl TextAttemptStore for MemoryAttemptStore {
    fn lookup(
        &self,
        key: &OperationKey,
    ) -> impl Future<Output = Result<Option<StoredAttempt>, AttemptStoreError>> + Send {
        let result = self.lock().map(|records| {
            records.get(&slot(key)).map(|record| StoredAttempt {
                fingerprint: record.fingerprint,
                state: record.state.clone(),
            })
        });
        std::future::ready(result)
    }

    fn admit(
        &self,
        key: &OperationKey,
        meta: &AttemptMeta<'_>,
    ) -> impl Future<Output = Result<Admission, AttemptStoreError>> + Send {
        let result = self
            .lock()
            .map(|mut records| match records.get(&slot(key)) {
                Some(record) => Admission::Existing(StoredAttempt {
                    fingerprint: record.fingerprint,
                    state: record.state.clone(),
                }),
                None => {
                    records.insert(
                        slot(key),
                        Record {
                            fingerprint: *meta.fingerprint,
                            state: AttemptState::Accepted,
                            basis: None,
                        },
                    );
                    Admission::Created
                }
            });
        std::future::ready(result)
    }

    fn begin_dispatch(
        &self,
        key: &OperationKey,
        owner: AttemptOwner,
        basis: Basis,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send {
        let result = self
            .lock()
            .map(|mut records| match records.get_mut(&slot(key)) {
                Some(record) if record.state == AttemptState::Accepted => {
                    record.state = AttemptState::Dispatched { owner };
                    record.basis = Some(basis);
                    true
                }
                _ => false,
            });
        std::future::ready(result)
    }

    fn record_candidate(
        &self,
        key: &OperationKey,
        answer: &Value,
        usage: Option<TextUsage>,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send {
        std::future::ready(self.transition(
            key,
            |state| matches!(state, AttemptState::Dispatched { .. }),
            AttemptState::Candidate {
                answer: answer.clone(),
                usage,
            },
        ))
    }

    fn settle(
        &self,
        key: &OperationKey,
        outcome: &TextOutcome,
        usage: Option<TextUsage>,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send {
        // An answer settles only through a candidate and a verdict; an unknown send never settles.
        let result = if matches!(outcome, TextOutcome::Answered(_) | TextOutcome::Unknown) {
            Err(AttemptStoreError::Invalid)
        } else {
            self.transition(
                key,
                |state| matches!(state, AttemptState::Dispatched { .. }),
                AttemptState::Settled {
                    outcome: outcome.clone(),
                    usage,
                },
            )
        };
        std::future::ready(result)
    }

    fn qualify(
        &self,
        key: &OperationKey,
        verdict: Verdict,
        basis_now: Option<Basis>,
    ) -> impl Future<Output = Result<QualifyResult, AttemptStoreError>> + Send {
        let result = self.lock().map(|mut records| {
            let Some(record) = records.get_mut(&slot(key)) else {
                return QualifyResult::NotCandidate;
            };
            let AttemptState::Candidate { answer, usage } = record.state.clone() else {
                return QualifyResult::NotCandidate;
            };
            if basis_now != record.basis {
                record.state = AttemptState::Settled {
                    outcome: TextOutcome::Stale,
                    usage,
                };
                return QualifyResult::BasisMoved;
            }
            record.state = match verdict {
                Verdict::Accepted => AttemptState::Settled {
                    outcome: TextOutcome::Answered(answer),
                    usage,
                },
                Verdict::Rejected => AttemptState::Rejected { usage },
            };
            QualifyResult::Applied
        });
        std::future::ready(result)
    }

    fn invalidate(
        &self,
        key: &OperationKey,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send {
        let result = self
            .lock()
            .map(|mut records| match records.get_mut(&slot(key)) {
                Some(record) => match record.state.clone() {
                    AttemptState::Settled {
                        outcome: TextOutcome::Answered(_),
                        usage,
                    } => {
                        record.state = AttemptState::Rejected { usage };
                        true
                    }
                    _ => false,
                },
                None => false,
            });
        std::future::ready(result)
    }

    fn mark_unknown(
        &self,
        key: &OperationKey,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send {
        std::future::ready(self.transition(
            key,
            |state| matches!(state, AttemptState::Dispatched { .. }),
            AttemptState::Unknown,
        ))
    }

    fn cancel(
        &self,
        key: &OperationKey,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send {
        std::future::ready(self.transition(
            key,
            |state| *state == AttemptState::Accepted,
            AttemptState::Cancelled,
        ))
    }
}
