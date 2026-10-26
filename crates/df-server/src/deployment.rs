//! Initial deployment ownership policy; it does not construct server runtime services.
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub(crate) enum DeploymentMode {
    #[default]
    SingleNode,
    MultiNode,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct InstanceId(String);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InstanceIdRefusal {
    Empty,
    TooLong,
    InvalidCharacter,
}

impl InstanceId {
    /// An explicitly configured, bounded identifier for this selected server owner.
    pub(crate) fn new(value: impl Into<String>) -> Result<Self, InstanceIdRefusal> {
        let value = value.into();
        if value.is_empty() {
            return Err(InstanceIdRefusal::Empty);
        }
        if value.len() > 64 {
            return Err(InstanceIdRefusal::TooLong);
        }
        let mut bytes = value.bytes();
        let first = bytes.next().expect("nonempty checked above");
        let valid_initial = first.is_ascii_alphanumeric();
        if !valid_initial
            || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        {
            return Err(InstanceIdRefusal::InvalidCharacter);
        }
        Ok(Self(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DeploymentRefusal {
    MultiNodeUnqualified,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionRoute<'a> {
    pub(crate) instance: &'a InstanceId,
    pub(crate) owner: SessionOwner,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionOwner {
    ProcessOwnedPerSessionActor,
}

/// A selected single-process policy. Construction starts no listener, actor, or storage client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeploymentPolicy {
    mode: DeploymentMode,
    instance: InstanceId,
}

impl DeploymentPolicy {
    /// Select the initial mode using the bounded default of one native server process.
    pub(crate) fn initial(instance: InstanceId) -> Self {
        Self {
            mode: DeploymentMode::default(),
            instance,
        }
    }

    /// Multi-node selection fails closed until durable owner routing and fencing are qualified.
    pub(crate) fn select(
        mode: DeploymentMode,
        instance: InstanceId,
    ) -> Result<Self, DeploymentRefusal> {
        match mode {
            DeploymentMode::SingleNode => Ok(Self { mode, instance }),
            DeploymentMode::MultiNode => Err(DeploymentRefusal::MultiNodeUnqualified),
        }
    }

    pub(crate) const fn mode(&self) -> DeploymentMode {
        self.mode
    }

    /// Every session is sent through this process's directory to its one serialization actor.
    pub(crate) fn route_session(&self) -> SessionRoute<'_> {
        SessionRoute {
            instance: &self.instance,
            owner: SessionOwner::ProcessOwnedPerSessionActor,
        }
    }

    pub(crate) fn instance(&self) -> &InstanceId {
        &self.instance
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StorageRoot {
    TelemetryDatabase,
    TelemetrySpool,
    TelemetrySegments,
    Postgresql,
    DurableMedia,
    WorkflowDatabase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TelemetryRootRefusal {
    InstanceMismatch,
    NotAbsolute(StorageRoot),
    ParentTraversal(StorageRoot),
    Overlap {
        first: StorageRoot,
        second: StorageRoot,
    },
}

/// Local telemetry storage roots are separate from gameplay, media, and workflow storage.
/// This is a lexical configuration check; it performs no filesystem access and cannot detect
/// symlink aliases or prove that a configured store is actually mounted or writable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TelemetryRoots {
    pub(crate) instance: InstanceId,
    pub(crate) database: PathBuf,
    pub(crate) spool: PathBuf,
    pub(crate) segments: PathBuf,
}

/// Local filesystem projections for the other storage owners, supplied by configuration.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OtherStorageRoots<'a> {
    pub(crate) postgresql: &'a Path,
    pub(crate) durable_media: &'a Path,
    pub(crate) workflow_database: &'a Path,
}

impl TelemetryRoots {
    pub(crate) fn validate_for(
        self,
        policy: &DeploymentPolicy,
        other: OtherStorageRoots<'_>,
    ) -> Result<Self, TelemetryRootRefusal> {
        if self.instance != *policy.instance() {
            return Err(TelemetryRootRefusal::InstanceMismatch);
        }

        let telemetry = [
            (StorageRoot::TelemetryDatabase, self.database.as_path()),
            (StorageRoot::TelemetrySpool, self.spool.as_path()),
            (StorageRoot::TelemetrySegments, self.segments.as_path()),
        ];
        let protected = [
            (StorageRoot::Postgresql, other.postgresql),
            (StorageRoot::DurableMedia, other.durable_media),
            (StorageRoot::WorkflowDatabase, other.workflow_database),
        ];

        for (owner, path) in telemetry.iter().chain(protected.iter()) {
            if !path.is_absolute() {
                return Err(TelemetryRootRefusal::NotAbsolute(*owner));
            }
            if path
                .components()
                .any(|component| component == Component::ParentDir)
            {
                return Err(TelemetryRootRefusal::ParentTraversal(*owner));
            }
        }

        for (index, (first_owner, first_path)) in telemetry.iter().enumerate() {
            for (second_owner, second_path) in
                telemetry.iter().skip(index + 1).chain(protected.iter())
            {
                if paths_overlap(first_path, second_path) {
                    return Err(TelemetryRootRefusal::Overlap {
                        first: *first_owner,
                        second: *second_owner,
                    });
                }
            }
        }

        Ok(self)
    }
}

fn paths_overlap(first: &Path, second: &Path) -> bool {
    first.starts_with(second) || second.starts_with(first)
}
