use std::{path::PathBuf, time::Duration};

use crate::{
    docker::{
        ContainerConfig, CreateBody, DockerProxyLauncher, ImageReference, PolicyRule, ProxyError,
    },
    docker_fake::{FakeDockerProxy, Request},
    launcher::{LaunchError, RuntimeLauncher},
    spec::{DESCRIPTOR_MOUNT, PlacementId, RuntimeLimits, RuntimeSpec, SECRETS_MOUNT, SPOOL_MOUNT},
};

fn image(character: char) -> ImageReference {
    ImageReference::new(&format!(
        "ghcr.io/example/df-runtime:release@sha256:{}",
        character.to_string().repeat(64)
    ))
    .expect("reference")
}

fn config() -> ContainerConfig {
    ContainerConfig::new(image('a'), vec![image('a'), image('b')]).expect("allowed")
}

fn spec() -> RuntimeSpec {
    RuntimeSpec {
        placement_id: PlacementId::new([7; 16]),
        slot: 3,
        descriptor_path: PathBuf::from("/run/dungeonflux/runtime/slot3/descriptor.toml"),
        limits: RuntimeLimits::default(),
    }
}

fn launcher(exit_code: i32) -> DockerProxyLauncher<FakeDockerProxy> {
    let config = config();
    DockerProxyLauncher::new(FakeDockerProxy::enforcing(&config, exit_code), config)
}

fn created_body(launcher: &DockerProxyLauncher<FakeDockerProxy>) -> CreateBody {
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

fn kinds(launcher: &DockerProxyLauncher<FakeDockerProxy>) -> Vec<&'static str> {
    launcher
        .proxy()
        .requests()
        .iter()
        .map(|request| match request {
            Request::Create(_) => "create",
            Request::Start(_) => "start",
            Request::Stop(..) => "stop",
            Request::Wait(_) => "wait",
            Request::Delete(_) => "delete",
        })
        .collect()
}

#[tokio::test]
async fn the_create_body_has_an_empty_environment() {
    let launcher = launcher(0);
    launcher.start(&spec()).await.expect("started");
    assert!(
        created_body(&launcher).env.is_empty(),
        "the runtime gets configuration only from its descriptor"
    );
}

#[tokio::test]
async fn the_create_body_meets_every_proxy_policy_rule_for_its_slot() {
    let launcher = launcher(0);
    launcher.start(&spec()).await.expect("started");
    let body = created_body(&launcher);
    let host = &body.host_config;
    assert_eq!(body.image, image('a').as_str());
    assert_eq!(body.user, "20003:10002");
    assert_eq!(body.name, "df-rt-07070707070707070707070707070707-slot3");
    assert_eq!(
        body.labels,
        [("com.dungeonflux.role".to_string(), "runtime".to_string())]
    );
    assert!(!host.privileged && host.readonly_rootfs);
    assert!(host.cap_add.is_empty() && host.cap_drop == ["ALL"] && host.devices.is_empty());
    assert_eq!(host.security_opt, ["no-new-privileges:true"]);
    assert_eq!(host.network_mode, "df-slot-3");
    assert_eq!(
        (host.pid_mode.as_str(), host.ipc_mode.as_str()),
        ("", "private")
    );
    assert_eq!(
        host.binds,
        [
            format!("/run/dungeonflux/runtime/slot3/descriptor.toml:{DESCRIPTOR_MOUNT}:ro"),
            format!("/run/dungeonflux/runtime/slot3/secrets:{SECRETS_MOUNT}:ro"),
        ]
    );
    assert_eq!(
        body.cmd,
        ["campaign-runtime", "--descriptor", DESCRIPTOR_MOUNT]
    );
    assert_eq!(launcher.proxy().refusal(&body), None);
}

#[tokio::test]
async fn the_docker_json_carries_tmpfs_as_an_object_and_the_name_in_the_target() {
    let launcher = launcher(0);
    launcher.start(&spec()).await.expect("started");
    let body = created_body(&launcher);
    let json = body.to_docker_json();
    let tmpfs = json["HostConfig"]["Tmpfs"].as_object().expect("an object");
    assert_eq!(tmpfs.len(), 2);
    assert!(tmpfs["/var/lib/dungeonflux/spool"].as_str().is_some());
    assert_eq!(json["Labels"]["com.dungeonflux.role"], "runtime");
    assert_eq!(
        body.target(),
        "/containers/create?name=df-rt-07070707070707070707070707070707-slot3"
    );
}

