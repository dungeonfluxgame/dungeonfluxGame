//! What the runner needs to start one campaign runtime. Everything the runtime itself needs to
//! know (database endpoint, certificates, lease settings) is in the descriptor file the spec
//! names; none of it travels as an argument or an environment variable.

use std::{fmt, path::PathBuf};

/// Where a container sees the read-only descriptor.
pub const DESCRIPTOR_MOUNT: &str = "/etc/dungeonflux/runtime.toml";
/// Where a container sees its read-only secrets directory.
pub const SECRETS_MOUNT: &str = "/run/secrets";
/// Where a container sees its writable spool, a per-container tmpfs.
pub const SPOOL_MOUNT: &str = "/var/lib/dungeonflux/spool";

/// The 16-byte id of a placement (hosted runtime design 6.2).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PlacementId([u8; 16]);

impl PlacementId {
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl fmt::Display for PlacementId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
            .iter()
            .try_for_each(|byte| write!(formatter, "{byte:02x}"))
    }
}

/// Resource limits of one runtime. The defaults are placeholders until the capacity round
/// measures p95 memory and CPU (design 7); they are configuration, not constants of the model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeLimits {
    pub memory_bytes: u64,
    pub cpu_millis: u32,
    pub process_limit: u32,
    pub tmp_bytes: u64,
    pub spool_bytes: u64,
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 1536 * 1024 * 1024,
            cpu_millis: 1000,
            process_limit: 256,
            tmp_bytes: 512 * 1024 * 1024,
            spool_bytes: 256 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeSpec {
    pub placement_id: PlacementId,
    /// The runtime slot on this host: it fixes the container's user id, network and mounts.
    pub slot: u8,
    /// Host path of the slot's descriptor file, mounted read-only into a container. The slot's
    /// `secrets` directory is its sibling.
    pub descriptor_path: PathBuf,
    pub limits: RuntimeLimits,
}
