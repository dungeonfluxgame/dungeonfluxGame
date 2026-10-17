//! Server-owned conversation routing over the currently present canonical cast and the other
//! party heroes at the table.
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

/// Another hero at the table who may be addressed by name. `id` is the hero's member id.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PartyHero {
    pub(super) id: String,
    pub(super) name: String,
}

/// A party hero the line is addressed to. `private_aside` is Jev's option label, never keywords.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HeroAddressee {
    pub(super) id: String,
    pub(super) private_aside: bool,
}

/// Rule-based addressee: a present-cast target or a party hero by member id.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ExactAddressee {
    Npc(Target),
    Hero(String),
}

pub(super) struct Decision {
    pub(super) target: Target,
    /// Set when the addressee is a party hero. `target` is then `Absent`, so a caller that ignores
    /// this field fails visibly instead of speaking to the focus.
    pub(super) hero: Option<HeroAddressee>,
    pub(super) source: &'static str,
    pub(super) reason: &'static str,
    pub(super) elapsed_ms: u128,
}

/// Rule-based parse of explicit names, vocatives and descriptions over the present cast only.
pub(super) fn exact(words: &str, cast: &[rpc::SceneActor]) -> Target {
    match exact_with_party(words, cast, &[]) {
        ExactAddressee::Npc(target) => target,
        // An empty party cannot match a hero.
        ExactAddressee::Hero(_) => Target::Absent,
    }
}

/// Rule-based parse of explicit names, vocatives and descriptions. Routing passes this result to
/// Jev as an advisory hint and uses it as the addressee only when Jev cannot answer. Party heroes
/// match by name exactly as present actors do, and a hero named as the recipient after "to" is
/// also hinted (for example "I hand the cup to Sigrun").
pub(super) fn exact_with_party(
    words: &str,
    cast: &[rpc::SceneActor],
    party: &[PartyHero],
) -> ExactAddressee {
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
    let in_clause = |name: &str| {
        clause
            .split(" and ")
            .any(|part| part.trim().eq_ignore_ascii_case(name))
    };
    let group = cast.iter().filter(|actor| in_clause(&actor.name)).count()
        + party.iter().filter(|hero| in_clause(&hero.name)).count();
    if group > 1 {
        return ExactAddressee::Npc(Target::Ambiguous);
    }
    let prefixed = prefix.is_some();
    let named_actors: Vec<_> = cast
        .iter()
        .filter(|actor| {
            addresses_by_name(
                direct,
                line,
                prefixed,
                &[actor.name.as_str(), actor.id.as_str()],
            )
        })
        .collect();
    let named_heroes: Vec<_> = party
        .iter()
        .filter(|hero| addresses_by_name(direct, line, prefixed, &[hero.name.as_str()]))
        .collect();
    match (named_actors.as_slice(), named_heroes.as_slice()) {
        ([actor], []) => return ExactAddressee::Npc(Target::Actor(actor.id.clone())),
        ([], [hero]) => return ExactAddressee::Hero(hero.id.clone()),
        ([], []) => {}
        _ => return ExactAddressee::Npc(Target::Ambiguous),
    }
    let recipients: Vec<_> = party
        .iter()
        .filter(|hero| recipient_after_to(line, &hero.name))
        .collect();
    match recipients.as_slice() {
        [hero] => return ExactAddressee::Hero(hero.id.clone()),
        [_, _, ..] => return ExactAddressee::Npc(Target::Ambiguous),
        [] => {}
    }
    if prefix.is_some() || line.starts_with("gentlemen") || line.starts_with("everyone") {
        return ExactAddressee::Npc(Target::Description);
    }
    if (direct.contains(',') || direct.contains(':')) && !clause.is_empty() {
        ExactAddressee::Npc(Target::PossibleAddress)
    } else {
        ExactAddressee::Npc(Target::Unspecified)
    }
}