#[tokio::test]
async fn the_spool_is_a_sized_tmpfs_owned_by_the_slot_and_never_a_bind() {
    let launcher = launcher(0);
    launcher.start(&spec()).await.expect("started");
    let host = created_body(&launcher).host_config;
    assert!(host.binds.iter().all(|bind| !bind.contains(SPOOL_MOUNT)));
    let spool = host
        .tmpfs
        .iter()
        .find(|(path, _)| path == SPOOL_MOUNT)
        .expect("a spool tmpfs");
    assert_eq!(
        spool.1,
        format!(
            "rw,noexec,nosuid,size={},uid=20003,gid=10002,mode=0700",
            256 * 1024 * 1024
        )
    );
}

#[tokio::test]
async fn limits_reach_the_container_in_the_unit_docker_expects() {
    let launcher = launcher(0);
    launcher.start(&spec()).await.expect("started");
    let host = created_body(&launcher).host_config;
    assert_eq!(host.memory_bytes, 1536 * 1024 * 1024);
    assert_eq!(
        host.memory_swap_bytes, host.memory_bytes,
        "no swap beyond memory"
    );
    assert_eq!(host.nano_cpus, 1_000_000_000);
    assert_eq!(host.pids_limit, 256);
    let tmp = host
        .tmpfs
        .iter()
        .find(|(path, _)| path == "/tmp")
        .expect("tmp");
    assert!(tmp.1.contains(&format!("size={}", 512 * 1024 * 1024)));
}

#[test]
fn an_image_outside_the_allow_list_cannot_become_a_launcher_configuration() {
    assert!(ContainerConfig::new(image('c'), vec![image('a')]).is_none());
}

#[test]
fn only_a_repository_pinned_by_manifest_digest_is_an_image_reference() {
    let digest = "a".repeat(64);
    for refused in [
        "ubuntu:latest".to_string(),
        format!("sha256:{digest}"),
        format!("@sha256:{digest}"),
        "repo@sha256:abc".to_string(),
        format!("repo@sha256:{}", "A".repeat(64)),
        digest.clone(),
    ] {
        assert!(ImageReference::new(&refused).is_none(), "{refused}");
    }
    assert!(ImageReference::new(&format!("repo/name:tag@sha256:{digest}")).is_some());
    assert!(ImageReference::new(&format!("repo/name@sha256:{digest}")).is_some());
}

#[tokio::test]
async fn the_fake_refuses_what_the_real_proxy_refuses() {
    let launcher = launcher(0);
    launcher.start(&spec()).await.expect("started");
    let body = created_body(&launcher);
    let refusal = |change: &dyn Fn(&mut CreateBody)| {
        let mut changed = body.clone();
        change(&mut changed);
        launcher.proxy().refusal(&changed)
    };
    assert_eq!(
        refusal(&|b| b.host_config.privileged = true),
        Some(PolicyRule::Privileged)
    );
    assert_eq!(
        refusal(&|b| b.host_config.binds.push("/etc:/host-etc".to_string())),
        Some(PolicyRule::MountNotTheSlotDescriptorOrSecrets)
    );
    assert_eq!(
        refusal(&|b| b.host_config.cap_add.push("NET_ADMIN".to_string())),
        Some(PolicyRule::CapabilitiesAdded)
    );
    assert_eq!(
        refusal(&|b| b.image = "sha256:".to_string() + &"a".repeat(64)),
        Some(PolicyRule::ImageNotAllowListed)
    );
    assert_eq!(
        refusal(&|b| b.host_config.security_opt.clear()),
        Some(PolicyRule::SecurityOptionNotNoNewPrivileges)
    );
    assert_eq!(
        refusal(&|b| b.host_config.network_mode = "df-slot-4".to_string()),
        Some(PolicyRule::NetworkNotTheSlotNetwork)
    );
    assert_eq!(
        refusal(&|b| b.name = "df-rt-x-slot4".to_string()),
        Some(PolicyRule::NameMustCarryTheSlot)
    );
    assert_eq!(
        refusal(&|b| b.host_config.tmpfs[1].1 = "rw,noexec,nosuid".to_string()),
        Some(PolicyRule::TmpfsOptionNotAllowed)
    );
    assert_eq!(
        refusal(&|b| b.user = "0:0".to_string()),
        Some(PolicyRule::UserOutsideSlotRange)
    );
    assert_eq!(
        refusal(&|b| b.host_config.memory_bytes = 3 * 1024 * 1024 * 1024),
        Some(PolicyRule::MemoryUnsetOrAboveMaximum)
    );
    assert_eq!(
        refusal(&|b| b.host_config.pids_limit = 2048),
        Some(PolicyRule::ProcessLimitUnsetOrAboveMaximum)
    );
    assert_eq!(
        refusal(&|b| b.host_config.tmpfs[1].1 = "rw,size=1024,uid=0,gid=0".to_string()),
        Some(PolicyRule::TmpfsOptionNotAllowed)
    );
    assert_eq!(
        refusal(&|b| b
            .host_config
            .tmpfs
            .push(("/etc".to_string(), "size=1".to_string()))),
        Some(PolicyRule::TmpfsPathNotAllowed)
    );
}

