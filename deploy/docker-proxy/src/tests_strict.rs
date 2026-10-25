//! Raw-text tests: the bodies here are JSON TEXT, because `json!` cannot express a repeated key, an escaped key name or trailing data.
use crate::http::{BadRequest, parse_head};
use crate::policy::{Kind, Meta, Policy, Probe, canonical_body, check};
use crate::strict;
use std::collections::HashMap;
use std::sync::Arc;

const IMAGE: &str = "ghcr.io/owner/dungeonflux-campaign-runtime:1.2.3@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TARGET: &str =
    "/v1.45/containers/create?name=df-rt-00000000-0000-0000-0000-000000000003-slot1";

#[derive(Debug)]
struct Fs(HashMap<String, Meta>);

impl Probe for Fs {
    fn symlink_metadata(&self, path: &str) -> Result<Meta, String> {
        self.0
            .get(path)
            .copied()
            .ok_or_else(|| "not found".to_string())
    }
}

fn policy() -> Policy {
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
    Policy::standard(vec![IMAGE.to_string()], Arc::new(Fs(map)))
}

/// A compliant create body as text, with `{host}` standing for the HostConfig object text and `{extra}` for extra top-level members.
fn body_text(host: &str, extra: &str) -> String {
    format!(
        r#"{{"Image":"{IMAGE}","User":"20001:10002","Labels":{{"com.dungeonflux.role":"runtime"}},{extra}"HostConfig":{host}}}"#
    )
}

const COMPLIANT_HOST: &str = r#"{"CapDrop":["ALL"],"NetworkMode":"df-slot-1","ReadonlyRootfs":true,"SecurityOpt":["no-new-privileges:true"],"Memory":1610612736,"NanoCpus":1000000000,"PidsLimit":256,"Tmpfs":{"/tmp":"size=64m"}}"#;
const BAD_HOST: &str = r#"{"Privileged":true,"CapAdd":["SYS_ADMIN"]}"#;

fn run(text: &str) -> Result<(), String> {
    check(&policy(), "POST", TARGET, text.as_bytes()).map_err(|r| r.0)
}

#[test]
fn the_text_form_of_a_compliant_body_is_accepted() {
    assert_eq!(run(&body_text(COMPLIANT_HOST, "")), Ok(()));
}

#[test]
fn a_repeated_hostconfig_is_refused_whichever_copy_is_compliant() {
    // The compliant copy LAST (what a last-wins parser sees) and FIRST, with the privileged one on the other side.
    let compliant_last = format!(
        r#"{{"Image":"{IMAGE}","User":"20001:10002","Labels":{{"com.dungeonflux.role":"runtime"}},"HostConfig":{BAD_HOST},"HostConfig":{COMPLIANT_HOST}}}"#
    );
    let compliant_first = format!(
        r#"{{"Image":"{IMAGE}","User":"20001:10002","Labels":{{"com.dungeonflux.role":"runtime"}},"HostConfig":{COMPLIANT_HOST},"HostConfig":{BAD_HOST}}}"#
    );
    for text in [compliant_last, compliant_first] {
        let refusal = run(&text).unwrap_err();
        assert!(refusal.contains("repeated key"), "{refusal}");
    }
}

#[test]
fn repeated_keys_are_refused_at_every_depth() {
    // EndpointsConfig repeated inside NetworkingConfig (attach extra networks while the checked copy is clean).
    let endpoints = format!(
        r#"{{"Image":"{IMAGE}","User":"20001:10002","Labels":{{"com.dungeonflux.role":"runtime"}},"NetworkingConfig":{{"EndpointsConfig":{{"data":{{}}}},"EndpointsConfig":{{"df-slot-1":{{}}}}}},"HostConfig":{COMPLIANT_HOST}}}"#
    );
    assert!(run(&endpoints).unwrap_err().contains("repeated key"));
    // Tmpfs repeated inside HostConfig.
    let tmpfs = r#"{"CapDrop":["ALL"],"NetworkMode":"df-slot-1","ReadonlyRootfs":true,"SecurityOpt":["no-new-privileges:true"],"Memory":1610612736,"NanoCpus":1000000000,"PidsLimit":256,"Tmpfs":{"/etc":"size=1m"},"Tmpfs":{"/tmp":"size=64m"}}"#;
    assert!(
        run(&body_text(tmpfs, ""))
            .unwrap_err()
            .contains("repeated key")
    );
    // A key repeated INSIDE Tmpfs.
    let inner = COMPLIANT_HOST.replace(
        r#""/tmp":"size=64m""#,
        r#""/tmp":"size=64m","/tmp":"size=64m""#,
    );
    assert!(
        run(&body_text(&inner, ""))
            .unwrap_err()
            .contains("repeated key")
    );
    // Labels repeated at the top level, and a label key repeated inside Labels.
    let labels = format!(
        r#"{{"Image":"{IMAGE}","User":"20001:10002","Labels":{{"com.dungeonflux.role":"runtime"}},"Labels":{{"com.dungeonflux.role":"runtime"}},"HostConfig":{COMPLIANT_HOST}}}"#
    );
    assert!(run(&labels).unwrap_err().contains("repeated key"));
    let inner_labels = format!(
        r#"{{"Image":"{IMAGE}","User":"20001:10002","Labels":{{"com.dungeonflux.role":"runtime","com.dungeonflux.role":"runtime"}},"HostConfig":{COMPLIANT_HOST}}}"#
    );
    assert!(run(&inner_labels).unwrap_err().contains("repeated key"));
    // A repeated top-level scalar (User) and a repeated array-element object.
    let user = format!(
        r#"{{"Image":"{IMAGE}","User":"0:0","User":"20001:10002","Labels":{{"com.dungeonflux.role":"runtime"}},"HostConfig":{COMPLIANT_HOST}}}"#
    );
    assert!(run(&user).unwrap_err().contains("repeated key"));
}

