//! The scene builder's size, count and time bounds on the host. They only protect against a
//! mistake (a pipeline that cannot finish, a run that never takes its lines), so they are
//! deliberately huge; scope one down when it is measured (owner, 2026-11-02).

/// Most stage inputs remembered; the oldest are forgotten first.
pub const MAXIMUM_REMEMBERED_INPUTS: usize = 1_000_000;
/// Most calls the fake stage worker remembers having started; the oldest are forgotten first.
pub const MAXIMUM_REMEMBERED_CALLS: usize = 1_000_000;
/// Most audit events one workbench run keeps for its answer; the oldest are forgotten first.
pub const MAXIMUM_KEPT_EVENTS: usize = 1_000_000;
/// The longest a build may take in the workbench's own time before it stops waiting, in
/// milliseconds: an hour, far past every deadline in the broker, so only a pipeline that cannot
/// finish reaches it.
pub const MAXIMUM_DEV_VIRTUAL_MS: u64 = 3_600_000;
/// How long one segmentation request may take. The S2 median was 1.5 s and the slowest 3 s, so
/// 30 s is about twenty times the median: a candidate, to be tuned from measurement (the plan
/// gives none).
pub const LOCATE_DEADLINE_MS: u64 = 30_000;
