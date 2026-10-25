//! A faithful fake of the body-checking Docker proxy: it enforces the rules of S2's proxy
//! (`deploy/docker-proxy/src/policy.rs`, hosted runtime design 7, D18) that concern the create
//! body and records every request, so a launcher test proves the runner's requests are
//! policy-compliant without a Docker daemon. `tests/proxy_contract.rs` keeps the fake honest:
//! it runs the proxy's real `policy::check` on the same bodies and compares the verdicts.

use std::{sync::Mutex, time::Duration};

use crate::{
    docker::{
        ContainerConfig, ContainerId, CreateBody, DockerProxy, ImageReference,
        MAXIMUM_MEMORY_BYTES, MAXIMUM_NAME_LENGTH, MAXIMUM_NANO_CPUS, MAXIMUM_PIDS, MAXIMUM_SLOT,
        MAXIMUM_TMPFS_BYTES, PolicyRule, ProxyError, ROLE_LABEL, RUNTIME_GID, RUNTIME_UID_BASE,
        SLOT_NETWORK_PREFIX,
    },
    launcher::ExitReport,
    spec::{DESCRIPTOR_MOUNT, SECRETS_MOUNT, SPOOL_MOUNT},
};

/// One request the proxy accepted, in order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Request {
    Create(Box<CreateBody>),
    Start(ContainerId),
    Stop(ContainerId, Duration),
    Wait(ContainerId),
    Delete(ContainerId),
}

pub struct FakeDockerProxy {
    allowed_images: Vec<ImageReference>,
    exit_code: i32,
    fail_start: bool,
    requests: Mutex<Vec<Request>>,
}

/// The slot a `User` names, when it is a runtime slot's `uid:gid`.
fn slot_of(body: &CreateBody) -> Option<u32> {
    let (uid, group) = body.user.split_once(':')?;
    let (uid, group): (u32, u32) = (uid.parse().ok()?, group.parse().ok()?);
    let slot = uid.checked_sub(RUNTIME_UID_BASE)?;
    (group == RUNTIME_GID && (1..=u32::from(MAXIMUM_SLOT)).contains(&slot)).then_some(slot)
}

/// `size=` as Docker reads it: digits with an optional lowercase k, m or g suffix.
fn tmpfs_size(value: &str) -> Option<u64> {
    let (digits, factor) = match value.as_bytes().last()? {
        b'k' => (&value[..value.len() - 1], 1024u64),
        b'm' => (&value[..value.len() - 1], 1024 * 1024),
        b'g' => (&value[..value.len() - 1], 1024 * 1024 * 1024),
        _ => (value, 1),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) || digits.len() > 12 {
        return None;
    }
    digits.parse::<u64>().ok()?.checked_mul(factor)
}

/// Every option is one of rw, size, uid, gid, mode, noexec, nosuid, nodev with this slot's ids,
/// and a size of at most 1 GiB is present.
fn tmpfs_options_allowed(options: &str, uid: u32) -> bool {
    let mut sized = false;
    for option in options.split(',') {
        let (key, value) = option.split_once('=').unwrap_or((option, ""));
        match key {
            "size" if tmpfs_size(value).is_some_and(|b| b > 0 && b <= MAXIMUM_TMPFS_BYTES) => {
                sized = true;
            }
            "uid" if value == uid.to_string() => {}
            "gid" if value == RUNTIME_GID.to_string() => {}
            "mode" if value.len() <= 4 && value.bytes().all(|b| (b'0'..=b'7').contains(&b)) => {}
            "rw" | "noexec" | "nosuid" | "nodev" if value.is_empty() => {}
            _ => return false,
        }
    }
    sized
}

impl FakeDockerProxy {
    /// A proxy enforcing the deployed settings; containers it starts exit with `exit_code`.
    pub fn enforcing(config: &ContainerConfig, exit_code: i32) -> Self {
        Self {
            allowed_images: config.allowed_images().to_vec(),
            exit_code,
            fail_start: false,
            requests: Mutex::new(Vec::new()),
        }
    }

    /// The same proxy, but `start` fails after a successful `create`.
    pub fn failing_to_start(mut self) -> Self {
        self.fail_start = true;
        self
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn record(&self, request: Request) {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request);
    }

