//! Server-authored phone choices, shared with native rendering-contract tests.
use df_protocol::common as rpc;

pub(super) fn is_simple_offer(offer: &rpc::GameplayActionOffer) -> bool {
    (offer.input_groups.is_empty()
        || offer.action_kind == rpc::GameplayActionKind::Travel as i32
        || offer
            .input_groups
            .iter()
            .any(|group| group.group_id == "dialogue-addressee"))
        && offer.destinations.len() <= 1
        && !matches!(
            rpc::GameplayActionKind::try_from(offer.action_kind),
            Ok(rpc::GameplayActionKind::CreateCharacter
                | rpc::GameplayActionKind::GreatswordAttack)
        )
}

pub(super) fn phone_offer_choices(offer: &rpc::GameplayActionOffer) -> Vec<(String, String)> {
    if offer.action_kind == rpc::GameplayActionKind::Travel as i32 {
        return offer
            .input_groups
            .iter()
            .filter(|group| group.group_id == "travel")
            .flat_map(|group| {
                group.options.iter().map(|option| {
                    (
                        format!("{}|{}", offer.offer_id, option.option_id),
                        option.label.clone(),
                    )
                })
            })
            .collect();
    }
    vec![(offer.offer_id.clone(), offer.label.clone())]
}

#[cfg(test)]
mod hs28_phone_tests {
    use super::*;

    #[test]
    fn hs28_phone_renders_server_travel_options_and_talk_offer() {
        let travel = rpc::GameplayActionOffer {
            offer_id: "travel:1:42".into(),
            action_kind: rpc::GameplayActionKind::Travel as i32,
            label: "Travel".into(),
            input_groups: vec![rpc::JourneyGroup {
                group_id: "travel".into(),
                label: "Where do you go?".into(),
                options: vec![
                    rpc::JourneyOption {
                        option_id: "exit:market-road".into(),
                        label: "Travel via market road".into(),
                        ..Default::default()
                    },
                    rpc::JourneyOption {
                        option_id: "known:place-next-town".into(),
                        label: "Travel to Next Town".into(),
                        ..Default::default()
                    },
                ],
            }],
            ..Default::default()
        };
        assert!(is_simple_offer(&travel));
        assert_eq!(
            phone_offer_choices(&travel),
            vec![
                (
                    "travel:1:42|exit:market-road".into(),
                    "Travel via market road".into()
                ),
                (
                    "travel:1:42|known:place-next-town".into(),
                    "Travel to Next Town".into()
                ),
            ]
        );
        let talk = rpc::GameplayActionOffer {
            offer_id: "opening-talk:bandit:1:42".into(),
            label: "Try another offer".into(),
            input_groups: vec![rpc::JourneyGroup {
                group_id: "dialogue-addressee".into(),
                options: vec![rpc::JourneyOption {
                    option_id: "bandit".into(),
                    label: "Dain Rusk".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(is_simple_offer(&talk));
        assert_eq!(
            phone_offer_choices(&talk),
            vec![(talk.offer_id.clone(), "Try another offer".into())]
        );
    }
}
