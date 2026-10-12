//! The room's table settings. A seated player changes them from a phone (`SET_TABLE_SETTING`);
//! the server owns and stores them, and every view carries them so each screen shows the same
//! choice. Presentation only: no setting changes a rule, a roll or an outcome.
use super::journey;
use df_model::checkpoint::Checkpoint;
use df_persistence::local_demo_scope::StoredTableSettings;
use df_protocol::common as rpc;
use df_types::MemberId;

/// Whether `member` holds a seat in this room. Only seated players may change the table.
pub(super) fn may_change(current: &Checkpoint, member: MemberId) -> bool {
    journey::participants(current)
        .iter()
        .any(|link| link.member == member)
}

/// What the table has chosen for the room.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TableSettings {
    /// Hides the numbered choice chips on the shared display (the narrator box takes their room).
    pub(super) choice_hints_hidden: bool,
}

impl TableSettings {
    /// The settings in force at startup: a choice a player stored earlier wins over the
    /// `DF_GAMEPLAY_CHOICE_HINTS` default.
    pub(super) fn startup(stored: Option<StoredTableSettings>, default_hidden: bool) -> Self {
        Self {
            choice_hints_hidden: stored.map_or(default_hidden, |stored| stored.choice_hints_hidden),
        }
    }

    pub(super) fn stored(self) -> StoredTableSettings {
        StoredTableSettings {
            choice_hints_hidden: self.choice_hints_hidden,
        }
    }

    /// The settings after one requested change. A change without a value is refused.
    pub(super) fn changed(self, change: &rpc::TableSettingChange) -> Result<Self, tonic::Status> {
        match change.value {
            Some(rpc::table_setting_change::Value::ChoiceHintsHidden(hidden)) => Ok(Self {
                choice_hints_hidden: hidden,
            }),
            None => Err(tonic::Status::invalid_argument(
                "table setting value required",
            )),
        }
    }

    /// Writes the settings into the journey of whichever audience the view is for.
    pub(super) fn attach(self, view: &mut rpc::ViewMessage) {
        let journey = match view.audience.as_mut() {
            Some(rpc::view_message::Audience::Player(player)) => player.journey.as_mut(),
            Some(rpc::view_message::Audience::Display(display)) => display.journey.as_mut(),
            None => None,
        };
        if let Some(journey) = journey {
            journey.choice_hints_hidden = self.choice_hints_hidden;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(hidden: bool) -> rpc::TableSettingChange {
        rpc::TableSettingChange {
            value: Some(rpc::table_setting_change::Value::ChoiceHintsHidden(hidden)),
        }
    }

    fn display_view() -> rpc::ViewMessage {
        rpc::ViewMessage {
            revision: None,
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView {
                    journey: Some(rpc::JourneyView::default()),
                    ..Default::default()
                },
            )),
        }
    }

    #[test]
    fn a_stored_choice_overrides_the_startup_default_and_absence_keeps_it() {
        let stored = Some(StoredTableSettings {
            choice_hints_hidden: false,
        });
        assert!(!TableSettings::startup(stored, true).choice_hints_hidden);
        assert!(TableSettings::startup(None, true).choice_hints_hidden);
        assert!(!TableSettings::startup(None, false).choice_hints_hidden);
    }

    #[test]
    fn a_change_sets_the_value_and_an_empty_change_is_refused() {
        let shown = TableSettings::startup(None, false);
        let hidden = shown.changed(&change(true)).unwrap();
        assert!(hidden.choice_hints_hidden);
        assert_eq!(
            hidden.stored(),
            StoredTableSettings {
                choice_hints_hidden: true
            }
        );
        let refused = shown
            .changed(&rpc::TableSettingChange { value: None })
            .unwrap_err();
        assert_eq!(refused.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn only_a_joined_seat_may_change_the_table() {
        let mut state = journey::initial().unwrap().state().clone();
        let seated = MemberId::from_bytes(&[0x71; 16]).unwrap();
        state.members.push(df_model::checkpoint::MembershipLink {
            member: seated,
            character: None,
        });
        let checkpoint =
            super::super::model::checkpoint(super::super::model::basis().unwrap(), state).unwrap();
        assert!(may_change(&checkpoint, seated));
        assert!(!may_change(
            &checkpoint,
            MemberId::from_bytes(&[0x72; 16]).unwrap()
        ));
        assert!(!may_change(
            &checkpoint,
            journey::bootstrap_member().unwrap()
        ));
    }

    #[test]
    fn every_audience_journey_carries_the_settings() {
        let hidden = TableSettings {
            choice_hints_hidden: true,
        };
        let mut display = display_view();
        hidden.attach(&mut display);
        let Some(rpc::view_message::Audience::Display(shown)) = display.audience else {
            panic!("display audience expected");
        };
        assert!(
            shown
                .journey
                .is_some_and(|journey| journey.choice_hints_hidden)
        );
        let mut player = rpc::ViewMessage {
            revision: None,
            audience: Some(rpc::view_message::Audience::Player(
                rpc::PlayerGameplayView {
                    journey: Some(rpc::JourneyView::default()),
                    ..Default::default()
                },
            )),
        };
        hidden.attach(&mut player);
        let Some(rpc::view_message::Audience::Player(seen)) = player.audience else {
            panic!("player audience expected");
        };
        assert!(
            seen.journey
                .is_some_and(|journey| journey.choice_hints_hidden)
        );
    }
}
