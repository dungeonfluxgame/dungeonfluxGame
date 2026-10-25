use crate::http::{BadRequest, MAX_BODY_BYTES, forward_head, parse_head, refusal_response};
use crate::policy::{Kind, Meta, Policy, Probe, check};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

const IMAGE: &str = "ghcr.io/owner/dungeonflux-campaign-runtime:1.2.3@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const NAME: &str = "df-rt-00000000-0000-0000-0000-000000000003-slot1";

/// A filesystem that exists only in a map: path -> (kind, owner). A path that is not in the map does not exist.
#[derive(Debug, Clone)]
struct FakeFs(HashMap<String, Meta>);

impl Probe for FakeFs {
    fn symlink_metadata(&self, path: &str) -> Result<Meta, String> {
        self.0
            .get(path)
            .copied()
            .ok_or_else(|| "not found".to_string())
    }
}

/// The tree the helper and the runner leave for slot 1: root-owned directories down to slot1, and the three entries owned by uid 20001.
fn good_fs() -> FakeFs {
    let mut map = HashMap::new();
    for dir in [
        "/run",
        "/run/dungeonflux",
        "/run/dungeonflux/runtime",
        "/run/dungeonflux/runtime/slot1",
    ] {
        map.insert(
            dir.to_string(),
            Meta {
                kind: Kind::Dir,
                uid: 0,
            },
        );
    }
    map.insert(
        "/run/dungeonflux/runtime/slot1/descriptor.toml".to_string(),
        Meta {
            kind: Kind::File,
            uid: 20001,
        },
    );
    map.insert(
        "/run/dungeonflux/runtime/slot1/secrets".to_string(),
        Meta {
            kind: Kind::Dir,
            uid: 20001,
        },
    );
    FakeFs(map)
}

fn policy_with(fs: FakeFs) -> Policy {
    Policy::standard(vec![IMAGE.to_string()], Arc::new(fs))
}

fn policy() -> Policy {
    policy_with(good_fs())
}

fn valid_body() -> Value {
    json!({
        "Image": IMAGE,
        "User": "20001:10002",
        "Labels": { "com.dungeonflux.role": "runtime", "com.dungeonflux.placement": "3" },
        "Cmd": ["campaign-runtime", "--descriptor", "/etc/dungeonflux/runtime.toml"],
        "StopTimeout": 100,
        "HostConfig": {
            "Binds": [
                "/run/dungeonflux/runtime/slot1/descriptor.toml:/etc/dungeonflux/runtime.toml:ro",
                "/run/dungeonflux/runtime/slot1/secrets:/run/secrets:ro"
            ],
            "Tmpfs": {
                "/tmp": "size=512m,uid=20001,gid=10002,mode=1777",
                "/var/lib/dungeonflux/spool": "size=256m,uid=20001,gid=10002"
            },
            "NetworkMode": "df-slot-1",
            "Privileged": false,
            "CapDrop": ["ALL"],
            "ReadonlyRootfs": true,
            "SecurityOpt": ["no-new-privileges:true"],
            "Memory": 1610612736u64,
            "MemorySwap": 1610612736u64,
            "NanoCpus": 1000000000u64,
            "PidsLimit": 256,
            "Init": true,
            "RestartPolicy": { "Name": "no" }
        },
        "NetworkingConfig": { "EndpointsConfig": { "df-slot-1": { "IPAMConfig": { "IPv4Address": "172.30.11.10" } } } }
    })
}

fn create(body: &Value) -> Result<(), String> {
    check(
        &policy(),
        "POST",
        &format!("/v1.45/containers/create?name={NAME}"),
        body.to_string().as_bytes(),
    )
    .map_err(|r| r.0)
}

fn mutated(f: impl FnOnce(&mut Value)) -> Result<(), String> {
    let mut body = valid_body();
    f(&mut body);
    create(&body)
}

#[test]
fn a_valid_runtime_create_is_accepted() {
    assert_eq!(create(&valid_body()), Ok(()));
}

