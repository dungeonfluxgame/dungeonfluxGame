//! The runner's create body against the REAL proxy policy. `deploy/docker-proxy`'s `strict.rs`
//! and `policy.rs` are compiled into this test as they are (not copied), so the body the runner
//! builds is judged by the code that will judge it in production, and the fake proxy's verdicts
//! are compared with the real ones on a set of broken bodies.

#![cfg(not(target_arch = "wasm32"))]
#![allow(
    dead_code,
    reason = "the proxy's modules are compiled whole; the test uses part of them"
)]

#[path = "../../../deploy/docker-proxy/src/strict.rs"]
#[allow(
    clippy::all,
    reason = "S2's proxy source is compiled whole; it is linted in its own crate"
)]
mod strict;

#[path = "../../../deploy/docker-proxy/src/policy.rs"]
#[allow(
    clippy::all,
    reason = "S2's proxy source is compiled whole; it is linted in its own crate"
)]
mod policy;

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use df_runner::{
    docker::{ContainerConfig, CreateBody, DockerProxyLauncher, ImageReference},
    docker_fake::{FakeDockerProxy, Request},
    launcher::RuntimeLauncher,
    spec::{PlacementId, RuntimeLimits, RuntimeSpec},
};
use policy::{Kind, Meta, Policy, Probe};

const SLOT: u8 = 3;