#[tokio::test]
async fn a_slot_outside_the_range_the_proxy_admits_is_refused_before_any_request() {
    let launcher = launcher(0);
    for slot in [0, 100, 255] {
        let mut out_of_range = spec();
        out_of_range.slot = slot;
        assert_eq!(
            launcher.start(&out_of_range).await.err(),
            Some(LaunchError::SlotOutOfRange),
            "slot {slot}"
        );
    }
    assert!(launcher.proxy().requests().is_empty());
    for slot in [1, 99] {
        let mut in_range = spec();
        in_range.slot = slot;
        in_range.descriptor_path = PathBuf::from(format!(
            "/run/dungeonflux/runtime/slot{slot}/descriptor.toml"
        ));
        assert!(launcher.start(&in_range).await.is_ok(), "slot {slot}");
    }
}

#[tokio::test]
async fn waiting_reports_the_exit_and_removes_the_container() {
    let launcher = launcher(3);
    let handle = launcher.start(&spec()).await.expect("started");
    let report = launcher.wait(&handle).await.expect("exit");
    assert_eq!(report.exit_code, Some(3));
    assert!(!report.is_clean());
    assert_eq!(kinds(&launcher).last(), Some(&"delete"));
}

#[tokio::test]
async fn stopping_passes_the_grace_period_to_the_proxy_then_waits_and_removes() {
    let launcher = launcher(0);
    let handle = launcher.start(&spec()).await.expect("started");
    let report = launcher
        .stop(&handle, Duration::from_secs(30))
        .await
        .expect("stopped");
    assert!(report.is_clean());
    assert_eq!(
        kinds(&launcher),
        ["create", "start", "stop", "wait", "delete"]
    );
    let grace = launcher
        .proxy()
        .requests()
        .into_iter()
        .find_map(|request| match request {
            Request::Stop(_, grace) => Some(grace),
            _ => None,
        });
    assert_eq!(grace, Some(Duration::from_secs(30)));
}

#[tokio::test]
async fn a_supervisor_wait_and_a_stop_delete_the_container_once() {
    let launcher = launcher(0);
    let handle = launcher.start(&spec()).await.expect("started");
    launcher
        .wait(&handle)
        .await
        .expect("exit seen by the supervisor");
    launcher
        .stop(&handle, Duration::from_secs(1))
        .await
        .expect("stop after the exit");
    let deletes = kinds(&launcher)
        .iter()
        .filter(|kind| **kind == "delete")
        .count();
    assert_eq!(deletes, 1);
}

#[tokio::test]
async fn a_container_created_but_not_started_is_removed_so_the_next_attempt_can_use_its_name() {
    let config = config();
    let proxy = FakeDockerProxy::enforcing(&config, 0).failing_to_start();
    let launcher = DockerProxyLauncher::new(proxy, config);
    assert_eq!(
        launcher.start(&spec()).await.err(),
        Some(LaunchError::Proxy(ProxyError::Unavailable))
    );
    assert_eq!(kinds(&launcher), ["create", "start", "delete"]);
}

#[tokio::test]
async fn a_policy_refusal_surfaces_as_a_typed_launch_error() {
    let launcher = launcher(0);
    let mut wrong = spec();
    wrong.descriptor_path = PathBuf::from("/tmp/descriptor.toml");
    assert_eq!(
        launcher.start(&wrong).await.err(),
        Some(LaunchError::Proxy(ProxyError::Refused(
            PolicyRule::MountNotTheSlotDescriptorOrSecrets
        )))
    );
}