#[test]
fn each_escalation_in_a_create_body_is_refused() {
    type Mutation = Box<dyn FnOnce(&mut Value)>;
    let cases: Vec<(&str, Mutation)> = vec![
        (
            "privileged",
            Box::new(|b| b["HostConfig"]["Privileged"] = json!(true)),
        ),
        (
            "cap_add",
            Box::new(|b| b["HostConfig"]["CapAdd"] = json!(["SYS_ADMIN"])),
        ),
        (
            "cap_drop missing",
            Box::new(|b| {
                b["HostConfig"].as_object_mut().unwrap().remove("CapDrop");
            }),
        ),
        (
            "cap_drop not all",
            Box::new(|b| b["HostConfig"]["CapDrop"] = json!(["NET_RAW"])),
        ),
        (
            "devices",
            Box::new(|b| b["HostConfig"]["Devices"] = json!([{ "PathOnHost": "/dev/kmsg" }])),
        ),
        (
            "host network",
            Box::new(|b| b["HostConfig"]["NetworkMode"] = json!("host")),
        ),
        (
            "other slot network",
            Box::new(|b| b["HostConfig"]["NetworkMode"] = json!("df-slot-2")),
        ),
        (
            "data network",
            Box::new(|b| b["HostConfig"]["NetworkMode"] = json!("data")),
        ),
        (
            "host pid",
            Box::new(|b| b["HostConfig"]["PidMode"] = json!("host")),
        ),
        (
            "host ipc",
            Box::new(|b| b["HostConfig"]["IpcMode"] = json!("host")),
        ),
        (
            "container ipc",
            Box::new(|b| b["HostConfig"]["IpcMode"] = json!("container:other")),
        ),
        (
            "host userns",
            Box::new(|b| b["HostConfig"]["UsernsMode"] = json!("host")),
        ),
        (
            "host uts",
            Box::new(|b| b["HostConfig"]["UTSMode"] = json!("host")),
        ),
        (
            "writable root",
            Box::new(|b| b["HostConfig"]["ReadonlyRootfs"] = json!(false)),
        ),
        (
            "root fs unspecified",
            Box::new(|b| {
                b["HostConfig"]
                    .as_object_mut()
                    .unwrap()
                    .remove("ReadonlyRootfs");
            }),
        ),
        (
            "no-new-privileges missing",
            Box::new(|b| b["HostConfig"]["SecurityOpt"] = json!([])),
        ),
        (
            "seccomp unconfined",
            Box::new(|b| {
                b["HostConfig"]["SecurityOpt"] =
                    json!(["no-new-privileges:true", "seccomp=unconfined"])
            }),
        ),
        (
            "apparmor unconfined",
            Box::new(|b| {
                b["HostConfig"]["SecurityOpt"] =
                    json!(["no-new-privileges:true", "apparmor=unconfined"])
            }),
        ),
        (
            "no memory limit",
            Box::new(|b| {
                b["HostConfig"].as_object_mut().unwrap().remove("Memory");
            }),
        ),
        (
            "memory too large",
            Box::new(|b| b["HostConfig"]["Memory"] = json!(64u64 * 1024 * 1024 * 1024)),
        ),
        (
            "no cpu limit",
            Box::new(|b| {
                b["HostConfig"].as_object_mut().unwrap().remove("NanoCpus");
            }),
        ),
        (
            "no pids limit",
            Box::new(|b| {
                b["HostConfig"].as_object_mut().unwrap().remove("PidsLimit");
            }),
        ),
        (
            "pids unlimited",
            Box::new(|b| b["HostConfig"]["PidsLimit"] = json!(-1)),
        ),
        (
            "port bindings",
            Box::new(|b| {
                b["HostConfig"]["PortBindings"] = json!({ "8081/tcp": [{ "HostPort": "8081" }] })
            }),
        ),
        (
            "publish all ports",
            Box::new(|b| b["HostConfig"]["PublishAllPorts"] = json!(true)),
        ),
        (
            "extra hosts",
            Box::new(|b| b["HostConfig"]["ExtraHosts"] = json!(["metadata:169.254.169.254"])),
        ),
        (
            "sysctls",
            Box::new(|b| b["HostConfig"]["Sysctls"] = json!({ "net.ipv4.ip_forward": "1" })),
        ),
        (
            "volumes from",
            Box::new(|b| b["HostConfig"]["VolumesFrom"] = json!(["control-plane"])),
        ),
        (
            "mounts",
            Box::new(|b| {
                b["HostConfig"]["Mounts"] =
                    json!([{ "Type": "bind", "Source": "/", "Target": "/host" }])
            }),
        ),
        (
            "restart policy",
            Box::new(|b| b["HostConfig"]["RestartPolicy"] = json!({ "Name": "always" })),
        ),
        (
            "runtime",
            Box::new(|b| b["HostConfig"]["Runtime"] = json!("nvidia")),
        ),
        (
            "cgroup parent",
            Box::new(|b| b["HostConfig"]["CgroupParent"] = json!("/")),
        ),
        (
            "docker socket bind",
            Box::new(|b| {
                b["HostConfig"]["Binds"] = json!(["/var/run/docker.sock:/var/run/docker.sock"])
            }),
        ),
        (
            "root bind",
            Box::new(|b| b["HostConfig"]["Binds"] = json!(["/:/host:ro"])),
        ),
        (
            "secrets root bind",
            Box::new(|b| {
                b["HostConfig"]["Binds"] = json!(["/run/dungeonflux/secrets:/run/secrets:ro"])
            }),
        ),
        (
            "another slot's secrets",
            Box::new(|b| {
                b["HostConfig"]["Binds"] =
                    json!(["/run/dungeonflux/runtime/slot2/secrets:/run/secrets:ro"])
            }),
        ),
        (
            "writable secrets",
            Box::new(|b| {
                b["HostConfig"]["Binds"] =
                    json!(["/run/dungeonflux/runtime/slot1/secrets:/run/secrets"])
            }),
        ),
        (
            "secrets at another path",
            Box::new(|b| {
                b["HostConfig"]["Binds"] = json!(["/run/dungeonflux/runtime/slot1/secrets:/etc:ro"])
            }),
        ),
        (
            "tmpfs elsewhere",
            Box::new(|b| {
                b["HostConfig"]["Tmpfs"] = json!({ "/etc": "size=1m,uid=20001,gid=10002" })
            }),
        ),
        (
            "tmpfs owned by root",
            Box::new(|b| b["HostConfig"]["Tmpfs"] = json!({ "/tmp": "size=1m,uid=0,gid=0" })),
        ),
        (
            "tmpfs owned by another slot",
            Box::new(|b| {
                b["HostConfig"]["Tmpfs"] = json!({ "/tmp": "size=1m,uid=20002,gid=10002" })
            }),
        ),
        (
            "tmpfs option exec",
            Box::new(|b| {
                b["HostConfig"]["Tmpfs"] = json!({ "/tmp": "size=1m,uid=20001,gid=10002,exec" })
            }),
        ),
        (
            "image tag only",
            Box::new(|b| b["Image"] = json!("ghcr.io/owner/dungeonflux-campaign-runtime:1.2.3")),
        ),
        (
            "image not allow-listed",
            Box::new(|b| {
                b["Image"] = json!(
                    "ghcr.io/owner/dungeonflux-campaign-runtime:1.2.3@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                )
            }),
        ),
        (
            "image missing",
            Box::new(|b| {
                b.as_object_mut().unwrap().remove("Image");
            }),
        ),
        ("root user", Box::new(|b| b["User"] = json!("0:0"))),
        ("name user", Box::new(|b| b["User"] = json!("root"))),
        ("uid 65532", Box::new(|b| b["User"] = json!("65532:10002"))),
        (
            "uid below the slot range",
            Box::new(|b| b["User"] = json!("20000:10002")),
        ),
        (
            "uid above the slot range",
            Box::new(|b| b["User"] = json!("20100:10002")),
        ),
        ("wrong group", Box::new(|b| b["User"] = json!("20001:0"))),
        (
            "uid of another slot",
            Box::new(|b| b["User"] = json!("20002:10002")),
        ),
        ("environment", Box::new(|b| b["Env"] = json!(["DF_X=1"]))),
        (
            "entrypoint",
            Box::new(|b| b["Entrypoint"] = json!(["/bin/sh"])),
        ),
        (
            "command",
            Box::new(|b| b["Cmd"] = json!(["sh", "-c", "id"])),
        ),
        (
            "exposed ports",
            Box::new(|b| b["ExposedPorts"] = json!({ "22/tcp": {} })),
        ),
        (
            "volumes",
            Box::new(|b| b["Volumes"] = json!({ "/data": {} })),
        ),
        ("tty", Box::new(|b| b["Tty"] = json!(true))),
        (
            "no runtime label",
            Box::new(|b| b["Labels"] = json!({ "com.dungeonflux.placement": "3" })),
        ),
        (
            "foreign label",
            Box::new(|b| {
                b["Labels"] = json!({ "com.dungeonflux.role": "runtime", "traefik.enable": "true" })
            }),
        ),
        (
            "stop timeout",
            Box::new(|b| b["StopTimeout"] = json!(100000)),
        ),
        (
            "endpoint on another network",
            Box::new(|b| b["NetworkingConfig"] = json!({ "EndpointsConfig": { "data": {} } })),
        ),
        (
            "runtime address outside the slot",
            Box::new(|b| {
                b["NetworkingConfig"]["EndpointsConfig"]["df-slot-1"]["IPAMConfig"]["IPv4Address"] =
                    json!("172.30.2.2")
            }),
        ),
        (
            "endpoint aliases",
            Box::new(|b| {
                b["NetworkingConfig"]["EndpointsConfig"]["df-slot-1"]["Aliases"] =
                    json!(["postgres"])
            }),
        ),
        (
            "missing host config",
            Box::new(|b| {
                b.as_object_mut().unwrap().remove("HostConfig");
            }),
        ),
    ];
    let count = cases.len();
    for (what, mutate) in cases {
        let result = mutated(mutate);
        assert!(result.is_err(), "{what} was accepted");
    }
    assert!(count > 60, "the escalation table shrank");
}

#[test]
fn the_container_name_must_be_a_runtime_name_and_carry_the_slot() {
    let body = valid_body().to_string();
    for name in [
        "control-plane",
        "df-rt-",
        "df-rt-UPPER",
        "df-rt-ok-slot2",
        "df-rt-ok_slot1",
        "df-rt-ok-slot1;rm",
    ] {
        let result = check(
            &policy(),
            "POST",
            &format!("/containers/create?name={name}"),
            body.as_bytes(),
        );
        assert!(result.is_err(), "{name} was accepted");
    }
    assert!(
        check(&policy(), "POST", "/containers/create", body.as_bytes()).is_err(),
        "a nameless container was accepted"
    );
    assert!(
        check(
            &policy(),
            "POST",
            &format!("/containers/create?name={NAME}&platform=linux"),
            body.as_bytes()
        )
        .is_err()
    );
}

#[test]
fn only_the_runner_calls_on_runtime_names_pass() {
    let p = policy();
    let ok = [
        ("POST", format!("/containers/{NAME}/start")),
        ("POST", format!("/containers/{NAME}/stop?t=100")),
        ("POST", format!("/containers/{NAME}/kill?signal=SIGKILL")),
        ("POST", format!("/containers/{NAME}/wait")),
        ("GET", format!("/containers/{NAME}/json")),
        ("GET", format!("/v1.45/containers/{NAME}/stats?stream=false")),
        ("DELETE", format!("/containers/{NAME}?force=1&v=1")),
        ("GET", "/_ping".to_string()),
        ("GET", "/containers/json?all=1&filters=%7B%22label%22%3A%5B%22com.dungeonflux.role%3Druntime%22%5D%7D".to_string()),
    ];
    for (method, target) in ok {
        assert_eq!(check(&p, method, &target, b""), Ok(()), "{method} {target}");
    }
}

#[test]
fn every_other_call_is_refused() {
    let p = policy();
    let refused = [
        ("POST", "/containers/control-plane/stop".to_string()),
        ("POST", "/containers/df-rt-x/start".to_string()),
        ("POST", format!("/containers/{NAME}/exec")),
        ("POST", format!("/containers/{NAME}/attach")),
        ("POST", format!("/containers/{NAME}/restart")),
        ("POST", format!("/containers/{NAME}/update")),
        ("POST", format!("/containers/{NAME}/rename?name=x")),
        ("POST", format!("/containers/{NAME}/pause")),
        ("POST", format!("/containers/{NAME}/start?detachKeys=x")),
        ("GET", format!("/containers/{NAME}/logs")),
        (
            "GET",
            format!("/containers/{NAME}/archive?path=/run/secrets"),
        ),
        ("PUT", format!("/containers/{NAME}/archive?path=/")),
        ("GET", format!("/containers/{NAME}/export")),
        ("GET", format!("/containers/{NAME}/stats")),
        ("GET", format!("/containers/{NAME}/stats?stream=true")),
        ("DELETE", format!("/containers/{NAME}?link=1")),
        ("POST", format!("/containers/{NAME}/stop?t=99999")),
        ("POST", format!("/containers/{NAME}/kill?signal=SIGSTOP")),
        ("GET", "/containers/json?all=1".to_string()),
        (
            "GET",
            "/containers/json?filters=%7B%22label%22%3A%5B%22other%22%5D%7D".to_string(),
        ),
        ("POST", "/containers/prune".to_string()),
        ("POST", "/images/create?fromImage=evil".to_string()),
        ("GET", "/images/json".to_string()),
        ("POST", "/build".to_string()),
        ("POST", "/networks/create".to_string()),
        ("GET", "/networks".to_string()),
        ("POST", "/volumes/create".to_string()),
        ("GET", "/info".to_string()),
        ("GET", "/version".to_string()),
        ("GET", "/events".to_string()),
        ("POST", "/exec/abc/start".to_string()),
        ("POST", "/auth".to_string()),
        ("POST", "/session".to_string()),
        ("GET", format!("/containers/{NAME}/../json")),
        ("GET", "/containers/%64f-rt-x/json".to_string()),
        ("GET", format!("/containers//{NAME}/json")),
    ];
    for (method, target) in refused {
        assert!(
            check(&p, method, &target, b"").is_err(),
            "{method} {target} was accepted"
        );
    }
    assert!(
        check(&p, "POST", &format!("/containers/{NAME}/start"), b"{}").is_err(),
        "a body on start was accepted"
    );
}

#[test]
fn a_create_body_that_is_not_json_or_not_an_object_is_refused() {
    let p = policy();
    for body in [&b""[..], b"not json", b"[]", b"null", b"\"x\""] {
        assert!(check(&p, "POST", &format!("/containers/create?name={NAME}"), body).is_err());
    }
}

#[test]
fn the_head_parser_refuses_smuggling_and_oversize() {
    let ok = parse_head(
        b"POST /containers/create?name=x HTTP/1.1\r\nHost: d\r\nContent-Length: 12\r\n\r\n",
    )
    .unwrap();
    assert_eq!((ok.method.as_str(), ok.content_length), ("POST", 12));
    let bad: Vec<&[u8]> = vec![
        b"POST /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
        b"POST /x HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\n",
        b"POST /x HTTP/1.1\r\nUpgrade: tcp\r\n\r\n",
        b"POST /x HTTP/1.0\r\n\r\n",
        b"POST x HTTP/1.1\r\n\r\n",
        b"post /x HTTP/1.1\r\n\r\n",
        b"POST /x HTTP/1.1 extra\r\n\r\n",
        b"POST /x HTTP/1.1\r\nBad Header: x\r\n\r\n",
        b"POST /x HTTP/1.1\r\nContent-Length: abc\r\n\r\n",
    ];
    for raw in bad {
        assert!(parse_head(raw).is_err(), "{}", String::from_utf8_lossy(raw));
    }
    let big = format!(
        "POST /x HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
        MAX_BODY_BYTES + 1
    );
    assert_eq!(
        parse_head(big.as_bytes()),
        Err(BadRequest("request body too large"))
    );
}

#[test]
fn the_forwarded_head_is_rebuilt_and_closes_the_connection() {
    let head = parse_head(b"POST /containers/x/start HTTP/1.1\r\nHost: evil\r\nX-Registry-Auth: secret\r\nExpect: 100-continue\r\nContent-Length: 0\r\n\r\n").unwrap();
    let forwarded = String::from_utf8(forward_head(&head, head.content_length)).unwrap();
    assert!(forwarded.contains("Connection: close"));
    assert!(forwarded.contains("Host: docker"));
    assert!(
        !forwarded.contains("evil")
            && !forwarded.contains("X-Registry-Auth")
            && !forwarded.contains("Expect")
    );
}

#[test]
fn the_refusal_is_a_403_without_control_characters() {
    let response = String::from_utf8(refusal_response("bad \"quote\"\r\nSet-Cookie: x")).unwrap();
    assert!(response.starts_with("HTTP/1.1 403 Forbidden\r\n"));
    assert_eq!(response.matches("\r\n\r\n").count(), 1);
    assert!(!response.contains("Set-Cookie: x\r\n"));
}

/// The create body `crates/df-runner/src/docker.rs` documents (its `create_body`, rebuilt here field for field in Docker's JSON), with
/// only the fields the proxy needs and the runner does not yet send added: the full allow-listed image reference, `User`, the
/// label, `SecurityOpt` and the container name in the query. Pins the contract in both directions: with these additions the
/// runner's body is accepted; without any one of them it is refused (the refusals are listed in the S2 request to O1).
fn runner_body(with_additions: bool) -> Value {
    let mut body = json!({
        "Image": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "Env": [],
        "Cmd": ["campaign-runtime", "--descriptor", "/etc/dungeonflux/runtime.toml"],
        "HostConfig": {
            "Privileged": false,
            "Binds": [
                "/run/dungeonflux/runtime/slot1/descriptor.toml:/etc/dungeonflux/runtime.toml:ro"
            ],
            "CapAdd": [],
            "CapDrop": ["ALL"],
            "NetworkMode": "df-slot-1",
            "Devices": [],
            "IpcMode": "private",
            "ReadonlyRootfs": true,
            "Memory": 1610612736u64,
            "NanoCpus": 1000000000u64,
            "PidsLimit": 256,
            "Tmpfs": {
                "/tmp": "rw,noexec,nosuid,size=536870912",
                "/var/lib/dungeonflux/spool": "rw,noexec,nosuid,size=268435456"
            }
        }
    });
    if with_additions {
        body["Image"] = json!(IMAGE);
        body["User"] = json!("20001:10002");
        body["Labels"] =
            json!({ "com.dungeonflux.role": "runtime", "com.dungeonflux.placement": "3" });
        body["HostConfig"]["SecurityOpt"] = json!(["no-new-privileges:true"]);
    }
    body
}

#[test]
fn the_runner_body_is_accepted_once_it_carries_the_fields_the_proxy_needs() {
    assert_eq!(create(&runner_body(true)), Ok(()));
}

#[test]
fn the_runner_body_as_documented_today_is_refused() {
    // Missing: the full image reference, User, the role label and SecurityOpt. The first check that fails names the image.
    let refusal = create(&runner_body(false)).unwrap_err();
    assert!(refusal.contains("image"), "{refusal}");
    for (what, edit) in [
        (
            "User",
            Box::new(|b: &mut Value| {
                b.as_object_mut().unwrap().remove("User");
            }) as Box<dyn Fn(&mut Value)>,
        ),
        (
            "Labels",
            Box::new(|b: &mut Value| {
                b.as_object_mut().unwrap().remove("Labels");
            }),
        ),
        (
            "SecurityOpt",
            Box::new(|b: &mut Value| {
                b["HostConfig"]
                    .as_object_mut()
                    .unwrap()
                    .remove("SecurityOpt");
            }),
        ),
    ] {
        let mut body = runner_body(true);
        edit(&mut body);
        assert!(
            create(&body).is_err(),
            "a body without {what} must be refused"
        );
    }
}

#[test]
fn security_opt_is_an_exact_allowlist() {
    // Accepted spellings.
    for ok in [
        json!(["no-new-privileges:true"]),
        json!(["no-new-privileges"]),
    ] {
        assert_eq!(
            mutated(|b| b["HostConfig"]["SecurityOpt"] = ok.clone()),
            Ok(())
        );
    }
    // Everything else is refused, even next to no-new-privileges: unconfined profiles in every spelling, a custom label, a custom
    // profile, systempaths, a non-string entry, an empty list, a missing key.
    for bad in [
        json!(["no-new-privileges:true", "seccomp=unconfined"]),
        json!(["no-new-privileges:true", "seccomp:unconfined"]),
        json!(["no-new-privileges:true", "apparmor=unconfined"]),
        json!(["no-new-privileges:true", "apparmor:unconfined"]),
        json!(["no-new-privileges:true", "label=disable"]),
        json!(["no-new-privileges:true", "label=type:spc_t"]),
        json!(["no-new-privileges:true", "label=user:root"]),
        json!(["no-new-privileges:true", "seccomp=/tmp/custom.json"]),
        json!(["no-new-privileges:true", "apparmor=docker-default"]),
        json!(["no-new-privileges:true", "systempaths=unconfined"]),
        json!(["no-new-privileges:true", "No-New-Privileges"]),
        json!(["no-new-privileges:false"]),
        json!(["no-new-privileges:true", 7]),
        json!([]),
        json!("no-new-privileges:true"),
    ] {
        assert!(
            mutated(|b| b["HostConfig"]["SecurityOpt"] = bad.clone()).is_err(),
            "{bad} must be refused"
        );
    }
    assert!(
        mutated(|b| {
            b["HostConfig"]
                .as_object_mut()
                .unwrap()
                .remove("SecurityOpt");
        })
        .is_err()
    );
}

#[test]
fn loose_typed_fields_are_refused_not_ignored() {
    for (what, edit) in [
        (
            "Init as a string",
            Box::new(|b: &mut Value| b["HostConfig"]["Init"] = json!("yes"))
                as Box<dyn Fn(&mut Value)>,
        ),
        (
            "AutoRemove as a number",
            Box::new(|b: &mut Value| b["HostConfig"]["AutoRemove"] = json!(1)),
        ),
        (
            "RestartPolicy as a string",
            Box::new(|b: &mut Value| b["HostConfig"]["RestartPolicy"] = json!("always")),
        ),
        (
            "RestartPolicy with another key",
            Box::new(|b: &mut Value| {
                b["HostConfig"]["RestartPolicy"] = json!({ "Name": "no", "X": 1 })
            }),
        ),
        (
            "AttachStdout true",
            Box::new(|b: &mut Value| b["AttachStdout"] = json!(true)),
        ),
        (
            "Tty as a string",
            Box::new(|b: &mut Value| b["Tty"] = json!("true")),
        ),
    ] {
        assert!(mutated(|b| edit(b)).is_err(), "{what} must be refused");
    }
}

fn create_on(fs: FakeFs, body: &Value) -> Result<(), String> {
    check(
        &policy_with(fs),
        "POST",
        &format!("/v1.45/containers/create?name={NAME}"),
        body.to_string().as_bytes(),
    )
    .map_err(|r| r.0)
}

#[test]
fn every_allowed_bind_is_checked_on_the_filesystem_and_the_good_tree_passes() {
    assert_eq!(create_on(good_fs(), &valid_body()), Ok(()));
}

#[test]
fn a_symlink_anywhere_on_a_bind_source_path_is_refused() {
    let root = "/run/dungeonflux/runtime";
    for link in [
        "/run/dungeonflux".to_string(),
        root.to_string(),
        format!("{root}/slot1"),
        format!("{root}/slot1/descriptor.toml"),
        format!("{root}/slot1/secrets"),
    ] {
        let mut fs = good_fs();
        let uid = fs.0[&link].uid;
        fs.0.insert(
            link.clone(),
            Meta {
                kind: Kind::Symlink,
                uid,
            },
        );
        let refusal = create_on(fs, &valid_body()).unwrap_err();
        assert!(refusal.contains("symlink"), "{link}: {refusal}");
    }
}

#[test]
fn a_bind_source_of_the_wrong_type_is_refused() {
    let root = "/run/dungeonflux/runtime/slot1";
    let cases = [
        (format!("{root}/descriptor.toml"), Kind::Dir, "regular file"),
        (
            format!("{root}/descriptor.toml"),
            Kind::Other,
            "regular file",
        ),
        (format!("{root}/secrets"), Kind::File, "directory"),
    ];
    for (path, kind, wanted) in cases {
        let mut fs = good_fs();
        fs.0.insert(path.clone(), Meta { kind, uid: 20001 });
        let refusal = create_on(fs, &valid_body()).unwrap_err();
        assert!(refusal.contains(wanted), "{path} as {kind:?}: {refusal}");
    }
}

#[test]
fn a_bind_source_with_the_wrong_owner_is_refused() {
    let root = "/run/dungeonflux/runtime/slot1";
    // The leaf must belong to this slot's uid: not root, not the runner, not another slot's runtime.
    for path in [format!("{root}/descriptor.toml"), format!("{root}/secrets")] {
        for uid in [0, 10004, 20002] {
            let mut fs = good_fs();
            let kind = fs.0[&path].kind;
            fs.0.insert(path.clone(), Meta { kind, uid });
            let refusal = create_on(fs, &valid_body()).unwrap_err();
            assert!(
                refusal.contains("owned by the slot"),
                "{path} owned by {uid}: {refusal}"
            );
        }
    }
    // Every directory above the leaf must be root's: a slot directory the runner owns could be renamed or replaced.
    for dir in ["/run", "/run/dungeonflux", "/run/dungeonflux/runtime", root] {
        let mut fs = good_fs();
        fs.0.insert(
            dir.to_string(),
            Meta {
                kind: Kind::Dir,
                uid: 10004,
            },
        );
        let refusal = create_on(fs, &valid_body()).unwrap_err();
        assert!(refusal.contains("owned by root"), "{dir}: {refusal}");
    }
}

#[test]
fn a_missing_bind_source_or_a_file_where_a_directory_belongs_is_refused() {
    let mut fs = good_fs();
    fs.0.remove("/run/dungeonflux/runtime/slot1/secrets");
    assert!(
        create_on(fs, &valid_body())
            .unwrap_err()
            .contains("cannot inspect")
    );
    let mut fs = good_fs();
    fs.0.insert(
        "/run/dungeonflux/runtime".to_string(),
        Meta {
            kind: Kind::File,
            uid: 0,
        },
    );
    assert!(
        create_on(fs, &valid_body())
            .unwrap_err()
            .contains("owned by root")
    );
}

#[test]
fn the_spool_is_a_sized_per_container_tmpfs_and_never_a_bind() {
    // A persistent spool bind would hand the next tenant of the slot the previous tenant's data: refused, however well-formed.
    let bind = "/run/dungeonflux/runtime/slot1/spool:/var/lib/dungeonflux/spool";
    let refusal = mutated(|b| {
        b["HostConfig"]["Binds"]
            .as_array_mut()
            .unwrap()
            .push(json!(bind));
    })
    .unwrap_err();
    assert!(refusal.contains("bind mount"), "{refusal}");
    // The tmpfs spool is accepted with a size, in every spelling Docker reads.
    for size in ["268435456", "256m", "262144k", "1g"] {
        let ok = mutated(|b| {
            b["HostConfig"]["Tmpfs"]["/var/lib/dungeonflux/spool"] =
                json!(format!("rw,noexec,nosuid,size={size}"));
        });
        assert_eq!(ok, Ok(()), "size={size}");
    }
    // Without a size, with a zero or oversize size, or with a malformed one it is refused (an unsized tmpfs defaults to half the memory).
    for options in [
        "rw,noexec,nosuid",
        "rw,size=0",
        "rw,size=2g",
        "rw,size=1025m",
        "rw,size=",
        "rw,size=12x",
        "rw,size=999999999999999g",
        "rw,size=1G",
    ] {
        let refusal = mutated(|b| {
            b["HostConfig"]["Tmpfs"]["/var/lib/dungeonflux/spool"] = json!(options);
        })
        .unwrap_err();
        assert!(refusal.contains("tmpfs"), "{options}: {refusal}");
    }
    // /tmp needs a size too.
    assert!(mutated(|b| b["HostConfig"]["Tmpfs"]["/tmp"] = json!("rw,noexec")).is_err());
}

#[test]
fn a_bind_listed_twice_is_refused() {
    let descriptor =
        "/run/dungeonflux/runtime/slot1/descriptor.toml:/etc/dungeonflux/runtime.toml:ro";
    let secrets = "/run/dungeonflux/runtime/slot1/secrets:/run/secrets:ro";
    // The valid body carries the descriptor and the secrets bind once each.
    assert_eq!(mutated(|_| {}), Ok(()));
    for duplicate in [descriptor, secrets] {
        let refusal = mutated(|b| {
            b["HostConfig"]["Binds"]
                .as_array_mut()
                .unwrap()
                .push(json!(duplicate));
        })
        .unwrap_err();
        assert!(refusal.contains("listed twice"), "{duplicate}: {refusal}");
    }
    // The same source mounted at a second target, and the same target fed from a second source, are refused too.
    let refusal = mutated(|b| {
        b["HostConfig"]["Binds"].as_array_mut().unwrap().push(json!(
            "/run/dungeonflux/runtime/slot1/secrets:/etc/other:ro"
        ));
    })
    .unwrap_err();
    assert!(
        refusal.contains("listed twice") || refusal.contains("not the descriptor"),
        "{refusal}"
    );
    let refusal = mutated(|b| {
        b["HostConfig"]["Binds"].as_array_mut().unwrap().push(json!(
            "/run/dungeonflux/runtime/slot1/descriptor.toml:/run/secrets:ro"
        ));
    })
    .unwrap_err();
    assert!(
        refusal.contains("listed twice") || refusal.contains("not the descriptor"),
        "{refusal}"
    );
}
