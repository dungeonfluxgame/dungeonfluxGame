//! Starts the runtime as a container through the body-checking Docker proxy (design 7, S2's
//! `deploy/docker-proxy`). The runner never talks to a Docker daemon: it sends typed requests to
//! the proxy port. The production adapter over the proxy's unix socket is not part of this crate
//! yet. The create body is built to the proxy's strict policy so a compliant request is never
//! refused: the full allow-listed image reference, the slot's user, name and network, not
//! privileged, no-new-privileges, all capabilities dropped, a read-only root, only the slot's
//! descriptor and secrets mounts (read-only), a sized spool tmpfs instead of a spool bind, the
//! runtime role label, the fixed command line and an empty environment.
//! `tests/proxy_contract.rs` runs the proxy's own `policy::check` on this body.

use std::{
    fmt,
    future::Future,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use serde_json::{Map, Value, json};

use crate::{
    launcher::{ExitReport, LaunchError, RuntimeLauncher},
    spec::{DESCRIPTOR_MOUNT, PlacementId, RuntimeLimits, RuntimeSpec, SECRETS_MOUNT, SPOOL_MOUNT},
};

/// First user id of the runtime slots; slot `n` runs as `RUNTIME_UID_BASE + n`.
pub const RUNTIME_UID_BASE: u32 = 20_000;
/// The group every runtime shares.
pub const RUNTIME_GID: u32 = 10_002;
/// Slot numbers the proxy admits (`1..=MAXIMUM_SLOT`).
pub const MAXIMUM_SLOT: u8 = 99;
/// Slot `n`'s container network is this prefix followed by `n`.
pub const SLOT_NETWORK_PREFIX: &str = "df-slot-";
pub const ROLE_LABEL: (&str, &str) = ("com.dungeonflux.role", "runtime");
/// Limits the proxy enforces on a create body.
pub const MAXIMUM_MEMORY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAXIMUM_NANO_CPUS: u64 = 8_000_000_000;
pub const MAXIMUM_PIDS: u32 = 1024;
pub const MAXIMUM_TMPFS_BYTES: u64 = 1024 * 1024 * 1024;
/// Longest container name the proxy admits.
pub const MAXIMUM_NAME_LENGTH: usize = 100;

/// An image reference the proxy's allow-list holds: `repository[:tag]@sha256:<manifest digest>`.
/// A bare `sha256:` is not one: Docker reads it as an image id, not the manifest digest the
/// deploy tool verified.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageReference(String);

impl ImageReference {
    pub fn new(value: &str) -> Option<Self> {
        let (repository, digest) = value.rsplit_once("@sha256:")?;
        let repository_ok = !repository.is_empty()
            && repository
                .bytes()
                .all(|b| b.is_ascii_graphic() && b != b'@');
        let digest_ok = digest.len() == 64
            && digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        (repository_ok && digest_ok).then(|| Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An opaque container id returned by the proxy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContainerId(pub String);

/// The create-body rules of the proxy's policy, by the outcome they protect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyRule {
    ImageNotAllowListed,
    UserOutsideSlotRange,
    NameMustCarryTheSlot,
    RoleLabelMissing,
    CommandNotTheRuntimeCommand,
    EnvironmentNotEmpty,
    Privileged,
    CapabilitiesAdded,
    CapabilitiesNotAllDropped,
    DevicesPresent,
    PidNamespaceNotPrivate,
    IpcNamespaceNotPrivate,
    RootNotReadOnly,
    NetworkNotTheSlotNetwork,
    SecurityOptionNotNoNewPrivileges,
    MemoryUnsetOrAboveMaximum,
    SwapNotEqualToMemory,
    CpuUnsetOrAboveMaximum,
    ProcessLimitUnsetOrAboveMaximum,
    MountNotTheSlotDescriptorOrSecrets,
    TmpfsPathNotAllowed,
    TmpfsOptionNotAllowed,
    /// A refusal by the real proxy whose text this crate does not map to a rule.
    Other,
}

impl fmt::Display for PolicyRule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProxyError {
    /// The proxy's policy refused the request; names the rule.
    Refused(PolicyRule),
    Unavailable,
    UnknownContainer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostConfig {
    pub privileged: bool,
    pub binds: Vec<String>,
    pub cap_add: Vec<String>,
    pub cap_drop: Vec<String>,
    pub network_mode: String,
    pub devices: Vec<String>,
    /// Empty: Docker's default private namespace (`private` is not a value it accepts here).
    pub pid_mode: String,
    pub ipc_mode: String,
    pub readonly_rootfs: bool,
    pub security_opt: Vec<String>,
    pub memory_bytes: u64,
    pub memory_swap_bytes: u64,
    pub nano_cpus: u64,
    pub pids_limit: u32,
    /// `(path, options)` of each tmpfs mount; Docker takes them as an object keyed by path.
    pub tmpfs: Vec<(String, String)>,
}

/// A create request: the container name (a query parameter) and the fields of Docker's create
/// body the runner sets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateBody {
    pub name: String,
    pub image: String,
    pub user: String,
    pub env: Vec<String>,
    pub cmd: Vec<String>,
    pub labels: Vec<(String, String)>,
    pub host_config: HostConfig,
}

impl CreateBody {
    /// The request target of the create call; the name travels in the query.
    pub fn target(&self) -> String {
        format!("/containers/create?name={}", self.name)
    }

    /// The JSON body Docker's create endpoint takes (names as in Docker's API, `Tmpfs` an object).
    pub fn to_docker_json(&self) -> Value {
        let host = &self.host_config;
        let tmpfs: Map<String, Value> = host
            .tmpfs
            .iter()
            .map(|(path, options)| (path.clone(), Value::String(options.clone())))
            .collect();
        let labels: Map<String, Value> = self
            .labels
            .iter()
            .map(|(key, value)| (key.clone(), Value::String(value.clone())))
            .collect();
        json!({
            "Image": self.image,
            "User": self.user,
            "Env": self.env,
            "Cmd": self.cmd,
            "Labels": labels,
            "HostConfig": {
                "Privileged": host.privileged,
                "Binds": host.binds,
                "CapAdd": host.cap_add,
                "CapDrop": host.cap_drop,
                "NetworkMode": host.network_mode,
                "Devices": host.devices,
                "PidMode": host.pid_mode,
                "IpcMode": host.ipc_mode,
                "ReadonlyRootfs": host.readonly_rootfs,
                "SecurityOpt": host.security_opt,
                "Memory": host.memory_bytes,
                "MemorySwap": host.memory_swap_bytes,
                "NanoCpus": host.nano_cpus,
                "PidsLimit": host.pids_limit,
                "Tmpfs": tmpfs,
            },
        })
    }
}

/// The proxy's request surface: create, start, stop, wait and delete only.
pub trait DockerProxy {
    fn create(
        &self,
        body: &CreateBody,
    ) -> impl Future<Output = Result<ContainerId, ProxyError>> + Send;
    fn start(&self, id: &ContainerId) -> impl Future<Output = Result<(), ProxyError>> + Send;
    fn stop(
        &self,
        id: &ContainerId,
        grace: Duration,
    ) -> impl Future<Output = Result<(), ProxyError>> + Send;
    fn wait(&self, id: &ContainerId)
    -> impl Future<Output = Result<ExitReport, ProxyError>> + Send;
    fn delete(&self, id: &ContainerId) -> impl Future<Output = Result<(), ProxyError>> + Send;
}

/// The deployed container settings: which images the runner may start.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContainerConfig {
    image: ImageReference,
    allowed_images: Vec<ImageReference>,
}

impl ContainerConfig {
    /// `None` when the chosen image is not in the deployed allow-list.
    pub fn new(image: ImageReference, allowed_images: Vec<ImageReference>) -> Option<Self> {
        allowed_images.contains(&image).then_some(Self {
            image,
            allowed_images,
        })
    }

    pub fn allowed_images(&self) -> &[ImageReference] {
        &self.allowed_images
    }

    fn create_body(&self, spec: &RuntimeSpec) -> CreateBody {
        let RuntimeLimits {
            memory_bytes,
            cpu_millis,
            process_limit,
            tmp_bytes,
            spool_bytes,
        } = spec.limits;
        let slot = spec.slot;
        let uid = RUNTIME_UID_BASE + u32::from(slot);
        // A container path is always POSIX, whatever platform builds the request.
        let slot_directory = spec
            .descriptor_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .display();
        CreateBody {
            name: format!("df-rt-{}-slot{slot}", spec.placement_id),
            image: self.image.as_str().to_string(),
            user: format!("{uid}:{RUNTIME_GID}"),
            env: Vec::new(),
            cmd: vec![
                "campaign-runtime".to_string(),
                "--descriptor".to_string(),
                DESCRIPTOR_MOUNT.to_string(),
            ],
            labels: vec![(ROLE_LABEL.0.to_string(), ROLE_LABEL.1.to_string())],
            host_config: HostConfig {
                privileged: false,
                binds: vec![
                    format!("{}:{DESCRIPTOR_MOUNT}:ro", spec.descriptor_path.display()),
                    format!("{slot_directory}/secrets:{SECRETS_MOUNT}:ro"),
                ],
                cap_add: Vec::new(),
                cap_drop: vec!["ALL".to_string()],
                network_mode: format!("{SLOT_NETWORK_PREFIX}{slot}"),
                devices: Vec::new(),
                pid_mode: String::new(),
                ipc_mode: "private".to_string(),
                readonly_rootfs: true,
                security_opt: vec!["no-new-privileges:true".to_string()],
                memory_bytes,
                memory_swap_bytes: memory_bytes,
                nano_cpus: u64::from(cpu_millis) * 1_000_000,
                pids_limit: process_limit,
                tmpfs: vec![
                    (
                        "/tmp".to_string(),
                        format!("rw,noexec,nosuid,size={tmp_bytes}"),
                    ),
                    (
                        SPOOL_MOUNT.to_string(),
                        format!(
                            "rw,noexec,nosuid,size={spool_bytes},uid={uid},gid={RUNTIME_GID},mode=0700"
                        ),
                    ),
                ],
            },
        }
    }
}

pub struct DockerProxyLauncher<P> {
    proxy: P,
    config: ContainerConfig,
}

impl<P: DockerProxy> DockerProxyLauncher<P> {
    pub fn new(proxy: P, config: ContainerConfig) -> Self {
        Self { proxy, config }
    }

    #[cfg(any(test, feature = "test-fakes"))]
    pub fn proxy(&self) -> &P {
        &self.proxy
    }

    /// Removes the container once: `wait` and `stop` may both see the exit, and the first one
    /// to arrive deletes. A failure is recorded and does not hide the exit.
    async fn clean_up(&self, handle: &DockerRuntime) {
        if handle.cleaned.swap(true, Ordering::AcqRel) {
            return;
        }
        self.delete_recording_failure(&handle.id, handle.placement_id)
            .await;
    }

    async fn delete_recording_failure(&self, id: &ContainerId, placement: PlacementId) {
        if let Err(error) = self.proxy.delete(id).await {
            df_observe::record_audit_line(
                "RUNNER",
                format!("RUNNER delete status=failed placement={placement} error={error:?}"),
            );
        }
    }
}

pub struct DockerRuntime {
    id: ContainerId,
    placement_id: PlacementId,
    cleaned: AtomicBool,
}

impl<P: DockerProxy + Sync> RuntimeLauncher for DockerProxyLauncher<P> {
    type Handle = DockerRuntime;

    async fn start(&self, spec: &RuntimeSpec) -> Result<DockerRuntime, LaunchError> {
        if !(1..=MAXIMUM_SLOT).contains(&spec.slot) {
            return Err(LaunchError::SlotOutOfRange);
        }
        let body = self.config.create_body(spec);
        let id = self.proxy.create(&body).await.map_err(LaunchError::Proxy)?;
        if let Err(error) = self.proxy.start(&id).await {
            // The created container would hold the name and block the next attempt.
            self.delete_recording_failure(&id, spec.placement_id).await;
            return Err(LaunchError::Proxy(error));
        }
        df_observe::record_audit_line(
            "RUNNER",
            format!(
                "RUNNER launch status=started launcher=docker placement={}",
                spec.placement_id
            ),
        );
        Ok(DockerRuntime {
            id,
            placement_id: spec.placement_id,
            cleaned: AtomicBool::new(false),
        })
    }

    async fn wait(&self, handle: &DockerRuntime) -> Result<ExitReport, LaunchError> {
        let report = self
            .proxy
            .wait(&handle.id)
            .await
            .map_err(LaunchError::Proxy)?;
        self.clean_up(handle).await;
        Ok(report)
    }

    async fn stop(
        &self,
        handle: &DockerRuntime,
        grace: Duration,
    ) -> Result<ExitReport, LaunchError> {
        self.proxy
            .stop(&handle.id, grace)
            .await
            .map_err(LaunchError::Proxy)?;
        let report = self
            .proxy
            .wait(&handle.id)
            .await
            .map_err(LaunchError::Proxy)?;
        self.clean_up(handle).await;
        df_observe::record_audit_line(
            "RUNNER",
            format!(
                "RUNNER stop status=requested placement={} exit_code={}",
                handle.placement_id,
                report
                    .exit_code
                    .map_or_else(|| "none".to_string(), |code| code.to_string()),
            ),
        );
        Ok(report)
    }
}
