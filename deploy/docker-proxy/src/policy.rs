//! The policy of the Docker API proxy (LAUNCH.md decision 18; planning/hosted-runtime-research.md section 7).
//!
//! The trusted runner may start, stop, kill, inspect, wait for and delete CAMPAIGN RUNTIME containers and nothing else.
//! This module decides, from the method, the target and the request body alone, whether one request may reach the Docker
//! daemon. It is pure (no I/O, no clock) so every rule is a unit test. Everything not listed is refused: unknown paths,
//! unknown JSON keys, unknown host settings. A container is addressed by NAME only, and only names that begin with
//! `df-rt-`, so the runner can never act on the control plane, the dispatcher, the proxy or any other container.

use serde_json::{Map, Value};
use std::sync::Arc;

/// What the policy needs to know about a path on the host (the one thing it cannot decide from the request alone).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    File,
    Dir,
    Symlink,
    Other,
}

/// The result of `symlink_metadata` (it never follows a symlink): the kind of the object and its owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Meta {
    pub kind: Kind,
    pub uid: u32,
}

/// Looks at one path without following a symlink. The production implementation is `FsProbe`; tests use a map.
pub trait Probe: Send + Sync + std::fmt::Debug {
    fn symlink_metadata(&self, path: &str) -> Result<Meta, String>;
}

/// The real filesystem (unix only; elsewhere every lookup fails, so every bind is refused).
#[derive(Debug)]
pub struct FsProbe;

