//! Server-owned conversation routing over the currently present canonical cast.
use df_protocol::common as rpc;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Target {
    Actor(String),
    Unspecified,
    Description,
    PossibleAddress,
    Ambiguous,
    Absent,
}

pub(super) struct Decision {
    pub(super) target: Target,
    pub(super) source: &'static str,
    pub(super) reason: &'static str,
    pub(super) elapsed_ms: u128,
}

/// Rule-based parse of explicit names, vocatives and descriptions. Routing passes this result to
/// Jev as an advisory hint and uses it as the addressee only when Jev cannot answer.
pub(super) fn exact(words: &str, cast: &[rpc::SceneActor]) -> Target {
    let lowered = words.trim().to_lowercase();
    let line = lowered.strip_prefix("excuse me, ").unwrap_or(&lowered);
    let prefix = [
        "i ask ",
        "i talk to ",
        "i speak to ",
        "i walk over to ",
        "to ",
    ]
    .iter()
    .find_map(|prefix| line.strip_prefix(prefix));
    let direct = prefix.unwrap_or(line).trim_start();
    let clause = direct.split([',', ':']).next().unwrap_or(direct);
    let group: Vec<_> = cast
        .iter()
        .filter(|actor| {
            clause
                .split(" and ")
                .any(|name| name.trim().eq_ignore_ascii_case(&actor.name))
        })
        .collect();
    if group.len() > 1 {
        return Target::Ambiguous;
    }
    let named: Vec<_> = cast
        .iter()
        .filter(|actor| {
            [&actor.name, &actor.id]
                .iter()
                .filter(|name| !name.is_empty())
                .any(|name| {
                    let name = name.to_lowercase();
                    [name.clone(), format!("the {name}")].iter().any(|name| {
                        direct.starts_with(&format!("{name},"))
                            || direct.starts_with(&format!("{name}:"))
                            || prefix.is_some()
                                && (direct == name || direct.starts_with(&format!("{name} ")))
                            || [format!(", {name}?"), format!(", {name}.")]
                                .iter()
                                .any(|end| line.ends_with(end))
                    })
                })
        })
        .collect();
    match named.as_slice() {
        [actor] => return Target::Actor(actor.id.clone()),
        [_, _, ..] => return Target::Ambiguous,
        _ => {}
    }
    if prefix.is_some() || line.starts_with("gentlemen") || line.starts_with("everyone") {
        return Target::Description;
    }
    if (direct.contains(',') || direct.contains(':')) && !clause.is_empty() {
        Target::PossibleAddress
    } else {
        Target::Unspecified
    }
}

pub(super) fn focus(cast: &[rpc::SceneActor], focus_name: &str) -> Target {
    let focused: Vec<_> = cast.iter().filter(|actor| actor.speaking).collect();
    if let [actor] = focused.as_slice() {
        return Target::Actor(actor.id.clone());
    }
    if focused.len() > 1 {
        return Target::Ambiguous;
    }
    let named: Vec<_> = cast
        .iter()
        .filter(|actor| actor.name == focus_name)
        .collect();
    if let [actor] = named.as_slice() {
        return Target::Actor(actor.id.clone());
    }
    if named.len() > 1 {
        return Target::Ambiguous;
    }
    match cast {
        [actor] => Target::Actor(actor.id.clone()),
        [] => Target::Unspecified,
        _ => Target::Ambiguous,
    }
}

pub(super) fn current<'a>(cast: &'a [rpc::SceneActor], id: &str) -> Option<&'a rpc::SceneActor> {
    let mut matching = cast.iter().filter(|actor| actor.id == id && !id.is_empty());
    let actor = matching.next()?;
    matching.next().is_none().then_some(actor)
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn cast() -> Vec<rpc::SceneActor> {
        vec![
            rpc::SceneActor {
                id: "courier".into(),
                name: "Courier".into(),
                speaking: true,
                ..Default::default()
            },
            rpc::SceneActor {
                id: "bandit".into(),
                name: "Bandit".into(),
                speaking: false,
                ..Default::default()
            },
        ]
    }
    #[test]
    fn explicit_target_is_distinct_from_a_person_mentioned_in_the_question() {
        let cast = cast();
        assert_eq!(
            exact(
                "Bandit, state your name and who sent you. What do you want from this courier?",
                &cast
            ),
            Target::Actor("bandit".into())
        );
        assert_eq!(
            exact("Courier, who is following you?", &cast),
            Target::Actor("courier".into())
        );
        assert_eq!(
            exact("Courier, what does Bandit want?", &cast),
            Target::Actor("courier".into())
        );
        assert_eq!(exact("Who is following you?", &cast), Target::Unspecified);
        assert_eq!(focus(&cast, "Courier"), Target::Actor("courier".into()));
        assert_eq!(
            exact("Captain, who sent you?", &cast),
            Target::PossibleAddress
        );
        assert_eq!(
            exact("Harbourmaster, who owns the crest on this packet?", &cast),
            Target::PossibleAddress
        );
        assert_eq!(
            exact("Bandit and Courier, who sent you?", &cast),
            Target::Ambiguous
        );
        assert_eq!(
            exact(
                "To the man standing behind the courier: what do you want from him?",
                &cast
            ),
            Target::Description
        );
    }
    #[test]
    fn current_target_rejects_absent_and_duplicate_ids_after_async_work() {
        let mut cast = cast();
        assert!(current(&cast, "bandit").is_some());
        cast.remove(1);
        assert!(current(&cast, "bandit").is_none());
        cast.push(cast[0].clone());
        assert!(current(&cast, "courier").is_none());
    }
}
