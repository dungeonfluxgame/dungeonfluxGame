//! Startup binds creator capability to the mounted image service; it submits no work.
use super::{
    creator_portraits::Portraits,
    imagery::Imagery,
    media_policy::{self, Capability},
};
use std::{io, path::Path};
pub(super) fn start(imagery: Imagery, _current: &Path) -> Result<Portraits, io::Error> {
    let enabled = match media_policy::gates()
        .input(Capability::CreatorPortraits)
        .ok()
        .as_deref()
    {
        None | Some("" | "off") => false,
        Some("on") => true,
        Some(_) => {
            return Err(io::Error::other(
                "DF_GAMEPLAY_CREATOR_PORTRAITS must be on or off",
            ));
        }
    };
    Ok(Portraits::new(enabled.then_some(imagery)))
}
