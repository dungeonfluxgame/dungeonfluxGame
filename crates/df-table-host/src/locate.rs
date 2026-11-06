//! `media locate`: the layout of a library picture from a SAM segmentation through the imagery API.
//!
//! The host asks `df_imagery::oneshot_segment` (the fal SAM backend, SAM 3.1 by default) for the
//! instances of one noun prompt in a picture of the media library, reduces each instance to a box,
//! a head point and its mask (a transparent PNG) with the imagery API's own helpers, and stores the
//! result in the playground's `layouts` folder as `<picture>.layout.json` (a
//! `df_scene_layout::SceneLayout`, bound to the picture by its digest). When the page reads a
//! fixture whose backdrop is that picture, the host attaches the layout to the backdrop, so bubbles
//! can anchor to its regions and the SAM layer can paint its masks (`playground set sam=`).
//!
//! A scan finds instances, not identities: a region gets a name from `media locate label` or from
//! `media locate identify` (the text service). A new scan keeps the names of the regions it finds
//! again (by overlap). The supplier key is read from `DF_FAL_KEY` or `FAL_KEY` in the host's
//! environment when a scan starts, passed to the imagery API and never written anywhere; an audit
//! line holds names, counts and times only. A scan is spend that is recorded and never blocked
//! (owner decision: no spend caps). Every refusal is a typed [`LocateError`].

pub mod cli;
mod identify;
mod limits;
pub mod request;
pub mod routes;
mod scan;
mod storage;
mod validate;

pub use self::cli::{MediaLocate, run};
pub use self::identify::{
    DEFAULT_MIN_CONFIDENCE_PERMILLE, IDENTIFY_SCOPE, IdentityAnswer, PreparedIdentity,
    apply_identification, ask_identity, identity_request, prepare_identity, store_identification,
};
pub use self::limits::{MAXIMUM_PICTURE_NAME_BYTES, SCAN_TIMEOUT};
pub use self::request::{
    CharacterRequest, IdentifiedLayout, IdentifyRequest, IdentityOutcome, IdentityRow, IoCause,
    LabelRequest, LocateError, LocateReply, LocateRequest, LocatedLayout, QuestionCause, ScanCause,
    ScanPrompt, ShowQuery, StoredLayout, TextServiceCause,
};
pub use self::routes::router;
pub use self::scan::{
    FAL_KEY_VARIABLES, KEEP_LABEL_OVERLAP_PERMILLE, Scanned, keep_labels, layout_from, relabel,
    scan_picture,
};
pub use self::storage::{
    LAYOUT_DIRECTORY, LAYOUT_SUFFIX, attach_layout, current_layout, layout_file_name, read_layout,
    write_layout,
};
pub(crate) use self::storage::{ShownBackdrop, shown_backdrop};
pub use self::validate::{DEFAULT_MASKS, check_label, check_masks, check_picture_name};

#[cfg(test)]
mod locate_tests;
#[cfg(test)]
mod request_tests;
#[cfg(test)]
mod routes_tests;