impl Probe for FsProbe {
    fn symlink_metadata(&self, path: &str) -> Result<Meta, String> {
        let metadata = std::fs::symlink_metadata(path).map_err(|error| error.kind().to_string())?;
        let file_type = metadata.file_type();
        let kind = if file_type.is_symlink() {
            Kind::Symlink
        } else if file_type.is_dir() {
            Kind::Dir
        } else if file_type.is_file() {
            Kind::File
        } else {
            Kind::Other
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(Meta {
                kind,
                uid: metadata.uid(),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = kind;
            Err("owner lookup needs a unix host".to_string())
        }
    }
}

/// Settings the policy needs. `allowed_images` is read from the root-owned allow-list on every request.
#[derive(Clone, Debug)]
pub struct Policy {
    /// Exact image references (`repository:tag@sha256:digest`) the runner may start, from `allowed-images`.
    pub allowed_images: Vec<String>,
    /// Slot `n` runs as uid `uid_base + n`.
    pub uid_base: u32,
    /// Highest slot number.
    pub max_slot: u32,
    /// The shared runtime group id.
    pub gid: u32,
    /// Slot `n`'s network is `network_prefix` + n and its subnet is `172.30.(subnet_base + n).0/24`.
    pub network_prefix: String,
    pub subnet_base: u32,
    /// Largest memory limit and process limit a runtime may be given.
    pub max_memory_bytes: u64,
    pub max_pids: i64,
    /// How bind sources are inspected (see `verify_bind_source`).
    pub probe: Arc<dyn Probe>,
}

impl Policy {
    /// The values of the staging and production run definitions.
    pub fn standard(allowed_images: Vec<String>, probe: Arc<dyn Probe>) -> Self {
        Policy {
            allowed_images,
            uid_base: 20_000,
            max_slot: 99,
            gid: 10_002,
            network_prefix: "df-slot-".to_string(),
            subnet_base: 10,
            max_memory_bytes: 2 * 1024 * 1024 * 1024,
            max_pids: 1024,
            probe,
        }
    }
}

/// Why a request was refused. The text is for the log and for the 403 body; it never contains the request body.
#[derive(Debug, PartialEq, Eq)]
pub struct Refusal(pub String);

fn refuse<T>(reason: impl Into<String>) -> Result<T, Refusal> {
    Err(Refusal(reason.into()))
}

/// Decide one request. `target` is the request target exactly as received (path and optional query).
pub fn check(policy: &Policy, method: &str, target: &str, body: &[u8]) -> Result<(), Refusal> {
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path, query),
        None => (target, ""),
    };
    if !path.is_ascii()
        || path.contains('%')
        || path.contains("..")
        || path.contains("//")
        || path.contains(';')
    {
        return refuse("the path contains an encoded, relative or non-ASCII part");
    }
    let path = strip_version(path);
    let query = parse_query(query)?;

    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match (method, segments.as_slice()) {
        ("GET", ["_ping"]) | ("HEAD", ["_ping"]) => {
            no_body(body)?;
            only_params(&query, &[])
        }
        ("POST", ["containers", "create"]) => {
            only_params(&query, &["name"])?;
            let name = param(&query, "name")
                .ok_or_else(|| Refusal("a runtime container needs a name".into()))?;
            runtime_name(name)?;
            create_body(policy, name, body)
        }
        ("GET", ["containers", "json"]) => {
            no_body(body)?;
            only_params(&query, &["all", "filters"])?;
            // Listing is allowed only when it is filtered to runtime containers by label.
            let filters = param(&query, "filters")
                .ok_or_else(|| Refusal("listing needs a label filter".into()))?;
            if filters_label_ok(filters) {
                Ok(())
            } else {
                refuse("listing must filter by label com.dungeonflux.role=runtime")
            }
        }
        ("POST", ["containers", name, "start"]) => container_call(name, body, &query, &[]),
        ("POST", ["containers", name, "wait"]) => {
            container_call(name, body, &query, &["condition"])
        }
        ("POST", ["containers", name, "stop"]) => {
            container_call(name, body, &query, &["t", "signal"])?;
            if let Some(t) = param(&query, "t") {
                match t.parse::<u32>() {
                    Ok(t) if t <= 300 => {}
                    _ => return refuse("stop timeout must be a number of seconds up to 300"),
                }
            }
            if let Some(signal) = param(&query, "signal") {
                signal_ok(signal)?;
            }
            Ok(())
        }
        ("POST", ["containers", name, "kill"]) => {
            container_call(name, body, &query, &["signal"])?;
            if let Some(signal) = param(&query, "signal") {
                signal_ok(signal)?;
            }
            Ok(())
        }
        ("GET", ["containers", name, "json"]) => container_call(name, body, &query, &[]),
        ("GET", ["containers", name, "stats"]) => {
            container_call(name, body, &query, &["stream", "one-shot"])?;
            // Usage is polled; a streaming stats connection would hold the proxy open forever.
            match param(&query, "stream") {
                Some("false") | Some("0") => Ok(()),
                _ => refuse("stats must be requested with stream=false"),
            }
        }
        ("DELETE", ["containers", name]) => {
            container_call(name, body, &query, &["force", "v"])?;
            Ok(())
        }
        _ => refuse(format!("{method} {path} is not a call the runner may make")),
    }
}

fn container_call(
    name: &str,
    body: &[u8],
    query: &[(String, String)],
    allowed: &[&str],
) -> Result<(), Refusal> {
    runtime_name(name)?;
    no_body(body)?;
    only_params(query, allowed)
}

fn runtime_name(name: &str) -> Result<(), Refusal> {
    let ok = name.len() >= 8
        && name.len() <= 100
        && name.starts_with("df-rt-")
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if ok {
        Ok(())
    } else {
        refuse("a container name must be df-rt- followed by lowercase letters, digits and dashes")
    }
}

fn signal_ok(signal: &str) -> Result<(), Refusal> {
    match signal {
        "SIGTERM" | "SIGKILL" | "SIGINT" | "SIGHUP" | "TERM" | "KILL" | "INT" | "HUP" | "15"
        | "9" | "2" | "1" => Ok(()),
        _ => refuse("signal not allowed"),
    }
}

fn strip_version(path: &str) -> &str {
    // /v1.45/containers/... -> /containers/...
    if let Some(rest) = path.strip_prefix("/v1.") {
        if let Some((digits, tail)) = rest.split_once('/') {
            if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
                return &path[path.len() - tail.len() - 1..];
            }
        }
    }
    path
}

fn no_body(body: &[u8]) -> Result<(), Refusal> {
    if body.is_empty() {
        Ok(())
    } else {
        refuse("this call takes no request body")
    }
}

