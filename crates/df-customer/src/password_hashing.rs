//! Argon2id password hashing off the async workers (coordinator decision of 2026-10-24, OWASP
//! parameters: 19 MiB, 2 passes, 1 lane).
//!
//! Hashing runs on the blocking pool behind a semaphore, so a flood of logins cannot occupy more
//! than `maximum_concurrent` cores; a request that cannot get a permit in time is `Busy`.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHasher as _, PasswordVerifier as _, SaltString},
};
use tokio::sync::Semaphore;

use crate::tokens::{PasswordHash, random_bytes};

/// Argon2id cost parameters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HashingParameters {
    pub memory_kib: u32,
    pub passes: u32,
    pub lanes: u32,
}

impl HashingParameters {
    /// OWASP's recommended Argon2id minimum.
    pub const PRODUCTION: Self = Self {
        memory_kib: 19 * 1024,
        passes: 2,
        lanes: 1,
    };

    /// The smallest valid cost, for tests only: the same code path, much faster.
    #[cfg(any(test, feature = "test-fakes"))]
    pub const TEST: Self = Self {
        memory_kib: 8,
        passes: 1,
        lanes: 1,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HashError {
    /// No permit within the wait deadline: too many hashes in flight.
    Busy,
    /// The random source or the hashing task failed.
    Failed,
}

/// The result of a verification. `needs_rehash` asks the caller to store a fresh hash.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Verification {
    pub is_match: bool,
    pub needs_rehash: bool,
}

pub struct PasswordHasher {
    parameters: HashingParameters,
    permits: Arc<Semaphore>,
    permit_wait: Duration,
    dummy: PasswordHash,
    verifications: AtomicU64,
}

impl PasswordHasher {
    /// Builds the hasher and the dummy hash used to equalize timing for unknown addresses.
    pub async fn new(
        parameters: HashingParameters,
        maximum_concurrent: usize,
        permit_wait: Duration,
    ) -> Result<Self, HashError> {
        let mut hasher = Self {
            parameters,
            permits: Arc::new(Semaphore::new(maximum_concurrent.max(1))),
            permit_wait,
            dummy: PasswordHash::from_phc(String::new()),
            verifications: AtomicU64::new(0),
        };
        let dummy_password =
            crate::tokens::hex(&random_bytes::<16>().map_err(|_| HashError::Failed)?);
        hasher.dummy = hasher.hash(&dummy_password).await?;
        Ok(hasher)
    }

    fn argon(parameters: HashingParameters) -> Result<Argon2<'static>, HashError> {
        let params = Params::new(
            parameters.memory_kib,
            parameters.passes,
            parameters.lanes,
            None,
        )
        .map_err(|_| HashError::Failed)?;
        Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
    }

    async fn on_blocking_pool<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, HashError> + Send + 'static,
    ) -> Result<T, HashError> {
        let permit =
            tokio::time::timeout(self.permit_wait, Arc::clone(&self.permits).acquire_owned())
                .await
                .map_err(|_| HashError::Busy)?
                .map_err(|_| HashError::Failed)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work()
        })
        .await
        .map_err(|_| HashError::Failed)?
    }

    pub async fn hash(&self, password: &str) -> Result<PasswordHash, HashError> {
        let parameters = self.parameters;
        let password = password.to_owned();
        let salt = random_bytes::<16>().map_err(|_| HashError::Failed)?;
        self.on_blocking_pool(move || {
            let salt = SaltString::encode_b64(&salt).map_err(|_| HashError::Failed)?;
            let phc = Self::argon(parameters)?
                .hash_password(password.as_bytes(), &salt)
                .map_err(|_| HashError::Failed)?
                .to_string();
            Ok(PasswordHash::from_phc(phc))
        })
        .await
    }

    /// Verifies in constant time (inside Argon2). An unparsable stored hash is a non-match.
    pub async fn verify(
        &self,
        password: &str,
        stored: &PasswordHash,
    ) -> Result<Verification, HashError> {
        self.verifications.fetch_add(1, Ordering::Relaxed);
        let parameters = self.parameters;
        let password = password.to_owned();
        let stored = stored.phc().to_owned();
        self.on_blocking_pool(move || {
            let Ok(parsed) = argon2::PasswordHash::new(&stored) else {
                return Ok(Verification {
                    is_match: false,
                    needs_rehash: false,
                });
            };
            let is_match = Self::argon(parameters)?
                .verify_password(password.as_bytes(), &parsed)
                .is_ok();
            let current = Params::try_from(&parsed).ok();
            let needs_rehash = is_match
                && current.is_none_or(|current| {
                    current.m_cost() != parameters.memory_kib
                        || current.t_cost() != parameters.passes
                        || current.p_cost() != parameters.lanes
                });
            Ok(Verification {
                is_match,
                needs_rehash,
            })
        })
        .await
    }

    /// Spends one verification on the dummy hash, so an unknown address costs what a known one
    /// does. Always a non-match.
    pub async fn verify_dummy(&self, password: &str) -> Result<(), HashError> {
        let dummy = self.dummy.clone();
        self.verify(password, &dummy).await.map(|_| ())
    }

    /// Verifications run so far: a test statistic, never a decision input.
    pub fn verification_count(&self) -> u64 {
        self.verifications.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn hasher(parameters: HashingParameters) -> PasswordHasher {
        PasswordHasher::new(parameters, 2, Duration::from_secs(5))
            .await
            .unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_hash_verifies_its_password_and_refuses_another() {
        let hasher = hasher(HashingParameters::TEST).await;
        let hash = hasher.hash("correct horse battery").await.unwrap();
        assert!(hash.phc().starts_with("$argon2id$v=19$m=8,t=1,p=1$"));
        let right = hasher.verify("correct horse battery", &hash).await.unwrap();
        assert_eq!(
            right,
            Verification {
                is_match: true,
                needs_rehash: false
            }
        );
        let wrong = hasher.verify("correct horse battern", &hash).await.unwrap();
        assert!(!wrong.is_match);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_hash_made_with_other_parameters_asks_for_a_rehash_on_match() {
        let old = hasher(HashingParameters::TEST).await;
        let hash = old.hash("correct horse battery").await.unwrap();
        let current = hasher(HashingParameters {
            memory_kib: 16,
            passes: 1,
            lanes: 1,
        })
        .await;
        let verification = current
            .verify("correct horse battery", &hash)
            .await
            .unwrap();
        assert!(verification.is_match && verification.needs_rehash);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_garbled_stored_hash_is_a_non_match_not_an_error() {
        let hasher = hasher(HashingParameters::TEST).await;
        let verification = hasher
            .verify(
                "anything at all",
                &PasswordHash::from_phc("garbage".to_owned()),
            )
            .await
            .unwrap();
        assert!(!verification.is_match);
    }

    #[tokio::test(start_paused = true)]
    async fn a_hash_waiting_past_the_permit_deadline_is_busy() {
        let hasher = PasswordHasher::new(HashingParameters::TEST, 1, Duration::from_secs(5))
            .await
            .unwrap();
        let _held = Arc::clone(&hasher.permits).acquire_owned().await.unwrap();
        assert_eq!(
            hasher.hash("correct horse battery").await.err(),
            Some(HashError::Busy)
        );
    }
}
