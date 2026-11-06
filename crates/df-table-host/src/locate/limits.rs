//! The bounds of the locate feature: picture names, scan time.

use std::time::Duration;

use df_imagery::oneshot_segment;

use super::storage::LAYOUT_SUFFIX;

/// The longest file name, in bytes, the common file systems accept (an external contract).
const MAXIMUM_FILE_NAME_BYTES: usize = 255;
/// What the widest temporary name of a layout write adds to its picture's name: a leading dot, the
/// layout suffix and `.<pid>.<counter>.tmp` (`atomic_file`, a `u32` and a `u64` in decimal).
const LAYOUT_TEMPORARY_EXTRA_BYTES: usize = 1 + LAYOUT_SUFFIX.len() + 1 + 10 + 1 + 20 + 4;
/// The longest picture file name accepted, in bytes: its layout file's temporary name must still
/// fit the file-system bound, so a scan never spends and then fails to store its layout.
pub const MAXIMUM_PICTURE_NAME_BYTES: usize =
    MAXIMUM_FILE_NAME_BYTES - LAYOUT_TEMPORARY_EXTRA_BYTES;
/// The limit for one scan, submit and polling together: the imagery API's own bound.
pub const SCAN_TIMEOUT: Duration = oneshot_segment::MAXIMUM_TIMEOUT;