    /// The first policy rule the body breaks.
    pub fn refusal(&self, body: &CreateBody) -> Option<PolicyRule> {
        let host = &body.host_config;
        let Some(slot) = slot_of(body) else {
            return Some(PolicyRule::UserOutsideSlotRange);
        };
        let uid = RUNTIME_UID_BASE + slot;
        let root = format!("/run/dungeonflux/runtime/slot{slot}");
        let wanted_binds = [
            format!("{root}/descriptor.toml:{DESCRIPTOR_MOUNT}:ro"),
            format!("{root}/secrets:{SECRETS_MOUNT}:ro"),
        ];
        let name_ok = body.name.len() >= 8
            && body.name.len() <= MAXIMUM_NAME_LENGTH
            && body.name.starts_with("df-rt-")
            && body
                .name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && (body.name.ends_with(&format!("-slot{slot}"))
                || body.name.contains(&format!("-slot{slot}-")));
        let label_ok = body
            .labels
            .iter()
            .any(|(key, value)| (key.as_str(), value.as_str()) == ROLE_LABEL);
        let image_allowed = self
            .allowed_images
            .iter()
            .any(|image| image.as_str() == body.image);
        let tmpfs_path_ok = host
            .tmpfs
            .iter()
            .all(|(path, _)| path == "/tmp" || path == SPOOL_MOUNT);
        let tmpfs_options_ok = host
            .tmpfs
            .iter()
            .all(|(_, options)| tmpfs_options_allowed(options, uid));
        let rules = [
            (!image_allowed, PolicyRule::ImageNotAllowListed),
            (!name_ok, PolicyRule::NameMustCarryTheSlot),
            (!label_ok, PolicyRule::RoleLabelMissing),
            (
                body.cmd != ["campaign-runtime", "--descriptor", DESCRIPTOR_MOUNT],
                PolicyRule::CommandNotTheRuntimeCommand,
            ),
            (!body.env.is_empty(), PolicyRule::EnvironmentNotEmpty),
            (host.privileged, PolicyRule::Privileged),
            (!host.cap_add.is_empty(), PolicyRule::CapabilitiesAdded),
            (
                host.cap_drop != ["ALL"],
                PolicyRule::CapabilitiesNotAllDropped,
            ),
            (!host.devices.is_empty(), PolicyRule::DevicesPresent),
            (
                !(host.pid_mode.is_empty() || host.pid_mode == "private"),
                PolicyRule::PidNamespaceNotPrivate,
            ),
            (
                !(host.ipc_mode.is_empty() || host.ipc_mode == "private"),
                PolicyRule::IpcNamespaceNotPrivate,
            ),
            (!host.readonly_rootfs, PolicyRule::RootNotReadOnly),
            (
                host.network_mode != format!("{SLOT_NETWORK_PREFIX}{slot}"),
                PolicyRule::NetworkNotTheSlotNetwork,
            ),
            (
                host.security_opt.is_empty()
                    || !host
                        .security_opt
                        .iter()
                        .all(|option| option == "no-new-privileges:true"),
                PolicyRule::SecurityOptionNotNoNewPrivileges,
            ),
            (
                host.memory_bytes == 0 || host.memory_bytes > MAXIMUM_MEMORY_BYTES,
                PolicyRule::MemoryUnsetOrAboveMaximum,
            ),
            (
                host.memory_swap_bytes != host.memory_bytes && host.memory_swap_bytes != 0,
                PolicyRule::SwapNotEqualToMemory,
            ),
            (
                host.nano_cpus == 0 || host.nano_cpus > MAXIMUM_NANO_CPUS,
                PolicyRule::CpuUnsetOrAboveMaximum,
            ),
            (
                host.pids_limit == 0 || host.pids_limit > MAXIMUM_PIDS,
                PolicyRule::ProcessLimitUnsetOrAboveMaximum,
            ),
            (
                !host.binds.iter().all(|bind| wanted_binds.contains(bind)),
                PolicyRule::MountNotTheSlotDescriptorOrSecrets,
            ),
            (!tmpfs_path_ok, PolicyRule::TmpfsPathNotAllowed),
            (!tmpfs_options_ok, PolicyRule::TmpfsOptionNotAllowed),
        ];
        rules
            .into_iter()
            .find_map(|(broken, rule)| broken.then_some(rule))
    }
}

impl DockerProxy for FakeDockerProxy {
    async fn create(&self, body: &CreateBody) -> Result<ContainerId, ProxyError> {
        if let Some(rule) = self.refusal(body) {
            return Err(ProxyError::Refused(rule));
        }
        self.record(Request::Create(Box::new(body.clone())));
        Ok(ContainerId(body.name.clone()))
    }

    async fn start(&self, id: &ContainerId) -> Result<(), ProxyError> {
        self.record(Request::Start(id.clone()));
        if self.fail_start {
            return Err(ProxyError::Unavailable);
        }
        Ok(())
    }

    async fn stop(&self, id: &ContainerId, grace: Duration) -> Result<(), ProxyError> {
        self.record(Request::Stop(id.clone(), grace));
        Ok(())
    }

    async fn wait(&self, id: &ContainerId) -> Result<ExitReport, ProxyError> {
        self.record(Request::Wait(id.clone()));
        Ok(ExitReport {
            exit_code: Some(self.exit_code),
        })
    }

    async fn delete(&self, id: &ContainerId) -> Result<(), ProxyError> {
        self.record(Request::Delete(id.clone()));
        Ok(())
    }
}
