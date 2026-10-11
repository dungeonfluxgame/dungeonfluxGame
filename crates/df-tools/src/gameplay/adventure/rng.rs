//! SplitMix64 stream specified by `tables.json`. Only `roll` exposes entropy.
pub(super) struct Dice {
    state: u64,
}
const STEP: u64 = 0x9e37_79b9_7f4a_7c15;
fn finalize(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}
impl Dice {
    pub(super) fn new(seed: u64) -> Self {
        Self {
            state: finalize(seed.wrapping_add(STEP)),
        }
    }
    pub(super) fn roll(&mut self, die: u8) -> u8 {
        // The caller admits only nonzero named dice; u128 represents 2^64 exactly.
        let limit = (1u128 << 64) - ((1u128 << 64) % u128::from(die));
        loop {
            self.state = self.state.wrapping_add(STEP);
            let value = finalize(self.state);
            if u128::from(value) < limit {
                return (value % u64::from(die)) as u8 + 1;
            }
        }
    }
}