/// A named way to break the runner's body.
type Change = (&'static str, Box<dyn Fn(&mut CreateBody)>);

#[derive(Debug)]
struct SlotTree(HashMap<String, Meta>);

impl Probe for SlotTree {
    fn symlink_metadata(&self, path: &str) -> Result<Meta, String> {
        self.0
            .get(path)
            .copied()
            .ok_or_else(|| "not found".to_string())
    }
}

/// The tree the root helper leaves for the slot: root-owned directories down to the slot, and
/// the descriptor and secrets owned by the slot's user.
fn slot_tree() -> SlotTree {
    let uid = 20_000 + u32::from(SLOT);
    let mut tree = HashMap::new();
    for directory in [
        "/run",
        "/run/dungeonflux",
        "/run/dungeonflux/runtime",
        "/run/dungeonflux/runtime/slot3",
    ] {
        tree.insert(
            directory.to_string(),
            Meta {
                kind: Kind::Dir,
                uid: 0,
            },
        );
    }
    tree.insert(
        "/run/dungeonflux/runtime/slot3/descriptor.toml".to_string(),
        Meta {
            kind: Kind::File,
            uid,
        },
    );
    tree.insert(
        "/run/dungeonflux/runtime/slot3/secrets".to_string(),
        Meta {
            kind: Kind::Dir,
            uid,
        },
    );
    SlotTree(tree)
}

fn image() -> ImageReference {
    ImageReference::new(&format!(
        "ghcr.io/example/df-runtime:release@sha256:{}",
        "a".repeat(64)
    ))
    .expect("reference")
}

/// The body the runner sends for slot 3, taken from the launcher itself.
fn runner_body() -> CreateBody {
    let config = ContainerConfig::new(image(), vec![image()]).expect("allowed");
    let launcher = DockerProxyLauncher::new(FakeDockerProxy::enforcing(&config, 0), config);
    let spec = RuntimeSpec {
        placement_id: PlacementId::new([7; 16]),
        slot: SLOT,
        descriptor_path: PathBuf::from("/run/dungeonflux/runtime/slot3/descriptor.toml"),
        limits: RuntimeLimits::default(),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime");
    runtime
        .block_on(launcher.start(&spec))
        .expect("the fake accepts the runner's body");
    launcher
        .proxy()
        .requests()
        .into_iter()
        .find_map(|request| match request {
            Request::Create(body) => Some(*body),
            _ => None,
        })
        .expect("a create request")
}

fn real_policy() -> Policy {
    Policy::standard(vec![image().as_str().to_string()], Arc::new(slot_tree()))
}

fn real_verdict(body: &CreateBody) -> Result<(), policy::Refusal> {
    let bytes = serde_json::to_vec(&body.to_docker_json()).expect("json");
    policy::check(&real_policy(), "POST", &body.target(), &bytes)
}

fn fake_refuses(body: &CreateBody) -> bool {
    let config = ContainerConfig::new(image(), vec![image()]).expect("allowed");
    FakeDockerProxy::enforcing(&config, 0)
        .refusal(body)
        .is_some()
}

#[test]
fn the_real_proxy_accepts_the_body_the_runner_builds() {
    let body = runner_body();
    assert_eq!(real_verdict(&body), Ok(()), "{:#}", body.to_docker_json());
}

#[test]
fn the_real_proxy_refuses_a_tmpfs_sent_as_an_array_so_the_object_form_is_required() {
    let body = runner_body();
    let mut json = body.to_docker_json();
    json["HostConfig"]["Tmpfs"] = serde_json::json!(["/tmp:rw,size=1024"]);
    let bytes = serde_json::to_vec(&json).expect("json");
    assert!(policy::check(&real_policy(), "POST", &body.target(), &bytes).is_err());
}

#[test]
fn the_fake_and_the_real_proxy_agree_on_every_broken_body() {
    let changes: Vec<Change> = vec![
        ("privileged", Box::new(|b| b.host_config.privileged = true)),
        (
            "added capability",
            Box::new(|b| b.host_config.cap_add.push("NET_ADMIN".into())),
        ),
        (
            "capabilities kept",
            Box::new(|b| b.host_config.cap_drop.clear()),
        ),
        (
            "a device",
            Box::new(|b| b.host_config.devices.push("/dev/kvm".into())),
        ),
        (
            "writable root",
            Box::new(|b| b.host_config.readonly_rootfs = false),
        ),
        (
            "another network",
            Box::new(|b| b.host_config.network_mode = "df-slot-4".into()),
        ),
        (
            "host network",
            Box::new(|b| b.host_config.network_mode = "host".into()),
        ),
        (
            "no security option",
            Box::new(|b| b.host_config.security_opt.clear()),
        ),
        (
            "another security option",
            Box::new(|b| b.host_config.security_opt = vec!["seccomp=unconfined".into()]),
        ),
        (
            "host pid",
            Box::new(|b| b.host_config.pid_mode = "host".into()),
        ),
        (
            "host ipc",
            Box::new(|b| b.host_config.ipc_mode = "host".into()),
        ),
        (
            "memory above the maximum",
            Box::new(|b| b.host_config.memory_bytes = 4 << 30),
        ),
        ("memory unset", Box::new(|b| b.host_config.memory_bytes = 0)),
        (
            "swap above memory",
            Box::new(|b| b.host_config.memory_swap_bytes = b.host_config.memory_bytes * 2),
        ),
        ("cpu unset", Box::new(|b| b.host_config.nano_cpus = 0)),
        (
            "cpu above eight",
            Box::new(|b| b.host_config.nano_cpus = 9_000_000_000),
        ),
        (
            "processes unset",
            Box::new(|b| b.host_config.pids_limit = 0),
        ),
        (
            "processes above the maximum",
            Box::new(|b| b.host_config.pids_limit = 5000),
        ),
        (
            "a host bind",
            Box::new(|b| b.host_config.binds.push("/etc:/host:ro".into())),
        ),
        (
            "another slot's bind",
            Box::new(|b| {
                b.host_config.binds = vec!["/run/dungeonflux/runtime/slot4/descriptor.toml:/etc/dungeonflux/runtime.toml:ro".into()]
            }),
        ),
        (
            "a writable descriptor",
            Box::new(|b| b.host_config.binds[0] = b.host_config.binds[0].replace(":ro", "")),
        ),
        (
            "a tmpfs elsewhere",
            Box::new(|b| {
                b.host_config
                    .tmpfs
                    .push(("/etc".into(), "rw,size=1024".into()))
            }),
        ),
        (
            "an unsized tmpfs",
            Box::new(|b| b.host_config.tmpfs[0].1 = "rw,noexec".into()),
        ),
        (
            "a tmpfs over the bound",
            Box::new(|b| b.host_config.tmpfs[0].1 = "rw,size=2g".into()),
        ),
        (
            "a tmpfs owned by root",
            Box::new(|b| b.host_config.tmpfs[1].1 = "rw,size=1024,uid=0,gid=10002".into()),
        ),
        (
            "a tmpfs option outside the list",
            Box::new(|b| b.host_config.tmpfs[0].1 = "rw,size=1024,exec".into()),
        ),
        (
            "an unlisted image",
            Box::new(|b| b.image = format!("evil/image@sha256:{}", "b".repeat(64))),
        ),
        (
            "a bare image id",
            Box::new(|b| b.image = format!("sha256:{}", "a".repeat(64))),
        ),
        ("root", Box::new(|b| b.user = "0:0".into())),
        ("another group", Box::new(|b| b.user = "20003:0".into())),
        (
            "a slot beyond the range",
            Box::new(|b| b.user = "20100:10002".into()),
        ),
        (
            "another slot's name",
            Box::new(|b| b.name = "df-rt-x-slot4".into()),
        ),
        (
            "a name without the prefix",
            Box::new(|b| b.name = "other-slot3".into()),
        ),
        ("no label", Box::new(|b| b.labels.clear())),
        ("an environment", Box::new(|b| b.env.push("A=1".into()))),
        ("another command", Box::new(|b| b.cmd = vec!["sh".into()])),
    ];
    for (name, change) in changes {
        let mut body = runner_body();
        change(&mut body);
        let real_refuses = real_verdict(&body).is_err();
        assert!(real_refuses, "the real proxy must refuse `{name}`");
        assert!(
            fake_refuses(&body),
            "the fake must refuse `{name}` as the real proxy does"
        );
    }
}
