//! What a page is for: the developer's playground, or a real phone on the player listener.
//!
//! The host serves the same page at both addresses, and the page reads its role from the address
//! it was loaded at, so one build draws both. A phone draws only the phone view, whatever the
//! host-global `Device` control says, keeps one name for itself across reloads, and does not read
//! the canvas (the player listener does not serve it). It is plain data; the page does the drawing.

use serde::{Deserialize, Serialize};

use crate::controls::Device;
use crate::path::PLAY;

/// What a page is for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageRole {
    /// The playground at `/playground`: the studio, with the devices the controls choose.
    #[default]
    Playground,
    /// A real phone at `path::PLAY`: only the phone view, filling the screen.
    Phone,
}

impl PageRole {
    pub const ALL: [Self; 2] = [Self::Playground, Self::Phone];

    pub fn key(self) -> &'static str {
        match self {
            Self::Playground => "playground",
            Self::Phone => "phone",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }

    /// The role of a page loaded at `pathname` (the address without its query).
    pub fn of_path(pathname: &str) -> Self {
        let is_play = pathname
            .strip_prefix(PLAY)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'));
        if is_play {
            Self::Phone
        } else {
            Self::Playground
        }
    }

    /// The devices this page draws when the host's control says `chosen`.
    pub fn shown_device(self, chosen: Device) -> Device {
        match self {
            Self::Playground => chosen,
            Self::Phone => Device::Phone,
        }
    }

    /// Whether the page reads the live canvas: only the shared screen draws it.
    pub fn reads_canvas(self) -> bool {
        self == Self::Playground
    }

    /// The start of the names this page gives itself (`page-xxxxxxxx`, `phone-xxxxxxxx`).
    fn client_prefix(self) -> &'static str {
        match self {
            Self::Playground => "page",
            Self::Phone => "phone",
        }
    }

    /// The name a page of this role gives itself from 32 random bits.
    pub fn client_name(self, random: u32) -> String {
        format!("{}-{random:08x}", self.client_prefix())
    }

    /// Whether `name` is a name `client_name` makes for this role: a kept name read back from the
    /// browser's storage is used only when it is.
    pub fn is_client_name(self, name: &str) -> bool {
        name.strip_prefix(self.client_prefix())
            .and_then(|rest| rest.strip_prefix('-'))
            .is_some_and(|random| {
                random.len() == 8
                    && random
                        .chars()
                        .all(|character| character.is_ascii_hexdigit())
            })
    }
}