#[test]
fn an_escaped_key_name_collides_with_the_plain_one() {
    // "Privileged" is "Privileged" after unescaping: the repeat is found, not hidden.
    let host = r#"{"Privileged":true,"Privileged":false,"CapDrop":["ALL"],"NetworkMode":"df-slot-1","ReadonlyRootfs":true,"SecurityOpt":["no-new-privileges:true"],"Memory":1610612736,"NanoCpus":1000000000,"PidsLimit":256}"#;
    assert!(
        run(&body_text(host, ""))
            .unwrap_err()
            .contains("repeated key")
    );
    // An escaped name that is the ONLY copy is just the plain name, checked as such.
    let only = r#"{"Privileged":true,"CapDrop":["ALL"],"NetworkMode":"df-slot-1","ReadonlyRootfs":true,"SecurityOpt":["no-new-privileges:true"],"Memory":1610612736,"NanoCpus":1000000000,"PidsLimit":256}"#;
    assert!(
        run(&body_text(only, ""))
            .unwrap_err()
            .contains("Privileged")
    );
}

#[test]
fn lowercase_and_other_case_variants_of_keys_are_refused_not_ignored() {
    // Go's encoding/json matches field names case-insensitively, so "hostconfig" would be applied by the daemon.
    for variant in ["hostconfig", "HOSTCONFIG", "hostConfig"] {
        let text = format!(
            r#"{{"Image":"{IMAGE}","User":"20001:10002","Labels":{{"com.dungeonflux.role":"runtime"}},"HostConfig":{COMPLIANT_HOST},"{variant}":{BAD_HOST}}}"#
        );
        let refusal = run(&text).unwrap_err();
        assert!(refusal.contains("not allowed"), "{variant}: {refusal}");
    }
    let host = COMPLIANT_HOST.replace(r#""CapDrop""#, r#""capdrop""#);
    assert!(run(&body_text(&host, "")).is_err());
    let host = format!(r#"{{"privileged":true,{}"#, &COMPLIANT_HOST[1..]);
    assert!(run(&body_text(&host, "")).is_err());
}

#[test]
fn trailing_data_after_the_object_is_refused() {
    let good = body_text(COMPLIANT_HOST, "");
    for tail in [" {}", "{}", " x", ",", "\u{0}"] {
        let refusal = run(&format!("{good}{tail}")).unwrap_err();
        assert!(refusal.contains("not strict JSON"), "{tail:?}: {refusal}");
    }
    // Only whitespace may follow.
    assert_eq!(run(&format!("{good}\n  ")), Ok(()));
}

#[test]
fn the_forwarded_body_is_the_canonical_form_of_what_was_checked() {
    let spaced = format!(" {}  ", body_text(COMPLIANT_HOST, "").replace(',', " ,\n "));
    let forwarded = canonical_body(&policy(), "POST", TARGET, spaced.as_bytes()).unwrap();
    let text = String::from_utf8(forwarded.clone()).unwrap();
    assert!(!text.contains(' ') || text.contains("size=64m"), "{text}");
    assert!(!text.contains('\n'));
    // The canonical form parses back to the same value and checks again.
    assert_eq!(
        strict::parse(&forwarded).unwrap(),
        strict::parse(spaced.as_bytes()).unwrap()
    );
    assert!(check(&policy(), "POST", TARGET, &forwarded).is_ok());
    // A request without a body forwards no body.
    assert_eq!(
        canonical_body(
            &policy(),
            "POST",
            "/v1.45/containers/df-rt-a-slot1/start",
            b""
        )
        .unwrap(),
        Vec::<u8>::new()
    );
}

#[test]
fn the_strict_parser_accepts_ordinary_json_and_refuses_the_rest() {
    assert!(strict::parse(br#"{"a":[1,2.5,{"b":null}],"c":"d"}"#).is_ok());
    assert!(strict::parse(br#"{"a":1,"a":2}"#).is_err());
    assert!(strict::parse(br#"[{"a":1,"a":1}]"#).is_err());
    assert!(strict::parse(br#"{"a":1} {"a":1}"#).is_err());
    assert!(strict::parse(b"").is_err());
    assert!(strict::parse(br#"{"a":1e999}"#).is_err());
}

#[test]
fn the_request_target_is_printable_ascii_only() {
    let line = |target: &str| format!("POST {target} HTTP/1.1\r\nHost: docker\r\n\r\n");
    assert!(parse_head(line("/v1.45/containers/create?name=df-rt-a-slot1").as_bytes()).is_ok());
    for target in [
        "/containers/create?name=a\u{1}b",
        "/containers/create?name=a\tb",
        "/containers/create?name=a\u{7f}b",
        "/containers/create?name=caf\u{e9}",
        "/containers/create?name=a\u{0}b",
        "/containers/create\u{85}",
        "/containers/create?name=a\u{a0}b",
        "/containers/create?name=a\u{2028}b",
    ] {
        assert_eq!(
            parse_head(line(target).as_bytes()),
            Err(BadRequest(
                "the request target has a control, space or non-ASCII byte"
            )),
            "{target:?}"
        );
    }
}