fn parse_query(query: &str) -> Result<Vec<(String, String)>, Refusal> {
    let mut pairs = Vec::new();
    if query.is_empty() {
        return Ok(pairs);
    }
    for part in query.split('&') {
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        let decoded = percent_decode(value)?;
        pairs.push((key.to_string(), decoded));
    }
    Ok(pairs)
}

fn percent_decode(value: &str) -> Result<String, Refusal> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                if i + 2 >= bytes.len() {
                    return refuse("bad percent escape in the query");
                }
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                    .map_err(|_| Refusal("bad percent escape".into()))?;
                let byte = u8::from_str_radix(hex, 16)
                    .map_err(|_| Refusal("bad percent escape".into()))?;
                out.push(byte);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| Refusal("the query is not UTF-8".into()))
}

fn param<'a>(query: &'a [(String, String)], key: &str) -> Option<&'a str> {
    query
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn only_params(query: &[(String, String)], allowed: &[&str]) -> Result<(), Refusal> {
    for (key, _) in query {
        if !allowed.contains(&key.as_str()) {
            return refuse(format!("query parameter {key} is not allowed here"));
        }
    }
    Ok(())
}

fn filters_label_ok(filters: &str) -> bool {
    match crate::strict::parse_str(filters) {
        Ok(Value::Object(map)) => {
            map.len() == 1
                && match map.get("label") {
                    Some(Value::Array(labels)) => {
                        labels.len() == 1 && labels[0] == "com.dungeonflux.role=runtime"
                    }
                    Some(Value::Object(labels)) => {
                        labels.len() == 1
                            && labels.get("com.dungeonflux.role=runtime")
                                == Some(&Value::Bool(true))
                    }
                    _ => false,
                }
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// containers/create

const TOP_LEVEL_KEYS: &[&str] = &[
    "Image",
    "Env",
    "Cmd",
    "User",
    "Labels",
    "HostConfig",
    "NetworkingConfig",
    "StopTimeout",
    "AttachStdout",
    "AttachStderr",
    "Tty",
];

/// The only SecurityOpt strings a runtime create may carry, compared with ==.
const ALLOWED_SECURITY_OPT: &[&str] = &["no-new-privileges", "no-new-privileges:true"];

const HOST_CONFIG_KEYS: &[&str] = &[
    "Binds",
    "Tmpfs",
    "NetworkMode",
    "Privileged",
    "CapAdd",
    "CapDrop",
    "Devices",
    "PidMode",
    "IpcMode",
    "UTSMode",
    "UsernsMode",
    "CgroupnsMode",
    "ReadonlyRootfs",
    "SecurityOpt",
    "Memory",
    "MemorySwap",
    "NanoCpus",
    "PidsLimit",
    "AutoRemove",
    "Init",
    "RestartPolicy",
    "PublishAllPorts",
    "ShmSize",
    "OomKillDisable",
    "Runtime",
];

fn create_body(policy: &Policy, name: &str, body: &[u8]) -> Result<(), Refusal> {
    // Strict parse: a repeated key anywhere, trailing data or invalid JSON is a refusal (see strict.rs).
    let value: Value = crate::strict::parse(body)
        .map_err(|why| Refusal(format!("the create body is not strict JSON: {why}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| Refusal("the create body is not a JSON object".into()))?;

    for key in object.keys() {
        if !TOP_LEVEL_KEYS.contains(&key.as_str()) {
            return refuse(format!("create body key {key} is not allowed"));
        }
    }

    // The image must be one of the digests the deploy tool verified.
    let image = object
        .get("Image")
        .and_then(Value::as_str)
        .ok_or_else(|| Refusal("create needs an Image".into()))?;
    if !image.contains("@sha256:") || !policy.allowed_images.iter().any(|allowed| allowed == image)
    {
        return refuse("the image is not in the allow-list of verified digests");
    }

    // The user fixes the slot; everything else must agree with it.
    let user = object
        .get("User")
        .and_then(Value::as_str)
        .ok_or_else(|| Refusal("create needs a numeric User".into()))?;
    let (uid, gid) = parse_user(user)?;
    if gid != policy.gid {
        return refuse("the group id is not the runtime group");
    }
    if uid <= policy.uid_base || uid > policy.uid_base + policy.max_slot {
        return refuse("the user id is outside the runtime slot range");
    }
    let slot = uid - policy.uid_base;

    // The container name carries the slot too, so a name cannot be reused for another slot.
    if !name.ends_with(&format!("-slot{slot}")) && !name.contains(&format!("-slot{slot}-")) {
        return refuse("the container name must carry the slot (-slot<N>)");
    }

    match object.get("Labels") {
        Some(Value::Object(labels)) => labels_ok(labels)?,
        _ => return refuse("create needs the label com.dungeonflux.role=runtime"),
    }
    if let Some(cmd) = object.get("Cmd") {
        let expected = [
            "campaign-runtime",
            "--descriptor",
            "/etc/dungeonflux/runtime.toml",
        ];
        let ok = cmd.as_array().is_some_and(|a| {
            a.len() == expected.len() && a.iter().zip(expected).all(|(x, e)| x == e)
        });
        if !ok {
            return refuse("Cmd must be absent or exactly the runtime command line");
        }
    }
    // The runtime starts with an EMPTY environment: its configuration is the descriptor file, never variables.
    if !empty_or_absent(object, "Env") {
        return refuse("Env must be empty");
    }
    for key in ["AttachStdout", "AttachStderr", "Tty"] {
        if object.get(key).is_some_and(|v| v != &Value::Bool(false)) {
            return refuse(format!("{key} must be false or absent"));
        }
    }
    if let Some(timeout) = object.get("StopTimeout") {
        match timeout.as_u64() {
            Some(t) if t <= 300 => {}
            _ => return refuse("StopTimeout must be at most 300"),
        }
    }

    let host = object
        .get("HostConfig")
        .and_then(Value::as_object)
        .ok_or_else(|| Refusal("create needs a HostConfig".into()))?;
    host_config(policy, slot, uid, host)?;
    networking_config(policy, slot, object.get("NetworkingConfig"))
}

fn parse_user(user: &str) -> Result<(u32, u32), Refusal> {
    let (uid, gid) = user
        .split_once(':')
        .ok_or_else(|| Refusal("User must be uid:gid".into()))?;
    let uid = uid
        .parse::<u32>()
        .map_err(|_| Refusal("User must be numeric".into()))?;
    let gid = gid
        .parse::<u32>()
        .map_err(|_| Refusal("User must be numeric".into()))?;
    Ok((uid, gid))
}

fn labels_ok(labels: &Map<String, Value>) -> Result<(), Refusal> {
    if labels.get("com.dungeonflux.role") != Some(&Value::String("runtime".into())) {
        return refuse("create needs the label com.dungeonflux.role=runtime");
    }
    for (key, value) in labels {
        if !key.starts_with("com.dungeonflux.") || !value.is_string() {
            return refuse("labels must be com.dungeonflux.* strings");
        }
    }
    Ok(())
}

fn empty_or_absent(host: &Map<String, Value>, key: &str) -> bool {
    match host.get(key) {
        None | Some(Value::Null) => true,
        Some(Value::Array(a)) => a.is_empty(),
        Some(Value::String(s)) => s.is_empty(),
        Some(Value::Object(o)) => o.is_empty(),
        Some(_) => false,
    }
}

fn private_namespace(host: &Map<String, Value>, key: &str) -> bool {
    match host.get(key) {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => s.is_empty() || s == "private",
        Some(_) => false,
    }
}

fn host_config(
    policy: &Policy,
    slot: u32,
    uid: u32,
    host: &Map<String, Value>,
) -> Result<(), Refusal> {
    for key in host.keys() {
        if !HOST_CONFIG_KEYS.contains(&key.as_str()) {
            return refuse(format!("HostConfig key {key} is not allowed"));
        }
    }
    if host
        .get("Privileged")
        .is_some_and(|v| v != &Value::Bool(false))
    {
        return refuse("Privileged containers are not allowed");
    }
    if !empty_or_absent(host, "CapAdd") {
        return refuse("CapAdd must be empty");
    }
    match host.get("CapDrop").and_then(Value::as_array) {
        Some(drop) if drop.len() == 1 && drop[0] == "ALL" => {}
        _ => return refuse("CapDrop must be exactly [\"ALL\"]"),
    }
    if !empty_or_absent(host, "Devices") {
        return refuse("devices are not allowed");
    }
    for key in ["PidMode", "IpcMode", "UTSMode", "UsernsMode"] {
        if !private_namespace(host, key) {
            return refuse(format!("{key} must be private"));
        }
    }
    if !(private_namespace(host, "CgroupnsMode")) {
        return refuse("CgroupnsMode must be private");
    }
    if host.get("ReadonlyRootfs") != Some(&Value::Bool(true)) {
        return refuse("ReadonlyRootfs must be true");
    }
    let wanted_network = format!("{}{}", policy.network_prefix, slot);
    if host.get("NetworkMode").and_then(Value::as_str) != Some(wanted_network.as_str()) {
        return refuse("NetworkMode must be the runtime network of this slot");
    }
    // SecurityOpt is an exact ALLOWLIST: no-new-privileges is required and nothing else is accepted (no seccomp, apparmor, label or
    // systempaths option, in any spelling). If a custom profile is ever shipped, add its exact string to ALLOWED_SECURITY_OPT.
    match host.get("SecurityOpt").and_then(Value::as_array) {
        Some(options)
            if !options.is_empty()
                && options.iter().all(|o| {
                    o.as_str()
                        .is_some_and(|o| ALLOWED_SECURITY_OPT.contains(&o))
                }) => {}
        _ => {
            return refuse(
                "SecurityOpt must be exactly no-new-privileges (no other option is allowed)",
            );
        }
    }
    match host.get("Memory").and_then(Value::as_u64) {
        Some(m) if m > 0 && m <= policy.max_memory_bytes => {}
        _ => return refuse("Memory must be set and within the configured maximum"),
    }
    if host
        .get("MemorySwap")
        .is_some_and(|v| v != host.get("Memory").unwrap_or(&Value::Null) && v != &Value::from(0))
    {
        return refuse("MemorySwap must equal Memory");
    }
    match host.get("NanoCpus").and_then(Value::as_u64) {
        Some(n) if n > 0 && n <= 8_000_000_000 => {}
        _ => return refuse("NanoCpus must be set (at most 8 CPUs)"),
    }
    match host.get("PidsLimit").and_then(Value::as_i64) {
        Some(p) if p > 0 && p <= policy.max_pids => {}
        _ => return refuse("PidsLimit must be set within the configured maximum"),
    }
    if host
        .get("PublishAllPorts")
        .is_some_and(|v| v != &Value::Bool(false))
    {
        return refuse("ports cannot be published");
    }
    if host
        .get("OomKillDisable")
        .is_some_and(|v| v != &Value::Bool(false))
    {
        return refuse("OomKillDisable is not allowed");
    }
    if let Some(runtime) = host.get("Runtime") {
        if runtime != "runc" && runtime != "" {
            return refuse("only the default runtime is allowed");
        }
    }
    if let Some(policy_object) = host.get("RestartPolicy") {
        let object = policy_object
            .as_object()
            .ok_or_else(|| Refusal("RestartPolicy must be an object".into()))?;
        if object
            .keys()
            .any(|k| k != "Name" && k != "MaximumRetryCount")
            || object
                .get("MaximumRetryCount")
                .is_some_and(|v| v.as_u64() != Some(0))
        {
            return refuse("RestartPolicy may only carry Name and a zero MaximumRetryCount");
        }
        if !matches!(
            object.get("Name").and_then(Value::as_str),
            None | Some("") | Some("no")
        ) {
            return refuse("containers are not restarted by Docker: the runner supervises them");
        }
    }
    for key in ["Init", "AutoRemove"] {
        if host.get(key).is_some_and(|v| !v.is_boolean()) {
            return refuse(format!("{key} must be a boolean"));
        }
    }
    if let Some(shm) = host.get("ShmSize") {
        match shm.as_u64() {
            Some(s) if s <= 64 * 1024 * 1024 => {}
            _ => return refuse("ShmSize is limited to 64 MiB"),
        }
    }

    // Mounts: the descriptor and the secrets directory of THIS slot, and nothing else. There is NO persistent spool bind: a spool that
    // outlives the container would hand the next tenant of the slot (the same uid) the previous tenant's spooled data, and a bind under
    // /run has no quota. The spool is a per-container Tmpfs with a size, below. The string must match exactly,
    // AND the host path behind it must be what it claims (`verify_bind_source`): a string comparison alone would let a symlink
    // planted at one of these paths mount any host directory.
    let root = format!("/run/dungeonflux/runtime/slot{slot}");
    let allowed_binds: [(String, String, Kind); 2] = [
        (
            format!("{root}/descriptor.toml:/etc/dungeonflux/runtime.toml:ro"),
            format!("{root}/descriptor.toml"),
            Kind::File,
        ),
        (
            format!("{root}/secrets:/run/secrets:ro"),
            format!("{root}/secrets"),
            Kind::Dir,
        ),
    ];
    if let Some(binds) = host.get("Binds") {
        let binds = binds
            .as_array()
            .ok_or_else(|| Refusal("Binds must be a list".into()))?;
        for bind in binds {
            let bind = bind
                .as_str()
                .ok_or_else(|| Refusal("a bind must be a string".into()))?;
            let Some((_, source, kind)) =
                allowed_binds.iter().find(|(allowed, _, _)| allowed == bind)
            else {
                return refuse(
                    "a bind mount is not the descriptor or secrets directory of this slot",
                );
            };
            verify_bind_source(policy, source, *kind, uid)?;
        }
    }
    if let Some(tmpfs) = host.get("Tmpfs") {
        let tmpfs = tmpfs
            .as_object()
            .ok_or_else(|| Refusal("Tmpfs must be an object".into()))?;
        for (path, options) in tmpfs {
            if path != "/tmp" && path != "/var/lib/dungeonflux/spool" {
                return refuse("a tmpfs mount is not /tmp or the spool directory");
            }
            tmpfs_options_ok(options.as_str().unwrap_or("\u{0}"), uid, policy.gid)?;
        }
    }
    Ok(())
}

/// Walks every component of a bind source with `symlink_metadata` and refuses a symlink anywhere on the path. Every directory above
/// the leaf must be a directory owned by root (the helper that creates `/run/dungeonflux/runtime` and each `slotN` is root, and the
/// runner cannot rename or replace them); the leaf must be the expected type and be owned by the slot's uid, so the runtime that
/// receives the mount is the owner of what it gets.
///
/// RESIDUAL (time of check to time of use): the proxy checks, then Docker mounts, and a path could change in between. That race is
/// closed by the directory ownership above, not by this code: no process the runner controls can change an entry of
/// `/run/dungeonflux/runtime` or of a `slotN` directory, and the proxy sees the tree read-only. This check catches a bad tree; the
/// permissions keep it from becoming one.
fn verify_bind_source(
    policy: &Policy,
    source: &str,
    expected: Kind,
    owner: u32,
) -> Result<(), Refusal> {
    let components: Vec<&str> = source.split('/').filter(|c| !c.is_empty()).collect();
    if !source.starts_with('/')
        || components.iter().any(|c| *c == "." || *c == "..")
        || components.is_empty()
    {
        return refuse("a bind source must be an absolute path without . or .. components");
    }
    let mut path = String::new();
    for (index, component) in components.iter().enumerate() {
        path.push('/');
        path.push_str(component);
        let meta = policy
            .probe
            .symlink_metadata(&path)
            .map_err(|error| Refusal(format!("cannot inspect the bind source {path}: {error}")))?;
        if meta.kind == Kind::Symlink {
            return refuse(format!("the bind source {path} is a symlink"));
        }
        if index + 1 == components.len() {
            if meta.kind != expected {
                return refuse(format!(
                    "the bind source {path} is not a {}",
                    if expected == Kind::File {
                        "regular file"
                    } else {
                        "directory"
                    }
                ));
            }
            if meta.uid != owner {
                return refuse(format!(
                    "the bind source {path} is not owned by the slot's uid"
                ));
            }
        } else if meta.kind != Kind::Dir || meta.uid != 0 {
            return refuse(format!("{path} must be a directory owned by root"));
        }
    }
    Ok(())
}

/// The largest tmpfs a runtime may have (it is memory-accounted against the container).
const MAX_TMPFS_BYTES: u64 = 1024 * 1024 * 1024;

/// `size=` as Docker reads it: digits with an optional lowercase k, m or g suffix (bare digits are bytes).
fn parse_tmpfs_size(value: &str) -> Option<u64> {
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

fn tmpfs_options_ok(options: &str, uid: u32, gid: u32) -> Result<(), Refusal> {
    let mut sized = false;
    for option in options.split(',') {
        let (key, value) = option.split_once('=').unwrap_or((option, ""));
        match key {
            "size"
                if parse_tmpfs_size(value)
                    .is_some_and(|bytes| bytes > 0 && bytes <= MAX_TMPFS_BYTES) =>
            {
                sized = true;
            }
            "uid" if value == uid.to_string() => {}
            "gid" if value == gid.to_string() => {}
            "mode" if value.len() <= 4 && value.bytes().all(|b| (b'0'..=b'7').contains(&b)) => {}
            "rw" | "noexec" | "nosuid" | "nodev" if value.is_empty() => {}
            _ => {
                return refuse(
                    "a tmpfs option is not one of rw, size, uid, gid, mode, noexec, nosuid, nodev with this slot's ids",
                );
            }
        }
    }
    // Every tmpfs carries a size: an unsized one would default to half the host's memory.
    if !sized {
        return refuse("a tmpfs mount must carry a size of at most 1 GiB");
    }
    Ok(())
}

fn networking_config(policy: &Policy, slot: u32, config: Option<&Value>) -> Result<(), Refusal> {
    let Some(config) = config else { return Ok(()) };
    let object = config
        .as_object()
        .ok_or_else(|| Refusal("NetworkingConfig must be an object".into()))?;
    for key in object.keys() {
        if key != "EndpointsConfig" {
            return refuse("NetworkingConfig may only carry EndpointsConfig");
        }
    }
    let Some(endpoints) = object.get("EndpointsConfig") else {
        return Ok(());
    };
    let endpoints = endpoints
        .as_object()
        .ok_or_else(|| Refusal("EndpointsConfig must be an object".into()))?;
    let network = format!("{}{}", policy.network_prefix, slot);
    for (name, endpoint) in endpoints {
        if name != &network {
            return refuse("an endpoint is not on this slot's network");
        }
        let endpoint = endpoint
            .as_object()
            .ok_or_else(|| Refusal("an endpoint must be an object".into()))?;
        for (key, value) in endpoint {
            match key.as_str() {
                "IPAMConfig" => {
                    let ipam = value
                        .as_object()
                        .ok_or_else(|| Refusal("IPAMConfig must be an object".into()))?;
                    for (k, v) in ipam {
                        let expected = format!("172.30.{}.10", policy.subnet_base + slot);
                        if k != "IPv4Address" || v.as_str() != Some(expected.as_str()) {
                            return refuse("the runtime address must be 172.30.<10 + slot>.10");
                        }
                    }
                }
                _ => return refuse("an endpoint may only carry IPAMConfig"),
            }
        }
    }
    Ok(())
}

/// The same decision as `check`, and, when the request is allowed, the body to FORWARD: the canonical serialisation of the value the
/// policy parsed (empty when the request has no body). The proxy never forwards the bytes it received: the daemon must decode exactly
/// the value that was checked, whatever the sender's key order, spacing, escapes or duplicate keys.
pub fn canonical_body(
    policy: &Policy,
    method: &str,
    target: &str,
    body: &[u8],
) -> Result<Vec<u8>, Refusal> {
    check(policy, method, target, body)?;
    if body.is_empty() {
        return Ok(Vec::new());
    }
    let value = crate::strict::parse(body)
        .map_err(|why| Refusal(format!("the body is not strict JSON: {why}")))?;
    Ok(crate::strict::canonical(&value))
}