/// Whether the line opens with, or closes on, one of `names` as its addressee.
fn addresses_by_name(direct: &str, line: &str, prefixed: bool, names: &[&str]) -> bool {
    names.iter().filter(|name| !name.is_empty()).any(|name| {
        let name = name.to_lowercase();
        [name.clone(), format!("the {name}")].iter().any(|name| {
            direct.starts_with(&format!("{name},"))
                || direct.starts_with(&format!("{name}:"))
                || prefixed && (direct == name || direct.starts_with(&format!("{name} ")))
                || [format!(", {name}?"), format!(", {name}.")]
                    .iter()
                    .any(|end| line.ends_with(end))
        })
    })
}

/// Whether the line names `name` as the recipient after "to", such as "hand it to Sigrun".
fn recipient_after_to(line: &str, name: &str) -> bool {
    let name = name.to_lowercase();
    !name.is_empty()
        && line
            .match_indices(&format!(" to {name}"))
            .any(|(start, matched)| {
                line[start + matched.len()..]
                    .chars()
                    .next()
                    .is_none_or(|next| !next.is_alphanumeric())
            })
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
    fn party() -> Vec<PartyHero> {
        vec![
            PartyHero {
                id: "sigrun-member".into(),
                name: "Sigrun".into(),
            },
            PartyHero {
                id: "brakka-member".into(),
                name: "Brakka".into(),
            },
        ]
    }
    fn npc(target: Target) -> ExactAddressee {
        ExactAddressee::Npc(target)
    }
    #[test]
    fn explicit_target_is_distinct_from_a_person_mentioned_in_the_question() {
        let cast = cast();
        assert_eq!(
            exact(
                "Bandit, state your name and who sent you. What do you want from this courier?",
                &cast,
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
                &cast,
            ),
            Target::Description
        );
    }
    #[test]
    fn party_hero_is_addressed_by_vocative_or_by_name_after_to_without_jev() {
        let cast = cast();
        let party = party();
        assert_eq!(
            exact_with_party(
                "Sigrun, should we head to the market after this?",
                &cast,
                &party
            ),
            ExactAddressee::Hero("sigrun-member".into())
        );
        assert_eq!(
            exact_with_party(
                "I lean over and whisper to Sigrun so Bandit can't hear: I think he overcharged me.",
                &cast,
                &party
            ),
            ExactAddressee::Hero("sigrun-member".into())
        );
        assert_eq!(
            exact_with_party("I hand the traveler's clothes to Sigrun.", &cast, &party),
            ExactAddressee::Hero("sigrun-member".into())
        );
        assert_eq!(
            exact_with_party("I walk over to Brakka.", &cast, &party),
            ExactAddressee::Hero("brakka-member".into())
        );
        assert_eq!(
            exact_with_party("Sigrun, who is following you?", &cast, &party),
            ExactAddressee::Hero("sigrun-member".into())
        );
    }
    #[test]
    fn hero_name_inside_a_longer_word_or_a_question_subject_is_not_an_addressee() {
        let cast = cast();
        let party = party();
        assert_eq!(
            exact_with_party("Who is Sigrun's brother?", &cast, &party),
            npc(Target::Unspecified)
        );
        assert_eq!(
            exact_with_party("I give the coins to Sigruna.", &cast, &party),
            npc(Target::Unspecified)
        );
        assert_eq!(
            exact_with_party("Courier, what does Sigrun want?", &cast, &party),
            npc(Target::Actor("courier".into()))
        );
    }
    #[test]
    fn two_named_heroes_or_a_hero_and_an_actor_are_ambiguous() {
        let cast = cast();
        let party = party();
        assert_eq!(
            exact_with_party("Sigrun and Brakka, who sent you?", &cast, &party),
            npc(Target::Ambiguous)
        );
        assert_eq!(
            exact_with_party("Sigrun and Courier, who sent you?", &cast, &party),
            npc(Target::Ambiguous)
        );
        assert_eq!(
            exact_with_party("I hand this to Sigrun and to Brakka.", &cast, &party),
            npc(Target::Ambiguous)
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
