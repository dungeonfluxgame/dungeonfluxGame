//! Operation ids that cannot repeat across runs.
//!
//! The operation memory is saved in every checkpoint, so an id the host mints after a restart must
//! not equal one an earlier run minted: it would be answered as a replay of an old turn and have no
//! effect. A counter that restarts at 1 cannot promise that; 128 random bits, as a real wire client
//! uses, can. The bits come from the operating system's seed of `RandomState`, which differs for
//! every call, so no new dependency and no clock is needed. The ids only have to be unique, not
//! secret.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

/// 64 random bits.
fn random_u64(salt: u64) -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(salt);
    hasher.finish()
}

/// 128 random bits, never zero (the wire refuses a zero operation id).
pub(super) fn random_u128() -> u128 {
    let wide = (u128::from(random_u64(1)) << 64) | u128::from(random_u64(2));
    wide.max(1)
}
