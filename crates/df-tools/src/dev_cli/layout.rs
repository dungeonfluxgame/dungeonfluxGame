//! Strict live layout command parsing; no unknown flag is silently ignored.
use super::{Client, CommandResult, get, post};
use crate::layout_contract::{self as contract, MoveRequest, TransformRequest, Unit};
use serde_json::json;
use std::collections::BTreeMap;
enum Command {
    Get(Option<String>),
    Transform(TransformRequest),
    Move(MoveRequest),
    Reset(Option<String>),
    Visibility { id: String, visible: bool },
}
fn parse(positional: &[String], options: &[(String, String)]) -> Result<Command, String> {
    let action = positional.get(1).map(String::as_str).unwrap_or("list");
    let id = positional.get(2).cloned();
    if positional.len() > 3 {
        return Err("layout received too many arguments".into());
    }
    let allowed: &[&str] = match action {
        "list" | "get" | "reset" | "hide" | "show" => &[],
        "transform" => &[
            "--translate-x",
            "--translate-y",
            "--unit",
            "--scale",
            "--scale-x",
            "--scale-y",
            "--rotate",
        ],
        "move" => &["--x", "--y", "--translate-x", "--translate-y", "--unit"],
        _ => {
            return Err(
                "layout needs list, get [id], move <id>, transform <id>, hide <id>, show <id> or reset [id]".into(),
            );
        }
    };
    let mut flags = BTreeMap::new();
    for (name, value) in options {
        if matches!(name.as_str(), "--server" | "--token-file" | "--timeout") {
            continue;
        }
        if !allowed.contains(&name.as_str()) {
            return Err(format!("unsupported layout flag {name}"));
        }
        if flags.insert(name.as_str(), value.as_str()).is_some() {
            return Err(format!("duplicate layout flag {name}"));
        }
    }
    if let Some(id) = &id {
        contract::validate_id(id).map_err(|e| e.to_string())?;
    }
    let unit = match flags.get("--unit").copied().unwrap_or("px") {
        "px" => Unit::Px,
        "percent" => Unit::Percent,
        _ => return Err("--unit must be px or percent".into()),
    };
    let number = |flag: &str| -> Result<Option<f64>, String> {
        flags
            .get(flag)
            .map(|v| {
                v.parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite())
                    .ok_or_else(|| format!("{flag} must be finite"))
            })
            .transpose()
    };
    match action {
        "list" => {
            if id.is_some() {
                return Err("layout list takes no ID; use layout get <id>".into());
            }
            Ok(Command::Get(None))
        }
        "get" => Ok(Command::Get(id)),
        "reset" => Ok(Command::Reset(id)),
        "hide" | "show" => Ok(Command::Visibility {
            id: id.ok_or_else(|| format!("layout {action} needs ID"))?,
            visible: action == "show",
        }),
        "move" => {
            if flags.contains_key("--x") && flags.contains_key("--translate-x")
                || flags.contains_key("--y") && flags.contains_key("--translate-y")
            {
                return Err("move translation axis specified twice".into());
            }
            let body = MoveRequest {
                id: id.ok_or("layout move needs ID")?,
                unit,
                x: number("--x")?.or(number("--translate-x")?),
                y: number("--y")?.or(number("--translate-y")?),
            };
            body.patch().map_err(|e| e.to_string())?;
            Ok(Command::Move(body))
        }
        "transform" => {
            let body = TransformRequest {
                id: id.ok_or("layout transform needs ID")?,
                unit,
                translate_x: number("--translate-x")?,
                translate_y: number("--translate-y")?,
                scale: number("--scale")?,
                scale_x: number("--scale-x")?,
                scale_y: number("--scale-y")?,
                rotate: number("--rotate")?,
            };
            body.patch().map_err(|e| e.to_string())?;
            Ok(Command::Transform(body))
        }
        _ => Err("unknown layout action".into()),
    }
}
pub(super) async fn run(
    client: &Client,
    positional: &[String],
    options: &[(String, String)],
) -> CommandResult {
    let command = match parse(positional, options) {
        Ok(command) => command,
        Err(error) => return client.print_error(error),
    };
    match command {
        Command::Get(id) => {
            get(
                client,
                &id.map_or_else(
                    || "/dev/layout".to_owned(),
                    |id| format!("/dev/layout?id={id}"),
                ),
            )
            .await
        }
        Command::Reset(id) => post(client, "/dev/layout/reset", &json!({"id":id})).await,
        Command::Move(body) => match serde_json::to_value(body) {
            Ok(body) => post(client, "/dev/layout/move", &body).await,
            Err(_) => client.print_error("layout move encoding failed"),
        },
        Command::Transform(body) => match serde_json::to_value(body) {
            Ok(body) => post(client, "/dev/layout/transform", &body).await,
            Err(_) => client.print_error("layout transform encoding failed"),
        },
        Command::Visibility { id, visible } => {
            post(
                client,
                "/dev/layout/visibility",
                &json!({"id": id, "visible": visible}),
            )
            .await
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn words(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }
    fn flags(parts: &[(&str, &str)]) -> Vec<(String, String)> {
        parts
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }
    #[test]
    fn invalid_flags_geometry_and_selectors_fail_before_http() {
        for options in [
            flags(&[("--unknown", "2")]),
            flags(&[("--scale", "NaN")]),
            flags(&[("--rotate", "400")]),
            flags(&[("--scale", "1"), ("--scale", "2")]),
            flags(&[("--scale", "1"), ("--scale-x", "2")]),
        ] {
            assert!(parse(&words(&["layout", "transform", "dm-overlay"]), &options).is_err());
        }
        assert!(parse(&words(&["layout", "get", ".tv-stage"]), &[]).is_err());
    }
    #[test]
    fn moves_are_translation_only_and_get_names_exact_target() {
        let Command::Move(body) = parse(
            &words(&["layout", "move", "dm-overlay"]),
            &flags(&[("--y", "-100"), ("--unit", "px")]),
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(body.patch().unwrap().translate_y.unwrap().value, -100.0);
        assert!(
            parse(
                &words(&["layout", "reset", "dm-overlay"]),
                &flags(&[("--rotate", "0")])
            )
            .is_err()
        );
    }
    #[test]
    fn hide_and_show_are_strict_component_commands() {
        let Command::Visibility { id, visible } =
            parse(&words(&["layout", "hide", "subtitle"]), &[]).unwrap()
        else {
            panic!()
        };
        assert_eq!(id, "subtitle");
        assert!(!visible);
        let Command::Visibility { id, visible } =
            parse(&words(&["layout", "show", "subtitle"]), &[]).unwrap()
        else {
            panic!()
        };
        assert_eq!(id, "subtitle");
        assert!(visible);
        assert!(parse(&words(&["layout", "hide"]), &[]).is_err());
        assert!(
            parse(
                &words(&["layout", "show", "subtitle"]),
                &flags(&[("--x", "1")])
            )
            .is_err()
        );
    }
}
