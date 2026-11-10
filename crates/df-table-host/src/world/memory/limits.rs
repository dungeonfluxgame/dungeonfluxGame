//! The bounds of the memory API. They only guard against mistakes, so they are generous.

/// The longest title, in characters.
pub const MAXIMUM_TITLE_CHARS: usize = 1_000;
/// The longest description, in characters.
pub const MAXIMUM_DESCRIPTION_CHARS: usize = 20_000;
/// The longest `created_by`, in characters.
pub const MAXIMUM_CREATED_BY_CHARS: usize = 1_000;
/// The most pictures one memory names.
pub const MAXIMUM_PICTURES: usize = 64;
/// The most links one memory has.
pub const MAXIMUM_LINKS: usize = 256;
/// The most tags one memory has.
pub const MAXIMUM_TAGS: usize = 64;
/// The most memories one owner keeps.
pub const MAXIMUM_MEMORIES_PER_OWNER: usize = 100_000;
/// The memories a list answers when it is given no limit.
pub const DEFAULT_PAGE: usize = 100;
/// The most memories one list answers.
pub const MAXIMUM_PAGE: usize = 10_000;
